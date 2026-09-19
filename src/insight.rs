//! Semantic hover and signature information for editor clients.

use std::sync::Arc;

use crate::{
    ast::{Expr, ExprKind, MatchPattern, Program, RangeKind, Span},
    database::{
        CompilerDatabase, DefinitionTarget, SemanticQueryResult, SemanticSnapshot,
        SourceDefinition, SourceDefinitionId,
    },
    documentation::{StandardLibraryDocumentation, language_item_uri, symbol_uri},
    effects::FunctionOperationSemantics,
    language::{LanguageCatalog, LanguageItem},
    lexer::{Token, TokenKind},
    semantic::{ResolvedCall, ResolvedMember},
    stdlib::{
        StandardLibrary, StateProviderProcesses, StdlibItem, StdlibItemId, StdlibSymbolId,
        StdlibTypeConstructorId, TypeRef as CatalogTypeRef,
    },
    stdlib_semantic::StandardLibrarySemanticExt,
    syntax::SourceDocument,
    type_display::display_type,
    types::{BuiltinType, TypeId, TypeKind},
    visit::{self, Visitor},
};

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct HoverInfo {
    pub span: Span,
    pub markdown: String,
    /// Stable compiler-owned reference page for the hovered catalog symbol.
    pub documentation_uri: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ParameterInformation {
    pub label: String,
    pub documentation: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SignatureInformation {
    pub label: String,
    pub documentation: String,
    pub parameters: Vec<ParameterInformation>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SignatureHelp {
    pub signatures: Vec<SignatureInformation>,
    pub active_signature: usize,
    pub active_parameter: usize,
}

pub(crate) fn hover(
    database: &mut CompilerDatabase,
    offset: usize,
) -> SemanticQueryResult<Option<HoverInfo>> {
    let offset = database.hover_query_offset(offset)?;
    let standard_library = database.context().standard_library();
    let Some(token) = database.token_at(offset)? else {
        return Ok(None);
    };
    if let Some((provider_name, selector_name, selector_span)) = database
        .recovering_parse()?
        .syntax()
        .state
        .as_ref()
        .and_then(|state| {
            state
                .provider
                .iter()
                .chain(
                    state
                        .provider_alternatives
                        .iter()
                        .map(|alternative| &alternative.provider),
                )
                .find(|provider| {
                    provider.selector.as_ref().is_some_and(|selector| {
                        selector.name_span.start <= offset && offset < selector.name_span.end
                    })
                })
        })
        .and_then(|provider| {
            provider.selector.as_ref().and_then(|selector| {
                (selector.name_span.start <= offset && offset < selector.name_span.end).then(|| {
                    (
                        provider.name.clone(),
                        selector.name.clone(),
                        selector.name_span,
                    )
                })
            })
        })
        && let Some(provider) = standard_library.state_provider_by_name(&provider_name)
        && let Some(selector) = provider
            .selectors
            .iter()
            .find(|selector| selector.name == selector_name)
    {
        let parameters = selector
            .parameters
            .iter()
            .map(|parameter| {
                format!(
                    "{}: {}",
                    parameter.name,
                    standard_library.render_type(parameter.ty)
                )
            })
            .collect::<Vec<_>>()
            .join(", ");
        let processes = match provider.processes {
            StateProviderProcesses::SourceState => " [\"game.exe\"]",
            StateProviderProcesses::Declared(_) => "",
        };
        let form = format!(
            "state {}.{}({parameters}){processes} {{ ... }}",
            provider.name, selector.name
        );
        let prose = crate::documentation::prose_markdown(
            selector.documentation.summary,
            selector.documentation.details,
        );
        return Ok(Some(HoverInfo {
            span: selector_span,
            markdown: format!(
                "```splitscript\n{form}\n```\n\n{}",
                crate::documentation::strip_intra_doc_links(&prose)
            ),
            documentation_uri: Some(symbol_uri(
                StdlibSymbolId::StateProvider(provider.id),
                &standard_library,
            )),
        }));
    }
    if let Some(markdown) = float_literal_markdown(database, &token, offset)? {
        return Ok(Some(HoverInfo {
            span: token.span,
            markdown,
            documentation_uri: None,
        }));
    }
    if let Some(rest_span) =
        array_rest_pattern_span_at(database.recovering_parse()?.syntax(), offset)
    {
        let catalog = LanguageCatalog::new();
        let item = catalog.item(crate::language::LanguageItemId::ArrayRestPattern);
        return Ok(Some(HoverInfo {
            span: rest_span,
            markdown: render_language_hover_with_form(item, None),
            documentation_uri: Some(language_item_uri(item.id)),
        }));
    }
    if let Some(kind) = range_pattern_kind_at(database.recovering_parse()?.syntax(), offset) {
        let catalog = LanguageCatalog::new();
        let item = catalog.item(crate::language::LanguageItemId::Range);
        let form = match kind {
            RangeKind::Exclusive => "start..<end",
            RangeKind::Inclusive => "start..=end",
        };
        return Ok(Some(HoverInfo {
            span: token.span,
            markdown: render_language_hover_with_form(item, Some(form)),
            documentation_uri: Some(language_item_uri(item.id)),
        }));
    }
    if let Some(constructor) = range_constructor_for_token(&token.kind)
        && let Some(analysis) = database.analysis_at(offset)?
        && matches!(analysis.type_kind, TypeKind::Range { .. })
        && let Some(context) = semantic_context(database)
    {
        let concrete_type = render_type(analysis.ty, &context);
        return Ok(Some(HoverInfo {
            span: token.span,
            markdown: render_stdlib_symbol_hover_with_form(
                standard_library.clone(),
                StdlibSymbolId::TypeConstructor(constructor),
                Some(&concrete_type),
            ),
            documentation_uri: Some(symbol_uri(
                StdlibSymbolId::TypeConstructor(constructor),
                &standard_library,
            )),
        }));
    }
    if matches!(&token.kind, TokenKind::Ident(name) if name == "component")
        && let Some(analysis) = database.analysis_at(offset)?
        && let Some(ExpressionResolution::Call(ResolvedCall::ManagedComponent { class, .. })) =
            analysis.resolution
        && let Some(context) = semantic_context(database)
        && let Some(class) = context.syntax().managed_class(class)
    {
        return Ok(Some(HoverInfo {
            span: token.span,
            markdown: format!(
                "```splitscript\nUnityGameObject.component<{}>() -> {}\u{2e}Ref!\n```\n\nFinds the managed component whose runtime class matches the declared Unity schema class `{}`. The returned live reference can be read field by field or converted into one transactional immutable snapshot with `.snapshot()`.",
                class.name, class.name, class.name
            ),
            documentation_uri: Some(symbol_uri(
                StdlibSymbolId::Type(crate::stdlib::StdlibTypeId::UnityGameObject),
                &standard_library,
            )),
        }));
    }
    let target = database.definition_at_query_offset(offset)?;
    if target.is_none()
        && let Some(analysis) = database.analysis_at(offset)?
        && let Some(ExpressionResolution::ValuePath {
            root: Some(crate::semantic::ResolvedValue::Variable(value)),
            members,
        }) = analysis.resolution
        && members.is_empty()
        && let Some(context) = semantic_context(database)
        && let Some((name, description)) = state_transform_binding(value, &context)
    {
        let ty = render_type(analysis.ty, &context);
        return Ok(Some(HoverInfo {
            span: token.span,
            markdown: source_markdown(&format!("{name}: {ty}"), description),
            documentation_uri: None,
        }));
    }
    let Some(target) = target else {
        return expression_type_hover(database, offset);
    };
    let shorthand_value = if matches!(
        &target,
        DefinitionTarget::Source(SourceDefinition {
            id: SourceDefinitionId::StructField(_),
            ..
        }) | DefinitionTarget::StandardLibrarySymbol(StdlibSymbolId::Field(_))
    ) {
        let definitions = database.definition_index()?;
        definitions
            .references_at_offset(offset)
            .find(|reference| matches!(reference.target, SourceDefinitionId::Value(_)))
            .and_then(|reference| definitions.get(reference.target))
            .cloned()
    } else {
        None
    };
    let (mut markdown, documentation_uri) = match target {
        DefinitionTarget::StandardLibrary(item) => {
            let type_arguments = database
                .analysis_at(offset)?
                .and_then(|analysis| match analysis.resolution {
                    Some(ExpressionResolution::Call(ResolvedCall::StandardLibrary {
                        item: resolved,
                        type_arguments,
                        ..
                    })) if resolved == item => Some(type_arguments),
                    _ => None,
                })
                .unwrap_or_default();
            let semantic = semantic_context(database);
            (
                render_stdlib_hover(
                    standard_library.clone(),
                    item,
                    substitutions(item, &type_arguments, semantic.as_ref()),
                ),
                Some(symbol_uri(StdlibSymbolId::Item(item), &standard_library)),
            )
        }
        DefinitionTarget::StandardLibrarySymbol(symbol) => (
            render_stdlib_symbol_hover(standard_library.clone(), symbol),
            Some(symbol_uri(symbol, &standard_library)),
        ),
        DefinitionTarget::Language(item) => {
            let catalog = LanguageCatalog::new();
            let item_metadata = catalog.item(item);
            let form = if item == crate::language::LanguageItemId::SelfValue {
                database.analysis_at(offset)?.and_then(|analysis| {
                    semantic_context(database).map(|context| {
                        let receiver = root_value_for_resolution(&analysis)
                            .and_then(crate::semantic::ResolvedValue::source_value)
                            .and_then(|value| context.semantics().value_type(value))
                            .unwrap_or(analysis.ty);
                        format!("self: {}", render_type(receiver, &context))
                    })
                })
            } else {
                None
            };
            (
                render_language_hover_with_form(item_metadata, form.as_deref()),
                Some(language_item_uri(item)),
            )
        }
        DefinitionTarget::Source(definition) => {
            if matches!(&token.kind, TokenKind::Ident(name) if name == "instances")
                && let Some(analysis) = database.analysis_at(offset)?
                && let Some(ExpressionResolution::Call(ResolvedCall::ManagedInstances { class })) =
                    analysis.resolution
                && definition.id == SourceDefinitionId::ManagedClass(class)
                && let Some(context) = semantic_context(database)
                && let Some(class) = context.syntax().managed_class(class)
            {
                return Ok(Some(HoverInfo {
                    span: token.span,
                    markdown: format!(
                        "```splitscript\n{}.instances() -> async [{}\u{2e}Ref]\n```\n\nCooperatively scans readable, writable, non-executable process memory and returns a completed snapshot of live references to `{}` objects. Scanning is bounded across ticks and stops automatically when the attached process closes.",
                        class.name, class.name, class.name
                    ),
                    documentation_uri: None,
                }));
            }
            if matches!(&token.kind, TokenKind::Ident(name) if name == "snapshot")
                && let Some(analysis) = database.analysis_at(offset)?
                && let Some(ExpressionResolution::Call(ResolvedCall::ManagedSnapshot {
                    class, ..
                })) = analysis.resolution
                && definition.id == SourceDefinitionId::ManagedClass(class)
                && let Some(context) = semantic_context(database)
                && let Some(class) = context.syntax().managed_class(class)
            {
                return Ok(Some(HoverInfo {
                    span: token.span,
                    markdown: format!(
                        "```splitscript\n{}.Ref.snapshot() -> {}!\n```\n\nReads every active instance field transactionally into one immutable local `{}` value. If any field read fails, the operation returns that error and exposes no partial snapshot.",
                        class.name, class.name, class.name
                    ),
                    documentation_uri: None,
                }));
            }
            if definition.id == SourceDefinitionId::State {
                let analysis = database.analysis_at(offset)?;
                if let Some((provider, context_index)) =
                    analysis.as_ref().and_then(provider_context_for_resolution)
                {
                    let context =
                        &standard_library.state_provider(provider).contexts[context_index];
                    let ty = standard_library.type_decl(context.ty);
                    let prose = crate::documentation::prose_markdown(
                        context.documentation.summary,
                        context.documentation.details,
                    );
                    let mut markdown = format!(
                        "```splitscript\n{}: {}\n```\n\n{}",
                        context.name,
                        ty.name,
                        crate::documentation::strip_intra_doc_links(&prose),
                    );
                    append_examples(&mut markdown, context.documentation.examples);
                    return Ok(Some(HoverInfo {
                        span: token.span,
                        markdown,
                        documentation_uri: Some(symbol_uri(
                            StdlibSymbolId::Type(context.ty),
                            &standard_library,
                        )),
                    }));
                }
                let Some(provider) = analysis.as_ref().and_then(provider_value_for_resolution)
                else {
                    let Some(context) = semantic_context(database) else {
                        return Ok(None);
                    };
                    let Some(markdown) = render_source_hover(&definition, &context) else {
                        return Ok(None);
                    };
                    return Ok(Some(HoverInfo {
                        span: token.span,
                        markdown,
                        documentation_uri: None,
                    }));
                };
                return Ok(Some(HoverInfo {
                    span: token.span,
                    markdown: render_stdlib_symbol_hover(
                        standard_library.clone(),
                        StdlibSymbolId::StateProvider(provider),
                    ),
                    documentation_uri: Some(symbol_uri(
                        StdlibSymbolId::StateProvider(provider),
                        &standard_library,
                    )),
                }));
            }
            let Some(context) = semantic_context(database) else {
                return Ok(None);
            };
            let Some(markdown) = render_source_hover(&definition, &context) else {
                return Ok(None);
            };
            (markdown, None)
        }
    };
    if let Some(value) = shorthand_value.as_ref()
        && let Some(context) = semantic_context(database)
        && let Some(value_markdown) = render_source_hover(value, &context)
    {
        markdown.push_str("\n\n**Value represented by the shorthand**\n\n");
        markdown.push_str(&value_markdown);
    }
    Ok(Some(HoverInfo {
        span: token.span,
        markdown,
        documentation_uri,
    }))
}

fn expression_type_hover(
    database: &mut CompilerDatabase,
    offset: usize,
) -> SemanticQueryResult<Option<HoverInfo>> {
    let Some(analysis) = database.analysis_at(offset)? else {
        return Ok(None);
    };
    if matches!(analysis.type_kind, TypeKind::Error) {
        return Ok(None);
    }
    let Some(context) = semantic_context(database) else {
        return Ok(None);
    };
    Ok(Some(HoverInfo {
        span: analysis.span,
        markdown: format!(
            "```splitscript\n{}\n```",
            render_type(analysis.ty, &context)
        ),
        documentation_uri: None,
    }))
}

fn range_constructor_for_token(kind: &TokenKind) -> Option<StdlibTypeConstructorId> {
    match kind {
        TokenKind::DotDotLt => Some(StdlibTypeConstructorId::ExclusiveRange),
        TokenKind::DotDotEq => Some(StdlibTypeConstructorId::InclusiveRange),
        _ => None,
    }
}

fn range_pattern_kind_at(program: &Program, offset: usize) -> Option<RangeKind> {
    struct Finder {
        offset: usize,
        kind: Option<RangeKind>,
    }

    impl<'ast> Visitor<'ast> for Finder {
        fn visit_pattern(&mut self, pattern: &'ast MatchPattern) {
            if let MatchPattern::IntRange {
                kind,
                operator_span,
                ..
            } = pattern
                && operator_span.start <= self.offset
                && self.offset < operator_span.end
            {
                self.kind = Some(*kind);
            }
            if self.kind.is_none() {
                visit::walk_pattern(self, pattern);
            }
        }
    }

    let mut finder = Finder { offset, kind: None };
    finder.visit_program(program);
    finder.kind
}

fn array_rest_pattern_span_at(program: &Program, offset: usize) -> Option<Span> {
    struct Finder {
        offset: usize,
        span: Option<Span>,
    }

    impl<'ast> Visitor<'ast> for Finder {
        fn visit_pattern(&mut self, pattern: &'ast MatchPattern) {
            if let MatchPattern::Array(array) = pattern
                && let Some(rest) = array.rest
                && rest.start <= self.offset
                && self.offset < rest.end
            {
                self.span = Some(rest);
            }
            if self.span.is_none() {
                visit::walk_pattern(self, pattern);
            }
        }
    }

    let mut finder = Finder { offset, span: None };
    finder.visit_program(program);
    finder.span
}

fn provider_value_for_resolution(
    analysis: &crate::database::PositionAnalysis,
) -> Option<crate::stdlib::StdlibStateProviderId> {
    match root_value_for_resolution(analysis)? {
        crate::semantic::ResolvedValue::StandardLibraryConstant(_) => None,
        crate::semantic::ResolvedValue::ProviderValue(provider) => Some(provider),
        crate::semantic::ResolvedValue::ProviderContext { provider, .. } => Some(provider),
        crate::semantic::ResolvedValue::ManagedStatic { .. } => None,
        crate::semantic::ResolvedValue::Variable(_)
        | crate::semantic::ResolvedValue::StateCandidate(_)
        | crate::semantic::ResolvedValue::CurrentSnapshot
        | crate::semantic::ResolvedValue::OldSnapshot
        | crate::semantic::ResolvedValue::SettingsView
        | crate::semantic::ResolvedValue::OldSettingsView
        | crate::semantic::ResolvedValue::CurrentState(_)
        | crate::semantic::ResolvedValue::OldState(_)
        | crate::semantic::ResolvedValue::Setting(_)
        | crate::semantic::ResolvedValue::OldSetting(_) => None,
    }
}

fn provider_context_for_resolution(
    analysis: &crate::database::PositionAnalysis,
) -> Option<(crate::stdlib::StdlibStateProviderId, usize)> {
    match root_value_for_resolution(analysis)? {
        crate::semantic::ResolvedValue::ProviderContext { provider, context } => {
            Some((provider, context as usize))
        }
        _ => None,
    }
}

fn root_value_for_resolution(
    analysis: &crate::database::PositionAnalysis,
) -> Option<crate::semantic::ResolvedValue> {
    match analysis.resolution.as_ref()? {
        ExpressionResolution::ValuePath { root, .. } => *root,
        ExpressionResolution::Call(call) => call.receiver()?.path().map(|(root, _)| root),
        ExpressionResolution::Member { .. }
        | ExpressionResolution::DynamicCall(_)
        | ExpressionResolution::FunctionValue(_)
        | ExpressionResolution::StructLiteral { .. }
        | ExpressionResolution::EnumConstructor { .. } => None,
    }
}

fn float_literal_markdown(
    database: &mut CompilerDatabase,
    token: &Token,
    offset: usize,
) -> SemanticQueryResult<Option<String>> {
    let TokenKind::Float(spelling) = &token.kind else {
        return Ok(None);
    };
    let Some(analysis) = database.analysis_at(offset)? else {
        return Ok(None);
    };
    let normalized = spelling.replace('_', "");
    let (ty, bits) = match analysis.type_kind {
        TypeKind::Builtin(BuiltinType::F32) => {
            let Ok(value) = normalized.parse::<f32>() else {
                return Ok(None);
            };
            ("f32", format!("0x{:08x}", value.to_bits()))
        }
        TypeKind::Builtin(BuiltinType::F64) => {
            let Ok(value) = normalized.parse::<f64>() else {
                return Ok(None);
            };
            ("f64", format!("0x{:016x}", value.to_bits()))
        }
        _ => return Ok(None),
    };
    Ok(Some(format!(
        "```splitscript\n{spelling}: {ty}\n```\n\n**Rounded IEEE-754 bits:** `{bits}`"
    )))
}

fn state_transform_binding(
    value: crate::ast::ValueId,
    context: &SemanticContext,
) -> Option<(&'static str, &'static str)> {
    context
        .syntax()
        .state
        .as_ref()?
        .all_fields()
        .find_map(|field| {
            let transform = field.transform.as_ref()?;
            if transform.value == value {
                Some(("value", "Raw candidate for this state field."))
            } else {
                None
            }
        })
}

pub(crate) fn signature_help(
    database: &mut CompilerDatabase,
    offset: usize,
) -> SemanticQueryResult<Option<SignatureHelp>> {
    let compiler_context = database.context();
    let standard_library = compiler_context.standard_library();
    let recovered = database.recovering_parse()?;
    let document = recovered.source_document().clone();
    let syntax = recovered.syntax().clone();
    let Some(call_site) = active_call(&document, offset) else {
        return Ok(None);
    };
    let mut semantic = semantic_context(database);
    let resolved = semantic.as_ref().and_then(|context| {
        call_expression_at(&syntax, call_site.open, &call_site.callee)
            .and_then(|expression| context.semantics().call(expression.id))
            .and_then(|call| match call {
                ResolvedCall::StandardLibrary {
                    item,
                    type_arguments,
                    ..
                } => Some((*item, type_arguments.clone())),
                _ => None,
            })
    });
    let mut selected = resolved.or_else(|| {
        standard_library
            .resolve_path(&call_site.callee)
            .map(|candidate| (candidate.item.id, Vec::new()))
    });
    if selected.is_none()
        && let Some((item, type_arguments, probe_semantic)) =
            infer_method_call(document.source(), &call_site, &compiler_context)
    {
        selected = Some((item, type_arguments));
        semantic = Some(probe_semantic);
    }
    let Some((item, type_arguments)) = selected else {
        return Ok(None);
    };
    let substitutions = substitutions(item, &type_arguments, semantic.as_ref());
    let documentation = StandardLibraryDocumentation::generate_with_library(
        &standard_library,
        item,
        &substitutions,
    );
    let signature = SignatureInformation {
        label: documentation.signature.clone(),
        documentation: documentation.details_markdown(),
        parameters: documentation
            .parameters
            .iter()
            .map(|parameter| ParameterInformation {
                label: parameter.name.to_owned(),
                documentation: parameter.documentation.to_owned(),
            })
            .collect(),
    };
    let active_parameter = call_site
        .active_parameter
        .min(signature.parameters.len().saturating_sub(1));
    Ok(Some(SignatureHelp {
        signatures: vec![signature],
        active_signature: 0,
        active_parameter,
    }))
}

use crate::hir::ExpressionResolution;

struct SemanticContext {
    standard_library: StandardLibrary,
    snapshot: Arc<SemanticSnapshot>,
}

fn semantic_context(database: &mut CompilerDatabase) -> Option<SemanticContext> {
    let snapshot = database.semantic_snapshot().ok()?;
    Some(SemanticContext {
        standard_library: snapshot.context().standard_library(),
        snapshot,
    })
}

impl SemanticContext {
    fn syntax(&self) -> &crate::ast::Program {
        self.snapshot.syntax()
    }

    fn semantics(&self) -> &crate::semantic::SemanticModel {
        self.snapshot.semantics()
    }

    fn effects(&self) -> Option<&crate::effects::OperationAnalysis> {
        self.snapshot.effects()
    }
}

fn substitutions(
    item: StdlibItemId,
    arguments: &[TypeId],
    context: Option<&SemanticContext>,
) -> Vec<(&'static str, String)> {
    let Some(context) = context else {
        return Vec::new();
    };
    context
        .standard_library
        .item(item)
        .signature
        .type_parameters
        .iter()
        .zip(arguments)
        .map(|(parameter, ty)| (parameter.name, render_type(*ty, context)))
        .collect()
}

fn render_type(ty: TypeId, context: &SemanticContext) -> String {
    display_type(ty, &context.snapshot)
}

fn syntax_for_binding(
    syntax: &crate::ast::Program,
    value: crate::ast::ValueId,
) -> Option<&crate::ast::ForBinding> {
    struct Finder<'ast> {
        value: crate::ast::ValueId,
        found: Option<&'ast crate::ast::ForBinding>,
    }

    impl<'ast> crate::visit::Visitor<'ast> for Finder<'ast> {
        fn visit_for_binding(&mut self, binding: &'ast crate::ast::ForBinding) {
            let mut contains = false;
            binding
                .binding
                .visit_bindings(&mut |leaf| contains |= leaf.id == self.value);
            if contains {
                self.found = Some(binding);
            }
        }
    }

    let mut finder = Finder { value, found: None };
    crate::visit::Visitor::visit_program(&mut finder, syntax);
    finder.found
}

fn syntax_parameter(
    syntax: &crate::ast::Program,
    value: crate::ast::ValueId,
) -> Option<&crate::ast::Parameter> {
    struct Finder<'ast> {
        value: crate::ast::ValueId,
        found: Option<&'ast crate::ast::Parameter>,
    }

    impl<'ast> crate::visit::Visitor<'ast> for Finder<'ast> {
        fn visit_parameter(&mut self, parameter: &'ast crate::ast::Parameter) {
            let mut contains = false;
            parameter
                .binding
                .visit_bindings(&mut |binding| contains |= binding.id == self.value);
            if contains {
                self.found = Some(parameter);
            }
        }
    }

    let mut finder = Finder { value, found: None };
    crate::visit::Visitor::visit_program(&mut finder, syntax);
    finder.found
}

