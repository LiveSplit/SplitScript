//! Compiler-owned semantic highlighting shared by the LSP and future editors.

use std::collections::{BTreeMap, HashMap, HashSet};

use crate::{
    ast::{
        Action, Expr, ExprKind, FunctionDecl, MatchPattern, Program, SettingFamilyDecl,
        SettingKind, SettingTextPart, Span, Stmt, TypeRef, ValueId, VariableDecl,
    },
    language::{LanguageCatalog, LanguageItemId, LanguageItemKind},
    lexer::{Lexeme, Token, TokenKind, TriviaKind},
    semantic::{ResolvedCall, ResolvedMember, ResolvedValue, SemanticModel},
    stdlib::StandardLibrary,
    syntax::SourceDocument,
    visit::{self, Visitor},
};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
#[repr(u32)]
pub enum SemanticTokenKind {
    Keyword,
    Type,
    Capability,
    Struct,
    Enum,
    EnumMember,
    Function,
    Method,
    Variable,
    Parameter,
    Property,
    Namespace,
    Constant,
    String,
    TemplateString,
    Number,
    Operator,
    Comment,
    Setting,
    SettingTitle,
    StateField,
    Lifecycle,
    Signature,
    Version,
    Debug,
}

impl SemanticTokenKind {
    pub const ALL: [Self; 25] = [
        Self::Keyword,
        Self::Type,
        Self::Capability,
        Self::Struct,
        Self::Enum,
        Self::EnumMember,
        Self::Function,
        Self::Method,
        Self::Variable,
        Self::Parameter,
        Self::Property,
        Self::Namespace,
        Self::Constant,
        Self::String,
        Self::TemplateString,
        Self::Number,
        Self::Operator,
        Self::Comment,
        Self::Setting,
        Self::SettingTitle,
        Self::StateField,
        Self::Lifecycle,
        Self::Signature,
        Self::Version,
        Self::Debug,
    ];

    pub const fn index(self) -> u32 {
        self as u32
    }

    pub const fn name(self) -> &'static str {
        match self {
            Self::Keyword => "keyword",
            Self::Type => "type",
            Self::Capability => "interface",
            Self::Struct => "struct",
            Self::Enum => "enum",
            Self::EnumMember => "enumMember",
            Self::Function => "function",
            Self::Method => "method",
            Self::Variable => "variable",
            Self::Parameter => "parameter",
            Self::Property => "property",
            Self::Namespace => "namespace",
            Self::Constant => "constant",
            Self::String => "string",
            Self::TemplateString => "templateString",
            Self::Number => "number",
            Self::Operator => "operator",
            Self::Comment => "comment",
            Self::Setting => "setting",
            Self::SettingTitle => "settingTitle",
            Self::StateField => "stateField",
            Self::Lifecycle => "lifecycle",
            Self::Signature => "signature",
            Self::Version => "version",
            Self::Debug => "debug",
        }
    }

    /// Resolves the stable editor-facing name of a semantic token kind.
    pub fn from_name(name: &str) -> Option<Self> {
        Self::ALL.into_iter().find(|kind| kind.name() == name)
    }
}

pub const MODIFIER_DECLARATION: u32 = 1 << 0;
pub const MODIFIER_READONLY: u32 = 1 << 1;
pub const MODIFIER_DEFAULT_LIBRARY: u32 = 1 << 2;
pub const MODIFIER_DEBUG: u32 = 1 << 3;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum LanguageTokenContext {
    Source,
    DocumentationFragment,
}

/// Returns the canonical semantic role of a compiler-owned language word.
///
/// Complete source only colors contextual words after the parser proves their
/// grammar role. Documentation signatures are intentionally incomplete
/// fragments, so their renderer opts into all catalogued and contextual words.
pub(crate) fn language_identifier_kind(
    name: &str,
    context: LanguageTokenContext,
) -> Option<SemanticTokenKind> {
    if matches!(name, "true" | "false") {
        return Some(SemanticTokenKind::Constant);
    }
    if matches!(name, "Some" | "None" | "Ok" | "Err" | "Item" | "End") {
        return Some(SemanticTokenKind::EnumMember);
    }

    let fragment = context == LanguageTokenContext::DocumentationFragment;
    if fragment
        && matches!(
            name,
            "in" | "where"
                | "attached"
                | "detached"
                | "key"
                | "choice"
                | "default"
                | "file"
                | "mime"
        )
    {
        return Some(SemanticTokenKind::Keyword);
    }

    let item = LanguageCatalog::new().item_by_name(name)?;
    match item.kind {
        LanguageItemKind::Keyword => Some(if name == "debug" {
            SemanticTokenKind::Debug
        } else {
            SemanticTokenKind::Keyword
        }),
        LanguageItemKind::Declaration => {
            let contextual = matches!(
                item.id,
                LanguageItemId::ManagedImage
                    | LanguageItemId::ManagedNamespace
                    | LanguageItemId::ManagedClass
            );
            (!contextual || fragment).then_some(SemanticTokenKind::Keyword)
        }
        LanguageItemKind::Action(_) => fragment.then_some(SemanticTokenKind::Lifecycle),
        LanguageItemKind::Syntax => match item.id {
            LanguageItemId::ManagedStaticField
            | LanguageItemId::ManagedMetadataNames
            | LanguageItemId::StateProviderAlternative
            | LanguageItemId::StatePointerField
                if fragment =>
            {
                Some(SemanticTokenKind::Keyword)
            }
            LanguageItemId::SignatureLiteral => Some(SemanticTokenKind::Signature),
            LanguageItemId::VersionLiteral => Some(SemanticTokenKind::Version),
            LanguageItemId::NativeStringDecoder | LanguageItemId::NativeUtf16LeDecoder
                if fragment =>
            {
                Some(SemanticTokenKind::Function)
            }
            _ => None,
        },
        LanguageItemKind::BuiltinType(_) => None,
        LanguageItemKind::SnapshotRoot => fragment.then_some(SemanticTokenKind::Variable),
    }
}

pub const SEMANTIC_TOKEN_MODIFIERS: [&str; 4] =
    ["declaration", "readonly", "defaultLibrary", "debug"];

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct SemanticHighlight {
    pub span: Span,
    pub kind: SemanticTokenKind,
    pub modifiers: u32,
}

#[derive(Debug, Clone, Default)]
pub struct SemanticHighlightIndex {
    highlights: Vec<SemanticHighlight>,
}

impl SemanticHighlightIndex {
    pub(crate) fn build(
        document: &SourceDocument,
        syntax: &Program,
        semantics: Option<&SemanticModel>,
        standard_library: StandardLibrary,
    ) -> Self {
        let mut value_kinds = ValueKindCollector::default();
        value_kinds.visit_program(syntax);
        let struct_names = syntax
            .structs
            .iter()
            .map(|structure| structure.name.as_str())
            .chain(
                syntax
                    .managed_class_declarations()
                    .into_iter()
                    .map(|class| class.name.as_str()),
            )
            .collect::<HashSet<_>>();
        let enum_names = syntax
            .enums
            .iter()
            .map(|enumeration| enumeration.name.as_str())
            .collect::<HashSet<_>>();
        let function_names = syntax
            .functions
            .iter()
            .map(|function| function.name.as_str())
            .collect::<HashSet<_>>();
        let mut collector = HighlightCollector {
            document,
            syntax,
            semantics,
            standard_library,
            entries: BTreeMap::new(),
            value_kinds: value_kinds.kinds,
            debug_ranges: value_kinds.debug_ranges,
        };
        collector.base_tokens(&struct_names, &enum_names, &function_names);
        collector.mark_managed_reference_types();
        collector.visit_program(syntax);
        collector.apply_debug_modifiers();
        Self {
            highlights: collector.entries.into_values().collect(),
        }
    }

    pub fn highlights(&self) -> &[SemanticHighlight] {
        &self.highlights
    }
}

#[derive(Default)]
struct ValueKindCollector {
    kinds: HashMap<ValueId, SemanticTokenKind>,
    debug_ranges: Vec<Span>,
}

impl<'ast> Visitor<'ast> for ValueKindCollector {
    fn visit_state_field(&mut self, field: &'ast crate::ast::StateField) {
        self.kinds.insert(field.id, SemanticTokenKind::StateField);
        visit::walk_state_field(self, field);
    }

    fn visit_setting(&mut self, setting: &'ast crate::ast::SettingDecl) {
        self.kinds.insert(setting.id, SemanticTokenKind::Setting);
    }

    fn visit_setting_family(&mut self, family: &'ast SettingFamilyDecl) {
        self.kinds
            .insert(family.binding_id, SemanticTokenKind::Variable);
    }

    fn visit_function(&mut self, function: &'ast FunctionDecl) {
        if function.debug_only {
            self.debug_ranges.push(function.span);
        }
        visit::walk_function(self, function);
    }

    fn visit_parameter(&mut self, parameter: &'ast crate::ast::Parameter) {
        parameter.binding.visit_bindings(&mut |binding| {
            self.kinds.insert(
                binding.id,
                if binding.name == "self" {
                    language_identifier_kind("self", LanguageTokenContext::Source)
                        .expect("`self` has a canonical language role")
                } else {
                    SemanticTokenKind::Parameter
                },
            );
        });
        visit::walk_parameter(self, parameter);
    }

