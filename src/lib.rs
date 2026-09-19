//! SplitScript compiler library.
//!
//! The compiler intentionally has a small public API: parse, type-check, and
//! compile a source file to a WebAssembly GC module.

mod abi;
mod scoped_globals;
pub use scoped_globals::{AttachmentShape, GlobalLifetime, ScopedGlobalAnalysis};
pub use splitscript_syntax::ast;
mod build_identity;
mod capabilities;
mod catalog;
mod codegen;
mod compilation_cancellation;
pub mod compiler;
mod completion;
mod constant;
mod database;
pub use splitscript_syntax::diagnostic;
mod documentation;
pub use documentation::{
    DocumentationIndexEntry, DocumentationPage, DocumentationReference, escape_markdown_symbol,
};
mod editorconfig;
mod effects;
mod equality;
mod formatter;
mod highlight;
mod hir;
mod inference;
mod inlay_hints;
mod insight;
mod intrinsic_registry;
mod language;
mod lexer;
mod lsp;
mod managed;
mod managed_read;
mod memory;
pub mod migration;
mod name_matching;
mod shape_selection;
use splitscript_syntax::parser;
mod refactor;
mod resolution;
mod selection_ranges;
mod semantic;
mod service;
mod signature;
mod stdlib;
mod stdlib_semantic;
mod structural;
mod symbols;
pub use splitscript_syntax::source as syntax;
pub use splitscript_syntax::visit;
pub mod tooling;
mod type_display;
mod typeck;
mod types;
mod validation;
mod wasm_ir;

pub use build_identity::{
    COMPILER_GIT_REVISION, COMPILER_VERSION, COMPILER_VERSION_TEXT, CompilerIdentity,
    compiler_identity,
};
pub use compilation_cancellation::{
    CompilationCancellation, CompilationCancelled, CompilationFailure, CompilationPhase,
};
pub use diagnostic::{
    Diagnostic, DiagnosticCode, DiagnosticFix, DiagnosticFixes, DiagnosticLabel,
    DiagnosticLabelStyle, DiagnosticSeverity, FixApplicability, TextEdit,
};
#[cfg(not(target_arch = "wasm32"))]
pub use editorconfig::load_for_path as format_options_for_path;
pub use editorconfig::{EditorConfigLayer, resolve as resolve_editorconfig};
pub use formatter::{
    DEFAULT_INDENT_WIDTH, DEFAULT_MAX_LINE_WIDTH, FormatOptions, IndentStyle, LineEnding,
    format_source, format_source_with_options,
};

/// Controls profile-sensitive semantic lowering and WebAssembly generation.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum BuildProfile {
    /// Keeps constructs marked for development-time diagnostics and logging.
    #[default]
    Debug,
    /// Erases debug-only constructs before backend reachability is computed.
    Release,
}

/// Configures how a warning participates in a particular compiler product.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum WarningLevel {
    /// Omits the diagnostic from the configured product.
    Allow,
    /// Publishes the diagnostic without rejecting compilation.
    #[default]
    Warn,
    /// Publishes the diagnostic as an error and rejects compilation.
    Deny,
}

/// Per-warning policy selected by a compiler host or project configuration.
///
/// Warning generation remains independent from this policy. This value is
/// applied only when diagnostics cross a compiler-product boundary, preserving
/// the original `SS100x` code even when a denied warning rejects a build.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, serde::Serialize, serde::Deserialize)]
#[serde(default, rename_all = "camelCase")]
pub struct WarningPolicy {
    must_use: WarningLevel,
    unused_binding: WarningLevel,
    unused_declaration: WarningLevel,
    unused_member: WarningLevel,
    value_block_semicolon: WarningLevel,
    ambiguous_retry_fallback: WarningLevel,
    static_setting_lookup: WarningLevel,
    suspicious_interpolation: WarningLevel,
    debug_only_use: WarningLevel,
    struct_field_shorthand: WarningLevel,
    empty_future_race: WarningLevel,
    always_matches_pattern: WarningLevel,
}

impl WarningPolicy {
    pub const fn level(self, code: DiagnosticCode) -> Option<WarningLevel> {
        match code {
            DiagnosticCode::MustUse => Some(self.must_use),
            DiagnosticCode::UnusedBinding => Some(self.unused_binding),
            DiagnosticCode::UnusedDeclaration => Some(self.unused_declaration),
            DiagnosticCode::UnusedMember => Some(self.unused_member),
            DiagnosticCode::ValueBlockSemicolon => Some(self.value_block_semicolon),
            DiagnosticCode::AmbiguousRetryFallback => Some(self.ambiguous_retry_fallback),
            DiagnosticCode::StaticSettingLookup => Some(self.static_setting_lookup),
            DiagnosticCode::SuspiciousInterpolation => Some(self.suspicious_interpolation),
            DiagnosticCode::DebugOnlyUse => Some(self.debug_only_use),
            DiagnosticCode::StructFieldShorthand => Some(self.struct_field_shorthand),
            DiagnosticCode::EmptyFutureRace => Some(self.empty_future_race),
            DiagnosticCode::AlwaysMatchesPattern => Some(self.always_matches_pattern),
            DiagnosticCode::Lexical
            | DiagnosticCode::Syntax
            | DiagnosticCode::Type
            | DiagnosticCode::Semantic => None,
        }
    }

