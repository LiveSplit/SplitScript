//! Release-only, size-driven inlining of single-use synchronous functions.
//!
//! Work on the emitted ABI so the cost includes real local/LEB encodings and
//! removal of the function declaration and body. Dependencies remain owned by
//! the normal backend plan: inlining does not erase any executed instruction.
//! The enclosing pipeline removes unused type groups and accepts the result
//! only when the complete file shrinks. Debug never enters this pipeline.

use std::{collections::BTreeSet, convert::Infallible, sync::Arc};

use wasm_encoder::{
    AbstractHeapType, BlockType, CodeSection, Function, FunctionSection, HeapType, Instruction,
    Module, ValType,
    reencode::{self, Reencode, RoundtripReencoder},
};
use wasmparser::{BinaryReader, CompositeInnerType, FunctionBody, Operator, Parser, Payload};

type Error = reencode::Error<Infallible>;

#[derive(Clone)]
struct Body {
    signature: u32,
    block_type: BlockType,
    params: Vec<ValType>,
    locals: Vec<ValType>,
    null_types: Arc<[HeapType]>,
    bytes: Vec<u8>,
    calls: Vec<u32>,
    supported: bool,
}

impl Body {
    fn reader(&self) -> FunctionBody<'_> {
        FunctionBody::new(BinaryReader::new(&self.bytes, 0))
    }

    fn encoded_size(&self) -> usize {
        self.bytes.len() + leb_size(self.bytes.len() as u32)
    }
}

fn leb_size(mut value: u32) -> usize {
    let mut size = 1;
    while value >= 128 {
        value >>= 7;
        size += 1;
    }
    size
}

/// Counts *all* references, including exports, elements, global initializers,
/// ref.func and tail calls. Only a sole ordinary call may be substituted.
struct References(Vec<u32>);

impl Reencode for References {
    type Error = Infallible;

    fn function_index(&mut self, index: u32) -> Result<u32, Error> {
        self.0[index as usize] += 1;
        Ok(index)
    }
}

fn read_bodies(wasm: &[u8]) -> Result<Vec<Option<Body>>, Error> {
    let mut types = Vec::new();
    let mut null_types: Arc<[HeapType]> = Arc::from([]);
    let mut signatures = Vec::new();
    let mut bodies = Vec::new();
    let mut encoder = RoundtripReencoder;
    for payload in Parser::new(0).parse_all(wasm) {
        match payload? {
            Payload::TypeSection(section) => {
                let mut nulls = Vec::new();
                for group in section {
                    for ty in group?.into_types() {
                        nulls.push(HeapType::Abstract {
                            shared: ty.composite_type.shared,
                            ty: if matches!(&ty.composite_type.inner, CompositeInnerType::Func(_)) {
                                AbstractHeapType::NoFunc
                            } else {
                                AbstractHeapType::None
                            },
                        });
                        types.push(match ty.composite_type.inner {
                            CompositeInnerType::Func(ty) => Some(ty),
                            _ => None,
                        });
                    }
                }
                null_types = nulls.into();
            }
            Payload::FunctionSection(section) => {
                signatures = section.into_iter().collect::<Result<Vec<_>, _>>()?;
            }
            Payload::CodeSectionEntry(body) => {
                let signature = signatures[bodies.len()];
                let ty = types[signature as usize].as_ref().unwrap();
                let params = ty
                    .params()
                    .iter()
                    .map(|ty| encoder.val_type(*ty))
                    .collect::<Result<Vec<_>, _>>()?;
                let results = ty
                    .results()
                    .iter()
                    .map(|ty| encoder.val_type(*ty))
                    .collect::<Result<Vec<_>, _>>()?;
                let mut locals = Vec::new();
                for local in body.get_locals_reader()? {
                    let (count, ty) = local?;
                    locals.extend(std::iter::repeat_n(encoder.val_type(ty)?, count as usize));
                }
                let mut calls = Vec::new();
                let mut supported = true;
                for op in body.get_operators_reader()? {
                    match op? {
                        Operator::Call { function_index } => calls.push(function_index),
                        // These are not currently emitted for synchronous source
                        // bodies. Keep their frame/control boundaries intact.
                        Operator::ReturnCall { .. }
                        | Operator::ReturnCallIndirect { .. }
                        | Operator::ReturnCallRef { .. }
                        | Operator::Try { .. }
                        | Operator::TryTable { .. }
                        | Operator::Delegate { .. }
                        | Operator::Resume { .. }
                        | Operator::ResumeThrow { .. }
                        | Operator::ResumeThrowRef { .. }
                        | Operator::Suspend { .. }
                        | Operator::Switch { .. } => supported = false,
                        _ => {}
                    }
                }
                bodies.push(Some(Body {
                    signature,
                    block_type: match results.as_slice() {
                        [] => BlockType::Empty,
                        [ty] => BlockType::Result(*ty),
                        _ => BlockType::FunctionType(
                            types
                                .iter()
                                .position(|t| {
                                    t.as_ref().is_some_and(|t| {
                                        t.params().is_empty() && t.results() == ty.results()
                                    })
                                })
                                .unwrap() as u32,
                        ),
                    },
                    params,
                    locals,
                    null_types: null_types.clone(),
                    bytes: wasm[body.range()].to_vec(),
                    calls,
                    supported,
                }));
            }
            _ => {}
        }
    }
    Ok(bodies)
}

