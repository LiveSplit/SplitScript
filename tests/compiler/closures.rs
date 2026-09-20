use splitscript::compiler::ast::Span;
use splitscript::compiler::types::TypeKind;
use splitscript::tooling::{
    database::{CompilerDatabase, DefinitionTarget, SourceDefinitionId},
    highlight::SemanticTokenKind,
    language::LanguageItemId,
};
use wasmparser::{Operator, Parser, Payload, Validator, WasmFeatures};

#[test]
fn effect_analysis_does_not_capture_unreferenced_sibling_closures() {
    let mut source = String::from("state \"game.exe\" {}\nfn select(seed: u32) -> () -> u32 {\n");
    for index in 0..48 {
        source.push_str(&format!(
            "let sibling{index}: () -> u32 = () => seed + {index}u32\n"
        ));
    }
    // A nested closure still needs its transitive lexical capture. Before the
    // fix, each sibling also copied every earlier sibling into its effect model.
    source.push_str("return () => { let nested: () -> u32 = () => sibling47(); nested() }\n}\nwhileAttached { print(select(2)()) }\n");
    let wasm = splitscript::compile(&source).expect("independent closure models must stay bounded");
    Validator::new_with_features(WasmFeatures::all())
        .validate_all(&wasm)
        .unwrap();
}

fn call_ref_count(wasm: &[u8]) -> usize {
    Parser::new(0)
        .parse_all(wasm)
        .filter_map(Result::ok)
        .filter_map(|payload| match payload {
            Payload::CodeSectionEntry(body) => Some(body),
            _ => None,
        })
        .flat_map(|body| body.get_operators_reader().unwrap().into_iter())
        .filter_map(Result::ok)
        .filter(|operator| matches!(operator, Operator::CallRef { .. }))
        .count()
}

#[test]
fn exact_callable_producers_use_direct_calls_and_heterogeneous_joins_stay_dynamic() {
    let exact = splitscript::compile(
        r#"
            state "game.exe" {}

            whileAttached {
                let addOne = value => value + 1
                print(addOne(4))
            }
        "#,
    )
    .expect("an exact closure target should compile");
    assert_eq!(call_ref_count(&exact), 0);

    let heterogeneous = splitscript::compile(
        r#"
            state "game.exe" {}

            fn choose(first: bool) -> (u32) -> u32 {
                if first {
                    return value => value + 1
                }
                return value => value + 2
            }

            whileAttached {
                let operation = choose(true)
                print(operation(4))
            }
        "#,
    )
    .expect("joined closure targets should retain dynamic dispatch");
    assert_eq!(call_ref_count(&heterogeneous), 1);
}

#[test]
fn infers_closure_parameters_and_results_bidirectionally() {
    let source = r#"
        state "game.exe" {}

        fn apply(value: u32, transform: (u32) -> u32) -> u32 {
            return transform(value)
        }

        whileAttached {
            let offset = 2u32
            let addOffset = value => value + offset
            let direct = addOffset(3)
            let contextual = apply(4, value => value * 2)
            print(direct)
            print(contextual)
        }
    "#;
    let checked = splitscript::check(splitscript::lower(splitscript::parse(source).unwrap()))
        .expect("closures should type-check");
    assert!(
        checked
            .diagnostics()
            .iter()
            .all(|diagnostic| !diagnostic.message.contains("unused variable `addOffset`")),
        "invoking a callable variable is a read: {:#?}",
        checked.diagnostics()
    );

    let callable_count = checked
        .semantics()
        .types()
        .iter()
        .filter(|(_, kind)| matches!(kind, TypeKind::Callable { .. }))
        .count();
    assert!(callable_count >= 1);
    let wasm = splitscript::codegen(&checked);
    Validator::new_with_features(WasmFeatures::all())
        .validate_all(&wasm)
        .expect("closures should lower to valid WebAssembly GC");
}

