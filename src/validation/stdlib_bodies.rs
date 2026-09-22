//! Semantic validation for privileged source-defined standard-library bodies.
//!
//! The catalog owns each public type scheme, while its ordinary SplitScript
//! body is inferred by the same checker as user code. This boundary proves the
//! two schemes agree before operation analysis or backend specialization can
//! consume the hidden function template.

use std::collections::HashMap;

use crate::{
    Diagnostic,
    ast::{FunctionId, Program, Span},
    hir::TypedProgram,
    semantic::SemanticModel,
    stdlib::{
        CapabilityBehavior, Implementation, ItemKind, Signature, StandardLibrary,
        StdlibCapabilityId, StdlibTypeConstructorId, TypeRef,
    },
    types::{TypeId, TypeKind, generic_parameter_name},
};

pub(super) fn validate_signatures(
    library: &StandardLibrary,
    syntax: &Program,
    hir: &TypedProgram,
    semantics: &SemanticModel,
) -> Vec<Diagnostic> {
    let mut diagnostics = Vec::new();
    for item in library.all_items() {
        let bodies = match item.implementation {
            Implementation::Intrinsic(_) | Implementation::CapabilityRequirement => continue,
            Implementation::LibraryBody { .. } => hir
                .library_function(item.id)
                .map(|function| vec![(function, item.signature)])
                .unwrap_or_default(),
            Implementation::LibraryOverloads { cases, .. } => cases
                .iter()
                .enumerate()
                .filter_map(|(index, case)| {
                    hir.library_overload_function(item.id, index)
                        .map(|function| (function, case.signature))
                })
                .collect(),
        };
        if bodies.is_empty()
            && hir.expects_library_bodies()
            && library.source_body_operations_are_initialized()
        {
            // Unselected templates were validated by the complete bootstrap.
            continue;
        }
        if bodies.is_empty() {
            diagnostics.push(Diagnostic::semantic(
                format!(
                    "standard-library body `{}` has no inferred function template",
                    item.qualified_name
                ),
                Span::default(),
            ));
            continue;
        }
        for (function, signature) in bodies {
            let span = syntax
                .functions
                .iter()
                .find(|declaration| declaration.id == function)
                .map(|declaration| declaration.span)
                .unwrap_or_default();
            if let Err(reason) =
                validate_signature(library, item.kind, signature, function, semantics)
            {
                diagnostics.push(
                    Diagnostic::semantic(
                        format!(
                            "standard-library body `{}` does not match its declared signature: {reason}",
                            item.qualified_name
                        ),
                        span,
                    )
                    .with_primary_label("this privileged body inferred a different type scheme"),
                );
            }
        }
    }
    diagnostics
}

