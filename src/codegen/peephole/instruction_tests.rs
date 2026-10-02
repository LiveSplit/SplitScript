use super::{Passes, optimize};
use wasm_encoder::{
    BlockType, CodeSection, EntityType, ExportKind, ExportSection, Function, FunctionSection,
    ImportSection, Instruction as I, Module, TypeSection, ValType,
};

fn module(
    types: TypeSection,
    signature: u32,
    locals: &[(u32, ValType)],
    ops: &[I<'_>],
    imports: &[(&str, u32)],
) -> Vec<u8> {
    let mut imported = ImportSection::new();
    for &(name, ty) in imports {
        imported.import("env", name, EntityType::Function(ty));
    }
    let mut functions = FunctionSection::new();
    functions.function(signature);
    let mut exports = ExportSection::new();
    exports.export("run", ExportKind::Func, imports.len() as u32);
    let mut body = Function::new(locals.iter().copied());
    for op in ops {
        body.instruction(op);
    }
    let mut code = CodeSection::new();
    code.function(&body);
    let mut module = Module::new();
    module
        .section(&types)
        .section(&imported)
        .section(&functions)
        .section(&exports)
        .section(&code);
    module.finish()
}

fn numeric(params: &[ValType], ops: &[I<'_>]) -> Vec<u8> {
    let mut types = TypeSection::new();
    types.ty().function(params.iter().copied(), [ValType::I64]);
    module(types, 0, &[], ops, &[])
}

fn instructions(wasm: &[u8]) -> Vec<u8> {
    optimize(
        wasm,
        Passes {
            instructions: true,
            ..Default::default()
        },
    )
}

fn instantiate(
    engine: &wasmtime::Engine,
    wasm: &[u8],
) -> (wasmtime::Store<Vec<i64>>, wasmtime::Instance) {
    wasmparser::Validator::new_with_features(wasmparser::WasmFeatures::all())
        .validate_all(wasm)
        .unwrap();
    let module = wasmtime::Module::new(engine, wasm).unwrap();
    let mut linker = wasmtime::Linker::new(engine);
    linker
        .func_wrap(
            "env",
            "effect",
            |mut caller: wasmtime::Caller<'_, Vec<i64>>, value: i64| {
                caller.data_mut().push(value);
                value
            },
        )
        .unwrap();
    linker
        .func_wrap(
            "env",
            "condition",
            |mut caller: wasmtime::Caller<'_, Vec<i64>>, value: i32| {
                caller.data_mut().push(i64::from(value));
                value
            },
        )
        .unwrap();
    let mut store = wasmtime::Store::new(engine, Vec::new());
    let instance = linker.instantiate(&mut store, &module).unwrap();
    (store, instance)
}

#[test]
fn inverted_integer_comparisons_preserve_signedness_and_boundaries() {
    let engine = wasmtime::Engine::default();
    for (narrow, comparisons) in [
        (
            false,
            vec![
                I::I64Eq,
                I::I64Ne,
                I::I64LtS,
                I::I64LtU,
                I::I64LeS,
                I::I64LeU,
                I::I64GtS,
                I::I64GtU,
                I::I64GeS,
                I::I64GeU,
            ],
        ),
        (
            true,
            vec![
                I::I32Eq,
                I::I32Ne,
                I::I32LtS,
                I::I32LtU,
                I::I32LeS,
                I::I32LeU,
                I::I32GtS,
                I::I32GtU,
                I::I32GeS,
                I::I32GeU,
            ],
        ),
    ] {
        for comparison in comparisons {
            let mut ops = vec![I::LocalGet(0)];
            if narrow {
                ops.push(I::I32WrapI64);
            }
            ops.push(I::LocalGet(1));
            if narrow {
                ops.push(I::I32WrapI64);
            }
            ops.extend([comparison, I::I32Eqz, I::I64ExtendI32U, I::End]);
            let baseline = numeric(&[ValType::I64, ValType::I64], &ops);
            let optimized = instructions(&baseline);
            assert!(optimized.len() < baseline.len());
            let (mut a, ai) = instantiate(&engine, &baseline);
            let (mut b, bi) = instantiate(&engine, &optimized);
            let af = ai.get_typed_func::<(i64, i64), i64>(&mut a, "run").unwrap();
            let bf = bi.get_typed_func::<(i64, i64), i64>(&mut b, "run").unwrap();
            let values = [
                0,
                1,
                -1,
                i64::MIN,
                i64::MAX,
                i64::from(i32::MIN),
                i64::from(u32::MAX),
                1i64 << 32,
            ];
            for x in values {
                for y in values {
                    assert_eq!(
                        af.call(&mut a, (x, y)).unwrap(),
                        bf.call(&mut b, (x, y)).unwrap()
                    );
                }
            }
        }
    }
}

