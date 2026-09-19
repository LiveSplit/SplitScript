//! failure semantics integration tests.

use super::*;

fn fixed_array_sizes(wasm: &[u8]) -> Vec<u32> {
    let mut sizes = Vec::new();
    for payload in Parser::new(0).parse_all(wasm) {
        let Ok(Payload::CodeSectionEntry(body)) = payload else {
            continue;
        };
        let mut operators = body
            .get_operators_reader()
            .expect("generated function bodies contain readable operators");
        while !operators.eof() {
            if let wasmparser::Operator::ArrayNewFixed { array_size, .. } =
                operators.read().expect("generated instructions decode")
            {
                sizes.push(array_size);
            }
        }
    }
    sizes
}

#[test]
fn error_payloads_are_materialized_once_per_result_layout_when_any_use_observes_them() {
    let message = "payload-demand-marker-".repeat(19);
    let discarded = format!(
        r#"
            state "game.exe" {{}}

            fn fail() -> u32! {{
                throw "{message}"
            }}

            whileAttached {{
                let value = fail() else 0
                print(value)
            }}
        "#
    );
    let discarded = splitscript::compile(&discarded).expect("discarded errors should compile");
    Validator::new_with_features(WasmFeatures::all())
        .validate_all(&discarded)
        .expect("erased error payloads must preserve the Result layout");
    assert!(
        !fixed_array_sizes(&discarded).contains(&(message.len() as u32)),
        "an error used only through `else` must not retain its literal payload"
    );

    let mixed = format!(
        r#"
            state "game.exe" {{}}

            fn fail() -> u32! {{
                throw "{message}"
            }}

            fn inspectFailure() {{
                let text = match fail() {{
                    Err(error) => error,
                    Ok(value) => value as String,
                }}
                print(text)
            }}

            whileAttached {{
                let value = fail() else 0
                print(value)
                inspectFailure()
            }}
        "#
    );
    let mixed = splitscript::compile(&mixed).expect("mixed error uses should compile");
    Validator::new_with_features(WasmFeatures::all())
        .validate_all(&mixed)
        .expect("materialized error payloads must validate");
    assert!(
        fixed_array_sizes(&mixed).contains(&(message.len() as u32)),
        "one observing caller must retain the shared function's payload for every caller"
    );
}

#[test]
fn observed_outer_results_retain_payloads_forwarded_by_question_mark() {
    let message = "propagated-payload-marker-".repeat(17);
    let source = format!(
        r#"
            state "game.exe" {{}}

            fn inner() -> u32! {{
                throw "{message}"
            }}

            fn outer() -> bool! {{
                inner()?
                return true
            }}

            whileAttached {{
                let text = match outer() {{
                    Err(error) => error,
                    Ok(value) => value as String,
                }}
                print(text)
            }}
        "#
    );
    let wasm = splitscript::compile(&source).expect("propagated errors should compile");
    Validator::new_with_features(WasmFeatures::all())
        .validate_all(&wasm)
        .expect("propagated payload demand must produce valid WebAssembly");
    assert!(
        fixed_array_sizes(&wasm).contains(&(message.len() as u32)),
        "payload demand must flow backward through `?` across different Result layouts"
    );
}

#[test]
fn process_selection_is_a_fallible_boolean_boundary() {
    let source = r#"
        state "game.exe" {
            marker: u8 at 0x1000
        }

        selectProcess {
            return process.read<u8>(0x2000)? == 42
        }
    "#;
    let checked = splitscript::check(splitscript::parse(source).unwrap())
        .expect("a selector may propagate a candidate-specific process error");
    let result = checked
        .semantics()
        .action_result(splitscript::compiler::ast::ActionKind::SelectProcess)
        .expect("the selector has an ABI result");
    let TypeKind::Result { value, .. } = checked.semantics().types().kind(result) else {
        panic!("selectProcess must use the ordinary Result representation");
    };
    assert_eq!(
        checked.semantics().types().kind(*value),
        &TypeKind::Builtin(splitscript::compiler::types::BuiltinType::Bool)
    );
    Validator::new_with_features(WasmFeatures::all())
        .validate_all(&splitscript::codegen(&checked))
        .expect("fallible process selection should produce valid WebAssembly");

    let unity = r#"
        state Unity ["game.exe"] {}
        selectProcess {
            let path = process.path()?
            return path.endsWith("game.exe")
        }
    "#;
    let unity = splitscript::check(splitscript::parse(unity).unwrap())
        .expect("provider setup must not hide the native candidate process");
    Validator::new_with_features(WasmFeatures::all())
        .validate_all(&splitscript::codegen(&unity))
        .expect("provider-backed process selection should produce valid WebAssembly");

    let invalid = r#"
        state "game.exe" {}
        selectProcess { return None }
    "#;
    let diagnostics =
        splitscript::compile(invalid).expect_err("None is not a process-selection decision");
    assert!(diagnostics.iter().any(|diagnostic| {
        diagnostic.message.contains("expected") && diagnostic.message.contains("bool")
    }));
}

#[test]
fn on_attach_is_a_fallible_boundary_without_changing_its_success_type() {
    let source = r#"
        state "game.exe" {}

        onAttach {
            let marker = process.read<u8>(0x1000)?
            if marker != 7 {
                throw "unsupported process build"
            }
        }
    "#;
    let checked = splitscript::check(splitscript::parse(source).unwrap())
        .expect("attachment initialization may reject its acquired process");
    let result = checked
        .semantics()
        .action_result(splitscript::compiler::ast::ActionKind::OnAttach)
        .expect("onAttach has a semantic success type");
    assert_eq!(
        checked.semantics().types().kind(result),
        &TypeKind::Builtin(splitscript::compiler::types::BuiltinType::None),
        "the implicit failure boundary must not become a nested public result"
    );
    Validator::new_with_features(WasmFeatures::all())
        .validate_all(&splitscript::codegen(&checked))
        .expect("fallible attachment initialization should produce valid WebAssembly");

    let conditional_shape = r#"
        enum Build { Full, Demo }
        let build: Build
        state "game.exe" {
            if build == Build.Full {
                value: u8 at 0x1000;
            } else {
                value: u8 at 0x2000;
            }
        }

        onAttach {
            let marker = process.read<u8>(0x3000)?
            if marker == 1 {
                build = Build.Full
                return
            }
            build = Build.Demo
        }
    "#;
    let checked = splitscript::check(splitscript::parse(conditional_shape).unwrap())
        .expect("rejection paths do not need to initialize a successful shape");
    Validator::new_with_features(WasmFeatures::all())
        .validate_all(&splitscript::codegen(&checked))
        .expect("fallible explicit shape selection should produce valid WebAssembly");

    let attachment_global = r#"
        let executable: Module
        state "game.exe" {}

        onAttach {
            process.read<u8>(0x1000)?
            executable = await process.mainModule()
        }

        whileAttached {
            print(executable.address)
        }
    "#;
    splitscript::compile(attachment_global)
        .expect("a rejected path is terminal and need not initialize successful attachment state");
}

#[test]
fn process_selection_exposes_only_the_synchronous_native_candidate_context() {
    let attachment_global = r#"
        let module: Module
        state "game.exe" {}
        selectProcess { return module.address != 0 }
        onAttach { module = await process.mainModule() }
    "#;
    let diagnostics = splitscript::compile(attachment_global)
        .expect_err("candidate selection runs before attachment globals are initialized");
    assert!(diagnostics.iter().any(|diagnostic| {
        diagnostic.message == "attachment-scoped global `module` is unavailable in `selectProcess`"
    }));

    let provider_context = r#"
        state Unity ["game.exe"] {}
        selectProcess {
            unity.scenes.active()
            return true
        }
    "#;
    let diagnostics = splitscript::compile(provider_context)
        .expect_err("Unity provider context does not exist before candidate acceptance");
    assert!(diagnostics.iter().any(|diagnostic| {
        diagnostic.message.contains("unknown") && diagnostic.message.contains("unity")
    }));

    let suspension = r#"
        state "game.exe" {}
        selectProcess {
            await process.mainModule()
            return true
        }
    "#;
    let diagnostics = splitscript::compile(suspension)
        .expect_err("candidate selection must finish synchronously");
    assert!(
        diagnostics.iter().any(|diagnostic| {
            diagnostic.message.contains("onAttach")
                && (diagnostic.message.contains("await")
                    || diagnostic.message.contains("mainModule"))
        }),
        "{diagnostics:?}"
    );
}

