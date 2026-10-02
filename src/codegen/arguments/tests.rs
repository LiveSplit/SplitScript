use super::specialize;
use wasm_encoder::{
    BlockType, CodeSection, ConstExpr, ElementSection, Elements, EntityType, ExportKind,
    ExportSection, Function, FunctionSection, ImportSection, Instruction as I, Module, RefType,
    TypeSection, ValType,
};

fn module(helper: &[I<'_>], caller: &[I<'_>], protection: u8) -> Vec<u8> {
    let mut types = TypeSection::new();
    types
        .ty()
        .function([ValType::I64, ValType::I32, ValType::I64], [ValType::I64]);
    types.ty().function([ValType::I64], [ValType::I64]);
    let mut imports = ImportSection::new();
    imports.import("env", "effect", EntityType::Function(1));
    let mut functions = FunctionSection::new();
    functions.function(0).function(1);
    let mut exports = ExportSection::new();
    exports.export("run", ExportKind::Func, 2);
    if protection == 1 {
        exports.export("helper", ExportKind::Func, 1);
    }
    let mut elements = ElementSection::new();
    if protection == 2 {
        elements.declared(Elements::Expressions(
            RefType::FUNCREF,
            vec![ConstExpr::ref_func(1)].into(),
        ));
    }
    let mut code = CodeSection::new();
    for ops in [helper, caller] {
        let mut function = Function::new([(1, ValType::I64)]);
        for op in ops {
            function.instruction(op);
        }
        code.function(&function);
    }
    let mut module = Module::new();
    module
        .section(&types)
        .section(&imports)
        .section(&functions)
        .section(&exports);
    if protection == 2 {
        module.section(&elements);
    }
    module.section(&code);
    module.finish()
}

fn run(wasm: &[u8], input: i64) -> (Result<i64, String>, Vec<i64>) {
    wasmparser::Validator::new_with_features(wasmparser::WasmFeatures::all())
        .validate_all(wasm)
        .unwrap();
    let mut config = wasmtime::Config::new();
    config.wasm_function_references(true);
    let engine = wasmtime::Engine::new(&config).unwrap();
    let module = wasmtime::Module::new(&engine, wasm).unwrap();
    let mut linker = wasmtime::Linker::new(&engine);
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
    let mut store = wasmtime::Store::new(&engine, Vec::new());
    let instance = linker.instantiate(&mut store, &module).unwrap();
    let result = instance
        .get_typed_func::<i64, i64>(&mut store, "run")
        .unwrap()
        .call(&mut store, input)
        .map_err(|_| "trap".to_owned());
    (result, store.into_data())
}

fn parameters(wasm: &[u8]) -> Vec<usize> {
    let mut types = Vec::new();
    let mut params = Vec::new();
    for payload in wasmparser::Parser::new(0).parse_all(wasm) {
        match payload.unwrap() {
            wasmparser::Payload::TypeSection(section) => {
                for ty in section.into_iter_err_on_gc_types() {
                    types.push(ty.unwrap().params().len());
                }
            }
            wasmparser::Payload::FunctionSection(section) => {
                for ty in section {
                    params.push(types[ty.unwrap() as usize]);
                }
            }
            _ => {}
        }
    }
    params
}

#[test]
fn constants_and_unused_parameters_preserve_effects_and_local_indices() {
    let before = module(
        &[
            I::I64Const(19),
            I::Call(0),
            I::LocalSet(2),
            I::LocalGet(0),
            I::LocalSet(3),
            I::LocalGet(1),
            I::I64ExtendI32U,
            I::LocalGet(3),
            I::I64Add,
            I::End,
        ],
        &[
            I::LocalGet(0),
            I::I32Const(10),
            I::I64Const(7),
            I::Call(1),
            I::LocalSet(1),
            I::LocalGet(0),
            I::I32Const(10),
            I::I64Const(9),
            I::Call(1),
            I::LocalGet(1),
            I::I64Add,
            I::End,
        ],
        0,
    );
    let after = specialize(&before);
    assert_eq!(parameters(&after), [1, 1]);
    for input in [-9, 0, 123] {
        assert_eq!(run(&before, input), (Ok((input + 10) * 2), vec![19, 19]));
        assert_eq!(run(&after, input), run(&before, input));
    }
}