fn validate_signature(
    library: &StandardLibrary,
    kind: ItemKind,
    signature: Signature,
    function: FunctionId,
    semantics: &SemanticModel,
) -> Result<(), String> {
    let expected_parameters = match kind {
        ItemKind::Function | ItemKind::Constant => signature
            .parameters
            .iter()
            .map(|parameter| (parameter.name, parameter.ty))
            .collect::<Vec<_>>(),
        ItemKind::Method { receiver } => std::iter::once(("receiver", receiver))
            .chain(
                signature
                    .parameters
                    .iter()
                    .map(|parameter| (parameter.name, parameter.ty)),
            )
            .collect(),
    };
    let actual_parameters = semantics.function_parameter_types(function);
    if actual_parameters.len() != expected_parameters.len() {
        return Err(format!(
            "declares {} value parameter(s), but its body inferred {}",
            expected_parameters.len(),
            actual_parameters.len()
        ));
    }

    let mut matcher = SchemeMatcher {
        library,
        semantics,
        function,
        inferred_to_declared: HashMap::new(),
    };
    for ((name, expected), actual) in expected_parameters
        .into_iter()
        .zip(actual_parameters.iter().copied())
    {
        matcher.slot(name, expected, actual)?;
    }
    let actual_result = if signature.result_is_async {
        semantics.function_completion(function)
    } else {
        semantics.function_result(function)
    };
    let Some(actual_result) = actual_result else {
        return Err("its body has no inferred result type".to_owned());
    };
    matcher.slot("result", signature.result, actual_result)?;

    for parameter in signature.type_parameters {
        let occurs = match kind {
            ItemKind::Function | ItemKind::Constant => false,
            ItemKind::Method { receiver } => type_ref_contains(receiver, parameter.name),
        } || signature
            .parameters
            .iter()
            .any(|value| type_ref_contains(value.ty, parameter.name))
            || type_ref_contains(signature.result, parameter.name);
        if !occurs {
            return Err(format!(
                "declared type parameter `{}` does not occur in the callable type scheme",
                parameter.name
            ));
        }
    }

    for inferred in semantics.function_type_parameters(function) {
        let Some(declared) = matcher.inferred_to_declared.get(inferred).copied() else {
            return Err(format!(
                "the body inferred an unbound type parameter `{}`",
                inferred_parameter_name(semantics, *inferred)
            ));
        };
        for required in semantics.generic_parameter_constraints(*inferred) {
            if !declared_type_has_capability(library, signature, declared, *required) {
                return Err(format!(
                    "inferred type parameter `{}` maps to `{}`, which does not guarantee the body's required `{}` capability",
                    inferred_parameter_name(semantics, *inferred),
                    render_declared_type(library, declared),
                    library.capability(*required).name,
                ));
            }
        }
    }
    Ok(())
}

struct SchemeMatcher<'a> {
    library: &'a StandardLibrary,
    semantics: &'a SemanticModel,
    function: FunctionId,
    inferred_to_declared: HashMap<TypeId, TypeRef>,
}

impl SchemeMatcher<'_> {
    fn slot(&mut self, slot: &str, expected: TypeRef, actual: TypeId) -> Result<(), String> {
        if let Err(detail) = self.ty(expected, actual) {
            return Err(format!(
                "{slot} is declared as `{}`, but the body inferred `{}`{detail}",
                render_declared_type(self.library, expected),
                render_actual_type(
                    self.library,
                    self.semantics,
                    actual,
                    &self.inferred_to_declared,
                ),
            ));
        }
        Ok(())
    }

    fn ty(&mut self, expected: TypeRef, actual: TypeId) -> Result<(), String> {
        if let TypeKind::GenericParameter { owner, .. } = self.semantics.types().kind(actual) {
            if *owner != self.function {
                return Err("; the inferred parameter belongs to another function".to_owned());
            }
            if let Some(bound) = self.inferred_to_declared.get(&actual) {
                return (*bound == expected).then_some(()).ok_or_else(|| {
                    "; repeated uses of one inferred parameter map to inconsistent declared types"
                        .to_owned()
                });
            }
            self.inferred_to_declared.insert(actual, expected);
            return Ok(());
        }
        match expected {
            TypeRef::Core(expected) => match self.semantics.types().kind(actual) {
                TypeKind::Builtin(actual) if *actual == expected => Ok(()),
                _ => Err(String::new()),
            },
            TypeRef::Standard(expected) => match self.semantics.types().kind(actual) {
                TypeKind::Standard(actual) if *actual == expected => Ok(()),
                TypeKind::SettingsView if expected == crate::stdlib::StdlibTypeId::SettingsView => {
                    Ok(())
                }
                _ => Err(String::new()),
            },
            TypeRef::Parameter(_) => {
                Err("; the body narrowed a declared type parameter to a concrete type".to_owned())
            }
            TypeRef::Associated(_) => {
                Err("; source bodies cannot independently narrow an associated type".to_owned())
            }
            TypeRef::Async(value) => {
                let TypeKind::Async {
                    value: actual_value,
                    ..
                } = self.semantics.types().kind(actual)
                else {
                    return Err(String::new());
                };
                self.ty(*value, *actual_value)
            }
            TypeRef::Iterator(item) => {
                let TypeKind::Iterator {
                    item: actual_item, ..
                } = self.semantics.types().kind(actual)
                else {
                    return Err(String::new());
                };
                self.ty(*item, *actual_item)
            }
            TypeRef::Callable { parameters, result } => {
                let TypeKind::Callable {
                    parameters: actual_parameters,
                    result: actual_result,
                    ..
                } = self.semantics.types().kind(actual)
                else {
                    return Err(String::new());
                };
                if parameters.len() != actual_parameters.len() {
                    return Err(String::new());
                }
                for (expected, actual) in parameters.iter().zip(actual_parameters) {
                    self.ty(*expected, *actual)?;
                }
                self.ty(*result, *actual_result)
            }
            TypeRef::Application {
                constructor,
                arguments,
            } => {
                let (actual_arguments, actual_length) =
                    constructed_arguments(self.semantics, constructor, actual)
                        .ok_or_else(String::new)?;
                if constructor == StdlibTypeConstructorId::Array && actual_length.is_some() {
                    return Err(
                        "; an unbounded array declaration inferred a fixed length".to_owned()
                    );
                }
                if arguments.len() != actual_arguments.len() {
                    return Err(String::new());
                }
                for (expected, actual) in arguments.iter().zip(actual_arguments) {
                    self.ty(*expected, actual)?;
                }
                Ok(())
            }
            TypeRef::FixedArray { element, length } => {
                let TypeKind::Array {
                    element: actual_element,
                    length: actual_length,
                    ..
                } = self.semantics.types().kind(actual)
                else {
                    return Err(String::new());
                };
                if actual_length.is_some_and(|actual| actual != length) {
                    return Err(format!(
                        "; declared array length is {length}, inferred {}",
                        actual_length.expect("checked above")
                    ));
                }
                self.ty(*element, *actual_element)
            }
        }
    }
}