fn append_attachment_shapes(
    description: &mut String,
    syntax: &crate::ast::Program,
    analysis: &crate::ScopedGlobalAnalysis,
    shapes: &[crate::AttachmentShape],
    total_shapes: usize,
) {
    if shapes.is_empty() || shapes.len() == total_shapes {
        return;
    }
    let names = shapes
        .iter()
        .map(|shape| format!("`{}`", analysis.shape_label(*shape, syntax)))
        .collect::<Vec<_>>();
    if !names.is_empty() {
        description.push_str("\n\n**Attachment shapes:** ");
        description.push_str(&names.join(", "));
    }
}

fn append_shape_availability(
    description: &mut String,
    predicate: &crate::semantic::ResolvedShapePredicate,
    context: &SemanticContext,
) {
    let alternatives = simplified_shape_alternatives(predicate, context.syntax());
    let alternatives = alternatives
        .iter()
        .map(|alternative| {
            alternative
                .iter()
                .map(|constraint| render_shape_constraint(*constraint, context))
                .collect::<Option<Vec<_>>>()
                .map(|constraints| constraints.join(" && "))
        })
        .collect::<Option<Vec<_>>>();
    let Some(alternatives) = alternatives else {
        return;
    };
    if alternatives.is_empty() || alternatives.iter().any(String::is_empty) {
        return;
    }
    description.push_str("\n\n**Available when:** ");
    description.push_str(
        &alternatives
            .iter()
            .map(|alternative| format!("`{alternative}`"))
            .collect::<Vec<_>>()
            .join(" or "),
    );
    description.push('.');
}

fn state_field_shape_availability(
    field: crate::ast::ValueId,
    context: &SemanticContext,
) -> Option<crate::semantic::ResolvedShapePredicate> {
    let semantics = context.semantics();
    let storage = semantics.state_storage_field(field)?;
    let mut alternatives = Vec::new();
    let mut found = false;
    for candidate in context
        .syntax()
        .state
        .as_ref()?
        .all_fields()
        .filter(|candidate| semantics.state_storage_field(candidate.id) == Some(storage))
    {
        found = true;
        let predicate = semantics.state_field_shape_predicate(candidate.id)?;
        alternatives.extend(predicate.alternatives.iter().cloned());
    }
    found.then_some(crate::semantic::ResolvedShapePredicate { alternatives })
}

/// Removes dimensions that do not affect a declaration's availability from
/// its finite truth table. The checked predicate deliberately retains complete
/// assignments for schema planning; hover should instead show the smallest
/// condition a user needs to establish.
fn simplified_shape_alternatives(
    predicate: &crate::semantic::ResolvedShapePredicate,
    syntax: &crate::ast::Program,
) -> Vec<Vec<crate::semantic::ResolvedShapeConstraint>> {
    let mut alternatives = Vec::new();
    for alternative in &predicate.alternatives {
        if !alternatives.contains(alternative) {
            alternatives.push(alternative.clone());
        }
    }

    loop {
        let before_subsumption = alternatives.len();
        let snapshot = alternatives.clone();
        alternatives.retain(|candidate| {
            !snapshot.iter().any(|other| {
                other.len() < candidate.len()
                    && other
                        .iter()
                        .all(|constraint| candidate.contains(constraint))
            })
        });

        let mut dimensions = Vec::new();
        for constraint in alternatives.iter().flatten() {
            if !dimensions.contains(&constraint.dimension) {
                dimensions.push(constraint.dimension);
            }
        }

        let mut collapsed = false;
        'dimensions: for dimension in dimensions {
            let Some(variants) = shape_dimension_variants(dimension, &alternatives, syntax) else {
                continue;
            };
            let mut bases = Vec::new();
            for alternative in &alternatives {
                if !alternative
                    .iter()
                    .any(|constraint| constraint.dimension == dimension)
                {
                    continue;
                }
                let base = alternative
                    .iter()
                    .filter(|constraint| constraint.dimension != dimension)
                    .copied()
                    .collect::<Vec<_>>();
                if !bases.contains(&base) {
                    bases.push(base);
                }
            }
            for base in bases {
                let covers_every_variant = variants.iter().all(|variant| {
                    alternatives.iter().any(|alternative| {
                        alternative.iter().any(|constraint| {
                            constraint.dimension == dimension && constraint.variant == *variant
                        }) && alternative
                            .iter()
                            .filter(|constraint| constraint.dimension != dimension)
                            .copied()
                            .eq(base.iter().copied())
                    })
                });
                if !covers_every_variant {
                    continue;
                }
                alternatives.retain(|alternative| {
                    !(alternative
                        .iter()
                        .any(|constraint| constraint.dimension == dimension)
                        && alternative
                            .iter()
                            .filter(|constraint| constraint.dimension != dimension)
                            .copied()
                            .eq(base.iter().copied()))
                });
                alternatives.push(base);
                collapsed = true;
                break 'dimensions;
            }
        }

        if !collapsed && alternatives.len() == before_subsumption {
            break;
        }
    }
    alternatives
}

fn shape_dimension_variants(
    dimension: crate::semantic::ResolvedShapeDimension,
    alternatives: &[Vec<crate::semantic::ResolvedShapeConstraint>],
    syntax: &crate::ast::Program,
) -> Option<Vec<crate::ast::EnumVariantId>> {
    let variant = alternatives
        .iter()
        .flatten()
        .find(|constraint| constraint.dimension == dimension)?
        .variant;
    let enumeration = syntax.enum_declarations().find(|enumeration| {
        enumeration
            .variants
            .iter()
            .any(|candidate| candidate.id == variant)
    })?;
    Some(
        enumeration
            .variants
            .iter()
            .map(|variant| variant.id)
            .collect(),
    )
}

