//! Syntax-level classification for expressions that can be evaluated without
//! runtime state.

use crate::{
    ast::{Expr, ExprKind, FunctionId, Program, Stmt},
    resolution::ProgramResolutions,
    stdlib::{Implementation, StandardLibrary},
};

pub(crate) fn is_constant(
    expression: &Expr,
    resolutions: &ProgramResolutions,
    program: &Program,
    library: &StandardLibrary,
) -> bool {
    constant_expression(
        expression,
        resolutions,
        program,
        library,
        &[],
        &mut Vec::new(),
    )
}

// Constructor functions may only return another constant expression. This is
// deliberately narrower than general purity: globals, host reads, mutation,
// control flow, and recursive evaluation cannot enter provider configuration.
fn constant_expression(
    expression: &Expr,
    resolutions: &ProgramResolutions,
    program: &Program,
    library: &StandardLibrary,
    parameters: &[&str],
    active: &mut Vec<FunctionId>,
) -> bool {
    let mut constant = |value: &Expr| {
        constant_expression(value, resolutions, program, library, parameters, active)
    };
    match &expression.kind {
        ExprKind::None
        | ExprKind::Bool(_)
        | ExprKind::Int { .. }
        | ExprKind::Float(_)
        | ExprKind::String(_) => true,
        ExprKind::Array(elements) => elements.iter().all(&mut constant),
        ExprKind::Range { start, end, .. } => constant(start) && constant(end),
        ExprKind::Struct { fields, .. } => fields.iter().all(|field| constant(&field.value)),
        ExprKind::Path(path) => {
            resolutions.expression_enum(expression.id).is_some()
                || matches!(path.as_slice(), [name] if parameters.contains(&name.as_str()))
        }
        ExprKind::Call {
            callee,
            receiver: None,
            args,
            ..
        } => {
            if !args.iter().all(&mut constant) {
                return false;
            }
            if resolutions.expression_enum(expression.id).is_some()
                || (callee.as_slice() == ["Some"] && args.len() == 1)
            {
                return true;
            }
            let qualified = callee.join(".");
            let name = match library
                .item_by_name(&qualified)
                .map(|item| item.implementation)
            {
                Some(Implementation::LibraryBody { function_name, .. }) => function_name,
                Some(_) => return false,
                None if callee.len() == 1 => &qualified,
                None => return false,
            };
            let Some(function) = program
                .functions
                .iter()
                .find(|function| function.name == name)
            else {
                return false;
            };
            if function.return_is_async
                || function.return_is_iterator
                || active.contains(&function.id)
                || active.len() >= 64
                || function.params.len() != args.len()
            {
                return false;
            }
            let [Stmt::Expression(body)] = function.body.statements.as_slice() else {
                return false;
            };
            let ExprKind::Return(Some(value)) = &body.kind else {
                return false;
            };
            let Some(parameters) = function
                .params
                .iter()
                .map(|parameter| {
                    parameter
                        .binding
                        .simple_binding()
                        .map(|binding| binding.name.as_str())
                })
                .collect::<Option<Vec<_>>>()
            else {
                return false;
            };
            active.push(function.id);
            let result =
                constant_expression(value, resolutions, program, library, &parameters, active);
            active.pop();
            result
        }
        ExprKind::Unary { expr, .. } => constant(expr),
        _ => false,
    }
}