    /// Sets one warning code, returning `false` for non-warning diagnostics.
    pub const fn set(&mut self, code: DiagnosticCode, level: WarningLevel) -> bool {
        let target = match code {
            DiagnosticCode::MustUse => &mut self.must_use,
            DiagnosticCode::UnusedBinding => &mut self.unused_binding,
            DiagnosticCode::UnusedDeclaration => &mut self.unused_declaration,
            DiagnosticCode::UnusedMember => &mut self.unused_member,
            DiagnosticCode::ValueBlockSemicolon => &mut self.value_block_semicolon,
            DiagnosticCode::AmbiguousRetryFallback => &mut self.ambiguous_retry_fallback,
            DiagnosticCode::StaticSettingLookup => &mut self.static_setting_lookup,
            DiagnosticCode::SuspiciousInterpolation => &mut self.suspicious_interpolation,
            DiagnosticCode::DebugOnlyUse => &mut self.debug_only_use,
            DiagnosticCode::StructFieldShorthand => &mut self.struct_field_shorthand,
            DiagnosticCode::EmptyFutureRace => &mut self.empty_future_race,
            DiagnosticCode::AlwaysMatchesPattern => &mut self.always_matches_pattern,
            DiagnosticCode::Lexical
            | DiagnosticCode::Syntax
            | DiagnosticCode::Type
            | DiagnosticCode::Semantic => return false,
        };
        *target = level;
        true
    }

    pub fn set_all(&mut self, level: WarningLevel) {
        for code in DiagnosticCode::WARNINGS {
            let changed = self.set(code, level);
            debug_assert!(changed);
        }
    }

    /// Applies this policy while retaining diagnostic codes and structured
    /// source information. Parser and type errors are never affected.
    pub fn apply(self, diagnostics: impl IntoIterator<Item = Diagnostic>) -> Vec<Diagnostic> {
        diagnostics
            .into_iter()
            .filter_map(|mut diagnostic| {
                if diagnostic.severity != DiagnosticSeverity::Warning {
                    return Some(diagnostic);
                }
                match self.level(diagnostic.code) {
                    Some(WarningLevel::Allow) => None,
                    Some(WarningLevel::Deny) => {
                        diagnostic.severity = DiagnosticSeverity::Error;
                        diagnostic.notes.push(format!(
                            "warning {} is denied by the active warning policy",
                            diagnostic.code
                        ));
                        Some(diagnostic)
                    }
                    Some(WarningLevel::Warn) | None => Some(diagnostic),
                }
            })
            .collect()
    }
}

/// Options shared by staged and one-shot compilation.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, serde::Serialize, serde::Deserialize)]
#[serde(default, rename_all = "camelCase")]
pub struct CompilerOptions {
    pub profile: BuildProfile,
    pub warnings: WarningPolicy,
}

/// Immutable compiler-wide services shared by every stage of one compilation.
///
/// The build-time privileged SplitScript loader supplies the bundled catalog;
/// this context is the runtime injection boundary for that validated graph.
/// Individual passes consume the context instead of reconstructing global
/// catalog state.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct CompilerContext {
    standard_library: stdlib::StandardLibrary,
    include_standard_library_bodies: bool,
}

impl Default for CompilerContext {
    fn default() -> Self {
        Self {
            standard_library: stdlib::StandardLibrary::default(),
            include_standard_library_bodies: true,
        }
    }
}

impl CompilerContext {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn with_standard_library(standard_library: stdlib::StandardLibrary) -> Self {
        Self {
            standard_library,
            include_standard_library_bodies: true,
        }
    }

    pub fn standard_library(&self) -> stdlib::StandardLibrary {
        self.standard_library.clone()
    }

    /// Documentation snippets need catalog signatures, capabilities, and
    /// precomputed effects, but never compile standard-library implementation
    /// bodies. Skipping their source injection avoids reparsing the entire
    /// library once for every independently highlighted example.
    pub(crate) fn without_standard_library_bodies(mut self) -> Self {
        self.include_standard_library_bodies = false;
        self
    }
}

/// Compiles the bundled source bodies as a self-contained program and derives
/// their transitive operational metadata through the ordinary semantic
/// pipeline. This runs once for each independently owned standard-library
/// graph and never depends on user source.
fn derive_standard_library_operation_metadata(
    standard_library: stdlib::StandardLibrary,
) -> Result<
    std::collections::HashMap<stdlib::StdlibItemId, stdlib::OperationMetadata>,
    Vec<Diagnostic>,
> {
    let context = CompilerContext::with_standard_library(standard_library.clone());
    let checked = check(lower(parse_with_context(
        context,
        "state \"__splitscript_standard_library__\" {}",
    )?))?;
    let mut operations = std::collections::HashMap::new();
    for item in standard_library.all_items() {
        let mut functions = checked.hir.library_functions(item.id);
        let Some(first) = functions.next() else {
            continue;
        };
        let metadata = functions.fold(
            checked.effects.function(first).metadata(),
            |combined, function| {
                combined.conservative_union(checked.effects.function(function).metadata())
            },
        );
        operations.insert(item.id, metadata);
    }
    Ok(operations)
}