fn render_shape_constraint(
    constraint: crate::semantic::ResolvedShapeConstraint,
    context: &SemanticContext,
) -> Option<String> {
    let syntax = context.syntax();
    let dimension = match constraint.dimension {
        crate::semantic::ResolvedShapeDimension::Global(value) => syntax
            .globals
            .iter()
            .filter_map(|global| global.binding.simple_binding())
            .find(|binding| binding.id == value)?
            .name
            .clone(),
        crate::semantic::ResolvedShapeDimension::StateField(value) => {
            let field = syntax
                .state
                .as_ref()?
                .all_fields()
                .find(|field| field.id == value)?;
            format!("current.{}", field.name)
        }
    };
    let enumeration = syntax.enum_declarations().find(|enumeration| {
        enumeration
            .variants
            .iter()
            .any(|variant| variant.id == constraint.variant)
    })?;
    let variant = enumeration
        .variants
        .iter()
        .find(|variant| variant.id == constraint.variant)?;
    Some(format!(
        "{dimension} is {}.{}",
        enumeration.name, variant.name
    ))
}

fn struct_field_memory_layout(
    structure: &crate::ast::StructDecl,
    field: crate::ast::StructFieldId,
    context: &SemanticContext,
) -> Option<((u32, u32), (u32, u32))> {
    let checked = context.snapshot.checked()?;
    let one = |width| {
        let struct_layout = checked
            .memory_layouts()
            .structure(structure.id, width)
            .ok()?;
        let field_layout = struct_layout
            .fields
            .iter()
            .find(|layout| layout.field == crate::memory::MemoryFieldId::Source(field))?;
        let size = checked
            .memory_layouts()
            .layout(field_layout.ty, checked.semantics(), width)
            .ok()?
            .size();
        Some((field_layout.offset, size))
    };
    Some((
        one(crate::memory::MemoryAddressWidth::Bit32)?,
        one(crate::memory::MemoryAddressWidth::Bit64)?,
    ))
}

fn append_source_capabilities(description: &mut String, ty: TypeId, context: &SemanticContext) {
    let Some(checked) = context.snapshot.checked() else {
        return;
    };
    let capabilities = context
        .standard_library
        .capabilities()
        .iter()
        .filter(|capability| {
            checked
                .capabilities()
                .has(ty, capability.id, context.semantics())
        })
        .map(|capability| capability.id)
        .collect::<Vec<_>>();
    let mut capabilities = context.standard_library.minimal_capabilities(&capabilities);
    // A custom user-facing formatter remains meaningful even when a derived
    // `Debug` implementation implies the bare `Display` capability.
    if checked
        .capabilities()
        .method_implementation(
            ty,
            crate::stdlib::StdlibCapabilityId::Display,
            crate::stdlib::StdlibItemId::DisplayToString,
            context.semantics(),
        )
        .is_some()
        && !capabilities.contains(&crate::stdlib::StdlibCapabilityId::Display)
    {
        capabilities.push(crate::stdlib::StdlibCapabilityId::Display);
    }
    if capabilities.is_empty() {
        return;
    }
    description.push_str("\n\n**Capabilities:** ");
    description.push_str(
        &capabilities
            .iter()
            .map(|capability| {
                let name = context.standard_library.capability(*capability).name;
                let implementation = match *capability {
                    crate::stdlib::StdlibCapabilityId::Display => checked
                        .capabilities()
                        .method_implementation(
                            ty,
                            crate::stdlib::StdlibCapabilityId::Display,
                            crate::stdlib::StdlibItemId::DisplayToString,
                            context.semantics(),
                        )
                        .is_some()
                        .then_some("custom")
                        .or_else(|| {
                            checked
                                .capabilities()
                                .has_derived_display(ty, context.semantics())
                                .then_some("derived")
                        }),
                    crate::stdlib::StdlibCapabilityId::Debug => checked
                        .capabilities()
                        .method_implementation(
                            ty,
                            crate::stdlib::StdlibCapabilityId::Debug,
                            crate::stdlib::StdlibItemId::DebugDebugString,
                            context.semantics(),
                        )
                        .is_some()
                        .then_some("custom")
                        .or_else(|| {
                            checked
                                .capabilities()
                                .has_derived_debug(ty, context.semantics())
                                .then_some("derived")
                        }),
                    _ => None,
                };
                if let Some(implementation) = implementation {
                    format!("`{name}` ({implementation})")
                } else {
                    format!("`{name}`")
                }
            })
            .collect::<Vec<_>>()
            .join(", "),
    );
}

fn render_source_hover(definition: &SourceDefinition, context: &SemanticContext) -> Option<String> {
    let syntax = context.syntax();
    let semantics = context.semantics();
    match definition.id {
        SourceDefinitionId::State => Some(source_markdown(
            "current / old: state snapshot",
            "`current` contains the latest accepted state and permits direct field replacement; `old` is the read-only preceding snapshot. Failed fields retain their accepted values.",
        )),
        SourceDefinitionId::Settings => Some(source_markdown(
            "settings / oldSettings: settings view",
            "Read-only, allocation-free views. `settings` selects the latest host settings and `oldSettings` selects their values from the preceding update.",
        )),
        SourceDefinitionId::Value(value) => {
            let ty = semantics.value_type(value)?;
            let ty = render_type(ty, context);
            let (signature, description) = if syntax
                .state
                .as_ref()
                .is_some_and(|state| state.provider_value == Some(value))
            {
                let name = syntax
                    .state
                    .as_ref()
                    .and_then(|state| state.refinement_value_name())
                    .unwrap_or("provider");
                let description = "Read-only state provider selected for the attached process. Match on it to refine provider-specific state fields and roots.";
                (format!("{name}: {ty}"), description.to_owned())
            } else if let Some(global) = syntax.globals.iter().find(|global| {
                let mut contains = false;
                global
                    .binding
                    .visit_bindings(&mut |binding| contains |= binding.id == value);
                contains
            }) {
                let scoped = context
                    .snapshot
                    .checked()
                    .and_then(|checked| checked.scoped_globals().lifetime(value));
                let kind = match scoped {
                    Some(crate::GlobalLifetime::Attachment) => {
                        if context.snapshot.checked().is_some_and(|checked| {
                            checked.scoped_globals().is_compiler_initialized(value)
                        }) {
                            "Attachment-scoped global variable; initialized automatically from the attached provider's schema and cleared on detach"
                        } else {
                            "Attachment-scoped global variable; initialized by `onAttach` and cleared on detach"
                        }
                    }
                    Some(crate::GlobalLifetime::Attempt) => {
                        "Attempt-scoped global variable; initialized by `onStart` and cleared after `onReset`"
                    }
                    None => "Global variable",
                };
                let mut description = documented_description(kind, global.documentation.as_deref());
                if scoped == Some(crate::GlobalLifetime::Attachment)
                    && let Some(checked) = context.snapshot.checked()
                {
                    let attachment = checked.scoped_globals();
                    let layouts = attachment.available_shapes(value).collect::<Vec<_>>();
                    append_attachment_shapes(
                        &mut description,
                        syntax,
                        attachment,
                        &layouts,
                        attachment.shapes().len(),
                    );
                }
                (format!("let {}: {ty}", definition.name), description)
            } else if let Some(field) = syntax
                .state
                .as_ref()
                .and_then(|state| state.all_fields().find(|field| field.id == value))
            {
                let mut description = documented_description(
                    "Transactional state field",
                    field.documentation.as_deref(),
                );
                if let Some(predicate) = state_field_shape_availability(field.id, context) {
                    append_shape_availability(&mut description, &predicate, context);
                }
                (format!("current.{}: {ty}", definition.name), description)
            } else if let Some(setting) = syntax.settings.iter().find(|setting| setting.id == value)
            {
                let mut description = format!("Setting: {}", setting.description);
                if let Some(tooltip) = &setting.tooltip {
                    description.push_str("\n\n");
                    description.push_str(tooltip);
                }
                (format!("settings.{}: {ty}", definition.name), description)
            } else if syntax_parameter(syntax, value).is_some() {
                (format!("{}: {ty}", definition.name), "Parameter".to_owned())
            } else if syntax_for_binding(syntax, value).is_some() {
                (
                    format!("{}: {ty}", definition.name),
                    "Read-only loop binding".to_owned(),
                )
            } else {
                (
                    format!("let {}: {ty}", definition.name),
                    "Local variable".to_owned(),
                )
            };
            Some(source_markdown(&signature, &description))
        }
        SourceDefinitionId::StructField(field) => {
            let ty = semantics.struct_field_type(field)?;
            let ty = render_type(ty, context);
            let structure = syntax.structs.iter().find(|structure| {
                structure
                    .fields
                    .iter()
                    .any(|candidate| candidate.id == field)
            })?;
            let field = structure
                .fields
                .iter()
                .find(|candidate| candidate.id == field)?;
            let mut description =
                documented_description("Struct field", field.documentation.as_deref());
            if let Some((bit32, bit64)) = struct_field_memory_layout(structure, field.id, context) {
                if bit32 == bit64 {
                    let (offset, size) = bit32;
                    let unit = if size == 1 { "byte" } else { "bytes" };
                    description.push_str(&format!(
                        "\n\n**Process-memory layout:** byte offset `0x{offset:x}` from the start of `{}`; size `{size}` {unit}.",
                        structure.name
                    ));
                } else {
                    description.push_str(&format!(
                        "\n\n**Process-memory layout:** 32-bit targets read byte offset `0x{:x}` with size `{}`; 64-bit targets read byte offset `0x{:x}` with size `{}`.",
                        bit32.0, bit32.1, bit64.0, bit64.1
                    ));
                }
            }
            Some(source_markdown(
                &format!("{}.{}: {ty}", structure.name, definition.name),
                &description,
            ))
        }
        SourceDefinitionId::Function(function) => {
            let function = syntax
                .functions
                .iter()
                .find(|candidate| candidate.id == function)?;
            let receiver = function.method_of.and_then(|_| {
                function
                    .params
                    .first()
                    .and_then(|parameter| semantics.value_type(parameter.id))
                    .map(|ty| render_type(ty, context))
            });
            let parameters = function
                .params
                .iter()
                .filter(|parameter| receiver.is_none() || parameter.name != "self")
                .map(|parameter| {
                    let ty = semantics.value_type(parameter.id)?;
                    Some(format!("{}: {}", parameter.name, render_type(ty, context)))
                })
                .collect::<Option<Vec<_>>>()?;
            let result = semantics.function_result(function.id)?;
            let associated_projections = semantics.function_associated_projections(function.id);
            let bounds = semantics
                .function_type_parameters(function.id)
                .iter()
                .filter_map(|parameter| {
                    let constraints = context
                        .standard_library
                        .minimal_capabilities(semantics.generic_parameter_constraints(*parameter));
                    (!constraints.is_empty()).then(|| {
                        format!(
                            "{}: {}",
                            associated_projections
                                .iter()
                                .find(|projection| projection.output == *parameter)
                                .map(|projection| format!(
                                    "{}.{}",
                                    render_type(projection.receiver, context),
                                    projection.name
                                ))
                                .unwrap_or_else(|| render_type(*parameter, context)),
                            constraints
                                .iter()
                                .map(|constraint| {
                                    context.standard_library.capability(*constraint).name
                                })
                                .collect::<Vec<_>>()
                                .join(" + ")
                        )
                    })
                })
                .collect::<Vec<_>>();
            let name = receiver.map_or_else(
                || function.name.clone(),
                |receiver| format!("{receiver}.{}", function.name),
            );
            let mut description = source_function_description(
                function.method_of.is_some(),
                function.debug_only,
                function.documentation.as_deref(),
                context
                    .effects()
                    .map(|effects| effects.function(function.id)),
            );
            if let Some(operation) = context
                .effects()
                .map(|effects| effects.function(function.id))
            {
                append_parameter_effect_dependencies(
                    &mut description,
                    function,
                    &operation,
                    context,
                );
            }
            if let Some(checked) = context.snapshot.checked() {
                let attachment = checked.scoped_globals();
                let allowed = attachment.function_shapes(function.id).collect::<Vec<_>>();
                append_attachment_shapes(
                    &mut description,
                    syntax,
                    attachment,
                    &allowed,
                    attachment.shapes().len(),
                );
                if attachment.function_requires_attempt(function.id) {
                    description.push_str(
                        "\n\n**Attempt state:** requires an attempt initialized by `onStart`.",
                    );
                }
            }
            Some(source_markdown(
                &format!(
                    "fn {name}({}) -> {}{}",
                    parameters.join(", "),
                    render_type(result, context),
                    if bounds.is_empty() {
                        String::new()
                    } else {
                        format!(" where {}", bounds.join(", "))
                    }
                ),
                &description,
            ))
        }
        SourceDefinitionId::Struct(structure) => {
            let structure = syntax
                .structs
                .iter()
                .find(|candidate| candidate.id == structure)?;
            let mut description =
                documented_description("Struct type", structure.documentation.as_deref());
            append_source_capabilities(
                &mut description,
                semantics.types().id_for_struct(structure.id),
                context,
            );
            Some(source_markdown(
                &format!("struct {}", structure.name),
                &description,
            ))
        }
        SourceDefinitionId::Enum(enumeration) => {
            let enumeration = syntax
                .enum_declarations()
                .find(|candidate| candidate.id == enumeration)?;
            let mut description =
                documented_description("Enum type", enumeration.documentation.as_deref());
            append_source_capabilities(
                &mut description,
                semantics.types().id_for_enum(enumeration.id),
                context,
            );
            let representation = semantics
                .enum_representation(enumeration.id)
                .map(|ty| render_type(ty, context));
            if let Some(checked) = context.snapshot.checked()
                && let Ok(layout) = checked.memory_layouts().enumeration(enumeration.id)
            {
                description.push_str(&format!(
                    "\n\n**Process-memory representation:** `{}` ({} byte{}). Unknown discriminants make the enclosing memory read fail.",
                    representation.as_deref().unwrap_or("<unknown>"),
                    layout.size,
                    if layout.size == 1 { "" } else { "s" },
                ));
            }
            Some(source_markdown(
                &format!(
                    "enum {}{}",
                    enumeration.name,
                    representation.map_or_else(String::new, |ty| format!(": {ty}"))
                ),
                &description,
            ))
        }
        SourceDefinitionId::EnumVariant(variant) => {
            let enumeration = syntax.enum_declarations().find(|enumeration| {
                enumeration
                    .variants
                    .iter()
                    .any(|candidate| candidate.id == variant)
            })?;
            let payload = semantics
                .enum_variant_payload(variant)
                .map(|ty| format!("({})", render_type(ty, context)));
            let variant_declaration = enumeration
                .variants
                .iter()
                .find(|candidate| candidate.id == variant)?;
            let mut description = documented_description(
                "Enum variant",
                variant_declaration.documentation.as_deref(),
            );
            let discriminant = context
                .snapshot
                .checked()
                .and_then(|checked| checked.memory_layouts().enumeration(enumeration.id).ok())
                .and_then(|layout| {
                    layout
                        .variants
                        .iter()
                        .find(|candidate| candidate.variant == variant)
                })
                .map(|layout| layout.value);
            if let Some(discriminant) = discriminant {
                description.push_str(&format!(
                    "\n\n**Process-memory discriminant:** `{discriminant}`."
                ));
            }
            Some(source_markdown(
                &format!(
                    "{}.{}{}{}",
                    enumeration.name,
                    definition.name,
                    payload.as_deref().unwrap_or_default(),
                    discriminant.map_or_else(String::new, |value| format!(" = {value}")),
                ),
                &description,
            ))
        }
        id @ (SourceDefinitionId::ManagedImage(_)
        | SourceDefinitionId::ManagedNamespace(_)
        | SourceDefinitionId::ManagedClass(_)
        | SourceDefinitionId::ManagedField(_)) => match find_managed_declaration(syntax, id)? {
            ManagedSourceDeclaration::Image(image) => Some(source_markdown(
                &format!("image \"{}\"", image.name),
                &documented_description("Managed image schema", image.documentation.as_deref()),
            )),
            ManagedSourceDeclaration::Namespace(namespace) => Some(source_markdown(
                &format!("namespace {}", namespace.name),
                &documented_description(
                    "Managed metadata namespace",
                    namespace.documentation.as_deref(),
                ),
            )),
            ManagedSourceDeclaration::Class(class) => Some(source_markdown(
                &format!("class {}", class.name),
                &documented_description(
                    "Managed reference and snapshot schema",
                    class.documentation.as_deref(),
                ),
            )),
            ManagedSourceDeclaration::Field(field) => {
                let mut description = documented_description(
                    if field.is_static {
                        "Static managed field"
                    } else {
                        "Managed instance field"
                    },
                    field.documentation.as_deref(),
                );
                if let Some(predicate) = semantics.managed_field_shape_predicate(field.id) {
                    append_shape_availability(&mut description, predicate, context);
                }
                Some(source_markdown(
                    &format!(
                        "{}{} {}",
                        if field.is_static { "static " } else { "" },
                        semantics
                            .managed_field_type(field.id)
                            .map(|ty| render_type(ty, context))
                            .unwrap_or_else(|| "<unknown>".to_owned()),
                        field.name
                    ),
                    &description,
                ))
            }
        },
    }
}

