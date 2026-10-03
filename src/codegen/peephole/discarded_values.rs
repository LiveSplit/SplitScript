//! Discard unobserved, nontrapping calculations while retaining every effect
//! and possible trap in their operands, in its original evaluation order.
use super::{I, fallthrough::Types};

struct Value {
    at: usize,
    pure: bool,
    inputs: Vec<Option<usize>>,
}

pub(super) fn run(ops: &mut Vec<I<'_>>, types: &Types, fields: &[Option<usize>]) {
    let mut values: Vec<Value> = Vec::new();
    let mut stack = Vec::new();
    let mut remove = vec![false; ops.len()];
    let mut drop_after = vec![false; ops.len()];
    for (at, op) in ops.iter().enumerate() {
        if matches!(op, I::Drop) {
            if let Some(Some(value)) = stack.pop() {
                remove[at] = true;
                let mut pending = vec![value];
                while let Some(value) = pending.pop() {
                    let value: &Value = &values[value];
                    if value.pure && value.inputs.iter().all(Option::is_some) {
                        remove[value.at] = true;
                        pending.extend(value.inputs.iter().flatten().copied());
                    } else {
                        drop_after[value.at] = true;
                    }
                }
            }
            continue;
        }
        let pure = inputs(op);
        let arity = pure
            .map(|n| (n, 1))
            .or_else(|| super::stack_locals::arity(op, types, fields));
        let Some((input_count, output_count)) = arity else {
            stack.clear();
            continue;
        };
        let arguments = if pure.is_none() {
            stack.truncate(stack.len().saturating_sub(input_count));
            Vec::new()
        } else if input_count <= stack.len() {
            stack.split_off(stack.len() - input_count)
        } else {
            stack.clear();
            vec![None; input_count]
        };
        if output_count == 1 {
            stack.push(Some(values.len()));
            values.push(Value {
                at,
                pure: pure.is_some(),
                inputs: arguments,
            });
        } else {
            stack.extend((0..output_count).map(|_| None));
        }
    }
    if !remove.iter().any(|&v| v) {
        return;
    }
    let original = std::mem::take(ops);
    for (at, op) in original.into_iter().enumerate() {
        if !remove[at] {
            ops.push(op);
        }
        if drop_after[at] {
            ops.push(I::Drop);
        }
    }
}

// Floating-point operations do not trap in Wasm. Their discarded result cannot
// expose NaN payload choices or signed zero; operands still execute normally.
fn inputs(op: &I<'_>) -> Option<usize> {
    Some(match op {
        op if super::pure_push(op) => 0,
        I::GlobalGet(_) | I::MemorySize(_) => 0,
        I::I32Eqz
        | I::I64Eqz
        | I::I32Clz
        | I::I32Ctz
        | I::I32Popcnt
        | I::I64Clz
        | I::I64Ctz
        | I::I64Popcnt
        | I::I32WrapI64
        | I::I64ExtendI32S
        | I::I64ExtendI32U
        | I::I32Extend8S
        | I::I32Extend16S
        | I::I64Extend8S
        | I::I64Extend16S
        | I::I64Extend32S
        | I::F32Abs
        | I::F32Neg
        | I::F32Ceil
        | I::F32Floor
        | I::F32Trunc
        | I::F32Nearest
        | I::F32Sqrt
        | I::F64Abs
        | I::F64Neg
        | I::F64Ceil
        | I::F64Floor
        | I::F64Trunc
        | I::F64Nearest
        | I::F64Sqrt
        | I::F32ConvertI32S
        | I::F32ConvertI32U
        | I::F32ConvertI64S
        | I::F32ConvertI64U
        | I::F64ConvertI32S
        | I::F64ConvertI32U
        | I::F64ConvertI64S
        | I::F64ConvertI64U
        | I::F32DemoteF64
        | I::F64PromoteF32
        | I::I32ReinterpretF32
        | I::I64ReinterpretF64
        | I::F32ReinterpretI32
        | I::F64ReinterpretI64
        | I::I32TruncSatF32S
        | I::I32TruncSatF32U
        | I::I32TruncSatF64S
        | I::I32TruncSatF64U
        | I::I64TruncSatF32S
        | I::I64TruncSatF32U
        | I::I64TruncSatF64S
        | I::I64TruncSatF64U
        | I::RefIsNull
        | I::RefTestNonNull(_)
        | I::RefTestNullable(_) => 1,
        I::I32Add
        | I::I32Sub
        | I::I32Mul
        | I::I32And
        | I::I32Or
        | I::I32Xor
        | I::I32Shl
        | I::I32ShrS
        | I::I32ShrU
        | I::I32Rotl
        | I::I32Rotr
        | I::I64Add
        | I::I64Sub
        | I::I64Mul
        | I::I64And
        | I::I64Or
        | I::I64Xor
        | I::I64Shl
        | I::I64ShrS
        | I::I64ShrU
        | I::I64Rotl
        | I::I64Rotr
        | I::I32Eq
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
        | I::F32Add
        | I::F32Sub
        | I::F32Mul
        | I::F32Div
        | I::F32Min
        | I::F32Max
        | I::F32Copysign
        | I::F64Add
        | I::F64Sub
        | I::F64Mul
        | I::F64Div
        | I::F64Min
        | I::F64Max
        | I::F64Copysign
        | I::F32Eq
        | I::F32Ne
        | I::F32Lt
        | I::F32Gt
        | I::F32Le
        | I::F32Ge
        | I::F64Eq
        | I::F64Ne
        | I::F64Lt
        | I::F64Gt
        | I::F64Le
        | I::F64Ge
        | I::RefEq => 2,
        I::Select | I::TypedSelect(_) => 3,
        _ => return None,
    })
}
