//! Bounded direct lookup for sorted expression arenas.

use crate::ast::ExprId;

pub(crate) fn expression_positions<T>(expressions: &[T], id: impl Fn(&T) -> ExprId) -> Vec<usize> {
    let Some(last) = expressions.last() else {
        return Vec::new();
    };
    let Some(length) = id(last).index().checked_add(1) else {
        return Vec::new();
    };
    // Parsed IDs are nearly dense. Keep the sorted lookup for sparse generated
    // IDs instead of allocating an index proportional to an arbitrary ID.
    if length > expressions.len().saturating_mul(2) {
        return Vec::new();
    }
    let mut positions = vec![usize::MAX; length];
    for (position, expression) in expressions.iter().enumerate() {
        positions[id(expression).index()] = position;
    }
    positions
}