enum ManagedSourceDeclaration<'ast> {
    Image(&'ast crate::ast::ManagedImageDecl),
    Namespace(&'ast crate::ast::ManagedNamespaceDecl),
    Class(&'ast crate::ast::ManagedClassDecl),
    Field(&'ast crate::ast::ManagedFieldDecl),
}

fn find_managed_declaration(
    syntax: &crate::ast::Program,
    target: SourceDefinitionId,
) -> Option<ManagedSourceDeclaration<'_>> {
    struct Finder<'ast> {
        target: SourceDefinitionId,
        found: Option<ManagedSourceDeclaration<'ast>>,
    }

    impl<'ast> crate::visit::Visitor<'ast> for Finder<'ast> {
        fn visit_managed_image(&mut self, image: &'ast crate::ast::ManagedImageDecl) {
            if self.target == SourceDefinitionId::ManagedImage(image.id) {
                self.found = Some(ManagedSourceDeclaration::Image(image));
            } else {
                crate::visit::walk_managed_image(self, image);
            }
        }

        fn visit_managed_namespace(&mut self, namespace: &'ast crate::ast::ManagedNamespaceDecl) {
            if self.target == SourceDefinitionId::ManagedNamespace(namespace.id) {
                self.found = Some(ManagedSourceDeclaration::Namespace(namespace));
            } else {
                crate::visit::walk_managed_namespace(self, namespace);
            }
        }

        fn visit_managed_class(&mut self, class: &'ast crate::ast::ManagedClassDecl) {
            if self.target == SourceDefinitionId::ManagedClass(class.id) {
                self.found = Some(ManagedSourceDeclaration::Class(class));
            } else {
                crate::visit::walk_managed_class(self, class);
            }
        }

        fn visit_managed_field(&mut self, field: &'ast crate::ast::ManagedFieldDecl) {
            if self.target == SourceDefinitionId::ManagedField(field.id) {
                self.found = Some(ManagedSourceDeclaration::Field(field));
            }
        }
    }

    let mut finder = Finder {
        target,
        found: None,
    };
    crate::visit::Visitor::visit_program(&mut finder, syntax);
    finder.found
}

fn append_parameter_effect_dependencies(
    description: &mut String,
    function: &crate::ast::FunctionDecl,
    operation: &FunctionOperationSemantics,
    context: &SemanticContext,
) {
    if operation.latent_parameter_operations.is_empty() {
        return;
    }
    let dependencies = operation
        .latent_parameter_operations
        .iter()
        .filter_map(|latent| {
            let parameter = function.params.get(latent.parameter)?;
            let mut path = parameter.name.clone();
            for member in &latent.fields {
                let name = match member {
                    ResolvedMember::StructField(field) => context
                        .syntax()
                        .structs
                        .iter()
                        .flat_map(|structure| &structure.fields)
                        .find(|candidate| candidate.id == *field)
                        .map(|field| field.name.as_str()),
                    ResolvedMember::ManagedField(field) => context
                        .syntax()
                        .managed_class_declarations()
                        .into_iter()
                        .flat_map(|class| &class.fields)
                        .find(|candidate| candidate.id == *field)
                        .map(|field| field.name.as_str()),
                    ResolvedMember::StandardField(field) => {
                        Some(context.standard_library.field(*field).name)
                    }
                    ResolvedMember::StateField(field) => context
                        .syntax()
                        .state
                        .iter()
                        .flat_map(|state| state.all_fields())
                        .find(|candidate| candidate.id == *field)
                        .map(|field| field.name.as_str()),
                    ResolvedMember::SettingField(setting) => context
                        .syntax()
                        .settings
                        .iter()
                        .find(|candidate| candidate.id == *setting)
                        .map(|setting| setting.name.as_str()),
                }?;
                path.push('.');
                path.push_str(name);
            }
            Some(match latent.kind {
                crate::effects::LatentOperationKind::Invoke => format!("invokes `{path}`"),
                crate::effects::LatentOperationKind::Iterate => format!("iterates `{path}`"),
            })
        })
        .collect::<Vec<_>>();
    if dependencies.is_empty() {
        return;
    }
    description.push_str("\n\n**Effect dependencies:** ");
    description.push_str(&dependencies.join("; "));
    description.push_str(". Concrete effects are determined at each call site.");
}

fn source_function_description(
    is_method: bool,
    debug_only: bool,
    documentation: Option<&str>,
    operation: Option<FunctionOperationSemantics>,
) -> String {
    let mut description =
        documented_description(if is_method { "Method" } else { "Function" }, documentation);
    if debug_only {
        description.push_str("\n\n**Build availability:** Debug builds only");
    }
    if let Some(operation) = operation {
        description.push_str("\n\n**Effects:** ");
        description.push_str(
            &operation
                .effects
                .iter()
                .map(|effect| effect.name())
                .collect::<Vec<_>>()
                .join(", "),
        );
        description.push_str("\n\n**Runtime behavior:** ");
        description.push_str(match operation.suspension {
            crate::stdlib::SuspensionKind::None => "synchronous",
            crate::stdlib::SuspensionKind::Suspends => "suspends",
        });
        if operation.availability == crate::stdlib::Availability::OnAttach {
            description.push_str("; available in suspending attachment code");
        }
        if operation.requires_attached_process {
            description.push_str("; requires an attached process and is unavailable in `onDetach`");
        }
        if operation.requires_state_snapshots {
            description.push_str("; requires committed `old` and `current` state snapshots");
        }
        if operation.cancellation == crate::stdlib::CancellationKind::ProcessClose {
            description.push_str("; cancels when the process closes");
        }
    }
    description
}

fn documented_description(kind: &str, documentation: Option<&str>) -> String {
    let mut description = kind.to_owned();
    if let Some(documentation) = documentation {
        description.push_str("\n\n");
        description.push_str(documentation);
    }
    description
}

fn source_markdown(signature: &str, description: &str) -> String {
    format!("```splitscript\n{signature}\n```\n\n{description}")
}

fn render_stdlib_hover(
    standard_library: StandardLibrary,
    item: StdlibItemId,
    substitutions: Vec<(&str, String)>,
) -> String {
    StandardLibraryDocumentation::generate_with_library(&standard_library, item, &substitutions)
        .hover_markdown()
}

fn render_stdlib_symbol_hover(library: StandardLibrary, symbol: StdlibSymbolId) -> String {
    render_stdlib_symbol_hover_with_form(library, symbol, None)
}

fn render_stdlib_symbol_hover_with_form(
    library: StandardLibrary,
    symbol: StdlibSymbolId,
    form_override: Option<&str>,
) -> String {
    let (form, documentation) = match symbol {
        StdlibSymbolId::StateProvider(id) => {
            let declaration = library.state_provider(id);
            let value_type = library.type_decl(declaration.process_type);
            let state_form = match declaration.processes {
                StateProviderProcesses::SourceState => "state \"game.exe\" { ... }".to_owned(),
                StateProviderProcesses::Declared(_) => {
                    format!("state {} {{ ... }}", declaration.name)
                }
            };
            (
                format!(
                    "{state_form}\n{}: {}",
                    declaration.value_name, value_type.name
                ),
                &declaration.documentation,
            )
        }
        StdlibSymbolId::Namespace(id) => {
            let declaration = library.namespace(id);
            (declaration.name.to_owned(), &declaration.documentation)
        }
        StdlibSymbolId::Capability(id) => {
            let declaration = library.capability(id);
            (declaration.name.to_owned(), &declaration.documentation)
        }
        StdlibSymbolId::TypeConstructor(id) => {
            let declaration = library.type_constructor(id);
            let mut form = form_override
                .map(str::to_owned)
                .unwrap_or_else(|| library.render_type_constructor(id));
            if form_override.is_none()
                && declaration.syntax == crate::stdlib::TypeConstructorSyntax::Named
                && !declaration.parameters.is_empty()
            {
                form = declaration.name.to_owned();
                form.push('<');
                for (index, parameter) in declaration.parameters.iter().enumerate() {
                    if index != 0 {
                        form.push_str(", ");
                    }
                    form.push_str(parameter.name);
                    let constraints = library.minimal_capabilities(parameter.constraints);
                    if !constraints.is_empty() {
                        form.push_str(": ");
                        for (constraint_index, constraint) in constraints.iter().enumerate() {
                            if constraint_index != 0 {
                                form.push_str(" + ");
                            }
                            form.push_str(library.capability(*constraint).name);
                        }
                    }
                }
                form.push('>');
            }
            (form, &declaration.documentation)
        }
        StdlibSymbolId::Type(id) => {
            let declaration = library.type_decl(id);
            (declaration.name.to_owned(), &declaration.documentation)
        }
        StdlibSymbolId::Field(id) => {
            let declaration = library.field(id);
            (
                format!(
                    "{}.{}: {}",
                    library.render_field_owner(declaration.owner),
                    declaration.name,
                    library.render_type(declaration.ty)
                ),
                &declaration.documentation,
            )
        }
        StdlibSymbolId::Variant(id) => {
            let declaration = library.variant(id);
            let owner = library.type_decl(declaration.owner);
            (
                format!("{}.{}", owner.name, declaration.name),
                &declaration.documentation,
            )
        }
        StdlibSymbolId::Item(id) => return render_stdlib_hover(library, id, Vec::new()),
    };
    let prose = crate::documentation::prose_markdown(documentation.summary, documentation.details);
    let mut markdown = format!(
        "```splitscript\n{form}\n```\n\n{}",
        crate::documentation::strip_intra_doc_links(&prose)
    );
    if let StdlibSymbolId::StateProvider(id) = symbol
        && library.state_provider(id).validation.is_some()
    {
        markdown.push_str("\n\n");
        markdown.push_str(&crate::documentation::strip_intra_doc_links(
            crate::stdlib::STATE_PROVIDER_MAPPING_LIFECYCLE_NOTE,
        ));
    }
    append_examples(&mut markdown, documentation.examples);
    markdown
}

fn render_language_hover_with_form(item: &LanguageItem, form_override: Option<&str>) -> String {
    let prose = crate::documentation::prose_markdown(
        item.documentation.summary,
        item.documentation.details,
    );
    let mut markdown = format!(
        "```splitscript\n{}\n```\n\n{}",
        form_override.unwrap_or(item.form),
        crate::documentation::strip_intra_doc_links(&prose)
    );
    append_examples(&mut markdown, item.documentation.examples);
    markdown
}

fn append_examples(markdown: &mut String, examples: &[crate::catalog::Example]) {
    if examples.is_empty() {
        return;
    }
    markdown.push_str("\n\n**Examples**");
    for example in examples {
        markdown.push_str(&format!(
            "\n\n_{}_\n\n```splitscript\n{}\n```",
            example.title, example.source
        ));
    }
}

#[derive(Debug)]
struct CallSite {
    open: Span,
    callee: Vec<String>,
    explicit_type_arguments: Vec<String>,
    method_dot: Option<usize>,
    active_parameter: usize,
}

fn active_call(document: &SourceDocument, offset: usize) -> Option<CallSite> {
    let tokens = document
        .tokens()
        .filter(|token| token.span.start < offset && token.kind != TokenKind::Eof)
        .collect::<Vec<_>>();
    let mut parentheses = Vec::new();
    for (index, token) in tokens.iter().enumerate() {
        match token.kind {
            TokenKind::LParen => parentheses.push(index),
            TokenKind::RParen => {
                parentheses.pop();
            }
            _ => {}
        }
    }
    let open_index = *parentheses.last()?;
    let (callee, explicit_type_arguments, method_dot) = callee_before(&tokens, open_index)?;
    let active_parameter = active_parameter(&tokens[open_index + 1..]);
    Some(CallSite {
        open: tokens[open_index].span,
        callee,
        explicit_type_arguments,
        method_dot,
        active_parameter,
    })
}

fn callee_before(
    tokens: &[&Token],
    open: usize,
) -> Option<(Vec<String>, Vec<String>, Option<usize>)> {
    let mut cursor = open.checked_sub(1)?;
    let mut explicit_type_arguments = Vec::new();
    if tokens[cursor].kind == TokenKind::Gt {
        let mut depth = 1usize;
        while cursor > 0 {
            cursor -= 1;
            match tokens[cursor].kind {
                TokenKind::Gt => depth += 1,
                TokenKind::Lt => {
                    depth -= 1;
                    if depth == 0 {
                        cursor = cursor.checked_sub(1)?;
                        break;
                    }
                }
                TokenKind::Ident(ref name) if depth == 1 => {
                    explicit_type_arguments.push(name.clone())
                }
                _ => {}
            }
        }
        if depth != 0 {
            return None;
        }
        explicit_type_arguments.reverse();
    }
    let method_dot = cursor
        .checked_sub(1)
        .filter(|index| tokens[*index].kind == TokenKind::Dot)
        .map(|index| tokens[index].span.start);
    let mut reversed = Vec::new();
    while let TokenKind::Ident(name) = &tokens[cursor].kind {
        reversed.push(name.clone());
        let Some(dot) = cursor.checked_sub(1) else {
            break;
        };
        if tokens[dot].kind != TokenKind::Dot {
            break;
        }
        let Some(previous) = dot.checked_sub(1) else {
            break;
        };
        cursor = previous;
    }
    (!reversed.is_empty()).then(|| {
        reversed.reverse();
        (reversed, explicit_type_arguments, method_dot)
    })
}

