//! diagnostics migration integration tests.

#[test]
fn javascript_template_interpolation_warns_with_both_intent_preserving_fixes() {
    use splitscript::{DiagnosticCode, DiagnosticSeverity, FixApplicability};

    let source = r#"
        state "game.exe" {}
        whileAttached {
            let score = 1200
            print(`Score: ${score}`)
        }
    "#;
    let recovered = splitscript::parse_recovering(source)
        .expect("JavaScript-style interpolation should retain a syntax tree");
    let diagnostic = recovered
        .diagnostics()
        .iter()
        .find(|diagnostic| diagnostic.code == DiagnosticCode::SuspiciousInterpolation)
        .expect("the unescaped dollar marker should warn");
    assert_eq!(diagnostic.severity, DiagnosticSeverity::Warning);
    assert_eq!(&source[diagnostic.span.start..diagnostic.span.end], "$");
    assert_eq!(diagnostic.fixes.len(), 2);
    assert!(
        diagnostic
            .fixes
            .iter()
            .all(|fix| fix.applicability == FixApplicability::MachineApplicable)
    );
    assert_eq!(diagnostic.fixes[0].edits[0].replacement, "");
    assert_eq!(diagnostic.fixes[1].edits[0].replacement, "\\$");

    for fix in diagnostic.fixes.iter() {
        let edit = &fix.edits[0];
        let mut fixed = source.to_owned();
        fixed.replace_range(edit.span.start..edit.span.end, &edit.replacement);
        let reparsed = splitscript::parse_recovering(&fixed).unwrap();
        assert!(
            reparsed
                .diagnostics()
                .iter()
                .all(|diagnostic| diagnostic.code != DiagnosticCode::SuspiciousInterpolation),
            "fix `{}` should make the intent explicit",
            fix.title
        );
    }

    for canonical in [r#"`{score}`"#, r#"`\${score}`"#, r#"`cost $5`"#] {
        let fixture = format!(
            "state \"game.exe\" {{}}\nwhileAttached {{ let score = 5 print({canonical}) }}"
        );
        let parsed = splitscript::parse_recovering(&fixture).unwrap();
        assert!(
            parsed
                .diagnostics()
                .iter()
                .all(|diagnostic| diagnostic.code != DiagnosticCode::SuspiciousInterpolation),
            "`{canonical}` should not warn"
        );
    }
}

#[test]
fn javascript_strict_equality_recovers_with_machine_applicable_fixes() {
    use splitscript::FixApplicability;

    for (operator, replacement) in [("===", "=="), ("!==", "!=")] {
        let source = format!(
            r#"
                state "game.exe" {{}}

                fn compare(left: i32, right: i32) -> bool {{
                    return left {operator} right
                }}
            "#
        );
        let recovered = splitscript::parse_recovering(&source)
            .expect("strict equality should retain a recoverable syntax tree");
        assert_eq!(recovered.diagnostics().len(), 1);
        let diagnostic = &recovered.diagnostics()[0];
        assert!(diagnostic.message.contains(replacement));
        assert_eq!(
            &source[diagnostic.span.start..diagnostic.span.end],
            operator
        );
        assert_eq!(diagnostic.fixes.len(), 1);
        let fix = &diagnostic.fixes[0];
        assert_eq!(fix.applicability, FixApplicability::MachineApplicable);
        assert_eq!(fix.edits.len(), 1);
        assert_eq!(fix.edits[0].replacement, replacement);

        let mut fixed = source.clone();
        let edit = &fix.edits[0];
        fixed.replace_range(edit.span.start..edit.span.end, &edit.replacement);
        splitscript::compile(&fixed).expect("the canonical equality replacement should compile");
    }
}

#[test]
fn familiar_bitwise_complement_recovers_as_overloaded_not() {
    use splitscript::FixApplicability;

    let source = r#"
        state "game.exe" {}

        fn complement(value: u8) -> u8 {
            return ~value
        }
    "#;
    let recovered = splitscript::parse_recovering(source)
        .expect("bitwise complement should retain a recoverable syntax tree");
    assert_eq!(recovered.diagnostics().len(), 1);
    let diagnostic = &recovered.diagnostics()[0];
    assert_eq!(
        diagnostic.message,
        "SplitScript overloads `!` for integer bitwise complement instead of using `~`"
    );
    assert_eq!(&source[diagnostic.span.start..diagnostic.span.end], "~");
    let [fix] = diagnostic.fixes.as_slice() else {
        panic!("the migration should provide one fix");
    };
    assert_eq!(fix.applicability, FixApplicability::MachineApplicable);
    let [edit] = fix.edits.as_slice() else {
        panic!("the migration fix should contain one edit");
    };
    assert_eq!(edit.replacement, "!");

    let fixed = source.replacen('~', "!", 1);
    splitscript::compile(&fixed).expect("the canonical integer complement should compile");
}

#[test]
fn rust_let_mut_recovers_by_removing_the_redundant_modifier() {
    use splitscript::FixApplicability;

    let source = r#"
        state "game.exe" {}
        let mut global = 1

        whileAttached {
            let mut local = global
            local += 1
            global = local
        }
    "#;
    let recovered = splitscript::parse_recovering(source)
        .expect("Rust let mut should retain recoverable variable declarations");
    assert_eq!(recovered.diagnostics().len(), 2);

    let mut fixed = source.to_owned();
    for diagnostic in recovered.diagnostics().iter().rev() {
        assert_eq!(
            diagnostic.message,
            "SplitScript `let` bindings are already mutable"
        );
        assert_eq!(&source[diagnostic.span.start..diagnostic.span.end], "mut");
        assert_eq!(diagnostic.fixes.len(), 1);
        let fix = &diagnostic.fixes[0];
        assert_eq!(fix.applicability, FixApplicability::MachineApplicable);
        assert_eq!(fix.edits.len(), 1);
        assert!(fix.edits[0].replacement.is_empty());
        fixed.replace_range(fix.edits[0].span.start..fix.edits[0].span.end, "");
    }
    splitscript::compile(&fixed).expect("removing both Rust mut modifiers should compile");
}

#[test]
fn csharp_string_equals_explains_exact_and_ascii_insensitive_equality() {
    let source = r#"
        state "game.exe" {}

        fn sameMap(left: String, right: String) -> bool {
            return left.Equals(right)
        }
    "#;
    let diagnostics = splitscript::compile(source)
        .expect_err("C# String.Equals should receive semantic migration guidance");

    assert_eq!(diagnostics.len(), 1, "unexpected cascade: {diagnostics:#?}");
    let diagnostic = &diagnostics[0];
    assert_eq!(
        diagnostic.message,
        "C# `String.Equals` becomes an equality expression in SplitScript"
    );
    assert_eq!(
        &source[diagnostic.span.start..diagnostic.span.end],
        "Equals"
    );
    assert!(diagnostic.fixes.is_empty());
    assert!(
        diagnostic
            .notes
            .iter()
            .any(|note| note.contains("left == right") && note.contains("exact UTF-8 text"))
    );
    assert!(diagnostic.notes.iter().any(|note| {
        note.contains("equalsIgnoreAsciiCase") && note.contains("ASCII letter case")
    }));
}

#[test]
fn csharp_substring_explains_length_and_utf8_boundary_differences() {
    let source = r#"
        state "game.exe" {}

        fn mapCode(value: String, start: u32, length: u32) -> String! {
            return value.Substring(start, length)
        }
    "#;
    let diagnostics = splitscript::compile(source)
        .expect_err("C# Substring should require a boundary-aware migration");

    assert_eq!(diagnostics.len(), 1, "unexpected cascade: {diagnostics:#?}");
    let diagnostic = &diagnostics[0];
    assert_eq!(
        diagnostic.message,
        "C# `String.Substring` needs an explicit UTF-8 boundary review"
    );
    assert_eq!(
        &source[diagnostic.span.start..diagnostic.span.end],
        "Substring"
    );
    assert!(diagnostic.fixes.is_empty());
    assert!(diagnostic.notes.iter().any(|note| {
        note.contains("slice(start, start + length)") && note.contains("proven ASCII")
    }));
    assert!(
        diagnostic
            .notes
            .iter()
            .any(|note| { note.contains("UTF-16 code units") && note.contains("UTF-8 bytes") })
    );
}

#[test]
fn csharp_string_index_of_explains_option_and_utf8_offsets() {
    let source = r#"
        state "game.exe" {}

        whileAttached {
            let index = "Straße_Complete".IndexOf("_")
            print(index)
        }
    "#;
    let diagnostics =
        splitscript::compile(source).expect_err("C# IndexOf needs index-unit and absence review");
    assert_eq!(diagnostics.len(), 1);
    let diagnostic = &diagnostics[0];
    assert_eq!(
        diagnostic.message,
        "C# `String.IndexOf` needs an explicit index-model review"
    );
    assert_eq!(
        &source[diagnostic.span.start..diagnostic.span.end],
        "IndexOf"
    );
    assert!(diagnostic.fixes.is_empty());
    assert!(diagnostic.notes.iter().any(|note| note.contains("None")));
    assert!(diagnostic.notes.iter().any(|note| note.contains("UTF-16")));
    assert!(diagnostic.notes.iter().any(|note| note.contains("UTF-8")));
}

#[test]
fn csharp_string_last_index_of_explains_option_and_utf8_offsets() {
    let source = r#"
        state "game.exe" {}

        whileAttached {
            let index = "folder/route/level".LastIndexOf("/")
            print(index)
        }
    "#;
    let diagnostics = splitscript::compile(source)
        .expect_err("C# LastIndexOf needs index-unit and absence review");
    assert_eq!(diagnostics.len(), 1);
    let diagnostic = &diagnostics[0];
    assert_eq!(
        diagnostic.message,
        "C# `String.LastIndexOf` needs an explicit index-model review"
    );
    assert_eq!(
        &source[diagnostic.span.start..diagnostic.span.end],
        "LastIndexOf"
    );
    assert!(diagnostic.fixes.is_empty());
    assert!(diagnostic.notes.iter().any(|note| note.contains("None")));
    assert!(diagnostic.notes.iter().any(|note| note.contains("UTF-16")));
    assert!(diagnostic.notes.iter().any(|note| note.contains("UTF-8")));
}

#[test]
fn csharp_string_replace_explains_fallible_immutable_replacement() {
    let source = r#"
        state "game.exe" {}

        fn normalizeMap(value: String) -> String! {
            return value.Replace("_", " ")
        }
    "#;
    let diagnostics = splitscript::compile(source)
        .expect_err("C# Replace needs Result-aware replacement guidance");

    assert_eq!(diagnostics.len(), 1, "unexpected cascade: {diagnostics:#?}");
    let diagnostic = &diagnostics[0];
    assert_eq!(
        diagnostic.message,
        "C# `String.Replace` becomes fallible `replaceAll` in SplitScript"
    );
    assert_eq!(
        &source[diagnostic.span.start..diagnostic.span.end],
        "Replace"
    );
    assert!(diagnostic.fixes.is_empty());
    assert!(diagnostic.notes.iter().any(|note| {
        note.contains("replaceAll(search, replacement)") && note.contains("non-null")
    }));
    assert!(
        diagnostic
            .notes
            .iter()
            .any(|note| note.contains("String!") && note.contains("Result"))
    );
    assert!(
        diagnostic
            .notes
            .iter()
            .any(|note| note.contains("empty string") && note.contains("deletion"))
    );
}

#[test]
fn csharp_string_padding_explains_direction_fill_and_width_model() {
    let source = r#"
        state "game.exe" {}

        whileAttached {
            let left = "7".PadLeft(3, '0')
            let right = "Split".PadRight(9)
            print(`{left}{right}`)
        }
    "#;
    let diagnostics =
        splitscript::compile(source).expect_err("C# padding needs width-model guidance");
    assert_eq!(diagnostics.len(), 2);
    for diagnostic in &diagnostics {
        assert_eq!(
            diagnostic.message,
            "C# string padding needs an explicit width-model review"
        );
        assert!(diagnostic.fixes.is_empty());
        assert!(diagnostic.notes.iter().any(|note| note.contains("UTF-16")));
        assert!(
            diagnostic
                .notes
                .iter()
                .any(|note| note.contains("Unicode scalar"))
        );
        assert!(diagnostic.notes.iter().any(|note| note.contains("' '")));
    }
    assert_eq!(
        &source[diagnostics[0].span.start..diagnostics[0].span.end],
        "PadLeft"
    );
    assert_eq!(
        &source[diagnostics[1].span.start..diagnostics[1].span.end],
        "PadRight"
    );
}

