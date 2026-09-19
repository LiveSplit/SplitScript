//! Canonical source formatting built on the compiler's lossless lexer and
//! strict parser.

use std::collections::HashSet;

use crate::{
    Diagnostic,
    ast::{
        Action, EnumDecl, Expr, ExprKind, FunctionDecl, ManagedClassDecl, ManagedImageDecl,
        ManagedNamespaceDecl, MatchArm, Program, SettingDecl, SettingFamilyDecl, SettingKind,
        StateDecl, StateField, Stmt, StructDecl, TypeApplicationDecl, VariableDecl,
    },
    lexer::{Lexeme, Token, TokenKind, TriviaKind},
    syntax::SourceDocument,
    visit::{self, Visitor},
};

pub const DEFAULT_MAX_LINE_WIDTH: usize = 100;
pub const DEFAULT_INDENT_WIDTH: usize = 4;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum IndentStyle {
    Spaces,
    Tabs,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum LineEnding {
    Lf,
    CrLf,
    Cr,
}

/// Stable formatter policy shared by the CLI and editor frontends.
///
/// The default remains compiler-owned. Exposing the policy as a value keeps a
/// future workspace or editor setting from requiring a second formatting
/// implementation.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct FormatOptions {
    /// Preferred maximum number of Unicode scalar values on one line.
    /// Unbreakable tokens may still exceed it.
    pub max_line_width: usize,
    /// Whether indentation is represented by spaces or tabs.
    pub indent_style: IndentStyle,
    /// Spaces per indentation level, or the display width of one indentation tab.
    pub indent_width: usize,
    /// Line ending written to the formatted document.
    pub line_ending: LineEnding,
    /// Whether the formatted document ends with a line ending.
    pub insert_final_newline: bool,
}

impl Default for FormatOptions {
    fn default() -> Self {
        Self {
            max_line_width: DEFAULT_MAX_LINE_WIDTH,
            indent_style: IndentStyle::Spaces,
            indent_width: DEFAULT_INDENT_WIDTH,
            line_ending: LineEnding::Lf,
            insert_final_newline: true,
        }
    }
}

/// Formats a syntactically valid SplitScript source file.
///
/// Formatting first runs the ordinary compiler parser, so it never needs a
/// second, potentially divergent grammar. Invalid source is returned as the
/// same structured syntax diagnostics produced by [`crate::parse`].
pub fn format_source(source: &str) -> Result<String, Vec<Diagnostic>> {
    format_source_with_options(source, FormatOptions::default())
}

/// Formats a source file with an explicit presentation policy.
pub fn format_source_with_options(
    source: &str,
    options: FormatOptions,
) -> Result<String, Vec<Diagnostic>> {
    let parsed = crate::parse(source)?;
    Ok(format_parsed_with_options(&parsed, options))
}

pub(crate) fn format_parsed(parsed: &crate::ParsedProgram) -> String {
    format_parsed_with_options(parsed, FormatOptions::default())
}

pub(crate) fn format_parsed_with_options(
    parsed: &crate::ParsedProgram,
    options: FormatOptions,
) -> String {
    let formatted = Formatter::new(parsed.source_document(), parsed.syntax(), options).finish();
    apply_output_style(wrap_long_lines(formatted, options), options)
}

fn apply_output_style(mut source: String, options: FormatOptions) -> String {
    if !options.insert_final_newline {
        source.pop();
    }
    match options.line_ending {
        LineEnding::Lf => source,
        LineEnding::CrLf => source.replace('\n', "\r\n"),
        LineEnding::Cr => source.replace('\n', "\r"),
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Separation {
    None,
    Space,
    Newline,
    BlankLine,
}

#[derive(Debug, Clone, Copy)]
struct HeaderRange {
    start: usize,
    opening_brace: usize,
}

#[derive(Debug, Clone, Copy)]
struct ContinuationRange {
    start: usize,
    end: usize,
}

#[derive(Debug)]
struct DelimiterRange {
    opening: usize,
    closing: usize,
    direct_breaks: Vec<usize>,
}

#[derive(Debug, Clone, Copy)]
struct BraceFrame {
    parent_indentation: usize,
    brace_indentation: usize,
    opening: usize,
}

#[derive(Default)]
struct HeaderCollector {
    ranges: Vec<HeaderRange>,
}

impl HeaderCollector {
    fn push(&mut self, start: usize, opening_brace: usize) {
        self.ranges.push(HeaderRange {
            start,
            opening_brace,
        });
    }
}

impl<'ast> Visitor<'ast> for HeaderCollector {
    fn visit_function(&mut self, function: &'ast FunctionDecl) {
        self.push(function.span.start, function.body.span.start);
        visit::walk_function(self, function);
    }

    fn visit_action(&mut self, action: &'ast Action) {
        self.push(action.span.start, action.body.span.start);
        self.visit_block(&action.body);
    }

    fn visit_managed_image(&mut self, image: &'ast ManagedImageDecl) {
        self.push(image.span.start, image.opening_span.start);
        visit::walk_managed_image(self, image);
    }

    fn visit_managed_namespace(&mut self, namespace: &'ast ManagedNamespaceDecl) {
        self.push(namespace.span.start, namespace.opening_span.start);
        visit::walk_managed_namespace(self, namespace);
    }

    fn visit_managed_class(&mut self, class: &'ast ManagedClassDecl) {
        self.push(class.span.start, class.opening_span.start);
        for group in &class.conditional_fields {
            self.push(group.span.start, group.opening_span.start);
        }
        visit::walk_managed_class(self, class);
    }

    fn visit_state(&mut self, state: &'ast StateDecl) {
        for alternative in &state.provider_alternatives {
            self.push(alternative.span.start, alternative.opening_span.start);
        }
        for group in &state.conditional_fields {
            self.push(group.span.start, group.opening_span.start);
        }
        visit::walk_state(self, state);
    }

    fn visit_stmt(&mut self, statement: &'ast Stmt) {
        match statement {
            Stmt::If {
                then_block,
                else_block,
                span,
                ..
            } => {
                self.push(span.start, then_block.span.start);
                if let Some(else_block) = else_block {
                    self.push(then_block.span.end, else_block.span.start);
                }
            }
            Stmt::While { body, span, .. } | Stmt::For { body, span, .. } => {
                self.push(span.start, body.span.start)
            }
            _ => {}
        }
        visit::walk_stmt(self, statement);
    }

    fn visit_expr(&mut self, expression: &'ast Expr) {
        if let ExprKind::If {
            then_expr,
            else_expr,
            ..
        } = &expression.kind
        {
            self.push(expression.span.start, then_expr.span.start);
            self.push(then_expr.span.end, else_expr.span.start);
        }
        visit::walk_expr(self, expression);
    }
}

struct SyntaxLayoutCollector<'a> {
    document: &'a SourceDocument,
    continuations: Vec<ContinuationRange>,
    join_before: HashSet<usize>,
    break_after: HashSet<usize>,
    compact_brace_opens: HashSet<usize>,
    compact_brace_closes: HashSet<usize>,
    generic_opens: HashSet<usize>,
    generic_closes: HashSet<usize>,
    fallible_type_suffixes: HashSet<usize>,
    index_opens: HashSet<usize>,
}

impl SyntaxLayoutCollector<'_> {
    fn collect<'a>(document: &'a SourceDocument, syntax: &Program) -> SyntaxLayoutCollector<'a> {
        let mut layout = SyntaxLayoutCollector {
            document,
            continuations: Vec::new(),
            join_before: HashSet::new(),
            break_after: HashSet::new(),
            compact_brace_opens: HashSet::new(),
            compact_brace_closes: HashSet::new(),
            generic_opens: HashSet::new(),
            generic_closes: HashSet::new(),
            fallible_type_suffixes: syntax
                .result_types
                .iter()
                .flat_map(|result| result.occurrences.iter().map(|span| span.start))
                .collect(),
            index_opens: HashSet::new(),
        };
        layout.visit_program(syntax);
        layout
    }

    /// Marks braces that delimit value-shaped syntax rather than executable
    /// or declaration blocks. The formatter may preserve these braces on one
    /// line when the author did so; ordinary blocks remain vertical.
    fn mark_compact_braces(&mut self, start: usize, end: usize) {
        for token in self
            .document
            .tokens()
            .filter(|token| start <= token.span.start && token.span.end <= end)
        {
            match token.kind {
                TokenKind::LBrace => {
                    self.compact_brace_opens.insert(token.span.start);
                }
                TokenKind::RBrace => {
                    self.compact_brace_closes.insert(token.span.start);
                }
                _ => {}
            }
        }
    }

    fn mark_outer_compact_braces(&mut self, start: usize, end: usize) {
        let tokens = self
            .document
            .tokens()
            .filter(|token| start <= token.span.start && token.span.end <= end)
            .collect::<Vec<_>>();
        if let Some(opening) = tokens.iter().find(|token| token.kind == TokenKind::LBrace)
            && let Some(closing) = tokens
                .iter()
                .rev()
                .find(|token| token.kind == TokenKind::RBrace)
        {
            self.compact_brace_opens.insert(opening.span.start);
            self.compact_brace_closes.insert(closing.span.start);
        }
    }

    fn continuation(&mut self, start: usize, end: usize) {
        if line_breaks(&self.document.source()[start..end]) > 0 {
            self.continuations.push(ContinuationRange { start, end });
        }
    }

    fn continuation_before_block(&mut self, start: usize, end: usize) {
        let end = self
            .document
            .tokens()
            .find(|token| {
                start <= token.span.start
                    && token.span.end <= end
                    && token.kind == TokenKind::LBrace
            })
            .map_or(end, |token| token.span.start);
        self.continuation(start, end);
    }

    fn mark_separator_after(&mut self, offset: usize) {
        if let Some(token) = self
            .document
            .tokens()
            .find(|token| token.span.start >= offset)
            && matches!(token.kind, TokenKind::Comma | TokenKind::Semicolon)
        {
            self.break_after.insert(token.span.end);
        }
    }

    fn block_after_keyword(&self, setting: &SettingDecl, keyword: &str) -> Option<(usize, usize)> {
        let mut saw_keyword = false;
        let mut depth = 0usize;
        let mut opening = None;
        for token in self.document.tokens().filter(|token| {
            setting.span.start <= token.span.start && token.span.end <= setting.span.end
        }) {
            if !saw_keyword {
                saw_keyword = matches!(&token.kind, TokenKind::Ident(name) if name == keyword);
                continue;
            }
            match token.kind {
                TokenKind::LBrace => {
                    if opening.is_none() {
                        opening = Some(token.span.start);
                    }
                    depth += 1;
                }
                TokenKind::RBrace if depth > 0 => {
                    depth -= 1;
                    if depth == 0 {
                        return opening.map(|opening| (opening, token.span.start));
                    }
                }
                _ => {}
            }
        }
        None
    }

    fn mark_top_level_commas(&mut self, opening: usize, closing: usize) {
        let mut depth = 0usize;
        for token in self
            .document
            .tokens()
            .filter(|token| opening < token.span.start && token.span.start < closing)
        {
            match token.kind {
                TokenKind::LParen
                | TokenKind::LBracket
                | TokenKind::LBrace
                | TokenKind::TemplateExprStart => depth += 1,
                TokenKind::RParen
                | TokenKind::RBracket
                | TokenKind::RBrace
                | TokenKind::TemplateExprEnd => depth = depth.saturating_sub(1),
                TokenKind::Comma if depth == 0 => {
                    self.break_after.insert(token.span.end);
                }
                _ => {}
            }
        }
    }

    fn mark_declaration_items_vertical(
        &mut self,
        span: crate::ast::Span,
        item_ends: impl IntoIterator<Item = usize>,
    ) {
        if let Some(opening) = self.document.tokens().find(|token| {
            span.start <= token.span.start
                && token.span.end <= span.end
                && token.kind == TokenKind::LBrace
        }) {
            self.break_after.insert(opening.span.end);
        }
        for end in item_ends {
            self.break_after.insert(end);
            self.mark_separator_after(end);
        }
    }
}