fn infer_method_call(
    source: &str,
    call: &CallSite,
    compiler_context: &crate::CompilerContext,
) -> Option<(StdlibItemId, Vec<TypeId>, SemanticContext)> {
    let method_dot = call.method_dot?;
    let line_end = source[call.open.start..]
        .find('\n')
        .map_or(source.len(), |relative| call.open.start + relative);
    let mut probe_source = String::with_capacity(source.len());
    probe_source.push_str(&source[..method_dot]);
    probe_source.push_str(&source[line_end..]);
    let mut probe = CompilerDatabase::with_context(compiler_context.clone(), probe_source);
    let line_start = source[..method_dot]
        .rfind(['\n', '\r'])
        .map_or(0, |offset| offset + 1);
    let analysis = (line_start..method_dot)
        .rev()
        .find_map(|offset| probe.analysis_at(offset).ok().flatten());
    let analysis = analysis?;
    let semantic = semantic_context(&mut probe);
    let semantic = semantic?;
    let applicable = compiler_context
        .standard_library()
        .methods_for_type(&analysis.type_kind);
    let method_name = call.callee.last()?;
    let item = applicable.into_iter().find(|item| {
        matches!(item.kind, crate::stdlib::ItemKind::Method { .. }) && item.name == method_name
    })?;
    let type_arguments = if call.explicit_type_arguments.is_empty() {
        inferred_method_type_arguments(item, analysis.ty, &analysis.type_kind)
    } else {
        call.explicit_type_arguments
            .iter()
            .filter_map(|name| {
                compiler_context
                    .standard_library()
                    .core_types()
                    .iter()
                    .find(|ty| ty.name == name)
                    .map(|ty| semantic.semantics().types().id_for_core(ty.id))
            })
            .collect()
    };
    Some((item.id, type_arguments, semantic))
}

fn inferred_method_type_arguments(
    item: &StdlibItem,
    receiver: TypeId,
    receiver_kind: &TypeKind,
) -> Vec<TypeId> {
    let declared = match item.kind {
        crate::stdlib::ItemKind::Method { receiver } => receiver,
        crate::stdlib::ItemKind::Function | crate::stdlib::ItemKind::Constant => {
            return Vec::new();
        }
    };
    item.signature
        .type_parameters
        .iter()
        .filter_map(|parameter| {
            catalog_receiver_argument(declared, parameter.name, receiver, receiver_kind)
        })
        .collect()
}

fn catalog_receiver_argument(
    declared: CatalogTypeRef,
    parameter: &str,
    receiver: TypeId,
    receiver_kind: &TypeKind,
) -> Option<TypeId> {
    match (declared, receiver_kind) {
        (CatalogTypeRef::Parameter(name), _) if name == parameter => Some(receiver),
        (
            CatalogTypeRef::Application {
                constructor,
                arguments: [CatalogTypeRef::Parameter(name)],
            },
            TypeKind::Array {
                element: actual, ..
            },
        ) if constructor == crate::stdlib::StdlibTypeConstructorId::Array && *name == parameter => {
            Some(*actual)
        }
        (
            CatalogTypeRef::Application {
                constructor,
                arguments: [CatalogTypeRef::Parameter(name)],
            },
            TypeKind::Option { value: actual, .. },
        ) if constructor == crate::stdlib::StdlibTypeConstructorId::Option
            && *name == parameter =>
        {
            Some(*actual)
        }
        (
            CatalogTypeRef::Application {
                constructor,
                arguments: [CatalogTypeRef::Parameter(name)],
            },
            TypeKind::Result { value: actual, .. },
        ) if constructor == crate::stdlib::StdlibTypeConstructorId::Result
            && *name == parameter =>
        {
            Some(*actual)
        }
        _ => None,
    }
}

fn active_parameter(tokens: &[&Token]) -> usize {
    let mut nesting = 0usize;
    let mut parameter = 0usize;
    for token in tokens {
        match token.kind {
            TokenKind::LParen | TokenKind::LBracket | TokenKind::LBrace => nesting += 1,
            TokenKind::RParen | TokenKind::RBracket | TokenKind::RBrace => {
                nesting = nesting.saturating_sub(1);
            }
            TokenKind::Comma if nesting == 0 => parameter += 1,
            _ => {}
        }
    }
    parameter
}

fn call_expression_at<'a>(syntax: &'a Program, open: Span, callee: &[String]) -> Option<&'a Expr> {
    let mut collector = CallCollector { calls: Vec::new() };
    collector.visit_program(syntax);
    collector
        .calls
        .into_iter()
        .filter(|expression| {
            expression.span.start <= open.start
                && open.end <= expression.span.end
                && matches!(&expression.kind, ExprKind::Call { callee: path, .. } if path == callee)
        })
        .min_by_key(|expression| expression.span.end - expression.span.start)
}

struct CallCollector<'ast> {
    calls: Vec<&'ast Expr>,
}