#[test]
fn overwritten_constants_are_initialized_on_every_activation() {
    let before = module(
        &[
            I::LocalGet(1),
            I::I32Const(1),
            I::I32Add,
            I::LocalSet(1),
            I::LocalGet(0),
            I::I64Eqz,
            I::If(BlockType::Empty),
            I::I32Const(99),
            I::LocalSet(1),
            I::End,
            I::LocalGet(1),
            I::I64ExtendI32U,
            I::End,
        ],
        &[
            I::LocalGet(0),
            I::I32Const(10),
            I::I64Const(7),
            I::Call(1),
            I::LocalSet(1),
            I::I64Const(0),
            I::I32Const(10),
            I::I64Const(9),
            I::Call(1),
            I::LocalGet(1),
            I::I64Add,
            I::End,
        ],
        0,
    );
    let after = specialize(&before);
    assert_eq!(parameters(&after), [1, 1]);
    for input in [0, 1, -1] {
        assert_eq!(run(&after, input), run(&before, input));
    }
    assert_eq!(run(&after, 1).0, Ok(110));
}

#[test]
fn differing_constants_and_unknown_argument_expressions_are_not_assumed_equal() {
    let helper = [
        I::LocalGet(0),
        I::LocalGet(1),
        I::I64ExtendI32U,
        I::I64Add,
        I::End,
    ];
    let before = module(
        &helper,
        &[
            I::LocalGet(0),
            I::I32Const(10),
            I::I64Const(7),
            I::Call(1),
            I::Drop,
            I::LocalGet(0),
            I::I32Const(11),
            I::I64Const(9),
            I::Call(1),
            I::End,
        ],
        0,
    );
    let after = specialize(&before);
    assert_eq!(parameters(&after), [2, 1]);
    assert_eq!(run(&after, 5), run(&before, 5));
    // The final, unused argument still calls the host and may trap first.
    let before = module(
        &helper,
        &[
            I::LocalGet(0),
            I::I32Const(10),
            I::I64Const(17),
            I::Call(0),
            I::I64Const(100),
            I::LocalGet(0),
            I::I64DivS,
            I::I64Add,
            I::Call(1),
            I::End,
        ],
        0,
    );
    let after = specialize(&before);
    assert_eq!(parameters(&after), [3, 1]);
    for input in [0, 2] {
        assert_eq!(run(&after, input), run(&before, input));
    }
    assert_eq!(run(&after, 0), (Err("trap".into()), vec![17]));
}

#[test]
fn exports_references_and_tail_calls_pin_signatures() {
    let helper = [I::LocalGet(1), I::I64ExtendI32U, I::End];
    let caller = [
        I::LocalGet(0),
        I::I32Const(10),
        I::I64Const(7),
        I::Call(1),
        I::End,
    ];
    for protection in [1, 2] {
        let before = module(&helper, &caller, protection);
        let after = specialize(&before);
        assert_eq!(before, after);
        assert_eq!(run(&after, 123).0, Ok(10));
    }
    let before = module(
        &helper,
        &[
            I::LocalGet(0),
            I::I32Const(10),
            I::I64Const(7),
            I::ReturnCall(1),
            I::End,
        ],
        0,
    );
    assert_eq!(specialize(&before), before);
}

#[test]
fn a_constant_before_an_effectful_argument_can_specialize_without_moving_effects() {
    let before = module(
        &[
            I::LocalGet(0),
            I::LocalGet(1),
            I::I64ExtendI32U,
            I::I64Add,
            I::LocalGet(2),
            I::I64Add,
            I::End,
        ],
        &[
            I::LocalGet(0),
            I::I32Const(10),
            I::LocalGet(0),
            I::Call(0),
            I::Call(1),
            I::End,
        ],
        0,
    );
    let after = specialize(&before);
    assert_eq!(parameters(&after), [2, 1]);
    for input in [0, 7, -91] {
        assert_eq!(run(&after, input), (Ok(input * 2 + 10), vec![input]));
        assert_eq!(run(&after, input), run(&before, input));
    }
}

#[test]
fn recursive_calls_keep_the_same_specialized_signature() {
    let before = module(
        &[
            I::LocalGet(0),
            I::I64Eqz,
            I::If(BlockType::Result(ValType::I64)),
            I::LocalGet(1),
            I::I64ExtendI32U,
            I::Else,
            I::LocalGet(0),
            I::I64Const(1),
            I::I64Sub,
            I::I32Const(10),
            I::I64Const(7),
            I::Call(1),
            I::I64Const(1),
            I::I64Add,
            I::End,
            I::End,
        ],
        &[
            I::LocalGet(0),
            I::I32Const(10),
            I::I64Const(7),
            I::Call(1),
            I::End,
        ],
        0,
    );
    let after = specialize(&before);
    assert_eq!(parameters(&after), [1, 1]);
    for input in [0, 1, 10] {
        assert_eq!(run(&after, input).0, Ok(10 + input));
        assert_eq!(run(&after, input), run(&before, input));
    }
}
