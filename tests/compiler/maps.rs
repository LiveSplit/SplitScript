use wasmparser::{Validator, WasmFeatures};

use splitscript::{
    compiler::stdlib::{StdlibFieldId, StdlibSymbolId},
    tooling::database::{CompilerDatabase, DefinitionTarget},
};

#[test]
fn maps_support_lookup_mutation_and_entry_iteration() {
    let source = r#"
        state "game.exe" {}

        whileAttached {
            let routes = Map.new<String, u32>()
            let inserted = routes.insert("Atrium", 12)
            routes["Vault"] = 20
            routes["Atrium"] = 14
            if routes.containsKey("Atrium") {
                print(routes["Atrium"])
            }
            print(routes)
            for { key, value } in routes {
                print(`{key}: {value}`)
            }
            for entry in routes {
                print(entry)
            }
            let removed = routes.remove("Vault")
            print(`{inserted} {removed} {routes.length()} {routes.isEmpty()}`)
            routes.clear()
        }
    "#;
    let wasm = splitscript::compile(source).expect("maps should compile");
    Validator::new_with_features(WasmFeatures::all())
        .validate_all(&wasm)
        .expect("map lowering should produce valid Wasm GC");
}

#[test]
fn empty_maps_infer_key_and_value_types_from_later_uses() {
    let source = r#"
        state "game.exe" {}

        whileAttached {
            let routes = Map.new()
            routes.insert("Atrium", 12)
            let value: u32 = routes["Atrium"]
            print(value)
        }
    "#;
    splitscript::compile(source).expect("map operations should infer both type arguments");
}

#[test]
fn unconstrained_empty_maps_diagnose_each_missing_type_argument() {
    let diagnostics = splitscript::compile(
        r#"
            state "game.exe" {}
            whileAttached {
                let routes = Map.new()
            }
        "#,
    )
    .expect_err("an unused map has no key or value constraints");
    assert!(diagnostics.iter().any(|diagnostic| {
        diagnostic
            .message
            .contains("cannot infer the key type of this empty map")
    }));
    assert!(diagnostics.iter().any(|diagnostic| {
        diagnostic
            .message
            .contains("cannot infer the value type of this empty map")
    }));
}

#[test]
fn map_index_compound_assignment_evaluates_as_a_read_and_write() {
    let source = r#"
        state "game.exe" {}
        whileAttached {
            let routes = Map.new<String, u32>()
            routes["Atrium"] = 12
            routes["Atrium"] += 2
            print(routes["Atrium"])
        }
    "#;
    let wasm = splitscript::compile(source).expect("map compound indexing should compile");
    Validator::new_with_features(WasmFeatures::all())
        .validate_all(&wasm)
        .expect("map compound indexing should produce valid Wasm GC");
}

#[test]
fn indexing_infers_one_capability_with_associated_key_and_value_types() {
    let source = r#"
        state "game.exe" {}

        fn indexed(values, key) {
            return values[key]
        }

        whileAttached {
            let names = ["Ada", "Grace"]
            let first: String = indexed(names, 0)
            let scores = Map.new<String, u32>()
            scores["Ada"] = 12
            let score: u32 = indexed(scores, "Ada")
            print(`{first}: {score}`)
        }
    "#;
    let wasm = splitscript::compile(source)
        .expect("indexing should infer its receiver, key, and value relationship");
    Validator::new_with_features(WasmFeatures::all())
        .validate_all(&wasm)
        .expect("generic indexing should produce valid Wasm GC");

    let mut database = CompilerDatabase::new(source);
    let hover = database
        .hover(source.find("indexed(values").unwrap())
        .unwrap()
        .expect("inferred indexing helper hover");
    assert!(
        hover
            .markdown
            .contains("fn indexed(values: T, key: T.Key) -> T.Value where T: Index"),
        "{}",
        hover.markdown
    );
}

