//! Replace private, single-allocation struct locals with individual field locals.
//! Every use must be a field access dominated by the allocation in structured
//! control flow. No aliases, identity tests or escaping references are accepted.
use super::{I, ValType, fallthrough::Types};
use wasm_encoder::reencode::{Reencode, RoundtripReencoder};

pub(super) fn run(
    ops: &mut Vec<I<'_>>,
    params: usize,
    locals: &mut Vec<ValType>,
    types: &Types,
    counts: &[Option<usize>],
) {
    let count = params + locals.len();
    let mut writes = vec![0; count];
    let mut candidates = Vec::new();
    for op in ops.iter_mut() {
        if super::control::map_labels(op, |d| d).is_err() {
            return;
        }
        match op {
            I::LocalSet(l) | I::LocalTee(l) => writes[*l as usize] += 1,
            _ => {}
        }
    }
    for at in 1..ops.len() {
        let (I::StructNew(ty) | I::StructNewDefault(ty), I::LocalSet(local) | I::LocalTee(local)) =
            (&ops[at - 1], &ops[at])
        else {
            continue;
        };
        if (*local as usize) < params || writes[*local as usize] != 1 {
            continue;
        }
        let ty = *ty;
        let Some(ty_info) = types.types.get(ty as usize) else {
            continue;
        };
        if ty_info.composite_type.shared {
            continue;
        }
        let wasmparser::CompositeInnerType::Struct(structure) = &ty_info.composite_type.inner
        else {
            continue;
        };
        let Some(fields) = structure
            .fields
            .iter()
            .map(|f| {
                let wasmparser::StorageType::Val(v) = f.element_type else {
                    return None;
                };
                let v = RoundtripReencoder.val_type(v).unwrap();
                if matches!(v,ValType::Ref(r) if !r.nullable) {
                    return None;
                }
                Some(v)
            })
            .collect::<Option<Vec<_>>>()
        else {
            continue;
        };
        candidates.push((at, *local, ty, fields));
    }
    let mut replacements: Vec<Option<Vec<I<'_>>>> = vec![None; ops.len()];
    for (allocation, local, ty, fields) in candidates {
        let mut scope = Vec::new();
        let mut allocation_scope = None;
        let mut accesses = Vec::new();
        let mut valid = true;
        // Every read must follow the allocation in the same scope or one of
        // its descendants. Leaving a block/loop or crossing an else arm loses
        // that proof: a branch could have skipped the allocation entirely.
        for (at, op) in ops.iter().enumerate() {
            if at == allocation {
                allocation_scope = Some(scope.clone());
            }
            let producer = matches!(op,I::LocalGet(l) if *l==local)
                || (at == allocation && matches!(op, I::LocalTee(_)));
            if producer {
                if allocation_scope
                    .as_ref()
                    .is_none_or(|parent| !scope.starts_with(parent))
                {
                    valid = false;
                    break;
                }
                let mut end = at + 1;
                let nonnull = matches!(ops.get(end), Some(I::RefAsNonNull));
                if nonnull {
                    end += 1;
                }
                let mut height = 0;
                let mut access = None;
                while end < ops.len() && end - at <= 512 {
                    let Some((inputs, outputs)) =
                        super::stack_locals::arity(&ops[end], types, counts)
                    else {
                        break;
                    };
                    if inputs > height {
                        access = match ops[end] {
                            I::StructGet {
                                struct_type_index,
                                field_index,
                            } if height == 0 && struct_type_index == ty => {
                                Some((end, field_index, false))
                            }
                            I::StructSet {
                                struct_type_index,
                                field_index,
                            } if height == 1 && struct_type_index == ty => {
                                Some((end, field_index, true))
                            }
                            _ => None,
                        };
                        break;
                    }
                    height = height - inputs + outputs;
                    end += 1;
                }
                let Some((end, field, store)) = access else {
                    valid = false;
                    break;
                };
                accesses.push((at, nonnull, end, field, store));
            }
            match op {
                I::Block(_) | I::Loop(_) | I::If(_) => scope.push(at),
                I::Else => *scope.last_mut().unwrap() = at,
                I::End => {
                    scope.pop();
                }
                _ => {}
            }
        }
        if !valid || accesses.is_empty() {
            continue;
        }
        let base = (params + locals.len()) as u32;
        locals.extend_from_slice(&fields);
        let mut initialize = Vec::new();
        for (field, &value) in fields.iter().enumerate().rev() {
            if matches!(ops[allocation - 1], I::StructNewDefault(_)) {
                initialize.push(match value {
                    ValType::I32 => I::I32Const(0),
                    ValType::I64 => I::I64Const(0),
                    ValType::F32 => I::F32Const(0.0.into()),
                    ValType::F64 => I::F64Const(0.0.into()),
                    ValType::V128 => I::V128Const(0),
                    ValType::Ref(r) => I::RefNull(r.heap_type),
                });
            }
            initialize.push(I::LocalSet(base + field as u32));
        }
        replacements[allocation - 1] = Some(initialize);
        replacements[allocation] = Some(Vec::new());
        for (at, nonnull, end, field, store) in accesses {
            replacements[at] = Some(Vec::new());
            if nonnull {
                replacements[at + 1] = Some(Vec::new());
            }
            replacements[end] = Some(vec![if store {
                I::LocalSet(base + field)
            } else {
                I::LocalGet(base + field)
            }]);
        }
    }
    // An immediately projected allocation does not need an object either.
    // Consume every field in reverse stack order, preserving all producers
    // (including their side effects and traps), and retain only the projection.
    for at in 0..ops.len().saturating_sub(1) {
        let I::StructNew(ty) = ops[at] else { continue };
        let I::StructGet {
            struct_type_index,
            field_index,
        } = ops[at + 1]
        else {
            continue;
        };
        if ty != struct_type_index {
            continue;
        }
        let info = &types.types[ty as usize];
        if info.composite_type.shared {
            continue;
        }
        let wasmparser::CompositeInnerType::Struct(structure) = &info.composite_type.inner else {
            continue;
        };
        let wasmparser::StorageType::Val(value) =
            structure.fields[field_index as usize].element_type
        else {
            continue;
        };
        let value = RoundtripReencoder.val_type(value).unwrap();
        if matches!(value,ValType::Ref(r) if !r.nullable) {
            continue;
        }
        let local = (params + locals.len()) as u32;
        locals.push(value);
        replacements[at] = Some(
            (0..structure.fields.len())
                .rev()
                .map(|field| {
                    if field == field_index as usize {
                        I::LocalSet(local)
                    } else {
                        I::Drop
                    }
                })
                .collect(),
        );
        replacements[at + 1] = Some(vec![I::LocalGet(local)]);
    }
    if replacements.iter().all(Option::is_none) {
        return;
    }
    let original = std::mem::take(ops);
    for (at, op) in original.into_iter().enumerate() {
        if let Some(replacement) = replacements[at].take() {
            ops.extend(replacement);
        } else {
            ops.push(op);
        }
    }
}
