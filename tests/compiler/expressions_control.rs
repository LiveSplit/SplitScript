//! expressions control integration tests.

use super::*;

#[test]
fn binary_integer_literals_type_check_and_report_boolean_misuse() {
    splitscript::compile(
        r#"
            state GBA {}

            fn binaryValues() -> bool {
                let flags: u8 = 0b1010_0101
                let mask = 0B1111_0000u8
                let maximum: u64 = 0b11111111_11111111_11111111_11111111_11111111_11111111_11111111_11111111
                return flags & mask == 0b1010_0000 && maximum == 0xffff_ffff_ffff_ffffu64
            }
        "#,
    )
    .expect("binary integer literals should compile with inference and suffixes");

    let diagnostics = splitscript::compile("state GBA {} split { return 0b100 || true }")
        .expect_err("an integer is not a boolean operand");
    assert!(
        diagnostics
            .iter()
            .any(|diagnostic| diagnostic.message == "expected `bool`, found an integer literal"),
        "{diagnostics:#?}"
    );
}

#[test]
fn is_patterns_flow_bindings_only_along_proven_boolean_edges() {
    let source = r#"
        state GBA {}

        fn positive(value: u32?) -> u32 {
            if value is Some(number) && number > 0 {
                return number
            }
            return 0
        }

        fn negated(value: u32?) -> u32 {
            if !(value is Some(number)) {
                return 0
            } else {
                return number
            }
        }

        fn falseEdge(value: u32?) -> bool {
            return !(value is Some(number)) || number > 0
        }

        fn guarded(value: u32?, other: u32?) -> u32 {
            return match value {
                Some(left) if other is Some(right) => left + right,
                _ => 0,
            }
        }

        fn loopBinding(value: u32?) {
            while value is Some(number) {
                print(number)
                break
            }
        }

        setup {
            print(positive(Some(1u32)))
            print(negated(Some(2u32)))
            print(falseEdge(Some(3u32)))
            print(guarded(Some(4u32), Some(5u32)))
            loopBinding(Some(6u32))
        }
    "#;
    let wasm = splitscript::compile(source)
        .expect("`is` bindings should follow short-circuit and branch control flow");
    wasmparser::Validator::new_with_features(wasmparser::WasmFeatures::all())
        .validate_all(&wasm)
        .expect("`is` pattern bindings should lower to valid Wasm");

    for invalid in [
        r#"state GBA {} fn bad(value: u32?) { if value is Some(number) || number > 0 {} }"#,
        r#"state GBA {} fn bad(value: u32?) { let matched = value is Some(number); print(number) }"#,
        r#"state GBA {} fn bad(value: u32?) { if value is Some(number) {} else { print(number) } }"#,
    ] {
        let diagnostics = splitscript::compile(invalid)
            .expect_err("a conditional binding must not escape onto an unproven path");
        assert!(
            diagnostics.iter().any(|diagnostic| diagnostic
                .message
                .contains("not available on this control-flow path")),
            "{diagnostics:#?}"
        );
    }
}

#[test]
fn irrefutable_is_patterns_warn_without_rejecting_the_program() {
    let source = r#"
        state GBA {}
        fn inspect(value: u32) {
            if value is _ {
                print(value)
            }
        }
        setup { inspect(7) }
    "#;
    let (wasm, warnings) = splitscript::compile_with_context_and_options_diagnostics(
        splitscript::CompilerContext::default(),
        source,
        splitscript::CompilerOptions::default(),
    )
    .expect("an irrefutable pattern test is legal");
    assert!(warnings.iter().any(|diagnostic| {
        diagnostic.code == splitscript::DiagnosticCode::AlwaysMatchesPattern
            && diagnostic.message.contains("always matches")
    }));
    wasmparser::Validator::new_with_features(wasmparser::WasmFeatures::all())
        .validate_all(&wasm)
        .expect("an irrefutable `is` test should still lower validly");
}

#[test]
fn negative_integer_literals_include_each_signed_minimum_and_reject_unsigned_types() {
    splitscript::compile(
        r#"
            state GBA {}

            fn minima() {
                let byte: i8 = -128
                let word: i16 = -32768
                let dword: i32 = -2147483648
                let qword: i64 = -9223372036854775808
                let exactFloat: f64 = -2147483648
                let inferred = -2147483648
                print(`{byte}:{word}:{dword}:{qword}:{exactFloat}:{inferred}`)
            }
        "#,
    )
    .expect("negative literals should represent every signed minimum directly");

    for source in [
        "state GBA {} fn bad() { let value: u8 = -1 }",
        "state GBA {} fn bad() { let value: i8 = -129 }",
        "state GBA {} fn bad() { let value = -9223372036854775809 }",
    ] {
        let diagnostics = splitscript::compile(source)
            .expect_err("an out-of-range negative literal should be rejected");
        assert!(
            diagnostics
                .iter()
                .any(|diagnostic| diagnostic.message.contains("integer literal does not fit")),
            "{diagnostics:#?}"
        );
    }
}

#[test]
fn user_call_type_mismatches_name_the_argument_and_label_the_parameter() {
    let source = r#"
        struct Pos {
            x: u16,
        }

        fn getX(pos: Pos) -> u16 {
            return pos.x
        }

        fn bar() -> u16 {
            return getX(6)
        }

        state GBA {}
    "#;
    let diagnostics = splitscript::compile(source).expect_err("the argument has the wrong type");
    let diagnostic = diagnostics
        .iter()
        .find(|diagnostic| diagnostic.message == "expected `Pos`, found an integer literal")
        .expect("the call site should explain the source-level mismatch");
    assert_eq!(diagnostic.labels.len(), 2, "{diagnostic:#?}");
    assert_eq!(
        diagnostic.labels[0].message.as_deref(),
        Some("this value is an integer literal")
    );
    assert_eq!(
        diagnostic.labels[1].message.as_deref(),
        Some("parameter `pos` requires `Pos`")
    );
    let parameter_start = source.find("pos: Pos").unwrap();
    assert_eq!(diagnostic.labels[1].span.start, parameter_start);
    assert_eq!(
        diagnostic.labels[1].span.end,
        parameter_start + "pos: Pos".len()
    );
}

#[test]
fn declared_type_mismatches_point_to_every_source_of_the_expectation() {
    let source = r#"
        struct Pos {
            x: u16,
        }

        struct Boxed {
            value: Pos,
        }

        enum Wrapped {
            Value(Pos),
        }

        let globalPos: Pos = 1

        state GBA {
            statePos: Pos = 2;
            filteredPos: Pos at 0x100 if true { 8 } else { value };
        }

        fn badReturn() -> Pos {
            return 3
        }

        fn exercise() {
            let badLocal: Pos = 4
            let localPos: Pos = Pos { x: 0 }
            localPos = 5
            let boxed = Boxed { value: 6 }
            let wrapped = Wrapped.Value(7)
        }
    "#;
    let diagnostics = splitscript::compile(source).expect_err("every value has the wrong type");
    let expectation_labels = diagnostics
        .iter()
        .filter(|diagnostic| diagnostic.message == "expected `Pos`, found an integer literal")
        .filter_map(|diagnostic| diagnostic.labels.get(1))
        .filter_map(|label| label.message.as_deref())
        .collect::<Vec<_>>();

    for expected in [
        "global variable `globalPos` is declared as `Pos`",
        "state field `statePos` is declared as `Pos`",
        "state field `filteredPos` is declared as `Pos`",
        "function `badReturn` is declared to return `Pos`",
        "variable `badLocal` is declared as `Pos`",
        "variable `localPos` has type `Pos`",
        "struct field `Boxed.value` is declared as `Pos`",
        "variant `Wrapped.Value` declares a payload of type `Pos`",
    ] {
        assert!(
            expectation_labels.contains(&expected),
            "missing `{expected}` from {expectation_labels:#?}; diagnostics: {diagnostics:#?}"
        );
    }
}

#[test]
fn expected_type_provenance_survives_wrappers_nesting_and_generic_constraints() {
    let source = r#"
        struct Pos {
            x: u16,
        }

        fn addOne(value) {
            return value + 1
        }

        fn failed() -> i32! {
            return Err("failed")
        }

        fn missing() -> Pos {}

        fn exercise() {
            let positions: [Pos] = [8]
            let maybe: i32? = None
            let optionalPos: Pos = maybe
            let resultPos: Pos = failed()
            let tiny: u8 = 300
            print(addOne(Pos { x: 1 }))
        }

        state GBA {}
    "#;
    let diagnostics = splitscript::compile(source).expect_err("the declarations are incompatible");

    for expected_label in [
        "variable `positions` is declared as `[Pos]`",
        "variable `optionalPos` is declared as `Pos`",
        "variable `resultPos` is declared as `Pos`",
        "variable `tiny` is declared as `u8`",
        "function `missing` is declared to return `Pos`",
        "parameter `value` requires `Numeric`",
    ] {
        assert!(
            diagnostics.iter().any(|diagnostic| diagnostic
                .labels
                .iter()
                .any(|label| label.message.as_deref() == Some(expected_label))),
            "missing `{expected_label}` from {diagnostics:#?}"
        );
    }

    assert!(diagnostics.iter().any(|diagnostic| {
        diagnostic
            .message
            .contains("integer literal does not fit in `u8`")
            && diagnostic.labels[0].message.as_deref()
                == Some("this integer literal does not fit in the declared type")
    }));
    assert!(diagnostics.iter().any(|diagnostic| {
        diagnostic
            .message
            .contains("type `Pos` does not satisfy the required `Numeric` capability")
    }));
}

#[test]
fn literal_setting_keys_are_checked_against_declared_runtime_keys() {
    let source = r#"
        enum Mode { Fast, Slow }
        state "game.exe" {}
        settings {
            "Boss" => boss key "split-boss": true,
            "Mode" => mode key "run-mode": choice {
                "Fast" => Mode.Fast default,
                "Slow" => Mode.Slow,
            },
            "Profile" => profile key "profile-name": "Runner",
            "Paths" {
                "Route" => route: file {},
            },
        }

        whileAttached {
            let enabled = settings.enabled("split-bos")
            let wrongKind = settings.enabled("run-mode")
            let wrongTextKind = settings.enabled("profile-name")
            let heading = settings.contains("_heading0")
            print(enabled)
            print(wrongKind)
            print(wrongTextKind)
            print(heading)
        }
    "#;
    let diagnostics =
        splitscript::compile(source).expect_err("statically invalid setting keys must be rejected");

    let typo = diagnostics
        .iter()
        .find(|diagnostic| {
            diagnostic
                .message
                .contains("unknown setting key `split-bos`")
        })
        .expect("the misspelled key has a focused diagnostic");
    assert_eq!(typo.labels.len(), 2);
    assert_eq!(typo.fixes.len(), 1);
    assert_eq!(typo.fixes[0].edits[0].replacement, "\"split-boss\"");
    assert!(
        typo.notes
            .iter()
            .any(|note| note.contains("computed string keys"))
    );

    let wrong_kind = diagnostics
        .iter()
        .find(|diagnostic| diagnostic.message.contains("names a choice setting"))
        .expect("a non-boolean key is distinguished from an unknown key");
    assert_eq!(wrong_kind.labels.len(), 2);
    assert!(
        wrong_kind
            .notes
            .iter()
            .any(|note| note.contains("settings.mode"))
    );

    let wrong_text_kind = diagnostics
        .iter()
        .find(|diagnostic| diagnostic.message.contains("names a text setting"))
        .expect("a text setting is distinguished from a boolean setting");
    assert_eq!(wrong_text_kind.labels.len(), 2);
    assert!(
        wrong_text_kind
            .notes
            .iter()
            .any(|note| note.contains("settings.profile"))
    );

    let heading = diagnostics
        .iter()
        .find(|diagnostic| diagnostic.message.contains("names a settings heading"))
        .expect("headings are not reported as value settings");
    assert_eq!(heading.labels.len(), 2);
    assert!(
        heading
            .notes
            .iter()
            .all(|note| !note.contains("settings._heading0")),
        "headings must not suggest a nonexistent direct value member"
    );
}

#[test]
fn dynamic_and_compatible_setting_key_lookups_remain_valid() {
    splitscript::compile(
        r#"
            enum Mode { Fast, Slow }
            state "game.exe" {}
            settings {
                "Boss" => boss key "split-boss": true,
                "Mode" => mode key "run-mode": choice {
                    "Fast" => Mode.Fast default,
                    "Slow" => Mode.Slow,
                },
            }

            fn selected(view: SettingsView, key: String) {
                return view.enabled(key) || view.contains(key)
            }

            whileAttached {
                print(settings.enabled("split-boss"))
                print(oldSettings.contains("run-mode"))
                print(selected(settings, currentKey()))
            }

            fn currentKey() { return "split-boss" }
        "#,
    )
    .expect("dynamic keys and compatible exact literals remain supported");
}