#[test]
fn boolean_and_conversion_rules_preserve_nonboolean_values_and_truncation() {
    let engine = wasmtime::Engine::default();
    let cases = [
        (
            vec![
                I::LocalGet(0),
                I::I64Eqz,
                I::I32Eqz,
                I::I32Eqz,
                I::I64ExtendI32U,
                I::End,
            ],
            true,
        ),
        (
            vec![
                I::LocalGet(0),
                I::I32WrapI64,
                I::I64ExtendI32S,
                I::I32WrapI64,
                I::I64ExtendI32U,
                I::End,
            ],
            true,
        ),
        (
            vec![
                I::LocalGet(0),
                I::I32WrapI64,
                I::I64ExtendI32U,
                I::I64Eqz,
                I::I64ExtendI32U,
                I::End,
            ],
            true,
        ),
        (
            vec![I::LocalGet(0), I::I32WrapI64, I::I64ExtendI32U, I::End],
            false,
        ),
        (
            vec![
                I::LocalGet(0),
                I::I32WrapI64,
                I::I32Eqz,
                I::I32Eqz,
                I::I64ExtendI32U,
                I::End,
            ],
            false,
        ),
        (
            vec![
                I::LocalGet(0),
                I::I32WrapI64,
                I::I32Eqz,
                I::I32Eqz,
                I::If(BlockType::Result(ValType::I64)),
                I::I64Const(5),
                I::Else,
                I::I64Const(7),
                I::End,
                I::End,
            ],
            true,
        ),
        (
            vec![
                I::I64Const(5),
                I::I64Const(7),
                I::LocalGet(0),
                I::I32WrapI64,
                I::I32Eqz,
                I::I32Eqz,
                I::Select,
                I::End,
            ],
            true,
        ),
        (
            vec![
                I::Block(BlockType::Result(ValType::I64)),
                I::I64Const(5),
                I::LocalGet(0),
                I::I32WrapI64,
                I::I32Eqz,
                I::I32Eqz,
                I::BrIf(0),
                I::Drop,
                I::I64Const(7),
                I::End,
                I::End,
            ],
            true,
        ),
    ];
    for (ops, shrinks) in cases {
        let baseline = numeric(&[ValType::I64], &ops);
        let optimized = instructions(&baseline);
        assert_eq!(optimized.len() < baseline.len(), shrinks);
        let (mut a, ai) = instantiate(&engine, &baseline);
        let (mut b, bi) = instantiate(&engine, &optimized);
        let af = ai.get_typed_func::<i64, i64>(&mut a, "run").unwrap();
        let bf = bi.get_typed_func::<i64, i64>(&mut b, "run").unwrap();
        for x in [0, 1, -1, 2, 42, i64::MIN, i64::MAX, 1i64 << 32] {
            assert_eq!(af.call(&mut a, x).unwrap(), bf.call(&mut b, x).unwrap());
        }
    }
}

