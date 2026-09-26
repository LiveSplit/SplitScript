//! Share identical, self-contained return tails without reordering effects.
use std::collections::BTreeMap;
use wasm_encoder::{BlockType, Encode, Instruction as I};

#[derive(Default)]
struct Tail {
    bytes: usize,
    sites: Vec<(usize, usize)>,
}

pub(super) fn fold(ops: &mut Vec<I<'_>>, results: usize, fields: &[Option<usize>]) {
    if !ops.iter().any(|op| matches!(op, I::Return)) {
        return;
    }
    let mut depth = 0;
    let mut depths = Vec::with_capacity(ops.len());
    for op in ops.iter_mut() {
        if super::control::map_labels(op, |d| d).is_err() {
            return;
        }
        depths.push(depth);
        match op {
            I::Block(_) | I::Loop(_) | I::If(_) => depth += 1,
            I::End if depth != 0 => depth -= 1,
            _ => {}
        }
    }
    // A reverse trie shares suffix comparisons. Only closed tails become
    // candidates; their result operands cannot depend on the old stack.
    let mut trie = BTreeMap::new();
    let mut tails = vec![Tail::default()];
    for end in 0..ops.len() {
        if !matches!(ops[end], I::Return) && end != ops.len() - 1 {
            continue;
        }
        let mut node = 0;
        let mut needed = results;
        let mut bytes = 1; // return, including implicit function fallthrough
        for start in (end.saturating_sub(256)..end).rev() {
            let Some((inputs, outputs)) = arity(&ops[start], fields) else {
                break;
            };
            needed = needed.saturating_sub(outputs) + inputs;
            let mut encoded = Vec::new();
            ops[start].encode(&mut encoded);
            bytes += encoded.len();
            node = *trie.entry((node, encoded)).or_insert_with(|| {
                tails.push(Tail {
                    bytes,
                    sites: Vec::new(),
                });
                tails.len() - 1
            });
            if needed == 0 {
                tails[node].sites.push((start, end));
            }
        }
    }
    let best = tails
        .into_iter()
        .filter(|t| t.sites.len() > 1)
        .filter_map(|tail| {
            let branches: usize = tail
                .sites
                .iter()
                .map(|(start, _)| super::encoded_size(&[I::Br(depths[*start])]))
                .sum();
            // Empty wrapper + its end + return for the original fallthrough.
            let cost = branches + tail.bytes + 4;
            (tail.bytes * tail.sites.len())
                .checked_sub(cost)
                .map(|saved| (saved, tail))
        })
        .max_by_key(|(saved, _)| *saved);
    let Some((saved, tail)) = best else { return };
    if saved == 0 {
        return;
    }
    let (start, end) = tail.sites[0];
    let shared = ops[start..end].to_vec();
    let original = std::mem::take(ops);
    ops.push(I::Block(BlockType::Empty));
    let mut sites = tail.sites.into_iter().peekable();
    let mut copied = 0;
    for (index, mut op) in original.into_iter().enumerate() {
        if index < copied {
            continue;
        }
        if sites.peek().is_some_and(|(start, _)| *start == index) {
            let (_, end) = sites.next().unwrap();
            ops.push(I::Br(depths[index]));
            copied = end + 1;
            continue;
        }
        if index == depths.len() - 1 {
            ops.push(I::Return);
            continue;
        }
        // The newly inserted label sits inside the function label. Existing
        // branches to the function must bypass it; all other targets stay put.
        super::control::map_labels(&mut op, |target| {
            target + u32::from(target == depths[index])
        })
        .unwrap();
        ops.push(op);
    }
    ops.push(I::End);
    ops.extend(shared);
    ops.push(I::Return);
    ops.push(I::End);
}

fn arity(op: &I<'_>, fields: &[Option<usize>]) -> Option<(usize, usize)> {
    Some(match op {
        I::Nop => (0, 0),
        I::RefNull(_) | I::RefFunc(_) | I::StructNewDefault(_) => (0, 1),
        I::LocalSet(_) | I::GlobalSet(_) | I::Drop => (1, 0),
        I::StructSet { .. } => (2, 0),
        I::StructNew(ty) => (fields.get(*ty as usize).copied().flatten()?, 1),
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
