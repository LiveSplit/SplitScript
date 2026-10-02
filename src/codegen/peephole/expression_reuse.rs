//! Reuse closed expressions within straight-line regions when encoded bytes
//! outweigh the temporary's stores, reads and declaration. No effect is moved.
use super::{Encode, I, ValType, fallthrough::Types};
use std::collections::HashMap;
use wasm_encoder::reencode::{Reencode, RoundtripReencoder};

struct Expression {
    start: usize,
    end: usize,
    key: Vec<u8>,
    ty: ValType,
}
struct Group {
    uses: Vec<(usize, usize)>,
    ty: ValType,
    bytes: usize,
}

pub(super) fn run(
    ops: &mut Vec<I<'_>>,
    params: &[ValType],
    locals: &mut Vec<ValType>,
    types: &Types,
    fields: &[Option<usize>],
) {
    if ops.len() > 50_000 || params.len() + locals.len() > 1024 {
        return;
    }
    for op in ops.iter_mut() {
        if super::control::map_labels(op, |depth| depth).is_err() {
            return;
        }
    }
    let local_types: Vec<_> = params.iter().chain(locals.iter()).copied().collect();
    let mut versions = vec![0u32; local_types.len()];
    let mut epoch = 0u32;
    let mut stack: Vec<Option<Expression>> = Vec::new();
    let mut seen: HashMap<Vec<u8>, usize> = HashMap::new();
    let mut scopes = Vec::new();
    let mut scope_budget = 32 * 1024 * 1024usize;
    let mut groups: Vec<Group> = Vec::new();
    let mut sizes = vec![0usize];
    let mut encoded = Vec::new();
    for (at, op) in ops.iter().enumerate() {
        encoded.clear();
        op.encode(&mut encoded);
        sizes.push(sizes[at] + encoded.len());
        if let Some((inputs, ty, reads_state)) = shape(
            op,
            stack.last().and_then(Option::as_ref),
            &local_types,
            types,
        ) {
            let mut args = if inputs <= stack.len() {
                stack.split_off(stack.len() - inputs)
            } else {
                stack.clear();
                Vec::new()
            };
            let mut contiguous = args.len() == inputs;
            let mut end = at;
            for arg in args.iter().rev() {
                if let Some(arg) = arg {
                    contiguous &= arg.end + 1 == end;
                    end = arg.start;
                } else {
                    contiguous = false;
                }
            }
            if !contiguous || at - end > 64 {
                stack.push(None);
                continue;
            }
            let mut key = Vec::new();
            key.extend_from_slice(&(encoded.len() as u32).to_le_bytes());
            key.extend_from_slice(&encoded);
            for arg in args.drain(..).flatten() {
                key.extend_from_slice(&(arg.key.len() as u32).to_le_bytes());
                key.extend(arg.key);
            }
            if let I::LocalGet(local) = op {
                key.extend_from_slice(&versions[*local as usize].to_le_bytes());
            }
            if reads_state {
                key.extend_from_slice(&epoch.to_le_bytes());
            }
            if end < at && sizes[at + 1] - sizes[end] > 2 {
                let group = *seen.entry(key.clone()).or_insert_with(|| {
                    let index = groups.len();
                    groups.push(Group {
                        uses: Vec::new(),
                        ty,
                        bytes: sizes[at + 1] - sizes[end],
                    });
                    index
                });
                groups[group].uses.push((end, at));
                if seen.len() > 1024 {
                    seen.clear();
                }
            }
            stack.push(Some(Expression {
                start: end,
                end: at,
                key,
                ty,
            }));
            continue;
        }
        // Only facts established before a conditional/block dominate its
        // continuation and both arms. Loop backedges require fresh facts.
        match op {
            I::Block(_) | I::If(_) => {
                let cost = seen.keys().map(|key| key.len() + 32).sum::<usize>();
                if cost > scope_budget {
                    return;
                }
                scope_budget -= cost;
                scopes.push(seen.clone());
                stack.clear();
                continue;
            }
            I::Loop(_) => {
                scopes.push(HashMap::new());
                seen.clear();
                stack.clear();
                continue;
            }
            I::Else => {
                let cost = scopes.last().map_or(0, |facts| {
                    facts.keys().map(|key| key.len() + 32).sum::<usize>()
                });
                if cost > scope_budget {
                    return;
                }
                scope_budget -= cost;
                seen = scopes.last().cloned().unwrap_or_default();
                stack.clear();
                continue;
            }
            I::End => {
                seen = scopes.pop().unwrap_or_default();
                stack.clear();
                continue;
            }
            I::BrIf(_) => {
                stack.pop();
                continue;
            }
            _ => {}
        }
        if let I::LocalSet(local) | I::LocalTee(local) = op {
            versions[*local as usize] += 1;
        } else if !matches!(op, I::Nop | I::Drop) {
            epoch += 1;
        }
        if let Some((inputs, outputs)) = super::stack_locals::arity(op, types, fields) {
            stack.truncate(stack.len().saturating_sub(inputs));
            stack.extend((0..outputs).map(|_| None));
        } else {
            stack.clear();
            seen.clear();
        }
    }
    groups.sort_by_key(|g| {
        (
            std::cmp::Reverse(g.uses.len().saturating_sub(1) * (g.bytes - 2)),
            g.uses[0].0,
        )
    });
    let mut occupied = vec![false; ops.len()];
    let mut removed = vec![false; ops.len()];
    let mut replacements = vec![None; ops.len()];
    let mut tees = vec![None; ops.len()];
    let mut slots: Vec<(ValType, u32, usize)> = Vec::new();
    for group in groups {
        // The original occurrence dominates every reuse. A retained larger
        // expression may contain that origin, but a replaced expression cannot.
        // Never substitute a later origin from just one conditional arm.
        let (first, last) = group.uses[0];
        if removed[first..=last].iter().any(|&value| value) {
            continue;
        }
        let uses: Vec<_> = group
            .uses
            .into_iter()
            .filter(|&(start, end)| start == first || !occupied[start..=end].iter().any(|&v| v))
            .collect();
        if uses.len() < 2 {
            continue;
        }
        let (start, first_end) = uses[0];
        let existing = slots
            .iter()
            .position(|&(ty, _, end)| ty == group.ty && end < start);
        let local = existing.map_or((params.len() + locals.len()) as u32, |s| slots[s].1);
        let mut temp = Vec::new();
        I::LocalGet(local).encode(&mut temp);
        let mut declaration = Vec::new();
        group.ty.encode(&mut declaration);
        let count = locals.iter().filter(|&&ty| ty == group.ty).count();
        let declaration_cost = if existing.is_some() {
            0
        } else if count > 0 {
            leb_size(count + 1) - leb_size(count)
        } else {
            declaration.len() + 2
        };
        let overhead = temp.len() + declaration_cost;
        if (uses.len() - 1) * group.bytes <= (uses.len() - 1) * temp.len() + overhead {
            continue;
        }
        let last = uses.last().unwrap().1;
        if let Some(slot) = existing {
            slots[slot].2 = last;
        } else {
            locals.push(group.ty);
            slots.push((group.ty, local, last));
        }
        tees[first_end] = Some(local);
        for (i, &(start, end)) in uses.iter().enumerate() {
            occupied[start..=end].fill(true);
            if i > 0 {
                replacements[start] = Some((end, local));
                removed[start..=end].fill(true);
            }
        }
    }
    if tees.iter().all(Option::is_none) {
        return;
    }
    let original = std::mem::take(ops);
    let mut at = 0;
    while at < original.len() {
        if let Some((end, local)) = replacements[at] {
            ops.push(I::LocalGet(local));
            at = end + 1;
        } else {
            ops.push(original[at].clone());
            if let Some(local) = tees[at] {
                ops.push(I::LocalTee(local));
            }
            at += 1;
        }
    }
}