#[test]
fn bounded_native_string_reads_are_fallible_and_state_sugar_infers_string() {
    use splitscript::compiler::{
        ast::{StateMemoryDecoder, StateSource},
        stdlib::StdlibTypeId,
        types::TypeKind,
    };

    let source = r#"
        state "game.exe" {
            mapName at "game.dll", 0x100, 0x20 as utf8(32);
            chapterName at 0x200 as utf16le(64)
        }

        whileAttached {
            let direct: String! = process.readUtf8(0x2000, 32)
            let wide: String! = process.readUtf16Le(0x3000, 64)
            print(current.mapName)
            print(current.chapterName)
        }
    "#;
    let checked = splitscript::check(splitscript::parse(source).unwrap()).unwrap();
    let field = &checked.syntax().state.as_ref().unwrap().fields[0];
    let StateSource::Pointer(path) = &field.source else {
        panic!("expected a pointer-backed state field");
    };
    assert!(matches!(
        path.decoder,
        Some(StateMemoryDecoder::Utf8 { max_bytes: 32, .. })
    ));
    let StateSource::Pointer(wide_path) =
        &checked.syntax().state.as_ref().unwrap().fields[1].source
    else {
        panic!("expected a pointer-backed UTF-16LE state field");
    };
    assert!(matches!(
        wide_path.decoder,
        Some(StateMemoryDecoder::Utf16Le { max_units: 64, .. })
    ));
    let field_type = checked.semantics().value_type(field.id).unwrap();
    assert_eq!(
        checked.semantics().types().kind(field_type),
        &TypeKind::Standard(StdlibTypeId::String)
    );
    Validator::new_with_features(WasmFeatures::all())
        .validate_all(&splitscript::codegen(&checked))
        .expect("bounded UTF-8 reads after a pointer path should validate");

    for (source, expected) in [
        (
            "state \"game.exe\" { name at 0x100 as utf8(0) }",
            "must allow at least one byte",
        ),
        (
            "state \"game.exe\" { name at 0x100 as utf8(4097) }",
            "limited to 4096 bytes",
        ),
        (
            "state \"game.exe\" { name at 0x100 as utf16le(0) }",
            "must allow at least one code unit",
        ),
        (
            "state \"game.exe\" { name at 0x100 as utf16le(2049) }",
            "limited to 2048 code units",
        ),
        (
            "state GBA { name at 0x02000000 as utf8(32) }",
            "does not yet support decoded string fields",
        ),
    ] {
        let errors = splitscript::check(splitscript::parse(source).unwrap()).unwrap_err();
        assert!(
            errors.iter().any(|error| error.message.contains(expected)),
            "missing `{expected}` diagnostic in {errors:#?}"
        );
    }
}

#[test]
fn explicitly_optional_pointer_fields_observe_read_failure_as_none() {
    use splitscript::compiler::{
        stdlib::StdlibTypeId,
        types::{BuiltinType, TypeKind},
    };

    let source = r#"
        state "game.exe" {
            scalar: i32? at 0x1000;
            mapName: String? at 0x2000 as utf8(32);
            chapterName: String? at 0x3000 as utf16le(32)
        }

        whileAttached {
            print(match current.scalar {
                Some(value) => value as String,
                None => "missing"
            })
            print(match current.mapName {
                Some(value) => value,
                None => "missing"
            })
            print(match current.chapterName {
                Some(value) => value,
                None => "missing"
            })
        }
    "#;
    let parsed = splitscript::parse(source).unwrap();
    assert!(matches!(
        parsed.syntax().state.as_ref().unwrap().fields[0].annotation,
        Some(splitscript::compiler::ast::TypeRef::Option(_))
    ));
    let checked = splitscript::check(parsed).expect("optional pointer fields should type-check");
    let fields = &checked.syntax().state.as_ref().unwrap().fields;
    for (field, expected) in [
        (&fields[0], TypeKind::Builtin(BuiltinType::I32)),
        (&fields[1], TypeKind::Standard(StdlibTypeId::String)),
        (&fields[2], TypeKind::Standard(StdlibTypeId::String)),
    ] {
        let field_type = checked.semantics().value_type(field.id).unwrap();
        let TypeKind::Option { value, .. } = checked.semantics().types().kind(field_type) else {
            panic!("optional pointer fields must retain their Option type")
        };
        assert_eq!(checked.semantics().types().kind(*value), &expected);
    }
    Validator::new_with_features(WasmFeatures::all())
        .validate_all(&splitscript::codegen(&checked))
        .expect("optional scalar and decoded-string pointer reads should validate");

    let gba = r#"
        state GBA {
            marker: u8? at 0x02000000
        }

        whileAttached {
            print(match current.marker {
                Some(value) => value as String,
                None => "missing"
            })
        }
    "#;
    let checked = splitscript::check(splitscript::parse(gba).unwrap())
        .expect("provider-backed optional pointer fields should type-check");
    Validator::new_with_features(WasmFeatures::all())
        .validate_all(&splitscript::codegen(&checked))
        .expect("provider-backed optional pointer reads should validate");
}

#[test]
fn option_and_result_values_use_explicit_typed_hir_conversions() {
    use splitscript::compiler::semantic::{ResolvedCall, ValueConversionKind};

    let source = r#"
        state "game.exe" {}

        enum Chapter {
            Village
        }

        fn maybe(flag: bool) -> i32? {
            if flag { return 7 }
            return None
        }

        fn maybeChapter(flag: bool) -> Chapter? {
            if flag { return Chapter.Village }
            return None
        }

        fn attempt(flag: bool) -> i32! {
            if flag { return 9 }
            return Err("attempt failed")
        }

        whileAttached {
            let optional: i32? = 5
            let chapter: Chapter? = Chapter.Village
            let empty: i32? = None
            let successful: i32! = 11
            let failed: i32! = Err("failed")
        }
    "#;
    let checked = splitscript::check(splitscript::parse(source).unwrap()).unwrap();

    let mut saw_option_lift = false;
    let mut saw_result_lift = false;
    let mut saw_optional_null = false;
    let mut error_constructors = 0;
    for expression in checked.typed_hir().expressions() {
        if let Some(conversion) = expression.conversion {
            match conversion.kind {
                ValueConversionKind::LiftOption => saw_option_lift = true,
                ValueConversionKind::LiftResult => saw_result_lift = true,
                ValueConversionKind::NoneToOptional | ValueConversionKind::NoneToDomainNullable => {
                }
            }
            assert_ne!(conversion.source, conversion.target);
        }
        if matches!(
            expression.kind,
            splitscript::compiler::hir::TypedExpressionKind::None
        ) && matches!(
            checked.semantics().types().kind(expression.ty),
            TypeKind::Option { .. }
        ) {
            saw_optional_null = true;
        }
        if matches!(
            checked.typed_hir().call(expression.id),
            Some(ResolvedCall::ResultError { .. })
        ) {
            error_constructors += 1;
        }
    }
    assert!(saw_option_lift);
    assert!(saw_result_lift);
    assert!(saw_optional_null);
    assert_eq!(error_constructors, 2);

    let lowered = splitscript::lower_wasm(&checked);
    for expression in checked.typed_hir().expressions() {
        assert_eq!(
            lowered
                .expression(expression.id)
                .expect("every typed expression should have a Wasm IR plan")
                .conversion,
            expression.conversion,
            "wrapper conversion edges must be copied into Wasm IR"
        );
    }

    Validator::new_with_features(WasmFeatures::all())
        .validate_all(&splitscript::codegen(&checked))
        .expect("option/result constructors and lifts should produce valid Wasm GC");
}

#[test]
fn wasm_ir_owns_scalar_expression_operations_catalog_calls_and_resolved_paths() {
    use splitscript::compiler::wasm_ir::{CallTarget, ExpressionKind};

    let source = r#"
        state "game.exe" {}

        fn calculate(input: i32) {
            let negated = -(input + 2)
            let text = negated as String
            if !false && negated != 0 {
                print(text)
            }
        }

        whileAttached {
            calculate(4)
        }
    "#;
    let checked = splitscript::check(splitscript::parse(source).unwrap()).unwrap();
    let lowered = splitscript::lower_wasm(&checked);
    let mut saw_path = false;
    let mut saw_negate = false;
    let mut saw_not = false;
    let mut saw_binary = false;
    let mut saw_cast = false;
    for typed in checked.typed_hir().expressions() {
        let expression = lowered
            .expression(typed.id)
            .expect("visible typed expressions should have Wasm IR plans");
        match &expression.kind {
            ExpressionKind::Path { root, .. } => {
                assert!(root.is_some());
                saw_path = true;
            }
            ExpressionKind::Call {
                target: CallTarget::Intrinsic { intrinsic, .. },
                ..
            } if *intrinsic == IntrinsicId::SignedNegate => saw_negate = true,
            ExpressionKind::Call {
                target: CallTarget::Intrinsic { intrinsic, .. },
                ..
            } if *intrinsic == IntrinsicId::BoolNot => saw_not = true,
            ExpressionKind::Unary { .. } => {
                panic!("checked unary operators must lower through catalog calls")
            }
            ExpressionKind::Binary { .. } => saw_binary = true,
            ExpressionKind::Cast { .. } => saw_cast = true,
            _ => {}
        }
    }
    assert!(saw_path && saw_negate && saw_not && saw_binary && saw_cast);

    Validator::new_with_features(WasmFeatures::all())
        .validate_all(&splitscript::codegen(&checked))
        .expect("Wasm IR scalar expression lowering should preserve valid codegen");
}