impl<'ast> Visitor<'ast> for SyntaxLayoutCollector<'_> {
    fn visit_type_application(&mut self, application: &'ast TypeApplicationDecl) {
        for occurrence in &application.occurrences {
            self.generic_opens.insert(occurrence.opening.start);
            self.generic_closes.insert(occurrence.closing.start);
        }
        for argument in &application.arguments {
            self.visit_type_ref(argument);
        }
    }

    fn visit_struct(&mut self, structure: &'ast StructDecl) {
        self.mark_declaration_items_vertical(
            structure.span,
            structure.fields.iter().map(|field| field.span.end),
        );
        visit::walk_struct(self, structure);
    }

    fn visit_enum(&mut self, enumeration: &'ast EnumDecl) {
        self.mark_declaration_items_vertical(
            enumeration.span,
            enumeration.variants.iter().map(|variant| variant.span.end),
        );
        visit::walk_enum(self, enumeration);
    }

    fn visit_managed_image(&mut self, image: &'ast ManagedImageDecl) {
        self.mark_declaration_items_vertical(
            image.span,
            image.items.iter().map(|item| item.span().end),
        );
        visit::walk_managed_image(self, image);
    }

    fn visit_managed_namespace(&mut self, namespace: &'ast ManagedNamespaceDecl) {
        self.mark_declaration_items_vertical(
            namespace.span,
            namespace.items.iter().map(|item| item.span().end),
        );
        visit::walk_managed_namespace(self, namespace);
    }

    fn visit_managed_class(&mut self, class: &'ast ManagedClassDecl) {
        self.mark_declaration_items_vertical(
            class.span,
            class
                .fields
                .iter()
                .map(|field| field.span.end)
                .chain(class.conditional_fields.iter().map(|group| group.span.end)),
        );
        for group in &class.conditional_fields {
            if let Some(else_span) = group.else_span {
                self.join_before.insert(else_span.start);
            }
            self.mark_declaration_items_vertical(
                group.span,
                group.fields.iter().map(|field| field.span.end),
            );
        }
        visit::walk_managed_class(self, class);
    }

    fn visit_state(&mut self, state: &'ast StateDecl) {
        for alternative in &state.provider_alternatives {
            self.mark_separator_after(alternative.span.end);
            for field in &alternative.fields {
                self.visit_state_field(field);
            }
        }
        for field in &state.fields {
            self.visit_state_field(field);
        }
        for group in &state.conditional_fields {
            if let Some(else_span) = group.else_span {
                self.join_before.insert(else_span.start);
            }
            self.break_after.insert(group.span.end);
            self.continuation_before_block(group.span.start, group.opening_span.start);
            for field in &group.fields {
                self.visit_state_field(field);
            }
        }
        self.break_after.insert(state.span.end);
    }

    fn visit_state_field(&mut self, field: &'ast StateField) {
        self.continuation_before_block(field.span.start, field.span.end);
        visit::walk_state_field(self, field);
    }

    fn visit_variable(&mut self, variable: &'ast VariableDecl) {
        self.continuation_before_block(variable.span.start, variable.span.end);
        visit::walk_variable(self, variable);
    }

    fn visit_stmt(&mut self, statement: &'ast Stmt) {
        match statement {
            Stmt::Assign { span, .. }
            | Stmt::StateAssign { span, .. }
            | Stmt::IndexAssign { span, .. }
            | Stmt::Suspend { span, .. }
            | Stmt::Yield { span, .. } => {
                self.continuation_before_block(span.start, span.end);
            }
            Stmt::Expression(expression) => {
                self.continuation_before_block(expression.span.start, expression.span.end);
            }
            Stmt::If { span, .. } | Stmt::While { span, .. } | Stmt::For { span, .. } => {
                self.break_after.insert(span.end);
            }
            Stmt::Debug { .. } | Stmt::Variable(_) => {}
        }
        visit::walk_stmt(self, statement);
    }

    fn visit_setting(&mut self, setting: &'ast SettingDecl) {
        if !matches!(setting.kind, SettingKind::Title { .. }) {
            self.join_before.extend(
                self.document
                    .tokens()
                    .filter(|token| {
                        setting.span.start <= token.span.start
                            && token.span.end <= setting.span.end
                            && token.kind == TokenKind::FatArrow
                    })
                    .map(|token| token.span.start),
            );
        }
        match &setting.kind {
            SettingKind::Bool { .. } | SettingKind::Text { .. } => {
                self.continuation(setting.span.start, setting.span.end);
            }
            SettingKind::Choice { options, .. } => {
                if let Some((opening, closing)) = self.block_after_keyword(setting, "choice") {
                    self.continuation(setting.span.start, opening);
                    self.mark_top_level_commas(opening, closing);
                }
                for option in options {
                    self.continuation(option.span.start, option.span.end);
                }
            }
            SettingKind::File { .. } => {
                if let Some((opening, closing)) = self.block_after_keyword(setting, "file") {
                    self.continuation(setting.span.start, opening);
                    self.mark_top_level_commas(opening, closing);
                }
            }
            SettingKind::Title { .. } => {}
        }
        self.mark_separator_after(setting.span.end);
    }

    fn visit_setting_family(&mut self, family: &'ast SettingFamilyDecl) {
        let tokens = self
            .document
            .tokens()
            .filter(|token| {
                family.span.start <= token.span.start && token.span.end <= family.span.end
            })
            .collect::<Vec<_>>();
        if let Some(opening) = tokens.iter().find(|token| token.kind == TokenKind::LBrace) {
            self.break_after.insert(opening.span.end);
        }
        if let Some(closing_index) = tokens
            .iter()
            .rposition(|token| token.kind == TokenKind::RBrace)
            && let Some(previous) = closing_index
                .checked_sub(1)
                .and_then(|index| tokens.get(index))
        {
            self.break_after.insert(previous.span.end);
        }
        self.mark_separator_after(family.span.end);
    }

    fn visit_expr(&mut self, expression: &'ast Expr) {
        if matches!(expression.kind, ExprKind::Struct { .. }) {
            self.mark_outer_compact_braces(expression.span.start, expression.span.end);
        }
        if let ExprKind::Is { pattern, .. } = &expression.kind {
            self.mark_compact_braces(pattern.span.start, pattern.span.end);
        }
        if let ExprKind::Match { value, .. } = &expression.kind
            && let Some(opening) = self.document.tokens().find(|token| {
                value.span.end <= token.span.start
                    && token.span.end <= expression.span.end
                    && token.kind == TokenKind::LBrace
            })
        {
            self.mark_top_level_commas(opening.span.start, expression.span.end);
        }
        if let ExprKind::Call {
            callee,
            name_span,
            receiver,
            type_arguments,
            type_argument_span,
            ..
        } = &expression.kind
        {
            if !type_arguments.is_empty()
                && let Some(span) = type_argument_span
            {
                self.generic_opens.insert(span.start);
                self.generic_closes.insert(span.end - 1);
            }
            // Indexed assignment is represented as a compiler-inserted call
            // to Array.set. Its outer Index node is intentionally absent, so
            // recover the one bracket between the receiver and `=` for layout.
            if callee.as_slice() == ["set"]
                && name_span.start == name_span.end
                && let Some(receiver) = receiver
                && let Some(opening) = self.document.tokens().find(|token| {
                    receiver.span.end <= token.span.start
                        && token.span.end <= name_span.start
                        && token.kind == TokenKind::LBracket
                })
            {
                self.index_opens.insert(opening.span.start);
            }
        }
        if let ExprKind::Index { bracket_span, .. } = &expression.kind {
            self.index_opens.insert(bracket_span.start);
        }
        visit::walk_expr(self, expression);
    }

    fn visit_match_arm(&mut self, arm: &'ast MatchArm) {
        let pattern_end = arm.guard.as_ref().map_or_else(
            || {
                self.document
                    .tokens()
                    .filter(|token| {
                        arm.span.start <= token.span.start
                            && token.span.end <= arm.value.span.start
                            && token.kind == TokenKind::FatArrow
                    })
                    .last()
                    .map_or(arm.value.span.start, |token| token.span.start)
            },
            |guard| guard.span.start,
        );
        self.mark_compact_braces(arm.span.start, pattern_end);
        if let Some(guard) = &arm.guard {
            self.continuation_before_block(guard.span.start, guard.span.end);
        }
        self.continuation_before_block(arm.value.span.start, arm.value.span.end);
        visit::walk_match_arm(self, arm);
    }

    fn visit_binding_pattern(&mut self, binding: &'ast crate::ast::BindingPattern) {
        self.mark_compact_braces(binding.pattern.span.start, binding.pattern.span.end);
        visit::walk_binding_pattern(self, binding);
    }
}

#[derive(Default)]
struct ValueBlockSemicolonCollector {
    spans: HashSet<usize>,
}

impl<'ast> Visitor<'ast> for ValueBlockSemicolonCollector {
    fn visit_expr(&mut self, expression: &'ast Expr) {
        if let ExprKind::Block(block) = &expression.kind
            && let Some(semicolon) = block.trailing_semicolon
            && matches!(block.statements.last(), Some(Stmt::Expression(_)))
        {
            self.spans.insert(semicolon.start);
        }
        visit::walk_expr(self, expression);
    }
}

struct Formatter<'a> {
    document: &'a SourceDocument,
    lexemes: Vec<Lexeme>,
    output: String,
    indentation: usize,
    line_start: bool,
    previous: Option<Lexeme>,
    previous_was_prefix: bool,
    previous_was_generic_close: bool,
    gap_line_breaks: usize,
    generic_depth: usize,
    generic_opens: HashSet<usize>,
    generic_closes: HashSet<usize>,
    fallible_type_suffixes: HashSet<usize>,
    index_opens: HashSet<usize>,
    bracket_depth: usize,
    multiline_headers: Vec<HeaderRange>,
    continuations: Vec<ContinuationRange>,
    delimiters: Vec<DelimiterRange>,
    break_after: HashSet<usize>,
    join_before: HashSet<usize>,
    compact_brace_opens: HashSet<usize>,
    compact_brace_closes: HashSet<usize>,
    trailing_comma_before: HashSet<usize>,
    trailing_semicolon_before: HashSet<usize>,
    omitted_commas: HashSet<usize>,
    omitted_semicolons: HashSet<usize>,
    brace_stack: Vec<BraceFrame>,
    current_line_indentation: usize,
    options: FormatOptions,
}

impl<'a> Formatter<'a> {
    fn new(document: &'a SourceDocument, syntax: &Program, options: FormatOptions) -> Self {
        let mut headers = HeaderCollector::default();
        headers.visit_program(syntax);
        let multiline_headers = headers
            .ranges
            .into_iter()
            .filter(|range| line_breaks(&document.source()[range.start..range.opening_brace]) > 0)
            .collect();
        let layout = SyntaxLayoutCollector::collect(document, syntax);
        let lexemes = formatting_lexemes(
            document,
            &layout.generic_closes,
            &layout.fallible_type_suffixes,
        );
        let trailing_punctuation = trailing_list_punctuation(
            document,
            &lexemes,
            syntax,
            &layout.break_after,
            &layout.generic_opens,
            &layout.generic_closes,
        );
        let delimiters = delimiter_ranges(
            document,
            &lexemes,
            &layout.generic_opens,
            &layout.generic_closes,
        );
        let mut value_block_semicolons = ValueBlockSemicolonCollector::default();
        value_block_semicolons.visit_program(syntax);
        Self {
            document,
            lexemes,
            output: String::with_capacity(document.source().len()),
            indentation: 0,
            line_start: true,
            previous: None,
            previous_was_prefix: false,
            previous_was_generic_close: false,
            gap_line_breaks: 0,
            generic_depth: 0,
            generic_opens: layout.generic_opens,
            generic_closes: layout.generic_closes,
            fallible_type_suffixes: layout.fallible_type_suffixes,
            index_opens: layout.index_opens,
            bracket_depth: 0,
            multiline_headers,
            continuations: layout.continuations,
            delimiters,
            join_before: layout.join_before,
            break_after: layout.break_after,
            compact_brace_opens: layout.compact_brace_opens,
            compact_brace_closes: layout.compact_brace_closes,
            trailing_comma_before: trailing_punctuation.commas,
            trailing_semicolon_before: trailing_punctuation.semicolons,
            omitted_commas: trailing_punctuation.omitted_commas,
            omitted_semicolons: value_block_semicolons.spans,
            brace_stack: Vec::new(),
            current_line_indentation: 0,
            options,
        }
    }

    fn finish(mut self) -> String {
        let lexemes = std::mem::take(&mut self.lexemes);
        for lexeme in &lexemes {
            match lexeme {
                Lexeme::Trivia(trivia) if trivia.kind == TriviaKind::Whitespace => {
                    let text = self.document.text(trivia.span);
                    self.gap_line_breaks += line_breaks(text);
                }
                Lexeme::Token(token) if token.kind == TokenKind::Eof => {}
                _ => self.write_lexeme(lexeme),
            }
        }

        while self.output.ends_with(char::is_whitespace) {
            self.output.pop();
        }
        self.output.push('\n');
        self.output
    }

    fn write_lexeme(&mut self, current: &Lexeme) {
        let current_token = token_kind(current);
        if matches!(current_token, Some(TokenKind::Comma))
            && self.omitted_commas.contains(&current.span().start)
        {
            return;
        }
        if matches!(current_token, Some(TokenKind::Semicolon))
            && self.omitted_semicolons.contains(&current.span().start)
        {
            return;
        }
        if matches!(current_token, Some(TokenKind::RBrace)) {
            self.indentation = self.brace_stack.last().map_or_else(
                || self.indentation.saturating_sub(1),
                |frame| frame.brace_indentation,
            );
        }

        if self.trailing_comma_before.contains(&current.span().start)
            && !matches!(
                self.previous.as_ref().and_then(token_kind),
                Some(TokenKind::Comma)
            )
            && self
                .previous
                .as_ref()
                .is_some_and(|previous| !is_comment(previous))
        {
            self.output.push(',');
        }
        if self
            .trailing_semicolon_before
            .contains(&current.span().start)
            && !matches!(
                self.previous.as_ref().and_then(token_kind),
                Some(TokenKind::Semicolon)
            )
            && self
                .previous
                .as_ref()
                .is_some_and(|previous| !is_comment(previous))
        {
            self.output.push(';');
        }

        let separation = self.separation(current);
        self.write_separation(separation);
        self.write_indentation(current);
        self.output.push_str(self.document.text(current.span()));
        self.line_start = self.output.ends_with(['\n', '\r']);

        let current_is_generic_open = self.generic_opens.contains(&current.span().start);
        let current_is_generic_close = self.generic_closes.contains(&current.span().start);
        if current_is_generic_open {
            self.generic_depth += 1;
        } else if current_is_generic_close {
            self.generic_depth -= 1;
        }
        if matches!(current_token, Some(TokenKind::LBracket)) {
            self.bracket_depth += 1;
        } else if matches!(current_token, Some(TokenKind::RBracket)) {
            self.bracket_depth = self.bracket_depth.saturating_sub(1);
        }

        self.previous_was_prefix = !self.fallible_type_suffixes.contains(&current.span().start)
            && current_token.is_some_and(|kind| {
                is_prefix_operator(kind, self.previous.as_ref().and_then(token_kind))
            });
        self.previous_was_generic_close = current_is_generic_close;
        if matches!(current_token, Some(TokenKind::LBrace)) {
            self.brace_stack.push(BraceFrame {
                parent_indentation: self.indentation,
                brace_indentation: self.current_line_indentation,
                opening: current.span().start,
            });
            self.indentation = self.current_line_indentation + 1;
        } else if matches!(current_token, Some(TokenKind::RBrace))
            && let Some(frame) = self.brace_stack.pop()
        {
            self.indentation = frame.parent_indentation;
        }
        self.previous = Some(current.clone());
        self.gap_line_breaks = 0;
    }

