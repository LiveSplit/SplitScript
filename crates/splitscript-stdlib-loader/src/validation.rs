//! Validation for the privileged source model before Rust catalog emission.
//!
//! This is deliberately source-facing validation. The compiler separately
//! validates the generated graph against its closed intrinsic/runtime trust
//! registries; the loader only guarantees that generation is total and that
//! every source-level reference is structurally meaningful.

use std::collections::{HashMap, HashSet};

use splitscript_syntax::PrimitiveType;

use crate::{
    Attribute, AttributeArgument, CallableOwnerDeclaration, Declaration, Documentation, Error,
    FunctionDeclaration, Library, StructDeclaration, Type, TypeParameter,
    visible_capability_associated_types,
};

pub(crate) fn validate(library: &Library) -> Vec<Error> {
    let mut validator = Validator::new(library);
    validator.validate();
    validator.errors
}

struct Validator<'a> {
    library: &'a Library,
    types: HashSet<&'a str>,
    private_types: HashSet<&'a str>,
    capabilities: HashSet<&'a str>,
    constructors: HashMap<&'a str, usize>,
    generated_items: HashSet<String>,
    example_owners: HashMap<String, String>,
    errors: Vec<Error>,
}

impl<'a> Validator<'a> {
    fn new(library: &'a Library) -> Self {
        let types = library
            .declarations
            .iter()
            .filter_map(|declaration| match declaration {
                Declaration::Struct(value) | Declaration::IntrinsicType(value) => {
                    Some(value.name.as_str())
                }
                Declaration::Enum(value) => Some(value.name.as_str()),
                _ => None,
            })
            .collect();
        let capabilities = library
            .declarations
            .iter()
            .filter_map(|declaration| match declaration {
                Declaration::Capability(value) => Some(value.name.as_str()),
                _ => None,
            })
            .collect();
        let private_types = library
            .declarations
            .iter()
            .filter_map(|declaration| match declaration {
                Declaration::Struct(value) | Declaration::IntrinsicType(value) if value.private => {
                    Some(value.name.as_str())
                }
                Declaration::Enum(value) if value.private => Some(value.name.as_str()),
                Declaration::TypeConstructor(value) if value.private => Some(value.name.as_str()),
                _ => None,
            })
            .collect();
        let constructors = library
            .declarations
            .iter()
            .filter_map(|declaration| match declaration {
                Declaration::TypeConstructor(value) => {
                    Some((value.name.as_str(), value.type_parameters.len()))
                }
                _ => None,
            })
            .collect();
        let generated_items = library
            .declarations
            .iter()
            .flat_map(declaration_items)
            .collect();
        Self {
            library,
            types,
            private_types,
            capabilities,
            constructors,
            generated_items,
            example_owners: HashMap::new(),
            errors: Vec::new(),
        }
    }

    fn validate(&mut self) {
        // State-provider names are resolved only after the `state` keyword;
        // ordinary declarations live in the value/type/namespace grammar.
        // Keep their uniqueness domains separate so `state Unity ...` can
        // coexist with the `Unity.*` API namespace without weakening either
        // domain's duplicate checking.
        let mut declaration_names = HashSet::new();
        let mut provider_names = HashSet::new();
        for declaration in &self.library.declarations {
            let name = declaration_name(declaration);
            let names = if matches!(declaration, Declaration::StateProvider(_)) {
                &mut provider_names
            } else {
                &mut declaration_names
            };
            if !names.insert(name) {
                self.error(format!("standard-library declaration `{name}` is repeated"));
            }
            match declaration {
                Declaration::Struct(value) | Declaration::IntrinsicType(value) => {
                    self.validate_struct(value)
                }
                Declaration::Enum(value) => {
                    let public = !value.private && !has_attribute(&value.attributes, "testOnly");
                    self.validate_documentation(&value.name, &value.documentation, false, public);
                    self.validate_attributes(
                        &value.name,
                        &value.attributes,
                        &["representation", "valueUsage", "capabilities", "testOnly"],
                    );
                    self.validate_representation(&value.name, &value.attributes, "enum");
                    self.validate_value_usage(&value.name, &value.attributes);
                    self.validate_capabilities(&value.name, &value.attributes);
                    let mut variants = HashSet::new();
                    for variant in &value.variants {
                        if !variants.insert(variant.name.as_str()) {
                            self.error(format!(
                                "enum `{}` repeats variant `{}`",
                                value.name, variant.name
                            ));
                        }
                        self.validate_documentation(
                            &format!("{}.{}", value.name, variant.name),
                            &variant.documentation,
                            false,
                            public,
                        );
                        self.validate_attributes(
                            &format!("{}.{}", value.name, variant.name),
                            &variant.attributes,
                            &[],
                        );
                    }
                }
                Declaration::Root(value) => {
                    self.validate_attributes("root", &value.attributes, &[]);
                    self.validate_functions("root", &value.functions, &[]);
                }
                Declaration::Namespace(value) => self.validate_owner(value, &[], true),
                Declaration::Capability(value) => {
                    self.validate_attributes(&value.name, &value.attributes, &["behavior"]);
                    self.require_name_attribute(
                        &value.name,
                        &value.attributes,
                        "behavior",
                        |name| {
                            matches!(
                                name,
                                "declared"
                                    | "structuralEquality"
                                    | "structuralMemoryLayout"
                                    | "structuralManagedRead"
                                    | "structuralMethods"
                            )
                        },
                    );
                    if value.type_parameters.len() != 1 {
                        self.error(format!(
                            "capability `{}` must have exactly one implicit receiver type",
                            value.name
                        ));
                    }
                    let visible = visible_capability_associated_types(self.library, value);
                    let mut owners = HashMap::new();
                    for (declaring_capability, associated) in &visible {
                        if let Some(previous) =
                            owners.insert(associated.name.as_str(), *declaring_capability)
                            && previous != *declaring_capability
                        {
                            self.error(format!(
                                "capability `{}` inherits ambiguous associated type `{}` from `{previous}` and `{declaring_capability}`",
                                value.name, associated.name
                            ));
                        }
                    }
                    let mut available = value.type_parameters.clone();
                    available.extend(
                        visible
                            .into_iter()
                            .filter(|(declaring_capability, _)| *declaring_capability != value.name)
                            .map(|(_, associated)| TypeParameter {
                                name: associated.name.clone(),
                                constraints: associated.constraints.clone(),
                            }),
                    );
                    self.validate_owner(value, &available, true);
                }
                Declaration::TypeConstructor(value) => {
                    self.validate_attributes(
                        &value.name,
                        &value.attributes,
                        &["mustUse", "capabilities"],
                    );
                    self.validate_must_use(&value.name, &value.attributes);
                    self.validate_capabilities(&value.name, &value.attributes);
                    self.validate_owner(value, &value.type_parameters, true);
                }
                Declaration::CoreExtension(value) => {
                    self.validate_attributes(&value.name, &value.attributes, &[]);
                    if PrimitiveType::parse(&value.name).is_none() {
                        self.error(format!(
                            "core extension `{}` does not name a primitive type",
                            value.name
                        ));
                    }
                    self.validate_functions(&value.name, &value.functions, &[]);
                }
                Declaration::StateProvider(value) => self.validate_provider(value),
            }
        }
        self.validate_capability_hierarchy();
        self.validate_private_type_boundaries();
    }