#[test]
fn wasm_ir_owns_gc_constructors_interpolation_and_signatures() {
    use splitscript::compiler::wasm_ir::{ExpressionKind, InterpolatedPart};

    let source = r#"
        state "game.exe" {}

        struct Pair {
            left: i32,
            right: i32
        }

        enum Event {
            Empty,
            Value(i32)
        }

        onAttach {
            let module = await process.module("GameAssembly.dll")
            let marker = await module.scan(sig"48 8B ?? B?")
            print(marker as String)
        }

        whileAttached {
            let values = [1, 2, 3]
            let pair = Pair { right: values[1], left: values[0] }
            let event = Event.Value(pair.left)
            print(`pair {pair.left}`)
        }
    "#;
    let checked = splitscript::check(splitscript::parse(source).unwrap()).unwrap();
    let lowered = splitscript::lower_wasm(&checked);
    let mut saw = [false; 6];
    for typed in checked.typed_hir().expressions() {
        let expression = lowered
            .expression(typed.id)
            .expect("visible typed expressions should have Wasm IR plans");
        match &expression.kind {
            ExpressionKind::String(_) => saw[0] = true,
            ExpressionKind::InterpolatedString(parts) => {
                assert!(parts.iter().any(|part| matches!(
                    part,
                    InterpolatedPart::Expression {
                        string_conversion_source: Some(_),
                        ..
                    }
                )));
                saw[1] = true;
            }
            ExpressionKind::Signature(_) => saw[2] = true,
            ExpressionKind::Array(elements) => {
                assert_eq!(elements.len(), 3);
                saw[3] = true;
            }
            ExpressionKind::Struct { fields, .. } => {
                assert_eq!(fields.len(), 2);
                assert_ne!(fields[0].0, fields[1].0);
                saw[4] = true;
            }
            ExpressionKind::Enum { payload, .. } if payload.is_some() => saw[5] = true,
            _ => {}
        }
    }
    assert!(saw.into_iter().all(|value| value));

    Validator::new_with_features(WasmFeatures::all())
        .validate_all(&splitscript::codegen(&checked))
        .expect("Wasm IR GC constructor lowering should preserve valid codegen");
}

#[test]
fn wasm_ir_owns_backend_call_targets_intrinsics_and_arguments() {
    use splitscript::{
        compiler::hir::TypedExpressionKind,
        compiler::stdlib::IntrinsicId,
        compiler::wasm_ir::{CallTarget, ExpressionKind},
    };

    let source = r#"
        state "game.exe" {}

        struct Counter { value: i32 }

        fn answer() -> i32 {
            return 42
        }

        fn Counter.increment() -> i32 {
            return self.value + 1
        }

        whileAttached {
            let counter = Counter { value: 4 }
            let direct = answer()
            let method = counter.increment()
            let bounded = direct.min(method)
            let failed: i32! = Err("failed")
        }
    "#;
    let checked = splitscript::check(splitscript::parse(source).unwrap()).unwrap();
    let lowered = splitscript::lower_wasm(&checked);
    let mut saw = [false; 5];

    for expression in checked.typed_hir().expressions() {
        let Some(expected_target) = checked.typed_hir().call(expression.id) else {
            continue;
        };
        let ExpressionKind::Call { target, arguments } = &lowered
            .expression(expression.id)
            .expect("every checked call should have a Wasm IR plan")
            .kind
        else {
            panic!("resolved calls must not remain deferred to typed HIR")
        };
        match &expression.kind {
            TypedExpressionKind::Call {
                arguments: expected_arguments,
                ..
            } => assert_eq!(arguments, expected_arguments),
            TypedExpressionKind::Binary { right, .. } => {
                assert_eq!(arguments.as_slice(), [*right])
            }
            _ => unreachable!("only calls and catalog-backed operators resolve call targets"),
        }
        match (target, expected_target) {
            (
                CallTarget::UserFunction { function },
                ResolvedCall::UserFunction {
                    function: expected,
                    type_arguments,
                    ..
                },
            ) => {
                assert_eq!(function.function, *expected);
                assert_eq!(&function.type_arguments, type_arguments);
                saw[0] = true;
            }
            (
                CallTarget::UserMethod { function, .. },
                ResolvedCall::UserMethod {
                    function: expected,
                    type_arguments,
                    ..
                },
            ) => {
                assert_eq!(function.function, *expected);
                assert_eq!(&function.type_arguments, type_arguments);
                saw[1] = true;
            }
            (
                CallTarget::Intrinsic {
                    item, intrinsic, ..
                },
                ResolvedCall::StandardLibrary { item: expected, .. },
            ) => {
                assert_eq!(item, expected);
                match intrinsic {
                    IntrinsicId::NumericMin => saw[2] = true,
                    IntrinsicId::NumericAdd => saw[4] = true,
                    _ => panic!("unexpected intrinsic call target `{intrinsic:?}`"),
                }
            }
            (
                CallTarget::ResultError { result },
                ResolvedCall::ResultError { result: expected },
            ) => {
                assert_eq!(result, expected);
                saw[3] = true;
            }
            (CallTarget::OptionSome { .. }, ResolvedCall::OptionSome { .. })
            | (CallTarget::ResultSuccess { .. }, ResolvedCall::ResultSuccess { .. }) => {}
            _ => panic!("Wasm IR call target disagrees with semantic resolution"),
        }
    }
    assert!(saw.into_iter().all(|value| value));

    Validator::new_with_features(WasmFeatures::all())
        .validate_all(&splitscript::codegen(&checked))
        .expect("Wasm IR call lowering should preserve valid codegen");
}

#[test]
fn context_free_null_and_err_request_wrapper_annotations() {
    let unit = r#"
        state "game.exe" {}
        whileAttached { let value = None }
    "#;
    splitscript::check(splitscript::parse(unit).unwrap())
        .expect("None is the context-free unit value");

    let result = r#"
        state "game.exe" {}
        whileAttached { let value = Err("failed") }
    "#;
    let errors = splitscript::check(splitscript::parse(result).unwrap())
        .expect_err("Err still needs its successful type from context");
    assert!(
        errors
            .iter()
            .any(|error| error.message.contains("add a `T!` annotation"))
    );
}

#[test]
fn none_is_a_first_class_unit_across_wrappers_storage_and_async_code() {
    let source = r#"
        state "game.exe" {}

        let globalUnit = None

        struct UnitBox {
            value: None
        }

        fn identity(value: None) -> None {
            return value
        }

        fn optional(flag: bool) -> None? {
            if flag {
                return Some(None)
            }
            return None
        }

        fn attempt(flag: bool) -> None! {
            if !flag {
                return Err("failed")
            }
            return None
        }

        fn propagate(value: None!) -> None! {
            value?
            return None
        }

        fn waitOne() {
            await nextTick()
        }

        onAttach {
            let awaited: None = await waitOne()
            let values: [None; 2] = [globalUnit, awaited]
            let boxed = UnitBox { value: values[0] }
            let present = optional(true)
            let absent = optional(false)
            let succeeded = propagate(attempt(true)) else None
            let same = attempt(true) == attempt(true)
            if boxed.value == identity(succeeded) && same {
                print("unit")
            }
            if present == absent {
                print("unexpected")
            }
        }
    "#;
    let checked = splitscript::check(splitscript::parse(source).unwrap())
        .expect("None should be a first-class unit value and type");
    let none = checked.semantics().types().id_for_core(CoreTypeId::None);
    assert_eq!(
        checked
            .semantics()
            .value_type(checked.syntax().globals[0].id),
        Some(none)
    );
    Validator::new_with_features(WasmFeatures::all())
        .validate_all(&splitscript::codegen(&checked))
        .expect("unit values should support erased and wrapped physical representations");
}

#[test]
fn void_is_not_a_type_spelling() {
    let source = r#"
        state "game.exe" {}
        fn legacy() -> void {}
    "#;
    let diagnostics = splitscript::compile(source).expect_err("void has no compatibility alias");
    assert!(
        diagnostics
            .iter()
            .any(|diagnostic| diagnostic.message.contains("unknown type `void`"))
    );
}

#[test]
fn plain_none_calls_need_no_bottom_reference_values() {
    let wasm = splitscript::compile(
        r#"
            state "game.exe" {}

            let globalUnit = None

            fn consume(value: None) -> None {
                return value
            }

            whileAttached {
                let localUnit = globalUnit
                consume(localUnit)
            }
        "#,
    )
    .expect("unit parameters and results should compile");
    Validator::new_with_features(WasmFeatures::all())
        .validate_all(&wasm)
        .expect("erased unit parameters and results should preserve the Wasm ABI");

    let bottom_nulls = Parser::new(0)
        .parse_all(&wasm)
        .filter_map(
            |payload| match payload.expect("generated Wasm should parse") {
                Payload::CodeSectionEntry(body) => Some(
                    body.get_operators_reader()
                        .expect("function operators should parse")
                        .into_iter()
                        .filter(|operator| {
                            matches!(
                                operator,
                                Ok(wasmparser::Operator::RefNull {
                                    hty: wasmparser::HeapType::Abstract {
                                        ty: wasmparser::AbstractHeapType::None,
                                        ..
                                    }
                                })
                            )
                        })
                        .count(),
                ),
                _ => None,
            },
        )
        .sum::<usize>();
    assert_eq!(
        bottom_nulls, 0,
        "a plain None argument and result should be erased rather than materialized"
    );
}