fn recursive(index: u32, bodies: &[Option<Body>], imports: u32) -> bool {
    let mut pending = bodies[(index - imports) as usize]
        .as_ref()
        .unwrap()
        .calls
        .clone();
    let mut seen = BTreeSet::new();
    while let Some(callee) = pending.pop() {
        if callee == index {
            return true;
        }
        if callee >= imports
            && seen.insert(callee)
            && let Some(body) = &bodies[(callee - imports) as usize]
        {
            pending.extend(&body.calls);
        }
    }
    false
}

fn default_value(ty: ValType, null_types: &[HeapType]) -> Option<Instruction<'static>> {
    Some(match ty {
        ValType::I32 => Instruction::I32Const(0),
        ValType::I64 => Instruction::I64Const(0),
        ValType::F32 => Instruction::F32Const(0.0.into()),
        ValType::F64 => Instruction::F64Const(0.0.into()),
        ValType::V128 => Instruction::V128Const(0),
        ValType::Ref(ty) if ty.nullable => Instruction::RefNull(match ty.heap_type {
            // Match the emitter's compact nulls, while keeping raw function
            // references in their distinct heap-type hierarchy.
            HeapType::Concrete(index) => null_types[index as usize],
            heap_type => heap_type,
        }),
        _ => return None,
    })
}

