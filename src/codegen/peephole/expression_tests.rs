use super::{I, Passes, instruction_tests::instantiate, optimize};
use wasm_encoder::{
    BlockType, CodeSection, ConstExpr, EntityType, ExportKind, ExportSection, FieldType, Function,
    FunctionSection, GlobalSection, GlobalType, HeapType, ImportSection, MemArg, MemorySection,
    MemoryType, Module, RefType, StorageType, TypeSection, ValType,
};

fn address() -> MemArg {
    MemArg {
        offset: 0,
        align: 2,
        memory_index: 0,
    }
}

fn module(ops: &[I<'_>], shared: bool) -> Vec<u8> {
    let mut types = TypeSection::new();
    types.ty().struct_([
        FieldType {
            element_type: StorageType::Val(ValType::I64),
            mutable: true,
        },
        FieldType {
            element_type: StorageType::Val(ValType::I64),
            mutable: false,
        },
    ]);
    types.ty().function([ValType::I32], [ValType::I64]);
    types.ty().function([ValType::I64], [ValType::I64]);
    types.ty().function([], []);
    let mut imports = ImportSection::new();
    imports.import("env", "effect", EntityType::Function(2));
    let mut functions = FunctionSection::new();
    functions.function(3).function(1);
    let mut memories = MemorySection::new();
    memories.memory(MemoryType {
        minimum: 1,
        maximum: Some(1),
        memory64: false,
        shared,
        page_size_log2: None,
    });
    let mut globals = GlobalSection::new();
    globals.global(
        GlobalType {
            val_type: ValType::I64,
            mutable: true,
            shared: false,
        },
        &ConstExpr::i64_const(0),
    );
    let mut exports = ExportSection::new();
    exports.export("run", ExportKind::Func, 2);
    let mut code = CodeSection::new();
    let mut mutate = Function::new([]);
    for op in [
        I::GlobalGet(0),
        I::I64Const(1),
        I::I64Add,
        I::GlobalSet(0),
        I::I32Const(0),
        I::GlobalGet(0),
        I::I32WrapI64,
        I::I32Store(address()),
        I::End,
    ] {
        mutate.instruction(&op);
    }
    code.function(&mutate);
    let mut run = Function::new([
        (2, ValType::I64),
        (
            1,
            ValType::Ref(RefType {
                nullable: true,
                heap_type: HeapType::Concrete(0),
            }),
        ),
    ]);
    for op in ops {
        run.instruction(op);
    }
    code.function(&run);
    let mut wasm = Module::new();
    wasm.section(&types)
        .section(&imports)
        .section(&functions)
        .section(&memories)
        .section(&globals)
        .section(&exports)
        .section(&code);
    wasm.finish()
}

fn reuse(wasm: &[u8]) -> Vec<u8> {
    optimize(
        wasm,
        Passes {
            expressions: true,
            locals: true,
            ..Default::default()
        },
    )
}

fn run(wasm: &[u8], arg: i32) -> (Result<i64, wasmtime::Trap>, Vec<i64>) {
    let mut config = wasmtime::Config::new();
    config.wasm_gc(true).wasm_function_references(true);
    let engine = wasmtime::Engine::new(&config).unwrap();
    let (mut store, instance) = instantiate(&engine, wasm);
    let value = instance
        .get_typed_func::<i32, i64>(&mut store, "run")
        .unwrap()
        .call(&mut store, arg)
        .map_err(|e| *e.downcast_ref::<wasmtime::Trap>().unwrap());
    (value, store.into_data())
}

fn expression(local: u32) -> Vec<I<'static>> {
    vec![
        I::LocalGet(local),
        I::I64Const(257),
        I::I64Mul,
        I::I64Const(17),
        I::I64Add,
    ]
}

#[test]
fn expressions_reuse_arithmetic_across_calls_without_moving_effects() {
    let mut ops = vec![I::LocalGet(0), I::I64ExtendI32U, I::LocalSet(1)];
    for _ in 0..3 {
        ops.extend(expression(1));
        ops.extend([I::Call(0), I::Drop]);
    }
    ops.extend(expression(1));
    ops.push(I::End);
    let before = module(&ops, false);
    let after = reuse(&before);
    assert!(before.len() - after.len() > 10);
    for wasm in [&before, &after] {
        assert_eq!(run(wasm, 2), (Ok(531), vec![531; 3]));
    }
}