#[test]
fn csharp_string_trim_explains_ascii_whitespace_boundaries() {
    let source = r#"
        state "game.exe" {}

        fn cleanLine(value: String) -> String {
            return value.Trim()
        }
    "#;
    let diagnostics = splitscript::compile(source)
        .expect_err("C# Trim needs an explicit whitespace-model review");

    assert_eq!(diagnostics.len(), 1, "unexpected cascade: {diagnostics:#?}");
    let diagnostic = &diagnostics[0];
    assert_eq!(
        diagnostic.message,
        "C# `String.Trim` needs an explicit whitespace-model review"
    );
    assert_eq!(&source[diagnostic.span.start..diagnostic.span.end], "Trim");
    assert!(diagnostic.fixes.is_empty());
    assert!(
        diagnostic.notes.iter().any(|note| {
            note.contains("trimAsciiWhitespace") && note.contains("ASCII whitespace")
        })
    );
    assert!(
        diagnostic
            .notes
            .iter()
            .any(|note| note.contains("Unicode whitespace"))
    );
    assert!(diagnostic.notes.iter().any(|note| {
        note.contains("TrimStart") && note.contains("TrimEnd") && note.contains("overloads")
    }));
}

#[test]
fn csharp_is_null_or_empty_explains_required_and_optional_strings() {
    let source = r#"
        state "game.exe" {}

        fn missingCheckpoint(value: String?) -> bool {
            return String.IsNullOrEmpty(value)
        }
    "#;
    let diagnostics = splitscript::compile(source)
        .expect_err("C# IsNullOrEmpty needs Option-aware migration guidance");

    assert_eq!(diagnostics.len(), 1, "unexpected cascade: {diagnostics:#?}");
    let diagnostic = &diagnostics[0];
    assert_eq!(
        diagnostic.message,
        "C# `String.IsNullOrEmpty` crosses SplitScript's Option boundary"
    );
    assert_eq!(
        &source[diagnostic.span.start..diagnostic.span.end],
        "IsNullOrEmpty"
    );
    assert!(diagnostic.fixes.is_empty());
    assert!(
        diagnostic
            .notes
            .iter()
            .any(|note| { note.contains("required `String`") && note.contains("value.isEmpty()") })
    );
    assert!(diagnostic.notes.iter().any(|note| {
        note.contains("String?") && note.contains("None") && note.contains("Some(text)")
    }));
    assert!(
        diagnostic
            .notes
            .iter()
            .any(|note| note.contains("Result or Option policy"))
    );
}

#[test]
fn csharp_is_null_or_white_space_explains_unicode_and_option_semantics() {
    let source = r#"
        state "game.exe" {}

        fn missingLabel(value: String?) -> bool {
            return String.IsNullOrWhiteSpace(value)
        }
    "#;
    let diagnostics = splitscript::compile(source)
        .expect_err("C# IsNullOrWhiteSpace needs Unicode- and Option-aware guidance");

    assert_eq!(diagnostics.len(), 1, "unexpected cascade: {diagnostics:#?}");
    let diagnostic = &diagnostics[0];
    assert_eq!(
        diagnostic.message,
        "C# `String.IsNullOrWhiteSpace` crosses SplitScript's Option boundary"
    );
    assert_eq!(
        &source[diagnostic.span.start..diagnostic.span.end],
        "IsNullOrWhiteSpace"
    );
    assert!(diagnostic.fixes.is_empty());
    assert!(
        diagnostic
            .notes
            .iter()
            .any(|note| { note.contains("required `String`") && note.contains("value.isBlank()") })
    );
    assert!(diagnostic.notes.iter().any(|note| {
        note.contains("String?") && note.contains("None") && note.contains("Some(text)")
    }));
    assert!(diagnostic.notes.iter().any(|note| {
        note.contains("Unicode `White_Space`") && note.contains("non-breaking space")
    }));
}

#[test]
fn csharp_length_distinguishes_arrays_from_utf8_strings() {
    use splitscript::FixApplicability;

    let array_source = r#"
        state "game.exe" {}

        fn count() -> u32 {
            let values = [2, 4, 7]
            return values.Length
        }
    "#;
    let diagnostics = splitscript::compile(array_source)
        .expect_err("C# array Length should receive a direct migration fix");

    assert_eq!(diagnostics.len(), 1, "unexpected cascade: {diagnostics:#?}");
    let diagnostic = &diagnostics[0];
    assert_eq!(
        diagnostic.message,
        "C# array `Length` is `length()` in SplitScript"
    );
    assert_eq!(
        &array_source[diagnostic.span.start..diagnostic.span.end],
        "Length"
    );
    assert!(
        diagnostic
            .notes
            .iter()
            .any(|note| note.contains("u32") && note.contains("[T; N]"))
    );
    assert_eq!(diagnostic.fixes.len(), 1);
    let fix = &diagnostic.fixes[0];
    assert_eq!(fix.applicability, FixApplicability::MachineApplicable);
    assert_eq!(fix.edits.len(), 1);
    assert_eq!(fix.edits[0].span, diagnostic.span);
    assert_eq!(fix.edits[0].replacement, "length()");

    let mut fixed = array_source.to_owned();
    fixed.replace_range(
        fix.edits[0].span.start..fix.edits[0].span.end,
        &fix.edits[0].replacement,
    );
    splitscript::compile(&fixed).expect("the array length fix should compile");

    let string_source = r#"
        state "game.exe" {}

        fn encodedLength(value: String) -> u32 {
            return value.Length
        }
    "#;
    let diagnostics = splitscript::compile(string_source)
        .expect_err("C# string Length should require an index-unit decision");

    assert_eq!(diagnostics.len(), 1, "unexpected cascade: {diagnostics:#?}");
    let diagnostic = &diagnostics[0];
    assert_eq!(
        diagnostic.message,
        "C# string `Length` has no encoding-neutral SplitScript rename"
    );
    assert_eq!(
        &string_source[diagnostic.span.start..diagnostic.span.end],
        "Length"
    );
    assert!(diagnostic.fixes.is_empty());
    assert!(
        diagnostic
            .notes
            .iter()
            .any(|note| { note.contains("value.isEmpty()") && note.contains("zero-length") })
    );
    assert!(
        diagnostic
            .notes
            .iter()
            .any(|note| { note.contains("UTF-16 code units") && note.contains("UTF-8 bytes") })
    );

    splitscript::compile(
        r#"
            state "game.exe" {}
            fn arrayLength(values: [u8]) -> u32 { return values.length() }
            fn stringLength(value: String) -> u32 { return value.byteLength() }
            fn empty(value: String) -> bool { return value.isEmpty() }
        "#,
    )
    .expect("canonical length operations should compile");
}

#[test]
fn csharp_collection_count_rewrites_only_resolved_arrays_and_sets() {
    use splitscript::FixApplicability;

    for (name, parameter) in [("array", "[u8]"), ("set", "Set<String>")] {
        let source = format!(
            r#"
                state "game.exe" {{}}

                fn count(values: {parameter}) -> u32 {{
                    return values.Count
                }}
            "#
        );
        let diagnostics = splitscript::compile(&source)
            .expect_err("C# collection Count should receive a direct migration fix");

        assert_eq!(
            diagnostics.len(),
            1,
            "unexpected {name} cascade: {diagnostics:#?}"
        );
        let diagnostic = &diagnostics[0];
        assert_eq!(
            diagnostic.message,
            "C# collection `Count` is `length()` in SplitScript"
        );
        assert_eq!(&source[diagnostic.span.start..diagnostic.span.end], "Count");
        assert!(diagnostic.notes.iter().any(|note| {
            note.contains("arrays") && note.contains("Set<T>") && note.contains("u32")
        }));
        assert!(
            diagnostic
                .notes
                .iter()
                .any(|note| { note.contains("fixed order") && note.contains("unique membership") })
        );
        assert_eq!(diagnostic.fixes.len(), 1);
        let fix = &diagnostic.fixes[0];
        assert_eq!(fix.applicability, FixApplicability::MachineApplicable);
        assert_eq!(fix.edits.len(), 1);
        assert_eq!(fix.edits[0].span, diagnostic.span);
        assert_eq!(fix.edits[0].replacement, "length()");

        let mut fixed = source;
        fixed.replace_range(
            fix.edits[0].span.start..fix.edits[0].span.end,
            &fix.edits[0].replacement,
        );
        splitscript::compile(&fixed).expect("the collection count fix should compile");
    }

    splitscript::compile(
        r#"
            struct Counter {
                Count: u32,
            }
            state "game.exe" {}
            fn count(value: Counter) -> u32 { return value.Count }
        "#,
    )
    .expect("a user struct field named Count must keep its ordinary meaning");
}

#[test]
fn csharp_string_join_explains_array_and_argument_order() {
    let source = r#"
        state "game.exe" {}

        fn route(values: [String]) -> String {
            return String.Join(".", values)
        }
    "#;
    let diagnostics = splitscript::compile(source)
        .expect_err("C# String.Join needs typed-array migration guidance");

    assert_eq!(diagnostics.len(), 1, "unexpected cascade: {diagnostics:#?}");
    let diagnostic = &diagnostics[0];
    assert_eq!(
        diagnostic.message,
        "C# `String.Join` needs an explicit collection conversion"
    );
    assert_eq!(&source[diagnostic.span.start..diagnostic.span.end], "Join");
    assert!(diagnostic.fixes.is_empty());
    assert!(diagnostic.notes.iter().any(|note| {
        note.contains("String.join(values, separator)") && note.contains("[String]")
    }));
    assert!(
        diagnostic
            .notes
            .iter()
            .any(|note| note.contains("argument order changes"))
    );
    assert!(diagnostic.notes.iter().any(|note| {
        note.contains("variadic") && note.contains("enumerables") && note.contains("range")
    }));
}

#[test]
fn csharp_static_numeric_parse_explains_result_based_string_parsing() {
    for call in ["Int32.Parse(text)", "Double.Parse(text)"] {
        let source = format!(
            r#"
                state "game.exe" {{}}

                fn parseLegacy(text: String) {{
                    {call}
                }}
            "#
        );
        let diagnostics = splitscript::compile(&source)
            .expect_err("C# static parsing should receive Result-based migration guidance");

        assert_eq!(diagnostics.len(), 1, "unexpected cascade: {diagnostics:#?}");
        let diagnostic = &diagnostics[0];
        assert_eq!(
            diagnostic.message,
            "C# static numeric parsing becomes `String.parse<T>()` in SplitScript"
        );
        assert_eq!(&source[diagnostic.span.start..diagnostic.span.end], "Parse");
        assert!(diagnostic.fixes.is_empty());
        assert!(diagnostic.notes.iter().any(|note| {
            note.contains("let value: i32 = text.parse()?") && note.contains("fallback")
        }));
        assert!(
            diagnostic
                .notes
                .iter()
                .any(|note| { note.contains("TryParse") && note.contains("Result control flow") })
        );
    }
}