#[test]
fn literal_setting_lookups_guide_towards_typed_members() {
    let source = r#"
        enum Mode { Fast, Slow }
        state "game.exe" {}
        settings {
            "Boss" => boss key "split-boss": true,
            "Mode" => mode key "run-mode": choice {
                "Fast" => Mode.Fast default,
                "Slow" => Mode.Slow,
            },
            for stage in 1..=2 {
                `Stage {stage}` key `stage-{stage}`: true,
            },
        }

        whileAttached {
            print(settings.enabled("split-boss"))
            print(oldSettings.enabled("split-boss"))
            print(settings.contains("split-boss"))
            print(settings.contains("run-mode"))
            print(settings.enabled("stage-1"))
        }
    "#;
    let checked = splitscript::check(splitscript::parse(source).unwrap())
        .expect("static setting guidance should remain a warning");
    let diagnostics = checked
        .diagnostics()
        .iter()
        .filter(|diagnostic| diagnostic.code == splitscript::DiagnosticCode::StaticSettingLookup)
        .collect::<Vec<_>>();
    assert_eq!(diagnostics.len(), 4, "{diagnostics:#?}");

    let enabled = diagnostics
        .iter()
        .find(|diagnostic| {
            diagnostic.message.contains("typed member")
                && diagnostic.fixes[0].edits[0].replacement == "settings.boss"
        })
        .expect("the current typed boolean member should be suggested");
    assert_eq!(
        enabled.fixes[0].applicability,
        splitscript::FixApplicability::MachineApplicable
    );
    assert!(diagnostics.iter().any(|diagnostic| {
        diagnostic.message.contains("typed member")
            && diagnostic.fixes[0].edits[0].replacement == "oldSettings.boss"
    }));

    let bool_membership = diagnostics
        .iter()
        .find(|diagnostic| {
            diagnostic.message.contains("contains")
                && diagnostic
                    .notes
                    .iter()
                    .any(|note| note.contains("settings.boss"))
        })
        .expect("known boolean membership should explain typed value access");
    assert_eq!(bool_membership.fixes.len(), 2);
    assert_eq!(bool_membership.fixes[0].edits[0].replacement, "true");
    assert_eq!(
        bool_membership.fixes[0].applicability,
        splitscript::FixApplicability::MachineApplicable
    );
    assert_eq!(
        bool_membership.fixes[1].edits[0].replacement,
        "settings.boss"
    );
    assert_eq!(
        bool_membership.fixes[1].applicability,
        splitscript::FixApplicability::MaybeIncorrect
    );

    let choice_membership = diagnostics
        .iter()
        .find(|diagnostic| {
            diagnostic.message.contains("contains")
                && diagnostic
                    .notes
                    .iter()
                    .any(|note| note.contains("settings.mode"))
        })
        .expect("known choice membership should explain typed value access");
    assert_eq!(choice_membership.fixes.len(), 1);
    assert_eq!(choice_membership.fixes[0].edits[0].replacement, "true");
}

#[test]
fn computed_and_generated_setting_lookups_do_not_suggest_typed_members() {
    let source = r#"
        state "game.exe" {}
        settings {
            "Boss" => boss key "split-boss": true,
            for stage in 1..=2 {
                `Stage {stage}` key `stage-{stage}`: true,
            },
        }

        fn selectedKey() { return "split-boss" }
        whileAttached {
            print(settings.enabled(selectedKey()))
            print(settings.enabled("stage-1"))
        }
    "#;
    let checked = splitscript::check(splitscript::parse(source).unwrap()).unwrap();
    assert!(
        checked.diagnostics().iter().all(|diagnostic| {
            diagnostic.code != splitscript::DiagnosticCode::StaticSettingLookup
        })
    );
}

#[test]
fn unused_settings_warn_without_guessing_through_dynamic_keys() {
    let source = r#"
        state "game.exe" {}
        settings {
            "General" {
                "Direct" => direct: true,
                "Previous" => previous: true,
                "Literal key" => literalKey key "literal-key": true,
                "Unused explicit" => unusedExplicit key "stable-key": true,
                "Unused implicit" => unusedImplicit: true,
                "Reserved" => _reserved: true,
                for stage in 1..=2 {
                    `Stage {stage}` key `stage-{stage}`: true,
                },
            },
        }

        whileAttached {
            print(settings.direct)
            print(oldSettings.previous)
            print(settings.enabled("literal-key"))
        }
    "#;
    let checked = splitscript::check(splitscript::parse(source).unwrap())
        .expect("unused settings should be warnings, not compilation errors");
    let unused = checked
        .diagnostics()
        .iter()
        .filter(|diagnostic| diagnostic.message.starts_with("unused setting"))
        .collect::<Vec<_>>();
    assert_eq!(unused.len(), 2, "{unused:#?}");

    let explicit = unused
        .iter()
        .find(|diagnostic| diagnostic.message.contains("unusedExplicit"))
        .expect("the unused setting with an explicit key should be reported");
    assert_eq!(explicit.code, splitscript::DiagnosticCode::UnusedMember);
    assert_eq!(explicit.fixes.len(), 1);
    assert_eq!(explicit.fixes[0].edits[0].replacement, "_unusedExplicit");

    let implicit = unused
        .iter()
        .find(|diagnostic| diagnostic.message.contains("unusedImplicit"))
        .expect("the unused setting with an implicit key should be reported");
    assert_eq!(implicit.fixes.len(), 1);
    assert_eq!(
        implicit.fixes[0].edits[0].replacement,
        "_unusedImplicit key \"unusedImplicit\""
    );

    let mut edits = unused
        .iter()
        .flat_map(|diagnostic| diagnostic.fixes[0].edits.iter())
        .collect::<Vec<_>>();
    edits.sort_by_key(|edit| std::cmp::Reverse(edit.span.start));
    let mut fixed = source.to_owned();
    for edit in edits {
        fixed.replace_range(edit.span.start..edit.span.end, &edit.replacement);
    }
    splitscript::compile(&fixed).expect("unused-setting fixes should preserve valid settings");

    let dynamic = r#"
        state "game.exe" {}
        settings {
            "First" => first: true,
            "Second" => second: true,
        }

        fn selectedKey() { return "first" }
        whileAttached {
            print(settings.enabled(selectedKey()))
        }
    "#;
    let checked = splitscript::check(splitscript::parse(dynamic).unwrap())
        .expect("computed setting-key lookup should remain valid");
    assert!(
        checked
            .diagnostics()
            .iter()
            .all(|diagnostic| !diagnostic.message.starts_with("unused setting")),
        "a dynamic key can address any declared setting: {:#?}",
        checked.diagnostics()
    );
}

#[test]
fn discarded_must_use_values_warn_without_failing_compilation() {
    let source = r#"
        state "game.exe" {}

        fn maybeValue() -> i32? {
            return None
        }

        whileAttached {
            "abc".replaceAll("a", "b")
            255u8.toString(16)
            maybeValue()
            timer.state()

            let values = Set.new<i32>()
            values.insert(1)

            let replaced = "abc".replaceAll("a", "b") else "abc"
            let optional = maybeValue()
            print(replaced)
            print(optional else 0)
        }
    "#;
    let checked = splitscript::check(splitscript::lower(splitscript::parse(source).unwrap()))
        .expect("warnings must not reject a valid program");
    assert_eq!(checked.diagnostics().len(), 4);
    assert!(checked.diagnostics().iter().all(|diagnostic| {
        diagnostic.severity == splitscript::DiagnosticSeverity::Warning
            && diagnostic.code == splitscript::DiagnosticCode::MustUse
            && diagnostic.message.starts_with("unused result of")
    }));
    assert!(checked.diagnostics().iter().any(|diagnostic| {
        diagnostic.message.contains("String.replaceAll")
            && diagnostic
                .notes
                .iter()
                .any(|note| note.contains("immutable"))
    }));
    assert!(checked.diagnostics().iter().any(|diagnostic| {
        diagnostic.message.contains("Integer.toString")
            && diagnostic
                .notes
                .iter()
                .any(|note| note.contains("does not modify its receiver"))
    }));
    assert!(checked.diagnostics().iter().any(|diagnostic| {
        diagnostic.message.contains("Option")
            && diagnostic
                .notes
                .iter()
                .any(|note| note.contains("inspected"))
    }));
    assert!(checked.diagnostics().iter().any(|diagnostic| {
        diagnostic.message.contains("timer.state")
            && diagnostic
                .notes
                .iter()
                .any(|note| note.contains("only produces"))
    }));

    let (wasm, warnings) = splitscript::compile_with_context_and_options_diagnostics(
        splitscript::CompilerContext::default(),
        source,
        splitscript::CompilerOptions::default(),
    )
    .expect("one-shot compilation should preserve warnings and emit an artifact");
    assert_eq!(warnings, checked.diagnostics());
    Validator::new_with_features(WasmFeatures::all())
        .validate_all(&wasm)
        .expect("discarded values should lower to valid Wasm");
}

#[test]
fn warning_policy_filters_or_denies_without_changing_semantic_checking() {
    let source = r#"
        state "game.exe" {}
        whileAttached {
            "abc".replaceAll("a", "b")
            let unread = 1
        }
    "#;
    let checked = splitscript::check(splitscript::lower(splitscript::parse(source).unwrap()))
        .expect("raw semantic checking should retain non-fatal warnings");
    assert_eq!(checked.diagnostics().len(), 2);

    let mut warnings = splitscript::WarningPolicy::default();
    assert!(warnings.set(
        splitscript::DiagnosticCode::MustUse,
        splitscript::WarningLevel::Allow,
    ));
    assert!(warnings.set(
        splitscript::DiagnosticCode::UnusedBinding,
        splitscript::WarningLevel::Deny,
    ));
    let diagnostics = splitscript::compile_with_options(
        source,
        splitscript::CompilerOptions {
            warnings,
            ..splitscript::CompilerOptions::default()
        },
    )
    .expect_err("a denied warning should reject the configured build");
    assert_eq!(diagnostics.len(), 1, "{diagnostics:#?}");
    assert_eq!(
        diagnostics[0].code,
        splitscript::DiagnosticCode::UnusedBinding
    );
    assert_eq!(
        diagnostics[0].severity,
        splitscript::DiagnosticSeverity::Error
    );
    assert!(
        diagnostics[0]
            .notes
            .iter()
            .any(|note| note.contains("denied by the active warning policy"))
    );
}

#[test]
fn unused_bindings_warn_by_identity_and_support_intentional_underscores() {
    let source = r#"
        state "game.exe" {}

        fn inspect(unusedParameter: i32, usedParameter: i32) {
            let unusedLocal = 1
            let _unusedLocal = 9
            let _intentional = 2
            let writtenOnly = 3
            writtenOnly = 4
            let compound = 1
            compound += 1
            let receiver = "level"
            receiver.byteLength()

            for unusedItem in [1] {
                print("tick")
            }

            let optional: i32? = Some(1)
            print(match optional {
                Some(unusedPayload) => "present",
                None => "absent"
            })
            print(usedParameter)
        }

        onAttach {
            let unusedModule = await process.module("game.exe")
            let _ignoredModule = await process.module("game.exe")
        }

        whileAttached {
            inspect(1, 2)
        }
    "#;
    let checked = splitscript::check(splitscript::lower(splitscript::parse(source).unwrap()))
        .expect("unused bindings should be non-fatal warnings");
    let unused = checked
        .diagnostics()
        .iter()
        .filter(|diagnostic| diagnostic.code == splitscript::DiagnosticCode::UnusedBinding)
        .collect::<Vec<_>>();
    assert_eq!(unused.len(), 6, "{unused:#?}");
    assert!(
        unused
            .iter()
            .all(|diagnostic| diagnostic.code == splitscript::DiagnosticCode::UnusedBinding)
    );
    for name in [
        "unusedParameter",
        "unusedLocal",
        "writtenOnly",
        "unusedItem",
        "unusedPayload",
        "unusedModule",
    ] {
        let diagnostic = unused
            .iter()
            .find(|diagnostic| diagnostic.message.contains(&format!("`{name}`")))
            .unwrap_or_else(|| panic!("missing unused warning for {name}: {unused:#?}"));
        assert_eq!(&source[diagnostic.span.start..diagnostic.span.end], name);
        assert_eq!(diagnostic.fixes.len(), 1);
        assert_eq!(
            diagnostic.fixes[0].applicability,
            splitscript::FixApplicability::MachineApplicable
        );
    }
    for name in [
        "_intentional",
        "_unusedLocal",
        "_ignoredModule",
        "compound",
        "receiver",
        "usedParameter",
    ] {
        assert!(
            unused
                .iter()
                .all(|diagnostic| !diagnostic.message.ends_with(&format!("`{name}`"))),
            "unexpected unused warning for {name}: {unused:#?}"
        );
    }

    let written = unused
        .iter()
        .find(|diagnostic| diagnostic.message.contains("`writtenOnly`"))
        .unwrap();
    assert_eq!(written.fixes[0].edits.len(), 2);
    assert_eq!(
        written.fixes[0]
            .edits
            .iter()
            .map(|edit| &source[edit.span.start..edit.span.end])
            .collect::<Vec<_>>(),
        ["writtenOnly", "writtenOnly"]
    );
    let local = unused
        .iter()
        .find(|diagnostic| diagnostic.message.contains("`unusedLocal`"))
        .unwrap();
    assert_eq!(local.fixes[0].edits[0].replacement, "__unusedLocal");
}