#[test]
fn callable_annotations_accept_multiple_parameters_and_async_results() {
    let source = r#"
        state "game.exe" {}

        fn consume(callback: (u32, String) -> async bool) {}
    "#;
    splitscript::check(splitscript::lower(splitscript::parse(source).unwrap()))
        .expect("callable type syntax should be accepted");
}

#[test]
fn closure_and_callable_arrows_link_to_their_language_documentation() {
    let source = r#"
        state "game.exe" {}

        fn apply(value: u32, transform: (u32) -> u32) -> u32 {
            return transform(value)
        }

        whileAttached {
            print(apply(4, value => value * 2))
        }
    "#;
    let mut database = CompilerDatabase::new(source);
    let callable_arrow = source.find("(u32) -> u32").unwrap() + "(u32) ".len();
    let closure_arrow = source.find("value => value").unwrap() + "value ".len();

    assert_eq!(
        database.definition_at(callable_arrow).unwrap(),
        Some(DefinitionTarget::Language(LanguageItemId::CallableType))
    );
    assert_eq!(
        database.definition_at(closure_arrow).unwrap(),
        Some(DefinitionTarget::Language(LanguageItemId::Closure))
    );
    assert!(
        database
            .hover(closure_arrow)
            .unwrap()
            .expect("closure arrow hover")
            .markdown
            .contains("lexical captures")
    );
}

#[test]
fn higher_order_helpers_infer_callable_parameters_from_usage() {
    let source = r#"
        state "game.exe" {}

        fn apply(value, transform) {
            return transform(value)
        }

        whileAttached {
            print(apply(4u32, value => value + 1))
            print(apply("ready", value => `{value}!`))
        }
    "#;
    let checked = splitscript::check(splitscript::lower(splitscript::parse(source).unwrap()))
        .expect("higher-order helper types should infer from each call site");
    let wasm = splitscript::codegen(&checked);
    Validator::new_with_features(WasmFeatures::all())
        .validate_all(&wasm)
        .expect("generic higher-order helper specializations should validate");
}

#[test]
fn named_functions_share_the_first_class_callable_representation() {
    let source = r#"
        state "game.exe" {}

        fn increment(value: u32) -> u32 {
            return value + 1
        }

        fn identity(value) {
            return value
        }

        fn apply(value, transform) {
            return transform(value)
        }

        whileAttached {
            let incrementLater = increment
            print(incrementLater(4))
            print(apply(8u32, identity))
            for value in [1u32, 2].iterator().map(increment) {
                print(value)
            }
        }
    "#;
    let checked = splitscript::check(splitscript::lower(splitscript::parse(source).unwrap()))
        .expect("named functions should coerce to inferred callable values");
    let wasm = splitscript::codegen(&checked);
    Validator::new_with_features(WasmFeatures::all())
        .validate_all(&wasm)
        .expect("named function adapters should produce valid WebAssembly GC");
}

#[test]
fn async_named_functions_are_callable_values() {
    let source = r#"
        state "game.exe" {}

        fn afterTick(value: u32) -> async u32 {
            await nextTick()
            return value
        }

        onAttach {
            let later = afterTick
            print(await later(4))
        }
    "#;
    let wasm = splitscript::compile(source).expect("async functions should be callable values");
    Validator::new_with_features(WasmFeatures::all())
        .validate_all(&wasm)
        .expect("async named function adapters should validate");
}

