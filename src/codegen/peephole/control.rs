//! Conservative structured-control simplification for compiler-emitted Wasm.
//! Preserve effects, traps and branch destinations; final body size gates apply.
use wasm_encoder::{BlockType, Instruction as I};

/// With an empty if signature, a bare branch cannot carry values from below
/// the if frame. Its only operand is the already evaluated condition.
fn conditional_branches(ops: &mut Vec<I<'_>>) {
    let candidates = ops
        .windows(3)
        .enumerate()
        .filter_map(|(index, window)| match window {
            [I::If(wasm_encoder::BlockType::Empty), I::Br(depth), I::End] => Some((index, *depth)),
            _ => None,
        })
        .collect::<Vec<_>>();
    if candidates.is_empty() {
        return;
    }
    let mut candidates = candidates.into_iter().peekable();
    let mut skip = 0;
    let original = std::mem::take(ops);
    for (index, op) in original.into_iter().enumerate() {
        if skip != 0 {
            skip -= 1;
            continue;
        }
        if candidates.peek().is_some_and(|&(at, _)| at == index) {
            let (_, depth) = candidates.next().unwrap();
            ops.push(if depth == 0 {
                I::Drop
            } else {
                I::BrIf(depth - 1)
            });
            skip = 2;
        } else {
            ops.push(op);
        }
    }
}

pub(super) fn map_labels(op: &mut I<'_>, mut map: impl FnMut(u32) -> u32) -> Result<(), ()> {
    match op {
        I::Br(depth)
        | I::BrIf(depth)
        | I::BrOnNull(depth)
        | I::BrOnNonNull(depth)
        | I::BrOnCast {
            relative_depth: depth,
            ..
        }
        | I::BrOnCastFail {
            relative_depth: depth,
            ..
        }
        | I::BrOnCastDescEq {
            relative_depth: depth,
            ..
        }
        | I::BrOnCastDescEqFail {
            relative_depth: depth,
            ..
        } => *depth = map(*depth),
        I::BrTable(targets, default) => {
            for depth in targets.to_mut() {
                *depth = map(*depth);
            }
            *default = map(*default);
        }
        // Their label/handler semantics need a separate analysis. Preserve
        // the body rather than treating unknown targets as unused labels.
        I::Try(_)
        | I::TryTable(..)
        | I::Delegate(_)
        | I::Rethrow(_)
        | I::Catch(_)
        | I::CatchAll
        | I::Resume { .. }
        | I::ResumeThrow { .. }
        | I::ResumeThrowRef { .. }
        | I::Switch { .. }
        | I::Suspend(_) => return Err(()),
        _ => {}
    }
    Ok(())
}

pub(super) fn remove_unused_labels(ops: &mut Vec<I<'_>>) {
    // Reject unsupported control instructions before making any change.
    if ops
        .iter_mut()
        .any(|op| map_labels(op, |depth| depth).is_err())
    {
        return;
    }
    constant_selections(ops);
    expression_selections(ops);
    simplify_if_arms(ops);
    conditional_branches(ops);
    // The implicit function label is always retained.
    let root = ops.len();
    let mut targeted = vec![false; root + 1];
    targeted[root] = true;
    let mut stack = vec![root];
    for (position, op) in ops.iter_mut().enumerate() {
        if map_labels(op, |depth| {
            targeted[stack[stack.len() - 1 - depth as usize]] = true;
            depth
        })
        .is_err()
        {
            return;
        }
        match op {
            I::Block(_) | I::Loop(_) | I::If(_) => stack.push(position),
            I::End => {
                stack.pop();
            }
            _ => {}
        }
    }
    let remove = ops
        .iter()
        .enumerate()
        .map(|(index, op)| matches!(op, I::Block(_) | I::Loop(_)) && !targeted[index])
        .collect::<Vec<_>>();
    if !remove.iter().any(|&remove| remove) {
        return;
    }

    // Entries contain whether a frame survives and its new absolute depth.
    let mut frames = vec![(true, 0usize)];
    let mut kept = 1usize;
    let mut position = 0;
    ops.retain_mut(|op| {
        let index = position;
        position += 1;
        map_labels(op, |depth| {
            let (survives, target) = frames[frames.len() - 1 - depth as usize];
            debug_assert!(survives);
            (kept - 1 - target) as u32
        })
        .unwrap();
        match op {
            I::Block(_) | I::Loop(_) | I::If(_) => {
                let survives = !remove[index];
                frames.push((survives, kept));
                kept += usize::from(survives);
                survives
            }
            I::End => {
                let (survives, _) = frames.pop().unwrap();
                kept -= usize::from(survives);
                survives
            }
            _ => true,
        }
    });
}