fn type_ref_contains(ty: TypeRef, parameter: &str) -> bool {
    match ty {
        TypeRef::Parameter(name) => name == parameter,
        TypeRef::Associated(_) => false,
        TypeRef::Async(value) | TypeRef::Iterator(value) => type_ref_contains(*value, parameter),
        TypeRef::Application { arguments, .. } => arguments
            .iter()
            .any(|argument| type_ref_contains(*argument, parameter)),
        TypeRef::FixedArray { element, .. } => type_ref_contains(*element, parameter),
        TypeRef::Callable { parameters, result } => {
            parameters
                .iter()
                .any(|ty| type_ref_contains(*ty, parameter))
                || type_ref_contains(*result, parameter)
        }
        TypeRef::Core(_) | TypeRef::Standard(_) => false,
    }
}

fn constructed_arguments(
    semantics: &SemanticModel,
    constructor: StdlibTypeConstructorId,
    actual: TypeId,
) -> Option<(Vec<TypeId>, Option<u32>)> {
    match (constructor, semantics.types().kind(actual)) {
        (
            StdlibTypeConstructorId::Array,
            TypeKind::Array {
                element, length, ..
            },
        ) => Some((vec![*element], *length)),
        (StdlibTypeConstructorId::Option, TypeKind::Option { value, .. })
        | (StdlibTypeConstructorId::Result, TypeKind::Result { value, .. }) => {
            Some((vec![*value], None))
        }
        (StdlibTypeConstructorId::Set, TypeKind::Set { element, .. }) => {
            Some((vec![*element], None))
        }
        (
            expected,
            TypeKind::Application {
                constructor,
                arguments,
                ..
            },
        ) if expected == *constructor => Some((arguments.clone(), None)),
        (
            StdlibTypeConstructorId::ExclusiveRange,
            TypeKind::Range {
                bound,
                kind: crate::ast::RangeKind::Exclusive,
                ..
            },
        )
        | (
            StdlibTypeConstructorId::InclusiveRange,
            TypeKind::Range {
                bound,
                kind: crate::ast::RangeKind::Inclusive,
                ..
            },
        ) => Some((vec![*bound], None)),
        _ => None,
    }
}