    fn separation(&self, current: &Lexeme) -> Separation {
        let Some(previous) = self.previous.as_ref() else {
            return Separation::None;
        };
        let previous_token = token_kind(previous);
        let current_token = token_kind(current);
        let preserved_break = if self.gap_line_breaks >= 2 {
            Separation::BlankLine
        } else {
            Separation::Newline
        };

        if self.generic_opens.contains(&current.span().start) {
            return Separation::None;
        }
        if self.index_opens.contains(&current.span().start) && !is_comment(previous) {
            return Separation::None;
        }

        // Commas terminate the preceding list item even when a recovery fix
        // inserted one at the beginning of the following source line.
        if matches!(current_token, Some(TokenKind::Comma)) {
            return Separation::None;
        }

        if self.is_multiline_block_opening(current) {
            return if self.current_line_indentation > self.indentation {
                Separation::Newline
            } else {
                Separation::Space
            };
        }

        if self.join_before.contains(&current.span().start) {
            return Separation::Space;
        }

        if self.break_after.contains(&previous.span().end) && !is_comment(current) {
            return if matches!(current_token, Some(TokenKind::RBrace)) {
                Separation::Newline
            } else {
                preserved_break
            };
        }

        if self.gap_line_breaks > 0
            && (is_comment(previous) || is_comment(current) || is_doc_comment(current))
        {
            return preserved_break;
        }
        if is_line_comment(previous) || is_doc_comment(previous) {
            return Separation::Newline;
        }
        if is_doc_comment(current) {
            return Separation::Newline;
        }
        if matches!(current_token, Some(TokenKind::RBrace))
            && self.compact_brace_closes.contains(&current.span().start)
            && !matches!(previous_token, Some(TokenKind::LBrace))
        {
            return if self.gap_line_breaks > 0 {
                preserved_break
            } else {
                Separation::Space
            };
        }
        if matches!(previous_token, Some(TokenKind::LBrace))
            && self.compact_brace_opens.contains(&previous.span().start)
            && !matches!(current_token, Some(TokenKind::RBrace))
        {
            return if self.gap_line_breaks > 0 {
                preserved_break
            } else {
                Separation::Space
            };
        }
        if matches!(current_token, Some(TokenKind::RBrace))
            && !matches!(previous_token, Some(TokenKind::LBrace))
        {
            return Separation::Newline;
        }
        if matches!(previous_token, Some(TokenKind::LBrace))
            && !matches!(current_token, Some(TokenKind::RBrace))
        {
            return Separation::Newline;
        }
        if matches!(previous_token, Some(TokenKind::Semicolon)) {
            return if self.bracket_depth > 0 {
                Separation::Space
            } else {
                Separation::Newline
            };
        }
        if matches!(previous_token, Some(TokenKind::RBrace))
            && matches!(current_token, Some(TokenKind::Ident(name)) if name == "else")
        {
            return Separation::Space;
        }
        if self.gap_line_breaks > 0 {
            return preserved_break;
        }
        if is_comment(current) || is_comment(previous) {
            return Separation::Space;
        }
        if needs_space(
            previous_token,
            current_token,
            self.previous_was_prefix,
            self.previous_was_generic_close,
            self.generic_depth,
        ) {
            Separation::Space
        } else {
            Separation::None
        }
    }

    fn write_separation(&mut self, separation: Separation) {
        match separation {
            Separation::None => {}
            Separation::Space => {
                if !self.line_start && !self.output.ends_with(char::is_whitespace) {
                    self.output.push(' ');
                }
            }
            Separation::Newline | Separation::BlankLine => {
                while self.output.ends_with([' ', '\t', '\r', '\n']) {
                    self.output.pop();
                }
                self.output.push('\n');
                if separation == Separation::BlankLine {
                    self.output.push('\n');
                }
                self.line_start = true;
            }
        }
    }

    fn write_indentation(&mut self, current: &Lexeme) {
        if self.line_start {
            let continuation = self.continuation_indentation(current);
            self.current_line_indentation = self.indentation + continuation;
            for _ in 0..self.current_line_indentation {
                match self.options.indent_style {
                    IndentStyle::Spaces => {
                        for _ in 0..self.options.indent_width.max(1) {
                            self.output.push(' ');
                        }
                    }
                    IndentStyle::Tabs => self.output.push('\t'),
                }
            }
            self.line_start = false;
        }
    }

    fn is_multiline_block_opening(&self, lexeme: &Lexeme) -> bool {
        self.multiline_headers
            .iter()
            .any(|range| range.opening_brace == lexeme.span().start)
    }

    fn continuation_indentation(&self, lexeme: &Lexeme) -> usize {
        let offset = lexeme.span().start;
        let syntax_offset = if self.generic_closes.contains(&offset) {
            self.delimiters
                .iter()
                .find(|range| range.closing == offset)
                .map(|range| range.opening)
        } else {
            match token_kind(lexeme) {
                Some(TokenKind::RParen | TokenKind::RBracket) => self
                    .delimiters
                    .iter()
                    .find(|range| range.closing == offset)
                    .map(|range| range.opening),
                Some(TokenKind::RBrace) => None,
                _ => Some(offset),
            }
        };
        let syntax_continuation =
            syntax_offset.is_some_and(|offset| self.has_syntax_continuation(offset));
        let active_delimiters = self
            .delimiters
            .iter()
            .filter(|range| {
                self.brace_stack
                    .last()
                    .is_none_or(|brace| brace.opening < range.opening)
                    && offset < range.closing
                    && range
                        .direct_breaks
                        .iter()
                        .any(|break_offset| *break_offset <= offset)
            })
            .collect::<Vec<_>>();
        let delimiter_continuation = active_delimiters
            .iter()
            .map(|range| range.opening)
            .min()
            .map_or(0, |outermost_opening| {
                active_delimiters.len()
                    + usize::from(self.has_syntax_continuation(outermost_opening))
            });
        delimiter_continuation.max(usize::from(syntax_continuation))
    }

    fn has_syntax_continuation(&self, offset: usize) -> bool {
        self.multiline_headers
            .iter()
            .filter(|range| {
                self.brace_stack
                    .last()
                    .is_none_or(|brace| brace.opening < range.start)
                    && range.start < offset
                    && offset < range.opening_brace
            })
            .max_by_key(|range| range.start)
            .is_some_and(|range| line_breaks(&self.document.source()[range.start..offset]) > 0)
            || self.continuations.iter().any(|range| {
                self.brace_stack
                    .last()
                    .is_none_or(|brace| brace.opening < range.start)
                    && range.start < offset
                    && offset < range.end
                    && line_breaks(&self.document.source()[range.start..offset]) > 0
            })
    }
}

#[derive(Debug, Clone, Copy)]
struct SourceLine {
    start: usize,
    end: usize,
    cutoff: usize,
}

/// Applies width-driven breaks as a syntax-aware refinement of canonical
/// formatting. Each refinement inserts only legal soft breaks and then runs
/// the ordinary formatter again, so indentation, comments, delimiters, and
/// trailing punctuation continue to have one implementation.
fn wrap_long_lines(mut source: String, options: FormatOptions) -> String {
    let max_width = options.max_line_width.max(1);
    for _ in 0..128 {
        let overlong = overlong_lines(&source, max_width, options.indent_width.max(1));
        if overlong.is_empty() {
            return source;
        }
        let Ok(parsed) = crate::parse(&source) else {
            return source;
        };
        let document = parsed.source_document();
        let layout = SyntaxLayoutCollector::collect(document, parsed.syntax());
        let compact_braces = compact_brace_ranges(document, &layout);
        let insertions = width_break_insertions(document, &overlong, &compact_braces);
        if insertions.is_empty() {
            return source;
        }

        for offset in insertions.into_iter().rev() {
            source.insert(offset, '\n');
        }
        let Ok(parsed) = crate::parse(&source) else {
            return source;
        };
        source = Formatter::new(parsed.source_document(), parsed.syntax(), options).finish();
    }
    source
}

fn overlong_lines(source: &str, max_width: usize, tab_width: usize) -> Vec<SourceLine> {
    let mut lines = Vec::new();
    let mut start = 0usize;
    for line in source.split_inclusive('\n') {
        let content = line.strip_suffix('\n').unwrap_or(line);
        let mut width = 0usize;
        let mut cutoff = None;
        for (offset, character) in content.char_indices() {
            width = if character == '\t' {
                width.saturating_add(tab_width - width % tab_width)
            } else {
                width.saturating_add(1)
            };
            if cutoff.is_none() && width > max_width {
                cutoff = Some(start + offset);
            }
        }
        if let Some(cutoff) = cutoff {
            lines.push(SourceLine {
                start,
                end: start + content.len(),
                cutoff,
            });
        }
        start += line.len();
    }
    lines
}

#[derive(Debug, Clone, Copy)]
struct CompactBraceRange {
    opening: usize,
    closing: usize,
}

#[derive(Debug, Clone, Copy)]
struct WidthBreak {
    offset: usize,
    priority: u8,
}

fn compact_brace_ranges(
    document: &SourceDocument,
    layout: &SyntaxLayoutCollector<'_>,
) -> Vec<CompactBraceRange> {
    let mut stack = Vec::new();
    let mut ranges = Vec::new();
    for token in document.tokens() {
        match token.kind {
            TokenKind::LBrace => stack.push(token.span.start),
            TokenKind::RBrace => {
                let Some(opening) = stack.pop() else {
                    continue;
                };
                if layout.compact_brace_opens.contains(&opening)
                    && layout.compact_brace_closes.contains(&token.span.start)
                {
                    ranges.push(CompactBraceRange {
                        opening,
                        closing: token.span.start,
                    });
                }
            }
            _ => {}
        }
    }
    ranges
}

fn width_break_insertions(
    document: &SourceDocument,
    lines: &[SourceLine],
    compact_braces: &[CompactBraceRange],
) -> Vec<usize> {
    let mut insertions = HashSet::new();
    for line in lines {
        let mut candidates = compact_braces
            .iter()
            .copied()
            .filter(|range| line.start <= range.opening && range.closing <= line.end)
            .collect::<Vec<_>>();
        candidates.sort_by_key(|range| range.opening);
        let compact = candidates
            .iter()
            .copied()
            .find(|range| range.opening <= line.cutoff && line.cutoff <= range.closing)
            .or_else(|| candidates.last().copied());
        if let Some(compact) = compact
            && expand_compact_brace(document, compact, &mut insertions)
        {
            continue;
        }

        let candidates = general_width_breaks(document, *line);
        if let Some(candidate) = candidates
            .iter()
            .copied()
            .filter(|candidate| candidate.offset <= line.cutoff)
            .max_by_key(|candidate| (candidate.priority, candidate.offset))
            .or_else(|| {
                candidates.iter().copied().max_by_key(|candidate| {
                    (candidate.priority, std::cmp::Reverse(candidate.offset))
                })
            })
        {
            insertions.insert(candidate.offset);
        }
    }
    let mut insertions = insertions.into_iter().collect::<Vec<_>>();
    insertions.sort_unstable();
    insertions
}

fn expand_compact_brace(
    document: &SourceDocument,
    range: CompactBraceRange,
    insertions: &mut HashSet<usize>,
) -> bool {
    let tokens = document
        .tokens()
        .filter(|token| range.opening <= token.span.start && token.span.start <= range.closing)
        .collect::<Vec<_>>();
    if tokens.len() <= 2 {
        return false;
    }

    let mut parentheses = 0usize;
    let mut brackets = 0usize;
    let mut braces = 0usize;
    insertions.insert(tokens[0].span.end);
    insertions.insert(range.closing);
    for token in tokens.iter().skip(1).take(tokens.len().saturating_sub(2)) {
        match token.kind {
            TokenKind::LParen => parentheses += 1,
            TokenKind::RParen => parentheses = parentheses.saturating_sub(1),
            TokenKind::LBracket => brackets += 1,
            TokenKind::RBracket => brackets = brackets.saturating_sub(1),
            TokenKind::LBrace => braces += 1,
            TokenKind::RBrace => braces = braces.saturating_sub(1),
            TokenKind::Comma if parentheses == 0 && brackets == 0 && braces == 0 => {
                insertions.insert(token.span.end);
            }
            _ => {}
        }
    }
    true
}

