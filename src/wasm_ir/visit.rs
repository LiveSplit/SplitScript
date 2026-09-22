//! Shared traversal for lowered Wasm control flow and expression DAGs.

use crate::ast::ExprId;

use super::{Block, Expression, ExpressionKind, InterpolatedPart, Program, Statement, Terminator};

pub trait Visitor {
    fn visit_program(&mut self, program: &Program) {
        walk_program(self, program);
    }

    fn visit_block(&mut self, block: &Block, program: &Program) {
        walk_block(self, block, program);
    }

    fn visit_statement(&mut self, statement: &Statement, program: &Program) {
        walk_statement(self, statement, program);
    }

    fn visit_terminator(&mut self, terminator: &Terminator, program: &Program) {
        walk_terminator(self, terminator, program);
    }

    fn visit_expression_id(&mut self, expression: ExprId, program: &Program) {
        let expression = program
            .expression(expression)
            .expect("lowered expression references belong to the Wasm IR program");
        self.visit_expression(expression, program);
    }

    fn visit_expression(&mut self, expression: &Expression, program: &Program) {
        walk_expression(self, expression, program);
    }
}

pub fn walk_program(visitor: &mut (impl Visitor + ?Sized), program: &Program) {
    for body in program.bodies() {
        visitor.visit_block(&body.entry, program);
    }
    for (_, expression) in program.global_initializers() {
        visitor.visit_expression_id(expression, program);
    }
    for state in program.state_expressions() {
        visitor.visit_block(&state.entry, program);
    }
    for transform in program.state_transforms() {
        visitor.visit_block(&transform.entry, program);
    }
}

pub fn walk_block(visitor: &mut (impl Visitor + ?Sized), block: &Block, program: &Program) {
    for statement in &block.statements {
        visitor.visit_statement(statement, program);
    }
    visitor.visit_terminator(&block.terminator, program);
}

pub fn walk_statement(
    visitor: &mut (impl Visitor + ?Sized),
    statement: &Statement,
    program: &Program,
) {
    match statement {
        Statement::DebugLocation(_) | Statement::BindPattern { .. } => {}
        Statement::Store { value, .. }
        | Statement::StateStore { value, .. }
        | Statement::StoreTemporary { value, .. }
        | Statement::Evaluate {
            expression: value, ..
        } => visitor.visit_expression_id(*value, program),
        Statement::IndexStore { target, value, .. } => {
            visitor.visit_expression_id(*target, program);
            visitor.visit_expression_id(*value, program);
        }
        Statement::If {
            condition,
            then_block,
            else_block,
        } => {
            visitor.visit_expression_id(*condition, program);
            visitor.visit_block(then_block, program);
            visitor.visit_block(else_block, program);
        }
        Statement::Match { value, arms, .. } => {
            visitor.visit_expression_id(*value, program);
            for arm in arms {
                if let Some(guard) = arm.guard {
                    visitor.visit_expression_id(guard, program);
                }
                visitor.visit_block(&arm.block, program);
            }
        }
        Statement::Fallback {
            value,
            fallback_block,
            success_block,
            ..
        } => {
            visitor.visit_expression_id(*value, program);
            visitor.visit_block(fallback_block, program);
            visitor.visit_block(success_block, program);
        }
        Statement::While {
            condition, body, ..
        } => {
            visitor.visit_expression_id(*condition, program);
            visitor.visit_block(body, program);
        }
        Statement::For {
            iterable,
            iterator_step,
            body,
            ..
        } => {
            visitor.visit_expression_id(*iterable, program);
            if let Some(iterator_step) = iterator_step {
                visitor.visit_expression_id(*iterator_step, program);
            }
            visitor.visit_block(body, program);
        }
        Statement::ForInit {
            iterable,
            iterator_step,
            ..
        } => {
            visitor.visit_expression_id(*iterable, program);
            if let Some(iterator_step) = iterator_step {
                visitor.visit_expression_id(*iterator_step, program);
            }
        }
    }
}

pub fn walk_terminator(
    visitor: &mut (impl Visitor + ?Sized),
    terminator: &Terminator,
    program: &Program,
) {
    match terminator {
        Terminator::Break(value) => {
            if let Some(value) = value {
                visitor.visit_expression_id(*value, program);
            }
        }
        Terminator::Fallthrough | Terminator::Continue => {}
        Terminator::AsyncWhile {
            header,
            continuation,
            ..
        } => {
            visitor.visit_block(header, program);
            visitor.visit_block(continuation, program);
        }
        Terminator::AsyncWhileCondition {
            condition, body, ..
        } => {
            visitor.visit_expression_id(*condition, program);
            visitor.visit_block(body, program);
        }
        Terminator::AsyncFor {
            iterator_step,
            body,
            continuation,
            ..
        } => {
            if let Some(iterator_step) = iterator_step {
                visitor.visit_expression_id(*iterator_step, program);
            }
            visitor.visit_block(body, program);
            visitor.visit_block(continuation, program);
        }
        Terminator::Return(value) => {
            if let Some(value) = value {
                visitor.visit_expression_id(*value, program);
            }
        }
        Terminator::Throw { error, .. } => visitor.visit_expression_id(*error, program),
        Terminator::Yield {
            value,
            continuation,
            ..
        } => {
            visitor.visit_expression_id(*value, program);
            visitor.visit_block(continuation, program);
        }
        Terminator::Retry {
            attempt,
            continuation,
            ..
        } => {
            visitor.visit_block(attempt, program);
            visitor.visit_block(continuation, program);
        }
        Terminator::RetryComplete { value, .. } => {
            visitor.visit_expression_id(*value, program);
        }
        Terminator::Suspend {
            value,
            continuation,
            ..
        } => {
            visitor.visit_expression_id(*value, program);
            visitor.visit_block(continuation, program);
        }
    }
}

