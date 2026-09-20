//! Injection boundary for ordinary SplitScript standard-library bodies.
//!
//! Bodies are authored in the privileged catalog source, but statements and
//! expressions are parsed only after they have been appended to an ordinary
//! compilation unit. Static library tokens are cached per graph, but one parser
//! still owns all syntax identities and constructed-type interning.

use crate::{
    Diagnostic,
    ast::{Expr, ExprKind, ManagedClassDecl, ManagedClassId, ManagedItemDecl, Program},
    lexer, parser,
    visit::{self, Visitor},
};

use super::{
    Implementation, ItemKind, ManagedRuntimeBackend, Signature, StandardLibrary, StdlibItem,
    TypeRef,
};

pub(crate) const RESERVED_FUNCTION_PREFIX: &str = "__splitscript_stdlib_";
pub(crate) const PROVIDER_PREPARATION_FUNCTION: &str =
    "__splitscript_stdlib_selected_provider_preparation";
pub(crate) const PROVIDER_BINDINGS_TYPE: &str = "__splitscript_stdlib_provider_bindings";
pub(crate) const MANAGED_POINTER_SIZE_FIELD: &str = "__pointer_size";
pub(crate) const MANAGED_LIST_LAYOUT_FIELD: &str = "__list_layout";
pub(crate) const MANAGED_OBJECT_TYPE_FIELD: &str = "__object_type";
pub(crate) const MANAGED_ARRAY_TYPE_FIELD: &str = "__array_type";
pub(crate) const MANAGED_MAP_READ_FIELD: &str = "__map_read";
pub(crate) const MANAGED_SET_READ_FIELD: &str = "__set_read";
pub(crate) const MANAGED_KEYED_VERIFY_FIELD: &str = "__keyed_verify";

struct SelectedProviderContext {
    index: usize,
    ty: &'static str,
    function_name: &'static str,
}