#[test]
fn source_structs_implement_indexing_from_exact_at_and_set_methods() {
    let source = r#"
        state "game.exe" {}

        struct Pair {
            first: u32,
            second: u32,
        }

        fn Pair.at(index: u32) -> u32 {
            if index == 0 {
                return self.first
            }
            return self.second
        }

        fn Pair.set(index: u32, value: u32) -> None {
            print(`setting {index} to {value}`)
        }

        whileAttached {
            let pair = Pair { first: 12, second: 34 }
            print(pair[1])
            pair[1] = 9
            pair[0] += 1
        }
    "#;
    let wasm = splitscript::compile(source)
        .expect("exact source methods should structurally implement indexing");
    Validator::new_with_features(WasmFeatures::all())
        .validate_all(&wasm)
        .expect("source-defined indexing should produce valid Wasm GC");
}

#[test]
fn map_editor_surface_exposes_only_the_approved_lookup_api() {
    let source = r#"
        state "game.exe" {}
        whileAttached {
            let routes = Map.new<String, u32>()
            routes.
        }
    "#;
    let mut database = CompilerDatabase::new(source);
    let offset = source.find("routes.").unwrap() + "routes.".len();
    let labels = database
        .completions(offset)
        .unwrap()
        .items
        .into_iter()
        .map(|item| item.label)
        .collect::<Vec<_>>();
    for method in [
        "length",
        "isEmpty",
        "containsKey",
        "insert",
        "remove",
        "clear",
        "iterator",
    ] {
        assert!(labels.contains(&method.to_owned()), "missing `{method}`");
    }
    for hidden in ["at", "set", "get"] {
        assert!(!labels.contains(&hidden.to_owned()), "leaked `{hidden}`");
    }
}

#[test]
fn collection_constructors_complete_in_expression_positions() {
    for (prefix, expected, hidden) in [("Se", "Set", "SetIterator"), ("Ma", "Map", "MapEntry")] {
        let source = format!(
            r#"
                state "game.exe" {{}}
                whileAttached {{
                    let collection = {prefix}
                }}
            "#
        );
        let mut database = CompilerDatabase::new(source.clone());
        let offset = source.find(prefix).unwrap() + prefix.len();
        let completion = database.completions(offset).unwrap();
        let item = completion
            .items
            .iter()
            .find(|item| item.label == expected)
            .unwrap_or_else(|| {
                panic!(
                    "missing `{expected}` for `{prefix}`: {:#?}",
                    completion.items
                )
            });
        assert_eq!(item.insert_text, expected);
        assert!(!item.is_snippet);
        assert!(
            completion.items.iter().all(|item| item.label != hidden),
            "expression completion leaked `{hidden}`"
        );
    }
}

#[test]
fn map_entry_binding_patterns_complete_public_fields() {
    let source = r#"
        state "game.exe" {}
        whileAttached {
            let routes = Map.new<String, u32>()
            for {
            } in routes {}
        }
    "#;
    let mut database = CompilerDatabase::new(source);
    let offset = source.find("            } in routes").unwrap();
    let labels = database
        .completions(offset)
        .unwrap()
        .items
        .into_iter()
        .map(|item| item.label)
        .collect::<Vec<_>>();
    assert!(
        labels.contains(&"key".to_owned()),
        "missing key: {labels:?}"
    );
    assert!(
        labels.contains(&"value".to_owned()),
        "missing value: {labels:?}"
    );
}

#[test]
fn map_entry_patterns_participate_in_nested_usefulness() {
    let source = r#"
        state "game.exe" {}
        whileAttached {
            let routes = Map.new<String, u32>()
            routes["Atrium"] = 12
            for entry in routes {
                match entry {
                    MapEntry { key: "Atrium", value } => print(value),
                    { key: _, value: _ } => {},
                }
            }
        }
    "#;
    splitscript::compile(source)
        .expect("generic standard-library structs should support nested patterns");
}