#[test]
fn unordered_float_comparisons_are_not_inverted() {
    let engine = wasmtime::Engine::default();
    for comparison in [I::F64Lt, I::F64Le, I::F64Gt, I::F64Ge] {
        let baseline = numeric(
            &[ValType::F64, ValType::F64],
            &[
                I::LocalGet(0),
                I::LocalGet(1),
                comparison,
                I::I32Eqz,
                I::I64ExtendI32U,
                I::End,
            ],
        );
        let optimized = instructions(&baseline);
        assert_eq!(baseline, optimized);
        let (mut store, instance) = instantiate(&engine, &optimized);
        let run = instance
            .get_typed_func::<(f64, f64), i64>(&mut store, "run")
            .unwrap();
        for x in [0.0, -0.0, f64::INFINITY, f64::NEG_INFINITY, f64::NAN] {
            assert_eq!(run.call(&mut store, (f64::NAN, x)).unwrap(), 1);
            assert_eq!(run.call(&mut store, (x, f64::NAN)).unwrap(), 1);
        }
    }
}

#[test]
fn identical_arms_preserve_condition_effects_typed_parameters_and_branch_values() {
    let engine = wasmtime::Engine::default();
    let mut types = TypeSection::new();
    types
        .ty()
        .function([ValType::I32, ValType::I64], [ValType::I64, ValType::I32]);
    types
        .ty()
        .function([ValType::I64], [ValType::I64, ValType::I32]);
    types.ty().function([ValType::I32], [ValType::I32]);
    types.ty().function([ValType::I64], [ValType::I64]);
    let shared = [
        I::I64Const(3),
        I::I64Add,
        I::Call(1),
        I::I32Const(7),
        I::Br(0),
    ];
    let mut ops = vec![
        I::LocalGet(1),
        I::LocalGet(0),
        I::Call(0),
        I::If(BlockType::FunctionType(1)),
    ];
    ops.extend(shared.clone());
    ops.push(I::Else);
    ops.extend(shared);
    ops.extend([I::End, I::End]);
    let baseline = module(types, 0, &[], &ops, &[("condition", 2), ("effect", 3)]);
    let optimized = optimize(
        &baseline,
        Passes {
            control: true,
            ..Default::default()
        },
    );
    assert!(optimized.len() < baseline.len());
    for wasm in [&baseline, &optimized] {
        let (mut store, instance) = instantiate(&engine, wasm);
        let run = instance
            .get_typed_func::<(i32, i64), (i64, i32)>(&mut store, "run")
            .unwrap();
        for condition in [0, 1, -7] {
            for value in [0, i64::MAX] {
                store.data_mut().clear();
                let result = value.wrapping_add(3);
                assert_eq!(
                    run.call(&mut store, (condition, value)).unwrap(),
                    (result, 7)
                );
                assert_eq!(store.data(), &[i64::from(condition), result]);
            }
        }
    }
    let baseline = numeric(
        &[],
        &[
            I::I64Const(1),
            I::I64Const(0),
            I::I64DivU,
            I::I64Eqz,
            I::If(BlockType::Result(ValType::I64)),
            I::I64Const(7),
            I::I64Const(3),
            I::I64Add,
            I::Else,
            I::I64Const(7),
            I::I64Const(3),
            I::I64Add,
            I::End,
            I::End,
        ],
    );
    let optimized = optimize(
        &baseline,
        Passes {
            control: true,
            ..Default::default()
        },
    );
    assert!(optimized.len() < baseline.len());
    for wasm in [&baseline, &optimized] {
        let (mut store, instance) = instantiate(&engine, wasm);
        let error = instance
            .get_typed_func::<(), i64>(&mut store, "run")
            .unwrap()
            .call(&mut store, ())
            .unwrap_err();
        assert_eq!(
            error.downcast_ref::<wasmtime::Trap>(),
            Some(&wasmtime::Trap::IntegerDivisionByZero)
        );
    }
}

