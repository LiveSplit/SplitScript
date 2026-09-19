//! Compiler-owned completion candidates shared by editor frontends.

use std::{
    collections::{BTreeMap, BTreeSet},
    sync::Arc,
};

mod patterns;
mod settings;
mod state;
mod top_level;
mod types;

use patterns::complete_pattern;
use settings::complete_settings_dsl;
use state::complete_state_dsl;
use top_level::{complete_state_header, complete_top_level};
use types::{complete_explicit_type_argument, complete_type_position};

use crate::{
    ast::{
        Block, Expr, ExprKind, FunctionId, MatchPattern, Program, SettingKind, Span, Stmt,
        TypeRef as SyntaxTypeRef, ValueId,
    },
    catalog::Documentation,
    database::{CompilerDatabase, SemanticQueryResult},
    documentation::{StandardLibraryDocumentation, language_item_uri, symbol_uri},
    effects::{OperationAnalysis, action_has_attached_process, action_has_state_snapshots},
    hir::ExpressionResolution,
    language::{
        LanguageCatalog, LanguageCompletionSite, LanguageItem, LanguageItemId, LanguageItemKind,
    },
    lexer::TokenKind,
    scoped_globals::{GlobalLifetime, ScopedGlobalAnalysis, action_has_attempt_scope},
    semantic::{ResolvedCall, ResolvedMember, ResolvedValue, SemanticModel},
    stdlib::{
        ItemKind, ItemVisibility, StandardLibrary, StdlibCapabilityId, StdlibItem, StdlibItemId,
        StdlibNamespace, StdlibSymbolId, StdlibTypeConstructor, StdlibTypeConstructorId,
        StdlibTypeId, TypeConstructorSyntax, TypeRef,
    },
    stdlib_semantic::StandardLibrarySemanticExt,
    syntax::SourceDocument,
    types::TypeKind,
    visit::{self, Visitor},
};

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub enum CompletionKind {
    Keyword,
    Snippet,
    Namespace,
    Function,
    Method,
    Variable,
    Setting,
    StateField,
    Property,
    Type,
    Struct,
    Enum,
    EnumMember,
    Constant,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CompletionItem {
    pub label: String,
    pub kind: CompletionKind,
    pub detail: Option<String>,
    pub documentation: Option<String>,
    /// Stable compiler-owned reference page for a catalog-backed completion.
    pub documentation_uri: Option<String>,
    pub insert_text: String,
    pub is_snippet: bool,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CompletionList {
    pub replacement: Span,
    pub items: Vec<CompletionItem>,
}

#[derive(Debug, Clone, Copy)]
struct ContextAvailability {
    attached_process: bool,
    active_attempt: bool,
    state_snapshots: bool,
    process_selection: bool,
    statement_position: bool,
    loop_control: bool,
    return_control: bool,
    generator_control: bool,
    method_receiver: bool,
}

/// Immutable lexical and syntactic facts shared by every completion strategy
/// for one cursor request. The lossless parser already owns this token stream;
/// keeping one indexed view prevents each grammar-specific strategy from
/// lexing and linearly rebuilding it independently.
pub(super) struct CompletionRequest<'a> {
    pub(super) source: &'a str,
    pub(super) syntax: &'a Program,
    pub(super) tokens: Vec<&'a crate::lexer::Token>,
    pub(super) offset: usize,
    pub(super) replacement: Span,
}

impl<'a> CompletionRequest<'a> {
    fn new(document: &'a SourceDocument, syntax: &'a Program, offset: usize) -> Self {
        let source = document.source();
        let offset = floor_char_boundary(source, offset.min(source.len()));
        Self {
            source,
            syntax,
            tokens: document
                .tokens()
                .filter(|token| !matches!(token.kind, TokenKind::Eof))
                .collect(),
            offset,
            replacement: identifier_span(source, offset),
        }
    }
}

pub(crate) fn complete(
    database: &mut CompilerDatabase,
    offset: usize,
) -> SemanticQueryResult<CompletionList> {
    let compiler_context = database.context();
    let standard_library = compiler_context.standard_library();
    // Keep the revision's shared recovery product alive for the entire request.
    // Completion only borrows its source and syntax; cloning the whole program
    // here used to make every query own an unnecessary deep syntax copy.
    let recovered = database.recovering_parse()?;
    let request = CompletionRequest::new(recovered.source_document(), recovered.syntax(), offset);
    let source = request.source;
    let syntax = request.syntax;
    let offset = request.offset;
    if let Some(completions) = complete_setting_key(&request) {
        Ok(completions)
    } else if let Some(completions) = complete_tick_rate_field(&request) {
        Ok(completions)
    } else if let Some(completions) = complete_settings_dsl(&request) {
        Ok(completions)
    } else if let Some(completions) = complete_state_header(&request, &standard_library) {
        Ok(completions)
    } else if let Some(completions) = complete_state_decoder(source, offset) {
        Ok(completions)
    } else if let Some(completions) = complete_managed_field_modifier(&request) {
        Ok(completions)
    } else if let Some(completions) =
        complete_explicit_type_argument(source, syntax, offset, &standard_library)
    {
        Ok(completions)
    } else if let Some(completions) = complete_type_position(&request, &standard_library) {
        Ok(completions)
    } else if let Some(completions) = complete_state_dsl(&request, &standard_library) {
        Ok(completions)
    } else if let Some(completions) = complete_pattern(database, &request, &standard_library)? {
        Ok(completions)
    } else if let Some(context) = member_context(&request) {
        Ok(complete_member(
            database,
            source,
            syntax,
            &request.tokens,
            context,
            compiler_context,
        ))
    } else {
        let action = syntax
            .actions
            .iter()
            .find(|action| contains_offset(action.body.span, offset))
            .map(|action| action.kind);
        let containing_function = syntax
            .functions
            .iter()
            .find(|function| contains_offset(function.body.span, offset));
        let inside_function = containing_function.is_some();
        let top_level = action.is_none() && is_top_level_offset(syntax, offset);
        let has_attached_process = action.is_none_or(action_has_attached_process);
        let has_state_snapshots = action
            .map(action_has_state_snapshots)
            .unwrap_or(inside_function);
        let needs_effects = !top_level
            && (syntax.globals.iter().any(|global| global.value.is_none())
                || ((!has_attached_process || !has_state_snapshots)
                    && syntax
                        .functions
                        .iter()
                        .any(|function| function.method_of.is_none())));
        let effects = needs_effects
            .then(|| {
                completion_operation_analysis(
                    database,
                    source,
                    identifier_span(source, offset),
                    compiler_context,
                )
            })
            .flatten();
        Ok(complete_root(
            source,
            syntax,
            offset,
            standard_library,
            ContextAvailability {
                attached_process: has_attached_process,
                active_attempt: inside_function || action.is_some_and(action_has_attempt_scope),
                state_snapshots: has_state_snapshots,
                process_selection: action == Some(crate::ast::ActionKind::SelectProcess),
                statement_position: is_statement_position(
                    source,
                    &request.tokens,
                    request.replacement.start,
                ),
                loop_control: cursor_inside_loop(syntax, offset),
                return_control: action.is_some() || inside_function,
                generator_control: cursor_inside_generator(syntax, offset),
                method_receiver: containing_function
                    .is_some_and(|function| function.method_of.is_some()),
            },
            effects.as_deref(),
            top_level,
        ))
    }
}

fn complete_managed_field_modifier(request: &CompletionRequest<'_>) -> Option<CompletionList> {
    let source = request.source;
    let syntax = request.syntax;
    let offset = request.offset;
    let replacement = request.replacement;
    if !syntax
        .managed_class_declarations()
        .into_iter()
        .any(|class| class.span.start < offset && offset < class.span.end)
    {
        return None;
    }

    let segment_start = source[..replacement.start]
        .rfind(['{', '}', ';'])
        .map_or(0, |index| index + 1);
    let tokens = request
        .tokens
        .iter()
        .copied()
        .filter(|token| segment_start <= token.span.start && token.span.end <= replacement.start)
        .collect::<Vec<_>>();
    let mut identifiers = tokens.iter().filter_map(|token| match &token.kind {
        TokenKind::Ident(name) => Some(name.as_str()),
        _ => None,
    });
    if identifiers.next()? != "String" || identifiers.next().is_none() {
        return None;
    }
    if tokens
        .iter()
        .any(|token| matches!(&token.kind, TokenKind::Ident(name) if name == "maxLength"))
    {
        return None;
    }

    let item = LanguageCatalog::new().item(LanguageItemId::ManagedStringMaxLength);
    let prefix = source[replacement.start..offset].to_owned();
    let mut builder = CompletionBuilder::new(prefix, replacement);
    builder.add(catalog_language_completion(
        item.name,
        CompletionKind::Keyword,
        item,
        "maxLength ${1:64};".to_owned(),
        true,
    ));
    Some(builder.finish())
}

fn complete_setting_key(request: &CompletionRequest<'_>) -> Option<CompletionList> {
    let source = request.source;
    let syntax = request.syntax;
    let offset = request.offset;
    let tokens = &request.tokens;
    let (index, token) = tokens.iter().enumerate().find(|(_, token)| {
        matches!(token.kind, TokenKind::String(_))
            && token.span.start < offset
            && offset < token.span.end
    })?;
    let [root, dot, method, open] = tokens.get(index.checked_sub(4)?..index)? else {
        return None;
    };
    let TokenKind::Ident(root_name) = &root.kind else {
        return None;
    };
    if !matches!(root_name.as_str(), "settings" | "oldSettings")
        || !matches!(dot.kind, TokenKind::Dot)
        || !matches!(open.kind, TokenKind::LParen)
    {
        return None;
    }
    let method = match &method.kind {
        TokenKind::Ident(method) if method == "enabled" || method == "contains" => method.as_str(),
        _ => return None,
    };
    let replacement = Span {
        start: token.span.start + 1,
        end: token.span.end.saturating_sub(1),
    };
    if !(replacement.start <= offset && offset <= replacement.end) {
        return None;
    }
    let prefix = source[replacement.start..offset].to_owned();
    let mut builder = CompletionBuilder::new(prefix, replacement);
    for setting in &syntax.settings {
        let compatible = match setting.kind {
            SettingKind::Bool { .. } => true,
            SettingKind::Text { .. } | SettingKind::Choice { .. } | SettingKind::File { .. } => {
                method == "contains"
            }
            SettingKind::Title { .. } => false,
        };
        if !compatible {
            continue;
        }
        let kind = match setting.kind {
            SettingKind::Bool { .. } => "boolean setting key",
            SettingKind::Text { .. } => "text setting key",
            SettingKind::Choice { .. } => "choice setting key",
            SettingKind::File { .. } => "file setting key",
            SettingKind::Title { .. } => unreachable!(),
        };
        let direct = if setting.source_visible {
            format!("; directly available as {root_name}.{}", setting.name)
        } else {
            String::new()
        };
        builder.add(CompletionItem {
            label: setting.runtime_key().to_owned(),
            kind: CompletionKind::Setting,
            detail: Some(format!("{kind}{direct}")),
            documentation: setting.tooltip.clone(),
            documentation_uri: None,
            insert_text: escape_string_contents(setting.runtime_key()),
            is_snippet: false,
        });
    }
    Some(builder.finish())
}

fn escape_string_contents(value: &str) -> String {
    let mut escaped = String::with_capacity(value.len());
    for character in value.chars() {
        match character {
            '\n' => escaped.push_str("\\n"),
            '\r' => escaped.push_str("\\r"),
            '\t' => escaped.push_str("\\t"),
            '\\' => escaped.push_str("\\\\"),
            '"' => escaped.push_str("\\\""),
            '\'' => escaped.push_str("\\'"),
            character => escaped.push(character),
        }
    }
    escaped
}

fn complete_tick_rate_field(request: &CompletionRequest<'_>) -> Option<CompletionList> {
    let source = request.source;
    let offset = request.offset;
    let replacement = request.replacement;
    let tokens = &request.tokens;

    let (open, close) = tokens.iter().enumerate().find_map(|(index, token)| {
        if !matches!(&token.kind, TokenKind::Ident(name) if name == "tickRate") {
            return None;
        }
        let open = tokens[index + 1..]
            .iter()
            .position(|token| matches!(token.kind, TokenKind::LBrace))?
            + index
            + 1;
        let close = matching_closing_brace(tokens, open).unwrap_or(tokens.len());
        let closing_start = tokens
            .get(close)
            .map_or(source.len(), |token| token.span.start);
        (tokens[open].span.end <= offset && offset <= closing_start).then_some((open, close))
    })?;

    let mut depth = 1_u32;
    let mut segment_has_colon = false;
    for token in &tokens[open + 1..close] {
        if token.span.start >= offset {
            break;
        }
        match token.kind {
            TokenKind::LBrace => depth += 1,
            TokenKind::RBrace => depth = depth.saturating_sub(1),
            TokenKind::Comma if depth == 1 => segment_has_colon = false,
            TokenKind::Colon if depth == 1 => segment_has_colon = true,
            _ => {}
        }
    }
    if depth != 1 || segment_has_colon {
        return None;
    }

    let mut declared = Vec::new();
    depth = 1;
    for (index, token) in tokens[open + 1..close].iter().enumerate() {
        match token.kind {
            TokenKind::LBrace => depth += 1,
            TokenKind::RBrace => depth = depth.saturating_sub(1),
            TokenKind::Ident(ref name)
                if depth == 1
                    && tokens
                        .get(open + index + 2)
                        .is_some_and(|next| matches!(next.kind, TokenKind::Colon))
                    && token.span != replacement =>
            {
                declared.push(name.as_str());
            }
            _ => {}
        }
    }

    let prefix = source[replacement.start..offset].to_owned();
    let mut builder = CompletionBuilder::new(prefix, replacement);
    if !declared.contains(&"attached") {
        builder.add(CompletionItem {
            label: "attached".to_owned(),
            kind: CompletionKind::Property,
            detail: Some("attached polling rate (Hz)".to_owned()),
            documentation: Some(
                "Polling rate used while a process is attached. Defaults to 120 Hz.".to_owned(),
            ),
            documentation_uri: Some(language_item_uri(LanguageItemId::TickRate)),
            insert_text: "attached: ${1:120},".to_owned(),
            is_snippet: true,
        });
    }
    if !declared.contains(&"detached") {
        builder.add(CompletionItem {
            label: "detached".to_owned(),
            kind: CompletionKind::Property,
            detail: Some("detached polling rate (Hz)".to_owned()),
            documentation: Some(
                "Polling rate used while waiting for a process. Defaults to 1 Hz.".to_owned(),
            ),
            documentation_uri: Some(language_item_uri(LanguageItemId::TickRate)),
            insert_text: "detached: ${1:1},".to_owned(),
            is_snippet: true,
        });
    }
    Some(builder.finish())
}

fn matching_closing_brace(tokens: &[&crate::lexer::Token], open: usize) -> Option<usize> {
    let mut depth = 0_u32;
    for (index, token) in tokens.iter().enumerate().skip(open) {
        match token.kind {
            TokenKind::LBrace => depth += 1,
            TokenKind::RBrace => {
                depth = depth.saturating_sub(1);
                if depth == 0 {
                    return Some(index);
                }
            }
            _ => {}
        }
    }
    None
}

/// Compact availability facts needed by root completion. Function-operation
/// availability and lifecycle-global lifetimes come from their canonical
/// semantic analyses; retaining the complete analyses or repaired snapshots
/// here would keep unrelated metadata or entire compiler databases alive.
#[derive(Debug, Default)]
struct CompletionEffects {
    attached_process: BTreeSet<FunctionId>,
    state_snapshots: BTreeSet<FunctionId>,
    global_lifetimes: BTreeMap<ValueId, GlobalLifetime>,
    attempt_functions: BTreeSet<FunctionId>,
}

impl CompletionEffects {
    fn from_analysis(
        syntax: &Program,
        analysis: &OperationAnalysis,
        scoped_globals: Option<&ScopedGlobalAnalysis>,
    ) -> Self {
        let mut facts = Self::default();
        for function in syntax
            .functions
            .iter()
            .filter(|function| function.method_of.is_none())
        {
            let effects = analysis.function(function.id);
            if effects.requires_attached_process {
                facts.attached_process.insert(function.id);
            }
            if effects.requires_state_snapshots {
                facts.state_snapshots.insert(function.id);
            }
            if scoped_globals.is_some_and(|globals| globals.function_requires_attempt(function.id))
            {
                facts.attempt_functions.insert(function.id);
            }
        }
        if let Some(scoped_globals) = scoped_globals {
            for global in &syntax.globals {
                if let Some(lifetime) = scoped_globals.lifetime(global.id) {
                    facts.global_lifetimes.insert(global.id, lifetime);
                }
            }
        }
        facts
    }
}

/// The containing database invalidates all facts on source edits. Repair keys
/// cover the complete identifier, so moving the caret inside it reuses facts.
#[derive(Debug, Default)]
pub(crate) struct CompletionEffectsCache {
    direct: Option<Arc<CompletionEffects>>,
    entries: std::collections::VecDeque<(Span, Option<Arc<CompletionEffects>>)>,
    #[cfg(test)]
    analyses: usize,
    #[cfg(test)]
    probes: usize,
}

impl CompletionEffectsCache {
    const CAPACITY: usize = 4;

    fn get(&mut self, key: Span) -> Option<Option<Arc<CompletionEffects>>> {
        if let Some(direct) = &self.direct {
            return Some(Some(Arc::clone(direct)));
        }
        let position = self.entries.iter().position(|(stored, _)| *stored == key)?;
        let entry = self.entries.remove(position)?;
        let facts = entry.1.clone();
        self.entries.push_back(entry);
        Some(facts)
    }

    fn insert(&mut self, key: Span, facts: Option<Arc<CompletionEffects>>) {
        if self.entries.len() == Self::CAPACITY {
            self.entries.pop_front();
        }
        self.entries.push_back((key, facts));
    }
}

fn completion_operation_analysis(
    database: &mut CompilerDatabase,
    source: &str,
    replacement: Span,
    compiler_context: crate::CompilerContext,
) -> Option<Arc<CompletionEffects>> {
    if let Some(cached) = database.completion_effects_cache().get(replacement) {
        return cached;
    }
    #[cfg(test)]
    {
        database.completion_effects_cache().analyses += 1;
    }
    if let Ok(snapshot) = database.semantic_snapshot()
        && let Some(effects) = snapshot.effects()
    {
        let scoped_globals = snapshot.checked().map(|checked| checked.scoped_globals());
        // Recovery can retain operation effects without the typed program
        // required by lifecycle-global analysis. A file with bare globals
        // therefore still needs the inert completion probe below; otherwise
        // every scoped global would appear universally available while the
        // user is typing an unresolved identifier.
        if scoped_globals.is_some()
            || !snapshot
                .syntax()
                .globals
                .iter()
                .any(|global| global.value.is_none())
        {
            let facts = Arc::new(CompletionEffects::from_analysis(
                snapshot.syntax(),
                effects,
                scoped_globals,
            ));
            database.completion_effects_cache().direct = Some(Arc::clone(&facts));
            return Some(facts);
        }
    }

    // A partially typed root identifier is normally an unknown expression and
    // prevents typed-HIR effect analysis. Replace only that identifier with a
    // valid inert value; declaration and function IDs remain stable, allowing
    // completion to retain the same transitive operation facts while typing.
    let mut probe_source = source.to_owned();
    probe_source.replace_range(replacement.start..replacement.end, "None");
    let mut probe = CompilerDatabase::with_context(compiler_context, probe_source);
    #[cfg(test)]
    {
        database.completion_effects_cache().probes += 1;
    }
    let facts = probe.semantic_snapshot().ok().and_then(|snapshot| {
        snapshot.effects().map(|effects| {
            Arc::new(CompletionEffects::from_analysis(
                snapshot.syntax(),
                effects,
                snapshot.checked().map(|checked| checked.scoped_globals()),
            ))
        })
    });
    database
        .completion_effects_cache()
        .insert(replacement, facts.clone());
    facts
}

fn complete_state_decoder(source: &str, offset: usize) -> Option<CompletionList> {
    let replacement = identifier_span(source, offset);
    let before = source[..replacement.start].trim_end();
    let before_as = before.strip_suffix("as")?;
    if before_as
        .chars()
        .next_back()
        .is_some_and(|character| character.is_alphanumeric() || character == '_')
    {
        return None;
    }
    let field_fragment = before_as
        .rsplit_once(['\n', '{', ';'])
        .map_or(before_as, |(_, fragment)| fragment);
    if !field_fragment.split_whitespace().any(|token| token == "at") {
        return None;
    }

    let prefix = source[replacement.start..offset].to_owned();
    let mut builder = CompletionBuilder::new(prefix, replacement);
    let language = LanguageCatalog::new();
    let item = language.item(LanguageItemId::NativeStringDecoder);
    builder.add(catalog_language_completion(
        item.name,
        CompletionKind::Function,
        item,
        "utf8(${1:maxBytes})".to_owned(),
        true,
    ));
    let item = language.item(LanguageItemId::NativeUtf16LeDecoder);
    builder.add(catalog_language_completion(
        item.name,
        CompletionKind::Function,
        item,
        "utf16le(${1:maxUtf16Units})".to_owned(),
        true,
    ));
    Some(builder.finish())
}

#[derive(Debug, Clone)]
struct MemberContext {
    receiver_path: Vec<String>,
    receiver_offset: usize,
    dot: usize,
    prefix: String,
    replacement: Span,
}

pub(super) struct CompletionBuilder {
    prefix: String,
    replacement: Span,
    items: BTreeMap<String, CompletionItem>,
}

impl CompletionBuilder {
    pub(super) fn new(prefix: String, replacement: Span) -> Self {
        Self {
            prefix,
            replacement,
            items: BTreeMap::new(),
        }
    }

    pub(super) fn add(&mut self, item: CompletionItem) {
        if self.accepts(&item) {
            self.items.entry(item.label.clone()).or_insert(item);
        }
    }

    /// Lexical bindings shadow catalog and top-level candidates with the same
    /// spelling, just as they do during name resolution.
    fn add_scoped(&mut self, item: CompletionItem) {
        if self.accepts(&item) {
            self.items.insert(item.label.clone(), item);
        }
    }

    fn accepts(&self, item: &CompletionItem) -> bool {
        item.label
            .to_ascii_lowercase()
            .starts_with(&self.prefix.to_ascii_lowercase())
    }

    pub(super) fn finish(self) -> CompletionList {
        let mut items = self.items.into_values().collect::<Vec<_>>();
        items.sort_by_key(|item| (item.kind, item.label.to_ascii_lowercase()));
        CompletionList {
            replacement: self.replacement,
            items,
        }
    }
}

fn complete_root(
    source: &str,
    syntax: &Program,
    offset: usize,
    standard_library: StandardLibrary,
    availability: ContextAvailability,
    effects: Option<&CompletionEffects>,
    top_level: bool,
) -> CompletionList {
    let replacement = identifier_span(source, offset);
    if top_level {
        return complete_top_level(source, syntax, offset);
    }
    let prefix = source[replacement.start..offset].to_owned();
    let mut builder = CompletionBuilder::new(prefix, replacement);

    for item in LanguageCatalog::new().items() {
        if !availability.state_snapshots
            && matches!(
                item.id,
                LanguageItemId::CurrentSnapshot | LanguageItemId::OldSnapshot
            )
        {
            continue;
        }
        if let Some(completion) = language_completion(item, availability) {
            builder.add(completion);
        }
    }
    let provider = if availability.process_selection {
        standard_library.default_state_provider()
    } else {
        selected_provider(syntax, &standard_library)
    };
    add_root_standard_library(
        &mut builder,
        &standard_library,
        availability.attached_process,
    );
    if availability.attached_process
        && let Some(provider) = provider
    {
        let ty = standard_library.type_decl(provider.process_type);
        builder.add(CompletionItem {
            label: provider.value_name.to_owned(),
            kind: CompletionKind::Variable,
            detail: Some(ty.name.to_owned()),
            documentation: Some(render_documentation(&provider.documentation)),
            documentation_uri: Some(symbol_uri(
                StdlibSymbolId::StateProvider(provider.id),
                &standard_library,
            )),
            insert_text: provider.value_name.to_owned(),
            is_snippet: false,
        });
        for context in provider.contexts {
            let ty = standard_library.type_decl(context.ty);
            builder.add(CompletionItem {
                label: context.name.to_owned(),
                kind: CompletionKind::Variable,
                detail: Some(ty.name.to_owned()),
                documentation: Some(render_documentation(&context.documentation)),
                documentation_uri: Some(symbol_uri(
                    StdlibSymbolId::Type(context.ty),
                    &standard_library,
                )),
                insert_text: context.name.to_owned(),
                is_snippet: false,
            });
        }
    }
    add_source_declarations(
        &mut builder,
        syntax,
        availability.attached_process,
        availability.active_attempt,
        availability.state_snapshots,
        effects,
    );
    add_state_source_bindings(&mut builder, syntax, offset);
    add_visible_bindings(&mut builder, syntax, offset);
    builder.finish()
}

fn add_state_source_bindings(builder: &mut CompletionBuilder, syntax: &Program, offset: usize) {
    let Some(state) = &syntax.state else {
        return;
    };
    if state.has_provider_alternatives() {
        let Some((_, fields)) = state.provider_variant_fields().find(|(_, fields)| {
            fields
                .iter()
                .any(|field| contains_offset(field.span, offset))
        }) else {
            return;
        };
        for field in fields {
            builder.add_scoped(simple_completion(
                &field.name,
                CompletionKind::StateField,
                "sibling state field",
            ));
        }
        return;
    }
    let in_common = state
        .fields
        .iter()
        .any(|field| contains_offset(field.span, offset));
    let active_group = state.conditional_fields.iter().find(|group| {
        group
            .fields
            .iter()
            .any(|field| contains_offset(field.span, offset))
    });
    if !in_common && active_group.is_none() {
        return;
    }
    for field in &state.fields {
        builder.add_scoped(simple_completion(
            &field.name,
            CompletionKind::StateField,
            "sibling state field",
        ));
    }
    if let Some(group) = active_group {
        for field in &group.fields {
            builder.add_scoped(simple_completion(
                &field.name,
                CompletionKind::StateField,
                "conditional sibling state field",
            ));
        }
    }
}

fn is_top_level_offset(syntax: &Program, offset: usize) -> bool {
    !syntax
        .actions
        .iter()
        .any(|action| contains_offset(action.body.span, offset))
        && !syntax
            .functions
            .iter()
            .any(|function| contains_offset(function.body.span, offset))
        && syntax
            .state
            .as_ref()
            .is_none_or(|state| !contains_offset(state.span, offset))
        && syntax
            .settings
            .iter()
            .all(|setting| !contains_offset(setting.span, offset))
        && syntax
            .structs
            .iter()
            .all(|structure| !contains_offset(structure.span, offset))
        && syntax
            .enums
            .iter()
            .all(|enumeration| !contains_offset(enumeration.span, offset))
        && syntax
            .globals
            .iter()
            .all(|global| !contains_offset(global.span, offset))
}

fn complete_member(
    database: &mut CompilerDatabase,
    source: &str,
    syntax: &Program,
    tokens: &[&crate::lexer::Token],
    context: MemberContext,
    compiler_context: crate::CompilerContext,
) -> CompletionList {
    let standard_library = compiler_context.standard_library();
    let mut builder = CompletionBuilder::new(context.prefix.clone(), context.replacement);
    let active_shape_facts = active_attachment_shape_facts(syntax, tokens, context.dot);
    let path = context
        .receiver_path
        .iter()
        .map(String::as_str)
        .collect::<Vec<_>>();

    match path.as_slice() {
        ["current"] | ["old"] if snapshot_context_available(syntax, context.dot) => {
            if let Some(state) = &syntax.state {
                for field in state.common_fields() {
                    builder.add(simple_completion(
                        &field.name,
                        CompletionKind::StateField,
                        "state field",
                    ));
                }
                for field in conditionally_common_state_fields(state) {
                    builder.add(simple_completion(
                        &field.name,
                        CompletionKind::StateField,
                        "state field shared by every shape branch",
                    ));
                }
                for (index, group) in state.conditional_fields.iter().enumerate() {
                    if shape_group_is_active(
                        syntax,
                        &state.conditional_fields,
                        index,
                        &active_shape_facts,
                    ) {
                        for field in &group.fields {
                            builder.add(simple_completion(
                                &field.name,
                                CompletionKind::StateField,
                                "conditional state field",
                            ));
                        }
                    }
                }
            }
        }
        ["settings"] | ["oldSettings"] => {
            for setting in &syntax.settings {
                if setting.source_visible && !matches!(setting.kind, SettingKind::Title { .. }) {
                    let mut completion = simple_completion(
                        &setting.name,
                        CompletionKind::Setting,
                        &setting.description,
                    );
                    completion.documentation = setting.tooltip.clone();
                    builder.add(completion);
                }
            }
        }
        [name] => {
            if let Some(provider) = selected_provider_at(syntax, &standard_library, context.dot)
                && provider.value_name == *name
            {
                add_inferred_fields(
                    &mut builder,
                    syntax,
                    &TypeKind::Standard(provider.process_type),
                    &standard_library,
                    &active_shape_facts,
                );
                add_inferred_methods(
                    &mut builder,
                    syntax,
                    &TypeKind::Standard(provider.process_type),
                    &[],
                    &standard_library,
                );
            }
            if let Some(context) = selected_provider_at(syntax, &standard_library, context.dot)
                .and_then(|provider| {
                    provider
                        .contexts
                        .iter()
                        .find(|context| context.name == *name)
                })
            {
                add_inferred_fields(
                    &mut builder,
                    syntax,
                    &TypeKind::Standard(context.ty),
                    &standard_library,
                    &active_shape_facts,
                );
                add_inferred_methods(
                    &mut builder,
                    syntax,
                    &TypeKind::Standard(context.ty),
                    &[],
                    &standard_library,
                );
            }
            if let Some(class) = syntax
                .managed_class_declarations()
                .into_iter()
                .find(|class| class.name == *name)
            {
                builder.add(CompletionItem {
                    label: "instances".to_owned(),
                    kind: CompletionKind::Method,
                    detail: Some(format!(
                        "{}.instances() -> async [{}\u{2e}Ref]",
                        class.name, class.name
                    )),
                    documentation: Some(
                        "Cooperatively scans writable process memory and returns a completed snapshot of live references to this managed class."
                            .to_owned(),
                    ),
                    documentation_uri: None,
                    insert_text: "instances()".to_owned(),
                    is_snippet: false,
                });
                for field in class.fields.iter().filter(|field| field.is_static) {
                    let mut completion = simple_completion(
                        &field.name,
                        CompletionKind::Property,
                        "managed static field",
                    );
                    completion.documentation = field.documentation.clone();
                    builder.add(completion);
                }
                for (index, group) in class.conditional_fields.iter().enumerate() {
                    if shape_group_is_active(
                        syntax,
                        &class.conditional_fields,
                        index,
                        &active_shape_facts,
                    ) {
                        for field in group.fields.iter().filter(|field| field.is_static) {
                            let mut completion = simple_completion(
                                &field.name,
                                CompletionKind::Property,
                                "conditional managed static field",
                            );
                            completion.documentation = field.documentation.clone();
                            builder.add(completion);
                        }
                    }
                }
            }
        }
        _ => {}
    }

    add_source_enum_variant_completions(&mut builder, syntax, &path.join("."), false);

    if !path.is_empty() {
        add_standard_library_path_members(&mut builder, &path, &standard_library);
    }

    if let Some(receiver) = infer_receiver(database, source, tokens, &context, compiler_context) {
        add_inferred_fields(
            &mut builder,
            syntax,
            &receiver.ty,
            &standard_library,
            &active_shape_facts,
        );
        add_inferred_methods(
            &mut builder,
            syntax,
            &receiver.ty,
            &receiver.constraints,
            &standard_library,
        );
    }
    builder.finish()
}

fn selected_provider(
    syntax: &Program,
    standard_library: &StandardLibrary,
) -> Option<&'static crate::stdlib::StdlibStateProvider> {
    let state = syntax.state.as_ref()?;
    state
        .provider
        .as_ref()
        .and_then(|provider| standard_library.state_provider_by_name(&provider.name))
        .or_else(|| {
            state
                .provider
                .is_none()
                .then(|| standard_library.default_state_provider())
                .flatten()
        })
}