#[test]
fn else_unwraps_options_and_results_with_value_or_return_fallbacks() {
    use splitscript::{
        compiler::hir::TypedExpressionKind,
        compiler::wasm_ir::{BodyOwner, ExpressionKind, LocalPurpose},
    };

    let source = r#"
        state "game.exe" {}

        fn choose(value: i32?) -> i32 {
            return value else 41
        }

        fn propagate(value: i32!) -> i32! {
            let unwrapped = value else return Err("propagated")
            return unwrapped + 1
        }

        fn nested(optional: i32?, result: i32!) -> i32 {
            return optional else result else 7
        }

        fn observe(value: i32?) {
            let unwrapped = value else return
            print(unwrapped as String)
        }

        whileAttached {
            let empty = choose(None)
            let present = choose(3)
            let failed = propagate(Err("failed"))
            let successful = propagate(5)
            observe(None)
        }
    "#;
    let checked = splitscript::check(splitscript::parse(source).unwrap()).unwrap();
    let fallbacks = checked
        .typed_hir()
        .expressions()
        .filter(|expression| matches!(expression.kind, TypedExpressionKind::Fallback { .. }))
        .collect::<Vec<_>>();
    let fallback_count = fallbacks.len();
    assert_eq!(
        fallback_count,
        5,
        "unexpected visible fallback spans: {:?}",
        fallbacks
            .iter()
            .map(|expression| expression.span)
            .collect::<Vec<_>>()
    );

    let lowered = splitscript::lower_wasm(&checked);
    for expression in checked.typed_hir().expressions() {
        let TypedExpressionKind::Fallback { value, fallback } = &expression.kind else {
            continue;
        };
        let ExpressionKind::Fallback {
            value: lowered_value,
            fallback: lowered_fallback,
        } = &lowered
            .expression(expression.id)
            .expect("fallback expression should have a Wasm IR plan")
            .kind
        else {
            panic!("fallback expressions must not remain deferred to typed HIR")
        };
        assert_eq!(lowered_value, value);
        assert_eq!(lowered_fallback, fallback);
    }
    assert!(
        checked
            .typed_hir()
            .expressions()
            .any(|expression| matches!(expression.kind, TypedExpressionKind::Return(Some(_))))
    );
    assert!(
        checked
            .typed_hir()
            .expressions()
            .any(|expression| matches!(expression.kind, TypedExpressionKind::Return(None)))
    );
    let planned_fallbacks = lowered
        .bodies()
        .filter(|body| match &body.owner {
            BodyOwner::Function(function) => {
                function.function.index() < checked.syntax().functions.len()
            }
            BodyOwner::Action(_) => true,
        })
        .flat_map(|body| &body.locals)
        .filter(|local| matches!(local.purpose, LocalPurpose::FallbackValue(_)))
        .count();
    assert_eq!(planned_fallbacks, fallback_count);

    Validator::new_with_features(WasmFeatures::all())
        .validate_all(&splitscript::codegen(&checked))
        .expect("value and returning fallbacks should produce valid Wasm control flow");
}

#[test]
fn question_mark_propagates_to_function_and_state_field_boundaries() {
    use splitscript::compiler::hir::TypedExpressionKind;

    let source = r#"
        state "game.exe" {
            selected = if readMemory {
                process.read<u16>(0x1000)?
            } else {
                7
            }
        }

        let readMemory = true

        fn increment(value: i32!) -> i32! {
            return value? + 1
        }

        fn rejectNegative(value: i32) -> i32! {
            if value < 0 {
                throw "negative values are not supported"
            }
            return value
        }

        whileAttached {
            let incremented = increment(3) else 0
            let rejected = rejectNegative(-1) else 0
        }
    "#;
    let checked = splitscript::check(splitscript::parse(source).unwrap()).unwrap();
    let propagation = checked
        .typed_hir()
        .expressions()
        .filter_map(|expression| match expression.kind {
            TypedExpressionKind::Propagate { target, .. } => Some(target),
            _ => None,
        })
        .collect::<Vec<_>>();
    assert_eq!(propagation.len(), 2);
    assert!(propagation.into_iter().all(|target| matches!(
        checked.semantics().types().kind(target.result()),
        TypeKind::Result { .. }
    )));

    let lowered = splitscript::lower_wasm(&checked);
    for expression in checked.typed_hir().expressions() {
        let TypedExpressionKind::Propagate { value, target } = &expression.kind else {
            continue;
        };
        let splitscript::compiler::wasm_ir::ExpressionKind::Propagate {
            value: lowered_value,
            target: lowered_target,
        } = &lowered
            .expression(expression.id)
            .expect("propagation expression should have a Wasm IR plan")
            .kind
        else {
            panic!("postfix propagation must not remain deferred to typed HIR")
        };
        assert_eq!((lowered_value, lowered_target), (value, target));
    }

    Validator::new_with_features(WasmFeatures::all())
        .validate_all(&splitscript::codegen(&checked))
        .expect("question-mark propagation should produce valid Wasm GC control flow");

    let invalid = r#"
        state "game.exe" {}
        whileAttached {
            let failed: i32! = Err("failed")
            let value = failed?
        }
    "#;
    let errors = splitscript::check(splitscript::parse(invalid).unwrap())
        .expect_err("actions are not implicit result boundaries");
    assert!(errors.iter().any(|error| {
        error
            .message
            .contains("state-field boundary, `selectProcess`, or a function returning `T!`")
    }));

    let invalid_throw = r#"
        state "game.exe" {}
        whileAttached { throw "actions do not return results" }
    "#;
    let errors = splitscript::check(splitscript::parse(invalid_throw).unwrap())
        .expect_err("throw requires an enclosing failure boundary");
    assert!(errors.iter().any(|error| {
        error
            .message
            .contains("fallible function, `selectProcess`, or an explicit catch boundary")
    }));
}

#[test]
fn else_rejects_values_that_are_not_option_or_result() {
    let source = r#"
        state "game.exe" {}
        whileAttached { let value = 1 else 2 }
    "#;
    let errors = splitscript::check(splitscript::parse(source).unwrap())
        .expect_err("plain values cannot be unwrapped");
    assert!(errors.iter().any(|error| {
        error
            .message
            .contains("`else` can only unwrap `T?` or `T!`")
    }));
}

#[test]
fn retry_next_to_fallback_requires_explicit_grouping() {
    let source = r#"
        state "game.exe" {}

        fn choose(value: i32?!) {
            return retry value else 0
        }
    "#;
    let checked = splitscript::check(splitscript::parse(source).unwrap())
        .expect("the current grouping is well typed even though it is visually ambiguous");
    let warning = checked
        .diagnostics()
        .iter()
        .find(|diagnostic| diagnostic.code == splitscript::DiagnosticCode::AmbiguousRetryFallback)
        .expect("an adjacent retry and fallback should require explicit grouping");
    assert_eq!(warning.severity, splitscript::DiagnosticSeverity::Warning);
    assert!(warning.message.contains("binds more tightly"));
    assert_eq!(warning.fixes.len(), 2);
    assert_eq!(
        warning.fixes[0].applicability,
        splitscript::FixApplicability::MachineApplicable
    );
    assert_eq!(
        warning.fixes[1].applicability,
        splitscript::FixApplicability::MaybeIncorrect
    );

    fn apply_fix(source: &str, fix: &splitscript::DiagnosticFix) -> String {
        let mut edits = fix.edits.clone();
        edits.sort_by_key(|edit| std::cmp::Reverse((edit.span.start, edit.span.end)));
        let mut fixed = source.to_owned();
        for edit in edits {
            fixed.replace_range(edit.span.start..edit.span.end, &edit.replacement);
        }
        fixed
    }

    let explicit_current = apply_fix(source, &warning.fixes[0]);
    assert!(explicit_current.contains("return (retry value) else 0"));
    assert!(
        splitscript::parse_recovering(&explicit_current)
            .unwrap()
            .diagnostics()
            .iter()
            .all(
                |diagnostic| diagnostic.code != splitscript::DiagnosticCode::AmbiguousRetryFallback
            )
    );

    let explicit_complete = apply_fix(source, &warning.fixes[1]);
    assert!(explicit_complete.contains("return retry (value else 0)"));
    assert!(
        splitscript::parse_recovering(&explicit_complete)
            .unwrap()
            .diagnostics()
            .iter()
            .all(
                |diagnostic| diagnostic.code != splitscript::DiagnosticCode::AmbiguousRetryFallback
            )
    );

    let invalid = r#"
        state "game.exe" {}

        fn engineModule() {
            return retry process.loadedModule("EngineWin64s.dll")
                else process.loadedModule("EngineWin64sv.dll")
                else {
                    throw "engine module is not loaded yet"
                }
        }
    "#;
    let diagnostics = splitscript::compile(invalid)
        .expect_err("retry currently completes before the adjacent fallback chain");
    let precedence = diagnostics
        .iter()
        .find(|diagnostic| diagnostic.code == splitscript::DiagnosticCode::AmbiguousRetryFallback)
        .expect("the invalid form should retain the grouping guidance and fixes");
    assert!(
        diagnostics.iter().any(|diagnostic| {
            diagnostic
                .message
                .contains("`retry` expects a result value (`T!`)")
        }),
        "{diagnostics:#?}"
    );
    let fixed = apply_fix(invalid, &precedence.fixes[1]);
    splitscript::compile(&fixed)
        .expect("moving the complete fallback chain into retry should fix this occurrence");
}

