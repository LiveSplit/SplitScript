//! Remove unreachable whole recursive type groups after inlining. A live group
//! keeps every member and its dependencies, preserving recursive type identity.
use std::convert::Infallible;
use wasm_encoder::{
    Module, TypeSection,
    reencode::{self, Reencode},
};
use wasmparser::{Parser, Payload};
type Error = reencode::Error<Infallible>;
struct Scan {
    used: Vec<bool>,
}
impl Reencode for Scan {
    type Error = Infallible;
    fn type_index(&mut self, index: u32) -> Result<u32, Error> {
        self.used[index as usize] = true;
        Ok(index)
    }
    fn parse_type_section(
        &mut self,
        _: &mut TypeSection,
        _: wasmparser::TypeSectionReader<'_>,
    ) -> Result<(), Error> {
        Ok(())
    }
}
pub(super) fn optimize(wasm: &[u8]) -> Vec<u8> {
    let mut groups = Vec::new();
    let mut count = 0;
    for payload in Parser::new(0).parse_all(wasm) {
        if let Payload::TypeSection(section) = payload.unwrap() {
            for group in section {
                let group = group.unwrap();
                let start = count;
                count += group.types().len();
                groups.push((start, group));
            }
        }
    }
    let mut scan = Scan {
        used: vec![false; count],
    };
    scan.parse_core_module(&mut Module::new(), Parser::new(0), wasm)
        .unwrap();
    let mut dependencies = Vec::new();
    for (_, group) in &groups {
        let mut refs = Scan {
            used: vec![false; count],
        };
        for ty in group.types() {
            refs.sub_type(ty.clone()).unwrap();
        }
        dependencies.push(
            refs.used
                .iter()
                .enumerate()
                .filter_map(|(i, &v)| v.then_some(i))
                .collect::<Vec<_>>(),
        );
    }
    let mut kept = vec![false; groups.len()];
    loop {
        let mut changed = false;
        for (g, (start, group)) in groups.iter().enumerate() {
            if !kept[g]
                && scan.used[*start..*start + group.types().len()]
                    .iter()
                    .any(|&v| v)
            {
                kept[g] = true;
                changed = true;
                for &dep in &dependencies[g] {
                    scan.used[dep] = true;
                }
            }
        }
        if !changed {
            break;
        }
    }
    let mut mapping = vec![None; count];
    let mut next = 0;
    for ((start, group), &keep) in groups.iter().zip(&kept) {
        if keep {
            for mapped in &mut mapping[*start..*start + group.types().len()] {
                *mapped = Some(next);
                next += 1;
            }
        }
    }
    struct Output {
        mapping: Vec<Option<u32>>,
        kept: Vec<bool>,
    }
    impl Reencode for Output {
        type Error = Infallible;
        fn type_index(&mut self, index: u32) -> Result<u32, Error> {
            Ok(self.mapping[index as usize].unwrap())
        }
        fn parse_type_section(
            &mut self,
            out: &mut TypeSection,
            section: wasmparser::TypeSectionReader<'_>,
        ) -> Result<(), Error> {
            for (index, group) in section.into_iter().enumerate() {
                let group = group?;
                if self.kept[index] {
                    self.parse_recursive_type_group(out.ty(), group)?;
                }
            }
            Ok(())
        }
    }
    let mut module = Module::new();
    Output { mapping, kept }
        .parse_core_module(&mut module, Parser::new(0), wasm)
        .unwrap();
    module.finish()
}

#[cfg(test)]
mod tests {
    use super::optimize;
    use wasm_encoder::{
        CodeSection, CompositeInnerType, CompositeType, ExportKind, ExportSection, FieldType,
        Function, FunctionSection, HeapType, Instruction, Module, RefType, StorageType, StructType,
        SubType, TypeSection, ValType,
    };
    use wasmparser::{Parser, Payload};

    #[test]
    fn live_recursive_groups_keep_unused_members_and_remap_all_references() {
        let mut types = TypeSection::new();
        types.ty().function([ValType::F64], []);
        let structure = |fields: Vec<FieldType>| SubType {
            is_final: true,
            supertype_idx: None,
            composite_type: CompositeType {
                inner: CompositeInnerType::Struct(StructType {
                    fields: fields.into(),
                }),
                shared: false,
                descriptor: None,
                describes: None,
            },
        };
        types.ty().rec([
            structure(vec![]),
            structure(vec![FieldType {
                element_type: StorageType::Val(ValType::Ref(RefType {
                    nullable: true,
                    heap_type: HeapType::Concrete(2),
                })),
                mutable: false,
            }]),
        ]);
        types.ty().function(
            [],
            [ValType::Ref(RefType {
                nullable: false,
                heap_type: HeapType::Concrete(2),
            })],
        );
        types.ty().function([ValType::I32], [ValType::I64]);
        let mut functions = FunctionSection::new();
        functions.function(3);
        let mut exports = ExportSection::new();
        exports.export("run", ExportKind::Func, 0);
        let mut body = Function::new([]);
        body.instruction(&Instruction::StructNewDefault(2))
            .instruction(&Instruction::End);
        let mut code = CodeSection::new();
        code.function(&body);
        let mut module = Module::new();
        module
            .section(&types)
            .section(&functions)
            .section(&exports)
            .section(&code);
        let before = module.finish();
        let after = optimize(&before);
        assert!(after.len() < before.len());
        for wasm in [&before, &after] {
            wasmparser::Validator::new_with_features(wasmparser::WasmFeatures::all())
                .validate_all(wasm)
                .unwrap();
        }
        let mut lengths = Vec::new();
        for p in Parser::new(0).parse_all(&after) {
            if let Payload::TypeSection(section) = p.unwrap() {
                for g in section {
                    lengths.push(g.unwrap().types().len());
                }
            }
        }
        assert_eq!(lengths, [2, 1]);
    }
}