/// Immutable source-owned products shared by every compiler stage for one
/// revision. Later stages add transformed facts without copying the lossless
/// document or syntax tree.
#[derive(Debug)]
struct ParsedSource {
    source_name: String,
    document: std::sync::Arc<syntax::SourceDocument>,
    syntax: std::sync::Arc<ast::Program>,
}

/// A source file that has been parsed but not semantically checked.
#[derive(Debug, Clone)]
pub struct ParsedProgram {
    context: CompilerContext,
    source: std::sync::Arc<ParsedSource>,
    syntax_diagnostics: Vec<Diagnostic>,
    resolution_diagnostics: Vec<Diagnostic>,
}

/// A lossless, partial parse intended for diagnostics and editor tooling.
/// Unlike [`ParsedProgram`], this remains available when syntax errors were
/// recovered at top-level declaration boundaries.
#[derive(Debug, Clone)]
pub struct RecoveredParse {
    context: CompilerContext,
    source: std::sync::Arc<ParsedSource>,
    diagnostics: Vec<Diagnostic>,
    resolution_diagnostics: Vec<Diagnostic>,
    recovery_nodes: Vec<syntax::RecoveryNode>,
}

impl RecoveredParse {
    pub fn context(&self) -> CompilerContext {
        self.context.clone()
    }

    pub fn source_document(&self) -> &syntax::SourceDocument {
        &self.source.document
    }

    pub fn source_name(&self) -> &str {
        &self.source.source_name
    }

    pub fn syntax(&self) -> &ast::Program {
        &self.source.syntax
    }

    pub fn diagnostics(&self) -> &[Diagnostic] {
        &self.diagnostics
    }

    /// Nominal declaration conflicts retained independently from syntax
    /// recovery. These are reported by checking, not by parsing.
    pub fn resolution_diagnostics(&self) -> &[Diagnostic] {
        &self.resolution_diagnostics
    }

    pub fn recovery_nodes(&self) -> &[syntax::RecoveryNode] {
        &self.recovery_nodes
    }
}

/// A parsed program with declaration identities collected into an inspectable
/// pre-type-check HIR product.
#[derive(Debug, Clone)]
pub struct LoweredProgram {
    context: CompilerContext,
    source: std::sync::Arc<ParsedSource>,
    /// User syntax plus compiler-owned standard-library bodies. Kept private
    /// so editor and public compiler queries never expose injected symbols.
    compilation_syntax: std::sync::Arc<ast::Program>,
    hir: std::sync::Arc<hir::DeclarationIndex>,
    resolutions: std::sync::Arc<resolution::ProgramResolutions>,
    /// Parser diagnostics retained only by editor-oriented recovered lowering.
    /// Strictly parsed programs always leave this empty.
    syntax_diagnostics: Vec<Diagnostic>,
    resolution_diagnostics: Vec<Diagnostic>,
}

impl LoweredProgram {
    pub fn context(&self) -> CompilerContext {
        self.context.clone()
    }

    pub fn source_document(&self) -> &syntax::SourceDocument {
        &self.source.document
    }

    pub fn source_name(&self) -> &str {
        &self.source.source_name
    }

    pub fn syntax(&self) -> &ast::Program {
        &self.source.syntax
    }

    pub fn hir(&self) -> &hir::DeclarationIndex {
        &self.hir
    }

    pub fn resolution_diagnostics(&self) -> &[Diagnostic] {
        &self.resolution_diagnostics
    }
}

impl From<ParsedProgram> for LoweredProgram {
    fn from(parsed: ParsedProgram) -> Self {
        lower(parsed)
    }
}

impl ParsedProgram {
    pub fn context(&self) -> CompilerContext {
        self.context.clone()
    }

    pub fn source_document(&self) -> &syntax::SourceDocument {
        &self.source.document
    }

    pub fn source_name(&self) -> &str {
        &self.source.source_name
    }

    pub fn syntax(&self) -> &ast::Program {
        &self.source.syntax
    }

    pub fn into_syntax(self) -> ast::Program {
        let syntax = std::sync::Arc::try_unwrap(self.source)
            .map_or_else(|source| source.syntax.clone(), |source| source.syntax);
        std::sync::Arc::try_unwrap(syntax).unwrap_or_else(|syntax| syntax.as_ref().clone())
    }
}

/// A successfully checked program and the resolved semantic facts needed by
/// later compiler stages and editor tooling.
#[derive(Debug, Clone)]
pub struct CheckedProgram {
    context: CompilerContext,
    source: std::sync::Arc<ParsedSource>,
    compilation_syntax: std::sync::Arc<ast::Program>,
    hir: hir::TypedProgram,
    diagnostics: Vec<Diagnostic>,
    semantics: semantic::SemanticModel,
    capabilities: capabilities::CapabilityAnalysis,
    effects: effects::OperationAnalysis,
    scoped_globals: ScopedGlobalAnalysis,
    enum_types: Vec<ast::EnumDecl>,
    array_types: Vec<types::ResolvedArrayType>,
    option_types: Vec<types::ResolvedOptionType>,
    result_types: Vec<types::ResolvedResultType>,
    async_types: Vec<types::ResolvedAsyncType>,
    iterator_types: Vec<types::ResolvedIteratorType>,
    callable_types: Vec<types::ResolvedCallableType>,
    range_types: Vec<types::ResolvedRangeType>,
    set_types: Vec<types::ResolvedSetType>,
    application_types: Vec<types::ResolvedApplicationType>,
}

