//! Share closed identical tails of if arms without reordering executed effects.
use super::{Encode, I, fallthrough::Types};
use wasm_encoder::BlockType;

pub(super) fn fold(ops: &mut Vec<I<'_>>, types: &Types, fields: &[Option<usize>]) {
    let n = ops.len();
    let mut targeted = vec![false; n + 1];
    let mut stack = vec![(n, None)];
    let mut pairs = Vec::new();
    for (at, op) in ops.iter_mut().enumerate() {
        if super::control::map_labels(op, |depth| {
            targeted[stack[stack.len() - 1 - depth as usize].0] = true;
            depth
        })
        .is_err()
        {
            return;
        }
        match op {
            I::Block(_) | I::Loop(_) | I::If(_) => stack.push((at, None)),
            I::Else => stack.last_mut().unwrap().1 = Some(at),
            I::End => {
                if let Some((start, Some(alternate))) = stack.pop() {
                    pairs.push((start, alternate, at));
                }
            }
            _ => {}
        }
    }
    let mut edits = Vec::new();
    for (start, alternate, end) in pairs {
        let I::If(ty @ (BlockType::Empty | BlockType::Result(_))) = ops[start] else {
            continue;
        };
        if targeted[start] {
            continue;
        }
        let mut needed = usize::from(matches!(ty, BlockType::Result(_)));
        let mut best = 0;
        let mut left_bytes = Vec::new();
        let mut right_bytes = Vec::new();
        for count in 1..=256.min(alternate - start - 1).min(end - alternate - 1) {
            let left = &ops[alternate - count];
            let right = &ops[end - count];
            left_bytes.clear();
            right_bytes.clear();
            left.encode(&mut left_bytes);
            right.encode(&mut right_bytes);
            if left_bytes != right_bytes {
                break;
            }
            let Some((inputs, outputs)) = super::stack_locals::arity(left, types, fields) else {
                break;
            };
            if outputs > needed {
                break;
            }
            needed = needed - outputs + inputs;
            if needed == 0 {
                best = count;
            }
        }
        if best > 0 {
            edits.push((start, alternate, end, best));
        }
    }
    // Prefer outer changes and leave overlapping inner opportunities for the
    // next bounded cleanup sweep. Never splice using stale instruction indices.
    if edits.is_empty() {
        return;
    }
    edits.sort_unstable_by_key(|&(start, _, _, _)| start);
    let original = std::mem::take(ops);
    let mut copied = 0;
    for (start, alternate, end, count) in edits {
        if start < copied {
            continue;
        }
        ops.extend_from_slice(&original[copied..start]);
        ops.push(I::If(BlockType::Empty));
        ops.extend_from_slice(&original[start + 1..alternate - count]);
        ops.push(I::Else);
        ops.extend_from_slice(&original[alternate + 1..end - count]);
        ops.push(I::End);
        ops.extend_from_slice(&original[end - count..end]);
        copied = end + 1;
    }
    ops.extend_from_slice(&original[copied..]);
}
