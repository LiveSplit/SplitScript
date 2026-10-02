//! Strictly size-gated Release Wasm cleanup. No floating-point folding.
use std::convert::Infallible;
use wasm_encoder::{
    CodeSection, Encode, Function, Instruction as I, Module, ValType,
    reencode::{self, Reencode},
};
use wasmparser::{CompositeInnerType, Parser, Payload};

mod branch_tails;
mod branch_values;
#[cfg(test)]
mod cleanup_tests;
mod control;
mod expression_reuse;
#[cfg(test)]
mod expression_tests;
mod fallthrough;
#[cfg(test)]
mod instruction_tests;
mod liveness;
mod local_layout;
mod propagation;
#[cfg(test)]
mod propagation_tests;
mod returns;
#[cfg(test)]
mod returns_tests;
mod scalar_structs;
mod stack_locals;
#[cfg(test)]
mod tests;

#[derive(Clone, Copy, Default)]
pub(super) struct Passes {
    pub instructions: bool,
    pub constants: bool,
    pub dead_code: bool,
    pub locals: bool,
    pub control: bool,
    pub returns: bool,
    pub propagation: bool,
    pub flow_locals: bool,
    pub expressions: bool,
}

pub(super) fn optimize(wasm: &[u8], passes: Passes) -> Vec<u8> {
    let mut module = Module::new();
    Cleanup::new(wasm, passes)
        .parse_core_module(&mut module, Parser::new(0), wasm)
        .unwrap();
    let result = module.finish();
    if result.len() < wasm.len() {
        result
    } else {
        wasm.to_vec()
    }
}

struct Cleanup {
    passes: Passes,
    parameters: Vec<usize>,
    next_body: usize,
    results: Vec<usize>,
    struct_fields: Vec<Option<usize>>,
    arities: fallthrough::Types,
}

impl Reencode for Cleanup {
    type Error = Infallible;
    fn parse_function_body(
        &mut self,
        code: &mut CodeSection,
        body: wasmparser::FunctionBody<'_>,
    ) -> Result<(), reencode::Error<Infallible>> {
        code.raw(&self.clean_body(body)?);
        Ok(())
    }
}

