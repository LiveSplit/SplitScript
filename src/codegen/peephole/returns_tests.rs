use super::{Passes, optimize};
use wasm_encoder::{
    BlockType, CodeSection, ConstExpr, ExportKind, ExportSection, Function, FunctionSection,
    GlobalSection, GlobalType, Instruction as I, MemArg, MemorySection, MemoryType, Module,
    TypeSection, ValType,
};

fn module(results: &[ValType], ops: &[I<'_>]) -> Vec<u8> {
    let mut types = TypeSection::new();
    types.ty().function([ValType::I32], results.iter().copied());
    types.ty().function([], [ValType::I64]);
    let mut functions = FunctionSection::new();
    functions.function(0).function(1);
    let mut memory = MemorySection::new();
    memory.memory(MemoryType {
        minimum: 1,
        maximum: None,
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
        &ConstExpr::i64_const(0),
    );
    let mut exports = ExportSection::new();
    exports
        .export("run", ExportKind::Func, 0)
        .export("observed", ExportKind::Global, 0);
    let mut body = Function::new([(2, ValType::I64)]);
    for op in ops {
        body.instruction(op);
    }
    let mut helper = Function::new([]);
    for op in [
        I::GlobalGet(0),
        I::I64Const(1),
        I::I64Add,
        I::GlobalSet(0),
        I::I64Const(11),
        I::End,
    ] {
        helper.instruction(&op);
    }
    let mut code = CodeSection::new();
    code.function(&body).function(&helper);
    let mut module = Module::new();
    module
        .section(&types)
        .section(&functions)
        .section(&memory)
        .section(&globals)
        .section(&exports)
        .section(&code);
    module.finish()
}

fn fold(wasm: &[u8]) -> Vec<u8> {
    let optimized = optimize(
        wasm,
        Passes {
            returns: true,
            ..Default::default()
        },
    );
    for wasm in [wasm, &optimized] {
        wasmparser::Validator::new_with_features(wasmparser::WasmFeatures::all())
            .validate_all(wasm)
            .unwrap();
    }
    optimized
}

fn run(engine: &wasmtime::Engine, wasm: &[u8], arg: i32) -> (Result<i64, wasmtime::Trap>, i64) {
    let module = wasmtime::Module::new(engine, wasm).unwrap();
    let mut store = wasmtime::Store::new(engine, ());
    let instance = wasmtime::Instance::new(&mut store, &module, &[]).unwrap();
    let result = instance
        .get_typed_func::<i32, i64>(&mut store, "run")
        .unwrap()
        .call(&mut store, arg)
        .map_err(|error| *error.downcast_ref::<wasmtime::Trap>().unwrap());
    let observed = instance
        .get_global(&mut store, "observed")
        .unwrap()
        .get(&mut store)
        .i64()
        .unwrap();
    (result, observed)
}

fn tail() -> Vec<I<'static>> {
    vec![
        I::GlobalGet(0),
        I::I64Const(2),
        I::I64Mul,
        I::LocalTee(2),
        I::GlobalSet(0),
        I::LocalGet(2),
        I::I64Const(4),
        I::I64Add,
    ]
}

fn two_exits(shared: &[I<'static>], explicit_last: bool) -> Vec<I<'static>> {
    let mut ops = vec![
        I::LocalGet(0),
        I::If(BlockType::Empty),
        I::I64Const(123), // discarded by the early return
        I::I64Const(10),
        I::GlobalSet(0),
        I::Call(1),
        I::Drop,
    ];
    ops.extend_from_slice(shared);
    ops.extend([
        I::Return,
        I::End,
        I::I64Const(20),
        I::GlobalSet(0),
        I::Call(1),
        I::Drop,
    ]);
    ops.extend_from_slice(shared);
    if explicit_last {
        ops.push(I::Return);
    }
    ops.push(I::End);
    ops
}

#[test]
fn shared_returns_preserve_effects_fallthrough_and_discarded_operands() {
    let engine = wasmtime::Engine::default();
    for explicit_last in [false, true] {
        let baseline = module(&[ValType::I64], &two_exits(&tail(), explicit_last));
        let optimized = fold(&baseline);
        assert!(optimized.len() < baseline.len());
        for wasm in [&baseline, &optimized] {
            assert_eq!(run(&engine, wasm, 0), (Ok(46), 42));
            assert_eq!(run(&engine, wasm, 1), (Ok(26), 22));
        }
    }
}

#[test]
fn shared_returns_remap_function_branches_and_preserve_loop_back_edges() {
    let engine = wasmtime::Engine::default();
    for prefix in [
        vec![
            I::Block(BlockType::Empty),
            I::I64Const(99),
            I::LocalGet(0),
            I::I32Const(2),
            I::I32Eq,
            I::BrIf(1),
            I::Drop,
            I::End,
        ],
        vec![
            I::Block(BlockType::Result(ValType::I64)),
            I::I64Const(99),
            I::LocalGet(0),
            I::I32Const(2),
            I::I32Eq,
            I::BrTable(vec![0].into(), 1),
            I::End,
            I::Drop,
        ],
    ] {
        let mut ops = prefix;
        ops.extend([
            I::Loop(BlockType::Empty),
            I::LocalGet(1),
            I::I64Const(1),
            I::I64Add,
            I::LocalTee(1),
            I::I64Const(3),
            I::I64LtU,
            I::BrIf(0),
            I::End,
        ]);
        ops.extend(two_exits(&tail(), false));
        let baseline = module(&[ValType::I64], &ops);
        let optimized = fold(&baseline);
        assert!(optimized.len() < baseline.len());
        for wasm in [&baseline, &optimized] {
            assert_eq!(run(&engine, wasm, 0), (Ok(46), 42));
            assert_eq!(run(&engine, wasm, 1), (Ok(26), 22));
            assert_eq!(run(&engine, wasm, 2), (Ok(99), 0));
        }
    }
}

#[test]
fn shared_returns_preserve_trap_identity_and_effect_order() {
    let engine = wasmtime::Engine::default();
    let mut shared = tail();
    shared.extend([
        I::Drop,
        I::I32Const(-1),
        I::I64Load(MemArg {
            offset: 0,
            align: 0,
            memory_index: 0,
        }),
    ]);
    let baseline = module(&[ValType::I64], &two_exits(&shared, false));
    let optimized = fold(&baseline);
    assert!(optimized.len() < baseline.len());
    for wasm in [&baseline, &optimized] {
        assert_eq!(
            run(&engine, wasm, 0),
            (Err(wasmtime::Trap::MemoryOutOfBounds), 42)
        );
        assert_eq!(
            run(&engine, wasm, 1),
            (Err(wasmtime::Trap::MemoryOutOfBounds), 22)
        );
    }
}

#[test]
fn shared_returns_support_multiple_and_zero_results() {
    let engine = wasmtime::Engine::default();
    for results in [vec![ValType::I64, ValType::I32], vec![]] {
        let mut shared = tail();
        shared.push(if results.is_empty() {
            I::Drop
        } else {
            I::I32Const(7)
        });
        let baseline = module(&results, &two_exits(&shared, false));
        let optimized = fold(&baseline);
        assert!(optimized.len() < baseline.len());
        for wasm in [&baseline, &optimized] {
            let module = wasmtime::Module::new(&engine, wasm).unwrap();
            for arg in [0, 1] {
                let mut store = wasmtime::Store::new(&engine, ());
                let instance = wasmtime::Instance::new(&mut store, &module, &[]).unwrap();
                if results.is_empty() {
                    instance
                        .get_typed_func::<i32, ()>(&mut store, "run")
                        .unwrap()
                        .call(&mut store, arg)
                        .unwrap();
                } else {
                    assert_eq!(
                        instance
                            .get_typed_func::<i32, (i64, i32)>(&mut store, "run")
                            .unwrap()
                            .call(&mut store, arg)
                            .unwrap(),
                        (if arg == 0 { 46 } else { 26 }, 7)
                    );
                }
                assert_eq!(
                    instance
                        .get_global(&mut store, "observed")
                        .unwrap()
                        .get(&mut store)
                        .i64(),
                    Some(if arg == 0 { 42 } else { 22 })
                );
            }
        }
    }
}

#[test]
fn shared_returns_leave_stack_dependent_or_unprofitable_tails_alone() {
    for ops in [
        vec![
            I::LocalGet(0),
            I::If(BlockType::Empty),
            I::I64Const(1),
            I::Return,
            I::End,
            I::I64Const(1),
            I::End,
        ],
        // The identical add needs a different operand from outside the suffix.
        vec![
            I::LocalGet(0),
            I::If(BlockType::Empty),
            I::I64Const(1),
            I::I64Const(7),
            I::I64Add,
            I::Return,
            I::End,
            I::I64Const(2),
            I::I64Const(7),
            I::I64Add,
            I::End,
        ],
    ] {
        let baseline = module(&[ValType::I64], &ops);
        assert_eq!(fold(&baseline), baseline);
    }
}

#[test]
fn shared_returns_preserve_deep_function_label_encodings() {
    let engine = wasmtime::Engine::default();
    // Adding the wrapper changes a function-label branch from a one-byte to
    // a two-byte depth. Final encoded size, not an instruction count, decides.
    let mut ops = vec![I::Block(BlockType::Empty); 127];
    ops.extend([
        I::I64Const(99),
        I::LocalGet(0),
        I::I32Const(2),
        I::I32Eq,
        I::BrIf(127),
        I::Drop,
    ]);
    ops.extend(std::iter::repeat_n(I::End, 127));
    ops.extend(two_exits(&tail(), false));
    let baseline = module(&[ValType::I64], &ops);
    let optimized = fold(&baseline);
    assert!(optimized.len() < baseline.len());
    for arg in [0, 1, 2] {
        assert_eq!(run(&engine, &baseline, arg), run(&engine, &optimized, arg));
    }
}

#[test]
fn shared_returns_account_for_struct_constructor_inputs() {
    let mut config = wasmtime::Config::new();
    config.wasm_gc(true).wasm_function_references(true);
    let engine = wasmtime::Engine::new(&config).unwrap();
    let mut types = TypeSection::new();
    types.ty().struct_(std::iter::repeat_n(
        wasm_encoder::FieldType {
            element_type: wasm_encoder::StorageType::Val(ValType::I64),
            mutable: false,
        },
        3,
    ));
    types.ty().function([ValType::I32], [ValType::I64]);
    let mut functions = FunctionSection::new();
    functions.function(1);
    let mut exports = ExportSection::new();
    exports.export("run", ExportKind::Func, 0);
    let mut body = Function::new([]);
    body.instruction(&I::LocalGet(0))
        .instruction(&I::If(BlockType::Empty));
    for end in [I::Return, I::End] {
        for op in [
            I::I64Const(1234567),
            I::I64Const(7654321),
            I::I64Const(42),
            I::StructNew(0),
            I::StructGet {
                struct_type_index: 0,
                field_index: 2,
            },
        ] {
            body.instruction(&op);
        }
        body.instruction(&end);
        if matches!(end, I::Return) {
            body.instruction(&I::End);
        }
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
    let optimized = fold(&baseline);
    assert!(optimized.len() < baseline.len());
    for wasm in [&baseline, &optimized] {
        let module = wasmtime::Module::new(&engine, wasm).unwrap();
        let mut store = wasmtime::Store::new(&engine, ());
        let instance = wasmtime::Instance::new(&mut store, &module, &[]).unwrap();
        let run = instance
            .get_typed_func::<i32, i64>(&mut store, "run")
            .unwrap();
        for arg in [0, 1] {
            assert_eq!(run.call(&mut store, arg).unwrap(), 42);
        }
    }
}

#[test]
fn shared_returns_respect_nondefaultable_local_initialization() {
    let mut config = wasmtime::Config::new();
    config.wasm_gc(true).wasm_function_references(true);
    let engine = wasmtime::Engine::new(&config).unwrap();
    for nullable in [false, true] {
        let mut types = TypeSection::new();
        types.ty().struct_([wasm_encoder::FieldType {
            element_type: wasm_encoder::StorageType::Val(ValType::I64),
            mutable: false,
        }]);
        types.ty().function([ValType::I32], [ValType::I64]);
        let mut functions = FunctionSection::new();
        functions.function(1);
        let mut exports = ExportSection::new();
        exports.export("run", ExportKind::Func, 0);
        let mut body = Function::new([(
            1,
            ValType::Ref(wasm_encoder::RefType {
                nullable,
                heap_type: wasm_encoder::HeapType::Concrete(0),
            }),
        )]);
        let shared = [
            I::LocalGet(1),
            I::StructGet {
                struct_type_index: 0,
                field_index: 0,
            },
            I::I64Const(100000),
            I::I64Add,
            I::I64Const(7),
            I::I64Mul,
        ];
        body.instruction(&I::LocalGet(0))
            .instruction(&I::If(BlockType::Empty));
        for (value, end) in [(42, I::Return), (7, I::End)] {
            body.instruction(&I::I64Const(value))
                .instruction(&I::StructNew(0))
                .instruction(&I::LocalSet(1));
            for op in &shared {
                body.instruction(op);
            }
            body.instruction(&end);
            if value == 42 {
                body.instruction(&I::End);
            }
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
        let optimized = fold(&baseline);
        assert_eq!(optimized.len() < baseline.len(), nullable);
        if !nullable {
            assert_eq!(optimized, baseline);
        }
        for wasm in [&baseline, &optimized] {
            let module = wasmtime::Module::new(&engine, wasm).unwrap();
            let mut store = wasmtime::Store::new(&engine, ());
            let instance = wasmtime::Instance::new(&mut store, &module, &[]).unwrap();
            let run = instance
                .get_typed_func::<i32, i64>(&mut store, "run")
                .unwrap();
            assert_eq!(run.call(&mut store, 0).unwrap(), 700049);
            assert_eq!(run.call(&mut store, 1).unwrap(), 700294);
        }
    }
}