#[derive(Debug)]
pub(super) struct RenderedLibraryBodies {
    pub(super) source: String,
    pub(super) body_ranges: Vec<(std::ops::Range<usize>, &'static str)>,
    tokens: Result<Vec<lexer::Token>, Diagnostic>,
}

fn body_source(
    item: &StdlibItem,
    signature: Signature,
    function_name: &str,
    body: &str,
    library: &StandardLibrary,
) -> String {
    let mut parameters = Vec::new();
    if let ItemKind::Method { receiver } = item.kind {
        parameters.push(render_parameter("self", receiver, library));
    }
    parameters.extend(
        signature
            .parameters
            .iter()
            .map(|parameter| render_parameter(parameter.name, parameter.ty, library)),
    );
    let result = if contains_type_parameter(signature.result) {
        String::new()
    } else {
        format!(
            " -> {}{}",
            if signature.result_is_async {
                "async "
            } else {
                ""
            },
            signature.result.render(library)
        )
    };
    format!(
        "fn {}({}){} {}",
        function_name,
        parameters.join(", "),
        result,
        body,
    )
}

fn render_parameter(name: &str, ty: TypeRef, library: &StandardLibrary) -> String {
    if contains_type_parameter(ty) {
        name.to_owned()
    } else {
        format!("{name}: {}", ty.render(library))
    }
}

fn contains_type_parameter(ty: TypeRef) -> bool {
    match ty {
        TypeRef::Parameter(_) | TypeRef::Associated(_) => true,
        TypeRef::Async(value) | TypeRef::Iterator(value) => contains_type_parameter(*value),
        TypeRef::Application { arguments, .. } => {
            arguments.iter().copied().any(contains_type_parameter)
        }
        TypeRef::FixedArray { element, .. } => contains_type_parameter(*element),
        TypeRef::Callable { parameters, result } => {
            parameters.iter().copied().any(contains_type_parameter)
                || contains_type_parameter(*result)
        }
        TypeRef::Core(_) | TypeRef::Standard(_) => false,
    }
}

pub(crate) fn augment_program_with_library_bodies(
    user_source: &str,
    user_program: &Program,
    library: &StandardLibrary,
) -> Result<Option<Program>, Vec<Diagnostic>> {
    let rendered = library.rendered_library_bodies();
    let mut combined = String::with_capacity(user_source.len() + rendered.source.len() + 2);
    combined.push_str(user_source);
    combined.push('\n');
    let library_start = combined.len();
    combined.push_str(&rendered.source);
    let mut body_ranges = rendered
        .body_ranges
        .iter()
        .map(|(range, name)| {
            (
                library_start + range.start..library_start + range.end,
                *name,
            )
        })
        .collect::<Vec<_>>();
    if !rendered.source.is_empty() {
        combined.push('\n');
    }
    let has_library_bodies = !rendered.source.is_empty();
    if let Some(selected) = selected_provider_preparation(user_source, user_program, library) {
        let source = managed_preparation_source(
            user_program,
            selected.function_name,
            &selected.arguments.join(", "),
            selected.managed_backend,
            &selected.contexts,
        );
        let start = combined.len();
        combined.push_str(&source);
        body_ranges.push((
            start..combined.len(),
            "the selected state-provider preparation",
        ));
        combined.push('\n');
    } else if !has_library_bodies {
        return Ok(None);
    }
    let tokens =
        augmented_tokens(&combined, library_start, rendered).map_err(|error| vec![error])?;
    let mut program = parse_augmented_program(user_source, &combined, tokens, body_ranges)?;
    if let Some(program) = &mut program {
        specialize_il2cpp_preparation(program, library);
    }
    Ok(program)
}

// Keep a known-width state provider from retaining the opposite discovery path.
// This only selects an implementation; ordinary typing, constant-configuration
// validation, and runtime profile/target validation still apply unchanged.
fn specialize_il2cpp_preparation(program: &mut Program, library: &StandardLibrary) {
    let body_name = |name: &str| match library.item_by_name_including_private(name)?.implementation
    {
        Implementation::LibraryBody { function_name, .. } => Some(function_name),
        _ => None,
    };
    let Some(generic) = body_name("Unity.providerIl2cpp") else {
        return;
    };
    let Some(index) = program
        .functions
        .iter()
        .position(|f| f.name == PROVIDER_PREPARATION_FUNCTION)
    else {
        return;
    };
    let Some(crate::ast::Stmt::Variable(binding)) =
        program.functions[index].body.statements.first()
    else {
        return;
    };
    let Some(Expr {
        kind: ExprKind::Suspend { value, .. },
        ..
    }) = &binding.value
    else {
        return;
    };
    let ExprKind::Call {
        callee,
        receiver: None,
        args,
        ..
    } = &value.kind
    else {
        return;
    };
    if callee.as_slice() != [generic] || args.len() != 1 {
        return;
    }
    let Some(width) = literal_profile_width(&args[0], program, library, 0) else {
        return;
    };
    let Some(specialized) = body_name(if width == 8 {
        "Unity.providerIl2cpp64"
    } else {
        "Unity.providerIl2cpp32"
    }) else {
        return;
    };
    let crate::ast::Stmt::Variable(binding) = &mut program.functions[index].body.statements[0]
    else {
        unreachable!()
    };
    let ExprKind::Suspend { value, .. } = &mut binding.value.as_mut().unwrap().kind else {
        unreachable!()
    };
    let ExprKind::Call { callee, .. } = &mut value.kind else {
        unreachable!()
    };
    *callee = vec![specialized.into()];
}

// Literal profiles and zero-argument constructor aliases cover the built-in
// catalog and ordinary custom descriptors. More complex constant expressions
// conservatively keep the generic implementation rather than guessing a width.
fn literal_profile_width(
    expression: &Expr,
    program: &Program,
    library: &StandardLibrary,
    depth: usize,
) -> Option<u32> {
    if depth >= 64 {
        return None;
    }
    match &expression.kind {
        ExprKind::Struct { name, fields, .. } if name == "Il2CppProfile" => {
            let field = fields.iter().find(|field| field.name == "pointerSize")?;
            match &field.value.kind {
                ExprKind::Path(path) if path.as_slice() == ["PointerSize", "Bit64"] => Some(8),
                ExprKind::Path(path) if path.as_slice() == ["PointerSize", "Bit32"] => Some(4),
                _ => None,
            }
        }
        ExprKind::Call {
            callee,
            receiver: None,
            args,
            ..
        } if args.is_empty() => {
            let qualified = callee.join(".");
            let name = match library
                .item_by_name(&qualified)
                .map(|item| item.implementation)
            {
                Some(Implementation::LibraryBody { function_name, .. }) => function_name,
                None if callee.len() == 1 => &qualified,
                _ => return None,
            };
            let function = program
                .functions
                .iter()
                .find(|function| function.name == name)?;
            if !function.params.is_empty()
                || function.return_is_async
                || function.return_is_iterator
            {
                return None;
            }
            let [crate::ast::Stmt::Expression(body)] = function.body.statements.as_slice() else {
                return None;
            };
            let ExprKind::Return(Some(value)) = &body.kind else {
                return None;
            };
            literal_profile_width(value, program, library, depth + 1)
        }
        _ => None,
    }
}

pub(super) fn render_library_bodies(library: &StandardLibrary) -> RenderedLibraryBodies {
    let mut source = String::new();
    let mut body_ranges = Vec::new();
    for item in library.all_items() {
        let bodies = match item.implementation {
            Implementation::Intrinsic(_) | Implementation::CapabilityRequirement => Vec::new(),
            Implementation::LibraryBody {
                function_name,
                body,
            } => vec![(item.signature, function_name, body)],
            Implementation::LibraryOverloads { cases, .. } => cases
                .iter()
                .map(|case| (case.signature, case.function_name, case.body))
                .collect(),
        };
        for (signature, function_name, body) in bodies {
            let body = body_source(item, signature, function_name, body, library);
            let start = source.len();
            source.push_str(&body);
            body_ranges.push((start..source.len(), item.qualified_name));
            source.push('\n');
        }
    }
    if source.ends_with('\n') {
        source.pop();
    }
    let tokens = lex_tokens(&source);
    RenderedLibraryBodies {
        source,
        body_ranges,
        tokens,
    }
}

fn lex_tokens(source: &str) -> Result<Vec<lexer::Token>, Diagnostic> {
    Ok(lexer::lex_lossless(source)?
        .into_lexemes()
        .into_iter()
        .filter_map(|lexeme| match lexeme {
            lexer::Lexeme::Token(token) => Some(token),
            lexer::Lexeme::Trivia(_) => None,
        })
        .collect())
}

fn augmented_tokens(
    combined: &str,
    library_start: usize,
    rendered: &RenderedLibraryBodies,
) -> Result<Vec<lexer::Token>, Diagnostic> {
    let library_end = library_start + rendered.source.len();
    // Normally both generated boundaries are separated by newlines and the
    // user prefix has already passed strict parsing. Preserve whole-source
    // lexical recovery/error behavior if any fragment cannot be lexed alone.
    let (Ok(library), Ok(mut tokens), Ok(suffix)) = (
        &rendered.tokens,
        lex_tokens(&combined[..library_start]),
        lex_tokens(&combined[library_end..]),
    ) else {
        return lex_tokens(combined);
    };
    tokens.pop(); // Only the suffix contributes the combined EOF.
    tokens.reserve(library.len() - 1 + suffix.len());
    tokens.extend(library[..library.len() - 1].iter().map(|token| {
        let mut token = token.clone();
        token.span.start += library_start;
        token.span.end += library_start;
        token
    }));
    tokens.extend(suffix.into_iter().map(|mut token| {
        token.span.start += library_end;
        token.span.end += library_end;
        token
    }));
    Ok(tokens)
}

fn parse_augmented_program(
    user_source: &str,
    combined: &str,
    tokens: Vec<lexer::Token>,
    body_ranges: Vec<(std::ops::Range<usize>, &'static str)>,
) -> Result<Option<Program>, Vec<Diagnostic>> {
    let output = parser::parse_recovering(combined, tokens);
    if !output
        .diagnostics
        .iter()
        .any(|diagnostic| diagnostic.severity == crate::DiagnosticSeverity::Error)
    {
        Ok(Some(output.program))
    } else {
        Err(output
            .diagnostics
            .into_iter()
            .map(|mut diagnostic| {
                let origin = if diagnostic.span.start <= user_source.len() {
                    "the already-parsed user program"
                } else {
                    body_ranges
                        .iter()
                        .find(|(range, _)| range.contains(&diagnostic.span.start))
                        .map_or("a generated standard-library boundary", |(_, name)| *name)
                };
                diagnostic.notes.push(format!(
                    "combined standard-library parsing failed inside {origin}"
                ));
                diagnostic
            })
            .collect())
    }
}

struct SchemaClass<'ast> {
    image: &'ast str,
    namespace: Vec<&'ast str>,
    class: &'ast ManagedClassDecl,
}

fn managed_preparation_source(
    program: &Program,
    preparation: &str,
    arguments: &str,
    managed_backend: Option<ManagedRuntimeBackend>,
    contexts: &[SelectedProviderContext],
) -> String {
    let classes = schema_classes(program);
    let instance_classes = managed_instance_classes(program);
    if classes.is_empty() && contexts.is_empty() {
        return format!(
            "fn {PROVIDER_PREPARATION_FUNCTION}() {{ return await {preparation}({arguments}) }}"
        );
    }

    let mut source = format!("struct {PROVIDER_BINDINGS_TYPE} {{\n");
    if !classes.is_empty() {
        source.push_str(&format!("    {MANAGED_POINTER_SIZE_FIELD}: u32,\n"));
        source.push_str(&format!("    {MANAGED_OBJECT_TYPE_FIELD}: (address, address, address, ManagedReadContext) -> address!,\n"));
        source.push_str(&format!(
            "    {MANAGED_LIST_LAYOUT_FIELD}: (address, u32, u32, u32, address, ManagedReadContext) -> UnityListLayout!,\n"
        ));
        source.push_str(&format!(
            "    {MANAGED_ARRAY_TYPE_FIELD}: (address, u32, u32, u32, ManagedReadContext) -> address!,\n"
        ));
    }
    if !classes.is_empty() {
        source.push_str(&format!(
            "    {MANAGED_MAP_READ_FIELD}: (address, u32, u32, u32, u32, u32, u32, u32, u32, u64, ManagedReadContext) -> UnityKeyedRead!,\n    {MANAGED_SET_READ_FIELD}: (address, u32, u32, u32, u32, u32, u32, u32, u32, u64, ManagedReadContext) -> UnityKeyedRead!,\n"
        ));
        source.push_str(&format!(
            "    {MANAGED_KEYED_VERIFY_FIELD}: (UnityKeyedRead) -> bool!,\n"
        ));
    }
    if !classes.is_empty() {
        source.push_str(&format!(
            "    {MANAGED_ARRAY_TYPE_FIELD}_class: (address, u32, u32, u32, address, ManagedReadContext) -> address!,\n\
                 {MANAGED_LIST_LAYOUT_FIELD}_class: (address, u32, u32, u32, address, address, ManagedReadContext) -> UnityListLayout!,\n\
                 {MANAGED_MAP_READ_FIELD}_class: (address, u32, u32, u32, u32, u32, u32, address, address, u32, u32, u64, ManagedReadContext) -> UnityKeyedRead!,\n\
                 {MANAGED_SET_READ_FIELD}_class: (address, u32, u32, u32, u32, u32, u32, address, address, u32, u32, u64, ManagedReadContext) -> UnityKeyedRead!,\n"
        ));
    }
    if !classes.is_empty() {
        let checker = "(address, u32, ManagedReadContext) -> bool!";
        for (name, signature) in schema_binding_signatures() {
            source.push_str(&format!("    {name}: {signature},\n"));
        }
        source.push_str(&format!(
            "    {MANAGED_ARRAY_TYPE_FIELD}_schema: (address, u32, u32, u32, {checker}, ManagedReadContext) -> address!,\n\
                 {MANAGED_LIST_LAYOUT_FIELD}_schema: (address, u32, u32, u32, address, {checker}, ManagedReadContext) -> UnityListLayout!,\n\
                 {MANAGED_MAP_READ_FIELD}_schema: (address, u32, u32, u32, u32, u32, u32, {checker}, {checker}, u32, u32, u64, ManagedReadContext) -> UnityKeyedRead!,\n\
                 {MANAGED_SET_READ_FIELD}_schema: (address, u32, u32, u32, u32, u32, u32, {checker}, {checker}, u32, u32, u64, ManagedReadContext) -> UnityKeyedRead!,\n"
        ));
    }
    for context in contexts {
        source.push_str(&format!(
            "    {}: {},\n",
            provider_context_field_name(context.index),
            context.ty
        ));
    }
    for class in &classes {
        source.push_str(&format!(
            "    {}: address,\n",
            managed_class_address_name(class.class.id.index())
        ));
        if instance_classes.contains(&class.class.id) {
            source.push_str(&format!(
                "    {}: address,\n",
                managed_instance_header_name(class.class.id.index())
            ));
        }
        for field in class_fields(class.class) {
            if field.is_static {
                source.push_str(&format!(
                    "    {}: address,\n",
                    managed_static_field_address_name(field.id.index())
                ));
            } else {
                source.push_str(&format!(
                    "    {}: u32,\n",
                    managed_field_offset_name(field.id.index())
                ));
            }
        }
        for field in class
            .class
            .conditional_fields
            .iter()
            .flat_map(|group| &group.fields)
        {
            source.push_str(&format!(
                "    {}: bool,\n",
                managed_field_presence_name(field.id.index())
            ));
        }
    }
    source.push_str("}\n");
    source.push_str(&format!("fn {PROVIDER_PREPARATION_FUNCTION}() {{\n"));
    if !classes.is_empty() {
        source.push_str(&format!(
            "    let __runtime = await {preparation}({arguments})\n"
        ));
    }
    for context in contexts {
        source.push_str(&format!(
            "    let {} = await {}()\n",
            provider_context_field_name(context.index),
            context.function_name,
        ));
    }
    if classes.is_empty() {
        source.push_str(&format!("    return {PROVIDER_BINDINGS_TYPE} {{\n"));
        push_provider_context_initializers(&mut source, contexts, "        ");
        source.push_str("    }\n");
        source.push_str("}\n");
        return source;
    }
    match managed_backend {
        Some(ManagedRuntimeBackend::Il2Cpp) => {
            source.push_str("    let __module = __runtime\n");
            source.push_str("    return {\n");
            source.push_str(&managed_backend_binding_source(
                &classes,
                &instance_classes,
                "__module",
                "__module.pointerSize",
                false,
                ".address",
                contexts,
            ));
            source.push_str("    }\n");
        }
        Some(ManagedRuntimeBackend::Mono) => {
            source.push_str("    let __module = __runtime\n");
            source.push_str("    return {\n");
            source.push_str(&managed_backend_binding_source(
                &classes,
                &instance_classes,
                "__module",
                "match __module.pointerSize { PointerSize.Bit32 => 4, PointerSize.Bit64 => 8 }",
                true,
                ".address",
                contexts,
            ));
            source.push_str("    }\n");
        }
        None => {
            source.push_str("    return {\n");
            source.push_str(&managed_backend_binding_source(
                &classes,
                &instance_classes,
                "__runtime",
                "__runtime.pointerBytes()",
                true,
                ".classAddress()",
                contexts,
            ));
            source.push_str("    }\n");
        }
    }
    source.push_str("}\n");
    source
}

fn managed_backend_binding_source(
    classes: &[SchemaClass<'_>],
    instance_classes: &std::collections::HashSet<ManagedClassId>,
    module: &str,
    pointer_size: &str,
    instance_header_is_async: bool,
    class_address: &str,
    contexts: &[SelectedProviderContext],
) -> String {
    let mut source = String::new();
    source.push_str(&format!(
        "            let {MANAGED_POINTER_SIZE_FIELD}: u32 = {pointer_size}\n"
    ));
    source.push_str(&format!(
        "            let __object_type_cache: [[address; 2]] = []\n\
                     let {MANAGED_OBJECT_TYPE_FIELD}: (address, address, address, ManagedReadContext) -> address! = (object, expected, original, context) => {{\n\
                         let charge = () => Unity.chargeManagedWork(context)\n\
                         let class = {module}.objectClass(object, charge)?\n\
                         if original != 0 {{ if class != original {{ throw \"managed snapshot runtime class changed during the read\" }} return class }}\n\
                         if expected == 0 {{ throw \"managed snapshot declared class is unavailable\" }}\n\
                         if class == expected {{ return class }}\n\
                         for cached in __object_type_cache {{ if !charge() {{ throw \"managed read work limit exceeded\" }} if cached[0] == class && cached[1] == expected {{ return class }} }}\n\
                         let current = class\n\
                         let depth: u32 = 0\n\
                         while current != expected {{\n\
                             if current == 0 {{ throw \"managed snapshot object is incompatible with its declared class\" }}\n\
                             if depth >= 128 {{ throw \"managed snapshot class hierarchy is cyclic or exceeds the depth limit\" }}\n\
                             current = {module}.parentClass(current, charge)?\n\
                             depth += 1\n\
                         }}\n\
                         if {module}.objectClass(object, charge)? != class {{ throw \"managed snapshot runtime class changed during discovery\" }}\n\
                         if __object_type_cache.length() >= 1024 {{ __object_type_cache.clear() }}\n\
                         __object_type_cache.push([class, expected])\n\
                         return class\n\
                     }}\n"
    ));
    source.push_str(&format!(
        "            let __class_contract_cache: [[address; 2]] = []\n\
                     let __class_contract: (address, address, ManagedReadContext) -> bool! = (type, expected, context) => {{\n\
                         let charge = () => Unity.chargeManagedWork(context)\n\
                         if expected == 0 {{ throw \"managed collection declared class is unavailable\" }}\n\
                         for cached in __class_contract_cache {{ if !charge() {{ throw \"managed read work limit exceeded\" }} if cached[0] == type && cached[1] == expected {{ return true }} }}\n\
                         let current = {module}.typeClass(type, charge)?\n\
                         let depth: u32 = 0\n\
                         while current != expected {{\n\
                             if current == 0 {{ throw \"managed collection element class is incompatible with its schema\" }}\n\
                             if depth >= 128 {{ throw \"managed collection class hierarchy is cyclic or exceeds the depth limit\" }}\n\
                             current = {module}.parentClass(current, charge)?\n\
                             depth += 1\n\
                         }}\n\
                         if __class_contract_cache.length() >= 1024 {{ __class_contract_cache.clear() }}\n\
                         __class_contract_cache.push([type, expected])\n\
                         return true\n\
                     }}\n"
    ));
    source.push_str(&schema_binding_bodies(module));
    source.push_str("            let __array_layout_cache: [UnityArrayLayout] = []\n");
    for mode in ["", "_class", "_schema"] {
        let nominal = mode == "_class";
        let schema = mode == "_schema";
        let binding = format!("{MANAGED_ARRAY_TYPE_FIELD}{mode}");
        let class_parameter = if schema {
            "(address, u32, ManagedReadContext) -> bool!, "
        } else if nominal {
            "address, "
        } else {
            ""
        };
        let class_argument = if schema {
            "verify, "
        } else if nominal {
            "elementClass, "
        } else {
            ""
        };
        let cached_check = if schema {
            "cached.validate(depth, elementBytes, elementKinds)?; verify(cached.elementType(depth)?, __pointer_size, context)?; return class"
        } else if nominal {
            "cached.validate(depth, elementBytes, elementKinds)?; __class_contract(cached.elementType(depth)?, elementClass, context)?; return class"
        } else {
            "return cached.validate(depth, elementBytes, elementKinds)"
        };
        let layout_check = if schema {
            "verify(layout.elementType(depth)?, __pointer_size, context)?;"
        } else if nominal {
            "__class_contract(layout.elementType(depth)?, elementClass, context)?;"
        } else {
            ""
        };
        source.push_str(&format!(
        "            let {binding}: (address, u32, u32, u32, {class_parameter}ManagedReadContext) -> address! = (object, depth, elementBytes, elementKinds, {class_argument}context) => {{\n\
                         let charge = () => Unity.chargeManagedWork(context)\n\
                         let class = {module}.objectClass(object, charge)?\n\
                         for cached in __array_layout_cache {{ if !charge() {{ throw \"managed read work limit exceeded\" }} if cached.runtimeClass == class {{ {cached_check} }} }}\n\
                         let layout = {module}.arrayLayout(class, charge)?\n\
                         if {module}.objectClass(object, charge)? != class {{ throw \"managed array class changed during discovery\" }}\n\
                         layout.validate(depth, elementBytes, elementKinds)?\n\
                         {layout_check}\n\
                         if __array_layout_cache.length() >= 1024 {{ __array_layout_cache.clear() }}\n\
                         __array_layout_cache.push(layout)\n\
                         return class\n\
                     }}\n"
    ));
    }
    source.push_str("            let __list_layout_cache: [UnityListLayout] = []\n");
    for mode in ["", "_class", "_schema"] {
        let nominal = mode == "_class";
        let schema = mode == "_schema";
        let binding = format!("{MANAGED_LIST_LAYOUT_FIELD}{mode}");
        let class_parameter = if schema {
            "(address, u32, ManagedReadContext) -> bool!, "
        } else if nominal {
            "address, "
        } else {
            ""
        };
        let class_argument = if schema {
            "verify, "
        } else if nominal {
            "elementClass, "
        } else {
            ""
        };
        let cached_check = if schema {
            "verify(cached.elements.elementType(depth)?, __pointer_size, context)?;"
        } else if nominal {
            "__class_contract(cached.elements.elementType(depth)?, elementClass, context)?;"
        } else {
            ""
        };
        let layout_check = if schema {
            "verify(layout.elements.elementType(depth)?, __pointer_size, context)?;"
        } else if nominal {
            "__class_contract(layout.elements.elementType(depth)?, elementClass, context)?;"
        } else {
            ""
        };
        source.push_str(&format!(
        "            let {binding}: (address, u32, u32, u32, address, {class_parameter}ManagedReadContext) -> UnityListLayout! = (object, depth, elementBytes, elementKinds, expectedClass, {class_argument}context) => {{\n\
                         if !Unity.chargeManagedWork(context) {{ throw \"managed read work limit exceeded\" }}\n\
                         let class = {module}.collectionClass(object)?\n\
                         if expectedClass != 0 && class != expectedClass {{ throw \"managed list class changed during the read\" }}\n\
                         for cached in __list_layout_cache {{ if !Unity.chargeManagedWork(context) {{ throw \"managed read work limit exceeded\" }} if cached.runtimeClass == class {{ cached.elements.validate(depth, elementBytes, elementKinds)?; {cached_check} return cached }} }}\n\
                         let layout = {module}.listLayout(object, () => Unity.chargeManagedWork(context))?\n\
                         if layout.runtimeClass != class {{ throw \"managed list class changed during discovery\" }}\n\
                         layout.elements.validate(depth, elementBytes, elementKinds)?\n\
                         {layout_check}\n\
                         if __list_layout_cache.length() >= 1024 {{ __list_layout_cache.clear() }}\n\
                         __list_layout_cache.push(layout)\n\
                         return layout\n\
                     }}\n"
    ));
    }
    source.push_str(&format!(
        "            let __keyed_array_cache: [[address; 2]] = []\n\
                     let __keyed_array: (address, address, ManagedReadContext) -> address! = (object, declared, context) => {{\n\
                         let charge = () => Unity.chargeManagedWork(context)\n\
                         let class = {module}.objectClass(object, charge)?\n\
                         for cached in __keyed_array_cache {{ if !charge() {{ throw \"managed read work limit exceeded\" }} if cached[0] == class && cached[1] == declared {{ return class }} }}\n\
                         {module}.verifyArrayType(class, declared, charge)?\n\
                         if {module}.objectClass(object, charge)? != class {{ throw \"Unity backing array class changed during discovery\" }}\n\
                         if __keyed_array_cache.length() >= 1024 {{ __keyed_array_cache.clear() }}\n\
                         __keyed_array_cache.push([class, declared])\n\
                         return class\n\
                     }}\n"
    ));
    for (base, cache, method, noun) in [
        (
            MANAGED_MAP_READ_FIELD,
            "__map_layout_cache",
            "dictionaryLayout",
            "dictionary",
        ),
        (
            MANAGED_SET_READ_FIELD,
            "__set_layout_cache",
            "setLayout",
            "set",
        ),
    ] {
        source.push_str(&format!(
            "            let {cache}: [UnityKeyedLayout] = []\n"
        ));
        for mode in ["", "_class", "_schema"] {
            let nominal = mode == "_class";
            let schema = mode == "_schema";
            let binding = format!("{base}{mode}");
            let class_parameter = if schema {
                "(address, u32, ManagedReadContext) -> bool!, (address, u32, ManagedReadContext) -> bool!, "
            } else if nominal {
                "address, address, "
            } else {
                ""
            };
            let class_argument = if schema {
                "verifyKey, verifyValue, "
            } else if nominal {
                "keyClass, valueClass, "
            } else {
                ""
            };
            let cached_check = if schema {
                "if cached.members.length() == 4 { verifyKey(cached.members[2].nestedType(keyDepth, cached.keyElements)?, __pointer_size, context)? } verifyValue(cached.members[cached.members.length() - 1].nestedType(valueDepth, cached.valueElements)?, __pointer_size, context)?;"
            } else if nominal {
                "cached.validateClasses(keyDepth, valueDepth, keyClass, valueClass, (type, expected) => __class_contract(type, expected, context))?;"
            } else {
                ""
            };
            let layout_check = if schema {
                "if layout.members.length() == 4 { verifyKey(layout.members[2].nestedType(keyDepth, layout.keyElements)?, __pointer_size, context)? } verifyValue(layout.members[layout.members.length() - 1].nestedType(valueDepth, layout.valueElements)?, __pointer_size, context)?;"
            } else if nominal {
                "layout.validateClasses(keyDepth, valueDepth, keyClass, valueClass, (type, expected) => __class_contract(type, expected, context))?;"
            } else {
                ""
            };
            source.push_str(&format!(
        "            let {binding}: (address, u32, u32, u32, u32, u32, u32, {class_parameter}u32, u32, u64, ManagedReadContext) -> UnityKeyedRead! = (object, keyLeafBytes, valueLeafBytes, keyLeafKinds, valueLeafKinds, keyDepth, valueDepth, {class_argument}scanBudget, elementBudget, byteBudget, context) => {{\n\
                         if !Unity.chargeManagedWork(context) {{ throw \"managed read work limit exceeded\" }}\n\
                         let class = {module}.collectionClass(object)?\n\
                         let keyBytes = if keyDepth == 0 {{ keyLeafBytes }} else {{ {MANAGED_POINTER_SIZE_FIELD} }}\n\
                         let valueBytes = if valueDepth == 0 {{ valueLeafBytes }} else {{ {MANAGED_POINTER_SIZE_FIELD} }}\n\
                         let keyKinds = if keyDepth == 0 {{ keyLeafKinds }} else {{ 1 << 0x1d }}\n\
                         let valueKinds = if valueDepth == 0 {{ valueLeafKinds }} else {{ 1 << 0x1d }}\n\
                         for cached in {cache} {{ if !Unity.chargeManagedWork(context) {{ throw \"managed read work limit exceeded\" }} if cached.runtimeClass == class {{ cached.validateTypes(keyDepth, keyLeafBytes, keyLeafKinds, valueDepth, valueLeafBytes, valueLeafKinds)?; {cached_check} return cached.readSlots(object, keyBytes, valueBytes, keyKinds, valueKinds, scanBudget, elementBudget, byteBudget, (array, declared) => __keyed_array(array, declared, context)) }} }}\n\
                         let layout = {module}.{method}(object, () => Unity.chargeManagedWork(context))?\n\
                         if layout.runtimeClass != class {{ throw \"managed {noun} class changed during discovery\" }}\n\
                         layout.validateTypes(keyDepth, keyLeafBytes, keyLeafKinds, valueDepth, valueLeafBytes, valueLeafKinds)?\n\
                         {layout_check}\n\
                         if {cache}.length() >= 1024 {{ {cache}.clear() }}\n\
                         {cache}.push(layout)\n\
                         return layout.readSlots(object, keyBytes, valueBytes, keyKinds, valueKinds, scanBudget, elementBudget, byteBudget, (array, declared) => __keyed_array(array, declared, context))\n\
                     }}\n\
"
    ));
        }
    }
    source.push_str(&format!("            let {MANAGED_KEYED_VERIFY_FIELD}: (UnityKeyedRead) -> bool! = read => read.verify()\n"));
    let mut images = std::collections::HashMap::new();
    for class in classes {
        let image_index = if let Some(index) = images.get(class.image) {
            *index
        } else {
            let index = images.len();
            source.push_str(&format!(
                "            let __image_{index} = await {module}.image({:?})\n",
                class.image
            ));
            images.insert(class.image, index);
            index
        };
        let candidates = class
            .class
            .metadata_name_candidates()
            .map(|(name, _)| {
                class
                    .namespace
                    .iter()
                    .copied()
                    .chain(std::iter::once(name))
                    .collect::<Vec<_>>()
                    .join(".")
            })
            .map(|name| format!("{name:?}"))
            .collect::<Vec<_>>()
            .join(", ");
        let class_local = format!("__class_{}", class.class.id.index());
        let class_lookup = if class
            .class
            .metadata_name_candidates()
            .any(|(name, _)| name.contains('+'))
        {
            "classAny"
        } else {
            "classAnyFlat"
        };
        source.push_str(&format!(
            "            let {class_local} = await __image_{image_index}.{class_lookup}([{candidates}])\n"
        ));
        source.push_str(&format!(
            "            let {} = {class_local}{class_address}\n",
            managed_class_address_name(class.class.id.index())
        ));
        if instance_classes.contains(&class.class.id) {
            let await_prefix = if instance_header_is_async {
                "await "
            } else {
                ""
            };
            source.push_str(&format!(
                "            let {} = {await_prefix}{class_local}.instanceHeader()\n",
                managed_instance_header_name(class.class.id.index())
            ));
        }
        for field in &class.class.fields {
            push_required_managed_field_binding(&mut source, &class_local, field);
        }
        for group in &class.class.conditional_fields {
            for field in &group.fields {
                push_optional_managed_field_binding(&mut source, &class_local, field);
            }
        }
    }
    source.push_str(&format!("            {PROVIDER_BINDINGS_TYPE} {{\n"));
    push_provider_context_initializers(&mut source, contexts, "                ");
    source.push_str(&format!("                {MANAGED_POINTER_SIZE_FIELD},\n"));
    source.push_str(&format!("                {MANAGED_LIST_LAYOUT_FIELD},\n"));
    source.push_str(&format!("                {MANAGED_ARRAY_TYPE_FIELD},\n"));
    source.push_str(&format!("                {MANAGED_OBJECT_TYPE_FIELD},\n"));
    source.push_str(&format!(
        "                {MANAGED_MAP_READ_FIELD}, {MANAGED_SET_READ_FIELD}, {MANAGED_KEYED_VERIFY_FIELD},\n"
    ));
    for base in [
        MANAGED_ARRAY_TYPE_FIELD,
        MANAGED_LIST_LAYOUT_FIELD,
        MANAGED_MAP_READ_FIELD,
        MANAGED_SET_READ_FIELD,
    ] {
        source.push_str(&format!("                {base}_class, {base}_schema,\n"));
    }
    for (name, _) in schema_binding_signatures() {
        source.push_str(&format!("                {name},\n"));
    }
    for class in classes {
        source.push_str(&format!(
            "                {},\n",
            managed_class_address_name(class.class.id.index())
        ));
        if instance_classes.contains(&class.class.id) {
            let name = managed_instance_header_name(class.class.id.index());
            source.push_str(&format!("                {name},\n"));
        }
        for field in class_fields(class.class) {
            let name = if field.is_static {
                managed_static_field_address_name(field.id.index())
            } else {
                managed_field_offset_name(field.id.index())
            };
            source.push_str(&format!("                {name},\n"));
        }
        for field in class
            .class
            .conditional_fields
            .iter()
            .flat_map(|group| &group.fields)
        {
            let name = managed_field_presence_name(field.id.index());
            source.push_str(&format!("                {name},\n"));
        }
    }
    source.push_str("            }\n");
    source
}

fn push_provider_context_initializers(
    source: &mut String,
    contexts: &[SelectedProviderContext],
    indentation: &str,
) {
    for context in contexts {
        let name = provider_context_field_name(context.index);
        source.push_str(&format!("{indentation}{name},\n"));
    }
}

fn managed_field_candidates(field: &crate::ast::ManagedFieldDecl) -> String {
    field
        .binding_name_candidates()
        .into_iter()
        .map(|(name, _, _)| format!("{name:?}"))
        .collect::<Vec<_>>()
        .join(", ")
}

fn push_required_managed_field_binding(
    source: &mut String,
    class_local: &str,
    field: &crate::ast::ManagedFieldDecl,
) {
    let candidates = managed_field_candidates(field);
    if field.is_static {
        let address = managed_static_field_address_name(field.id.index());
        source.push_str(&format!(
            "            let {address} = await {class_local}.staticFieldAny([{candidates}])\n"
        ));
    } else {
        let offset = managed_field_offset_name(field.id.index());
        source.push_str(&format!(
            "            let {offset} = (await {class_local}.fieldAny([{candidates}])).offset\n"
        ));
    }
}

fn push_optional_managed_field_binding(
    source: &mut String,
    class_local: &str,
    field: &crate::ast::ManagedFieldDecl,
) {
    let candidates = managed_field_candidates(field);
    let probe = format!("__field_{}_conditional_probe", field.id.index());
    if field.is_static {
        let address = managed_static_field_address_name(field.id.index());
        source.push_str(&format!(
            "            let {probe} = await {class_local}.probeStaticFieldAny([{candidates}])\n"
        ));
        source.push_str(&format!(
            "            let {address}: address = match {probe} {{ Some(address) => address, None => 0 }}\n"
        ));
    } else {
        let offset = managed_field_offset_name(field.id.index());
        source.push_str(&format!(
            "            let {probe} = await {class_local}.probeFieldAny([{candidates}])\n"
        ));
        source.push_str(&format!(
            "            let {offset}: u32 = match {probe} {{ Some(field) => field.offset, None => 0 }}\n"
        ));
    }
    let present = managed_field_presence_name(field.id.index());
    source.push_str(&format!(
        "            let {present} = match {probe} {{ Some(_) => true, None => false }}\n"
    ));
}

fn schema_classes(program: &Program) -> Vec<SchemaClass<'_>> {
    let mut classes = Vec::new();
    for image in &program.managed_images {
        collect_schema_classes(&mut classes, &image.name, &[], &image.items);
    }
    classes
}

fn collect_schema_classes<'ast>(
    output: &mut Vec<SchemaClass<'ast>>,
    image: &'ast str,
    namespace: &[&'ast str],
    items: &'ast [ManagedItemDecl],
) {
    for item in items {
        match item {
            ManagedItemDecl::Namespace(item) => {
                let mut nested = namespace.to_vec();
                nested.push(&item.name);
                collect_schema_classes(output, image, &nested, &item.items);
            }
            ManagedItemDecl::Class(class) => output.push(SchemaClass {
                image,
                namespace: namespace.to_vec(),
                class,
            }),
        }
    }
}

fn class_fields(class: &ManagedClassDecl) -> impl Iterator<Item = &crate::ast::ManagedFieldDecl> {
    class.fields.iter().chain(
        class
            .conditional_fields
            .iter()
            .flat_map(|group| &group.fields),
    )
}

pub(crate) fn managed_field_offset_name(field: usize) -> String {
    format!("__field_{field}_offset")
}

pub(crate) fn managed_field_presence_name(field: usize) -> String {
    format!("__field_{field}_present")
}

pub(crate) fn managed_static_field_address_name(field: usize) -> String {
    format!("__field_{field}_static_address")
}

pub(crate) fn managed_class_address_name(class: usize) -> String {
    format!("__class_{class}_address")
}

pub(crate) fn managed_instance_header_name(class: usize) -> String {
    format!("__class_{class}_instance_header")
}

pub(crate) fn provider_context_field_name(context: usize) -> String {
    format!("__provider_context_{context}")
}

fn managed_instance_classes(program: &Program) -> std::collections::HashSet<ManagedClassId> {
    struct Collector<'a> {
        classes: &'a [ManagedClassDecl],
        program: &'a Program,
        found: std::collections::HashSet<ManagedClassId>,
    }