#[test]
fn unused_analysis_tracks_each_destructured_leaf() {
    let source = r#"
        struct Point { x: i32, y: i32 }
        let Point { x: usedGlobal, y: unusedGlobal } = Point { x: 1, y: 2 }
        state "game.exe" {}

        fn inspect(Point { x: usedParameter, y: unusedParameter }: Point) {
            print(usedParameter + usedGlobal)
        }

        setup {
            inspect(Point { x: 3, y: 4 })
        }
    "#;
    let checked = splitscript::check(splitscript::lower(splitscript::parse(source).unwrap()))
        .expect("unused destructured leaves should remain warnings");
    let unused = checked
        .diagnostics()
        .iter()
        .filter(|diagnostic| {
            matches!(
                diagnostic.code,
                splitscript::DiagnosticCode::UnusedBinding
                    | splitscript::DiagnosticCode::UnusedDeclaration
            )
        })
        .map(|diagnostic| diagnostic.message.as_str())
        .collect::<Vec<_>>();
    assert!(
        unused
            .iter()
            .any(|message| message.contains("`unusedParameter`")),
        "{unused:#?}"
    );
    assert!(
        unused
            .iter()
            .any(|message| message.contains("`unusedGlobal`")),
        "{unused:#?}"
    );
    assert!(
        !unused
            .iter()
            .any(|message| message.contains("`usedParameter`")),
        "{unused:#?}"
    );
    assert!(
        !unused
            .iter()
            .any(|message| message.contains("`usedGlobal`")),
        "{unused:#?}"
    );
}

#[test]
fn unused_declarations_follow_reachable_calls_and_global_reads() {
    let source = r#"
        enum LiveKind {
            Active,
            Dormant
        }

        struct LiveStruct {
            kind: LiveKind,
            ignored: i32,
            _reserved: i32
        }

        enum DeadKind {
            Inactive
        }

        struct DeadStruct {
            kind: DeadKind
        }

        enum _IntentionalEnum {
            Reserved
        }

        struct _IntentionalStruct {
            value: i32
        }

        let stateRoot = 1
        let actionRoot = 2
        let deadGlobal = 3
        let _intentionalGlobal = 4

        state "game.exe" {
            value = stateRoot
        }

        fn reachableRoot() {
            reachableLeaf()
            reachableType().kind
        }

        fn reachableLeaf() {
            print(actionRoot)
        }

        fn reachableType() -> LiveStruct {
            return LiveStruct {
                kind: LiveKind.Active,
                ignored: 1,
                _reserved: 2
            }
        }

        fn deadParent() {
            deadLeaf()
        }

        fn deadLeaf() {
            print("dead")
        }

        fn deadRecursive() {
            deadRecursive()
        }

        fn deadTyped(_value: DeadStruct) {}

        fn _intentionalFunction() {
            print("reserved")
        }

        whileAttached {
            reachableRoot()
        }
    "#;
    let checked = splitscript::check(splitscript::lower(splitscript::parse(source).unwrap()))
        .expect("unused declarations should be non-fatal warnings");
    let unused = checked
        .diagnostics()
        .iter()
        .filter(|diagnostic| {
            diagnostic.message.starts_with("unused global")
                || diagnostic.message.starts_with("unused function")
                || diagnostic.message.starts_with("unused struct")
                || diagnostic.message.starts_with("unused enum")
        })
        .collect::<Vec<_>>();

    assert!(unused.iter().all(|diagnostic| {
        diagnostic.code
            == if diagnostic.message.starts_with("unused struct field")
                || diagnostic.message.starts_with("unused enum variant")
            {
                splitscript::DiagnosticCode::UnusedMember
            } else {
                splitscript::DiagnosticCode::UnusedDeclaration
            }
    }));

    for (name, source_name) in [
        ("deadGlobal", "deadGlobal"),
        ("deadParent", "deadParent"),
        ("deadLeaf", "deadLeaf"),
        ("deadRecursive", "deadRecursive"),
        ("deadTyped", "deadTyped"),
        ("DeadStruct", "DeadStruct"),
        ("DeadKind", "DeadKind"),
        ("LiveStruct.ignored", "ignored"),
        ("LiveKind.Dormant", "Dormant"),
    ] {
        let diagnostic = unused
            .iter()
            .find(|diagnostic| diagnostic.message.ends_with(&format!("`{name}`")))
            .unwrap_or_else(|| panic!("missing unused warning for {name}: {unused:#?}"));
        assert_eq!(
            &source[diagnostic.span.start..diagnostic.span.end],
            source_name
        );
        assert!(diagnostic.fixes.is_empty());
    }
    for name in [
        "stateRoot",
        "actionRoot",
        "reachableRoot",
        "reachableLeaf",
        "reachableType",
        "LiveStruct",
        "LiveKind",
        "LiveStruct.kind",
        "LiveStruct._reserved",
        "LiveKind.Active",
        "_intentionalGlobal",
        "_intentionalFunction",
        "_IntentionalStruct",
        "_IntentionalEnum",
    ] {
        assert!(
            unused
                .iter()
                .all(|diagnostic| !diagnostic.message.ends_with(&format!("`{name}`"))),
            "unexpected unused warning for {name}: {unused:#?}"
        );
    }
}

#[test]
fn debug_only_reachability_warns_transitively_and_offers_erasure_fixes() {
    let source = r#"
        let diagnosticValue = makeDiagnosticValue()

        state "game.exe" {}

        fn makeDiagnosticValue() -> String {
            return diagnosticLeaf()
        }

        fn diagnosticLeaf() -> String {
            return "debug-profile-marker"
        }

        fn callback(value: i32) -> i32 {
            return value + 1
        }

        fn debugReachedContainer() {
            let nestedOnlyDebug = 8
            debug print(nestedOnlyDebug)
        }

        debug fn alreadyDebug() {
            let nestedDebugLocal = 9
            print(nestedDebugLocal)
        }

        whileAttached {
            let localValue = 7
            debug print(localValue)
            debug print(diagnosticValue)
            debug let callbackValue = callback
            debug print(callbackValue(1))
            debug debugReachedContainer()
            debug alreadyDebug()
        }
    "#;
    let checked = splitscript::check(splitscript::parse(source).unwrap())
        .expect("profile-aware unused diagnostics should be non-fatal warnings");
    let warnings = checked
        .diagnostics()
        .iter()
        .filter(|diagnostic| diagnostic.code == splitscript::DiagnosticCode::DebugOnlyUse)
        .collect::<Vec<_>>();

    for name in [
        "diagnosticValue",
        "makeDiagnosticValue",
        "diagnosticLeaf",
        "callback",
        "debugReachedContainer",
        "localValue",
    ] {
        let warning = warnings
            .iter()
            .find(|diagnostic| diagnostic.message.contains(&format!("`{name}`")))
            .unwrap_or_else(|| panic!("missing debug-only-use warning for {name}: {warnings:#?}"));
        let [fix] = warning.fixes.as_slice() else {
            panic!("{name} should have one safe debug-modifier fix: {warning:#?}");
        };
        assert_eq!(
            fix.applicability,
            splitscript::FixApplicability::MachineApplicable
        );
        assert_eq!(fix.edits[0].replacement, "debug ");
        assert_eq!(fix.edits[0].span.start, fix.edits[0].span.end);
    }

    assert!(
        warnings.iter().all(|diagnostic| {
            !diagnostic.message.contains("callbackValue")
                && !diagnostic.message.contains("alreadyDebug")
                && !diagnostic.message.contains("nestedDebugLocal")
                && !diagnostic.message.contains("nestedOnlyDebug")
        }),
        "declarations already inside debug-only code must not receive redundant guidance"
    );
    assert!(checked.diagnostics().iter().all(|diagnostic| {
        !matches!(
            diagnostic.code,
            splitscript::DiagnosticCode::UnusedBinding
                | splitscript::DiagnosticCode::UnusedDeclaration
        )
    }));

    let mut fixed = source.to_owned();
    let mut edits = warnings
        .iter()
        .flat_map(|warning| warning.fixes.iter())
        .flat_map(|fix| fix.edits.iter())
        .collect::<Vec<_>>();
    edits.sort_by_key(|edit| std::cmp::Reverse(edit.span.start));
    for edit in edits {
        fixed.replace_range(edit.span.start..edit.span.end, &edit.replacement);
    }
    let release = splitscript::compile_with_options(
        &fixed,
        splitscript::CompilerOptions {
            profile: splitscript::BuildProfile::Release,
            ..splitscript::CompilerOptions::default()
        },
    )
    .expect("applying the suggested modifiers should produce a release build");
    assert!(
        !contains_string_literal(&release, b"debug-profile-marker"),
        "release reachability should erase the complete debug-only helper chain"
    );
}

#[test]
fn release_visible_consumers_keep_profile_aware_unused_analysis_quiet() {
    let source = r#"
        let shared = 1

        state "game.exe" {}

        fn sharedHelper() -> i32 { return shared }

        whileAttached {
            print(sharedHelper())
            debug print(sharedHelper())
            let local = shared
            print(local)
            debug print(local)
        }
    "#;
    let checked = splitscript::check(splitscript::parse(source).unwrap()).unwrap();
    assert!(
        checked
            .diagnostics()
            .iter()
            .all(|diagnostic| diagnostic.code != splitscript::DiagnosticCode::DebugOnlyUse),
        "release-visible reads must suppress debug-only-use warnings: {:#?}",
        checked.diagnostics()
    );
}

#[test]
fn release_visible_writes_prevent_an_unsafe_debug_modifier_fix() {
    let source = r#"
        let detail = 0
        state "game.exe" {}
        whileAttached {
            detail = 1
            debug print(detail)
        }
    "#;
    let checked = splitscript::check(splitscript::parse(source).unwrap()).unwrap();
    let warning = checked
        .diagnostics()
        .iter()
        .find(|diagnostic| diagnostic.code == splitscript::DiagnosticCode::DebugOnlyUse)
        .expect("the release-retained global should still receive guidance");
    assert_eq!(
        warning.message,
        "global `detail` is only read by debug code"
    );
    assert!(
        warning.fixes.is_empty(),
        "the release assignment would become invalid"
    );
    assert!(warning.notes.iter().any(|note| note.contains("assigned")));
}

#[test]
fn unused_state_fields_follow_snapshot_reads_and_candidate_dependencies() {
    let source = r#"
        state "game.exe" {
            base = 1;
            observed = base + 1;
            unusedBase = 2;
            unusedDerived = unusedBase + 1;
            effectOnly = pollOnly();
            replaced = 4;
            compounded = 5;
            _intentional = 0;
        }

        fn pollOnly() {
            print("the polling expression still executes")
            return 3
        }

        whileAttached {
            current.replaced = 6
            current.compounded += 1
            setVariable("Observed", current.observed)
        }
    "#;
    let checked = splitscript::check(splitscript::parse(source).unwrap())
        .expect("unused state fields should be non-fatal warnings");
    let unused = checked
        .diagnostics()
        .iter()
        .filter(|diagnostic| diagnostic.message.starts_with("unused state field"))
        .collect::<Vec<_>>();

    assert_eq!(unused.len(), 4, "{unused:#?}");
    for name in ["unusedBase", "unusedDerived", "effectOnly", "replaced"] {
        let diagnostic = unused
            .iter()
            .find(|diagnostic| diagnostic.message.ends_with(&format!("`{name}`")))
            .unwrap_or_else(|| panic!("missing unused warning for {name}: {unused:#?}"));
        assert_eq!(diagnostic.code, splitscript::DiagnosticCode::UnusedMember);
        assert_eq!(&source[diagnostic.span.start..diagnostic.span.end], name);
        assert!(diagnostic.fixes.is_empty());
    }
    for name in ["base", "observed", "compounded", "_intentional"] {
        assert!(
            unused
                .iter()
                .all(|diagnostic| !diagnostic.message.ends_with(&format!("`{name}`"))),
            "unexpected unused warning for {name}: {unused:#?}"
        );
    }
    assert!(
        checked
            .diagnostics()
            .iter()
            .all(|diagnostic| diagnostic.message != "unused function `pollOnly`"),
        "an effectful helper called by polling remains execution-reachable"
    );
}

