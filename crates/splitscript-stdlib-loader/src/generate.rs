use std::collections::HashSet;

use splitscript_syntax::{PrimitiveType, standard_library::TypeConstructorSyntax};

use crate::{
    Attribute, AttributeArgument, CallableOwnerDeclaration, Declaration, Error,
    FunctionDeclaration, Library, StructDeclaration, Type, TypeParameter,
    visible_capability_associated_types,
};

pub fn generate_catalog(library: &Library) -> Result<String, Vec<Error>> {
    let generator = CatalogGenerator::new(library)?;
    Ok(generator.generate())
}

struct CatalogGenerator<'a> {
    library: &'a Library,
    type_names: HashSet<&'a str>,
    capability_names: HashSet<&'a str>,
}

#[derive(Clone, Copy)]
struct FieldEmissionOptions {
    owner_private: bool,
    test_only: bool,
}

impl<'a> CatalogGenerator<'a> {
    fn new(library: &'a Library) -> Result<Self, Vec<Error>> {
        let errors = crate::validation::validate(library);
        if !errors.is_empty() {
            return Err(errors);
        }
        let type_names = library
            .declarations
            .iter()
            .filter_map(|declaration| match declaration {
                Declaration::Struct(declaration) | Declaration::IntrinsicType(declaration) => {
                    Some(declaration.name.as_str())
                }
                Declaration::Enum(declaration) => Some(declaration.name.as_str()),
                _ => None,
            })
            .collect();
        let capability_names = library
            .declarations
            .iter()
            .filter_map(|declaration| match declaration {
                Declaration::Capability(declaration) => Some(declaration.name.as_str()),
                _ => None,
            })
            .collect();
        Ok(Self {
            library,
            type_names,
            capability_names,
        })
    }

    fn generate(&self) -> String {
        let mut output = String::new();
        output.push_str("pub(super) const STATE_PROVIDERS: &[StdlibStateProvider] = &[\n");
        for declaration in &self.library.declarations {
            if let Declaration::StateProvider(provider) = declaration {
                let process_type = attribute_name(&provider.attributes, "processType");
                let attachment = attribute_name(&provider.attributes, "attachment");
                let direct_read = attribute_name(&provider.attributes, "directRead");
                let validation = optional_attribute_name(&provider.attributes, "validate")
                    .map_or_else(
                        || "None".to_owned(),
                        |item| format!("Some(StdlibItemId::{item})"),
                    );
                let preparation = optional_attribute_name(&provider.attributes, "prepare")
                    .map_or_else(
                        || "None".to_owned(),
                        |item| format!("Some(StdlibItemId::{item})"),
                    );
                let source_processes =
                    optional_attribute_name(&provider.attributes, "processes") == Some("source");
                let default = provider
                    .attributes
                    .iter()
                    .any(|attribute| attribute.name == "default");
                let readable_ranges = provider
                    .attributes
                    .iter()
                    .filter(|attribute| attribute.name == "readableRange")
                    .map(|attribute| {
                        let [
                            AttributeArgument::Integer(start),
                            AttributeArgument::Integer(end),
                        ] = attribute.arguments.as_slice()
                        else {
                            unreachable!("readable ranges are validated before generation")
                        };
                        format!("StateProviderMemoryRange {{ start: {start}, end: {end} }}")
                    })
                    .collect::<Vec<_>>()
                    .join(",");
                let processes = if source_processes {
                    "StateProviderProcesses::SourceState".to_owned()
                } else {
                    format!(
                        "StateProviderProcesses::Declared(&[{}])",
                        provider
                            .processes
                            .iter()
                            .map(|process| quote(process))
                            .collect::<Vec<_>>()
                            .join(",")
                    )
                };
                let attachment = if attachment == "identity" {
                    "StateProviderAttachment::Identity".to_owned()
                } else {
                    format!("StateProviderAttachment::Callable(StdlibItemId::{attachment})")
                };
                let selectors = provider
                    .selectors
                    .iter()
                    .map(|selector| {
                        let preparation = attribute_name(&selector.attributes, "prepare");
                        let managed_backend = optional_attribute_name(
                            &selector.attributes,
                            "managedBackend",
                        )
                        .map_or_else(
                            || "None".to_owned(),
                            |backend| {
                                let variant = match backend {
                                    "il2cpp" => "Il2Cpp",
                                    "mono" => "Mono",
                                    _ => unreachable!("managed backends are validated"),
                                };
                                format!("Some(ManagedRuntimeBackend::{variant})")
                            },
                        );
                        let parameters = selector
                            .parameters
                            .iter()
                            .map(|parameter| {
                                format!(
                                    "StateProviderSelectorParameter {{ name: {}, ty: {} }}",
                                    quote(&parameter.name),
                                    self.type_ref(&parameter.ty, &[], &[]),
                                )
                            })
                            .collect::<Vec<_>>()
                            .join(",");
                        format!(
                            "StateProviderSelector {{ name: {}, parameters: &[{parameters}], preparation: StdlibItemId::{preparation}, managed_backend: {managed_backend}, documentation: {} }}",
                            quote(&selector.name),
                            self.documentation(&selector.documentation),
                        )
                    })
                    .collect::<Vec<_>>()
                    .join(",");
                let contexts = provider
                    .contexts
                    .iter()
                    .map(|context| {
                        let ty = match &context.ty {
                            crate::Type::Name(name) => ident(name),
                            _ => unreachable!("provider context types are validated as nominal types"),
                        };
                        let preparation = attribute_name(&context.attributes, "prepare");
                        format!(
                            "StateProviderContext {{ name: {}, ty: StdlibTypeId::{ty}, preparation: StdlibItemId::{preparation}, documentation: {} }}",
                            quote(&context.name),
                            self.documentation(&context.documentation),
                        )
                    })
                    .collect::<Vec<_>>()
                    .join(",");
                output.push_str(&format!(
                    "StdlibStateProvider {{ id: StdlibStateProviderId::{}, name: {}, value_name: {}, processes: {}, default: {default}, process_type: StdlibTypeId::{}, attachment: {}, validation: {validation}, preparation: {preparation}, direct_read: StdlibItemId::{}, readable_ranges: &[{readable_ranges}], contexts: &[{contexts}], selectors: &[{selectors}], documentation: {} }},\n",
                    ident(&provider.name),
                    quote(&provider.name),
                    quote(&provider.value_name),
                    processes,
                    process_type,
                    attachment,
                    direct_read,
                    self.documentation(&provider.documentation),
                ));
            }
        }
        output.push_str("];\n\npub(super) const CAPABILITIES: &[StdlibCapability] = &[\n");
        for declaration in &self.library.declarations {
            if let Declaration::Capability(owner) = declaration {
                let behavior = match attribute_name(&owner.attributes, "behavior") {
                    "declared" => "Declared",
                    "structuralEquality" => "StructuralEquality",
                    "structuralMemoryLayout" => "StructuralMemoryLayout",
                    "structuralMethods" => "StructuralMethods",
                    other => other,
                };
                let super_capabilities = owner
                    .type_parameters
                    .first()
                    .map(|parameter| parameter.constraints.as_slice())
                    .unwrap_or_default()
                    .iter()
                    .map(|capability| format!("StdlibCapabilityId::{}", ident(capability)))
                    .collect::<Vec<_>>()
                    .join(",");
                output.push_str(&format!(
                    "StdlibCapability {{ id: StdlibCapabilityId::{}, name: {}, super_capabilities: &[{super_capabilities}], behavior: CapabilityBehavior::{behavior}, associated_types: {}, documentation: {} }},\n",
                    ident(&owner.name), quote(&owner.name),
                    self.associated_type_requirements(owner),
                    self.documentation(&owner.documentation)
                ));
            }
        }
        output
            .push_str("];\n\npub(super) const TYPE_CONSTRUCTORS: &[StdlibTypeConstructor] = &[\n");
        for declaration in &self.library.declarations {
            if let Declaration::TypeConstructor(owner) = declaration {
                let id = ident(&owner.name);
                let syntax = match owner
                    .type_constructor_syntax
                    .expect("type constructors always have a source form")
                {
                    TypeConstructorSyntax::Named => "Named",
                    TypeConstructorSyntax::Array => "Array",
                    TypeConstructorSyntax::Optional => "Optional",
                    TypeConstructorSyntax::Fallible => "Fallible",
                    TypeConstructorSyntax::ExclusiveRange => "ExclusiveRange",
                    TypeConstructorSyntax::InclusiveRange => "InclusiveRange",
                };
                let must_use = optional_attribute_name(&owner.attributes, "mustUse")
                    .map(|reason| format!("Some({})", quote(reason)))
                    .unwrap_or_else(|| "None".to_owned());
                output.push_str(&format!(
                    "StdlibTypeConstructor {{ id: StdlibTypeConstructorId::{id}, public: {}, syntax: TypeConstructorSyntax::{syntax}, name: {}, parameters: {}, capabilities: {}, must_use: {must_use}, associated_types: {}, documentation: {} }},\n",
                    !owner.private,
                    quote(&owner.name),
                    self.type_parameters(&owner.type_parameters, owner),
                    self.capabilities(&owner.attributes),
                    self.associated_type_definitions(owner),
                    self.documentation(&owner.documentation)
                ));
            }
        }
        output.push_str("];\n\npub(super) const NAMESPACES: &[StdlibNamespace] = &[\n");
        for declaration in &self.library.declarations {
            if let Declaration::Namespace(owner) = declaration {
                let id = path_ident(&owner.name);
                output.push_str(&format!(
                    "StdlibNamespace {{ id: StdlibNamespaceId::{id}, name: {}, path: &[{}], documentation: {} }},\n",
                    quote(owner.name.rsplit('.').next().unwrap()),
                    owner.name.split('.').map(quote).collect::<Vec<_>>().join(","),
                    self.documentation(&owner.documentation)
                ));
            }
        }
        output.push_str("];\n\npub(super) const TYPES: &[StdlibType] = &[\n");
        for declaration in &self.library.declarations {
            match declaration {
                Declaration::Struct(declaration) => {
                    self.emit_type(&mut output, declaration, "Struct")
                }
                Declaration::IntrinsicType(declaration) => {
                    self.emit_type(&mut output, declaration, "Intrinsic")
                }
                Declaration::Enum(declaration) => self.emit_enum_type(&mut output, declaration),
                _ => {}
            }
        }
        output.push_str("];\n\npub(super) const FIELDS: &[StdlibField] = &[\n");
        for declaration in &self.library.declarations {
            match declaration {
                Declaration::Struct(declaration) | Declaration::IntrinsicType(declaration) => {
                    self.emit_fields(
                        &mut output,
                        &declaration.name,
                        &format!(
                            "StdlibOwner::Type(StdlibTypeId::{})",
                            ident(&declaration.name)
                        ),
                        &declaration.fields,
                        &[],
                        FieldEmissionOptions {
                            owner_private: declaration.private,
                            test_only: has_attribute(&declaration.attributes, "testOnly"),
                        },
                    );
                }
                Declaration::TypeConstructor(declaration) => self.emit_fields(
                    &mut output,
                    &declaration.name,
                    &format!(
                        "StdlibOwner::TypeConstructor(StdlibTypeConstructorId::{})",
                        ident(&declaration.name)
                    ),
                    &declaration.fields,
                    &declaration.type_parameters,
                    FieldEmissionOptions {
                        owner_private: declaration.private,
                        test_only: has_attribute(&declaration.attributes, "testOnly"),
                    },
                ),
                _ => {}
            }
        }
        output.push_str("];\n\npub(super) const VARIANTS: &[StdlibVariant] = &[\n");
        for declaration in &self.library.declarations {
            if let Declaration::Enum(declaration) = declaration {
                let owner = ident(&declaration.name);
                for variant in &declaration.variants {
                    output.push_str(&format!(
                        "StdlibVariant {{ id: StdlibVariantId::{}{}, owner: StdlibTypeId::{owner}, name: {}, documentation: {} }},\n",
                        owner, ident(&variant.name), quote(&variant.name),
                        self.documentation(&variant.documentation)
                    ));
                }
            }
        }
        output.push_str("];\n\npub(super) const ITEMS: &[StdlibItem] = &[\n");
        self.emit_all_items(&mut output);
        output.push_str("];\n");
        output
    }