    impl<'ast> Visitor<'ast> for Collector<'_> {
        fn visit_expr(&mut self, expression: &'ast Expr) {
            if let ExprKind::Call {
                callee,
                receiver,
                type_arguments,
                ..
            } = &expression.kind
            {
                if receiver.is_none()
                    && let [class_name, method] = callee.as_slice()
                    && method == "instances"
                    && let Some(class) = self.classes.iter().find(|class| class.name == *class_name)
                {
                    self.found.insert(class.id);
                }
                if (receiver.is_some() || callee.len() > 1)
                    && callee.last().is_some_and(|method| method == "component")
                    && let [crate::ast::TypeRef::Named(name)] = type_arguments.as_slice()
                    && let Some(class) = self
                        .classes
                        .iter()
                        .find(|class| class.name == self.program.type_name(*name))
                {
                    self.found.insert(class.id);
                }
            }
            visit::walk_expr(self, expression);
        }
    }

    let classes = program
        .managed_class_declarations()
        .into_iter()
        .cloned()
        .collect::<Vec<_>>();
    let mut collector = Collector {
        classes: &classes,
        program,
        found: std::collections::HashSet::new(),
    };
    collector.visit_program(program);
    collector.found
}

struct SelectedProviderPreparation<'source> {
    function_name: &'static str,
    arguments: Vec<&'source str>,
    managed_backend: Option<ManagedRuntimeBackend>,
    contexts: Vec<SelectedProviderContext>,
}