fn selected_provider_at(
    syntax: &Program,
    standard_library: &StandardLibrary,
    offset: usize,
) -> Option<&'static crate::stdlib::StdlibStateProvider> {
    if syntax.actions.iter().any(|action| {
        action.kind == crate::ast::ActionKind::SelectProcess
            && contains_offset(action.body.span, offset)
    }) {
        standard_library.default_state_provider()
    } else {
        selected_provider(syntax, standard_library)
    }
}

fn is_statement_position(
    source: &str,
    tokens: &[&crate::lexer::Token],
    cursor_start: usize,
) -> bool {
    let before = tokens.partition_point(|token| token.span.end <= cursor_start);
    let Some(previous) = before.checked_sub(1).map(|index| tokens[index]) else {
        return true;
    };
    if matches!(previous.kind, TokenKind::LBrace | TokenKind::Semicolon) {
        return true;
    }
    if !source[previous.span.end..cursor_start].contains(['\n', '\r']) {
        return false;
    }
    !matches!(
        previous.kind,
        TokenKind::Assign
            | TokenKind::PlusAssign
            | TokenKind::MinusAssign
            | TokenKind::StarAssign
            | TokenKind::SlashAssign
            | TokenKind::PercentAssign
            | TokenKind::OrAssign
            | TokenKind::AndAssign
            | TokenKind::CaretAssign
            | TokenKind::ShlAssign
            | TokenKind::ShrAssign
            | TokenKind::Plus
            | TokenKind::Minus
            | TokenKind::Star
            | TokenKind::Slash
            | TokenKind::Percent
            | TokenKind::Or
            | TokenKind::And
            | TokenKind::Caret
            | TokenKind::OrOr
            | TokenKind::AndAnd
            | TokenKind::EqEq
            | TokenKind::BangEq
            | TokenKind::Lt
            | TokenKind::Le
            | TokenKind::Gt
            | TokenKind::Ge
            | TokenKind::Shl
            | TokenKind::Shr
            | TokenKind::Dot
            | TokenKind::Comma
            | TokenKind::Colon
            | TokenKind::LParen
            | TokenKind::LBracket
            | TokenKind::FatArrow
    )
}

fn cursor_inside_loop(syntax: &Program, offset: usize) -> bool {
    struct Finder {
        offset: usize,
        loop_start: Option<usize>,
        closure_start: Option<usize>,
    }

    impl<'ast> Visitor<'ast> for Finder {
        fn visit_stmt(&mut self, statement: &'ast Stmt) {
            let body = match statement {
                Stmt::While { body, .. } | Stmt::For { body, .. } => Some(body),
                _ => None,
            };
            if let Some(body) = body
                && contains_offset(body.span, self.offset)
            {
                self.loop_start = Some(
                    self.loop_start
                        .map_or(body.span.start, |start| start.max(body.span.start)),
                );
            }
            visit::walk_stmt(self, statement);
        }

        fn visit_expr(&mut self, expression: &'ast Expr) {
            match &expression.kind {
                ExprKind::Loop(body) if contains_offset(body.span, self.offset) => {
                    self.loop_start = Some(
                        self.loop_start
                            .map_or(body.span.start, |start| start.max(body.span.start)),
                    );
                }
                ExprKind::Closure { body, .. }
                    if contains_offset(expression.span, self.offset)
                        || contains_offset(body.span, self.offset) =>
                {
                    self.closure_start =
                        Some(self.closure_start.map_or(expression.span.start, |start| {
                            start.max(expression.span.start)
                        }));
                }
                _ => {}
            }
            visit::walk_expr(self, expression);
        }
    }

    let mut finder = Finder {
        offset,
        loop_start: None,
        closure_start: None,
    };
    finder.visit_program(syntax);
    finder
        .loop_start
        .is_some_and(|start| finder.closure_start.is_none_or(|closure| closure < start))
}

fn cursor_inside_generator(syntax: &Program, offset: usize) -> bool {
    struct Finder {
        offset: usize,
        innermost: Option<(usize, bool)>,
    }

    impl Finder {
        fn record(&mut self, span: Span, generator: bool) {
            if contains_offset(span, self.offset) {
                let width = span.end.saturating_sub(span.start);
                if self
                    .innermost
                    .is_none_or(|(current_width, _)| width < current_width)
                {
                    self.innermost = Some((width, generator));
                }
            }
        }
    }

    impl<'ast> Visitor<'ast> for Finder {
        fn visit_function(&mut self, function: &'ast crate::ast::FunctionDecl) {
            self.record(function.body.span, function.return_is_iterator);
            visit::walk_function(self, function);
        }

        fn visit_expr(&mut self, expression: &'ast Expr) {
            if let ExprKind::Closure {
                return_annotation,
                body,
                ..
            } = &expression.kind
            {
                self.record(
                    body.span,
                    return_annotation.is_some_and(|ty| matches!(ty, SyntaxTypeRef::Iterator(_))),
                );
            }
            visit::walk_expr(self, expression);
        }
    }

    let mut finder = Finder {
        offset,
        innermost: None,
    };
    finder.visit_program(syntax);
    finder.innermost.is_some_and(|(_, generator)| generator)
}

fn language_completion(
    item: &LanguageItem,
    availability: ContextAvailability,
) -> Option<CompletionItem> {
    // `settings` names both the top-level declaration and the live settings
    // view. Top-level completion is handled before this expression-oriented
    // path, so expose the value interpretation here instead of dropping it
    // with declarations in general.
    if item.id == LanguageItemId::Settings {
        return Some(CompletionItem {
            label: item.name.to_owned(),
            kind: CompletionKind::Variable,
            detail: Some("SettingsView".to_owned()),
            documentation: Some(render_documentation(&item.documentation)),
            documentation_uri: Some(language_item_uri(item.id)),
            insert_text: item.name.to_owned(),
            is_snippet: false,
        });
    }
    if matches!(
        item.id,
        LanguageItemId::NativeStringDecoder | LanguageItemId::NativeUtf16LeDecoder
    ) {
        return None;
    }
    let (kind, insert_text, is_snippet) = match item.kind {
        LanguageItemKind::Action(_) | LanguageItemKind::Declaration => return None,
        LanguageItemKind::BuiltinType(_) => (CompletionKind::Type, item.name.to_owned(), false),
        LanguageItemKind::SnapshotRoot => (CompletionKind::Variable, item.name.to_owned(), false),
        LanguageItemKind::Keyword | LanguageItemKind::Syntax => {
            let completion = LanguageCatalog::new().completion(item.id)?;
            let available = match completion.site {
                LanguageCompletionSite::Expression => true,
                LanguageCompletionSite::Statement => availability.statement_position,
                LanguageCompletionSite::Loop => availability.loop_control,
                LanguageCompletionSite::Return => availability.return_control,
                LanguageCompletionSite::Generator => availability.generator_control,
                LanguageCompletionSite::Method => availability.method_receiver,
            };
            if !available {
                return None;
            }
            (
                if completion.is_snippet {
                    CompletionKind::Snippet
                } else {
                    CompletionKind::Keyword
                },
                completion.insert_text.to_owned(),
                completion.is_snippet,
            )
        }
    };
    Some(catalog_language_completion(
        item.name,
        kind,
        item,
        insert_text,
        is_snippet,
    ))
}

fn catalog_language_completion(
    label: &str,
    kind: CompletionKind,
    item: &LanguageItem,
    insert_text: String,
    is_snippet: bool,
) -> CompletionItem {
    CompletionItem {
        label: label.to_owned(),
        kind,
        detail: Some(item.form.to_owned()),
        documentation: Some(render_documentation(&item.documentation)),
        documentation_uri: Some(language_item_uri(item.id)),
        insert_text,
        is_snippet,
    }
}

