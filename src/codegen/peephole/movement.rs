//! Late size-gated expression movement. The established pipeline runs first,
//! so early rewrites cannot hide a smaller inlining or local-allocation result.
use super::{I, fallthrough::Types};
use wasm_encoder::BlockType;

fn condition_inputs(op: &I<'_>) -> Option<usize> {
    super::discarded_values::inputs(op).or_else(|| super::control::condition_inputs(op))
}
fn arity(op: &I<'_>, types: &Types, fields: &[Option<usize>]) -> Option<(usize, usize)> {
    super::discarded_values::inputs(op)
        .map(|n| (n, 1))
        .or_else(|| super::stack_locals::arity(op, types, fields))
}

pub(super) fn sink(ops: &mut Vec<I<'_>>, types: &Types, fields: &[Option<usize>]) {
    let mut at = 0;
    while at < ops.len() {
        let I::LocalSet(local) = ops[at] else {
            at += 1;
            continue;
        };
        let mut end = at + 1;
        let mut height = 0;
        while end < ops.len() && end - at <= 256 {
            if matches!(ops[end], I::LocalGet(l) if l == local) {
                break;
            }
            if matches!(ops[end], I::LocalSet(l) | I::LocalTee(l) if l == local) {
                break;
            }
            let Some((inputs, outputs)) = arity(&ops[end], types, fields) else {
                break;
            };
            if inputs > height {
                break;
            }
            height = height - inputs + outputs;
            end += 1;
        }
        if end <= at + 1 || end >= ops.len() || !matches!(ops[end],I::LocalGet(l) if l==local) {
            at += 1;
            continue;
        }
        let mut needed = 1;
        let mut start = at;
        let mut writes = Vec::new();
        while start > 0 && at - start < 512 && needed > 0 {
            let Some((inputs, outputs)) = arity(&ops[start - 1], types, fields) else {
                break;
            };
            if outputs > needed {
                break;
            }
            needed = needed - outputs + inputs;
            if let I::LocalSet(l) | I::LocalTee(l) = ops[start - 1] {
                writes.push(l);
            }
            start -= 1;
        }
        if needed != 0
            || ops[at + 1..end]
                .iter()
                .any(|op| matches!(op,I::LocalGet(l) if writes.contains(l)))
        {
            at += 1;
            continue;
        }
        // Moving a producer past another expression is safe if one side
        // consists solely of nontrapping, state-independent calculations,
        // or both sides only read external state and one cannot trap. Fresh
        // allocations cannot change a global or linear-memory size. Local
        // dependencies still matter even though the value reads are inert.
        let inert = |op: &I<'_>| {
            super::discarded_values::inputs(op).is_some()
                && !matches!(op, I::GlobalGet(_) | I::MemorySize(_))
        };
        let producer = &ops[start..at];
        let intervening = &ops[at + 1..end];
        let producer_inert = producer.iter().all(inert);
        let intervening_inert = intervening.iter().all(inert);
        let nontrapping = |op: &I<'_>| super::discarded_values::inputs(op).is_some();
        let readonly = |op: &I<'_>| {
            condition_inputs(op).is_some()
                || matches!(
                    op,
                    I::LocalSet(_)
                        | I::Drop
                        | I::Nop
                        | I::StructNew(_)
                        | I::StructNewDefault(_)
                        | I::ArrayNew(_)
                        | I::ArrayNewDefault(_)
                        | I::ArrayNewFixed { .. }
                )
        };
        let compatible_reads = (producer.iter().all(nontrapping)
            && intervening.iter().all(readonly))
            || (intervening.iter().all(nontrapping) && producer.iter().all(readonly));
        if !producer_inert && !intervening_inert && !compatible_reads {
            at += 1;
            continue;
        }
        if intervening.iter().any(|op| {
            matches!(op, I::LocalSet(written) | I::LocalTee(written)
                if producer.iter().any(|op| matches!(op, I::LocalGet(read) if read == written)))
        }) {
            at += 1;
            continue;
        }
        let mut replacement = ops[at + 1..end].to_vec();
        replacement.extend_from_slice(&ops[start..at]);
        replacement.push(I::LocalTee(local));
        ops.splice(start..end + 1, replacement);
        at = end;
    }
}