fn selected_provider_preparation<'source>(
    user_source: &'source str,
    user_program: &Program,
    library: &StandardLibrary,
) -> Option<SelectedProviderPreparation<'source>> {
    let state = user_program.state.as_ref()?;
    let provider = state
        .provider
        .as_ref()
        .and_then(|reference| library.state_provider_by_name(&reference.name))
        .or_else(|| {
            state
                .provider
                .is_none()
                .then(|| library.default_state_provider())
                .flatten()
        })?;
    let (preparation, arguments, managed_backend) = if let Some(reference) = &state.provider
        && let Some(selected) = &reference.selector
    {
        let selector = provider
            .selectors
            .iter()
            .find(|candidate| candidate.name == selected.name)?;
        (
            selector.preparation,
            selected
                .arguments
                .iter()
                .map(|argument| &user_source[argument.span.start..argument.span.end])
                .collect(),
            selector.managed_backend,
        )
    } else {
        (provider.preparation?, Vec::new(), None)
    };
    let item = library.item(preparation);
    let Implementation::LibraryBody { function_name, .. } = item.implementation else {
        return None;
    };
    let referenced_contexts = provider_contexts_used(user_program, provider);
    let contexts = provider
        .contexts
        .iter()
        .enumerate()
        .filter(|(index, _)| referenced_contexts.contains(index))
        .map(|(index, context)| {
            let item = library.item(context.preparation);
            let Implementation::LibraryBody { function_name, .. } = item.implementation else {
                unreachable!("validated provider context preparations have source bodies")
            };
            SelectedProviderContext {
                index,
                ty: library.type_decl(context.ty).name,
                function_name,
            }
        })
        .collect();
    Some(SelectedProviderPreparation {
        function_name,
        arguments,
        managed_backend,
        contexts,
    })
}