#[test]
fn shared_shape_state_fields_produce_one_logical_unused_warning() {
    let source = r#"
        enum Build { Steam, GOG }
        let build: Build
        state "game.exe" {
            if build == Build.Steam {
                level: u32 at 0x100;
                spare: u32 at 0x104;
            } else {
                level: u32 at 0x200;
                spare: u32 at 0x204;
            }
        }

        onAttach { build = Build.Steam }
        split { return current.level != old.level }
    "#;
    let checked = splitscript::check(splitscript::parse(source).unwrap())
        .expect("shared shape fields should be analyzed by logical identity");
    let unused = checked
        .diagnostics()
        .iter()
        .filter(|diagnostic| diagnostic.message.starts_with("unused state field"))
        .collect::<Vec<_>>();

    assert_eq!(unused.len(), 1, "{unused:#?}");
    assert_eq!(unused[0].message, "unused state field `spare`");
    assert_eq!(unused[0].labels.len(), 2, "{unused:#?}");
    assert_eq!(&source[unused[0].span.start..unused[0].span.end], "spare");
}

#[test]
fn structural_equality_observes_complete_struct_and_enum_shapes() {
    let source = r#"
        struct Pair {
            left: i32,
            right: i32
        }

        enum Mode {
            First,
            Second
        }

        state "game.exe" {}

        fn structsEqual(left: Pair, right: Pair) -> bool {
            return left == right
        }

        fn modesEqual(left: Mode, right: Mode) -> bool {
            return left == right
        }

        fn structsDiffer(left: Pair, right: Pair) -> bool {
            return left.notEquals(right)
        }

        whileAttached {
            if structsEqual(
                Pair { left: 1, right: 2 },
                Pair { left: 1, right: 2 }
            ) {}
            if modesEqual(Mode.First, Mode.First) {}
            if structsDiffer(
                Pair { left: 1, right: 2 },
                Pair { left: 1, right: 3 }
            ) {}
        }
    "#;
    let checked = splitscript::check(splitscript::lower(splitscript::parse(source).unwrap()))
        .expect("structural equality should type check");
    let member_warnings = checked
        .diagnostics()
        .iter()
        .filter(|diagnostic| {
            diagnostic.message.starts_with("unused struct field")
                || diagnostic.message.starts_with("unused enum variant")
        })
        .collect::<Vec<_>>();
    assert!(member_warnings.is_empty(), "{member_warnings:#?}");
    let resolved_items = checked
        .semantics()
        .calls()
        .filter_map(|(_, call)| match call {
            ResolvedCall::StandardLibrary { item, .. } => Some(*item),
            _ => None,
        })
        .collect::<Vec<_>>();
    assert!(resolved_items.contains(&StdlibItemId::EquatableEquals));
    assert!(resolved_items.contains(&StdlibItemId::EquatableNotEquals));
    Validator::new_with_features(WasmFeatures::all())
        .validate_all(&splitscript::codegen(&checked))
        .expect("catalog-resolved structural equality should produce valid Wasm");
}

#[test]
fn implicit_display_marks_custom_formatters_and_derived_fields_as_used() {
    let source = r#"
        struct RawPosition {
            x: u16,
            y: u16,
        }

        struct Position {
            x: u16,
            y: u16,
        }

        state "game.exe" {}

        fn Position.toString() -> String {
            return `({self.x}, {self.y})`
        }

        setup {
            setVariable("Raw", RawPosition { x: 1, y: 2 })
            setVariable("Position", Position { x: 3, y: 4 })
        }
    "#;
    let checked = splitscript::check(splitscript::parse(source).unwrap())
        .expect("implicit Display implementations should be reachable");
    let unused_protocol_declarations = checked
        .diagnostics()
        .iter()
        .filter(|diagnostic| {
            diagnostic.message.contains("RawPosition")
                || diagnostic.message.contains("Position")
                || diagnostic.message.contains("toString")
        })
        .collect::<Vec<_>>();
    assert!(
        unused_protocol_declarations.is_empty(),
        "{unused_protocol_declarations:#?}"
    );
}

#[test]
fn if_expressions_infer_branches_bidirectionally_and_lower_to_wasm() {
    let source = r#"
        enum Selected {
            Number(u16),
            Text(String)
        }

        state "game.exe" {
            selected = if useText {
                Selected.Text("DLC")
            } else {
                Selected.Number(process.read<u16>(0x1234 as address) else 0)
            }
        }

        let useText = false

        fn selectedText(value: Selected) {
            return match value {
                Selected.Number(number) => number as String,
                Selected.Text(text) => text
            }
        }

        whileAttached {
            let inferred: u16 = if useText { 1 } else if !useText { 2 } else { 3 }
            setVariable("Selected", selectedText(current.selected))
            setVariable("Inferred", inferred as String)
        }
    "#;
    let checked = splitscript::check(splitscript::parse(source).unwrap())
        .expect("if expressions should compile");
    let lowered = splitscript::lower_wasm(&checked);
    let mut if_expressions = 0;
    for expression in checked.typed_hir().expressions() {
        let splitscript::compiler::hir::TypedExpressionKind::If {
            condition,
            then_expr,
            else_expr,
        } = &expression.kind
        else {
            continue;
        };
        let splitscript::compiler::wasm_ir::ExpressionKind::If {
            condition: lowered_condition,
            then_expr: lowered_then,
            else_expr: lowered_else,
        } = &lowered
            .expression(expression.id)
            .expect("if expression should have a Wasm IR plan")
            .kind
        else {
            panic!("expression-valued if must not remain deferred to typed HIR")
        };
        assert_eq!(
            (*lowered_condition, *lowered_then, *lowered_else),
            (*condition, *then_expr, *else_expr)
        );
        if_expressions += 1;
    }
    assert!(
        if_expressions >= 3,
        "nested if expressions should all lower"
    );

    let wasm = splitscript::codegen(&checked);
    Validator::new_with_features(WasmFeatures::all())
        .validate_all(&wasm)
        .expect("if expressions should produce valid WebAssembly GC");
}

#[test]
fn if_expressions_require_an_else_and_matching_branch_types() {
    let missing_else = r#"
        state "game.exe" {}
        whileAttached { let value = if true { 1 } }
    "#;
    let errors = splitscript::compile(missing_else).unwrap_err();
    assert!(
        errors
            .iter()
            .any(|error| error.message.contains("needs an `else`"))
    );

    let mismatched = r#"
        state "game.exe" {}
        whileAttached { let value = if true { 1 } else { "one" } }
    "#;
    let errors = splitscript::compile(mismatched).unwrap_err();
    assert!(
        errors
            .iter()
            .any(|error| error.message.contains("types do not match")
                || error.message.contains("capability"))
    );
}

#[test]
fn nested_value_blocks_typecheck_lower_and_preserve_statement_control_flow() {
    let source = r#"
        state "game.exe" {
            computed: u32 = {
                let base = 4
                base + 1
            };
        }

        fn choose(flag: bool) -> u32 {
            let value = if flag {
                let left = 10
                left + 1
            } else {
                let right = 20
                right + 2
            }
            return value
        }

        fn recover(value: u32!) -> u32 {
            return value else {
                let fallback = 7
                fallback
            }
        }

        fn describe(flag: bool) -> String {
            return match flag {
                true => {
                    let label = "enabled"
                    label
                },
                false => {
                    let label = "disabled"
                    label
                },
            }
        }

        fn early(flag: bool) -> u32 {
            let value = if flag {
                return 1
            } else {
                2
            }
            return value
        }

        fn optional(flag: bool) -> u32? {
            return if flag {
                3
            } else {
                let unavailable = None
            }
        }

        fn later() -> async u32 {
            let value = {
                await nextTick()
                choose(true)
            }
            return value
        }

        setup {
            print(choose(false))
            print({
                let status = describe(true)
                status
            })
        }
    "#;
    let wasm = splitscript::compile(source)
        .expect("value blocks should compile in every expression context");
    Validator::new_with_features(WasmFeatures::all())
        .validate_all(&wasm)
        .expect("value blocks should produce valid WebAssembly GC");

    let out_of_scope = splitscript::compile(
        r#"
            state "game.exe" {}
            setup {
                let value = { let hidden = 1; hidden }
                print(hidden)
            }
        "#,
    )
    .expect_err("value-block bindings must remain scoped to the block");
    assert!(
        out_of_scope
            .iter()
            .any(|diagnostic| diagnostic.message.contains("unknown variable `hidden`")),
        "{out_of_scope:#?}"
    );
}

#[test]
fn control_flow_keywords_are_never_typed_expressions_in_ordinary_positions() {
    let source = r#"
        state "game.exe" {}

        fn describe(stop: bool) -> String {
            print(if stop { return "stopped" } else { "continuing" })
            return "finished"
        }

        fn firstPresent(left: i32?, right: i32?) -> i32 {
            return left else right else return -1
        }

        whileAttached {
            let index = 0
            while index < 3 {
                print(if index == 1 { continue } else { index })
                let value: i32? = None
                print(value else break)
                index += 1
            }
            print(describe(false))
            print(firstPresent(None, 7))
        }
    "#;

    let wasm = splitscript::compile(source)
        .expect("control transfers should compose as ordinary Never expressions");
    Validator::new_with_features(WasmFeatures::all())
        .validate_all(&wasm)
        .expect("control-flow expressions should produce valid Wasm GC");
}

#[test]
fn value_blocks_explain_missing_values_function_returns_and_tail_semicolons() {
    let missing_value = splitscript::compile(
        r#"
            state "game.exe" {}
            fn value() -> u32 {
                return {
                    let temporary = 1
                }
            }
        "#,
    )
    .expect_err("a value-bearing block needs a tail expression");
    assert!(
        missing_value.iter().any(|diagnostic| diagnostic
            .message
            .contains("needs a final `u32` expression")),
        "{missing_value:#?}"
    );

    let implicit_return = splitscript::compile(
        r#"
            state "game.exe" {}
            fn value() -> u32 {
                42
            }
        "#,
    )
    .expect_err("function bodies require an explicit return");
    let diagnostic = implicit_return
        .iter()
        .find(|diagnostic| diagnostic.message.contains("do not implicitly return"))
        .expect("the diagnostic should distinguish function bodies from value blocks");
    assert_eq!(diagnostic.fixes.len(), 1, "{diagnostic:#?}");
    assert_eq!(diagnostic.fixes[0].edits[0].replacement, "return ");

    let checked = splitscript::check(splitscript::lower(
        splitscript::parse(
            r#"
                state "game.exe" {}
                setup {
                    let value: u32 = {
                        let base = 1
                        base + 1;
                    }
                    print(value)
                }
            "#,
        )
        .expect("a tail semicolon remains accepted syntax"),
    ))
    .expect("the tail semicolon is a warning, not an error");
    let warning = checked
        .diagnostics()
        .iter()
        .find(|diagnostic| diagnostic.code == splitscript::DiagnosticCode::ValueBlockSemicolon)
        .expect("the accepted tail semicolon should be explained");
    assert_eq!(warning.fixes.len(), 1, "{warning:#?}");
    assert_eq!(warning.fixes[0].edits[0].replacement, "");
    let formatted = splitscript::format_source(
        r#"state "game.exe" {} setup { let value: u32 = { 1; }; print(value) }"#,
    )
    .expect("formatting accepts warning-bearing source");
    assert!(formatted.contains("{\n        1\n    }"), "{formatted}");
}

#[test]
fn final_if_else_supplies_the_value_of_a_multi_statement_block() {
    let source = r#"
        state "game.exe" {
            rstate: u8 = {
                let cstate: u8 = process.read<u8>(0xf600)?
                if cstate != 5 {
                    0
                } else {
                    1
                }
            }
        }

        whileAttached {
            let local: u8 = {
                let cstate = current.rstate
                if cstate != 5 { 0 } else { 1 }
            }
            print(local)
        }
    "#;

    let wasm = splitscript::compile(source)
        .expect("a final if/else should provide the surrounding value block's value");
    Validator::new_with_features(WasmFeatures::all())
        .validate_all(&wasm)
        .expect("the normalized value-bearing if/else should produce valid Wasm GC");

    let formatted = splitscript::format_source(source)
        .expect("formatting should preserve a value-bearing final if/else");
    splitscript::compile(&formatted)
        .expect("the formatted value-bearing final if/else should still compile");
}