fn inline(caller: &Body, callee: &Body, index: u32) -> Result<(Vec<u8>, Vec<ValType>), Error> {
    let mut encoder = RoundtripReencoder;
    let mut needs_block = false;
    let mut initialized = vec![false; callee.params.len() + callee.locals.len()];
    initialized[..callee.params.len()].fill(true);
    let mut reset = BTreeSet::new();
    let mut used = vec![false; callee.params.len()];
    let mut written = vec![false; callee.params.len()];
    let mut nesting = 0;
    for op in callee.reader().get_operators_reader()? {
        let op = op?;
        match op {
            Operator::LocalGet { local_index } if (local_index as usize) < used.len() => {
                used[local_index as usize] = true;
            }
            Operator::LocalSet { local_index } | Operator::LocalTee { local_index }
                if (local_index as usize) < used.len() =>
            {
                used[local_index as usize] = true;
                written[local_index as usize] = true;
            }
            _ => {}
        }
        match op {
            Operator::Block { .. } | Operator::Loop { .. } | Operator::If { .. } => nesting += 1,
            Operator::End => nesting -= 1,
            Operator::Return
            | Operator::Br { .. }
            | Operator::BrIf { .. }
            | Operator::BrTable { .. }
            | Operator::BrOnNull { .. }
            | Operator::BrOnNonNull { .. }
            | Operator::BrOnCast { .. }
            | Operator::BrOnCastFail { .. } => needs_block = true,
            Operator::LocalGet { local_index }
                if !initialized[local_index as usize]
                    && default_value(
                        callee.locals[local_index as usize - callee.params.len()],
                        &callee.null_types,
                    )
                    .is_some() =>
            {
                reset.insert(local_index);
            }
            Operator::LocalSet { local_index } | Operator::LocalTee { local_index }
                if nesting == 0 =>
            {
                initialized[local_index as usize] = true;
            }
            _ => {}
        }
    }
    let caller_ops = caller
        .reader()
        .get_operators_reader()?
        .into_iter()
        .map(|op| encoder.instruction(op?))
        .collect::<Result<Vec<_>, Error>>()?;
    let call = caller_ops
        .iter()
        .position(|op| matches!(op, Instruction::Call(target) if *target == index))
        .unwrap();
    let mut frames = Vec::new();
    for op in &caller_ops[..call] {
        match op {
            Instruction::Block(_) | Instruction::If(_) => frames.push(false),
            Instruction::Loop(_) => frames.push(true),
            Instruction::End => {
                frames.pop();
            }
            _ => {}
        }
    }
    // Outside a loop each fresh inline slot already has its entry default.
    if !frames.contains(&true) {
        reset.clear();
    }
    let mut substitutions = vec![None; callee.params.len()];
    let mut omit = vec![false; caller_ops.len()];
    // A suffix of pure argument pushes can be forwarded directly. Caller
    // locals cannot be mutated by the callee; globals and loads cannot be
    // forwarded because intervening effects could change their values.
    for (parameter, producer) in (0..callee.params.len()).rev().zip((0..call).rev()) {
        if !matches!(
            caller_ops[producer],
            Instruction::LocalGet(_)
                | Instruction::I32Const(_)
                | Instruction::I64Const(_)
                | Instruction::F32Const(_)
                | Instruction::F64Const(_)
                | Instruction::V128Const(_)
                | Instruction::RefNull(_)
                | Instruction::RefFunc(_)
        ) {
            break;
        }
        if !written[parameter] {
            substitutions[parameter] = Some(caller_ops[producer].clone());
            omit[producer] = true;
        }
    }
    let mut locals = caller.locals.clone();
    let mut local_map = vec![0; callee.params.len() + callee.locals.len()];
    for (local, ty) in callee.params.iter().chain(&callee.locals).enumerate() {
        if local < callee.params.len() && (substitutions[local].is_some() || !used[local]) {
            continue;
        }
        local_map[local] = (caller.params.len() + locals.len()) as u32;
        locals.push(*ty);
    }
    let mut function = Function::new_with_locals_types(locals.iter().copied());
    for (position, op) in caller_ops.iter().enumerate() {
        if omit[position] {
            continue;
        }
        match op {
            Instruction::Call(function_index) if *function_index == index && position == call => {
                // Arguments are already evaluated left-to-right on the stack.
                for parameter in (0..callee.params.len()).rev() {
                    if substitutions[parameter].is_some() {
                        continue;
                    }
                    function.instruction(&if used[parameter] {
                        Instruction::LocalSet(local_map[parameter])
                    } else {
                        Instruction::Drop
                    });
                }
                // A call inside a loop must get fresh locals on *every* entry.
                for local in &reset {
                    let ty = callee.locals[*local as usize - callee.params.len()];
                    function.instruction(&default_value(ty, &callee.null_types).unwrap());
                    function.instruction(&Instruction::LocalSet(local_map[*local as usize]));
                }
                if needs_block {
                    function.instruction(&Instruction::Block(callee.block_type));
                }
                let mut depth: u32 = 0;
                for op in callee.reader().get_operators_reader()? {
                    let instruction = match op? {
                        Operator::LocalGet { local_index } => substitutions
                            .get(local_index as usize)
                            .and_then(Clone::clone)
                            .unwrap_or(Instruction::LocalGet(local_map[local_index as usize])),
                        Operator::LocalSet { local_index } => {
                            Instruction::LocalSet(local_map[local_index as usize])
                        }
                        Operator::LocalTee { local_index } => {
                            Instruction::LocalTee(local_map[local_index as usize])
                        }
                        Operator::Return => Instruction::Br(depth),
                        op @ (Operator::Block { .. }
                        | Operator::Loop { .. }
                        | Operator::If { .. }) => {
                            depth += 1;
                            encoder.instruction(op)?
                        }
                        Operator::End => {
                            if depth == 0 && !needs_block {
                                continue;
                            }
                            depth = depth.saturating_sub(1);
                            Instruction::End
                        }
                        op => encoder.instruction(op)?,
                    };
                    function.instruction(&instruction);
                }
                // The callee's final end closes the replacement block. Branches
                // to its implicit function label therefore keep their depths.
            }
            op => {
                function.instruction(op);
            }
        }
    }
    Ok((function.into_raw_body(), locals))
}

