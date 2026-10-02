//! Bounded straight-line copy and stack cleanup. Moving a complete producer
//! across only pure pushes preserves calls, memory effects and trap ordering.
use super::fallthrough::Types;
use wasm_encoder::Instruction as I;

pub(super) fn run(ops: &mut Vec<I<'_>>, count: usize, types: &Types, fields: &[Option<usize>]) {
    // Straight-line copy propagation. Control boundaries discard all facts.
    let mut known: Vec<Option<I<'_>>> = vec![None; count];
    for at in 0..ops.len() {
        match ops[at] {
            I::LocalGet(local) => {
                if let Some(value) = &known[local as usize] {
                    ops[at] = value.clone();
                }
            }
            I::LocalSet(local) | I::LocalTee(local) => {
                let value = at.checked_sub(1).and_then(|prev| match &ops[prev] {
                    I::LocalGet(src) if *src != local => Some(ops[prev].clone()),
                    I::I32Const(_) | I::I64Const(_) => Some(ops[prev].clone()),
                    _ => None,
                });
                for (index, slot) in known.iter_mut().enumerate() {
                    if index == local as usize
                        || matches!(slot, Some(I::LocalGet(src)) if *src == local)
                    {
                        *slot = None;
                    }
                }
                known[local as usize] = value;
            }
            _ if arity(&ops[at], types, fields).is_none() => known.fill(None),
            _ => {}
        }
    }
    // Sink a complete expression through a short sequence of pure pushes,
    // introducing a tee at its first use. No executed effect changes order.
    let mut at = 0;
    while at < ops.len() {
        let I::LocalSet(local) = ops[at] else {
            at += 1;
            continue;
        };
        let mut end = at + 1;
        while end < ops.len() && end - at <= 32 {
            if matches!(ops[end], I::LocalGet(l) if l == local) {
                break;
            }
            if !super::pure_push(&ops[end]) {
                break;
            }
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
        let mut replacement = ops[at + 1..end].to_vec();
        replacement.extend_from_slice(&ops[start..at]);
        replacement.push(I::LocalTee(local));
        ops.splice(start..end + 1, replacement);
        at = end;
    }
    let mut reads = vec![0; count];
    let mut writes = vec![0; count];
    let mut use_at = vec![0; count];
    for (at, op) in ops.iter().enumerate() {
        match op {
            I::LocalGet(l) => {
                reads[*l as usize] += 1;
                use_at[*l as usize] = at;
            }
            I::LocalSet(l) | I::LocalTee(l) => writes[*l as usize] += 1,
            _ => {}
        }
    }
    let mut remove = vec![false; ops.len()];
    for at in 0..ops.len() {
        let I::LocalSet(local) = ops[at] else {
            continue;
        };
        let local = local as usize;
        let end = use_at[local];
        if reads[local] != 1 || writes[local] != 1 || end <= at || end - at > 512 {
            continue;
        }
        let mut height = 0;
        let mut valid = true;
        for next in at + 1..end {
            if remove[next] {
                continue;
            }
            let Some((inputs, outputs)) = arity(&ops[next], types, fields) else {
                valid = false;
                break;
            };
            if inputs > height {
                valid = false;
                break;
            }
            height = height - inputs + outputs;
        }
        if valid && height == 0 {
            remove[at] = true;
            remove[end] = true;
        }
    }
    let mut at = 0;
    ops.retain(|_| {
        let keep = !remove[at];
        at += 1;
        keep
    });
}
fn arity(op: &I<'_>, types: &Types, fields: &[Option<usize>]) -> Option<(usize, usize)> {
    Some(match op {
        I::Call(f) => {
            let ty = types.functions.get(*f as usize)?;
            let wasmparser::CompositeInnerType::Func(ty) =
                &types.types.get(*ty as usize)?.composite_type.inner
            else {
                return None;
            };
            (ty.params().len(), ty.results().len())
        }
        I::Nop => (0, 0),
        I::Drop | I::LocalSet(_) | I::GlobalSet(_) => (1, 0),
        I::LocalTee(_) => (1, 1),
        I::StructNew(ty) => (fields.get(*ty as usize).copied().flatten()?, 1),
        I::StructNewDefault(_) => (0, 1),
        I::StructSet { .. } => (2, 0),
        I::ArrayNew(_) => (2, 1),
        I::ArrayNewDefault(_) => (1, 1),
        I::ArrayNewFixed { array_size, .. } => (*array_size as usize, 1),
        I::ArraySet(_) => (3, 0),
        I::I32Store(_)
        | I::I64Store(_)
        | I::F32Store(_)
        | I::F64Store(_)
        | I::I32Store8(_)
        | I::I32Store16(_)
        | I::I64Store8(_)
        | I::I64Store16(_)
        | I::I64Store32(_) => (2, 0),
        _ => (super::control::condition_inputs(op)?, 1),
    })
}