#[test]
fn struct_field_shorthand_compiles_formats_and_guides_repeated_initializers() {
    let source = r#"
        struct Point {
            x: u32,
            y: u32,
        }
        state "game.exe" {}
        fn point(x: u32, y: u32) -> Point {
            return Point { x, y }
        }
        setup { print(point(1, 2)) }
    "#;
    splitscript::compile(source).expect("shorthand fields should initialize like `x: x`");

    let formatted = splitscript::format_source(source).expect("shorthand fields should format");
    assert!(formatted.contains("return Point { x, y }"), "{formatted}");
    splitscript::compile(&formatted).expect("formatted shorthand should remain valid");

    let repeated = r#"
        struct Point { x: u32, y: u32 }
        state "game.exe" {}
        fn point(x: u32, y: u32) -> Point {
            return Point { x: x, y: y + 1 }
        }
    "#;
    let checked = splitscript::check(splitscript::parse(repeated).unwrap())
        .expect("repeated initializer guidance is a warning");
    let warning = checked
        .diagnostics()
        .iter()
        .find(|diagnostic| diagnostic.code == splitscript::DiagnosticCode::StructFieldShorthand)
        .expect("an exact `x: x` initializer should suggest shorthand");
    assert_eq!(warning.fixes.len(), 1, "{warning:#?}");
    assert_eq!(
        warning.fixes[0].applicability,
        splitscript::FixApplicability::MachineApplicable
    );
    assert_eq!(warning.fixes[0].edits[0].replacement, "");
    assert_eq!(
        &repeated[warning.fixes[0].edits[0].span.start..warning.fixes[0].edits[0].span.end],
        ": x"
    );
    assert_eq!(
        checked
            .diagnostics()
            .iter()
            .filter(|diagnostic| {
                diagnostic.code == splitscript::DiagnosticCode::StructFieldShorthand
            })
            .count(),
        1,
        "a nontrivial `y + 1` initializer must remain explicit"
    );
}

#[test]
fn struct_pattern_shorthand_guides_repeated_bindings() {
    let source = r#"
        struct Pos { x: u16, y: u16 }
        state "game.exe" {}
        fn use(Pos { x, y: y }) -> u16 {
            return x + y
        }
    "#;
    let checked = splitscript::check(splitscript::parse(source).unwrap())
        .expect("repeated pattern-binding guidance is a warning");
    let warning = checked
        .diagnostics()
        .iter()
        .find(|diagnostic| diagnostic.code == splitscript::DiagnosticCode::StructFieldShorthand)
        .expect("an exact `y: y` pattern should suggest shorthand");
    assert_eq!(warning.fixes.len(), 1, "{warning:#?}");
    assert_eq!(
        warning.fixes[0].applicability,
        splitscript::FixApplicability::MachineApplicable
    );
    assert_eq!(warning.fixes[0].edits[0].replacement, "");
    assert_eq!(
        &source[warning.fixes[0].edits[0].span.start..warning.fixes[0].edits[0].span.end],
        ": y"
    );
}

#[test]
fn struct_shorthand_does_not_consume_control_flow_blocks() {
    let source = r#"
        struct Point { x: u32 }
        state "game.exe" {}

        fn valid(point: Point) -> bool { return point.x > 0 }
        fn choose(enabled: bool, value: u32, values: [u32]) -> u32 {
            if enabled { print(value) }
            while enabled { return value }
            for item in values { print(item) }
            if valid(Point { x: value }) { print(value) }
            if (Point { x: value }).x > 0 { print(value) }
            return if enabled { value } else { 0 }
        }
    "#;
    splitscript::compile(source).expect(
        "a header's outer brace starts its body while nested delimiters still accept structs",
    );
}

#[test]
fn while_loops_typecheck_lower_and_validate() {
    let source = include_str!("../while_loop.split");
    let checked = splitscript::check(splitscript::parse(source).unwrap())
        .expect("while loops should typecheck");
    let while_attached = checked
        .typed_hir()
        .action_body(splitscript::compiler::ast::ActionKind::WhileAttached)
        .expect("the fixture has a whileAttached action");
    assert!(while_attached.statements.iter().any(|statement| {
        matches!(
            statement.kind,
            splitscript::compiler::hir::TypedStatementKind::Expression(_)
        )
    }));
    let function = checked
        .typed_hir()
        .function_bodies()
        .next()
        .expect("the fixture has a function");
    assert!(function.body.statements.iter().any(|statement| {
        matches!(
            statement.kind,
            splitscript::compiler::hir::TypedStatementKind::While { .. }
        )
    }));

    let lowered = splitscript::lower_wasm(&checked);
    assert!(lowered.bodies().any(|body| {
        body.entry.statements.iter().any(|statement| {
            matches!(
                statement,
                splitscript::compiler::wasm_ir::Statement::While { .. }
            )
        })
    }));

    let wasm = splitscript::codegen(&checked);
    Validator::new_with_features(WasmFeatures::all())
        .validate_all(&wasm)
        .expect("while loops should produce valid WebAssembly GC");
}