pub(super) fn selections(ops: &mut Vec<I<'_>>) {
    let mut frames = Vec::new();
    let original = std::mem::take(ops);
    ops.reserve(original.len());
    // Rewrite completed inner expressions before inspecting their parents.
    // This handles chains in one traversal while retaining the scan limits.
    for op in original {
        let end = ops.len();
        ops.push(op);
        match &ops[end] {
            I::Block(_) | I::Loop(_) | I::If(_) => frames.push((end, None)),
            I::Else => {
                if let Some(frame) = frames.last_mut() {
                    frame.1 = Some(end);
                }
            }
            I::End => {
                let Some((start, Some(alternate))) = frames.pop() else {
                    continue;
                };
                let I::If(BlockType::Result(ty)) = ops[start] else {
                    continue;
                };
                let yes = &ops[start + 1..alternate];
                let no = &ops[alternate + 1..end];
                if !closed_pure_expression(yes) || !closed_pure_expression(no) {
                    continue;
                }
                let reads = yes
                    .iter()
                    .chain(no)
                    .filter_map(|op| match op {
                        I::LocalGet(i) => Some(*i),
                        _ => None,
                    })
                    .collect::<Vec<_>>();
                let Some(condition) = selection_condition_start(ops, start, &reads) else {
                    continue;
                };
                let mut replacement = Vec::new();
                replacement.extend_from_slice(yes);
                replacement.extend_from_slice(no);
                replacement.extend_from_slice(&ops[condition..start]);
                replacement.push(if matches!(ty, wasm_encoder::ValType::Ref(_)) {
                    I::TypedSelect(ty)
                } else {
                    I::Select
                });
                if super::encoded_size(&replacement) < super::encoded_size(&ops[condition..=end]) {
                    ops.truncate(condition);
                    ops.extend(replacement);
                }
            }
            _ => {}
        }
    }
}

fn closed_pure_expression(ops: &[I<'_>]) -> bool {
    if ops.is_empty() || ops.len() > 256 {
        return false;
    }
    let mut needed = 1;
    for op in ops.iter().rev() {
        if needed == 0 {
            return false;
        }
        let Some(inputs) = pure_expression_inputs(op) else {
            return false;
        };
        needed = needed - 1 + inputs;
    }
    needed == 0
}

fn pure_expression_inputs(op: &I<'_>) -> Option<usize> {
    super::discarded_values::inputs(op)
}

// Walk a complete condition, including structured expressions with no label
// exits or observable writes. Only inert arms move before it; the condition
// keeps its own evaluation order, local effects, and potentially trapping reads.
fn selection_condition_start(ops: &[I<'_>], end: usize, reads: &[u32]) -> Option<usize> {
    let lower = end.saturating_sub(256);
    let safe = |op: &I<'_>| {
        !matches!(op, I::LocalSet(i) | I::LocalTee(i) if reads.contains(i))
            && (condition_inputs(op).is_some() || matches!(op, I::LocalSet(_) | I::Drop | I::Nop))
    };
    let mut cursor = end;
    let mut needed = 1;
    while cursor > lower && needed != 0 {
        cursor -= 1;
        let op = &ops[cursor];
        let inputs = if matches!(op, I::End) {
            let mut depth = 1;
            loop {
                if cursor == lower {
                    return None;
                }
                cursor -= 1;
                match &ops[cursor] {
                    I::End => depth += 1,
                    I::If(BlockType::Empty | BlockType::Result(_))
                    | I::Block(BlockType::Empty | BlockType::Result(_)) => {
                        depth -= 1;
                        if depth == 0 {
                            break;
                        }
                    }
                    I::Else => {}
                    op if safe(op) => {}
                    _ => return None,
                }
            }
            match ops[cursor] {
                I::If(BlockType::Result(_)) => 1,
                I::Block(BlockType::Result(_)) => 0,
                _ => return None,
            }
        } else {
            if !safe(op) {
                return None;
            }
            condition_inputs(op)?
        };
        needed = needed - 1 + inputs;
    }
    (needed == 0).then_some(cursor)
}