fn general_width_breaks(document: &SourceDocument, line: SourceLine) -> Vec<WidthBreak> {
    let mut candidates = Vec::new();
    for token in document
        .tokens()
        .filter(|token| line.start < token.span.start && token.span.end <= line.end)
    {
        let candidate = match &token.kind {
            TokenKind::Comma | TokenKind::Assign => Some(WidthBreak {
                offset: token.span.end,
                priority: 5,
            }),
            TokenKind::OrOr | TokenKind::AndAnd => Some(WidthBreak {
                offset: token.span.start,
                priority: 4,
            }),
            TokenKind::Plus
            | TokenKind::Minus
            | TokenKind::Star
            | TokenKind::Slash
            | TokenKind::Percent
            | TokenKind::EqEq
            | TokenKind::BangEq
            | TokenKind::Lt
            | TokenKind::Le
            | TokenKind::Gt
            | TokenKind::Ge
            | TokenKind::Or
            | TokenKind::Caret
            | TokenKind::And
            | TokenKind::Shl
            | TokenKind::Shr => Some(WidthBreak {
                offset: token.span.start,
                priority: 3,
            }),
            TokenKind::Dot => Some(WidthBreak {
                offset: token.span.start,
                priority: 1,
            }),
            TokenKind::Ident(name) if name == "else" => Some(WidthBreak {
                offset: token.span.start,
                priority: 6,
            }),
            TokenKind::Ident(name) if matches!(name.as_str(), "as" | "is") => Some(WidthBreak {
                offset: token.span.start,
                priority: 4,
            }),
            _ => None,
        };
        if let Some(candidate) = candidate {
            candidates.push(candidate);
        }
    }
    candidates
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum DelimiterKind {
    Parenthesis,
    Bracket,
    Brace,
    Angle,
}

struct PendingDelimiter {
    kind: DelimiterKind,
    opening: usize,
    direct_breaks: Vec<usize>,
}

/// Splits maximal-munch operators at parser-confirmed type boundaries.
///
/// The lossless lexer must continue to produce `>=`, `>>`, `>>=`, and `!=` as
/// operators. Once parsing has identified source-accurate generic closers and
/// result postfixes, the formatter needs their logical token boundaries so
/// indentation, trailing punctuation, and mandatory spacing behave exactly as
/// if the source had separated the characters already.
fn formatting_lexemes(
    document: &SourceDocument,
    generic_closes: &HashSet<usize>,
    fallible_type_suffixes: &HashSet<usize>,
) -> Vec<Lexeme> {
    let mut output = Vec::with_capacity(document.lexemes().len());
    for lexeme in document.lexemes() {
        let Lexeme::Token(token) = lexeme else {
            output.push(lexeme.clone());
            continue;
        };
        if token.kind == TokenKind::BangEq && fallible_type_suffixes.contains(&token.span.start) {
            output.push(Lexeme::Token(Token {
                kind: TokenKind::Bang,
                span: crate::ast::Span {
                    start: token.span.start,
                    end: token.span.start + 1,
                },
            }));
            output.push(Lexeme::Token(Token {
                kind: TokenKind::Assign,
                span: crate::ast::Span {
                    start: token.span.start + 1,
                    end: token.span.end,
                },
            }));
            continue;
        }
        let close_count = (token.span.start..token.span.end)
            .take_while(|offset| generic_closes.contains(offset))
            .count();
        if close_count == 0 {
            output.push(lexeme.clone());
            continue;
        }

        for offset in token.span.start..token.span.start + close_count {
            output.push(Lexeme::Token(Token {
                kind: TokenKind::Gt,
                span: crate::ast::Span {
                    start: offset,
                    end: offset + 1,
                },
            }));
        }
        let residual_start = token.span.start + close_count;
        if residual_start == token.span.end {
            continue;
        }
        let kind = match &document.source()[residual_start..token.span.end] {
            "=" => TokenKind::Assign,
            ">" => TokenKind::Gt,
            ">=" => TokenKind::Ge,
            residual => unreachable!("unsupported generic-close residual `{residual}`"),
        };
        output.push(Lexeme::Token(Token {
            kind,
            span: crate::ast::Span {
                start: residual_start,
                end: token.span.end,
            },
        }));
    }
    coalesce_contextual_operators(output, generic_closes)
}

/// Rejoins operator characters that straddle a parser-confirmed type
/// boundary. The original lossless lexer sees `>==` as `>=` and `=`; after the
/// generic close is fissioned, the two adjacent assignment characters are the
/// equality operator belonging to the surrounding expression.
fn coalesce_contextual_operators(
    lexemes: Vec<Lexeme>,
    generic_closes: &HashSet<usize>,
) -> Vec<Lexeme> {
    let mut output: Vec<Lexeme> = Vec::with_capacity(lexemes.len());
    for lexeme in lexemes {
        if let Lexeme::Token(current) = &lexeme
            && !generic_closes.contains(&current.span.start)
            && let Some(Lexeme::Token(previous)) = output.last_mut()
            && !generic_closes.contains(&previous.span.start)
            && previous.span.end == current.span.start
        {
            let combined = match (&previous.kind, &current.kind) {
                (TokenKind::Assign, TokenKind::Assign) => Some(TokenKind::EqEq),
                (TokenKind::Gt, TokenKind::Gt) => Some(TokenKind::Shr),
                (TokenKind::Gt, TokenKind::Ge) => Some(TokenKind::ShrAssign),
                _ => None,
            };
            if let Some(kind) = combined {
                previous.kind = kind;
                previous.span.end = current.span.end;
                continue;
            }
        }
        output.push(lexeme);
    }
    output
}

fn delimiter_ranges(
    document: &SourceDocument,
    lexemes: &[Lexeme],
    generic_opens: &HashSet<usize>,
    generic_closes: &HashSet<usize>,
) -> Vec<DelimiterRange> {
    let mut stack = Vec::<PendingDelimiter>::new();
    let mut ranges = Vec::new();
    for lexeme in lexemes {
        match lexeme {
            Lexeme::Trivia(trivia) if trivia.kind == TriviaKind::Whitespace => {
                if line_breaks(document.text(trivia.span)) > 0
                    && let Some(delimiter) = stack.last_mut()
                {
                    delimiter.direct_breaks.push(trivia.span.end);
                }
            }
            Lexeme::Trivia(trivia) if trivia.kind == TriviaKind::BlockComment => {
                if line_breaks(document.text(trivia.span)) > 0
                    && let Some(delimiter) = stack.last_mut()
                {
                    delimiter.direct_breaks.push(trivia.span.end);
                }
            }
            Lexeme::Token(token) if generic_opens.contains(&token.span.start) => {
                stack.push(PendingDelimiter {
                    kind: DelimiterKind::Angle,
                    opening: token.span.start,
                    direct_breaks: Vec::new(),
                });
            }
            Lexeme::Token(token) if generic_closes.contains(&token.span.start) => {
                close_delimiter(
                    &mut stack,
                    &mut ranges,
                    DelimiterKind::Angle,
                    token.span.start,
                );
            }
            Lexeme::Token(token) => match token.kind {
                TokenKind::LParen => stack.push(PendingDelimiter {
                    kind: DelimiterKind::Parenthesis,
                    opening: token.span.start,
                    direct_breaks: Vec::new(),
                }),
                TokenKind::LBracket => stack.push(PendingDelimiter {
                    kind: DelimiterKind::Bracket,
                    opening: token.span.start,
                    direct_breaks: Vec::new(),
                }),
                TokenKind::RParen => close_delimiter(
                    &mut stack,
                    &mut ranges,
                    DelimiterKind::Parenthesis,
                    token.span.start,
                ),
                TokenKind::RBracket => close_delimiter(
                    &mut stack,
                    &mut ranges,
                    DelimiterKind::Bracket,
                    token.span.start,
                ),
                _ => {}
            },
            Lexeme::Trivia(_) => {}
        }
    }
    ranges
}

#[derive(Default)]
struct TrailingPunctuation {
    commas: HashSet<usize>,
    semicolons: HashSet<usize>,
    omitted_commas: HashSet<usize>,
}

fn trailing_list_punctuation(
    document: &SourceDocument,
    lexemes: &[Lexeme],
    syntax: &Program,
    break_after: &HashSet<usize>,
    generic_opens: &HashSet<usize>,
    generic_closes: &HashSet<usize>,
) -> TrailingPunctuation {
    #[derive(Debug)]
    struct ListDelimiter {
        kind: DelimiterKind,
        has_direct_break: bool,
        has_direct_comma: bool,
        last_direct_comma: Option<usize>,
        pending_break_after_comma: bool,
    }

    let mut stack = Vec::<ListDelimiter>::new();
    let mut closings = HashSet::new();
    let mut omitted_commas = HashSet::new();
    for lexeme in lexemes {
        match lexeme {
            Lexeme::Trivia(trivia)
                if matches!(
                    trivia.kind,
                    TriviaKind::Whitespace | TriviaKind::BlockComment
                ) && line_breaks(document.text(trivia.span)) > 0 =>
            {
                if let Some(delimiter) = stack.last_mut()
                    && delimiter.last_direct_comma.is_some()
                {
                    delimiter.pending_break_after_comma = true;
                }
            }
            Lexeme::Token(token) => {
                let opening = if generic_opens.contains(&token.span.start) {
                    Some(DelimiterKind::Angle)
                } else {
                    match token.kind {
                        TokenKind::LParen => Some(DelimiterKind::Parenthesis),
                        TokenKind::LBracket => Some(DelimiterKind::Bracket),
                        TokenKind::LBrace => Some(DelimiterKind::Brace),
                        _ => None,
                    }
                };
                if let Some(kind) = opening {
                    if let Some(parent) = stack.last_mut() {
                        parent.has_direct_break |= parent.pending_break_after_comma;
                        parent.pending_break_after_comma = false;
                        parent.last_direct_comma = None;
                    }
                    stack.push(ListDelimiter {
                        kind,
                        has_direct_break: false,
                        has_direct_comma: false,
                        last_direct_comma: None,
                        pending_break_after_comma: false,
                    });
                    continue;
                }
                if token.kind == TokenKind::Comma {
                    if let Some(delimiter) = stack.last_mut() {
                        delimiter.has_direct_comma = true;
                        delimiter.has_direct_break |= break_after.contains(&token.span.end);
                        delimiter.last_direct_comma = Some(token.span.start);
                        delimiter.pending_break_after_comma = false;
                    }
                    continue;
                }
                let closing = if generic_closes.contains(&token.span.start) {
                    Some(DelimiterKind::Angle)
                } else {
                    match token.kind {
                        TokenKind::RParen => Some(DelimiterKind::Parenthesis),
                        TokenKind::RBracket => Some(DelimiterKind::Bracket),
                        TokenKind::RBrace => Some(DelimiterKind::Brace),
                        _ => None,
                    }
                };
                if let Some(kind) = closing
                    && let Some(delimiter) = stack.pop()
                    && delimiter.kind == kind
                {
                    if delimiter.has_direct_break && delimiter.has_direct_comma {
                        closings.insert(token.span.start);
                    } else if !delimiter.has_direct_break
                        && let Some(comma) = delimiter.last_direct_comma
                    {
                        omitted_commas.insert(comma);
                    }
                    if let Some(parent) = stack.last_mut() {
                        parent.last_direct_comma = None;
                    }
                    continue;
                }
                if let Some(delimiter) = stack.last_mut() {
                    delimiter.has_direct_break |= delimiter.pending_break_after_comma;
                    delimiter.pending_break_after_comma = false;
                    delimiter.last_direct_comma = None;
                }
            }
            Lexeme::Trivia(_) => {}
        }
    }
    let mut collector = TrailingPunctuationCollector {
        document,
        punctuation: TrailingPunctuation {
            commas: closings,
            semicolons: HashSet::new(),
            omitted_commas,
        },
    };
    collector.visit_program(syntax);
    collector.punctuation
}

struct TrailingPunctuationCollector<'a> {
    document: &'a SourceDocument,
    punctuation: TrailingPunctuation,
}

impl TrailingPunctuationCollector<'_> {
    fn mark_comma(&mut self, span: crate::ast::Span) {
        if let Some(closing) = nonempty_closing_brace(self.document, span) {
            self.punctuation.commas.insert(closing);
        }
    }

    fn mark_semicolon(&mut self, span: crate::ast::Span) {
        if let Some(closing) = nonempty_closing_brace(self.document, span) {
            self.punctuation.commas.remove(&closing);
            self.punctuation.semicolons.insert(closing);
        }
    }

    fn mark_comma_for_items(
        &mut self,
        span: crate::ast::Span,
        items: impl IntoIterator<Item = crate::ast::Span>,
        single_item_is_vertical: bool,
    ) {
        let items = items.into_iter().collect::<Vec<_>>();
        let itemized = (items.len() == 1
            && (single_item_is_vertical
                || line_breaks(&self.document.source()[span.start..span.end]) > 0))
            || items
                .windows(2)
                .any(|pair| line_breaks(&self.document.source()[pair[0].end..pair[1].start]) > 0);
        if itemized {
            self.mark_comma(span);
            return;
        }
        if let Some(closing) = nonempty_closing_brace(self.document, span) {
            self.punctuation.commas.remove(&closing);
            if let Some(comma) = trailing_comma_before(self.document, span, closing) {
                self.punctuation.omitted_commas.insert(comma);
            }
        }
    }
}

impl<'ast> Visitor<'ast> for TrailingPunctuationCollector<'_> {
    fn visit_type_application(&mut self, application: &'ast TypeApplicationDecl) {
        for occurrence in &application.occurrences {
            if line_breaks(
                &self.document.source()[occurrence.opening.end..occurrence.closing.start],
            ) > 0
            {
                self.punctuation.commas.insert(occurrence.closing.start);
            } else if let Some(comma) =
                trailing_comma_before(self.document, occurrence.span, occurrence.closing.start)
            {
                self.punctuation.omitted_commas.insert(comma);
            }
        }
        for argument in &application.arguments {
            self.visit_type_ref(argument);
        }
    }

    fn visit_program(&mut self, program: &'ast Program) {
        if let Some(policy) = program.tick_rate
            && (policy.attached.is_some() || policy.detached.is_some())
        {
            self.mark_comma(policy.span);
        }
        if !program.settings.is_empty()
            && let Some(span) = program.settings_span
        {
            self.mark_comma(span);
        }
        visit::walk_program(self, program);
    }

    fn visit_state(&mut self, state: &'ast StateDecl) {
        if !state.provider_alternatives.is_empty() {
            self.mark_comma(state.span);
            for alternative in &state.provider_alternatives {
                if !alternative.fields.is_empty() {
                    self.mark_semicolon(alternative.span);
                }
                for field in &alternative.fields {
                    self.visit_state_field(field);
                }
            }
        }
        let last_field = state.fields.iter().map(|field| field.span.end).max();
        let last_group = state
            .conditional_fields
            .iter()
            .map(|group| group.span.end)
            .max();
        if last_field.is_some_and(|field| last_group.is_none_or(|group| field > group)) {
            self.mark_semicolon(state.span);
        }
        for field in &state.fields {
            self.visit_state_field(field);
        }
        for group in &state.conditional_fields {
            if !group.fields.is_empty() {
                self.mark_semicolon(group.span);
            }
            if let Some(condition) = &group.condition {
                self.visit_expr(condition);
            }
            for field in &group.fields {
                self.visit_state_field(field);
            }
        }
    }

    fn visit_struct(&mut self, structure: &'ast StructDecl) {
        if !structure.fields.is_empty() {
            self.mark_comma(structure.span);
        }
        visit::walk_struct(self, structure);
    }

    fn visit_enum(&mut self, enumeration: &'ast EnumDecl) {
        if !enumeration.variants.is_empty() {
            self.mark_comma(enumeration.span);
        }
        visit::walk_enum(self, enumeration);
    }

    fn visit_setting(&mut self, setting: &'ast SettingDecl) {
        match &setting.kind {
            SettingKind::Title { .. } => {
                self.mark_comma(setting.span);
            }
            SettingKind::Choice { options, .. } if !options.is_empty() => {
                self.mark_comma(setting.span);
            }
            SettingKind::File { filters, .. } if !filters.is_empty() => {
                self.mark_comma(setting.span);
            }
            SettingKind::Bool { .. }
            | SettingKind::Text { .. }
            | SettingKind::Choice { .. }
            | SettingKind::File { .. } => {}
        }
        visit::walk_setting(self, setting);
    }

    fn visit_setting_family(&mut self, family: &'ast SettingFamilyDecl) {
        self.mark_comma(family.span);
        visit::walk_setting_family(self, family);
    }

    fn visit_expr(&mut self, expression: &'ast Expr) {
        match &expression.kind {
            ExprKind::Struct { fields, .. } if !fields.is_empty() => {
                self.mark_comma_for_items(
                    expression.span,
                    fields.iter().map(|field| field.value.span),
                    false,
                );
            }
            ExprKind::Match { arms, .. } if !arms.is_empty() => {
                self.mark_comma(expression.span);
            }
            _ => {}
        }
        visit::walk_expr(self, expression);
    }
}