#[test]
fn declared_struct_enum_and_array_layouts_are_semantic_facts() {
    let source = r#"
        state "game.exe" {}

        struct Inventory {
            names: [String],
            code: u16
        }

        enum Lookup {
            Missing,
            Found(Inventory)
        }

        whileAttached {
            let lookup = Lookup.Found(Inventory {
                names: ["Moon"],
                code: 7
            })
        }
    "#;
    let checked = splitscript::check(splitscript::parse(source).unwrap()).unwrap();
    let syntax = checked.syntax();
    let semantics = checked.semantics();
    let inventory = &syntax.structs[0];

    let names_type = semantics
        .struct_field_type(inventory.fields[0].id)
        .expect("struct field layouts should expose semantic types");
    let TypeKind::Array {
        layout,
        element: names_element,
        ..
    } = semantics.types().kind(names_type)
    else {
        panic!("the names field should have a constructed array type");
    };
    assert_eq!(
        semantics.types().kind(*names_element),
        &TypeKind::Standard(StdlibTypeId::String)
    );

    let splitscript::compiler::ast::TypeRef::Array(names_array) = inventory.fields[0].ty else {
        panic!("the source annotation should reference its array layout");
    };
    assert_eq!(*layout, names_array);
    assert_eq!(
        semantics.array_element_type(names_array),
        Some(*names_element)
    );

    let code_type = semantics.struct_field_type(inventory.fields[1].id).unwrap();
    assert_eq!(
        semantics.types().kind(code_type),
        &TypeKind::Builtin(BuiltinType::U16)
    );

    let enumeration = &syntax.enums[0];
    assert!(
        semantics
            .enum_variant_payloads()
            .any(|(variant, payload)| variant == enumeration.variants[0].id && payload.is_none())
    );
    let found_payload = semantics
        .enum_variant_payload(enumeration.variants[1].id)
        .expect("payload variants should expose their semantic payload type");
    assert_eq!(
        semantics.types().kind(found_payload),
        &TypeKind::Struct(inventory.id)
    );

    Validator::new_with_features(WasmFeatures::all())
        .validate_all(&splitscript::codegen(&checked))
        .expect("semantic declaration layouts should drive valid Wasm GC types");
}

#[test]
fn replaceable_deep_pointer_flags_do_not_require_background_watcher_registration() {
    let source = r#"
        let teleporterLoadingPath: MemoryPath? = None
        let teleporterTransitionPath: MemoryPath? = None
        let loadingScreenPath: MemoryPath? = None

        state "AER.exe" {
            teleporterLoading: bool = readFlag(teleporterLoadingPath);
            teleporterTransition: bool = readFlag(teleporterTransitionPath);
            loadingScreen: bool = readFlag(loadingScreenPath);
        }

        tickRate {
            attached: 58,
            detached: 2,
        }

        fn readFlag(path: MemoryPath?) -> bool {
            let resolvedPath = path else return false
            let address = resolvedPath.resolve() else return false
            return process.read<bool>(address) else false
        }

        onAttach {
            let mono = await process.module("mono.dll")
            let teleporter = mono.address.offset(0x1f6964)
            teleporterLoadingPath = teleporter.memoryPath(
                [0x30, 0xd5c],
                0xe90 + 0x5a,
            )
            teleporterTransitionPath = teleporter.memoryPath(
                [0x30, 0xd5c],
                0xe90 + 0x59,
            )
            loadingScreenPath = mono.address.offset(0x1f696c).memoryPath(
                [0x80, 0x90, 0x40, 0x1c, 0x4, 0xc],
                0x4,
            )
        }

        isLoading {
            return current.teleporterLoading
                || current.teleporterTransition
                || current.loadingScreen
        }
    "#;

    splitscript::compile(source).expect(
        "attachment-owned paths should re-resolve replaceable objects on every state update",
    );
}

#[test]
fn bounded_settings_and_static_singleton_paths_do_not_require_runtime_registration() {
    let source = r#"
        image "Assembly-CSharp" {
            class Main {
                static Main instance;
                i32 actualLevelId from "ActualLevelId";
                bool buttonClicked from "ButtonClicked";
                bool isInMainMenu from "IsInMainMenu";
                bool pause from "Pause";
            }
        }

        state Unity.mono(MonoVersion.V2) ["Bzzzt.exe"] {
            level: i32 = Main.instance?.actualLevelId?;
            click: bool = Main.instance?.buttonClicked?;
            mainMenu: bool = Main.instance?.isInMainMenu?;
            pause: bool = Main.instance?.pause?;
        }

        settings {
            "Split by Level" => levels key "levels": true,
            "Levels" {
                for level in 1..=12 {
                    `Level {level}` key `{level}`: false,
                },
                "Level 13" => level13 key "13": true,
                for level in 14..=25 {
                    `Level {level}` key `{level}`: false,
                },
                "Level 26" => level26 key "26": true,
                for level in 27..=38 {
                    `Level {level}` key `{level}`: false,
                },
                "Level 39" => level39 key "39": true,
                for level in 40..=51 {
                    `Level {level}` key `{level}`: false,
                },
            },
        }

        start {
            return current.mainMenu != old.mainMenu && !current.mainMenu
        }

        split {
            return current.click
                && current.click != old.click
                && !current.pause
                && (current.level == 52
                    || (settings.levels && settings.enabled(current.level as String)))
        }

        reset {
            return current.mainMenu != old.mainMenu && current.mainMenu
        }
    "#;

    splitscript::compile(source).expect(
        "finite setting families and static singleton paths should cover bounded helper setup",
    );
}

#[test]
fn legacy_manual_managed_paths_point_to_the_schema_workflow() {
    let source = r#"
        state "AWC.exe" {
            levelState: i32 at 0x100;
        }

        onAttach {
            let mono = await Unity.mono(MonoVersion.V2)
            print(mono)
        }
    "#;

    let diagnostics = splitscript::compile(source)
        .expect_err("manual managed traversal should not remain a public API");
    let diagnostic = diagnostics
        .iter()
        .find(|diagnostic| diagnostic.migration_topic() == Some("asl.unity.managed-schema"))
        .expect("legacy Unity traversal should point to the managed-schema guide");
    assert!(diagnostic.message.contains("`image` schema"));
}

#[test]
fn catalog_queries_expose_generic_calls_effects_and_docs_for_editor_tooling() {
    let library = StandardLibrary::new();
    let process_type = library
        .type_by_name("Process")
        .expect("Process should be an explicit standard-library type");
    assert_eq!(process_type.id, StdlibTypeId::Process);
    assert!(
        process_type
            .documentation
            .summary
            .contains("attached game process")
    );
    assert!(library.type_by_name("UnityImage").is_none());
    let unity_image = library.type_decl(StdlibTypeId::UnityImage);
    assert_eq!(
        unity_image.id,
        splitscript::compiler::stdlib::StdlibTypeId::UnityImage
    );
    assert!(
        library.public_field(unity_image.id, "address").is_none(),
        "private Unity binder details must not leak into the public member surface"
    );
    let read = library
        .method_candidates("read")
        .into_iter()
        .next()
        .expect("generic process reads should resolve through the catalog");
    assert_eq!(read.item.id, StdlibItemId::ProcessRead);
    assert!(library.method_candidates("get").is_empty());
    let min = library.method_candidates("min");
    assert_eq!(min.len(), 1);
    assert_eq!(min[0].item.id, StdlibItemId::NumericMin);
    assert_eq!(min[0].item.signature.type_parameters[0].name, "Self");
    assert_eq!(
        min[0].item.signature.type_parameters[0].constraints,
        [splitscript::compiler::stdlib::StdlibCapabilityId::Numeric]
    );
    assert!(library.method_candidates("missing").is_empty());
    assert_eq!(
        library.render_signature(read.item.id),
        "Process.read<T>(address: address) -> T! where T: MemoryReadable"
    );
    assert!(library.method_candidates("readManagedString").is_empty());
    assert_eq!(
        library.render_signature(StdlibItemId::TimerState),
        "timer.state() -> TimerState"
    );
    assert_eq!(
        library.render_signature(StdlibItemId::TimerCurrentSplitIndex),
        "timer.currentSplitIndex() -> u64?"
    );
    assert_eq!(
        library.render_signature(StdlibItemId::TimerSegmentWasSplit),
        "timer.segmentWasSplit(index: u64) -> bool?"
    );
    assert_eq!(
        library.render_signature(StdlibItemId::TimerSkipSplit),
        "timer.skipSplit() -> None"
    );
    assert_eq!(
        library.render_signature(StdlibItemId::TimerUndoSplit),
        "timer.undoSplit() -> None"
    );
    assert_eq!(
        library.render_signature(StdlibItemId::ProcessPath),
        "Process.path() -> String!"
    );
    assert_eq!(
        library.render_signature(StdlibItemId::RuntimeOperatingSystem),
        "runtime.operatingSystem() -> String!"
    );
    assert_eq!(
        library.render_signature(StdlibItemId::RuntimeArchitecture),
        "runtime.architecture() -> String!"
    );
    let next_tick = library
        .item_by_name("nextTick")
        .expect("nextTick should be catalog-backed");
    assert_eq!(
        library.render_signature(next_tick.id),
        "nextTick() -> async None"
    );
    assert_eq!(
        library.operation_semantics(next_tick.id).suspension,
        SuspensionKind::Suspends
    );
    assert_eq!(
        library.operation_semantics(next_tick.id).cancellation,
        CancellationKind::ProcessClose
    );
    let process_closed = library
        .item_by_name("Process.closed")
        .expect("Process.closed should be catalog-backed");
    assert_eq!(
        library.render_signature(process_closed.id),
        "Process.closed() -> async Never"
    );
    assert_eq!(
        library.render_operation_semantics(process_closed.id),
        "available in suspending attachment code; suspends; cancels when the process closes"
    );

    assert!(library.item_by_name("UnityClass.fieldAny").is_none());
    assert_eq!(
        library.operation_semantics(read.item.id).suspension,
        SuspensionKind::None
    );
    assert_eq!(
        library.render_signature(StdlibItemId::ProcessFollow),
        "Process.follow(base: address, offsets: [i64]) -> address!"
    );
    assert_eq!(
        library.render_signature(StdlibItemId::ProcessReadRelative32),
        "Process.readRelative32(address: address) -> address!"
    );
}