#[test]
fn csharp_convert_calls_explain_source_specific_split_script_forms() {
    let integer_methods = [
        "ToSByte", "ToByte", "ToInt16", "ToUInt16", "ToInt32", "ToUInt32", "ToInt64", "ToUInt64",
    ];
    for owner in ["Convert", "System.Convert"] {
        for method in integer_methods {
            let source = format!(
                r#"
                    state "game.exe" {{}}
                    fn convert(value: f64) {{ return {owner}.{method}(value) }}
                "#
            );
            let diagnostics = splitscript::compile(&source)
                .expect_err("C# integer conversion should need source-specific guidance");
            assert_eq!(diagnostics.len(), 1, "unexpected cascade: {diagnostics:#?}");
            let diagnostic = &diagnostics[0];
            assert_eq!(
                diagnostic.message,
                "C# integer conversion needs a source-specific SplitScript rewrite"
            );
            assert_eq!(&source[diagnostic.span.start..diagnostic.span.end], method);
            assert!(diagnostic.fixes.is_empty());
            assert!(diagnostic.notes.iter().any(|note| {
                note.contains("value.round() as i32")
                    && note.contains("saturates")
                    && note.contains("NaN")
            }));
            assert!(diagnostic.notes.iter().any(|note| {
                note.contains("if value { 1 } else { 0 }") && note.contains("text.parse()")
            }));
        }

        for method in ["ToSingle", "ToDouble"] {
            let source = format!(
                r#"
                    state "game.exe" {{}}
                    fn convert(value: i32) {{ return {owner}.{method}(value) }}
                "#
            );
            let diagnostics = splitscript::compile(&source)
                .expect_err("C# floating conversion should need source-specific guidance");
            assert_eq!(diagnostics.len(), 1, "unexpected cascade: {diagnostics:#?}");
            let diagnostic = &diagnostics[0];
            assert_eq!(
                diagnostic.message,
                "C# floating conversion needs a source-specific SplitScript rewrite"
            );
            assert_eq!(&source[diagnostic.span.start..diagnostic.span.end], method);
            assert!(diagnostic.fixes.is_empty());
            assert!(
                diagnostic
                    .notes
                    .iter()
                    .any(|note| { note.contains("value as f32") && note.contains("value as f64") })
            );
            assert!(diagnostic.notes.iter().any(|note| {
                note.contains("text.parse()") && note.contains("locale-independent")
            }));
        }

        let boolean_source = format!(
            r#"
                state "game.exe" {{}}
                fn convert(value: i32) {{ return {owner}.ToBoolean(value) }}
            "#
        );
        let diagnostics = splitscript::compile(&boolean_source)
            .expect_err("C# boolean conversion should need source-specific guidance");
        assert_eq!(diagnostics.len(), 1, "unexpected cascade: {diagnostics:#?}");
        let diagnostic = &diagnostics[0];
        assert_eq!(
            diagnostic.message,
            "C# boolean conversion needs a source-specific SplitScript expression"
        );
        assert_eq!(
            &boolean_source[diagnostic.span.start..diagnostic.span.end],
            "ToBoolean"
        );
        assert!(diagnostic.fixes.is_empty());
        assert!(
            diagnostic
                .notes
                .iter()
                .any(|note| note.contains("value != 0"))
        );
        assert!(diagnostic.notes.iter().any(|note| {
            note.contains("equalsIgnoreAsciiCase") && note.contains("malformed text")
        }));

        let string_source = format!(
            r#"
                state "game.exe" {{}}
                fn convert(value: i32) {{ return {owner}.ToString(value) }}
            "#
        );
        let diagnostics = splitscript::compile(&string_source)
            .expect_err("C# string conversion should separate display and formatting");
        assert_eq!(diagnostics.len(), 1, "unexpected cascade: {diagnostics:#?}");
        let diagnostic = &diagnostics[0];
        assert_eq!(
            diagnostic.message,
            "C# `Convert.ToString` needs a Display or integer-radix rewrite"
        );
        assert_eq!(
            &string_source[diagnostic.span.start..diagnostic.span.end],
            "ToString"
        );
        assert!(diagnostic.fixes.is_empty());
        assert!(
            diagnostic
                .notes
                .iter()
                .any(|note| { note.contains("value as String") && note.contains("setVariable") })
        );
        assert!(diagnostic.notes.iter().any(|note| {
            note.contains("integer.toString(radix)") && note.contains("2 through 36")
        }));
        assert!(diagnostic.notes.iter().any(|note| {
            note.contains("leading minus sign") && note.contains("two's-complement")
        }));
    }

    splitscript::compile(
        r#"
            state "game.exe" {}
            fn widen(value: u8) -> i32 { return value as i32 }
            fn rounded(value: f64) -> i32 { return value.round() as i32 }
            fn boolInteger(value: bool) -> i32 { return if value { 1 } else { 0 } }
            fn parsed(text: String) -> i32! { return text.parse() }
            fn floating(value: i32) -> f64 { return value as f64 }
            fn boolNumeric(value: i32) -> bool { return value != 0 }
            fn displayed(value: i32) -> String { return value as String }
        "#,
    )
    .expect("canonical source-specific conversions should compile");
}

#[test]
fn csharp_square_root_explains_receiver_width() {
    for (owner, width) in [("Math", "f64"), ("MathF", "f32")] {
        let source = format!(
            r#"
                state "game.exe" {{}}

                fn length(value: f64) {{
                    return {owner}.Sqrt(value)
                }}
            "#
        );
        let diagnostics = splitscript::compile(&source)
            .expect_err("C# square root should receive receiver-width migration guidance");

        assert_eq!(diagnostics.len(), 1, "unexpected cascade: {diagnostics:#?}");
        let diagnostic = &diagnostics[0];
        assert_eq!(
            diagnostic.message,
            "C# square root is a type-preserving `sqrt` method in SplitScript"
        );
        assert_eq!(&source[diagnostic.span.start..diagnostic.span.end], "Sqrt");
        assert!(diagnostic.fixes.is_empty());
        assert!(
            diagnostic
                .notes
                .iter()
                .any(|note| { note.contains(&format!("as {width}")) && note.contains(".sqrt()") })
        );
        assert!(
            diagnostic
                .notes
                .iter()
                .any(|note| note.contains("signed zero") && note.contains("IEEE 754"))
        );
    }

    splitscript::compile(
        r#"
            state "game.exe" {}
            fn wide(value: f64) -> f64 { return value.sqrt() }
            fn narrow(value: f32) -> f32 { return value.sqrt() }
        "#,
    )
    .expect("both canonical receiver widths should compile");
}

#[test]
fn csharp_truncation_explains_receiver_width() {
    for (owner, width) in [("Math", "f64"), ("MathF", "f32")] {
        let source = format!(
            r#"
                state "game.exe" {{}}

                fn whole(value: f64) {{
                    return {owner}.Truncate(value)
                }}
            "#
        );
        let diagnostics = splitscript::compile(&source)
            .expect_err("C# truncation should receive receiver-width migration guidance");

        assert_eq!(diagnostics.len(), 1, "unexpected cascade: {diagnostics:#?}");
        let diagnostic = &diagnostics[0];
        assert_eq!(
            diagnostic.message,
            "C# truncation is a type-preserving `truncate` method in SplitScript"
        );
        assert_eq!(
            &source[diagnostic.span.start..diagnostic.span.end],
            "Truncate"
        );
        assert!(diagnostic.fixes.is_empty());
        assert!(
            diagnostic.notes.iter().any(|note| {
                note.contains(&format!("as {width}")) && note.contains(".truncate()")
            })
        );
        assert!(
            diagnostic
                .notes
                .iter()
                .any(|note| note.contains("toward zero") && note.contains("IEEE 754"))
        );
    }

    splitscript::compile(
        r#"
            state "game.exe" {}
            fn wide(value: f64) -> f64 { return value.truncate() }
            fn narrow(value: f32) -> f32 { return value.truncate() }
        "#,
    )
    .expect("both canonical receiver widths should compile");
}

#[test]
fn csharp_rounding_explains_overloads_and_receiver_width() {
    for (owner, width) in [("Math", "f64"), ("MathF", "f32")] {
        for arguments in ["value", "value, 2"] {
            let source = format!(
                r#"
                    state "game.exe" {{}}

                    fn rounded(value: f64) {{
                        return {owner}.Round({arguments})
                    }}
                "#
            );
            let diagnostics = splitscript::compile(&source)
                .expect_err("C# rounding should receive overload migration guidance");

            assert_eq!(diagnostics.len(), 1, "unexpected cascade: {diagnostics:#?}");
            let diagnostic = &diagnostics[0];
            assert_eq!(
                diagnostic.message,
                "C# rounding needs a receiver method and overload review in SplitScript"
            );
            assert_eq!(&source[diagnostic.span.start..diagnostic.span.end], "Round");
            assert!(diagnostic.fixes.is_empty());
            assert!(diagnostic.notes.iter().any(|note| {
                note.contains(&format!("as {width}")) && note.contains(".round()")
            }));
            assert!(
                diagnostic
                    .notes
                    .iter()
                    .any(|note| note.contains("roundTo(digits)"))
            );
            assert!(diagnostic.notes.iter().any(|note| {
                note.contains("MidpointRounding") && note.contains("AwayFromZero")
            }));
        }
    }

    splitscript::compile(
        r#"
            state "game.exe" {}
            fn wide(value: f64) -> f64 { return value.round() }
            fn narrow(value: f32) -> f32 { return value.round() }
            fn precise(value: f64) -> f64 { return value.roundTo(2) }
        "#,
    )
    .expect("canonical midpoint-to-even rounding forms should compile");
}

#[test]
fn csharp_directed_rounding_explains_direction_width_and_qualified_paths() {
    for (method, canonical, direction, message) in [
        (
            "Floor",
            "floor",
            "negative infinity",
            "C# floor is a type-preserving `floor` method in SplitScript",
        ),
        (
            "Ceiling",
            "ceil",
            "positive infinity",
            "C# ceiling is a type-preserving `ceil` method in SplitScript",
        ),
    ] {
        for (owner, width) in [
            ("Math", "f64"),
            ("MathF", "f32"),
            ("System.Math", "f64"),
            ("System.MathF", "f32"),
        ] {
            let source = format!(
                r#"
                    state "game.exe" {{}}

                    fn rounded(value: f64) {{
                        return {owner}.{method}(value)
                    }}
                "#
            );
            let diagnostics = splitscript::compile(&source)
                .expect_err("C# directed rounding should receive migration guidance");

            assert_eq!(diagnostics.len(), 1, "unexpected cascade: {diagnostics:#?}");
            let diagnostic = &diagnostics[0];
            assert_eq!(diagnostic.message, message);
            assert_eq!(&source[diagnostic.span.start..diagnostic.span.end], method);
            assert!(diagnostic.fixes.is_empty());
            assert!(diagnostic.notes.iter().any(|note| {
                note.contains(&format!("as {width}")) && note.contains(&format!(".{canonical}()"))
            }));
            assert!(
                diagnostic
                    .notes
                    .iter()
                    .any(|note| note.contains(direction) && note.contains("IEEE 754"))
            );
            assert!(
                diagnostic
                    .notes
                    .iter()
                    .any(|note| note.contains("decimal inputs"))
            );
        }
    }

    splitscript::compile(
        r#"
            state "game.exe" {}
            fn floorWide(value: f64) -> f64 { return value.floor() }
            fn floorNarrow(value: f32) -> f32 { return value.floor() }
            fn ceilWide(value: f64) -> f64 { return value.ceil() }
            fn ceilNarrow(value: f32) -> f32 { return value.ceil() }
        "#,
    )
    .expect("canonical directed-rounding forms should compile");
}

#[test]
fn csharp_minimum_and_maximum_explain_type_preservation_and_float_edges() {
    for (method, canonical, zero, message) in [
        (
            "Min",
            "min",
            "negative zero",
            "C# minimum is a receiver-based `min` method in SplitScript",
        ),
        (
            "Max",
            "max",
            "positive zero",
            "C# maximum is a receiver-based `max` method in SplitScript",
        ),
    ] {
        for owner in ["Math", "MathF", "System.Math", "System.MathF"] {
            let source = format!(
                r#"
                    state "game.exe" {{}}

                    fn bounded(left: i32, right: i32) {{
                        return {owner}.{method}(left, right)
                    }}
                "#
            );
            let diagnostics = splitscript::compile(&source)
                .expect_err("C# numeric bounds should receive migration guidance");

            assert_eq!(diagnostics.len(), 1, "unexpected cascade: {diagnostics:#?}");
            let diagnostic = &diagnostics[0];
            assert_eq!(diagnostic.message, message);
            assert_eq!(&source[diagnostic.span.start..diagnostic.span.end], method);
            assert!(diagnostic.fixes.is_empty());
            assert!(
                diagnostic
                    .notes
                    .iter()
                    .any(|note| note.contains(&format!("left.{canonical}(right)")))
            );
            assert!(diagnostic.notes.iter().any(|note| {
                note.contains("signedness") && note.contains("propagate NaN") && note.contains(zero)
            }));
            assert!(diagnostic.notes.iter().any(|note| {
                note.contains("implicit numeric conversions") && note.contains("decimal")
            }));
        }
    }

    splitscript::compile(
        r#"
            state "game.exe" {}
            fn signed(left: i16, right: i16) -> i16 { return left.min(right) }
            fn unsigned(left: u64, right: u64) -> u64 { return left.max(right) }
            fn narrow(left: f32, right: f32) -> f32 { return left.min(right) }
            fn wide(left: f64, right: f64) -> f64 { return left.max(right) }
        "#,
    )
    .expect("canonical numeric minimum and maximum forms should compile");
}

#[test]
fn csharp_absolute_value_explains_signed_minimum_and_receiver_type() {
    for owner in ["Math", "MathF", "System.Math", "System.MathF"] {
        let source = format!(
            r#"
                state "game.exe" {{}}

                fn magnitude(value: i32) {{
                    return {owner}.Abs(value)
                }}
            "#
        );
        let diagnostics = splitscript::compile(&source)
            .expect_err("C# absolute value should receive migration guidance");

        assert_eq!(diagnostics.len(), 1, "unexpected cascade: {diagnostics:#?}");
        let diagnostic = &diagnostics[0];
        assert_eq!(
            diagnostic.message,
            "C# absolute value is a receiver-based `abs` method in SplitScript"
        );
        assert_eq!(&source[diagnostic.span.start..diagnostic.span.end], "Abs");
        assert!(diagnostic.fixes.is_empty());
        assert!(
            diagnostic
                .notes
                .iter()
                .any(|note| note.contains("value.abs()") && note.contains("signed"))
        );
        assert!(diagnostic.notes.iter().any(|note| {
            note.contains("minimum value") && note.contains("C#") && note.contains("throws")
        }));
        assert!(
            diagnostic
                .notes
                .iter()
                .any(|note| note.contains("unsigned inputs") && note.contains("decimal"))
        );
    }

    splitscript::compile(
        r#"
            state "game.exe" {}
            fn integer(value: i32) -> i32 { return value.abs() }
            fn narrow(value: f32) -> f32 { return value.abs() }
            fn wide(value: f64) -> f64 { return value.abs() }
        "#,
    )
    .expect("canonical signed absolute-value forms should compile");
}