fn nonempty_closing_brace(document: &SourceDocument, span: crate::ast::Span) -> Option<usize> {
    let tokens = document
        .tokens()
        .filter(|token| span.start <= token.span.start && token.span.end <= span.end)
        .collect::<Vec<_>>();
    let closing_index = tokens
        .iter()
        .rposition(|token| token.kind == TokenKind::RBrace)?;
    let mut depth = 0usize;
    let mut opening_index = None;
    for index in (0..closing_index).rev() {
        match tokens[index].kind {
            TokenKind::RBrace => depth += 1,
            TokenKind::LBrace if depth == 0 => {
                opening_index = Some(index);
                break;
            }
            TokenKind::LBrace => depth -= 1,
            _ => {}
        }
    }
    let opening_index = opening_index?;
    (opening_index + 1 < closing_index).then_some(tokens[closing_index].span.start)
}

fn trailing_comma_before(
    document: &SourceDocument,
    span: crate::ast::Span,
    closing: usize,
) -> Option<usize> {
    document
        .tokens()
        .take_while(|token| token.span.start < closing)
        .filter(|token| span.start <= token.span.start)
        .last()
        .filter(|token| token.kind == TokenKind::Comma)
        .map(|token| token.span.start)
}

fn close_delimiter(
    stack: &mut Vec<PendingDelimiter>,
    ranges: &mut Vec<DelimiterRange>,
    expected: DelimiterKind,
    closing: usize,
) {
    let Some(delimiter) = stack.pop() else {
        return;
    };
    if delimiter.kind == expected && !delimiter.direct_breaks.is_empty() {
        ranges.push(DelimiterRange {
            opening: delimiter.opening,
            closing,
            direct_breaks: delimiter.direct_breaks,
        });
    }
}

fn token_kind(lexeme: &Lexeme) -> Option<&TokenKind> {
    match lexeme {
        Lexeme::Token(token) => Some(&token.kind),
        Lexeme::Trivia(_) => None,
    }
}

fn is_comment(lexeme: &Lexeme) -> bool {
    matches!(
        lexeme,
        Lexeme::Trivia(trivia)
            if matches!(trivia.kind, TriviaKind::LineComment | TriviaKind::BlockComment)
    )
}

fn is_line_comment(lexeme: &Lexeme) -> bool {
    matches!(lexeme, Lexeme::Trivia(trivia) if trivia.kind == TriviaKind::LineComment)
}

fn is_doc_comment(lexeme: &Lexeme) -> bool {
    matches!(token_kind(lexeme), Some(TokenKind::DocComment(_)))
}

fn line_breaks(text: &str) -> usize {
    let bytes = text.as_bytes();
    let mut count = 0;
    let mut index = 0;
    while index < bytes.len() {
        match bytes[index] {
            b'\n' => count += 1,
            b'\r' => {
                count += 1;
                if bytes.get(index + 1) == Some(&b'\n') {
                    index += 1;
                }
            }
            _ => {}
        }
        index += 1;
    }
    count
}

fn needs_space(
    previous: Option<&TokenKind>,
    current: Option<&TokenKind>,
    previous_was_prefix: bool,
    previous_was_generic_close: bool,
    generic_depth: usize,
) -> bool {
    let (Some(previous), Some(current)) = (previous, current) else {
        return false;
    };

    if matches!(previous, TokenKind::Minus) && matches!(current, TokenKind::Gt) {
        return false;
    }
    if matches!(previous, TokenKind::DotDotLt | TokenKind::DotDotEq)
        || matches!(current, TokenKind::DotDotLt | TokenKind::DotDotEq)
    {
        return false;
    }
    if matches!(current, TokenKind::DotDot) {
        return matches!(previous, TokenKind::Comma);
    }
    if matches!(previous, TokenKind::DotDot) {
        return false;
    }
    if matches!(previous, TokenKind::Ident(name) if matches!(name.as_str(), "sig" | "v"))
        && matches!(current, TokenKind::String(_))
    {
        return false;
    }
    if generic_depth > 0 && (matches!(previous, TokenKind::Lt) || matches!(current, TokenKind::Gt))
    {
        return false;
    }
    // A block opening is separated from its header, but a brace nested directly
    // inside another delimiter is its operand rather than a new header. This
    // covers ordinary block expressions and anonymous struct patterns such as
    // `Possible.Value({ x, y })` without weakening `if value { ... }`.
    if matches!(current, TokenKind::LBrace) {
        return !matches!(
            previous,
            TokenKind::LParen | TokenKind::LBracket | TokenKind::TemplateExprStart
        );
    }
    if previous_was_prefix {
        return false;
    }
    if previous_was_generic_close && matches!(current, TokenKind::Bang | TokenKind::Question) {
        return false;
    }

    if matches!(
        current,
        TokenKind::RParen
            | TokenKind::RBrace
            | TokenKind::RBracket
            | TokenKind::Comma
            | TokenKind::Semicolon
            | TokenKind::Dot
            | TokenKind::Colon
            | TokenKind::Question
            | TokenKind::TemplateChunk(_)
            | TokenKind::TemplateExprStart
            | TokenKind::TemplateExprEnd
            | TokenKind::TemplateEnd
    ) || matches!(
        previous,
        TokenKind::LParen
            | TokenKind::LBracket
            | TokenKind::Dot
            | TokenKind::TemplateStart
            | TokenKind::TemplateChunk(_)
            | TokenKind::TemplateExprStart
            | TokenKind::TemplateExprEnd
    ) {
        return false;
    }

    if matches!(current, TokenKind::LParen) {
        return matches!(previous, TokenKind::Colon | TokenKind::Comma)
            || matches!(previous, TokenKind::Ident(name) if is_prefix_keyword(name))
            || (!previous_was_generic_close && is_spaced_operator(previous));
    }
    if matches!(current, TokenKind::LBracket) {
        return true;
    }
    if matches!(previous, TokenKind::Comma | TokenKind::Colon) {
        return true;
    }
    if matches!(current, TokenKind::Bang | TokenKind::Tilde) {
        return matches!(previous, TokenKind::Ident(name) if is_prefix_keyword(name))
            || is_spaced_operator(previous);
    }
    if matches!(current, TokenKind::Minus) && is_prefix_boundary(previous) {
        return !matches!(
            previous,
            TokenKind::LParen
                | TokenKind::LBracket
                | TokenKind::LBrace
                | TokenKind::Comma
                | TokenKind::Colon
                | TokenKind::TemplateExprStart
        );
    }
    if is_spaced_operator(previous) || is_spaced_operator(current) {
        return true;
    }

    true
}

fn is_prefix_keyword(name: &str) -> bool {
    matches!(
        name,
        "if" | "while" | "match" | "break" | "return" | "throw" | "else" | "await" | "retry"
    )
}

fn is_prefix_operator(current: &TokenKind, previous: Option<&TokenKind>) -> bool {
    matches!(current, TokenKind::Bang | TokenKind::Tilde)
        && previous.is_none_or(|previous| {
            is_prefix_boundary(previous)
                || matches!(previous, TokenKind::Ident(name) if is_prefix_keyword(name))
        })
        || matches!(current, TokenKind::Minus)
            && previous.is_none_or(|previous| {
                is_prefix_boundary(previous)
                    || matches!(previous, TokenKind::Ident(name) if is_prefix_keyword(name))
            })
}

fn is_prefix_boundary(token: &TokenKind) -> bool {
    matches!(
        token,
        TokenKind::LParen
            | TokenKind::LBracket
            | TokenKind::LBrace
            | TokenKind::Comma
            | TokenKind::Colon
            | TokenKind::FatArrow
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
            | TokenKind::TemplateExprStart
    )
}