/// Keep the condition's effects and the original typed label. Replacing the
/// if with a block preserves branch depths, block parameters and return paths.
fn simplify_if_arms(ops: &mut Vec<I<'_>>) {
    use wasm_encoder::Encode;
    enum Change {
        Constant(bool),
        Identical(bool),
        Swap,
    }
    let mut stack = Vec::new();
    let mut changes = Vec::new();
    for (at, op) in ops.iter().enumerate() {
        match op {
            I::Block(_) | I::Loop(_) | I::If(_) => stack.push((at, None)),
            I::Else => stack.last_mut().unwrap().1 = Some(at),
            I::End => {
                let Some((start, alternate)) = stack.pop() else {
                    continue;
                };
                if !matches!(ops[start], I::If(_)) {
                    continue;
                }
                if start != 0
                    && let I::I32Const(value) = ops[start - 1]
                {
                    changes.push((start, alternate, at, Change::Constant(value != 0)));
                    continue;
                }
                let Some(alternate) = alternate else { continue };
                let count = alternate - start - 1;
                // Bound comparisons even for deeply nested candidate arms.
                let mut identical = false;
                if count != 0 && count <= 256 && count == at - alternate - 1 {
                    let mut left = Vec::new();
                    let mut right = Vec::new();
                    for op in &ops[start + 1..alternate] {
                        op.encode(&mut left);
                    }
                    for op in &ops[alternate + 1..at] {
                        op.encode(&mut right);
                    }
                    identical = left == right;
                }
                let negate = start != 0 && matches!(ops[start - 1], I::I32Eqz);
                if identical || negate {
                    changes.push((
                        start,
                        Some(alternate),
                        at,
                        if identical {
                            Change::Identical(negate)
                        } else {
                            Change::Swap
                        },
                    ));
                }
            }
            _ => {}
        }
    }
    if changes.is_empty() {
        return;
    }
    // Prefer the outermost match; overlapping inner matches can wait for the
    // next bounded cleanup sweep. Never splice using already-shifted indices.
    changes.sort_unstable_by_key(|&(start, _, _, _)| start);
    let original = std::mem::take(ops);
    let mut copied = 0;
    for (start, alternate, end, change) in changes {
        if start < copied {
            continue;
        }
        let I::If(ty) = original[start] else {
            unreachable!()
        };
        let remove_condition = !matches!(change, Change::Identical(false));
        ops.extend_from_slice(&original[copied..start - usize::from(remove_condition)]);
        match change {
            Change::Constant(taken) => {
                // Keep the original typed label, including block parameters
                // and branch payloads. Label cleanup can remove it if unused.
                ops.push(I::Block(ty));
                if taken {
                    ops.extend_from_slice(&original[start + 1..alternate.unwrap_or(end)]);
                } else if let Some(alternate) = alternate {
                    ops.extend_from_slice(&original[alternate + 1..end]);
                }
            }
            Change::Identical(_) => {
                ops.extend([I::Drop, I::Block(ty)]);
                ops.extend_from_slice(&original[start + 1..alternate.unwrap()]);
            }
            Change::Swap => {
                let alternate = alternate.unwrap();
                // Swapping complete arms leaves their typed label and every
                // nested relative branch depth unchanged.
                ops.push(I::If(ty));
                ops.extend_from_slice(&original[alternate + 1..end]);
                ops.push(I::Else);
                ops.extend_from_slice(&original[start + 1..alternate]);
            }
        }
        ops.push(I::End);
        copied = end + 1;
    }
    ops.extend_from_slice(&original[copied..]);
}

/// Factor the final assignment in both arms into one assignment after the if.
/// No expressions move between arms or across effects. An explicit branch to
/// the if's label could bypass an assignment, so such joins are excluded.
pub(super) fn merge_if_assignments(
    ops: &mut Vec<I<'_>>,
    parameters: usize,
    locals: &[wasm_encoder::ValType],
) {
    let root = ops.len();
    let mut targeted = vec![false; root + 1];
    let mut stack = vec![(root, None)];
    let mut pairs = Vec::new();
    for (position, op) in ops.iter_mut().enumerate() {
        if map_labels(op, |depth| {
            targeted[stack[stack.len() - 1 - depth as usize].0] = true;
            depth
        })
        .is_err()
        {
            return;
        }
        match op {
            I::Block(_) | I::Loop(_) | I::If(_) => stack.push((position, None)),
            I::Else => stack.last_mut().unwrap().1 = Some(position),
            I::End => {
                let (start, alternate) = stack.pop().unwrap();
                if let Some(alternate) = alternate {
                    pairs.push((start, alternate, position));
                }
            }
            _ => {}
        }
    }
    let mut remove = vec![false; root];
    let mut append = vec![None; root];
    let mut changed = false;
    for (start, alternate, end) in pairs {
        if targeted[start] || !matches!(ops[start], I::If(wasm_encoder::BlockType::Empty)) {
            continue;
        }
        let (I::LocalSet(left), I::LocalSet(right)) = (&ops[alternate - 1], &ops[end - 1]) else {
            continue;
        };
        if left != right || (*left as usize) < parameters {
            continue;
        }
        let index = *left;
        let ty = locals[index as usize - parameters];
        ops[start] = I::If(wasm_encoder::BlockType::Result(ty));
        remove[alternate - 1] = true;
        remove[end - 1] = true;
        append[end] = Some(index);
        changed = true;
    }
    if !changed {
        return;
    }
    let original = std::mem::take(ops);
    for (index, op) in original.into_iter().enumerate() {
        if !remove[index] {
            ops.push(op);
        }
        if let Some(local) = append[index] {
            ops.push(I::LocalSet(local));
        }
    }
}

