//! Size-gated sharing of bodies that differ only in integer constants.
//! Keep each original function as a wrapper, preserving all existing indices.
use std::{collections::BTreeMap, convert::Infallible};
use wasm_encoder::{
    CodeSection, Encode, Function, FunctionSection, Instruction as I, Module, TypeSection, ValType,
    reencode::{self, Reencode, RoundtripReencoder},
};
use wasmparser::{CompositeInnerType, Operator, Parser, Payload};

type Error = reencode::Error<Infallible>;

#[cfg(test)]
mod tests;

#[derive(Clone, Copy, PartialEq, Eq)]
enum Integer {
    I32(i32),
    I64(i64),
}

impl Integer {
    fn instruction(self) -> I<'static> {
        match self {
            Self::I32(v) => I::I32Const(v),
            Self::I64(v) => I::I64Const(v),
        }
    }
    fn ty(self) -> ValType {
        match self {
            Self::I32(_) => ValType::I32,
            Self::I64(_) => ValType::I64,
        }
    }
}

struct Signature {
    params: Vec<ValType>,
    results: Vec<ValType>,
}
struct Body<'a> {
    signature: u32,
    locals: Vec<(u32, ValType)>,
    ops: Vec<I<'a>>,
    constants: Vec<Integer>,
    bytes: &'a [u8],
}
struct Shared {
    signature: Signature,
    body: Vec<u8>,
    members: Vec<usize>,
}

fn integer(op: &I<'_>) -> Option<Integer> {
    match op {
        I::I32Const(v) => Some(Integer::I32(*v)),
        I::I64Const(v) => Some(Integer::I64(*v)),
        _ => None,
    }
}

fn encoded_len(value: u32) -> usize {
    let mut bytes = Vec::new();
    value.encode(&mut bytes);
    bytes.len()
}
fn body_size(bytes: &[u8]) -> usize {
    bytes.len() + encoded_len(bytes.len() as u32)
}