impl Cleanup {
    fn new(wasm: &[u8], passes: Passes) -> Self {
        let mut type_parameters = Vec::new();
        let mut parameters = Vec::new();
        let mut type_results = Vec::new();
        let mut results = Vec::new();
        let mut struct_fields = Vec::new();
        let mut arities = fallthrough::Types::default();
        for payload in Parser::new(0).parse_all(wasm) {
            match payload.unwrap() {
                Payload::TypeSection(section) => {
                    for group in section {
                        for ty in group.unwrap().into_types() {
                            if passes.returns {
                                type_results.push(match &ty.composite_type.inner {
                                    CompositeInnerType::Func(ty) => ty.results().len(),
                                    _ => 0,
                                });
                            }
                            if passes.returns || passes.instructions || passes.expressions {
                                struct_fields.push(match &ty.composite_type.inner {
                                    CompositeInnerType::Struct(ty) => Some(ty.fields.len()),
                                    _ => None,
                                });
                            }
                            type_parameters.push(match &ty.composite_type.inner {
                                CompositeInnerType::Func(ty) => ty.params().len(),
                                _ => 0,
                            });
                            arities.types.push(ty);
                        }
                    }
                }
                Payload::ImportSection(section) => {
                    for import in section.into_imports() {
                        match import.unwrap().ty {
                            wasmparser::TypeRef::Func(ty) | wasmparser::TypeRef::FuncExact(ty) => {
                                arities.functions.push(ty);
                                arities.imported += 1;
                            }
                            wasmparser::TypeRef::Global(ty) => {
                                arities.shared |= ty.shared;
                                arities.globals.push(ty);
                            }
                            wasmparser::TypeRef::Memory(ty) => arities.shared |= ty.shared,
                            _ => {}
                        }
                    }
                }
                Payload::FunctionSection(section) => {
                    for ty in section {
                        let ty = ty.unwrap() as usize;
                        arities.functions.push(ty as u32);
                        parameters.push(type_parameters[ty]);
                        if passes.returns {
                            results.push(type_results[ty]);
                        }
                    }
                }
                Payload::GlobalSection(section) => {
                    for global in section {
                        let ty = global.unwrap().ty;
                        arities.shared |= ty.shared;
                        arities.globals.push(ty);
                    }
                }
                Payload::MemorySection(section) => {
                    for memory in section {
                        arities.shared |= memory.unwrap().shared;
                    }
                }
                _ => {}
            }
        }
        Self {
            passes,
            parameters,
            next_body: 0,
            results,
            struct_fields,
            arities,
        }
    }
    fn clean_body(
        &mut self,
        body: wasmparser::FunctionBody<'_>,
    ) -> Result<Vec<u8>, reencode::Error<Infallible>> {
        let mut function = self.new_function_with_parsed_locals(&body)?;
        let parameter_count = self.parameters[self.next_body];
        self.next_body += 1;
        let mut ops = Vec::new();
        let mut dead = None;
        let fallthroughs = if self.passes.control {
            fallthrough::find(&body, &self.arities, self.next_body - 1)
        } else {
            Vec::new()
        };
        let mut fallthroughs = fallthroughs.into_iter().peekable();
        for (at, op) in body.get_operators_reader()?.into_iter().enumerate() {
            let op = op?;
            let op = if fallthroughs
                .peek()
                .is_some_and(|&(position, _)| position == at)
            {
                if fallthroughs.next().unwrap().1 {
                    wasmparser::Operator::Drop
                } else {
                    continue;
                }
            } else {
                op
            };
            // These constructs introduce control boundaries not modeled by
            // the structured dead-code scan. The compiler does not currently
            // emit them, but preserve a future such body verbatim.
            if matches!(
                op,
                wasmparser::Operator::Try { .. }
                    | wasmparser::Operator::TryTable { .. }
                    | wasmparser::Operator::Delegate { .. }
            ) {
                return Ok(body.as_bytes().to_vec());
            }
            let op = self.instruction(op)?;
            if self.passes.dead_code {
                if let Some(nesting) = &mut dead {
                    match op {
                        I::Block(_) | I::Loop(_) | I::If(_) => {
                            *nesting += 1;
                            continue;
                        }
                        I::End if *nesting != 0 => {
                            *nesting -= 1;
                            continue;
                        }
                        I::End | I::Else if *nesting == 0 => dead = None,
                        _ => continue,
                    }
                } else if matches!(op, I::Return | I::Br(_) | I::BrTable(..) | I::Unreachable) {
                    dead = Some(0);
                }
            }
            ops.push(op);
            loop {
                let old_len = ops.len();
                if self.passes.constants {
                    fold(&mut ops);
                }
                if self.passes.instructions {
                    simplify(&mut ops);
                    default_structs(&mut ops, &self.struct_fields);
                }
                if ops.len() == old_len {
                    break;
                }
            }
        }
        let mut locals = Vec::new();
        if self.passes.propagation {
            propagation::fold_locals(&mut ops);
        }
        if self.passes.locals || self.passes.control || self.passes.returns {
            for local in body.get_locals_reader()? {
                let (count, ty) = local?;
                locals.extend(std::iter::repeat_n(self.val_type(ty)?, count as usize));
            }
        }
        // Non-defaultable locals have structured definite-initialization
        // rules. A new merge point may lose that proof even when all incoming
        // paths initialize the local. Keep such bodies out of return folding.
        if self.passes.returns
            && !locals
                .iter()
                .any(|ty| matches!(ty, ValType::Ref(ty) if !ty.nullable))
        {
            returns::fold(
                &mut ops,
                self.results[self.next_body - 1],
                &self.struct_fields,
            );
        }
        if self.passes.control {
            if self.passes.flow_locals && self.passes.locals {
                scalar_structs::run(
                    &mut ops,
                    parameter_count,
                    &mut locals,
                    &self.arities,
                    &self.struct_fields,
                );
            }
            // Hoisting a tail introduces a merge point; non-defaultable local
            // initialization inside an arm does not survive that merge.
            if !locals
                .iter()
                .any(|ty| matches!(ty, ValType::Ref(ty) if !ty.nullable))
            {
                branch_tails::fold(&mut ops, &self.arities, &self.struct_fields);
            }
            control::remove_unused_labels(&mut ops);
            control::merge_if_assignments(&mut ops, parameter_count, &locals);
            branch_values::fold(&mut ops, &self.arities, &self.struct_fields);
        }
        if self.passes.locals {
            if self.passes.flow_locals {
                stack_locals::run(
                    &mut ops,
                    parameter_count + locals.len(),
                    &self.arities,
                    &self.struct_fields,
                );
                let signature = self.arities.functions[self.arities.imported + self.next_body - 1];
                let CompositeInnerType::Func(ty) =
                    &self.arities.types[signature as usize].composite_type.inner
                else {
                    unreachable!()
                };
                let params = ty
                    .params()
                    .iter()
                    .map(|&ty| reencode::RoundtripReencoder.val_type(ty))
                    .collect::<Result<Vec<_>, _>>()?;
                liveness::reuse(
                    &mut ops,
                    &params,
                    &locals,
                    &self.arities,
                    &self.struct_fields,
                );
            }
            if self.passes.expressions && !self.arities.shared {
                let signature = self.arities.functions[self.arities.imported + self.next_body - 1];
                let CompositeInnerType::Func(ty) =
                    &self.arities.types[signature as usize].composite_type.inner
                else {
                    unreachable!()
                };
                let params = ty
                    .params()
                    .iter()
                    .map(|&ty| reencode::RoundtripReencoder.val_type(ty))
                    .collect::<Result<Vec<_>, _>>()?;
                expression_reuse::run(
                    &mut ops,
                    &params,
                    &mut locals,
                    &self.arities,
                    &self.struct_fields,
                );
            }
            let locals = compact_locals(&mut ops, parameter_count, locals);
            if self.passes.instructions {
                let mut simplified = Vec::with_capacity(ops.len());
                for op in ops {
                    simplified.push(op);
                    loop {
                        let before = simplified.len();
                        simplify(&mut simplified);
                        if before == simplified.len() {
                            break;
                        }
                    }
                }
                ops = simplified;
            }
            let locals = local_layout::reorder(&mut ops, parameter_count, locals);
            function = Function::new_with_locals_types(locals);
        }
        for op in &ops {
            function.instruction(op);
        }
        // Do not grow a body even if another body offers unrelated savings.
        Ok(if function.byte_len() < body.as_bytes().len() {
            function.into_raw_body()
        } else {
            body.as_bytes().to_vec()
        })
    }
}