fn add_root_standard_library(
    builder: &mut CompletionBuilder,
    library: &StandardLibrary,
    has_attached_process: bool,
) {
    for namespace in library
        .namespaces()
        .iter()
        .filter(|namespace| namespace.path.len() == 1)
    {
        builder.add(stdlib_namespace_completion(namespace, library));
    }
    for ty in library.types() {
        builder.add(CompletionItem {
            label: ty.name.to_owned(),
            kind: match ty.kind {
                crate::stdlib::StdlibTypeKind::Intrinsic => CompletionKind::Type,
                crate::stdlib::StdlibTypeKind::Struct => CompletionKind::Struct,
                crate::stdlib::StdlibTypeKind::Enum => CompletionKind::Enum,
            },
            detail: Some("standard-library type".to_owned()),
            documentation: Some(render_documentation(&ty.documentation)),
            documentation_uri: Some(symbol_uri(StdlibSymbolId::Type(ty.id), library)),
            insert_text: ty.name.to_owned(),
            is_snippet: false,
        });
    }
    // A named generic constructor is meaningful in an expression only when it
    // owns a public static member, such as `Set.new` or `Map.new`. Do not leak
    // ordinary generic type names (including implementation-only iterator
    // adapters) into value completion merely because they are valid in types.
    for constructor in library.public_type_constructors().filter(|constructor| {
        constructor.syntax == TypeConstructorSyntax::Named
            && library.items().any(|item| {
                item.owner == crate::stdlib::StdlibOwner::TypeConstructor(constructor.id)
                    && item.visibility == ItemVisibility::Public
                    && !matches!(item.kind, ItemKind::Method { .. })
            })
    }) {
        builder.add(stdlib_type_constructor_completion(
            constructor,
            library,
            constructor.name.to_owned(),
            false,
        ));
    }
    for item in library.items() {
        let Some(path) = library.item_path(item) else {
            continue;
        };
        if path.len() == 1 {
            if !has_attached_process
                && library
                    .operation_semantics(item.id)
                    .requires_attached_process
            {
                continue;
            }
            builder.add(stdlib_completion(
                item.name,
                item,
                CompletionKind::Function,
                library,
            ));
        }
    }
}

fn stdlib_type_constructor_completion(
    constructor: &StdlibTypeConstructor,
    library: &StandardLibrary,
    insert_text: String,
    is_snippet: bool,
) -> CompletionItem {
    CompletionItem {
        label: constructor.name.to_owned(),
        kind: CompletionKind::Type,
        detail: Some(library.render_type_constructor(constructor.id)),
        documentation: Some(render_documentation(&constructor.documentation)),
        documentation_uri: Some(symbol_uri(
            StdlibSymbolId::TypeConstructor(constructor.id),
            library,
        )),
        insert_text,
        is_snippet,
    }
}

fn stdlib_namespace_completion(
    namespace: &StdlibNamespace,
    library: &StandardLibrary,
) -> CompletionItem {
    CompletionItem {
        label: namespace.name.to_owned(),
        kind: CompletionKind::Namespace,
        detail: Some("standard-library namespace".to_owned()),
        documentation: Some(render_documentation(&namespace.documentation)),
        documentation_uri: Some(symbol_uri(StdlibSymbolId::Namespace(namespace.id), library)),
        insert_text: namespace.name.to_owned(),
        is_snippet: false,
    }
}

fn add_standard_library_path_members(
    builder: &mut CompletionBuilder,
    prefix: &[&str],
    library: &StandardLibrary,
) {
    if let [type_name] = prefix
        && let Some(ty) = library.type_by_name(type_name)
    {
        for variant in library.variants_of(ty.id) {
            builder.add(CompletionItem {
                label: variant.name.to_owned(),
                kind: CompletionKind::EnumMember,
                detail: Some(format!("{}.{}", ty.name, variant.name)),
                documentation: Some(render_documentation(&variant.documentation)),
                documentation_uri: Some(symbol_uri(StdlibSymbolId::Variant(variant.id), library)),
                insert_text: variant.name.to_owned(),
                is_snippet: false,
            });
        }
    }

    for item in library.items() {
        if let [type_name] = prefix
            && let Some(constructor) = library.named_type_constructor_by_name(type_name)
            && item.owner == crate::stdlib::StdlibOwner::TypeConstructor(constructor.id)
            && matches!(item.kind, crate::stdlib::ItemKind::Method { .. })
        {
            continue;
        }
        let Some(path) = library.item_path(item) else {
            continue;
        };
        if path.len() <= prefix.len() || path[..prefix.len()] != *prefix {
            continue;
        }
        let label = path[prefix.len()];
        if path.len() == prefix.len() + 1 {
            let kind = if item.kind == ItemKind::Constant {
                CompletionKind::Constant
            } else {
                CompletionKind::Function
            };
            builder.add(stdlib_completion(label, item, kind, library));
        }
    }

    for namespace in library.namespaces().iter().filter(|namespace| {
        namespace.path.len() == prefix.len() + 1 && namespace.path[..prefix.len()] == *prefix
    }) {
        builder.add(stdlib_namespace_completion(namespace, library));
    }
}

fn stdlib_completion(
    label: &str,
    item: &StdlibItem,
    kind: CompletionKind,
    library: &StandardLibrary,
) -> CompletionItem {
    let documentation = StandardLibraryDocumentation::generate_with_library(library, item.id, &[]);
    CompletionItem {
        label: label.to_owned(),
        kind,
        detail: Some(documentation.signature.clone()),
        documentation: Some(documentation.summary_markdown()),
        documentation_uri: Some(symbol_uri(StdlibSymbolId::Item(item.id), library)),
        insert_text: if item.kind == ItemKind::Constant {
            label.to_owned()
        } else {
            function_snippet(label, item)
        },
        is_snippet: item.kind != ItemKind::Constant,
    }
}

fn function_snippet(label: &str, item: &StdlibItem) -> String {
    let parameters = item
        .signature
        .parameters
        .iter()
        .enumerate()
        .map(|(index, parameter)| format!("${{{}:{}}}", index + 1, parameter.name))
        .collect::<Vec<_>>()
        .join(", ");
    format!("{label}({parameters})")
}

fn add_source_declarations(
    builder: &mut CompletionBuilder,
    syntax: &Program,
    has_attached_process: bool,
    has_active_attempt: bool,
    has_state_snapshots: bool,
    effects: Option<&CompletionEffects>,
) {
    if syntax
        .state
        .as_ref()
        .is_some_and(|state| state.provider_value.is_some())
    {
        builder.add(simple_completion(
            "provider",
            CompletionKind::Variable,
            "selected state provider",
        ));
    }
    for global in &syntax.globals {
        if effects.is_some_and(|effects| {
            matches!(
                effects.global_lifetimes.get(&global.id),
                Some(GlobalLifetime::Attachment) if !has_attached_process
            ) || matches!(
                effects.global_lifetimes.get(&global.id),
                Some(GlobalLifetime::Attempt) if !has_active_attempt
            )
        }) {
            continue;
        }
        let mut names = std::collections::HashSet::new();
        global.binding.visit_bindings(&mut |binding| {
            if names.insert(binding.name.clone()) {
                builder.add(simple_completion(
                    &binding.name,
                    CompletionKind::Variable,
                    "global variable",
                ));
            }
        });
    }
    for function in &syntax.functions {
        if function.method_of.is_some() {
            continue;
        }
        if !has_attached_process
            && effects.is_none_or(|effects| effects.attached_process.contains(&function.id))
        {
            continue;
        }
        if !has_state_snapshots
            && effects.is_none_or(|effects| effects.state_snapshots.contains(&function.id))
        {
            continue;
        }
        if !has_active_attempt
            && effects.is_some_and(|effects| effects.attempt_functions.contains(&function.id))
        {
            continue;
        }
        let parameters = function
            .params
            .iter()
            .enumerate()
            .map(|(index, parameter)| format!("${{{}:{}}}", index + 1, parameter.name))
            .collect::<Vec<_>>()
            .join(", ");
        builder.add(CompletionItem {
            label: function.name.clone(),
            kind: CompletionKind::Function,
            detail: Some("user function".to_owned()),
            documentation: None,
            documentation_uri: None,
            insert_text: format!("{}({parameters})", function.name),
            is_snippet: true,
        });
    }
    for structure in &syntax.structs {
        builder.add(simple_completion(
            &structure.name,
            CompletionKind::Struct,
            "struct type",
        ));
    }
    add_source_enum_type_completions(builder, syntax, None, false);
}

pub(super) fn add_source_enum_type_completions(
    builder: &mut CompletionBuilder,
    syntax: &Program,
    required_name: Option<&str>,
    require_payloadless_variant: bool,
) {
    for enumeration in syntax.enum_declarations().filter(|enumeration| {
        required_name.is_none_or(|required| enumeration.name == required)
            && (!require_payloadless_variant
                || enumeration
                    .variants
                    .iter()
                    .any(|variant| variant.payload.is_none()))
    }) {
        builder.add(simple_completion(
            &enumeration.name,
            CompletionKind::Enum,
            "enum type",
        ));
    }
}

pub(super) fn add_source_enum_variant_completions(
    builder: &mut CompletionBuilder,
    syntax: &Program,
    enum_name: &str,
    payloadless_only: bool,
) {
    let Some(enumeration) = syntax
        .enum_declarations()
        .find(|enumeration| enumeration.name == enum_name)
    else {
        return;
    };
    for variant in enumeration
        .variants
        .iter()
        .filter(|variant| !payloadless_only || variant.payload.is_none())
    {
        let (insert_text, is_snippet) = if variant.payload.is_some() {
            (format!("{}(${{1:value}})", variant.name), true)
        } else {
            (variant.name.clone(), false)
        };
        builder.add(CompletionItem {
            label: variant.name.clone(),
            kind: CompletionKind::EnumMember,
            detail: Some(format!("{}.{}", enumeration.name, variant.name)),
            documentation: variant.documentation.clone(),
            documentation_uri: None,
            insert_text,
            is_snippet,
        });
    }
}

fn snapshot_context_available(syntax: &Program, offset: usize) -> bool {
    if syntax
        .functions
        .iter()
        .any(|function| contains_offset(function.body.span, offset))
    {
        return true;
    }
    syntax
        .actions
        .iter()
        .find(|action| contains_offset(action.body.span, offset))
        .is_some_and(|action| action_has_state_snapshots(action.kind))
}

fn add_visible_bindings(builder: &mut CompletionBuilder, syntax: &Program, offset: usize) {
    if let Some(function) = syntax
        .functions
        .iter()
        .find(|function| contains_offset(function.body.span, offset))
    {
        for parameter in &function.params {
            add_binding_pattern(builder, &parameter.binding, "parameter");
        }
        add_block_bindings(builder, &function.body, offset);
        return;
    }
    if let Some(action) = syntax
        .actions
        .iter()
        .find(|action| contains_offset(action.body.span, offset))
    {
        add_block_bindings(builder, &action.body, offset);
    }
}

fn add_block_bindings(builder: &mut CompletionBuilder, block: &Block, offset: usize) {
    for statement in &block.statements {
        let span = statement_span(statement);
        if offset < span.start {
            break;
        }
        if offset <= span.end {
            add_statement_inner_bindings(builder, statement, offset);
            break;
        }
        add_completed_statement_binding(builder, statement);
    }
}

fn add_completed_statement_binding(builder: &mut CompletionBuilder, statement: &Stmt) {
    match statement {
        Stmt::Debug { statement, .. } => add_completed_statement_binding(builder, statement),
        Stmt::Variable(variable) => {
            add_binding_pattern(builder, &variable.binding, "local variable");
        }
        Stmt::Suspend {
            binding: Some(binding),
            ..
        } => add_scoped_variable(builder, &binding.name, "local variable"),
        Stmt::Assign { .. }
        | Stmt::StateAssign { .. }
        | Stmt::IndexAssign { .. }
        | Stmt::If { .. }
        | Stmt::While { .. }
        | Stmt::For { .. }
        | Stmt::Suspend { binding: None, .. }
        | Stmt::Yield { .. }
        | Stmt::Expression(_) => {}
    }
}

fn add_statement_inner_bindings(builder: &mut CompletionBuilder, statement: &Stmt, offset: usize) {
    match statement {
        Stmt::Debug { statement, .. } => {
            if contains_offset(statement_span(statement), offset) {
                add_statement_inner_bindings(builder, statement, offset);
            }
        }
        Stmt::Variable(variable) => {
            if let Some(value) = &variable.value {
                add_expression_bindings(builder, value, offset);
            }
        }
        Stmt::Assign { value, .. }
        | Stmt::Suspend { value, .. }
        | Stmt::Yield { value, .. }
        | Stmt::Expression(value) => {
            add_expression_bindings(builder, value, offset);
        }
        Stmt::StateAssign { target, value, .. } | Stmt::IndexAssign { target, value, .. } => {
            for expression in [target, value] {
                if contains_offset(expression.span, offset) {
                    add_expression_bindings(builder, expression, offset);
                    break;
                }
            }
        }
        Stmt::If {
            condition,
            then_block,
            else_block,
            ..
        } => {
            if contains_offset(condition.span, offset) {
                add_expression_bindings(builder, condition, offset);
            } else if contains_offset(then_block.span, offset) {
                add_condition_binding_names(builder, condition, true);
                add_block_bindings(builder, then_block, offset);
            } else if let Some(else_block) = else_block
                && contains_offset(else_block.span, offset)
            {
                add_condition_binding_names(builder, condition, false);
                add_block_bindings(builder, else_block, offset);
            }
        }
        Stmt::While {
            condition, body, ..
        } => {
            if contains_offset(condition.span, offset) {
                add_expression_bindings(builder, condition, offset);
            } else if contains_offset(body.span, offset) {
                add_condition_binding_names(builder, condition, true);
                add_block_bindings(builder, body, offset);
            }
        }
        Stmt::For {
            binding,
            iterable,
            body,
            ..
        } => {
            if contains_offset(iterable.span, offset) {
                add_expression_bindings(builder, iterable, offset);
            } else if contains_offset(body.span, offset) {
                add_binding_pattern(builder, &binding.binding, "loop binding");
                add_block_bindings(builder, body, offset);
            }
        }
    }
}

fn add_expression_bindings(builder: &mut CompletionBuilder, expression: &Expr, offset: usize) {
    if !contains_offset(expression.span, offset) {
        return;
    }
    match &expression.kind {
        ExprKind::Match { value, arms } => {
            if contains_offset(value.span, offset) {
                add_expression_bindings(builder, value, offset);
                return;
            }
            for arm in arms {
                if let Some(guard) = arm
                    .guard
                    .as_ref()
                    .filter(|guard| contains_offset(guard.span, offset))
                {
                    add_pattern_binding(builder, &arm.pattern);
                    add_expression_bindings(builder, guard, offset);
                    return;
                }
                if contains_offset(arm.value.span, offset) {
                    add_pattern_binding(builder, &arm.pattern);
                    if let Some(guard) = &arm.guard {
                        add_condition_binding_names(builder, guard, true);
                    }
                    add_expression_bindings(builder, &arm.value, offset);
                    return;
                }
            }
        }
        ExprKind::Is { value, .. } => add_expression_bindings(builder, value, offset),
        ExprKind::InterpolatedString(parts) => {
            for part in parts {
                if let crate::ast::InterpolatedPart::Expr(part) = part {
                    add_expression_bindings(builder, part, offset);
                }
            }
        }
        ExprKind::Array(values) => add_child_expression_bindings(builder, values, offset),
        ExprKind::Range { start, end, .. } => {
            add_expression_bindings(builder, start, offset);
            add_expression_bindings(builder, end, offset);
        }
        ExprKind::Block(block) | ExprKind::Loop(block) => {
            add_block_bindings(builder, block, offset)
        }
        ExprKind::Struct { fields, .. } => {
            for field in fields {
                add_expression_bindings(builder, &field.value, offset);
            }
        }
        ExprKind::If {
            condition,
            then_expr,
            else_expr,
        } => {
            if contains_offset(condition.span, offset) {
                add_expression_bindings(builder, condition, offset);
            } else if contains_offset(then_expr.span, offset) {
                add_condition_binding_names(builder, condition, true);
                add_expression_bindings(builder, then_expr, offset);
            } else if contains_offset(else_expr.span, offset) {
                add_condition_binding_names(builder, condition, false);
                add_expression_bindings(builder, else_expr, offset);
            }
        }
        ExprKind::Fallback { value, fallback } => {
            add_expression_bindings(builder, value, offset);
            add_expression_bindings(builder, fallback, offset);
        }
        ExprKind::Break(Some(value))
        | ExprKind::Return(Some(value))
        | ExprKind::Throw(value)
        | ExprKind::Suspend { value, .. }
        | ExprKind::Propagate(value)
        | ExprKind::Member {
            receiver: value, ..
        }
        | ExprKind::Unary { expr: value, .. } => {
            add_expression_bindings(builder, value, offset);
        }
        ExprKind::Index {
            receiver, index, ..
        } => {
            add_expression_bindings(builder, receiver, offset);
            add_expression_bindings(builder, index, offset);
        }
        ExprKind::Cast { expr, .. } => add_expression_bindings(builder, expr, offset),
        ExprKind::Binary { op, left, right } => {
            if contains_offset(left.span, offset) {
                add_expression_bindings(builder, left, offset);
            } else if contains_offset(right.span, offset) {
                match op {
                    crate::ast::BinaryOp::And => add_condition_binding_names(builder, left, true),
                    crate::ast::BinaryOp::Or => add_condition_binding_names(builder, left, false),
                    _ => {}
                }
                add_expression_bindings(builder, right, offset);
            }
        }
        ExprKind::Call { args, .. } => add_child_expression_bindings(builder, args, offset),
        ExprKind::Invoke { callee, args } => {
            add_expression_bindings(builder, callee, offset);
            add_child_expression_bindings(builder, args, offset);
        }
        ExprKind::Closure { params, body, .. } => {
            for parameter in params {
                add_binding_pattern(builder, &parameter.binding, "closure parameter");
            }
            add_expression_bindings(builder, body, offset);
        }
        ExprKind::Error
        | ExprKind::None
        | ExprKind::Break(None)
        | ExprKind::Continue
        | ExprKind::IteratorEnd
        | ExprKind::Return(None)
        | ExprKind::Bool(_)
        | ExprKind::Int { .. }
        | ExprKind::Float(_)
        | ExprKind::Char(_)
        | ExprKind::String(_)
        | ExprKind::Signature(_)
        | ExprKind::Path(_) => {}
    }
}

fn add_child_expression_bindings(
    builder: &mut CompletionBuilder,
    expressions: &[Expr],
    offset: usize,
) {
    for expression in expressions {
        add_expression_bindings(builder, expression, offset);
    }
}

fn add_pattern_binding(builder: &mut CompletionBuilder, pattern: &MatchPattern) {
    pattern.visit_bindings(&mut |binding| {
        add_scoped_variable(builder, &binding.name, "pattern binding");
    });
}

fn add_binding_pattern(
    builder: &mut CompletionBuilder,
    binding: &crate::ast::BindingPattern,
    detail: &str,
) {
    binding.visit_bindings(&mut |leaf| {
        add_scoped_variable(builder, &leaf.name, detail);
    });
}

#[derive(Clone)]
struct CompletionConditionFlow {
    when_true: Option<BTreeSet<String>>,
    when_false: Option<BTreeSet<String>>,
}

fn add_condition_binding_names(builder: &mut CompletionBuilder, condition: &Expr, outcome: bool) {
    let flow = completion_condition_flow(condition);
    let bindings = if outcome {
        flow.when_true
    } else {
        flow.when_false
    };
    for name in bindings.into_iter().flatten() {
        add_scoped_variable(builder, &name, "conditional pattern binding");
    }
}