struct Output<'a> {
    bodies: &'a [Option<Body>],
    indices: &'a [Option<u32>],
}

impl Reencode for Output<'_> {
    type Error = Infallible;

    fn function_index(&mut self, index: u32) -> Result<u32, Error> {
        Ok(self.indices[index as usize].expect("inlined functions have no remaining references"))
    }

    fn parse_function_section(
        &mut self,
        functions: &mut FunctionSection,
        _: wasmparser::FunctionSectionReader<'_>,
    ) -> Result<(), Error> {
        for body in self.bodies.iter().flatten() {
            functions.function(body.signature);
        }
        Ok(())
    }

    fn parse_code_section(
        &mut self,
        code: &mut CodeSection,
        _: wasmparser::CodeSectionReader<'_>,
    ) -> Result<(), Error> {
        for body in self.bodies.iter().flatten() {
            // Reuse the encoder's exhaustive function-reference remapping for
            // calls, ref.func, exports, elements and constant expressions alike.
            self.parse_function_body(code, body.reader())?;
        }
        Ok(())
    }
}

fn optimize_inner(
    wasm: &[u8],
    imports: u32,
    candidates: BTreeSet<u32>,
) -> Result<(Vec<u8>, Vec<Option<u32>>), Error> {
    let mut bodies = read_bodies(wasm)?;
    let mut cleanup = super::peephole::BodyCleanup::new(wasm);
    // Bound speculative body rewriting even for unusually large call graphs.
    let mut budget = 5_000_000usize;
    let count = imports as usize + bodies.len();
    let mut references = References(vec![0; count]);
    references.parse_core_module(&mut Module::new(), Parser::new(0), wasm)?;
    let mut callers = vec![None; count];
    for (caller, body) in bodies.iter().enumerate() {
        for callee in &body.as_ref().unwrap().calls {
            callers[*callee as usize] = Some(caller);
        }
    }
    for index in candidates {
        if references.0[index as usize] != 1 || recursive(index, &bodies, imports) {
            continue;
        }
        let Some(caller_index) = callers[index as usize] else {
            continue;
        };
        let callee_index = (index - imports) as usize;
        let callee = bodies[callee_index].as_ref().unwrap();
        if !callee.supported {
            continue;
        }
        let Some(caller) = bodies[caller_index].as_ref() else {
            continue;
        };
        if !caller.supported || !caller.calls.contains(&index) {
            continue;
        }
        let cost = caller.bytes.len() + callee.bytes.len();
        let Some(remaining) = budget.checked_sub(cost) else {
            break;
        };
        budget = remaining;
        let (bytes, _) = inline(caller, callee, index)?;
        let bytes = cleanup.clean(&bytes, caller_index);
        let locals: Vec<ValType> = FunctionBody::new(BinaryReader::new(&bytes, 0))
            .get_locals_reader()?
            .into_iter()
            .map(|local| {
                let (n, ty) = local?;
                Ok(std::iter::repeat_n(
                    RoundtripReencoder.val_type(ty)?,
                    n as usize,
                ))
            })
            .collect::<Result<Vec<_>, Error>>()?
            .into_iter()
            .flatten()
            .collect();
        let new_size = bytes.len() + leb_size(bytes.len() as u32);
        let old_size = caller.encoded_size() + callee.encoded_size() + leb_size(callee.signature);
        // Small temporary growth can pay for itself when dead signatures and
        // call chains disappear. The final module must still strictly shrink.
        if new_size > old_size + 64 {
            continue;
        }
        let moved_calls = callee.calls.clone();
        for target in &moved_calls {
            if references.0[*target as usize] == 1 {
                callers[*target as usize] = Some(caller_index);
            }
        }
        let caller = bodies[caller_index].as_mut().unwrap();
        // Cleanup can erase calls in constant/dead branches. Keep the graph
        // aligned with the actual replacement, not the pre-cleanup callee.
        caller.calls = FunctionBody::new(BinaryReader::new(&bytes, 0))
            .get_operators_reader()?
            .into_iter()
            .filter_map(|op| match op {
                Ok(Operator::Call { function_index }) => Some(Ok(function_index)),
                Err(error) => Some(Err(error)),
                _ => None,
            })
            .collect::<Result<Vec<_>, _>>()?;
        caller.bytes = bytes;
        caller.locals = locals;
        bodies[callee_index] = None;
    }
    let mut next = imports;
    let mut indices = (0..imports).map(Some).collect::<Vec<_>>();
    indices.extend(bodies.iter().map(|body| {
        body.as_ref().map(|_| {
            let index = next;
            next += 1;
            index
        })
    }));
    let mut module = Module::new();
    Output {
        bodies: &bodies,
        indices: &indices,
    }
    .parse_core_module(&mut module, Parser::new(0), wasm)?;
    Ok((module.finish(), indices))
}

