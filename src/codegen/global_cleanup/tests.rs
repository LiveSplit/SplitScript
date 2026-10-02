use super::optimize;
use wasm_encoder::{
    CodeSection, ConstExpr, EntityType, ExportKind, ExportSection, Function, FunctionSection,
    GlobalSection, GlobalType, ImportSection, Instruction as I, MemorySection, MemoryType, Module,
    TypeSection, ValType,
};

fn global(ty: ValType, mutable: bool) -> GlobalType {
    GlobalType {
        val_type: ty,
        mutable,
        shared: false,
    }
}

fn module(globals: GlobalSection, ops: &[I<'_>], exported: Option<u32>) -> Vec<u8> {
    let mut types = TypeSection::new();
    types.ty().function([], [ValType::I64]);
    let mut imports = ImportSection::new();
    imports.import("env", "effect", EntityType::Function(0));
    imports.import(
        "env",
        "value",
        EntityType::Global(global(ValType::I64, true)),
    );
    let mut functions = FunctionSection::new();
    functions.function(0);
    let mut exports = ExportSection::new();
    exports.export("run", ExportKind::Func, 1);
    if let Some(index) = exported {
        exports.export("value", ExportKind::Global, index);
    }
    let mut body = Function::new([]);
    for op in ops {
        body.instruction(op);
    }
    let mut code = CodeSection::new();
    code.function(&body);
    let mut output = Module::new();
    output
        .section(&types)
        .section(&imports)
        .section(&functions)
        .section(&globals)
        .section(&exports)
        .section(&code);
    output.finish()
}

fn instantiate(wasm: &[u8]) -> (wasmtime::Store<u32>, wasmtime::Instance, wasmtime::Global) {
    wasmparser::Validator::new_with_features(wasmparser::WasmFeatures::all())
        .validate_all(wasm)
        .unwrap();
    let mut config = wasmtime::Config::new();
    config.wasm_gc(true).wasm_function_references(true);
    let engine = wasmtime::Engine::new(&config).unwrap();
    let module = wasmtime::Module::new(&engine, wasm).unwrap();
    let mut linker = wasmtime::Linker::new(&engine);
    linker
        .func_wrap("env", "effect", |mut caller: wasmtime::Caller<'_, u32>| {
            *caller.data_mut() += 1;
            17_i64
        })
        .unwrap();
    let mut store = wasmtime::Store::new(&engine, 0);
    let imported = wasmtime::Global::new(
        &mut store,
        wasmtime::GlobalType::new(wasmtime::ValType::I64, wasmtime::Mutability::Var),
        wasmtime::Val::I64(7),
    )
    .unwrap();
    linker.define(&store, "env", "value", imported).unwrap();
    let instance = linker.instantiate(&mut store, &module).unwrap();
    (store, instance, imported)
}

#[test]
fn unread_stores_preserve_effects_traps_and_remap_observable_globals() {
    for trap in [false, true] {
        let mut globals = GlobalSection::new();
        globals.global(global(ValType::I64, true), &ConstExpr::i64_const(0)); // 1 unread
        globals.global(global(ValType::I64, true), &ConstExpr::i64_const(0)); // 2 exported
        let mut ops = vec![
            I::Call(0),
            I::GlobalSet(1),
            I::I64Const(9),
            I::GlobalSet(0),
            I::GlobalGet(0),
            I::GlobalSet(2),
        ];
        if trap {
            ops.extend([I::I64Const(1), I::I64Const(0), I::I64DivS, I::GlobalSet(1)]);
        }
        ops.extend([I::GlobalGet(2), I::End]);
        let before = module(globals, &ops, Some(2));
        let after = optimize(&before);
        assert!(after.len() < before.len());
        for wasm in [&before, &after] {
            let (mut store, instance, imported) = instantiate(wasm);
            let result = instance
                .get_typed_func::<(), i64>(&mut store, "run")
                .unwrap()
                .call(&mut store, ());
            if trap {
                assert!(result.is_err());
            } else {
                assert_eq!(result.unwrap(), 9);
            }
            assert_eq!(*store.data(), 1);
            assert_eq!(imported.get(&mut store).i64(), Some(9));
            assert_eq!(
                instance
                    .get_global(&mut store, "value")
                    .unwrap()
                    .get(&mut store)
                    .i64(),
                Some(9)
            );
        }
    }
}

