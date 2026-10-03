use super::{
    I, Passes,
    instruction_tests::{instantiate, module},
    optimize,
};
use wasm_encoder::{BlockType, TypeSection, ValType};

fn cleanup(wasm: &[u8], locals: bool) -> Vec<u8> {
    optimize(
        wasm,
        Passes {
            instructions: true,
            movements: true,
            control: !locals,
            locals,
            flow_locals: locals,
            ..Default::default()
        },
    )
}

fn count_selects(wasm: &[u8]) -> usize {
    wasmparser::Parser::new(0)
        .parse_all(wasm)
        .filter_map(|p| match p.unwrap() {
            wasmparser::Payload::CodeSectionEntry(body) => Some(
                body.get_operators_reader()
                    .unwrap()
                    .into_iter()
                    .filter(|op| matches!(op, Ok(wasmparser::Operator::Select)))
                    .count(),
            ),
            _ => None,
        })
        .sum()
}

#[test]
fn structured_conditions_preserve_writes_and_operand_snapshots() {
    let engine = wasmtime::Engine::default();
    for read_written_local in [false, true] {
        let mut types = TypeSection::new();
        types.ty().function([ValType::I32], [ValType::I64]);
        let mut ops = vec![
            I::LocalGet(0),
            I::If(BlockType::Result(ValType::I32)),
            I::I32Const(3),
            I::LocalTee(1),
            I::Else,
            I::I32Const(0),
            I::End,
            I::If(BlockType::Result(ValType::I64)),
        ];
        if read_written_local {
            ops.extend([I::LocalGet(1), I::I64ExtendI32U]);
        } else {
            ops.push(I::I64Const(17));
        }
        ops.extend([I::Else, I::I64Const(23), I::End, I::End]);
        let before = module(types, 0, &[(1, ValType::I32)], &ops, &[]);
        let after = cleanup(&before, false);
        assert_eq!(count_selects(&after), usize::from(!read_written_local));
        for wasm in [&before, &after] {
            for input in [0, 1, -7] {
                let (mut store, instance) = instantiate(&engine, wasm);
                let value = instance
                    .get_typed_func::<i32, i64>(&mut store, "run")
                    .unwrap()
                    .call(&mut store, input)
                    .unwrap();
                assert_eq!(
                    value,
                    if input == 0 {
                        23
                    } else if read_written_local {
                        3
                    } else {
                        17
                    }
                );
            }
        }
    }
}

#[test]
fn structured_conditions_keep_calls_and_escaping_branches() {
    let engine = wasmtime::Engine::default();
    for escape in [false, true] {
        let mut types = TypeSection::new();
        types.ty().function([ValType::I32], [ValType::I64]);
        types.ty().function([ValType::I64], [ValType::I64]);
        let mut ops = vec![
            I::LocalGet(0),
            I::If(BlockType::Result(ValType::I32)),
            I::I64Const(11),
            I::Call(0),
        ];
        if escape {
            ops.push(I::Return);
        } else {
            ops.push(I::I32WrapI64);
        }
        ops.extend([
            I::Else,
            I::I32Const(0),
            I::End,
            I::If(BlockType::Result(ValType::I64)),
            I::I64Const(17),
            I::Else,
            I::I64Const(23),
            I::End,
            I::End,
        ]);
        let before = module(types, 0, &[], &ops, &[("effect", 1)]);
        let after = cleanup(&before, false);
        assert_eq!(count_selects(&after), 0);
        for wasm in [&before, &after] {
            for input in [0, 1] {
                let (mut store, instance) = instantiate(&engine, wasm);
                let value = instance
                    .get_typed_func::<i32, i64>(&mut store, "run")
                    .unwrap()
                    .call(&mut store, input)
                    .unwrap();
                assert_eq!(
                    value,
                    if input == 0 {
                        23
                    } else if escape {
                        11
                    } else {
                        17
                    }
                );
                assert_eq!(
                    store.data().as_slice(),
                    if input == 0 { &[][..] } else { &[11][..] }
                );
            }
        }
    }
}

#[test]
fn float_selections_preserve_nan_infinity_and_signed_zero() {
    let mut types = TypeSection::new();
    types.ty().function([ValType::F64], [ValType::I64]);
    let before = module(
        types,
        0,
        &[],
        &[
            I::LocalGet(0),
            I::F64Const(0.0.into()),
            I::F64Lt,
            I::If(BlockType::Result(ValType::F64)),
            I::LocalGet(0),
            I::F64Neg,
            I::Else,
            I::F64Const((-0.0).into()),
            I::End,
            I::I64ReinterpretF64,
            I::End,
        ],
        &[],
    );
    let after = cleanup(&before, false);
    assert!(after.len() < before.len());
    assert_eq!(count_selects(&after), 1);
    let engine = wasmtime::Engine::default();
    for wasm in [&before, &after] {
        for input in [f64::NAN, f64::INFINITY, f64::NEG_INFINITY, -0.0, 0.0, -2.5] {
            let (mut store, instance) = instantiate(&engine, wasm);
            let value = instance
                .get_typed_func::<f64, i64>(&mut store, "run")
                .unwrap()
                .call(&mut store, input)
                .unwrap();
            assert_eq!(
                value as u64,
                (if input < 0.0 { -input } else { -0.0f64 }).to_bits()
            );
        }
    }
}