#[test]
fn csharp_power_guides_squares_and_typed_masks_without_claiming_general_pow() {
    for owner in ["Math", "MathF", "System.Math", "System.MathF"] {
        let source = format!(
            r#"
                state "game.exe" {{}}

                fn power(value: f64, exponent: f64) {{
                    return {owner}.Pow(value, exponent)
                }}
            "#
        );
        let diagnostics = splitscript::compile(&source)
            .expect_err("C# power should receive exponent-specific migration guidance");

        assert_eq!(diagnostics.len(), 1, "unexpected cascade: {diagnostics:#?}");
        let diagnostic = &diagnostics[0];
        assert_eq!(
            diagnostic.message,
            "C# power needs an exponent-specific rewrite in SplitScript"
        );
        assert_eq!(&source[diagnostic.span.start..diagnostic.span.end], "Pow");
        assert!(diagnostic.fixes.is_empty());
        assert!(
            diagnostic
                .notes
                .iter()
                .any(|note| note.contains("value.squared()") && note.contains("f64 or f32"))
        );
        assert!(
            diagnostic
                .notes
                .iter()
                .any(|note| note.contains("1u64 << exponent") && note.contains("shift range"))
        );
        assert!(
            diagnostic
                .notes
                .iter()
                .any(|note| note.contains("fractional") && note.contains("do not yet"))
        );
    }

    splitscript::compile(
        r#"
            state "game.exe" {}
            fn integer(value: i32) -> i32 { return value.squared() }
            fn narrow(value: f32) -> f32 { return value.squared() }
            fn wide(value: f64) -> f64 { return value.squared() }
        "#,
    )
    .expect("canonical squaring forms should compile");
}

#[test]
fn a_user_binding_named_int32_keeps_its_parse_method() {
    let source = r#"
        struct Parser {}
        state "game.exe" {}

        fn Parser.Parse(text: String) -> i32 {
            return text.parse() else 0
        }

        fn parseWith(Int32: Parser, text: String) -> i32 {
            return Int32.Parse(text)
        }
    "#;

    splitscript::compile(source)
        .expect("a resolved user binding must take precedence over foreign migration patterns");
}

#[test]
fn legacy_settings_add_explains_static_declarations_and_families() {
    let source = r#"
        state "game.exe" {}

        setup {
            settings.Add("2", true, "Levels")
        }
    "#;
    let diagnostics = splitscript::compile(source)
        .expect_err("legacy runtime settings registration should need migration guidance");

    assert_eq!(diagnostics.len(), 1, "unexpected cascade: {diagnostics:#?}");
    let diagnostic = &diagnostics[0];
    assert_eq!(
        diagnostic.message,
        "ASL `settings.Add` calls become declarations in SplitScript"
    );
    assert_eq!(&source[diagnostic.span.start..diagnostic.span.end], "Add");
    assert!(diagnostic.fixes.is_empty());
    assert!(
        diagnostic.notes.iter().any(|note| {
            note.contains("inside `settings`") && note.contains("key \"host-key\"")
        })
    );
    assert!(
        diagnostic.notes.iter().any(|note| {
            note.contains("bounded numbered family") && note.contains("start..=end")
        })
    );
    assert!(diagnostic.notes.iter().any(|note| {
        note.contains("settings.enabled(key)") && note.contains("declared boolean settings")
    }));
}

#[test]
fn familiar_unique_and_key_value_collection_types_have_canonical_fixes() {
    use splitscript::FixApplicability;

    let source = r#"
        state "game.exe" {}

        fn inspect(
            seen: HashSet<String>,
            routes: Dictionary<String, u32>,
            interface: IDictionary<String, u32>,
            rustRoutes: HashMap<String, u32>,
        ) {
            print(seen)
            print(routes)
            print(interface)
            print(rustRoutes)
        }

        whileAttached {
            let seen = HashSet.new<String>()
            let routes = Dictionary.new<String, u32>()
            inspect(seen, routes, routes, routes)
        }
    "#;
    let recovered = splitscript::parse_recovering(source).unwrap();

    assert_eq!(recovered.diagnostics().len(), 6);
    for diagnostic in recovered.diagnostics() {
        let spelling = &source[diagnostic.span.start..diagnostic.span.end];
        let replacement = match spelling {
            "HashSet" => "Set",
            "Dictionary" | "IDictionary" | "HashMap" => "Map",
            _ => panic!("unexpected foreign collection spelling `{spelling}`"),
        };
        let [fix] = diagnostic.fixes.as_slice() else {
            panic!("foreign collection spelling should have one fix: {diagnostic:#?}");
        };
        assert_eq!(fix.applicability, FixApplicability::MachineApplicable);
        assert_eq!(fix.edits.len(), 1);
        assert_eq!(fix.edits[0].span, diagnostic.span);
        assert_eq!(fix.edits[0].replacement, replacement);
    }

    let fixed = source
        .replace("IDictionary", "Map")
        .replace("Dictionary", "Map")
        .replace("HashMap", "Map")
        .replace("HashSet", "Set");
    splitscript::compile(&fixed)
        .expect("applying the canonical collection type replacements should compile");
}

#[test]
fn current_state_fields_can_be_replaced_but_old_state_remains_read_only() {
    let source = r#"
        state "game.exe" {
            scene: i32 at 0x1000;
        }

        whileAttached {
            if current.scene == 7 || current.scene == 8 {
                current.scene += 1
            }
        }
    "#;
    splitscript::compile(source).expect("the current snapshot is intentionally mutable");

    let old_source = source.replace("current.scene += 1", "old.scene = current.scene");
    let diagnostics =
        splitscript::parse(&old_source).expect_err("the historical snapshot must remain read-only");
    assert_eq!(diagnostics.len(), 1);
    assert_eq!(diagnostics[0].message, "`old` state is read-only");
    assert_eq!(
        &old_source[diagnostics[0].span.start..diagnostics[0].span.end],
        "old.scene"
    );
    assert!(
        diagnostics[0]
            .notes
            .iter()
            .any(|note| note.contains("`current`"))
    );
    assert_eq!(
        diagnostics[0].migration_topic(),
        Some("asl.state.mutable-current")
    );
}

#[test]
fn current_state_assignment_uses_field_types_and_snapshot_availability() {
    let source = r#"
        state "game.exe" {
            scene: i32 at 0x1000;
        }

        fn normalizeScene() {
            current.scene = 1
        }

        whileAttached {
            normalizeScene()
        }
    "#;
    splitscript::compile(source)
        .expect("snapshot-aware helpers may replace fields on the current snapshot");

    let unavailable = source.replace("whileAttached", "onDetach");
    let diagnostics = splitscript::compile(&unavailable)
        .expect_err("a current-state mutation requires committed snapshots");
    assert!(diagnostics.iter().any(|diagnostic| {
        diagnostic
            .message
            .contains("requires state snapshots and is unavailable in `onDetach`")
    }));
    assert!(
        diagnostics.iter().any(|diagnostic| {
            diagnostic.migration_topic() == Some("asl.state.helper-snapshots")
        })
    );

    let wrong_type = source.replace("current.scene = 1", "current.scene = false");
    let diagnostics = splitscript::compile(&wrong_type)
        .expect_err("the assigned value must match the state field");
    let mismatch = diagnostics
        .iter()
        .find(|diagnostic| diagnostic.message == "expected `i32`, found `bool`")
        .unwrap_or_else(|| panic!("the assignment has a type mismatch: {diagnostics:#?}"));
    assert!(mismatch.labels.iter().any(|label| {
        label
            .message
            .as_deref()
            .is_some_and(|message| message.contains("state field has type `i32`"))
    }));
}

#[test]
fn duplicate_option_and_result_postfixes_have_a_focused_diagnostic() {
    use splitscript::{DiagnosticLabelStyle, FixApplicability};

    for annotation in ["i32??", "i32!!"] {
        let source = format!(
            r#"
                state "game.exe" {{}}
                fn invalid(value: {annotation}) {{}}
            "#
        );
        let errors =
            splitscript::parse(&source).expect_err("duplicate postfixes should be rejected");
        assert_eq!(errors.len(), 1);
        assert!(
            errors[0].message.contains("two adjacent"),
            "unexpected diagnostic for {annotation}: {}",
            errors[0].message
        );
        assert_eq!(errors[0].labels.len(), 1);
        assert_eq!(errors[0].labels[0].style, DiagnosticLabelStyle::Primary);
        assert_eq!(
            errors[0].labels[0].message.as_deref(),
            Some("this wrapper duplicates the preceding wrapper")
        );
        assert_eq!(errors[0].notes.len(), 1);
        assert_eq!(errors[0].fixes.len(), 1);
        assert_eq!(
            errors[0].fixes[0].applicability,
            FixApplicability::MachineApplicable
        );
        assert_eq!(errors[0].fixes[0].edits.len(), 1);
        let edit = &errors[0].fixes[0].edits[0];
        assert_eq!(&source[edit.span.start..edit.span.end], &annotation[4..]);
        assert!(edit.replacement.is_empty());
    }

    for annotation in ["i32!?", "i32?!"] {
        let source = format!(
            r#"
                state "game.exe" {{}}
                fn valid(value: {annotation}) {{}}
            "#
        );
        splitscript::parse(&source).unwrap_or_else(|errors| {
            panic!("mixed wrapper `{annotation}` should parse: {errors:?}")
        });
    }
}

#[test]
fn string_addition_recommends_explicit_string_construction() {
    use splitscript::DiagnosticLabelStyle;

    for expression in [
        "left + right",
        "left + count",
        "left += right",
        "values[0] += right",
    ] {
        let statement = if expression.contains("+=") {
            expression.to_owned()
        } else {
            format!("let combined = {expression}")
        };
        let source = format!(
            r#"
                state "game.exe" {{}}
                fn combine(left: String, right: String, count: u32, values: [String]) {{
                    {statement}
                }}
            "#
        );
        let errors = splitscript::compile(&source)
            .expect_err("string addition should produce migration guidance");
        let diagnostic = errors
            .iter()
            .find(|diagnostic| diagnostic.message.contains("does not concatenate strings"))
            .unwrap_or_else(|| {
                panic!("missing string-addition diagnostic for `{expression}`: {errors:#?}")
            });
        assert_eq!(diagnostic.labels.len(), 1);
        assert_eq!(diagnostic.labels[0].style, DiagnosticLabelStyle::Primary);
        assert_eq!(
            diagnostic.labels[0].message.as_deref(),
            Some("construct the string explicitly")
        );
        assert!(
            diagnostic
                .notes
                .iter()
                .any(|note| note.contains("template literal") && note.contains("Display"))
        );
        assert!(
            diagnostic
                .notes
                .iter()
                .any(|note| note.contains("String.concat") && note.contains("[String]"))
        );
        assert!(diagnostic.fixes.is_empty());
    }
}

#[test]
fn fallible_values_explain_only_the_available_handling_forms() {
    let invalid = splitscript::compile(
        r#"
            state "game.exe" {}

            fn parts() -> [String]! {
                return ["alpha"]
            }

            fn first() -> String {
                return parts()[0]
            }
        "#,
    )
    .expect_err("indexing an unhandled fallible array must fail");
    let invalid = invalid
        .iter()
        .find(|diagnostic| diagnostic.message.contains("fallible value"))
        .unwrap_or_else(|| panic!("missing fallible-value guidance: {invalid:#?}"));
    assert!(
        invalid
            .notes
            .iter()
            .any(|note| note.contains("else fallback"))
    );
    assert!(
        invalid
            .notes
            .iter()
            .any(|note| note.contains("match value"))
    );
    assert!(
        invalid
            .notes
            .iter()
            .all(|note| !note.contains("postfix `?`"))
    );

    let propagating = splitscript::compile(
        r#"
            state "game.exe" {}

            fn parts() -> [String]! {
                return ["alpha"]
            }

            fn first() -> String! {
                return parts()[0]
            }
        "#,
    )
    .expect_err("the fallible array must be propagated before indexing");
    let propagating = propagating
        .iter()
        .find(|diagnostic| diagnostic.message.contains("fallible value"))
        .unwrap_or_else(|| panic!("missing propagation guidance: {propagating:#?}"));
    assert!(
        propagating
            .notes
            .iter()
            .any(|note| note.contains("postfix `?`") && note.contains("available here"))
    );
    assert!(propagating.fixes.is_empty());
}

#[test]
fn direct_fallible_optional_returns_recommend_else_return_none() {
    use splitscript::FixApplicability;

    let source = r#"
        state "game.exe" {}

        fn optionalName() -> String? {
            return process.readUtf16Le(0x100, 32) else None
        }
    "#;
    let errors = splitscript::compile(source)
        .expect_err("a value fallback cannot stand in for the optional function return");
    let diagnostic = errors
        .iter()
        .find(|diagnostic| diagnostic.message.contains("optional function return"))
        .unwrap_or_else(|| panic!("missing direct-return guidance: {errors:#?}"));
    assert!(
        diagnostic
            .notes
            .iter()
            .any(|note| note.contains("else return None"))
    );
    assert_eq!(diagnostic.fixes.len(), 1);
    assert_eq!(
        diagnostic.fixes[0].applicability,
        FixApplicability::MachineApplicable
    );
    assert_eq!(diagnostic.fixes[0].edits[0].replacement, "return ");
    let edit = &diagnostic.fixes[0].edits[0];
    let fixed = format!(
        "{}{}{}",
        &source[..edit.span.start],
        edit.replacement,
        &source[edit.span.end..]
    );
    splitscript::compile(&fixed).expect("the machine-applicable rewrite must compile");
}