#[test]
fn swapping_nested_arms_preserves_the_selected_calls() {
    let engine = wasmtime::Engine::default();
    let mut types = TypeSection::new();
    types.ty().function([ValType::I64], [ValType::I64]);
    types
        .ty()
        .function([ValType::I32, ValType::I32], [ValType::I64]);
    let mut ops = vec![
        I::LocalGet(0),
        I::I32Eqz,
        I::If(BlockType::Result(ValType::I64)),
    ];
    for (yes, no) in [(10, 20), (30, 40)] {
        if yes == 30 {
            ops.push(I::Else);
        }
        ops.extend([
            I::LocalGet(1),
            I::I32Eqz,
            I::If(BlockType::Result(ValType::I64)),
            I::I64Const(yes),
            I::Call(0),
            I::Else,
            I::I64Const(no),
            I::Call(0),
            I::End,
        ]);
    }
    ops.extend([I::End, I::End]);
    let baseline = module(types, 1, &[], &ops, &[("effect", 0)]);
    let passes = Passes {
        control: true,
        ..Default::default()
    };
    let optimized = optimize(&optimize(&baseline, passes), passes);
    assert!(optimized.len() < baseline.len());
    for wasm in [&baseline, &optimized] {
        let (mut store, instance) = instantiate(&engine, wasm);
        let run = instance
            .get_typed_func::<(i32, i32), i64>(&mut store, "run")
            .unwrap();
        for a in [0, 1, -7] {
            for b in [0, 1, -7] {
                store.data_mut().clear();
                let expected = if a == 0 {
                    if b == 0 { 10 } else { 20 }
                } else if b == 0 {
                    30
                } else {
                    40
                };
                assert_eq!(run.call(&mut store, (a, b)).unwrap(), expected);
                assert_eq!(store.data(), &[expected]);
            }
        }
    }
}