#[test]
fn process_closed_is_available_in_suspending_attached_lifecycle_actions() {
    let wasm = splitscript::compile(
        r#"
            state "game.exe" {}
            whileAttached {
                await process.closed()
            }
        "#,
    )
    .expect("waiting for process closure is valid while attached");
    wasmparser::Validator::new_with_features(wasmparser::WasmFeatures::all())
        .validate_all(&wasm)
        .expect("a never-completing whileAttached continuation should produce valid Wasm");
}

#[test]
fn process_operations_reject_detach_lifecycle_use() {
    let errors = splitscript::compile(
        r#"
            state "game.exe" {}
            onDetach {
                let value = process.read<i32>(0x1000) else 0
                print(value as String)
            }
        "#,
    )
    .expect_err("process access should not be available before attachment");
    assert!(errors.iter().any(|error| {
        error.message
            == "`Process.read` requires an attached process and is unavailable in `onDetach`"
    }));
}

#[test]
fn managed_snapshot_reads_require_an_attached_process() {
    let diagnostics = splitscript::compile(
        r#"
            image "Assembly-CSharp" {
                class GameManager {
                    static GameManager instance;
                    i32 points;
                }
            }
            state Unity ["game.exe"] {}
            setup {
                let manager = GameManager.instance else return
                manager.snapshot()
            }
        "#,
    )
    .expect_err("managed snapshots must not read a process during setup");
    assert!(diagnostics.iter().any(|diagnostic| {
        diagnostic.message
            == "`snapshot` requires an attached process and is unavailable in `setup`"
    }));
}

#[test]
fn typed_unity_components_require_the_unity_provider_and_a_schema_class() {
    let provider_diagnostics = splitscript::compile(
        r#"
            image "Assembly-CSharp" {
                class Player {}
            }
            state "game.exe" {}
            fn player(object: UnityGameObject) {
                return object.component<Player>()
            }
        "#,
    )
    .expect_err("component lookup depends on Unity attachment metadata");
    assert!(provider_diagnostics.iter().any(|diagnostic| {
        diagnostic.message == "Unity component lookup requires a Unity state provider"
    }));

    let type_diagnostics = splitscript::compile(
        r#"
            state Unity ["game.exe"] {}
            fn value(object: UnityGameObject) {
                return object.component<u32>()
            }
        "#,
    )
    .expect_err("component lookup should only accept managed schema classes");
    assert!(type_diagnostics.iter().any(|diagnostic| {
        diagnostic.message == "`component<T>` requires a declared managed class for `T`"
    }));
}

#[test]
fn detach_does_not_expose_a_closed_process_or_uninitialized_snapshots() {
    let process_errors = splitscript::compile(
        r#"
            state "game.exe" {}
            onDetach { process.read<i32>(0x1000) }
        "#,
    )
    .expect_err("a closed process handle must not remain usable");
    assert!(process_errors.iter().any(|diagnostic| {
        diagnostic.message
            == "`Process.read` requires an attached process and is unavailable in `onDetach`"
    }));

    let snapshot_errors = splitscript::compile(
        r#"
            state "game.exe" { level: u32 at 0x100 }
            onDetach { print(current.level) }
        "#,
    )
    .expect_err("a process can close before its first snapshot commits");
    assert!(snapshot_errors.iter().any(|diagnostic| {
        diagnostic
            .message
            .contains("state snapshots are not guaranteed to exist in `onDetach`")
    }));
}

#[test]
fn setup_is_process_independent_and_cannot_suspend_or_read_snapshots() {
    let process_errors = splitscript::compile(
        r#"
            state "game.exe" {}
            setup { process.read<i32>(0x1000) }
        "#,
    )
    .expect_err("setup must not access the process provider");
    assert!(process_errors.iter().any(|error| {
        error.message == "`Process.read` requires an attached process and is unavailable in `setup`"
    }));

    let snapshot_errors = splitscript::compile(
        r#"
            state "game.exe" { level: i32 at 0x1000 }
            setup { print(current.level) }
        "#,
    )
    .expect_err("setup runs before state snapshots exist");
    assert!(snapshot_errors.iter().any(|error| {
        error
            .message
            .contains("state snapshots are not available during `setup`")
    }));

    let suspension_errors = splitscript::compile(
        r#"
            state "game.exe" {}
            setup { await nextTick() }
        "#,
    )
    .expect_err("setup must finish synchronously during module start");
    assert!(
        suspension_errors
            .iter()
            .any(|error| error.message == "`await` is not available in this synchronous body")
    );
}

#[test]
fn on_state_ready_has_the_attached_process_and_committed_snapshots_but_cannot_suspend() {
    splitscript::compile(
        r#"
            state "game.exe" { level: u32 at 0x100 }
            onStateReady {
                print(process.name())
                print(old.level)
                print(current.level)
            }
        "#,
    )
    .expect("post-snapshot initialization should expose process and state values");

    let diagnostics = splitscript::compile(
        r#"
            state "game.exe" {}
            onStateReady { await nextTick() }
        "#,
    )
    .expect_err("post-snapshot initialization must complete synchronously");
    assert!(diagnostics.iter().any(|diagnostic| {
        diagnostic.message == "`await` is not available in this synchronous body"
    }));
}

#[test]
fn call_result_fields_parse_before_detached_effects_are_checked() {
    let source = r#"
        state "game.exe" {}

        struct LevelTimeParts {
            minutes: f32,
            seconds: f32,
            hundredths: f32
        }

        fn baz() {
            return process.read(0x200) else process.read(0x100) else LevelTimeParts {
                minutes: 0.0,
                seconds: 0.0,
                hundredths: 0.0
            }
        }

        onDetach {
            let minutes = baz().minutes
        }
    "#;

    splitscript::parse(source).expect("a field on a call result should parse");
    let attached = source.replace("onDetach", "whileAttached");
    let wasm = splitscript::compile(&attached)
        .expect("a call-result field should type-check and lower while attached");
    Validator::new_with_features(WasmFeatures::all())
        .validate_all(&wasm)
        .expect("a call-result field should produce valid Wasm");
    let diagnostics = splitscript::compile(source)
        .expect_err("the process-dependent helper should still be rejected while detached");
    assert!(
        diagnostics.iter().any(|diagnostic| {
            diagnostic.message
                == "`baz` requires an attached process and is unavailable in `onDetach`"
        }),
        "{diagnostics:#?}"
    );
    assert!(
        diagnostics
            .iter()
            .all(|diagnostic| diagnostic.code != splitscript::DiagnosticCode::Syntax)
    );
}

#[test]
fn immediate_process_failures_are_results_and_not_awaitable_intrinsics() {
    let source = include_str!("../fallible_process_operations.split");
    let wasm = splitscript::compile(source).expect("fallible process operations should compile");
    Validator::new_with_features(WasmFeatures::all())
        .validate_all(&wasm)
        .expect("process failure sentinels should lower to valid Result values");

    let diagnostics = splitscript::compile(
        r#"
            state "game.exe" {}
            onAttach {
                let value = await process.read<i32>(0x1000)
            }
        "#,
    )
    .expect_err("immediate Result operations should use retry rather than await");
    assert!(diagnostics.iter().any(|diagnostic| {
        diagnostic
            .message
            .contains("`await` expects an async value")
    }));
}