/// Mirrors the type checker's path proof algebra using only recovered syntax.
/// Completion cannot require a fully valid condition, while the type checker
/// remains the authority that validates types and binding identity.
fn completion_condition_flow(condition: &Expr) -> CompletionConditionFlow {
    let unknown = || CompletionConditionFlow {
        when_true: Some(BTreeSet::new()),
        when_false: Some(BTreeSet::new()),
    };
    match &condition.kind {
        ExprKind::Bool(value) => CompletionConditionFlow {
            when_true: value.then(BTreeSet::new),
            when_false: (!value).then(BTreeSet::new),
        },
        ExprKind::Is { pattern, .. } => {
            let mut bindings = BTreeSet::new();
            pattern
                .kind
                .visit_bindings(&mut |binding| _ = bindings.insert(binding.name.clone()));
            CompletionConditionFlow {
                when_true: Some(bindings),
                when_false: Some(BTreeSet::new()),
            }
        }
        ExprKind::Unary {
            op: crate::ast::UnaryOp::Not,
            expr,
        } => {
            let flow = completion_condition_flow(expr);
            CompletionConditionFlow {
                when_true: flow.when_false,
                when_false: flow.when_true,
            }
        }
        ExprKind::Binary {
            op: crate::ast::BinaryOp::And,
            left,
            right,
        } => {
            let left = completion_condition_flow(left);
            let right = completion_condition_flow(right);
            CompletionConditionFlow {
                when_true: sequence_completion_paths(&left.when_true, &right.when_true),
                when_false: join_completion_paths(
                    &left.when_false,
                    &sequence_completion_paths(&left.when_true, &right.when_false),
                ),
            }
        }
        ExprKind::Binary {
            op: crate::ast::BinaryOp::Or,
            left,
            right,
        } => {
            let left = completion_condition_flow(left);
            let right = completion_condition_flow(right);
            CompletionConditionFlow {
                when_true: join_completion_paths(
                    &left.when_true,
                    &sequence_completion_paths(&left.when_false, &right.when_true),
                ),
                when_false: sequence_completion_paths(&left.when_false, &right.when_false),
            }
        }
        _ => unknown(),
    }
}

fn sequence_completion_paths(
    first: &Option<BTreeSet<String>>,
    second: &Option<BTreeSet<String>>,
) -> Option<BTreeSet<String>> {
    let mut combined = first.clone()?;
    combined.extend(second.clone()?);
    Some(combined)
}

fn join_completion_paths(
    first: &Option<BTreeSet<String>>,
    second: &Option<BTreeSet<String>>,
) -> Option<BTreeSet<String>> {
    match (first, second) {
        (None, path) | (path, None) => path.clone(),
        (Some(first), Some(second)) => Some(first.intersection(second).cloned().collect()),
    }
}

fn add_scoped_variable(builder: &mut CompletionBuilder, name: &str, detail: &str) {
    builder.add_scoped(simple_completion(name, CompletionKind::Variable, detail));
}

fn conditionally_common_state_fields(
    state: &crate::ast::StateDecl,
) -> Vec<&crate::ast::StateField> {
    let mut common = Vec::new();
    let mut chain_start = 0;
    while chain_start < state.conditional_fields.len() {
        let mut chain_end = chain_start + 1;
        while chain_end < state.conditional_fields.len()
            && state.conditional_fields[chain_end].else_span.is_some()
        {
            chain_end += 1;
        }
        let chain = &state.conditional_fields[chain_start..chain_end];
        if chain.last().is_some_and(|group| group.condition.is_none()) {
            for candidate in &chain[0].fields {
                let declarations = chain
                    .iter()
                    .map(|group| {
                        group
                            .fields
                            .iter()
                            .find(|field| field.name == candidate.name)
                    })
                    .collect::<Option<Vec<_>>>();
                if declarations.is_some_and(|declarations| {
                    let mut annotation = None;
                    declarations.iter().all(|field| match field.annotation {
                        Some(found) if annotation.is_some_and(|expected| expected != found) => {
                            false
                        }
                        Some(found) => {
                            annotation = Some(found);
                            true
                        }
                        None => true,
                    })
                }) {
                    common.push(candidate);
                }
            }
        }
        chain_start = chain_end;
    }
    common
}

fn contains_offset(span: Span, offset: usize) -> bool {
    span.start <= offset && offset <= span.end
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
enum CompletionShapeDimension {
    Value(crate::ast::ValueId),
}

fn statement_span(statement: &Stmt) -> Span {
    match statement {
        Stmt::Debug { span, .. }
        | Stmt::Assign { span, .. }
        | Stmt::StateAssign { span, .. }
        | Stmt::IndexAssign { span, .. }
        | Stmt::If { span, .. }
        | Stmt::While { span, .. }
        | Stmt::For { span, .. }
        | Stmt::Suspend { span, .. }
        | Stmt::Yield { span, .. } => *span,
        Stmt::Variable(variable) => variable.span,
        Stmt::Expression(expression) => expression.span,
    }
}

fn add_inferred_fields(
    builder: &mut CompletionBuilder,
    syntax: &Program,
    receiver: &TypeKind,
    standard_library: &StandardLibrary,
    active_shape_facts: &[(CompletionShapeDimension, crate::ast::EnumVariantId)],
) {
    match receiver {
        TypeKind::Error => {}
        TypeKind::StateSnapshot => {
            if let Some(state) = &syntax.state {
                for field in state.common_fields() {
                    builder.add(simple_completion(
                        &field.name,
                        CompletionKind::Property,
                        "state field",
                    ));
                }
                for field in conditionally_common_state_fields(state) {
                    builder.add(simple_completion(
                        &field.name,
                        CompletionKind::Property,
                        "state field shared by every shape branch",
                    ));
                }
                for (index, group) in state.conditional_fields.iter().enumerate() {
                    if shape_group_is_active(
                        syntax,
                        &state.conditional_fields,
                        index,
                        active_shape_facts,
                    ) {
                        for field in &group.fields {
                            builder.add(simple_completion(
                                &field.name,
                                CompletionKind::Property,
                                "conditional state field",
                            ));
                        }
                    }
                }
            }
        }
        TypeKind::SettingsView => {
            for setting in &syntax.settings {
                if setting.source_visible && !matches!(setting.kind, SettingKind::Title { .. }) {
                    let mut completion = simple_completion(
                        &setting.name,
                        CompletionKind::Setting,
                        &setting.description,
                    );
                    completion.documentation = setting.tooltip.clone();
                    builder.add(completion);
                }
            }
        }
        TypeKind::Struct(id) => {
            if let Some(structure) = syntax.structs.iter().find(|structure| structure.id == *id) {
                for field in &structure.fields {
                    builder.add(simple_completion(
                        &field.name,
                        CompletionKind::Property,
                        "struct field",
                    ));
                }
            }
        }
        TypeKind::ManagedClass(id) | TypeKind::ManagedReference(id) => {
            if let Some(class) = syntax.managed_class(*id) {
                let conditional_fields = class
                    .conditional_fields
                    .iter()
                    .enumerate()
                    .filter(|(index, _)| {
                        shape_group_is_active(
                            syntax,
                            &class.conditional_fields,
                            *index,
                            active_shape_facts,
                        )
                    })
                    .flat_map(|(_, group)| &group.fields);
                for field in class
                    .fields
                    .iter()
                    .chain(conditional_fields)
                    .filter(|field| !field.is_static)
                {
                    let mut completion = simple_completion(
                        &field.name,
                        CompletionKind::Property,
                        if matches!(receiver, TypeKind::ManagedReference(_)) {
                            "fallible live managed field"
                        } else {
                            "managed snapshot field"
                        },
                    );
                    completion.documentation = field.documentation.clone();
                    builder.add(completion);
                }
                if matches!(receiver, TypeKind::ManagedReference(_)) {
                    builder.add(CompletionItem {
                        label: "snapshot".to_owned(),
                        kind: CompletionKind::Method,
                        detail: Some(format!("{}.Ref.snapshot() -> {}!", class.name, class.name)),
                        documentation: Some(
                            "Reads every active instance field transactionally and returns one immutable local snapshot. If any field read fails, no partial snapshot is exposed."
                                .to_owned(),
                        ),
                        documentation_uri: None,
                        insert_text: "snapshot()".to_owned(),
                        is_snippet: false,
                    });
                }
            }
        }
        TypeKind::Standard(owner) => {
            for field in standard_library.public_fields(*owner) {
                builder.add(CompletionItem {
                    label: field.name.to_owned(),
                    kind: CompletionKind::Property,
                    detail: Some(format!(
                        "{}.{}",
                        standard_library.type_decl(*owner).name,
                        field.name
                    )),
                    documentation: Some(render_documentation(&field.documentation)),
                    documentation_uri: Some(symbol_uri(
                        StdlibSymbolId::Field(field.id),
                        standard_library,
                    )),
                    insert_text: field.name.to_owned(),
                    is_snippet: false,
                });
            }
        }
        TypeKind::Array { .. } => {
            add_constructor_fields(builder, standard_library, StdlibTypeConstructorId::Array)
        }
        TypeKind::Option { .. } => {
            add_constructor_fields(builder, standard_library, StdlibTypeConstructorId::Option)
        }
        TypeKind::Result { .. } => {
            add_constructor_fields(builder, standard_library, StdlibTypeConstructorId::Result)
        }
        TypeKind::Range { kind, .. } => add_constructor_fields(
            builder,
            standard_library,
            match kind {
                crate::ast::RangeKind::Exclusive => StdlibTypeConstructorId::ExclusiveRange,
                crate::ast::RangeKind::Inclusive => StdlibTypeConstructorId::InclusiveRange,
            },
        ),
        TypeKind::Set { .. } => {
            add_constructor_fields(builder, standard_library, StdlibTypeConstructorId::Set)
        }
        TypeKind::Application { constructor, .. } => {
            add_constructor_fields(builder, standard_library, *constructor)
        }
        TypeKind::Builtin(_)
        | TypeKind::Enum(_)
        | TypeKind::GenericParameter { .. }
        | TypeKind::Async { .. }
        | TypeKind::Iterator { .. }
        | TypeKind::Callable { .. } => {}
    }
}

fn add_constructor_fields(
    builder: &mut CompletionBuilder,
    standard_library: &StandardLibrary,
    owner: StdlibTypeConstructorId,
) {
    for field in standard_library.public_constructor_fields(owner) {
        builder.add(CompletionItem {
            label: field.name.to_owned(),
            kind: CompletionKind::Property,
            detail: Some(format!(
                "{}.{}",
                standard_library.render_field_owner(field.owner),
                field.name
            )),
            documentation: Some(render_documentation(&field.documentation)),
            documentation_uri: Some(symbol_uri(
                StdlibSymbolId::Field(field.id),
                standard_library,
            )),
            insert_text: field.name.to_owned(),
            is_snippet: false,
        });
    }
}

fn active_attachment_shape_facts(
    syntax: &Program,
    tokens: &[&crate::lexer::Token],
    offset: usize,
) -> Vec<(CompletionShapeDimension, crate::ast::EnumVariantId)> {
    struct Finder<'a, 'tokens> {
        syntax: &'a Program,
        tokens: &'tokens [&'tokens crate::lexer::Token],
        offset: usize,
        facts: Vec<(CompletionShapeDimension, crate::ast::EnumVariantId)>,
    }

    impl<'ast> Visitor<'ast> for Finder<'ast, '_> {
        fn visit_stmt(&mut self, statement: &'ast Stmt) {
            if let Stmt::If {
                condition,
                then_block,
                else_block,
                ..
            } = statement
            {
                if contains_offset(then_block.span, self.offset) {
                    collect_attachment_shape_facts(self.syntax, condition, &mut self.facts);
                } else if else_block
                    .as_ref()
                    .is_some_and(|block| contains_offset(block.span, self.offset))
                {
                    collect_attachment_shape_falsy_facts(self.syntax, condition, &mut self.facts);
                }
            }
            visit::walk_stmt(self, statement);
        }

        fn visit_expr(&mut self, expression: &'ast Expr) {
            if let ExprKind::If {
                condition,
                then_expr,
                else_expr,
                ..
            } = &expression.kind
            {
                if contains_offset(then_expr.span, self.offset) {
                    collect_attachment_shape_facts(self.syntax, condition, &mut self.facts);
                } else if contains_offset(else_expr.span, self.offset) {
                    collect_attachment_shape_falsy_facts(self.syntax, condition, &mut self.facts);
                }
            }
            if let ExprKind::Match { value, arms } = &expression.kind
                && expression.span.start <= self.offset
                && cursor_is_inside_braces(self.tokens, expression.span.start, self.offset)
                && let Some(arm) = arms.iter().rev().find(|arm| arm.span.start <= self.offset)
                && let Some(fact) = pattern_shape_fact(self.syntax, value, &arm.pattern)
            {
                self.facts.push(fact);
            }
            visit::walk_expr(self, expression);
        }
    }

    let mut finder = Finder {
        syntax,
        tokens,
        offset,
        facts: Vec::new(),
    };
    finder.visit_program(syntax);
    if let Some(fact) = fallback_shape_match_fact(syntax, tokens, offset) {
        finder.facts.push(fact);
    }
    finder
        .facts
        .sort_by_key(|(dimension, variant)| (*dimension, variant.index()));
    finder.facts.dedup();
    finder.facts
}

fn shape_group_is_active<Field>(
    syntax: &Program,
    groups: &[crate::ast::ConditionalFieldsDecl<Field>],
    target: usize,
    active: &[(CompletionShapeDimension, crate::ast::EnumVariantId)],
) -> bool {
    let mut chain_start = target;
    while chain_start > 0 && groups[chain_start].else_span.is_some() {
        chain_start -= 1;
    }
    for group in &groups[chain_start..target] {
        let Some(condition) = &group.condition else {
            return false;
        };
        if shape_condition_value(syntax, active, condition) != Some(false) {
            return false;
        }
    }
    groups[target]
        .condition
        .as_ref()
        .is_none_or(|condition| shape_condition_value(syntax, active, condition) == Some(true))
}

fn shape_condition_value(
    syntax: &Program,
    active: &[(CompletionShapeDimension, crate::ast::EnumVariantId)],
    condition: &Expr,
) -> Option<bool> {
    match &condition.kind {
        ExprKind::Bool(value) => Some(*value),
        ExprKind::Unary {
            op: crate::ast::UnaryOp::Not,
            expr,
        } => shape_condition_value(syntax, active, expr).map(|value| !value),
        ExprKind::Binary {
            op: crate::ast::BinaryOp::And,
            left,
            right,
        } => match (
            shape_condition_value(syntax, active, left),
            shape_condition_value(syntax, active, right),
        ) {
            (Some(false), _) | (_, Some(false)) => Some(false),
            (Some(true), Some(true)) => Some(true),
            _ => None,
        },
        ExprKind::Binary {
            op: crate::ast::BinaryOp::Or,
            left,
            right,
        } => match (
            shape_condition_value(syntax, active, left),
            shape_condition_value(syntax, active, right),
        ) {
            (Some(true), _) | (_, Some(true)) => Some(true),
            (Some(false), Some(false)) => Some(false),
            _ => None,
        },
        ExprKind::Binary {
            op: crate::ast::BinaryOp::Eq | crate::ast::BinaryOp::Ne,
            left,
            right,
        } => {
            let fact = attachment_shape_fact(syntax, left, right)
                .or_else(|| attachment_shape_fact(syntax, right, left))?;
            let selected = active
                .iter()
                .find(|(field, _)| *field == fact.0)
                .map(|(_, variant)| *variant == fact.1)?;
            Some(
                if matches!(
                    condition.kind,
                    ExprKind::Binary {
                        op: crate::ast::BinaryOp::Ne,
                        ..
                    }
                ) {
                    !selected
                } else {
                    selected
                },
            )
        }
        ExprKind::Is { value, pattern, .. } => {
            let fact = pattern_shape_fact(syntax, value, &pattern.kind)?;
            active
                .iter()
                .find(|(dimension, _)| *dimension == fact.0)
                .map(|(_, variant)| *variant == fact.1)
        }
        _ => None,
    }
}

fn collect_attachment_shape_facts(
    syntax: &Program,
    expression: &Expr,
    output: &mut Vec<(CompletionShapeDimension, crate::ast::EnumVariantId)>,
) {
    match &expression.kind {
        ExprKind::Binary {
            op: crate::ast::BinaryOp::And,
            left,
            right,
        } => {
            collect_attachment_shape_facts(syntax, left, output);
            collect_attachment_shape_facts(syntax, right, output);
        }
        ExprKind::Binary {
            op: crate::ast::BinaryOp::Eq,
            left,
            right,
        } => {
            if let Some(fact) = attachment_shape_fact(syntax, left, right)
                .or_else(|| attachment_shape_fact(syntax, right, left))
            {
                output.push(fact);
            }
        }
        ExprKind::Is { value, pattern, .. } => {
            if let Some(fact) = pattern_shape_fact(syntax, value, &pattern.kind) {
                output.push(fact);
            }
        }
        _ => {}
    }
}

fn collect_attachment_shape_falsy_facts(
    syntax: &Program,
    expression: &Expr,
    output: &mut Vec<(CompletionShapeDimension, crate::ast::EnumVariantId)>,
) {
    if let ExprKind::Binary {
        op: crate::ast::BinaryOp::Or,
        left,
        right,
    } = &expression.kind
    {
        collect_attachment_shape_falsy_facts(syntax, left, output);
        collect_attachment_shape_falsy_facts(syntax, right, output);
    } else if let Some(fact) = inverse_attachment_shape_fact(syntax, expression) {
        output.push(fact);
    }
}

fn inverse_attachment_shape_fact(
    syntax: &Program,
    expression: &Expr,
) -> Option<(CompletionShapeDimension, crate::ast::EnumVariantId)> {
    if let ExprKind::Is { value, pattern, .. } = &expression.kind {
        return inverse_shape_fact(syntax, pattern_shape_fact(syntax, value, &pattern.kind)?);
    }
    let ExprKind::Binary {
        op: crate::ast::BinaryOp::Eq | crate::ast::BinaryOp::Ne,
        left,
        right,
    } = &expression.kind
    else {
        return None;
    };
    let selected = attachment_shape_fact(syntax, left, right)
        .or_else(|| attachment_shape_fact(syntax, right, left))?;
    if matches!(
        expression.kind,
        ExprKind::Binary {
            op: crate::ast::BinaryOp::Ne,
            ..
        }
    ) {
        return Some(selected);
    }
    inverse_shape_fact(syntax, selected)
}

fn inverse_shape_fact(
    syntax: &Program,
    selected: (CompletionShapeDimension, crate::ast::EnumVariantId),
) -> Option<(CompletionShapeDimension, crate::ast::EnumVariantId)> {
    let enumeration = syntax.enum_declarations().find(|enumeration| {
        enumeration
            .variants
            .iter()
            .any(|variant| variant.id == selected.1)
    })?;
    if enumeration.variants.len() != 2 {
        return None;
    }
    let inverse = enumeration
        .variants
        .iter()
        .find(|variant| variant.id != selected.1)?;
    Some((selected.0, inverse.id))
}

fn attachment_shape_fact(
    syntax: &Program,
    dimension: &Expr,
    variant: &Expr,
) -> Option<(CompletionShapeDimension, crate::ast::EnumVariantId)> {
    let dimension = completion_expression_path(dimension)?;
    let dimension = completion_shape_dimension(syntax, &dimension)?;
    let variant = completion_expression_path(variant)?;
    let [enum_name, variant_name] = variant.as_slice() else {
        return None;
    };
    let enumeration = syntax
        .enum_declarations()
        .find(|enumeration| enumeration.name == *enum_name)?;
    let variant = enumeration
        .variants
        .iter()
        .find(|variant| variant.name == *variant_name)?;
    Some((dimension, variant.id))
}

fn completion_shape_dimension(syntax: &Program, path: &[&str]) -> Option<CompletionShapeDimension> {
    Some(match path {
        [name] => {
            let global = syntax.globals.iter().find(|global| {
                global
                    .simple_binding()
                    .is_some_and(|binding| binding.name == *name)
            });
            if let Some(global) = global {
                CompletionShapeDimension::Value(global.id)
            } else {
                let field = syntax
                    .state
                    .as_ref()?
                    .all_fields()
                    .find(|field| field.name == *name)?;
                CompletionShapeDimension::Value(field.id)
            }
        }
        ["current", name] => {
            let field = syntax
                .state
                .as_ref()?
                .all_fields()
                .find(|field| field.name == *name)?;
            CompletionShapeDimension::Value(field.id)
        }
        _ => return None,
    })
}

fn pattern_shape_fact(
    syntax: &Program,
    value: &Expr,
    pattern: &MatchPattern,
) -> Option<(CompletionShapeDimension, crate::ast::EnumVariantId)> {
    let MatchPattern::Enum {
        enumeration,
        variant,
        payload: None,
    } = pattern
    else {
        return None;
    };
    let value_path = completion_expression_path(value)?;
    let dimension = completion_shape_dimension(syntax, &value_path)?;
    let enumeration = syntax
        .enum_declarations()
        .find(|candidate| candidate.name == enumeration.name)?;
    let variant = enumeration
        .variants
        .iter()
        .find(|candidate| candidate.name == *variant)?;
    Some((dimension, variant.id))
}