    fn validate_private_type_boundaries(&mut self) {
        for declaration in &self.library.declarations {
            match declaration {
                Declaration::Struct(value) | Declaration::IntrinsicType(value) => {
                    if value.private {
                        continue;
                    }
                    for field in value.fields.iter().filter(|field| !field.private) {
                        self.validate_public_type_ref(
                            &format!("{}.{}", value.name, field.name),
                            &field.ty,
                        );
                    }
                    self.validate_public_function_types(&value.name, &value.functions);
                }
                Declaration::TypeConstructor(value) => {
                    if value.private {
                        continue;
                    }
                    self.validate_public_function_types(&value.name, &value.functions);
                    for field in value.fields.iter().filter(|field| !field.private) {
                        self.validate_public_type_ref(
                            &format!("{}.{}", value.name, field.name),
                            &field.ty,
                        );
                    }
                }
                Declaration::Root(value)
                | Declaration::Namespace(value)
                | Declaration::Capability(value)
                | Declaration::CoreExtension(value) => {
                    self.validate_public_function_types(&value.name, &value.functions);
                    for field in value.fields.iter().filter(|field| !field.private) {
                        self.validate_public_type_ref(
                            &format!("{}.{}", value.name, field.name),
                            &field.ty,
                        );
                    }
                }
                Declaration::StateProvider(value) => {
                    if value.attributes.iter().any(|attribute| {
                        attribute.name == "processType"
                            && matches!(attribute.arguments.as_slice(), [AttributeArgument::Name(name)] if self.private_types.contains(name.as_str()))
                    }) {
                        self.error(format!(
                            "state provider `{}` cannot expose a private process type",
                            value.name
                        ));
                    }
                }
                Declaration::Enum(_) => {}
            }
        }
    }

    fn validate_public_function_types(&mut self, owner: &str, functions: &[FunctionDeclaration]) {
        for function in functions.iter().filter(|function| !function.private) {
            let qualified = if owner == "root" {
                function.name.clone()
            } else {
                format!("{owner}.{}", function.name)
            };
            for parameter in &function.parameters {
                self.validate_public_type_ref(
                    &format!("{qualified} parameter `{}`", parameter.name),
                    &parameter.ty,
                );
            }
            self.validate_public_type_ref(&format!("{qualified} result"), &function.result);
        }
    }

    fn validate_public_type_ref(&mut self, owner: &str, ty: &Type) {
        match ty {
            Type::Name(name) => {
                if self.private_types.contains(name.as_str()) {
                    self.error(format!(
                        "public standard-library surface `{owner}` exposes private type `{name}`"
                    ));
                }
            }
            Type::Async(value)
            | Type::Iterator(value)
            | Type::Array(value)
            | Type::Option(value)
            | Type::Result(value)
            | Type::ExclusiveRange(value)
            | Type::InclusiveRange(value) => self.validate_public_type_ref(owner, value),
            Type::FixedArray { element, .. } => self.validate_public_type_ref(owner, element),
            Type::Application {
                constructor,
                arguments,
            } => {
                if self.private_types.contains(constructor.as_str()) {
                    self.error(format!(
                        "public standard-library surface `{owner}` exposes private type constructor `{constructor}`"
                    ));
                }
                for argument in arguments {
                    self.validate_public_type_ref(owner, argument);
                }
            }
            Type::Callable { parameters, result } => {
                for parameter in parameters {
                    self.validate_public_type_ref(owner, parameter);
                }
                self.validate_public_type_ref(owner, result);
            }
        }
    }

    fn validate_capability_hierarchy(&mut self) {
        let hierarchy = self
            .library
            .declarations
            .iter()
            .filter_map(|declaration| match declaration {
                Declaration::Capability(capability) => Some((
                    capability.name.as_str(),
                    capability
                        .type_parameters
                        .first()
                        .map(|parameter| parameter.constraints.as_slice())
                        .unwrap_or_default(),
                )),
                _ => None,
            })
            .collect::<HashMap<_, _>>();
        let mut completed = HashSet::new();
        for capability in hierarchy.keys().copied() {
            let mut active = HashSet::new();
            if capability_hierarchy_has_cycle(capability, &hierarchy, &mut active, &mut completed) {
                self.error(format!(
                    "capability hierarchy contains a cycle through `{capability}`"
                ));
                break;
            }
        }
    }

    fn validate_struct(&mut self, value: &StructDeclaration) {
        let public = !value.private && !has_attribute(&value.attributes, "testOnly");
        self.validate_documentation(&value.name, &value.documentation, false, public);
        self.validate_attributes(
            &value.name,
            &value.attributes,
            &[
                "representation",
                "valueUsage",
                "capabilities",
                "testOnly",
                "publicConstruction",
            ],
        );
        if has_attribute(&value.attributes, "publicConstruction") {
            let arguments =
                self.attribute_names(&value.name, &value.attributes, "publicConstruction", false);
            if !arguments.is_empty()
                || value.private
                || value.fields.iter().any(|field| field.private)
                || !self
                    .attribute_names(&value.name, &value.attributes, "representation", true)
                    .first()
                    .is_some_and(|name| name == "gcStruct")
            {
                self.error(format!("`{}`: @publicConstruction requires a public GC struct with public fields and no attribute arguments", value.name));
            }
        }
        self.validate_representation(&value.name, &value.attributes, "");
        self.validate_value_usage(&value.name, &value.attributes);
        self.validate_capabilities(&value.name, &value.attributes);
        let owner = CallableOwnerDeclaration {
            name: value.name.clone(),
            private: !public,
            type_constructor_syntax: None,
            type_parameters: Vec::new(),
            documentation: value.documentation.clone(),
            attributes: value.attributes.clone(),
            fields: value.fields.clone(),
            associated_types: value.associated_types.clone(),
            functions: value.functions.clone(),
        };
        self.validate_owner_members(&owner, &[], public);
        if owner
            .functions
            .iter()
            .filter(|function| has_attribute(&function.attributes, "display"))
            .count()
            > 1
        {
            self.error(format!(
                "standard-library type `{}` has multiple display implementations",
                owner.name
            ));
        }
        // The shared owner-member validation keeps fields, associated types,
        // and functions in one type scope so members can refer to the concrete
        // associated definitions.
    }

    fn validate_owner(
        &mut self,
        owner: &CallableOwnerDeclaration,
        inherited: &[TypeParameter],
        public: bool,
    ) {
        self.validate_documentation(&owner.name, &owner.documentation, false, public);
        self.validate_type_parameters(&owner.name, &owner.type_parameters);
        self.validate_owner_members(owner, inherited, public);
    }

    fn validate_owner_members(
        &mut self,
        owner: &CallableOwnerDeclaration,
        inherited: &[TypeParameter],
        public: bool,
    ) {
        let mut available_types = inherited.to_vec();
        let mut names = owner
            .type_parameters
            .iter()
            .map(|parameter| parameter.name.as_str())
            .collect::<HashSet<_>>();
        for associated in &owner.associated_types {
            let qualified = format!("{}.{}", owner.name, associated.name);
            if !names.insert(associated.name.as_str()) {
                self.error(format!(
                    "`{}` repeats associated type `{}`",
                    owner.name, associated.name
                ));
            }
            if public && associated.documentation.summary.trim().is_empty() {
                self.error(format!("`{qualified}` is missing documentation"));
            }
            let mut constraints = HashSet::new();
            for constraint in &associated.constraints {
                if !constraints.insert(constraint.as_str()) {
                    self.error(format!(
                        "`{qualified}` repeats capability constraint `{constraint}`"
                    ));
                }
                if !self.capabilities.contains(constraint.as_str()) {
                    self.error(format!(
                        "`{qualified}` references unknown capability `{constraint}`"
                    ));
                }
            }
            available_types.push(TypeParameter {
                name: associated.name.clone(),
                constraints: associated.constraints.clone(),
            });
        }
        for associated in &owner.associated_types {
            if let Some(value) = &associated.value {
                self.validate_type(
                    &format!("{}.{}", owner.name, associated.name),
                    value,
                    &available_types,
                );
            }
        }
        self.validate_fields(&owner.name, &owner.fields, &available_types, public);
        self.validate_functions(&owner.name, &owner.functions, &available_types);
    }