#[test]
fn attached_process_requirements_propagate_through_function_call_graphs() {
    let safe_source = r#"
        state "game.exe" {}

        struct Reader {
            address: address
        }

        fn Reader.readValue() {
            return process.read<i32>(self.address) else 0
        }

        fn relay(reader: Reader) {
            return reader.readValue()
        }

        fn recursiveRelay(reader: Reader, recurse) {
            if recurse {
                return recursiveRelay(reader, false)
            }
            return relay(reader)
        }

        whileAttached {
            let reader = Reader { address: 0x1000 }
            print(recursiveRelay(reader, true) as String)
        }
    "#;
    let checked = splitscript::check(splitscript::lower(splitscript::parse(safe_source).unwrap()))
        .expect("process-dependent helpers should be callable while attached");
    for name in ["readValue", "relay", "recursiveRelay"] {
        let function = checked
            .syntax()
            .functions
            .iter()
            .find(|function| function.name == name)
            .expect("test helper should exist");
        assert!(
            checked
                .effects()
                .function(function.id)
                .requires_attached_process,
            "{name} should inherit its process requirement"
        );
        let effects = checked.effects().function(function.id).effects;
        assert!(
            effects.contains(&Effect::ReadsProcess),
            "{name} should inherit its process-read effect"
        );
        assert!(effects.contains(&Effect::RequiresAttachedProcess));
    }

    let detached_source = safe_source.replace("whileAttached", "onDetach");
    let errors = splitscript::compile(&detached_source)
        .expect_err("a transitive process dependency should be rejected while detached");
    assert!(errors.iter().any(|error| {
        error.message
            == "`recursiveRelay` requires an attached process and is unavailable in `onDetach`"
    }));
}

#[test]
fn closure_effects_remain_latent_until_the_callable_is_invoked() {
    let declarations = r#"
        state "game.exe" {}

        fn makeReader() {
            return () => process.read<u32>(0x100)
        }
    "#;
    let stored = format!(
        "{declarations}\nsetup {{\n    let reader = makeReader()\n    print(\"ready\")\n}}"
    );
    splitscript::compile(&stored)
        .expect("constructing and storing a process-reading closure must not execute it");

    let invoked =
        format!("{declarations}\nsetup {{\n    let reader = makeReader()\n    print(reader())\n}}");
    let diagnostics = splitscript::compile(&invoked)
        .expect_err("invoking the closure must apply its attached-process requirement");
    assert!(
        diagnostics.iter().any(|diagnostic| {
            diagnostic.message
                == "`callable` requires an attached process and is unavailable in `setup`"
        }),
        "{diagnostics:#?}"
    );
}

#[test]
fn named_function_effects_remain_latent_until_invocation() {
    let declarations = r#"
        state "game.exe" {}

        fn readValue(fallback: u32) {
            return process.read<u32>(0x100) else fallback
        }
    "#;
    let stored =
        format!("{declarations}\nsetup {{\n    let reader = readValue\n    print(\"ready\")\n}}");
    splitscript::compile(&stored).expect("storing an effectful named function must not execute it");

    let invoked = format!(
        "{declarations}\nsetup {{\n    let reader = readValue\n    print(reader(0u32))\n}}"
    );
    let diagnostics = splitscript::compile(&invoked)
        .expect_err("invoking an effectful named function must apply its effects");
    assert!(
        diagnostics.iter().any(|diagnostic| {
            diagnostic.message
                == "`callable` requires an attached process and is unavailable in `setup`"
        }),
        "{diagnostics:#?}"
    );
}

#[test]
fn higher_order_effects_are_instantiated_per_call_site() {
    let source = r#"
        state "game.exe" {}

        fn invoke(callback) {
            return callback()
        }

        fn makeReader() {
            return () => process.read<u32>(0x100)
        }

        setup {
            print(invoke(() => 42u32))
        }

        whileAttached {
            print(invoke(makeReader()))
        }
    "#;
    splitscript::compile(source).expect(
        "a pure callback in setup must stay pure even when another specialization reads process memory",
    );
    let mut database = splitscript::tooling::database::CompilerDatabase::new(source);
    let hover = database
        .hover(source.find("invoke(callback)").unwrap())
        .unwrap()
        .expect("higher-order helper hover");
    assert!(
        hover
            .markdown
            .contains("**Effect dependencies:** invokes `callback`"),
        "{}",
        hover.markdown
    );

    let invalid = source.replace("print(invoke(() => 42u32))", "print(invoke(makeReader()))");
    let diagnostics = splitscript::compile(&invalid)
        .expect_err("the concrete effectful callback must be rejected in setup");
    assert!(
        diagnostics.iter().any(|diagnostic| {
            diagnostic.message
                == "`invoke` requires an attached process and is unavailable in `setup`"
        }),
        "{diagnostics:#?}"
    );
}

#[test]
fn lazy_iterator_adapters_apply_callback_effects_only_when_consumed() {
    let declarations = r#"
        state "game.exe" {}

        fn makeReader() {
            return (fallback: u32) => process.read<u32>(0x100) else fallback
        }
    "#;
    let stored = format!(
        "{declarations}\nsetup {{\n    let mapped = [1u32].iterator().map(makeReader())\n    print(\"ready\")\n}}"
    );
    splitscript::compile(&stored)
        .expect("constructing a lazy map adapter must not invoke its callback");

    let consumed = format!(
        "{declarations}\nsetup {{\n    for value in [1u32].iterator().map(makeReader()) {{\n        print(value)\n    }}\n}}"
    );
    let diagnostics = splitscript::compile(&consumed)
        .expect_err("iterating the adapter must instantiate its callback effects");
    assert!(
        diagnostics
            .iter()
            .any(|diagnostic| { diagnostic.message.contains("requires an attached process") }),
        "{diagnostics:#?}"
    );
}

#[test]
fn generic_iterable_helpers_preserve_lazy_adapter_effects() {
    let source = r#"
        state "game.exe" {}

        fn consume(values) {
            for value in values {
                print(value)
            }
        }

        fn makeReader() {
            return (fallback: u32) => process.read<u32>(0x100) else fallback
        }

        setup {
            consume([1u32].iterator().map(value => value + 1))
        }

        whileAttached {
            consume([1u32].iterator().map(makeReader()))
        }
    "#;
    splitscript::compile(source)
        .expect("a generic consumer must instantiate a pure adapter independently");

    let invalid = source.replace(
        "consume([1u32].iterator().map(value => value + 1))",
        "consume([1u32].iterator().map(makeReader()))",
    );
    let diagnostics = splitscript::compile(&invalid)
        .expect_err("generic iteration must preserve the concrete adapter callback effects");
    assert!(
        diagnostics.iter().any(|diagnostic| {
            diagnostic.message
                == "`consume` requires an attached process and is unavailable in `setup`"
        }),
        "{diagnostics:#?}"
    );
}

#[test]
fn generator_closure_effects_remain_latent_until_iteration() {
    let generator = r#"let generator: () -> iterator u32 = () -> iterator u32 => {
        yield process.read<u32>(0x100) else 0u32
    }"#;
    let stored = format!(
        "state \"game.exe\" {{}}\nsetup {{\n    {generator}\n    let values = generator()\n    print(\"ready\")\n}}"
    );
    splitscript::compile(&stored)
        .expect("invoking a generator closure must construct it without executing its body");

    let consumed = format!(
        "state \"game.exe\" {{}}\nsetup {{\n    {generator}\n    for value in generator() {{\n        print(value)\n    }}\n}}"
    );
    let diagnostics =
        splitscript::check(splitscript::lower(splitscript::parse(&consumed).unwrap()))
            .expect_err("iterating a generator closure must apply its body effects");
    assert!(
        diagnostics
            .iter()
            .any(|diagnostic| diagnostic.message.contains("requires an attached process")),
        "{diagnostics:#?}"
    );
}

#[test]
fn state_snapshot_requirements_propagate_through_function_call_graphs() {
    let source = r#"
        state "game.exe" {
            level: u32 at 0x100
        }

        fn enteredLevel(level) {
            return old.level != level && current.level == level
        }

        fn relay(level) {
            return enteredLevel(level)
        }

        split {
            return relay(7u32)
        }
    "#;
    let checked = splitscript::check(splitscript::lower(splitscript::parse(source).unwrap()))
        .expect("snapshot-dependent helpers should be callable from timer actions");
    for name in ["enteredLevel", "relay"] {
        let function = checked
            .syntax()
            .functions
            .iter()
            .find(|function| function.name == name)
            .expect("test helper should exist");
        let operation = checked.effects().function(function.id);
        assert!(operation.requires_state_snapshots, "{name}");
        assert!(operation.effects.contains(&Effect::RequiresStateSnapshots));
    }

    let wasm = splitscript::compile(source).expect("snapshot helpers should lower to Wasm");
    Validator::new_with_features(WasmFeatures::all())
        .validate_all(&wasm)
        .expect("snapshot helpers should produce valid Wasm GC");
}

#[test]
fn snapshot_dependent_helpers_are_rejected_without_committed_snapshots() {
    let declarations = r#"
        state "game.exe" {
            level: u32 at 0x100
        }

        fn changed() {
            return old.level != current.level
        }
    "#;
    for action in ["setup", "onAttach", "onDetach"] {
        let source = format!("{declarations}\n{action} {{ print(changed()) }}");
        let diagnostics = splitscript::compile(&source)
            .expect_err("a snapshot-dependent helper needs committed snapshots");
        assert!(
            diagnostics.iter().any(|diagnostic| {
                diagnostic.message
                    == format!(
                        "`changed` requires state snapshots and is unavailable in `{action}`"
                    )
            }),
            "{action}: {diagnostics:#?}"
        );
    }

    let state_source = r#"
        state "game.exe" {
            level: u32 at 0x100;
            changed = didChange()
        }

        fn didChange() {
            return old.level != current.level
        }
    "#;
    let diagnostics = splitscript::compile(state_source)
        .expect_err("state polling must not call snapshot-dependent helpers");
    assert!(diagnostics.iter().any(|diagnostic| {
        diagnostic.message
            == "`didChange` requires state snapshots and is unavailable in a state field expression"
    }), "{diagnostics:#?}");
}

