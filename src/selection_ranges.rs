//! Syntax-aware selection expansion for editor clients.
//!
//! Selection ranges deliberately come from the recovering syntax tree rather
//! than semantic products. They therefore remain useful while the author is
//! in the middle of typing an expression or declaration.

use crate::{
    ast::{
        Action, Block, EnumDecl, Expr, ExprKind, FunctionDecl, MatchArm, MatchPattern, PatternNode,
        Program, SettingDecl, SettingFamilyDecl, SettingKind, Span, StateDecl, StateField, Stmt,
        StructDecl, VariableDecl,
    },
    lexer::{Lexeme, TokenKind, TriviaKind},
    syntax::SourceDocument,
    visit::{self, Visitor},
};

/// Returns a strictly growing chain beginning at the caret and ending at the
/// complete document. Every parent contains the preceding child.
pub(crate) fn selection_ranges(
    document: &SourceDocument,
    syntax: &Program,
    offset: usize,
) -> Vec<Span> {
    let source_len = document.source().len();
    let offset = offset.min(source_len);
    let mut collector = SpanCollector {
        spans: vec![Span {
            start: 0,
            end: source_len,
        }],
    };
    collector.collect_program_only_nodes(syntax);
    collector.visit_program(syntax);

    if let Some(lexeme) = document.lexemes().iter().find(|lexeme| {
        let span = lexeme.span();
        span.start <= offset
            && offset < span.end
            && match lexeme {
                Lexeme::Token(token) => token.kind != TokenKind::Eof,
                Lexeme::Trivia(trivia) => matches!(
                    trivia.kind,
                    TriviaKind::LineComment | TriviaKind::BlockComment
                ),
            }
    }) {
        collector.push(lexeme.span());
    } else if let Some(token) = document.symbol_token_at(offset) {
        collector.push(token.span);
    }

    let mut candidates = collector
        .spans
        .into_iter()
        .filter(|span| {
            span.start <= span.end
                && span.end <= source_len
                && contains_position(*span, offset, source_len)
        })
        .collect::<Vec<_>>();
    candidates.sort_by_key(|span| (span.end - span.start, span.start, span.end));
    candidates.dedup();

    let mut chain = vec![Span {
        start: offset,
        end: offset,
    }];
    for candidate in candidates {
        let child = *chain.last().unwrap();
        if candidate != child && contains_span(candidate, child) {
            chain.push(candidate);
        }
    }
    chain
}

fn contains_position(span: Span, offset: usize, source_len: usize) -> bool {
    span.start <= offset && (offset < span.end || (offset == source_len && span.end == source_len))
}

fn contains_span(parent: Span, child: Span) -> bool {
    parent.start <= child.start && child.end <= parent.end
}

#[derive(Default)]
struct SpanCollector {
    spans: Vec<Span>,
}

impl SpanCollector {
    fn push(&mut self, span: Span) {
        if span.start != span.end {
            self.spans.push(span);
        }
    }

    fn collect_program_only_nodes(&mut self, program: &Program) {
        if let Some(tick_rate) = program.tick_rate {
            self.push(tick_rate.span);
            if let Some(attached) = tick_rate.attached {
                self.push(attached.span);
            }
            if let Some(detached) = tick_rate.detached {
                self.push(detached.span);
            }
        }
        if let Some(settings) = program.settings_span {
            self.push(settings);
        }
        for occurrence in &program.type_name_occurrences {
            for span in occurrence {
                self.push(*span);
            }
        }
        for application in &program.type_applications {
            for occurrence in &application.occurrences {
                self.push(occurrence.span);
            }
        }
    }
}

impl<'ast> Visitor<'ast> for SpanCollector {
    fn visit_state(&mut self, state: &'ast StateDecl) {
        self.push(state.span);
        if let Some(provider) = &state.provider {
            self.push(provider.span);
            if let Some(selector) = &provider.selector {
                self.push(selector.name_span);
                self.push(selector.span);
            }
        }
        for alternative in &state.provider_alternatives {
            self.push(alternative.span);
            self.push(alternative.provider.span);
            if let Some(selector) = &alternative.provider.selector {
                self.push(selector.name_span);
                self.push(selector.span);
            }
        }
        visit::walk_state(self, state);
    }