    fn documentation(&self, documentation: &crate::Documentation) -> String {
        let examples = documentation
            .examples
            .iter()
            .map(example_expression)
            .collect::<Vec<_>>()
            .join(",");
        format!(
            "Documentation {{ summary: {}, details: {}, examples: &[{examples}], related: &[] }}",
            quote(&documentation.summary),
            quote(&documentation.details)
        )
    }

    fn emit_type(&self, output: &mut String, declaration: &StructDeclaration, kind: &str) {
        let id = ident(&declaration.name);
        let display = declaration
            .functions
            .iter()
            .find(|function| has_attribute(&function.attributes, "display"))
            .map_or_else(
                || "None".to_owned(),
                |function| {
                    format!(
                        "Some(StdlibItemId::{}{})",
                        path_ident(&declaration.name),
                        ident(&function.name)
                    )
                },
            );
        if has_attribute(&declaration.attributes, "testOnly") {
            output.push_str("#[cfg(test)] ");
        }
        output.push_str(&format!(
            "StdlibType {{ id: StdlibTypeId::{id}, name: {}, visibility: TypeVisibility::{}, kind: StdlibTypeKind::{kind}, capabilities: {}, associated_types: {}, display: {display}, representation: {}, value_usage: {}, public_construction: {}, documentation: {} }},\n",
            quote(&declaration.name), if declaration.private { "LibraryPrivate" } else { "Public" }, self.capabilities(&declaration.attributes),
            self.associated_type_definitions_for_type(declaration),
            self.representation(&declaration.attributes), self.value_usage(&declaration.attributes),
            has_attribute(&declaration.attributes, "publicConstruction"),
            self.documentation(&declaration.documentation)
        ));
    }

    fn emit_enum_type(&self, output: &mut String, declaration: &crate::EnumDeclaration) {
        let id = ident(&declaration.name);
        output.push_str(&format!(
            "StdlibType {{ id: StdlibTypeId::{id}, name: {}, visibility: TypeVisibility::{}, kind: StdlibTypeKind::Enum, capabilities: {}, associated_types: &[], display: None, representation: {}, value_usage: {}, public_construction: false, documentation: {} }},\n",
            quote(&declaration.name), if declaration.private { "LibraryPrivate" } else { "Public" }, self.capabilities(&declaration.attributes),
            self.representation(&declaration.attributes), self.value_usage(&declaration.attributes),
            self.documentation(&declaration.documentation)
        ));
    }

    fn emit_fields(
        &self,
        output: &mut String,
        owner: &str,
        owner_expression: &str,
        fields: &[crate::FieldDeclaration],
        type_parameters: &[TypeParameter],
        options: FieldEmissionOptions,
    ) {
        let owner_id = ident(owner);
        for field in fields {
            if options.test_only {
                output.push_str("#[cfg(test)] ");
            }
            output.push_str(&format!(
                "StdlibField {{ id: StdlibFieldId::{}{}, owner: {owner_expression}, name: {}, ty: {}, visibility: FieldVisibility::{}, documentation: {} }},\n",
                owner_id, ident(&field.name),
                quote(&field.name),
                self.type_ref(&field.ty, type_parameters, &[]),
                if options.owner_private || field.private {
                    "RuntimePrivate"
                } else {
                    "Public"
                },
                self.documentation(&field.documentation)
            ));
        }
    }

