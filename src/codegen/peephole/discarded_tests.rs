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
            locals: true,
            discarded_values: true,
            ..Default::default()
        },
    )
}

fn numeric(ops: &[I<'_>]) -> Vec<u8> {
    let mut types = TypeSection::new();
    types.ty().function([], [ValType::I64]);
    types.ty().function([ValType::I64], [ValType::I64]);
    module(types, 0, &[(1, ValType::I64)], ops, &[("effect", 1)])
}

fn run(wasm: &[u8]) -> (Result<i64, wasmtime::Trap>, Vec<i64>) {
    let mut config = wasmtime::Config::new();
    config.wasm_gc(true).wasm_function_references(true);
    let engine = wasmtime::Engine::new(&config).unwrap();
    let (mut store, instance) = instantiate(&engine, wasm);
    let value = instance
        .get_typed_func::<(), i64>(&mut store, "run")
        .unwrap()
        .call(&mut store, ())
        .map_err(|error| *error.downcast_ref::<wasmtime::Trap>().unwrap());
    (value, store.into_data())
}

#[test]
fn discarded_calculations_keep_operand_calls_and_eager_select_order() {
    for condition in [0, 1, -7] {
        let before = numeric(&[
            I::I64Const(1),
            I::Call(0),
            I::I64Const(2),
            I::Call(0),
            I::I64Add,
            I::I64Const(3),
            I::Call(0),
            I::I32Const(condition),
            I::Select,
            I::Drop,
            I::I64Const(7),
            I::End,
        ]);
        let after = cleanup(&before);
        assert!(after.len() < before.len());
        for wasm in [&before, &after] {
            assert_eq!(run(wasm), (Ok(7), vec![1, 2, 3]));
        }
    }
}

#[test]
fn discarded_calculations_keep_assignments_and_stack_values_across_effects() {
    let before = numeric(&[
        I::I64Const(13),
        I::LocalTee(0),
        I::I64Const(2),
        I::Call(0),
        I::Drop,
        I::I64Const(3),
        I::I64Mul,
        I::Drop,
        I::LocalGet(0),
        I::End,
    ]);
    let after = cleanup(&before);
    assert!(after.len() < before.len());
    for wasm in [&before, &after] {
        assert_eq!(run(wasm), (Ok(13), vec![2]));
    }
}

#[test]
fn discarded_float_calculations_can_remove_nan_and_infinity_results() {
    let before = numeric(&[
        I::F64Const((-1.0).into()),
        I::F64Sqrt,
        I::F64Const(0.0.into()),
        I::F64Div,
        I::I64TruncSatF64S,
        I::Drop,
        I::F32Const(f32::INFINITY.into()),
        I::F32Const(f32::NEG_INFINITY.into()),
        I::F32Add,
        I::F32Nearest,
        I::Drop,
        I::I64Const(7),
        I::End,
    ]);
    let after = cleanup(&before);
    assert!(after.len() + 20 < before.len());
    assert_eq!(run(&before), run(&after));
}

#[test]
fn discarded_calculations_preserve_first_trap_and_later_calls() {
    for (mut producer, trap) in [
        (
            vec![I::I64Const(1), I::I64Const(0), I::I64DivS],
            wasmtime::Trap::IntegerDivisionByZero,
        ),
        (
            vec![I::F64Const(f64::NAN.into()), I::I64TruncF64S],
            wasmtime::Trap::BadConversionToInteger,
        ),
    ] {
        let mut ops = vec![I::I64Const(1), I::Call(0), I::Drop];
        ops.append(&mut producer);
        ops.extend([
            I::I64Const(2),
            I::Call(0),
            I::I64Mul,
            I::Drop,
            I::I64Const(7),
            I::End,
        ]);
        let before = numeric(&ops);
        let after = cleanup(&before);
        assert!(after.len() < before.len());
        for wasm in [&before, &after] {
            assert_eq!(run(wasm), (Err(trap), vec![1]));
        }
    }
}

#[test]
fn discarded_calculations_preserve_gc_null_traps() {
    let mut types = TypeSection::new();
    types.ty().struct_([FieldType {
        element_type: StorageType::Val(ValType::I64),
        mutable: false,
    }]);
    types.ty().function([], [ValType::I64]);
    let before = module(
        types,
        1,
        &[],
        &[
            I::RefNull(HeapType::Concrete(0)),
            I::StructGet {
                struct_type_index: 0,
                field_index: 0,
            },
            I::I64Const(3),
            I::I64Add,
            I::Drop,
            I::I64Const(7),
            I::End,
        ],
        &[],
    );
    let after = cleanup(&before);
    assert!(after.len() < before.len());
    for wasm in [&before, &after] {
        assert_eq!(run(wasm), (Err(wasmtime::Trap::NullReference), vec![]));
    }
}

#[test]
fn discarded_values_respect_control_and_multiple_result_boundaries() {
    let mut types = TypeSection::new();
    types.ty().function([], [ValType::I64]);
    types.ty().function([], [ValType::I64, ValType::I64]);
    let before = module(
        types,
        0,
        &[],
        &[
            I::I64Const(5),
            I::Block(BlockType::FunctionType(1)),
            I::I64Const(2),
            I::I64Const(3),
            I::End,
            I::I64Add,
            I::Drop,
            I::I64Const(6),
            I::I64Mul,
            I::Drop,
            I::I64Const(7),
            I::End,
        ],
        &[],
    );
    let after = cleanup(&before);
    assert_eq!(run(&before), (Ok(7), vec![]));
    assert_eq!(run(&before), run(&after));
}

#[test]
fn late_liveness_preserves_structural_reference_initializers() {
    for condition in [0, 1] {
        for first_read in [false, true] {
            let mut types = TypeSection::new();
            types.ty().struct_([FieldType {
                element_type: StorageType::Val(ValType::I64),
                mutable: false,
            }]);
            types.ty().function([], [ValType::I64]);
            let mut ops = vec![I::I64Const(7), I::StructNew(0), I::LocalSet(0)];
            if first_read {
                ops.extend([
                    I::LocalGet(0),
                    I::StructGet {
                        struct_type_index: 0,
                        field_index: 0,
                    },
                    I::Drop,
                ]);
            }
            ops.extend([
                I::I32Const(condition),
                I::If(BlockType::Empty),
                I::I64Const(17),
                I::StructNew(0),
                I::LocalSet(0),
                I::Else,
                I::I64Const(23),
                I::StructNew(0),
                I::LocalSet(0),
                I::End,
                I::LocalGet(0),
                I::StructGet {
                    struct_type_index: 0,
                    field_index: 0,
                },
                I::End,
            ]);
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
                &ops,
                &[],
            );
            let after = optimize(
                &before,
                Passes {
                    locals: true,
                    flow_locals: true,
                    ..Default::default()
                },
            );
            let expected = (Ok(if condition == 0 { 23 } else { 17 }), vec![]);
            assert_eq!(run(&before), expected);
            assert_eq!(run(&after), expected);
        }
    }
}
