use super::optimize;
use wasm_encoder::{
    CodeSection, ConstExpr, ElementSection, Elements, EntityType, ExportKind, ExportSection,
    Function, FunctionSection, ImportSection, Instruction as I, Module, RefType, TableSection,
    TableType, TypeSection, ValType,
};

fn module(
    params: &[ValType],
    results: &[ValType],
    locals: &[(u32, ValType)],
    bodies: &[Vec<I<'_>>],
    imports: u32,
    padding_types: usize,
) -> Vec<u8> {
    let mut types = TypeSection::new();
    types
        .ty()
        .function(params.iter().copied(), results.iter().copied());
    types.ty().function([ValType::I64], [ValType::I64]);
    for _ in 0..padding_types {
        types.ty().function([], []);
    }
    let mut imported = ImportSection::new();
    for _ in 0..imports {
        imported.import("env", "effect", EntityType::Function(1));
    }
    let mut functions = FunctionSection::new();
    let mut code = CodeSection::new();
    let mut exports = ExportSection::new();
    for (i, ops) in bodies.iter().enumerate() {
        functions.function(0);
        let mut body = Function::new(locals.iter().copied());
        for op in ops {
            body.instruction(op);
        }
        code.function(&body);
        exports.export(&format!("f{i}"), ExportKind::Func, imports + i as u32);
    }
    let mut table = TableSection::new();
    table.table(TableType {
        element_type: RefType::FUNCREF,
        table64: false,
        minimum: bodies.len() as u64,
        maximum: None,
        shared: false,
    });
    exports.export("table", ExportKind::Table, 0);
    let mut elements = ElementSection::new();
    elements.active(
        None,
        &ConstExpr::i32_const(0),
        Elements::Functions(
            (imports..imports + bodies.len() as u32)
                .collect::<Vec<_>>()
                .into(),
        ),
    );
    let mut module = Module::new();
    module
        .section(&types)
        .section(&imported)
        .section(&functions)
        .section(&table)
        .section(&exports)
        .section(&elements)
        .section(&code);
    module.finish()
}

fn validate(wasm: &[u8]) {
    wasmparser::Validator::new_with_features(wasmparser::WasmFeatures::all())
        .validate_all(wasm)
        .unwrap();
}

fn instantiate(
    engine: &wasmtime::Engine,
    wasm: &[u8],
    imports: usize,
) -> (wasmtime::Store<Vec<i64>>, wasmtime::Instance) {
    validate(wasm);
    let module = wasmtime::Module::new(engine, wasm).unwrap();
    let mut store = wasmtime::Store::new(engine, Vec::new());
    let effect = wasmtime::Func::wrap(
        &mut store,
        |mut caller: wasmtime::Caller<'_, Vec<i64>>, value: i64| {
            caller.data_mut().push(value);
            value.wrapping_add(1)
        },
    );
    let instance = wasmtime::Instance::new(
        &mut store,
        &module,
        &vec![wasmtime::Extern::Func(effect); imports],
    )
    .unwrap();
    (store, instance)
}

fn accumulated(value: i64, parameter: u32, local: u32, repeats: usize) -> Vec<I<'static>> {
    let mut ops = vec![I::LocalGet(parameter), I::LocalSet(local)];
    for _ in 0..repeats {
        ops.extend([
            I::LocalGet(local),
            I::I64Const(value),
            I::I64Add,
            I::LocalSet(local),
        ]);
    }
    ops.extend([I::LocalGet(local), I::End]);
    ops
}