fn inferred_parameter_name(semantics: &SemanticModel, parameter: TypeId) -> String {
    let TypeKind::GenericParameter { index, .. } = semantics.types().kind(parameter) else {
        unreachable!("function type parameters are generic")
    };
    generic_parameter_name(*index)
}

fn declared_type_has_capability(
    library: &StandardLibrary,
    signature: Signature,
    ty: TypeRef,
    capability: StdlibCapabilityId,
) -> bool {
    match ty {
        TypeRef::Core(core) => library.core_type_has_capability(core, capability),
        TypeRef::Standard(standard) => library.type_has_capability(standard, capability),
        TypeRef::Parameter(name) => signature
            .type_parameters
            .iter()
            .find(|parameter| parameter.name == name)
            .is_some_and(|parameter| {
                library.capabilities_satisfy(parameter.constraints, capability)
            }),
        TypeRef::Associated(_) => false,
        TypeRef::Async(_) => false,
        TypeRef::Iterator(_) => matches!(
            capability,
            StdlibCapabilityId::Iterator
                | StdlibCapabilityId::Iterable
                | StdlibCapabilityId::Debug
                | StdlibCapabilityId::Display
        ),
        TypeRef::Application {
            constructor,
            arguments: [element],
        } => match library.capability(capability).behavior {
            CapabilityBehavior::StructuralEquality
                if constructor == StdlibTypeConstructorId::Option
                    || constructor == StdlibTypeConstructorId::Result =>
            {
                declared_type_has_capability(library, signature, *element, capability)
            }
            _ => false,
        },
        TypeRef::FixedArray { element, .. } => {
            library.capability(capability).behavior == CapabilityBehavior::StructuralMemoryLayout
                && declared_type_has_capability(library, signature, *element, capability)
        }
        TypeRef::Callable { .. } => false,
        TypeRef::Application { .. } => false,
    }
}

fn render_declared_type(library: &StandardLibrary, ty: TypeRef) -> String {
    library.render_type(ty)
}