    fn validate_fields(
        &mut self,
        owner: &str,
        fields: &[crate::FieldDeclaration],
        inherited: &[TypeParameter],
        public: bool,
    ) {
        let mut names = HashSet::new();
        for field in fields {
            let qualified = format!("{owner}.{}", field.name);
            if !names.insert(field.name.as_str()) {
                self.error(format!("type `{owner}` repeats field `{}`", field.name));
            }
            self.validate_documentation(
                &qualified,
                &field.documentation,
                false,
                public && !field.private,
            );
            self.validate_attributes(&qualified, &field.attributes, &[]);
            self.validate_type(&qualified, &field.ty, inherited);
        }
    }

    fn validate_functions(
        &mut self,
        owner: &str,
        functions: &[FunctionDeclaration],
        inherited: &[TypeParameter],
    ) {
        let mut names = HashSet::new();
        let mut overload_names = HashSet::new();
        for function in functions {
            if !names.insert(function.name.as_str()) {
                overload_names.insert(function.name.as_str());
            }
        }
        names.clear();
        for name in &overload_names {
            let cases = functions
                .iter()
                .filter(|function| function.name == **name)
                .collect::<Vec<_>>();
            if cases
                .iter()
                .any(|function| function.private != cases[0].private)
            {
                self.error(format!(
                    "`{owner}.{name}` implementation cases must have the same visibility"
                ));
            }
            self.validate_capability_overload(owner, name, &cases, inherited);
        }
        let mut operator_bindings = HashSet::new();
        for function in functions {
            let qualified = if owner == "root" {
                function.name.clone()
            } else {
                format!("{owner}.{}", function.name)
            };
            let first_declaration = names.insert(function.name.as_str());
            if function.private {
                if function.documentation != Documentation::default() {
                    self.error(format!(
                        "private standard-library helper `{qualified}` must use ordinary comments instead of public documentation comments"
                    ));
                }
            } else if first_declaration || !overload_names.contains(function.name.as_str()) {
                self.validate_documentation(
                    &qualified,
                    &function.documentation,
                    true,
                    !function.private,
                );
            } else if function.documentation != Documentation::default() {
                self.error(format!(
                    "`{qualified}` implementation cases must document the public operation only once"
                ));
            }
            self.validate_attributes(
                &qualified,
                &function.attributes,
                &[
                    "availability",
                    "cancellation",
                    "intrinsic",
                    "display",
                    "mustUse",
                    "operator",
                    "requires",
                ],
            );
            self.validate_must_use(&qualified, &function.attributes);
            self.validate_operator(owner, &qualified, function);
            if function.private
                && function
                    .attributes
                    .iter()
                    .any(|attribute| matches!(attribute.name.as_str(), "display" | "operator"))
            {
                self.error(format!(
                    "private standard-library helper `{qualified}` cannot define a public display or operator binding"
                ));
            }
            if let Some([AttributeArgument::Name(operator)]) = function
                .attributes
                .iter()
                .find(|attribute| attribute.name == "operator")
                .map(|attribute| attribute.arguments.as_slice())
                && matches!(
                    operator.as_str(),
                    "add"
                        | "subtract"
                        | "lessThan"
                        | "lessThanOrEqual"
                        | "greaterThan"
                        | "greaterThanOrEqual"
                )
                && !operator_bindings.insert(operator.as_str())
            {
                self.error(format!(
                    "`{owner}` declares more than one `{operator}` operator implementation"
                ));
            }
            if has_attribute(&function.attributes, "mustUse")
                && function.result == Type::Name("None".to_owned())
            {
                self.error(format!(
                    "`{qualified}` cannot be `@mustUse` because it returns `None`"
                ));
            }
            if has_attribute(&function.attributes, "display") {
                if function
                    .attributes
                    .iter()
                    .find(|attribute| attribute.name == "display")
                    .is_some_and(|attribute| !attribute.arguments.is_empty())
                {
                    self.error(format!(
                        "`{qualified}` marker attribute `@display` does not accept arguments"
                    ));
                }
                let owner_is_standard_type = self.types.contains(owner);
                let owner_has_display = self.library.declarations.iter().any(|declaration| {
                    matches!(
                        declaration,
                        Declaration::Struct(value) | Declaration::IntrinsicType(value)
                            if value.name == owner
                                && value.attributes.iter().any(|attribute| {
                                    attribute.name == "capabilities"
                                        && attribute.arguments.iter().any(|argument| {
                                            matches!(
                                                argument,
                                                AttributeArgument::Name(capability)
                                                    if capability == "Display"
                                            )
                                        })
                                })
                    )
                });
                if !owner_is_standard_type {
                    self.error(format!(
                        "`{qualified}` uses `@display` outside a standard-library type"
                    ));
                }
                if function.is_static || !function.parameters.is_empty() {
                    self.error(format!(
                        "`{qualified}` display implementation must be a parameterless method"
                    ));
                }
                if function.result != Type::Name("String".to_owned()) {
                    self.error(format!(
                        "`{qualified}` display implementation must return `String`"
                    ));
                }
                if !owner_has_display {
                    self.error(format!(
                        "`{qualified}` provides a display implementation but `{owner}` does not declare `Display`"
                    ));
                }
                if function.body.is_none() {
                    self.error(format!(
                        "`{qualified}` display implementation must have a source body"
                    ));
                }
            }
            let intrinsic = self
                .optional_name_attribute(&qualified, &function.attributes, "intrinsic")
                .is_some();
            self.validate_intrinsic_context(&qualified, function, intrinsic);
            let capability_requirement = function.body.is_none()
                && !function
                    .attributes
                    .iter()
                    .any(|attribute| attribute.name == "intrinsic")
                && self.library.declarations.iter().any(|declaration| {
                    let Declaration::Capability(capability) = declaration else {
                        return false;
                    };
                    capability.name == owner
                });
            match (intrinsic, function.body.is_some(), capability_requirement) {
                (true, true, _) => self.error(format!(
                    "`{qualified}` cannot have both an intrinsic binding and a source body"
                )),
                (false, false, false) => self.error(format!(
                    "`{qualified}` must have either an intrinsic binding or a source body"
                )),
                (true, false, true) => self.error(format!(
                    "`{qualified}` capability requirements cannot bind an intrinsic"
                )),
                _ => {}
            }
            if capability_requirement && function.is_static {
                self.error(format!(
                    "`{qualified}` capability requirement must be a receiver method"
                ));
            }
            self.validate_type_parameters(&qualified, &function.type_parameters);
            let parameters =
                self.effective_function_type_parameters(&qualified, function, inherited);
            let mut parameter_names = HashSet::new();
            for parameter in &function.parameters {
                let parameter_owner = format!("{qualified}.{}", parameter.name);
                if !parameter_names.insert(parameter.name.as_str()) {
                    self.error(format!(
                        "`{qualified}` repeats parameter `{}`",
                        parameter.name
                    ));
                }
                if !function.private {
                    self.validate_documentation(
                        &parameter_owner,
                        &parameter.documentation,
                        false,
                        false,
                    );
                } else if parameter.documentation != Documentation::default() {
                    self.error(format!(
                        "private standard-library parameter `{parameter_owner}` must use ordinary comments instead of public documentation comments"
                    ));
                }
                self.validate_attributes(&parameter_owner, &parameter.attributes, &["literal"]);
                if let Some(rule) =
                    self.optional_name_attribute(&parameter_owner, &parameter.attributes, "literal")
                    && !matches!(rule, "string" | "signature")
                {
                    self.error(format!(
                        "`{parameter_owner}` has unknown literal rule `{rule}`"
                    ));
                }
                self.validate_type(&parameter_owner, &parameter.ty, &parameters);
            }
            self.validate_type(
                &format!("{qualified} result"),
                &function.result,
                &parameters,
            );
        }
    }