#[test]
fn named_function_values_remain_function_editor_symbols() {
    let source = r#"state "game.exe" {}
fn increment(value: u32) -> u32 {
    return value + 1
}
whileAttached {
    let later = increment
    print(later(4))
}"#;
    let mut database = CompilerDatabase::new(source);
    database
        .check()
        .expect("function-value fixture should check");
    let declaration = source.find("increment(value").unwrap();
    let reference = source.rfind("increment").unwrap();
    let definition = database
        .definition_at(reference)
        .unwrap()
        .expect("function values should navigate to their declarations");
    let DefinitionTarget::Source(definition) = definition else {
        panic!("named function values should have source definitions");
    };
    assert_eq!(definition.span.start, declaration);
    assert!(matches!(definition.id, SourceDefinitionId::Function(_)));
    assert_eq!(database.references_at(reference, true).unwrap().len(), 2);
    assert_eq!(
        database
            .rename_at(reference, "advance")
            .unwrap()
            .edits
            .len(),
        2
    );
    let highlights = database.semantic_highlights().unwrap();
    let highlight = highlights
        .highlights()
        .iter()
        .find(|highlight| highlight.span.start == reference)
        .expect("function-value reference should be highlighted");
    assert_eq!(highlight.kind, SemanticTokenKind::Function);
    let hover = database
        .hover(reference)
        .unwrap()
        .expect("function values should retain function hover documentation");
    assert!(
        hover.markdown.contains("fn increment"),
        "{}",
        hover.markdown
    );
}

#[test]
fn closure_parameters_are_first_class_editor_symbols() {
    let source = r#"state "game.exe" {}
whileAttached {
    let combine: (u16, u16) -> u16 = (x, y) => x + y
    print(combine(1, 2))
}"#;
    let mut database = CompilerDatabase::new(source);
    database
        .check()
        .expect("the closure fixture should type-check");

    let declaration_x = source.find("(x, y)").unwrap() + 1;
    let declaration_y = source.find("x, y").unwrap() + "x, ".len();
    let use_x = source.find("=> x + y").unwrap() + "=> ".len();
    let use_y = source.find("x + y").unwrap() + "x + ".len();

    let x_definition = database
        .definition_at(use_x)
        .unwrap()
        .expect("closure parameter use should navigate");
    let DefinitionTarget::Source(x_definition) = x_definition else {
        panic!("closure parameters should be source definitions");
    };
    assert!(matches!(x_definition.id, SourceDefinitionId::Value(_)));
    assert_eq!(
        x_definition.span,
        Span {
            start: declaration_x,
            end: declaration_x + 1,
        }
    );
    assert_eq!(
        database.definition_at(declaration_x).unwrap(),
        Some(DefinitionTarget::Source(x_definition))
    );

    let hover = database
        .hover(use_y)
        .unwrap()
        .expect("closure parameter use should have hover information");
    assert!(hover.markdown.contains("y: u16"), "{}", hover.markdown);
    assert!(hover.markdown.contains("Parameter"), "{}", hover.markdown);

    let hints = database
        .inlay_hints(Span {
            start: 0,
            end: source.len(),
        })
        .unwrap();
    assert!(
        hints
            .iter()
            .any(|hint| hint.position == declaration_x + 1 && hint.label == ": u16")
    );
    assert!(
        hints
            .iter()
            .any(|hint| hint.position == declaration_y + 1 && hint.label == ": u16")
    );
    let closing_parenthesis = source.find(") => x + y").unwrap() + 1;
    assert!(
        hints
            .iter()
            .any(|hint| hint.position == closing_parenthesis && hint.label == " -> u16")
    );

    let highlights = database.semantic_highlights().unwrap();
    for offset in [declaration_x, declaration_y, use_x, use_y] {
        let highlight = highlights
            .highlights()
            .iter()
            .find(|highlight| highlight.span.start <= offset && offset < highlight.span.end)
            .expect("each closure parameter occurrence should be highlighted");
        assert_eq!(highlight.kind, SemanticTokenKind::Parameter);
    }

    assert_eq!(database.references_at(use_x, true).unwrap().len(), 2);
    let renamed = database.rename_at(use_x, "left").unwrap();
    assert_eq!(renamed.edits.len(), 2);
    assert!(
        renamed
            .edits
            .iter()
            .any(|edit| edit.span.start == declaration_x)
    );
    assert!(renamed.edits.iter().any(|edit| edit.span.start == use_x));
}