#[test]
fn constants_include_initial_value_and_all_writes_across_calls() {
    for changed in [false, true] {
        let mut globals = GlobalSection::new();
        globals.global(global(ValType::I64, true), &ConstExpr::i64_const(0));
        globals.global(global(ValType::I64, true), &ConstExpr::i64_const(0)); // always removable
        let before = module(
            globals,
            &[
                I::GlobalGet(1),
                I::I64Const(if changed { 3 } else { 0 }),
                I::GlobalSet(1),
                I::End,
            ],
            None,
        );
        let after = optimize(&before);
        assert!(after.len() < before.len());
        for wasm in [&before, &after] {
            let (mut store, instance, _) = instantiate(wasm);
            let run = instance
                .get_typed_func::<(), i64>(&mut store, "run")
                .unwrap();
            assert_eq!(run.call(&mut store, ()).unwrap(), 0);
            assert_eq!(
                run.call(&mut store, ()).unwrap(),
                if changed { 3 } else { 0 }
            );
        }
    }
}

#[test]
fn float_bit_patterns_and_large_constants_are_not_approximated() {
    for bits in [(-0.0_f64).to_bits(), 0x7ff8_0000_0000_0001] {
        let mut globals = GlobalSection::new();
        globals.global(
            global(ValType::F64, true),
            &ConstExpr::f64_const(0.0.into()),
        );
        globals.global(global(ValType::I64, true), &ConstExpr::i64_const(0));
        let before = module(
            globals,
            &[
                I::F64Const(f64::from_bits(bits).into()),
                I::GlobalSet(1),
                I::GlobalGet(1),
                I::I64ReinterpretF64,
                I::End,
            ],
            None,
        );
        for wasm in [&before, &optimize(&before)] {
            let (mut store, instance, _) = instantiate(wasm);
            assert_eq!(
                instance
                    .get_typed_func::<(), i64>(&mut store, "run")
                    .unwrap()
                    .call(&mut store, ())
                    .unwrap() as u64,
                bits
            );
        }
    }
    let mut globals = GlobalSection::new();
    globals.global(global(ValType::I64, true), &ConstExpr::i64_const(i64::MAX));
    let before = module(globals, &[I::GlobalGet(1), I::End], None);
    assert_eq!(optimize(&before), before);
}

#[test]
fn module_initializers_and_data_offsets_pin_and_remap_global_references() {
    let mut globals = GlobalSection::new();
    globals.global(global(ValType::I32, false), &ConstExpr::i32_const(0)); // removed
    globals.global(global(ValType::I32, false), &ConstExpr::i32_const(3)); // offset
    globals.global(global(ValType::I32, false), &ConstExpr::global_get(1)); // copy
    let mut memory = MemorySection::new();
    memory.memory(MemoryType {
        minimum: 1,
        maximum: None,
        memory64: false,
        shared: false,
        page_size_log2: None,
    });
    let mut data = wasm_encoder::DataSection::new();
    data.active(0, &ConstExpr::global_get(2), [42]);
    let mut exports = ExportSection::new();
    exports.export("memory", ExportKind::Memory, 0);
    let mut m = Module::new();
    m.section(&memory)
        .section(&globals)
        .section(&exports)
        .section(&data);
    let before = m.finish();
    let after = optimize(&before);
    assert!(after.len() < before.len());
    for wasm in [&before, &after] {
        let (mut store, instance, _) = instantiate(wasm);
        assert_eq!(
            instance
                .get_memory(&mut store, "memory")
                .unwrap()
                .data(&store)[3],
            42
        );
    }
}