pub(super) fn optimize(wasm: &[u8], report: Option<&mut super::CodegenReport>) -> Vec<u8> {
    let mut types = Vec::new();
    let mut function_types = Vec::new();
    let mut bodies = Vec::new();
    let mut groups: BTreeMap<Vec<u8>, Vec<usize>> = BTreeMap::new();
    let mut imports = 0;
    let mut encoder = RoundtripReencoder;
    for payload in Parser::new(0).parse_all(wasm) {
        match payload.unwrap() {
            Payload::TypeSection(section) => {
                for group in section {
                    for ty in group.unwrap().into_types() {
                        let shared = ty.composite_type.shared;
                        types.push(match ty.composite_type.inner {
                            CompositeInnerType::Func(ty) if !shared => Some(Signature {
                                params: ty
                                    .params()
                                    .iter()
                                    .map(|v| encoder.val_type(*v).unwrap())
                                    .collect(),
                                results: ty
                                    .results()
                                    .iter()
                                    .map(|v| encoder.val_type(*v).unwrap())
                                    .collect(),
                            }),
                            _ => None,
                        });
                    }
                }
            }
            Payload::ImportSection(section) => {
                for import in section.into_imports() {
                    if matches!(
                        import.unwrap().ty,
                        wasmparser::TypeRef::Func(_) | wasmparser::TypeRef::FuncExact(_)
                    ) {
                        imports += 1;
                    }
                }
            }
            Payload::FunctionSection(section) => {
                function_types = section.into_iter().map(Result::unwrap).collect()
            }
            Payload::CodeSectionEntry(body) => {
                let signature = function_types[bodies.len()];
                let mut locals = Vec::new();
                for local in body.get_locals_reader().unwrap() {
                    let (count, ty) = local.unwrap();
                    locals.push((count, encoder.val_type(ty).unwrap()));
                }
                let mut shape = Vec::new();
                signature.encode(&mut shape);
                shape.extend(Function::new(locals.iter().copied()).into_raw_body());
                let mut constants = Vec::new();
                let mut ops = Vec::new();
                let mut supported = types[signature as usize].is_some();
                for op in body.get_operators_reader().unwrap() {
                    let op = op.unwrap();
                    // Wrapping tail calls would turn constant-stack recursion
                    // into growing wrapper frames. Keep other frame-sensitive
                    // exception/continuation constructs intact as well.
                    if matches!(
                        op,
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
                            | Operator::Switch { .. }
                    ) {
                        supported = false;
                    }
                    let op = encoder.instruction(op).unwrap();
                    if let Some(value) = integer(&op) {
                        constants.push(value);
                        match value {
                            Integer::I32(_) => I::I32Const(0),
                            Integer::I64(_) => I::I64Const(0),
                        }
                        .encode(&mut shape);
                    } else {
                        op.encode(&mut shape);
                    }
                    ops.push(op);
                }
                if supported && !constants.is_empty() {
                    groups.entry(shape).or_default().push(bodies.len());
                }
                bodies.push(Body {
                    signature,
                    locals,
                    ops,
                    constants,
                    bytes: body.as_bytes(),
                });
            }
            _ => {}
        }
    }
    let mut replacements = vec![None; bodies.len()];
    let mut shared = Vec::new();
    for members in groups.into_values().filter(|g| g.len() > 1) {
        // Respect the resource limits enforced by wasmparser and Wasm engines.
        if types.len() + shared.len() >= 1_000_000
            || imports as usize + bodies.len() + shared.len() >= 1_000_000
        {
            break;
        }
        let first = &bodies[members[0]];
        let original_type = types[first.signature as usize].as_ref().unwrap();
        let params = original_type.params.len() as u32;
        let mut columns: Vec<Vec<Integer>> = Vec::new();
        let mut positions = Vec::new();
        for at in 0..first.constants.len() {
            let values = members
                .iter()
                .map(|i| bodies[*i].constants[at])
                .collect::<Vec<_>>();
            positions.push(if values.iter().all(|v| *v == values[0]) {
                None
            } else {
                let index = columns
                    .iter()
                    .position(|column| *column == values)
                    .unwrap_or_else(|| {
                        columns.push(values);
                        columns.len() - 1
                    });
                Some(index as u32)
            });
            // Bound column comparisons rather than constructing an arbitrary
            // number of parameters only to reject the group afterward.
            if columns.len() > 8 {
                break;
            }
        }
        if columns.is_empty()
            || columns.len() > 8
            || params as usize + columns.len() > 1000
            || params + columns.len() as u32 + first.locals.iter().map(|(n, _)| n).sum::<u32>()
                > 50_000
        {
            continue;
        }
        let added = columns.len() as u32;
        let mut helper = Function::new(first.locals.iter().copied());
        let mut constant = 0;
        for op in &first.ops {
            let mut op = op.clone();
            if integer(&op).is_some() {
                if let Some(parameter) = positions[constant] {
                    op = I::LocalGet(params + parameter);
                }
                constant += 1;
            } else if let I::LocalGet(index) | I::LocalSet(index) | I::LocalTee(index) = &mut op
                && *index >= params
            {
                *index += added;
            }
            helper.instruction(&op);
        }
        let helper = helper.into_raw_body();
        if helper.len() > 7_654_321 {
            continue;
        }
        let helper_index = imports + bodies.len() as u32 + shared.len() as u32;
        let mut wrappers = Vec::new();
        for member in 0..members.len() {
            let mut wrapper = Function::new([]);
            for index in 0..params {
                wrapper.instruction(&I::LocalGet(index));
            }
            for column in &columns {
                wrapper.instruction(&column[member].instruction());
            }
            wrapper
                .instruction(&I::Call(helper_index))
                .instruction(&I::End);
            wrappers.push(wrapper.into_raw_body());
        }
        let signature = Signature {
            params: original_type
                .params
                .iter()
                .copied()
                .chain(columns.iter().map(|c| c[0].ty()))
                .collect(),
            results: original_type.results.clone(),
        };
        let mut encoded_type = vec![0x60];
        (signature.params.len() as u32).encode(&mut encoded_type);
        for ty in &signature.params {
            ty.encode(&mut encoded_type);
        }
        (signature.results.len() as u32).encode(&mut encoded_type);
        for ty in &signature.results {
            ty.encode(&mut encoded_type);
        }
        let before: usize = members.iter().map(|i| body_size(bodies[*i].bytes)).sum();
        let after = wrappers.iter().map(|b| body_size(b)).sum::<usize>()
            + body_size(&helper)
            + encoded_type.len()
            + encoded_len(types.len() as u32 + shared.len() as u32);
        if after >= before {
            continue;
        }
        for (member, wrapper) in members.iter().zip(wrappers) {
            replacements[*member] = Some(wrapper);
        }
        shared.push(Shared {
            signature,
            body: helper,
            members,
        });
    }
    if shared.is_empty() {
        return wasm.to_vec();
    }
    let mut output = Module::new();
    Output {
        bodies: &bodies,
        replacements: &replacements,
        shared: &shared,
        type_count: types.len() as u32,
    }
    .parse_core_module(&mut output, Parser::new(0), wasm)
    .unwrap();
    let output = output.finish();
    // Include section-count/length LEB changes and all wrapper/type overhead.
    if output.len() >= wasm.len() {
        return wasm.to_vec();
    }
    if let Some(report) = report {
        for (at, helper) in shared.iter().enumerate() {
            let index = imports + bodies.len() as u32 + at as u32;
            let names = helper
                .members
                .iter()
                .map(|i| {
                    let index = imports + *i as u32;
                    report
                        .functions
                        .iter()
                        .find(|(n, _)| *n == index)
                        .map(|(_, name)| name.clone())
                        .unwrap_or_else(|| format!("function {index}"))
                })
                .collect::<Vec<_>>();
            report
                .functions
                .push((index, format!("shared constants: {}", names.join(", "))));
        }
    }
    output
}

struct Output<'a, 'b> {
    bodies: &'a [Body<'b>],
    replacements: &'a [Option<Vec<u8>>],
    shared: &'a [Shared],
    type_count: u32,
}
impl Reencode for Output<'_, '_> {
    type Error = Infallible;
    fn parse_type_section(
        &mut self,
        types: &mut TypeSection,
        reader: wasmparser::TypeSectionReader<'_>,
    ) -> Result<(), Error> {
        reencode::utils::parse_type_section(self, types, reader)?;
        for helper in self.shared {
            types.ty().function(
                helper.signature.params.iter().copied(),
                helper.signature.results.iter().copied(),
            );
        }
        Ok(())
    }
    fn parse_function_section(
        &mut self,
        functions: &mut FunctionSection,
        reader: wasmparser::FunctionSectionReader<'_>,
    ) -> Result<(), Error> {
        reencode::utils::parse_function_section(self, functions, reader)?;
        for i in 0..self.shared.len() {
            functions.function(self.type_count + i as u32);
        }
        Ok(())
    }
    fn parse_code_section(
        &mut self,
        code: &mut CodeSection,
        _: wasmparser::CodeSectionReader<'_>,
    ) -> Result<(), Error> {
        for (body, replacement) in self.bodies.iter().zip(self.replacements) {
            code.raw(replacement.as_deref().unwrap_or(body.bytes));
        }
        for helper in self.shared {
            code.raw(&helper.body);
        }
        Ok(())
    }
}