/// Locals are private to this activation. An unread store can be dropped, but
/// its producer must still execute (and may trap). Parameter indices and the
/// function signature stay fixed. This is not liveness-based local reuse.
fn compact_locals(ops: &mut Vec<I<'_>>, parameters: usize, locals: Vec<ValType>) -> Vec<ValType> {
    let mut read = vec![false; parameters + locals.len()];
    for op in ops.iter() {
        if let I::LocalGet(index) = op {
            read[*index as usize] = true;
        }
    }
    ops.retain_mut(|op| match op {
        I::LocalSet(index) if !read[*index as usize] => {
            *op = I::Drop;
            true
        }
        I::LocalTee(index) if !read[*index as usize] => false,
        _ => true,
    });
    let mut indices = vec![0; read.len()];
    for (index, mapped) in indices.iter_mut().take(parameters).enumerate() {
        *mapped = index as u32;
    }
    let mut kept = Vec::new();
    for (index, ty) in locals.into_iter().enumerate() {
        if read[parameters + index] {
            indices[parameters + index] = (parameters + kept.len()) as u32;
            kept.push(ty);
        }
    }
    for op in ops {
        if let I::LocalGet(index) | I::LocalSet(index) | I::LocalTee(index) = op {
            *index = indices[*index as usize];
        }
    }
    kept
}