#[test]
fn closure_hints_render_complete_inferred_signatures() {
    let source = r#"state "game.exe" {}
whileAttached {
    let increment: (u16) -> u32 = value => value as u32 + 1
    print(increment(4))
}"#;
    let mut database = CompilerDatabase::new(source);
    let parameter = source.find("value =>").unwrap();
    let hints = database
        .inlay_hints(Span {
            start: 0,
            end: source.len(),
        })
        .unwrap();

    assert!(
        hints
            .iter()
            .any(|hint| hint.position == parameter && hint.label == "(")
    );
    assert_eq!(
        hints
            .iter()
            .filter(|hint| hint.position == parameter + "value".len())
            .map(|hint| hint.label.as_str())
            .collect::<Vec<_>>(),
        [": u16", ") -> u32"]
    );
}

#[test]
fn explicit_closure_results_constrain_the_body_and_suppress_result_hints() {
    let source = r#"state "game.exe" {}
whileAttached {
    let widen = (value: u16) -> u32 => value as u32
    print(widen(4))
}"#;
    let mut database = CompilerDatabase::new(source);
    database
        .check()
        .expect("explicit closure result annotations should type-check");
    let result = source.find("u32 =>").unwrap();
    assert!(database.definition_at(result).unwrap().is_some());
    let hints = database
        .inlay_hints(Span {
            start: 0,
            end: source.len(),
        })
        .unwrap();
    let closing_parenthesis = source.find(") -> u32").unwrap() + 1;
    assert!(
        hints
            .iter()
            .all(|hint| hint.position != closing_parenthesis || !hint.label.starts_with(" ->"))
    );

    let invalid = source.replace("value as u32", "false");
    let diagnostics = CompilerDatabase::new(invalid)
        .check()
        .expect_err("the explicit result must constrain the closure body");
    assert!(
        diagnostics.iter().any(|diagnostic| {
            diagnostic.message.contains("expected `u32`, found `bool`")
                && diagnostic.labels.iter().any(|label| {
                    label.message.as_deref().is_some_and(|message| {
                        message.contains("closure is declared to return `u32`")
                    })
                })
        }),
        "{diagnostics:#?}"
    );
}

#[test]
fn explicit_async_closure_results_describe_the_future_value() {
    let source = r#"state "game.exe" {}
onAttach {
    let delayed: (u16) -> async u16 = (value: u16) -> async u16 => {
        await nextTick()
        value
    }
    print(await delayed(4))
}"#;
    splitscript::check(splitscript::lower(splitscript::parse(source).unwrap()))
        .expect("explicit async closure results should type-check");
}

#[test]
fn closure_returns_target_the_closure_instead_of_the_enclosing_function() {
    let source = r#"state "game.exe" {}

fn makeAnswer() -> () -> u32 {
    return () => {
        return 42
    }
}

onAttach {
    print(makeAnswer()())
}"#;
    let checked = splitscript::check(splitscript::lower(splitscript::parse(source).unwrap()))
        .expect("return inside a closure should complete that closure");
    wasmparser::Validator::new_with_features(wasmparser::WasmFeatures::all())
        .validate_all(&splitscript::codegen(&checked))
        .expect("closure-local returns should lower to valid WebAssembly");
}

#[test]
fn closures_cannot_break_or_continue_enclosing_loops() {
    for (control_flow, expected) in [
        ("break", "`break` is only available inside a loop"),
        ("continue", "`continue` is only available inside a loop"),
    ] {
        let source = format!(
            r#"state "game.exe" {{}}

onAttach {{
    while true {{
        let callback = () => {control_flow}
        callback()
    }}
}}"#
        );
        let diagnostics =
            splitscript::check(splitscript::lower(splitscript::parse(&source).unwrap()))
                .expect_err("a closure must not escape into its enclosing loop");
        assert!(
            diagnostics
                .iter()
                .any(|diagnostic| diagnostic.message == expected),
            "{diagnostics:#?}"
        );
    }
}