fn is_spaced_operator(token: &TokenKind) -> bool {
    matches!(
        token,
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
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn canonicalizes_basic_declarations_statements_and_expressions() {
        let source = r#"state "game.exe"{}
fn add(left:i32,right:i32)->i32{let sum=left+right;if sum>10{return sum}else{return 10}}
whileAttached{let answer=add(1,2);print(answer as String)}"#;
        let expected = r#"state "game.exe" {}
fn add(left: i32, right: i32) -> i32 {
    let sum = left + right;
    if sum > 10 {
        return sum
    } else {
        return 10
    }
}
whileAttached {
    let answer = add(1, 2);
    print(answer as String)
}
"#;

        let formatted = format_source(source).unwrap();
        assert_eq!(formatted, expected);
        assert_eq!(format_source(&formatted).unwrap(), formatted);
        crate::check(crate::lower(crate::parse(&formatted).unwrap())).unwrap();
    }

    #[test]
    fn formats_bare_attachment_globals_without_inventing_initializers() {
        let source = "let module\nlet base:address\nstate\"game.exe\"{}\nonAttach{module=await process.mainModule()\nbase=module.address}";
        let expected = r#"let module
let base: address
state "game.exe" {}
onAttach {
    module = await process.mainModule()
    base = module.address
}
"#;
        let formatted = format_source(source).unwrap();
        assert_eq!(formatted, expected);
        assert_eq!(format_source(&formatted).unwrap(), formatted);
    }

    #[test]
    fn formats_process_selection_as_an_ordinary_lifecycle_block() {
        let source = r#"state Unity["game.exe"]{}
selectProcess{let path=process.path()?;return path.endsWith("/wanted/game.exe")}"#;
        let expected = r#"state Unity ["game.exe"] {}
selectProcess {
    let path = process.path()?;
    return path.endsWith("/wanted/game.exe")
}
"#;

        let formatted = format_source(source).unwrap();
        assert_eq!(formatted, expected);
        assert_eq!(format_source(&formatted).unwrap(), formatted);
    }

    #[test]
    fn formats_managed_schemas_as_vertical_semicolon_delimited_declarations() {
        let source = r#"enum Edition{Base,DlcDemo}
let edition:Edition
state"game.exe"{}
onAttach{edition=Edition.Base}
image"Assembly-CSharp"{namespace Game{class Player from"RuntimePlayer"{f32 health;}}class GameManager{static GameManager instance from["Instance","_instance",];i32 points from"_points";if edition==Edition.Base{i32 gameState;i32 currentLevel;}if edition==Edition.DlcDemo{i32 gameState from"GameState";String currentScene from"_currentScene" maxLength 64;}}}
fn identity(value:GameManager.Ref)->GameManager.Ref{return value}"#;
        let expected = r#"enum Edition {
    Base,
    DlcDemo,
}
let edition: Edition
state "game.exe" {}
onAttach {
    edition = Edition.Base
}
image "Assembly-CSharp" {
    namespace Game {
        class Player from "RuntimePlayer" {
            f32 health;
        }
    }
    class GameManager {
        static GameManager instance from ["Instance", "_instance"];
        i32 points from "_points";
        if edition == Edition.Base {
            i32 gameState;
            i32 currentLevel;
        }
        if edition == Edition.DlcDemo {
            i32 gameState from "GameState";
            String currentScene from "_currentScene" maxLength 64;
        }
    }
}
fn identity(value: GameManager.Ref) -> GameManager.Ref {
    return value
}
"#;
        let formatted = format_source(source).unwrap();
        assert_eq!(formatted, expected);
        assert_eq!(format_source(&formatted).unwrap(), formatted);
    }

    #[test]
    fn formats_callable_types_and_arrow_closures() {
        let source = r#"state "game.exe"{}
fn apply(value:u32,transform:(u32)->u32)->u32{return transform(value)}
whileAttached{let add=(left:u32,right:u32)->u32=>{left+right};let result=add(1,2);print(result)}"#;
        let expected = r#"state "game.exe" {}
fn apply(value: u32, transform: (u32) -> u32) -> u32 {
    return transform(value)
}
whileAttached {
    let add = (left: u32, right: u32) -> u32 => {
        left + right
    };
    let result = add(1, 2);
    print(result)
}
"#;
        let formatted = format_source(source).unwrap();
        assert_eq!(formatted, expected);
        assert_eq!(format_source(&formatted).unwrap(), formatted);
        crate::check(crate::lower(crate::parse(&formatted).unwrap())).unwrap();
    }

    #[test]
    fn formats_irrefutable_patterns_at_every_binding_site() {
        let source = r#"struct Point{x:i32,y:i32}
let Point{x:globalX,y:globalY}=Point{x:1,y:2}
state "game.exe"{}
fn sum(Point{x,y}:Point)->i32{return x+y}
whileAttached{let Point{x:localX,y:localY}=Point{x:globalX,y:globalY};let add=(Point{x,y}:Point)=>x+y;for Point{x:itemX,y:_} in [Point{x:localX,y:localY}]{print(add(Point{x:itemX,y:0}))}}"#;
        let formatted = format_source(source).expect("binding patterns should format");
        for fragment in [
            "x: globalX, y: globalY",
            "x, y",
            "}: Point) -> i32",
            "}: Point) => x + y",
            "x: itemX, y: _",
        ] {
            assert!(formatted.contains(fragment), "{fragment}:\n{formatted}");
        }
        assert_eq!(format_source(&formatted).unwrap(), formatted);
        crate::compile(&formatted).expect("formatted binding patterns should compile");
    }

    #[test]
    fn joins_anonymous_struct_patterns_to_their_wrapper_delimiter() {
        let source = r#"struct Pos{x:u16,y:u16}
enum Possible{Value(Pos),Impossible(Never)}
state "game.exe"{}
fn unwrap(Possible.Value( { x,y } ):Possible)->u16{return x+y}"#;
        let expected = r#"struct Pos {
    x: u16,
    y: u16,
}
enum Possible {
    Value(Pos),
    Impossible(Never),
}
state "game.exe" {}
fn unwrap(Possible.Value({ x, y }): Possible) -> u16 {
    return x + y
}
"#;

        let formatted = format_source(source).unwrap();
        assert_eq!(formatted, expected);
        assert_eq!(format_source(&formatted).unwrap(), formatted);
        crate::compile(&formatted).expect("the formatted wrapper pattern should compile");
    }

    #[test]
    fn formats_value_loops_and_break_values() {
        let source = r#"state "game.exe"{}
fn choose(flag:bool)->i32{return loop{if flag{break 7}else{break -1}}}"#;
        let expected = r#"state "game.exe" {}
fn choose(flag: bool) -> i32 {
    return loop {
        if flag {
            break 7
        } else {
            break -1
        }
    }
}
"#;
        let formatted = format_source(source).unwrap();
        assert_eq!(formatted, expected);
        assert_eq!(format_source(&formatted).unwrap(), formatted);
        crate::compile(&formatted).expect("formatted value loops should compile");
    }

    #[test]
    fn uses_trailing_commas_only_for_multiline_lists() {
        let source = r#"struct Point { x: i32, y: i32, }
enum Mode { First, Second, }
state "game.exe" { value = 1 }
fn compact() { print(1, 2,) }
fn multiline() {
    print(
        1,
        2
    )
    let values = [
        1,
        2
    ]
}"#;

        let formatted = format_source(source).unwrap();
        assert!(
            formatted.contains("struct Point {\n    x: i32,\n    y: i32,\n}"),
            "{formatted}"
        );
        assert!(
            formatted.contains("enum Mode {\n    First,\n    Second,\n}"),
            "{formatted}"
        );
        assert!(formatted.contains("print(1, 2)"), "{formatted}");
        assert!(formatted.contains("        2,\n    )"), "{formatted}");
        assert!(formatted.contains("        2,\n    ]"), "{formatted}");
        assert_eq!(format_source(&formatted).unwrap(), formatted);
    }

    #[test]
    fn formats_process_readable_enum_representations_and_discriminants() {
        let source = "enum GameState:i32{Mission=0,TitleScreen,Results=6,}\nstate \"game.exe\"{}";
        let expected = r#"enum GameState: i32 {
    Mission = 0,
    TitleScreen,
    Results = 6,
}
state "game.exe" {}
"#;
        let formatted = format_source(source).unwrap();
        assert_eq!(formatted, expected);
        assert_eq!(format_source(&formatted).unwrap(), formatted);
    }

    #[test]
    fn keeps_literal_spellings_templates_signatures_and_type_postfixes() {
        let source = r#"state "game.exe"{bytes:[u8;6] at 0x100}
fn probe(value:[u8]!)->String{let missing:[u8]?=None;let outcome:i32! = Err("missing");let fixed:[u8;3]=[1,2,3];let sigValue=sig"48 8B ??";let version=v"1.2.3.4";return `{value.length()}:{fixed.length()}:{missing==None}:{sigValue as String}`}
fn nested()->[[u8]]{return [[1]]}
fn fallible()->f32!{return Err("missing")}
fn optional()->f32?{return None}"#;
        let source = format!(
            "{source}\nfn asynchronous()->async f32{{return 1.0}}\nlet subnormal:f32=1e-45"
        );
        let formatted = format_source(&source).unwrap();

        assert!(formatted.contains("bytes: [u8; 6] at 0x100"), "{formatted}");
        assert!(
            formatted.contains("fixed: [u8; 3] = [1, 2, 3]"),
            "{formatted}"
        );
        assert!(formatted.contains("value: [u8]!"), "{formatted}");
        assert!(formatted.contains("missing: [u8]? = None"));
        assert!(formatted.contains("outcome: i32! = Err(\"missing\")"));
        assert!(!formatted.contains("i32!="));
        assert!(formatted.contains("fn nested() -> [[u8]] {"), "{formatted}");
        assert!(formatted.contains("fn fallible() -> f32! {"), "{formatted}");
        assert!(formatted.contains("fn optional() -> f32? {"), "{formatted}");
        assert!(
            formatted.contains("fn asynchronous() -> async f32 {"),
            "{formatted}"
        );
        assert!(!formatted.contains("f32!{"), "{formatted}");
        assert!(!formatted.contains("f32?{"), "{formatted}");
        assert!(formatted.contains("sig\"48 8B ??\""));
        assert!(formatted.contains("v\"1.2.3.4\""));
        assert!(formatted.contains("let subnormal: f32 = 1e-45"));
        assert!(formatted.contains("`{value.length()}:"));
        assert_eq!(format_source(&formatted).unwrap(), formatted);
    }

    #[test]
    fn joins_array_indexes_to_their_receiver() {
        let source = r#"state "game.exe" {}
fn select(matrix: [[i32]], row: u32, column: u32) -> i32 {
    return matrix [ row ] [ column ]
}"#;
        let formatted = format_source(source).unwrap();
        assert!(
            formatted.contains("return matrix[row][column]"),
            "{formatted}"
        );
        assert_eq!(format_source(&formatted).unwrap(), formatted);
        crate::check(crate::lower(crate::parse(&formatted).unwrap())).unwrap();
    }

    #[test]
    fn formats_indexed_assignment_as_an_ordinary_assignment() {
        let source = r#"state "game.exe"{}
whileAttached{let values=[1,2]
values [ 1 ]=9
values [ 0 ] += 2}"#;
        let expected = r#"state "game.exe" {}
whileAttached {
    let values = [1, 2]
    values[1] = 9
    values[0] += 2
}
"#;
        let formatted = format_source(source).unwrap();
        assert_eq!(formatted, expected);
        assert_eq!(format_source(&formatted).unwrap(), formatted);
        crate::check(crate::lower(crate::parse(&formatted).unwrap())).unwrap();
    }

    #[test]
    fn formats_current_state_field_assignment_as_an_ordinary_assignment() {
        let source = r#"state "game.exe"{scene:i32 at 0x1000}
whileAttached{current . scene=old . scene
current.scene +=1}"#;
        let expected = r#"state "game.exe" {
    scene: i32 at 0x1000;
}
whileAttached {
    current.scene = old.scene
    current.scene += 1
}
"#;
        let formatted = format_source(source).unwrap();
        assert_eq!(formatted, expected);
        assert_eq!(format_source(&formatted).unwrap(), formatted);
        crate::compile(&formatted).expect("formatted current-state assignments should compile");
    }

    #[test]
    fn formats_bounded_state_string_decoders_as_part_of_the_pointer_path() {
        let source = r#"state "game.exe"{mapName at "game.dll",0x100,-0x20 as utf8(64);chapterName at 0xffff_ffff_ffff_fff0,-0x10 as utf16le(32)}"#;
        let expected = r#"state "game.exe" {
    mapName at "game.dll", 0x100, -0x20 as utf8(64);
    chapterName at 0xffff_ffff_ffff_fff0, -0x10 as utf16le(32);
}
"#;

        let formatted = format_source(source).unwrap();
        assert_eq!(formatted, expected);
        assert_eq!(format_source(&formatted).unwrap(), formatted);
    }

    #[test]
    fn formats_dynamic_state_pointer_bases() {
        let source = r#"state "game.exe"{value:u32 at base,0x20;base:address=0x1000}"#;
        let expected = r#"state "game.exe" {
    value: u32 at base, 0x20;
    base: address = 0x1000;
}
"#;

        let formatted = format_source(source).unwrap();
        assert_eq!(formatted, expected);
        assert_eq!(format_source(&formatted).unwrap(), formatted);
        crate::compile(&formatted).expect("formatted dynamic state bases should compile");
    }

    #[test]
    fn formats_state_field_filters_as_part_of_the_field() {
        let source = r#"state "game.exe"{scene:i32 at 0x1000 if value==7||value==8{Err("transient")}else{value};entities:i32 at 0x2000}"#;
        let expected = r#"state "game.exe" {
    scene: i32 at 0x1000 if value == 7 || value == 8 {
        Err("transient")
    } else {
        value
    };
    entities: i32 at 0x2000;
}
"#;

        let formatted = format_source(source).unwrap();
        assert_eq!(formatted, expected);
        assert_eq!(format_source(&formatted).unwrap(), formatted);
        crate::compile(&formatted).expect("formatted state field filters should compile");
    }

    #[test]
    fn returns_parser_diagnostics_for_invalid_source() {
        let errors = format_source("state \"game.exe\" { let broken = }").unwrap_err();
        assert!(!errors.is_empty());
    }

    #[test]
    fn preserves_ordinary_and_setting_documentation_comments() {
        let source = "  // heading\r\nstate \"game.exe\"{} // process\r\n\r\nsettings{\r\n/// first line\r\n/// second line\r\n\"Enabled\"=>enabled key \"legacy-enabled\":true // setting\r\n}\r\n/* footer */";
        let expected = r#"// heading
state "game.exe" {} // process

settings {
    /// first line
    /// second line
    "Enabled" => enabled key "legacy-enabled": true // setting
}
/* footer */
"#;

        let formatted = format_source(source).unwrap();
        assert_eq!(formatted, expected);
        assert_eq!(format_source(&formatted).unwrap(), formatted);
    }

    #[test]
    fn formats_compile_time_setting_families() {
        let source = r#"state "game.exe"{}
settings{"Levels"{/// One switch per level.
for level in 2 ..= 36{`Level {level}` key `{level}`:true}}}"#;
        let expected = r#"state "game.exe" {}
settings {
    "Levels" {
        /// One switch per level.
        for level in 2..=36 {
            `Level {level}` key `{level}`: true,
        },
    },
}
"#;
        let formatted = format_source(source).unwrap();
        assert_eq!(formatted, expected);
        assert_eq!(format_source(&formatted).unwrap(), formatted);
        crate::compile(&formatted).expect("formatted settings families should compile");
    }

    #[test]
    fn preserves_source_declaration_documentation_comments() {
        let source = r#"state "game.exe"{
/// Current stage.
stage:i32 at 0x100
}
/// Shared counter.
let count=0
/// Coordinate pair.
struct Point{
/// Horizontal coordinate.
x:i32
}
/// Converts a point.
fn describe(point:Point){return point.x as String}
"#;
        let expected = r#"state "game.exe" {
    /// Current stage.
    stage: i32 at 0x100;
}
/// Shared counter.
let count = 0
/// Coordinate pair.
struct Point {
    /// Horizontal coordinate.
    x: i32,
}
/// Converts a point.
fn describe(point: Point) {
    return point.x as String
}
"#;
        let formatted = format_source(source).unwrap();
        assert_eq!(formatted, expected);
        assert_eq!(format_source(&formatted).unwrap(), formatted);
    }

    #[test]
    fn puts_a_multiline_header_brace_at_the_header_indentation() {
        let source = r#"state "game.exe" {}
split {
    if old.timerStopped != current.timerStopped
        && !current.timerStopped
        && isFirstLevel(current.levelOrScene) {
        return true
    }

}"#;
        let expected = r#"state "game.exe" {}
split {
    if old.timerStopped != current.timerStopped
        && !current.timerStopped
        && isFirstLevel(current.levelOrScene)
    {
        return true
    }
}
"#;

        let formatted = format_source(source).unwrap();
        assert_eq!(formatted, expected);
        assert_eq!(format_source(&formatted).unwrap(), formatted);
    }

    #[test]
    fn formats_for_headers_and_bodies_idempotently() {
        let source = r#"state "game.exe"{}
whileAttached{for value in [1,2,3]{if value==2{continue}print(value as String)}}"#;
        let expected = r#"state "game.exe" {}
whileAttached {
    for value in [1, 2, 3] {
        if value == 2 {
            continue
        }
        print(value as String)
    }
}
"#;
        let formatted = format_source(source).unwrap();
        assert_eq!(formatted, expected);
        assert_eq!(format_source(&formatted).unwrap(), formatted);
    }

    #[test]
    fn keeps_range_operators_compact_in_values_and_types() {
        let source = r#"state "game.exe"{}
fn visit(values:u16 ..< u16?){for value in values else 0u16 ..< 0{print(value)}}
fn classify(value:i16)->bool{return match value{-10 ..< 0|10 ..= 20=>true,_=>false}}
whileAttached{let values=1u16 ..= 3;visit(0u16+1 ..< 8/2);for value in values{print(value)}}"#;
        let expected = r#"state "game.exe" {}
fn visit(values: u16..<u16?) {
    for value in values else 0u16..<0 {
        print(value)
    }
}
fn classify(value: i16) -> bool {
    return match value {
        -10..<0 | 10..=20 => true,
        _ => false,
    }
}
whileAttached {
    let values = 1u16..=3;
    visit(0u16 + 1..<8 / 2);
    for value in values {
        print(value)
    }
}
"#;
        let formatted = format_source(source).unwrap();
        assert_eq!(formatted, expected);
        assert_eq!(format_source(&formatted).unwrap(), formatted);
    }

    #[test]
    fn preserves_intentional_blank_lines_between_control_flow_statements() {
        let separated = r#"state "game.exe" {}
split {
    if first {
        return true
    }

    if second {
        return true
    }
}
"#;
        assert_eq!(format_source(separated).unwrap(), separated);

        let compact = r#"state "game.exe" {}
split {
    if first {
        return true
    }
    if second {
        return true
    }
}
"#;
        assert_eq!(format_source(compact).unwrap(), compact);
    }

    #[test]
    fn indents_state_reads_and_breaks_match_arms() {
        let source = r#"state "game.exe" {
value = process.read(
0x1000
)
}
fn describe(value) {
return match value {0=>"zero",1=>`value {value+1}`,_=>"other"}
}"#;
        let expected = r#"state "game.exe" {
    value = process.read(
        0x1000
    );
}
fn describe(value) {
    return match value {
        0 => "zero",
        1 => `value {value + 1}`,
        _ => "other",
    }
}
"#;

        let formatted = format_source(source).unwrap();
        assert_eq!(formatted, expected);
        assert_eq!(format_source(&formatted).unwrap(), formatted);
    }

    #[test]
    fn formats_file_version_match_patterns_as_literals() {
        let source = r#"state "game.exe"{}
fn supported(version:FileVersion)->bool{return match version{v"1.2.3.4"=>true,_=>false}}"#;
        let expected = r#"state "game.exe" {}
fn supported(version: FileVersion) -> bool {
    return match version {
        v"1.2.3.4" => true,
        _ => false,
    }
}
"#;

        let formatted = format_source(source).unwrap();
        assert_eq!(formatted, expected);
        assert_eq!(format_source(&formatted).unwrap(), formatted);
    }

    #[test]
    fn formats_string_match_patterns_as_literals() {
        let source = r#"state "game.exe"{}
fn classify(name:String)->u8{return match name{"game.exe"=>1,"escaped\nname"=>2,_=>0}}"#;
        let expected = r#"state "game.exe" {}
fn classify(name: String) -> u8 {
    return match name {
        "game.exe" => 1,
        "escaped\nname" => 2,
        _ => 0,
    }
}
"#;

        let formatted = format_source(source).unwrap();
        assert_eq!(formatted, expected);
        assert_eq!(format_source(&formatted).unwrap(), formatted);
    }

    #[test]
    fn formats_recursive_array_match_patterns() {
        let source = r#"state "game.exe"{}
fn decode(values:[u8?;2])->u8{return match values{[Some(value),None]|[None,Some(value)]=>value,[None|Some(_),_]=>0}}"#;
        let expected = r#"state "game.exe" {}
fn decode(values: [u8?; 2]) -> u8 {
    return match values {
        [Some(value), None] | [None, Some(value)] => value,
        [None | Some(_), _] => 0,
    }
}
"#;
        assert_eq!(format_source(source).unwrap(), expected);
    }

    #[test]
    fn formats_recursive_struct_match_patterns() {
        let source = r#"struct Point{x:u32,y:u32}
state "game.exe"{}
fn inspect(point:Point)->u32{return match point{Point{x:0,y}|Point{x:1,y:y}=>y,_=>0}}"#;
        let expected = r#"struct Point {
    x: u32,
    y: u32,
}
state "game.exe" {}
fn inspect(point: Point) -> u32 {
    return match point {
        Point { x: 0, y } | Point { x: 1, y: y } => y,
        _ => 0,
    }
}
"#;
        let formatted = format_source(source).unwrap();
        assert_eq!(formatted, expected);
        assert_eq!(format_source(&formatted).unwrap(), formatted);
    }

    #[test]
    fn keeps_short_struct_values_and_patterns_inline() {
        let source = r#"struct Pos{x:u16,y:u16}
state "game.exe"{}
fn inspect(value:Pos)->u16{let Pos{x,y}=Pos{x:1,y:2}
let{x:left,y:right}=value
return x+y+left+right}"#;
        let expected = r#"struct Pos {
    x: u16,
    y: u16,
}
state "game.exe" {}
fn inspect(value: Pos) -> u16 {
    let Pos { x, y } = Pos { x: 1, y: 2 }
    let { x: left, y: right } = value
    return x + y + left + right
}
"#;

        let formatted = format_source(source).unwrap();
        assert_eq!(formatted, expected);
        assert_eq!(format_source(&formatted).unwrap(), formatted);
    }

    #[test]
    fn preserves_authored_multiline_struct_values_and_patterns() {
        let source = r#"struct Pos {
    x: u16,
    y: u16,
}
state "game.exe" {}
fn inspect(value: Pos) -> u16 {
    let Pos {
        x,
        y,
    } = Pos {
        x: 1,
        y: 2,
    }
    return x + y
}
"#;

        let formatted = format_source(source).unwrap();
        assert_eq!(formatted, source);
        assert_eq!(format_source(&formatted).unwrap(), formatted);
    }

    #[test]
    fn expands_struct_values_that_exceed_the_default_width() {
        let source = r#"struct PositionSnapshot{horizontalCoordinate:u32,verticalCoordinate:u32,descriptiveLabel:String}
state "game.exe"{}
fn snapshot(){let value=PositionSnapshot{horizontalCoordinate:123,verticalCoordinate:456,descriptiveLabel:"ready"}}"#;
        let expected = r#"struct PositionSnapshot {
    horizontalCoordinate: u32,
    verticalCoordinate: u32,
    descriptiveLabel: String,
}
state "game.exe" {}
fn snapshot() {
    let value = PositionSnapshot {
        horizontalCoordinate: 123,
        verticalCoordinate: 456,
        descriptiveLabel: "ready",
    }
}
"#;

        let formatted = format_source(source).unwrap();
        assert_eq!(formatted, expected);
        assert!(
            formatted
                .lines()
                .all(|line| line.chars().count() <= DEFAULT_MAX_LINE_WIDTH)
        );
        assert_eq!(format_source(&formatted).unwrap(), formatted);
    }

    #[test]
    fn wraps_other_expressions_with_a_configurable_width() {
        let source = r#"state "game.exe"{}
fn sum(){return alpha+beta+gamma+delta+epsilon+zeta+eta+theta}"#;
        let options = FormatOptions {
            max_line_width: 40,
            ..FormatOptions::default()
        };
        let formatted = format_source_with_options(source, options).unwrap();

        assert!(formatted.contains("\n        +"));
        assert!(
            formatted
                .lines()
                .all(|line| line.chars().count() <= options.max_line_width)
        );
        assert_eq!(
            format_source_with_options(&formatted, options).unwrap(),
            formatted
        );
    }

    #[test]
    fn applies_configurable_indentation_line_endings_and_final_newline() {
        let source = "state \"game.exe\"{}\nwhileAttached{if true{print(1)}}";
        let options = FormatOptions {
            indent_style: IndentStyle::Tabs,
            indent_width: 3,
            line_ending: LineEnding::CrLf,
            insert_final_newline: false,
            ..FormatOptions::default()
        };
        let formatted = format_source_with_options(source, options).unwrap();

        assert!(formatted.contains("\r\n\tif true {\r\n\t\tprint(1)"));
        assert!(!formatted.ends_with(['\r', '\n']));
        assert!(!formatted.replace("\r\n", "").contains(['\r', '\n']));
        assert_eq!(
            format_source_with_options(&formatted, options).unwrap(),
            formatted
        );
    }

    #[test]
    fn preserves_binary_integer_literals_and_their_suffixes() {
        let source = r#"state GBA{flags:u8 at 0b0010_0000;mask:u16 at 0B1111u16}split{return current.flags&0b10!=0}"#;
        let expected = r#"state GBA {
    flags: u8 at 0b0010_0000;
    mask: u16 at 0B1111u16;
}
split {
    return current.flags & 0b10 != 0
}
"#;

        let formatted = format_source(source).unwrap();
        assert_eq!(formatted, expected);
        assert_eq!(format_source(&formatted).unwrap(), formatted);
    }

    #[test]
    fn indents_multiline_statement_expressions_without_indenting_nested_blocks_twice() {
        let source = r#"state "game.exe" {
choice = if enabled {
value
} else {
fallback
}
}
split {
let creditsTransition = !isDlcDemo
&& isFinalBaseLevel(old.levelOrScene)
&& isBaseCredits(current.levelOrScene)
return creditsTransition
}"#;
        let expected = r#"state "game.exe" {
    choice = if enabled {
        value
    } else {
        fallback
    };
}
split {
    let creditsTransition = !isDlcDemo
        && isFinalBaseLevel(old.levelOrScene)
        && isBaseCredits(current.levelOrScene)
    return creditsTransition
}
"#;

        let formatted = format_source(source).unwrap();
        assert_eq!(formatted, expected);
        assert_eq!(format_source(&formatted).unwrap(), formatted);
    }

    #[test]
    fn formats_retry_blocks_as_ordinary_prefix_expression_operands() {
        let source = r#"state "game.exe"{}
onAttach{
let health=retry{
let player=process.read<address>(0x1000)?
process.read<i32>(player)?
}
print(health)
}"#;
        let expected = r#"state "game.exe" {}
onAttach {
    let health = retry {
        let player = process.read<address>(0x1000)?
        process.read<i32>(player)?
    }
    print(health)
}
"#;

        let formatted = format_source(source).unwrap();
        assert_eq!(formatted, expected);
        assert_eq!(format_source(&formatted).unwrap(), formatted);
    }

    #[test]
    fn formats_chained_fallbacks_with_a_throw_expression() {
        let source = r#"state "game.exe"{}
fn engineModule(){return retry{let engine=process.loadedModule("EngineWin64s.dll")else process.loadedModule("EngineWin64sv.dll")else throw "engine module is not loaded yet"
engine}}"#;
        let expected = r#"state "game.exe" {}
fn engineModule() {
    return retry {
        let engine = process.loadedModule("EngineWin64s.dll")
            else process.loadedModule("EngineWin64sv.dll")
            else throw "engine module is not loaded yet"
        engine
    }
}
"#;

        let formatted = format_source(source).unwrap();
        assert_eq!(formatted, expected);
        assert_eq!(format_source(&formatted).unwrap(), formatted);
        crate::compile(&formatted).expect("formatted generic fallbacks should compile");
    }

    #[test]
    fn indents_match_arm_values_and_calls_nested_in_continued_expressions() {
        let source = r#"state "game.exe" {}
fn riftEnabled(rift) {
return match rift {
Rift.None => false,
Rift.Purple => settings.parent
&& settings.child,
}
}
fn shouldSplit() {
return gotTimePiece
&& detailedCheckpointEnabled(
chapter,
act,
checkpoint,
)
|| shouldSplitAtPosition(
key,
x,
y,
z,
)
}"#;
        let expected = r#"state "game.exe" {}
fn riftEnabled(rift) {
    return match rift {
        Rift.None => false,
        Rift.Purple => settings.parent
            && settings.child,
    }
}
fn shouldSplit() {
    return gotTimePiece
        && detailedCheckpointEnabled(
            chapter,
            act,
            checkpoint,
        )
        || shouldSplitAtPosition(
            key,
            x,
            y,
            z,
        )
}
"#;

        let formatted = format_source(source).unwrap();
        assert_eq!(formatted, expected);
        assert_eq!(format_source(&formatted).unwrap(), formatted);
    }

    #[test]
    fn block_indentation_supersedes_an_enclosing_multiline_call() {
        let source = r#"state "game.exe" {}
enum DelayedSplit {
Inactive,
ReceiveMinishCap,
GetFourSword
}
whileAttached {
debug print(match delayedSplit {
DelayedSplit.ReceiveMinishCap => "Receive Minish Cap",
DelayedSplit.GetFourSword => "Get Four Sword",
DelayedSplit.Inactive => "Delayed split"
})
}"#;
        let expected = r#"state "game.exe" {}
enum DelayedSplit {
    Inactive,
    ReceiveMinishCap,
    GetFourSword,
}
whileAttached {
    debug print(match delayedSplit {
        DelayedSplit.ReceiveMinishCap => "Receive Minish Cap",
        DelayedSplit.GetFourSword => "Get Four Sword",
        DelayedSplit.Inactive => "Delayed split",
    })
}
"#;

        let formatted = format_source(source).unwrap();
        assert_eq!(formatted, expected);
        assert_eq!(format_source(&formatted).unwrap(), formatted);
    }

    #[test]
    fn block_indentation_supersedes_an_enclosing_multiline_header() {
        let source = r#"state "game.exe" {}
fn positionSettingEnabled(key: i32) -> bool {
if !detailedChapterEnabled(if key == 5 {
5
} else {
4
}) {
return false
}
return true
}"#;
        let expected = r#"state "game.exe" {}
fn positionSettingEnabled(key: i32) -> bool {
    if !detailedChapterEnabled(if key == 5 {
        5
    } else {
        4
    }) {
        return false
    }
    return true
}
"#;

        let formatted = format_source(source).unwrap();
        assert_eq!(formatted, expected);
        assert_eq!(format_source(&formatted).unwrap(), formatted);
    }

    #[test]
    fn indents_nested_settings_and_their_specialized_entries() {
        let source = r#"state "game.exe" {}
enum Mode {
A,
B
}
settings{"Group"{/// Enables the feature.
"Enabled"
=>enabled:true,"Profile"=>profile:"Runner","Mode"
=>mode:choice{"First"=>Mode.A,"Second"=>Mode.B default},"Input"=>input:file{"Text"=>"*.txt",mime=>"text/plain"}}}"#;
        let expected = r#"state "game.exe" {}
enum Mode {
    A,
    B,
}
settings {
    "Group" {
        /// Enables the feature.
        "Enabled" => enabled: true,
        "Profile" => profile: "Runner",
        "Mode" => mode: choice {
            "First" => Mode.A,
            "Second" => Mode.B default,
        },
        "Input" => input: file {
            "Text" => "*.txt",
            mime => "text/plain",
        },
    },
}
"#;

        let formatted = format_source(source).unwrap();
        assert_eq!(formatted, expected);
        assert_eq!(format_source(&formatted).unwrap(), formatted);
    }

    #[test]
    fn formats_the_language_showcases_idempotently() {
        for source in [
            include_str!("../examples/lunistice.split"),
            include_str!("../examples/hello_lunistice.split"),
            include_str!("../examples/lso_desktop_settings.split"),
            include_str!("../examples/minish_cap.split"),
        ] {
            assert!(
                !source.contains('\r'),
                "tracked source fixtures must use canonical LF line endings"
            );
            for input in [source.to_owned(), source.replace('\n', "\r\n")] {
                let formatted = format_source(&input).unwrap();
                crate::parse(&formatted).unwrap();
                assert_eq!(format_source(&formatted).unwrap(), formatted);
            }
        }
        let minish_cap = include_str!("../examples/minish_cap.split");
        assert_eq!(format_source(minish_cap).unwrap(), minish_cap);
        assert!(
            format_source(include_str!("../examples/lunistice.split"))
                .unwrap()
                .contains("state Unity.il2cpp(Il2CppProfile.unity2022_3_0f1X64()) [")
        );
    }

    #[test]
    fn formats_declarative_tick_rates_as_a_trailing_comma_block() {
        let source = "state \"game.exe\" {}\ntickRate {\nattached:60,\ndetached:2\n}";
        let expected = r#"state "game.exe" {}
tickRate {
    attached: 60,
    detached: 2,
}
"#;
        let formatted = format_source(source).unwrap();
        assert_eq!(formatted, expected);
        assert_eq!(format_source(&formatted).unwrap(), formatted);
    }

    #[test]
    fn formats_named_state_provider_alternatives() {
        let source = r#"state{provider Windows:Native["game.exe"]{level:u32 at 0x100},provider Advance:GBA{level:u32 at 0x03000010}}"#;
        let expected = r#"state {
    provider Windows: Native ["game.exe"] {
        level: u32 at 0x100;
    },
    provider Advance: GBA {
        level: u32 at 0x03000010;
    },
}
"#;
        let formatted = format_source(source).unwrap();
        assert_eq!(formatted, expected);
        assert_eq!(format_source(&formatted).unwrap(), formatted);
    }

    #[test]
    fn formats_attachment_shape_globals_and_assignments() {
        let source = r#"enum Edition{Base,Demo}
enum Storefront{Steam,GOG}
let edition:Edition
let storefront:Storefront
state "game.exe"{level:u32 at 0x100}
onAttach{edition=Edition.Base
storefront=Storefront.Steam}"#;
        let expected = r#"enum Edition {
    Base,
    Demo,
}
enum Storefront {
    Steam,
    GOG,
}
let edition: Edition
let storefront: Storefront
state "game.exe" {
    level: u32 at 0x100;
}
onAttach {
    edition = Edition.Base
    storefront = Storefront.Steam
}
"#;
        let formatted = format_source(source).unwrap();
        assert_eq!(formatted, expected);
        assert_eq!(format_source(&formatted).unwrap(), formatted);
    }

    #[test]
    fn formats_shared_layout_conditions_in_state_and_managed_schemas() {
        let source = r#"enum Edition{Base,Demo}
let edition:Edition
image "Assembly-CSharp"{class GameManager{if edition==Edition.Base{static u32 level;}else{static u32 scene;}}}
state Unity ["game.exe"]{if edition==Edition.Base{level:u8 at 0x100;}else if edition==Edition.Demo{scene:u8 at 0x200;}else{unknown:u8 at 0x300;}}"#;
        let expected = r#"enum Edition {
    Base,
    Demo,
}
let edition: Edition
image "Assembly-CSharp" {
    class GameManager {
        if edition == Edition.Base {
            static u32 level;
        } else {
            static u32 scene;
        }
    }
}
state Unity ["game.exe"] {
    if edition == Edition.Base {
        level: u8 at 0x100;
    } else if edition == Edition.Demo {
        scene: u8 at 0x200;
    } else {
        unknown: u8 at 0x300;
    }
}
"#;
        let formatted = format_source(source).unwrap();
        assert_eq!(formatted, expected);
        assert_eq!(format_source(&formatted).unwrap(), formatted);
    }

    #[test]
    fn formats_configured_state_provider_selectors_as_ordinary_calls() {
        let source = r#"state Unity.il2cpp ( Il2CppProfile.unity2021_3_11f1X64() )["game.exe","demo.exe"]{}"#;
        let expected = r#"state Unity.il2cpp(Il2CppProfile.unity2021_3_11f1X64()) ["game.exe", "demo.exe"] {}
"#;
        let formatted = format_source(source).unwrap();
        assert_eq!(formatted, expected);
        assert_eq!(format_source(&formatted).unwrap(), formatted);
    }

    #[test]
    fn keeps_generic_call_arguments_attached_without_a_turbofish() {
        let source = r#"state "game.exe"{}
whileAttached{let value=process.read<[u8;4]> (0);print<u32> (value [0])}"#;
        let expected = r#"state "game.exe" {}
whileAttached {
    let value = process.read<[u8; 4]>(0);
    print<u32>(value[0])
}
"#;

        let formatted = format_source(source).unwrap();
        assert_eq!(formatted, expected);
        assert_eq!(format_source(&formatted).unwrap(), formatted);
    }

    #[test]
    fn formats_deeply_nested_generic_call_arguments() {
        let source = r#"state "game.exe"{}
fn example(){make < Box < Box < u32 > > > ()}
"#;
        let expected = r#"state "game.exe" {}
fn example() {
    make<Box<Box<u32>>>()
}
"#;

        let formatted = format_source(source).unwrap();
        assert_eq!(formatted, expected);
        assert_eq!(format_source(&formatted).unwrap(), formatted);
    }

    #[test]
    fn keeps_generic_type_arguments_attached() {
        let source = r#"state "game.exe"{}
fn contains(visited:Set < String >,other:Set < String, >)->bool{return visited.contains("Atrium")||other.contains("Library")}"#;
        let expected = r#"state "game.exe" {}
fn contains(visited: Set<String>, other: Set<String>) -> bool {
    return visited.contains("Atrium") || other.contains("Library")
}
"#;

        let formatted = format_source(source).unwrap();
        assert_eq!(formatted, expected);
        assert_eq!(format_source(&formatted).unwrap(), formatted);
    }

    #[test]
    fn separates_generic_closers_from_following_assignments() {
        let source = r#"state "game.exe"{}
fn example(){let doneMaps:Set<String>=Set.new<String>();let nested:Set<Set<String>>=Set.new<Set<String>>()}
"#;
        let expected = r#"state "game.exe" {}
fn example() {
    let doneMaps: Set<String> = Set.new<String>();
    let nested: Set<Set<String>> = Set.new<Set<String>>()
}
"#;

        let formatted = format_source(source).unwrap();
        assert_eq!(formatted, expected);
        assert_eq!(format_source(&formatted).unwrap(), formatted);
    }

    #[test]
    fn does_not_split_shift_and_comparison_operators_in_expressions() {
        let source = r#"state "game.exe"{}
fn operators(left,right){let compared=left>=right;let shifted=left>>right;left>>=right}
"#;
        let expected = r#"state "game.exe" {}
fn operators(left, right) {
    let compared = left >= right;
    let shifted = left >> right;
    left >>= right
}
"#;

        let formatted = format_source(source).unwrap();
        assert_eq!(formatted, expected);
        assert_eq!(format_source(&formatted).unwrap(), formatted);
    }

    #[test]
    fn generic_assignment_boundaries_round_trip_across_whitespace_variants() {
        for ty in [
            "Set<String>",
            "Set<Set<String>>",
            "Set<Set<Set<String>>>",
            "Set<String>?",
            "Set<String>!",
            "Set<Set<String>>!",
        ] {
            for gap in ["", " ", "\t"] {
                let source = format!(
                    "state \"game.exe\" {{}}\nfn example() {{ let value: {ty}{gap}={gap}sourceValue() }}"
                );
                let formatted = format_source(&source).unwrap_or_else(|diagnostics| {
                    panic!("failed to format `{ty}` with gap {gap:?}: {diagnostics:?}")
                });
                assert!(
                    formatted.contains(&format!("let value: {ty} = sourceValue()")),
                    "{formatted}"
                );
                assert_eq!(format_source(&formatted).unwrap(), formatted);
            }
        }
    }

    #[test]
    fn separates_generic_cast_targets_from_equality_operators() {
        for (comparison, canonical) in [
            ("expr as List<u32> == foo", "expr as List<u32> == foo"),
            ("expr as List<u32>==foo", "expr as List<u32> == foo"),
            (
                "expr as List<List<u32>>==foo",
                "expr as List<List<u32>> == foo",
            ),
            ("expr as T!=foo", "expr as T != foo"),
            ("expr as T! == foo", "expr as T! == foo"),
        ] {
            let source =
                format!("state \"game.exe\" {{}}\nfn compare(expr, foo) {{ return {comparison} }}");
            let expected = format!(
                r#"state "game.exe" {{}}
fn compare(expr, foo) {{
    return {canonical}
}}
"#
            );

            let formatted = format_source(&source).unwrap();
            assert_eq!(formatted, expected);
            assert_eq!(format_source(&formatted).unwrap(), formatted);
        }
    }

    #[test]
    fn cast_type_operator_boundaries_format_idempotently() {
        let cases = [
            ("value as T==other", "value as T == other"),
            ("value as T>other", "value as T > other"),
            ("value as T>=other", "value as T >= other"),
            ("value as T>>other", "value as T >> other"),
            ("value as T<<other", "value as T << other"),
            ("value as T<=other", "value as T <= other"),
            ("value as Box<u32>==other", "value as Box<u32> == other"),
            ("value as Box<u32>!=other", "value as Box<u32> != other"),
            ("value as Box<u32>>other", "value as Box<u32> > other"),
            ("value as Box<u32>>=other", "value as Box<u32> >= other"),
            ("value as Box<u32>>>other", "value as Box<u32> >> other"),
            ("value as Box<u32><other", "value as Box<u32> < other"),
            ("value as Box<u32><=other", "value as Box<u32> <= other"),
            ("value as Box<u32><<other", "value as Box<u32> << other"),
            (
                "value as Box<Box<u32>>==other",
                "value as Box<Box<u32>> == other",
            ),
            (
                "value as Box<Box<u32>>>other",
                "value as Box<Box<u32>> > other",
            ),
            (
                "value as Box<Box<u32>>>>other",
                "value as Box<Box<u32>> >> other",
            ),
            ("value as T!=other", "value as T != other"),
            ("value as T! == other", "value as T! == other"),
            ("value as T!!=other", "value as T! != other"),
            ("value as T?==other", "value as T? == other"),
            ("value as Box<u32>?==other", "value as Box<u32>? == other"),
            ("value as Box<u32>!!=other", "value as Box<u32>! != other"),
            ("value as T!?", "value as T!?"),
            ("value as T?!", "value as T?!"),
            ("value as [u32; 4]==other", "value as [u32; 4] == other"),
            ("(value as T)?", "(value as T)?"),
            ("(value as T?)?", "(value as T?)?"),
            ("(value as T!)?", "(value as T!)?"),
        ];

        for (expression, canonical) in cases {
            let source = format!(
                "state \"game.exe\" {{}} fn compare(value, other) {{ return {expression} }}"
            );
            let formatted = format_source(&source).unwrap_or_else(|diagnostics| {
                panic!("failed to format `{expression}`: {diagnostics:?}")
            });
            assert!(
                formatted.contains(&format!("return {canonical}")),
                "{formatted}"
            );
            assert_eq!(format_source(&formatted).unwrap(), formatted);
        }

        for expression in ["value as T??", "value as T!!"] {
            let source =
                format!("state \"game.exe\" {{}} fn compare(value) {{ return {expression} }}");
            let diagnostics =
                format_source(&source).expect_err("formatting must reject duplicate type wrappers");
            assert!(
                diagnostics
                    .iter()
                    .any(|diagnostic| diagnostic.message.contains("two adjacent")),
                "{diagnostics:?}"
            );
        }

        let less_than =
            "state \"game.exe\" {} fn compare(value, other) { return (value as T)<other }";
        let formatted = format_source(less_than).unwrap();
        assert!(formatted.contains("return (value as T) < other"));
        assert_eq!(format_source(&formatted).unwrap(), formatted);
    }

    #[test]
    fn adds_trailing_commas_to_multiline_generic_types() {
        let source = r#"state "game.exe" {}
fn visit(values: Set<
String
>) {}
"#;
        let formatted = format_source(source).unwrap();
        assert!(formatted.contains("Set<\n"), "{formatted}");
        assert!(formatted.contains("String,\n"), "{formatted}");
        assert_eq!(format_source(&formatted).unwrap(), formatted);
    }

    #[test]
    fn formats_nested_multiline_generic_closers_as_distinct_delimiters() {
        let source = r#"state "game.exe" {}
fn visit(values: Set<
Set<
String
>
>) {}
"#;
        let expected = r#"state "game.exe" {}
fn visit(values: Set<
    Set<
        String,
    >,
>) {}
"#;

        let formatted = format_source(source).unwrap();
        assert_eq!(formatted, expected);
        assert_eq!(format_source(&formatted).unwrap(), formatted);
    }

    #[test]
    fn spaces_grouped_operands_after_binary_operators() {
        let source = r#"state "game.exe"{}
fn atMissionPoint(point:MissionPoint,y:f64)->bool{return y==(point.y as f64).roundTo(3)}"#;
        let expected = r#"state "game.exe" {}
fn atMissionPoint(point: MissionPoint, y: f64) -> bool {
    return y == (point.y as f64).roundTo(3)
}
"#;

        let formatted = format_source(source).unwrap();
        assert_eq!(formatted, expected);
        assert_eq!(format_source(&formatted).unwrap(), formatted);
    }

    #[test]
    fn spaces_grouped_expressions_after_prefix_keywords() {
        let source = r#"state "game.exe" {}
fn contains(x: f32) -> bool {return(x > 0.0)}"#;
        let expected = r#"state "game.exe" {}
fn contains(x: f32) -> bool {
    return (x > 0.0)
}
"#;

        let formatted = format_source(source).unwrap();
        assert_eq!(formatted, expected);
        assert_eq!(format_source(&formatted).unwrap(), formatted);
    }
}