fn render_actual_type(
    library: &StandardLibrary,
    semantics: &SemanticModel,
    ty: TypeId,
    parameter_bindings: &HashMap<TypeId, TypeRef>,
) -> String {
    match semantics.types().kind(ty) {
        TypeKind::Error => "<unknown>".to_owned(),
        TypeKind::Builtin(core) => core.to_string(),
        TypeKind::Standard(standard) => library.type_decl(*standard).name.to_owned(),
        TypeKind::StateSnapshot => "StateSnapshot".to_owned(),
        TypeKind::SettingsView => "SettingsView".to_owned(),
        TypeKind::Struct(structure) => format!("struct#{structure}"),
        TypeKind::Enum(enumeration) => format!("enum#{enumeration}"),
        TypeKind::ManagedClass(class) => format!("class#{class}"),
        TypeKind::ManagedReference(class) => format!("class#{class}.Ref"),
        TypeKind::GenericParameter { index, .. } => parameter_bindings
            .get(&ty)
            .map(|bound| render_declared_type(library, *bound))
            .unwrap_or_else(|| generic_parameter_name(*index)),
        TypeKind::Array {
            element, length, ..
        } => {
            let element = render_actual_type(library, semantics, *element, parameter_bindings);
            match length {
                Some(length) => format!("[{element}; {length}]"),
                None => format!("[{element}]"),
            }
        }
        TypeKind::Option { value, .. } => format!(
            "{}?",
            render_actual_type(library, semantics, *value, parameter_bindings)
        ),
        TypeKind::Result { value, .. } => format!(
            "{}!",
            render_actual_type(library, semantics, *value, parameter_bindings)
        ),
        TypeKind::Async { value, .. } => format!(
            "async {}",
            render_actual_type(library, semantics, *value, parameter_bindings)
        ),
        TypeKind::Iterator { item, .. } => format!(
            "iterator {}",
            render_actual_type(library, semantics, *item, parameter_bindings)
        ),
        TypeKind::Callable {
            parameters, result, ..
        } => {
            let parameters = parameters
                .iter()
                .map(|parameter| {
                    render_actual_type(library, semantics, *parameter, parameter_bindings)
                })
                .collect::<Vec<_>>()
                .join(", ");
            format!(
                "({parameters}) -> {}",
                render_actual_type(library, semantics, *result, parameter_bindings)
            )
        }
        TypeKind::Set { element, .. } => format!(
            "Set<{}>",
            render_actual_type(library, semantics, *element, parameter_bindings)
        ),
        TypeKind::Application {
            constructor,
            arguments,
            ..
        } => {
            let name = library.type_constructor(*constructor).name;
            let arguments = arguments
                .iter()
                .map(|argument| {
                    render_actual_type(library, semantics, *argument, parameter_bindings)
                })
                .collect::<Vec<_>>()
                .join(", ");
            format!("{name}<{arguments}>")
        }
        TypeKind::Range { bound, kind, .. } => {
            let bound = render_actual_type(library, semantics, *bound, parameter_bindings);
            format!("{bound}{}{bound}", kind.operator())
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::stdlib::{CoreTypeId, Parameter, ParameterRule, TypeParameter};

    fn checked(source: &str) -> crate::CheckedProgram {
        crate::check(crate::lower(crate::parse(source).expect("fixture parses")))
            .expect("fixture checks")
    }

    #[test]
    fn bundled_source_body_schemes_match_the_catalog() {
        let checked = checked("state \"game.exe\" {}");
        assert!(
            validate_signatures(
                &checked.context().standard_library(),
                checked.syntax(),
                checked.typed_hir(),
                checked.semantics(),
            )
            .is_empty()
        );
    }

    #[test]
    fn nested_generic_result_mismatches_are_reported_before_specialization() {
        static T_ARGUMENT: [TypeRef; 1] = [TypeRef::Parameter("T")];
        static RESULT_ARGUMENT: [TypeRef; 1] = [TypeRef::Application {
            constructor: StdlibTypeConstructorId::Result,
            arguments: &T_ARGUMENT,
        }];
        static TYPE_PARAMETERS: [TypeParameter; 1] = [TypeParameter {
            name: "T",
            constraints: &[],
        }];
        let checked =
            checked("state \"game.exe\" {}\nfn wrap(value) { return value.discardError() }");
        let function = checked
            .syntax()
            .functions
            .iter()
            .find(|function| function.name == "wrap")
            .expect("fixture function")
            .id;
        let signature = Signature {
            type_parameters: &TYPE_PARAMETERS,
            explicit_type_parameters: 0,
            parameters: &[],
            result_is_async: false,
            result: TypeRef::Application {
                constructor: StdlibTypeConstructorId::Option,
                arguments: &RESULT_ARGUMENT,
            },
        };
        let error = validate_signature(
            &checked.context().standard_library(),
            ItemKind::Method {
                receiver: TypeRef::Application {
                    constructor: StdlibTypeConstructorId::Result,
                    arguments: &T_ARGUMENT,
                },
            },
            signature,
            function,
            checked.semantics(),
        )
        .expect_err("nested result mismatch must fail");
        assert!(error.contains("result is declared as `T!?`"), "{error}");
        assert!(error.contains("body inferred `T?`"), "{error}");
    }

    #[test]
    fn more_general_inferred_array_templates_accept_declared_fixed_arrays() {
        static T: TypeRef = TypeRef::Parameter("T");
        static TYPE_PARAMETERS: [TypeParameter; 1] = [TypeParameter {
            name: "T",
            constraints: &[],
        }];
        static PARAMETERS: [Parameter; 1] = [Parameter {
            name: "values",
            ty: TypeRef::FixedArray {
                element: &T,
                length: 2,
            },
            rule: ParameterRule::Value,
            documentation: "",
        }];
        let checked = checked(
            "state \"game.exe\" {}\nfn count(values) { values = []; return values.length() }",
        );
        let function = checked
            .syntax()
            .functions
            .iter()
            .find(|function| function.name == "count")
            .expect("fixture function")
            .id;
        validate_signature(
            &checked.context().standard_library(),
            ItemKind::Function,
            Signature {
                type_parameters: &TYPE_PARAMETERS,
                explicit_type_parameters: 1,
                parameters: &PARAMETERS,
                result_is_async: false,
                result: TypeRef::Core(CoreTypeId::U32),
            },
            function,
            checked.semantics(),
        )
        .expect("an unbounded inferred array accepts the declared fixed-array calls");
    }

    #[test]
    fn concrete_inferred_parameters_cannot_narrow_declared_generics() {
        static T: TypeRef = TypeRef::Parameter("T");
        static TYPE_PARAMETERS: [TypeParameter; 1] = [TypeParameter {
            name: "T",
            constraints: &[],
        }];
        static PARAMETERS: [Parameter; 1] = [Parameter {
            name: "value",
            ty: T,
            rule: ParameterRule::Value,
            documentation: "",
        }];
        let checked =
            checked("state \"game.exe\" {}\nfn concrete(value: i32) -> i32 { return value }");
        let function = checked
            .syntax()
            .functions
            .iter()
            .find(|function| function.name == "concrete")
            .expect("fixture function")
            .id;
        let error = validate_signature(
            &checked.context().standard_library(),
            ItemKind::Function,
            Signature {
                type_parameters: &TYPE_PARAMETERS,
                explicit_type_parameters: 1,
                parameters: &PARAMETERS,
                result_is_async: false,
                result: T,
            },
            function,
            checked.semantics(),
        )
        .expect_err("a concrete body cannot implement all declared generic calls");
        assert!(error.contains("body narrowed"), "{error}");
    }

    #[test]
    fn inferred_capability_requirements_must_be_declared() {
        static T: TypeRef = TypeRef::Parameter("T");
        static TYPE_PARAMETERS: [TypeParameter; 1] = [TypeParameter {
            name: "T",
            constraints: &[],
        }];
        static PARAMETERS: [Parameter; 2] = [
            Parameter {
                name: "left",
                ty: T,
                rule: ParameterRule::Value,
                documentation: "",
            },
            Parameter {
                name: "right",
                ty: T,
                rule: ParameterRule::Value,
                documentation: "",
            },
        ];
        let checked =
            checked("state \"game.exe\" {}\nfn equal(left, right) { return left == right }");
        let function = checked
            .syntax()
            .functions
            .iter()
            .find(|function| function.name == "equal")
            .expect("fixture function")
            .id;
        let error = validate_signature(
            &checked.context().standard_library(),
            ItemKind::Function,
            Signature {
                type_parameters: &TYPE_PARAMETERS,
                explicit_type_parameters: 1,
                parameters: &PARAMETERS,
                result_is_async: false,
                result: TypeRef::Core(CoreTypeId::Bool),
            },
            function,
            checked.semantics(),
        )
        .expect_err("undeclared inferred constraints must fail");
        assert!(
            error.contains("does not guarantee the body's required `Equatable` capability"),
            "{error}"
        );

        static INTEGER_PARAMETERS: [TypeParameter; 1] = [TypeParameter {
            name: "T",
            constraints: &[StdlibCapabilityId::Integer],
        }];
        validate_signature(
            &checked.context().standard_library(),
            ItemKind::Function,
            Signature {
                type_parameters: &INTEGER_PARAMETERS,
                explicit_type_parameters: 1,
                parameters: &PARAMETERS,
                result_is_async: false,
                result: TypeRef::Core(CoreTypeId::Bool),
            },
            function,
            checked.semantics(),
        )
        .expect("Integer transitively guarantees the inferred Equatable requirement");
    }
}