    fn emit_all_items(&self, output: &mut String) {
        for declaration in &self.library.declarations {
            match declaration {
                Declaration::Root(owner) => self.emit_functions(
                    output,
                    owner,
                    "",
                    "StdlibOwner::Root",
                    "TypeRef::Core(CoreTypeId::None)",
                    None,
                ),
                Declaration::Namespace(owner) => self.emit_functions(
                    output,
                    owner,
                    &owner.name,
                    &format!(
                        "StdlibOwner::Namespace(StdlibNamespaceId::{})",
                        path_ident(&owner.name)
                    ),
                    "TypeRef::Core(CoreTypeId::None)",
                    None,
                ),
                Declaration::Capability(owner) => self.emit_functions(
                    output,
                    owner,
                    &owner.name,
                    &format!(
                        "StdlibOwner::Capability(StdlibCapabilityId::{})",
                        ident(&owner.name)
                    ),
                    &format!(
                        "TypeRef::Parameter({})",
                        quote(
                            &owner
                                .type_parameters
                                .first()
                                .expect("validated capabilities have one type parameter")
                                .name
                        )
                    ),
                    Some(&owner.type_parameters),
                ),
                Declaration::TypeConstructor(owner) => self.emit_functions(
                    output,
                    owner,
                    &owner.name,
                    &format!(
                        "StdlibOwner::TypeConstructor(StdlibTypeConstructorId::{})",
                        ident(&owner.name)
                    ),
                    &self.application_type(&owner.name, &owner.type_parameters),
                    Some(&owner.type_parameters),
                ),
                Declaration::CoreExtension(owner) => self.emit_functions(
                    output,
                    owner,
                    &owner.name,
                    &format!("StdlibOwner::Core(CoreTypeId::{})", ident(&owner.name)),
                    &format!("TypeRef::Core(CoreTypeId::{})", ident(&owner.name)),
                    None,
                ),
                Declaration::Struct(declaration) | Declaration::IntrinsicType(declaration) => {
                    let mut functions = declaration.functions.clone();
                    if declaration.private {
                        for function in &mut functions {
                            function.private = true;
                        }
                    }
                    let owner = CallableOwnerDeclaration {
                        name: declaration.name.clone(),
                        private: declaration.private,
                        type_constructor_syntax: None,
                        type_parameters: Vec::new(),
                        documentation: declaration.documentation.clone(),
                        attributes: declaration.attributes.clone(),
                        fields: Vec::new(),
                        associated_types: declaration.associated_types.clone(),
                        functions,
                    };
                    self.emit_functions(
                        output,
                        &owner,
                        &declaration.name,
                        &format!(
                            "StdlibOwner::Type(StdlibTypeId::{})",
                            ident(&declaration.name)
                        ),
                        &format!(
                            "TypeRef::Standard(StdlibTypeId::{})",
                            ident(&declaration.name)
                        ),
                        None,
                    );
                }
                Declaration::Enum(_) | Declaration::StateProvider(_) => {}
            }
        }
    }

    fn emit_functions(
        &self,
        output: &mut String,
        owner: &CallableOwnerDeclaration,
        prefix: &str,
        owner_expression: &str,
        receiver: &str,
        inherited: Option<&[TypeParameter]>,
    ) {
        let mut emitted = HashSet::new();
        for function in &owner.functions {
            if !emitted.insert(function.name.as_str()) {
                continue;
            }
            let overloads = owner
                .functions
                .iter()
                .filter(|candidate| candidate.name == function.name)
                .collect::<Vec<_>>();
            let id_prefix = if prefix.is_empty() {
                String::new()
            } else {
                path_ident(prefix)
            };
            let id = format!("{id_prefix}{}", ident(&function.name));
            let intrinsic = optional_attribute_name(&function.attributes, "intrinsic");
            // Instance members can refer to their owner's parameters in both
            // their signature and body. Static members are constructors or
            // factories with no receiver carrying those arguments, so their
            // generic scope is deliberately independent from the owner.
            let mut type_parameters = if function.is_static {
                Vec::new()
            } else {
                inherited.unwrap_or_default().to_vec()
            };
            type_parameters.extend(function.type_parameters.clone());
            for constrained in &function.where_constraints {
                let parameter = type_parameters
                    .iter_mut()
                    .find(|parameter| parameter.name == constrained.name)
                    .expect("validated where clauses reference an available type parameter");
                parameter
                    .constraints
                    .extend(constrained.constraints.clone());
            }
            let kind = if function.is_constant {
                "ItemKind::Constant".to_owned()
            } else if function.is_static {
                "ItemKind::Function".to_owned()
            } else {
                format!("ItemKind::Method {{ receiver: {receiver} }}")
            };
            if has_attribute(&owner.attributes, "testOnly") {
                output.push_str("#[cfg(test)] ");
            }
            if overloads.len() == 1 {
                self.emit_item(
                    output,
                    function,
                    owner,
                    &id,
                    intrinsic,
                    &type_parameters,
                    owner_expression,
                    prefix,
                    &kind,
                );
            } else {
                self.emit_capability_overload(
                    output,
                    function,
                    owner,
                    &id,
                    &overloads,
                    owner_expression,
                    prefix,
                    &kind,
                );
            }
        }
    }

    #[allow(clippy::too_many_arguments)]
    fn emit_capability_overload(
        &self,
        output: &mut String,
        public: &FunctionDeclaration,
        owner: &CallableOwnerDeclaration,
        id: &str,
        cases: &[&FunctionDeclaration],
        owner_expression: &str,
        prefix: &str,
        kind: &str,
    ) {
        let associated_types = self.visible_associated_types(owner);
        let mut public_parameters = public.type_parameters.clone();
        public_parameters[0].constraints = vec!["Numeric".to_owned()];
        let case_values = cases
            .iter()
            .map(|case| {
                let mut parameters = case.type_parameters.clone();
                for constrained in &case.where_constraints {
                    parameters
                        .iter_mut()
                        .find(|parameter| parameter.name == constrained.name)
                        .expect("validated overload constraints name a parameter")
                        .constraints
                        .extend(constrained.constraints.clone());
                }
                let capability = &parameters[0].constraints[0];
                let function_name = format!(
                    "__splitscript_stdlib_{id}{}",
                    ident(capability)
                );
                format!(
                    "LibraryOverloadCase {{ capability: StdlibCapabilityId::{}, signature: Signature {{ type_parameters: {}, explicit_type_parameters: {}, parameters: &[{}], result_is_async: {}, result: {} }}, function_name: {}, body: {} }}",
                    ident(capability),
                    self.type_parameters(&parameters, owner),
                    case.type_parameters.len(),
                    case.parameters.iter().map(|parameter| self.parameter(parameter, &parameters, &associated_types)).collect::<Vec<_>>().join(","),
                    case.result_is_async,
                    self.type_ref(&case.result, &parameters, &associated_types),
                    quote(&function_name),
                    quote(case.body.as_deref().expect("validated overload cases have bodies")),
                )
            })
            .collect::<Vec<_>>()
            .join(",");
        let qualified_name = if prefix.is_empty() {
            public.name.clone()
        } else {
            format!("{prefix}.{}", public.name)
        };
        let must_use = optional_attribute_name(&public.attributes, "mustUse")
            .map(|reason| format!("Some({})", quote(reason)))
            .unwrap_or_else(|| "None".to_owned());
        let examples = public
            .documentation
            .examples
            .first()
            .map(example_expression)
            .map(|example| format!("&[{example}]"))
            .unwrap_or_else(|| "&[]".to_owned());
        output.push_str(&format!(
            "StdlibItem {{ id: StdlibItemId::{id}, owner: {owner_expression}, visibility: ItemVisibility::{}, name: {}, qualified_name: {}, kind: {kind}, binary_operator: None, unary_operator: None, signature: Signature {{ type_parameters: {}, explicit_type_parameters: {}, parameters: &[{}], result_is_async: {}, result: {} }}, must_use: {must_use}, deprecation: None, documentation: Documentation {{ summary: {}, details: {}, examples: {examples}, related: &[] }}, intrinsic_context: None, implementation: Implementation::LibraryOverloads {{ dispatch_parameter: 0, cases: &[{case_values}] }} }},\n",
            if owner.private || public.private { "LibraryPrivate" } else { "Public" },
            quote(&public.name),
            quote(&qualified_name),
            self.type_parameters(&public_parameters, owner),
            public.type_parameters.len(),
            public.parameters.iter().map(|parameter| self.parameter(parameter, &public_parameters, &associated_types)).collect::<Vec<_>>().join(","),
            public.result_is_async,
            self.type_ref(&public.result, &public_parameters, &associated_types),
            quote(&public.documentation.summary),
            quote(&public.documentation.details),
        ));
    }