#[test]
fn explicit_generic_calls_accept_named_and_constructed_types() {
    let source = r#"
        state "game.exe" {}

        struct Header {
            marker: u32
        }

        whileAttached {
            let header = process.read<Header>(0)
            let bytes = process.read<[u8; 4]>(4)
            print<u32>((header else Header { marker: 0 }).marker)
            print<u8>((bytes else [0, 0, 0, 0])[0])
        }
    "#;
    splitscript::compile(source)
        .expect("explicit generic calls should accept every MemoryReadable source type");

    for rejected_type in ["String", "char"] {
        let source = format!(
            "state \"game.exe\" {{}}\nwhileAttached {{ let value = process.read<{rejected_type}>(0) }}"
        );
        let errors = splitscript::compile(&source)
            .expect_err("generic constraints still apply to explicit type arguments");
        assert!(
            errors
                .iter()
                .any(|error| error.message.contains("MemoryReadable")),
            "{errors:#?}"
        );
    }

    splitscript::compile(
        r#"
            state "game.exe" {}
            whileAttached { let value = process.read.u32(0) }
        "#,
    )
    .expect_err("the former dotted type-selector syntax must not remain available");
}

#[test]
fn managed_readable_accepts_memory_layouts_and_managed_strings() {
    let checked = splitscript::check(
        splitscript::parse(
            r#"
        state "game.exe" {}
        enum Mode: u32 { Idle, Running }
        struct Header { mode: Mode, history: [u32; 2] }
        fn readAt(at) { return process.read(at) }
        setup {
            let optional: String? = Some("Hana")
        }
        whileAttached {
            let header: Header! = readAt(0 as address)
        }
    "#,
        )
        .unwrap(),
    )
    .unwrap();
    use splitscript::compiler::stdlib::StdlibCapabilityId;
    let managed_readable = StdlibCapabilityId::ManagedReadable;
    let memory_readable = StdlibCapabilityId::MemoryReadable;
    for (ty, kind) in checked.semantics().types().iter() {
        if checked
            .capabilities()
            .has(ty, memory_readable, checked.semantics())
        {
            assert!(
                checked
                    .capabilities()
                    .has(ty, managed_readable, checked.semantics()),
                "{kind:?}"
            );
        }
    }
    let types = checked.semantics().types();
    let string = types.id_for_standard(StdlibTypeId::String);
    let optional = types
        .iter()
        .find_map(|(ty, kind)| {
            matches!(kind, TypeKind::Option { value, .. } if *value == string).then_some(ty)
        })
        .unwrap();
    for ty in [string, optional] {
        assert!(
            checked
                .capabilities()
                .has(ty, managed_readable, checked.semantics())
        );
        assert!(
            !checked
                .capabilities()
                .has(ty, memory_readable, checked.semantics())
        );
    }
    let generic = checked
        .semantics()
        .function_type_parameters(checked.syntax().functions[0].id)[0];
    assert!(
        checked
            .capabilities()
            .has(generic, managed_readable, checked.semantics())
    );
}

#[test]
fn managed_readable_rejects_types_without_implemented_decoders() {
    for value_type in [
        "char",
        "[char]",
        "Map<String, char>",
        "Map<char, String>",
        "Set<char>",
        "Header",
    ] {
        let source = format!(
            r#"
            state Unity ["game.exe"] {{}}
            struct Header {{ text: String }}
            image "Assembly-CSharp" {{
                class Probe {{ {value_type} value; }}
            }}
        "#
        );
        let errors = splitscript::compile(&source)
            .map(|wasm| wasm.len())
            .expect_err("capability proofs require an implemented decoder for the entire value");
        assert!(
            errors
                .iter()
                .any(|error| error.message.contains("no supported managed decoder")),
            "{value_type}: {errors:#?}"
        );
    }
}

#[test]
fn integer_represented_enums_are_memory_readable_recursively() {
    let source = r#"
        enum GameState: i32 {
            Mission = 0,
            TitleScreen,
            Menu,
            Results = 6,
        }

        enum SignedState: i8 {
            Negative = -1,
            Zero,
        }

        enum WideState: u64 {
            Maximum = 18446744073709551615,
        }

        struct Snapshot {
            current: GameState,
            history: [GameState; 2],
        }

        state "game.exe" {
            snapshot: Snapshot at 0x100;
            signed: SignedState at 0x200;
            wide: WideState at 0x208;
        }

        split {
            return old.snapshot.current != GameState.Results
                && current.snapshot.current == GameState.Results
        }
    "#;
    let wasm = splitscript::compile(source)
        .expect("integer-represented enums should derive recursive MemoryReadable layouts");
    Validator::new_with_features(WasmFeatures::all())
        .validate_all(&wasm)
        .expect("represented enum reads should produce valid Wasm GC");
}

#[test]
fn process_readable_enum_declarations_are_validated_eagerly() {
    for (declaration, expected) in [
        (
            "enum Bad: bool { Value = 0 }",
            "an enum process-memory representation must be one of",
        ),
        (
            "enum Bad: u8 { Value(String) = 0 }",
            "process-readable enum variant `Bad.Value` cannot carry a payload",
        ),
        (
            "enum Bad: u8 { First = 1, Second = 1 }",
            "both use discriminant `1`",
        ),
        ("enum Bad: i8 { TooLarge = 128 }", "does not fit in `i8`"),
        ("enum Bad: u8 {}", "an enum needs at least one variant"),
        (
            "enum Bad { Value = 0 }",
            "an explicit discriminant requires an integer representation on the enum",
        ),
    ] {
        let source = format!("{declaration}\nstate \"game.exe\" {{}}");
        let diagnostics = splitscript::compile(&source)
            .expect_err("invalid process-readable enum declarations must fail even when unused");
        assert!(
            diagnostics
                .iter()
                .any(|diagnostic| diagnostic.message.contains(expected)),
            "{declaration}: {diagnostics:#?}"
        );
    }

    let diagnostics = splitscript::compile(
        r#"
            enum Ordinary { Value }
            state "game.exe" {}
            whileAttached { let value = process.read<Ordinary>(0) }
        "#,
    )
    .expect_err("an ordinary enum has no process-memory representation");
    assert!(diagnostics.iter().any(|diagnostic| {
        diagnostic.message.contains("MemoryReadable")
            || diagnostic
                .message
                .contains("no declared process-memory representation")
    }));
}

#[test]
fn known_alternate_modules_and_runtime_pointer_bounds_need_no_enumeration_or_index_array() {
    let source = r#"
        state "Hades.exe" {}

        fn engineModule() {
            return retry {
                let engine = process.loadedModule("EngineWin64s.dll")
                    else process.loadedModule("EngineWin64sv.dll")
                    else throw "engine module is not loaded yet"
                engine
            }
        }

        fn findGameUi(screenManager: address) -> address? {
            let cursor = process.read<address>(screenManager.offset(0x48)) else return None
            let end = process.read<address>(screenManager.offset(0x50)) else return None
            while cursor < end {
                let screen = process.read<address>(cursor) else return None
                let vtable = process.read<address>(screen) else return None
                let getType = process.read<address>(vtable.offset(0x68)) else return None
                let screenType = process.read<i32>(getType.offset(1)) else return None
                if (screenType & 0x7) == 7 {
                    return Some(screen)
                }
                cursor = cursor.offset(8)
            }
            return None
        }

        onAttach {
            let engine = await engineModule()
            let gameUi = findGameUi(engine.address) else 0
            print(gameUi)
            await process.closed()
        }
    "#;

    let wasm = splitscript::compile(source)
        .expect("known alternate modules and runtime-bounded pointer walks should compose");
    Validator::new_with_features(WasmFeatures::all())
        .validate_all(&wasm)
        .expect("the focused Hades discovery shape should produce valid Wasm GC");
}

#[test]
fn timer_lifecycle_actions_cannot_depend_on_an_attachment() {
    for action in ["onStart", "onReset"] {
        for (expression, expected) in [
            ("process.name()", "requires an attached process"),
            ("current.value", "state snapshots are unavailable"),
        ] {
            let source = format!(
                r#"
                    state "game.exe" {{ value: u32 at 0x100; }}
                    {action} {{
                        print({expression})
                    }}
                "#
            );
            let diagnostics = splitscript::compile(&source)
                .expect_err("timer lifecycle actions run even without an attached process");
            assert!(
                diagnostics
                    .iter()
                    .any(|diagnostic| diagnostic.message.contains(expected)),
                "{diagnostics:#?}"
            );
        }
    }
}

#[test]
fn async_failures_store_results_before_completing_the_poll() {
    let wasm = splitscript::compile(include_str!("../async_failure.split"))
        .expect("async throwing and propagating functions compile");
    Validator::new_with_features(WasmFeatures::all())
        .validate_all(&wasm)
        .expect("async errors must return poll flags, not Result references");
}