fn local_module(ops: &[I<'_>]) -> Vec<u8> {
    let mut types = TypeSection::new();
    types.ty().function([ValType::I64], [ValType::I64]);
    module(types, 0, &[(1, ValType::I64)], ops, &[("effect", 0)])
}

fn run(wasm: &[u8], input: i64) -> (i64, Vec<i64>) {
    let engine = wasmtime::Engine::default();
    let (mut store, instance) = instantiate(&engine, wasm);
    let value = instance
        .get_typed_func::<i64, i64>(&mut store, "run")
        .unwrap()
        .call(&mut store, input)
        .unwrap();
    (value, store.into_data())
}

#[test]
fn local_sinking_crosses_independent_arithmetic_and_effects() {
    for producer_calls in [false, true] {
        let mut ops = if producer_calls {
            vec![
                I::LocalGet(0),
                I::Call(0),
                I::LocalSet(1),
                I::LocalGet(0),
                I::I64Const(2),
                I::I64Add,
            ]
        } else {
            vec![
                I::LocalGet(0),
                I::I64Const(3),
                I::I64Mul,
                I::LocalSet(1),
                I::I64Const(7),
                I::Call(0),
            ]
        };
        ops.extend([I::LocalGet(1), I::I64Add, I::LocalGet(1), I::I64Add, I::End]);
        let before = local_module(&ops);
        let after = cleanup(&before, true);
        assert!(after.len() < before.len());
        for input in [-7, 0, 2] {
            let expected = if producer_calls {
                (input * 3 + 2, vec![input])
            } else {
                (input * 6 + 7, vec![7])
            };
            assert_eq!(run(&before, input), expected);
            assert_eq!(run(&after, input), expected);
        }
    }
}

#[test]
fn local_sinking_keeps_dependencies_in_both_directions_and_call_order() {
    for (ops, expected) in [
        (
            vec![
                I::LocalGet(0),
                I::I64Const(3),
                I::I64Mul,
                I::LocalSet(1),
                I::I64Const(9),
                I::LocalSet(0),
                I::I64Const(7),
                I::Call(0),
                I::LocalGet(1),
                I::I64Add,
                I::LocalGet(1),
                I::I64Add,
                I::End,
            ],
            (19, vec![7]),
        ),
        (
            vec![
                I::I64Const(3),
                I::LocalTee(0),
                I::I64Const(2),
                I::I64Mul,
                I::LocalSet(1),
                I::LocalGet(0),
                I::I64Const(1),
                I::I64Add,
                I::LocalGet(1),
                I::I64Add,
                I::LocalGet(1),
                I::I64Add,
                I::End,
            ],
            (16, vec![]),
        ),
        (
            vec![
                I::I64Const(5),
                I::Call(0),
                I::LocalSet(1),
                I::I64Const(7),
                I::Call(0),
                I::LocalGet(1),
                I::I64Add,
                I::LocalGet(1),
                I::I64Add,
                I::End,
            ],
            (17, vec![5, 7]),
        ),
    ] {
        let before = local_module(&ops);
        let after = cleanup(&before, true);
        assert_eq!(run(&before, 2), expected);
        assert_eq!(run(&after, 2), expected);
    }
}

#[test]
fn local_sinking_keeps_null_traps_before_later_effects() {
    let mut types = TypeSection::new();
    types.ty().function([ValType::FUNCREF], [ValType::I64]);
    types.ty().function([ValType::I64], [ValType::I64]);
    let before = module(
        types,
        0,
        &[(1, ValType::I64)],
        &[
            I::LocalGet(0),
            I::RefAsNonNull,
            I::RefIsNull,
            I::I64ExtendI32U,
            I::LocalSet(1),
            I::I64Const(7),
            I::Call(0),
            I::LocalGet(1),
            I::I64Add,
            I::LocalGet(1),
            I::I64Add,
            I::End,
        ],
        &[("effect", 1)],
    );
    let after = cleanup(&before, true);
    let mut config = wasmtime::Config::new();
    config.wasm_function_references(true);
    let engine = wasmtime::Engine::new(&config).unwrap();
    for wasm in [&before, &after] {
        let (mut store, instance) = instantiate(&engine, wasm);
        let error = instance
            .get_typed_func::<Option<wasmtime::Func>, i64>(&mut store, "run")
            .unwrap()
            .call(&mut store, None)
            .unwrap_err();
        assert_eq!(
            error.downcast_ref::<wasmtime::Trap>(),
            Some(&wasmtime::Trap::NullReference)
        );
        assert!(store.data().is_empty());
    }
}

fn state_module(ops: &[I<'_>]) -> Vec<u8> {
    use wasm_encoder::{
        CodeSection, ConstExpr, ExportKind, ExportSection, Function, FunctionSection,
        GlobalSection, GlobalType, MemorySection, MemoryType, Module,
    };
    let mut types = TypeSection::new();
    types.ty().function([], [ValType::I64]);
    let mut functions = FunctionSection::new();
    functions.function(0).function(0);
    let mut memories = MemorySection::new();
    memories.memory(MemoryType {
        minimum: 1,
        maximum: Some(1),
        memory64: false,
        shared: false,
        page_size_log2: None,
    });
    let mut globals = GlobalSection::new();
    globals.global(
        GlobalType {
            val_type: ValType::I64,
            mutable: true,
            shared: false,
        },
        &ConstExpr::i64_const(5),
    );
    let mut exports = ExportSection::new();
    exports.export("run", ExportKind::Func, 1);
    let mut code = CodeSection::new();
    let mut mutate = Function::new([]);
    for op in [I::I64Const(9), I::GlobalSet(0), I::I64Const(3), I::End] {
        mutate.instruction(&op);
    }
    code.function(&mutate);
    let mut body = Function::new([(1, ValType::I64)]);
    for op in ops {
        body.instruction(op);
    }
    code.function(&body);
    let mut module = Module::new();
    module
        .section(&types)
        .section(&functions)
        .section(&memories)
        .section(&globals)
        .section(&exports)
        .section(&code);
    module.finish()
}

fn state_run(wasm: &[u8]) -> Result<i64, wasmtime::Trap> {
    let engine = wasmtime::Engine::default();
    let (mut store, instance) = instantiate(&engine, wasm);
    instance
        .get_typed_func::<(), i64>(&mut store, "run")
        .unwrap()
        .call(&mut store, ())
        .map_err(|error| *error.downcast_ref::<wasmtime::Trap>().unwrap())
}

#[test]
fn local_sinking_and_conditions_keep_global_mutations() {
    for (ops, locals, expected) in [
        (
            vec![
                I::GlobalGet(0),
                I::LocalSet(0),
                I::I64Const(9),
                I::GlobalSet(0),
                I::I64Const(2),
                I::LocalGet(0),
                I::I64Add,
                I::End,
            ],
            true,
            7,
        ),
        (
            vec![
                I::Call(0),
                I::LocalSet(0),
                I::GlobalGet(0),
                I::I64Const(2),
                I::I64Add,
                I::LocalGet(0),
                I::I64Add,
                I::End,
            ],
            true,
            14,
        ),
        (
            vec![
                I::GlobalGet(0),
                I::I32WrapI64,
                I::If(BlockType::Result(ValType::I32)),
                I::I64Const(9),
                I::GlobalSet(0),
                I::I32Const(1),
                I::Else,
                I::I32Const(0),
                I::End,
                I::If(BlockType::Result(ValType::I64)),
                I::GlobalGet(0),
                I::Else,
                I::I64Const(23),
                I::End,
                I::End,
            ],
            false,
            9,
        ),
    ] {
        let before = state_module(&ops);
        let after = cleanup(&before, locals);
        assert_eq!(state_run(&before), Ok(expected));
        assert_eq!(state_run(&after), Ok(expected));
    }
}

#[test]
fn local_sinking_crosses_nontrapping_state_reads_without_losing_load_traps() {
    for address in [0, 65536] {
        let before = state_module(&[
            I::I32Const(address),
            I::I64Load(wasm_encoder::MemArg {
                offset: 0,
                align: 3,
                memory_index: 0,
            }),
            I::LocalSet(0),
            I::GlobalGet(0),
            I::I64Const(2),
            I::I64Add,
            I::LocalGet(0),
            I::I64Add,
            I::LocalGet(0),
            I::I64Add,
            I::End,
        ]);
        let after = cleanup(&before, true);
        assert!(after.len() < before.len());
        let expected = if address == 0 {
            Ok(7)
        } else {
            Err(wasmtime::Trap::MemoryOutOfBounds)
        };
        assert_eq!(state_run(&before), expected);
        assert_eq!(state_run(&after), expected);
    }
}