    #[allow(clippy::too_many_arguments)]
    fn emit_item(
        &self,
        output: &mut String,
        function: &FunctionDeclaration,
        owner: &CallableOwnerDeclaration,
        id: &str,
        intrinsic: Option<&str>,
        type_parameters: &[TypeParameter],
        owner_expression: &str,
        prefix: &str,
        kind: &str,
    ) {
        let associated_types = self.visible_associated_types(owner);
        let qualified_name = if prefix.is_empty() {
            function.name.clone()
        } else {
            format!("{prefix}.{}", function.name)
        };
        let implementation = if let Some(intrinsic) = intrinsic {
            format!("Implementation::Intrinsic(IntrinsicId::{intrinsic})")
        } else if function.body.is_none() {
            "Implementation::CapabilityRequirement".to_owned()
        } else {
            let function_name = format!("__splitscript_stdlib_{id}");
            format!(
                "Implementation::LibraryBody {{ function_name: {}, body: {} }}",
                quote(&function_name),
                quote(
                    function
                        .body
                        .as_deref()
                        .expect("validated source-defined functions have bodies")
                ),
            )
        };
        let intrinsic_context = if intrinsic.is_some() {
            intrinsic_context_expression(&function.attributes)
        } else {
            "None".to_owned()
        };
        let must_use = optional_attribute_name(&function.attributes, "mustUse")
            .map(|reason| format!("Some({})", quote(reason)))
            .unwrap_or_else(|| "None".to_owned());
        let binary_operator = optional_attribute_name(&function.attributes, "operator")
            .and_then(|operator| match operator {
                "add" => Some("Some(StandardBinaryOperator::Add)"),
                "subtract" => Some("Some(StandardBinaryOperator::Subtract)"),
                "multiply" => Some("Some(StandardBinaryOperator::Multiply)"),
                "divide" => Some("Some(StandardBinaryOperator::Divide)"),
                "remainder" => Some("Some(StandardBinaryOperator::Remainder)"),
                "bitOr" => Some("Some(StandardBinaryOperator::BitOr)"),
                "bitXor" => Some("Some(StandardBinaryOperator::BitXor)"),
                "bitAnd" => Some("Some(StandardBinaryOperator::BitAnd)"),
                "shiftLeft" => Some("Some(StandardBinaryOperator::ShiftLeft)"),
                "shiftRight" => Some("Some(StandardBinaryOperator::ShiftRight)"),
                "equal" => Some("Some(StandardBinaryOperator::Equal)"),
                "notEqual" => Some("Some(StandardBinaryOperator::NotEqual)"),
                "lessThan" => Some("Some(StandardBinaryOperator::LessThan)"),
                "lessThanOrEqual" => Some("Some(StandardBinaryOperator::LessThanOrEqual)"),
                "greaterThan" => Some("Some(StandardBinaryOperator::GreaterThan)"),
                "greaterThanOrEqual" => Some("Some(StandardBinaryOperator::GreaterThanOrEqual)"),
                "not" | "negate" => None,
                _ => unreachable!("validated operator binding"),
            })
            .unwrap_or("None");
        let unary_operator = optional_attribute_name(&function.attributes, "operator")
            .and_then(|operator| match operator {
                "not" => Some("Some(StandardUnaryOperator::Not)"),
                "negate" => Some("Some(StandardUnaryOperator::Negate)"),
                "add" | "subtract" | "multiply" | "divide" | "remainder" | "bitOr" | "bitXor"
                | "bitAnd" | "shiftLeft" | "shiftRight" | "equal" | "notEqual" | "lessThan"
                | "lessThanOrEqual" | "greaterThan" | "greaterThanOrEqual" => None,
                _ => unreachable!("validated operator binding"),
            })
            .unwrap_or("None");
        let examples = function
            .documentation
            .examples
            .first()
            .map(example_expression)
            .map(|example| format!("&[{example}]"))
            .unwrap_or_else(|| "&[]".to_owned());
        output.push_str(&format!(
                "StdlibItem {{ id: StdlibItemId::{id}, owner: {owner_expression}, visibility: ItemVisibility::{}, name: {}, qualified_name: {}, kind: {kind}, binary_operator: {binary_operator}, unary_operator: {unary_operator}, signature: Signature {{ type_parameters: {}, explicit_type_parameters: {}, parameters: &[{}], result_is_async: {}, result: {} }}, must_use: {must_use}, deprecation: None, documentation: Documentation {{ summary: {}, details: {}, examples: {examples}, related: &[] }}, intrinsic_context: {intrinsic_context}, implementation: {implementation} }},\n",
                if owner.private || function.private { "LibraryPrivate" } else { "Public" },
                quote(&function.name),
                quote(&qualified_name),
                self.type_parameters(type_parameters, owner),
                function.type_parameters.len(),
                function.parameters.iter().map(|parameter| self.parameter(parameter, type_parameters, &associated_types)).collect::<Vec<_>>().join(","),
                function.result_is_async,
                self.type_ref(&function.result, type_parameters, &associated_types),
                quote(&function.documentation.summary), quote(&function.documentation.details),
            ));
    }

    fn type_parameters(
        &self,
        parameters: &[TypeParameter],
        owner: &CallableOwnerDeclaration,
    ) -> String {
        let values = parameters.iter().map(|parameter| {
            let constraints = if self.library.declarations.iter().any(|declaration| matches!(declaration, Declaration::Capability(candidate) if candidate.name == owner.name))
                && owner.type_parameters.iter().any(|owner_parameter| owner_parameter.name == parameter.name)
            {
                vec![owner.name.clone()]
            } else if !parameter.constraints.is_empty() {
                parameter.constraints.clone()
            } else {
                Vec::new()
            };
            format!(
                "TypeParameter {{ name: {}, constraints: &[{}] }}",
                quote(&parameter.name),
                constraints.iter().map(|constraint| format!("StdlibCapabilityId::{}", ident(constraint))).collect::<Vec<_>>().join(",")
            )
        }).collect::<Vec<_>>();
        format!("&[{}]", values.join(","))
    }

    fn visible_associated_types(
        &self,
        owner: &CallableOwnerDeclaration,
    ) -> Vec<crate::AssociatedTypeDeclaration> {
        let Some(capability) =
            self.library
                .declarations
                .iter()
                .find_map(|declaration| match declaration {
                    Declaration::Capability(candidate) if candidate.name == owner.name => {
                        Some(candidate)
                    }
                    _ => None,
                })
        else {
            return owner.associated_types.clone();
        };
        visible_capability_associated_types(self.library, capability)
            .into_iter()
            .map(|(_, associated)| associated.clone())
            .collect()
    }