#[test]
fn capability_failures_name_requirements_and_finite_accepted_types() {
    for expression in [
        "Duration.fromMilliseconds(value)",
        "Duration.fromMilliseconds<i64>(value)",
    ] {
        let source = format!(
            r#"
                state "game.exe" {{}}
                fn duration(value: i64) -> Duration {{
                    return {expression}
                }}
            "#
        );
        splitscript::compile(&source).unwrap_or_else(|errors| {
            panic!("numeric duration input `{expression}` failed: {errors:#?}")
        });
    }

    let structural = splitscript::compile(
        r#"
            state "game.exe" {}
            fn invalidRead() {
                process.read<String>(0x100)
            }
        "#,
    )
    .expect_err("String is not memory readable");
    let diagnostic = structural
        .iter()
        .find(|diagnostic| diagnostic.message.contains("`MemoryReadable` capability"))
        .unwrap_or_else(|| panic!("missing structural capability guidance: {structural:#?}"));
    assert!(!diagnostic.message.contains("accepted concrete types"));
}

#[test]
fn failed_initializers_keep_poisoned_bindings_without_follow_on_errors() {
    use splitscript::{
        compiler::ast::Stmt,
        tooling::database::{CompilerDatabase, DefinitionTarget, SourceDefinitionId},
    };

    let source = r#"
        state "game.exe" {}

        let globalBroken = missingGlobal()

        whileAttached {
            let localBroken = [1, 2].missingMember(1)
            let member = localBroken.value
            let indexed = localBroken[0]
            let optional: i32? = localBroken
            let fallible: i32! = localBroken
            localBroken.missingAgain()
            if localBroken == 0 {
                print(localBroken)
                print(globalBroken)
                print(member)
                print(indexed)
                print(optional)
                print(fallible)
            }
        }

        onAttach {
            let attachedBroken = await process.missingAwait()
            print(attachedBroken)
        }
    "#;
    let diagnostics = splitscript::compile(source)
        .expect_err("the three unresolved calls remain primary compilation errors");
    let messages = diagnostics
        .iter()
        .map(|diagnostic| diagnostic.message.as_str())
        .collect::<Vec<_>>();

    assert_eq!(
        messages.len(),
        3,
        "unexpected diagnostic cascade: {messages:#?}"
    );
    assert!(messages.contains(&"unknown function `missingGlobal`"));
    assert!(
        messages
            .iter()
            .any(|message| message.contains("has no method `missingMember`"))
    );
    assert!(
        messages
            .iter()
            .any(|message| message.contains("has no method `missingAwait`"))
    );
    assert!(!messages.iter().any(|message| {
        message.contains("unknown variable")
            || message.contains("<unknown>")
            || message.contains("does not satisfy")
            || message.contains("cannot be indexed")
    }));

    let mut database = CompilerDatabase::new(source);
    database.check().expect_err("the source remains invalid");
    let recovered = database.recovering_check().unwrap();
    let Stmt::Variable(local) = &recovered.syntax().actions[0].body.statements[0] else {
        panic!("expected the failed local declaration");
    };
    let use_site = source.find("localBroken.value").unwrap();
    assert!(matches!(
        database.definition_at(use_site).unwrap(),
        Some(DefinitionTarget::Source(definition))
            if definition.id == SourceDefinitionId::Value(local.id)
    ));
    assert!(database.rename_target_at(use_site).unwrap().is_some());
}

#[test]
fn familiar_declaration_keywords_recover_as_let_with_machine_applicable_fixes() {
    use splitscript::{FixApplicability, compiler::ast::Stmt};

    let source = r#"
        state "game.exe" {}
        const baseAddress = 5
        var fallbackAddress = 0

        onAttach {
            const module = await process.module("GameAssembly.dll")
        }

        whileAttached {
            const address = baseAddress
        }
    "#;
    let recovered = splitscript::parse_recovering(source).unwrap();

    assert_eq!(recovered.diagnostics().len(), 4);
    for diagnostic in recovered.diagnostics() {
        let keyword = &source[diagnostic.span.start..diagnostic.span.end];
        assert!(matches!(keyword, "const" | "var"));
        assert_eq!(
            diagnostic.message,
            format!("SplitScript uses `let` instead of `{keyword}` for variable declarations")
        );
        assert_eq!(diagnostic.fixes.len(), 1);
        let fix = &diagnostic.fixes[0];
        assert_eq!(fix.title, format!("replace `{keyword}` with `let`"));
        assert_eq!(fix.applicability, FixApplicability::MachineApplicable);
        assert_eq!(fix.edits.len(), 1);
        assert_eq!(fix.edits[0].span, diagnostic.span);
        assert_eq!(fix.edits[0].replacement, "let");
    }

    assert_eq!(recovered.syntax().globals.len(), 2);
    assert!(matches!(
        recovered.syntax().actions[0].body.statements[0],
        Stmt::Variable(ref variable)
            if matches!(variable.value.as_ref().unwrap().kind, splitscript::compiler::ast::ExprKind::Suspend { .. })
    ));
    assert!(matches!(
        recovered.syntax().actions[1].body.statements[0],
        Stmt::Variable(_)
    ));
    assert!(recovered.recovery_nodes().is_empty());
    splitscript::compile(&source.replace("const", "let").replace("var", "let"))
        .expect("applying every suggested replacement should produce valid source");
}

#[test]
fn csharp_record_declaration_recovers_as_struct_with_a_machine_applicable_fix() {
    use splitscript::FixApplicability;

    let source = r#"
        record Position {
            x: i32,
            y: i32,
        }

        state "game.exe" {}

        whileAttached {
            let point = Position { x: 1, y: 2 }
            print(point)
        }
    "#;

    splitscript::compile(source).expect_err("`record` must not remain an accepted alias");

    let recovered = splitscript::parse_recovering(source).unwrap();
    assert_eq!(recovered.syntax().structs.len(), 1);
    assert_eq!(recovered.diagnostics().len(), 1);

    let diagnostic = &recovered.diagnostics()[0];
    assert_eq!(
        diagnostic.message,
        "SplitScript uses `struct` instead of C#'s `record` declaration keyword"
    );
    assert_eq!(
        &source[diagnostic.span.start..diagnostic.span.end],
        "record"
    );
    assert_eq!(diagnostic.fixes.len(), 1);

    let fix = &diagnostic.fixes[0];
    assert_eq!(fix.title, "replace `record` with `struct`");
    assert_eq!(fix.applicability, FixApplicability::MachineApplicable);
    assert_eq!(fix.edits.len(), 1);
    assert_eq!(fix.edits[0].span, diagnostic.span);
    assert_eq!(fix.edits[0].replacement, "struct");

    splitscript::compile(&source.replacen("record", "struct", 1))
        .expect("applying the suggested replacement should produce valid source");
}

#[test]
fn null_recovers_as_none_with_machine_applicable_fixes() {
    use splitscript::FixApplicability;

    let source = r#"
        state "game.exe" {}

        fn maybeLevel(selected) -> u32? {
            if selected { return 7 }
            return null
        }

        fn levelName(level: u32?) {
            return match level {
                null => "None",
                Some(value) => value as String
            }
        }
    "#;
    let recovered = splitscript::parse_recovering(source).unwrap();

    assert_eq!(recovered.diagnostics().len(), 2);
    for diagnostic in recovered.diagnostics() {
        assert_eq!(
            diagnostic.message,
            "SplitScript uses `None` instead of `null` for absent optional values"
        );
        assert_eq!(&source[diagnostic.span.start..diagnostic.span.end], "null");
        assert_eq!(diagnostic.fixes.len(), 1);
        let fix = &diagnostic.fixes[0];
        assert_eq!(fix.title, "replace `null` with `None`");
        assert_eq!(fix.applicability, FixApplicability::MachineApplicable);
        assert_eq!(fix.edits.len(), 1);
        assert_eq!(fix.edits[0].span, diagnostic.span);
        assert_eq!(fix.edits[0].replacement, "None");
    }

    assert!(recovered.recovery_nodes().is_empty());
    splitscript::compile(&source.replace("null", "None"))
        .expect("applying every suggested replacement should produce valid source");
}

#[test]
fn familiar_function_and_string_spellings_have_machine_applicable_fixes() {
    use splitscript::FixApplicability;

    let source = r#"
        state "game.exe" {}

        func firstLabel() -> string {
            return "First"
        }

        function identity(value: string) -> String {
            return value
        }

        function string.isEmpty() {
            return self.byteLength() == 0
        }

        function elapsed() -> TimeSpan {
            return TimeSpan.fromSeconds(1.0)
        }

        debug function trace(message: string) {
            print(message)
        }

        whileAttached {
            print(identity(firstLabel()))
            debug trace("updated")
        }
    "#;
    let recovered = splitscript::parse_recovering(source).unwrap();

    assert_eq!(recovered.diagnostics().len(), 11);
    for diagnostic in recovered.diagnostics() {
        let spelling = &source[diagnostic.span.start..diagnostic.span.end];
        let (replacement, message, title) = match spelling {
            "func" | "function" => (
                "fn",
                format!("SplitScript uses `fn` instead of `{spelling}` for functions"),
                format!("replace `{spelling}` with `fn`"),
            ),
            "string" => (
                "String",
                "SplitScript uses `String` instead of `string` for the string type".to_owned(),
                "replace `string` with `String`".to_owned(),
            ),
            "TimeSpan" => (
                "Duration",
                "SplitScript uses `Duration` instead of `TimeSpan` for timer durations".to_owned(),
                "replace `TimeSpan` with `Duration`".to_owned(),
            ),
            _ => panic!("unexpected familiar spelling `{spelling}`"),
        };
        assert_eq!(diagnostic.message, message);
        assert_eq!(diagnostic.fixes.len(), 1);
        let fix = &diagnostic.fixes[0];
        assert_eq!(fix.title, title);
        assert_eq!(fix.applicability, FixApplicability::MachineApplicable);
        assert_eq!(fix.edits.len(), 1);
        assert_eq!(fix.edits[0].span, diagnostic.span);
        assert_eq!(fix.edits[0].replacement, replacement);
    }

    assert_eq!(recovered.syntax().functions.len(), 5);
    assert!(recovered.syntax().functions[4].debug_only);
    assert!(recovered.recovery_nodes().is_empty());
    let fixed = source
        .replace("function", "fn")
        .replace("func", "fn")
        .replace("string", "String")
        .replace("TimeSpan", "Duration");
    splitscript::compile(&fixed)
        .expect("applying every suggested replacement should produce valid source");
}

#[test]
fn csharp_numeric_type_names_have_machine_applicable_fixes() {
    use splitscript::FixApplicability;

    let source = r#"
        state "game.exe" {}

        struct CSharpNumbers {
            signed8: sbyte,
            unsigned8: byte,
            signed16: short,
            unsigned16: ushort,
            signed32: int,
            unsigned32: uint,
            signed64: long,
            unsigned64: ulong,
            single: float,
            doublePrecision: double
        }

        fn int.identity() -> int {
            return self
        }
    "#;
    let recovered = splitscript::parse_recovering(source).unwrap();
    let expected = [
        ("sbyte", "i8"),
        ("byte", "u8"),
        ("short", "i16"),
        ("ushort", "u16"),
        ("int", "i32"),
        ("uint", "u32"),
        ("long", "i64"),
        ("ulong", "u64"),
        ("float", "f32"),
        ("double", "f64"),
        ("int", "i32"),
        ("int", "i32"),
    ];

    assert_eq!(recovered.diagnostics().len(), expected.len());
    for (diagnostic, (csharp_name, splitscript_name)) in
        recovered.diagnostics().iter().zip(expected)
    {
        assert_eq!(
            &source[diagnostic.span.start..diagnostic.span.end],
            csharp_name
        );
        assert_eq!(
            diagnostic.message,
            format!(
                "SplitScript uses `{splitscript_name}` instead of `{csharp_name}` for this numeric type"
            )
        );
        assert_eq!(diagnostic.fixes.len(), 1);
        let fix = &diagnostic.fixes[0];
        assert_eq!(
            fix.title,
            format!("replace `{csharp_name}` with `{splitscript_name}`")
        );
        assert_eq!(fix.applicability, FixApplicability::MachineApplicable);
        assert_eq!(fix.edits.len(), 1);
        assert_eq!(fix.edits[0].span, diagnostic.span);
        assert_eq!(fix.edits[0].replacement, splitscript_name);
    }

    assert!(recovered.recovery_nodes().is_empty());
    let mut fixed = source.to_owned();
    for diagnostic in recovered.diagnostics().iter().rev() {
        let edit = &diagnostic.fixes[0].edits[0];
        fixed.replace_range(edit.span.start..edit.span.end, &edit.replacement);
    }
    splitscript::compile(&fixed)
        .expect("applying every suggested numeric type replacement should produce valid source");
}