    fn visit_variable(&mut self, variable: &'ast VariableDecl) {
        variable.binding.visit_bindings(&mut |binding| {
            self.kinds.insert(binding.id, SemanticTokenKind::Variable);
        });
        if variable.debug_only {
            self.debug_ranges.push(variable.span);
        }
        visit::walk_variable(self, variable);
    }

    fn visit_suspension_binding(&mut self, binding: &'ast crate::ast::SuspensionBinding) {
        self.kinds.insert(binding.id, SemanticTokenKind::Variable);
        if let Some(annotation) = &binding.annotation {
            self.visit_type_ref(annotation);
        }
    }

    fn visit_for_binding(&mut self, binding: &'ast crate::ast::ForBinding) {
        binding.binding.visit_bindings(&mut |leaf| {
            self.kinds.insert(leaf.id, SemanticTokenKind::Variable);
        });
    }

    fn visit_match_arm(&mut self, arm: &'ast crate::ast::MatchArm) {
        arm.pattern.visit_bindings(&mut |binding| {
            self.kinds.insert(binding.id, SemanticTokenKind::Variable);
        });
        visit::walk_match_arm(self, arm);
    }

    fn visit_expr(&mut self, expression: &'ast Expr) {
        if let ExprKind::Is { pattern, .. } = &expression.kind {
            pattern.kind.visit_bindings(&mut |binding| {
                self.kinds.insert(binding.id, SemanticTokenKind::Variable);
            });
        }
        visit::walk_expr(self, expression);
    }

    fn visit_stmt(&mut self, statement: &'ast Stmt) {
        if let Stmt::Debug { span, .. } = statement {
            self.debug_ranges.push(*span);
        }
        visit::walk_stmt(self, statement);
    }
}

struct HighlightCollector<'a> {
    document: &'a SourceDocument,
    syntax: &'a Program,
    semantics: Option<&'a SemanticModel>,
    standard_library: StandardLibrary,
    entries: BTreeMap<usize, SemanticHighlight>,
    value_kinds: HashMap<ValueId, SemanticTokenKind>,
    debug_ranges: Vec<Span>,
}