    fn associated_type_requirements(&self, owner: &CallableOwnerDeclaration) -> String {
        let values = owner
            .associated_types
            .iter()
            .map(|associated| {
                let constraints = associated
                    .constraints
                    .iter()
                    .map(|constraint| format!("StdlibCapabilityId::{}", ident(constraint)))
                    .collect::<Vec<_>>()
                    .join(",");
                format!(
                    "StdlibAssociatedType {{ name: {}, constraints: &[{constraints}], documentation: {} }}",
                    quote(&associated.name),
                    self.documentation(&associated.documentation),
                )
            })
            .collect::<Vec<_>>()
            .join(",");
        format!("&[{values}]")
    }

    fn associated_type_definitions(&self, owner: &CallableOwnerDeclaration) -> String {
        self.associated_type_definitions_from(&owner.type_parameters, &owner.associated_types)
    }

    fn associated_type_definitions_for_type(&self, owner: &StructDeclaration) -> String {
        self.associated_type_definitions_from(&[], &owner.associated_types)
    }

    fn associated_type_definitions_from(
        &self,
        parameters: &[TypeParameter],
        associated_types: &[crate::AssociatedTypeDeclaration],
    ) -> String {
        let values = associated_types
            .iter()
            .map(|associated| {
                let value = associated
                    .value
                    .as_ref()
                    .expect("validated type-constructor associated types define values");
                format!(
                    "StdlibAssociatedTypeDefinition {{ name: {}, value: {}, documentation: {} }}",
                    quote(&associated.name),
                    self.type_ref(value, parameters, associated_types),
                    self.documentation(&associated.documentation),
                )
            })
            .collect::<Vec<_>>()
            .join(",");
        format!("&[{values}]")
    }

    fn parameter(
        &self,
        parameter: &crate::Parameter,
        type_parameters: &[TypeParameter],
        associated_types: &[crate::AssociatedTypeDeclaration],
    ) -> String {
        let rule = optional_attribute_name(&parameter.attributes, "literal");
        let constructor = if rule.is_some() {
            "literal_parameter"
        } else {
            "parameter"
        };
        let rule = rule
            .map(|rule| match rule {
                "string" => ", ParameterRule::StringLiteral",
                "signature" => ", ParameterRule::SignatureLiteral",
                _ => "",
            })
            .unwrap_or("");
        format!(
            "{constructor}({}, {}{rule}, {})",
            quote(&parameter.name),
            self.type_ref(&parameter.ty, type_parameters, associated_types),
            quote(&parameter.documentation.summary)
        )
    }

    fn application_type(&self, constructor: &str, parameters: &[TypeParameter]) -> String {
        format!(
            "TypeRef::Application {{ constructor: StdlibTypeConstructorId::{}, arguments: &[{}] }}",
            ident(constructor),
            parameters
                .iter()
                .map(|parameter| format!("TypeRef::Parameter({})", quote(&parameter.name)))
                .collect::<Vec<_>>()
                .join(",")
        )
    }

    fn type_ref(
        &self,
        ty: &Type,
        parameters: &[TypeParameter],
        associated_types: &[crate::AssociatedTypeDeclaration],
    ) -> String {
        match ty {
            Type::Async(value) => format!(
                "TypeRef::Async(&{})",
                self.type_ref(value, parameters, associated_types)
            ),
            Type::Iterator(item) => format!(
                "TypeRef::Iterator(&{})",
                self.type_ref(item, parameters, associated_types)
            ),
            Type::Option(value) => format!(
                "TypeRef::Application {{ constructor: StdlibTypeConstructorId::Option, arguments: &[{}] }}",
                self.type_ref(value, parameters, associated_types)
            ),
            Type::Result(value) => format!(
                "TypeRef::Application {{ constructor: StdlibTypeConstructorId::Result, arguments: &[{}] }}",
                self.type_ref(value, parameters, associated_types)
            ),
            Type::ExclusiveRange(value) => format!(
                "TypeRef::Application {{ constructor: StdlibTypeConstructorId::ExclusiveRange, arguments: &[{}] }}",
                self.type_ref(value, parameters, associated_types)
            ),
            Type::InclusiveRange(value) => format!(
                "TypeRef::Application {{ constructor: StdlibTypeConstructorId::InclusiveRange, arguments: &[{}] }}",
                self.type_ref(value, parameters, associated_types)
            ),
            Type::Array(element) => format!(
                "TypeRef::Application {{ constructor: StdlibTypeConstructorId::Array, arguments: &[{}] }}",
                self.type_ref(element, parameters, associated_types)
            ),
            Type::FixedArray { element, length } => format!(
                "TypeRef::FixedArray {{ element: &{}, length: {length} }}",
                self.type_ref(element, parameters, associated_types)
            ),
            Type::Callable {
                parameters: callable_parameters,
                result,
            } => format!(
                "TypeRef::Callable {{ parameters: &[{}], result: &{} }}",
                callable_parameters
                    .iter()
                    .map(|parameter| self.type_ref(parameter, parameters, associated_types))
                    .collect::<Vec<_>>()
                    .join(","),
                self.type_ref(result, parameters, associated_types),
            ),
            Type::Application {
                constructor,
                arguments,
            } => format!(
                "TypeRef::Application {{ constructor: StdlibTypeConstructorId::{}, arguments: &[{}] }}",
                ident(constructor),
                arguments
                    .iter()
                    .map(|argument| self.type_ref(argument, parameters, associated_types))
                    .collect::<Vec<_>>()
                    .join(",")
            ),
            Type::Name(name) if parameters.iter().any(|parameter| parameter.name == *name) => {
                format!("TypeRef::Parameter({})", quote(name))
            }
            Type::Name(name)
                if associated_types
                    .iter()
                    .any(|associated| associated.name == *name) =>
            {
                format!("TypeRef::Associated({})", quote(name))
            }
            Type::Name(name) if is_core_type(name) => {
                format!("TypeRef::Core(CoreTypeId::{})", ident(name))
            }
            Type::Name(name) if self.type_names.contains(name.as_str()) => {
                format!("TypeRef::Standard(StdlibTypeId::{})", ident(name))
            }
            _ => unreachable!("validated standard-library types resolve before generation"),
        }
    }

    fn capabilities(&self, attributes: &[Attribute]) -> String {
        let values = attribute_names(attributes, "capabilities");
        format!(
            "&[{}]",
            values
                .iter()
                .map(|capability| {
                    assert!(self.capability_names.contains(capability.as_str()));
                    format!("StdlibCapabilityId::{}", ident(capability))
                })
                .collect::<Vec<_>>()
                .join(",")
        )
    }

    fn representation(&self, attributes: &[Attribute]) -> String {
        let values = attribute_names(attributes, "representation");
        match values.as_slice() {
            [kind, storage] if kind == "scalar" => format!(
                "RuntimeRepresentation::Scalar {{ storage: CoreTypeId::{} }}",
                ident(storage)
            ),
            [kind, element, rest @ ..] if kind == "gcArray" => format!(
                "RuntimeRepresentation::GcArray {{ element: CoreTypeId::{}, mutable: {}, nullable: {} }}",
                ident(element),
                rest.iter().any(|value| value == "mutable"),
                rest.iter().any(|value| value == "nullable")
            ),
            [kind, rest @ ..] if kind == "gcStruct" => format!(
                "RuntimeRepresentation::GcStruct {{ nullable: {} }}",
                rest.iter().any(|value| value == "nullable")
            ),
            [kind, rest @ ..] if kind == "enum" => format!(
                "RuntimeRepresentation::Enum {{ nullable: {} }}",
                rest.iter().any(|value| value == "nullable")
            ),
            _ => panic!("invalid generated representation `{values:?}`"),
        }
    }

    fn value_usage(&self, attributes: &[Attribute]) -> String {
        let values = attribute_names(attributes, "valueUsage");
        let has = |name: &str| values.iter().any(|value| value == name);
        format!(
            "ValueUsage {{ struct_field: {}, enum_payload: {}, state_field: {}, local_variable: {}, global_variable: {} }}",
            has("structField"),
            has("enumPayload"),
            has("stateField"),
            has("localVariable"),
            has("globalVariable")
        )
    }
}

