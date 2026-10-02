//! Minimize local declarations and LEB-encoded indices without changing slots.
use super::{Encode, Function, I, ValType};

fn leb_size(mut value: usize) -> usize {
    let mut size = 1;
    while value >= 128 {
        value >>= 7;
        size += 1;
    }
    size
}

pub(super) fn reorder(ops: &mut [I<'_>], params: usize, locals: Vec<ValType>) -> Vec<ValType> {
    if locals.len() < 2 {
        return locals;
    }
    let mut counts = vec![0usize; locals.len()];
    for op in ops.iter() {
        if let I::LocalGet(l) | I::LocalSet(l) | I::LocalTee(l) = op
            && let Some(index) = (*l as usize).checked_sub(params)
        {
            counts[index] += 1;
        }
    }
    let keys = locals
        .iter()
        .map(|ty| {
            let mut bytes = Vec::new();
            ty.encode(&mut bytes);
            bytes
        })
        .collect::<Vec<_>>();
    let cost = |order: &[usize]| {
        Function::new_with_locals_types(order.iter().map(|&old| locals[old])).byte_len()
            + order
                .iter()
                .enumerate()
                .map(|(new, &old)| counts[old] * leb_size(params + new))
                .sum::<usize>()
    };
    let mut best: Vec<_> = (0..locals.len()).collect();
    let original_cost = cost(&best);
    let mut best_cost = original_cost;
    let mut frequent = best.clone();
    frequent.sort_by_key(|&old| std::cmp::Reverse(counts[old]));
    let mut grouped = frequent.clone();
    grouped.sort_by(|&a, &b| keys[a].cmp(&keys[b]));
    let mut tiered = frequent.clone();
    let mut start = 0;
    while start < tiered.len() {
        let width = leb_size(params + start);
        let mut end = start + 1;
        while end < tiered.len() && leb_size(params + end) == width {
            end += 1;
        }
        tiered[start..end].sort_by(|&a, &b| keys[a].cmp(&keys[b]));
        start = end;
    }
    for candidate in [frequent, grouped, tiered] {
        let candidate_cost = cost(&candidate);
        if candidate_cost < best_cost {
            best_cost = candidate_cost;
            best = candidate;
        }
    }
    if best_cost == original_cost {
        return locals;
    }
    let mut mapping = vec![0; locals.len()];
    for (new, &old) in best.iter().enumerate() {
        mapping[old] = (params + new) as u32;
    }
    for op in ops {
        if let I::LocalGet(l) | I::LocalSet(l) | I::LocalTee(l) = op
            && let Some(index) = (*l as usize).checked_sub(params)
        {
            *l = mapping[index];
        }
    }
    best.into_iter().map(|old| locals[old]).collect()
}
