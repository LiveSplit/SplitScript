use super::{
    I, Passes,
    instruction_tests::{instantiate, module},
    optimize,
};
use wasm_encoder::{BlockType, FieldType, HeapType, RefType, StorageType, TypeSection, ValType};

fn cleanup(wasm: &[u8]) -> Vec<u8> {
    optimize(
        wasm,
        Passes {
            instructions: true,
            locals: true,
            control: true,
            flow_locals: true,
            ..Default::default()
        },
    )
}

fn gc_module(fields: &[StorageType], ops: &[I<'_>]) -> Vec<u8> {
    let mut types = TypeSection::new();
    types
        .ty()
        .struct_(fields.iter().map(|&element_type| FieldType {
            element_type,
            mutable: true,
        }));
    types.ty().function([ValType::I32], [ValType::I64]);
    types.ty().function([ValType::I64], [ValType::I64]);
    module(
        types,
        1,
        &[
            (
                1,
                ValType::Ref(RefType {
                    nullable: true,
                    heap_type: HeapType::Concrete(0),
                }),
            ),
            (2, ValType::I64),
        ],
        ops,
        &[("effect", 2)],
    )
}

fn get(field_index: u32) -> I<'static> {
    I::StructGet {
        struct_type_index: 0,
        field_index,
    }
}
fn set(field_index: u32) -> I<'static> {
    I::StructSet {
        struct_type_index: 0,
        field_index,
    }
}

fn allocations(wasm: &[u8]) -> usize {
    wasmparser::Parser::new(0)
        .parse_all(wasm)
        .filter_map(|p| {
            if let wasmparser::Payload::CodeSectionEntry(body) = p.unwrap() {
                Some(body)
            } else {
                None
            }
        })
        .map(|body| {
            body.get_operators_reader()
                .unwrap()
                .into_iter()
                .filter(|op| {
                    matches!(
                        op.as_ref().unwrap(),
                        wasmparser::Operator::StructNew { .. }
                            | wasmparser::Operator::StructNewDefault { .. }
                    )
                })
                .count()
        })
        .sum()
}

fn run(wasm: &[u8], arg: i32) -> (Result<i64, wasmtime::Trap>, Vec<i64>) {
    let mut config = wasmtime::Config::new();
    config.wasm_gc(true);
    config.wasm_function_references(true);
    let engine = wasmtime::Engine::new(&config).unwrap();
    let (mut store, instance) = instantiate(&engine, wasm);
    let result = instance
        .get_typed_func::<i32, i64>(&mut store, "run")
        .unwrap()
        .call(&mut store, arg)
        .map_err(|e| *e.downcast_ref::<wasmtime::Trap>().unwrap());
    (result, store.into_data())
}

#[test]
fn scalar_fields_preserve_constructor_and_mutation_effects() {
    let before = gc_module(
        &[StorageType::Val(ValType::I64); 2],
        &[
            I::I64Const(17),
            I::Call(0),
            I::I64Const(23),
            I::Call(0),
            I::StructNew(0),
            I::LocalTee(1),
            get(0),
            I::Call(0),
            I::Drop,
            I::LocalGet(1),
            I::RefAsNonNull,
            I::LocalGet(1),
            get(0),
            I::I64Const(42),
            I::Call(0),
            I::I64Add,
            set(0),
            I::LocalGet(1),
            get(0),
            I::LocalGet(1),
            get(1),
            I::I64Add,
            I::End,
        ],
    );
    let after = cleanup(&before);
    assert_eq!(allocations(&after), 0);
    assert!(after.len() < before.len());
    for wasm in [&before, &after] {
        assert_eq!(run(wasm, 0), (Ok(82), vec![17, 23, 17, 42]));
    }
}