impl<'ast> Visitor<'ast> for CallCollector<'ast> {
    fn visit_expr(&mut self, expression: &'ast Expr) {
        if matches!(expression.kind, ExprKind::Call { .. }) {
            self.calls.push(expression);
        }
        visit::walk_expr(self, expression);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn hover_uses_resolved_catalog_signature_effects_and_examples() {
        let source = r#"
enum Edition { Alternate }
let edition: Edition
state "game.exe" {}
onAttach { edition = Edition.Alternate }

whileAttached {
    let value: i32 = 8
    let bounded = value.clamp(0, 7)
}
"#;
        let offset = source.find("clamp").unwrap() + 2;
        let mut database = CompilerDatabase::new(source);
        let hover = database.hover(offset).unwrap().expect("catalog hover");
        assert!(hover.markdown.contains("i32.clamp"));
        assert!(hover.markdown.contains("Self = i32"));
        assert!(hover.markdown.contains("**Parameters**"));
        assert!(hover.markdown.contains("**Effects:** pure"));
        assert!(!hover.markdown.contains("**Effects:** pure."));
        assert!(
            !hover
                .markdown
                .contains("**Runtime behavior:** available everywhere; synchronous.")
        );
        assert!(hover.markdown.contains("**Examples**"));
        assert!(
            hover
                .markdown
                .contains("let visibleStage = 9u32.clamp(1, 7)")
        );
        assert!(!hover.markdown.contains("value.min"));
        assert!(!hover.markdown.contains("value.max"));
        assert!(!hover.markdown.contains("setTickRate"));
    }

    #[test]
    fn hover_describes_source_defined_associated_constants() {
        let source = r#"
state "game.exe" {}
setup {
    let value = f32.NaN
}
"#;
        let mut database = CompilerDatabase::new(source);
        let hover = database
            .hover(source.find("NaN").unwrap() + 1)
            .unwrap()
            .expect("associated constant hover");
        assert!(hover.markdown.contains("f32.NaN: f32"));
        assert!(
            hover
                .markdown
                .contains("The canonical 32-bit not-a-number value")
        );
        assert!(hover.markdown.contains("let measurement = f32.NaN"));
    }

    #[test]
    fn hover_reflows_wrapped_doc_comments_within_one_paragraph() {
        let source = r#"
state "game.exe" {}
onDetach {
    timer.pauseGameTime()
}
"#;
        let mut database = CompilerDatabase::new(source);
        let hover = database
            .hover(source.find("pauseGameTime").unwrap() + 2)
            .unwrap()
            .expect("timer hover");
        assert!(hover.markdown.contains(
            "This explicit operation is intended for lifecycle transitions such as ensuring game time does not advance after the attached process closes."
        ));
        assert!(!hover.markdown.contains("explicit\n\noperation"));
        assert!(!hover.markdown.contains("game\n\ntime"));
    }

    #[test]
    fn method_self_hover_combines_receiver_type_and_language_documentation() {
        let source = r#"
struct Position {
    x: i32,
}

state "game.exe" {}

fn Position.value() -> i32 {
    return self.x
}
"#;
        let offset = source.find("self.x").unwrap() + 2;
        let mut database = CompilerDatabase::new(source);
        let hover = database.hover(offset).unwrap().expect("self hover");
        assert!(
            hover.markdown.contains("self: Position"),
            "{}",
            hover.markdown
        );
        assert!(
            hover.markdown.contains("current method receiver"),
            "{}",
            hover.markdown
        );
        assert_eq!(
            hover.documentation_uri.as_deref(),
            Some("/language/self.md")
        );
    }

    #[test]
    fn string_ascii_case_conversion_hover_comes_from_the_catalog() {
        let source = r#"
state "game.exe" {}
whileAttached {
    let map = "MAP_A".toAsciiLowerCase()
}
"#;
        let mut database = CompilerDatabase::new(source);
        let hover = database
            .hover(source.find("toAsciiLowerCase").unwrap() + 2)
            .unwrap()
            .expect("string conversion hover");
        assert!(
            hover
                .markdown
                .contains("String.toAsciiLowerCase() -> String")
        );
        assert!(hover.markdown.contains("Only `A` through `Z` are changed"));
        assert!(hover.markdown.contains("**Effects:** allocates"));
        assert!(
            hover
                .markdown
                .contains("let map = \"FOREST\".toAsciiLowerCase()")
        );
    }

    #[test]
    fn hover_uses_derived_effects_for_source_defined_catalog_functions() {
        let source = r#"
state "game.exe" {}
fn resolve(module: Module) -> address! {
    return module.readRelative32(0)
}
whileAttached {
    let running = timer.isRunning()
}
"#;
        let mut database = CompilerDatabase::new(source);
        let relative = database
            .hover(source.find("readRelative32").unwrap() + 2)
            .unwrap()
            .expect("source-defined method hover");
        assert!(
            relative
                .markdown
                .contains("**Effects:** reads process memory, requires an attached process")
        );
        assert!(relative.markdown.contains(
            "**Runtime behavior:** available while a process is attached, except in onDetach; synchronous"
        ));

        let timer = database
            .hover(source.find("isRunning").unwrap() + 2)
            .unwrap()
            .expect("source-defined function hover");
        assert!(timer.markdown.contains("**Effects:** reads timer state"));
        assert!(
            timer
                .markdown
                .contains("**Runtime behavior:** available everywhere; synchronous")
        );
    }

    #[test]
    fn hover_shows_transitive_state_snapshot_requirements() {
        let source = r#"
state "game.exe" { level: u32 at 0x100 }

fn changed() {
    return old.level != current.level
}

fn relay() {
    return changed()
}

split {
    return relay()
}
"#;
        let mut database = CompilerDatabase::new(source);
        let hover = database
            .hover(source.rfind("relay").unwrap() + 2)
            .unwrap()
            .expect("relay hover");
        assert!(
            hover
                .markdown
                .contains("**Effects:** requires state snapshots")
        );
        assert!(hover.markdown.contains(
            "**Runtime behavior:** synchronous; requires committed `old` and `current` state snapshots"
        ));
    }

    #[test]
    fn namespace_hover_uses_catalog_documentation() {
        let source = r#"
state "game.exe" {}

whileAttached {
    let running = timer.isRunning()
}
"#;
        let mut database = CompilerDatabase::new(source);
        let hover = database
            .hover(source.find("timer").unwrap() + 2)
            .unwrap()
            .expect("namespace hover");
        let start = source.find("timer").unwrap();
        assert_eq!(
            hover.span,
            Span {
                start,
                end: start + 5
            }
        );
        assert!(hover.markdown.contains("```splitscript\ntimer\n```"));
        assert!(hover.markdown.contains("Reads information from the timer"));
        assert!(
            hover
                .markdown
                .contains("let currentTimerState = timer.state()")
        );
    }

    #[test]
    fn language_hover_is_served_from_the_language_catalog() {
        let source = "state \"game.exe\" {}\nwhileAttached { let version = v\"1.2.3.4\"; retry process.read<i32>(0) }";
        let offset = source.find("retry").unwrap() + 1;
        let mut database = CompilerDatabase::new(source);
        let hover = database.hover(offset).unwrap().expect("language hover");
        assert!(
            hover
                .markdown
                .contains("let value = retry fallibleExpression")
        );
        assert!(hover.markdown.contains("Retries synchronous fallible work"));
        assert!(hover.markdown.contains("let health = retry"));
        assert!(!hover.markdown.contains("fn readMarker"));

        let version = database
            .hover(source.find("v\"").unwrap())
            .unwrap()
            .expect("version-literal hover");
        assert!(version.markdown.contains("v\"major.minor.build.private\""));
        assert!(
            version
                .markdown
                .contains("exactly four decimal `u16` components")
        );
    }

    #[test]
    fn is_patterns_share_hover_navigation_references_and_rename_identity() {
        let source = r#"state "game.exe" {}
fn inspect(value: u32?) {
    if value is Some(number) && number > 0 {
        print(number)
    }
}"#;
        let declaration = source.find("number)").unwrap();
        let condition_use = source.find("number >").unwrap();
        let body_use = source.rfind("number)").unwrap();
        let mut database = CompilerDatabase::new(source);

        let keyword = database
            .hover(source.find(" is ").unwrap() + 2)
            .unwrap()
            .expect("`is` should have language documentation");
        assert!(keyword.markdown.contains("expression is pattern"));
        assert!(keyword.markdown.contains("control-flow paths"));
        assert_eq!(
            keyword.documentation_uri.as_deref(),
            Some("/language/is.md")
        );

        let binding = database
            .hover(declaration + 1)
            .unwrap()
            .expect("the pattern declaration should expose its inferred type");
        assert!(
            binding.markdown.contains("number: u32"),
            "{}",
            binding.markdown
        );

        for usage in [condition_use, body_use] {
            assert!(matches!(
                database.definition_at(usage + 1).unwrap(),
                Some(crate::database::DefinitionTarget::Source(definition))
                    if definition.span.start == declaration
            ));
        }
        let references = database.references_at(declaration + 1, true).unwrap();
        assert_eq!(references.len(), 3, "{references:#?}");
        let rename = database
            .rename_at(declaration + 1, "present")
            .expect("conditional bindings should be ordinary rename identities");
        assert_eq!(rename.edits.len(), 3, "{rename:#?}");
    }

    #[test]
    fn destructured_declarations_have_leaf_hover_navigation_and_rename_identity() {
        let source = r#"
struct Point { x: i32, y: i32 }
state "game.exe" {}
setup {
    let Point { x: horizontal, y: vertical } = Point { x: 3, y: 4 }
    print(horizontal + vertical)
}
"#;
        let declaration = source.find("horizontal,").unwrap();
        let usage = source.rfind("horizontal").unwrap();
        let mut database = CompilerDatabase::new(source);

        let hover = database
            .hover(declaration + 1)
            .unwrap()
            .expect("binding hover");
        assert!(
            hover.markdown.contains("let horizontal: i32"),
            "{}",
            hover.markdown
        );
        assert!(matches!(
            database.definition_at(usage + 1).unwrap(),
            Some(crate::database::DefinitionTarget::Source(definition))
                if definition.span.start == declaration
        ));
        let references = database.references_at(declaration + 1, true).unwrap();
        assert_eq!(references.len(), 2, "{references:#?}");
        let rename = database.rename_at(declaration + 1, "left").unwrap();
        assert_eq!(rename.edits.len(), 2, "{rename:#?}");
    }

    #[test]
    fn tick_rate_policy_and_fields_share_lifecycle_documentation() {
        let source = "state \"game.exe\" {}\ntickRate { attached: 60, detached: 2, }";
        let mut database = CompilerDatabase::new(source);
        for spelling in ["tickRate", "attached", "detached"] {
            let hover = database
                .hover(source.find(spelling).unwrap() + 1)
                .unwrap()
                .expect("tick-rate hover");
            assert!(
                hover
                    .markdown
                    .contains("tickRate { attached: 60, detached: 2 }")
            );
            assert!(hover.markdown.contains("1 Hz with no host process"));
            assert!(hover.markdown.contains("120 Hz while acquiring or polling"));
        }
    }

    #[test]
    fn floating_point_literal_hover_shows_the_resolved_width_and_exact_bits() {
        let source = r#"state "game.exe" {}
whileAttached {
    let smallest32: f32 = 1e-45
    let smallest64: f64 = 5e-324
    let ordinary = 1.25
}"#;
        let mut database = CompilerDatabase::new(source);

        let single_offset = source.find("1e-45").unwrap();
        let single = database.hover(single_offset).unwrap().expect("f32 hover");
        assert_eq!(
            single.span,
            Span {
                start: single_offset,
                end: single_offset + 5,
            }
        );
        assert!(single.markdown.contains("1e-45: f32"));
        assert!(single.markdown.contains("`0x00000001`"));

        let double_offset = source.find("5e-324").unwrap();
        let double = database.hover(double_offset).unwrap().expect("f64 hover");
        assert!(double.markdown.contains("5e-324: f64"));
        assert!(double.markdown.contains("`0x0000000000000001`"));

        let ordinary_offset = source.find("1.25").unwrap();
        let ordinary = database
            .hover(ordinary_offset)
            .unwrap()
            .expect("default f64 hover");
        assert!(ordinary.markdown.contains("1.25: f64"));
        assert!(ordinary.markdown.contains("`0x3ff4000000000000`"));
    }

    #[test]
    fn expression_hover_shows_inferred_types_for_operators_literals_and_calls() {
        let source = r#"state "game.exe" {}
fn add(left: u64, right: u64) -> u64 { return left + right }
whileAttached {
    let value: u64 = (1 + 2) * 3
    let message = "ready"
    let enabled = true
    let sum = add(4, 5)
}"#;
        let mut database = CompilerDatabase::new(source);

        let plus = source.find("1 + 2").unwrap() + 2;
        let addition = database.hover(plus).unwrap().expect("addition hover");
        assert_eq!(&source[addition.span.start..addition.span.end], "(1 + 2)");
        assert_eq!(addition.markdown, "```splitscript\nu64\n```");

        let integer = source.find("1 + 2").unwrap();
        let integer = database
            .hover(integer)
            .unwrap()
            .expect("integer literal hover");
        assert_eq!(integer.markdown, "```splitscript\nu64\n```");

        for (spelling, expected) in [("\"ready\"", "String"), ("true", "bool")] {
            let hover = database
                .hover(source.find(spelling).unwrap() + 1)
                .unwrap()
                .unwrap_or_else(|| panic!("{spelling} literal hover"));
            assert_eq!(hover.markdown, format!("```splitscript\n{expected}\n```"));
        }

        let call = source.find("add(4, 5)").unwrap();
        let closing_parenthesis = call + "add(4, 5)".len() - 1;
        let call = database
            .hover(closing_parenthesis)
            .unwrap()
            .expect("call expression hover");
        assert_eq!(&source[call.span.start..call.span.end], "add(4, 5)");
        assert_eq!(call.markdown, "```splitscript\nu64\n```");
    }

    #[test]
    fn range_operator_hover_combines_type_information_with_type_form_documentation() {
        let source = r#"fn exclusiveStart(values: u16..<u16) -> u16 {
    return values.start
}

fn inclusiveEnd(values: u8..=u8) -> u8 {
    return values.end
}

fn ranges() {
    let exclusive = 1u16..<4
    let inclusive = 1u8..=4
}
"#;
        let mut database = CompilerDatabase::new(source);

        for (operator, expected_type, expected_summary, expected_uri) in [
            (
                "..<",
                "u16..<u16",
                "upper bound is excluded",
                "/stdlib/type-forms/exclusive-range/index.md",
            ),
            (
                "..=",
                "u8..=u8",
                "upper bound is included",
                "/stdlib/type-forms/inclusive-range/index.md",
            ),
        ] {
            let occurrences = source
                .match_indices(operator)
                .map(|(offset, _)| offset)
                .collect::<Vec<_>>();
            assert_eq!(occurrences.len(), 2);

            let type_hover = database
                .hover(occurrences[0] + 1)
                .unwrap()
                .expect("range operator in type position has hover documentation");
            assert!(type_hover.markdown.contains(&format!("T{operator}T")));
            assert!(type_hover.markdown.contains(expected_summary));
            assert_eq!(type_hover.documentation_uri.as_deref(), Some(expected_uri));

            let expression_hover = database
                .hover(occurrences[1] + 1)
                .unwrap()
                .expect("range operator in expression position has hover documentation");
            assert!(expression_hover.markdown.contains(expected_type));
            assert!(expression_hover.markdown.contains(expected_summary));
            assert_eq!(
                expression_hover.documentation_uri.as_deref(),
                Some(expected_uri)
            );
        }
    }

    #[test]
    fn range_pattern_operator_hover_explains_pattern_semantics() {
        let source = r#"state "game.exe" {}
fn classify(value: i16) -> bool {
    return match value {
        -10..<0 | 10..=20 => true,
        _ => false,
    }
}"#;
        let mut database = CompilerDatabase::new(source);
        for (operator, form, endpoint) in [
            ("..<", "start..<end", "excludes the upper endpoint"),
            ("..=", "start..=end", "includes the upper endpoint"),
        ] {
            let offset = source.find(operator).unwrap() + 1;
            let hover = database
                .hover(offset)
                .unwrap()
                .expect("range pattern operator should have hover documentation");
            assert!(hover.markdown.contains(form), "{}", hover.markdown);
            assert!(hover.markdown.contains(endpoint), "{}", hover.markdown);
            assert_eq!(
                hover.documentation_uri.as_deref(),
                Some("/language/range.md")
            );
        }
    }

    #[test]
    fn array_rest_operator_hover_explains_prefix_and_suffix_semantics() {
        let source = r#"state "game.exe" {}
fn classify(value: [u8]) -> bool {
    return match value {
        [0x53, .., 0] => true,
        _ => false,
    }
}"#;
        let offset = source.find("..").unwrap() + 1;
        let mut database = CompilerDatabase::new(source);
        let hover = database
            .hover(offset)
            .unwrap()
            .expect("array rest marker should have hover documentation");
        assert!(hover.markdown.contains("[prefix, .., suffix]"));
        assert!(hover.markdown.contains("zero or more elements"));
        assert_eq!(
            hover.documentation_uri.as_deref(),
            Some("/language/array-rest-pattern.md")
        );
    }

    #[test]
    fn native_process_hover_comes_from_the_typed_provider_declaration() {
        let source = "state \"game.exe\" {}\nwhileAttached { let attached = process }";
        let offset = source.rfind("process").unwrap() + 1;
        let mut database = CompilerDatabase::new(source);
        let hover = database
            .hover(offset)
            .unwrap()
            .expect("process provider hover");
        assert!(hover.markdown.contains("state \"game.exe\" { ... }"));
        assert!(hover.markdown.contains("process: Process"));
        assert!(hover.markdown.contains("has type `Process`"));
    }

    #[test]
    fn provider_context_hover_is_typed_documented_and_navigates_to_state() {
        let source = "state Unity [\"game.exe\"] {}\nwhileAttached { let scenes = unity.scenes }";
        let offset = source.rfind("unity").unwrap() + 1;
        let mut database = CompilerDatabase::new(source);
        let hover = database
            .hover(offset)
            .unwrap()
            .expect("Unity provider-context hover");
        assert!(hover.markdown.contains("unity: UnityContext"));
        assert!(
            hover
                .markdown
                .contains("attachment-scoped Unity engine facilities")
        );
        assert_eq!(
            hover.documentation_uri.as_deref(),
            Some("/stdlib/types/UnityContext/index.md")
        );

        assert!(matches!(
            database.definition_at(offset).unwrap(),
            Some(crate::database::DefinitionTarget::Source(definition))
                if definition.id == crate::database::SourceDefinitionId::State
        ));
    }

    #[test]
    fn matched_process_name_hover_comes_from_the_standard_library() {
        let source =
            "state [\"game.exe\", \"demo.exe\"] {}\nwhileAttached { print(process.name()) }";
        let offset = source.find("name()").unwrap() + 1;
        let mut database = CompilerDatabase::new(source);
        let hover = database.hover(offset).unwrap().expect("process.name hover");
        assert!(hover.markdown.contains("Process.name() -> String"));
        assert!(
            hover
                .markdown
                .contains("configured process name that matched during attachment")
        );
        assert!(hover.markdown.contains("requires an attached process"));
    }

    #[test]
    fn main_module_hover_explains_matched_process_identity() {
        let source = "state [\"game.exe\", \"demo.exe\"] {}\nonAttach { let executable = await process.mainModule() }";
        let offset = source.find("mainModule").unwrap() + 1;
        let mut database = CompilerDatabase::new(source);
        let hover = database
            .hover(offset)
            .unwrap()
            .expect("process.mainModule hover");
        assert!(
            hover
                .markdown
                .contains("Process.mainModule() -> async Module")
        );
        assert!(hover.markdown.contains("main executable module"));
        assert!(
            hover
                .markdown
                .contains("available in suspending attachment code; suspends")
        );
    }

    #[test]
    fn field_hover_does_not_repeat_a_documentation_summary() {
        let source = "state \"game.exe\" {}\nonAttach { let executable = await process.mainModule(); print(executable.address) }";
        let offset = source.rfind("address").unwrap() + 1;
        let mut database = CompilerDatabase::new(source);
        let hover = database
            .hover(offset)
            .unwrap()
            .expect("Module.address hover");
        assert_eq!(
            hover
                .markdown
                .matches("Returns the module base address.")
                .count(),
            1,
            "the concise summary must not be manufactured into a second paragraph"
        );
        assert!(
            hover
                .markdown
                .contains("Relative virtual addresses within the image")
        );
    }

    #[test]
    fn process_closed_hover_explains_inert_attachment_waiting() {
        let source = "state \"game.exe\" {}\nonAttach { await process.closed() }";
        let offset = source.find("closed").unwrap() + 1;
        let mut database = CompilerDatabase::new(source);
        let hover = database
            .hover(offset)
            .unwrap()
            .expect("process.closed hover");
        assert!(hover.markdown.contains("Process.closed() -> async Never"));
        assert!(
            hover
                .markdown
                .contains("never closes or detaches the process itself")
        );
        assert!(hover.markdown.contains(
            "available in suspending attachment code; suspends; cancels when the process closes"
        ));
    }

    #[test]
    fn settings_and_lifecycle_hover_share_the_language_catalog() {
        let source = include_str!("../examples/lso_desktop_settings.split");
        let mut database = CompilerDatabase::new(source);
        for (needle, expected) in [
            ("\"Player\"", "Declares a free-form text-input setting"),
            ("choice", "Declares an enum-backed setting choice"),
            ("onAttach", "Initializes one state-provider attachment"),
        ] {
            let offset = source.find(needle).unwrap() + 1;
            let hover = database.hover(offset).unwrap().expect("language hover");
            assert!(
                hover.markdown.contains(expected),
                "missing catalog documentation for {needle}"
            );
            if needle == "onAttach" {
                assert!(
                    hover
                        .markdown
                        .contains("let module = await process.module(\"GameAssembly.dll\")")
                );
                assert!(!hover.markdown.contains("onDetach {}"));
            }
        }
    }

    #[test]
    fn detach_lifecycle_hover_describes_the_exact_event() {
        let source = "state \"game.exe\" {}\nonDetach { setTickRate(1) }";
        let mut database = CompilerDatabase::new(source);
        let hover = database
            .hover(source.find("onDetach").unwrap() + 1)
            .unwrap()
            .expect("onDetach hover");
        assert!(
            hover
                .markdown
                .contains("once when a logical attachment whose")
        );
        assert!(hover.markdown.contains("completed ends"));
        assert!(hover.markdown.contains("emulator provider also detaches"));
        assert!(
            hover.markdown.contains(
                "does not run when attachment initialization was still pending or rejected"
            )
        );
        assert!(
            hover
                .markdown
                .contains("never runs for the initial detached state")
        );
        assert!(
            hover
                .markdown
                .contains("use `setup` for one-time script initialization")
        );
    }

    #[test]
    fn process_selection_hover_describes_candidate_and_error_semantics() {
        let source = r#"state Unity ["game.exe"] {}
selectProcess { return process.path()?.endsWith("game.exe") }"#;
        let mut database = CompilerDatabase::new(source);
        let hover = database
            .hover(source.find("selectProcess").unwrap() + 1)
            .unwrap()
            .expect("selectProcess hover");
        assert!(hover.markdown.contains("same-name process candidates"));
        assert!(hover.markdown.contains("implicit error boundary"));
        assert!(hover.markdown.contains("reject only the current candidate"));
        assert!(hover.documentation_uri.is_some());
    }

    #[test]
    fn dynamic_tick_rate_hover_explains_the_declarative_lifecycle_policy() {
        let source = "state \"game.exe\" {}\nonAttach { setTickRate(30) }";
        let mut database = CompilerDatabase::new(source);
        let hover = database
            .hover(source.find("setTickRate").unwrap() + 2)
            .unwrap()
            .expect("setTickRate hover");

        assert!(
            hover
                .markdown
                .contains("120 Hz immediately after attaching")
        );
        assert!(hover.markdown.contains("1 Hz during module startup"));
        assert!(hover.markdown.contains("top-level `tickRate` declaration"));
        assert!(hover.markdown.contains("next attachment transition"));
        assert!(hover.markdown.contains("setTickRate(30)"));
        assert!(hover.documentation_uri.is_some());
    }

    #[test]
    fn timer_lifecycle_hover_describes_detached_sampling() {
        let source = "state \"game.exe\" {}\nonStart {}\nonReset {}";
        let mut database = CompilerDatabase::new(source);
        for (action, transition) in [
            ("onStart", "leave `TimerState.NotRunning`"),
            ("onReset", "enter `TimerState.NotRunning`"),
        ] {
            let hover = database
                .hover(source.find(action).unwrap() + 1)
                .unwrap()
                .expect("timer lifecycle hover");
            assert!(hover.markdown.contains(transition), "{}", hover.markdown);
            assert!(hover.markdown.contains("while detached"));
            assert!(hover.markdown.contains("following update"));
        }
    }

    #[test]
    fn source_hover_renders_inferred_value_and_field_types() {
        let source = r#"
struct Point { x: i32 }
let global = 1
state "game.exe" {
    point: Point = process.read(0);
    inventory: [u8; 6] at 0x100;
}
settings {
    "General" {
        /// Controls the optional behavior.
        "Enabled" => enabled: true
    }
}
fn inspect(point) {
    let local = point.x + global
    if settings.enabled { print(local as String) }
}
whileAttached {
    let timerState = timer.state()
    let inventory = current.inventory
    for item in inventory {
        print(item as String)
    }
    inspect(current.point)
}
"#;
        let mut database = CompilerDatabase::new(source);
        for (offset, signature, description) in [
            (
                source.find("point) {").unwrap(),
                "point: Point",
                "Parameter",
            ),
            (
                source.find("point.x").unwrap() + "point.".len(),
                "Point.x: i32",
                "Struct field",
            ),
            (
                source.find("local as").unwrap(),
                "let local: i32",
                "Local variable",
            ),
            (
                source.find("timerState =").unwrap(),
                "let timerState: TimerState",
                "Local variable",
            ),
            (
                source.find("inventory =").unwrap(),
                "let inventory: [u8; 6]",
                "Local variable",
            ),
            (
                source.find("item in inventory").unwrap(),
                "item: u8",
                "Read-only loop binding",
            ),
            (
                source.find("+ global").unwrap() + 2,
                "let global: i32",
                "Global variable",
            ),
            (
                source.find("current.point").unwrap() + "current.".len(),
                "current.point: Point",
                "Transactional state field",
            ),
            (
                source.find("current.inventory").unwrap() + "current.".len(),
                "current.inventory: [u8; 6]",
                "Transactional state field",
            ),
            (
                source.find("settings.enabled").unwrap() + "settings.".len(),
                "settings.enabled: bool",
                "Controls the optional behavior.",
            ),
            (
                source.rfind("inspect").unwrap(),
                "fn inspect(point: Point) -> None",
                "Function",
            ),
        ] {
            let hover = database.hover(offset).unwrap().expect("source hover");
            assert!(
                hover.markdown.contains(signature),
                "missing `{signature}` in {}",
                hover.markdown
            );
            assert!(
                hover.markdown.contains(description),
                "missing `{description}` in {}",
                hover.markdown
            );
        }
    }

    #[test]
    fn conditional_field_hover_shows_exact_shape_availability() {
        let source = r#"
enum Edition { Base, Demo }
enum Phase { Menu, Playing, Credits }
let edition: Edition

image "Assembly-CSharp" {
    class GameManager {
        static GameManager instance;
        if edition is Edition.Base {
            u32 level;
        } else {
            u32 scene;
        }
    }
}

state Unity ["game.exe"] {
    phase: Phase = Phase.Playing;
    if phase is Phase.Playing {
        playingScore: u32 at 0x100;
    } else {
        menuValue: u32 at 0x104;
    }
    if phase is Phase.Playing {
        shared: u32 at 0x108;
    } else {
        shared: u32 at 0x10c;
    }
}

onAttach { edition = Edition.Base }

whileAttached {
    let manager = GameManager.instance else return
    if edition is Edition.Base {
        print(manager.level else 0)
    }
    if current.phase is Phase.Playing {
        print(current.playingScore)
    }
    print(current.shared)
}
"#;
        let mut database = CompilerDatabase::new(source);

        for (needle, expected) in [
            ("level;", "**Available when:** `edition is Edition.Base`."),
            ("scene;", "**Available when:** `edition is Edition.Demo`."),
            (
                "playingScore:",
                "**Available when:** `current.phase is Phase.Playing`.",
            ),
        ] {
            let hover = database
                .hover(source.find(needle).unwrap() + 1)
                .unwrap()
                .unwrap_or_else(|| panic!("hover for {needle}"));
            assert!(hover.markdown.contains(expected), "{}", hover.markdown);
        }

        let fallback = database
            .hover(source.find("menuValue:").unwrap() + 1)
            .unwrap()
            .expect("fallback state-field hover");
        assert!(fallback.markdown.contains("`current.phase is Phase.Menu`"));
        assert!(
            fallback
                .markdown
                .contains("`current.phase is Phase.Credits`")
        );
        assert!(fallback.markdown.contains(" or "));

        let common = database
            .hover(source.rfind("current.shared").unwrap() + "current.".len())
            .unwrap()
            .expect("common state-field hover");
        assert!(!common.markdown.contains("Available when"));

        for (needle, expected) in [
            ("manager.level", "`edition is Edition.Base`"),
            ("current.playingScore", "`current.phase is Phase.Playing`"),
        ] {
            let hover = database
                .hover(source.rfind(needle).unwrap() + needle.rfind('.').unwrap() + 1)
                .unwrap()
                .unwrap_or_else(|| panic!("usage hover for {needle}"));
            assert!(hover.markdown.contains(expected), "{}", hover.markdown);
        }
    }

    #[test]
    fn memory_readable_struct_field_hover_shows_canonical_offset_and_size() {
        let source = r#"
struct Header {
    tag: u8,
    count: u32,
    flags: u16,
}
struct Packet {
    header: Header,
    samples: [u16; 3],
}
struct Metadata {
    label: String,
}
struct PointerHeader {
    tag: u8,
    next: address,
    value: u16,
}
state "game.exe" {}
"#;
        let mut database = CompilerDatabase::new(source);

        for (field, structure, offset, size, unit) in [
            ("tag", "Header", 0, 1, "byte"),
            ("count", "Header", 4, 4, "bytes"),
            ("flags", "Header", 8, 2, "bytes"),
            ("header", "Packet", 0, 12, "bytes"),
            ("samples", "Packet", 12, 6, "bytes"),
        ] {
            let hover = database
                .hover(source.find(&format!("{field}:")).unwrap() + 1)
                .unwrap()
                .unwrap_or_else(|| panic!("{structure}.{field} hover"));
            let expected = format!(
                "**Process-memory layout:** byte offset `0x{offset:x}` from the start of `{structure}`; size `{size}` {unit}."
            );
            assert!(hover.markdown.contains(&expected), "{}", hover.markdown);
        }

        let label = database
            .hover(source.find("label:").unwrap() + 1)
            .unwrap()
            .expect("Metadata.label hover");
        assert!(label.markdown.contains("Metadata.label: String"));
        assert!(!label.markdown.contains("Process-memory layout"));

        let value = database
            .hover(source.find("value:").unwrap() + 1)
            .unwrap()
            .expect("PointerHeader.value hover");
        assert!(
            value.markdown.contains(
                "32-bit targets read byte offset `0x8` with size `2`; 64-bit targets read byte offset `0x10` with size `2`"
            ),
            "{}",
            value.markdown,
        );
    }

    #[test]
    fn hover_distinguishes_attachment_globals_and_layout_constrained_helpers() {
        let source = r#"
enum Build { Steam, GOG }
let build: Build
let steamBase
let gogBase
state "game.exe" {
    if build == Build.Steam { level: u32 at 0x10; }
    else { level: u32 at 0x20; }
}
onAttach {
    if process.name() == "game.exe" {
        steamBase = 0x1000 as address
        build = Build.Steam
        return
    }

    gogBase = 0x2000 as address
    build = Build.GOG
}
fn steamReady() { return steamBase != 0 }
split {
    return match build {
        Build.Steam => steamReady(),
        Build.GOG => gogBase != 0,
    }
}
"#;
        let mut database = CompilerDatabase::new(source);
        let global = database
            .hover(source.find("steamBase\n").unwrap() + 1)
            .unwrap()
            .expect("attachment global hover");
        assert!(global.markdown.contains("let steamBase: address"));
        assert!(
            global
                .markdown
                .contains("Attachment-scoped global variable")
        );
        assert!(
            global
                .markdown
                .contains("**Attachment shapes:** `build = Build.Steam`")
        );

        let helper = database
            .hover(source.find("steamReady() {").unwrap() + 1)
            .unwrap()
            .expect("shape-constrained helper hover");
        assert!(helper.markdown.contains("requires an attached process"));
        assert!(
            helper
                .markdown
                .contains("**Attachment shapes:** `build = Build.Steam`")
        );
    }

    #[test]
    fn hover_describes_attempt_scoped_globals_and_helpers() {
        let source = r#"
let elapsed
state "game.exe" {}
onStart { elapsed = 0.0 }
fn elapsedTime() { return Duration.fromSeconds(elapsed) }
gameTime { return elapsedTime() }
"#;
        let mut database = CompilerDatabase::new(source);
        let global = database
            .hover(source.find("elapsed\n").unwrap() + 1)
            .unwrap()
            .expect("attempt global hover");
        assert!(global.markdown.contains("let elapsed: f64"));
        assert!(global.markdown.contains("Attempt-scoped global variable"));
        assert!(global.markdown.contains("initialized by `onStart`"));

        let helper = database
            .hover(source.find("elapsedTime() {").unwrap() + 1)
            .unwrap()
            .expect("attempt helper hover");
        assert!(
            helper
                .markdown
                .contains("**Attempt state:** requires an attempt initialized by `onStart`.")
        );
    }

    #[test]
    fn managed_schema_declarations_have_source_hover_and_documentation() {
        let source = r#"
enum Edition { Alternate }
let edition: Edition
state "game.exe" {}
onAttach { edition = Edition.Alternate }
/// Gameplay metadata.
image "Assembly-CSharp" {
    /// Runtime namespace.
    namespace Game {
        /// The player component.
        class Player {
            /// Current hit points.
            static f32 health from "_health";

            if edition == Edition.Alternate {
                /// Armor in the alternate release.
                f32 armor;
            }
        }
    }
}
let player: Player.Ref? = None
"#;
        let mut database = CompilerDatabase::new(source);
        database.check().expect("managed schema hover fixture");
        for (needle, signature, description) in [
            (
                "Assembly-CSharp",
                "image \"Assembly-CSharp\"",
                "Gameplay metadata.",
            ),
            ("Game {", "namespace Game", "Runtime namespace."),
            ("Player {", "class Player", "The player component."),
            ("health from", "static f32 health", "Current hit points."),
            ("armor;", "f32 armor", "Armor in the alternate release."),
        ] {
            let hover = database
                .hover(source.find(needle).unwrap() + 1)
                .unwrap()
                .expect("managed source hover");
            assert!(hover.markdown.contains(signature), "{}", hover.markdown);
            assert!(hover.markdown.contains(description), "{}", hover.markdown);
        }
        let reference = database
            .hover(source.rfind("player:").unwrap() + 1)
            .unwrap()
            .expect("managed reference hover");
        assert!(reference.markdown.contains("let player: Player.Ref?"));
    }

    #[test]
    fn managed_snapshot_hover_explains_the_transaction_and_navigates_to_its_class() {
        use crate::database::{DefinitionTarget, SourceDefinitionId};

        let source = r#"
image "Assembly-CSharp" {
    class GameManager {
        static GameManager instance;
        i32 points;
    }
}
state Unity ["game.exe"] {
    manager: GameManager = GameManager.instance?.snapshot()?;
}
"#;
        let mut database = CompilerDatabase::new(source);
        database.check().expect("managed snapshot hover fixture");
        let offset = source.rfind("snapshot").unwrap() + 1;
        let hover = database.hover(offset).unwrap().expect("snapshot hover");
        assert!(
            hover
                .markdown
                .contains("GameManager.Ref.snapshot() -> GameManager!")
        );
        assert!(hover.markdown.contains("no partial snapshot"));
        assert!(matches!(
            database.definition_at(offset).unwrap(),
            Some(DefinitionTarget::Source(definition))
                if matches!(definition.id, SourceDefinitionId::ManagedClass(_))
                    && definition.name == "GameManager"
        ));
    }

    #[test]
    fn managed_instances_hover_explains_cooperative_discovery() {
        use crate::database::{DefinitionTarget, SourceDefinitionId};

        let source = r#"
image "Assembly-CSharp" {
    class Enemy {
        i32 health;
    }
}
state Unity ["game.exe"] {}
onAttach {
    let enemies = await Enemy.instances()
}
"#;
        let mut database = CompilerDatabase::new(source);
        database.check().expect("managed instance hover fixture");
        let method_offset = source.rfind("instances").unwrap() + 1;
        let hover = database
            .hover(method_offset)
            .unwrap()
            .expect("instances hover");
        assert!(
            hover
                .markdown
                .contains("Enemy.instances() -> async [Enemy.Ref]")
        );
        assert!(hover.markdown.contains("bounded across ticks"));
        assert!(matches!(
            database.definition_at(method_offset).unwrap(),
            Some(DefinitionTarget::Source(definition))
                if matches!(definition.id, SourceDefinitionId::ManagedClass(_))
                    && definition.name == "Enemy"
        ));
        let class_offset = source.rfind("Enemy.instances").unwrap() + 1;
        assert!(matches!(
            database.definition_at(class_offset).unwrap(),
            Some(DefinitionTarget::Source(definition))
                if matches!(definition.id, SourceDefinitionId::ManagedClass(_))
                    && definition.name == "Enemy"
        ));
    }

    #[test]
    fn managed_component_hover_uses_the_selected_schema_class() {
        let source = r#"
image "Assembly-CSharp" {
    class PlayerController {
        i32 health;
    }
}
state Unity ["game.exe"] {}
whileAttached {
    let scene = unity.scenes.active() else return
    let object = scene.find("World/Player") else return
    let player = object.component<PlayerController>() else return
    print(player.health else 0)
}
"#;
        let mut database = CompilerDatabase::new(source);
        database.check().expect("managed component hover fixture");
        let offset = source.rfind("component").unwrap() + 1;
        let hover = database.hover(offset).unwrap().expect("component hover");
        assert!(
            hover
                .markdown
                .contains("UnityGameObject.component<PlayerController>() -> PlayerController.Ref!")
        );
        assert!(hover.markdown.contains("runtime class"));
        assert!(matches!(
            database.definition_at(offset).unwrap(),
            Some(DefinitionTarget::StandardLibrarySymbol(
                StdlibSymbolId::Type(crate::stdlib::StdlibTypeId::UnityGameObject)
            ))
        ));
    }

    #[test]
    fn every_managed_schema_keyword_has_language_hover() {
        let source = r#"
image "Assembly-CSharp" {
    namespace Game {
        class Player from ["Game.Player", "Player"] {
            static Player instance from "Instance";
            String name;
        }
    }
}
state Unity ["game.exe"] {}
"#;
        let mut database = CompilerDatabase::new(source);
        database.check().expect("managed schema hover fixture");

        for (needle, form, summary, uri) in [
            (
                "image ",
                "image \"Assembly-CSharp\"",
                "managed types exposed by one runtime image",
                "/language/image.md",
            ),
            (
                "namespace ",
                "namespace Name",
                "managed metadata namespace",
                "/language/namespace.md",
            ),
            (
                "class ",
                "class Name from",
                "typed managed class binding",
                "/language/class.md",
            ),
            (
                "static ",
                "static Type field",
                "managed field read through its class",
                "/language/static.md",
            ),
            (
                "from [",
                "class Name from",
                "runtime metadata names",
                "/language/from.md",
            ),
        ] {
            let hover = database
                .hover(source.find(needle).unwrap() + 1)
                .unwrap()
                .expect("managed keyword hover");
            assert!(
                hover.markdown.contains(form),
                "{needle}: {}",
                hover.markdown
            );
            assert!(
                hover.markdown.contains(summary),
                "{needle}: {}",
                hover.markdown
            );
            assert_eq!(hover.documentation_uri.as_deref(), Some(uri));
        }

        let second_from = source.rfind("from ").unwrap() + 1;
        let hover = database
            .hover(second_from)
            .unwrap()
            .expect("managed field `from` hover");
        assert!(hover.markdown.contains("runtime metadata names"));
        assert_eq!(
            hover.documentation_uri.as_deref(),
            Some("/language/from.md")
        );
    }

    #[test]
    fn source_hover_includes_user_documentation() {
        let source = r#"
/// A point in game memory.
struct Point {
    /// Horizontal position.
    x: i32
}
/// The current run mode.
enum Mode {
    /// Normal gameplay.
    Active
}
/// Accumulated collectible count.
let total = 0
state "game.exe" {
    /// Latest player position.
    point: Point at 0x100
}
/// Inspects the current state.
///
/// This is safe to call every tick.
fn inspect(point: Point, mode: Mode) {
    if mode == Mode.Active {
        print(point.x as String)
    }
}
whileAttached {
    inspect(current.point, Mode.Active)
    print(total as String)
}
"#;
        let mut database = CompilerDatabase::new(source);
        for (needle, expected) in [
            ("struct Point", "A point in game memory."),
            ("point.x", "Horizontal position."),
            ("enum Mode", "The current run mode."),
            ("Mode.Active", "Normal gameplay."),
            ("total as", "Accumulated collectible count."),
            ("current.point", "Latest player position."),
            (
                "inspect(current",
                "Inspects the current state.\n\nThis is safe to call every tick.",
            ),
        ] {
            let offset = source.find(needle).unwrap()
                + match needle {
                    "struct Point" => "struct ".len(),
                    "enum Mode" => "enum ".len(),
                    "point.x" => "point.".len(),
                    "Mode.Active" => "Mode.".len(),
                    "current.point" => "current.".len(),
                    _ => 0,
                };
            let hover = database.hover(offset).unwrap().expect("source hover");
            assert!(
                hover.markdown.contains(expected),
                "missing `{expected}` in {}",
                hover.markdown
            );
        }
    }

    #[test]
    fn first_class_state_snapshot_hover_preserves_type_and_field_documentation() {
        let source = r#"
state "game.exe" {
    /// Current level number.
    level: u32 at 0x100
}
fn levelOf(snapshot) {
    return snapshot.level
}
whileAttached {
    let captured = current
    print(levelOf(captured))
}
"#;
        let mut database = CompilerDatabase::new(source);
        for (offset, expected) in [
            (source.find("snapshot)").unwrap(), "snapshot: StateSnapshot"),
            (
                source.find("captured =").unwrap(),
                "let captured: StateSnapshot",
            ),
            (
                source.find("snapshot.level").unwrap() + "snapshot.".len(),
                "Current level number.",
            ),
        ] {
            let hover = database.hover(offset).unwrap().expect("snapshot hover");
            assert!(
                hover.markdown.contains(expected),
                "missing `{expected}` in {}",
                hover.markdown
            );
        }
    }

    #[test]
    fn state_filter_bindings_have_hover_documentation() {
        let source = r#"state "game.exe" {
    scene: i32 at 0x100 if value == 7 { Err("transient") } else { value };
        }"#;
        let mut database = CompilerDatabase::new(source);
        let offset = source.find("value ==").unwrap();
        let hover = database
            .hover(offset)
            .unwrap()
            .expect("state filter hover for value");
        assert!(
            hover.markdown.contains("value: i32"),
            "missing value type in {}",
            hover.markdown
        );
    }

    #[test]
    fn first_class_settings_view_hover_preserves_type_and_setting_documentation() {
        let source = r#"
settings {
    /// Enables automatic splitting.
    "Enabled" => enabled: true
}
state "game.exe" {}
fn isEnabled(view) {
    return view.enabled
}
whileAttached {
    let captured = settings
    print(isEnabled(captured))
}
"#;
        let mut database = CompilerDatabase::new(source);
        for (offset, expected) in [
            (source.find("view)").unwrap(), "view: SettingsView"),
            (
                source.find("captured =").unwrap(),
                "let captured: SettingsView",
            ),
            (
                source.find("view.enabled").unwrap() + "view.".len(),
                "Enables automatic splitting.",
            ),
        ] {
            let hover = database
                .hover(offset)
                .unwrap()
                .expect("settings view hover");
            assert!(
                hover.markdown.contains(expected),
                "missing `{expected}` in {}",
                hover.markdown
            );
        }
    }

    #[test]
    fn settings_key_lookup_hover_comes_from_the_standard_library() {
        let source = r#"
settings {
    "Boss" => boss key "split-boss": true
}
state "game.exe" {}
whileAttached {
    let shouldSplit = settings.enabled("split-boss")
}
"#;
        let offset = source.find("enabled(").unwrap();
        let mut database = CompilerDatabase::new(source);
        let hover = database.hover(offset).unwrap().expect("method hover");
        assert!(
            hover
                .markdown
                .contains("SettingsView.enabled(key: String) -> bool")
        );
        assert!(hover.markdown.contains("stable host-map key"));
        assert!(hover.markdown.contains("**Effects:** pure"));
    }

    #[test]
    fn integer_radix_hover_comes_from_the_capability_catalog() {
        let source = r#"
state "game.exe" {}
whileAttached {
    let hexadecimal = 255u8.toString(16) else ""
    print(hexadecimal)
}
"#;
        let offset = source.find("toString(").unwrap();
        let mut database = CompilerDatabase::new(source);
        let hover = database.hover(offset).unwrap().expect("method hover");
        assert!(hover.markdown.contains("toString(radix: u32) -> String!"));
        assert!(hover.markdown.contains("Radices from 2 through 36"));
        assert!(hover.markdown.contains("**Effects:** allocates"));
    }

    #[test]
    fn source_function_hover_renders_propagated_effects_after_semantic_validation_errors() {
        let source = r#"
fn readValue() -> f32! {
    return process.read<f32>(0)
}
fn bar() -> f32! {
    return readValue()
}
state "game.exe" {}
onDetach {
    let value = bar()
}
"#;
        let offset = source.rfind("bar()").unwrap();
        let mut database = CompilerDatabase::new(source);
        let hover = database
            .hover(offset)
            .unwrap()
            .expect("source function hover should survive the detached-call diagnostic");
        assert!(hover.markdown.contains("fn bar() -> f32!"));
        assert!(
            hover
                .markdown
                .contains("**Effects:** reads process memory, requires an attached process")
        );
        assert!(hover.markdown.contains(
            "**Runtime behavior:** synchronous; requires an attached process and is unavailable in `onDetach`"
        ));
    }

    #[test]
    fn source_function_hover_does_not_punctuate_metadata_fragments() {
        let source = r#"
fn identity(value: i32) -> i32 {
    return value
}
state "game.exe" {}
whileAttached {
    let value = identity(1)
}
"#;
        let offset = source.rfind("identity").unwrap();
        let mut database = CompilerDatabase::new(source);
        let hover = database.hover(offset).unwrap().expect("function hover");
        assert!(
            hover
                .markdown
                .contains("\n\nFunction\n\n**Effects:** pure\n\n**Runtime behavior:** synchronous")
        );
        assert!(!hover.markdown.contains("Function."));
        assert!(!hover.markdown.contains("pure."));
        assert!(!hover.markdown.contains("synchronous."));
    }

    #[test]
    fn inferred_async_function_hover_includes_async_in_the_result_type() {
        let source = r#"
state "game.exe" {}
fn loadModule() {
    let module = await process.module("game.dll")
    return module
}
onAttach {
    let module = await loadModule()
    print(module.address)
}
"#;
        let offset = source.rfind("loadModule").unwrap();
        let mut database = CompilerDatabase::new(source);
        let hover = database.hover(offset).unwrap().expect("function hover");
        assert!(
            hover.markdown.contains("fn loadModule() -> async Module"),
            "{}",
            hover.markdown
        );
        assert!(hover.markdown.contains("**Runtime behavior:** suspends"));
    }

    #[test]
    fn stored_future_hover_preserves_the_async_value_type() {
        let source = r#"
state "game.exe" {}
onAttach {
    let pending = process.mainModule()
    print("created")
    let module = await pending
    print(module.address)
}
"#;
        let offset = source.rfind("pending").unwrap();
        let mut database = CompilerDatabase::new(source);
        let hover = database.hover(offset).unwrap().expect("future hover");
        assert!(
            hover.markdown.contains("let pending: async Module"),
            "{}",
            hover.markdown
        );
        assert!(hover.markdown.contains("Local variable"));
    }

    #[test]
    fn bottom_future_hover_preserves_the_never_completion_type() {
        let source = r#"
state "game.exe" {}
onAttach {
    let pending = process.closed()
    await pending
}
"#;
        let offset = source.rfind("pending").unwrap();
        let mut database = CompilerDatabase::new(source);
        let hover = database.hover(offset).unwrap().expect("future hover");
        assert!(
            hover.markdown.contains("let pending: async Never"),
            "{}",
            hover.markdown
        );
    }

    #[test]
    fn inferred_generic_function_hover_shows_parameters_and_capability_bounds() {
        let source = r#"
fn smaller(left, right) {
    return left.min(right)
}
state "game.exe" {}
whileAttached {
    let value: u32 = smaller(3u32, 7u32)
}
"#;
        let offset = source.rfind("smaller").unwrap();
        let mut database = CompilerDatabase::new(source);
        let hover = database
            .hover(offset)
            .unwrap()
            .expect("generic function hover");
        assert!(
            hover
                .markdown
                .contains("fn smaller(left: T, right: T) -> T where T: Numeric"),
            "{}",
            hover.markdown
        );
    }

    #[test]
    fn string_conversion_and_interpolation_share_the_display_capability() {
        let source = r#"
fn twoDigits(value) {
    if value == 0 {
        return "00"
    }
    if value < 10 {
        return `0{value}`
    }
    return value as String
}
state "game.exe" {}
whileAttached {
    let value = twoDigits(7u32)
}
"#;
        let offset = source.rfind("twoDigits").unwrap();
        let mut database = CompilerDatabase::new(source);
        let hover = database
            .hover(offset)
            .unwrap()
            .expect("generic function hover");
        assert_eq!(
            hover.markdown.lines().nth(1),
            Some("fn twoDigits(value: T) -> String where T: Numeric + Display"),
            "{}",
            hover.markdown
        );
    }

    #[test]
    fn source_type_hover_shows_structurally_satisfied_capabilities() {
        let source = r#"
struct Position { x: i32, }
fn Position.toString() -> String { return `{self.x}` }
state "game.exe" {}
whileAttached { print(Position { x: 3 }) }
"#;
        let mut database = CompilerDatabase::new(source);
        let hover = database
            .hover(source.find("Position").unwrap() + 1)
            .unwrap()
            .expect("struct hover");
        assert!(
            hover.markdown.contains("**Capabilities:**")
                && hover.markdown.contains("`Display` (custom)"),
            "{}",
            hover.markdown
        );

        let derived_source = r#"
struct Position { x: i32, }
state "game.exe" {}
whileAttached { print(Position { x: 3 }) }
"#;
        let mut database = CompilerDatabase::new(derived_source);
        let hover = database
            .hover(derived_source.find("Position").unwrap() + 1)
            .unwrap()
            .expect("derived struct hover");
        assert!(
            hover.markdown.contains("`Debug` (derived)"),
            "{}",
            hover.markdown
        );
    }

    #[test]
    fn integer_hierarchy_removes_redundant_lunistice_level_text_bounds() {
        let source = include_str!("../examples/lunistice.split");
        let offset = source.find("levelText").unwrap();
        let mut database = CompilerDatabase::new(source);
        let hover = database
            .hover(offset)
            .unwrap()
            .expect("levelText function hover");
        assert_eq!(
            hover.markdown.lines().nth(1),
            Some("fn levelText(level: T) -> String where T: Integer"),
            "{}",
            hover.markdown
        );
    }

    #[test]
    fn signature_help_tracks_nested_arguments_and_inferred_types() {
        let source = r#"
state "game.exe" {}

whileAttached {
    let value: i32 = 8
    let bounded = value.clamp((0), 7)
}
"#;
        let offset = source.find(", 7").unwrap() + 2;
        let mut database = CompilerDatabase::new(source);
        let help = database
            .signature_help(offset)
            .unwrap()
            .expect("signature help");
        assert_eq!(help.active_parameter, 1);
        assert!(help.signatures[0].label.contains("i32.clamp"));
        assert_eq!(help.signatures[0].parameters[1].label, "maximum");
    }

    #[test]
    fn signature_help_probes_an_unfinished_method_call() {
        let source = concat!(
            "state \"game.exe\" {}\n",
            "whileAttached {\n",
            "    let value: i32 = 8\n",
            "    let bounded = value.clamp(0, \n",
            "}\n"
        );
        let offset = source.find("clamp(0, ").unwrap() + "clamp(0, ".len();
        let mut database = CompilerDatabase::new(source);
        let help = database
            .signature_help(offset)
            .unwrap()
            .expect("incomplete calls should retain signature help");
        assert_eq!(help.active_parameter, 1);
        assert!(help.signatures[0].label.starts_with("i32.clamp"));
    }

    #[test]
    fn signature_help_resolves_explicit_types_on_captured_process_values() {
        let source = concat!(
            "state \"game.exe\" {}\n",
            "whileAttached {\n",
            "    let attached = process\n",
            "    let value = attached.read<u32>(\n",
            "}\n"
        );
        let offset = source.find("read<u32>(").unwrap() + "read<u32>(".len();
        let mut database = CompilerDatabase::new(source);
        let help = database
            .signature_help(offset)
            .unwrap()
            .expect("explicit generic method signature help");
        assert!(help.signatures[0].label.starts_with("Process.read"));
        assert!(help.signatures[0].label.contains("-> u32!"));
    }

    #[test]
    fn signature_help_resolves_explicit_types_on_expression_receivers() {
        let source = concat!(
            "state \"game.exe\" {}\n",
            "fn attachedProcess() { return process }\n",
            "whileAttached {\n",
            "    let value = attachedProcess().read<u32>(\n",
            "}\n"
        );
        let offset = source.find("read<u32>(").unwrap() + "read<u32>(".len();
        let mut database = CompilerDatabase::new(source);
        let help = database
            .signature_help(offset)
            .unwrap()
            .expect("expression receiver should retain generic method signature help");
        assert!(help.signatures[0].label.starts_with("Process.read"));
        assert!(help.signatures[0].label.contains("-> u32!"));
    }

    #[test]
    fn hover_recovers_from_untyped_state_field_member_access() {
        let source = r#"state GBA {
    pos at 0x100,
}

