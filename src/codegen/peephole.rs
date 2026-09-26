//! Strictly size-gated Release Wasm cleanup. No floating-point folding.
use std::convert::Infallible;
use wasm_encoder::{
    CodeSection, Encode, Function, Instruction as I, Module, ValType,
    reencode::{self, Reencode},
};
use wasmparser::{CompositeInnerType, Parser, Payload};

mod control;
mod returns;
#[cfg(test)]
mod returns_tests;
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
}

pub(super) fn optimize(wasm: &[u8], passes: Passes) -> Vec<u8> {
    let mut type_parameters = Vec::new();
    let mut parameters = Vec::new();
    let mut type_results = Vec::new();
    let mut results = Vec::new();
    let mut struct_fields = Vec::new();
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
                            struct_fields.push(match &ty.composite_type.inner {
                                CompositeInnerType::Struct(ty) => Some(ty.fields.len()),
                                _ => None,
                            });
                        }
                        type_parameters.push(match &ty.composite_type.inner {
                            CompositeInnerType::Func(ty) => ty.params().len(),
                            _ => 0,
                        });
                    }
                }
            }
            Payload::FunctionSection(section) => {
                for ty in section {
                    let ty = ty.unwrap() as usize;
                    parameters.push(type_parameters[ty]);
                    if passes.returns {
                        results.push(type_results[ty]);
                    }
                }
            }
            _ => {}
        }
    }
    let mut module = Module::new();
    Cleanup {
        passes,
        parameters,
        next_body: 0,
        results,
        struct_fields,
    }
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
    fn clean_body(
        &mut self,
        body: wasmparser::FunctionBody<'_>,
    ) -> Result<Vec<u8>, reencode::Error<Infallible>> {
        let mut function = self.new_function_with_parsed_locals(&body)?;
        let parameter_count = self.parameters[self.next_body];
        self.next_body += 1;
        let mut ops = Vec::new();
        let mut dead = None;
        for op in body.get_operators_reader()? {
            let op = op?;
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
                }
                if ops.len() == old_len {
                    break;
                }
            }
        }
        let mut locals = Vec::new();
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
            control::remove_unused_labels(&mut ops);
            control::merge_if_assignments(&mut ops, parameter_count, &locals);
        }
        if self.passes.locals {
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
    if matches!(ops.last(), Some(I::Nop)) {
        ops.pop();
        return;
    }
    let replacement = match ops.as_slice() {
        [.., I::LocalSet(a), I::LocalGet(b)] if a == b => Some(Some(I::LocalTee(*a))),
        [.., I::LocalTee(a), I::Drop] => Some(Some(I::LocalSet(*a))),
        [.., I::LocalGet(a), I::LocalSet(b)] if a == b => Some(None),
        [.., producer, I::Drop] if pure_push(producer) => Some(None),
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

fn fold(ops: &mut Vec<I<'_>>) {
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