#[test]
fn gc_null_checks_preserve_effect_and_trap_order() {
    use wasm_encoder::{FieldType, HeapType, RefType, StorageType};
    let mut config = wasmtime::Config::new();
    config.wasm_gc(true).wasm_function_references(true);
    let engine = wasmtime::Engine::new(&config).unwrap();
    // Struct writes, ordinary/packed array reads, and array writes. Guarded
    // operands either call the host or trap before the eventual GC operation.
    for flavor in 0..5 {
        for null in [false, true] {
            for guard in 0..3 {
                let packed = flavor >= 3;
                let mut types = TypeSection::new();
                types.ty().struct_([FieldType {
                    element_type: StorageType::Val(ValType::I64),
                    mutable: true,
                }]);
                types.ty().array(
                    &if packed {
                        StorageType::I8
                    } else {
                        StorageType::Val(ValType::I64)
                    },
                    true,
                );
                types.ty().function([], [ValType::I64]);
                types.ty().function([ValType::I64], [ValType::I64]);
                types.ty().function([ValType::I32], [ValType::I32]);
                let ty = u32::from(flavor != 0);
                let mut ops = Vec::new();
                if null {
                    ops.push(I::RefNull(HeapType::Concrete(ty)));
                } else if flavor == 0 {
                    ops.push(I::StructNewDefault(0));
                } else {
                    ops.extend([
                        if packed {
                            I::I32Const(255)
                        } else {
                            I::I64Const(9)
                        },
                        I::I32Const(1),
                        I::ArrayNew(1),
                    ]);
                }
                ops.extend([I::LocalSet(0), I::LocalGet(0), I::RefAsNonNull]);
                let wide = flavor == 0 || flavor == 2;
                if flavor == 2 {
                    ops.push(I::I32Const(0));
                }
                match (guard, wide) {
                    (0, true) => ops.push(I::I64Const(9)),
                    (0, false) => ops.push(I::I32Const(0)),
                    (1, true) => ops.extend([I::I64Const(9), I::Call(0)]),
                    (1, false) => ops.extend([I::I32Const(0), I::Call(1)]),
                    (_, true) => ops.extend([I::I64Const(1), I::I64Const(0), I::I64DivU]),
                    (_, false) => ops.extend([I::I32Const(1), I::I32Const(0), I::I32DivU]),
                }
                match flavor {
                    0 => ops.extend([
                        I::StructSet {
                            struct_type_index: 0,
                            field_index: 0,
                        },
                        I::LocalGet(0),
                        I::StructGet {
                            struct_type_index: 0,
                            field_index: 0,
                        },
                    ]),
                    1 => ops.push(I::ArrayGet(1)),
                    2 => ops.extend([
                        I::ArraySet(1),
                        I::LocalGet(0),
                        I::I32Const(0),
                        I::ArrayGet(1),
                    ]),
                    3 => ops.extend([I::ArrayGetS(1), I::I64ExtendI32S]),
                    _ => ops.extend([I::ArrayGetU(1), I::I64ExtendI32S]),
                }
                ops.push(I::End);
                let baseline = module(
                    types,
                    2,
                    &[(
                        1,
                        ValType::Ref(RefType {
                            nullable: true,
                            heap_type: HeapType::Concrete(ty),
                        }),
                    )],
                    &ops,
                    &[("effect", 3), ("condition", 4)],
                );
                let optimized = instructions(&baseline);
                let mut checks = 0;
                for payload in wasmparser::Parser::new(0).parse_all(&optimized) {
                    if let wasmparser::Payload::CodeSectionEntry(body) = payload.unwrap() {
                        checks += body
                            .get_operators_reader()
                            .unwrap()
                            .into_iter()
                            .filter(|op| matches!(op, Ok(wasmparser::Operator::RefAsNonNull)))
                            .count();
                    }
                }
                assert_eq!(checks, usize::from(guard != 0));
                for wasm in [&baseline, &optimized] {
                    let (mut store, instance) = instantiate(&engine, wasm);
                    let result = instance
                        .get_typed_func::<(), i64>(&mut store, "run")
                        .unwrap()
                        .call(&mut store, ());
                    if null || guard == 2 {
                        assert_eq!(
                            result.unwrap_err().downcast_ref::<wasmtime::Trap>(),
                            Some(&if null {
                                wasmtime::Trap::NullReference
                            } else {
                                wasmtime::Trap::IntegerDivisionByZero
                            })
                        );
                    } else {
                        assert_eq!(
                            result.unwrap(),
                            if flavor == 3 {
                                -1
                            } else if flavor == 4 {
                                255
                            } else {
                                9
                            }
                        );
                    }
                    let expected = if !null && guard == 1 {
                        vec![if wide { 9 } else { 0 }]
                    } else {
                        vec![]
                    };
                    assert_eq!(store.data(), &expected);
                }
            }
        }
    }
}

#[test]
fn nonnull_constructor_results_do_not_need_another_assertion() {
    use wasm_encoder::{FieldType, StorageType};
    let mut config = wasmtime::Config::new();
    config.wasm_gc(true).wasm_function_references(true);
    let engine = wasmtime::Engine::new(&config).unwrap();
    for array in [false, true] {
        let mut types = TypeSection::new();
        if array {
            types.ty().array(&StorageType::Val(ValType::I64), true);
        } else {
            types.ty().struct_([FieldType {
                element_type: StorageType::Val(ValType::I64),
                mutable: true,
            }]);
        }
        types.ty().function([], [ValType::I64]);
        let mut ops = if array {
            vec![I::I32Const(1), I::ArrayNewDefault(0)]
        } else {
            vec![I::StructNewDefault(0)]
        };
        ops.extend([I::RefAsNonNull, I::RefIsNull, I::I64ExtendI32U, I::End]);
        let baseline = module(types, 1, &[], &ops, &[]);
        let optimized = instructions(&baseline);
        assert!(optimized.len() < baseline.len());
        for wasm in [&baseline, &optimized] {
            let (mut store, instance) = instantiate(&engine, wasm);
            assert_eq!(
                instance
                    .get_typed_func::<(), i64>(&mut store, "run")
                    .unwrap()
                    .call(&mut store, ())
                    .unwrap(),
                0
            );
        }
    }
}
