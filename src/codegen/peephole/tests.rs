use super::{Passes, optimize};
use wasm_encoder::{
    BlockType, CodeSection, ExportKind, ExportSection, Function, FunctionSection, Instruction as I,
    Module, TypeSection, ValType,
};

fn module(ops: &[I<'_>]) -> Vec<u8> {
    module_with_locals(&[], &[(1, ValType::I64)], ops)
}

pub(super) fn module_with_locals(
    params: &[ValType],
    locals: &[(u32, ValType)],
    ops: &[I<'_>],
) -> Vec<u8> {
    let mut types = TypeSection::new();
    types.ty().function(params.iter().copied(), [ValType::I64]);
    let mut functions = FunctionSection::new();
    functions.function(0);
    let mut exports = ExportSection::new();
    exports.export("run", ExportKind::Func, 0);
    let mut body = Function::new(locals.iter().copied());
    for op in ops {
        body.instruction(op);
    }
    let mut code = CodeSection::new();
    code.function(&body);
    let mut module = Module::new();
    module
        .section(&types)
        .section(&functions)
        .section(&exports)
        .section(&code);
    module.finish()
}

#[test]
fn unused_locals_preserve_tee_values_and_trapping_producers() {
    let engine = wasmtime::Engine::default();
    for (ops, expected) in [
        (vec![I::I64Const(42), I::LocalTee(0), I::End], Ok(42)),
        (
            vec![
                I::I64Const(1),
                I::I64Const(0),
                I::I64DivU,
                I::LocalSet(0),
                I::I64Const(7),
                I::End,
            ],
            Err(()),
        ),
        (
            vec![
                I::I64Const(42),
                I::LocalSet(0),
                I::I64Const(9),
                I::Return,
                I::LocalGet(0),
                I::Drop,
                I::End,
            ],
            Ok(9),
        ),
    ] {
        let baseline = module(&ops);
        let optimized = optimize(
            &baseline,
            Passes {
                locals: true,
                dead_code: true,
                instructions: true,
                ..Default::default()
            },
        );
        assert!(optimized.len() < baseline.len());
        assert_eq!(execute(&engine, &baseline), expected);
        assert_eq!(execute(&engine, &optimized), expected);
        for payload in wasmparser::Parser::new(0).parse_all(&optimized) {
            if let wasmparser::Payload::CodeSectionEntry(body) = payload.unwrap() {
                assert_eq!(body.get_locals_reader().unwrap().get_count(), 0);
            }
        }
    }
}

#[test]
fn local_compaction_preserves_parameters_loop_state_and_large_indices() {
    let baseline = module_with_locals(
        &[ValType::I64, ValType::I32],
        &[(128, ValType::I64)],
        &[
            I::LocalGet(0),
            I::LocalSet(129),
            I::I64Const(0),
            I::LocalSet(2),
            I::Loop(BlockType::Empty),
            I::LocalGet(2),
            I::I64Const(1),
            I::I64Add,
            I::LocalTee(2),
            I::I64Const(3),
            I::I64LtS,
            I::BrIf(0),
            I::End,
            I::LocalGet(129),
            I::LocalGet(2),
            I::I64Add,
            I::End,
        ],
    );
    let optimized = optimize(
        &baseline,
        Passes {
            locals: true,
            ..Default::default()
        },
    );
    assert!(optimized.len() < baseline.len());
    let engine = wasmtime::Engine::default();
    for wasm in [&baseline, &optimized] {
        wasmparser::Validator::new_with_features(wasmparser::WasmFeatures::all())
            .validate_all(wasm)
            .unwrap();
        let module = wasmtime::Module::new(&engine, wasm).unwrap();
        let mut store = wasmtime::Store::new(&engine, ());
        let instance = wasmtime::Instance::new(&mut store, &module, &[]).unwrap();
        assert_eq!(
            instance
                .get_typed_func::<(i64, i32), i64>(&mut store, "run")
                .unwrap()
                .call(&mut store, (39, 7))
                .unwrap(),
            42
        );
    }
}

pub(super) fn execute(engine: &wasmtime::Engine, wasm: &[u8]) -> Result<i64, ()> {
    wasmparser::Validator::new_with_features(wasmparser::WasmFeatures::all())
        .validate_all(wasm)
        .unwrap();
    let module = wasmtime::Module::new(engine, wasm).unwrap();
    let mut store = wasmtime::Store::new(engine, ());
    let instance = wasmtime::Instance::new(&mut store, &module, &[]).unwrap();
    instance
        .get_typed_func::<(), i64>(&mut store, "run")
        .unwrap()
        .call(&mut store, ())
        .map_err(|_| ())
}

#[test]
fn integer_folding_preserves_wasm_overflow_shifts_and_traps() {
    let engine = wasmtime::Engine::default();
    for (ops, expected) in [
        (
            vec![
                I::I32Const(i32::MAX),
                I::I32Const(1),
                I::I32Add,
                I::I64ExtendI32S,
                I::End,
            ],
            Ok(i32::MIN as i64),
        ),
        (
            vec![
                I::I32Const(1),
                I::I32Const(65),
                I::I32Shl,
                I::I64ExtendI32S,
                I::End,
            ],
            Ok(2),
        ),
        (
            vec![
                I::I32Const(-1),
                I::I32Const(31),
                I::I32ShrU,
                I::I64ExtendI32S,
                I::End,
            ],
            Ok(1),
        ),
        (
            vec![
                I::I32Const(i32::MIN),
                I::I32Const(-1),
                I::I32DivS,
                I::I64ExtendI32S,
                I::End,
            ],
            Err(()),
        ),
        (
            vec![I::I64Const(i64::MAX), I::I64Const(1), I::I64Add, I::End],
            Ok(i64::MIN),
        ),
        (
            vec![I::I64Const(1), I::I64Const(129), I::I64Shl, I::End],
            Ok(2),
        ),
        (
            vec![I::I64Const(12), I::I64Const(0), I::I64DivU, I::End],
            Err(()),
        ),
        (
            vec![I::I64Const(i64::MIN), I::I64Const(-1), I::I64DivS, I::End],
            Err(()),
        ),
    ] {
        let baseline = module(&ops);
        let optimized = optimize(
            &baseline,
            Passes {
                constants: true,
                instructions: true,
                ..Default::default()
            },
        );
        assert!(optimized.len() <= baseline.len());
        assert_eq!(execute(&engine, &baseline), expected);
        assert_eq!(execute(&engine, &optimized), expected);
    }
}

#[test]
fn dead_code_keeps_enclosing_labels_and_else_paths() {
    let engine = wasmtime::Engine::default();
    for (ops, expected) in [
        (
            vec![
                I::Block(BlockType::Result(ValType::I64)),
                I::I64Const(7),
                I::Br(0),
                I::Block(BlockType::Empty),
                I::Unreachable,
                I::End,
                I::I64Const(999),
                I::End,
                I::End,
            ],
            7,
        ),
        (
            vec![
                I::I32Const(0),
                I::If(BlockType::Result(ValType::I64)),
                I::I64Const(7),
                I::Return,
                I::I64Const(999),
                I::Else,
                I::I64Const(9),
                I::End,
                I::End,
            ],
            9,
        ),
    ] {
        let baseline = module(&ops);
        let optimized = optimize(
            &baseline,
            Passes {
                dead_code: true,
                ..Default::default()
            },
        );
        assert!(optimized.len() < baseline.len());
        assert_eq!(execute(&engine, &baseline), Ok(expected));
        assert_eq!(execute(&engine, &optimized), Ok(expected));
    }
}

#[test]
fn local_cleanup_preserves_values_and_does_not_drop_trapping_producers() {
    let engine = wasmtime::Engine::default();
    for (ops, expected) in [
        (
            vec![
                I::I64Const(42),
                I::LocalSet(0),
                I::LocalGet(0),
                I::I64Const(0),
                I::I64Add,
                I::End,
            ],
            Ok(42),
        ),
        (
            vec![
                I::I64Const(42),
                I::I64Const(0),
                I::I64DivU,
                I::Drop,
                I::I64Const(7),
                I::End,
            ],
            Err(()),
        ),
    ] {
        let baseline = module(&ops);
        let optimized = optimize(
            &baseline,
            Passes {
                instructions: true,
                ..Default::default()
            },
        );
        assert_eq!(execute(&engine, &baseline), expected);
        assert_eq!(execute(&engine, &optimized), expected);
    }
}

#[test]
fn floating_point_payloads_are_not_folded() {
    for bits in [(-0.0_f64).to_bits(), 0x7ff8_0000_0000_0042] {
        let baseline = module(&[
            I::F64Const(f64::from_bits(bits).into()),
            I::I64ReinterpretF64,
            I::End,
        ]);
        let optimized = optimize(
            &baseline,
            Passes {
                constants: true,
                instructions: true,
                ..Default::default()
            },
        );
        assert_eq!(optimized, baseline);
    }
}

#[test]
fn exception_control_boundaries_are_left_untouched() {
    let baseline = module(&[
        I::TryTable(BlockType::Empty, std::borrow::Cow::Borrowed(&[])),
        I::Nop,
        I::End,
        I::I64Const(42),
        I::LocalTee(0),
        I::End,
    ]);
    wasmparser::Validator::new_with_features(wasmparser::WasmFeatures::all())
        .validate_all(&baseline)
        .unwrap();
    let optimized = optimize(
        &baseline,
        Passes {
            instructions: true,
            constants: true,
            dead_code: true,
            locals: true,
            control: false,
            returns: false,
        },
    );
    assert_eq!(optimized, baseline);
}

#[test]
fn unused_labels_preserve_values_traps_and_outer_branch_targets() {
    let mut config = wasmtime::Config::new();
    config.wasm_gc(true).wasm_function_references(true);
    let engine = wasmtime::Engine::new(&config).unwrap();
    for (ops, expected) in [
        (
            vec![
                I::Block(BlockType::Result(ValType::I64)),
                I::I64Const(42),
                I::End,
                I::End,
            ],
            Ok(42),
        ),
        // Removing both wrappers must remap a branch to the function label.
        (
            vec![
                I::Block(BlockType::Empty),
                I::Block(BlockType::Empty),
                I::I64Const(42),
                I::Br(2),
                I::End,
                I::End,
                I::I64Const(7),
                I::End,
            ],
            Ok(42),
        ),
        // A branch may discard operands: keep its targeted block intact.
        (
            vec![
                I::Block(BlockType::Empty),
                I::Block(BlockType::Empty),
                I::I64Const(7),
                I::I32Const(1),
                I::BrIf(1),
                I::Drop,
                I::End,
                I::End,
                I::I64Const(42),
                I::End,
            ],
            Ok(42),
        ),
        (
            vec![
                I::Block(BlockType::Empty),
                I::Block(BlockType::Empty),
                I::I32Const(0),
                I::BrTable(vec![1].into(), 1),
                I::End,
                I::End,
                I::I64Const(42),
                I::End,
            ],
            Ok(42),
        ),
        (
            vec![
                I::Block(BlockType::Empty),
                I::Block(BlockType::Empty),
                I::I32Const(17),
                I::BrTable(vec![1].into(), 1),
                I::End,
                I::End,
                I::I64Const(42),
                I::End,
            ],
            Ok(42),
        ),
        // The loop's back edge survives even when its nested wrapper does not.
        (
            vec![
                I::Loop(BlockType::Empty),
                I::Block(BlockType::Empty),
                I::LocalGet(0),
                I::I64Const(1),
                I::I64Add,
                I::LocalTee(0),
                I::I64Const(3),
                I::I64LtS,
                I::BrIf(1),
                I::End,
                I::End,
                I::LocalGet(0),
                I::End,
            ],
            Ok(3),
        ),
        (
            vec![
                I::Block(BlockType::Result(ValType::I64)),
                I::I64Const(1),
                I::I64Const(0),
                I::I64DivU,
                I::End,
                I::End,
            ],
            Err(()),
        ),
        (
            vec![
                I::Block(BlockType::Empty),
                I::Block(BlockType::Empty),
                I::RefNull(wasm_encoder::HeapType::Abstract {
                    shared: false,
                    ty: wasm_encoder::AbstractHeapType::Any,
                }),
                I::BrOnNull(1),
                I::Drop,
                I::End,
                I::End,
                I::I64Const(42),
                I::End,
            ],
            Ok(42),
        ),
    ] {
        let baseline = module(&ops);
        let optimized = optimize(
            &baseline,
            Passes {
                control: true,
                ..Default::default()
            },
        );
        assert!(optimized.len() < baseline.len());
        assert_eq!(execute(&engine, &baseline), expected);
        assert_eq!(execute(&engine, &optimized), expected);
    }
}

#[test]
fn unused_labels_preserve_typed_block_parameters_and_multiple_results() {
    let mut types = TypeSection::new();
    types.ty().function([], [ValType::I64]);
    types
        .ty()
        .function([ValType::I64], [ValType::I64, ValType::I64]);
    let mut functions = FunctionSection::new();
    functions.function(0);
    let mut exports = ExportSection::new();
    exports.export("run", ExportKind::Func, 0);
    let mut body = Function::new([]);
    for op in [
        I::I64Const(17),
        I::Block(BlockType::FunctionType(1)),
        I::I64Const(25),
        I::End,
        I::I64Add,
        I::End,
    ] {
        body.instruction(&op);
    }
    let mut code = CodeSection::new();
    code.function(&body);
    let mut module = Module::new();
    module
        .section(&types)
        .section(&functions)
        .section(&exports)
        .section(&code);
    let baseline = module.finish();
    let optimized = optimize(
        &baseline,
        Passes {
            control: true,
            ..Default::default()
        },
    );
    let engine = wasmtime::Engine::default();
    assert!(optimized.len() < baseline.len());
    assert_eq!(execute(&engine, &baseline), Ok(42));
    assert_eq!(execute(&engine, &optimized), Ok(42));
}

#[test]
fn conditional_jumps_preserve_conditions_and_loop_back_edges() {
    let engine = wasmtime::Engine::default();
    for (ops, expected) in [
        (
            vec![
                I::Block(BlockType::Empty),
                I::I32Const(1),
                I::If(BlockType::Empty),
                I::Br(1),
                I::End,
                I::I64Const(99),
                I::Return,
                I::End,
                I::I64Const(42),
                I::End,
            ],
            42,
        ),
        (
            vec![
                I::Block(BlockType::Empty),
                I::I32Const(0),
                I::If(BlockType::Empty),
                I::Br(1),
                I::End,
                I::I64Const(99),
                I::Return,
                I::End,
                I::I64Const(42),
                I::End,
            ],
            99,
        ),
        (
            vec![
                I::I64Const(42),
                I::LocalTee(0),
                I::I64Eqz,
                I::If(BlockType::Empty),
                I::Br(0),
                I::End,
                I::LocalGet(0),
                I::End,
            ],
            42,
        ),
        (
            vec![
                I::Loop(BlockType::Empty),
                I::LocalGet(0),
                I::I64Const(1),
                I::I64Add,
                I::LocalTee(0),
                I::I64Const(3),
                I::I64LtS,
                I::If(BlockType::Empty),
                I::Br(1),
                I::End,
                I::End,
                I::LocalGet(0),
                I::End,
            ],
            3,
        ),
    ] {
        let baseline = module(&ops);
        let optimized = optimize(
            &baseline,
            Passes {
                control: true,
                ..Default::default()
            },
        );
        assert!(optimized.len() < baseline.len());
        assert_eq!(execute(&engine, &baseline), Ok(expected));
        assert_eq!(execute(&engine, &optimized), Ok(expected));
    }
}

#[test]
fn factoring_if_assignments_keeps_arm_effects_and_early_exits() {
    let engine = wasmtime::Engine::default();
    for condition in [0, 1] {
        let baseline = module(&[
            I::I32Const(condition),
            I::If(BlockType::Empty),
            I::I64Const(17),
            I::LocalSet(0),
            I::Else,
            I::I64Const(25),
            I::LocalSet(0),
            I::End,
            I::LocalGet(0),
            I::End,
        ]);
        let optimized = optimize(
            &baseline,
            Passes {
                control: true,
                ..Default::default()
            },
        );
        assert!(optimized.len() < baseline.len());
        assert_eq!(
            execute(&engine, &baseline),
            Ok(if condition == 0 { 25 } else { 17 })
        );
        assert_eq!(execute(&engine, &optimized), execute(&engine, &baseline));
    }
    // A taken branch to the if label must retain the old local value.
    let baseline = module(&[
        I::I32Const(1),
        I::If(BlockType::Empty),
        I::Br(0),
        I::I64Const(17),
        I::LocalSet(0),
        I::Else,
        I::I64Const(25),
        I::LocalSet(0),
        I::End,
        I::LocalGet(0),
        I::End,
    ]);
    let optimized = optimize(
        &baseline,
        Passes {
            control: true,
            ..Default::default()
        },
    );
    assert_eq!(optimized, baseline);
    assert_eq!(execute(&engine, &optimized), Ok(0));
    // The untaken arm's trap must not move out of the conditional.
    let baseline = module(&[
        I::I32Const(0),
        I::If(BlockType::Empty),
        I::I64Const(1),
        I::I64Const(0),
        I::I64DivU,
        I::LocalSet(0),
        I::Else,
        I::I64Const(42),
        I::LocalSet(0),
        I::End,
        I::LocalGet(0),
        I::End,
    ]);
    let optimized = optimize(
        &baseline,
        Passes {
            control: true,
            ..Default::default()
        },
    );
    assert!(optimized.len() < baseline.len());
    assert_eq!(execute(&engine, &baseline), Ok(42));
    assert_eq!(execute(&engine, &optimized), Ok(42));
}

#[test]
fn selections_preserve_nonboolean_conditions_and_modified_arm_locals() {
    let engine = wasmtime::Engine::default();
    for condition in [0, 1, -1, i32::MIN] {
        for (yes, no) in [(1, 0), (0, 1), (17, 25), (42, 42)] {
            let baseline = module(&[
                I::I32Const(condition),
                I::If(BlockType::Result(ValType::I64)),
                I::I64Const(yes),
                I::Else,
                I::I64Const(no),
                I::End,
                I::End,
            ]);
            let optimized = optimize(
                &baseline,
                Passes {
                    control: true,
                    ..Default::default()
                },
            );
            assert!(optimized.len() < baseline.len());
            assert_eq!(
                execute(&engine, &baseline),
                Ok(if condition == 0 { no } else { yes })
            );
            assert_eq!(execute(&engine, &optimized), execute(&engine, &baseline));
        }
    }
    let baseline = module(&[
        I::I64Const(42),
        I::LocalTee(0),
        I::I64Eqz,
        I::If(BlockType::Result(ValType::I64)),
        I::I64Const(7),
        I::Else,
        I::LocalGet(0),
        I::End,
        I::End,
    ]);
    let optimized = optimize(
        &baseline,
        Passes {
            control: true,
            ..Default::default()
        },
    );
    assert_eq!(optimized, baseline);
    assert_eq!(execute(&engine, &optimized), Ok(42));
    // A condition trap remains executable even with identical arms.
    let baseline = module(&[
        I::I32Const(1),
        I::I32Const(0),
        I::I32DivU,
        I::If(BlockType::Result(ValType::I64)),
        I::I64Const(42),
        I::Else,
        I::I64Const(42),
        I::End,
        I::End,
    ]);
    let optimized = optimize(
        &baseline,
        Passes {
            control: true,
            ..Default::default()
        },
    );
    assert_eq!(execute(&engine, &baseline), Err(()));
    assert_eq!(execute(&engine, &optimized), Err(()));
}

#[test]
fn selections_preserve_floating_point_bits() {
    let engine = wasmtime::Engine::default();
    for condition in [0, 1] {
        let nan = 0x7ff8000000000042u64;
        let negative_zero = 0x8000000000000000u64;
        let baseline = module(&[
            I::I32Const(condition),
            I::If(BlockType::Result(ValType::F64)),
            I::F64Const(f64::from_bits(nan).into()),
            I::Else,
            I::F64Const(f64::from_bits(negative_zero).into()),
            I::End,
            I::I64ReinterpretF64,
            I::End,
        ]);
        let optimized = optimize(
            &baseline,
            Passes {
                control: true,
                ..Default::default()
            },
        );
        assert!(optimized.len() < baseline.len());
        assert_eq!(
            execute(&engine, &baseline),
            Ok(if condition == 0 { negative_zero } else { nan } as i64)
        );
        assert_eq!(execute(&engine, &optimized), execute(&engine, &baseline));
    }
}