fn fallback_shape_match_fact(
    syntax: &Program,
    tokens: &[&crate::lexer::Token],
    offset: usize,
) -> Option<(CompletionShapeDimension, crate::ast::EnumVariantId)> {
    let (match_index, open_index) = tokens
        .iter()
        .enumerate()
        .take_while(|(_, token)| token.span.start < offset)
        .filter_map(|(index, token)| {
            matches!(&token.kind, TokenKind::Ident(name) if name == "match").then(|| {
                let open = tokens[index + 1..]
                    .iter()
                    .position(|candidate| matches!(candidate.kind, TokenKind::LBrace))?
                    + index
                    + 1;
                cursor_is_inside_braces(tokens, token.span.start, offset).then_some((index, open))
            })?
        })
        .last()?;
    let dimension_path = tokens[match_index + 1..open_index]
        .iter()
        .filter_map(|token| match &token.kind {
            TokenKind::Ident(name) => Some(name.as_str()),
            _ => None,
        })
        .collect::<Vec<_>>();
    let dimension = completion_shape_dimension(syntax, &dimension_path)?;
    let arrow_index = tokens[open_index + 1..]
        .iter()
        .enumerate()
        .take_while(|(_, token)| token.span.start < offset)
        .filter(|(_, token)| matches!(token.kind, TokenKind::FatArrow))
        .map(|(relative, _)| open_index + 1 + relative)
        .last()?;
    let pattern_start = tokens[open_index + 1..arrow_index]
        .iter()
        .rposition(|token| matches!(token.kind, TokenKind::Comma | TokenKind::LBrace))
        .map_or(open_index + 1, |relative| open_index + 2 + relative);
    let pattern_path = tokens[pattern_start..arrow_index]
        .iter()
        .filter_map(|token| match &token.kind {
            TokenKind::Ident(name) => Some(name.as_str()),
            _ => None,
        })
        .collect::<Vec<_>>();
    let [enum_name, variant_name] = pattern_path.as_slice() else {
        return None;
    };
    let enumeration = syntax
        .enum_declarations()
        .find(|enumeration| enumeration.name == *enum_name)?;
    let variant = enumeration
        .variants
        .iter()
        .find(|variant| variant.name == *variant_name)?;
    Some((dimension, variant.id))
}

fn completion_expression_path(expression: &Expr) -> Option<Vec<&str>> {
    match &expression.kind {
        ExprKind::Path(path) => Some(path.iter().map(String::as_str).collect()),
        ExprKind::Member { receiver, name, .. } => {
            let mut path = completion_expression_path(receiver)?;
            path.push(name);
            Some(path)
        }
        _ => None,
    }
}

fn cursor_is_inside_braces(tokens: &[&crate::lexer::Token], start: usize, offset: usize) -> bool {
    let search_start = tokens.partition_point(|token| token.span.start < start);
    let Some(open) = tokens[search_start..]
        .iter()
        .position(|token| matches!(token.kind, TokenKind::LBrace))
        .map(|relative| search_start + relative)
    else {
        return false;
    };
    let mut depth = 0_u32;
    for token in tokens[open..]
        .iter()
        .take_while(|token| token.span.start < offset)
    {
        match token.kind {
            TokenKind::LBrace => depth += 1,
            TokenKind::RBrace => {
                depth = depth.saturating_sub(1);
                if depth == 0 {
                    return false;
                }
            }
            _ => {}
        }
    }
    depth != 0
}

fn add_inferred_methods(
    builder: &mut CompletionBuilder,
    syntax: &Program,
    receiver: &TypeKind,
    generic_constraints: &[StdlibCapabilityId],
    standard_library: &StandardLibrary,
) {
    if matches!(receiver, TypeKind::Standard(StdlibTypeId::UnityGameObject)) {
        builder.add(CompletionItem {
            label: "component".to_owned(),
            kind: CompletionKind::Method,
            detail: Some("UnityGameObject.component<T>() -> T.Ref!".to_owned()),
            documentation: Some(
                "Finds the managed component whose runtime class matches the declared Unity schema class `T`."
                    .to_owned(),
            ),
            documentation_uri: Some(symbol_uri(
                StdlibSymbolId::Type(StdlibTypeId::UnityGameObject),
                standard_library,
            )),
            insert_text: "component<${1:Class}>()".to_owned(),
            is_snippet: true,
        });
    }
    let methods = standard_library
        .methods_for_type(receiver)
        .into_iter()
        .filter(|item| {
            !(matches!(
                item.id,
                StdlibItemId::ArrayPush
                    | StdlibItemId::ArrayExtend
                    | StdlibItemId::ArrayRemoveAt
                    | StdlibItemId::ArrayRemove
                    | StdlibItemId::ArrayPop
                    | StdlibItemId::ArrayClear
            ) && matches!(
                receiver,
                TypeKind::Array {
                    length: Some(_),
                    ..
                }
            ))
        })
        .chain(
            matches!(receiver, TypeKind::GenericParameter { .. })
                .then(|| {
                    standard_library
                        .methods()
                        .filter(|item| {
                            let receiver = match item.kind {
                                ItemKind::Method { receiver } => receiver,
                                ItemKind::Function => {
                                    return false;
                                }
                                ItemKind::Constant => return false,
                            };
                            let TypeRef::Parameter(parameter) = receiver else {
                                return false;
                            };
                            item.signature
                                .type_parameters
                                .iter()
                                .find(|candidate| candidate.name == parameter)
                                .is_some_and(|parameter| {
                                    parameter.constraints.iter().all(|constraint| {
                                        standard_library
                                            .capabilities_satisfy(generic_constraints, *constraint)
                                    })
                                })
                        })
                        .collect::<Vec<_>>()
                })
                .unwrap_or_default(),
        );
    for item in methods {
        let ItemKind::Method { .. } = item.kind else {
            unreachable!()
        };
        builder.add(stdlib_completion(
            item.name,
            item,
            CompletionKind::Method,
            standard_library,
        ));
    }
    for function in &syntax.functions {
        if function.method_of.is_some_and(|declared| {
            syntax_receiver_matches(syntax, declared, receiver, standard_library)
        }) {
            let parameters = function
                .params
                .iter()
                .enumerate()
                .map(|(index, parameter)| format!("${{{}:{}}}", index + 1, parameter.name))
                .collect::<Vec<_>>()
                .join(", ");
            builder.add(CompletionItem {
                label: function.name.clone(),
                kind: CompletionKind::Method,
                detail: Some("user method".to_owned()),
                documentation: None,
                documentation_uri: None,
                insert_text: format!("{}({parameters})", function.name),
                is_snippet: true,
            });
        }
    }
}

fn syntax_receiver_matches(
    syntax: &Program,
    declared: SyntaxTypeRef,
    receiver: &TypeKind,
    standard_library: &StandardLibrary,
) -> bool {
    match (declared, receiver) {
        (SyntaxTypeRef::Array(expected), TypeKind::Array { layout, .. }) => expected == *layout,
        (SyntaxTypeRef::Option(expected), TypeKind::Option { layout, .. }) => expected == *layout,
        (SyntaxTypeRef::Result(expected), TypeKind::Result { layout, .. }) => expected == *layout,
        (SyntaxTypeRef::Named(name), TypeKind::Standard(actual)) => standard_library
            .type_by_name(syntax.type_name(name))
            .is_some_and(|expected| expected.id == *actual),
        (SyntaxTypeRef::Named(name), TypeKind::Struct(actual)) => syntax
            .structs
            .iter()
            .any(|structure| structure.name == syntax.type_name(name) && structure.id == *actual),
        (SyntaxTypeRef::Named(name), TypeKind::Enum(actual)) => syntax
            .enums
            .iter()
            .any(|item| item.name == syntax.type_name(name) && item.id == *actual),
        (syntax, TypeKind::Builtin(actual)) => syntax.core_type() == Some(*actual),
        _ => false,
    }
}

#[derive(Debug, Clone)]
struct ReceiverFacts {
    ty: TypeKind,
    constraints: Vec<StdlibCapabilityId>,
    recovered: bool,
}

/// Owned facts only: retaining a repaired database would also retain its
/// complete augmented syntax and semantic products. The surrounding query
/// cache invalidates these entries whenever the source revision changes.
#[derive(Debug, Default)]
pub(crate) struct ReceiverCache {
    entries: std::collections::VecDeque<((usize, usize), Option<ReceiverFacts>)>,
    #[cfg(test)]
    probes: usize,
}

impl ReceiverCache {
    const CAPACITY: usize = 8;

    fn get(&mut self, key: (usize, usize)) -> Option<Option<ReceiverFacts>> {
        let position = self.entries.iter().position(|(stored, _)| *stored == key)?;
        let entry = self.entries.remove(position)?;
        let facts = entry.1.clone();
        self.entries.push_back(entry);
        Some(facts)
    }

    fn insert(&mut self, key: (usize, usize), facts: Option<ReceiverFacts>) {
        if self.entries.len() == Self::CAPACITY {
            self.entries.pop_front();
        }
        self.entries.push_back((key, facts));
    }
}

fn infer_receiver(
    database: &mut CompilerDatabase,
    source: &str,
    tokens: &[&crate::lexer::Token],
    context: &MemberContext,
    compiler_context: crate::CompilerContext,
) -> Option<ReceiverFacts> {
    // Within one revision the dot and replacement end determine the receiver
    // and repaired suffix, even while the caret moves inside the same member.
    let key = (context.dot, context.replacement.end);
    if let Some(cached) = database.completion_receiver_cache().get(key) {
        return cached;
    }
    let receiver_offset = context.receiver_offset;
    // Reuse the current revision before considering a repaired source. In LSP
    // use this database normally already contains the diagnostic pass, so a
    // second database used to repeat the complete semantic pipeline merely to
    // discover one receiver type.
    let direct = analyze_receiver_database(database, receiver_offset, context.dot);
    if direct.as_ref().is_some_and(|facts| !facts.recovered) {
        database
            .completion_receiver_cache()
            .insert(key, direct.clone());
        return direct;
    }

    let mut probe_source = String::with_capacity(source.len());
    probe_source.push_str(&source[..context.dot]);
    let suffix_start = completion_probe_suffix_end(tokens, context.replacement.end);
    probe_source.push_str(&source[suffix_start..]);
    #[cfg(test)]
    {
        database.completion_receiver_cache().probes += 1;
    }
    let facts =
        analyze_receiver_source(probe_source, receiver_offset, compiler_context, context.dot)
            .or(direct);
    database
        .completion_receiver_cache()
        .insert(key, facts.clone());
    facts
}

fn analyze_receiver_source(
    source: String,
    receiver_offset: usize,
    compiler_context: crate::CompilerContext,
    dot: usize,
) -> Option<ReceiverFacts> {
    let mut database = CompilerDatabase::with_context(compiler_context, source);
    analyze_receiver_database(&mut database, receiver_offset, dot)
}

fn analyze_receiver_database(
    database: &mut CompilerDatabase,
    receiver_offset: usize,
    dot: usize,
) -> Option<ReceiverFacts> {
    let analysis = database.analysis_at(receiver_offset).ok()??;
    let snapshot = database.semantic_snapshot().ok()?;
    let recovered = snapshot.checked().is_none();
    let receiver_type =
        completion_receiver_type(&analysis, snapshot.semantics(), receiver_offset, dot)?;
    let constraints = snapshot
        .semantics()
        .generic_parameter_constraints(receiver_type)
        .to_vec();
    let ty = snapshot.semantics().types().kind(receiver_type).clone();
    Some(ReceiverFacts {
        ty,
        constraints,
        recovered,
    })
}

fn completion_receiver_type(
    analysis: &crate::database::PositionAnalysis,
    semantics: &SemanticModel,
    receiver_offset: usize,
    dot: usize,
) -> Option<crate::types::TypeId> {
    // An expression ending before the dot is the receiver itself, including
    // calls, indexing, propagation, and fields on expression receivers.
    if analysis.span.end <= dot {
        return Some(analysis.ty);
    }
    // Plain dotted paths are a single expression. Select the typed prefix at
    // the completion dot instead of using the final field or call result.
    let segment = analysis.segments.iter().position(|segment| {
        segment.span.start <= receiver_offset && receiver_offset < segment.span.end
    })?;
    let (root, members, path_segments) = match analysis.resolution.as_ref()? {
        ExpressionResolution::ValuePath { root, members } => {
            ((*root)?, members.as_slice(), analysis.segments.len())
        }
        ExpressionResolution::Call(call) => {
            let path_segments = analysis.segments.len().checked_sub(1)?;
            if segment + 1 == path_segments {
                match call {
                    ResolvedCall::UserMethod { receiver_type, .. }
                    | ResolvedCall::ManagedSnapshot { receiver_type, .. }
                    | ResolvedCall::ManagedComponent { receiver_type, .. }
                    | ResolvedCall::StandardLibrary {
                        receiver_type: Some(receiver_type),
                        ..
                    } => {
                        return Some(*receiver_type);
                    }
                    _ => {}
                }
            }
            let (root, members) = call.receiver()?.path()?;
            (root, members, path_segments)
        }
        _ => return None,
    };
    let root_segments = path_segments.checked_sub(members.len())?;
    if segment == 0 {
        match root {
            ResolvedValue::CurrentState(_)
            | ResolvedValue::OldState(_)
            | ResolvedValue::CurrentSnapshot
            | ResolvedValue::OldSnapshot => {
                return Some(semantics.types().id_for_state_snapshot());
            }
            ResolvedValue::Setting(_)
            | ResolvedValue::OldSetting(_)
            | ResolvedValue::SettingsView
            | ResolvedValue::OldSettingsView => {
                return Some(semantics.types().id_for_settings_view());
            }
            _ => {}
        }
    }
    if segment + 1 == root_segments {
        return match root {
            ResolvedValue::ManagedStatic { field, .. } => semantics.managed_field_value_type(field),
            _ => semantics.value_type(root.source_value()?),
        };
    }
    match members.get(segment.checked_sub(root_segments)?)? {
        ResolvedMember::StateField(field) | ResolvedMember::SettingField(field) => {
            semantics.value_type(*field)
        }
        ResolvedMember::StructField(field) => semantics.struct_field_type(*field),
        // Managed members have distinct live and owned projections. Let the
        // repair path check the concrete receiver instead of guessing here.
        ResolvedMember::ManagedField(_) => None,
        // Constructor fields may depend on the concrete receiver's type
        // arguments. Keep the repair path until those substitutions are known.
        ResolvedMember::StandardField(_) => None,
    }
}

/// Removes an already-written call and propagation suffix from a completion
/// probe. When completion is manually requested on `receiver.method()`, the
/// identifier replacement alone would leave `receiver()` and infer the wrong
/// expression (or fail to type it entirely). The probe needs the type of the
/// expression before the member dot, regardless of whether the member text is
/// partial or already followed by its postfix syntax.
fn completion_probe_suffix_end(tokens: &[&crate::lexer::Token], identifier_end: usize) -> usize {
    let mut index = tokens.partition_point(|token| token.span.start < identifier_end);
    if index == tokens.len() {
        return identifier_end;
    }
    if !matches!(tokens[index].kind, TokenKind::LParen) {
        return identifier_end;
    }

    let mut depth = 0_u32;
    let mut end = identifier_end;
    while let Some(token) = tokens.get(index) {
        match token.kind {
            TokenKind::LParen => depth += 1,
            TokenKind::RParen => {
                depth -= 1;
                if depth == 0 {
                    end = token.span.end;
                    index += 1;
                    break;
                }
            }
            TokenKind::Eof => return identifier_end,
            _ => {}
        }
        index += 1;
    }
    if depth != 0 {
        return identifier_end;
    }
    if tokens
        .get(index)
        .is_some_and(|token| matches!(token.kind, TokenKind::Question))
    {
        end = tokens[index].span.end;
    }
    end
}

fn member_context(request: &CompletionRequest<'_>) -> Option<MemberContext> {
    let source = request.source;
    let offset = request.offset;
    let replacement = request.replacement;
    let dot_index = request
        .tokens
        .partition_point(|token| token.span.end <= replacement.start)
        .checked_sub(1)?;
    let dot_token = request.tokens[dot_index];
    if !matches!(dot_token.kind, TokenKind::Dot) {
        return None;
    }
    let receiver = request.tokens.get(dot_index.checked_sub(1)?)?;
    if matches!(receiver.kind, TokenKind::Eof) || receiver.span.start == receiver.span.end {
        return None;
    }

    let mut receiver_path = Vec::new();
    let mut index = dot_index;
    while let Some(identifier_index) = index.checked_sub(1) {
        let TokenKind::Ident(name) = &request.tokens[identifier_index].kind else {
            break;
        };
        receiver_path.push(name.clone());
        let Some(previous_dot) = identifier_index.checked_sub(1) else {
            break;
        };
        if !matches!(request.tokens[previous_dot].kind, TokenKind::Dot) {
            break;
        }
        index = previous_dot;
    }
    receiver_path.reverse();
    Some(MemberContext {
        receiver_path,
        receiver_offset: receiver.span.end.checked_sub(1)?,
        dot: dot_token.span.start,
        prefix: source[replacement.start..offset].to_owned(),
        replacement,
    })
}

fn identifier_span(source: &str, offset: usize) -> Span {
    let mut start = offset;
    while start > 0 && is_identifier_byte(source.as_bytes()[start - 1]) {
        start -= 1;
    }
    let mut end = offset;
    while end < source.len() && is_identifier_byte(source.as_bytes()[end]) {
        end += 1;
    }
    Span { start, end }
}

fn is_identifier_byte(byte: u8) -> bool {
    splitscript_syntax::is_identifier_continue_byte(byte)
}

fn floor_char_boundary(source: &str, mut offset: usize) -> usize {
    while !source.is_char_boundary(offset) {
        offset -= 1;
    }
    offset
}

fn simple_completion(label: &str, kind: CompletionKind, detail: &str) -> CompletionItem {
    CompletionItem {
        label: label.to_owned(),
        kind,
        detail: Some(detail.to_owned()),
        documentation: None,
        documentation_uri: None,
        insert_text: label.to_owned(),
        is_snippet: false,
    }
}