fn add_block_types(wasm: &[u8]) -> Vec<u8> {
    struct Add;
    impl Reencode for Add {
        type Error = Infallible;
        fn parse_type_section(
            &mut self,
            out: &mut wasm_encoder::TypeSection,
            section: wasmparser::TypeSectionReader<'_>,
        ) -> Result<(), Error> {
            let mut extra = Vec::new();
            for group in section.clone() {
                for ty in group?.into_types() {
                    if let CompositeInnerType::Func(ty) = ty.composite_type.inner
                        && ty.results().len() > 1
                    {
                        extra.push(
                            ty.results()
                                .iter()
                                .map(|&t| self.val_type(t))
                                .collect::<Result<Vec<_>, _>>()?,
                        );
                    }
                }
            }
            reencode::utils::parse_type_section(self, out, section)?;
            extra.sort();
            extra.dedup();
            for results in extra {
                out.ty().function([], results);
            }
            Ok(())
        }
    }
    let mut module = Module::new();
    Add.parse_core_module(&mut module, Parser::new(0), wasm)
        .unwrap();
    module.finish()
}

/// Expand single-reference direct callees. The enclosing pipeline checks the
/// complete output after cleanup and type removal before accepting any change.
pub(super) fn expand(wasm: &[u8]) -> (Vec<u8>, Vec<Option<u32>>) {
    let wasm = add_block_types(wasm);
    let mut imports = 0;
    let mut count = 0;
    for payload in Parser::new(0).parse_all(&wasm) {
        match payload.unwrap() {
            Payload::ImportSection(s) => {
                for i in s.into_imports() {
                    if matches!(
                        i.unwrap().ty,
                        wasmparser::TypeRef::Func(_) | wasmparser::TypeRef::FuncExact(_)
                    ) {
                        imports += 1;
                    }
                }
            }
            Payload::FunctionSection(s) => count = s.count(),
            _ => {}
        }
    }
    optimize_inner(&wasm, imports, (imports..imports + count).collect()).unwrap()
}

#[cfg(test)]
mod tests;