    fn validate_intrinsic_context(
        &mut self,
        qualified: &str,
        function: &FunctionDeclaration,
        intrinsic: bool,
    ) {
        let context_attributes = function.attributes.iter().filter(|attribute| {
            matches!(
                attribute.name.as_str(),
                "availability" | "cancellation" | "requires"
            )
        });
        if !intrinsic && context_attributes.count() != 0 {
            self.error(format!(
                "`{qualified}` cannot declare intrinsic context metadata; source-defined behavior is inferred from its body"
            ));
        }

        let requirements = function
            .attributes
            .iter()
            .find(|attribute| attribute.name == "requires")
            .map(|attribute| attribute.arguments.as_slice())
            .unwrap_or_default();
        let mut names = HashSet::new();
        for argument in requirements {
            let AttributeArgument::Name(requirement) = argument else {
                self.error(format!("`{qualified}` requirements must be unquoted names"));
                continue;
            };
            if !matches!(requirement.as_str(), "attachedProcess" | "stateSnapshots") {
                self.error(format!(
                    "`{qualified}` has unknown intrinsic requirement `{requirement}`"
                ));
            }
            if !names.insert(requirement.as_str()) {
                self.error(format!(
                    "`{qualified}` repeats intrinsic requirement `{requirement}`"
                ));
            }
        }

        let availability =
            self.optional_name_attribute(qualified, &function.attributes, "availability");
        if availability.is_some_and(|availability| availability != "onAttach") {
            self.error(format!(
                "`{qualified}` has unknown intrinsic availability `{}`",
                availability.unwrap()
            ));
        }
        let cancellation =
            self.optional_name_attribute(qualified, &function.attributes, "cancellation");
        if cancellation.is_some_and(|cancellation| cancellation != "processClose") {
            self.error(format!(
                "`{qualified}` has unknown cancellation policy `{}`",
                cancellation.unwrap()
            ));
        }
        if cancellation == Some("processClose") && !function.result_is_async {
            self.error(format!(
                "`{qualified}` cancels on process close but does not return `async T`"
            ));
        }
        if cancellation == Some("processClose") && !names.contains("attachedProcess") {
            self.error(format!(
                "`{qualified}` cancels on process close but does not require an attached process"
            ));
        }
    }

    fn validate_capability_overload(
        &mut self,
        owner: &str,
        name: &str,
        cases: &[&FunctionDeclaration],
        inherited: &[TypeParameter],
    ) {
        let qualified = if owner == "root" {
            name.to_owned()
        } else {
            format!("{owner}.{name}")
        };
        if cases.len() != 2 {
            self.error(format!(
                "`{qualified}` capability-directed implementation must have exactly an `Integer` and a `Float` case"
            ));
            return;
        }
        let first = cases[0];
        let same_shape = cases[1..].iter().all(|case| {
            case.is_static == first.is_static
                && case.parameters.len() == first.parameters.len()
                && case
                    .parameters
                    .iter()
                    .zip(&first.parameters)
                    .all(|(left, right)| {
                        left.name == right.name
                            && left.ty == right.ty
                            && left.attributes == right.attributes
                    })
                && case.result == first.result
                && case.result_is_async == first.result_is_async
                && case.type_parameters.len() == first.type_parameters.len()
                && case
                    .type_parameters
                    .iter()
                    .zip(&first.type_parameters)
                    .all(|(left, right)| left.name == right.name)
        });
        if !same_shape {
            self.error(format!(
                "`{qualified}` implementation cases must have identical value parameters, result type, and type-parameter names"
            ));
        }
        if first.type_parameters.len() != 1 || !inherited.is_empty() {
            self.error(format!(
                "`{qualified}` capability-directed implementation currently requires exactly one callable type parameter"
            ));
        }
        let mut dispatch = HashSet::new();
        for (index, case) in cases.iter().enumerate() {
            if case.body.is_none()
                || case
                    .attributes
                    .iter()
                    .any(|attribute| attribute.name == "intrinsic")
            {
                self.error(format!(
                    "`{qualified}` implementation cases must be source-defined"
                ));
            }
            let invalid_attribute = case
                .attributes
                .iter()
                .any(|attribute| attribute.name != "mustUse" || index != 0);
            if invalid_attribute {
                self.error(format!(
                    "`{qualified}` capability-directed implementations only allow `@mustUse` on the documented public case"
                ));
            }
            let mut parameters = case.type_parameters.clone();
            for constrained in &case.where_constraints {
                if let Some(parameter) = parameters
                    .iter_mut()
                    .find(|parameter| parameter.name == constrained.name)
                {
                    parameter
                        .constraints
                        .extend(constrained.constraints.clone());
                }
            }
            let constraints = parameters
                .first()
                .map(|parameter| parameter.constraints.as_slice())
                .unwrap_or_default();
            if constraints.len() != 1 || !matches!(constraints[0].as_str(), "Integer" | "Float") {
                self.error(format!(
                    "`{qualified}` implementation cases must dispatch directly on `Integer` and `Float`"
                ));
            } else {
                dispatch.insert(constraints[0].clone());
            }
        }
        if dispatch.len() != 2 {
            self.error(format!(
                "`{qualified}` capability-directed implementation must cover both `Integer` and `Float`"
            ));
        }
    }

    fn effective_function_type_parameters(
        &mut self,
        owner: &str,
        function: &FunctionDeclaration,
        inherited: &[TypeParameter],
    ) -> Vec<TypeParameter> {
        let mut parameters = inherited.to_vec();
        for parameter in &function.type_parameters {
            if parameters
                .iter()
                .any(|inherited| inherited.name == parameter.name)
            {
                self.error(format!(
                    "`{owner}` repeats inherited type parameter `{}`",
                    parameter.name
                ));
            } else {
                parameters.push(parameter.clone());
            }
        }
        let mut constrained_names = HashSet::new();
        for constrained in &function.where_constraints {
            if !constrained_names.insert(constrained.name.as_str()) {
                self.error(format!(
                    "`{owner}` repeats where clause for `{}`",
                    constrained.name
                ));
            }
            let Some(parameter) = parameters
                .iter_mut()
                .find(|parameter| parameter.name == constrained.name)
            else {
                self.error(format!(
                    "`{owner}` constrains unknown type parameter `{}`",
                    constrained.name
                ));
                continue;
            };
            for constraint in &constrained.constraints {
                if !self.capabilities.contains(constraint.as_str()) {
                    self.error(format!(
                        "`{owner}` references unknown capability `{constraint}`"
                    ));
                }
                if parameter.constraints.contains(constraint) {
                    self.error(format!(
                        "`{owner}` repeats capability constraint `{constraint}`"
                    ));
                } else {
                    parameter.constraints.push(constraint.clone());
                }
            }
        }
        parameters
    }