impl HighlightCollector<'_> {
    fn insert_language_token(&mut self, span: Span, name: &str, modifiers: u32) {
        let kind = language_identifier_kind(name, LanguageTokenContext::DocumentationFragment)
            .unwrap_or_else(|| panic!("language token `{name}` has no canonical highlight role"));
        self.insert(span, kind, modifiers);
    }

    fn mark_language_ident(&mut self, span: Span, name: &str, modifiers: u32) {
        let kind = language_identifier_kind(name, LanguageTokenContext::DocumentationFragment)
            .unwrap_or_else(|| panic!("language token `{name}` has no canonical highlight role"));
        self.mark_ident(span, name, kind, modifiers);
    }

    fn mark_managed_reference_types(&mut self) {
        for declaration in &self.syntax.managed_reference_types {
            for occurrence in &declaration.occurrences {
                self.insert(occurrence.class, SemanticTokenKind::Struct, 0);
                self.insert(occurrence.dot, SemanticTokenKind::Operator, 0);
                self.insert(occurrence.reference, SemanticTokenKind::Type, 0);
            }
        }
    }

    fn base_tokens(
        &mut self,
        struct_names: &HashSet<&str>,
        enum_names: &HashSet<&str>,
        function_names: &HashSet<&str>,
    ) {
        let mut previous_token: Option<&Token> = None;
        for lexeme in self.document.lexemes() {
            match lexeme {
                Lexeme::Trivia(trivia)
                    if matches!(
                        trivia.kind,
                        TriviaKind::LineComment | TriviaKind::BlockComment
                    ) =>
                {
                    self.insert(trivia.span, SemanticTokenKind::Comment, 0);
                }
                Lexeme::Trivia(_) => {}
                Lexeme::Token(token) => {
                    let kind = match &token.kind {
                        TokenKind::Ident(name)
                            if language_identifier_kind(name, LanguageTokenContext::Source)
                                .is_some() =>
                        {
                            language_identifier_kind(name, LanguageTokenContext::Source)
                        }
                        TokenKind::Ident(name)
                            if is_capability(&self.standard_library, name) =>
                        {
                            Some(SemanticTokenKind::Capability)
                        }
                        TokenKind::Ident(name) if is_builtin_type(&self.standard_library, name) => {
                            Some(SemanticTokenKind::Type)
                        }
                        TokenKind::Ident(name) if struct_names.contains(name.as_str()) => {
                            Some(SemanticTokenKind::Struct)
                        }
                        TokenKind::Ident(name) if enum_names.contains(name.as_str()) => {
                            Some(SemanticTokenKind::Enum)
                        }
                        TokenKind::Ident(name) if function_names.contains(name.as_str()) => {
                            Some(SemanticTokenKind::Function)
                        }
                        TokenKind::Ident(name) if is_namespace(&self.standard_library, name) => {
                            Some(SemanticTokenKind::Namespace)
                        }
                        TokenKind::String(_)
                            if previous_token.is_some_and(|previous| {
                                matches!(&previous.kind, TokenKind::Ident(name) if name == "sig")
                            }) => Some(SemanticTokenKind::Signature),
                        TokenKind::String(_)
                            if previous_token.is_some_and(|previous| {
                                matches!(&previous.kind, TokenKind::Ident(name) if name == "v")
                            }) => Some(SemanticTokenKind::Version),
                        TokenKind::Char(_) | TokenKind::String(_) => {
                            Some(SemanticTokenKind::String)
                        }
                        TokenKind::TemplateStart
                        | TokenKind::TemplateChunk(_)
                        | TokenKind::TemplateEnd => Some(SemanticTokenKind::TemplateString),
                        TokenKind::DocComment(_) => Some(SemanticTokenKind::Comment),
                        TokenKind::Int(_) | TokenKind::Float(_) => Some(SemanticTokenKind::Number),
                        kind if is_operator(kind) => Some(SemanticTokenKind::Operator),
                        _ => None,
                    };
                    if let Some(kind) = kind {
                        self.insert(token.span, kind, 0);
                    }
                    if token.kind != TokenKind::Eof {
                        previous_token = Some(token);
                    }
                }
            }
        }
    }

    fn insert(&mut self, span: Span, kind: SemanticTokenKind, modifiers: u32) {
        if span.start != span.end {
            self.entries.insert(
                span.start,
                SemanticHighlight {
                    span,
                    kind,
                    modifiers,
                },
            );
        }
    }

    fn mark_ident(&mut self, span: Span, name: &str, kind: SemanticTokenKind, modifiers: u32) {
        if let Some(token) = self.document.tokens().find(|token| {
            span.start <= token.span.start
                && token.span.end <= span.end
                && matches!(&token.kind, TokenKind::Ident(spelling) if spelling == name)
        }) {
            self.insert(token.span, kind, modifiers);
        }
    }

    fn mark_none_type(&mut self, span: Span, ty: TypeRef) {
        if !self.type_contains_none(ty) {
            return;
        }
        let spans = self
            .document
            .tokens()
            .filter(|token| {
                span.start <= token.span.start
                    && token.span.end <= span.end
                    && matches!(&token.kind, TokenKind::Ident(name) if name == "None")
            })
            .map(|token| token.span)
            .collect::<Vec<_>>();
        for span in spans {
            self.insert(span, SemanticTokenKind::Type, 0);
        }
    }

    fn type_contains_none(&self, ty: TypeRef) -> bool {
        match ty {
            TypeRef::Core(crate::ast::PrimitiveType::None) => true,
            TypeRef::Core(_) | TypeRef::Named(_) | TypeRef::ManagedReference(_) => false,
            TypeRef::Array(id) => self
                .syntax
                .array_types
                .iter()
                .find(|array| array.id == id)
                .is_some_and(|array| self.type_contains_none(array.element)),
            TypeRef::Option(id) => self
                .syntax
                .option_types
                .iter()
                .find(|option| option.id == id)
                .is_some_and(|option| self.type_contains_none(option.value)),
            TypeRef::Result(id) => self
                .syntax
                .result_types
                .iter()
                .find(|result| result.id == id)
                .is_some_and(|result| self.type_contains_none(result.value)),
            TypeRef::Async(id) => self
                .syntax
                .async_types
                .iter()
                .find(|future| future.id == id)
                .is_some_and(|future| self.type_contains_none(future.value)),
            TypeRef::Iterator(id) => self
                .syntax
                .iterator_types
                .iter()
                .find(|iterator| iterator.id == id)
                .is_some_and(|iterator| self.type_contains_none(iterator.item)),
            TypeRef::Callable(id) => self
                .syntax
                .callable_types
                .iter()
                .find(|callable| callable.id == id)
                .is_some_and(|callable| {
                    callable
                        .parameters
                        .iter()
                        .any(|parameter| self.type_contains_none(*parameter))
                        || self.type_contains_none(callable.result)
                }),
            TypeRef::Range(id) => self
                .syntax
                .range_types
                .iter()
                .find(|range| range.id == id)
                .is_some_and(|range| {
                    self.type_contains_none(range.lower) || self.type_contains_none(range.upper)
                }),
            TypeRef::Application(id) => self
                .syntax
                .type_applications
                .iter()
                .find(|application| application.id == id)
                .is_some_and(|application| {
                    application
                        .arguments
                        .iter()
                        .any(|argument| self.type_contains_none(*argument))
                }),
        }
    }

    fn mark_last_ident_before(
        &mut self,
        span: Span,
        end: usize,
        name: &str,
        kind: SemanticTokenKind,
        modifiers: u32,
    ) {
        if let Some(token) = self
            .document
            .tokens()
            .filter(|token| {
                span.start <= token.span.start
                    && token.span.end <= end
                    && matches!(&token.kind, TokenKind::Ident(spelling) if spelling == name)
            })
            .last()
        {
            self.insert(token.span, kind, modifiers);
        }
    }

    fn mark_first_string(&mut self, span: Span, kind: SemanticTokenKind) {
        if let Some(token) = self.document.tokens().find(|token| {
            span.start <= token.span.start
                && token.span.end <= span.end
                && matches!(token.kind, TokenKind::String(_))
        }) {
            self.insert(token.span, kind, MODIFIER_DECLARATION | MODIFIER_READONLY);
        }
    }

    fn mark_path(&mut self, expression: &Expr, names: &[String], call: bool) {
        let mut tokens = self.document.tokens().filter(|token| {
            expression.span.start <= token.span.start && token.span.end <= expression.span.end
        });
        let spans = names
            .iter()
            .filter_map(|name| {
                tokens.find_map(|token| {
                    matches!(&token.kind, TokenKind::Ident(spelling) if spelling == name)
                        .then_some(token.span)
                })
            })
            .collect::<Vec<_>>();
        if spans.is_empty() {
            return;
        }

        // Semantic highlighting walks the parser's recovery tree so it remains
        // available while a document is being edited. Enum resolution happens
        // on a cloned tree, where `Mode.Active` is rewritten from a generic path
        // into an enum expression. The expression ID bridges those two trees:
        // prefer the resolved enum identity over the path-shaped syntax instead
        // of styling the variant as an ordinary property.
        if self
            .semantics
            .and_then(|model| model.enum_variant(expression.id))
            .is_some()
            && spans.len() >= 2
        {
            if spans.len() > 2 {
                self.insert(spans[0], SemanticTokenKind::Type, 0);
                for span in &spans[1..spans.len() - 1] {
                    self.insert(*span, SemanticTokenKind::Enum, 0);
                }
            } else {
                self.insert(spans[0], SemanticTokenKind::Enum, 0);
            }
            self.insert(*spans.last().unwrap(), SemanticTokenKind::EnumMember, 0);
            return;
        }

        let resolution = self
            .semantics
            .and_then(|semantics| semantics.call(expression.id));
        let call_suffix_width = usize::from(call);

        let resolved_value = self.semantics.and_then(|model| {
            model.value(expression.id).or_else(|| {
                model.call(expression.id).and_then(|call| match call {
                    ResolvedCall::StandardLibrary { receiver, .. } => receiver
                        .as_ref()
                        .and_then(|receiver| receiver.path().map(|(root, _)| root)),
                    ResolvedCall::UserMethod { receiver, .. } => {
                        receiver.path().map(|(root, _)| root)
                    }
                    ResolvedCall::ManagedSnapshot { receiver, .. } => {
                        receiver.path().map(|(root, _)| root)
                    }
                    ResolvedCall::ManagedComponent { receiver, .. } => {
                        receiver.path().map(|(root, _)| root)
                    }
                    ResolvedCall::UserFunction { .. }
                    | ResolvedCall::ManagedInstances { .. }
                    | ResolvedCall::OptionSome { .. }
                    | ResolvedCall::IteratorItem { .. }
                    | ResolvedCall::ResultSuccess { .. }
                    | ResolvedCall::ResultError { .. } => None,
                })
            })
        });
        if matches!(resolution, Some(ResolvedCall::ManagedInstances { .. }))
            && let Some(owner) = spans.first()
        {
            self.insert(*owner, SemanticTokenKind::Type, 0);
        }
        let current_state_path = matches!(resolved_value, Some(ResolvedValue::CurrentState(_)));
        if let Some(value) = resolved_value {
            match value {
                ResolvedValue::StandardLibraryConstant(item) => {
                    let constant_segment = self
                        .standard_library
                        .item_path(self.standard_library.item(item))
                        .map_or(0, |path| path.len().saturating_sub(1));
                    if let Some(owner) = spans.first() {
                        self.insert(*owner, SemanticTokenKind::Type, MODIFIER_DEFAULT_LIBRARY);
                    }
                    if let Some(constant) = spans.get(constant_segment) {
                        self.insert(
                            *constant,
                            SemanticTokenKind::Constant,
                            MODIFIER_READONLY | MODIFIER_DEFAULT_LIBRARY,
                        );
                    }
                }
                ResolvedValue::ProviderValue(_) | ResolvedValue::ProviderContext { .. } => {
                    self.insert(spans[0], SemanticTokenKind::Variable, MODIFIER_READONLY);
                }
                ResolvedValue::ManagedStatic { .. } => {
                    self.insert(spans[0], SemanticTokenKind::Type, 0);
                    if let Some(field) = spans.get(1) {
                        self.insert(*field, SemanticTokenKind::Property, MODIFIER_READONLY);
                    }
                }
                ResolvedValue::Variable(id) => {
                    let readonly = self.syntax.state.as_ref().is_some_and(|state| {
                        state.provider_value == Some(id)
                            || state.all_fields().any(|field| {
                                field
                                    .transform
                                    .as_ref()
                                    .is_some_and(|transform| transform.value == id)
                            })
                    });
                    self.insert(
                        spans[0],
                        self.value_kind(id),
                        if readonly { MODIFIER_READONLY } else { 0 },
                    );
                }
                ResolvedValue::CurrentSnapshot | ResolvedValue::OldSnapshot => {
                    self.insert(spans[0], SemanticTokenKind::Variable, MODIFIER_READONLY);
                }
                ResolvedValue::SettingsView | ResolvedValue::OldSettingsView => {
                    self.insert(spans[0], SemanticTokenKind::Variable, MODIFIER_READONLY);
                }
                ResolvedValue::CurrentState(_) => {
                    self.insert(spans[0], SemanticTokenKind::Variable, MODIFIER_READONLY);
                    if let Some(span) = spans.get(1) {
                        self.insert(*span, SemanticTokenKind::StateField, 0);
                    }
                }
                ResolvedValue::OldState(_) => {
                    self.insert(spans[0], SemanticTokenKind::Variable, MODIFIER_READONLY);
                    if let Some(span) = spans.get(1) {
                        self.insert(*span, SemanticTokenKind::StateField, MODIFIER_READONLY);
                    }
                }
                ResolvedValue::StateCandidate(id) => {
                    self.insert(spans[0], self.value_kind(id), MODIFIER_READONLY);
                }
                ResolvedValue::Setting(id) | ResolvedValue::OldSetting(id) => {
                    self.insert(spans[0], SemanticTokenKind::Variable, MODIFIER_READONLY);
                    if let Some(span) = spans.get(1) {
                        self.insert(*span, self.value_kind(id), MODIFIER_READONLY);
                    }
                }
            }
        } else if matches!(names.first().map(String::as_str), Some("current" | "old")) {
            self.insert(spans[0], SemanticTokenKind::Variable, MODIFIER_READONLY);
            if let Some(span) = spans.get(1) {
                self.insert(*span, SemanticTokenKind::StateField, MODIFIER_READONLY);
            }
        } else if matches!(
            names.first().map(String::as_str),
            Some("settings" | "oldSettings")
        ) {
            self.insert(spans[0], SemanticTokenKind::Variable, MODIFIER_READONLY);
            if let Some(span) = spans.get(1) {
                self.insert(*span, SemanticTokenKind::Setting, MODIFIER_READONLY);
            }
        }

        // Lexical highlighting cannot tell a builtin type spelling such as
        // `address` from a field with the same name. Resolved member identities
        // take precedence over those source-wide name heuristics. Members are
        // a suffix because roots such as `current.field` can consume more than
        // one written path segment before ordinary struct/standard fields.
        let resolved_receiver = resolution.and_then(|call| match call {
            ResolvedCall::UserMethod { receiver, .. }
            | ResolvedCall::ManagedSnapshot { receiver, .. }
            | ResolvedCall::ManagedComponent { receiver, .. } => Some(receiver),
            ResolvedCall::StandardLibrary {
                receiver: Some(receiver),
                ..
            } => Some(receiver),
            _ => None,
        });
        let expression_receiver =
            resolved_receiver.is_some_and(|receiver| receiver.expression().is_some());
        if let Some(members) = self.semantics.and_then(|model| {
            model
                .path_members(expression.id)
                .or_else(|| resolved_receiver.map(|receiver| receiver.members()))
        }) {
            let start = if call && expression_receiver {
                0
            } else {
                spans
                    .len()
                    .saturating_sub(call_suffix_width + members.len())
            };
            for (span, member) in spans.iter().skip(start).take(members.len()).zip(members) {
                let (kind, modifiers) = match member {
                    ResolvedMember::StateField(_) => (
                        SemanticTokenKind::StateField,
                        if current_state_path {
                            0
                        } else {
                            MODIFIER_READONLY
                        },
                    ),
                    ResolvedMember::SettingField(_) => {
                        (SemanticTokenKind::Setting, MODIFIER_READONLY)
                    }
                    ResolvedMember::StandardField(_) => (
                        SemanticTokenKind::Property,
                        MODIFIER_READONLY | MODIFIER_DEFAULT_LIBRARY,
                    ),
                    ResolvedMember::StructField(_) => {
                        (SemanticTokenKind::Property, MODIFIER_READONLY)
                    }
                    ResolvedMember::ManagedField(_) => {
                        (SemanticTokenKind::Property, MODIFIER_READONLY)
                    }
                };
                self.insert(*span, kind, modifiers);
            }
        }

        let unresolved_member_start = usize::from(!expression_receiver);
        let unresolved_member_end = spans.len().saturating_sub(call_suffix_width);
        for span in spans
            .iter()
            .take(unresolved_member_end)
            .skip(unresolved_member_start)
        {
            self.entries.entry(span.start).or_insert(SemanticHighlight {
                span: *span,
                kind: SemanticTokenKind::Property,
                modifiers: MODIFIER_READONLY,
            });
        }
        if call {
            let target = *spans.last().unwrap();
            let kind = match resolution {
                Some(
                    ResolvedCall::OptionSome { .. }
                    | ResolvedCall::IteratorItem { .. }
                    | ResolvedCall::ResultSuccess { .. }
                    | ResolvedCall::ResultError { .. },
                ) => SemanticTokenKind::EnumMember,
                Some(ResolvedCall::UserMethod { .. }) => SemanticTokenKind::Method,
                _ if spans.len() == 1 => SemanticTokenKind::Function,
                _ => SemanticTokenKind::Method,
            };
            let modifiers = if matches!(resolution, Some(ResolvedCall::StandardLibrary { .. })) {
                MODIFIER_DEFAULT_LIBRARY
            } else {
                0
            };
            self.insert(target, kind, modifiers);
        }
    }

    fn value_kind(&self, id: ValueId) -> SemanticTokenKind {
        self.value_kinds
            .get(&id)
            .copied()
            .unwrap_or(SemanticTokenKind::Variable)
    }

    fn mark_pattern(&mut self, pattern: &MatchPattern, span: Span, binding_modifiers: u32) {
        match pattern {
            MatchPattern::Struct {
                name_span, fields, ..
            } => {
                if let Some(name_span) = name_span {
                    self.insert(*name_span, SemanticTokenKind::Struct, 0);
                }
                for field in fields {
                    self.mark_pattern(&field.pattern.kind, field.pattern.span, binding_modifiers);
                    self.insert(
                        field.name_span,
                        SemanticTokenKind::Property,
                        MODIFIER_READONLY,
                    );
                }
            }
            MatchPattern::Enum {
                enumeration,
                variant,
                payload,
            } => {
                let mut segments = enumeration.name.split('.').peekable();
                while let Some(segment) = segments.next() {
                    self.mark_ident(
                        span,
                        segment,
                        if segments.peek().is_some() {
                            SemanticTokenKind::Type
                        } else {
                            SemanticTokenKind::Enum
                        },
                        0,
                    );
                }
                self.mark_ident(span, variant, SemanticTokenKind::EnumMember, 0);
                if let Some(payload) = payload {
                    self.mark_pattern(&payload.kind, payload.span, binding_modifiers);
                }
            }
            MatchPattern::Binding(binding) => self.mark_ident(
                span,
                &binding.name,
                self.value_kinds
                    .get(&binding.id)
                    .copied()
                    .unwrap_or(SemanticTokenKind::Variable),
                MODIFIER_DECLARATION | binding_modifiers,
            ),
            MatchPattern::OptionSome(payload)
            | MatchPattern::IteratorItem(payload)
            | MatchPattern::ResultSuccess(payload)
            | MatchPattern::ResultError(payload) => {
                self.mark_pattern(&payload.kind, payload.span, binding_modifiers);
            }
            MatchPattern::Array(array) => {
                for element in array.elements() {
                    self.mark_pattern(&element.kind, element.span, binding_modifiers);
                }
            }
            MatchPattern::Alternation(elements) => {
                for element in elements {
                    self.mark_pattern(&element.kind, element.span, binding_modifiers);
                }
            }
            _ => {}
        }
    }

    fn apply_debug_modifiers(&mut self) {
        for highlight in self.entries.values_mut() {
            if self
                .debug_ranges
                .iter()
                .any(|range| range.start <= highlight.span.start && highlight.span.end <= range.end)
            {
                highlight.modifiers |= MODIFIER_DEBUG;
            }
        }
    }
}