fn provider_contexts_used(
    program: &Program,
    provider: &crate::stdlib::StdlibStateProvider,
) -> std::collections::HashSet<usize> {
    struct Collector<'a> {
        provider: &'a crate::stdlib::StdlibStateProvider,
        found: std::collections::HashSet<usize>,
    }

    impl<'ast> Visitor<'ast> for Collector<'_> {
        fn visit_expr(&mut self, expression: &'ast Expr) {
            let root = match &expression.kind {
                ExprKind::Path(path) => path.first(),
                ExprKind::Call {
                    callee,
                    receiver: None,
                    ..
                } => callee.first(),
                _ => None,
            };
            if let Some(root) = root
                && let Some((index, _)) = self
                    .provider
                    .contexts
                    .iter()
                    .enumerate()
                    .find(|(_, context)| context.name == root)
            {
                self.found.insert(index);
            }
            visit::walk_expr(self, expression);
        }
    }

    let mut collector = Collector {
        provider,
        found: std::collections::HashSet::new(),
    };
    collector.visit_program(program);
    collector.found
}

fn schema_binding_signatures() -> Vec<(&'static str, String)> {
    let checker = "(address, u32, ManagedReadContext) -> bool!";
    vec![
        (
            "__schema_storage",
            "(address, u32, u32, ManagedReadContext) -> bool!".into(),
        ),
        (
            "__schema_class",
            "(address, address, ManagedReadContext) -> bool!".into(),
        ),
        (
            "__schema_proof",
            "(address, u32, bool, ManagedReadContext) -> bool!".into(),
        ),
        (
            "__schema_array",
            format!("(address, u32, ManagedReadContext, {checker}) -> bool!"),
        ),
        (
            "__schema_list",
            format!("(address, u32, ManagedReadContext, {checker}) -> bool!"),
        ),
        (
            "__schema_set",
            format!("(address, u32, ManagedReadContext, {checker}) -> bool!"),
        ),
        (
            "__schema_map",
            format!("(address, u32, ManagedReadContext, {checker}, {checker}) -> bool!"),
        ),
    ]
}