fn example_expression(example: &crate::Example) -> String {
    if let Some(validation) = &example.validation_source {
        format!(
            "Example::checked({}, {}, {})",
            quote(&example.title),
            quote(&example.source),
            quote(validation),
        )
    } else if is_complete_program_example(&example.source) {
        format!(
            "Example::complete_program({}, {})",
            quote(&example.title),
            quote(&example.source),
        )
    } else if let Some(provider) = &example.state_provider {
        format!(
            "Example::provider_on_attach_body({}, {}, {})",
            quote(&example.title),
            quote(&example.source),
            quote(provider),
        )
    } else {
        format!(
            "Example::on_attach_body({}, {})",
            quote(&example.title),
            quote(&example.source),
        )
    }
}

fn is_complete_program_example(source: &str) -> bool {
    let source = source.trim_start();
    [
        "state",
        "settings",
        "tickRate",
        "setup",
        "onAttach",
        "onDetach",
        "whileAttached",
        "start",
        "split",
        "reset",
        "isLoading",
        "gameTime",
        "fn",
        "struct",
        "enum",
        "image",
    ]
    .into_iter()
    .any(|keyword| {
        source.strip_prefix(keyword).is_some_and(|rest| {
            rest.starts_with(char::is_whitespace) || rest.starts_with('{') || rest.starts_with('(')
        })
    })
}

fn attribute_name<'a>(attributes: &'a [Attribute], name: &str) -> &'a str {
    optional_attribute_name(attributes, name)
        .unwrap_or_else(|| panic!("missing generated `@{name}` attribute"))
}

fn has_attribute(attributes: &[Attribute], name: &str) -> bool {
    attributes.iter().any(|attribute| attribute.name == name)
}

fn optional_attribute_name<'a>(attributes: &'a [Attribute], name: &str) -> Option<&'a str> {
    let attribute = attributes.iter().find(|attribute| attribute.name == name)?;
    match attribute.arguments.as_slice() {
        [AttributeArgument::Name(value) | AttributeArgument::String(value)] => Some(value),
        _ => panic!("generated `@{name}` must have exactly one argument"),
    }
}

fn attribute_names(attributes: &[Attribute], name: &str) -> Vec<String> {
    attributes
        .iter()
        .find(|attribute| attribute.name == name)
        .map(|attribute| {
            attribute
                .arguments
                .iter()
                .map(|argument| match argument {
                    AttributeArgument::Name(value) | AttributeArgument::String(value) => {
                        value.clone()
                    }
                    AttributeArgument::Integer(_) => {
                        panic!("generated `@{name}` expects names, not integers")
                    }
                })
                .collect()
        })
        .unwrap_or_default()
}

fn intrinsic_context_expression(attributes: &[Attribute]) -> String {
    let requirements = attribute_names(attributes, "requires");
    let requires = |name: &str| requirements.iter().any(|requirement| requirement == name);
    let availability = if optional_attribute_name(attributes, "availability") == Some("onAttach") {
        "Availability::OnAttach"
    } else {
        "Availability::Everywhere"
    };
    let cancellation =
        if optional_attribute_name(attributes, "cancellation") == Some("processClose") {
            "CancellationKind::ProcessClose"
        } else {
            "CancellationKind::None"
        };
    format!(
        "Some(IntrinsicContext {{ availability: {availability}, requires_attached_process: {}, requires_state_snapshots: {}, cancellation: {cancellation} }})",
        requires("attachedProcess"),
        requires("stateSnapshots"),
    )
}

fn quote(value: &str) -> String {
    format!("{value:?}")
}

fn is_core_type(name: &str) -> bool {
    PrimitiveType::parse(name).is_some()
}

pub fn generate_ids(library: &Library) -> Result<String, Vec<Error>> {
    let mut capabilities = Vec::new();
    let mut providers = Vec::new();
    let mut constructors = Vec::new();
    let mut namespaces = Vec::new();
    let mut types = Vec::new();
    let mut fields = Vec::new();
    let mut variants = Vec::new();
    let mut items = Vec::new();

    for declaration in &library.declarations {
        match declaration {
            Declaration::Root(owner) => {
                items.extend(unique_function_idents(&owner.functions, ""));
            }
            Declaration::StateProvider(provider) => providers.push(ident(&provider.name)),
            Declaration::Namespace(owner) => {
                let owner_id = path_ident(&owner.name);
                namespaces.push(owner_id.clone());
                items.extend(unique_function_idents(&owner.functions, &owner_id));
            }
            Declaration::Capability(owner) => {
                let owner_id = ident(&owner.name);
                capabilities.push(owner_id.clone());
                items.extend(unique_function_idents(&owner.functions, &owner_id));
            }
            Declaration::TypeConstructor(owner) => {
                let owner_id = ident(&owner.name);
                constructors.push(owner_id.clone());
                fields.extend(
                    owner
                        .fields
                        .iter()
                        .map(|field| format!("{owner_id}{}", ident(&field.name))),
                );
                items.extend(unique_function_idents(&owner.functions, &owner_id));
            }
            Declaration::CoreExtension(owner) => {
                let owner_id = ident(&owner.name);
                items.extend(unique_function_idents(&owner.functions, &owner_id));
            }
            Declaration::Struct(declaration) | Declaration::IntrinsicType(declaration) => {
                let owner_id = ident(&declaration.name);
                types.push(owner_id.clone());
                fields.extend(
                    declaration
                        .fields
                        .iter()
                        .map(|field| format!("{owner_id}{}", ident(&field.name))),
                );
                items.extend(unique_function_idents(&declaration.functions, &owner_id));
            }
            Declaration::Enum(declaration) => {
                let owner_id = ident(&declaration.name);
                types.push(owner_id.clone());
                variants.extend(
                    declaration
                        .variants
                        .iter()
                        .map(|variant| format!("{owner_id}{}", ident(&variant.name))),
                );
            }
        }
    }

    let groups = [
        ("capability", &capabilities),
        ("state provider", &providers),
        ("type constructor", &constructors),
        ("namespace", &namespaces),
        ("type", &types),
        ("field", &fields),
        ("variant", &variants),
        ("item", &items),
    ];
    let mut errors = Vec::new();
    for (description, values) in groups {
        let mut seen = HashSet::new();
        for value in values {
            if !seen.insert(value) {
                errors.push(Error {
                    message: format!("duplicate generated {description} identity `{value}`"),
                    start: 0,
                    end: 0,
                });
            }
        }
    }
    if !errors.is_empty() {
        return Err(errors);
    }

    let mut output = String::new();
    emit_group(&mut output, "StdlibCapabilityId", &capabilities);
    emit_group(&mut output, "StdlibStateProviderId", &providers);
    emit_group(&mut output, "StdlibTypeConstructorId", &constructors);
    emit_group(&mut output, "StdlibNamespaceId", &namespaces);
    emit_group(&mut output, "StdlibTypeId", &types);
    emit_group(&mut output, "StdlibFieldId", &fields);
    emit_group(&mut output, "StdlibVariantId", &variants);
    emit_group(&mut output, "StdlibItemId", &items);
    Ok(output)
}

fn unique_function_idents(functions: &[FunctionDeclaration], prefix: &str) -> Vec<String> {
    let mut names = HashSet::new();
    functions
        .iter()
        .filter(|function| names.insert(function.name.as_str()))
        .map(|function| format!("{prefix}{}", ident(&function.name)))
        .collect()
}

fn emit_group(output: &mut String, name: &str, values: &[String]) {
    output.push_str("catalog_id!(");
    output.push_str(name);
    output.push_str(", ");
    output.push_str(name);
    output.push_str("Discriminant {");
    for value in values {
        output.push_str(value);
        output.push(',');
    }
    output.push_str("});\n");
}