/// Semantic facts retained for editor tooling even when type checking reports
/// errors. Expressions that could not be typed may be absent, while facts from
/// independent declarations and expressions remain queryable.
#[derive(Debug, Clone)]
pub struct RecoveredCheck {
    data: RecoveryData,
}

#[derive(Debug, Clone)]
enum RecoveryData {
    Checked(std::sync::Arc<CheckedProgram>),
    Partial(std::sync::Arc<PartialCheck>),
}

#[derive(Debug)]
struct PartialCheck {
    context: CompilerContext,
    source: std::sync::Arc<ParsedSource>,
    hir: std::sync::Arc<hir::DeclarationIndex>,
    semantics: semantic::SemanticModel,
    diagnostics: Vec<Diagnostic>,
    enum_types: Vec<ast::EnumDecl>,
    effects: Option<effects::OperationAnalysis>,
}

impl RecoveredCheck {
    fn partial(data: PartialCheck) -> Self {
        Self {
            data: RecoveryData::Partial(std::sync::Arc::new(data)),
        }
    }

    /// Used only when strict and recovery validation prerequisites agree.
    pub(crate) fn from_checked(checked: std::sync::Arc<CheckedProgram>) -> Self {
        Self {
            data: RecoveryData::Checked(checked),
        }
    }

    pub fn context(&self) -> CompilerContext {
        match &self.data {
            RecoveryData::Checked(checked) => checked.context(),
            RecoveryData::Partial(partial) => partial.context.clone(),
        }
    }

    pub fn source_document(&self) -> &syntax::SourceDocument {
        match &self.data {
            RecoveryData::Checked(checked) => checked.source_document(),
            RecoveryData::Partial(partial) => &partial.source.document,
        }
    }

    pub fn source_name(&self) -> &str {
        match &self.data {
            RecoveryData::Checked(checked) => checked.source_name(),
            RecoveryData::Partial(partial) => &partial.source.source_name,
        }
    }

    pub fn syntax(&self) -> &ast::Program {
        match &self.data {
            RecoveryData::Checked(checked) => checked.syntax(),
            RecoveryData::Partial(partial) => &partial.source.syntax,
        }
    }

    pub fn hir(&self) -> &hir::DeclarationIndex {
        match &self.data {
            RecoveryData::Checked(checked) => checked.typed_hir().declarations(),
            RecoveryData::Partial(partial) => &partial.hir,
        }
    }

    pub fn semantics(&self) -> &semantic::SemanticModel {
        match &self.data {
            RecoveryData::Checked(checked) => checked.semantics(),
            RecoveryData::Partial(partial) => &partial.semantics,
        }
    }

    pub fn diagnostics(&self) -> &[Diagnostic] {
        match &self.data {
            RecoveryData::Checked(checked) => checked.diagnostics(),
            RecoveryData::Partial(partial) => &partial.diagnostics,
        }
    }

    pub fn enum_types(&self) -> &[ast::EnumDecl] {
        match &self.data {
            RecoveryData::Checked(checked) => checked.enum_types(),
            RecoveryData::Partial(partial) => &partial.enum_types,
        }
    }

    /// Operational effects are available when type recovery completed without
    /// errors, even if a later semantic validation rejected the program.
    pub fn effects(&self) -> Option<&effects::OperationAnalysis> {
        match &self.data {
            RecoveryData::Checked(checked) => Some(&checked.effects),
            RecoveryData::Partial(partial) => partial.effects.as_ref(),
        }
    }
}

impl CheckedProgram {
    pub fn context(&self) -> CompilerContext {
        self.context.clone()
    }

    pub fn source_document(&self) -> &syntax::SourceDocument {
        &self.source.document
    }

    pub fn source_name(&self) -> &str {
        &self.source.source_name
    }

    pub fn syntax(&self) -> &ast::Program {
        &self.source.syntax
    }

    pub fn semantics(&self) -> &semantic::SemanticModel {
        &self.semantics
    }

    pub fn hir(&self) -> &hir::DeclarationIndex {
        self.hir.declarations()
    }

    pub fn typed_hir(&self) -> &hir::TypedProgram {
        &self.hir
    }

    /// Non-fatal diagnostics produced while checking this valid program.
    pub fn diagnostics(&self) -> &[Diagnostic] {
        &self.diagnostics
    }

    pub fn memory_layouts(&self) -> &memory::MemoryLayouts {
        self.capabilities.memory()
    }

    pub fn equality(&self) -> &equality::EqualityCapabilities {
        self.capabilities.equality()
    }

    pub fn capabilities(&self) -> &capabilities::CapabilityAnalysis {
        &self.capabilities
    }

    pub fn effects(&self) -> &effects::OperationAnalysis {
        &self.effects
    }

    pub fn scoped_globals(&self) -> &ScopedGlobalAnalysis {
        &self.scoped_globals
    }