fn schema_binding_bodies(module: &str) -> String {
    let checker = "(address, u32, ManagedReadContext) -> bool!";
    format!(
        r#"
            let __schema_proof_cache: [[address; 2]] = []
            let __schema_proof: (address, u32, bool, ManagedReadContext) -> bool! = (type, schema, remember, context) => {{
                if !Unity.chargeManagedWork(context) {{ throw "managed read work limit exceeded" }}
                if remember {{
                    if __schema_proof_cache.length() >= 1024 {{ __schema_proof_cache.clear() }}
                    __schema_proof_cache.push([type, schema as address])
                    return true
                }}
                for cached in __schema_proof_cache {{
                    if !Unity.chargeManagedWork(context) {{ throw "managed read work limit exceeded" }}
                    if cached[0] == type && cached[1] == (schema as address) {{ return true }}
                }}
                return false
            }}
            let __schema_storage: (address, u32, u32, ManagedReadContext) -> bool! = (type, bytes, kinds, context) =>
                {module}.typeStorage(type, bytes, kinds, () => Unity.chargeManagedWork(context))
            let __schema_class: (address, address, ManagedReadContext) -> bool! = (type, expected, context) => {{
                {module}.typeStorage(type, __pointer_size, 1 << 0x12, () => Unity.chargeManagedWork(context))?
                __class_contract(type, expected, context)
            }}
            let __schema_array: (address, u32, ManagedReadContext, {checker}) -> bool! = (type, width, context, verify) =>
                verify({module}.vectorElementType(type, () => Unity.chargeManagedWork(context))?, width, context)
            let __schema_list: (address, u32, ManagedReadContext, {checker}) -> bool! = (type, width, context, verify) => {{
                let layout = {module}.listTypeLayout(type, () => Unity.chargeManagedWork(context))?
                verify(layout.elements.elementType(0)?, width, context)
            }}
            let __schema_set: (address, u32, ManagedReadContext, {checker}) -> bool! = (type, width, context, verify) => {{
                let layout = {module}.setTypeLayout(type, () => Unity.chargeManagedWork(context))?
                verify(layout.members[layout.members.length() - 1].typeAddress, width, context)
            }}
            let __schema_map: (address, u32, ManagedReadContext, {checker}, {checker}) -> bool! = (type, width, context, verifyKey, verifyValue) => {{
                let layout = {module}.dictionaryTypeLayout(type, () => Unity.chargeManagedWork(context))?
                verifyKey(layout.members[2].typeAddress, width, context)?
                verifyValue(layout.members[layout.members.length() - 1].typeAddress, width, context)
            }}
"#
    )
}