#[test]
fn scalar_defaults_are_reset_on_every_loop_iteration() {
    let before = gc_module(
        &[StorageType::Val(ValType::I64); 2],
        &[
            I::Loop(BlockType::Empty),
            I::StructNewDefault(0),
            I::LocalSet(1),
            I::LocalGet(3),
            I::LocalGet(1),
            get(1),
            I::I64Add,
            I::LocalSet(3),
            I::LocalGet(1),
            I::LocalGet(2),
            I::I64Const(1),
            I::I64Add,
            I::Call(0),
            set(0),
            I::LocalGet(1),
            I::I64Const(99),
            set(1),
            I::LocalGet(3),
            I::LocalGet(1),
            get(0),
            I::LocalGet(1),
            get(0),
            I::I64Add,
            I::I64Add,
            I::LocalSet(3),
            I::LocalGet(2),
            I::I64Const(1),
            I::I64Add,
            I::LocalTee(2),
            I::I64Const(3),
            I::I64LtU,
            I::BrIf(0),
            I::End,
            I::LocalGet(3),
            I::End,
        ],
    );
    let after = cleanup(&before);
    assert_eq!(allocations(&after), 0);
    for wasm in [&before, &after] {
        assert_eq!(run(wasm, 0), (Ok(12), vec![1, 2, 3]));
    }
}

#[test]
fn scalar_projection_keeps_unused_field_effects_and_traps() {
    for projected in 0..2 {
        let before = gc_module(
            &[StorageType::Val(ValType::I64); 2],
            &[
                I::I64Const(17),
                I::Call(0),
                I::I64Const(42),
                I::Call(0),
                I::LocalGet(0),
                I::I64ExtendI32U,
                I::I64DivU,
                I::StructNew(0),
                get(projected),
                I::End,
            ],
        );
        let after = cleanup(&before);
        assert_eq!(allocations(&after), 0);
        for wasm in [&before, &after] {
            assert_eq!(
                run(wasm, 2),
                (Ok(if projected == 0 { 17 } else { 21 }), vec![17, 42])
            );
            assert_eq!(
                run(wasm, 0),
                (Err(wasmtime::Trap::IntegerDivisionByZero), vec![17, 42])
            );
        }
    }
}

#[test]
fn scalar_replacement_rejects_identity_and_default_null_reads() {
    let before = gc_module(
        &[StorageType::Val(ValType::I64)],
        &[
            I::LocalGet(0),
            I::If(BlockType::Empty),
            I::I64Const(17),
            I::StructNew(0),
            I::LocalSet(1),
            I::End,
            I::LocalGet(1),
            get(0),
            I::End,
        ],
    );
    let after = cleanup(&before);
    assert_eq!(allocations(&after), 1);
    for wasm in [&before, &after] {
        assert_eq!(run(wasm, 0), (Err(wasmtime::Trap::NullReference), vec![]));
        assert_eq!(run(wasm, 1), (Ok(17), vec![]));
    }
    let before = gc_module(
        &[StorageType::Val(ValType::I64)],
        &[
            I::I64Const(17),
            I::StructNew(0),
            I::LocalSet(1),
            I::LocalGet(1),
            I::LocalGet(1),
            I::RefEq,
            I::I64ExtendI32U,
            I::End,
        ],
    );
    let after = cleanup(&before);
    assert_eq!(allocations(&after), 1);
    for wasm in [&before, &after] {
        assert_eq!(run(wasm, 0), (Ok(1), vec![]));
    }
}

#[test]
fn scalar_replacement_keeps_packed_field_truncation() {
    let before = gc_module(
        &[StorageType::I8],
        &[
            I::I32Const(255),
            I::StructNew(0),
            I::LocalSet(1),
            I::LocalGet(1),
            I::StructGetS {
                struct_type_index: 0,
                field_index: 0,
            },
            I::I64ExtendI32S,
            I::End,
        ],
    );
    let after = cleanup(&before);
    assert_eq!(allocations(&after), 1);
    for wasm in [&before, &after] {
        assert_eq!(run(wasm, 0), (Ok(-1), vec![]));
    }
}