#[test]
fn csharp_ulong_state_fields_rewrite_to_unsigned_memory_reads() {
    use splitscript::FixApplicability;

    let source = r#"
        state "Alkali.exe" {
            fileTimer: ulong at "Alkali.exe", 0x568D10;
        }

        split {
            return current.fileTimer == 18_446_744_073_709_551_615u64
        }
    "#;
    let recovered = splitscript::parse_recovering(source).unwrap();

    assert_eq!(recovered.diagnostics().len(), 1);
    let diagnostic = &recovered.diagnostics()[0];
    assert_eq!(&source[diagnostic.span.start..diagnostic.span.end], "ulong");
    assert_eq!(diagnostic.fixes.len(), 1);
    let fix = &diagnostic.fixes[0];
    assert_eq!(fix.applicability, FixApplicability::MachineApplicable);
    assert_eq!(fix.edits[0].replacement, "u64");

    let mut fixed = source.to_owned();
    fixed.replace_range(
        fix.edits[0].span.start..fix.edits[0].span.end,
        &fix.edits[0].replacement,
    );
    splitscript::compile(&fixed)
        .expect("the unsigned 64-bit state field should retain its complete memory range");
}

#[test]
fn unknown_calls_suggest_canonical_names_across_naming_styles() {
    use splitscript::FixApplicability;

    let cases = [
        (
            "Duration.FromSeconds(1.0)",
            "FromSeconds",
            "fromSeconds",
            "Duration.fromSeconds",
        ),
        (
            "Duration.from_seconds(1.0)",
            "from_seconds",
            "fromSeconds",
            "Duration.fromSeconds",
        ),
        (
            "Duration.fromSecnds(1.0)",
            "fromSecnds",
            "fromSeconds",
            "Duration.fromSeconds",
        ),
        (
            "Duration.FromMinutes(1.0)",
            "FromMinutes",
            "fromMinutes",
            "Duration.fromMinutes",
        ),
        (
            "Duration.from_hours(1.0)",
            "from_hours",
            "fromHours",
            "Duration.fromHours",
        ),
        ("value.ClAmP(0, 10)", "ClAmP", "clamp", "clamp"),
        (
            "\"MAP\".ToLower()",
            "ToLower",
            "toAsciiLowerCase",
            "toAsciiLowerCase",
        ),
        (
            "\"map\".ToUpper()",
            "ToUpper",
            "toAsciiUpperCase",
            "toAsciiUpperCase",
        ),
        ("\"a_b\".Split(\"_\")", "Split", "split", "split"),
        (
            "value.increment_by(1)",
            "increment_by",
            "incrementBy",
            "incrementBy",
        ),
        ("values.AddRange([2, 3])", "AddRange", "extend", "extend"),
        ("values.Remove(1)", "Remove", "remove", "remove"),
        ("value.swap_bytes()", "swap_bytes", "swapBytes", "swapBytes"),
        ("value.from_be()", "from_be", "swapBytes", "swapBytes"),
        ("add_one(value)", "add_one", "addOne", "addOne"),
    ];

    for (call, misspelled, replacement, suggested_display) in cases {
        let source = format!(
            r#"
                state "game.exe" {{}}

                fn addOne(value: u32) -> u32 {{
                    return value + 1
                }}

                fn u32.incrementBy(amount: u32) -> u32 {{
                    return self + amount
                }}

                whileAttached {{
                    let value: u32 = 5
                    let values = [1, 2]
                    {call}
                }}
            "#
        );
        let errors = splitscript::compile(&source).expect_err("the misspelled call must fail");
        assert_eq!(errors.len(), 1, "unexpected diagnostics for `{call}`");
        let diagnostic = &errors[0];
        assert!(
            diagnostic
                .message
                .contains(&format!("did you mean `{suggested_display}`?")),
            "{}",
            diagnostic.message
        );
        assert_eq!(
            &source[diagnostic.span.start..diagnostic.span.end],
            misspelled
        );
        assert_eq!(diagnostic.fixes.len(), 1);
        let fix = &diagnostic.fixes[0];
        assert_eq!(fix.applicability, FixApplicability::MachineApplicable);
        assert_eq!(fix.edits.len(), 1);
        assert_eq!(fix.edits[0].span, diagnostic.span);
        assert_eq!(fix.edits[0].replacement, replacement);

        let mut fixed = source;
        let edit = &fix.edits[0];
        fixed.replace_range(edit.span.start..edit.span.end, &edit.replacement);
        splitscript::compile(&fixed)
            .expect("applying the suggested call-name replacement should compile");
    }
}

#[test]
fn common_timespan_constructors_have_composable_machine_fixes() {
    use splitscript::FixApplicability;

    let source = r#"
        state "game.exe" {}

        gameTime {
            let seconds = TimeSpan.FromSeconds(1.5)
            let milliseconds = TimeSpan.FromMilliseconds(250.0)
            return seconds + milliseconds
        }
    "#;
    let parsed = splitscript::parse_recovering(source).unwrap();
    assert_eq!(parsed.diagnostics().len(), 2);
    assert_eq!(
        parsed
            .diagnostics()
            .iter()
            .map(|diagnostic| &source[diagnostic.span.start..diagnostic.span.end])
            .collect::<Vec<_>>(),
        ["TimeSpan", "TimeSpan"]
    );
    for diagnostic in parsed.diagnostics() {
        assert_eq!(diagnostic.fixes.len(), 1, "{diagnostic:?}");
        assert_eq!(
            diagnostic.fixes[0].applicability,
            FixApplicability::MachineApplicable
        );
        assert_eq!(diagnostic.fixes[0].edits.len(), 1);
    }

    let mut fixed = source.to_owned();
    for edit in parsed
        .diagnostics()
        .iter()
        .map(|diagnostic| &diagnostic.fixes[0].edits[0])
        .rev()
    {
        fixed.replace_range(edit.span.start..edit.span.end, &edit.replacement);
    }

    let constructor_diagnostics = splitscript::compile(&fixed)
        .expect_err("the C# constructor casing should still need migration");
    assert_eq!(constructor_diagnostics.len(), 2);
    assert_eq!(
        constructor_diagnostics
            .iter()
            .map(|diagnostic| &fixed[diagnostic.span.start..diagnostic.span.end])
            .collect::<Vec<_>>(),
        ["FromSeconds", "FromMilliseconds"]
    );
    for diagnostic in constructor_diagnostics.iter().rev() {
        assert_eq!(diagnostic.fixes.len(), 1, "{diagnostic:?}");
        assert_eq!(
            diagnostic.fixes[0].applicability,
            FixApplicability::MachineApplicable
        );
        let edit = &diagnostic.fixes[0].edits[0];
        fixed.replace_range(edit.span.start..edit.span.end, &edit.replacement);
    }

    assert!(fixed.contains("Duration.fromSeconds(1.5)"));
    assert!(fixed.contains("Duration.fromMilliseconds(250.0)"));
    splitscript::compile(&fixed)
        .expect("applying every constructor migration fix should produce valid source");
}

#[test]
fn timespan_zero_migrates_to_the_duration_constructor() {
    use splitscript::FixApplicability;

    let source = r#"
        state "game.exe" {}

        gameTime {
            return TimeSpan.Zero
        }
    "#;
    let parsed = splitscript::parse_recovering(source).unwrap();
    assert_eq!(parsed.diagnostics().len(), 1);
    let type_edit = &parsed.diagnostics()[0].fixes[0].edits[0];
    assert_eq!(
        &source[type_edit.span.start..type_edit.span.end],
        "TimeSpan"
    );
    assert_eq!(type_edit.replacement, "Duration");

    let mut fixed = source.to_owned();
    fixed.replace_range(
        type_edit.span.start..type_edit.span.end,
        &type_edit.replacement,
    );
    let diagnostics = splitscript::compile(&fixed)
        .expect_err("the C# static property still needs a constructor-call rewrite");
    assert_eq!(diagnostics.len(), 1);
    let diagnostic = &diagnostics[0];
    assert_eq!(
        &fixed[diagnostic.span.start..diagnostic.span.end],
        "Duration.Zero"
    );
    assert_eq!(diagnostic.fixes.len(), 1);
    assert_eq!(
        diagnostic.fixes[0].applicability,
        FixApplicability::MachineApplicable
    );
    let value_edit = &diagnostic.fixes[0].edits[0];
    assert_eq!(value_edit.replacement, "Duration.zero()");
    fixed.replace_range(
        value_edit.span.start..value_edit.span.end,
        &value_edit.replacement,
    );

    splitscript::compile(&fixed).expect("the fully migrated zero duration should compile");
}

#[test]
fn timespan_parse_requests_an_explicit_semantic_migration() {
    let source = r#"
        state "game.exe" {}

        gameTime {
            return TimeSpan.Parse("00:00:55.75")
        }
    "#;
    let parsed = splitscript::parse_recovering(source).unwrap();
    assert_eq!(parsed.diagnostics().len(), 1);
    let type_edit = &parsed.diagnostics()[0].fixes[0].edits[0];
    let mut fixed = source.to_owned();
    fixed.replace_range(
        type_edit.span.start..type_edit.span.end,
        &type_edit.replacement,
    );

    let diagnostics = splitscript::compile(&fixed)
        .expect_err("culture-sensitive duration parsing requires an explicit rewrite");
    assert_eq!(diagnostics.len(), 1);
    let diagnostic = &diagnostics[0];
    assert_eq!(
        diagnostic.message,
        "C# `TimeSpan.Parse` needs an explicit duration migration"
    );
    assert_eq!(&fixed[diagnostic.span.start..diagnostic.span.end], "Parse");
    assert!(diagnostic.fixes.is_empty());
    assert!(
        diagnostic
            .notes
            .iter()
            .any(|note| note.contains("fixed literal"))
    );
    assert!(
        diagnostic
            .notes
            .iter()
            .any(|note| note.contains("converted to text"))
    );
    assert!(
        diagnostic
            .notes
            .iter()
            .any(|note| note.contains("culture-sensitive"))
    );
}

#[test]
fn timespan_ticks_explain_the_exact_unit_and_range_boundary() {
    let source = r#"
        state "game.exe" {}

        gameTime {
            return TimeSpan.FromTicks(12_345)
        }
    "#;
    let parsed = splitscript::parse_recovering(source).unwrap();
    assert_eq!(parsed.diagnostics().len(), 1);
    let type_edit = &parsed.diagnostics()[0].fixes[0].edits[0];
    let mut fixed = source.to_owned();
    fixed.replace_range(
        type_edit.span.start..type_edit.span.end,
        &type_edit.replacement,
    );

    let diagnostics = splitscript::compile(&fixed)
        .expect_err("C# ticks require an explicit unit and range review");
    assert_eq!(diagnostics.len(), 1);
    let diagnostic = &diagnostics[0];
    assert_eq!(
        diagnostic.message,
        "C# ticks must be converted to a language-level duration unit"
    );
    assert_eq!(
        &fixed[diagnostic.span.start..diagnostic.span.end],
        "FromTicks"
    );
    assert!(diagnostic.fixes.is_empty());
    assert!(diagnostic.notes.iter().any(|note| note.contains("100")));
    assert!(
        diagnostic
            .notes
            .iter()
            .any(|note| note.contains("overflow"))
    );
    assert!(
        diagnostic
            .notes
            .iter()
            .any(|note| note.contains("fromNanoseconds"))
    );
}

#[test]
fn legacy_process_identity_points_to_the_attached_process_api() {
    use splitscript::FixApplicability;

    let source = r#"
        state ["game.exe", "game-demo.exe"] {}

        onAttach {
            print(game.ProcessName)
        }
    "#;
    let errors = splitscript::compile(source)
        .expect_err("the legacy ASL process identity path should need migration");
    assert_eq!(errors.len(), 1);
    let diagnostic = &errors[0];
    assert_eq!(
        diagnostic.message,
        "ASL `game.ProcessName` is `process.name()` in SplitScript"
    );
    assert_eq!(
        &source[diagnostic.span.start..diagnostic.span.end],
        "game.ProcessName"
    );
    assert_eq!(diagnostic.fixes.len(), 1);
    let fix = &diagnostic.fixes[0];
    assert_eq!(fix.applicability, FixApplicability::MachineApplicable);
    assert_eq!(fix.edits.len(), 1);
    assert_eq!(fix.edits[0].replacement, "process.name()");

    let mut fixed = source.to_owned();
    fixed.replace_range(
        fix.edits[0].span.start..fix.edits[0].span.end,
        &fix.edits[0].replacement,
    );
    splitscript::compile(&fixed).expect("the process identity rewrite should compile");
}