#[cfg(test)]
mod tests {
    use std::sync::Arc;

    use super::*;

    fn assert_augmented_tokens_match_source(
        prefix: &str,
        rendered: &RenderedLibraryBodies,
        suffix: &str,
    ) {
        let combined = format!("{prefix}\n{}\n{suffix}", rendered.source);
        let actual = augmented_tokens(&combined, prefix.len() + 1, rendered);
        let expected = lex_tokens(&combined);
        match (actual, expected) {
            (Ok(actual), Ok(expected)) => assert_eq!(actual, expected),
            (Err(actual), Err(expected)) => {
                assert_eq!(format!("{actual:?}"), format!("{expected:?}"));
            }
            (actual, expected) => panic!("token assembly mismatch: {actual:?} vs {expected:?}"),
        }
    }

    #[test]
    fn cached_library_tokens_preserve_offsets_and_generated_provider_tokens() {
        let library = StandardLibrary::new();
        let isolated = StandardLibrary::isolated_bundled();
        let sources = [
            "",
            "state \"game.exe\" {} // trailing comment",
            "fn identity(value) { return value }\nfn array(value: [u32]) { return value }",
            "// Unicode 🦊\r\nfn text() { return `héllo {1 + 2}` }",
            "/// documentation at the boundary",
            "fn partial(value: [u32]) {", // Parser recovery still sees the same stream.
        ];
        let managed_program = crate::parse(
            r#"image "Assembly-CSharp" { class Player { u32 state; } }
               state Unity ["game.exe"] {}"#,
        )
        .unwrap()
        .into_syntax();
        let provider = managed_preparation_source(
            &managed_program,
            "__prepare",
            "",
            None,
            &[SelectedProviderContext {
                index: 0,
                ty: "UnityContext",
                function_name: "__prepare_unity_context",
            }],
        );
        for library in [&library, &isolated] {
            let rendered = library.rendered_library_bodies();
            for source in sources {
                for suffix in ["", provider.as_str()] {
                    assert_augmented_tokens_match_source(source, rendered, suffix);
                }
            }
        }
    }

    #[test]
    fn token_cache_preserves_empty_libraries_and_lexical_failure_behavior() {
        for source in [
            "",
            "fn helper() {}",
            "*/ fn helper() {}",
            "/*",
            "\"unfinished",
        ] {
            let rendered = RenderedLibraryBodies {
                tokens: lex_tokens(source),
                source: source.to_owned(),
                body_ranges: Vec::new(),
            };
            for prefix in [
                "",
                "/*",
                "fn f() { return `unfinished",
                "\"unfinished",
                "🦊",
            ] {
                for suffix in ["", "*/ fn after() {}", "\"unfinished", "🦊"] {
                    assert_augmented_tokens_match_source(prefix, &rendered, suffix);
                }
            }
        }
    }