    fn validate_operator(&mut self, owner: &str, qualified: &str, function: &FunctionDeclaration) {
        let Some(operator) = function
            .attributes
            .iter()
            .find(|attribute| attribute.name == "operator")
        else {
            return;
        };
        let name = match operator.arguments.as_slice() {
            [AttributeArgument::Name(name)] => Some(name.as_str()),
            _ => None,
        };
        let arity = match name {
            Some(
                "add" | "subtract" | "multiply" | "divide" | "remainder" | "bitOr" | "bitXor"
                | "bitAnd" | "shiftLeft" | "shiftRight" | "equal" | "notEqual" | "lessThan"
                | "lessThanOrEqual" | "greaterThan" | "greaterThanOrEqual",
            ) => Some(1),
            Some("not" | "negate") => Some(0),
            _ => None,
        };
        if arity.is_none() {
            self.error(format!(
                "`{qualified}` attribute `@operator` expects a supported operator name"
            ));
        }
        let owner_supports_methods = self.types.contains(owner)
            || self.capabilities.contains(owner)
            || PrimitiveType::parse(owner).is_some();
        if !owner_supports_methods
            || function.is_static
            || arity.is_some_and(|arity| function.parameters.len() != arity)
        {
            self.error(format!(
                "`{qualified}` operator implementation must be a method with the operator's required arity"
            ));
        }
    }

    fn validate_provider(&mut self, value: &crate::StateProviderDeclaration) {
        self.validate_documentation(&value.name, &value.documentation, true, true);
        self.validate_attributes(
            &value.name,
            &value.attributes,
            &[
                "processType",
                "processes",
                "attachment",
                "validate",
                "prepare",
                "directRead",
                "readableRange",
                "default",
            ],
        );
        let mut readable_ranges = Vec::new();
        for attribute in value
            .attributes
            .iter()
            .filter(|attribute| attribute.name == "readableRange")
        {
            let [
                AttributeArgument::Integer(start),
                AttributeArgument::Integer(end),
            ] = attribute.arguments.as_slice()
            else {
                self.error(format!(
                    "`{}` attribute `@readableRange` expects integer start and exclusive end arguments",
                    value.name
                ));
                continue;
            };
            if start >= end {
                self.error(format!(
                    "`{}` attribute `@readableRange` must have a start below its exclusive end",
                    value.name
                ));
                continue;
            }
            if *end > u64::from(u32::MAX) + 1 {
                self.error(format!(
                    "`{}` attribute `@readableRange` must fit the provider's `u32` guest-address space",
                    value.name
                ));
                continue;
            }
            if readable_ranges
                .iter()
                .any(|(previous_start, previous_end)| start < previous_end && previous_start < end)
            {
                self.error(format!(
                    "`{}` has overlapping `@readableRange` declarations",
                    value.name
                ));
                continue;
            }
            readable_ranges.push((*start, *end));
        }
        if let Some(name) =
            self.optional_name_attribute(&value.name, &value.attributes, "processType")
        {
            if !self.types.contains(name) {
                self.error(format!(
                    "`{}` has invalid `@processType({name})`",
                    value.name
                ));
            }
        } else {
            self.error(format!("`{}` is missing `@processType(...)`", value.name));
        }
        let generated_items = self.generated_items.clone();
        self.require_name_attribute(&value.name, &value.attributes, "attachment", |attachment| {
            attachment == "identity" || generated_items.contains(attachment)
        });
        if let Some(validation) =
            self.optional_name_attribute(&value.name, &value.attributes, "validate")
            && !generated_items.contains(validation)
        {
            self.error(format!(
                "`{}` has invalid `@validate({validation})`",
                value.name
            ));
        }
        if let Some(preparation) =
            self.optional_name_attribute(&value.name, &value.attributes, "prepare")
            && !generated_items.contains(preparation)
        {
            self.error(format!(
                "`{}` has invalid `@prepare({preparation})`",
                value.name
            ));
        }
        let process_mode =
            self.optional_name_attribute(&value.name, &value.attributes, "processes");
        let source_processes = process_mode == Some("source");
        if let Some(mode) = process_mode
            && mode != "source"
        {
            self.error(format!(
                "state provider `{}` has unknown process source `{mode}`; expected `source`",
                value.name
            ));
        }
        if let Some(name) =
            self.optional_name_attribute(&value.name, &value.attributes, "directRead")
        {
            if !self.generated_items.contains(name) {
                self.error(format!(
                    "`{}` has invalid `@directRead({name})`",
                    value.name
                ));
            }
        } else {
            self.error(format!("`{}` is missing `@directRead(...)`", value.name));
        }
        if value.processes.is_empty() && !source_processes {
            self.error(format!(
                "state provider `{}` declares no processes",
                value.name
            ));
        }
        if source_processes && !value.processes.is_empty() {
            self.error(format!(
                "state provider `{}` uses `@processes(source)` and cannot also declare process names",
                value.name
            ));
        }
        if has_attribute(&value.attributes, "default") && !source_processes {
            self.error(format!(
                "state provider `{}` can be `@default` only when it uses `@processes(source)`",
                value.name
            ));
        }
        let mut context_names = HashSet::new();
        for context in &value.contexts {
            let qualified = format!("{}.{}", value.name, context.name);
            self.validate_documentation(&qualified, &context.documentation, true, true);
            self.validate_attributes(&qualified, &context.attributes, &["prepare"]);
            if !context_names.insert(context.name.as_str()) {
                self.error(format!(
                    "state provider `{}` declares context value `{}` more than once",
                    value.name, context.name
                ));
            }
            if context.name == value.value_name {
                self.error(format!(
                    "state provider `{}` uses `{}` for both its primary and context value",
                    value.name, context.name
                ));
            }
            match &context.ty {
                crate::Type::Name(name) if self.types.contains(name.as_str()) => {}
                crate::Type::Name(name) => self.error(format!(
                    "state-provider context `{qualified}` has unknown type `{name}`"
                )),
                _ => self.error(format!(
                    "state-provider context `{qualified}` must use a nominal standard-library type"
                )),
            }
            if let Some(preparation) =
                self.optional_name_attribute(&qualified, &context.attributes, "prepare")
            {
                if !self.generated_items.contains(preparation) {
                    self.error(format!(
                        "state-provider context `{qualified}` has invalid `@prepare({preparation})`"
                    ));
                }
            } else {
                self.error(format!(
                    "state-provider context `{qualified}` is missing `@prepare(...)`"
                ));
            }
        }
        let mut selector_names = HashSet::new();
        for selector in &value.selectors {
            let qualified = format!("{}.{}", value.name, selector.name);
            self.validate_attributes(
                &qualified,
                &selector.attributes,
                &["prepare", "managedBackend"],
            );
            if let Some(preparation) =
                self.optional_name_attribute(&qualified, &selector.attributes, "prepare")
            {
                if !self.generated_items.contains(preparation) {
                    self.error(format!(
                        "state-provider selector `{qualified}` has invalid `@prepare({preparation})`"
                    ));
                }
            } else {
                self.error(format!(
                    "state-provider selector `{qualified}` is missing `@prepare(...)`"
                ));
            }
            if let Some(backend) =
                self.optional_name_attribute(&qualified, &selector.attributes, "managedBackend")
                && !matches!(backend, "il2cpp" | "mono")
            {
                self.error(format!(
                    "state-provider selector `{qualified}` has invalid managed backend `{backend}`; expected `il2cpp` or `mono`"
                ));
            }
            if !selector_names.insert(selector.name.as_str()) {
                self.error(format!(
                    "state provider `{}` repeats selector `{}`",
                    value.name, selector.name
                ));
            }
            self.validate_documentation(&qualified, &selector.documentation, true, false);
            let mut parameter_names = HashSet::new();
            for parameter in &selector.parameters {
                if !parameter_names.insert(parameter.name.as_str()) {
                    self.error(format!(
                        "state-provider selector `{qualified}` repeats parameter `{}`",
                        parameter.name
                    ));
                }
                self.validate_type(&qualified, &parameter.ty, &[]);
            }
        }
    }

