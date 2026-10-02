//! Compact conditional exits while preserving evaluation and label semantics.
use super::{I, fallthrough::Types};
use wasm_encoder::BlockType;

pub(super) fn fold(ops: &mut Vec<I<'_>>, types: &Types, fields: &[Option<usize>]) {
    leading_exits(ops, types, fields);
    let mut changes = Vec::new();
    for at in 0..ops.len().saturating_sub(3) {
        let [I::If(BlockType::Empty), value, I::Br(depth), I::End] = &ops[at..at + 4] else {
            continue;
        };
        if *depth == 0
            || !matches!(
                value,
                I::I32Const(_)
                    | I::I64Const(_)
                    | I::F32Const(_)
                    | I::F64Const(_)
                    | I::RefNull(_)
                    | I::RefFunc(_)
            )
        {
            continue;
        }
        let mut start = at;
        let mut needed = 1;
        while start > at.saturating_sub(256) && needed > 0 {
            let Some((inputs, outputs)) =
                super::stack_locals::arity(&ops[start - 1], types, fields)
            else {
                break;
            };
            if outputs > needed {
                break;
            }
            needed = needed - outputs + inputs;
            start -= 1;
        }
        if needed != 0 || changes.last().is_some_and(|&(_, end, _)| start < end) {
            continue;
        }
        let mut replacement = vec![value.clone()];
        replacement.extend_from_slice(&ops[start..at]);
        replacement.extend([I::BrIf(depth - 1), I::Drop]);
        changes.push((start, at + 4, replacement));
    }
    for (start, end, replacement) in changes.into_iter().rev() {
        ops.splice(start..end, replacement);
    }
}

// A block beginning with a conditional exit is an if/else expression. Keep
// its label around the old continuation, so all of that code's targets stay
// unchanged. Both the condition and early value must be closed expressions.
fn leading_exits(ops: &mut Vec<I<'_>>, types: &Types, fields: &[Option<usize>]) {
    let mut changes = Vec::new();
    for start in 0..ops.len() {
        let I::Block(ty @ (BlockType::Empty | BlockType::Result(_))) = ops[start] else {
            continue;
        };
        let mut at = start + 1;
        let mut height = 0;
        while at < ops.len() && at - start < 256 {
            let Some((inputs, outputs)) = super::stack_locals::arity(&ops[at], types, fields)
            else {
                break;
            };
            if inputs > height {
                break;
            }
            height = height - inputs + outputs;
            at += 1;
        }
        if height != 1 || !matches!(ops.get(at), Some(I::If(BlockType::Empty))) {
            continue;
        }
        let arm = at + 1;
        let mut end = arm;
        height = 0;
        while end < ops.len() && end - arm < 256 {
            let Some((inputs, outputs)) = super::stack_locals::arity(&ops[end], types, fields)
            else {
                break;
            };
            if inputs > height {
                break;
            }
            height = height - inputs + outputs;
            end += 1;
        }
        if height != usize::from(matches!(ty, BlockType::Result(_)))
            || !matches!(ops.get(end..end + 2), Some([I::Br(1), I::End]))
            || changes.last().is_some_and(|&(_, prior, _)| start < prior)
        {
            continue;
        }
        let mut replacement = ops[start + 1..at].to_vec();
        replacement.push(I::If(ty));
        replacement.extend_from_slice(&ops[arm..end]);
        replacement.push(I::Else);
        changes.push((start, end + 2, replacement));
    }
    for (start, end, replacement) in changes.into_iter().rev() {
        ops.splice(start..end, replacement);
    }
}