    /// Source enum layouts visible to semantic analysis. Standard-library
    /// enums retain their catalog identities and are not synthesized here.
    pub fn enum_types(&self) -> &[ast::EnumDecl] {
        &self.enum_types
    }
}

const IN_MEMORY_SOURCE_NAME: &str = "input.split";

/// Parses one SplitScript source file without running semantic analysis.
pub fn parse(source: &str) -> Result<ParsedProgram, Vec<Diagnostic>> {
    parse_named_with_context(CompilerContext::default(), IN_MEMORY_SOURCE_NAME, source)
}

pub fn parse_with_context(
    context: CompilerContext,
    source: &str,
) -> Result<ParsedProgram, Vec<Diagnostic>> {
    parse_named_with_context(context, IN_MEMORY_SOURCE_NAME, source)
}

/// Parses a source file while retaining its debugger-visible path or URI.
pub fn parse_named(
    source_name: impl Into<String>,
    source: &str,
) -> Result<ParsedProgram, Vec<Diagnostic>> {
    parse_named_with_context(CompilerContext::default(), source_name, source)
}

pub fn parse_named_with_context(
    context: CompilerContext,
    source_name: impl Into<String>,
    source: &str,
) -> Result<ParsedProgram, Vec<Diagnostic>> {
    let recovered = parse_recovering_named_with_context(context.clone(), source_name, source)?;
    if recovered
        .diagnostics
        .iter()
        .any(|diagnostic| diagnostic.severity == DiagnosticSeverity::Error)
    {
        return Err(recovered.diagnostics);
    }
    Ok(ParsedProgram {
        context,
        source: recovered.source,
        syntax_diagnostics: recovered.diagnostics,
        resolution_diagnostics: recovered.resolution_diagnostics,
    })
}

/// Parses as much of one SplitScript source file as possible. Lexical and
/// parser errors are returned alongside an offset-preserving partial syntax
/// tree so editor features can continue operating on independent valid regions.
pub fn parse_recovering(source: &str) -> Result<RecoveredParse, Vec<Diagnostic>> {
    parse_recovering_named_with_context(CompilerContext::default(), IN_MEMORY_SOURCE_NAME, source)
}

pub fn parse_recovering_with_context(
    context: CompilerContext,
    source: &str,
) -> Result<RecoveredParse, Vec<Diagnostic>> {
    parse_recovering_named_with_context(context, IN_MEMORY_SOURCE_NAME, source)
}

pub fn parse_recovering_named_with_context(
    context: CompilerContext,
    source_name: impl Into<String>,
    source: &str,
) -> Result<RecoveredParse, Vec<Diagnostic>> {
    let (lexed, mut diagnostics) = lexer::lex_lossless_recovering(source);
    let tokens = lexed.tokens().cloned().collect();
    let output = parser::parse_recovering(source, tokens);
    diagnostics.extend(output.diagnostics);
    diagnostics.sort_by_key(|diagnostic| {
        (
            diagnostic.span.start,
            u8::from(diagnostic.code != DiagnosticCode::Lexical),
            diagnostic.span.end,
        )
    });
    let resolution_diagnostics =
        resolution::validate_declarations(&output.program, &context.standard_library());
    Ok(RecoveredParse {
        context,
        source: std::sync::Arc::new(ParsedSource {
            source_name: source_name.into(),
            document: std::sync::Arc::new(syntax::SourceDocument::from_lexed(source, lexed)),
            syntax: std::sync::Arc::new(output.program),
        }),
        diagnostics,
        resolution_diagnostics,
        recovery_nodes: output.recovery_nodes,
    })
}

/// Lowers parsed declarations into the inspectable pre-type-check HIR.
pub fn lower(parsed: ParsedProgram) -> LoweredProgram {
    lower_for_tooling(parsed).unwrap_or_else(|diagnostics| {
        panic!(
            "validated standard-library bodies must parse as ordinary SplitScript: {diagnostics:#?}"
        )
    })
}

/// Lowers a strictly parsed source while keeping compiler-owned augmentation
/// failures as data for resilient editor queries.
///
/// Batch compilation calls [`lower`] and retains the invariant panic because
/// generated standard-library source is compiler-owned. The editor database
/// uses this boundary to fall back to recovered semantics rather than letting
/// an internal augmentation failure terminate the language server.
pub(crate) fn lower_for_tooling(parsed: ParsedProgram) -> Result<LoweredProgram, Vec<Diagnostic>> {
    let syntax = &parsed.source.syntax;
    let syntax_diagnostics = parsed.syntax_diagnostics;
    let mut resolution_diagnostics = parsed.resolution_diagnostics;
    let augmented = if parsed.context.include_standard_library_bodies {
        stdlib::augment_program_with_library_bodies(
            parsed.source.document.source(),
            syntax,
            &parsed.context.standard_library(),
        )?
    } else {
        None
    };
    // Lowering and checking only read this tree. Database stage transitions
    // can share all injected library bodies instead of deep-cloning them.
    let compilation_syntax = augmented
        .map(std::sync::Arc::new)
        .unwrap_or_else(|| std::sync::Arc::clone(&parsed.source.syntax));
    let mut resolutions = resolution::ProgramResolutions::default();
    resolution_diagnostics.extend(resolution::resolve_program(
        &compilation_syntax,
        &parsed.context.standard_library(),
        &mut resolutions,
    ));
    let hir = hir::DeclarationIndex::lower(syntax);
    Ok(LoweredProgram {
        context: parsed.context,
        source: parsed.source,
        compilation_syntax,
        hir: std::sync::Arc::new(hir),
        resolutions: std::sync::Arc::new(resolutions),
        syntax_diagnostics,
        resolution_diagnostics,
    })
}