fn encoded_size(ops: &[I<'_>]) -> usize {
    let mut bytes = Vec::new();
    for op in ops {
        op.encode(&mut bytes);
    }
    bytes.len()
}

/// Default construction keeps the allocation and type, omitting only literal
/// default operands. Float defaults must be positive zero, bit for bit.
fn default_structs(ops: &mut Vec<I<'_>>, fields: &[Option<usize>]) {
    let Some(I::StructNew(ty)) = ops.last() else {
        return;
    };
    let ty = *ty;
    let Some(count) = fields.get(ty as usize).copied().flatten() else {
        return;
    };
    if count == 0 || count >= ops.len() {
        return;
    }
    let start = ops.len() - count - 1;
    if ops[start..ops.len() - 1].iter().all(|op| match op {
        I::I32Const(0) | I::I64Const(0) | I::RefNull(_) => true,
        I::F32Const(value) => value.bits() == 0,
        I::F64Const(value) => value.bits() == 0,
        _ => false,
    }) {
        ops.truncate(start);
        ops.push(I::StructNewDefault(ty));
    }
}

fn pure_push(op: &I<'_>) -> bool {
    matches!(
        op,
        I::I32Const(_)
            | I::I64Const(_)
            | I::F32Const(_)
            | I::F64Const(_)
            | I::V128Const(_)
            | I::LocalGet(_)
            | I::RefNull(_)
            | I::RefFunc(_)
    )
}

fn simplify(ops: &mut Vec<I<'_>>) {
    if let [.., identity, value, operation] = ops.as_slice()
        && nontrapping_operand(value)
        && matches!(
            (identity, operation),
            (I::I32Const(0), I::I32Add | I::I32Or | I::I32Xor)
                | (I::I64Const(0), I::I64Add | I::I64Or | I::I64Xor)
                | (I::I32Const(1), I::I32Mul)
                | (I::I64Const(1), I::I64Mul)
                | (I::I32Const(-1), I::I32And)
                | (I::I64Const(-1), I::I64And)
        )
    {
        let value = value.clone();
        ops.truncate(ops.len() - 3);
        ops.push(value);
        return;
    }
    if matches!(ops.last(), Some(I::Nop)) {
        ops.pop();
        return;
    }
    // These GC consumers already trap on null. Only move that check across
    // individual nontrapping reads/constants, never calls, stores or traps.
    if let [.., I::RefAsNonNull, operand, consumer] = ops.as_slice()
        && nontrapping_operand(operand)
        && matches!(
            consumer,
            I::StructSet { .. } | I::ArrayGet(_) | I::ArrayGetS(_) | I::ArrayGetU(_)
        )
    {
        ops.remove(ops.len() - 3);
        return;
    }
    if let [.., I::RefAsNonNull, index, value, I::ArraySet(_)] = ops.as_slice()
        && nontrapping_operand(index)
        && nontrapping_operand(value)
    {
        ops.remove(ops.len() - 4);
        return;
    }
    // Boolean-producing instructions already return exactly zero or one.
    if let [.., producer, I::I32Eqz, I::I32Eqz] = ops.as_slice()
        && boolean_result(producer)
    {
        ops.truncate(ops.len() - 2);
        return;
    }
    // Conditions consume truthiness, so normalizing an arbitrary i32 twice
    // is redundant here even when its value is not already zero or one.
    if let [.., I::I32Eqz, I::I32Eqz, condition] = ops.as_slice()
        && matches!(
            condition,
            I::If(_) | I::BrIf(_) | I::Select | I::TypedSelect(_)
        )
    {
        let condition = condition.clone();
        ops.truncate(ops.len() - 3);
        ops.push(condition);
        return;
    }
    let replacement = match ops.as_slice() {
        [.., producer, I::RefAsNonNull]
            if matches!(
                producer,
                I::StructNew(_)
                    | I::StructNewDefault(_)
                    | I::ArrayNew(_)
                    | I::ArrayNewDefault(_)
                    | I::ArrayNewFixed { .. }
                    | I::RefFunc(_)
                    | I::RefCastNonNull(_)
                    | I::RefAsNonNull
            ) =>
        {
            Some(Some(producer.clone()))
        }
        [.., compare, I::I32Eqz] if invert_integer_comparison(compare).is_some() => {
            Some(invert_integer_comparison(compare))
        }
        [.., I::I64ExtendI32S | I::I64ExtendI32U, I::I32WrapI64] => Some(None),
        [.., I::I64ExtendI32S | I::I64ExtendI32U, I::I64Eqz] => Some(Some(I::I32Eqz)),
        [.., I::LocalSet(a), I::LocalGet(b)] if a == b => Some(Some(I::LocalTee(*a))),
        [.., I::LocalTee(a), I::Drop] => Some(Some(I::LocalSet(*a))),
        [.., I::LocalGet(a), I::LocalSet(b)] if a == b => Some(None),
        [.., producer, I::Drop] if nontrapping_operand(producer) => Some(None),
        [.., I::Else, I::End] => Some(Some(I::End)),
        [.., I::If(wasm_encoder::BlockType::Empty), I::End] => Some(Some(I::Drop)),
        [
            ..,
            I::I32Const(0),
            I::I32Add | I::I32Sub | I::I32Or | I::I32Xor | I::I32Shl | I::I32ShrS | I::I32ShrU,
        ] => Some(None),
        [
            ..,
            I::I64Const(0),
            I::I64Add | I::I64Sub | I::I64Or | I::I64Xor | I::I64Shl | I::I64ShrS | I::I64ShrU,
        ] => Some(None),
        [.., I::I32Const(1), I::I32Mul | I::I32DivS | I::I32DivU] => Some(None),
        [.., I::I64Const(1), I::I64Mul | I::I64DivS | I::I64DivU] => Some(None),
        [.., I::I32Const(-1), I::I32And] | [.., I::I64Const(-1), I::I64And] => Some(None),
        [.., I::I32Const(0), I::I32Eq] => Some(Some(I::I32Eqz)),
        [.., I::I64Const(0), I::I64Eq] => Some(Some(I::I64Eqz)),
        [
            ..,
            I::RefAsNonNull,
            next @ (I::StructGet { .. }
            | I::StructGetS { .. }
            | I::StructGetU { .. }
            | I::ArrayLen
            | I::RefCastNonNull(_)),
        ] => Some(Some(next.clone())),
        _ => None,
    };
    if let Some(replacement) = replacement {
        ops.truncate(ops.len() - 2);
        if let Some(op) = replacement {
            ops.push(op);
        }
    }
}

fn nontrapping_operand(op: &I<'_>) -> bool {
    pure_push(op) || matches!(op, I::GlobalGet(_))
}

fn boolean_result(op: &I<'_>) -> bool {
    matches!(
        op,
        I::I32Eqz
            | I::I64Eqz
            | I::RefIsNull
            | I::RefEq
            | I::RefTestNonNull(_)
            | I::RefTestNullable(_)
    ) || invert_integer_comparison(op).is_some()
}

// Floating-point inequalities are deliberately excluded: unordered NaNs make
// !(a < b) different from a >= b. Integer comparisons have no unordered case.
fn invert_integer_comparison(op: &I<'_>) -> Option<I<'static>> {
    Some(match op {
        I::I32Eq => I::I32Ne,
        I::I32Ne => I::I32Eq,
        I::I32LtS => I::I32GeS,
        I::I32GeS => I::I32LtS,
        I::I32LtU => I::I32GeU,
        I::I32GeU => I::I32LtU,
        I::I32LeS => I::I32GtS,
        I::I32GtS => I::I32LeS,
        I::I32LeU => I::I32GtU,
        I::I32GtU => I::I32LeU,
        I::I64Eq => I::I64Ne,
        I::I64Ne => I::I64Eq,
        I::I64LtS => I::I64GeS,
        I::I64GeS => I::I64LtS,
        I::I64LtU => I::I64GeU,
        I::I64GeU => I::I64LtU,
        I::I64LeS => I::I64GtS,
        I::I64GtS => I::I64LeS,
        I::I64LeU => I::I64GtU,
        I::I64GtU => I::I64LeU,
        _ => return None,
    })
}

fn fold(ops: &mut Vec<I<'_>>) {
    let unary = match ops.as_slice() {
        [.., I::I64Const(value), I::I32WrapI64] => Some(I::I32Const(*value as i32)),
        [.., I::I32Const(value), I::I64ExtendI32S] => Some(I::I64Const(i64::from(*value))),
        [.., I::I32Const(value), I::I64ExtendI32U] => Some(I::I64Const(i64::from(*value as u32))),
        [.., I::I32Const(value), I::I32Eqz] => Some(I::I32Const(i32::from(*value == 0))),
        [.., I::I64Const(value), I::I64Eqz] => Some(I::I32Const(i32::from(*value == 0))),
        _ => None,
    };
    if let Some(replacement) = unary
        && encoded_size(std::slice::from_ref(&replacement)) <= encoded_size(&ops[ops.len() - 2..])
    {
        ops.truncate(ops.len() - 2);
        ops.push(replacement);
        return;
    }
    let replacement = match ops.as_slice() {
        [.., I::I32Const(a), I::I32Const(b), op] => match op {
            I::I32Add => Some(I::I32Const(a.wrapping_add(*b))),
            I::I32Sub => Some(I::I32Const(a.wrapping_sub(*b))),
            I::I32Mul => Some(I::I32Const(a.wrapping_mul(*b))),
            I::I32And => Some(I::I32Const(a & b)),
            I::I32Or => Some(I::I32Const(a | b)),
            I::I32Xor => Some(I::I32Const(a ^ b)),
            I::I32Shl => Some(I::I32Const(a.wrapping_shl(*b as u32))),
            I::I32ShrS => Some(I::I32Const(a.wrapping_shr(*b as u32))),
            I::I32ShrU => Some(I::I32Const((*a as u32).wrapping_shr(*b as u32) as i32)),
            I::I32DivS => a.checked_div(*b).map(I::I32Const),
            I::I32DivU => (*a as u32)
                .checked_div(*b as u32)
                .map(|v| I::I32Const(v as i32)),
            I::I32Eq => Some(I::I32Const(i32::from(a == b))),
            I::I32Ne => Some(I::I32Const(i32::from(a != b))),
            I::I32LtS => Some(I::I32Const(i32::from(a < b))),
            I::I32LtU => Some(I::I32Const(i32::from((*a as u32) < (*b as u32)))),
            I::I32LeS => Some(I::I32Const(i32::from(a <= b))),
            I::I32LeU => Some(I::I32Const(i32::from((*a as u32) <= (*b as u32)))),
            I::I32GtS => Some(I::I32Const(i32::from(a > b))),
            I::I32GtU => Some(I::I32Const(i32::from((*a as u32) > (*b as u32)))),
            I::I32GeS => Some(I::I32Const(i32::from(a >= b))),
            I::I32GeU => Some(I::I32Const(i32::from((*a as u32) >= (*b as u32)))),
            _ => None,
        },
        [.., I::I64Const(a), I::I64Const(b), op] => match op {
            I::I64Add => Some(I::I64Const(a.wrapping_add(*b))),
            I::I64Sub => Some(I::I64Const(a.wrapping_sub(*b))),
            I::I64Mul => Some(I::I64Const(a.wrapping_mul(*b))),
            I::I64And => Some(I::I64Const(a & b)),
            I::I64Or => Some(I::I64Const(a | b)),
            I::I64Xor => Some(I::I64Const(a ^ b)),
            I::I64Shl => Some(I::I64Const(a.wrapping_shl(*b as u32))),
            I::I64ShrS => Some(I::I64Const(a.wrapping_shr(*b as u32))),
            I::I64ShrU => Some(I::I64Const((*a as u64).wrapping_shr(*b as u32) as i64)),
            I::I64DivS => a.checked_div(*b).map(I::I64Const),
            I::I64DivU => (*a as u64)
                .checked_div(*b as u64)
                .map(|v| I::I64Const(v as i64)),
            I::I64Eq => Some(I::I32Const(i32::from(a == b))),
            I::I64Ne => Some(I::I32Const(i32::from(a != b))),
            I::I64LtS => Some(I::I32Const(i32::from(a < b))),
            I::I64LtU => Some(I::I32Const(i32::from((*a as u64) < (*b as u64)))),
            I::I64LeS => Some(I::I32Const(i32::from(a <= b))),
            I::I64LeU => Some(I::I32Const(i32::from((*a as u64) <= (*b as u64)))),
            I::I64GtS => Some(I::I32Const(i32::from(a > b))),
            I::I64GtU => Some(I::I32Const(i32::from((*a as u64) > (*b as u64)))),
            I::I64GeS => Some(I::I32Const(i32::from(a >= b))),
            I::I64GeU => Some(I::I32Const(i32::from((*a as u64) >= (*b as u64)))),
            _ => None,
        },
        _ => None,
    };
    if let Some(replacement) = replacement
        && encoded_size(std::slice::from_ref(&replacement)) <= encoded_size(&ops[ops.len() - 3..])
    {
        ops.truncate(ops.len() - 3);
        ops.push(replacement);
    }
}

/// Reuse module signatures and stack arities while analyzing call arguments.
pub(super) struct CallOperands(Cleanup);
impl CallOperands {
    pub(super) fn new(wasm: &[u8]) -> Self {
        Self(Cleanup::new(
            wasm,
            Passes {
                instructions: true,
                ..Default::default()
            },
        ))
    }
    /// Locate inert arguments, walking across other complete expressions but
    /// never across control boundaries or a shared multi-result producer.
    pub(super) fn inert_arguments(
        &self,
        ops: &[I<'_>],
        at: usize,
        params: usize,
    ) -> Vec<Option<usize>> {
        let mut result = vec![None; params];
        let mut cursor = at;
        for param in (0..params).rev() {
            let end = cursor;
            let mut needed = 1;
            while cursor > at.saturating_sub(512) && needed != 0 {
                let op = &ops[cursor - 1];
                let arity = if pure_push(op) {
                    Some((0, 1))
                } else {
                    stack_locals::arity(op, &self.0.arities, &self.0.struct_fields)
                };
                let Some((inputs, outputs)) = arity else {
                    return result;
                };
                if outputs > needed {
                    return result;
                }
                needed = needed - outputs + inputs;
                cursor -= 1;
            }
            if needed != 0 {
                return result;
            }
            if cursor + 1 == end && pure_push(&ops[cursor]) {
                result[param] = Some(cursor);
            }
        }
        result
    }
}

/// Reuse module signatures across all candidate bodies in a single inlining run.
pub(super) struct BodyCleanup(Cleanup);
impl BodyCleanup {
    pub(super) fn new(wasm: &[u8]) -> Self {
        Self(Cleanup::new(
            wasm,
            Passes {
                instructions: true,
                constants: true,
                dead_code: true,
                locals: true,
                control: true,
                returns: true,
                propagation: true,
                flow_locals: true,
                expressions: false,
            },
        ))
    }
    pub(super) fn clean(&mut self, bytes: &[u8], defined: usize) -> Vec<u8> {
        let mut bytes = bytes.to_vec();
        for _ in 0..2 {
            self.0.next_body = defined;
            bytes = self
                .0
                .clean_body(wasmparser::FunctionBody::new(
                    wasmparser::BinaryReader::new(&bytes, 0),
                ))
                .unwrap();
        }
        bytes
    }
}