    #[test]
    #[ignore = "manual release benchmark; timings are not correctness thresholds"]
    fn benchmark_library_token_assembly() {
        use std::{hint::black_box, time::Instant};

        let library = StandardLibrary::new();
        let rendered = library.rendered_library_bodies();
        let prefix = "state \"game.exe\" {}\n";
        let combined = format!("{prefix}{}\n", rendered.source);
        let mut samples = [Vec::new(), Vec::new()];
        // Alternate order within each pair; exclude graph/cache initialization
        // and token disposal from both paths. The reference is the original
        // augmentation lexer, including its owned-token copy.
        for iteration in 0..220 {
            for branch in 0..2 {
                let path = (iteration + branch) % 2;
                let start = Instant::now();
                let tokens = if path == 0 {
                    lexer::lex_lossless(black_box(&combined))
                        .unwrap()
                        .tokens()
                        .cloned()
                        .collect::<Vec<_>>()
                } else {
                    augmented_tokens(black_box(&combined), prefix.len(), rendered).unwrap()
                };
                let elapsed = start.elapsed().as_nanos();
                if iteration >= 20 {
                    samples[path].push(elapsed);
                }
                black_box(tokens);
            }
        }
        println!(
            "library_source_bytes={} cached_tokens={}",
            rendered.source.len(),
            rendered.tokens.as_ref().unwrap().len()
        );
        for (name, mut samples) in ["whole_source_lex", "cached_library_tokens"]
            .into_iter()
            .zip(samples)
        {
            samples.sort_unstable();
            println!(
                "{name}: median_us={:.1} p95_us={:.1}",
                samples[100] as f64 / 1_000.0,
                samples[189] as f64 / 1_000.0
            );
        }
    }

    #[test]
    fn source_body_analysis_initializes_safely_from_parallel_callers() {
        let library = Arc::new(StandardLibrary {
            graph: Arc::new(
                crate::stdlib::graph::StandardLibraryGraph::build()
                    .expect("bundled graph is valid"),
            ),
        });
        std::thread::scope(|scope| {
            for _ in 0..8 {
                let library = Arc::clone(&library);
                scope.spawn(move || library.initialize_source_body_operations());
            }
        });
        assert!(library.source_body_operations_are_initialized());
    }

    #[test]
    fn augmentation_parses_deterministically_under_parallel_load() {
        let library = Arc::new(StandardLibrary::new());
        std::thread::scope(|scope| {
            for worker in 0..8 {
                let library = Arc::clone(&library);
                scope.spawn(move || {
                    for iteration in 0..16 {
                        augment_program_with_library_bodies(
                            "state \"parallel-probe.exe\" {}",
                            &crate::parse("state \"parallel-probe.exe\" {}")
                                .unwrap()
                                .into_syntax(),
                            &library,
                        )
                        .unwrap_or_else(|diagnostics| {
                            panic!("worker {worker} iteration {iteration} failed: {diagnostics:#?}")
                        });
                    }
                });
            }
        });
    }

    #[test]
    fn automatic_unity_schema_binding_targets_only_the_backend_neutral_adapter() {
        let program = crate::parse(
            r#"
                image "Assembly-CSharp" {
                    class GameManager {
                        static GameManager instance;
                        u32 state;
                    }
                }
                state Unity ["game.exe"] {}
            "#,
        )
        .expect("the schema fixture should parse")
        .into_syntax();

        let source = managed_preparation_source(&program, "__prepare", "", None, &[]);
        assert!(source.contains("await __runtime.image(\"Assembly-CSharp\")"));
        assert!(source.contains("await __image_0.classAnyFlat([\"GameManager\"])"));
        assert!(source.contains("__runtime.pointerBytes()"));
        assert!(!source.contains("__runtime.il2cpp"));
        assert!(!source.contains("__runtime.mono"));
        assert!(!source.contains("UnityRuntimeBackend"));
    }

    #[test]
    fn provider_context_preparation_is_demand_driven_without_managed_discovery() {
        let program = crate::parse(
            r#"
                state Unity ["game.exe"] {
                    scene = unity.scenes.active();
                }
            "#,
        )
        .expect("the provider-context fixture should parse")
        .into_syntax();
        let contexts = [SelectedProviderContext {
            index: 0,
            ty: "UnityContext",
            function_name: "__prepare_unity_context",
        }];

        let source =
            managed_preparation_source(&program, "__prepare_managed_runtime", "", None, &contexts);

        assert!(source.contains("let __provider_context_0 = await __prepare_unity_context()"));
        assert!(source.contains("__provider_context_0: UnityContext"));
        assert!(!source.contains("__prepare_managed_runtime"));
        assert!(!source.contains("let __runtime"));
    }

    #[test]
    fn provider_context_and_managed_schema_share_one_attachment_struct() {
        let program = crate::parse(
            r#"
                image "Assembly-CSharp" {
                    class GameManager { u32 state; }
                }
                state Unity ["game.exe"] {
                    scene = unity.scenes.active();
                }
            "#,
        )
        .expect("the mixed Unity fixture should parse")
        .into_syntax();
        let contexts = [SelectedProviderContext {
            index: 0,
            ty: "UnityContext",
            function_name: "__prepare_unity_context",
        }];

        let source =
            managed_preparation_source(&program, "__prepare_managed_runtime", "", None, &contexts);

        assert!(source.contains("let __runtime = await __prepare_managed_runtime()"));
        assert!(source.contains("let __provider_context_0 = await __prepare_unity_context()"));
        assert!(source.contains("__provider_context_0: UnityContext"));
        assert!(source.contains(MANAGED_POINTER_SIZE_FIELD));
    }

    #[test]
    fn typed_scene_components_prepare_only_the_selected_class_header() {
        let program = crate::parse(
            r#"
                image "Assembly-CSharp" {
                    class Player {}
                    class Enemy {}
                }
                state Unity ["game.exe"] {}
                fn player(object: UnityGameObject) {
                    return object.component<Player>()
                }
            "#,
        )
        .expect("the typed component fixture should parse")
        .into_syntax();

        let source = managed_preparation_source(&program, "__prepare", "", None, &[]);
        let classes = program.managed_class_declarations();
        let player = classes
            .iter()
            .find(|class| class.name == "Player")
            .expect("Player class");
        let enemy = classes
            .iter()
            .find(|class| class.name == "Enemy")
            .expect("Enemy class");

        assert!(source.contains(&managed_instance_header_name(player.id.index())));
        assert!(!source.contains(&managed_instance_header_name(enemy.id.index())));
    }

    #[test]
    fn explicit_unity_schema_binding_keeps_its_prunable_concrete_backend() {
        let program = crate::parse(
            r#"
                image "Assembly-CSharp" {
                    class GameManager { u32 state; }
                }
                state Unity.il2cpp(Il2CppProfile.unity2021_3_11f1X64()) ["game.exe"] {}
            "#,
        )
        .expect("the schema fixture should parse")
        .into_syntax();

        let source = managed_preparation_source(
            &program,
            "__prepare",
            "2020",
            Some(ManagedRuntimeBackend::Il2Cpp),
            &[],
        );
        assert!(source.contains("let __module = __runtime\n"));
        assert!(!source.contains("__runtime.il2cpp"));
        assert!(source.contains("await __module.image(\"Assembly-CSharp\")"));
        assert!(!source.contains("__runtime.mono"));
    }
}