    fn validate_type_parameters(&mut self, owner: &str, parameters: &[TypeParameter]) {
        let mut names = HashSet::new();
        for parameter in parameters {
            if !names.insert(parameter.name.as_str()) {
                self.error(format!(
                    "`{owner}` repeats type parameter `{}`",
                    parameter.name
                ));
            }
            let mut constraints = HashSet::new();
            for constraint in &parameter.constraints {
                if !constraints.insert(constraint.as_str()) {
                    self.error(format!(
                        "`{owner}` repeats capability constraint `{constraint}`"
                    ));
                }
                if !self.capabilities.contains(constraint.as_str()) {
                    self.error(format!(
                        "`{owner}` references unknown capability `{constraint}`"
                    ));
                }
            }
        }
    }

    fn validate_type(&mut self, owner: &str, ty: &Type, parameters: &[TypeParameter]) {
        match ty {
            Type::Async(value) | Type::Iterator(value) => {
                self.validate_type(owner, value, parameters)
            }
            Type::Name(name)
                if PrimitiveType::parse(name).is_some()
                    || self.types.contains(name.as_str())
                    || parameters.iter().any(|parameter| parameter.name == *name) => {}
            Type::Name(name) => self.error(format!("`{owner}` references unknown type `{name}`")),
            Type::Array(element) => {
                self.require_constructor(owner, "Array", 1);
                self.validate_type(owner, element, parameters);
            }
            Type::FixedArray { element, .. } => {
                self.require_constructor(owner, "Array", 1);
                self.validate_type(owner, element, parameters);
            }
            Type::Option(value) => {
                self.require_constructor(owner, "Option", 1);
                self.validate_type(owner, value, parameters);
            }
            Type::Result(value) => {
                self.require_constructor(owner, "Result", 1);
                self.validate_type(owner, value, parameters);
            }
            Type::ExclusiveRange(value) => {
                self.require_constructor(owner, "ExclusiveRange", 1);
                self.validate_type(owner, value, parameters);
            }
            Type::InclusiveRange(value) => {
                self.require_constructor(owner, "InclusiveRange", 1);
                self.validate_type(owner, value, parameters);
            }
            Type::Application {
                constructor,
                arguments,
            } => {
                self.require_constructor(owner, constructor, arguments.len());
                for argument in arguments {
                    self.validate_type(owner, argument, parameters);
                }
            }
            Type::Callable {
                parameters: callable_parameters,
                result,
            } => {
                for parameter in callable_parameters {
                    self.validate_type(owner, parameter, parameters);
                }
                self.validate_type(owner, result, parameters);
            }
        }
    }

    fn require_constructor(&mut self, owner: &str, name: &str, arity: usize) {
        match self.constructors.get(name).copied() {
            Some(expected) if expected == arity => {}
            Some(expected) => self.error(format!(
                "`{owner}` applies `{name}` to {arity} types, but it expects {expected}"
            )),
            None => self.error(format!(
                "`{owner}` references unknown type constructor `{name}`"
            )),
        }
    }

    fn validate_representation(&mut self, owner: &str, attributes: &[Attribute], kind: &str) {
        let values = self.attribute_names(owner, attributes, "representation", true);
        let valid = match values.as_slice() {
            [name, storage] if name == "scalar" => PrimitiveType::parse(storage).is_some(),
            [name, element, rest @ ..] if name == "gcArray" => {
                PrimitiveType::parse(element).is_some()
                    && rest
                        .iter()
                        .all(|value| matches!(value.as_str(), "mutable" | "nullable"))
            }
            [name, rest @ ..] if name == "gcStruct" || name == "enum" => {
                rest.iter().all(|value| value == "nullable")
            }
            _ => false,
        };
        if !valid || (!kind.is_empty() && values.first().map(String::as_str) != Some(kind)) {
            self.error(format!("`{owner}` has an invalid runtime representation"));
        }
    }

    fn validate_value_usage(&mut self, owner: &str, attributes: &[Attribute]) {
        let values = self.attribute_names(owner, attributes, "valueUsage", true);
        if values.iter().any(|value| {
            !matches!(
                value.as_str(),
                "structField" | "enumPayload" | "stateField" | "localVariable" | "globalVariable"
            )
        }) {
            self.error(format!("`{owner}` has an invalid value-usage rule"));
        }
    }

    fn validate_capabilities(&mut self, owner: &str, attributes: &[Attribute]) {
        for capability in self.attribute_names(owner, attributes, "capabilities", false) {
            if !self.capabilities.contains(capability.as_str()) {
                self.error(format!(
                    "`{owner}` references unknown capability `{capability}`"
                ));
            }
        }
    }

    fn validate_documentation(
        &mut self,
        owner: &str,
        docs: &Documentation,
        require_details: bool,
        require_example: bool,
    ) {
        if docs.summary.trim().is_empty() || (require_details && docs.details.trim().is_empty()) {
            self.error(format!("`{owner}` has incomplete documentation"));
        }
        if require_example && docs.examples.is_empty() {
            self.error(format!(
                "`{owner}` must have at least one focused documentation example"
            ));
        }
        for value in &docs.examples {
            if value.title.trim().is_empty() || value.source.trim().is_empty() {
                self.error(format!("`{owner}` has an incomplete documentation example"));
            }
            if let Some(provider) = &value.state_provider
                && !self.library.declarations.iter().any(|declaration| {
                    matches!(
                        declaration,
                        Declaration::StateProvider(candidate) if candidate.name == *provider
                    )
                })
            {
                self.error(format!(
                    "`{owner}` documentation example references unknown state provider `{provider}`"
                ));
            }
            if let Some(previous) = self
                .example_owners
                .insert(value.source.clone(), owner.to_owned())
            {
                self.error(format!(
                    "`{owner}` reuses the visible documentation example from `{previous}`"
                ));
            }
        }
    }

    fn validate_must_use(&mut self, owner: &str, attributes: &[Attribute]) {
        let Some(attribute) = attributes
            .iter()
            .find(|attribute| attribute.name == "mustUse")
        else {
            return;
        };
        match attribute.arguments.as_slice() {
            [AttributeArgument::String(reason)] if !reason.trim().is_empty() => {}
            _ => self.error(format!(
                "`{owner}` attribute `@mustUse` expects one non-empty string argument"
            )),
        }
    }

    fn validate_attributes(&mut self, owner: &str, attributes: &[Attribute], known: &[&str]) {
        let mut names = HashSet::new();
        for attribute in attributes {
            if !names.insert(attribute.name.as_str()) {
                self.error(format!("`{owner}` repeats attribute `@{}`", attribute.name));
            }
            if !known.contains(&attribute.name.as_str()) {
                self.error(format!(
                    "`{owner}` uses unknown privileged attribute `@{}`",
                    attribute.name
                ));
            }
        }
    }

