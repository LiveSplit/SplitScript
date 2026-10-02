use super::{I, Passes, optimize, tests::module_with_locals};
use wasm_encoder::{BlockType, ValType};

fn check(
    ops: &[I<'_>],
    locals: &[(u32, ValType)],
    cases: &[(i32, Result<i64, ()>)],
) -> (usize, usize) {
    let baseline = module_with_locals(&[ValType::I32], locals, ops);
    let optimized = optimize(
        &baseline,
        Passes {
            propagation: true,
            constants: true,
            instructions: true,
            locals: true,
            ..Default::default()
        },
    );
    let engine = wasmtime::Engine::default();
    for wasm in [&baseline, &optimized] {
        wasmparser::Validator::new_with_features(wasmparser::WasmFeatures::all())
            .validate_all(wasm)
            .unwrap();
        let module = wasmtime::Module::new(&engine, wasm).unwrap();
        let mut store = wasmtime::Store::new(&engine, ());
        let instance = wasmtime::Instance::new(&mut store, &module, &[]).unwrap();
        let run = instance
            .get_typed_func::<i32, i64>(&mut store, "run")
            .unwrap();
        for &(input, expected) in cases {
            assert_eq!(run.call(&mut store, input).map_err(|_| ()), expected);
        }
    }
    assert!(optimized.len() <= baseline.len());
    (baseline.len(), optimized.len())
}

#[test]
fn packed_constants_fold_across_nested_scopes() {
    let (before, after) = check(
        &[
            I::I64Const(30064839263),
            I::LocalSet(1),
            I::LocalGet(0),
            I::If(BlockType::Empty),
            I::LocalGet(1),
            I::I64Const(32),
            I::I64ShrU,
            I::Return,
            I::Else,
            I::Block(BlockType::Empty),
            I::End,
            I::LocalGet(1),
            I::I32WrapI64,
            I::I64ExtendI32U,
            I::Return,
            I::End,
            I::I64Const(-1),
            I::End,
        ],
        &[(1, ValType::I64)],
        &[(0, Ok(68191)), (1, Ok(7)), (-1, Ok(7))],
    );
    assert!(before - after >= 10);
}

#[test]
fn conditional_initialization_does_not_escape_its_arm() {
    for branch in [I::BrIf(0), I::BrTable(vec![0].into(), 0)] {
        check(
            &[
                I::Block(BlockType::Empty),
                I::LocalGet(0),
                branch,
                I::I64Const(42),
                I::LocalSet(1),
                I::End,
                I::LocalGet(1),
                I::End,
            ],
            &[(1, ValType::I64)],
            // br_table always exits, while br_if only exits for nonzero.
            &[(1, Ok(0)), (-1, Ok(0))],
        );
    }
    check(
        &[
            I::LocalGet(0),
            I::If(BlockType::Empty),
            I::I64Const(42),
            I::LocalSet(1),
            I::Else,
            I::LocalGet(1),
            I::Return,
            I::End,
            I::LocalGet(1),
            I::End,
        ],
        &[(1, ValType::I64)],
        &[(0, Ok(0)), (1, Ok(42))],
    );
}

#[test]
fn loop_backedges_and_changing_writes_preserve_values() {
    check(
        &[
            I::I64Const(7),
            I::LocalSet(1),
            I::I64Const(0),
            I::LocalSet(2),
            I::Loop(BlockType::Empty),
            I::LocalGet(2),
            I::LocalGet(1),
            I::I64Add,
            I::LocalSet(2),
            I::LocalGet(0),
            I::I32Const(1),
            I::I32Sub,
            I::LocalTee(0),
            I::BrIf(0),
            I::End,
            I::LocalGet(2),
            I::End,
        ],
        &[(2, ValType::I64)],
        &[(1, Ok(7)), (3, Ok(21))],
    );
    check(
        &[
            I::I64Const(7),
            I::LocalSet(1),
            I::LocalGet(0),
            I::If(BlockType::Empty),
            I::I64Const(9),
            I::LocalSet(1),
            I::End,
            I::LocalGet(1),
            I::End,
        ],
        &[(1, ValType::I64)],
        &[(0, Ok(7)), (1, Ok(9))],
    );
    // The first iteration reads the default value; later iterations read 7.
    check(
        &[
            I::Loop(BlockType::Empty),
            I::LocalGet(2),
            I::LocalGet(1),
            I::I64Add,
            I::LocalSet(2),
            I::I64Const(7),
            I::LocalSet(1),
            I::LocalGet(0),
            I::I32Const(1),
            I::I32Sub,
            I::LocalTee(0),
            I::BrIf(0),
            I::End,
            I::LocalGet(2),
            I::End,
        ],
        &[(2, ValType::I64)],
        &[(1, Ok(0)), (3, Ok(14))],
    );
}

#[test]
fn repeated_equal_writes_and_parameter_initial_values() {
    check(
        &[
            I::LocalGet(0),
            I::I64ExtendI32S,
            I::LocalSet(1),
            I::I32Const(42),
            I::LocalSet(0),
            I::LocalGet(0),
            I::I64ExtendI32S,
            I::LocalGet(1),
            I::I64Add,
            I::End,
        ],
        &[(1, ValType::I64)],
        &[(0, Ok(42)), (-8, Ok(34))],
    );
    let (before, after) = check(
        &[
            I::I64Const(42),
            I::LocalSet(1),
            I::LocalGet(0),
            I::If(BlockType::Empty),
            I::I64Const(42),
            I::LocalSet(1),
            I::End,
            I::LocalGet(1),
            I::I64Const(2),
            I::I64Add,
            I::End,
        ],
        &[(1, ValType::I64)],
        &[(0, Ok(44)), (1, Ok(44))],
    );
    assert!(after < before);
}

#[test]
fn propagated_divisions_preserve_traps_and_wrapping() {
    for (a, b, expected) in [(7, 0, Err(())), (i64::MIN, -1, Err(())), (15, 3, Ok(5))] {
        check(
            &[
                I::I64Const(a),
                I::LocalSet(1),
                I::I64Const(b),
                I::LocalSet(2),
                I::LocalGet(1),
                I::LocalGet(2),
                I::I64DivS,
                I::End,
            ],
            &[(2, ValType::I64)],
            &[(0, expected)],
        );
    }
    for (op, expected) in [(I::I64ExtendI32S, -1), (I::I64ExtendI32U, 4294967295)] {
        check(&[I::I32Const(-1), op, I::End], &[], &[(0, Ok(expected))]);
    }
    check(
        &[
            I::I64Const(i64::MAX),
            I::I32WrapI64,
            I::I64ExtendI32S,
            I::End,
        ],
        &[],
        &[(0, Ok(-1))],
    );
}

#[test]
fn expanded_literals_are_rejected_when_the_body_grows() {
    let mut ops = vec![I::I64Const(i64::MAX), I::LocalSet(1), I::I64Const(0)];
    for _ in 0..8 {
        // Unknown arithmetic prevents constant folding from hiding expansion.
        ops.extend([
            I::LocalGet(0),
            I::I64ExtendI32S,
            I::LocalGet(1),
            I::I64Mul,
            I::I64Add,
        ]);
    }
    ops.push(I::End);
    let (before, after) = check(&ops, &[(1, ValType::I64)], &[(0, Ok(0)), (1, Ok(-8))]);
    assert_eq!(after, before);
}