#[test]
fn while_requires_bool_conditions() {
    let errors = splitscript::compile(r#"state "game.exe" {} whileAttached { while 1 {} }"#)
        .expect_err("while conditions must be bool");
    assert!(errors.iter().any(|error| {
        error.message.contains("types do not match") || error.message.contains("bool")
    }));
}

#[test]
fn loop_expressions_infer_break_values_and_diverge_without_breaks() {
    let source = r#"
        state "game.exe" {}

        fn choose(positive: bool) -> i32 {
            return loop {
                if positive {
                    break 7
                }
                break -1
            }
        }

        fn waitForever() -> async Never {
            loop {
                await nextTick()
            }
        }

        setup {
            let none: None = loop { break }
            let fallbackNone: None = loop {
                let missing: i32? = None
                missing else break
            }
            print(choose(true))
        }
    "#;
    let checked = splitscript::check(splitscript::parse(source).unwrap())
        .expect("loop results should infer from all breaks and expected types");
    let wasm = splitscript::codegen(&checked);
    Validator::new_with_features(WasmFeatures::all())
        .validate_all(&wasm)
        .expect("sync, unit, and suspending divergent loops should produce valid Wasm GC");
}

#[test]
fn break_values_only_target_the_nearest_loop_expression() {
    let while_value =
        splitscript::compile(r#"state "game.exe" {} setup { while true { break 7 } }"#)
            .expect_err("while loops must not accept value-carrying breaks");
    assert!(while_value.iter().any(|diagnostic| {
        diagnostic
            .message
            .contains("only a `loop` expression can break with a value")
    }));

    let nested_while = splitscript::compile(
        r#"
            state "game.exe" {}
            fn value() -> i32 {
                return loop {
                    while true {
                        break 7
                    }
                }
            }
        "#,
    )
    .expect_err("a break must not skip a nested while to target an outer value loop");
    assert!(nested_while.iter().any(|diagnostic| {
        diagnostic
            .message
            .contains("only a `loop` expression can break with a value")
    }));

    let conflicting = splitscript::compile(
        r#"
            state "game.exe" {}
            fn value(flag: bool) {
                return loop {
                    if flag { break 7 }
                    break false
                }
            }
        "#,
    )
    .expect_err("all loop break values must have one inferred type");
    assert!(
        !conflicting.is_empty(),
        "conflicting break types must be rejected"
    );
}

#[test]
fn on_attach_loops_lower_suspending_back_edges_to_async_states() {
    let source = include_str!("../async_loop.split");
    let checked = splitscript::check(splitscript::parse(source).unwrap())
        .expect("await and retry should work inside while loops");
    let lowered = splitscript::lower_wasm(&checked);
    let body = lowered
        .body(splitscript::compiler::wasm_ir::BodyOwner::Action(
            splitscript::compiler::ast::ActionKind::OnAttach,
        ))
        .expect("the fixture has an onAttach body");
    assert!(body.async_state_count >= 15);
    assert!(matches!(
        body.entry.terminator,
        splitscript::compiler::wasm_ir::Terminator::AsyncWhile { .. }
    ));

    Validator::new_with_features(WasmFeatures::all())
        .validate_all(&splitscript::codegen(&checked))
        .expect("suspending loops should produce valid WebAssembly GC");
}

#[test]
fn for_loops_infer_elements_lower_and_validate() {
    let source = include_str!("../for_loop.split");
    let checked = splitscript::check(splitscript::parse(source).unwrap())
        .expect("fixed and general arrays should be iterable");
    let lowered = splitscript::lower_wasm(&checked);

    assert!(lowered.bodies().any(|body| {
        body.entry.statements.iter().any(|statement| {
            matches!(
                statement,
                splitscript::compiler::wasm_ir::Statement::For { .. }
            )
        })
    }));

    let on_attach = lowered
        .body(splitscript::compiler::wasm_ir::BodyOwner::Action(
            splitscript::compiler::ast::ActionKind::OnAttach,
        ))
        .expect("the fixture has an onAttach body");
    assert!(matches!(
        on_attach.entry.terminator,
        splitscript::compiler::wasm_ir::Terminator::AsyncFor { .. }
    ));

    Validator::new_with_features(WasmFeatures::all())
        .validate_all(&splitscript::codegen(&checked))
        .expect("sync and suspending for loops should produce valid WebAssembly GC");
}

#[test]
fn for_loops_require_iterables_and_keep_bindings_scoped_and_read_only() {
    let not_array =
        splitscript::compile(r#"state "game.exe" {} whileAttached { for value in 42 {} }"#)
            .expect_err("non-iterable values cannot be used in for loops");
    assert!(
        not_array.iter().any(|error| {
            error
                .message
                .contains("`for ... in` requires an `Iterable` or `Iterator` value")
        }),
        "{not_array:#?}"
    );

    let assignment = splitscript::compile(
        r#"state "game.exe" {} whileAttached { for value in [1] { value = 2 } }"#,
    )
    .expect_err("loop bindings are read-only");
    assert!(
        assignment
            .iter()
            .any(|error| error.message.contains("cannot assign")
                || error.message.contains("constant"))
    );

    let outside = splitscript::compile(
        r#"state "game.exe" {} whileAttached { for value in [1] {} print(value) }"#,
    )
    .expect_err("loop bindings are lexically scoped");
    assert!(
        outside
            .iter()
            .any(|error| error.message.contains("unknown variable `value`"))
    );
}

#[test]
fn for_loops_infer_array_and_element_types_from_the_body() {
    let source = r#"
        state "game.exe" {}

        fn sum(values) -> u16 {
            let total = 0u16
            for value in values {
                total += value
            }
            return total
        }

        fn emptySum() -> u16 {
            let total = 0u16
            for value in [] {
                total += value
            }
            return total
        }

        fn exactSum(values: [u16; 2]) -> u16 {
            let total = 0u16
            for value in values {
                total += value
            }
            return total
        }

        whileAttached {
            print(sum([1u16, 2u16]) as String)
            print(emptySum() as String)
            print(exactSum([1u16, 2u16]) as String)
        }
    "#;
    let checked = splitscript::check(splitscript::parse(source).unwrap())
        .expect("loop-body uses should infer both named and empty-array element types");
    Validator::new_with_features(WasmFeatures::all())
        .validate_all(&splitscript::codegen(&checked))
        .expect("backward-inferred for loops should produce valid Wasm");
}

#[test]
fn arrays_compare_structurally_when_their_elements_are_equatable() {
    let source = r#"
        state "game.exe" {}

        whileAttached {
            let fixed: [u8; 3] = [1, 2, 3]
            let growable: [u8] = [1, 2, 3]
            let nested: [[u8]] = [[1, 2], [3]]
            print(fixed == [1, 2, 3])
            print(growable != [1, 2])
            print(nested == [[1, 2], [3]])
        }
    "#;

    let checked = splitscript::check(splitscript::parse(source).unwrap())
        .expect("fixed, growable, and nested arrays should support structural equality");
    Validator::new_with_features(WasmFeatures::all())
        .validate_all(&splitscript::codegen(&checked))
        .expect("array equality helpers should produce valid Wasm");
}

#[test]
fn array_patterns_are_exact_recursive_and_bind_values() {
    let source = r#"
        state "game.exe" {}

        fn fixed(values: [u32?; 2]) -> u32 {
            return match values {
                [Some(first), Some(second)] => first + second,
                _ => 0,
            }
        }

        fn growable(values: [u8]) -> u8 {
            return match values {
                [0x53, value, 0] => value,
                _ => 0,
            }
        }

        whileAttached {
            print(fixed([Some(4), Some(5)]))
            print(growable([0x53, 7, 0]))
        }
    "#;

    let checked = splitscript::check(splitscript::parse(source).unwrap())
        .expect("array patterns should recursively test and bind their elements");
    Validator::new_with_features(WasmFeatures::all())
        .validate_all(&splitscript::codegen(&checked))
        .expect("recursive array patterns should produce valid Wasm");
}

#[test]
fn irrefutable_fixed_array_patterns_are_exhaustive() {
    let source = r#"
        state "game.exe" {}

        fn add(values: [u32; 2]) -> u32 {
            return match values {
                [first, second] => first + second,
            }
        }
    "#;

    splitscript::compile(source)
        .expect("an exact fixed array of bindings covers every value of its type");
}

#[test]
fn one_array_pattern_cannot_bind_the_same_name_twice() {
    let diagnostics = splitscript::compile(
        r#"
            state "game.exe" {}

            fn add(values: [u32; 2]) -> u32 {
                return match values {
                    [value, value] => value,
                }
            }
        "#,
    )
    .expect_err("duplicate bindings in one recursive pattern are ambiguous");

    assert!(
        diagnostics.iter().any(|diagnostic| diagnostic
            .message
            .contains("pattern binds `value` more than once")),
        "{diagnostics:#?}"
    );
}

#[test]
fn arrays_of_non_equatable_values_do_not_compare() {
    let diagnostics = splitscript::compile(
        r#"
            state "game.exe" {}

            whileAttached {
                let callbacks = [() => 1]
                print(callbacks == callbacks)
            }
        "#,
    )
    .expect_err("an array cannot derive equality when its element cannot compare");

    assert!(
        diagnostics.iter().any(|diagnostic| diagnostic
            .message
            .contains("array element does not support equality")),
        "{diagnostics:#?}"
    );
}

#[test]
fn fixed_array_patterns_require_the_declared_length() {
    let diagnostics = splitscript::compile(
        r#"
            state "game.exe" {}

            fn classify(values: [u8; 2]) -> bool {
                return match values {
                    [1] => true,
                    _ => false,
                }
            }
        "#,
    )
    .expect_err("a fixed array pattern with the wrong length should be rejected");

    assert!(
        diagnostics
            .iter()
            .any(|diagnostic| diagnostic.message.contains("requires exactly 2")),
        "{diagnostics:#?}"
    );
}

#[test]
fn growable_array_patterns_require_a_fallback_arm() {
    let diagnostics = splitscript::compile(
        r#"
            state "game.exe" {}

            fn first(values: [u8]) -> u8 {
                return match values {
                    [value] => value,
                }
            }
        "#,
    )
    .expect_err("an exact growable array pattern cannot cover every length");

    assert!(
        diagnostics
            .iter()
            .any(|diagnostic| diagnostic.message.contains("non-exhaustive array match")),
        "{diagnostics:#?}"
    );
}

#[test]
fn array_rest_patterns_are_checked_for_fixed_and_growable_arrays() {
    let source = r#"
        state "game.exe" {}

        fn fixed(values: [bool; 3]) -> bool {
            return match values {
                [true, ..] => true,
                [false, ..] => false,
            }
        }

        fn growable(values: [u8]) -> u8 {
            return match values {
                [] => 0,
                [first, ..] => first,
            }
        }

        fn suffix(values: [u8]) -> u8 {
            return match values {
                [.., last] => last,
                [] => 0,
            }
        }
    "#;

    splitscript::compile(source)
        .expect("array rest patterns should participate in symbolic exhaustiveness checking");
}

#[test]
fn array_rest_patterns_reject_too_many_fixed_elements() {
    let diagnostics = splitscript::compile(
        r#"
            state "game.exe" {}

            fn classify(values: [u8; 2]) -> bool {
                return match values {
                    [1, 2, .., 3] => true,
                    _ => false,
                }
            }
        "#,
    )
    .expect_err("a fixed array cannot satisfy a rest pattern with too many explicit elements");

    assert!(
        diagnostics
            .iter()
            .any(|diagnostic| diagnostic.message.contains("requires at least 3 elements")),
        "{diagnostics:#?}"
    );
}

#[test]
fn array_rest_patterns_reject_a_second_marker_and_format_canonically() {
    let diagnostics = splitscript::compile(
        r#"
            state "game.exe" {}
            fn classify(values: [u8]) -> bool {
                return match values {
                    [.., 1, ..] => true,
                    _ => false,
                }
            }
        "#,
    )
    .expect_err("one array pattern may contain only one rest marker");
    assert!(
        diagnostics
            .iter()
            .any(|diagnostic| diagnostic.message.contains("only one `..` rest marker")),
        "{diagnostics:#?}"
    );

    let formatted = splitscript::format_source(
        r#"state "game.exe" {} fn classify(values: [u8]) -> bool { return match values { [ 1 , .. , 2 ] => true, _ => false, } }"#,
    )
    .expect("array rest patterns should format");
    assert!(formatted.contains("[1, .., 2]"), "{formatted}");
    splitscript::compile(&formatted).expect("the formatted array rest pattern should compile");
}

#[test]
fn array_rest_patterns_preserve_nested_usefulness_correlations() {
    let diagnostics = splitscript::compile(
        r#"
            state "game.exe" {}

            fn classify(values: [bool]) -> bool {
                return match values {
                    [true, .., false] => true,
                    [true, false] => false,
                    _ => false,
                }
            }
        "#,
    )
    .expect_err("an exact pattern covered by a rest pattern should be unreachable");

    assert!(
        diagnostics
            .iter()
            .any(|diagnostic| diagnostic.message.contains("unreachable match arm")),
        "{diagnostics:#?}"
    );
}

#[test]
fn array_rest_exhaustiveness_tracks_when_prefix_and_suffix_stop_overlapping() {
    let diagnostics = splitscript::compile(
        r#"
            state "game.exe" {}

            fn classify(values: [bool]) -> bool {
                return match values {
                    [] => true,
                    [_] => true,
                    [_, _] => true,
                    [_, _, _] => true,
                    [_, _, _, _] => true,
                    [_, _, _, _, _] => true,
                    [_, _, _, _, _, _] => true,
                    [_, _, _, true, ..] => true,
                    [.., false, _, _, _] => true,
                }
            }
        "#,
    )
    .expect_err("separated prefix and suffix constraints leave longer arrays uncovered");

    assert!(
        diagnostics
            .iter()
            .any(|diagnostic| diagnostic.message.contains("non-exhaustive array match")),
        "{diagnostics:#?}"
    );
}

#[test]
fn pattern_alternatives_require_the_same_binding_names_and_types() {
    let names = splitscript::compile(
        r#"
            enum Side {
                Left(u32),
                Right(u32),
            }

            state "game.exe" {}

            fn value(side: Side) -> u32 {
                return match side {
                    Side.Left(value) | Side.Right(other) => value,
                }
            }
        "#,
    )
    .expect_err("every alternative must establish the same arm bindings");
    assert!(
        names
            .iter()
            .any(|diagnostic| diagnostic.message.contains("does not bind `value`")),
        "{names:#?}"
    );
    assert!(
        names.iter().any(|diagnostic| diagnostic
            .message
            .contains("binds `other`, but the other alternatives do not")),
        "{names:#?}"
    );

    let types = splitscript::compile(
        r#"
            enum Mixed {
                Number(u32),
                Text(String),
            }

            state "game.exe" {}

            fn value(mixed: Mixed) -> u32 {
                return match mixed {
                    Mixed.Number(value) | Mixed.Text(value) => 1,
                }
            }
        "#,
    )
    .expect_err("one logical binding must have one type");
    assert!(
        types.iter().any(|diagnostic| diagnostic
            .message
            .contains("types do not match: `u32` and `String`")),
        "{types:#?}"
    );
}

#[test]
fn wrapper_payload_patterns_are_recursive_and_preserve_exhaustiveness() {
    let source = r#"
        enum Message {
            Payload([String; 2]),
            Empty,
        }

        state "game.exe" {}

        fn classify(value: String?) -> u8 {
            return match value {
                Some("Inf") | Some("inf") => 1,
                Some(_) => 2,
                None => 3,
            }
        }

        fn inspect(message: Message) -> String {
            return match message {
                Message.Payload(["ok" | "ready", value]) => value,
                Message.Payload(_) => "other",
                Message.Empty => "empty",
            }
        }
    "#;

    let checked = splitscript::check(splitscript::parse(source).unwrap())
        .expect("wrapper and enum payloads should accept arbitrary recursive patterns");
    Validator::new_with_features(WasmFeatures::all())
        .validate_all(&splitscript::codegen(&checked))
        .expect("recursive payload patterns should produce valid Wasm");

    let diagnostics = splitscript::compile(
        r#"
            state "game.exe" {}

            fn classify(value: String?) -> bool {
                return match value {
                    Some("ready") => true,
                    None => false,
                }
            }
        "#,
    )
    .expect_err("one literal payload does not cover every present optional value");
    assert!(
        diagnostics.iter().any(|diagnostic| diagnostic
            .message
            .contains("non-exhaustive match: missing `Some(value)`")),
        "{diagnostics:#?}"
    );
}

#[test]
fn recursive_exhaustiveness_combines_arms_without_losing_field_correlations() {
    splitscript::compile(
        r#"
            enum Signal {
                Flag(bool),
                Empty,
            }

            struct Pair {
                left: bool,
                right: bool,
            }

            state "game.exe" {}

            fn optional(value: bool?) -> u8 {
                return match value {
                    Some(true) => 1,
                    Some(false) => 2,
                    None => 3,
                }
            }

            fn enumeration(value: Signal) -> u8 {
                return match value {
                    Signal.Flag(true) => 1,
                    Signal.Flag(false) => 2,
                    Signal.Empty => 3,
                }
            }

            fn structure(value: Pair) -> bool {
                return match value {
                    Pair { left: true } => true,
                    Pair { left: false } => false,
                }
            }

            fn array(value: [bool; 2]) -> u8 {
                return match value {
                    [false, false] => 0,
                    [false, true] => 1,
                    [true, false] => 2,
                    [true, true] => 3,
                }
            }
        "#,
    )
    .expect("nested finite constructors should jointly cover their complete value space");

    let correlated = splitscript::compile(
        r#"
            struct Pair { left: bool, right: bool }
            state "game.exe" {}
            fn diagonal(value: Pair) -> bool {
                return match value {
                    Pair { left: true, right: true } => true,
                    Pair { left: false, right: false } => false,
                }
            }
        "#,
    )
    .expect_err("two diagonal points do not cover a pair of booleans");
    assert!(
        correlated
            .iter()
            .any(|diagnostic| diagnostic.message.contains("non-exhaustive struct match")),
        "{correlated:#?}"
    );

    let unreachable = splitscript::compile(
        r#"
            struct Pair { left: bool, right: bool }
            state "game.exe" {}
            fn covered(value: Pair) -> bool {
                return match value {
                    Pair { left: true } => true,
                    Pair { left: false } => false,
                    _ => true,
                }
            }
        "#,
    )
    .expect_err("several earlier arms can jointly make a later arm unreachable");
    assert!(
        unreachable
            .iter()
            .any(|diagnostic| diagnostic.message.contains("unreachable match arm `_`")),
        "{unreachable:#?}"
    );
}

#[test]
fn recursive_exhaustiveness_reports_a_nested_missing_witness() {
    let diagnostics = splitscript::compile(
        r#"
            state "game.exe" {}
            fn classify(value: bool?) -> u8 {
                return match value {
                    Some(true) => 1,
                    None => 2,
                }
            }
        "#,
    )
    .expect_err("the false optional payload is not covered");
    assert!(
        diagnostics.iter().any(|diagnostic| diagnostic
            .message
            .contains("non-exhaustive match: missing `Some(false)`")),
        "{diagnostics:#?}"
    );
}

#[test]
fn struct_patterns_validate_fields_types_and_exhaustiveness() {
    let source = r#"
        struct Point {
            x: u32,
            y: u32,
        }

        state "game.exe" {}

        fn sum(point: Point) -> u32 {
            return match point {
                Point { x, y } => x + y,
            }
        }

        fn classify(point: Point) -> bool {
            return match point {
                Point { x: 0 } => true,
                _ => false,
            }
        }
    "#;
    splitscript::compile(source)
        .expect("partial struct patterns and shorthand bindings should compile");

    let unknown = splitscript::compile(
        r#"
            struct Point { x: u32 }
            state "game.exe" {}
            fn inspect(point: Point) -> bool {
                return match point { Point { y: _ } => true, _ => false }
            }
        "#,
    )
    .expect_err("unknown struct pattern fields should be rejected");
    assert!(
        unknown
            .iter()
            .any(|diagnostic| diagnostic.message.contains("has no field `y`")),
        "{unknown:#?}"
    );

    let duplicate = splitscript::compile(
        r#"
            struct Point { x: u32 }
            state "game.exe" {}
            fn inspect(point: Point) -> bool {
                return match point { Point { x: _, x: _ } => true, _ => false }
            }
        "#,
    )
    .expect_err("duplicate struct pattern fields should be rejected");
    assert!(
        duplicate.iter().any(|diagnostic| diagnostic
            .message
            .contains("duplicate struct pattern field `x`")),
        "{duplicate:#?}"
    );

    let wrong_type = splitscript::compile(
        r#"
            struct Point { x: u32 }
            state "game.exe" {}
            fn inspect(point: Point) -> bool {
                return match point { Point { x: "zero" } => true, _ => false }
            }
        "#,
    )
    .expect_err("field patterns should be checked against their field type");
    assert!(
        wrong_type.iter().any(|diagnostic| diagnostic
            .message
            .contains("types do not match: `u32` and `String`")),
        "{wrong_type:#?}"
    );

    let non_exhaustive = splitscript::compile(
        r#"
            struct Point { x: u32 }
            state "game.exe" {}
            fn inspect(point: Point) -> bool {
                return match point { Point { x: 0 } => true }
            }
        "#,
    )
    .expect_err("a constrained struct pattern does not cover every value");
    assert!(
        non_exhaustive
            .iter()
            .any(|diagnostic| diagnostic.message == "non-exhaustive struct match: add a `_` arm"),
        "{non_exhaustive:#?}"
    );
}

#[test]
fn break_and_continue_require_loops() {
    for (keyword, expected) in [
        ("break", "`break` is only available inside a loop"),
        ("continue", "`continue` is only available inside a loop"),
    ] {
        let source = format!(r#"state "game.exe" {{}} whileAttached {{ {keyword} }}"#);
        let errors = splitscript::compile(&source).expect_err("loop control needs a loop");
        assert!(errors.iter().any(|error| error.message.contains(expected)));
    }

    for (branch, expected) in [
        ("else break", "`break` is only available inside a loop"),
        (
            "else continue",
            "`continue` is only available inside a loop",
        ),
    ] {
        let source = format!(
            r#"state "game.exe" {{}} whileAttached {{ let absent: i32? = None; let value = absent {branch} }}"#
        );
        let errors = splitscript::compile(&source).expect_err("fallback loop control needs a loop");
        assert!(errors.iter().any(|error| error.message.contains(expected)));
    }
}

#[test]
fn compound_assignments_reuse_binary_typing_and_lowering() {
    use splitscript::{
        compiler::ast::{ActionKind, BinaryOp},
        compiler::stdlib::IntrinsicId,
        compiler::wasm_ir::{BodyOwner, CallTarget, Statement},
    };

    let source = r#"
        state "game.exe" {}

        let integer = 8u32
        let elapsed = 1.0

        onAttach {
            let attempts = 1u32
            let game = await process.module("game.exe")
            attempts += 1
            print(attempts as String)
        }

        whileAttached {
            integer += 2
            integer -= 1
            integer *= 3
            integer /= 2
            integer %= 7
            integer |= 0x10
            integer &= 0xff
            integer ^= 3
            integer <<= 1
            integer >>= 2

            elapsed += 0.5
            elapsed -= 0.25
            elapsed *= 2.0
            elapsed /= 2.0
        }
    "#;
    let checked = splitscript::check(splitscript::parse(source).unwrap()).unwrap();
    let lowered = splitscript::lower_wasm(&checked);
    let while_attached = lowered
        .body(BodyOwner::Action(ActionKind::WhileAttached))
        .expect("the whileAttached action should have a lowered body");
    assert_eq!(
        while_attached
            .entry
            .statements
            .iter()
            .filter_map(|statement| match statement {
                Statement::Store {
                    operation:
                        Some(splitscript::compiler::wasm_ir::AssignmentOperation::Primitive(op)),
                    ..
                } => Some(*op),
                Statement::Store {
                    operation:
                        Some(splitscript::compiler::wasm_ir::AssignmentOperation::Call(
                            CallTarget::Intrinsic { intrinsic, .. },
                        )),
                    ..
                } => match intrinsic {
                    IntrinsicId::NumericAdd => Some(BinaryOp::Add),
                    IntrinsicId::NumericSubtract => Some(BinaryOp::Sub),
                    IntrinsicId::NumericMultiply => Some(BinaryOp::Mul),
                    IntrinsicId::NumericDivide => Some(BinaryOp::Div),
                    IntrinsicId::IntegerRemainder => Some(BinaryOp::Rem),
                    IntrinsicId::IntegerBitOr => Some(BinaryOp::BitOr),
                    IntrinsicId::IntegerBitXor => Some(BinaryOp::BitXor),
                    IntrinsicId::IntegerBitAnd => Some(BinaryOp::BitAnd),
                    IntrinsicId::IntegerShiftLeft => Some(BinaryOp::Shl),
                    IntrinsicId::IntegerShiftRight => Some(BinaryOp::Shr),
                    _ => None,
                },
                _ => None,
            })
            .collect::<Vec<_>>(),
        [
            BinaryOp::Add,
            BinaryOp::Sub,
            BinaryOp::Mul,
            BinaryOp::Div,
            BinaryOp::Rem,
            BinaryOp::BitOr,
            BinaryOp::BitAnd,
            BinaryOp::BitXor,
            BinaryOp::Shl,
            BinaryOp::Shr,
            BinaryOp::Add,
            BinaryOp::Sub,
            BinaryOp::Mul,
            BinaryOp::Div,
        ]
    );
    Validator::new_with_features(WasmFeatures::all())
        .validate_all(&splitscript::codegen(&checked))
        .expect("compound assignments should lower to valid numeric Wasm operations");

    let invalid = r#"
        state "game.exe" {}
        whileAttached {
            let enabled = true
            enabled += true
        }
    "#;
    let errors = splitscript::compile(invalid)
        .expect_err("compound arithmetic must reject non-numeric operands");
    assert!(errors.iter().any(|error| {
        error.message.contains("bool") && error.message.contains("`Numeric` capability")
    }));
}

#[test]
fn numeric_min_max_and_clamp_are_type_directed_and_validate() {
    let source = r#"
        state "game.exe" {}

        whileAttached {
            let signedByte: i8 = -5
            let unsignedWord: u16 = 500
            let signedWide: i64 = -100
            let unsignedWide: u64 = 100
            let single: f32 = 1.5
            let double: f64 = -2.5

            let a = signedByte.min(2)
            let b = unsignedWord.max(1000)
            let c = signedWide.clamp(-50, 50)
            let d = unsignedWide.clamp(10, 90)
            let eMin = single.min(1.0)
            let e = eMin.max(0.0)
            let f = double.clamp(-1.0, 1.0)
            let inferredInput = 3
            let inferredFromResult: u16 = inferredInput.min(7)

            setVariable("Integers", `{a}:{b}:{c}:{d}:{inferredFromResult}`)
            if e > f as f32 { print("bounded") }
        }
    "#;
    let wasm = splitscript::compile(source).expect("numeric bounds should compile");
    Validator::new_with_features(WasmFeatures::all())
        .validate_all(&wasm)
        .expect("numeric bounds should produce valid WebAssembly");
}

#[test]
fn numeric_bounds_reject_non_numeric_receivers_and_wrong_arity() {
    for source in [
        r#"state "game.exe" {} whileAttached { let value = true; let bounded = value.min(false) }"#,
        r#"state "game.exe" {} whileAttached { let value = "a"; let bounded = value.max("b") }"#,
        r#"state "game.exe" {} whileAttached { let value: u32 = 1; value.clamp(2) }"#,
    ] {
        assert!(splitscript::compile(source).is_err());
    }
}

#[test]
fn runtime_text_outputs_accept_display_values() {
    let source = r#"
        state "game.exe" {}
        whileAttached {
            print("tick")
            print(42)
            print(-7i64)
            print(true)
            setVariable("Score", 9u16)
            setVariable("Flag", false)
            let explicit = true as String
            let interpolated = `flag={false}`
            print(explicit)
            print(interpolated)
        }
    "#;
    let wasm = splitscript::compile(source)
        .expect("print and setVariable should display supported values");
    Validator::new_with_features(WasmFeatures::all())
        .validate_all(&wasm)
        .expect("generic runtime text output should produce valid Wasm");

    for call in ["print(value)", "setVariable(\"Value\", value)"] {
        let source = format!(
            "struct Value {{ number: i32 }} state \"game.exe\" {{}} whileAttached {{ let value = Value {{ number: 1 }}; {call} }}"
        );
        let wasm = splitscript::compile(&source)
            .expect("structs should receive a derived Display implementation");
        Validator::new_with_features(WasmFeatures::all())
            .validate_all(&wasm)
            .expect("derived runtime text output should produce valid Wasm");
    }
}

#[test]
fn standard_types_can_supply_source_defined_display_implementations() {
    let source = r#"
        state "game.exe" {}
        whileAttached {
            let version = v"1.2.3.4"
            print(version)
            setVariable("Version", version)
            let cast = version as String
            let interpolated = `version {version}`
            print(cast)
            print(interpolated)
        }
    "#;
    let wasm = splitscript::compile(source)
        .expect("every Display entry point should use the catalog implementation");
    Validator::new_with_features(WasmFeatures::all())
        .validate_all(&wasm)
        .expect("source-defined Display calls should produce valid Wasm");
}

#[test]
fn strings_are_gc_values_with_content_equality_length_and_predicates() {
    let source = r#"
        state "game.exe" {}
        whileAttached {
            let message = "tick"
            if message == "tick"
                && message != "tock"
                && message.byteLength() == 4u32
                && message.contains("ic")
                && message.startsWith("ti")
                && message.endsWith("ck") {
                print(message)
            }
        }
    "#;
    let wasm = splitscript::compile(source).expect("String values should type check");
    Validator::new_with_features(WasmFeatures::all())
        .validate_all(&wasm)
        .expect("GC String operations should produce valid Wasm");
}

#[test]
fn template_strings_interpolate_strings_castable_values_and_nested_templates() {
    let source = r#"
        state "game.exe" {}

        fn format(name, value: u16, location: address) {
            let count = `{value + 1}`
            return `{name}: {count} @ {location} \{escaped\}`
        }

        onAttach {
            print(format("Score", 41, 0x1234 as address))
        }
    "#;
    let wasm = splitscript::compile(source).expect("template strings should compile");
    Validator::new_with_features(WasmFeatures::all())
        .validate_all(&wasm)
        .expect("template string lowering should produce valid Wasm");
}

#[test]
fn template_strings_use_derived_structural_display() {
    let source = r#"
        struct Value { number: i32 }
        state "game.exe" {}
        fn format(value: Value) -> String {
            return `value={value}`
        }
    "#;
    let wasm = splitscript::compile(source)
        .expect("template strings should derive Display for source structs");
    Validator::new_with_features(WasmFeatures::all())
        .validate_all(&wasm)
        .expect("derived template conversion should produce valid Wasm");
}

#[test]
fn user_functions_are_typed_and_can_call_forward_declarations() {
    let source = r#"
        state "game.exe" {}

        fn isFinalLevel(level: i32) -> bool {
            return stage(level) == 7
        }

        fn stage(level: i32) -> i32 {
            return (level / 2) + 1
        }

        whileAttached {
            let label = "level"
            if (isFinalLevel(13) && label.byteLength() == 5u32) {
                print(label)
            }
        }
    "#;
    let wasm = splitscript::compile(source).expect("user functions should compile");
    Validator::new_with_features(WasmFeatures::all())
        .validate_all(&wasm)
        .expect("user function calls should produce valid Wasm");
}

#[test]
fn user_function_and_method_calls_expose_stable_callable_ids() {
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
        }
    "#;
    let checked = splitscript::check(splitscript::parse(source).unwrap()).unwrap();
    let direct_target = checked.syntax().functions[0].id;
    let method_target = checked.syntax().functions[1].id;
    assert_ne!(direct_target, method_target);

    let statements = &checked.syntax().actions[0].body.statements;
    let splitscript::compiler::ast::Stmt::Variable(counter) = &statements[0] else {
        panic!("expected the method receiver binding");
    };
    let splitscript::compiler::ast::Stmt::Variable(direct) = &statements[1] else {
        panic!("expected the direct call binding");
    };
    let splitscript::compiler::ast::Stmt::Variable(method) = &statements[2] else {
        panic!("expected the method call binding");
    };
    assert_eq!(
        checked.semantics().call(direct.value.as_ref().unwrap().id),
        Some(&ResolvedCall::UserFunction {
            function: direct_target,
            type_arguments: Vec::new(),
            signature: vec![checked.semantics().function_result(direct_target).unwrap()],
        })
    );
    assert_eq!(
        checked.semantics().call(method.value.as_ref().unwrap().id),
        Some(&ResolvedCall::UserMethod {
            function: method_target,
            type_arguments: Vec::new(),
            signature: checked
                .semantics()
                .function_parameter_types(method_target)
                .iter()
                .copied()
                .chain(checked.semantics().function_result(method_target))
                .collect(),
            receiver: ResolvedReceiver::Path {
                root: ResolvedValue::Variable(counter.id),
                members: Vec::new(),
            },
            receiver_type: checked
                .semantics()
                .expression_type(counter.value.as_ref().unwrap().id)
                .expect("the method receiver has a semantic type"),
        })
    );

    Validator::new_with_features(WasmFeatures::all())
        .validate_all(&splitscript::codegen(&checked))
        .expect("ID-resolved user calls should produce valid Wasm");
}

#[test]
fn match_payload_bindings_and_method_receivers_resolve_by_value_id() {
    let source = r#"
        state "game.exe" {}

        struct Counter { value: i32 }
        enum MaybeCounter {
            Counter(Counter),
            Empty
        }

        fn Counter.increment() -> i32 {
            return self.value + 1
        }

        fn read(value: MaybeCounter) -> i32 {
            return match value {
                MaybeCounter.Counter(counter) => counter.increment(),
                MaybeCounter.Empty => 0
            }
        }

        whileAttached {
            let result = read(MaybeCounter.Counter(Counter { value: 4 }))
        }
    "#;
    let checked = splitscript::check(splitscript::parse(source).unwrap()).unwrap();
    let method_target = checked.syntax().functions[0].id;
    let splitscript::compiler::ast::Stmt::Expression(splitscript::compiler::ast::Expr {
        kind: splitscript::compiler::ast::ExprKind::Return(Some(matched)),
        ..
    }) = &checked.syntax().functions[1].body.statements[0]
    else {
        panic!("expected the match return expression");
    };
    let splitscript::compiler::ast::ExprKind::Match { arms, .. } = &matched.kind else {
        panic!("expected a match expression");
    };
    let splitscript::compiler::ast::MatchPattern::Enum {
        payload: Some(payload),
        ..
    } = &arms[0].pattern
    else {
        panic!("expected a payload binding");
    };
    let splitscript::compiler::ast::MatchPattern::Binding(binding) = &payload.kind else {
        panic!("expected a payload binding");
    };
    assert_eq!(
        checked.semantics().pattern_variant(arms[0].pattern_id),
        Some(ResolvedEnumVariantId::Source(
            checked.syntax().enums[0].variants[0].id,
        ))
    );
    assert_eq!(
        checked
            .typed_hir()
            .pattern(arms[0].pattern_id)
            .and_then(|pattern| pattern.variant),
        Some(ResolvedEnumVariantId::Source(
            checked.syntax().enums[0].variants[0].id,
        ))
    );
    let Some(ResolvedCall::UserMethod {
        function, receiver, ..
    }) = checked.semantics().call(arms[0].value.id)
    else {
        panic!("expected the payload method call to resolve");
    };
    assert_eq!(*function, method_target);
    assert_eq!(
        receiver,
        &ResolvedReceiver::Path {
            root: ResolvedValue::Variable(binding.id),
            members: Vec::new(),
        }
    );

    let lowered = splitscript::lower_wasm(&checked);
    let splitscript::compiler::wasm_ir::ExpressionKind::Match {
        value: lowered_value,
        arms: lowered_arms,
    } = &lowered
        .expression(matched.id)
        .expect("match expression should have a Wasm IR plan")
        .kind
    else {
        panic!("resolved match must not remain deferred to typed HIR")
    };
    let splitscript::compiler::ast::ExprKind::Match {
        value: matched_value,
        ..
    } = &matched.kind
    else {
        unreachable!()
    };
    assert_eq!(*lowered_value, matched_value.id);
    assert_eq!(lowered_arms[0].pattern_id, arms[0].pattern_id);
    let splitscript::compiler::wasm_ir::LoweredPattern::Enum {
        enumeration,
        variant,
        payload: Some(ref lowered_payload),
    } = lowered_arms[0].pattern
    else {
        panic!("enum patterns should retain their resolved identities")
    };
    assert_eq!(
        enumeration,
        splitscript::compiler::types::EnumTypeId::Source(checked.syntax().enums[0].id)
    );
    assert_eq!(
        variant,
        ResolvedEnumVariantId::Source(checked.syntax().enums[0].variants[0].id)
    );
    assert!(matches!(
        lowered_payload.as_ref(),
        splitscript::compiler::wasm_ir::LoweredPattern::Binding(id) if *id == binding.id
    ));

    let splitscript::compiler::ast::Stmt::Variable(result) =
        &checked.syntax().actions[0].body.statements[0]
    else {
        panic!("expected the result binding");
    };
    let splitscript::compiler::ast::ExprKind::Call { args, .. } =
        &result.value.as_ref().unwrap().kind
    else {
        panic!("expected the read call");
    };
    assert_eq!(
        checked.semantics().enum_variant(args[0].id),
        Some(ResolvedEnumVariantId::Source(
            checked.syntax().enums[0].variants[0].id,
        ))
    );
    assert_eq!(
        checked.typed_hir().enum_variant(args[0].id),
        Some(ResolvedEnumVariantId::Source(
            checked.syntax().enums[0].variants[0].id,
        ))
    );

    Validator::new_with_features(WasmFeatures::all())
        .validate_all(&splitscript::codegen(&checked))
        .expect("ID-resolved pattern receivers should produce valid Wasm");
}

#[test]
fn member_paths_resolve_struct_and_standard_fields_to_stable_ids() {
    let source = r#"
        state "game.exe" {}

        struct Inner { value: i32 }
        struct Outer { inner: Inner }

        fn Inner.increment() -> i32 {
            return self.value + 1
        }

        onAttach {
            let module = await process.module("GameAssembly.dll")
            let outer = Outer { inner: Inner { value: 4 } }
            let nested = outer.inner.value
            let method = outer.inner.increment()
            let address = module.address
        }
    "#;
    let checked = splitscript::check(splitscript::parse(source).unwrap()).unwrap();
    let inner_value = checked.syntax().structs[0].fields[0].id;
    let outer_inner = checked.syntax().structs[1].fields[0].id;
    assert_ne!(inner_value, outer_inner);

    let statements = &checked.syntax().actions[0].body.statements;
    let splitscript::compiler::ast::Stmt::Variable(outer) = &statements[1] else {
        panic!("expected the outer binding");
    };
    assert_eq!(
        checked
            .semantics()
            .struct_literal_fields(outer.value.as_ref().unwrap().id),
        Some([ResolvedStructFieldId::Source(outer_inner)].as_slice())
    );
    assert_eq!(
        checked
            .typed_hir()
            .struct_literal_fields(outer.value.as_ref().unwrap().id),
        Some([ResolvedStructFieldId::Source(outer_inner)].as_slice())
    );
    let splitscript::compiler::ast::Stmt::Variable(nested) = &statements[2] else {
        panic!("expected the nested field binding");
    };
    assert_eq!(
        checked
            .semantics()
            .path_members(nested.value.as_ref().unwrap().id),
        Some(
            [
                ResolvedMember::StructField(outer_inner),
                ResolvedMember::StructField(inner_value),
            ]
            .as_slice()
        )
    );
    let (nested_root, nested_members) = checked
        .typed_hir()
        .value_path(nested.value.as_ref().unwrap().id)
        .expect("typed HIR should materialize resolved paths");
    assert_eq!(nested_root, Some(ResolvedValue::Variable(outer.id)));
    assert_eq!(
        nested_members,
        [
            ResolvedMember::StructField(outer_inner),
            ResolvedMember::StructField(inner_value),
        ]
    );

    let splitscript::compiler::ast::Stmt::Variable(method) = &statements[3] else {
        panic!("expected the nested receiver binding");
    };
    let Some(ResolvedCall::UserMethod { receiver, .. }) =
        checked.semantics().call(method.value.as_ref().unwrap().id)
    else {
        panic!("expected a resolved nested method receiver");
    };
    assert_eq!(
        receiver,
        &ResolvedReceiver::Path {
            root: ResolvedValue::Variable(outer.id),
            members: vec![ResolvedMember::StructField(outer_inner)],
        }
    );

    let splitscript::compiler::ast::Stmt::Variable(address) = &statements[4] else {
        panic!("expected the built-in field binding");
    };
    assert_eq!(
        checked
            .semantics()
            .path_members(address.value.as_ref().unwrap().id),
        Some([ResolvedMember::StandardField(StdlibFieldId::ModuleAddress,)].as_slice())
    );

    Validator::new_with_features(WasmFeatures::all())
        .validate_all(&splitscript::codegen(&checked))
        .expect("ID-resolved member chains should produce valid Wasm");
}

#[test]
fn value_paths_resolve_globals_parameters_and_locals_to_declaration_ids() {
    let source = r#"
        let seed = 7
        state "game.exe" {}

        fn identity(value: i32) -> i32 {
            return value
        }

        whileAttached {
            let copy: i32 = seed
            let result = identity(copy)
        }
    "#;
    let checked = splitscript::check(splitscript::parse(source).unwrap()).unwrap();
    let global = checked.syntax().globals[0].id;
    let parameter = checked.syntax().functions[0].params[0].id;
    let splitscript::compiler::ast::Stmt::Expression(splitscript::compiler::ast::Expr {
        kind: splitscript::compiler::ast::ExprKind::Return(Some(parameter_path)),
        ..
    }) = &checked.syntax().functions[0].body.statements[0]
    else {
        panic!("expected the parameter return");
    };
    assert_eq!(
        checked.semantics().value(parameter_path.id),
        Some(ResolvedValue::Variable(parameter))
    );

    let statements = &checked.syntax().actions[0].body.statements;
    let splitscript::compiler::ast::Stmt::Variable(copy) = &statements[0] else {
        panic!("expected the local copy");
    };
    assert_eq!(
        checked.semantics().value(copy.value.as_ref().unwrap().id),
        Some(ResolvedValue::Variable(global))
    );
    let splitscript::compiler::ast::Stmt::Variable(result) = &statements[1] else {
        panic!("expected the result binding");
    };
    let splitscript::compiler::ast::ExprKind::Call { args, .. } =
        &result.value.as_ref().unwrap().kind
    else {
        panic!("expected the identity call");
    };
    assert_eq!(
        checked.semantics().value(args[0].id),
        Some(ResolvedValue::Variable(copy.id))
    );

    Validator::new_with_features(WasmFeatures::all())
        .validate_all(&splitscript::codegen(&checked))
        .expect("ID-resolved value reads should produce valid Wasm");
}

#[test]
fn snapshot_paths_resolve_state_and_setting_ids_with_temporal_identity() {
    let source = r#"
        state "game.exe" {
            score: i32 at 0x1000
        }

        settings {
            "Enabled" => enabled: true
        }

        whileAttached {
            let currentScore = current.score
            let oldScore = old.score
            let enabled = settings.enabled
            let wasEnabled = oldSettings.enabled
        }
    "#;
    let checked = splitscript::check(splitscript::parse(source).unwrap()).unwrap();
    let state = checked.syntax().state.as_ref().unwrap().fields[0].id;
    let setting = checked.syntax().settings[0].id;
    let statements = &checked.syntax().actions[0].body.statements;
    let expected = [
        ResolvedValue::CurrentState(state),
        ResolvedValue::OldState(state),
        ResolvedValue::Setting(setting),
        ResolvedValue::OldSetting(setting),
    ];
    for (statement, expected) in statements.iter().zip(expected) {
        let splitscript::compiler::ast::Stmt::Variable(variable) = statement else {
            panic!("expected a snapshot binding");
        };
        assert_eq!(
            checked
                .semantics()
                .value(variable.value.as_ref().unwrap().id),
            Some(expected)
        );
    }

    Validator::new_with_features(WasmFeatures::all())
        .validate_all(&splitscript::codegen(&checked))
        .expect("ID-resolved snapshot reads should produce valid Wasm");
}

#[test]
fn assignments_resolve_local_and_global_targets_by_id() {
    let source = r#"
        let counter = 0
        state "game.exe" {}

        whileAttached {
            let local: i32 = 1
            local = 2
            counter = local
        }
    "#;
    let checked = splitscript::check(splitscript::parse(source).unwrap()).unwrap();
    let global = checked.syntax().globals[0].id;
    let statements = &checked.syntax().actions[0].body.statements;
    let splitscript::compiler::ast::Stmt::Variable(local) = &statements[0] else {
        panic!("expected the local declaration");
    };
    let splitscript::compiler::ast::Stmt::Assign {
        id: local_write, ..
    } = &statements[1]
    else {
        panic!("expected the local assignment");
    };
    let splitscript::compiler::ast::Stmt::Assign {
        id: global_write, ..
    } = &statements[2]
    else {
        panic!("expected the global assignment");
    };
    assert_eq!(
        checked.semantics().assignment_target(*local_write),
        Some(local.id)
    );
    assert_eq!(
        checked.semantics().assignment_target(*global_write),
        Some(global)
    );
    assert_eq!(
        checked
            .typed_hir()
            .assignment(*local_write)
            .map(|assignment| assignment.target),
        Some(local.id)
    );
    assert_eq!(
        checked
            .typed_hir()
            .assignment(*global_write)
            .map(|assignment| assignment.target),
        Some(global)
    );

    Validator::new_with_features(WasmFeatures::all())
        .validate_all(&splitscript::codegen(&checked))
        .expect("ID-resolved assignments should produce valid Wasm");
}