    fn visit_state_field(&mut self, field: &'ast StateField) {
        self.push(field.span);
        if let Some(transform) = &field.transform {
            self.push(transform.span);
        }
        visit::walk_state_field(self, field);
    }

    fn visit_setting(&mut self, setting: &'ast SettingDecl) {
        self.push(setting.span);
        if let Some(key) = &setting.external_key {
            self.push(key.span());
        }
        if let SettingKind::Choice { options, .. } = &setting.kind {
            for option in options {
                self.push(option.span);
            }
        }
    }

    fn visit_setting_family(&mut self, family: &'ast SettingFamilyDecl) {
        self.push(family.span);
        self.push(family.binding_span);
        self.push(family.range_span);
        self.push(family.label.span);
        if let Some(key) = &family.key {
            self.push(key.span);
        }
    }

    fn visit_struct(&mut self, structure: &'ast StructDecl) {
        self.push(structure.span);
        for field in &structure.fields {
            self.push(field.span);
        }
        visit::walk_struct(self, structure);
    }

    fn visit_enum(&mut self, enumeration: &'ast EnumDecl) {
        self.push(enumeration.span);
        for variant in &enumeration.variants {
            self.push(variant.span);
        }
        visit::walk_enum(self, enumeration);
    }

    fn visit_managed_image(&mut self, image: &'ast crate::ast::ManagedImageDecl) {
        self.push(image.span);
        self.push(image.name_span);
        visit::walk_managed_image(self, image);
    }

    fn visit_managed_namespace(&mut self, namespace: &'ast crate::ast::ManagedNamespaceDecl) {
        self.push(namespace.span);
        self.push(namespace.name_span);
        visit::walk_managed_namespace(self, namespace);
    }

    fn visit_managed_class(&mut self, class: &'ast crate::ast::ManagedClassDecl) {
        self.push(class.span);
        self.push(class.name_span);
        if let Some(span) = class.metadata_names.span {
            self.push(span);
        }
        visit::walk_managed_class(self, class);
    }

    fn visit_managed_field(&mut self, field: &'ast crate::ast::ManagedFieldDecl) {
        self.push(field.span);
        self.push(field.type_span);
        self.push(field.name_span);
        if let Some(span) = field.metadata_names.span {
            self.push(span);
        }
        self.visit_type_ref(&field.ty);
    }

    fn visit_function(&mut self, function: &'ast FunctionDecl) {
        self.push(function.span);
        if let Some(annotation) = function.return_annotation_span {
            self.push(annotation);
        }
        visit::walk_function(self, function);
    }

    fn visit_parameter(&mut self, parameter: &'ast crate::ast::Parameter) {
        self.push(parameter.span);
        self.collect_pattern_node(&parameter.binding.pattern);
        visit::walk_parameter(self, parameter);
    }

    fn visit_action(&mut self, action: &'ast Action) {
        self.push(action.span);
        self.visit_block(&action.body);
    }

    fn visit_variable(&mut self, variable: &'ast VariableDecl) {
        self.push(variable.span);
        self.collect_pattern_node(&variable.binding.pattern);
        visit::walk_variable(self, variable);
    }

    fn visit_for_binding(&mut self, binding: &'ast crate::ast::ForBinding) {
        self.push(binding.span);
        self.collect_pattern_node(&binding.binding.pattern);
    }

    fn visit_block(&mut self, block: &'ast Block) {
        self.push(block.span);
        visit::walk_block(self, block);
    }

    fn visit_stmt(&mut self, statement: &'ast Stmt) {
        self.push(statement_span(statement));
        visit::walk_stmt(self, statement);
    }

    fn visit_expr(&mut self, expression: &'ast Expr) {
        self.push(expression.span);
        if let ExprKind::Is { pattern, .. } = &expression.kind {
            self.collect_pattern_node(pattern);
        }
        if let ExprKind::Closure {
            return_annotation_span: Some(span),
            ..
        } = &expression.kind
        {
            self.push(*span);
        }
        visit::walk_expr(self, expression);
    }