impl<'ast> Visitor<'ast> for HighlightCollector<'_> {
    fn visit_program(&mut self, program: &'ast Program) {
        if let Some(policy) = program.tick_rate {
            self.insert_language_token(policy.keyword_span, "tickRate", 0);
            if let Some(rate) = policy.attached {
                self.insert_language_token(rate.keyword_span, "attached", 0);
            }
            if let Some(rate) = policy.detached {
                self.insert_language_token(rate.keyword_span, "detached", 0);
            }
        }
        if let Some(state) = &program.state {
            for alternative in &state.provider_alternatives {
                self.insert_language_token(alternative.keyword_span, "provider", 0);
            }
        }
        for provider in program.state.iter().flat_map(|state| {
            state.provider.iter().chain(
                state
                    .provider_alternatives
                    .iter()
                    .map(|alternative| &alternative.provider),
            )
        }) {
            if self
                .standard_library
                .state_provider_by_name(&provider.name)
                .is_none()
            {
                continue;
            }
            self.mark_ident(
                provider.span,
                &provider.name,
                SemanticTokenKind::Type,
                MODIFIER_READONLY | MODIFIER_DEFAULT_LIBRARY,
            );
            if let Some(selector) = &provider.selector
                && self
                    .standard_library
                    .state_provider_by_name(&provider.name)
                    .is_some_and(|provider| {
                        provider
                            .selectors
                            .iter()
                            .any(|candidate| candidate.name == selector.name)
                    })
            {
                self.mark_ident(
                    selector.name_span,
                    &selector.name,
                    SemanticTokenKind::Method,
                    MODIFIER_READONLY | MODIFIER_DEFAULT_LIBRARY,
                );
            }
        }
        visit::walk_program(self, program);
    }

    fn visit_state_field(&mut self, field: &'ast crate::ast::StateField) {
        self.mark_ident(
            field.span,
            &field.name,
            SemanticTokenKind::StateField,
            MODIFIER_DECLARATION | MODIFIER_READONLY,
        );
        if let Some(annotation) = field.annotation {
            let end = match &field.source {
                crate::ast::StateSource::Expression(value) => value.span.start,
                crate::ast::StateSource::Pointer(_) => field.span.end,
            };
            self.mark_none_type(
                Span {
                    start: field.span.start,
                    end,
                },
                annotation,
            );
        }
        if let crate::ast::StateSource::Pointer(path) = &field.source {
            if let Some(span) = path.at_span {
                self.insert_language_token(span, "at", 0);
            }
            if let Some(decoder) = path.decoder {
                let (span, name) = match decoder {
                    crate::ast::StateMemoryDecoder::Utf8 { span, .. } => (span, "utf8"),
                    crate::ast::StateMemoryDecoder::Utf16Le { span, .. } => (span, "utf16le"),
                };
                self.mark_language_ident(span, name, MODIFIER_READONLY | MODIFIER_DEFAULT_LIBRARY);
            }
        }
        visit::walk_state_field(self, field);
    }

    fn visit_setting(&mut self, setting: &'ast crate::ast::SettingDecl) {
        if let Some(key) = &setting.external_key {
            self.insert_language_token(key.keyword_span, "key", 0);
        }
        if matches!(setting.kind, SettingKind::Title { .. }) {
            self.mark_first_string(setting.span, SemanticTokenKind::SettingTitle);
        } else {
            self.mark_ident(
                setting.span,
                &setting.name,
                SemanticTokenKind::Setting,
                MODIFIER_DECLARATION,
            );
            match &setting.kind {
                SettingKind::Choice {
                    keyword_span,
                    options,
                    ..
                } => {
                    self.insert_language_token(*keyword_span, "choice", 0);
                    for option in options {
                        if let Some(default_span) = option.default_span {
                            self.insert_language_token(default_span, "default", 0);
                        }
                        let Some(variant) = self
                            .semantics
                            .and_then(|semantics| semantics.setting_choice_option(option.id))
                        else {
                            continue;
                        };
                        let Some(enumeration) =
                            self.syntax.enum_declarations().find(|enumeration| {
                                enumeration
                                    .variants
                                    .iter()
                                    .any(|candidate| candidate.id == variant)
                            })
                        else {
                            continue;
                        };
                        self.mark_ident(option.span, &enumeration.name, SemanticTokenKind::Enum, 0);
                        self.mark_ident(
                            option.span,
                            &option.variant,
                            SemanticTokenKind::EnumMember,
                            0,
                        );
                    }
                }
                SettingKind::File {
                    keyword_span,
                    filters,
                } => {
                    self.insert_language_token(*keyword_span, "file", 0);
                    for filter in filters {
                        if let crate::ast::SettingFileFilter::Mime { keyword_span, .. } = filter {
                            self.insert_language_token(*keyword_span, "mime", 0);
                        }
                    }
                }
                SettingKind::Bool { .. } | SettingKind::Text { .. } | SettingKind::Title { .. } => {
                }
            }
        }
    }

    fn visit_setting_family(&mut self, family: &'ast SettingFamilyDecl) {
        self.insert_language_token(family.in_span, "in", 0);
        self.insert(
            family.binding_span,
            SemanticTokenKind::Variable,
            MODIFIER_DECLARATION | MODIFIER_READONLY,
        );
        if let Some(span) = family.key_keyword_span {
            self.insert_language_token(span, "key", 0);
        }
        for pattern in family.key.iter().chain(std::iter::once(&family.label)) {
            for part in &pattern.parts {
                if let SettingTextPart::Binding { span } = part {
                    self.insert(*span, SemanticTokenKind::Variable, MODIFIER_READONLY);
                }
            }
        }
    }

    fn visit_struct(&mut self, structure: &'ast crate::ast::StructDecl) {
        self.mark_ident(
            structure.span,
            &structure.name,
            SemanticTokenKind::Struct,
            MODIFIER_DECLARATION,
        );
        for field in &structure.fields {
            self.mark_ident(
                field.span,
                &field.name,
                SemanticTokenKind::Property,
                MODIFIER_DECLARATION | MODIFIER_READONLY,
            );
            self.mark_none_type(field.span, field.ty);
        }
        visit::walk_struct(self, structure);
    }

    fn visit_enum(&mut self, enumeration: &'ast crate::ast::EnumDecl) {
        self.mark_ident(
            enumeration.span,
            &enumeration.name,
            SemanticTokenKind::Enum,
            MODIFIER_DECLARATION,
        );
        for variant in &enumeration.variants {
            self.mark_ident(
                variant.span,
                &variant.name,
                SemanticTokenKind::EnumMember,
                MODIFIER_DECLARATION | MODIFIER_READONLY,
            );
            if let Some(payload) = variant.payload {
                self.mark_none_type(variant.span, payload);
            }
        }
        visit::walk_enum(self, enumeration);
    }

    fn visit_managed_image(&mut self, image: &'ast crate::ast::ManagedImageDecl) {
        self.insert_language_token(image.keyword_span, "image", 0);
        visit::walk_managed_image(self, image);
    }

    fn visit_managed_namespace(&mut self, namespace: &'ast crate::ast::ManagedNamespaceDecl) {
        self.insert_language_token(namespace.keyword_span, "namespace", 0);
        self.insert(
            namespace.name_span,
            SemanticTokenKind::Namespace,
            MODIFIER_DECLARATION,
        );
        visit::walk_managed_namespace(self, namespace);
    }

    fn visit_managed_class(&mut self, class: &'ast crate::ast::ManagedClassDecl) {
        self.insert_language_token(class.keyword_span, "class", 0);
        self.insert(
            class.name_span,
            SemanticTokenKind::Struct,
            MODIFIER_DECLARATION,
        );
        if let Some(from) = class.metadata_names.keyword_span {
            self.insert_language_token(from, "from", 0);
        }
        visit::walk_managed_class(self, class);
    }

    fn visit_managed_field(&mut self, field: &'ast crate::ast::ManagedFieldDecl) {
        if let Some(span) = field.static_span {
            self.insert_language_token(span, "static", 0);
        }
        if let Some(span) = field.metadata_names.keyword_span {
            self.insert_language_token(span, "from", 0);
        }
        self.insert(
            field.name_span,
            SemanticTokenKind::Property,
            MODIFIER_DECLARATION | MODIFIER_READONLY,
        );
        self.mark_none_type(field.type_span, field.ty);
        self.visit_type_ref(&field.ty);
    }

    fn visit_function(&mut self, function: &'ast FunctionDecl) {
        self.mark_last_ident_before(
            function.span,
            function.body.span.start,
            &function.name,
            if function.method_of.is_some() {
                SemanticTokenKind::Method
            } else {
                SemanticTokenKind::Function
            },
            MODIFIER_DECLARATION,
        );
        if let (Some(annotation), Some(span)) =
            (function.return_annotation, function.return_annotation_span)
        {
            self.mark_none_type(span, annotation);
        }
        visit::walk_function(self, function);
    }

    fn visit_parameter(&mut self, parameter: &'ast crate::ast::Parameter) {
        self.mark_pattern(
            &parameter.binding.pattern.kind,
            parameter.binding.pattern.span,
            0,
        );
        if let Some(annotation) = parameter.annotation {
            self.mark_none_type(parameter.span, annotation);
        }
        visit::walk_parameter(self, parameter);
    }

    fn visit_action(&mut self, action: &'ast Action) {
        self.mark_language_ident(action.span, action.kind.name(), MODIFIER_DECLARATION);
        self.visit_block(&action.body);
    }

    fn visit_variable(&mut self, variable: &'ast VariableDecl) {
        let initializer_start = variable
            .value
            .as_ref()
            .map_or(variable.span.end, |value| value.span.start);
        self.mark_pattern(
            &variable.binding.pattern.kind,
            variable.binding.pattern.span,
            0,
        );
        if let Some(annotation) = variable.annotation {
            self.mark_none_type(
                Span {
                    start: variable.name_span.end,
                    end: initializer_start,
                },
                annotation,
            );
        }
        visit::walk_variable(self, variable);
    }

    fn visit_suspension_binding(&mut self, binding: &'ast crate::ast::SuspensionBinding) {
        self.mark_ident(
            binding.span,
            &binding.name,
            SemanticTokenKind::Variable,
            MODIFIER_DECLARATION,
        );
        if let Some(annotation) = &binding.annotation {
            self.mark_none_type(binding.span, *annotation);
            self.visit_type_ref(annotation);
        }
    }

    fn visit_for_binding(&mut self, binding: &'ast crate::ast::ForBinding) {
        self.mark_pattern(
            &binding.binding.pattern.kind,
            binding.binding.pattern.span,
            MODIFIER_READONLY,
        );
    }

    fn visit_stmt(&mut self, statement: &'ast Stmt) {
        if let Stmt::For { in_span, .. } = statement {
            self.insert_language_token(*in_span, "in", 0);
        }
        visit::walk_stmt(self, statement);
    }

    fn visit_match_arm(&mut self, arm: &'ast crate::ast::MatchArm) {
        self.mark_pattern(&arm.pattern, arm.span, 0);
        visit::walk_match_arm(self, arm);
    }

    fn visit_expr(&mut self, expression: &'ast Expr) {
        match &expression.kind {
            ExprKind::Inspect { keyword_span, .. } => {
                self.insert_language_token(*keyword_span, "inspect", 0);
            }
            ExprKind::Is {
                pattern,
                keyword_span,
                ..
            } => {
                self.insert_language_token(*keyword_span, "is", 0);
                self.mark_pattern(&pattern.kind, pattern.span, 0);
            }
            ExprKind::Path(names) => self.mark_path(expression, names, false),
            ExprKind::Member { name_span, .. } => {
                self.insert(*name_span, SemanticTokenKind::Property, MODIFIER_READONLY)
            }
            ExprKind::Call { callee, .. } => self.mark_path(expression, callee, true),
            ExprKind::Struct {
                name: _,
                name_span,
                fields,
            } => {
                // Visit values first. A shorthand field's synthesized value
                // expression has the same span as its field name, so the
                // structural field role must deliberately win semantic-token
                // precedence over the supplying local variable.
                for field in fields {
                    self.visit_expr(&field.value);
                }
                self.insert(*name_span, SemanticTokenKind::Struct, 0);
                for field in fields {
                    self.insert(
                        field.name_span,
                        SemanticTokenKind::Property,
                        MODIFIER_READONLY,
                    );
                }
                return;
            }
            ExprKind::Closure {
                return_annotation: Some(result),
                return_annotation_span: Some(span),
                ..
            } => self.mark_none_type(*span, *result),
            _ => {}
        }
        visit::walk_expr(self, expression);
    }
}