fn selectable(op: &I<'_>) -> bool {
    matches!(
        op,
        I::I32Const(_) | I::I64Const(_) | I::F32Const(_) | I::F64Const(_) | I::LocalGet(_)
    )
}

// Only instructions with one result and a known stack effect. Unknown calls,
// control boundaries and writes stop the backwards expression walk.
pub(super) fn condition_inputs(op: &I<'_>) -> Option<usize> {
    Some(match op {
        op if selectable(op) => 0,
        I::GlobalGet(_) => 0,
        I::I32Eqz
        | I::I64Eqz
        | I::I32WrapI64
        | I::I64ExtendI32S
        | I::I64ExtendI32U
        | I::RefIsNull
        | I::RefAsNonNull
        | I::LocalTee(_)
        | I::StructGet { .. }
        | I::StructGetS { .. }
        | I::StructGetU { .. }
        | I::ArrayLen
        | I::RefTestNonNull(_)
        | I::RefTestNullable(_)
        | I::RefCastNonNull(_)
        | I::RefCastNullable(_)
        | I::I32Load(_)
        | I::I64Load(_)
        | I::F32Load(_)
        | I::F64Load(_)
        | I::I32Load8S(_)
        | I::I32Load8U(_)
        | I::I32Load16S(_)
        | I::I32Load16U(_)
        | I::I64Load8S(_)
        | I::I64Load8U(_)
        | I::I64Load16S(_)
        | I::I64Load16U(_)
        | I::I64Load32S(_)
        | I::I64Load32U(_) => 1,
        I::I32Eq
        | I::I32Ne
        | I::I32LtS
        | I::I32LtU
        | I::I32GtS
        | I::I32GtU
        | I::I32LeS
        | I::I32LeU
        | I::I32GeS
        | I::I32GeU
        | I::I64Eq
        | I::I64Ne
        | I::I64LtS
        | I::I64LtU
        | I::I64GtS
        | I::I64GtU
        | I::I64LeS
        | I::I64LeU
        | I::I64GeS
        | I::I64GeU
        | I::I32Add
        | I::I32Sub
        | I::I32Mul
        | I::I32And
        | I::I32Or
        | I::I32Xor
        | I::I32Shl
        | I::I32ShrS
        | I::I32ShrU
        | I::I64Add
        | I::I64Sub
        | I::I64Mul
        | I::I64And
        | I::I64Or
        | I::I64Xor
        | I::I64Shl
        | I::I64ShrS
        | I::I64ShrU
        | I::ArrayGet(_)
        | I::ArrayGetS(_)
        | I::ArrayGetU(_) => 2,
        _ => return None,
    })
}