fn leb_size(mut n: usize) -> usize {
    let mut bytes = 1;
    while n >= 128 {
        bytes += 1;
        n >>= 7;
    }
    bytes
}

fn shape(
    op: &I<'_>,
    top: Option<&Expression>,
    locals: &[ValType],
    types: &Types,
) -> Option<(usize, ValType, bool)> {
    use ValType::*;
    Some(match op {
        I::LocalGet(local) => (0, locals[*local as usize], false),
        I::GlobalGet(global) => {
            let global = types.globals.get(*global as usize)?;
            (
                0,
                RoundtripReencoder.val_type(global.content_type).ok()?,
                global.mutable,
            )
        }
        I::I32Const(_) => (0, I32, false),
        I::I64Const(_) => (0, I64, false),
        I::F32Const(_) => (0, F32, false),
        I::F64Const(_) => (0, F64, false),
        I::I32Add
        | I::I32Sub
        | I::I32Mul
        | I::I32And
        | I::I32Or
        | I::I32Xor
        | I::I32DivU
        | I::I32DivS
        | I::I32RemU
        | I::I32RemS
        | I::I32Shl
        | I::I32ShrU
        | I::I32ShrS
        | I::I32Rotl
        | I::I32Rotr
        | I::I32Eq
        | I::I32Ne
        | I::I32LtS
        | I::I32LtU
        | I::I32LeS
        | I::I32LeU
        | I::I32GtS
        | I::I32GtU
        | I::I32GeS
        | I::I32GeU
        | I::I64Eq
        | I::I64Ne
        | I::I64LtS
        | I::I64LtU
        | I::I64LeS
        | I::I64LeU
        | I::I64GtS
        | I::I64GtU
        | I::I64GeS
        | I::I64GeU
        | I::RefEq => (2, I32, false),
        I::I64Add
        | I::I64Sub
        | I::I64Mul
        | I::I64And
        | I::I64Or
        | I::I64Xor
        | I::I64DivU
        | I::I64DivS
        | I::I64RemU
        | I::I64RemS
        | I::I64Shl
        | I::I64ShrU
        | I::I64ShrS
        | I::I64Rotl
        | I::I64Rotr => (2, I64, false),
        I::I32Eqz
        | I::I64Eqz
        | I::I32WrapI64
        | I::I32Clz
        | I::I32Ctz
        | I::I32Popcnt
        | I::I32Extend8S
        | I::I32Extend16S
        | I::RefIsNull => (1, I32, false),
        I::I64ExtendI32S
        | I::I64ExtendI32U
        | I::I64Clz
        | I::I64Ctz
        | I::I64Popcnt
        | I::I64Extend8S
        | I::I64Extend16S
        | I::I64Extend32S => (1, I64, false),
        I::ArrayLen => (1, I32, false),
        I::RefCastNonNull(heap_type) => (
            1,
            Ref(wasm_encoder::RefType {
                nullable: false,
                heap_type: *heap_type,
            }),
            false,
        ),
        I::RefCastNullable(heap_type) => (
            1,
            Ref(wasm_encoder::RefType {
                nullable: true,
                heap_type: *heap_type,
            }),
            false,
        ),
        I::RefTestNonNull(_) | I::RefTestNullable(_) => (1, I32, false),
        I::StructGet {
            struct_type_index,
            field_index,
        }
        | I::StructGetS {
            struct_type_index,
            field_index,
        }
        | I::StructGetU {
            struct_type_index,
            field_index,
        } => {
            let ty = &types.types.get(*struct_type_index as usize)?.composite_type;
            if ty.shared {
                return None;
            }
            let wasmparser::CompositeInnerType::Struct(ty) = &ty.inner else {
                return None;
            };
            let field = ty.fields.get(*field_index as usize)?;
            let value = match field.element_type {
                wasmparser::StorageType::Val(v) => RoundtripReencoder.val_type(v).ok()?,
                _ => I32,
            };
            (1, value, field.mutable)
        }
        I::I32Load(_) | I::I32Load8U(_) | I::I32Load8S(_) | I::I32Load16U(_) | I::I32Load16S(_) => {
            (1, I32, true)
        }
        I::I64Load(_)
        | I::I64Load8U(_)
        | I::I64Load8S(_)
        | I::I64Load16U(_)
        | I::I64Load16S(_)
        | I::I64Load32U(_)
        | I::I64Load32S(_) => (1, I64, true),
        I::F32Load(_) => (1, F32, true),
        I::F64Load(_) => (1, F64, true),
        I::RefAsNonNull => {
            let Ref(mut r) = top?.ty else { return None };
            r.nullable = false;
            (1, Ref(r), false)
        }
        _ => return None,
    })
}
