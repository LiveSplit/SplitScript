//! Release-only removal of unobservable globals and propagation of constants.
//! Imported/exported globals and references outside function bodies stay pinned.
use std::convert::Infallible;
use wasm_encoder::{
    CodeSection, Encode, GlobalSection, Instruction as I, Module,
    reencode::{self, Reencode, RoundtripReencoder},
};
use wasmparser::{Operator as O, Parser, Payload, TypeRef};

type Error = reencode::Error<Infallible>;

#[cfg(test)]
mod tests;

#[derive(Clone, Copy, PartialEq)]
enum Literal {
    I32(i32),
    I64(i64),
    F32(u32),
    F64(u64),
    Null(wasmparser::HeapType),
}

impl Literal {
    fn from_op(op: &O<'_>) -> Option<Self> {
        Some(match op {
            O::I32Const { value } => Self::I32(*value),
            O::I64Const { value } => Self::I64(*value),
            O::F32Const { value } => Self::F32(value.bits()),
            O::F64Const { value } => Self::F64(value.bits()),
            O::RefNull { hty } => Self::Null(*hty),
            _ => return None,
        })
    }

    fn instruction(self) -> I<'static> {
        match self {
            Self::I32(v) => I::I32Const(v),
            Self::I64(v) => I::I64Const(v),
            Self::F32(v) => I::F32Const(f32::from_bits(v).into()),
            Self::F64(v) => I::F64Const(f64::from_bits(v).into()),
            Self::Null(ty) => I::RefNull(RoundtripReencoder.heap_type(ty).unwrap()),
        }
    }
}

#[derive(Default)]
struct Global {
    init: Option<Literal>,
    pinned: bool,
    read: bool,
    changed: bool,
}

struct Analyze {
    globals: Vec<Global>,
}

impl Reencode for Analyze {
    type Error = Infallible;

    // Covers exports, initializers, data/element offsets, and any unfamiliar
    // instruction referencing a global. Such uses cannot be rewritten here.
    fn global_index(&mut self, index: u32) -> Result<u32, Error> {
        self.globals[index as usize].pinned = true;
        Ok(index)
    }

    fn parse_function_body(
        &mut self,
        _: &mut CodeSection,
        body: wasmparser::FunctionBody<'_>,
    ) -> Result<(), Error> {
        let mut previous = None;
        for op in body.get_operators_reader()? {
            let op = op?;
            match &op {
                O::GlobalGet { global_index } => self.globals[*global_index as usize].read = true,
                O::GlobalSet { global_index } => {
                    let global = &mut self.globals[*global_index as usize];
                    if previous.is_none() || previous != global.init {
                        global.changed = true;
                    }
                }
                _ => {
                    self.instruction(op.clone())?;
                }
            }
            previous = Literal::from_op(&op);
        }
        Ok(())
    }
}

pub(super) fn optimize(wasm: &[u8]) -> Vec<u8> {
    let mut globals = Vec::new();
    let mut imported = 0;
    for payload in Parser::new(0).parse_all(wasm) {
        match payload.unwrap() {
            Payload::ImportSection(section) => {
                for import in section.into_imports() {
                    if matches!(import.unwrap().ty, TypeRef::Global(_)) {
                        globals.push(Global {
                            pinned: true,
                            ..Default::default()
                        });
                        imported += 1;
                    }
                }
            }
            Payload::GlobalSection(section) => {
                for global in section {
                    let global = global.unwrap();
                    let mut ops = global.init_expr.get_operators_reader();
                    let init = Literal::from_op(&ops.read().unwrap());
                    let simple = matches!(ops.read().unwrap(), O::End) && ops.eof();
                    globals.push(Global {
                        init: if simple { init } else { None },
                        // Preserve potentially effectful/allocating initializers,
                        // and all shared globals (including atomic accesses).
                        pinned: !simple || init.is_none() || global.ty.shared,
                        ..Default::default()
                    });
                }
            }
            _ => {}
        }
    }
    if globals.is_empty() {
        return wasm.to_vec();
    }
    let mut analysis = Analyze { globals };
    analysis
        .parse_core_module(&mut Module::new(), Parser::new(0), wasm)
        .unwrap();
    let mut next = 0;
    let mut indices = Vec::new();
    for (index, global) in analysis.globals.iter().enumerate() {
        let replace_reads = !global.changed
            && global.init.is_some_and(|init| {
                let mut literal = Vec::new();
                init.instruction().encode(&mut literal);
                let mut get = Vec::new();
                I::GlobalGet(index as u32).encode(&mut get);
                literal.len() <= get.len()
            });
        let remove = !global.pinned && (!global.read || replace_reads);
        indices.push(if remove {
            None
        } else {
            let index = next;
            next += 1;
            Some(index)
        });
    }
    if indices.iter().all(Option::is_some) {
        return wasm.to_vec();
    }
    let mut output = Module::new();
    Rewrite {
        globals: analysis.globals,
        indices,
        imported,
    }
    .parse_core_module(&mut output, Parser::new(0), wasm)
    .unwrap();
    let result = output.finish();
    if result.len() < wasm.len() {
        result
    } else {
        wasm.to_vec()
    }
}

struct Rewrite {
    globals: Vec<Global>,
    indices: Vec<Option<u32>>,
    imported: usize,
}

impl Reencode for Rewrite {
    type Error = Infallible;

    fn global_index(&mut self, index: u32) -> Result<u32, Error> {
        Ok(self.indices[index as usize].expect("all remaining global references are retained"))
    }

    fn parse_global_section(
        &mut self,
        output: &mut GlobalSection,
        section: wasmparser::GlobalSectionReader<'_>,
    ) -> Result<(), Error> {
        for (index, global) in section.into_iter().enumerate() {
            let global = global?;
            if self.indices[self.imported + index].is_some() {
                self.parse_global(output, global)?;
            }
        }
        Ok(())
    }

    fn instruction<'a>(&mut self, op: O<'a>) -> Result<I<'a>, Error> {
        match op {
            O::GlobalGet { global_index } if self.indices[global_index as usize].is_none() => {
                Ok(self.globals[global_index as usize]
                    .init
                    .unwrap()
                    .instruction())
            }
            O::GlobalSet { global_index } if self.indices[global_index as usize].is_none() => {
                Ok(I::Drop)
            }
            _ => reencode::utils::instruction(self, op),
        }
    }
}