fn bar() {
    return current.pos.x > old.pos.y
}
"#;
        let mut database = CompilerDatabase::new(source);
        for needle in ["GBA", "current", "old"] {
            let offset = source.find(needle).unwrap() + needle.len() - 1;
            assert!(
                database.hover(offset).unwrap().is_some(),
                "{needle} should retain hover despite the recoverable separator error"
            );
        }
        assert!(
            database
                .diagnostics()
                .iter()
                .any(|diagnostic| diagnostic.message == "expected `;` between state fields")
        );
    }

    #[test]
    fn hover_preserves_independent_semantics_across_parser_errors() {
        let source = r#"
fn retained(value: i32) -> i32 {
    return value
}

state GBA {}

split {
    let broken = 0b102
    return retained(1) == 1
}
"#;
        let mut database = CompilerDatabase::new(source);
        assert!(
            database
                .recovering_parse()
                .unwrap()
                .diagnostics()
                .iter()
                .any(|diagnostic| diagnostic
                    .message
                    .contains("not valid in a binary integer literal"))
        );

        let hover = database
            .hover(source.rfind("retained").unwrap() + 1)
            .unwrap()
            .expect("an unrelated parser error must not disable semantic hover");
        assert!(hover.markdown.contains("fn retained(value: i32) -> i32"));
    }
}