pub fn walk_expression(
    visitor: &mut (impl Visitor + ?Sized),
    expression: &Expression,
    program: &Program,
) {
    visit_expression_children(&expression.kind, |child| {
        visitor.visit_expression_id(child, program);
    });
}

/// Visits direct expression edges in deterministic evaluation order.
///
/// Analyses that own their own worklist can use this without inheriting the
/// recursive behavior of [`Visitor`]. This is the one exhaustive child-shape
/// match for the Wasm expression IR.
pub fn visit_expression_children(kind: &ExpressionKind, mut visit: impl FnMut(ExprId)) {
    match kind {
        ExpressionKind::None
        | ExpressionKind::IteratorEnd
        | ExpressionKind::ValueBlock
        | ExpressionKind::Loop
        | ExpressionKind::Bool(_)
        | ExpressionKind::Int { .. }
        | ExpressionKind::Float(_)
        | ExpressionKind::Char(_)
        | ExpressionKind::String(_)
        | ExpressionKind::Signature(_)
        | ExpressionKind::Temporary(_)
        | ExpressionKind::FallbackSuccess { .. }
        | ExpressionKind::Path { .. }
        | ExpressionKind::FunctionValue { .. } => {}
        ExpressionKind::Member { receiver, .. } => visit(*receiver),
        ExpressionKind::Index { receiver, index } => {
            visit(*receiver);
            visit(*index);
        }
        ExpressionKind::InterpolatedString(parts) => {
            for part in parts {
                if let InterpolatedPart::Expression { expression, .. } = part {
                    visit(*expression);
                }
            }
        }
        ExpressionKind::Array(elements) => elements.iter().copied().for_each(&mut visit),
        ExpressionKind::Range { start, end, .. } => {
            visit(*start);
            visit(*end);
        }
        ExpressionKind::Struct { fields, .. } => {
            fields.iter().map(|(_, value)| *value).for_each(&mut visit);
        }
        ExpressionKind::Enum { payload, .. } => payload.iter().copied().for_each(&mut visit),
        ExpressionKind::Unary { operand, .. } => visit(*operand),
        ExpressionKind::Cast { value }
        | ExpressionKind::Inspect { value, .. }
        | ExpressionKind::Transparent { value }
        | ExpressionKind::Suspend { value, .. }
        | ExpressionKind::Propagate { value, .. } => visit(*value),
        ExpressionKind::Binary { left, right, .. } => {
            visit(*left);
            visit(*right);
        }
        ExpressionKind::Call { target, arguments } => {
            if let Some((
                crate::semantic::ResolvedReceiver::Expression {
                    expression: receiver,
                    ..
                },
                _,
            )) = target.receiver_with_type()
            {
                visit(*receiver);
            }
            arguments.iter().copied().for_each(&mut visit);
        }
        ExpressionKind::Invoke { callee, arguments } => {
            if let crate::semantic::DynamicCallCallee::Expression(callee) = callee {
                visit(*callee);
            }
            arguments.iter().copied().for_each(&mut visit);
        }
        ExpressionKind::Closure { body, .. } => visit(*body),
        ExpressionKind::If {
            condition,
            then_expr,
            else_expr,
        } => {
            visit(*condition);
            visit(*then_expr);
            visit(*else_expr);
        }
        ExpressionKind::Fallback { value, fallback } => {
            visit(*value);
            visit(*fallback);
        }
        ExpressionKind::Break(value) | ExpressionKind::Return(value) => {
            value.iter().copied().for_each(&mut visit)
        }
        ExpressionKind::Throw { error, .. } => visit(*error),
        ExpressionKind::Continue => {}
        ExpressionKind::Match { value, arms } => {
            visit(*value);
            for arm in arms {
                arm.guard.iter().copied().for_each(&mut visit);
                visit(arm.value);
            }
        }
        ExpressionKind::Is { value, .. } => visit(*value),
    }
}

#[cfg(test)]
mod tests {
    use crate::ast::{ExprKind, Stmt};

    use super::*;

    #[test]
    fn child_edges_follow_deterministic_evaluation_order() {
        let parsed = crate::parse(
            r#"
                state "game.exe" {}

                fn choose(value: i32) {
                    let selected = match value {
                        1 if true => 2,
                        _ => 3
                    }
                }
            "#,
        )
        .unwrap();
        let Stmt::Variable(variable) = &parsed.syntax().functions[0].body.statements[0] else {
            panic!("expected a variable declaration")
        };
        let ExprKind::Match { value, arms } = &variable.value.as_ref().unwrap().kind else {
            panic!("expected a match expression")
        };
        let expected = [
            value.id,
            arms[0].guard.as_ref().unwrap().id,
            arms[0].value.id,
            arms[1].value.id,
        ];
        let match_id = variable.value.as_ref().unwrap().id;
        let lowered = crate::lower(parsed);
        let checked = crate::check(lowered).unwrap();
        let program = crate::lower_wasm(&checked);
        let kind = program
            .expression(match_id)
            .expect("the match expression is lowered")
            .kind
            .clone();
        let mut visited = Vec::new();
        visit_expression_children(&kind, |child| visited.push(child));
        assert_eq!(visited, expected);
    }
}
