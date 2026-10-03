//! Backward local liveness on structured control flow. Dead stores retain their
//! producers; same-typed slots share storage only without interference.
//! The worklist and body/local limits bound compile time. No partial liveness
//! result is used when the analysis budget is exhausted.
use super::{I, ValType};
use std::collections::VecDeque;

pub(super) fn reuse(
    ops: &mut [I<'_>],
    params: &[ValType],
    locals: &[ValType],
    types: &super::fallthrough::Types,
    fields: &[Option<usize>],
) {
    let n = ops.len();
    let count = params.len() + locals.len();
    if count == 0 || count > 1024 || n > 50000 {
        return;
    }
    let mut ends = vec![n; n];
    let mut alternates = vec![None; n];
    let mut stack = Vec::new();
    for (at, op) in ops.iter().enumerate() {
        match op {
            I::Block(_) | I::Loop(_) | I::If(_) => stack.push(at),
            I::Else => {
                let Some(&start) = stack.last() else { return };
                alternates[start] = Some(at);
            }
            I::End => {
                if let Some(start) = stack.pop() {
                    ends[start] = at;
                    if let Some(e) = alternates[start] {
                        ends[e] = at;
                    }
                }
            }
            _ => {}
        }
    }
    let mut succ = vec![Vec::new(); n];
    stack.push(n);
    for (at, op) in ops.iter().enumerate() {
        let mut targets = Vec::new();
        if super::control::map_labels(&mut op.clone(), |d| {
            targets.push(d);
            d
        })
        .is_err()
        {
            return;
        }
        for depth in targets {
            let Some(&frame) = stack.get(stack.len().saturating_sub(depth as usize + 1)) else {
                return;
            };
            let target = if frame == n {
                n
            } else if matches!(ops[frame], I::Loop(_)) {
                frame + 1
            } else {
                ends[frame] + 1
            };
            if target < n {
                succ[at].push(target);
            }
        }
        match op {
            I::Block(_) | I::Loop(_) => {
                stack.push(at);
                if at + 1 < n {
                    succ[at].push(at + 1);
                }
            }
            I::If(_) => {
                stack.push(at);
                succ[at].push(at + 1);
                let other = alternates[at].map_or(ends[at], |e| e + 1);
                if other < n {
                    succ[at].push(other);
                }
            }
            I::Else => {
                let target = ends[at] + 1;
                if target < n {
                    succ[at].push(target);
                }
            }
            I::End => {
                stack.pop();
                if at + 1 < n {
                    succ[at].push(at + 1);
                }
            }
            I::Br(_)
            | I::BrTable(..)
            | I::Return
            | I::Unreachable
            | I::ReturnCall(_)
            | I::ReturnCallIndirect { .. }
            | I::ReturnCallRef(_) => {}
            _ => {
                if at + 1 < n {
                    succ[at].push(at + 1);
                }
            }
        }
    }
    let words = count.div_ceil(64);
    let mut live = vec![vec![0u64; words]; n];
    let mut pred = vec![Vec::new(); n];
    for (at, next) in succ.iter().enumerate() {
        for &next in next {
            pred[next].push(at);
        }
    }
    let mut pending = (0..n).rev().collect::<VecDeque<_>>();
    let mut queued = vec![true; n];
    let mut out = vec![0u64; words];
    let mut budget = n * 64;
    while let Some(at) = pending.pop_front() {
        if budget == 0 {
            return;
        }
        budget -= 1;
        queued[at] = false;
        out.fill(0);
        for &next in &succ[at] {
            for (a, b) in out.iter_mut().zip(&live[next]) {
                *a |= *b;
            }
        }
        match ops[at] {
            I::LocalGet(l) => out[l as usize / 64] |= 1u64 << (l % 64),
            I::LocalSet(l) | I::LocalTee(l) => out[l as usize / 64] &= !(1u64 << (l % 64)),
            _ => {}
        }
        if live[at] != out {
            live[at].clone_from(&out);
            for &prev in &pred[at] {
                if !queued[prev] {
                    queued[prev] = true;
                    pending.push_back(prev);
                }
            }
        }
    }
    let needs_initialization = |local: u32| {
        (local as usize)
            .checked_sub(params.len())
            .and_then(|index| locals.get(index))
            .is_some_and(|ty| matches!(ty,ValType::Ref(reference) if !reference.nullable))
    };
    for at in 0..n {
        let (local, tee) = match ops[at] {
            I::LocalSet(l) => (l, false),
            I::LocalTee(l) => (l, true),
            _ => continue,
        };
        // Runtime liveness alone cannot remove the initializer required by
        // Wasm's structural proof, even when both subsequent arms overwrite it.
        if needs_initialization(local) {
            continue;
        }
        if !succ[at]
            .iter()
            .any(|&next| live[next][local as usize / 64] >> (local % 64) & 1 != 0)
        {
            ops[at] = if tee { I::Nop } else { I::Drop };
        }
    }
    // Keep a stored value on the operand stack until its first read when the
    // intervening straight-line statements balance their own stack. If later
    // reads still need the local, assign it with a tee at that first read.
    for at in 0..n {
        let I::LocalSet(local) = ops[at] else {
            continue;
        };
        let mut height = 0;
        for end in at + 1..n.min(at + 513) {
            if matches!(ops[end], I::LocalSet(l) | I::LocalTee(l) if l == local) {
                break;
            }
            if matches!(ops[end], I::LocalGet(l) if l == local) {
                if height == 0 {
                    let needed = needs_initialization(local)
                        || succ[end]
                            .iter()
                            .any(|&next| live[next][local as usize / 64] >> (local % 64) & 1 != 0);
                    ops[at] = I::Nop;
                    ops[end] = if needed { I::LocalTee(local) } else { I::Nop };
                }
                break;
            }
            let Some((inputs, outputs)) = super::stack_locals::arity(&ops[end], types, fields)
            else {
                break;
            };
            if inputs > height {
                break;
            }
            height = height - inputs + outputs;
        }
    }
    // Copies allow equal values to share a slot; all other writes interfere
    // with the values that must survive them. Parameter slots stay fixed.
    let mut graph = vec![vec![false; count]; count];
    let mut hints = vec![Vec::new(); count];
    let mut frequencies = vec![0usize; count];
    for (at, op) in ops.iter().enumerate() {
        let local = match op {
            I::LocalGet(l) | I::LocalSet(l) | I::LocalTee(l) => *l as usize,
            _ => continue,
        };
        frequencies[local] += 1;
        if matches!(op, I::LocalGet(_)) {
            continue;
        }
        let source = if at > 0 {
            if let I::LocalGet(l) = ops[at - 1] {
                Some(l as usize)
            } else {
                None
            }
        } else {
            None
        };
        if let Some(source) = source {
            hints[local].push(source);
            hints[source].push(local);
        }
        out.fill(0);
        for &next in &succ[at] {
            for (a, b) in out.iter_mut().zip(&live[next]) {
                *a |= *b;
            }
        }
        for other in 0..count {
            if Some(other) != source && other != local && out[other / 64] >> (other % 64) & 1 != 0 {
                graph[local][other] = true;
                graph[other][local] = true;
            }
        }
    }
    // A local read at entry needs its default, not a parameter's incoming value.
    for local in params.len()..count {
        if live[0][local / 64] >> (local % 64) & 1 != 0 {
            graph[local][..params.len()].fill(true);
            for row in graph.iter_mut().take(params.len()) {
                row[local] = true;
            }
        }
    }
    let types = params.iter().chain(locals).copied().collect::<Vec<_>>();
    let mut mapping = vec![usize::MAX; count];
    for (p, m) in mapping.iter_mut().enumerate().take(params.len()) {
        *m = p;
    }
    let mut order = (params.len()..count).collect::<Vec<_>>();
    order.sort_by_key(|&l| std::cmp::Reverse(frequencies[l]));
    let mut slots = (0..params.len()).collect::<Vec<_>>();
    for local in order {
        let allowed = |slot: usize| {
            types[slot] == types[local]
                && (0..count).all(|other| !graph[local][other] || mapping[other] != slot)
        };
        let hint = hints[local]
            .iter()
            .map(|&h| mapping[h])
            .find(|&slot| slot != usize::MAX && allowed(slot));
        let slot = hint
            .or_else(|| slots.iter().copied().find(|&slot| allowed(slot)))
            .unwrap_or_else(|| {
                slots.push(local);
                local
            });
        mapping[local] = slot;
    }
    for op in ops {
        if let I::LocalGet(l) | I::LocalSet(l) | I::LocalTee(l) = op {
            *l = mapping[*l as usize] as u32;
        }
    }
}