fn constant_selections(ops: &mut Vec<I<'_>>) {
    use wasm_encoder::{BlockType, Encode, ValType};
    let mut changes = Vec::new();
    for (index, window) in ops.windows(5).enumerate() {
        let [I::If(BlockType::Result(ty)), yes, I::Else, no, I::End] = window else {
            continue;
        };
        if !matches!(
            ty,
            ValType::I32 | ValType::I64 | ValType::F32 | ValType::F64
        ) || !selectable(yes)
            || !selectable(no)
        {
            continue;
        }
        let mut a = Vec::new();
        let mut b = Vec::new();
        yes.encode(&mut a);
        no.encode(&mut b);
        let mut start = index;
        let replacement = if a == b {
            vec![I::Drop, yes.clone()]
        } else if matches!(
            (yes, no),
            (I::I32Const(1), I::I32Const(0))
                | (I::I64Const(1), I::I64Const(0))
                | (I::I32Const(0), I::I32Const(1))
                | (I::I64Const(0), I::I64Const(1))
        ) {
            let mut replacement = vec![I::I32Eqz];
            if matches!(yes, I::I32Const(1) | I::I64Const(1)) {
                replacement.push(I::I32Eqz);
            }
            if *ty == ValType::I64 {
                replacement.push(I::I64ExtendI32U);
            }
            replacement
        } else {
            // Move only the two nontrapping values before the complete
            // condition expression. A condition that updates an arm's local
            // would change the selected value and must be left intact.
            let mut needed = 1;
            while start != 0 && needed != 0 {
                let op = &ops[start - 1];
                let Some(inputs) = condition_inputs(op) else {
                    break;
                };
                if let I::LocalTee(written) = op
                    && [yes, no]
                        .iter()
                        .any(|arm| matches!(arm, I::LocalGet(read) if read == written))
                {
                    break;
                }
                needed = needed - 1 + inputs;
                start -= 1;
            }
            if needed != 0 {
                continue;
            }
            let mut replacement = vec![yes.clone(), no.clone()];
            replacement.extend_from_slice(&ops[start..index]);
            replacement.push(I::Select);
            replacement
        };
        if changes.last().is_some_and(|(_, end, _)| start <= *end) {
            continue;
        }
        if super::encoded_size(&replacement) < super::encoded_size(&ops[start..index + 5]) {
            changes.push((start, index + 4, replacement));
        }
    }
    if changes.is_empty() {
        return;
    }
    let mut changes = changes.into_iter().peekable();
    let mut until = 0;
    let original = std::mem::take(ops);
    for (index, op) in original.into_iter().enumerate() {
        if index < until {
            continue;
        }
        if changes.peek().is_some_and(|(start, _, _)| *start == index) {
            let (_, end, replacement) = changes.next().unwrap();
            ops.extend(replacement);
            until = end + 1;
        } else {
            ops.push(op);
        }
    }
}
/// Eagerly evaluate only closed, nontrapping arms. Their local/global reads
/// must not move ahead of a condition that can change those values.
fn expression_selections(ops: &mut Vec<I<'_>>) {
    let mut frames = Vec::new();
    let mut changes = Vec::new();
    for (end, op) in ops.iter().enumerate() {
        match op {
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
                let mut condition = start;
                let mut needed = 1;
                while condition > start.saturating_sub(256) && needed != 0 {
                    let op = &ops[condition - 1];
                    let Some(inputs) = condition_inputs(op) else {
                        break;
                    };
                    if matches!(op, I::LocalTee(i) if reads.contains(i)) {
                        break;
                    }
                    needed = needed - 1 + inputs;
                    condition -= 1;
                }
                if needed != 0
                    || changes
                        .last()
                        .is_some_and(|&(_, previous_end, _)| condition <= previous_end)
                {
                    continue;
                }
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
                    changes.push((condition, end, replacement));
                }
            }
            _ => {}
        }
    }
    if changes.is_empty() {
        return;
    }
    let original = std::mem::take(ops);
    let mut copied = 0;
    for (start, end, replacement) in changes {
        ops.extend_from_slice(&original[copied..start]);
        ops.extend(replacement);
        copied = end + 1;
    }
    ops.extend_from_slice(&original[copied..]);
}

fn closed_pure_expression(ops: &[I<'_>]) -> bool {
    if ops.is_empty() || ops.len() > 32 {
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
    Some(match op {
        I::I32Const(_)
        | I::I64Const(_)
        | I::F32Const(_)
        | I::F64Const(_)
        | I::LocalGet(_)
        | I::GlobalGet(_)
        | I::RefNull(_)
        | I::RefFunc(_) => 0,
        I::I32Eqz
        | I::I64Eqz
        | I::I32WrapI64
        | I::I64ExtendI32S
        | I::I64ExtendI32U
        | I::RefIsNull
        | I::RefTestNonNull(_)
        | I::RefTestNullable(_) => 1,
        I::I32Eq
        | I::I32Ne
        | I::I32LtS
        | I::I32LtU
        | I::I32GtS
        | I::I32GtU
        | I::I32LeS
        | I::I32LeU
        | I::I32GeS
        | I::I32GeU
        | I::I64Eq
        | I::I64Ne
        | I::I64LtS
        | I::I64LtU
        | I::I64GtS
        | I::I64GtU
        | I::I64LeS
        | I::I64LeU
        | I::I64GeS
        | I::I64GeU
        | I::I32Add
        | I::I32Sub
        | I::I32Mul
        | I::I32And
        | I::I32Or
        | I::I32Xor
        | I::I32Shl
        | I::I32ShrS
        | I::I32ShrU
        | I::I64Add
        | I::I64Sub
        | I::I64Mul
        | I::I64And
        | I::I64Or
        | I::I64Xor
        | I::I64Shl
        | I::I64ShrS
        | I::I64ShrU => 2,
        _ => return None,
    })
}