fn is_builtin_type(standard_library: &StandardLibrary, name: &str) -> bool {
    TypeRef::parse(name).is_some()
        || standard_library.type_by_name(name).is_some()
        || standard_library
            .named_type_constructor_by_name(name)
            .is_some()
}

fn is_capability(standard_library: &StandardLibrary, name: &str) -> bool {
    standard_library
        .capabilities()
        .iter()
        .any(|capability| capability.name == name)
}

fn is_namespace(standard_library: &StandardLibrary, name: &str) -> bool {
    standard_library.namespace_by_name(name).is_some()
}

fn is_operator(kind: &TokenKind) -> bool {
    matches!(
        kind,
        TokenKind::FatArrow
            | TokenKind::Assign
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
            | TokenKind::Bang
            | TokenKind::Question
            | TokenKind::Tilde
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
            | TokenKind::DotDot
            | TokenKind::DotDotLt
            | TokenKind::DotDotEq
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::database::CompilerDatabase;

    #[test]
    fn every_documented_keyword_is_semantically_highlighted() {
        for item in crate::language::LanguageCatalog::new().items() {
            if item.kind == crate::language::LanguageItemKind::Keyword {
                assert!(
                    language_identifier_kind(item.name, LanguageTokenContext::Source).is_some(),
                    "documented keyword `{}` is missing from semantic highlighting",
                    item.name
                );
            }
        }
    }

    #[test]
    fn loop_and_wrapper_variants_have_consistent_semantic_highlighting() {
        let source = r#"fn wrappers(value: i32?, step: IteratorStep<i32>) {
    let present = Some(5)
    let absent: i32? = None
    let success: i32! = Ok(5)
    let failure: i32! = Err("failed")
    let item: IteratorStep<i32> = Item(5)
    let end: IteratorStep<i32> = End
    loop {
        match value {
            Some(inner) => break,
            None => break,
        }
        match step {
            Item(inner) => break,
            End => break,
        }
    }

}"#;
        let mut database = CompilerDatabase::new(source);
        let highlights = database.semantic_highlights().unwrap();
        assert!(contains(
            source,
            &highlights,
            "loop",
            SemanticTokenKind::Keyword,
            0
        ));
        for spelling in ["Some", "None", "Ok", "Err", "Item", "End"] {
            for (offset, _) in source.match_indices(spelling) {
                assert_eq!(
                    kind_at(&highlights, offset),
                    Some(SemanticTokenKind::EnumMember),
                    "`{spelling}` at {offset} was not highlighted as an enum variant"
                );
            }
        }
        assert!(!highlights.highlights().iter().any(|highlight| {
            &source[highlight.span.start..highlight.span.end] == "None"
                && highlight.kind == SemanticTokenKind::Constant
        }));
    }

    #[test]
    fn implicit_method_self_is_highlighted_as_a_keyword() {
        let source = r#"
struct Position {
    x: i32,
}

state "game.exe" {}

fn Position.value() -> i32 {
    return self.x
}
"#;
        let mut database = CompilerDatabase::new(source);
        let highlights = database.semantic_highlights().unwrap();
        assert_eq!(
            kind_at(&highlights, source.find("self.x").unwrap()),
            Some(SemanticTokenKind::Keyword)
        );
    }

    #[test]
    fn destructured_binding_leaves_keep_their_binding_site_roles() {
        let source = r#"
struct Point { x: i32, y: i32 }
state "game.exe" {}
fn observe(Point { x: parameterX, y: parameterY }: Point) {
    let Point { x: localX, y: localY } = Point { x: parameterX, y: parameterY }
    for Point { x: itemX, y: itemY } in [Point { x: localX, y: localY }] {}
}
"#;
        let mut database = CompilerDatabase::new(source);
        let highlights = database.semantic_highlights().unwrap();
        for name in ["parameterX", "parameterY"] {
            let offset = source.find(name).unwrap();
            assert_eq!(
                kind_at(&highlights, offset),
                Some(SemanticTokenKind::Parameter)
            );
        }
        for name in ["localX", "localY", "itemX", "itemY"] {
            let offset = source.find(name).unwrap();
            assert_eq!(
                kind_at(&highlights, offset),
                Some(SemanticTokenKind::Variable)
            );
        }
    }

    #[test]
    fn associated_constants_are_highlighted_as_readonly_library_values() {
        let source = r#"
state "game.exe" {}
setup {
    let value = f32.NaN
}
"#;
        let mut database = CompilerDatabase::new(source);
        let highlights = database.semantic_highlights().unwrap();
        assert!(contains(
            source,
            &highlights,
            "f32",
            SemanticTokenKind::Type,
            MODIFIER_DEFAULT_LIBRARY
        ));
        assert!(contains(
            source,
            &highlights,
            "NaN",
            SemanticTokenKind::Constant,
            MODIFIER_READONLY | MODIFIER_DEFAULT_LIBRARY
        ));
    }

    #[test]
    fn is_keywords_and_pattern_bindings_have_canonical_semantic_roles() {
        let source = r#"state "game.exe" {}
fn observe(value: u32?) {
    if value is Some(number) && number > 0 {
        print(number)
    }
}"#;
        let mut database = CompilerDatabase::new(source);
        let highlights = database.semantic_highlights().unwrap();
        assert!(contains(
            source,
            &highlights,
            "is",
            SemanticTokenKind::Keyword,
            0,
        ));
        let declaration = source.find("number)").unwrap();
        let declaration_highlight = highlights
            .highlights()
            .iter()
            .find(|highlight| highlight.span.start == declaration)
            .expect("pattern declaration is highlighted");
        assert_eq!(declaration_highlight.kind, SemanticTokenKind::Variable);
        assert_eq!(
            declaration_highlight.modifiers & MODIFIER_DECLARATION,
            MODIFIER_DECLARATION
        );
        for usage in [
            source.find("number >").unwrap(),
            source.rfind("number)").unwrap(),
        ] {
            assert_eq!(
                kind_at(&highlights, usage),
                Some(SemanticTokenKind::Variable)
            );
        }
    }

    fn contains(
        source: &str,
        index: &SemanticHighlightIndex,
        spelling: &str,
        kind: SemanticTokenKind,
        modifiers: u32,
    ) -> bool {
        index.highlights().iter().any(|highlight| {
            &source[highlight.span.start..highlight.span.end] == spelling
                && highlight.kind == kind
                && highlight.modifiers & modifiers == modifiers
        })
    }

    fn kind_at(index: &SemanticHighlightIndex, offset: usize) -> Option<SemanticTokenKind> {
        index
            .highlights()
            .iter()
            .find(|highlight| highlight.span.start <= offset && offset < highlight.span.end)
            .map(|highlight| highlight.kind)
    }

    #[test]
    fn highlights_member_access_dots_as_operators_without_splitting_float_literals() {
        let source = r#"
struct Position {
    x: f64,
}

state "game.exe" {}

fn offset(position: Position) -> f64 {
    return position.x + 1.5
}
"#;
        let mut database = CompilerDatabase::new(source);
        database
            .check()
            .expect("member access highlighting fixture");
        let highlights = database.semantic_highlights().unwrap();

        let accessor = source.find("position.x").unwrap() + "position".len();
        assert_eq!(
            kind_at(&highlights, accessor),
            Some(SemanticTokenKind::Operator)
        );

        let decimal_point = source.find("1.5").unwrap() + 1;
        assert_eq!(
            kind_at(&highlights, decimal_point),
            Some(SemanticTokenKind::Number)
        );
    }

    #[test]
    fn highlights_character_literals_as_strings_and_char_as_a_builtin_type() {
        let source = "state \"game.exe\" {}\nfn identity(value: char) -> char { return 'x' }";
        let mut database = CompilerDatabase::new(source);
        database.check().expect("character highlighting fixture");
        let highlights = database.semantic_highlights().unwrap();

        assert!(contains(
            source,
            &highlights,
            "char",
            SemanticTokenKind::Type,
            0,
        ));
        assert!(contains(
            source,
            &highlights,
            "'x'",
            SemanticTokenKind::String,
            0,
        ));
    }

    #[test]
    fn distinguishes_template_strings_from_ordinary_strings() {
        let source = concat!(
            "state \"game.exe\" {}\n",
            "fn label(value: i32) -> String { return `value={value}` }",
        );
        let mut database = CompilerDatabase::new(source);
        database.check().expect("template highlighting fixture");
        let highlights = database.semantic_highlights().unwrap();

        assert!(contains(
            source,
            &highlights,
            "\"game.exe\"",
            SemanticTokenKind::String,
            0,
        ));
        assert!(contains(
            source,
            &highlights,
            "value=",
            SemanticTokenKind::TemplateString,
            0,
        ));
    }

    #[test]
    fn highlights_contextual_keywords_only_in_their_grammar_positions() {
        let source = r#"
enum Mode {
    A,
}

state "game.exe" {
    value: i32 at 0x100;
}

tickRate {
    attached: 60,
    detached: 2,
}

settings {
    "Flag" => flag key "stable-flag": true,
    "Mode" => mode: choice {
        "A" => Mode.A default,
    },
    "Path" => path: file {
        mime => "text/plain",
    },
}

fn names(at: i32, key: i32, choice: i32, default: i32, file: i32, mime: i32, in: i32, attached: i32, detached: i32) -> i32 {
    return at + key + choice + default + file + mime + in + attached + detached
}

whileAttached {
    for item in [1] {
        print(item)
    }

    print(names(1, 2, 3, 4, 5, 6, 7, 8, 9))
}
"#;
        let mut database = CompilerDatabase::new(source);
        database.check().expect("contextual keyword fixture");
        let highlights = database.semantic_highlights().unwrap();

        for spelling in [
            "at 0x100",
            "key \"stable-flag\"",
            "choice {",
            "default,",
            "file {",
            "mime =>",
            "in [1]",
            "attached: 60",
            "detached: 2",
        ] {
            let offset = source.find(spelling).unwrap();
            assert_eq!(
                kind_at(&highlights, offset),
                Some(SemanticTokenKind::Keyword),
                "wrong contextual highlighting for `{spelling}`"
            );
        }

        for name in [
            "at", "key", "choice", "default", "file", "mime", "in", "attached", "detached",
        ] {
            let offset = source.find(&format!("{name}: i32")).unwrap();
            assert_eq!(
                kind_at(&highlights, offset),
                Some(SemanticTokenKind::Parameter),
                "ordinary `{name}` parameter was treated as contextual syntax"
            );
        }
    }

    #[test]
    fn highlights_managed_schema_declarations_by_their_language_roles() {
        let source = r#"
enum Edition { Alternate }
let edition: Edition
state "game.exe" {}
onAttach { edition = Edition.Alternate }
image "Assembly-CSharp" {
    namespace Game {
        class Player from "RuntimePlayer" {
            static f32 health from "_health";
            String name;
            if edition == Edition.Alternate {
                f32 armor;
            }
        }
    }
}
let player: Player.Ref? = None
"#;
        let mut database = CompilerDatabase::new(source);
        database
            .check()
            .expect("managed schema highlighting fixture");
        let highlights = database.semantic_highlights().unwrap();

        for keyword in ["image", "namespace", "class", "from", "static", "if"] {
            assert!(contains(
                source,
                &highlights,
                keyword,
                SemanticTokenKind::Keyword,
                0,
            ));
        }
        assert!(contains(
            source,
            &highlights,
            "Game",
            SemanticTokenKind::Namespace,
            MODIFIER_DECLARATION,
        ));
        assert!(contains(
            source,
            &highlights,
            "Player",
            SemanticTokenKind::Struct,
            MODIFIER_DECLARATION,
        ));
        assert!(contains(
            source,
            &highlights,
            "Alternate",
            SemanticTokenKind::EnumMember,
            MODIFIER_DECLARATION | MODIFIER_READONLY,
        ));
        assert!(contains(
            source,
            &highlights,
            "Ref",
            SemanticTokenKind::Type,
            0,
        ));
        assert!(contains(
            source,
            &highlights,
            "health",
            SemanticTokenKind::Property,
            MODIFIER_DECLARATION | MODIFIER_READONLY,
        ));
    }

    #[test]
    fn highlights_managed_instance_discovery_as_a_type_method_call() {
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
        database
            .check()
            .expect("managed instance highlighting fixture");
        let highlights = database.semantic_highlights().unwrap();
        let call_start = source.rfind("Enemy.instances").unwrap();
        assert_eq!(
            kind_at(&highlights, call_start),
            Some(SemanticTokenKind::Type)
        );
        assert_eq!(
            kind_at(&highlights, call_start + "Enemy.".len()),
            Some(SemanticTokenKind::Method)
        );
    }

    #[test]
    fn compiler_highlights_domain_constructs_and_resolved_references() {
        let source = r#"
enum Mode {
    Active
}

state "game.exe" {
    level = process.read<i32>(0);
    mapName at 0x100 as utf8(32);
    chapterName at 0x200 as utf16le(64);
}

settings {
    "General" {
        "Enabled" => enabled key "legacy-enabled": true
    }
}

debug fn observe(mode: Mode) {
    debug print(mode as String)
}

fn preserveUnit(value: None) -> None {
    return value
}

whileAttached {
    let unit: None = None
    preserveUnit(unit)
    let mode = Mode.Active
    let marker = await process.scan(0, 1, sig"48 ??")
    let version = v"1.2.3.4"
    if current.level == 1 {
        observe(mode)
    }
}
"#;
        let mut database = CompilerDatabase::new(source);
        let first = database
            .semantic_highlights()
            .expect("highlighting should survive incomplete semantic information");
        let second = database
            .semantic_highlights()
            .expect("highlighting should be cached");
        assert!(std::sync::Arc::ptr_eq(&first, &second));

        assert!(contains(
            source,
            &first,
            "\"General\"",
            SemanticTokenKind::SettingTitle,
            MODIFIER_DECLARATION | MODIFIER_READONLY
        ));
        assert!(contains(
            source,
            &first,
            "\"Enabled\"",
            SemanticTokenKind::String,
            0
        ));
        assert!(!contains(
            source,
            &first,
            "\"Enabled\"",
            SemanticTokenKind::Setting,
            0
        ));
        assert!(contains(
            source,
            &first,
            "enabled",
            SemanticTokenKind::Setting,
            MODIFIER_DECLARATION
        ));
        assert!(contains(
            source,
            &first,
            "key",
            SemanticTokenKind::Keyword,
            0
        ));
        assert!(contains(
            source,
            &first,
            "\"legacy-enabled\"",
            SemanticTokenKind::String,
            0
        ));
        assert!(contains(
            source,
            &first,
            "true",
            SemanticTokenKind::Constant,
            0
        ));
        assert!(!contains(
            source,
            &first,
            "true",
            SemanticTokenKind::Keyword,
            0
        ));
        assert!(contains(source, &first, "None", SemanticTokenKind::Type, 0));
        assert!(contains(
            source,
            &first,
            "None",
            SemanticTokenKind::EnumMember,
            0
        ));
        assert!(!contains(
            source,
            &first,
            "None",
            SemanticTokenKind::Constant,
            0
        ));
        assert!(contains(
            source,
            &first,
            "level",
            SemanticTokenKind::StateField,
            MODIFIER_READONLY
        ));
        assert!(contains(
            source,
            &first,
            "process",
            SemanticTokenKind::Variable,
            MODIFIER_READONLY
        ));
        assert!(contains(
            source,
            &first,
            "whileAttached",
            SemanticTokenKind::Lifecycle,
            MODIFIER_DECLARATION
        ));
        assert!(contains(
            source,
            &first,
            "Mode",
            SemanticTokenKind::Enum,
            MODIFIER_DECLARATION
        ));
        assert!(contains(
            source,
            &first,
            "Active",
            SemanticTokenKind::EnumMember,
            MODIFIER_DECLARATION
        ));
        assert_eq!(
            first
                .highlights()
                .iter()
                .filter(|highlight| {
                    &source[highlight.span.start..highlight.span.end] == "Active"
                        && highlight.kind == SemanticTokenKind::EnumMember
                })
                .count(),
            2,
            "both the declaration and constructor use should be enum members"
        );
        assert!(!contains(
            source,
            &first,
            "Active",
            SemanticTokenKind::Property,
            0
        ));
        assert!(contains(
            source,
            &first,
            "\"48 ??\"",
            SemanticTokenKind::Signature,
            0
        ));
        assert!(contains(
            source,
            &first,
            "sig",
            SemanticTokenKind::Signature,
            0
        ));
        assert!(contains(
            source,
            &first,
            "\"1.2.3.4\"",
            SemanticTokenKind::Version,
            0
        ));
        assert!(contains(source, &first, "v", SemanticTokenKind::Version, 0));
        assert!(first.highlights().iter().any(|highlight| {
            highlight.modifiers & MODIFIER_DEBUG != 0
                && &source[highlight.span.start..highlight.span.end] == "print"
        }));
        assert!(contains(
            source,
            &first,
            "scan",
            SemanticTokenKind::Method,
            MODIFIER_DEFAULT_LIBRARY
        ));
        assert!(contains(
            source,
            &first,
            "utf8",
            SemanticTokenKind::Function,
            MODIFIER_READONLY | MODIFIER_DEFAULT_LIBRARY
        ));
        assert!(contains(
            source,
            &first,
            "utf16le",
            SemanticTokenKind::Function,
            MODIFIER_READONLY | MODIFIER_DEFAULT_LIBRARY
        ));
    }

    #[test]
    fn highlights_sibling_state_references_as_readonly_state_fields() {
        let source = r#"
state "game.exe" {
    dependent: u32 at source;
    source: address = 0x1000;
}
"#;
        let mut database = CompilerDatabase::new(source);
        database
            .check()
            .expect("sibling state highlighting fixture");
        let highlights = database.semantic_highlights().unwrap();
        let reference = source.find("at source").unwrap() + "at ".len();
        let highlight = highlights
            .highlights()
            .iter()
            .find(|highlight| highlight.span.start == reference)
            .expect("dynamic state base should be highlighted");
        assert_eq!(highlight.kind, SemanticTokenKind::StateField);
        assert_eq!(highlight.modifiers, MODIFIER_READONLY);
    }

    #[test]
    fn highlights_for_keywords_and_read_only_element_bindings() {
        let source = r#"state "game.exe" {}
whileAttached {
    for value in [1, 2] {
        print(value as String)
    }
}"#;
        let mut database = CompilerDatabase::new(source);
        let index = database.semantic_highlights().unwrap();
        assert!(contains(
            source,
            &index,
            "for",
            SemanticTokenKind::Keyword,
            0
        ));
        assert!(contains(
            source,
            &index,
            "in",
            SemanticTokenKind::Keyword,
            0
        ));
        assert!(contains(
            source,
            &index,
            "value",
            SemanticTokenKind::Variable,
            MODIFIER_DECLARATION | MODIFIER_READONLY
        ));
    }

    #[test]
    fn highlights_state_filter_context() {
        let source = r#"state "game.exe" {
    scene: i32 at 0x100 if value == 7 { Err("transient") } else { value };
}"#;
        let mut database = CompilerDatabase::new(source);
        let index = database.semantic_highlights().unwrap();
        assert!(contains(
            source,
            &index,
            "if",
            SemanticTokenKind::Keyword,
            0
        ));
        assert!(contains(
            source,
            &index,
            "value",
            SemanticTokenKind::Variable,
            MODIFIER_READONLY
        ));
    }

    #[test]
    fn resolved_fields_override_builtin_type_name_highlighting() {
        let source = r#"struct Marker {
    address: i32
}
state "game.exe" {}
fn observe(module: Module, marker: Marker) {
    if module.address == 0x1000 && marker.address == 1 {
        print("found")
    }
}"#;
        let mut database = CompilerDatabase::new(source);
        let index = database.semantic_highlights().unwrap();

        for (path, expected_modifiers) in [
            (
                "module.address",
                MODIFIER_READONLY | MODIFIER_DEFAULT_LIBRARY,
            ),
            ("marker.address", MODIFIER_READONLY),
        ] {
            let start = source.find(path).unwrap() + path.find('.').unwrap() + 1;
            let highlight = index
                .highlights()
                .iter()
                .find(|highlight| highlight.span.start == start)
                .expect("the resolved field should be highlighted");
            assert_eq!(highlight.kind, SemanticTokenKind::Property);
            assert_eq!(highlight.modifiers, expected_modifiers);
        }
    }

    #[test]
    fn inferred_and_explicit_memory_reads_highlight_their_written_call_shape() {
        let source = r#"state "game.exe" {
    inferred: u32 = process.read(0x100);
    selected: u32 = process.read<u32>(0x104);
}"#;
        let mut database = CompilerDatabase::new(source);
        let index = database.semantic_highlights().unwrap();

        let highlight_at = |offset| {
            index
                .highlights()
                .iter()
                .find(|highlight| highlight.span.start == offset)
                .copied()
                .expect("the resolved call segment should have a semantic token")
        };

        let inferred = source.find("process.read(0x100)").unwrap();
        let inferred_process = highlight_at(inferred);
        assert_eq!(inferred_process.kind, SemanticTokenKind::Variable);
        assert_eq!(inferred_process.modifiers, MODIFIER_READONLY);
        let inferred_read = highlight_at(inferred + "process.".len());
        assert_eq!(inferred_read.kind, SemanticTokenKind::Method);
        assert_eq!(inferred_read.modifiers, MODIFIER_DEFAULT_LIBRARY);

        let selected = source.find("process.read<u32>(0x104)").unwrap();
        let selected_process = highlight_at(selected);
        assert_eq!(selected_process.kind, SemanticTokenKind::Variable);
        assert_eq!(selected_process.modifiers, MODIFIER_READONLY);
        let selected_read = highlight_at(selected + "process.".len());
        assert_eq!(selected_read.kind, SemanticTokenKind::Method);
        assert_eq!(selected_read.modifiers, MODIFIER_DEFAULT_LIBRARY);
        let selected_type = highlight_at(selected + "process.read<".len());
        assert_eq!(selected_type.kind, SemanticTokenKind::Type);
        assert_eq!(selected_type.modifiers, 0);
    }

    #[test]
    fn highlights_fields_and_methods_on_expression_receivers() {
        let source = r#"struct Path {
    address: address
}
fn Path.resolve() { return self.address }
struct Layout {
    isLoading: Path,
    level: Path,
    video: Path
}
fn selectedLayout() {
    return Layout {
        isLoading: Path { address: 1 },
        level: Path { address: 2 },
        video: Path { address: 3 }
    }
}
state "game.exe" {
    loading: address = selectedLayout().isLoading.resolve();
    level: address = selectedLayout().level.resolve();
    video: address = selectedLayout().video.resolve();
}"#;
        let mut database = CompilerDatabase::new(source);
        let index = database.semantic_highlights().unwrap();

        for field in ["isLoading", "level", "video"] {
            let path = format!("selectedLayout().{field}.resolve");
            let field_start = source.find(&path).unwrap() + "selectedLayout().".len();
            let field_highlight = index
                .highlights()
                .iter()
                .find(|highlight| highlight.span.start == field_start)
                .expect("the expression-receiver field should have a semantic token");
            assert_eq!(field_highlight.kind, SemanticTokenKind::Property);
            assert_eq!(field_highlight.modifiers, MODIFIER_READONLY);

            let method_start = field_start + field.len() + 1;
            let method_highlight = index
                .highlights()
                .iter()
                .find(|highlight| highlight.span.start == method_start)
                .expect("the expression-receiver method should have a semantic token");
            assert_eq!(method_highlight.kind, SemanticTokenKind::Method);
        }
    }

    #[test]
    fn struct_shorthand_is_highlighted_as_the_destination_field() {
        let source = r#"struct Point { x: u32 }
state "game.exe" {}
fn point(value: u32) -> Point {
    let x = value
    return Point { x }
}"#;
        let shorthand = source.rfind("{ x }").unwrap() + 2;
        let mut database = CompilerDatabase::new(source);
        let highlights = database.semantic_highlights().unwrap();
        let highlight = highlights
            .highlights()
            .iter()
            .find(|highlight| highlight.span.start == shorthand)
            .expect("the shorthand field should have a semantic token");
        assert_eq!(highlight.kind, SemanticTokenKind::Property);
        assert_eq!(highlight.modifiers, MODIFIER_READONLY);
    }

    #[test]
    fn struct_pattern_shorthand_is_highlighted_as_the_matched_field() {
        let source = r#"struct Point { x: u32 }
state "game.exe" {}
fn observe(point: Point) -> u32 {
    return match point { Point { x } => x }
}"#;
        let shorthand = source.rfind("{ x }").unwrap() + 2;
        let mut database = CompilerDatabase::new(source);
        let highlights = database.semantic_highlights().unwrap();
        let highlight = highlights
            .highlights()
            .iter()
            .find(|highlight| highlight.span.start == shorthand)
            .expect("the shorthand pattern field should have a semantic token");
        assert_eq!(highlight.kind, SemanticTokenKind::Property);
        assert_eq!(highlight.modifiers, MODIFIER_READONLY);
    }

    #[test]
    fn highlights_state_providers_and_snapshot_roots_as_domain_values() {
        let source = r#"state GBA {
    room: u8 at 0x03000010
}
settings {
    "Enabled" => enabled: true
}
whileAttached {
    let changed = current.room != old.room
        || settings.enabled != oldSettings.enabled
}"#;
        let mut database = CompilerDatabase::new(source);
        let highlights = database.semantic_highlights().unwrap();
        assert!(contains(
            source,
            &highlights,
            "GBA",
            SemanticTokenKind::Type,
            MODIFIER_READONLY | MODIFIER_DEFAULT_LIBRARY
        ));
        for root in ["current", "old", "settings", "oldSettings"] {
            assert!(contains(
                source,
                &highlights,
                root,
                SemanticTokenKind::Variable,
                MODIFIER_READONLY
            ));
        }
    }

    #[test]
    fn highlights_provider_contexts_as_read_only_values() {
        let source = r#"state Unity ["game.exe"] {}
whileAttached {
    let scenes = unity.scenes
}"#;
        let mut database = CompilerDatabase::new(source);
        let highlights = database.semantic_highlights().unwrap();
        assert!(contains(
            source,
            &highlights,
            "unity",
            SemanticTokenKind::Variable,
            MODIFIER_READONLY
        ));
        assert!(contains(
            source,
            &highlights,
            "scenes",
            SemanticTokenKind::Property,
            MODIFIER_READONLY
        ));
    }

    #[test]
    fn highlights_current_state_fields_as_mutable_and_old_fields_as_read_only() {
        let source = r#"state "game.exe" {
    room: u8 at 0x100;
}
whileAttached {
    current.room = old.room
}"#;
        let mut database = CompilerDatabase::new(source);
        let highlights = database.semantic_highlights().unwrap();

        let current_field = source.find("current.room").unwrap() + "current.".len();
        let old_field = source.find("old.room").unwrap() + "old.".len();
        let highlight_at = |offset| {
            highlights
                .highlights()
                .iter()
                .find(|highlight| highlight.span.start == offset)
                .expect("state field should have a semantic token")
        };

        assert_eq!(
            highlight_at(current_field).kind,
            SemanticTokenKind::StateField
        );
        assert_eq!(highlight_at(current_field).modifiers, 0);
        assert_eq!(highlight_at(old_field).kind, SemanticTokenKind::StateField);
        assert_eq!(highlight_at(old_field).modifiers, MODIFIER_READONLY);
    }

    #[test]
    fn highlights_resolved_choice_setting_variants() {
        let source = r#"enum CaptureMode {
    WindowTitle,
    ExecutableName
}
state "game.exe" {}
settings {
    "Capture Source" => captureMode: choice {
        "Window Title" => CaptureMode.WindowTitle,
        "Executable Name" => CaptureMode.ExecutableName default
    }
}"#;
        let mut database = CompilerDatabase::new(source);
        let highlights = database.semantic_highlights().unwrap();
        for spelling in ["WindowTitle", "ExecutableName"] {
            assert_eq!(
                highlights
                    .highlights()
                    .iter()
                    .filter(|highlight| {
                        &source[highlight.span.start..highlight.span.end] == spelling
                            && highlight.kind == SemanticTokenKind::EnumMember
                    })
                    .count(),
                2,
                "both the declaration and choice option should be enum members"
            );
        }
    }

    #[test]
    fn syntax_recovery_keeps_lexical_highlighting_available() {
        let source = "// still editing\nstate \"game.exe\" {";
        let mut database = CompilerDatabase::new(source);
        let index = database
            .semantic_highlights()
            .expect("recoverable syntax errors should retain highlighting");
        assert!(contains(
            source,
            &index,
            "// still editing",
            SemanticTokenKind::Comment,
            0
        ));
        assert!(contains(
            source,
            &index,
            "state",
            SemanticTokenKind::Keyword,
            0
        ));
    }

    #[test]
    fn parser_errors_do_not_discard_semantic_highlighting_elsewhere() {
        for broken in ["0b102", "\"unfinished"] {
            let source = format!(
                r#"
fn retained(value: i32) -> i32 {{
    return value
}}
state GBA {{}}
split {{
    let broken = {broken}
    let result = retained(1)
}}
"#
            );
            let mut database = CompilerDatabase::new(&source);
            let index = database
                .semantic_highlights()
                .expect("front-end errors should preserve independent semantic highlighting");
            let call = source.rfind("retained").unwrap() + 1;
            assert_eq!(kind_at(&index, call), Some(SemanticTokenKind::Function));
        }
    }
}