#[test]
fn expressions_invalidate_changed_locals_and_keep_stack_snapshots() {
    let mut ops = vec![I::LocalGet(0), I::I64ExtendI32U, I::LocalSet(1)];
    for _ in 0..2 {
        ops.extend(expression(1));
        ops.extend([I::Call(0), I::Drop]);
    }
    ops.extend(expression(1)); // Value stays on the stack while its input changes.
    ops.extend([I::LocalGet(1), I::I64Const(1), I::I64Add, I::LocalSet(1)]);
    ops.extend(expression(1));
    ops.extend([I::I64Add, I::Call(0), I::Drop]);
    ops.extend(expression(1));
    ops.push(I::End);
    let before = module(&ops, false);
    let after = reuse(&before);
    assert!(after.len() < before.len());
    for wasm in [&before, &after] {
        assert_eq!(run(wasm, 2), (Ok(788), vec![531, 531, 1319]));
    }
}

#[test]
fn expressions_invalidate_memory_and_globals_after_stores_and_calls() {
    let value = [
        I::I32Const(0),
        I::I32Load(address()),
        I::I64ExtendI32U,
        I::GlobalGet(0),
        I::I64Add,
        I::I64Const(17),
        I::I64Mul,
    ];
    let mut ops = Vec::new();
    for mutation in [
        vec![],
        vec![I::Call(1)],
        vec![I::I32Const(0), I::I32Const(9), I::I32Store(address())],
        vec![I::I64Const(3), I::GlobalSet(0)],
    ] {
        ops.extend(mutation);
        ops.extend_from_slice(&value);
        ops.extend_from_slice(&value);
        ops.extend([I::I64Add, I::Call(0), I::Drop]);
    }
    ops.extend([I::I64Const(0), I::End]);
    let before = module(&ops, false);
    let after = reuse(&before);
    assert!(after.len() < before.len());
    for wasm in [&before, &after] {
        assert_eq!(run(wasm, 0), (Ok(0), vec![0, 68, 340, 408]));
    }
    let shared = module(&ops, true);
    wasmparser::Validator::new_with_features(wasmparser::WasmFeatures::all())
        .validate_all(&shared)
        .unwrap();
    let without_reuse = optimize(
        &shared,
        Passes {
            locals: true,
            ..Default::default()
        },
    );
    assert_eq!(
        reuse(&shared),
        without_reuse,
        "shared memory may change outside this activation"
    );
}

#[test]
fn expressions_preserve_mutable_and_immutable_struct_reads() {
    let field = |field_index| I::StructGet {
        struct_type_index: 0,
        field_index,
    };
    let value = [
        I::LocalGet(3),
        field(0),
        I::LocalGet(3),
        field(1),
        I::I64Add,
        I::I64Const(257),
        I::I64Mul,
    ];
    let mut ops = vec![
        I::I64Const(5),
        I::I64Const(7),
        I::StructNew(0),
        I::LocalSet(3),
    ];
    for mutation in [
        vec![],
        vec![
            I::LocalGet(3),
            I::I64Const(13),
            I::StructSet {
                struct_type_index: 0,
                field_index: 0,
            },
        ],
    ] {
        ops.extend(mutation);
        ops.extend_from_slice(&value);
        ops.extend_from_slice(&value);
        ops.extend([I::I64Add, I::Call(0), I::Drop]);
    }
    ops.extend([I::I64Const(0), I::End]);
    let before = module(&ops, false);
    let after = reuse(&before);
    assert!(after.len() < before.len());
    for wasm in [&before, &after] {
        assert_eq!(run(wasm, 0), (Ok(0), vec![6168, 10280]));
    }
}

#[test]
fn expressions_reuse_only_values_dominating_both_arms_and_the_join() {
    let mut ops = vec![I::LocalGet(0), I::I64ExtendI32S, I::LocalSet(1)];
    ops.extend(expression(1));
    ops.extend([I::Call(0), I::Drop, I::LocalGet(0), I::If(BlockType::Empty)]);
    ops.extend(expression(1));
    ops.extend([I::Call(0), I::Drop, I::Else]);
    ops.extend(expression(1));
    ops.extend([I::Call(0), I::Drop, I::End]);
    ops.extend(expression(1));
    ops.extend([I::Call(0), I::Drop]);
    // This second conditional computes a value in only one arm. Its cached
    // value must never be read by the opposite arm or beyond the join.
    ops.extend([
        I::LocalGet(0),
        I::If(BlockType::Empty),
        I::I64Const(8),
        I::LocalSet(1),
    ]);
    ops.extend(expression(1));
    ops.extend([I::Call(0), I::Drop, I::Else]);
    ops.extend(expression(1));
    ops.extend([I::Call(0), I::Drop, I::End]);
    ops.extend(expression(1));
    ops.push(I::End);
    let before = module(&ops, false);
    let after = reuse(&before);
    assert!(after.len() < before.len());
    for wasm in [&before, &after] {
        assert_eq!(run(wasm, 0), (Ok(17), vec![17; 4]));
        assert_eq!(run(wasm, 2), (Ok(2073), vec![531, 531, 531, 2073]));
    }
}

