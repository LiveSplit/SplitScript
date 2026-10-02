use wasm_encoder::{
    BlockType, CodeSection, ConstExpr, ElementSection, Elements, ExportKind, ExportSection,
    Function, FunctionSection, Instruction as I, Module, TypeSection, ValType,
};

use super::{Output, inline, read_bodies};
use crate::codegen::optimization::optimize;
use wasm_encoder::reencode::Reencode;

fn function(locals: &[ValType], instructions: &[I<'_>]) -> Function {
    let mut function = Function::new_with_locals_types(locals.iter().copied());
    for instruction in instructions {
        function.instruction(instruction);
    }
    function
}

fn module(params: &[ValType], functions: &[Function], referenced: bool) -> Vec<u8> {
    let mut types = TypeSection::new();
    types.ty().function(params.iter().copied(), [ValType::I32]);
    let mut declarations = FunctionSection::new();
    let mut code = CodeSection::new();
    for function in functions {
        declarations.function(0);
        code.function(function);
    }
    let mut exports = ExportSection::new();
    exports.export("run", ExportKind::Func, functions.len() as u32 - 1);
    let mut module = Module::new();
    module
        .section(&types)
        .section(&declarations)
        .section(&exports);
    if referenced {
        let mut elements = ElementSection::new();
        elements.declared(Elements::Expressions(
            wasm_encoder::RefType::FUNCREF,
            std::borrow::Cow::Owned(vec![ConstExpr::ref_func(0)]),
        ));
        module.section(&elements);
    }
    module.section(&code);
    module.finish()
}

fn validate(wasm: &[u8]) {
    wasmparser::Validator::new_with_features(wasmparser::WasmFeatures::all())
        .validate_all(wasm)
        .unwrap();
}

fn run(wasm: &[u8]) -> i32 {
    validate(wasm);
    let mut config = wasmtime::Config::new();
    config.wasm_function_references(true);
    config.wasm_gc(true);
    let engine = wasmtime::Engine::new(&config).unwrap();
    let module = wasmtime::Module::new(&engine, wasm).unwrap();
    let mut store = wasmtime::Store::new(&engine, ());
    let instance = wasmtime::Instance::new(&mut store, &module, &[]).unwrap();
    instance
        .get_typed_func::<(), i32>(&mut store, "run")
        .unwrap()
        .call(&mut store, ())
        .unwrap()
}

#[test]
fn accepts_only_strict_savings_and_updates_report_indices() {
    let wasm = module(
        &[],
        &[
            function(&[], &[I::I32Const(42), I::End]),
            function(&[], &[I::Call(0), I::End]),
        ],
        false,
    );
    let mut report = crate::codegen::CodegenReport {
        functions: vec![(0, "helper".into()), (1, "run".into())],
        ..Default::default()
    };
    let optimized = optimize(wasm.clone(), Some(&mut report));
    assert!(optimized.len() < wasm.len());
    assert_eq!(report.functions, vec![(0, "run".into())]);
    assert_eq!(report.inlined_functions, ["helper"]);
    assert_eq!(run(&wasm), 42);
    assert_eq!(run(&optimized), 42);

    let wasm = module(
        &[ValType::I32; 3],
        &[
            function(
                &[],
                &[
                    I::LocalGet(0),
                    I::LocalGet(1),
                    I::I32Add,
                    I::LocalGet(2),
                    I::I32Add,
                    I::End,
                ],
            ),
            function(
                &[],
                &[
                    I::LocalGet(0),
                    I::I32Const(0),
                    I::I32Add,
                    I::LocalGet(1),
                    I::I32Const(0),
                    I::I32Add,
                    I::LocalGet(2),
                    I::I32Const(0),
                    I::I32Add,
                    I::Call(0),
                    I::End,
                ],
            ),
        ],
        false,
    );
    validate(&wasm);
    let optimized = optimize(wasm.clone(), None);
    assert!(optimized.len() < wasm.len());
    for bytes in [&wasm, &optimized] {
        let engine = wasmtime::Engine::default();
        let module = wasmtime::Module::new(&engine, bytes).unwrap();
        let mut store = wasmtime::Store::new(&engine, ());
        let instance = wasmtime::Instance::new(&mut store, &module, &[]).unwrap();
        let run = instance
            .get_typed_func::<(i32, i32, i32), i32>(&mut store, "run")
            .unwrap();
        assert_eq!(run.call(&mut store, (7, 11, 13)).unwrap(), 31);
    }
}

#[test]
fn preserves_early_return_and_branches_to_function_label() {
    for instructions in [
        vec![
            I::I32Const(1),
            I::If(BlockType::Empty),
            I::I32Const(42),
            I::Return,
            I::End,
            I::I32Const(9),
            I::End,
        ],
        vec![I::I32Const(42), I::Br(0), I::I32Const(9), I::End],
        vec![
            I::I32Const(42),
            I::I32Const(1),
            I::BrTable(std::borrow::Cow::Owned(vec![0]), 0),
            I::End,
        ],
    ] {
        let wasm = module(
            &[],
            &[
                function(&[], &instructions),
                function(&[], &[I::Call(0), I::I32Const(1), I::I32Add, I::End]),
            ],
            false,
        );
        let optimized = optimize(wasm.clone(), None);
        assert!(optimized.len() < wasm.len());
        assert_eq!(run(&wasm), 43);
        assert_eq!(run(&optimized), 43);
    }
}

#[test]
fn refuses_shared_referenced_exported_and_recursive_functions() {
    let helper = function(&[], &[I::I32Const(42), I::End]);
    for wasm in [
        module(
            &[],
            &[
                helper.clone(),
                function(&[], &[I::Call(0), I::Call(0), I::I32Add, I::End]),
            ],
            false,
        ),
        module(&[], &[helper, function(&[], &[I::Call(0), I::End])], true),
        module(&[], &[function(&[], &[I::Call(0), I::End])], false),
        module(
            &[],
            &[
                function(&[], &[I::Call(1), I::End]),
                function(&[], &[I::Call(0), I::End]),
                function(&[], &[I::I32Const(0), I::End]),
            ],
            false,
        ),
    ] {
        validate(&wasm);
        assert_eq!(optimize(wasm.clone(), None), wasm);
    }
}

#[test]
fn handles_nested_single_use_calls_deterministically() {
    let wasm = module(
        &[],
        &[
            function(&[], &[I::Call(1), I::End]),
            function(&[], &[I::I32Const(42), I::End]),
            function(&[], &[I::Call(0), I::End]),
        ],
        false,
    );
    let optimized = optimize(wasm.clone(), None);
    assert_eq!(optimized, optimize(wasm, None));
    assert_eq!(read_bodies(&optimized).unwrap().len(), 1);
    assert_eq!(run(&optimized), 42);
}

#[test]
fn fresh_locals_inside_loop_and_local_index_leb_boundary() {
    // A long body makes the removed body's length prefix large enough that
    // resetting its single local still wins. The caller crosses local 127.
    let mut instructions = vec![I::Nop; 128];
    instructions.extend([
        I::LocalGet(0),
        I::I32Const(1),
        I::I32Add,
        I::LocalTee(0),
        I::End,
    ]);
    let wasm = module(
        &[],
        &[
            function(&[ValType::I32], &instructions),
            function(
                &[ValType::I32; 128],
                &[
                    I::Loop(BlockType::Empty),
                    I::Call(0),
                    I::LocalGet(0),
                    I::I32Add,
                    I::LocalSet(0),
                    I::LocalGet(1),
                    I::I32Const(1),
                    I::I32Add,
                    I::LocalTee(1),
                    I::I32Const(3),
                    I::I32LtU,
                    I::BrIf(0),
                    I::End,
                    I::LocalGet(0),
                    I::End,
                ],
            ),
        ],
        false,
    );
    let optimized = optimize(wasm.clone(), None);
    assert!(optimized.len() < wasm.len());
    assert_eq!(run(&wasm), 3);
    assert_eq!(run(&optimized), 3);
}

#[test]
fn remaps_parameters_and_preserves_argument_order_and_traps() {
    // Exercise the substitution independently of profitability: this ABI is
    // deliberately too expensive to inline, but the rewrite must remain sound.
    let wasm = module(
        &[ValType::I32; 2],
        &[
            function(
                &[ValType::I32],
                &[
                    I::LocalGet(0),
                    I::LocalGet(1),
                    I::I32DivS,
                    I::LocalSet(2),
                    I::LocalGet(2),
                    I::End,
                ],
            ),
            function(&[], &[I::LocalGet(0), I::LocalGet(1), I::Call(0), I::End]),
        ],
        false,
    );
    let mut bodies = read_bodies(&wasm).unwrap();
    let (bytes, locals) =
        inline(bodies[1].as_ref().unwrap(), bodies[0].as_ref().unwrap(), 0).unwrap();
    let caller = bodies[1].as_mut().unwrap();
    caller.bytes = bytes;
    caller.locals = locals;
    bodies[0] = None;
    let mut rewritten = Module::new();
    Output {
        bodies: &bodies,
        indices: &[None, Some(0)],
    }
    .parse_core_module(&mut rewritten, wasmparser::Parser::new(0), &wasm)
    .unwrap();
    for wasm in [wasm, rewritten.finish()] {
        validate(&wasm);
        let engine = wasmtime::Engine::default();
        let module = wasmtime::Module::new(&engine, wasm).unwrap();
        let mut store = wasmtime::Store::new(&engine, ());
        let instance = wasmtime::Instance::new(&mut store, &module, &[]).unwrap();
        let function = instance
            .get_typed_func::<(i32, i32), i32>(&mut store, "run")
            .unwrap();
        assert_eq!(function.call(&mut store, (84, 2)).unwrap(), 42);
        assert!(function.call(&mut store, (84, 0)).is_err());
    }
}

#[test]
fn fresh_function_reference_locals_use_the_function_null_hierarchy() {
    let wasm = module(
        &[],
        &[
            function(
                &[ValType::Ref(wasm_encoder::RefType {
                    nullable: true,
                    heap_type: wasm_encoder::HeapType::Concrete(0),
                })],
                &[I::LocalGet(0), I::RefIsNull, I::End],
            ),
            function(&[], &[I::Call(0), I::End]),
        ],
        false,
    );
    // Validate the reset even when the strict size gate would reject this
    // tiny body: GC `none` is not assignable to a function-reference local.
    let mut bodies = read_bodies(&wasm).unwrap();
    let (bytes, locals) =
        inline(bodies[1].as_ref().unwrap(), bodies[0].as_ref().unwrap(), 0).unwrap();
    bodies[1].as_mut().unwrap().bytes = bytes;
    bodies[1].as_mut().unwrap().locals = locals;
    bodies[0] = None;
    let mut rewritten = Module::new();
    Output {
        bodies: &bodies,
        indices: &[None, Some(0)],
    }
    .parse_core_module(&mut rewritten, wasmparser::Parser::new(0), &wasm)
    .unwrap();
    assert_eq!(run(&wasm), 1);
    assert_eq!(run(&rewritten.finish()), 1);
}

#[test]
fn multiple_results_and_early_returns_keep_tuple_order() {
    let mut types = TypeSection::new();
    types
        .ty()
        .function([ValType::I32], [ValType::I32, ValType::I64]);
    let mut funcs = FunctionSection::new();
    funcs.function(0);
    funcs.function(0);
    let mut exports = ExportSection::new();
    exports.export("run", ExportKind::Func, 1);
    let mut code = CodeSection::new();
    code.function(&function(
        &[],
        &[
            I::LocalGet(0),
            I::If(BlockType::Empty),
            I::I32Const(7),
            I::I64Const(11),
            I::Return,
            I::End,
            I::I32Const(13),
            I::I64Const(17),
            I::End,
        ],
    ));
    code.function(&function(&[], &[I::LocalGet(0), I::Call(0), I::End]));
    let mut module = Module::new();
    module
        .section(&types)
        .section(&funcs)
        .section(&exports)
        .section(&code);
    let before = module.finish();
    let after = crate::codegen::optimization::optimize(before.clone(), None);
    let (expanded, _) = super::expand(&before);
    assert_eq!(read_bodies(&expanded).unwrap().len(), 1);
    assert!(after.len() <= before.len());
    let engine = wasmtime::Engine::default();
    for wasm in [&before, &expanded, &after] {
        validate(wasm);
        let module = wasmtime::Module::new(&engine, wasm).unwrap();
        let mut store = wasmtime::Store::new(&engine, ());
        let instance = wasmtime::Instance::new(&mut store, &module, &[]).unwrap();
        let run = instance
            .get_typed_func::<i32, (i32, i64)>(&mut store, "run")
            .unwrap();
        assert_eq!(run.call(&mut store, 0).unwrap(), (13, 17));
        assert_eq!(run.call(&mut store, 1).unwrap(), (7, 11));
    }
}

#[test]
fn inlining_graph_ignores_calls_erased_by_constant_cleanup() {
    // Function 0's dead branch calls function 2. Inlining 0 into 1 must not
    // leave a stale call edge when function 2 is considered later.
    let before = module(
        &[],
        &[
            function(
                &[],
                &[
                    I::I32Const(0),
                    I::If(BlockType::Result(ValType::I32)),
                    I::Call(2),
                    I::Else,
                    I::I32Const(42),
                    I::End,
                    I::End,
                ],
            ),
            function(&[], &[I::Call(0), I::End]),
            function(&[], &[I::I32Const(7), I::End]),
            function(&[], &[I::Call(1), I::End]),
        ],
        false,
    );
    let (after, _) = super::expand(&before);
    assert_eq!(run(&before), 42);
    assert_eq!(run(&after), 42);
}