#[test]
fn map_entry_pattern_fields_support_hover_and_binding_rename() {
    let shorthand_source = r#"
        state "game.exe" {}
        whileAttached {
            let map = Map.new<String, u32>()
            map.insert("foo", 123)
            for { key, value } in map {
                print(`key: {key}, value: {value}`)
            }
        }
    "#;
    let pattern = shorthand_source.find("for { key, value }").unwrap();
    let key = pattern + "for { ".len();
    let value = pattern + "for { key, ".len();

    let mut database = CompilerDatabase::new(shorthand_source);
    assert_eq!(
        database.definition_at(key).unwrap(),
        Some(DefinitionTarget::StandardLibrarySymbol(
            StdlibSymbolId::Field(StdlibFieldId::MapEntryKey),
        ))
    );
    let hover = database
        .hover(key)
        .unwrap()
        .expect("a catalog struct shorthand should expose both identities");
    assert!(hover.markdown.contains("MapEntry<K, V>.key: K"));
    assert!(
        hover
            .markdown
            .contains("**Value represented by the shorthand**")
    );
    assert!(hover.markdown.contains("key: String"));

    for (offset, new_name, expected) in [
        (key, "name", "key: name"),
        (value, "number", "value: number"),
    ] {
        let mut database = CompilerDatabase::new(shorthand_source);
        let plan = database
            .rename_at(offset, new_name)
            .expect("renaming a catalog struct shorthand should split field and binding names");
        assert!(
            plan.edits
                .iter()
                .any(|edit| { edit.span.start == offset && edit.replacement == expected }),
            "missing `{expected}` shorthand expansion: {plan:#?}"
        );
    }

    let explicit_source = r#"
        state "game.exe" {}
        whileAttached {
            let map = Map.new<String, u32>()
            for { key: name, value: number } in map {
                print(`{name}: {number}`)
            }
        }
    "#;
    let pattern = explicit_source
        .find("for { key: name, value: number }")
        .unwrap();
    let key = pattern + "for { ".len();
    let value = pattern + "for { key: name, ".len();
    let mut database = CompilerDatabase::new(explicit_source);

    assert_eq!(
        database.definition_at(key).unwrap(),
        Some(DefinitionTarget::StandardLibrarySymbol(
            StdlibSymbolId::Field(StdlibFieldId::MapEntryKey),
        ))
    );
    assert_eq!(
        database.definition_at(value).unwrap(),
        Some(DefinitionTarget::StandardLibrarySymbol(
            StdlibSymbolId::Field(StdlibFieldId::MapEntryValue),
        ))
    );
    let hover = database
        .hover(key)
        .unwrap()
        .expect("an explicit catalog struct field should have hover information");
    assert!(hover.markdown.contains("MapEntry<K, V>.key: K"));
    assert!(hover.markdown.contains("Key stored by this entry."));
}

#[test]
fn nested_keyed_collections_derive_structural_equality() {
    for profile in [
        splitscript::BuildProfile::Debug,
        splitscript::BuildProfile::Release,
    ] {
        let wasm = splitscript::compile_with_options(
            include_str!("../keyed_equality.split"),
            splitscript::CompilerOptions {
                profile,
                ..Default::default()
            },
        )
        .unwrap();
        Validator::new_with_features(WasmFeatures::all())
            .validate_all(&wasm)
            .unwrap();
    }
}

#[test]
fn map_equality_requires_equatable_values_without_restricting_map_storage() {
    let source = r#"
        state "game.exe" {}
        fn callback() {}
        whileAttached {
            let values = Map.new()
            values["callback"] = callback
            print(values.length())
        }
    "#;
    splitscript::compile(source).unwrap();
    let source = source.replace("print(values.length())", "print(values == values)");
    let errors = splitscript::compile(&source)
        .map(|wasm| wasm.len())
        .unwrap_err();
    assert!(format!("{errors:?}").contains("equality"));
}