fn numeric(ops: &[I<'_>]) -> Vec<u8> {
    let mut types = TypeSection::new();
    types.ty().function([ValType::I32], [ValType::I64]);
    types.ty().function([ValType::I64], [ValType::I64]);
    module(types, 0, &[(2, ValType::I64)], ops, &[("effect", 1)])
}

#[test]
fn common_branch_tails_keep_arm_effects_and_conditional_traps() {
    for result in [false, true] {
        let mut ops = vec![
            I::LocalGet(0),
            I::If(if result {
                BlockType::Result(ValType::I64)
            } else {
                BlockType::Empty
            }),
        ];
        for (i, value) in [17, 23].into_iter().enumerate() {
            if i == 1 {
                ops.push(I::Else);
            }
            ops.extend([
                I::I64Const(value),
                I::Call(0),
                I::Drop,
                I::I64Const(42),
                I::Call(0),
                I::LocalGet(0),
                I::I64ExtendI32U,
                I::I64DivU,
            ]);
            if !result {
                ops.push(I::LocalSet(1));
            }
        }
        ops.push(I::End);
        if !result {
            ops.push(I::LocalGet(1));
        }
        ops.push(I::End);
        let before = numeric(&ops);
        let after = cleanup(&before);
        assert!(after.len() < before.len());
        for wasm in [&before, &after] {
            assert_eq!(run(wasm, 2), (Ok(21), vec![17, 42]));
            assert_eq!(
                run(wasm, 0),
                (Err(wasmtime::Trap::IntegerDivisionByZero), vec![23, 42])
            );
        }
    }
}

#[test]
fn common_branch_tails_do_not_run_on_branches_to_the_join() {
    let before = numeric(&[
        I::LocalGet(0),
        I::If(BlockType::Empty),
        I::LocalGet(0),
        I::BrIf(0),
        I::I64Const(42),
        I::Call(0),
        I::Drop,
        I::Else,
        I::I64Const(42),
        I::Call(0),
        I::Drop,
        I::End,
        I::I64Const(7),
        I::End,
    ]);
    let after = cleanup(&before);
    for wasm in [&before, &after] {
        assert_eq!(run(wasm, 1), (Ok(7), vec![]));
        assert_eq!(run(wasm, 0), (Ok(7), vec![42]));
    }
}

#[test]
fn stack_forwarding_preserves_multiple_lifetimes_later_reads_and_trap_order() {
    let before = numeric(&[
        I::I64Const(17),
        I::Call(0),
        I::LocalSet(1),
        I::I64Const(23),
        I::Call(0),
        I::Drop,
        I::LocalGet(1),
        I::Call(0),
        I::Drop,
        I::LocalGet(0),
        I::If(BlockType::Empty),
        I::LocalGet(1),
        I::Call(0),
        I::Drop,
        I::End,
        I::I64Const(42),
        I::Call(0),
        I::LocalSet(1),
        I::I64Const(7),
        I::LocalGet(0),
        I::I64ExtendI32U,
        I::I64DivU,
        I::Drop,
        I::LocalGet(1),
        I::Call(0),
        I::End,
    ]);
    let after = cleanup(&before);
    assert!(after.len() < before.len());
    for wasm in [&before, &after] {
        assert_eq!(run(wasm, 1), (Ok(42), vec![17, 23, 17, 17, 42, 42]));
        assert_eq!(
            run(wasm, 0),
            (
                Err(wasmtime::Trap::IntegerDivisionByZero),
                vec![17, 23, 17, 42]
            )
        );
    }
}

#[test]
fn common_branch_tails_preserve_nondefaultable_local_initialization() {
    let mut types = TypeSection::new();
    types.ty().struct_([FieldType {
        element_type: StorageType::Val(ValType::I64),
        mutable: false,
    }]);
    types.ty().function([ValType::I32], [ValType::I64]);
    let before = module(
        types,
        1,
        &[(
            1,
            ValType::Ref(RefType {
                nullable: false,
                heap_type: HeapType::Concrete(0),
            }),
        )],
        &[
            I::LocalGet(0),
            I::If(BlockType::Result(ValType::I64)),
            I::I64Const(17),
            I::StructNew(0),
            I::LocalSet(1),
            I::LocalGet(1),
            get(0),
            I::Else,
            I::I64Const(23),
            I::StructNew(0),
            I::LocalSet(1),
            I::LocalGet(1),
            get(0),
            I::End,
            I::End,
        ],
        &[],
    );
    let after = optimize(
        &before,
        Passes {
            control: true,
            ..Default::default()
        },
    );
    for wasm in [&before, &after] {
        assert_eq!(run(wasm, 1), (Ok(17), vec![]));
        assert_eq!(run(wasm, 0), (Ok(23), vec![]));
    }
}