#[test]
fn merging_preserves_parameter_mutation_locals_effects_and_function_references() {
    let engine = wasmtime::Engine::default();
    let bodies = [3, 7].map(|value| {
        let mut ops = vec![
            I::LocalGet(0),
            I::I64Const(2),
            I::I64Add,
            I::LocalSet(0),
            I::LocalGet(0),
            I::Call(0),
            I::LocalSet(0),
        ];
        ops.extend(accumulated(value, 0, 1, 12));
        ops
    });
    let baseline = module(
        &[ValType::I64],
        &[ValType::I64],
        &[(1, ValType::I64)],
        &bodies,
        1,
        0,
    );
    let mut report = crate::codegen::CodegenReport {
        functions: vec![(1, "a".into()), (2, "b".into())],
        ..Default::default()
    };
    let optimized = optimize(&baseline, Some(&mut report));
    assert!(optimized.len() < baseline.len());
    assert_eq!(optimized, optimize(&baseline, None));
    assert_eq!(report.functions.len(), 3);
    assert_eq!(report.functions[2].0, 3);
    // Repeated occurrences of a varying constant share one new parameter.
    for payload in wasmparser::Parser::new(0).parse_all(&optimized) {
        if let wasmparser::Payload::TypeSection(types) = payload.unwrap() {
            let types = types
                .into_iter_err_on_gc_types()
                .collect::<Result<Vec<_>, _>>()
                .unwrap();
            assert_eq!(types.last().unwrap().params().len(), 2);
        }
    }
    for wasm in [&baseline, &optimized] {
        let (mut store, instance) = instantiate(&engine, wasm, 1);
        for (i, value) in [3i64, 7].into_iter().enumerate() {
            let direct = instance
                .get_typed_func::<i64, i64>(&mut store, &format!("f{i}"))
                .unwrap();
            let table = instance.get_table(&mut store, "table").unwrap();
            let wasmtime::Ref::Func(Some(indirect)) = table.get(&mut store, i as u64).unwrap()
            else {
                panic!()
            };
            let indirect = indirect.typed::<i64, i64>(&store).unwrap();
            for arg in [0i64, -100, i64::MAX] {
                let expected = arg.wrapping_add(3).wrapping_add(12 * value);
                for function in [&direct, &indirect] {
                    store.data_mut().clear();
                    assert_eq!(function.call(&mut store, arg).unwrap(), expected);
                    assert_eq!(store.data(), &[arg.wrapping_add(2)]);
                }
            }
        }
    }
}

#[test]
fn merging_preserves_mixed_constants_multiple_results_and_traps() {
    let engine = wasmtime::Engine::default();
    for trap in [false, true] {
        let bodies = [(3, i32::MIN), (7, i32::MAX)].map(|(value, tag)| {
            let mut ops = accumulated(value, 0, 1, 12);
            ops.pop();
            ops.extend([I::LocalGet(0), I::Call(0), I::Drop]);
            if trap {
                ops.extend([I::I64Const(0), I::I64DivU]);
            }
            ops.extend([I::I32Const(tag), I::End]);
            ops
        });
        let baseline = module(
            &[ValType::I64],
            &[ValType::I64, ValType::I32],
            &[(1, ValType::I64)],
            &bodies,
            1,
            0,
        );
        let optimized = optimize(&baseline, None);
        assert!(optimized.len() < baseline.len());
        for wasm in [&baseline, &optimized] {
            let (mut store, instance) = instantiate(&engine, wasm, 1);
            for (i, (value, tag)) in [(3, i32::MIN), (7, i32::MAX)].into_iter().enumerate() {
                store.data_mut().clear();
                let result = instance
                    .get_typed_func::<i64, (i64, i32)>(&mut store, &format!("f{i}"))
                    .unwrap()
                    .call(&mut store, 5);
                if trap {
                    assert_eq!(
                        result.unwrap_err().downcast_ref::<wasmtime::Trap>(),
                        Some(&wasmtime::Trap::IntegerDivisionByZero)
                    );
                } else {
                    assert_eq!(result.unwrap(), (5 + 12 * value, tag));
                }
                assert_eq!(store.data(), &[5]);
            }
        }
    }
}

#[test]
fn merging_accounts_for_local_function_and_type_index_encoding_boundaries() {
    let engine = wasmtime::Engine::default();
    let bodies = [3, 7].map(|v| accumulated(v, 126, 127, 160));
    let baseline = module(
        &[ValType::I64; 127],
        &[ValType::I64],
        &[(1, ValType::I64)],
        &bodies,
        126,
        126,
    );
    let optimized = optimize(&baseline, None);
    assert!(optimized.len() < baseline.len());
    for wasm in [&baseline, &optimized] {
        let (mut store, instance) = instantiate(&engine, wasm, 126);
        for (i, value) in [3, 7].into_iter().enumerate() {
            let function = instance.get_func(&mut store, &format!("f{i}")).unwrap();
            let mut results = [wasmtime::Val::I64(0)];
            function
                .call(&mut store, &vec![wasmtime::Val::I64(2); 127], &mut results)
                .unwrap();
            assert_eq!(results[0].i64(), Some(2 + 160 * value));
        }
    }
}

