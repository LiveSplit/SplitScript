//! Compiler-owned inferred-type hints shared by LSP clients.

use crate::{
    ast::{
        BindingPattern, Expr, ExprKind, ForBinding, FunctionDecl, Span, StateField,
        SuspensionBinding, VariableDecl,
    },
    database::SemanticSnapshot,
    lexer::{Token, TokenKind},
    type_display::display_type,
    visit::{self, Visitor},
};

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct InlayHint {
    /// Byte position at which the editor renders the virtual annotation.
    pub position: usize,
    /// Source-shaped inferred type, including the declaration separator.
    pub label: String,
}

pub(crate) fn inferred_type_hints(
    snapshot: &SemanticSnapshot,
    requested_range: Span,
) -> Vec<InlayHint> {
    let tokens = snapshot.source_document().tokens().collect::<Vec<_>>();
    let mut collector = InlayHintCollector {
        snapshot,
        requested_range,
        tokens,
        hints: Vec::new(),
    };
    collector.visit_program(snapshot.syntax());
    collector.hints.sort_by_key(|hint| hint.position);
    collector.hints
}

struct InlayHintCollector<'a> {
    snapshot: &'a SemanticSnapshot,
    requested_range: Span,
    tokens: Vec<&'a Token>,
    hints: Vec<InlayHint>,
}

impl InlayHintCollector<'_> {
    fn add_inferred_pattern(&mut self, pattern: &BindingPattern) {
        let Some(ty) = self.snapshot.semantics().value_type(pattern.id) else {
            return;
        };
        if self.snapshot.semantics().types().contains_error(ty) {
            return;
        }
        self.add_hint(
            pattern.name_span.end,
            format!(": {}", display_type(ty, self.snapshot)),
        );
    }

    fn add_hint(&mut self, position: usize, label: String) {
        if position < self.requested_range.start || position > self.requested_range.end {
            return;
        }
        self.hints.push(InlayHint { position, label });
    }

    fn add_inferred_value(&mut self, id: crate::ast::ValueId, name: &str, span: Span) {
        let Some(ty) = self.snapshot.semantics().value_type(id) else {
            return;
        };
        if self.snapshot.semantics().types().contains_error(ty) {
            return;
        }
        let first_candidate = self
            .tokens
            .partition_point(|token| token.span.start < span.start);
        let Some(identifier) = self.tokens[first_candidate..]
            .iter()
            .take_while(|token| token.span.end <= span.end)
            .find(|token| matches!(&token.kind, TokenKind::Ident(spelling) if spelling == name))
        else {
            return;
        };
        self.add_hint(
            identifier.span.end,
            format!(": {}", display_type(ty, self.snapshot)),
        );
    }

    fn add_inferred_function_result(&mut self, function: &FunctionDecl) {
        let Some(ty) = self.snapshot.semantics().function_result(function.id) else {
            return;
        };
        if self.snapshot.semantics().types().contains_error(ty) {
            return;
        }
        let first_candidate = self
            .tokens
            .partition_point(|token| token.span.start < function.span.start);
        let Some(closing_parenthesis) = self.tokens[first_candidate..]
            .iter()
            .take_while(|token| token.span.end <= function.body.span.start)
            .filter(|token| token.kind == TokenKind::RParen)
            .last()
        else {
            return;
        };
        self.add_hint(
            closing_parenthesis.span.end,
            format!(" -> {}", display_type(ty, self.snapshot)),
        );
    }

    fn add_inferred_closure_result(&mut self, expression: &Expr, arrow_span: Span) {
        let Some(ty) = self.snapshot.semantics().expression_type(expression.id) else {
            return;
        };
        let crate::types::TypeKind::Callable { result, .. } =
            self.snapshot.semantics().types().kind(ty)
        else {
            return;
        };
        if self.snapshot.semantics().types().contains_error(*result) {
            return;
        }
        let result = display_type(*result, self.snapshot);
        let ExprKind::Closure { params, .. } = &expression.kind else {
            unreachable!("closure result hints are requested only for closures")
        };

        // The bare one-parameter shorthand has no closing delimiter at which
        // a result annotation can be displayed unambiguously. Virtual grouping
        // keeps the source unchanged while presenting `(value: T) -> U =>`.
        if params.len() == 1 && expression.span.start == params[0].name_span.start {
            self.add_hint(params[0].name_span.start, "(".to_owned());
            self.add_hint(params[0].name_span.end, format!(") -> {result}"));
            return;
        }

        let first_candidate = self
            .tokens
            .partition_point(|token| token.span.start < expression.span.start);
        if let Some(closing_parenthesis) = self.tokens[first_candidate..]
            .iter()
            .take_while(|token| token.span.end <= arrow_span.start)
            .filter(|token| token.kind == TokenKind::RParen)
            .last()
        {
            self.add_hint(closing_parenthesis.span.end, format!(" -> {result}"));
        }
    }
}