fn render_documentation<Id>(documentation: &Documentation<Id>) -> String {
    crate::documentation::strip_intra_doc_links(&crate::documentation::prose_markdown(
        documentation.summary,
        documentation.details,
    ))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn labels(database: &mut CompilerDatabase, needle: &str) -> Vec<String> {
        let offset = database
            .source()
            .find(needle)
            .expect("completion marker exists")
            + needle.len();
        database
            .completions(offset)
            .expect("completion should succeed")
            .items
            .into_iter()
            .map(|item| item.label)
            .collect()
    }

    #[test]
    fn valid_field_receivers_need_no_probe_and_invalidate_on_edits() {
        let source = r#"
struct Point { coordinate: i32 }
state "game.exe" {}
whileAttached {
    let point = Point { coordinate: 1 }
    print(point.coordinate)
}
"#;
        let mut database = CompilerDatabase::new(source);
        let first = labels(&mut database, "point.");
        assert!(first.contains(&"coordinate".to_owned()));
        assert_eq!(database.completion_receiver_cache().probes, 0);
        assert_eq!(first, labels(&mut database, "point."));
        assert_eq!(labels(&mut database, "point.coo"), vec!["coordinate"]);
        assert_eq!(database.completion_receiver_cache().probes, 0);

        database.set_source(
            r#"
state "game.exe" {}
whileAttached {
    let point = "text"
    print(point.byteLength())
}
"#,
        );
        assert!(database.completion_receiver_cache().entries.is_empty());
        let changed = labels(&mut database, "point.");
        assert!(changed.contains(&"byteLength".to_owned()));
        assert!(!changed.contains(&"coordinate".to_owned()));
        assert_eq!(database.completion_receiver_cache().probes, 0);
    }

    #[test]
    fn direct_receivers_select_path_prefixes_and_call_results() {
        let declarations = r#"
struct Leaf { count: i32 }
struct Holder { leaf: Leaf }
fn Leaf.label() { return "text" }
fn make() { return Holder { leaf: Leaf { count: 1 } } }
state "game.exe" { position: Holder = make() }
"#;
        for (expression, marker, expected, absent) in [
            (
                "current.position.leaf.count",
                "current.",
                "position",
                "count",
            ),
            ("old.position.leaf.count", "old.", "position", "count"),
            ("holder.leaf.count", "holder.", "leaf", "count"),
            ("holder.leaf.count", "holder.leaf.", "count", "leaf"),
            (
                "current.position.leaf.count",
                "current.position.",
                "leaf",
                "count",
            ),
            (
                "current.position.leaf.count",
                "current.position.leaf.",
                "count",
                "leaf",
            ),
            ("holder.leaf.label()", "holder.", "leaf", "label"),
            ("holder.leaf.label()", "holder.leaf.", "label", "byteLength"),
            (
                "holder.leaf.label().byteLength()",
                "holder.leaf.label().",
                "byteLength",
                "count",
            ),
            ("make().leaf.count", "make().", "leaf", "count"),
            ("make().leaf.count", "make().leaf.", "count", "leaf"),
            ("make().leaf.label()", "make().leaf.", "label", "byteLength"),
            ("[1i32][0].min(2)", "[1i32][0].", "min", "length"),
        ] {
            let source = format!(
                "{declarations}\nwhileAttached {{\nlet holder = make()\nprint({expression})\n}}"
            );
            let mut database = CompilerDatabase::new(source);
            assert!(
                database.semantic_snapshot().unwrap().checked().is_some(),
                "{expression}"
            );
            let candidates = labels(&mut database, marker);
            assert!(
                candidates.contains(&expected.to_owned()),
                "{marker}: {candidates:?}"
            );
            assert!(
                !candidates.contains(&absent.to_owned()),
                "{marker}: {candidates:?}"
            );
            assert_eq!(database.completion_receiver_cache().probes, 0, "{marker}");
        }
    }

    #[test]
    fn recovered_receiver_probes_are_still_reused() {
        let source = r#"
struct Point { coordinate: i32 }
state "game.exe" {}
whileAttached {
    let point = Point { coordinate: 1 }
    point.coo
}
"#;
        let mut database = CompilerDatabase::new(source);
        assert_eq!(labels(&mut database, "point.coo"), vec!["coordinate"]);
        assert_eq!(database.completion_receiver_cache().probes, 1);
        assert_eq!(labels(&mut database, "point.co"), vec!["coordinate"]);
        assert_eq!(database.completion_receiver_cache().probes, 1);
    }

    #[test]
    fn direct_receivers_preserve_propagation_and_generic_constraints() {
        for (source, marker) in [
            (
                "fn smaller(value: i32?) -> i32 { return (value else 0).min(3) }",
                "(value else 0).",
            ),
            (
                "fn smaller(value: i32!) -> i32! { return value?.min(3) }",
                "value?.",
            ),
            (
                "fn smaller(value, other) { return value.min(other) }",
                "value.",
            ),
        ] {
            let mut database = CompilerDatabase::new(format!("state \"game.exe\" {{}}\n{source}"));
            assert!(
                database.semantic_snapshot().unwrap().checked().is_some(),
                "{source}"
            );
            let candidates = labels(&mut database, marker);
            assert!(
                candidates.contains(&"min".to_owned()),
                "{source}: {candidates:?}"
            );
            assert!(
                !candidates.contains(&"length".to_owned()),
                "{source}: {candidates:?}"
            );
            assert_eq!(database.completion_receiver_cache().probes, 0, "{source}");
        }
    }

    #[test]
    fn unresolved_member_receiver_probes_cache_failures() {
        let mut database =
            CompilerDatabase::new("state \"game.exe\" {}\nwhileAttached { missing.unknown }");
        let first = labels(&mut database, "missing.");
        assert_eq!(database.completion_receiver_cache().probes, 1);
        assert_eq!(first, labels(&mut database, "missing."));
        assert_eq!(database.completion_receiver_cache().probes, 1);
        assert!(database.completion_receiver_cache().entries[0].1.is_none());
    }

    const EFFECT_COMPLETION_DECLARATIONS: &str = r#"
state "game.exe" { level: u32 at 0x100 }
fn readsProcess() { let value = process.read<u32>(0x100) else return }
fn readsState() { return current.level }
fn relay() { readsProcess() }
fn safe() { print("safe") }
"#;

    #[test]
    fn root_effect_facts_are_shared_without_losing_context_filtering() {
        let mut database = CompilerDatabase::new(format!(
            "{EFFECT_COMPLETION_DECLARATIONS}\nsetup {{ safe() }}\nonAttach {{ safe() }}\nonDetach {{ safe() }}"
        ));
        for marker in ["setup { ", "onAttach { ", "onDetach { "] {
            let candidates = labels(&mut database, marker);
            assert!(candidates.contains(&"safe".to_owned()));
            assert!(!candidates.contains(&"readsState".to_owned()));
            for name in ["readsProcess", "relay"] {
                assert_eq!(
                    candidates.contains(&name.to_owned()),
                    marker == "onAttach { "
                );
            }
        }
        let cache = database.completion_effects_cache();
        assert_eq!(cache.analyses, 1);
        assert_eq!(cache.probes, 0);
        assert!(cache.direct.is_some());
    }

    #[test]
    fn partial_root_effect_probes_are_reused_and_invalidated_by_edits() {
        let source = format!("{EFFECT_COMPLETION_DECLARATIONS}\nonDetach {{ sa }}");
        let mut database = CompilerDatabase::new(source.clone());
        let candidates = labels(&mut database, "onDetach { ");
        assert!(candidates.contains(&"safe".to_owned()));
        for absent in ["readsProcess", "relay", "readsState"] {
            assert!(!candidates.contains(&absent.to_owned()));
        }
        assert_eq!(labels(&mut database, "onDetach { sa"), vec!["safe"]);
        assert_eq!(database.completion_effects_cache().probes, 1);
        database.set_source(source.replace("print(\"safe\")", "readsProcess()"));
        assert!(database.completion_effects_cache().entries.is_empty());
        assert!(labels(&mut database, "onDetach { sa").is_empty());
        assert_eq!(database.completion_effects_cache().probes, 1);
    }

    #[test]
    fn failed_root_effect_probes_are_cached_separately_and_bounded() {
        let source =
            format!("{EFFECT_COMPLETION_DECLARATIONS}\nonDetach {{\nsa\nsa\nsa\nsa\nsa\n}}");
        let offsets = source
            .match_indices("sa\n")
            .map(|(offset, _)| offset + 2)
            .collect::<Vec<_>>();
        let mut database = CompilerDatabase::new(source);
        for &offset in &offsets {
            assert!(database.completions(offset).unwrap().items.is_empty());
            assert!(database.completions(offset).unwrap().items.is_empty());
        }
        let cache = database.completion_effects_cache();
        assert_eq!(cache.probes, offsets.len());
        assert_eq!(cache.entries.len(), CompletionEffectsCache::CAPACITY);
        assert!(cache.entries.iter().all(|(_, facts)| facts.is_none()));
        database.completions(offsets[0]).unwrap();
        assert_eq!(
            database.completion_effects_cache().probes,
            offsets.len() + 1
        );
    }

    #[test]
    fn root_completion_skips_unused_effect_analysis() {
        for (source, marker) in [
            (format!("{EFFECT_COMPLETION_DECLARATIONS}\nwhi"), "\nwhi"),
            (
                "state \"game.exe\" {}\nonDetach { pri }".to_owned(),
                "{ pri",
            ),
            (
                format!("{EFFECT_COMPLETION_DECLARATIONS}\nwhileAttached {{ sa }}"),
                "{ sa",
            ),
        ] {
            let mut database = CompilerDatabase::new(source);
            assert!(!labels(&mut database, marker).is_empty());
            assert_eq!(database.completion_effects_cache().analyses, 0);
            assert_eq!(database.completion_effects_cache().probes, 0);
        }
    }

    #[test]
    fn completes_only_missing_tick_rate_fields() {
        let mut empty = CompilerDatabase::new("state \"game.exe\" {}\ntickRate {\n    \n}");
        let completions = labels(&mut empty, "tickRate {");
        assert!(completions.contains(&"attached".to_owned()));
        assert!(completions.contains(&"detached".to_owned()));

        let mut partial = CompilerDatabase::new(
            "state \"game.exe\" {}\ntickRate {\n    attached: 60,\n    det\n}",
        );
        let completion = partial
            .completions(partial.source().find("det\n").unwrap() + 3)
            .expect("tick-rate completion should succeed");
        assert_eq!(completion.items.len(), 1, "{completion:#?}");
        assert_eq!(completion.items[0].label, "detached");
        assert_eq!(completion.items[0].insert_text, "detached: ${1:1},");
        assert!(completion.items[0].is_snippet);
    }

    #[test]
    fn completes_bounded_managed_string_policy_after_a_field_name() {
        for declaration in ["String scene ma", "String? subtitle from \"Caption\" ma"] {
            let source = format!(
                "image \"Assembly-CSharp\" {{\n    class Game {{\n        {declaration}\n    }}\n}}\nstate Unity [\"game.exe\"] {{}}"
            );
            let mut database = CompilerDatabase::new(source);
            let completion = database
                .completions(database.source().find("ma\n").unwrap() + 2)
                .expect("managed-field completion should recover incomplete syntax");
            assert_eq!(completion.items.len(), 1, "{completion:#?}");
            assert_eq!(completion.items[0].label, "maxLength");
            assert_eq!(completion.items[0].insert_text, "maxLength ${1:64};");
        }
    }

    #[test]
    fn completes_only_compatible_declared_setting_keys_inside_lookup_strings() {
        let source = r#"state "game.exe" {}
enum Mode { Fast, Slow }
settings {
    /// Splits at the boss.
    "Boss" => boss key "split-boss": true,
    "Mode" => mode key "run-mode": choice {
        "Fast" => Mode.Fast default,
        "Slow" => Mode.Slow,
    },
    for level in 2..=3 { `Level {level}` key `level-{level}`: true },
}
whileAttached {
    let boss = settings.enabled("split")
    let known = oldSettings.contains("")
}
"#;
        let mut enabled = CompilerDatabase::new(source);
        let enabled_offset =
            source.find("settings.enabled(\"split").unwrap() + "settings.enabled(\"split".len();
        let completion = enabled.completions(enabled_offset).unwrap();
        assert_eq!(completion.items.len(), 1, "{completion:#?}");
        assert_eq!(completion.items[0].label, "split-boss");
        assert_eq!(completion.items[0].insert_text, "split-boss");
        assert_eq!(
            &source[completion.replacement.start..completion.replacement.end],
            "split"
        );
        assert!(
            completion.items[0]
                .detail
                .as_deref()
                .unwrap()
                .contains("settings.boss")
        );
        assert_eq!(
            completion.items[0].documentation.as_deref(),
            Some("Splits at the boss.")
        );

        let mut contains = CompilerDatabase::new(source);
        let contains_offset =
            source.find("oldSettings.contains(\"\"").unwrap() + "oldSettings.contains(\"".len();
        let labels = contains
            .completions(contains_offset)
            .unwrap()
            .items
            .into_iter()
            .map(|item| item.label)
            .collect::<Vec<_>>();
        for expected in ["split-boss", "run-mode", "level-2", "level-3"] {
            assert!(
                labels.contains(&expected.to_owned()),
                "missing {expected}: {labels:#?}"
            );
        }
    }

    #[test]
    fn tick_rate_field_completion_is_not_offered_in_values_or_other_blocks() {
        let mut value =
            CompilerDatabase::new("state \"game.exe\" {}\ntickRate {\n    attached: det\n}");
        assert!(
            labels(&mut value, "attached: det")
                .iter()
                .all(|label| label != "detached")
        );

        let mut action =
            CompilerDatabase::new("state \"game.exe\" {}\nwhileAttached {\n    att\n}");
        assert!(
            labels(&mut action, "    att")
                .iter()
                .all(|label| label != "attached")
        );
    }

    #[test]
    fn settings_completion_offers_contextual_entry_and_kind_snippets() {
        let mut entries = CompilerDatabase::new("state \"game.exe\" {}\nsettings {\n    \n}");
        let labels = labels(&mut entries, "settings {");
        for expected in [
            "boolean setting",
            "text input setting",
            "settings group",
            "choice setting",
            "file setting",
            "for setting family",
        ] {
            assert!(
                labels.contains(&expected.to_owned()),
                "missing {expected}: {labels:#?}"
            );
        }

        let source = "state \"game.exe\" {}\nsettings {\n    \"Mode\" => mode: ch\n}";
        let mut kinds = CompilerDatabase::new(source);
        let completion = kinds
            .completions(source.find("ch\n").unwrap() + 2)
            .expect("setting-kind completion should succeed");
        assert_eq!(completion.items.len(), 1);
        assert_eq!(completion.items[0].label, "choice");
        assert!(completion.items[0].is_snippet);
        assert!(completion.items[0].insert_text.starts_with("choice {"));
    }

    #[test]
    fn settings_completion_understands_groups_choices_and_file_filters() {
        let mut group = CompilerDatabase::new(
            "state \"game.exe\" {}\nsettings {\n    \"General\" {\n        \n    },\n}",
        );
        assert!(labels(&mut group, "\"General\" {").contains(&"boolean setting".to_owned()));

        let mut choice = CompilerDatabase::new(
            "enum Mode { A }\nstate \"game.exe\" {}\nsettings {\n    \"Mode\" => mode: choice {\n        \n    },\n}",
        );
        assert_eq!(labels(&mut choice, "choice {"), vec!["choice option"]);

        let mut file = CompilerDatabase::new(
            "state \"game.exe\" {}\nsettings {\n    \"Input\" => input: file {\n        \n    },\n}",
        );
        let labels = labels(&mut file, "file {");
        assert!(labels.contains(&"named filter".to_owned()));
        assert!(labels.contains(&"fallback filter".to_owned()));
        assert!(labels.contains(&"MIME filter".to_owned()));
    }

    #[test]
    fn choice_setting_values_complete_source_enums_and_payloadless_variants() {
        let declarations = r#"
enum Mode { Fast, Slow, Custom(u8) }
enum Other { First }
enum PayloadOnly { Value(u8) }
state "game.exe" {}
"#;

        let source = format!(
            "{declarations}settings {{\n    \"Mode\" => mode: choice {{\n        \"Fast\" => Mo\n    }},\n}}"
        );
        let mut enum_type = CompilerDatabase::new(source);
        assert_eq!(labels(&mut enum_type, "=> Mo"), vec!["Mode"]);

        let source = format!(
            "{declarations}settings {{\n    \"Mode\" => mode: choice {{\n        \"Slow\" => Mode.S\n    }},\n}}"
        );
        let mut variant = CompilerDatabase::new(source);
        assert_eq!(labels(&mut variant, "Mode.S"), vec!["Slow"]);

        let source = format!(
            "{declarations}settings {{\n    \"Mode\" => mode: choice {{\n        \"Fast\" => Mode.Fast,\n        \"Slow\" => \n    }},\n}}"
        );
        let mut constrained = CompilerDatabase::new(source);
        assert_eq!(labels(&mut constrained, "\"Slow\" => "), vec!["Mode"]);

        let source = format!(
            "{declarations}settings {{\n    \"Mode\" => mode: choice {{\n        \"Fast\" => Mode.\n    }},\n}}"
        );
        let mut payloadless = CompilerDatabase::new(source);
        assert_eq!(labels(&mut payloadless, "Mode."), vec!["Fast", "Slow"]);
    }

    #[test]
    fn state_completion_offers_fields_types_and_sources_contextually() {
        let mut empty = CompilerDatabase::new("state \"game.exe\" {\n    \n}");
        let candidates = labels(&mut empty, "state \"game.exe\" {");
        for expected in [
            "expression field",
            "memory field",
            "inferred memory field",
            "module pointer field",
            "UTF-8 string field",
            "UTF-16LE string field",
        ] {
            assert!(
                candidates.contains(&expected.to_owned()),
                "missing {expected}: {candidates:#?}"
            );
        }

        let source = "struct Position { x: f32, }\nstate \"game.exe\" {\n    position: Pos\n}";
        let mut typed = CompilerDatabase::new(source);
        let candidates = labels(&mut typed, "position: Pos");
        assert!(candidates.contains(&"Position".to_owned()));
        assert!(!candidates.contains(&"print".to_owned()));

        let source = "state \"game.exe\" {\n    position: i32 a\n}";
        let mut source_kind = CompilerDatabase::new(source);
        let candidates = labels(&mut source_kind, "i32 a");
        assert_eq!(candidates, vec!["at"]);
    }

    #[test]
    fn shape_refinement_completes_conditional_state_and_managed_fields() {
        let source = r#"
enum Edition { Base, Demo }
let edition: Edition
image "Assembly-CSharp" {
    class GameManager {
        static GameManager instance;
        if edition == Edition.Base { u32 level; }
    }
}
state Unity ["game.exe"] {
    if edition == Edition.Base { scene: u8 at 0x100; }
}
onAttach { edition = Edition.Base }
split {
    let manager = GameManager.instance else return false
    if edition == Edition.Base {
        current.
        manager.
    }
    return false
}
"#;
        let mut state = CompilerDatabase::new(source);
        assert!(labels(&mut state, "current.").contains(&"scene".to_owned()));
        let managed_source = source.replace("        current.\n", "        current.scene\n");
        let mut managed = CompilerDatabase::new(&managed_source);
        let managed = labels(&mut managed, "manager.");
        assert!(managed.contains(&"level".to_owned()), "{managed:#?}");
    }

    #[test]
    fn every_type_grammar_position_uses_the_shared_type_catalog() {
        let declarations = r#"
struct Position {
    x: i32,
}
enum Mode {
    Fast,
}
"#;
        let cases = [
            (
                format!("{declarations}\nfn inspect(value: ) {{}}\nstate \"game.exe\" {{}}"),
                "value: ",
            ),
            (
                format!("{declarations}\nfn inspect() ->  {{}}\nstate \"game.exe\" {{}}"),
                "-> ",
            ),
            (
                format!("{declarations}\nlet globalValue:  = None\nstate \"game.exe\" {{}}"),
                "globalValue: ",
            ),
            (
                format!(
                    "{declarations}\nfn inspect() {{ let localValue:  = None }}\nstate \"game.exe\" {{}}"
                ),
                "localValue: ",
            ),
            (
                format!(
                    "{declarations}\nfn inspect(value) {{ let cast = value as  }}\nstate \"game.exe\" {{}}"
                ),
                "value as ",
            ),
            (
                format!("{declarations}\nstate GBA {{ watched:  at 0x100; }}"),
                "watched: ",
            ),
            (
                format!("{declarations}\nstruct Holder {{ field: , }}\nstate \"game.exe\" {{}}"),
                "field: ",
            ),
            (
                format!("{declarations}\nenum Wrapped {{ Value(), }}\nstate \"game.exe\" {{}}"),
                "Value(",
            ),
            (
                format!("{declarations}\nlet genericValue: Set< = None\nstate \"game.exe\" {{}}"),
                "Set<",
            ),
            (
                format!("{declarations}\nlet arrayValue: [ = None\nstate \"game.exe\" {{}}"),
                "arrayValue: [",
            ),
        ];

        for (source, marker) in cases {
            let mut database = CompilerDatabase::new(source);
            let offset = database.source().find(marker).unwrap() + marker.len();
            let completions = database.completions(offset).unwrap();
            let labels = completions
                .items
                .iter()
                .map(|item| item.label.as_str())
                .collect::<Vec<_>>();
            for expected in [
                "i32", "String", "Position", "Mode", "Set", "[T]", "[T; N]", "T?", "T!", "async T",
            ] {
                assert!(
                    labels.contains(&expected),
                    "missing `{expected}` at `{marker}`: {labels:#?}"
                );
            }
            for value_candidate in ["print", "process", "current", "return"] {
                assert!(
                    !labels.contains(&value_candidate),
                    "value candidate `{value_candidate}` leaked into `{marker}`: {labels:#?}"
                );
            }
            assert!(completions.items.iter().all(|item| matches!(
                item.kind,
                CompletionKind::Type | CompletionKind::Struct | CompletionKind::Enum
            )));
            let set = completions
                .items
                .iter()
                .find(|item| item.label == "Set")
                .unwrap();
            assert_eq!(set.insert_text, "Set<${1:T}>");
            assert!(set.is_snippet);
            assert!(set.documentation.is_some());
        }
    }

    #[test]
    fn private_standard_library_types_are_not_completed() {
        let mut database = CompilerDatabase::new("let value:  = None\nstate \"game.exe\" {}");
        let candidates = labels(&mut database, "value: ");
        for private_type in [
            "MonoLayout",
            "MonoModule",
            "MonoImage",
            "MonoClass",
            "UnityModule",
            "UnityImage",
            "UnityClass",
            "UnityField",
        ] {
            assert!(!candidates.contains(&private_type.to_owned()));
        }
    }

    #[test]
    fn enum_representation_completion_only_offers_fixed_width_integers() {
        let source = "enum GameState:  { Mission }\nstate \"game.exe\" {}";
        let mut database = CompilerDatabase::new(source);
        let candidates = labels(&mut database, "GameState: ");
        for integer in ["i8", "u8", "i16", "u16", "i32", "u32", "i64", "u64"] {
            assert!(
                candidates.contains(&integer.to_owned()),
                "missing {integer}: {candidates:#?}"
            );
        }
        for invalid in ["bool", "f32", "address", "String", "GameState"] {
            assert!(
                !candidates.contains(&invalid.to_owned()),
                "unexpected {invalid}: {candidates:#?}"
            );
        }
    }

    #[test]
    fn value_colons_are_not_mistaken_for_type_annotations() {
        let source = r#"
struct Position {
    x: i32,
}
state "game.exe" {}
fn inspect() {
    let position = Position { x: pri }
}
"#;
        let mut database = CompilerDatabase::new(source);
        let completions = labels(&mut database, "x: pri");
        assert!(
            completions.contains(&"print".to_owned()),
            "{completions:#?}"
        );
    }

    #[test]
    fn state_completion_respects_specialized_provider_grammars() {
        let mut gba = CompilerDatabase::new("state GBA {\n    \n}");
        let candidates = labels(&mut gba, "state GBA {");
        assert!(candidates.contains(&"memory field".to_owned()));
        assert!(!candidates.contains(&"module pointer field".to_owned()));
        assert!(!candidates.contains(&"UTF-8 string field".to_owned()));
    }

    #[test]
    fn state_completion_finishes_pointer_paths_without_hiding_expressions() {
        let source = "state \"game.exe\" {\n    value: i32 at 0x1000 a\n}";
        let mut pointer = CompilerDatabase::new(source);
        assert_eq!(labels(&mut pointer, "0x1000 a"), vec!["as"]);

        let source = "state GBA {\n    value: i32 at 0x1000 \n}";
        let mut gba = CompilerDatabase::new(source);
        assert_eq!(labels(&mut gba, "0x1000 "), vec!["if"]);

        let source = "state \"game.exe\" { value = process.rea }";
        let mut expression = CompilerDatabase::new(source);
        assert!(labels(&mut expression, "process.rea").contains(&"read".to_owned()));
    }

    #[test]
    fn top_level_completion_contains_only_declarations_and_lifecycle_blocks() {
        let source = "state \"game.exe\" {}\n\n";
        let mut database = CompilerDatabase::new(source);
        let candidates = labels(&mut database, source);
        for expected in [
            "fn",
            "struct",
            "enum",
            "let",
            "settings",
            "tickRate",
            "setup",
            "selectProcess",
            "onAttach",
            "whileAttached",
            "split",
        ] {
            assert!(
                candidates.contains(&expected.to_owned()),
                "missing {expected}: {candidates:#?}"
            );
        }
        for unavailable in [
            "state", "print", "process", "timer", "i32", "if", "return", "current",
        ] {
            assert!(
                !candidates.contains(&unavailable.to_owned()),
                "unexpected {unavailable}: {candidates:#?}"
            );
        }

        let source = "state \"game.exe\" {}\nsettings {}\ntickRate {}\nsplit {}\n\n";
        let mut declared = CompilerDatabase::new(source);
        let candidates = labels(&mut declared, source);
        for duplicate in ["state", "settings", "tickRate", "split"] {
            assert!(!candidates.contains(&duplicate.to_owned()));
        }
    }

    #[test]
    fn state_header_completion_guides_processes_lists_and_catalog_providers() {
        let mut target = CompilerDatabase::new("state ");
        let candidates = labels(&mut target, "state ");
        assert!(candidates.contains(&"\"game.exe\"".to_owned()));
        assert!(candidates.contains(&"[\"game.exe\", ...]".to_owned()));
        assert!(candidates.contains(&"GBA".to_owned()));
        assert!(!candidates.contains(&"Process".to_owned()));

        let mut provider = CompilerDatabase::new("state G");
        assert_eq!(
            labels(&mut provider, "state G"),
            vec!["GBA", "GCN", "Genesis"]
        );

        let mut provider_body = CompilerDatabase::new("state GBA ");
        assert_eq!(labels(&mut provider_body, "state GBA "), vec!["{"]);

        let mut process_body = CompilerDatabase::new("state [\"game.exe\", \"demo.exe\"] ");
        assert_eq!(
            labels(&mut process_body, "state [\"game.exe\", \"demo.exe\"] "),
            vec!["{"]
        );
    }

    #[test]
    fn completes_domains_catalogs_and_inferred_members() {
        let source = r#"
struct Position {
    x: f32
}

enum Mode {
    Idle,
    Active(i32)
}

fn Position.coordinate() {
    return self.x
}

fn moduleAddress(module: Module) {
    module.ad
}

state "game.exe" {
    position: Position = process.read(0)
}

settings {
    "General" {
        "Enabled" => enabled: true
    }
}

whileAttached {
    let number: i32 = 4
    let snapshot = current
    let capturedSettings = settings
    current.po
    snapshot.po
    capturedSettings.en
    settings.en
    Mode.Ac
    process.re
    process.na
    process.main
    process.cl
    number.cl
    current.position.co
    current.position.x
}
"#;
        let mut database = CompilerDatabase::new(source);
        assert!(labels(&mut database, "current.po").contains(&"position".to_owned()));
        assert!(labels(&mut database, "snapshot.po").contains(&"position".to_owned()));
        assert!(labels(&mut database, "capturedSettings.en").contains(&"enabled".to_owned()));
        assert!(labels(&mut database, "settings.en").contains(&"enabled".to_owned()));
        assert!(labels(&mut database, "Mode.Ac").contains(&"Active".to_owned()));
        assert!(labels(&mut database, "process.re").contains(&"read".to_owned()));
        assert!(labels(&mut database, "process.na").contains(&"name".to_owned()));
        assert!(labels(&mut database, "process.main").contains(&"mainModule".to_owned()));
        assert!(labels(&mut database, "process.cl").contains(&"closed".to_owned()));
        assert!(labels(&mut database, "number.cl").contains(&"clamp".to_owned()));
        assert!(labels(&mut database, "module.ad").contains(&"address".to_owned()));
        assert!(labels(&mut database, "current.position.co").contains(&"coordinate".to_owned()));
        assert!(labels(&mut database, "current.position.x").contains(&"x".to_owned()));

        let mut bare = CompilerDatabase::new(
            "state \"game.exe\" {}\nwhileAttached { let number: i32 = 4\nnumber.\n}",
        );
        assert!(labels(&mut bare, "number.").contains(&"min".to_owned()));

        let mut float_type =
            CompilerDatabase::new("state \"game.exe\" {}\nsetup { let value = f32. }");
        let completions = float_type
            .completions(float_type.source().find("f32.").unwrap() + "f32.".len())
            .expect("float associated items should complete");
        for name in ["NaN", "positiveInfinity", "negativeInfinity"] {
            let item = completions
                .items
                .iter()
                .find(|item| item.label == name)
                .unwrap_or_else(|| panic!("missing `{name}`: {completions:#?}"));
            assert_eq!(item.kind, CompletionKind::Constant);
            assert_eq!(item.insert_text, name);
            assert!(!item.is_snippet);
        }

        let mut keyed_settings = CompilerDatabase::new(
            "state \"game.exe\" {}\nsettings { \"Flag\" => flag key \"flag\": true }\nwhileAttached { settings.en }",
        );
        assert!(labels(&mut keyed_settings, "settings.en").contains(&"enabled".to_owned()));

        let mut generated_settings = CompilerDatabase::new(
            "state \"game.exe\" {}\nsettings { for level in 2..=4 { `{level}`: true } }\nwhileAttached { settings._setting }",
        );
        assert!(
            labels(&mut generated_settings, "settings._setting").is_empty(),
            "compile-time family implementation names must not become members"
        );
    }

    #[test]
    fn completes_methods_after_fields_on_expression_receivers() {
        let source = r#"
struct Path {
    address: address
}

fn Path.resolve() {
    return self.address
}

struct Layout {
    isLoading: Path
}

fn selectedLayout() {
    return Layout {
        isLoading: Path { address: 0x1000 }
    }
}

state "game.exe" {
    loading: bool = selectedLayout().isLoading.resolve()
}
"#;
        let mut database = CompilerDatabase::new(source);
        assert!(
            labels(&mut database, "selectedLayout().isLoading.resolve")
                .contains(&"resolve".to_owned())
        );
    }

    #[test]
    fn completes_transactional_snapshots_on_live_managed_references() {
        let source = r#"
image "Assembly-CSharp" {
    class GameManager {
        static GameManager instance;
        i32 points;
    }
}
state Unity ["game.exe"] {}
fn capture(manager: GameManager.Ref) {
    manager.snap
}
"#;
        let mut database = CompilerDatabase::new(source);
        let offset = source.find("manager.snap").unwrap() + "manager.snap".len();
        let completions = database.completions(offset).unwrap();
        let snapshot = completions
            .items
            .iter()
            .find(|item| item.label == "snapshot")
            .expect("live managed references should complete `snapshot`");
        assert_eq!(snapshot.kind, CompletionKind::Method);
        assert_eq!(
            snapshot.detail.as_deref(),
            Some("GameManager.Ref.snapshot() -> GameManager!")
        );
        assert_eq!(snapshot.insert_text, "snapshot()");
        assert!(
            snapshot
                .documentation
                .as_deref()
                .is_some_and(|documentation| documentation.contains("transactionally"))
        );
    }

    #[test]
    fn completes_cooperative_instance_discovery_on_managed_classes() {
        let source = r#"
image "Assembly-CSharp" {
    class Enemy {
        i32 health;
    }
}
state Unity ["game.exe"] {}
onAttach {
    let enemies = await Enemy.inst
}
"#;
        let mut database = CompilerDatabase::new(source);
        let offset = source.find("Enemy.inst").unwrap() + "Enemy.inst".len();
        let completions = database.completions(offset).unwrap();
        let instances = completions
            .items
            .iter()
            .find(|item| item.label == "instances")
            .expect("managed classes should complete `instances`");
        assert_eq!(instances.kind, CompletionKind::Method);
        assert_eq!(
            instances.detail.as_deref(),
            Some("Enemy.instances() -> async [Enemy.Ref]")
        );
        assert_eq!(instances.insert_text, "instances()");
        assert!(
            instances
                .documentation
                .as_deref()
                .is_some_and(|documentation| documentation.contains("Cooperatively"))
        );
    }

    #[test]
    fn completes_typed_components_on_unity_game_objects() {
        let source = r#"
image "Assembly-CSharp" {
    class PlayerController {
        i32 health;
    }
}
state Unity ["game.exe"] {}
whileAttached {
    let scene = unity.scenes.active() else return
    let player = scene.find("World/Player") else return
    player.comp
}
"#;
        let mut database = CompilerDatabase::new(source);
        let offset = source.find("player.comp").unwrap() + "player.comp".len();
        let completions = database.completions(offset).unwrap();
        let component = completions
            .items
            .iter()
            .find(|item| item.label == "component")
            .expect("Unity GameObjects should complete `component<T>()`");
        assert_eq!(component.kind, CompletionKind::Method);
        assert_eq!(
            component.detail.as_deref(),
            Some("UnityGameObject.component<T>() -> T.Ref!")
        );
        assert_eq!(component.insert_text, "component<${1:Class}>()");
    }

    #[test]
    fn completes_catalog_methods_on_existing_expression_receiver_calls() {
        let source = r#"
struct Layout {
    isLoading: MemoryPath
}

fn selectedLayout(layout: Layout) {
    return layout
}

fn inspect(layout: Layout) {
    selectedLayout(layout).isLoading.resolve()
}

state "game.exe" {}
"#;
        let mut database = CompilerDatabase::new(source);
        assert!(
            labels(&mut database, "selectedLayout(layout).isLoading.resolve")
                .contains(&"resolve".to_owned())
        );
    }

    #[test]
    fn completes_array_members_and_indexed_elements_on_snapshot_fields() {
        let source = include_str!("../examples/minish_cap.split");
        let element_source = source.replacen("old.inventory[5]", "old.inventory[5].", 1);
        let mut database = CompilerDatabase::new(element_source);
        let element = labels(&mut database, "old.inventory[5].");
        assert!(element.contains(&"min".to_owned()), "{element:#?}");
        assert!(element.contains(&"max".to_owned()), "{element:#?}");
        assert!(element.contains(&"clamp".to_owned()), "{element:#?}");
        for array_method in ["set", "length", "isEmpty", "contains", "indexOf"] {
            assert!(
                !element.contains(&array_method.to_owned()),
                "array method `{array_method}` leaked onto u8 completion: {element:#?}"
            );
        }

        let incomplete = r#"
state "game.exe" {
    inventory: [u8; 6] at 0x1000
}

split {
    old.inventory.
}
"#;
        let mut database = CompilerDatabase::new(incomplete);
        let completions = labels(&mut database, "old.inventory.");
        assert!(!completions.contains(&"get".to_owned()));
        assert!(completions.contains(&"set".to_owned()));
        assert!(completions.contains(&"length".to_owned()));
        assert!(completions.contains(&"isEmpty".to_owned()));
        assert!(completions.contains(&"contains".to_owned()));
        assert!(completions.contains(&"indexOf".to_owned()));
        assert!(!completions.contains(&"push".to_owned()));
        assert!(!completions.contains(&"extend".to_owned()));
        assert!(!completions.contains(&"removeAt".to_owned()));
        assert!(!completions.contains(&"remove".to_owned()));
        assert!(!completions.contains(&"pop".to_owned()));
        assert!(!completions.contains(&"clear".to_owned()));

        let growable = r#"
state "game.exe" {}

split {
    let values: [u8] = []
    values.
}
"#;
        let mut database = CompilerDatabase::new(growable);
        let completions = labels(&mut database, "values.");
        assert!(completions.contains(&"push".to_owned()));
        assert!(completions.contains(&"extend".to_owned()));
        assert!(completions.contains(&"removeAt".to_owned()));
        assert!(completions.contains(&"remove".to_owned()));
        assert!(completions.contains(&"pop".to_owned()));
        assert!(completions.contains(&"clear".to_owned()));
    }

    #[test]
    fn completes_string_members_in_match_arms_and_on_inferred_locals() {
        let source = include_str!("../examples/minish_cap.split");

        let literal_source = source.replacen("2 => \"½\"", "2 => \"½\".", 1);
        let mut database = CompilerDatabase::new(literal_source);
        let literal = labels(&mut database, "2 => \"½\".");
        for member in [
            "byteLength",
            "isEmpty",
            "contains",
            "startsWith",
            "endsWith",
            "equalsIgnoreAsciiCase",
            "toAsciiLowerCase",
            "toAsciiUpperCase",
            "trimAsciiWhitespace",
            "replaceAll",
            "split",
            "parse",
            "slice",
        ] {
            assert!(literal.contains(&member.to_owned()), "{literal:#?}");
        }

        let local_source = source.replacen("return fraction", "return fraction.", 1);
        let mut database = CompilerDatabase::new(local_source);
        let local = labels(&mut database, "return fraction.");
        for member in [
            "byteLength",
            "isEmpty",
            "contains",
            "startsWith",
            "endsWith",
            "equalsIgnoreAsciiCase",
            "toAsciiLowerCase",
            "toAsciiUpperCase",
            "trimAsciiWhitespace",
            "replaceAll",
            "split",
            "parse",
            "slice",
        ] {
            assert!(local.contains(&member.to_owned()), "{local:#?}");
        }
    }

    #[test]
    fn member_completion_uses_token_boundaries_across_trivia_and_eof() {
        for expression in ["number.", "number.   ", "number. /* gap */ cl"] {
            let source = format!(
                "state \"game.exe\" {{}}\nsetup {{\n    let number: i32 = 4\n    {expression}\n}}"
            );
            let offset = source.find(expression).unwrap() + expression.len();
            let mut database = CompilerDatabase::new(source);
            let completion = database
                .completions(offset)
                .expect("member completion should survive trivia and token boundaries");
            assert!(
                completion.items.iter().any(|item| item.label == "clamp"),
                "{expression}: {completion:#?}"
            );
        }
    }

    #[test]
    fn completes_only_shape_fields_refined_in_match_arms() {
        let source = r#"
enum Build { V8, V9 }
let build: Build
state "game.exe" {
    if build == Build.V8 {
        loading: i32 at 0x100; bike: i16 at 0x104;
    } else {
        loading: i32 at 0x200; bike: u16 at 0x204; video: u8 at 0x206;
    }
}
onAttach { build = Build.V8 }
split {
    return match build {
        Build.V8 => current.,
        Build.V9 => old.,
    }
}
"#;
        let mut database = CompilerDatabase::new(source);
        let v8 = labels(&mut database, "Build.V8 => current.");
        assert!(v8.contains(&"loading".to_owned()));
        assert!(v8.contains(&"bike".to_owned()), "{v8:#?}");
        assert!(!v8.contains(&"video".to_owned()));

        let mut database = CompilerDatabase::new(source);
        let v9 = labels(&mut database, "Build.V9 => old.");
        assert!(v9.contains(&"loading".to_owned()));
        assert!(v9.contains(&"bike".to_owned()));
        assert!(v9.contains(&"video".to_owned()));

        let outside = format!("{source}\nwhileAttached {{ current. }}");
        let mut database = CompilerDatabase::new(outside);
        let outside = labels(&mut database, "whileAttached { current.");
        assert!(outside.contains(&"loading".to_owned()));
        assert!(!outside.contains(&"bike".to_owned()), "{outside:#?}");
        assert!(!outside.contains(&"video".to_owned()));
    }

    #[test]
    fn generic_type_arguments_complete_on_captured_process_values() {
        let source = r#"
state "game.exe" {}
whileAttached {
    let attached = process
    attached.read<i
}
"#;
        let mut database = CompilerDatabase::new(source);
        let completions = labels(&mut database, "attached.read<i");
        assert!(completions.contains(&"i32".to_owned()));
        assert!(completions.contains(&"i64".to_owned()));
    }

    #[test]
    fn constrained_generic_parameters_complete_their_available_methods() {
        let source = r#"
state "game.exe" {}
fn smaller(value, other) {
    let result = value.min(other)
    value.
    return result
}
"#;
        let mut database = CompilerDatabase::new(source);
        let completions = labels(&mut database, "\n    value.");
        assert!(completions.contains(&"min".to_owned()));
        assert!(completions.contains(&"max".to_owned()));
        assert!(completions.contains(&"clamp".to_owned()));
        assert!(!completions.contains(&"toString".to_owned()));
        assert!(!completions.contains(&"length".to_owned()));
    }

    #[test]
    fn destructured_bindings_are_completed_by_their_leaf_names() {
        let source = r#"
struct Point { x: i32, y: i32 }
let Point { x: globalX, y: globalY } = Point { x: 1, y: 2 }
state "game.exe" {}
fn inspect(Point { x, y }: Point) {
    let Point { x: localX, y: localY } = Point { x, y }
    for Point { x: itemX, y: itemY } in [Point { x, y }] {

    }
}
"#;
        let mut database = CompilerDatabase::new(source);
        let completions = labels(&mut database, "] {\n");
        for leaf in [
            "globalX", "globalY", "x", "y", "localX", "localY", "itemX", "itemY",
        ] {
            assert!(
                completions.contains(&leaf.to_owned()),
                "{leaf}: {completions:#?}"
            );
        }
        assert!(!completions.iter().any(|label| label.contains("Point {")));
    }

    #[test]
    fn inherited_capabilities_complete_super_capability_methods() {
        let source = r#"
state "game.exe" {}
fn masked(value) {
    let result = value & 1
    value.
    return result
}
"#;
        let mut database = CompilerDatabase::new(source);
        let completions = labels(&mut database, "\n    value.");
        assert!(completions.contains(&"min".to_owned()));
        assert!(completions.contains(&"max".to_owned()));
        assert!(completions.contains(&"clamp".to_owned()));
        assert!(completions.contains(&"toString".to_owned()));
    }

    #[test]
    fn gba_states_expose_only_the_typed_provider_value() {
        let source = "state GBA { room: u8 = gba.re }";
        let mut database = CompilerDatabase::new(source);
        assert!(labels(&mut database, "gba.re").contains(&"read".to_owned()));

        let root_source = "state GBA {}\nwhileAttached { gb }";
        let mut database = CompilerDatabase::new(root_source);
        let provider_items = labels(&mut database, " gb");
        assert!(provider_items.contains(&"gba".to_owned()));

        let process_source = "state GBA {}\nwhileAttached { pro }";
        let mut database = CompilerDatabase::new(process_source);
        assert!(!labels(&mut database, " pro").contains(&"process".to_owned()));

        let native_source = "state \"game.exe\" {}\nwhileAttached { pro }";
        let mut database = CompilerDatabase::new(native_source);
        assert!(labels(&mut database, " pro").contains(&"process".to_owned()));
    }

    #[test]
    fn provider_values_complete_only_with_an_attached_process() {
        for (state, prefix, provider) in [
            ("state \"game.exe\" {}", "pro", "process"),
            ("state GBA {}", "gb", "gba"),
        ] {
            for action in ["setup", "onDetach", "onStart", "onReset"] {
                let detached = format!("{state}\n{action} {{ {prefix} }}");
                let mut database = CompilerDatabase::new(detached);
                assert!(
                    !labels(&mut database, &format!("{{ {prefix}")).contains(&provider.to_owned()),
                    "{provider} must not complete in {action}"
                );
            }

            for action in ["onAttach", "whileAttached", "split"] {
                let attached = format!("{state}\n{action} {{ {prefix} }}");
                let mut database = CompilerDatabase::new(attached);
                assert!(
                    labels(&mut database, &format!("{{ {prefix}")).contains(&provider.to_owned()),
                    "{provider} should complete in {action}"
                );
            }
        }
    }

    #[test]
    fn attempt_scoped_globals_and_helpers_complete_only_during_an_active_attempt() {
        let declarations = r#"
let runTimeSeconds: f64
state "game.exe" {}
onStart { runTimeSeconds = 0.0 }
fn updateRunTime() { runTimeSeconds += 1.0 }
"#;

        for action in ["setup", "onAttach", "onDetach", "whileAttached", "start"] {
            for prefix in ["runT", "updateR"] {
                let source = format!("{declarations}\n{action} {{ {prefix} }}");
                let mut database = CompilerDatabase::new(source);
                assert!(
                    !labels(&mut database, &format!("{action} {{ {prefix}"))
                        .iter()
                        .any(|label| label == "runTimeSeconds" || label == "updateRunTime"),
                    "attempt-scoped declarations must not complete in {action}"
                );
            }
        }

        for action in ["onReset", "split", "reset", "isLoading", "gameTime"] {
            let source = format!("{declarations}\n{action} {{ runT }}");
            let mut database = CompilerDatabase::new(source);
            assert!(
                labels(&mut database, &format!("{action} {{ runT"))
                    .contains(&"runTimeSeconds".to_owned()),
                "attempt-scoped globals should complete in {action}"
            );

            let source = format!("{declarations}\n{action} {{ updateR }}");
            let mut database = CompilerDatabase::new(source);
            assert!(
                labels(&mut database, &format!("{action} {{ updateR"))
                    .contains(&"updateRunTime".to_owned()),
                "attempt-dependent helpers should complete in {action}"
            );
        }

        let source = declarations.replace(
            "onStart { runTimeSeconds = 0.0 }",
            "onStart { runTimeSeconds = 0.0; runT }",
        );
        let mut database = CompilerDatabase::new(source);
        assert!(
            labels(&mut database, "; runT").contains(&"runTimeSeconds".to_owned()),
            "the attempt initializer must be able to complete its own globals"
        );
    }

    #[test]
    fn attachment_scoped_globals_complete_only_with_an_attached_process() {
        let declarations = r#"
let moduleBase: address
state "game.exe" {}
onAttach { moduleBase = 0x1000 }
"#;

        for action in ["setup", "onDetach", "onStart", "onReset"] {
            let source = format!("{declarations}\n{action} {{ moduleB }}");
            let mut database = CompilerDatabase::new(source);
            assert!(
                !labels(&mut database, &format!("{action} {{ moduleB"))
                    .contains(&"moduleBase".to_owned()),
                "attachment-scoped globals must not complete in {action}"
            );
        }

        for action in ["whileAttached", "start", "split"] {
            let source = format!("{declarations}\n{action} {{ moduleB }}");
            let mut database = CompilerDatabase::new(source);
            assert!(
                labels(&mut database, &format!("{action} {{ moduleB"))
                    .contains(&"moduleBase".to_owned()),
                "attachment-scoped globals should complete in {action}"
            );
        }

        let source = declarations.replace(
            "onAttach { moduleBase = 0x1000 }",
            "onAttach { moduleBase = 0x1000; moduleB }",
        );
        let mut database = CompilerDatabase::new(source);
        assert!(
            labels(&mut database, "; moduleB").contains(&"moduleBase".to_owned()),
            "the attachment initializer must be able to complete its own globals"
        );
    }

    #[test]
    fn process_selection_completes_the_native_candidate_before_provider_setup() {
        let source = "state Unity [\"game.exe\"] {}\nselectProcess { pro }";
        let mut database = CompilerDatabase::new(source);
        let roots = labels(&mut database, "{ pro");
        assert!(roots.contains(&"process".to_owned()));
        assert!(!roots.contains(&"unity".to_owned()));

        let source = "state Unity [\"game.exe\"] {}\nselectProcess { process.pa }";
        let mut database = CompilerDatabase::new(source);
        assert!(labels(&mut database, "process.pa").contains(&"path".to_owned()));
    }

    #[test]
    fn detached_completion_filters_transitive_attached_process_functions() {
        let declarations = r#"
state "game.exe" {}

fn readsProcess() {
    let value: i32 = process.read<i32>(0x100) else return
}

fn relay() {
    readsProcess()
}

fn safe() {
    print("safe")
}
"#;

        let detached_relay = format!("{declarations}\nonDetach {{ rel }}");
        let mut database = CompilerDatabase::new(detached_relay);
        assert!(!labels(&mut database, "{ rel").contains(&"relay".to_owned()));

        let detached_direct = format!("{declarations}\nonDetach {{ rea }}");
        let mut database = CompilerDatabase::new(detached_direct);
        assert!(!labels(&mut database, "{ rea").contains(&"readsProcess".to_owned()));

        let detached_safe = format!("{declarations}\nonDetach {{ sa }}");
        let mut database = CompilerDatabase::new(detached_safe);
        assert!(labels(&mut database, "{ sa").contains(&"safe".to_owned()));

        let setup_relay = format!("{declarations}\nsetup {{ rel }}");
        let mut database = CompilerDatabase::new(setup_relay);
        assert!(!labels(&mut database, "{ rel").contains(&"relay".to_owned()));

        let attached = format!("{declarations}\nonAttach {{ rel }}");
        let mut database = CompilerDatabase::new(attached);
        assert!(labels(&mut database, "{ rel").contains(&"relay".to_owned()));
    }

    #[test]
    fn completion_scopes_snapshot_roots_and_snapshot_dependent_functions() {
        let declarations = r#"
state "game.exe" { level: u32 at 0x100 }

fn changed() {
    return old.level != current.level
}

fn relay() {
    return changed()
}
"#;
        for action in ["setup", "onAttach", "onDetach", "onStart", "onReset"] {
            let source = format!("{declarations}\n{action} {{ cur }}");
            let mut database = CompilerDatabase::new(source);
            assert!(!labels(&mut database, "{ cur").contains(&"current".to_owned()));

            let source = format!("{declarations}\n{action} {{ rel }}");
            let mut database = CompilerDatabase::new(source);
            assert!(!labels(&mut database, "{ rel").contains(&"relay".to_owned()));
        }

        let source = format!("{declarations}\nsplit {{ cur }}");
        let mut database = CompilerDatabase::new(source);
        assert!(labels(&mut database, "{ cur").contains(&"current".to_owned()));

        let source = format!("{declarations}\nsplit {{ rel }}");
        let mut database = CompilerDatabase::new(source);
        assert!(labels(&mut database, "{ rel").contains(&"relay".to_owned()));

        let function = format!("{declarations}\nfn another() {{ cur }}");
        let mut database = CompilerDatabase::new(function);
        assert!(labels(&mut database, "{ cur").contains(&"current".to_owned()));
    }

    #[test]
    fn root_completion_comes_from_language_standard_library_and_source() {
        let source = "state \"game.exe\" {}\nlet customValue = 0\nwhileAttached { pri }";
        let mut database = CompilerDatabase::new(source);
        let print_labels = labels(&mut database, "pri");
        assert!(print_labels.contains(&"print".to_owned()));

        let offset = source.find("customValue").unwrap() + 2;
        let all = database.completions(offset).unwrap();
        assert!(all.items.iter().any(|item| item.label == "customValue"));

        let empty_prefix = source.find(" pri").unwrap() + 1;
        let all = database.completions(empty_prefix).unwrap();
        for absent in [
            "whileAttached",
            "class",
            "closure",
            "enum",
            "range",
            "else",
            "as",
            "async",
            "break",
            "continue",
            "self",
        ] {
            assert!(
                all.items.iter().all(|item| item.label != absent),
                "`{absent}` is not valid as an unqualified completion here: {all:#?}"
            );
        }
        for present in ["let", "if", "while", "loop", "for", "match", "return"] {
            assert!(
                all.items.iter().any(|item| item.label == present),
                "missing `{present}`: {all:#?}"
            );
        }
        assert!(all.items.iter().all(|item| item.label != "utf8"));

        let loop_source = "state \"game.exe\" {}\nsplit { while true { con } }";
        let mut loop_database = CompilerDatabase::new(loop_source);
        let loop_items = labels(&mut loop_database, "con");
        assert!(loop_items.contains(&"continue".to_owned()));

        let method_source =
            "state \"game.exe\" {}\nstruct Point { x: i32 }\nfn Point.value() { sel }";
        let mut method_database = CompilerDatabase::new(method_source);
        assert!(labels(&mut method_database, "sel").contains(&"self".to_owned()));
    }

    #[test]
    fn settings_completes_as_both_a_declaration_and_an_expression_value() {
        let source = r#"state "game.exe" {}
settings {
    "User Name" => userName: "Player",
}
whileAttached {
    setVariable("UserName", setti)
}
"#;
        let mut database = CompilerDatabase::new(source);
        let completions = database
            .completions(source.find("setti)").unwrap() + "setti".len())
            .expect("expression completion should succeed");
        let settings = completions
            .items
            .iter()
            .find(|item| item.label == "settings")
            .expect("the current settings view should complete in expressions");
        assert_eq!(settings.kind, CompletionKind::Variable);
        assert_eq!(settings.detail.as_deref(), Some("SettingsView"));
        assert_eq!(settings.insert_text, "settings");
        assert!(!settings.is_snippet);
        assert_eq!(
            settings.documentation_uri.as_deref(),
            Some("/language/settings.md")
        );

        let top_level = "state \"game.exe\" {}\nsetti";
        let mut database = CompilerDatabase::new(top_level);
        let completion = database
            .completions(top_level.len())
            .expect("top-level completion should succeed");
        let declaration = completion
            .items
            .iter()
            .find(|item| item.label == "settings")
            .expect("the settings declaration should still complete at top level");
        assert!(declaration.is_snippet);
        assert!(declaration.insert_text.starts_with("settings {"));
    }

    #[test]
    fn provider_context_completion_follows_attachment_availability_and_type() {
        let mut database = CompilerDatabase::new("state Unity [\"game.exe\"] {}\nonAttach { uni }");
        let attached = labels(&mut database, "uni }");
        assert!(attached.contains(&"unity".to_owned()), "{attached:#?}");

        let mut database =
            CompilerDatabase::new("state Unity [\"game.exe\"] {}\nonAttach { unity. }");
        let context = labels(&mut database, "unity.");
        assert!(context.contains(&"scenes".to_owned()), "{context:#?}");

        let mut database =
            CompilerDatabase::new("state Unity [\"game.exe\"] {}\nonAttach { unity.scenes. }");
        let scenes = labels(&mut database, "unity.scenes.");
        for method in ["active", "loaded", "persistent"] {
            assert!(
                scenes.contains(&method.to_owned()),
                "missing `{method}`: {scenes:#?}"
            );
        }

        let source = "state Unity [\"game.exe\"] {}\nonDetach { uni }";
        let mut database = CompilerDatabase::new(source);
        let detached_offset = source.rfind("uni }").unwrap() + "uni".len();
        let detached = database.completions(detached_offset).unwrap();
        assert!(detached.items.iter().all(|item| item.label != "unity"));
    }

    #[test]
    fn completes_value_loops_with_language_documentation() {
        let source = "state \"game.exe\" {}\nfn choose() { let value = loo }";
        let mut database = CompilerDatabase::new(source);
        let completions = database
            .completions(source.find("loo }").unwrap() + 3)
            .unwrap();
        let item = completions
            .items
            .iter()
            .find(|item| item.label == "loop")
            .expect("loop should complete in expression position");

        assert_eq!(item.kind, CompletionKind::Snippet);
        assert_eq!(item.insert_text, "loop {\n    $0\n}");
        assert!(item.is_snippet);
        assert!(item.documentation.as_deref().unwrap().contains("break"));
        assert_eq!(item.documentation_uri.as_deref(), Some("/language/loop.md"));
    }

    #[test]
    fn state_decoder_completion_is_contextual_and_inserts_its_bound() {
        let source = "state \"game.exe\" { mapName at 0x100 as ut }";
        let mut database = CompilerDatabase::new(source);
        let offset = source.find("ut }").unwrap() + 2;
        let completions = database.completions(offset).unwrap();
        let decoder = completions
            .items
            .iter()
            .find(|item| item.label == "utf8")
            .expect("the pointer-state decoder should complete after `as`");
        assert_eq!(decoder.kind, CompletionKind::Function);
        assert_eq!(decoder.insert_text, "utf8(${1:maxBytes})");
        assert!(decoder.is_snippet);
        let wide = completions
            .items
            .iter()
            .find(|item| item.label == "utf16le")
            .expect("the UTF-16LE decoder should share the state-decoder completion");
        assert_eq!(wide.insert_text, "utf16le(${1:maxUtf16Units})");
        assert!(wide.is_snippet);

        let cast_source = "state \"game.exe\" {} whileAttached { let value = 1 as ut }";
        let mut database = CompilerDatabase::new(cast_source);
        let offset = cast_source.find("ut }").unwrap() + 2;
        assert!(
            database
                .completions(offset)
                .unwrap()
                .items
                .iter()
                .all(|item| item.label != "utf8" && item.label != "utf16le")
        );
    }

    #[test]
    fn root_completion_includes_preceding_lexical_bindings() {
        let source = r#"
state "game.exe" {}

fn inspect(parameter: i32) {
    let localValue = parameter
    loc
}

onAttach {
    let image = await unity.il2cpp()
    let gameManagerClass = await image.class("GameManager")
    gam
    let gameManagerInstance = 0
}
"#;
        let mut database = CompilerDatabase::new(source);
        let action_labels = labels(&mut database, "\n    gam");
        assert!(action_labels.contains(&"gameManagerClass".to_owned()));
        assert!(!action_labels.contains(&"gameManagerInstance".to_owned()));

        let function_labels = labels(&mut database, "\n    loc");
        assert!(function_labels.contains(&"localValue".to_owned()));

        let parameter_offset = source.find("localValue = par").unwrap() + "localValue = par".len();
        let parameter_completion = database.completions(parameter_offset).unwrap();
        assert!(
            parameter_completion.items.iter().any(
                |item| item.label == "parameter" && item.detail.as_deref() == Some("parameter")
            )
        );
    }

    #[test]
    fn managed_classes_expose_only_globally_refined_shape_fields() {
        let schema = r#"
enum Edition { BaseGame, Demo }
let edition: Edition
image "Assembly-CSharp" {
    class GameManager {
        static GameManager instance;
        u32 common;
        if edition == Edition.BaseGame { u32 level; }
        else { u32 scene; }
    }
}
state Unity ["game.exe"] {}
onAttach { edition = Edition.BaseGame }
"#;

        let source = format!("{schema}\nwhileAttached {{ GameManager. }}");
        let mut database = CompilerDatabase::new(source);
        let class = labels(&mut database, "GameManager.");
        assert!(class.contains(&"instance".to_owned()), "{class:#?}");
        assert!(!class.contains(&"layout".to_owned()), "{class:#?}");
        assert!(!class.contains(&"Layout".to_owned()), "{class:#?}");

        let source = format!(
            "{schema}\nwhileAttached {{\n    let manager = GameManager.instance else return\n    if edition == Edition.BaseGame {{\n        manager.\n    }}\n}}"
        );
        let mut database = CompilerDatabase::new(source);
        let fields = labels(&mut database, "        manager.");
        assert!(fields.contains(&"common".to_owned()), "{fields:#?}");
        assert!(fields.contains(&"level".to_owned()), "{fields:#?}");
        assert!(!fields.contains(&"scene".to_owned()), "{fields:#?}");

        let source = format!(
            "{schema}\nwhileAttached {{\n    let manager = GameManager.instance else return\n    if edition == Edition.BaseGame {{\n        print(\"base\")\n    }} else {{\n        manager.\n    }}\n}}"
        );
        let mut database = CompilerDatabase::new(source);
        let fields = labels(&mut database, "        manager.");
        assert!(fields.contains(&"common".to_owned()), "{fields:#?}");
        assert!(!fields.contains(&"level".to_owned()), "{fields:#?}");
        assert!(fields.contains(&"scene".to_owned()), "{fields:#?}");
    }

    #[test]
    fn for_bindings_complete_only_inside_the_loop_body() {
        let source = r#"
state "game.exe" {}
whileAttached {
    let values = [1, 2]
    for element in values {
        ele
    }
    ele
}
"#;
        let mut database = CompilerDatabase::new(source);
        assert!(labels(&mut database, "        ele").contains(&"element".to_owned()));
        assert!(!labels(&mut database, "    ele\n}").contains(&"element".to_owned()));
    }

    #[test]
    fn conditional_pattern_bindings_complete_only_on_proven_paths() {
        let source = r#"
state "game.exe" {}
fn inspect(value: u32?) {
    if value is Some(number) && num {
        number
    } else {
        missing
    }
}
"#;
        let mut database = CompilerDatabase::new(source);
        assert!(labels(&mut database, "&& num").contains(&"number".to_owned()));
        assert!(labels(&mut database, "        number").contains(&"number".to_owned()));
        assert!(!labels(&mut database, "        missing").contains(&"number".to_owned()));

        let source = r#"
state "game.exe" {}
fn inspect(value: u32?) {
    if !(value is Some(number)) {
        missing
    } else {
        num
    }
}
"#;
        let mut database = CompilerDatabase::new(source);
        assert!(!labels(&mut database, "        missing").contains(&"number".to_owned()));
        assert!(labels(&mut database, "        num").contains(&"number".to_owned()));
    }

    #[test]
    fn state_filter_bindings_complete_with_the_field_type() {
        let source = r#"state "game.exe" {
    scene: i32 at 0x100 if value.min(7) == 7 { Err("transient") } else { value }
}"#;
        let mut database = CompilerDatabase::new(source);
        let completions = labels(&mut database, "value.");
        assert!(completions.contains(&"min".to_owned()));
        assert!(completions.contains(&"max".to_owned()));
        assert!(completions.contains(&"clamp".to_owned()));
        assert!(!completions.contains(&"length".to_owned()));
    }
}