/// Resolves and type-checks a parsed program without invoking the Wasm backend.
pub fn check(lowered: impl Into<LoweredProgram>) -> Result<CheckedProgram, Vec<Diagnostic>> {
    check_impl(lowered.into(), None)
}

/// Keeps partial facts from a failed strict attempt for the editor database.
/// Success and failures before inference do not create a recovery product.
pub(crate) fn check_for_tooling(
    lowered: LoweredProgram,
) -> (
    Result<CheckedProgram, Vec<Diagnostic>>,
    Option<RecoveredCheck>,
) {
    let mut recovered = None;
    let result = check_impl(lowered, Some(&mut recovered));
    (result, recovered)
}

fn check_impl(
    lowered: LoweredProgram,
    recovery: Option<&mut Option<RecoveredCheck>>,
) -> Result<CheckedProgram, Vec<Diagnostic>> {
    let LoweredProgram {
        context,
        source,
        compilation_syntax,
        hir,
        resolutions,
        syntax_diagnostics,
        resolution_diagnostics,
    } = lowered;
    if syntax_diagnostics
        .iter()
        .chain(&resolution_diagnostics)
        .any(|diagnostic| diagnostic.severity == DiagnosticSeverity::Error)
    {
        let mut diagnostics = syntax_diagnostics;
        diagnostics.extend(resolution_diagnostics);
        return Err(diagnostics);
    }
    let typeck::RecoveringCheckOutput {
        mut output,
        diagnostics: inference_diagnostics,
    } = typeck::check_recovering_with_library(
        &compilation_syntax,
        &resolutions,
        context.standard_library(),
    );
    if !inference_diagnostics.is_empty() {
        if let Some(recovery) = recovery {
            let mut diagnostics = syntax_diagnostics.clone();
            diagnostics.extend(resolution_diagnostics);
            diagnostics.extend(inference_diagnostics.iter().cloned());
            *recovery = Some(RecoveredCheck::partial(PartialCheck {
                context,
                source,
                hir,
                semantics: output.semantics,
                diagnostics,
                enum_types: output.enum_types,
                effects: None,
            }));
        }
        let mut diagnostics = inference_diagnostics;
        diagnostics.extend(syntax_diagnostics);
        diagnostics.sort_by_key(|diagnostic| (diagnostic.span.start, diagnostic.span.end));
        return Err(diagnostics);
    }
    let typed_hir = hir::TypedProgram::build(
        hir,
        &compilation_syntax,
        &output.semantics,
        context.standard_library(),
        context.include_standard_library_bodies,
        hir::visible_expression_count(&source.syntax),
        source.syntax.functions.len(),
    );
    output
        .semantics
        .set_visible_expression_count(typed_hir.visible_expression_count());
    let validation = validation::validate(
        context.standard_library(),
        &compilation_syntax,
        &typed_hir,
        &output.semantics,
        &output.enum_types,
    );
    if validation
        .diagnostics
        .iter()
        .any(|diagnostic| diagnostic.severity == DiagnosticSeverity::Error)
    {
        // Recovery builds typed HIR and validation only without resolution
        // diagnostics. Reuse this result only when those invariants agree.
        if resolution_diagnostics.is_empty()
            && let Some(recovery) = recovery
        {
            let mut diagnostics = syntax_diagnostics.clone();
            diagnostics.extend(validation.diagnostics.iter().cloned());
            *recovery = Some(RecoveredCheck::partial(PartialCheck {
                context,
                source,
                hir: typed_hir.declarations_arc(),
                semantics: output.semantics,
                diagnostics,
                enum_types: output.enum_types,
                effects: Some(validation.effects),
            }));
        }
        let mut diagnostics = syntax_diagnostics;
        diagnostics.extend(validation.diagnostics);
        return Err(diagnostics);
    }
    let mut diagnostics = syntax_diagnostics;
    diagnostics.extend(validation.diagnostics);
    Ok(CheckedProgram {
        context,
        source,
        compilation_syntax,
        hir: typed_hir,
        diagnostics,
        semantics: output.semantics,
        capabilities: validation.capabilities,
        effects: validation.effects,
        scoped_globals: validation.scoped_globals,
        enum_types: output.enum_types,
        array_types: output.array_types,
        option_types: output.option_types,
        result_types: output.result_types,
        async_types: output.async_types,
        iterator_types: output.iterator_types,
        callable_types: output.callable_types,
        range_types: output.range_types,
        set_types: output.set_types,
        application_types: output.application_types,
    })
}