impl<'ast> Visitor<'ast> for InlayHintCollector<'_> {
    fn visit_state_field(&mut self, field: &'ast StateField) {
        if field.annotation.is_none() {
            self.add_inferred_value(field.id, &field.name, field.span);
        }
        visit::walk_state_field(self, field);
    }

    fn visit_function(&mut self, function: &'ast FunctionDecl) {
        if function.return_annotation.is_none() {
            self.add_inferred_function_result(function);
        }
        visit::walk_function(self, function);
    }

    fn visit_parameter(&mut self, parameter: &'ast crate::ast::Parameter) {
        if parameter.annotation.is_none() {
            self.add_inferred_pattern(&parameter.binding);
        }
        visit::walk_parameter(self, parameter);
    }

    fn visit_variable(&mut self, variable: &'ast VariableDecl) {
        if variable.annotation.is_none() {
            self.add_inferred_pattern(&variable.binding);
        }
        visit::walk_variable(self, variable);
    }

    fn visit_suspension_binding(&mut self, binding: &'ast SuspensionBinding) {
        if binding.annotation.is_none() {
            self.add_inferred_value(binding.id, &binding.name, binding.span);
        }
        if let Some(annotation) = &binding.annotation {
            self.visit_type_ref(annotation);
        }
    }

    fn visit_for_binding(&mut self, binding: &'ast ForBinding) {
        self.add_inferred_pattern(&binding.binding);
    }

    fn visit_expr(&mut self, expression: &'ast Expr) {
        if let ExprKind::Closure {
            return_annotation,
            arrow_span,
            ..
        } = &expression.kind
            && return_annotation.is_none()
        {
            // Parameter hints must be emitted first when the bare shorthand's
            // parameter and result annotations share one editor position.
            visit::walk_expr(self, expression);
            self.add_inferred_closure_result(expression, *arrow_span);
        } else {
            visit::walk_expr(self, expression);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::database::CompilerDatabase;

    #[test]
    fn reports_only_inferred_declaration_types_in_the_requested_range() {
        let source = r#"state "game.exe" {}
let global = 7
let explicit: i32 = 8
fn identity(value) {
    return value
}
fn annotatedReturn() -> i32 {
    return 1
}
whileAttached {
    let local = identity(global)
    let annotated: i32 = local
    for item in [1, 2] {
        print(item as String)
    }
}"#;
        let mut database = CompilerDatabase::new(source);
        let snapshot = database.semantic_snapshot().unwrap();
        let range = Span {
            start: source.find("fn identity").unwrap(),
            end: source.len(),
        };
        let hints = inferred_type_hints(&snapshot, range);
        assert_eq!(
            hints
                .iter()
                .map(|hint| (&source[..hint.position], hint.label.as_str()))
                .map(|(before, label)| (before.split_whitespace().last().unwrap(), label))
                .collect::<Vec<_>>(),
            [
                ("identity(value", ": T"),
                ("identity(value)", " -> T"),
                ("local", ": i32"),
                ("item", ": i32")
            ]
        );
    }

    #[test]
    fn reports_collection_types_inferred_from_later_uses() {
        let source = r#"state "game.exe" {}
whileAttached {
    let values = []
    values.push(7u16)
    let visited = Set.new()
    visited.insert("Atrium")
}"#;
        let mut database = CompilerDatabase::new(source);
        let snapshot = database.semantic_snapshot().unwrap();
        let hints = inferred_type_hints(
            &snapshot,
            Span {
                start: 0,
                end: source.len(),
            },
        );
        assert!(hints.iter().any(|hint| hint.label == ": [u16]"));
        assert!(hints.iter().any(|hint| hint.label == ": Set<String>"));
    }

    #[test]
    fn distinguishes_future_values_from_awaited_completion_values() {
        let source = r#"state "game.exe" {}
fn discover() {
    return await process.mainModule()
}
onAttach {
    let pending = discover()
    let module = await pending
    print(module.address)
}"#;
        let mut database = CompilerDatabase::new(source);
        let snapshot = database.semantic_snapshot().unwrap();
        let hints = inferred_type_hints(
            &snapshot,
            Span {
                start: 0,
                end: source.len(),
            },
        );
        assert!(hints.iter().any(|hint| hint.label == " -> async Module"));
        assert!(hints.iter().any(|hint| hint.label == ": async Module"));
        assert!(hints.iter().any(|hint| hint.label == ": Module"));
    }

    #[test]
    fn reports_never_as_the_completion_of_a_stored_process_wait() {
        let source = r#"state "game.exe" {}
onAttach {
    let pending = process.closed()
    await pending
}"#;
        let mut database = CompilerDatabase::new(source);
        let snapshot = database.semantic_snapshot().unwrap();
        let hints = inferred_type_hints(
            &snapshot,
            Span {
                start: 0,
                end: source.len(),
            },
        );
        assert!(hints.iter().any(|hint| hint.label == ": async Never"));
    }

    #[test]
    fn shows_options_inferred_from_none_initialized_global_assignments() {
        let source = r#"let pending = None
let unit = None
state "game.exe" {}
whileAttached {
    pending = Instant.now()
    if unit == None { print("unit") }
}"#;
        let mut database = CompilerDatabase::new(source);
        let snapshot = database.semantic_snapshot().unwrap();
        let hints = inferred_type_hints(
            &snapshot,
            Span {
                start: 0,
                end: source.len(),
            },
        );
        assert!(hints.iter().any(|hint| {
            hint.position == source.find("pending").unwrap() + "pending".len()
                && hint.label == ": Instant?"
        }));
        assert!(hints.iter().any(|hint| {
            hint.position == source.find("unit").unwrap() + "unit".len() && hint.label == ": None"
        }));
    }

    #[test]
    fn shows_types_inferred_for_attachment_scoped_globals() {
        let source = r#"let module
state "game.exe" {}
onAttach { module = await process.mainModule() }
"#;
        let mut database = CompilerDatabase::new(source);
        let snapshot = database.semantic_snapshot().unwrap();
        let hints = inferred_type_hints(
            &snapshot,
            Span {
                start: 0,
                end: source.len(),
            },
        );
        assert!(hints.iter().any(|hint| {
            hint.position == source.find("module").unwrap() + "module".len()
                && hint.label == ": Module"
        }));
    }

    #[test]
    fn failed_inference_does_not_publish_fabricated_type_hints() {
        let source = r#"state GBA {
    ok at 0x100;
}
split {
    let copy = current.ok
    return false
}"#;
        let mut database = CompilerDatabase::new(source);
        let snapshot = database
            .semantic_snapshot()
            .expect("recovering semantics should remain available");
        let field = &snapshot.syntax().state.as_ref().unwrap().fields[0];
        let field_type = snapshot.semantics().value_type(field.id).unwrap();
        assert_eq!(
            snapshot.semantics().types().kind(field_type),
            &crate::types::TypeKind::Error
        );

        let hints = inferred_type_hints(
            &snapshot,
            Span {
                start: 0,
                end: source.len(),
            },
        );
        assert!(hints.is_empty(), "{hints:#?}");
        assert!(database.diagnostics().iter().any(|diagnostic| {
            diagnostic
                .message
                .contains("cannot infer the memory type of state field `ok`")
        }));
    }

    #[test]
    fn unsuffixed_numeric_literals_publish_rust_style_default_hints() {
        let source = r#"state "game.exe" {}
whileAttached {
    let integer = 7
    let float = 1.5
}"#;
        let mut database = CompilerDatabase::new(source);
        let snapshot = database.semantic_snapshot().unwrap();
        let hints = inferred_type_hints(
            &snapshot,
            Span {
                start: 0,
                end: source.len(),
            },
        );
        assert!(hints.iter().any(|hint| hint.label == ": i32"), "{hints:#?}");
        assert!(hints.iter().any(|hint| hint.label == ": f64"), "{hints:#?}");
    }

    #[test]
    fn inferred_annotations_apply_to_the_complete_binding_pattern() {
        let source = r#"
struct Point { x: i32, y: i32 }
state "game.exe" {}
fn observe(Point { x, y }) {
    let Point { x: localX, y: localY } = Point { x, y }
    for Point { x: itemX, y: itemY } in [Point { x, y }] {}
}
"#;
        let mut database = CompilerDatabase::new(source);
        let snapshot = database.semantic_snapshot().unwrap();
        let hints = inferred_type_hints(
            &snapshot,
            Span {
                start: 0,
                end: source.len(),
            },
        );
        for pattern in [
            "Point { x, y }",
            "Point { x: localX, y: localY }",
            "Point { x: itemX, y: itemY }",
        ] {
            let position = source.find(pattern).unwrap() + pattern.len();
            assert!(
                hints
                    .iter()
                    .any(|hint| hint.position == position && hint.label == ": Point"),
                "{pattern}: {hints:#?}"
            );
        }
    }
}