#[test]
fn merging_rejects_unprofitable_different_and_overparameterized_bodies() {
    let tiny = [vec![I::I64Const(3), I::End], vec![I::I64Const(7), I::End]];
    let mut different = [accumulated(3, 0, 1, 12), accumulated(7, 0, 1, 12)];
    different[1][4] = I::I64Sub;
    let too_many = [0, 100].map(|offset| {
        let mut ops = vec![I::I64Const(0), I::LocalSet(1)];
        for _ in 0..10 {
            for n in 1..=9 {
                ops.extend([
                    I::LocalGet(1),
                    I::I64Const(n + offset),
                    I::I64Add,
                    I::LocalSet(1),
                ]);
            }
        }
        ops.extend([I::LocalGet(1), I::End]);
        ops
    });
    for bodies in [tiny, different, too_many] {
        let baseline = module(
            &[ValType::I64],
            &[ValType::I64],
            &[(1, ValType::I64)],
            &bodies,
            1,
            0,
        );
        validate(&baseline);
        let mut report = crate::codegen::CodegenReport::default();
        let before = report.clone();
        assert_eq!(optimize(&baseline, Some(&mut report)), baseline);
        assert_eq!(report, before);
    }
    // Do not exceed the engine limit by adding parameters to a large signature.
    let bodies = [3, 7].map(|v| accumulated(v, 999, 1000, 600));
    let baseline = module(
        &[ValType::I64; 1000],
        &[ValType::I64],
        &[(1, ValType::I64)],
        &bodies,
        1,
        0,
    );
    validate(&baseline);
    assert_eq!(optimize(&baseline, None), baseline);
}

#[test]
fn merging_preserves_tail_recursion_without_adding_wrapper_frames() {
    let mut config = wasmtime::Config::new();
    config.wasm_tail_call(true);
    let engine = wasmtime::Engine::new(&config).unwrap();
    let bodies = [3, 7].map(|value| {
        let mut ops = vec![
            I::LocalGet(0),
            I::I64Eqz,
            I::If(wasm_encoder::BlockType::Result(ValType::I64)),
            I::I64Const(value),
            I::Else,
            I::LocalGet(0),
            I::I64Const(1),
            I::I64Sub,
            I::LocalSet(0),
        ];
        for _ in 0..12 {
            ops.extend([I::LocalGet(0), I::I64Const(0), I::I64Add, I::LocalSet(0)]);
        }
        ops.extend([I::LocalGet(0), I::ReturnCall(1), I::End, I::End]);
        ops
    });
    let baseline = module(&[ValType::I64], &[ValType::I64], &[], &bodies, 1, 0);
    let optimized = optimize(&baseline, None);
    assert_eq!(optimized, baseline);
    let (mut store, instance) = instantiate(&engine, &optimized, 1);
    let function = instance
        .get_typed_func::<i64, i64>(&mut store, "f1")
        .unwrap();
    assert_eq!(function.call(&mut store, 0).unwrap(), 7);
    assert_eq!(function.call(&mut store, 100_000).unwrap(), 3);
}

#[test]
fn merging_preserves_gc_types_and_reference_locals() {
    let mut config = wasmtime::Config::new();
    config.wasm_gc(true).wasm_function_references(true);
    let engine = wasmtime::Engine::new(&config).unwrap();
    let mut types = TypeSection::new();
    types.ty().struct_([wasm_encoder::FieldType {
        element_type: wasm_encoder::StorageType::Val(ValType::I64),
        mutable: false,
    }]);
    types.ty().function([], [ValType::I64]);
    let mut functions = FunctionSection::new();
    let mut code = CodeSection::new();
    let mut exports = ExportSection::new();
    for (i, value) in [3, 7].into_iter().enumerate() {
        functions.function(1);
        exports.export(&format!("f{i}"), ExportKind::Func, i as u32);
        let mut body = Function::new([(
            1,
            ValType::Ref(RefType {
                nullable: true,
                heap_type: wasm_encoder::HeapType::Concrete(0),
            }),
        )]);
        body.instruction(&I::I64Const(value))
            .instruction(&I::StructNew(0))
            .instruction(&I::LocalSet(0));
        for _ in 0..12 {
            for op in [
                I::LocalGet(0),
                I::StructGet {
                    struct_type_index: 0,
                    field_index: 0,
                },
                I::I64Const(1),
                I::I64Add,
                I::StructNew(0),
                I::LocalSet(0),
            ] {
                body.instruction(&op);
            }
        }
        body.instruction(&I::LocalGet(0))
            .instruction(&I::StructGet {
                struct_type_index: 0,
                field_index: 0,
            })
            .instruction(&I::End);
        code.function(&body);
    }
    let mut module = Module::new();
    module
        .section(&types)
        .section(&functions)
        .section(&exports)
        .section(&code);
    let baseline = module.finish();
    let optimized = optimize(&baseline, None);
    assert!(optimized.len() < baseline.len());
    for wasm in [&baseline, &optimized] {
        let (mut store, instance) = instantiate(&engine, wasm, 0);
        for (i, expected) in [15, 19].into_iter().enumerate() {
            assert_eq!(
                instance
                    .get_typed_func::<(), i64>(&mut store, &format!("f{i}"))
                    .unwrap()
                    .call(&mut store, ())
                    .unwrap(),
                expected
            );
        }
    }
}