    fn visit_match_arm(&mut self, arm: &'ast MatchArm) {
        self.push(arm.span);
        visit::walk_match_arm(self, arm);
    }
}

impl SpanCollector {
    fn collect_pattern_node(&mut self, pattern: &PatternNode) {
        self.push(pattern.span);
        match &pattern.kind {
            MatchPattern::Struct { fields, .. } => {
                for field in fields {
                    self.push(field.name_span);
                    self.collect_pattern_node(&field.pattern);
                }
            }
            MatchPattern::Enum {
                payload: Some(payload),
                ..
            }
            | MatchPattern::OptionSome(payload)
            | MatchPattern::IteratorItem(payload)
            | MatchPattern::ResultSuccess(payload)
            | MatchPattern::ResultError(payload) => self.collect_pattern_node(payload),
            MatchPattern::Array(array) => {
                for element in array.elements() {
                    self.collect_pattern_node(element);
                }
            }
            MatchPattern::Alternation(alternatives) => {
                for alternative in alternatives {
                    self.collect_pattern_node(alternative);
                }
            }
            _ => {}
        }
    }
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

#[cfg(test)]
mod tests {
    use super::*;
    use crate::database::CompilerDatabase;

    fn selected_text<'a>(source: &'a str, ranges: &[Span]) -> Vec<&'a str> {
        ranges
            .iter()
            .map(|range| &source[range.start..range.end])
            .collect()
    }

    #[test]
    fn expands_through_expression_statement_block_declaration_and_document() {
        let source = r#"state "game.exe" {}

fn calculate(value: i32) -> i32 {
    let result = value * (1 + 2)
    return result
}
"#;
        let mut database = CompilerDatabase::new(source);
        let parsed = database.recovering_parse().unwrap();
        let offset = source.find("1 + 2").unwrap();
        let ranges = selection_ranges(parsed.source_document(), parsed.syntax(), offset);
        let text = selected_text(source, &ranges);

        for expected in [
            "",
            "1",
            "(1 + 2)",
            "value * (1 + 2)",
            "let result = value * (1 + 2)",
            "{\n    let result = value * (1 + 2)\n    return result\n}",
            "fn calculate(value: i32) -> i32 {\n    let result = value * (1 + 2)\n    return result\n}",
            source,
        ] {
            assert!(
                text.contains(&expected),
                "missing selection `{expected}` in {text:?}"
            );
        }
        assert!(
            ranges
                .windows(2)
                .all(|pair| { pair[0] != pair[1] && contains_span(pair[1], pair[0]) })
        );
    }

    #[test]
    fn recovering_syntax_still_expands_inside_an_unfinished_expression() {
        let source = "state \"game.exe\" {}\nwhileAttached { let value = (1 + }";
        let mut database = CompilerDatabase::new(source);
        let parsed = database.recovering_parse().unwrap();
        let offset = source.find('1').unwrap();
        let ranges = selection_ranges(parsed.source_document(), parsed.syntax(), offset);
        let text = selected_text(source, &ranges);

        assert!(text.contains(&"1"));
        assert_eq!(text.last(), Some(&source));
    }

    #[test]
    fn is_patterns_expand_through_nested_pattern_and_condition_nodes() {
        let source = r#"state "game.exe" {}
fn inspect(value: u32?) {
    if value is Some(number) && number > 0 {
        print(number)
    }
}"#;
        let mut database = CompilerDatabase::new(source);
        let parsed = database.recovering_parse().unwrap();
        let offset = source.find("number)").unwrap() + 1;
        let ranges = selection_ranges(parsed.source_document(), parsed.syntax(), offset);
        let text = selected_text(source, &ranges);
        for expected in [
            "number",
            "Some(number)",
            "value is Some(number)",
            "value is Some(number) && number > 0",
        ] {
            assert!(text.contains(&expected), "missing `{expected}` in {text:?}");
        }
    }
}