    fn require_name_attribute(
        &mut self,
        owner: &str,
        attributes: &[Attribute],
        name: &str,
        valid: impl FnOnce(&str) -> bool,
    ) {
        let Some(value) = self.optional_name_attribute(owner, attributes, name) else {
            self.error(format!("`{owner}` is missing `@{name}(...)`"));
            return;
        };
        if !valid(value) {
            self.error(format!("`{owner}` has invalid `@{name}({value})`"));
        }
    }

    fn optional_name_attribute<'b>(
        &mut self,
        owner: &str,
        attributes: &'b [Attribute],
        name: &str,
    ) -> Option<&'b str> {
        let attribute = attributes.iter().find(|attribute| attribute.name == name)?;
        match attribute.arguments.as_slice() {
            [AttributeArgument::Name(value)] => Some(value),
            _ => {
                self.error(format!(
                    "`{owner}` attribute `@{name}` expects one name argument"
                ));
                None
            }
        }
    }

    fn attribute_names(
        &mut self,
        owner: &str,
        attributes: &[Attribute],
        name: &str,
        required: bool,
    ) -> Vec<String> {
        let Some(attribute) = attributes.iter().find(|attribute| attribute.name == name) else {
            if required {
                self.error(format!("`{owner}` is missing `@{name}(...)`"));
            }
            return Vec::new();
        };
        let mut values = Vec::new();
        for argument in &attribute.arguments {
            match argument {
                AttributeArgument::Name(value) => values.push(value.clone()),
                AttributeArgument::String(_) | AttributeArgument::Integer(_) => self.error(
                    format!("`{owner}` attribute `@{name}` expects name arguments"),
                ),
            }
        }
        values
    }

    fn error(&mut self, message: String) {
        self.errors.push(Error {
            message,
            start: 0,
            end: 0,
        });
    }
}

fn declaration_name(declaration: &Declaration) -> &str {
    match declaration {
        Declaration::Struct(value) | Declaration::IntrinsicType(value) => &value.name,
        Declaration::Enum(value) => &value.name,
        Declaration::Root(value)
        | Declaration::Namespace(value)
        | Declaration::Capability(value)
        | Declaration::TypeConstructor(value)
        | Declaration::CoreExtension(value) => &value.name,
        Declaration::StateProvider(value) => &value.name,
    }
}

fn has_attribute(attributes: &[Attribute], name: &str) -> bool {
    attributes.iter().any(|attribute| attribute.name == name)
}

fn capability_hierarchy_has_cycle<'a>(
    capability: &'a str,
    hierarchy: &HashMap<&'a str, &'a [String]>,
    active: &mut HashSet<&'a str>,
    completed: &mut HashSet<&'a str>,
) -> bool {
    if completed.contains(capability) {
        return false;
    }
    if !active.insert(capability) {
        return true;
    }
    let cyclic = hierarchy
        .get(capability)
        .into_iter()
        .flat_map(|supers| supers.iter())
        .filter(|super_capability| hierarchy.contains_key(super_capability.as_str()))
        .any(|super_capability| {
            capability_hierarchy_has_cycle(super_capability, hierarchy, active, completed)
        });
    active.remove(capability);
    completed.insert(capability);
    cyclic
}

fn declaration_items(declaration: &Declaration) -> Vec<String> {
    let (prefix, functions) = match declaration {
        Declaration::Root(value) => (String::new(), value.functions.as_slice()),
        Declaration::Namespace(value)
        | Declaration::Capability(value)
        | Declaration::TypeConstructor(value)
        | Declaration::CoreExtension(value) => (id_path(&value.name), value.functions.as_slice()),
        Declaration::Struct(value) | Declaration::IntrinsicType(value) => {
            (id(&value.name), value.functions.as_slice())
        }
        Declaration::Enum(_) | Declaration::StateProvider(_) => return Vec::new(),
    };
    functions
        .iter()
        .map(|function| format!("{prefix}{}", id(&function.name)))
        .collect()
}

fn id_path(path: &str) -> String {
    path.split('.').map(id).collect()
}

fn id(name: &str) -> String {
    if name
        .chars()
        .all(|character| !character.is_ascii_lowercase())
    {
        let mut characters = name.chars();
        let Some(first) = characters.next() else {
            return String::new();
        };
        let mut result = first.to_ascii_uppercase().to_string();
        result.extend(characters.map(|character| character.to_ascii_lowercase()));
        result
    } else {
        let mut characters = name.chars();
        let Some(first) = characters.next() else {
            return String::new();
        };
        let mut result = first.to_ascii_uppercase().to_string();
        let mut after_digit = false;
        for character in characters {
            if after_digit && character.is_ascii_alphabetic() {
                result.push(character.to_ascii_uppercase());
                after_digit = false;
            } else {
                result.push(character);
                after_digit = character.is_ascii_digit();
            }
        }
        result
    }
}

#[cfg(test)]
mod tests {
    use super::validate;
    use crate::{generate_catalog, parse};

    #[test]
    fn intrinsic_context_metadata_is_reserved_for_intrinsic_bindings() {
        let source = r#"
root {
    @requires(attachedProcess)
    private fn helper() -> None {}
}
"#;
        let errors = generate_catalog(&parse(source).unwrap()).unwrap_err();
        assert!(errors.iter().any(|error| {
            error
                .message
                .contains("source-defined behavior is inferred from its body")
        }));
    }

    #[test]
    fn intrinsic_context_metadata_rejects_incoherent_facts() {
        let source = r#"
root {
    @requires(attachedProcess, attachedProcess, unknown)
    @cancellation(processClose)
    @intrinsic(NextTick)
    private fn wait() -> None;
}
"#;
        let errors = generate_catalog(&parse(source).unwrap()).unwrap_err();
        for expected in [
            "unknown intrinsic requirement `unknown`",
            "repeats intrinsic requirement `attachedProcess`",
            "cancels on process close but does not return `async T`",
        ] {
            assert!(
                errors.iter().any(|error| error.message.contains(expected)),
                "missing `{expected}` in {errors:#?}"
            );
        }
    }

    #[test]
    fn invalid_type_references_are_reported_before_generation() {
        let source = r#"
/// Arrays.
typeConstructor [T] {}
/// A value.
@representation(gcStruct)
@valueUsage(localVariable)
struct Value {
    /// Missing.
    field: Missing,
}
"#;
        let errors = generate_catalog(&parse(source).unwrap()).unwrap_err();
        assert!(
            errors
                .iter()
                .any(|error| error.message.contains("unknown type `Missing`")),
            "{errors:#?}"
        );
    }

    #[test]
    fn missing_callable_examples_are_structured_errors_not_panics() {
        let source = r#"
root {
    /// Prints.
    @intrinsic(Print)
    fn print() -> None;
}
"#;
        let errors = generate_catalog(&parse(source).unwrap()).unwrap_err();
        assert!(
            errors
                .iter()
                .any(|error| error.message.contains("at least one focused")),
            "{errors:#?}"
        );
    }

    #[test]
    fn public_declarations_require_focused_examples() {
        let source = r#"
/// A documented value.
///
/// Represents a public standard-library value.
@representation(gcStruct)
@valueUsage(localVariable)
struct Value {
    /// A public field.
    ///
    /// Stores the value.
    field: u32,
}
"#;
        let errors = generate_catalog(&parse(source).unwrap()).unwrap_err();
        assert!(errors.iter().any(|error| {
            error
                .message
                .contains("`Value` must have at least one focused")
        }));
        assert!(errors.iter().any(|error| {
            error
                .message
                .contains("`Value.field` must have at least one focused")
        }));
    }