#[test]
fn legacy_process_identity_is_not_rewritten_outside_attachment_context() {
    let source = r#"
        state "game.exe" {}

        fn processName() -> String {
            return game.ProcessName
        }
    "#;
    let errors = splitscript::compile(source)
        .expect_err("ordinary functions cannot implicitly capture the process");
    assert_eq!(errors.len(), 1);
    assert_eq!(
        errors[0].message,
        "ASL `game.ProcessName` is `process.name()` in SplitScript"
    );
    assert!(errors[0].fixes.is_empty());
    assert!(
        errors[0]
            .notes
            .iter()
            .any(|note| note.contains("pass the name into an ordinary function"))
    );

    let user_defined = r#"
        state "game.exe" {}

        struct Game {
            ProcessName: String,
        }

        fn readName(game: Game) -> String {
            return game.ProcessName
        }
    "#;
    splitscript::compile(user_defined)
        .expect("a user-defined `game.ProcessName` path must retain its ordinary meaning");
}

#[test]
fn legacy_main_module_query_points_to_explicit_executable_discovery() {
    let source = r#"
        state "game.exe" {}

        onAttach {
            let executable = modules.First()
        }
    "#;
    let errors = splitscript::compile(source)
        .expect_err("the legacy main-module query should need migration guidance");
    assert_eq!(errors.len(), 1, "unexpected cascade: {errors:#?}");
    let diagnostic = &errors[0];
    assert_eq!(
        diagnostic.message,
        "ASL `modules.First()` usually means the attached executable module"
    );
    assert_eq!(&source[diagnostic.span.start..diagnostic.span.end], "First");
    assert!(diagnostic.fixes.is_empty());
    assert_eq!(diagnostic.migration_topic(), Some("asl.process.modules"));
    assert!(
        diagnostic
            .notes
            .iter()
            .any(|note| note.contains("await process.mainModule()"))
    );
    assert!(
        diagnostic
            .notes
            .iter()
            .any(|note| note.contains("versionInfo()"))
    );
}

#[test]
fn legacy_named_module_queries_distinguish_optional_required_and_enumerated_modules() {
    let query = r#"
        state "game.exe" {}

        onAttach {
            let steam = modules.Any("steam_api.dll")
        }
    "#;
    let errors = splitscript::compile(query)
        .expect_err("a module-list query should lead to typed module discovery");
    assert_eq!(errors.len(), 1, "unexpected cascade: {errors:#?}");
    let diagnostic = &errors[0];
    assert_eq!(
        diagnostic.message,
        "ASL module-list queries need an intent-specific SplitScript probe"
    );
    assert!(diagnostic.fixes.is_empty());
    assert!(
        diagnostic
            .notes
            .iter()
            .any(|note| note.contains("process.loadedModule(name)"))
    );
    assert!(
        diagnostic
            .notes
            .iter()
            .any(|note| note.contains("await process.module(name)"))
    );
    assert!(
        diagnostic
            .notes
            .iter()
            .any(|note| note.contains("full module enumeration"))
    );

    let enumeration = r#"
        state "game.exe" {}

        onAttach {
            for module in modules {
                print(module)
            }
        }
    "#;
    let errors = splitscript::compile(enumeration)
        .expect_err("plain ASL module enumeration is not a native collection");
    assert_eq!(errors.len(), 1, "unexpected cascade: {errors:#?}");
    assert_eq!(
        errors[0].message,
        "ASL's enumerable `modules` collection has no direct SplitScript value"
    );
    assert!(
        errors[0]
            .notes
            .iter()
            .any(|note| note.contains("mapped memory ranges"))
    );

    let user_defined = r#"
        state "game.exe" {}

        struct ModuleBag {}

        fn ModuleBag.First() -> u32 {
            return 1
        }

        fn first(modules: ModuleBag) -> u32 {
            return modules.First()
        }
    "#;
    splitscript::compile(user_defined)
        .expect("a resolved user value named `modules` must keep its ordinary methods");
}

#[test]
fn legacy_timer_split_index_requires_explicit_none_handling() {
    let source = r#"
        state "game.exe" {
            level: u32 at 0x100
        }

        split {
            return timer.CurrentSplitIndex == 0 && current.level == 2
        }
    "#;
    let errors = splitscript::compile(source)
        .expect_err("the legacy signed split-index property should need migration");
    assert_eq!(errors.len(), 1);
    let diagnostic = &errors[0];
    assert_eq!(
        diagnostic.message,
        "ASL `timer.CurrentSplitIndex` is optional in SplitScript"
    );
    assert_eq!(
        &source[diagnostic.span.start..diagnostic.span.end],
        "timer.CurrentSplitIndex"
    );
    assert!(diagnostic.fixes.is_empty());
    assert!(
        diagnostic
            .notes
            .iter()
            .any(|note| note.contains("returns `u64?`"))
    );
    assert!(
        diagnostic
            .notes
            .iter()
            .any(|note| note.contains("else return false"))
    );

    let migrated = r#"
        state "game.exe" {
            level: u32 at 0x100
        }

        split {
            let index = timer.currentSplitIndex() else return false
            return index == 0 && current.level == 2
        }
    "#;
    splitscript::compile(migrated)
        .expect("handling the absent split index explicitly should compile");
}

#[test]
fn user_defined_timer_split_index_member_keeps_its_meaning() {
    let source = r#"
        state "game.exe" {}

        struct LegacyTimer {
            CurrentSplitIndex: u64,
        }

        fn readIndex(timer: LegacyTimer) -> u64 {
            return timer.CurrentSplitIndex
        }
    "#;
    splitscript::compile(source)
        .expect("a user-defined timer member must not trigger ASL migration guidance");
}

#[test]
fn legacy_timer_phase_maps_to_the_exhaustive_timer_state() {
    use splitscript::FixApplicability;

    let source = r#"
        state "game.exe" {}

        split {
            return timer.CurrentPhase == TimerPhase.Running
        }
    "#;
    let diagnostics = splitscript::compile(source)
        .expect_err("legacy timer phase paths should use the typed timer state");
    assert_eq!(diagnostics.len(), 1, "unexpected cascade: {diagnostics:#?}");
    let diagnostic = &diagnostics[0];
    assert_eq!(
        diagnostic.message,
        "ASL `timer.CurrentPhase` is `timer.state()` in SplitScript"
    );
    assert_eq!(
        &source[diagnostic.span.start..diagnostic.span.end],
        "timer.CurrentPhase"
    );
    let [fix] = diagnostic.fixes.as_slice() else {
        panic!("the current phase path should have one direct fix: {diagnostic:#?}");
    };
    assert_eq!(fix.applicability, FixApplicability::MachineApplicable);
    let [edit] = fix.edits.as_slice() else {
        panic!("the current phase fix should contain one edit: {fix:#?}");
    };
    let mut fixed = source.to_owned();
    fixed.replace_range(edit.span.start..edit.span.end, &edit.replacement);

    let diagnostics = splitscript::compile(&fixed)
        .expect_err("the legacy enum variant should be diagnosed after fixing the receiver");
    assert_eq!(diagnostics.len(), 1, "unexpected cascade: {diagnostics:#?}");
    let diagnostic = &diagnostics[0];
    assert_eq!(
        diagnostic.message,
        "ASL `TimerPhase.Running` is `TimerState.Running` in SplitScript"
    );
    assert_eq!(
        &fixed[diagnostic.span.start..diagnostic.span.end],
        "TimerPhase.Running"
    );
    let [fix] = diagnostic.fixes.as_slice() else {
        panic!("the legacy enum variant should have one direct fix: {diagnostic:#?}");
    };
    assert_eq!(fix.applicability, FixApplicability::MachineApplicable);
    let [edit] = fix.edits.as_slice() else {
        panic!("the enum-variant fix should contain one edit: {fix:#?}");
    };
    fixed.replace_range(edit.span.start..edit.span.end, &edit.replacement);
    splitscript::compile(&fixed)
        .expect("the canonical typed timer-state comparison should compile");
}

#[test]
fn legacy_timer_phase_type_in_match_patterns_gets_focused_guidance() {
    let source = r#"
        state "game.exe" {}

        fn isRunning(state: TimerState) -> bool {
            return match state {
                TimerPhase.Running => true,
                _ => false,
            }
        }
    "#;
    let diagnostics = splitscript::compile(source)
        .expect_err("an unknown legacy enum in a match should get timer-state guidance");
    assert_eq!(diagnostics.len(), 1, "unexpected cascade: {diagnostics:#?}");
    let diagnostic = &diagnostics[0];
    assert_eq!(
        diagnostic.message,
        "ASL `TimerPhase` is `TimerState` in SplitScript"
    );
    assert_eq!(
        &source[diagnostic.span.start..diagnostic.span.end],
        "TimerPhase"
    );
    assert!(diagnostic.fixes.is_empty());
    assert!(
        diagnostic
            .notes
            .iter()
            .any(|note| note.contains("TimerState.Unknown"))
    );
    assert!(
        diagnostic
            .notes
            .iter()
            .any(|note| note.contains("numeric comparisons"))
    );
}

#[test]
fn user_defined_timer_phase_paths_keep_their_meaning() {
    let source = r#"
        state "game.exe" {}

        enum TimerPhase {
            Running,
        }

        fn isRunning(state: TimerPhase) -> bool {
            return state == TimerPhase.Running
        }
    "#;
    splitscript::compile(source)
        .expect("a source-defined TimerPhase enum must not trigger ASL migration guidance");
}

#[test]
fn legacy_elapsed_time_paths_and_stopwatches_point_to_monotonic_instants() {
    let source = r#"
        state "game.exe" {}

        fn showStopwatch(value: Stopwatch) {
            print(value)
        }

        whileAttached {
            let direct = DateTime.Now
            let timeOfDay = System.DateTime.Now.TimeOfDay
            let tickCount = Environment.TickCount
            let tickCount64 = System.Environment.TickCount64
            print(direct)
            print(timeOfDay)
            print(tickCount)
            print(tickCount64)
        }
    "#;
    let errors = splitscript::compile(source)
        .expect_err("legacy elapsed-time paths and Stopwatch should need a monotonic rewrite");
    assert_eq!(errors.len(), 5);
    for diagnostic in &errors {
        assert_eq!(
            diagnostic.message,
            "use SplitScript's monotonic clock for elapsed-time checks"
        );
        assert!(diagnostic.fixes.is_empty());
        assert!(
            diagnostic
                .notes
                .iter()
                .any(|note| note.contains("hasElapsed"))
        );
    }
    let mut diagnosed = errors
        .iter()
        .map(|error| &source[error.span.start..error.span.end])
        .collect::<Vec<_>>();
    diagnosed.sort_unstable();
    assert_eq!(
        diagnosed,
        [
            "DateTime.Now",
            "Environment.TickCount",
            "Stopwatch",
            "System.DateTime.Now.TimeOfDay",
            "System.Environment.TickCount64",
        ]
    );

    let migrated = r#"
        state "game.exe" {}

        fn debounceReady(startedAt: Instant) -> bool {
            return startedAt.hasElapsed(Duration.fromMilliseconds(500))
        }
    "#;
    splitscript::compile(migrated).expect("an event-anchored monotonic delay should compile");
}

#[test]
fn livesplit_run_real_time_is_not_silently_replaced_with_an_instant() {
    let source = r#"
        state "game.exe" {}

        split {
            return timer.CurrentTime.RealTime.Value.TotalMilliseconds > 500
        }
    "#;
    let errors = splitscript::compile(source)
        .expect_err("LiveSplit run-relative time needs a distinct host capability");
    assert_eq!(errors.len(), 1);
    let diagnostic = &errors[0];
    assert_eq!(
        diagnostic.message,
        "LiveSplit run real time is not the same as SplitScript's monotonic clock"
    );
    assert_eq!(
        &source[diagnostic.span.start..diagnostic.span.end],
        "timer.CurrentTime.RealTime.Value.TotalMilliseconds"
    );
    assert!(diagnostic.fixes.is_empty());
    assert!(
        diagnostic
            .notes
            .iter()
            .any(|note| note.contains("not exposed by the current host contract"))
    );
}

#[test]
fn livesplit_current_game_time_requires_a_host_snapshot() {
    let source = r#"
        state "game.exe" {}

        split {
            return timer.CurrentTime.GameTime.Value.TotalSeconds > 1.0
        }
    "#;
    let errors = splitscript::compile(source)
        .expect_err("LiveSplit game time is not readable through the current host contract");
    assert_eq!(errors.len(), 1);
    let diagnostic = &errors[0];
    assert_eq!(
        diagnostic.message,
        "LiveSplit's current game time is not readable through the current host contract"
    );
    assert_eq!(
        &source[diagnostic.span.start..diagnostic.span.end],
        "timer.CurrentTime.GameTime.Value.TotalSeconds"
    );
    assert!(diagnostic.fixes.is_empty());
    assert!(
        diagnostic
            .notes
            .iter()
            .any(|note| note.contains("coherent optional timer snapshot"))
    );
}