/// Runs error-tolerant type inference without invoking typed-HIR construction
/// or the WebAssembly backend.
pub fn check_recovering(lowered: impl Into<LoweredProgram>) -> RecoveredCheck {
    let LoweredProgram {
        context,
        source,
        compilation_syntax,
        hir,
        resolutions,
        syntax_diagnostics,
        resolution_diagnostics,
    } = lowered.into();
    let mut recovered = typeck::check_recovering_with_library(
        &compilation_syntax,
        &resolutions,
        context.standard_library(),
    );
    let validation = (!syntax_diagnostics
        .iter()
        .any(|diagnostic| diagnostic.severity == DiagnosticSeverity::Error)
        && resolution_diagnostics.is_empty()
        && recovered.diagnostics.is_empty())
    .then(|| {
        let typed_hir = hir::TypedProgram::build(
            hir.clone(),
            &compilation_syntax,
            &recovered.output.semantics,
            context.standard_library(),
            context.include_standard_library_bodies,
            hir::visible_expression_count(&source.syntax),
            source.syntax.functions.len(),
        );
        recovered
            .output
            .semantics
            .set_visible_expression_count(typed_hir.visible_expression_count());
        validation::validate(
            context.standard_library(),
            &compilation_syntax,
            &typed_hir,
            &recovered.output.semantics,
            &recovered.output.enum_types,
        )
    });
    let mut diagnostics = syntax_diagnostics;
    diagnostics.extend(resolution_diagnostics);
    diagnostics.extend(recovered.diagnostics);
    if let Some(validation) = &validation {
        diagnostics.extend(validation.diagnostics.iter().cloned());
    }
    RecoveredCheck::partial(PartialCheck {
        context,
        source,
        hir,
        semantics: recovered.output.semantics,
        diagnostics,
        enum_types: recovered.output.enum_types,
        effects: validation.map(|validation| validation.effects),
    })
}

/// Lowers a checked program into the inspectable Wasm-oriented control-flow
/// and storage plan consumed by the binary encoder.
pub fn lower_wasm(checked: &CheckedProgram) -> codegen::BackendProgram<'_> {
    lower_wasm_with_options(checked, CompilerOptions::default())
}

/// Lowers with explicit profile-sensitive compiler options.
pub fn lower_wasm_with_options(
    checked: &CheckedProgram,
    options: CompilerOptions,
) -> codegen::BackendProgram<'_> {
    let wasm_ir = wasm_ir::Program::lower(
        &checked.hir,
        &checked.semantics,
        &checked.effects,
        &checked.capabilities,
        &checked.scoped_globals,
        options.profile,
    );
    codegen::BackendProgram::new(checked, wasm_ir)
}

/// Generates WebAssembly from a successfully checked program.
pub fn codegen(checked: &CheckedProgram) -> Vec<u8> {
    codegen_with_options(checked, CompilerOptions::default())
}

/// Generates WebAssembly with explicit compiler options.
pub fn codegen_with_options(checked: &CheckedProgram, options: CompilerOptions) -> Vec<u8> {
    codegen::compile(lower_wasm_with_options(checked, options))
}

pub fn compile(source: &str) -> Result<Vec<u8>, Vec<Diagnostic>> {
    compile_with_options(source, CompilerOptions::default())
}

pub fn compile_with_context(
    context: CompilerContext,
    source: &str,
) -> Result<Vec<u8>, Vec<Diagnostic>> {
    compile_with_context_and_options(context, source, CompilerOptions::default())
}

/// Runs the complete compiler pipeline with explicit options.
pub fn compile_with_options(
    source: &str,
    options: CompilerOptions,
) -> Result<Vec<u8>, Vec<Diagnostic>> {
    compile_with_context_and_options(CompilerContext::default(), source, options)
}

pub fn compile_with_context_and_options(
    context: CompilerContext,
    source: &str,
    options: CompilerOptions,
) -> Result<Vec<u8>, Vec<Diagnostic>> {
    compile_with_context_and_options_diagnostics(context, source, options)
        .map(|(artifact, _diagnostics)| artifact)
}

/// Runs the complete compiler pipeline and preserves non-fatal diagnostics
/// alongside the generated artifact.
///
/// Convenience compilation functions intentionally return only the artifact
/// on success. Interactive and command-line hosts should use this entry point
/// so warnings are not lost merely because code generation succeeded.
pub fn compile_with_context_and_options_diagnostics(
    context: CompilerContext,
    source: &str,
    options: CompilerOptions,
) -> Result<(Vec<u8>, Vec<Diagnostic>), Vec<Diagnostic>> {
    compile_named_with_context_and_options_diagnostics(
        context,
        IN_MEMORY_SOURCE_NAME,
        source,
        options,
    )
}

/// Runs the complete compiler pipeline while retaining the source file's real
/// path or URI for debugger metadata.
pub fn compile_named_with_context_and_options_diagnostics(
    context: CompilerContext,
    source_name: impl Into<String>,
    source: &str,
    options: CompilerOptions,
) -> Result<(Vec<u8>, Vec<Diagnostic>), Vec<Diagnostic>> {
    match compile_named_with_context_and_options_cancellable(
        context,
        source_name,
        source,
        options,
        &CompilationCancellation::new(),
    ) {
        Ok(output) => Ok(output),
        Err(CompilationFailure::Diagnostics(diagnostics)) => Err(diagnostics),
        Err(CompilationFailure::Cancelled(_)) => {
            unreachable!("a private uncancelled token cannot be cancelled")
        }
    }
}