    #[test]
    fn declarations_may_have_multiple_focused_examples() {
        let source = r#"
/// Values.
///
/// Provides documented values.
///
/// # Example
///
/// Read one value
///
/// ```splitscript
/// let first = 1
/// ```
///
/// # Example
///
/// Read another value
///
/// ```splitscript
/// let second = 2
/// ```
namespace values {}
"#;
        generate_catalog(&parse(source).unwrap()).unwrap();
    }

    #[test]
    fn example_context_must_name_a_declared_state_provider() {
        let source = r#"
/// Values.
///
/// Provides documented values.
///
/// # Example
///
/// Read a value
///
/// ```splitscript state Missing
/// let value = 1
/// ```
namespace values {}
"#;
        let errors = generate_catalog(&parse(source).unwrap()).unwrap_err();
        assert!(errors.iter().any(|error| {
            error
                .message
                .contains("references unknown state provider `Missing`")
        }));
    }

    #[test]
    fn visible_examples_must_be_distinct() {
        let source = r#"
/// First namespace.
///
/// Provides the first API.
///
/// # Example
///
/// Use it
///
/// ```splitscript
/// let value = 1
/// ```
namespace first {}

/// Second namespace.
///
/// Provides the second API.
///
/// # Example
///
/// Use it too
///
/// ```splitscript
/// let value = 1
/// ```
namespace second {}
"#;
        let errors = generate_catalog(&parse(source).unwrap()).unwrap_err();
        assert!(errors.iter().any(|error| {
            error
                .message
                .contains("reuses the visible documentation example")
        }));
    }

    #[test]
    fn must_use_requires_a_reason_and_a_returned_value() {
        let malformed = r#"
/// Optional values.
@mustUse("")
typeConstructor T? {}
"#;
        let errors = generate_catalog(&parse(malformed).unwrap()).unwrap_err();
        assert!(errors.iter().any(|error| {
            error
                .message
                .contains("expects one non-empty string argument")
        }));

        let void_callable = r#"
root {
    /// Prints a message.
    ///
    /// Writes diagnostic output.
    ///
    /// # Example
    ///
    /// Print
    ///
    /// ```splitscript
    /// print()
    /// ```
    @mustUse("Observe this value.")
    @intrinsic(Print)
    fn print() -> None;
}
"#;
        let errors = generate_catalog(&parse(void_callable).unwrap()).unwrap_err();
        assert!(errors.iter().any(|error| {
            error
                .message
                .contains("cannot be `@mustUse` because it returns `None`")
        }));
    }

    #[test]
    fn callable_implementations_are_exclusive_and_required() {
        let both = r#"
root {
    /// Prints.
    ///
    /// Prints a message.
    ///
    /// # Example
    ///
    /// Print
    ///
    /// ```splitscript
    /// print()
    /// ```
    @intrinsic(Print)
    fn print() -> None {}
}
"#;
        let errors = generate_catalog(&parse(both).unwrap()).unwrap_err();
        assert!(errors.iter().any(|error| error.message.contains("both")));

        let neither = both
            .replace("    @intrinsic(Print)\n", "")
            .replace(" {}", ";");
        let errors = generate_catalog(&parse(&neither).unwrap()).unwrap_err();
        assert!(errors.iter().any(|error| error.message.contains("either")));
    }

    #[test]
    fn structural_method_capabilities_generate_declarative_requirements() {
        let source = r#"
/// Displayable values.
///
/// # Example
///
/// Display a value
///
/// ```splitscript
/// print(5)
/// ```
@behavior(structuralMethods)
capability Display {
    /// Converts this value to text.
    ///
    /// User types provide the matching method.
    ///
    /// # Example
    ///
    /// Convert a value
    ///
    /// ```splitscript
    /// value.display()
    /// ```
    fn display() -> None;
}
"#;
        let generated = generate_catalog(&parse(source).unwrap()).unwrap();
        assert!(generated.contains("CapabilityBehavior::StructuralMethods"));
        assert!(generated.contains("Implementation::CapabilityRequirement"));
    }

    #[test]
    fn generic_body_accepts_constructed_parameter_shapes() {
        let source = r#"
/// Arrays.
///
/// # Example
///
/// Store values
///
/// ```splitscript
/// let values: [u32] = []
/// ```
typeConstructor [T] {}

/// Values.
///
/// # Example
///
/// Use values
///
/// ```splitscript
/// let copiedValues: [u32] = []
/// ```
@behavior(declared)
capability Values {
    /// Copies values.
    ///
    /// Returns the input values.
    ///
    /// # Example
    ///
    /// Copy values
    ///
    /// ```splitscript
    /// let copied = values.copy()
    /// ```
    fn copy(
        /// The values to copy.
        values: [Self],
    ) -> [Self] {
        return values
    }
}
"#;
        let generated = generate_catalog(&parse(source).unwrap()).unwrap();
        assert!(generated.contains("Implementation::LibraryBody"));
        assert!(generated.contains("TypeRef::Application"));
    }

    #[test]
    fn callable_where_clauses_must_reference_known_parameters_and_capabilities() {
        let source = r#"
/// Arrays.
typeConstructor [T] {
    /// Invalid parameter constraint.
    fn badParameter() -> bool where Missing: Equatable {
        return false
    }

    /// Invalid capability constraint.
    fn badCapability() -> bool where T: MissingCapability {
        return false
    }
}
"#;
        let errors = generate_catalog(&parse(source).unwrap()).unwrap_err();
        assert!(
            errors.iter().any(|error| error
                .message
                .contains("constrains unknown type parameter `Missing`")),
            "{errors:#?}"
        );
        assert!(
            errors.iter().any(|error| error
                .message
                .contains("references unknown capability `MissingCapability`")),
            "{errors:#?}"
        );
    }

    #[test]
    fn capability_hierarchy_rejects_cycles() {
        let source = r#"
/// First capability.
@behavior(declared)
capability First: Second {}
/// Second capability.
@behavior(declared)
capability Second: First {}
"#;
        let errors = generate_catalog(&parse(source).unwrap()).unwrap_err();
        assert!(
            errors
                .iter()
                .any(|error| error.message.contains("hierarchy contains a cycle")),
            "{errors:#?}"
        );
    }

    #[test]
    fn capability_hierarchy_rejects_ambiguous_inherited_associated_types() {
        let source = r#"
capability First {
    type Item;
}
capability Second {
    type Item;
}
capability Combined: First + Second {}
"#;
        let library = parse(source).expect("capability hierarchy should parse");
        let errors = validate(&library);
        assert!(
            errors.iter().any(|error| error.message.contains(
                "capability `Combined` inherits ambiguous associated type `Item` from `First` and `Second`"
            )),
            "{errors:#?}"
        );
    }

    #[test]
    fn private_types_cannot_escape_through_public_signatures() {
        let source = r#"
/// Internal layout.
@representation(gcStruct)
@valueUsage(localVariable)
private struct InternalLayout {
    /// Internal offset.
    offset: u64,
}

/// Public wrapper.
@representation(gcStruct)
@valueUsage(localVariable)
struct PublicWrapper {
    /// Leaked implementation detail.
    layout: InternalLayout,
}
"#;
        let errors = generate_catalog(&parse(source).unwrap()).unwrap_err();
        assert!(
            errors.iter().any(|error| error.message.contains(
                "public standard-library surface `PublicWrapper.layout` exposes private type `InternalLayout`"
            )),
            "{errors:#?}"
        );
    }
}