fn path_ident(path: &str) -> String {
    path.split('.').map(ident).collect()
}

fn ident(name: &str) -> String {
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
    use crate::parse;

    use super::*;

    #[test]
    fn intrinsic_asyncness_and_context_are_emitted_from_privileged_source() {
        let source = r#"
root {
    @requires(attachedProcess)
    @cancellation(processClose)
    @availability(onAttach)
    @intrinsic(NextTick)
    private fn nextTick() -> async None;
}
"#;
        let generated = generate_catalog(&parse(source).unwrap()).unwrap();
        assert!(generated.contains("result_is_async: true"));
        assert!(generated.contains("availability: Availability::OnAttach"));
        assert!(generated.contains("requires_attached_process: true"));
        assert!(generated.contains("cancellation: CancellationKind::ProcessClose"));
    }

    #[test]
    fn source_paths_and_member_names_produce_existing_style_identities() {
        let source = r#"
root { @intrinsic(Print) fn print() -> None; }
namespace process.read {
    @intrinsic(ProcessReadManagedString)
    fn managedString() -> String;
}
@behavior(declared)
capability Numeric { @intrinsic(NumericMin) fn min() -> Self; }
@representation(gcStruct)
@valueUsage(localVariable)
struct Duration {
    seconds: i64,
    @intrinsic(DurationFromSeconds)
    static fn fromSeconds() -> Duration;
}
@representation(enum)
@valueUsage(localVariable)
enum TimerState { NotRunning }
@processType(GbaEmulator)
@attachment(identity)
@directRead(GbaEmulatorRead)
stateProvider GBA as gba { "mGBA" }
"#;
        let generated = generate_ids(&parse(source).unwrap()).unwrap();
        for identity in [
            "Print",
            "ProcessRead",
            "ProcessReadManagedString",
            "NumericMin",
            "DurationSeconds",
            "DurationFromSeconds",
            "TimerStateNotRunning",
            "Gba",
        ] {
            assert!(generated.contains(identity), "missing {identity}");
        }
    }

    #[test]
    fn summary_only_documentation_is_not_copied_into_details() {
        let source = r#"
/// A documented structure.
///
/// # Example
///
/// Construct the struct
///
/// ```splitscript
/// let value = Example { field: 1 }
/// ```
@representation(gcStruct)
@valueUsage(localVariable)
struct Example {
    /// A concise field summary.
    ///
    /// # Example
    ///
    /// Read the field
    ///
    /// ```splitscript
    /// let field = value.field
    /// ```
    field: i32,
}
"#;
        let generated = generate_catalog(&parse(source).unwrap()).unwrap();
        assert!(generated.contains("summary: \"A concise field summary.\", details: \"\""));
        assert!(!generated.contains(
            "summary: \"A concise field summary.\", details: \"A concise field summary.\""
        ));
    }

    #[test]
    fn generic_constructor_fields_keep_their_owner_and_parameter() {
        let source = r#"
/// Exclusive ranges.
///
/// # Example
///
/// Iterate over a range
///
/// ```splitscript
/// for value in 1..<3 { print(value) }
/// ```
typeConstructor T..<T {
    /// The lower endpoint.
    ///
    /// # Example
    ///
    /// Read the beginning
    ///
    /// ```splitscript
    /// let beginning = (1..<3).start
    /// ```
    start: T,
}

/// Integer values.
///
/// # Example
///
/// Store an integer
///
/// ```splitscript
/// let value = 1
/// ```
@behavior(declared)
capability Integer {}
"#;
        let library = parse(source).unwrap();
        let generated = generate_catalog(&library).unwrap();
        assert!(generated.contains("StdlibFieldId::ExclusiveRangeStart"));
        assert!(generated.contains(
            "owner: StdlibOwner::TypeConstructor(StdlibTypeConstructorId::ExclusiveRange)"
        ));
        assert!(generated.contains("ty: TypeRef::Parameter(\"T\")"));

        let ids = generate_ids(&library).unwrap();
        assert!(ids.contains("ExclusiveRangeStart"));
    }

    #[test]
    fn source_body_generates_an_ordinary_hidden_function() {
        let source = r#"
/// Duration.
///
/// # Example
///
/// Store a duration
///
/// ```splitscript
/// let delay: Duration = Duration.fromFrames(120, 60)
/// ```
@representation(gcStruct)
@valueUsage(localVariable)
struct Duration {
    /// Constructs a duration.
    ///
    /// Delegates to a primitive.
    ///
    /// # Example
    ///
    /// Convert frames
    ///
    /// ```splitscript
    /// return Duration.fromFrames(120, 60)
    /// ```
    static fn fromFrames(
        /// Frame count.
        frames: i64,
        /// Frames per second.
        fps: i64,
    ) -> Duration {
        return Duration.fromParts(frames / fps, 0)
    }
}
"#;
        let generated = generate_catalog(&parse(source).unwrap()).unwrap();
        assert!(generated.contains("Implementation::LibraryBody"));
        assert!(generated.contains("__splitscript_stdlib_DurationFromFrames"));
        assert!(generated.contains("return Duration.fromParts(frames / fps, 0)"));
        assert!(!generated.contains("IntrinsicId::DurationFromFrames"));
    }

    #[test]
    fn integer_and_float_source_cases_generate_one_numeric_operation() {
        let source = r#"
/// Numeric values.
///
/// # Example
///
/// Use a numeric value
///
/// ```splitscript
/// let numericValue = 1
/// ```
@behavior(declared)
capability Numeric {}
/// Integer values.
///
/// # Example
///
/// Use an integer
///
/// ```splitscript
/// let integerValue = 2
/// ```
@behavior(declared)
capability Integer: Numeric {}
/// Floating-point values.
///
/// # Example
///
/// Use a floating-point value
///
/// ```splitscript
/// let floatValue = 1.5
/// ```
@behavior(declared)
capability Float: Numeric {}

root {
    /// Preserves a numeric value.
    ///
    /// Selects the implementation for the caller's concrete numeric type.
    ///
    /// # Example
    ///
    /// Preserve a value
    ///
    /// ```splitscript
    /// let value = preserve(2)
    /// ```
    fn preserve<T: Integer>(
        /// The value to preserve.
        value: T,
    ) -> T {
        return value
    }

    fn preserve<T: Float>(
        /// The value to preserve.
        value: T,
    ) -> T {
        return value
    }
}
"#;
        let generated = generate_catalog(&parse(source).unwrap()).unwrap();
        assert!(generated.contains("Implementation::LibraryOverloads"));
        assert!(generated.contains(
            "TypeParameter { name: \"T\", constraints: &[StdlibCapabilityId::Numeric] }"
        ));
        assert!(generated.contains("capability: StdlibCapabilityId::Integer"));
        assert!(generated.contains("capability: StdlibCapabilityId::Float"));
        assert!(generated.contains("__splitscript_stdlib_PreserveInteger"));
        assert!(generated.contains("__splitscript_stdlib_PreserveFloat"));

        let invalid = source.replace("fn preserve<T: Float>", "fn preserve<T: Integer>");
        let errors = generate_catalog(&parse(&invalid).unwrap()).unwrap_err();
        assert!(
            errors
                .iter()
                .any(|error| error.message.contains("cover both `Integer` and `Float`")),
            "{errors:#?}"
        );
    }

    #[test]
    fn standard_types_can_name_a_source_defined_display_implementation() {
        let source = r#"
/// Displayable values.
///
/// # Example
///
/// Interpolate a value
///
/// ```splitscript
/// let text = `{42}`
/// ```
@behavior(declared)
capability Display {}

/// Text.
///
/// # Example
///
/// Store text
///
/// ```splitscript
/// let text: String = "value"
/// ```
@representation(gcArray, u8, mutable, nullable)
@valueUsage(localVariable)
@capabilities(Display)
intrinsic type String {}

/// A file version.
///
/// # Example
///
/// Store a version
///
/// ```splitscript
/// let version: FileVersion = fileVersion
/// ```
@representation(gcStruct, nullable)
@valueUsage(localVariable)
@capabilities(Display)
struct FileVersion {
    /// Major component.
    ///
    /// # Example
    ///
    /// Read the major component
    ///
    /// ```splitscript
    /// let major = version.major
    /// ```
    major: u16,

    /// Formats the version.
    ///
    /// Uses dotted components.
    ///
    /// # Example
    ///
    /// Display a version
    ///
    /// ```splitscript
    /// print(version)
    /// ```
    @display
    fn toString() -> String {
        return `{self.major}`
    }
}
"#;
        let generated = generate_catalog(&parse(source).unwrap()).unwrap();
        assert!(generated.contains("display: Some(StdlibItemId::FileVersionToString)"));
        assert!(generated.contains("Implementation::LibraryBody"));
        assert!(generated.contains("__splitscript_stdlib_FileVersionToString"));
    }

    #[test]
    fn non_callable_examples_survive_catalog_generation_as_focused_snippets() {
        let source = r#"
/// Timer operations.
///
/// # Example
///
/// Inspect the timer
///
/// ```splitscript
/// let state = timer.state()
/// ```
namespace timer {}

/// Optional values.
///
/// # Example
///
/// Store an optional value
///
/// ```splitscript
/// let value: u32? = 4
/// ```
typeConstructor T? {}

/// Lifecycle hooks.
///
/// # Example
///
/// Pause after detaching
///
/// ```splitscript
/// onDetach {
///     timer.pauseGameTime()
/// }
/// ```
namespace lifecycle {}
"#;
        let generated = generate_catalog(&parse(source).unwrap()).unwrap();
        assert!(generated.contains(
            "Example::on_attach_body(\"Inspect the timer\", \"let state = timer.state()\")"
        ));
        assert!(generated.contains(
            "Example::on_attach_body(\"Store an optional value\", \"let value: u32? = 4\")"
        ));
        assert!(
            generated.contains("Example::complete_program(\"Pause after detaching\", \"onDetach {")
        );
    }

    #[test]
    fn example_generation_preserves_authored_hidden_context_and_provider_context() {
        let hidden = crate::Example {
            title: "Read a value".into(),
            source: "let value = process.read<u32>(0x1000) else 0".into(),
            validation_source: Some(
                "state \"game.exe\" {}\nonAttach {\nlet value = process.read<u32>(0x1000) else 0\n}"
                    .into(),
            ),
            state_provider: None,
        };
        assert!(example_expression(&hidden).starts_with("Example::checked("));

        let provider = crate::Example {
            title: "Read GBA memory".into(),
            source: "let value: u8 = gba.read(0x03000010) else 0".into(),
            validation_source: None,
            state_provider: Some("GBA".into()),
        };
        assert_eq!(
            example_expression(&provider),
            "Example::provider_on_attach_body(\"Read GBA memory\", \"let value: u8 = gba.read(0x03000010) else 0\", \"GBA\")"
        );
    }

    #[test]
    fn generic_capability_body_is_preserved_as_a_typed_template() {
        let source = r#"
/// Numeric values.
///
/// # Example
///
/// Add numbers
///
/// ```splitscript
/// let total = 1 + 2
/// ```
@behavior(declared)
capability Numeric {
    /// Restricts a value.
    ///
    /// Uses numeric primitives to apply both bounds.
    ///
    /// # Example
    ///
    /// Restrict a value
    ///
    /// ```splitscript
    /// let bounded = value.clamp(0, 7)
    /// ```
    fn clamp(
        /// The lower bound.
        minimum: Self,
        /// The upper bound.
        maximum: Self,
    ) -> Self {
        let lowerBounded = self.max(minimum)
        return lowerBounded.min(maximum)
    }
}
"#;
        let generated = generate_catalog(&parse(source).unwrap()).unwrap();
        assert!(generated.contains("Implementation::LibraryBody"));
        assert!(generated.contains(
            "TypeParameter { name: \"Self\", constraints: &[StdlibCapabilityId::Numeric] }"
        ));
        assert!(generated.contains("let lowerBounded = self.max(minimum)"));
        assert!(!generated.contains("IntrinsicId::NumericClamp"));
    }

    #[test]
    fn capability_constraints_generate_super_capabilities() {
        let source = r#"
/// Displayable values.
///
/// # Example
///
/// Display a value
///
/// ```splitscript
/// let text = `{1}`
/// ```
@behavior(declared)
capability Display {}
/// Equatable values.
///
/// # Example
///
/// Compare values
///
/// ```splitscript
/// let equal = 1 == 1
/// ```
@behavior(structuralEquality)
capability Equatable {}
/// Numeric values.
///
/// # Example
///
/// Add values
///
/// ```splitscript
/// let total = 1 + 3
/// ```
@behavior(declared)
capability Numeric: Equatable {}
/// Integer values.
///
/// # Example
///
/// Shift a value
///
/// ```splitscript
/// let mask = 1 << 2
/// ```
@behavior(declared)
capability Integer: Numeric + Display {}
"#;
        let generated = generate_catalog(&parse(source).unwrap()).unwrap();
        assert!(generated.contains(
            "super_capabilities: &[StdlibCapabilityId::Numeric,StdlibCapabilityId::Display]"
        ));
        assert!(generated.contains("super_capabilities: &[StdlibCapabilityId::Equatable]"));
    }

    #[test]
    fn bundled_source_generates_final_typed_catalog_arrays() {
        let source = include_str!("../../../stdlib/standard.split");
        let generated = generate_catalog(&parse(source).unwrap()).unwrap();
        let retired_invocation = ["standard_", "library!"].concat();

        for declaration in [
            "pub(super) const STATE_PROVIDERS: &[StdlibStateProvider]",
            "pub(super) const TYPES: &[StdlibType]",
            "pub(super) const ITEMS: &[StdlibItem]",
        ] {
            assert!(generated.contains(declaration), "missing `{declaration}`");
        }
        assert!(generated.contains("StateProviderProcesses::SourceState"));
        assert!(generated.contains("StateProviderAttachment::Identity"));
        assert!(generated.contains("StateProviderMemoryRange { start: 1048576, end: 33554432 }"));
        assert!(generated.contains("ItemKind::Method"));
        assert!(generated.contains(
            "TypeRef::FixedArray { element: &TypeRef::Core(CoreTypeId::U16), length: 3 }"
        ));
        assert!(generated.contains(
            "TypeRef::Application { constructor: StdlibTypeConstructorId::Array, arguments: &[TypeRef::Async(&TypeRef::Parameter(\"T\"))] }"
        ));
        assert!(!generated.contains(&retired_invocation));
    }

    #[test]
    fn configured_state_provider_selectors_generate_typed_catalog_data() {
        let source = include_str!("../../../stdlib/standard.split").replace(
            "stateProvider Native as process {}",
            r#"stateProvider Native as process {
    /// Selects a runtime metadata version.
    ///
    /// This bypasses automatic version detection.
    @prepare(Print)
    selector runtime(version: u32, mono: MonoVersion),
}"#,
        );
        let generated = generate_catalog(&parse(&source).unwrap()).unwrap();
        assert!(generated.contains(
            "StateProviderSelector { name: \"runtime\", parameters: &[StateProviderSelectorParameter { name: \"version\", ty: TypeRef::Core(CoreTypeId::U32) },StateProviderSelectorParameter { name: \"mono\", ty: TypeRef::Standard(StdlibTypeId::MonoVersion) }], preparation: StdlibItemId::Print, managed_backend: None"
        ));
    }
}