/// Runs the complete compiler pipeline with cooperative cancellation at stable
/// phase boundaries.
///
/// Cancellation is a host-control outcome, not a source diagnostic. Existing
/// one-shot entry points use a private uncancelled token and retain their
/// original result types.
pub fn compile_named_with_context_and_options_cancellable(
    context: CompilerContext,
    source_name: impl Into<String>,
    source: &str,
    options: CompilerOptions,
    cancellation: &CompilationCancellation,
) -> Result<(Vec<u8>, Vec<Diagnostic>), CompilationFailure> {
    let analyzed = analyze_named_with_context_and_options_cancellable(
        context,
        source_name,
        source,
        options,
        cancellation,
    )?;
    let lowered = lower_analyzed_compilation_cancellable(analyzed, cancellation)?;
    encode_lowered_compilation_cancellable(lowered, cancellation)
}

/// Owned result of strict source analysis, ready for Wasm lowering.
///
/// Its representation is intentionally opaque so hosts can retain it across a
/// scheduler yield without depending on compiler-internal semantic data.
#[derive(Debug)]
pub struct AnalyzedCompilation {
    checked: std::sync::Arc<CheckedProgram>,
    diagnostics: Vec<Diagnostic>,
    options: CompilerOptions,
}

/// Owned Wasm IR and semantic input, ready for binary encoding.
///
/// Keeping the checked program beside the IR avoids serializing either product
/// when an embedded host yields between compiler phases.
#[derive(Debug)]
pub struct LoweredCompilation {
    checked: std::sync::Arc<CheckedProgram>,
    diagnostics: Vec<Diagnostic>,
    wasm_ir: wasm_ir::Program,
}

/// Performs strict analysis and retains its owned semantic product.
pub fn analyze_named_with_context_and_options_cancellable(
    context: CompilerContext,
    source_name: impl Into<String>,
    source: &str,
    options: CompilerOptions,
    cancellation: &CompilationCancellation,
) -> Result<AnalyzedCompilation, CompilationFailure> {
    cancellation
        .checkpoint(CompilationPhase::Analysis)
        .map_err(CompilationFailure::Cancelled)?;
    let mut database =
        database::CompilerDatabase::with_context_and_source_name(context, source_name, source);
    database.set_warning_policy(options.warnings);
    let diagnostics = database.diagnostics().to_vec();
    if diagnostics
        .iter()
        .any(|diagnostic| diagnostic.severity == DiagnosticSeverity::Error)
    {
        return Err(CompilationFailure::Diagnostics(diagnostics));
    }
    let checked = database
        .check()
        .expect("an error-free diagnostic set has a strictly checked program");
    Ok(AnalyzedCompilation {
        checked,
        diagnostics,
        options,
    })
}

/// Lowers one analyzed product into owned Wasm IR.
pub fn lower_analyzed_compilation_cancellable(
    analyzed: AnalyzedCompilation,
    cancellation: &CompilationCancellation,
) -> Result<LoweredCompilation, CompilationFailure> {
    cancellation
        .checkpoint(CompilationPhase::WasmLowering)
        .map_err(CompilationFailure::Cancelled)?;
    let wasm_ir = wasm_ir::Program::lower(
        analyzed.checked.typed_hir(),
        analyzed.checked.semantics(),
        analyzed.checked.effects(),
        &analyzed.checked.capabilities,
        &analyzed.checked.scoped_globals,
        analyzed.options.profile,
    );
    Ok(LoweredCompilation {
        checked: analyzed.checked,
        diagnostics: analyzed.diagnostics,
        wasm_ir,
    })
}

/// Encodes owned Wasm IR and checks cancellation before publication.
pub fn encode_lowered_compilation_cancellable(
    lowered: LoweredCompilation,
    cancellation: &CompilationCancellation,
) -> Result<(Vec<u8>, Vec<Diagnostic>), CompilationFailure> {
    cancellation
        .checkpoint(CompilationPhase::WasmEncoding)
        .map_err(CompilationFailure::Cancelled)?;
    let backend = codegen::BackendProgram::new(&lowered.checked, lowered.wasm_ir);
    let artifact = codegen::compile(backend);
    cancellation
        .checkpoint(CompilationPhase::Publication)
        .map_err(CompilationFailure::Cancelled)?;
    Ok((artifact, lowered.diagnostics))
}

#[cfg(test)]
mod compiler_context_tests {
    use super::*;

    #[test]
    fn separately_owned_standard_library_flows_through_every_stage() {
        let bundled = stdlib::StandardLibrary::new();
        let isolated = stdlib::StandardLibrary::isolated_bundled();
        assert_ne!(bundled, isolated, "graphs must have independent identity");

        let context = CompilerContext::with_standard_library(isolated.clone());
        let parsed = parse_with_context(context, "state \"game.exe\" {}")
            .expect("the alternate validated graph should parse");
        assert_eq!(parsed.context().standard_library(), isolated);
        let checked = check(lower(parsed)).expect("the alternate graph should type-check");
        assert_eq!(checked.context().standard_library(), isolated);
        let lowered = lower_wasm(&checked);
        assert_eq!(lowered.standard_library(), &isolated);
        assert!(!codegen(&checked).is_empty());
    }
}

#[cfg(test)]
mod managed_collection_layout_tests;
