//! Specialize private direct-call signatures using facts from every call site.
//! Only inert, single-instruction arguments may be erased. Unknown argument
//! expressions, exported functions and function references remain untouched.
use std::convert::Infallible;
use wasm_encoder::{
    CodeSection, Function, FunctionSection, Instruction as I, Module, TypeSection, ValType,
    reencode::{self, Reencode, RoundtripReencoder},
};
use wasmparser::{CompositeInnerType, Parser, Payload};

type Error = reencode::Error<Infallible>;

#[cfg(test)]
mod tests;

#[derive(Clone, Copy, PartialEq, Eq)]
enum Integer {
    I32(i32),
    I64(i64),
}
impl Integer {
    fn read(op: &I<'_>) -> Option<Self> {
        match op {
            I::I32Const(v) => Some(Self::I32(*v)),
            I::I64Const(v) => Some(Self::I64(*v)),
            _ => None,
        }
    }
    fn instruction(self) -> I<'static> {
        match self {
            Self::I32(v) => I::I32Const(v),
            Self::I64(v) => I::I64Const(v),
        }
    }
}

#[derive(Clone)]
struct Signature {
    params: Vec<ValType>,
    results: Vec<ValType>,
}
struct Body<'a> {
    signature: u32,
    locals: Vec<(u32, ValType)>,
    ops: Vec<I<'a>>,
}
struct Plan {
    signature: u32,
    removed: Vec<bool>,
    constants: Vec<Option<Integer>>,
    mapping: Vec<u32>,
    materialized: Vec<(usize, ValType, Integer)>,
}
struct References(Vec<usize>);
impl Reencode for References {
    type Error = Infallible;
    fn function_index(&mut self, index: u32) -> Result<u32, Error> {
        self.0[index as usize] += 1;
        Ok(index)
    }
}
/// The caller performs cleanup and compares complete module sizes afterward.
pub(super) fn specialize(wasm: &[u8]) -> Vec<u8> {
    let mut types = Vec::new();
    let mut signatures = Vec::new();
    let mut imported = 0;
    let mut bodies = Vec::new();
    let mut encoder = RoundtripReencoder;
    for payload in Parser::new(0).parse_all(wasm) {
        match payload.unwrap() {
            Payload::TypeSection(section) => {
                for group in section {
                    for ty in group.unwrap().into_types() {
                        types.push(
                            if ty.is_final
                                && ty.supertype_idx.is_none()
                                && !ty.composite_type.shared
                            {
                                if let CompositeInnerType::Func(f) = ty.composite_type.inner {
                                    Some(Signature {
                                        params: f
                                            .params()
                                            .iter()
                                            .map(|&v| encoder.val_type(v).unwrap())
                                            .collect(),
                                        results: f
                                            .results()
                                            .iter()
                                            .map(|&v| encoder.val_type(v).unwrap())
                                            .collect(),
                                    })
                                } else {
                                    None
                                }
                            } else {
                                None
                            },
                        );
                    }
                }
            }
            Payload::ImportSection(section) => {
                for import in section.into_imports() {
                    if let wasmparser::TypeRef::Func(ty) | wasmparser::TypeRef::FuncExact(ty) =
                        import.unwrap().ty
                    {
                        signatures.push(ty);
                        imported += 1;
                    }
                }
            }
            Payload::FunctionSection(section) => {
                signatures.extend(section.into_iter().map(Result::unwrap))
            }
            Payload::CodeSectionEntry(body) => bodies.push(Body {
                signature: signatures[imported + bodies.len()],
                locals: body
                    .get_locals_reader()
                    .unwrap()
                    .into_iter()
                    .map(|l| {
                        let (n, ty) = l.unwrap();
                        (n, encoder.val_type(ty).unwrap())
                    })
                    .collect(),
                ops: body
                    .get_operators_reader()
                    .unwrap()
                    .into_iter()
                    .map(|op| encoder.instruction(op.unwrap()).unwrap())
                    .collect(),
            }),
            _ => {}
        }
    }
    let mut refs = References(vec![0; signatures.len()]);
    refs.parse_core_module(&mut Module::new(), Parser::new(0), wasm)
        .unwrap();
    let mut calls = vec![0; signatures.len()];
    let mut erasable: Vec<_> = signatures
        .iter()
        .map(|&ty| vec![true; types[ty as usize].as_ref().map_or(0, |s| s.params.len())])
        .collect();
    let mut constants: Vec<Vec<Option<Integer>>> =
        erasable.iter().map(|p| vec![None; p.len()]).collect();
    let operands = super::peephole::CallOperands::new(wasm);
    for body in &bodies {
        for (at, op) in body.ops.iter().enumerate() {
            let I::Call(index) = op else { continue };
            let index = *index as usize;
            for (param, position) in operands
                .inert_arguments(&body.ops, at, erasable[index].len())
                .into_iter()
                .enumerate()
            {
                let Some(position) = position else {
                    erasable[index][param] = false;
                    continue;
                };
                let value = Integer::read(&body.ops[position]);
                if calls[index] == 0 {
                    constants[index][param] = value;
                } else if constants[index][param] != value {
                    constants[index][param] = None;
                }
            }
            calls[index] += 1;
        }
    }
    let original_types = types.len();
    let mut plans: Vec<Option<Plan>> = (0..signatures.len()).map(|_| None).collect();
    for (defined, body) in bodies.iter().enumerate() {
        let index = imported + defined;
        if calls[index] == 0 || refs.0[index] != calls[index] {
            continue;
        }
        let Some(sig) = &types[body.signature as usize] else {
            continue;
        };
        let mut read = vec![false; sig.params.len()];
        let mut written = read.clone();
        for op in &body.ops {
            match op {
                I::LocalGet(p) if (*p as usize) < read.len() => read[*p as usize] = true,
                I::LocalSet(p) | I::LocalTee(p) if (*p as usize) < read.len() => {
                    written[*p as usize] = true
                }
                _ => {}
            }
        }
        let removed: Vec<_> = (0..sig.params.len())
            .map(|p| erasable[index][p] && (!read[p] || constants[index][p].is_some()))
            .collect();
        if !removed.iter().any(|&p| p) {
            continue;
        }
        let new = Signature {
            params: sig
                .params
                .iter()
                .enumerate()
                .filter_map(|(p, &v)| (!removed[p]).then_some(v))
                .collect(),
            results: sig.results.clone(),
        };
        let original_params = sig.params.clone();
        let signature = types
            .iter()
            .position(|ty| {
                ty.as_ref()
                    .is_some_and(|ty| ty.params == new.params && ty.results == new.results)
            })
            .unwrap_or_else(|| {
                types.push(Some(new));
                types.len() - 1
            }) as u32;
        let mut mapping = Vec::new();
        let mut next = 0;
        for &remove in &removed {
            mapping.push(next);
            next += u32::from(!remove);
        }
        for (count, _) in &body.locals {
            for _ in 0..*count {
                mapping.push(next);
                next += 1;
            }
        }
        let mut materialized = Vec::new();
        for p in 0..removed.len() {
            if removed[p] && read[p] && written[p] {
                mapping[p] = next;
                next += 1;
                materialized.push((p, original_params[p], constants[index][p].unwrap()));
            }
        }
        plans[index] = Some(Plan {
            signature,
            removed,
            constants: constants[index].clone(),
            mapping,
            materialized,
        });
    }
    if plans.iter().all(Option::is_none) {
        return wasm.to_vec();
    }
    let mut rewritten = Vec::new();
    for (defined, body) in bodies.iter().enumerate() {
        let mut erase = vec![false; body.ops.len()];
        for (at, op) in body.ops.iter().enumerate() {
            let I::Call(index) = op else { continue };
            let Some(plan) = &plans[*index as usize] else {
                continue;
            };
            for (param, position) in operands
                .inert_arguments(&body.ops, at, plan.removed.len())
                .into_iter()
                .enumerate()
            {
                if plan.removed[param] {
                    erase[position.unwrap()] = true;
                }
            }
        }
        let plan = plans[imported + defined].as_ref();
        let extra = plan
            .map(|p| {
                p.materialized
                    .iter()
                    .map(|(_, ty, _)| (1, *ty))
                    .collect::<Vec<_>>()
            })
            .unwrap_or_default();
        let mut function =
            Function::new(body.locals.iter().copied().chain(extra).collect::<Vec<_>>());
        if let Some(plan) = plan {
            for &(param, _, constant) in &plan.materialized {
                function
                    .instruction(&constant.instruction())
                    .instruction(&I::LocalSet(plan.mapping[param]));
            }
        }
        for (at, op) in body.ops.iter().enumerate() {
            if erase[at] {
                continue;
            }
            let mut op = op.clone();
            if let Some(plan) = &plans[imported + defined] {
                match &mut op {
                    I::LocalGet(p) | I::LocalSet(p) | I::LocalTee(p) => {
                        let index = *p as usize;
                        if plan.removed.get(index).copied().unwrap_or(false)
                            && !plan.materialized.iter().any(|(p, _, _)| *p == index)
                        {
                            op = match op {
                                I::LocalGet(_) => plan.constants[index].unwrap().instruction(),
                                I::LocalSet(_) => I::Drop,
                                _ => I::Nop,
                            };
                        } else {
                            *p = plan.mapping[index];
                        }
                    }
                    _ => {}
                }
            }
            function.instruction(&op);
        }
        rewritten.push(function.into_raw_body());
    }
    struct Output<'a> {
        types: &'a [Option<Signature>],
        plans: &'a [Option<Plan>],
        bodies: &'a [Vec<u8>],
        imported: usize,
        next: usize,
    }
    impl Reencode for Output<'_> {
        type Error = Infallible;
        fn parse_type_section(
            &mut self,
            out: &mut TypeSection,
            section: wasmparser::TypeSectionReader<'_>,
        ) -> Result<(), Error> {
            reencode::utils::parse_type_section(self, out, section)?;
            for sig in self.types {
                let sig = sig.as_ref().unwrap();
                out.ty()
                    .function(sig.params.iter().copied(), sig.results.iter().copied());
            }
            Ok(())
        }
        fn parse_function_section(
            &mut self,
            out: &mut FunctionSection,
            section: wasmparser::FunctionSectionReader<'_>,
        ) -> Result<(), Error> {
            for (defined, ty) in section.into_iter().enumerate() {
                out.function(
                    self.plans[self.imported + defined]
                        .as_ref()
                        .map_or(ty?, |p| p.signature),
                );
            }
            Ok(())
        }
        fn parse_function_body(
            &mut self,
            out: &mut CodeSection,
            _: wasmparser::FunctionBody<'_>,
        ) -> Result<(), Error> {
            out.raw(&self.bodies[self.next]);
            self.next += 1;
            Ok(())
        }
    }
    let mut output = Module::new();
    Output {
        types: &types[original_types..],
        plans: &plans,
        bodies: &rewritten,
        imported,
        next: 0,
    }
    .parse_core_module(&mut output, Parser::new(0), wasm)
    .unwrap();
    output.finish()
}