#[test]
fn expressions_do_not_reuse_values_across_loop_backedges() {
    let mut ops = expression(1);
    ops.extend([I::LocalSet(2), I::Loop(BlockType::Empty), I::LocalGet(2)]);
    ops.extend(expression(1));
    ops.extend(expression(1));
    ops.extend([
        I::I64Add,
        I::I64Add,
        I::LocalSet(2),
        I::LocalGet(1),
        I::I64Const(1),
        I::I64Add,
        I::LocalTee(1),
        I::I64Const(3),
        I::I64LtU,
        I::BrIf(0),
        I::End,
        I::LocalGet(2),
        I::End,
    ]);
    let before = module(&ops, false);
    let after = reuse(&before);
    assert!(after.len() < before.len());
    for wasm in [&before, &after] {
        assert_eq!(run(wasm, 0), (Ok(1661), vec![]));
    }
}

#[test]
fn overlapping_expressions_do_not_move_the_cache_origin_into_one_arm() {
    let mut ops = vec![I::I64Const(5), I::LocalSet(1)];
    for _ in 0..2 {
        ops.extend(expression(1));
        ops.extend([
            I::I64Const(10_000_000),
            I::I64Mul,
            I::I64Const(12345),
            I::I64Add,
            I::I64Const(99),
            I::I64Mul,
            I::I64Const(16384),
            I::I64Add,
            I::Call(0),
            I::Drop,
        ]);
    }
    ops.extend([I::LocalGet(0), I::If(BlockType::Result(ValType::I64))]);
    ops.extend(expression(1));
    ops.push(I::Else);
    ops.extend(expression(1));
    ops.extend([I::End, I::End]);
    let before = module(&ops, false);
    let after = reuse(&before);
    assert!(after.len() < before.len());
    let expected = (1302 * 10_000_000i64 + 12345) * 99 + 16384;
    for wasm in [&before, &after] {
        for arg in [0, 1] {
            assert_eq!(run(wasm, arg), (Ok(1302), vec![expected; 2]));
        }
    }
}

#[test]
fn expressions_preserve_first_trap_and_nondefaultable_reference_initialization() {
    let value = [
        I::I64Const(42),
        I::LocalGet(0),
        I::I64ExtendI32U,
        I::I64DivU,
    ];
    let mut ops = vec![I::I64Const(17), I::Call(0), I::Drop];
    for _ in 0..3 {
        ops.extend_from_slice(&value);
        ops.extend([I::Call(0), I::Drop]);
    }
    ops.extend([I::I64Const(0), I::End]);
    let before = module(&ops, false);
    let after = reuse(&before);
    assert!(after.len() < before.len());
    for wasm in [&before, &after] {
        assert_eq!(run(wasm, 2), (Ok(0), vec![17, 21, 21, 21]));
        assert_eq!(
            run(wasm, 0),
            (Err(wasmtime::Trap::IntegerDivisionByZero), vec![17])
        );
    }
    let mut ops = vec![
        I::LocalGet(0),
        I::If(BlockType::Empty),
        I::I64Const(5),
        I::I64Const(7),
        I::StructNew(0),
        I::LocalSet(3),
        I::End,
    ];
    for value in 0..8 {
        ops.extend([
            I::LocalGet(3),
            I::RefCastNonNull(HeapType::Concrete(0)),
            I::I64Const(value),
            I::StructSet {
                struct_type_index: 0,
                field_index: 0,
            },
        ]);
    }
    ops.extend([
        I::LocalGet(0),
        I::If(BlockType::Empty),
        I::LocalGet(3),
        I::RefCastNonNull(HeapType::Concrete(0)),
        I::I64Const(42),
        I::StructSet {
            struct_type_index: 0,
            field_index: 0,
        },
        I::End,
        I::LocalGet(3),
        I::StructGet {
            struct_type_index: 0,
            field_index: 0,
        },
        I::End,
    ]);
    let before = module(&ops, false);
    let after = reuse(&before);
    assert!(after.len() < before.len());
    for wasm in [&before, &after] {
        assert_eq!(run(wasm, 1), (Ok(42), vec![]));
        assert_eq!(run(wasm, 0), (Err(wasmtime::Trap::CastFailure), vec![]));
    }
}