#[test]
fn livesplit_run_metadata_paths_require_a_host_snapshot() {
    for path in ["timer.CurrentSplit.Name", "timer.Run.CategoryName"] {
        let source = format!(
            r#"
                state "game.exe" {{}}

                whileAttached {{
                    print({path})
                }}
            "#
        );
        let errors = splitscript::compile(&source)
            .expect_err("LiveSplit run metadata is not exposed by the current host contract");
        assert_eq!(errors.len(), 1, "unexpected diagnostics for {path}");
        let diagnostic = &errors[0];
        assert_eq!(
            diagnostic.message,
            "LiveSplit run and segment metadata is not exposed by the current host contract"
        );
        assert_eq!(&source[diagnostic.span.start..diagnostic.span.end], path);
        assert!(diagnostic.fixes.is_empty());
    }
}

#[test]
fn livesplit_timer_configuration_reads_and_writes_require_a_host_contract() {
    for path in ["timer.Run.Offset", "timer.CurrentTimingMethod"] {
        let read_source = format!(
            r#"
                state "game.exe" {{}}

                whileAttached {{
                    print({path})
                }}
            "#
        );
        let errors = splitscript::compile(&read_source)
            .expect_err("timer configuration reads need an explicit host contract");
        assert_eq!(errors.len(), 1, "unexpected read diagnostics for {path}");
        let diagnostic = &errors[0];
        assert_eq!(
            diagnostic.message,
            "LiveSplit run offset and timing-method control needs an explicit host contract"
        );
        assert_eq!(
            &read_source[diagnostic.span.start..diagnostic.span.end],
            path
        );
        assert!(diagnostic.fixes.is_empty());

        let write_source = format!(
            r#"
                state "game.exe" {{}}

                whileAttached {{
                    {path} = 0
                }}
            "#
        );
        let errors = splitscript::compile(&write_source)
            .expect_err("timer configuration writes need an explicit host contract");
        assert_eq!(errors.len(), 1, "unexpected write diagnostics for {path}");
        let diagnostic = &errors[0];
        assert_eq!(
            diagnostic.message,
            "LiveSplit run offset and timing-method control needs an explicit host contract"
        );
        assert_eq!(
            &write_source[diagnostic.span.start..diagnostic.span.end],
            path
        );
        assert!(diagnostic.fixes.is_empty());
    }
}

#[test]
fn user_defined_timer_members_keep_their_meaning() {
    let source = r#"
        state "game.exe" {}

        struct RunInfo {
            CategoryName: String,
        }

        struct TimerInfo {
            Run: RunInfo,
        }

        fn category(timer: TimerInfo) {
            return timer.Run.CategoryName
        }
    "#;
    splitscript::compile(source).expect("user-defined timer members should not be migration paths");
}

#[test]
fn user_defined_datetime_member_keeps_its_meaning() {
    let source = r#"
        state "game.exe" {}

        struct Clock {
            Now: u64,
        }

        fn readTimestamp(DateTime: Clock) -> u64 {
            return DateTime.Now
        }
    "#;
    splitscript::compile(source)
        .expect("a user-defined DateTime value must not trigger migration guidance");
}

#[test]
fn unrelated_unknown_methods_do_not_receive_noisy_suggestions() {
    let source = r#"
        state "game.exe" {}

        whileAttached {
            let value: u32 = 5
            value.completelyUnrelated()
        }
    "#;
    let errors = splitscript::compile(source).expect_err("the unknown method must fail");
    assert_eq!(errors.len(), 1);
    assert_eq!(
        errors[0].message,
        "type `u32` has no method `completelyUnrelated`"
    );
    assert!(errors[0].fixes.is_empty());
}

#[test]
fn asl_string_n_fields_offer_an_encoding_aware_rewrite() {
    use splitscript::{FixApplicability, compiler::ast::StateMemoryDecoder};

    let source = r#"
        state "game.exe" {
            string50 map : "game.exe", 0x100, 0x20;
            after: u32 at 0x200
        }
    "#;
    let recovered = splitscript::parse_recovering(source).unwrap();
    assert_eq!(recovered.diagnostics().len(), 1);
    let diagnostic = &recovered.diagnostics()[0];
    assert_eq!(
        diagnostic.message,
        "ASL `stringN` fields need an explicit SplitScript memory decoder"
    );
    assert_eq!(
        &source[diagnostic.span.start..diagnostic.span.end],
        "string50"
    );
    assert!(
        diagnostic
            .notes
            .iter()
            .any(|note| note.contains("auto-detects UTF-16"))
    );
    assert_eq!(diagnostic.fixes.len(), 2);
    let utf8_fix = &diagnostic.fixes[0];
    assert_eq!(utf8_fix.applicability, FixApplicability::MaybeIncorrect);
    assert!(
        utf8_fix
            .title
            .contains("assuming the memory contains UTF-8")
    );
    assert_eq!(utf8_fix.edits.len(), 3);
    let utf16_fix = &diagnostic.fixes[1];
    assert_eq!(utf16_fix.applicability, FixApplicability::MaybeIncorrect);
    assert!(
        utf16_fix
            .title
            .contains("assuming the memory contains UTF-16LE")
    );
    assert_eq!(utf16_fix.edits.len(), 3);

    let state = recovered.syntax().state.as_ref().unwrap();
    assert_eq!(
        state
            .fields
            .iter()
            .map(|field| field.name.as_str())
            .collect::<Vec<_>>(),
        ["map", "after"]
    );
    assert!(matches!(
        &state.fields[0].source,
        splitscript::compiler::ast::StateSource::Pointer(path)
            if matches!(path.decoder, Some(StateMemoryDecoder::Utf8 { max_bytes: 50, .. }))
    ));

    let mut utf8_fixed = source.to_owned();
    for edit in utf8_fix.edits.iter().rev() {
        utf8_fixed.replace_range(edit.span.start..edit.span.end, &edit.replacement);
    }
    assert!(utf8_fixed.contains("map at \"game.exe\", 0x100, 0x20 as utf8(50)"));
    splitscript::compile(&utf8_fixed)
        .expect("the suggested explicit UTF-8 decoder syntax should compile");

    let mut utf16_fixed = source.to_owned();
    for edit in utf16_fix.edits.iter().rev() {
        utf16_fixed.replace_range(edit.span.start..edit.span.end, &edit.replacement);
    }
    assert!(utf16_fixed.contains("map at \"game.exe\", 0x100, 0x20 as utf16le(25)"));
    splitscript::compile(&utf16_fixed)
        .expect("the suggested explicit UTF-16LE decoder syntax should compile");
}

#[test]
fn odd_asl_string_bounds_do_not_offer_an_inexact_utf16_rewrite() {
    let source = r#"
        state "game.exe" {
            string15 map : 0x100
        }
    "#;
    let recovered = splitscript::parse_recovering(source).unwrap();
    let diagnostic = &recovered.diagnostics()[0];
    assert_eq!(diagnostic.fixes.len(), 1);
    assert!(diagnostic.fixes[0].title.contains("UTF-8"));
    assert!(
        diagnostic
            .notes
            .iter()
            .any(|note| note.contains("two-byte code units"))
    );
}

#[test]
fn string_n_like_field_names_are_not_treated_as_asl_types() {
    let source = r#"
        state "game.exe" {
            string50: u32 at 0x100
        }
    "#;
    let recovered = splitscript::parse_recovering(source).unwrap();
    assert!(recovered.diagnostics().is_empty());
    assert_eq!(
        recovered.syntax().state.as_ref().unwrap().fields[0].name,
        "string50"
    );
}

#[test]
fn duplicate_state_blocks_explain_conditional_version_fields_without_cascades() {
    let source = r#"
        state "game.exe" {
            first: u32 at 0x100
        }
        state "game.exe" {
            second: u32 at 0x200
        }
        whileAttached { print(current.first) }
    "#;
    let recovered = splitscript::parse_recovering(source).unwrap();
    assert_eq!(recovered.diagnostics().len(), 1);
    let diagnostic = &recovered.diagnostics()[0];
    assert_eq!(
        diagnostic.message,
        "SplitScript uses one `state` declaration with conditional fields for game versions"
    );
    assert_eq!(diagnostic.labels.len(), 2);
    assert!(
        diagnostic
            .notes
            .iter()
            .any(|note| note.contains("enum global"))
    );
    assert!(diagnostic.fixes.is_empty());
    assert_eq!(recovered.syntax().actions.len(), 1);
    assert_eq!(
        recovered.syntax().state.as_ref().unwrap().fields[0].name,
        "first"
    );
}

#[test]
fn reported_asl_helper_gaps_produce_focused_migration_diagnostics() {
    for (source, expected_message, expected_topic) in [
        (
            r#"
                state "game.exe" {}
                fn useWatchers(values: MemoryWatcherList<u32>) { print(values) }
            "#,
            "ASL `MemoryWatcherList` has no single collection-shaped replacement",
            "asl.state.memory-watcher-list",
        ),
        (
            r#"
                state "game.exe" {}
                fn startWorker() { Task.Run() }
            "#,
            "`Task.Run` threads do not belong in SplitScript's cooperative execution model",
            "asl.async.task-run",
        ),
    ] {
        let diagnostics = splitscript::compile(source).expect_err("legacy helper should be guided");
        let diagnostic = diagnostics
            .iter()
            .find(|diagnostic| diagnostic.message == expected_message)
            .unwrap_or_else(|| panic!("missing focused diagnostic in {diagnostics:#?}"));
        assert_eq!(diagnostic.migration_topic(), Some(expected_topic));
        assert!(diagnostic.fixes.is_empty());
    }
}

#[test]
fn legacy_unity_traversal_points_to_managed_schemas() {
    for source in [
        r#"
            state "game.exe" {}
            onAttach { let runtime = await Unity.mono(MonoVersion.V2) }
        "#,
        r#"
            state "game.exe" {}
            fn inspect(runtime: MonoModule) { print(runtime) }
        "#,
        r#"
            state "game.exe" {}
            onAttach {
                let mono = 0
                mono.Make<String>("Game.Manager", "state")
            }
        "#,
    ] {
        let diagnostics = splitscript::compile(source)
            .expect_err("legacy managed traversal should receive focused guidance");
        let diagnostic = diagnostics
            .iter()
            .find(|diagnostic| diagnostic.migration_topic() == Some("asl.unity.managed-schema"))
            .unwrap_or_else(|| panic!("missing managed-schema diagnostic in {diagnostics:#?}"));
        assert_eq!(
            diagnostic.message,
            "declare Unity managed metadata with an `image` schema"
        );
        assert!(diagnostic.fixes.is_empty());
    }
}

#[test]
fn settings_map_syntax_guides_to_typed_settings_view_operations() {
    use splitscript::FixApplicability;

    let source = r#"
        state "game.exe" {}
        settings { "Enabled" => enabled key "enabled": true }

        fn selected(key: String) -> bool {
            return settings[key]
        }
    "#;
    let diagnostics = splitscript::compile(source)
        .expect_err("ASL settings indexing should receive focused guidance");
    let diagnostic = diagnostics
        .iter()
        .find(|diagnostic| diagnostic.message.contains("settings-map lookup"))
        .expect("settings indexing diagnostic");
    assert_eq!(
        diagnostic.migration_topic(),
        Some("asl.settings.dynamic-lookup")
    );
    let [fix] = diagnostic.fixes.as_slice() else {
        panic!("settings indexing should offer one direct rewrite");
    };
    assert_eq!(fix.applicability, FixApplicability::MachineApplicable);

    let mut fixed = source.to_owned();
    for edit in fix.edits.iter().rev() {
        fixed.replace_range(edit.span.start..edit.span.end, &edit.replacement);
    }
    assert!(fixed.contains("settings.enabled(key)"));
    splitscript::compile(&fixed).expect("the dynamic boolean settings lookup should compile");
}

#[test]
fn settings_contains_key_guides_to_contains_with_an_automatic_rewrite() {
    use splitscript::FixApplicability;

    let source = r#"
        state "game.exe" {}
        settings { "Enabled" => enabled key "enabled": true }

        fn declared(key: String) -> bool {
            return settings.ContainsKey(key)
        }
    "#;
    let diagnostics = splitscript::compile(source)
        .expect_err("ASL settings membership should receive focused guidance");
    let diagnostic = diagnostics
        .iter()
        .find(|diagnostic| diagnostic.message.contains("settings-map lookup"))
        .expect("settings membership diagnostic");
    let [fix] = diagnostic.fixes.as_slice() else {
        panic!("ContainsKey should offer one direct rewrite");
    };
    assert_eq!(fix.applicability, FixApplicability::MachineApplicable);
    assert_eq!(fix.edits[0].replacement, "contains");

    let mut fixed = source.to_owned();
    let edit = &fix.edits[0];
    fixed.replace_range(edit.span.start..edit.span.end, &edit.replacement);
    splitscript::compile(&fixed).expect("the settings membership replacement should compile");
}
