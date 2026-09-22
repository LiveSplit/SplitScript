//! compiler queries integration tests.

#[test]
fn compiler_context_identity_survives_every_pipeline_product() {
    let context = splitscript::CompilerContext::new();
    let parsed = splitscript::parse_with_context(context.clone(), "state \"game.exe\" {}").unwrap();
    assert_eq!(parsed.context(), context);

    let lowered = splitscript::lower(parsed);
    assert_eq!(lowered.context(), context);

    let checked = splitscript::check(lowered).unwrap();
    assert_eq!(checked.context(), context);
    assert_eq!(
        checked.typed_hir().standard_library(),
        &context.standard_library()
    );

    let wasm_ir = splitscript::lower_wasm(&checked);
    assert_eq!(wasm_ir.standard_library(), &context.standard_library());

    let mut database = splitscript::tooling::database::CompilerDatabase::with_context(
        context.clone(),
        "state \"game.exe\" {}",
    );
    assert_eq!(database.context(), context);
    assert_eq!(database.check().unwrap().context(), context);
}

#[test]
fn sibling_state_fields_are_navigable_and_complete_in_state_sources() {
    use splitscript::{
        compiler::semantic::ResolvedValue,
        tooling::database::{CompilerDatabase, DefinitionTarget, SourceDefinitionId},
    };

    let source = r#"
        state "game.exe" {
            dependent: u32 = source;
            source: u32 at 0x1000;
        }
    "#;
    let mut database = CompilerDatabase::new(source);
    let checked = database
        .check()
        .expect("sibling state references should check");
    let fields = checked
        .syntax()
        .state
        .as_ref()
        .unwrap()
        .all_fields()
        .collect::<Vec<_>>();
    let source_field = fields.iter().find(|field| field.name == "source").unwrap();
    let reference = source.find("= source").unwrap() + 2;
    assert!(matches!(
        database.analysis_at(reference).unwrap().unwrap().resolution,
        Some(splitscript::compiler::hir::ExpressionResolution::ValuePath {
            root: Some(ResolvedValue::StateCandidate(field)),
            ..
        }) if field == source_field.id
    ));
    assert!(matches!(
        database.definition_at(reference).unwrap(),
        Some(DefinitionTarget::Source(definition))
            if definition.id == SourceDefinitionId::Value(source_field.id)
    ));
    let hover = database
        .hover(reference)
        .unwrap()
        .expect("sibling state references should retain field hover information");
    assert!(hover.markdown.contains("current.source: u32"));
    assert!(hover.markdown.contains("Transactional state field"));
    let labels = database
        .completions(reference)
        .unwrap()
        .items
        .into_iter()
        .map(|item| item.label)
        .collect::<Vec<_>>();
    assert!(labels.contains(&"source".to_owned()));
    assert!(labels.contains(&"dependent".to_owned()));
}

#[test]
fn compiler_database_publishes_non_fatal_warnings() {
    use splitscript::{DiagnosticSeverity, tooling::database::CompilerDatabase};

    let mut database = CompilerDatabase::new(
        r#"state "game.exe" {} whileAttached { "abc".replaceAll("a", "b") }"#,
    );
    assert!(database.check().is_ok());
    let diagnostics = database.diagnostics();
    assert_eq!(diagnostics.len(), 1);
    assert_eq!(diagnostics[0].severity, DiagnosticSeverity::Warning);
    assert!(diagnostics[0].message.contains("replaceAll"));
}

#[test]
fn inspect_is_a_documented_highlighted_expression_keyword() {
    use splitscript::tooling::{
        database::{CompilerDatabase, DefinitionTarget},
        highlight::SemanticTokenKind,
        language::LanguageItemId,
    };

    let source = "state \"game.exe\" {}\nwhileAttached {\n    let value = inspect(40 + 2)\n    print(value)\n}";
    let inspect = source.find("inspect").unwrap();
    let mut database = CompilerDatabase::new(source);
    database
        .check()
        .expect("inspect should type check as an expression");

    assert_eq!(
        database.definition_at(inspect).unwrap(),
        Some(DefinitionTarget::Language(LanguageItemId::Inspect))
    );
    assert!(
        database
            .hover(inspect)
            .unwrap()
            .expect("inspect should expose its language documentation")
            .markdown
            .contains("evaluates its operand exactly once")
    );
    assert!(
        database
            .semantic_highlights()
            .unwrap()
            .highlights()
            .iter()
            .any(|highlight| highlight.span.start == inspect
                && highlight.kind == SemanticTokenKind::Keyword)
    );

    let completion_source = "state \"game.exe\" {}\nwhileAttached {\n    let value = ins\n}";
    let mut completion_database = CompilerDatabase::new(completion_source);
    assert!(
        completion_database
            .completions(completion_source.find("ins").unwrap() + 3)
            .unwrap()
            .items
            .iter()
            .any(|item| item.label == "inspect"),
        "inspect should be offered wherever an expression can start"
    );
}

#[test]
fn represented_enum_hovers_explain_memory_layout_and_effective_discriminants() {
    use splitscript::tooling::database::CompilerDatabase;

    let source = r#"
        enum GameState: i32 {
            Mission = 0,
            TitleScreen,
            Results = 6,
        }
        state "game.exe" {}
        whileAttached { print(GameState.Results) }
    "#;
    let mut database = CompilerDatabase::new(source);
    database
        .check()
        .expect("represented enum hover fixture should check");

    let enum_offset = source.find("GameState: i32").unwrap() + 2;
    let enum_hover = database.hover(enum_offset).unwrap().unwrap();
    assert!(enum_hover.markdown.contains("enum GameState: i32"));
    assert!(
        enum_hover
            .markdown
            .contains("Process-memory representation:** `i32` (4 bytes)")
    );
    assert!(enum_hover.markdown.contains("MemoryReadable"));

    let implicit_offset = source.find("TitleScreen").unwrap() + 2;
    let implicit_hover = database.hover(implicit_offset).unwrap().unwrap();
    assert!(
        implicit_hover
            .markdown
            .contains("GameState.TitleScreen = 1")
    );
    assert!(
        implicit_hover
            .markdown
            .contains("Process-memory discriminant:** `1`")
    );
}

#[test]
fn one_shot_compilation_reports_syntax_and_independent_type_errors_together() {
    let source = r#"
        state GBA {}

        fn conflictingReturns() {
            if true { return 5 }
            return false
        }

        fn malformedLiteral(value: u32) -> bool {
            return value > 0gxb101
        }
    "#;
    let diagnostics = splitscript::compile(source).expect_err("both errors reject compilation");
    assert!(
        diagnostics
            .iter()
            .any(|diagnostic| { diagnostic.message == "unknown integer type suffix `gxb101`" })
    );
    assert!(diagnostics.iter().any(|diagnostic| {
        diagnostic
            .message
            .starts_with("type `bool` does not satisfy the required `Numeric` capability")
    }));
}

#[test]
fn compiler_database_applies_warning_policy_without_losing_semantics() {
    use splitscript::{
        DiagnosticCode, DiagnosticSeverity, WarningLevel, WarningPolicy,
        tooling::database::CompilerDatabase,
    };

    let source = r#"state "game.exe" {} whileAttached { let unread = 1 }"#;
    let mut database = CompilerDatabase::new(source);
    database
        .check()
        .expect("warning policy must not invalidate semantic products");

    let mut policy = WarningPolicy::default();
    assert!(policy.set(DiagnosticCode::UnusedBinding, WarningLevel::Deny));
    assert!(database.set_warning_policy(policy));
    let diagnostics = database.diagnostics();
    assert_eq!(diagnostics.len(), 1);
    assert_eq!(diagnostics[0].code, DiagnosticCode::UnusedBinding);
    assert_eq!(diagnostics[0].severity, DiagnosticSeverity::Error);
    database
        .check()
        .expect("denial changes the product status, not semantic checking");

    assert!(policy.set(DiagnosticCode::UnusedBinding, WarningLevel::Allow));
    assert!(database.set_warning_policy(policy));
    assert!(database.diagnostics().is_empty());
}

#[test]
fn vscode_manifest_tracks_the_lsp_semantic_token_legend() {
    use std::collections::BTreeSet;

    use serde_json::Value;
    use splitscript::tooling::highlight::{SEMANTIC_TOKEN_MODIFIERS, SemanticTokenKind};

    let manifest: Value = serde_json::from_str(include_str!("../../editors/vscode/package.json"))
        .expect("VS Code manifest should be valid JSON");
    serde_json::from_str::<Value>(include_str!(
        "../../editors/vscode/language-configuration.json"
    ))
    .expect("language configuration should be valid JSON");
    let grammar: Value = serde_json::from_str(include_str!(
        "../../editors/vscode/syntaxes/splitscript.tmLanguage.json"
    ))
    .expect("fallback grammar should be valid JSON");
    let grammar_source = include_str!("../../editors/vscode/syntaxes/splitscript.tmLanguage.json");

    assert_eq!(
        manifest["contributes"]["languages"][0]["extensions"][0],
        ".split"
    );
    assert_eq!(grammar["scopeName"], "source.splitscript");
    let fallback_keyword_patterns = grammar["repository"]["language"]["patterns"]
        .as_array()
        .unwrap()
        .iter()
        .filter(|pattern| {
            pattern["name"]
                .as_str()
                .is_some_and(|name| name.starts_with("keyword."))
        })
        .map(|pattern| pattern["match"].as_str().unwrap())
        .collect::<Vec<_>>();
    for keyword in splitscript::tooling::language::LanguageCatalog::new()
        .items()
        .filter_map(|item| match item.kind {
            splitscript::tooling::language::LanguageItemKind::Keyword => Some(item.name),
            _ => None,
        })
    {
        assert!(
            fallback_keyword_patterns
                .iter()
                .flat_map(|pattern| {
                    pattern.split(|character: char| {
                        !(character.is_ascii_alphanumeric() || character == '_')
                    })
                })
                .any(|spelling| spelling == keyword),
            "fallback grammar does not classify catalog keyword `{keyword}` as a keyword"
        );
    }
    assert!(
        grammar_source.contains("Some|None|Ok|Err"),
        "fallback grammar should recognize wrapper enum variants"
    );
    for action in splitscript::tooling::language::LanguageCatalog::new()
        .items()
        .filter_map(|item| match item.kind {
            splitscript::tooling::language::LanguageItemKind::Action(_) => Some(item.name),
            _ => None,
        })
    {
        assert!(
            grammar_source.contains(action),
            "fallback grammar is missing catalog action `{action}`"
        );
    }
    assert_eq!(
        manifest["contributes"]["configurationDefaults"]["[splitscript]"]["editor.semanticHighlighting.enabled"],
        true
    );
    let commands = manifest["contributes"]["commands"]
        .as_array()
        .unwrap()
        .iter()
        .map(|command| command["command"].as_str().unwrap())
        .collect::<BTreeSet<_>>();
    for command in [
        "splitscript.buildRelease",
        "splitscript.startDebugWatch",
        "splitscript.stopDebugWatch",
    ] {
        assert!(commands.contains(command));
    }
    let editor_actions = manifest["contributes"]["menus"]["editor/title"]
        .as_array()
        .unwrap();
    assert!(
        editor_actions
            .iter()
            .any(|menu| menu["command"] == "splitscript.buildRelease")
    );
    for command in ["splitscript.startDebugWatch", "splitscript.stopDebugWatch"] {
        assert!(
            !editor_actions.iter().any(|menu| menu["command"] == command),
            "debug watch commands should not duplicate the debugger button in the editor title"
        );
    }
    let editor_context_actions = manifest["contributes"]["menus"]["editor/context"]
        .as_array()
        .unwrap();
    for command in ["splitscript.startDebugWatch", "splitscript.stopDebugWatch"] {
        assert!(
            editor_context_actions
                .iter()
                .any(|menu| menu["command"] == command),
            "debug watch command `{command}` should remain available in the editor context menu"
        );
    }
    assert!(
        manifest["contributes"]["configuration"]["properties"]["splitScript.compiler.profile"]
            .is_null()
    );

    let standard_types = BTreeSet::from([
        "namespace",
        "type",
        "class",
        "enum",
        "interface",
        "struct",
        "typeParameter",
        "parameter",
        "variable",
        "property",
        "enumMember",
        "event",
        "function",
        "method",
        "macro",
        "label",
        "comment",
        "string",
        "keyword",
        "number",
        "regexp",
        "operator",
        "decorator",
    ]);
    let expected_types = SemanticTokenKind::ALL
        .iter()
        .map(|kind| kind.name())
        .filter(|name| !standard_types.contains(name))
        .collect::<BTreeSet<_>>();
    let contributed_types = manifest["contributes"]["semanticTokenTypes"]
        .as_array()
        .unwrap()
        .iter()
        .map(|item| item["id"].as_str().unwrap())
        .collect::<BTreeSet<_>>();
    assert_eq!(contributed_types, expected_types);

    let standard_modifiers = BTreeSet::from([
        "declaration",
        "definition",
        "readonly",
        "static",
        "deprecated",
        "abstract",
        "async",
        "modification",
        "documentation",
        "defaultLibrary",
    ]);
    let expected_modifiers = SEMANTIC_TOKEN_MODIFIERS
        .iter()
        .copied()
        .filter(|modifier| !standard_modifiers.contains(modifier))
        .collect::<BTreeSet<_>>();
    let contributed_modifiers = manifest["contributes"]["semanticTokenModifiers"]
        .as_array()
        .unwrap()
        .iter()
        .map(|item| item["id"].as_str().unwrap())
        .collect::<BTreeSet<_>>();
    assert_eq!(contributed_modifiers, expected_modifiers);
}

#[test]
fn compiler_stages_retain_the_lossless_source_document() {
    let source = "/* lead */\r\nstate \"game.exe\" { /* body */ } // tail\r\n";
    let parsed = splitscript::parse(source).expect("commented source should parse");
    assert_eq!(parsed.source_document().reconstruct(), source);
    assert_eq!(parsed.source_document().source(), source);

    let lowered = splitscript::lower(parsed);
    assert_eq!(lowered.source_document().reconstruct(), source);

    let checked = splitscript::check(lowered).expect("commented source should check");
    assert_eq!(checked.source_document().reconstruct(), source);
    assert_eq!(checked.source_document().trivia().count(), 10);
}

#[test]
fn compiler_database_caches_queries_and_invalidates_on_source_changes() {
    use std::sync::Arc;

    use splitscript::{DiagnosticCode, tooling::database::CompilerDatabase};

    let valid = r#"
        state "game.exe" {}
        whileAttached { print("cached") }
    "#;
    let mut database = CompilerDatabase::new(valid);
    assert_eq!(database.revision().index(), 0);

    let recovered = database.recovering_parse().unwrap();
    assert!(Arc::ptr_eq(
        &recovered,
        &database.recovering_parse().unwrap()
    ));
    let parsed = database.parse().unwrap();
    assert!(Arc::ptr_eq(&parsed, &database.parse().unwrap()));
    let lowered = database.lower().unwrap();
    assert!(Arc::ptr_eq(&lowered, &database.lower().unwrap()));
    let checked = database.check().unwrap();
    assert!(Arc::ptr_eq(&checked, &database.check().unwrap()));
    let no_diagnostics = database.diagnostics();
    assert!(no_diagnostics.is_empty());
    assert!(Arc::ptr_eq(&no_diagnostics, &database.diagnostics()));

    assert!(!database.set_source(valid));
    assert_eq!(database.revision().index(), 0);
    assert!(Arc::ptr_eq(&checked, &database.check().unwrap()));

    let invalid = r#"
        state "game.exe" {}
        whileAttached {
            let broken = +
            print("partial syntax remains available")
        }
    "#;
    assert!(database.set_source(invalid));
    assert_eq!(database.revision().index(), 1);
    let recovered_error = database.recovering_parse().unwrap();
    assert_eq!(recovered_error.diagnostics().len(), 1);
    assert_eq!(recovered_error.syntax().actions[0].body.statements.len(), 2);
    let parse_errors = database.parse().unwrap_err();
    assert_eq!(parse_errors[0].code, DiagnosticCode::Syntax);
    let diagnostics = database.diagnostics();
    assert!(
        diagnostics
            .iter()
            .any(|diagnostic| diagnostic.code == DiagnosticCode::Syntax)
    );
    assert!(Arc::ptr_eq(&diagnostics, &database.diagnostics()));

    assert!(database.set_source(valid));
    assert_eq!(database.revision().index(), 2);
    let rechecked = database.check().unwrap();
    assert!(!Arc::ptr_eq(&checked, &rechecked));
}

#[test]
fn compiler_database_caches_formatting_without_type_checking() {
    use std::sync::Arc;

    use splitscript::tooling::database::CompilerDatabase;

    let source = "state \"game.exe\"{}\nwhileAttached{let broken:bool=42}";
    let mut database = CompilerDatabase::new(source);
    assert!(database.check().is_err());

    let formatted = database.format().unwrap();
    assert!(Arc::ptr_eq(&formatted, &database.format().unwrap()));
    assert_eq!(
        &*formatted,
        "state \"game.exe\" {}\nwhileAttached {\n    let broken: bool = 42\n}\n"
    );

    assert!(database.set_source("state \"game.exe\" {"));
    assert!(database.format().is_err());
}

#[test]
fn compiler_database_recovers_after_an_unterminated_template_interpolation_edit() {
    use splitscript::tooling::database::CompilerDatabase;

    let valid = r#"
        state "game.exe" {}
        let once = false
        whileAttached {
            if !once {
                print(`GameManager {once}`)
                once = true
            }
        }
        fn after() { return 7 }
    "#;
    let malformed = valid.replace("{once}`", "{once)");
    let mut database = CompilerDatabase::new(valid);

    assert!(database.set_source(malformed));
    assert!(
        !database.diagnostics().is_empty(),
        "the incomplete interpolation should remain diagnosable"
    );
    assert!(database.set_source(valid));
    database
        .check()
        .expect("a later valid edit must recover the complete editor pipeline");
}

#[test]
fn declaration_resolution_errors_do_not_poison_syntax_queries() {
    use splitscript::{DiagnosticCode, tooling::database::CompilerDatabase};

    let source = "enum TimerState { Custom }\nstate \"game.exe\" {}\n";
    let mut database = CompilerDatabase::new(source);
    let recovered = database.recovering_parse().unwrap();
    assert!(recovered.diagnostics().is_empty());
    assert_eq!(recovered.resolution_diagnostics().len(), 1);
    assert_eq!(
        recovered.resolution_diagnostics()[0].code,
        DiagnosticCode::Type
    );

    database.parse().expect("the syntax tree remains valid");
    assert_eq!(
        &*database.format().unwrap(),
        "enum TimerState {\n    Custom,\n}\nstate \"game.exe\" {}\n"
    );
    assert!(database.lower().is_ok());

    let errors = database.check().unwrap_err();
    assert_eq!(errors.len(), 1);
    assert_eq!(errors[0].span.start, "enum ".len());
    assert_eq!(errors[0].span.end, "enum TimerState".len());
}

#[test]
fn unknown_nominal_types_are_resolved_after_parsing() {
    use splitscript::{DiagnosticCode, tooling::database::CompilerDatabase};

    let source = "fn use(value: Mystery) {}\nstate \"game.exe\" {}\n";
    let parsed = splitscript::parse(source).expect("unknown names are still valid type syntax");
    let errors = splitscript::check(parsed).expect_err("resolution should reject the type name");
    assert_eq!(errors.len(), 1);
    assert_eq!(errors[0].code, DiagnosticCode::Type);
    assert_eq!(errors[0].message, "unknown type `Mystery`");
    assert_eq!(&source[errors[0].span.start..errors[0].span.end], "Mystery");

    let mut database = CompilerDatabase::new(source);
    assert!(database.parse().is_ok());
    assert!(database.format().is_ok());
    let recovered = database
        .recovering_check()
        .expect("unknown types must not panic recovering semantics");
    assert!(
        recovered
            .diagnostics()
            .iter()
            .any(|diagnostic| diagnostic.message == "unknown type `Mystery`")
    );
}

#[test]
fn strict_and_recovering_checks_share_post_type_validation() {
    use splitscript::{DiagnosticCode, tooling::database::CompilerDatabase};

    let source = r#"
        fn readValue() -> f32! {
            return process.read<f32>(0)
        }
        state "game.exe" {}
        onDetach {
            let value = readValue()
        }
    "#;
    let mut database = CompilerDatabase::new(source);
    let strict = database.check().unwrap_err();
    assert!(strict.iter().any(|error| {
        error.code == DiagnosticCode::Semantic && error.message.contains("onDetach")
    }));

    let recovered = database.recovering_check().unwrap();
    assert!(recovered.diagnostics().iter().any(|error| {
        error.code == DiagnosticCode::Semantic && error.message.contains("onDetach")
    }));
    assert!(
        recovered.effects().is_some(),
        "derived effect facts survive post-type validation diagnostics"
    );
}

#[test]
fn struct_literals_resolve_their_nominal_identity_after_parsing() {
    let source = r#"
        state "game.exe" {}
        whileAttached {
            let value = Mystery { field: 1 }
        }
    "#;
    let parsed = splitscript::parse(source).expect("struct-literal shape is purely syntactic");
    let errors = splitscript::check(parsed).expect_err("the nominal struct must resolve later");
    assert!(
        errors
            .iter()
            .any(|error| error.message == "unknown struct type `Mystery`")
    );
}

#[test]
fn enum_syntax_stays_named_while_semantics_publish_resolved_variants() {
    use splitscript::compiler::ast::{
        EnumReference, Expr, ExprKind, MatchPattern, StateSource, Stmt,
    };

    let source = r#"
        enum Mode {
            Idle,
            Active
        }
        state "game.exe" { mode = Mode.Idle }
        fn active(mode: Mode) -> bool {
            return match mode {
                Mode.Idle => false,
                Mode.Active => true
            }
        }
    "#;
    let parsed = splitscript::parse(source).unwrap();
    assert!(matches!(
        parsed.syntax().functions[0].params[0].annotation,
        Some(splitscript::compiler::ast::TypeRef::Named(_))
    ));
    let StateSource::Expression(initializer) =
        &parsed.syntax().state.as_ref().unwrap().fields[0].source
    else {
        panic!("expected an expression-backed field");
    };
    assert!(matches!(&initializer.kind, ExprKind::Path(_)));
    let Stmt::Expression(Expr {
        kind: ExprKind::Return(Some(matched)),
        ..
    }) = &parsed.syntax().functions[0].body.statements[0]
    else {
        panic!("expected a match return");
    };
    let ExprKind::Match { arms, .. } = &matched.kind else {
        panic!("expected a match expression");
    };
    assert!(matches!(
        &arms[0].pattern,
        MatchPattern::Enum {
            enumeration: EnumReference { .. },
            ..
        }
    ));

    let lowered = splitscript::lower(parsed);
    assert!(matches!(
        lowered.syntax().functions[0].params[0].annotation,
        Some(splitscript::compiler::ast::TypeRef::Named(_))
    ));
    let StateSource::Expression(initializer) =
        &lowered.syntax().state.as_ref().unwrap().fields[0].source
    else {
        unreachable!();
    };
    assert!(matches!(&initializer.kind, ExprKind::Path(_)));
    let Stmt::Expression(Expr {
        kind: ExprKind::Return(Some(matched)),
        ..
    }) = &lowered.syntax().functions[0].body.statements[0]
    else {
        unreachable!();
    };
    let ExprKind::Match { arms, .. } = &matched.kind else {
        unreachable!();
    };
    assert!(matches!(
        &arms[0].pattern,
        MatchPattern::Enum {
            enumeration: EnumReference { .. },
            ..
        }
    ));
    let checked = splitscript::check(lowered).unwrap();
    let StateSource::Expression(initializer) =
        &checked.syntax().state.as_ref().unwrap().fields[0].source
    else {
        unreachable!();
    };
    assert!(checked.semantics().enum_variant(initializer.id).is_some());
    let Stmt::Expression(Expr {
        kind: ExprKind::Return(Some(matched)),
        ..
    }) = &checked.syntax().functions[0].body.statements[0]
    else {
        unreachable!();
    };
    let ExprKind::Match { arms, .. } = &matched.kind else {
        unreachable!();
    };
    assert!(
        checked
            .semantics()
            .pattern_variant(arms[0].pattern_id)
            .is_some()
    );
}

#[test]
fn unknown_pattern_enums_are_resolution_errors_not_syntax_errors() {
    let source = r#"
        state "game.exe" {}
        fn check(value: i32) -> bool {
            return match value { Missing.Value => true, _ => false }
        }
    "#;
    let parsed = splitscript::parse(source).expect("qualified patterns are valid syntax");
    let errors = splitscript::check(parsed).expect_err("the enum name must resolve");
    assert!(errors.iter().any(|error| {
        error.code == splitscript::DiagnosticCode::Type
            && error.message == "unknown enum `Missing`"
            && &source[error.span.start..error.span.end] == "Missing"
    }));
}

#[test]
fn nested_array_patterns_expose_enum_and_binding_definitions() {
    use splitscript::tooling::database::{CompilerDatabase, DefinitionTarget, SourceDefinitionId};
    use splitscript::tooling::highlight::SemanticTokenKind;

    let source = r#"
        enum Mode { Idle, Active }
        state "game.exe" {}

        fn classify(values: [Mode; 2]) -> Mode {
            return match values {
                [Mode.Idle, selected] => selected,
                [_, selected] => selected,
            }
        }
    "#;
    let mut database = CompilerDatabase::new(source);
    let checked = database.check().expect("nested patterns should type check");
    let enumeration = checked.syntax().enums[0].id;
    let idle = checked.syntax().enums[0].variants[0].id;

    let pattern = source.find("Mode.Idle, selected").unwrap();
    assert!(matches!(
        database.definition_at(pattern + 1).unwrap(),
        Some(DefinitionTarget::Source(definition))
            if definition.id == SourceDefinitionId::Enum(enumeration)
    ));
    assert!(matches!(
        database.definition_at(pattern + "Mode.".len()).unwrap(),
        Some(DefinitionTarget::Source(definition))
            if definition.id == SourceDefinitionId::EnumVariant(idle)
    ));

    let declaration = pattern + "Mode.Idle, ".len();
    let use_offset = source.find("=> selected").unwrap() + "=> ".len();
    assert_eq!(
        database.definition_at(use_offset).unwrap(),
        database.definition_at(declaration).unwrap()
    );
    assert!(
        database
            .hover(declaration)
            .unwrap()
            .expect("a nested pattern binding should have hover information")
            .markdown
            .contains("selected: Mode")
    );
    assert_eq!(
        database
            .rename_target_at(declaration)
            .unwrap()
            .expect("a nested pattern binding should be renameable")
            .name,
        "selected"
    );
    let highlights = database.semantic_highlights().unwrap();
    assert!(highlights.highlights().iter().any(|highlight| {
        highlight.span.start == pattern + "Mode.".len()
            && highlight.kind == SemanticTokenKind::EnumMember
    }));
    assert!(highlights.highlights().iter().any(|highlight| {
        highlight.span.start == declaration && highlight.kind == SemanticTokenKind::Variable
    }));
}

#[test]
fn alternative_pattern_bindings_have_one_editor_identity() {
    use splitscript::tooling::database::{CompilerDatabase, DefinitionTarget, SourceDefinitionId};
    use splitscript::tooling::highlight::SemanticTokenKind;

    let source = r#"
        enum Side { Left(u32), Right(u32), Idle }
        state "game.exe" {}

        fn unwrap(side: Side?) -> u32 {
            return match side {
                Some(Side.Left(value)) | Some(Side.Right(value)) => value,
                Some(_) => 0,
                None => 0,
            }
        }
    "#;
    let mut database = CompilerDatabase::new(source);
    database
        .check()
        .expect("compatible alternative bindings should type check");
    let first = source.find("value)) | ").unwrap();
    let operator = first + "value)) ".len();
    let second = source.find("value)) =>").unwrap();
    let used = source.rfind("=> value").unwrap() + "=> ".len();

    let target = database.definition_at(used).unwrap();
    assert!(matches!(
        &target,
        Some(DefinitionTarget::Source(definition))
            if matches!(definition.id, SourceDefinitionId::Value(_))
                && definition.span.start == first
    ));
    assert_eq!(database.definition_at(first).unwrap(), target);
    assert_eq!(database.definition_at(second).unwrap(), target);
    assert_eq!(
        database.definition_at(operator).unwrap(),
        Some(DefinitionTarget::Language(
            splitscript::tooling::language::LanguageItemId::Match
        ))
    );
    for offset in [first, second, used] {
        assert!(
            database
                .hover(offset)
                .unwrap()
                .expect("every logical binding occurrence should have hover information")
                .markdown
                .contains("value: u32")
        );
    }
    let rename = database.rename_at(second, "payload").unwrap();
    assert_eq!(rename.edits.len(), 3);
    assert!(
        rename
            .edits
            .iter()
            .all(|edit| &source[edit.span.start..edit.span.end] == "value")
    );
    let highlights = database.semantic_highlights().unwrap();
    for offset in [first, second] {
        assert!(highlights.highlights().iter().any(|highlight| {
            highlight.span.start == offset && highlight.kind == SemanticTokenKind::Variable
        }));
    }
}

#[test]
fn compiler_database_exposes_types_resolutions_and_references() {
    use std::sync::Arc;

    use splitscript::{
        compiler::ast::{ExprKind, StateSource, Stmt},
        compiler::hir::{DeclarationId, ExpressionResolution},
        compiler::semantic::{ResolvedCall, ResolvedReceiver, ResolvedValue},
        compiler::stdlib::StdlibItemId,
        compiler::types::{BuiltinType, TypeKind},
        tooling::database::{
            CompilerDatabase, DefinitionTarget, DocumentHighlightKind, SourceDefinitionId,
            ValueReferenceKind,
        },
    };

    let source = r#"
        let global = 1
        state "game.exe" {
            level: i32 at 0x1000
        }
        fn bump(value) {
            let local = value + global.min(10)
            global += local
            return local
        }
        whileAttached {
            let result = bump(current.level)
            print(result as String)
        }
    "#;
    let mut database = CompilerDatabase::new(source);
    let checked = database.check().unwrap();
    let syntax = checked.syntax();
    let global = syntax.globals[0].id;
    let state = syntax.state.as_ref().unwrap().fields[0].id;
    assert!(matches!(
        syntax.state.as_ref().unwrap().fields[0].source,
        StateSource::Pointer(_)
    ));
    let function = &syntax.functions[0];
    let parameter = function.params[0].id;
    let Stmt::Variable(local) = &function.body.statements[0] else {
        panic!("expected the local declaration");
    };
    let local_id = local.id;
    let ExprKind::Binary { right, .. } = &local.value.as_ref().unwrap().kind else {
        panic!("expected the local binary initializer");
    };
    let min_call = right.id;
    let Stmt::Assign { id: assignment, .. } = function.body.statements[1] else {
        panic!("expected the global assignment");
    };
    let Stmt::Variable(result) = &syntax.actions[0].body.statements[0] else {
        panic!("expected the result declaration");
    };
    let bump_call = result.value.as_ref().unwrap().id;
    let ExprKind::Call { args, .. } = &result.value.as_ref().unwrap().kind else {
        panic!("expected the user-function call");
    };
    let state_path = args[0].id;

    let declarations = database.declarations_named("bump").unwrap();
    assert_eq!(declarations.len(), 1);
    assert_eq!(declarations[0].id, DeclarationId::Function(function.id));

    let local_type = database.value_type(local_id).unwrap().unwrap();
    assert_eq!(
        database.type_kind(local_type).unwrap(),
        Some(TypeKind::Builtin(BuiltinType::I32))
    );
    assert_eq!(
        database
            .expression_type(local.value.as_ref().unwrap().id)
            .unwrap(),
        Some(local_type)
    );
    assert_eq!(database.value_type(parameter).unwrap(), Some(local_type));
    assert_eq!(
        database.function_result_type(function.id).unwrap(),
        Some(local_type)
    );

    assert!(matches!(
        database.resolved_call(bump_call).unwrap(),
        Some(ResolvedCall::UserFunction {
            function: target, ..
        }) if target == function.id
    ));
    assert!(matches!(
        database.resolved_call(min_call).unwrap(),
        Some(ResolvedCall::StandardLibrary {
            receiver: Some(ResolvedReceiver::Path {
                root: ResolvedValue::Variable(target),
                ..
            }),
            ..
        }) if target == global
    ));
    assert!(matches!(
        database.resolved_path(state_path).unwrap(),
        Some(path) if path.root == Some(ResolvedValue::CurrentState(state))
    ));
    assert_eq!(
        database.assignment_target(assignment).unwrap(),
        Some(global)
    );

    let references = database.reference_index().unwrap();
    assert!(Arc::ptr_eq(
        &references,
        &database.reference_index().unwrap()
    ));
    assert_eq!(
        references
            .references_to(global)
            .iter()
            .map(|reference| reference.kind)
            .collect::<Vec<_>>(),
        [ValueReferenceKind::Read, ValueReferenceKind::Write]
    );
    assert_eq!(references.references_to(parameter).len(), 1);
    assert_eq!(references.references_to(local_id).len(), 2);
    assert_eq!(references.references_to(state).len(), 1);
    assert_eq!(references.references_to(result.id).len(), 1);

    let min_position = source.find("global.min").unwrap() + 1;
    let min_analysis = database.analysis_at(min_position).unwrap().unwrap();
    assert_eq!(min_analysis.expression, min_call);
    assert_eq!(min_analysis.type_kind, TypeKind::Builtin(BuiltinType::I32));
    assert_eq!(
        min_analysis
            .segments
            .iter()
            .map(|segment| (
                &segment.name[..],
                &source[segment.span.start..segment.span.end]
            ))
            .collect::<Vec<_>>(),
        [("global", "global"), ("min", "min")]
    );
    let min_method_position = source.find("global.min").unwrap() + "global.".len();
    assert_eq!(
        min_analysis.segment_at(min_method_position).unwrap().name,
        "min"
    );
    assert!(matches!(
        database.definition_at(min_position).unwrap(),
        Some(DefinitionTarget::Source(definition))
            if definition.id == SourceDefinitionId::Value(global)
    ));
    assert_eq!(
        database.definition_at(min_method_position).unwrap(),
        Some(DefinitionTarget::StandardLibrary(StdlibItemId::NumericMin))
    );
    assert!(matches!(
        min_analysis.resolution,
        Some(ExpressionResolution::Call(ResolvedCall::StandardLibrary {
            receiver: Some(ResolvedReceiver::Path {
                root: ResolvedValue::Variable(target),
                ..
            }),
            ..
        })) if target == global
    ));

    let state_position = source.find("current.level").unwrap() + 2;
    let state_analysis = database.analysis_at(state_position).unwrap().unwrap();
    assert_eq!(state_analysis.expression, state_path);
    assert_eq!(
        state_analysis
            .segments
            .iter()
            .map(|segment| segment.name.as_str())
            .collect::<Vec<_>>(),
        ["current", "level"]
    );
    assert!(matches!(
        state_analysis.resolution,
        Some(ExpressionResolution::ValuePath {
            root: Some(ResolvedValue::CurrentState(target)),
            ..
        }) if target == state
    ));
    assert_eq!(
        database.definition_at(state_position).unwrap(),
        Some(DefinitionTarget::Source(
            database
                .definition_index()
                .unwrap()
                .get(SourceDefinitionId::State)
                .unwrap()
                .clone()
        ))
    );
    let state_field_position = source.find("current.level").unwrap() + "current.".len();
    assert!(matches!(
        database.definition_at(state_field_position).unwrap(),
        Some(DefinitionTarget::Source(definition))
            if definition.id == SourceDefinitionId::Value(state)
    ));

    let literal_position = source.find("min(10)").unwrap() + "min(".len();
    let literal_analysis = database.analysis_at(literal_position).unwrap().unwrap();
    assert_eq!(
        literal_analysis.type_kind,
        TypeKind::Builtin(BuiltinType::I32)
    );
    assert!(literal_analysis.resolution.is_none());
    assert_eq!(
        &source[literal_analysis.span.start..literal_analysis.span.end],
        "10"
    );
    assert!(database.analysis_at(source.len() + 1).unwrap().is_none());

    assert_eq!(
        database
            .document_highlights_at(min_position)
            .unwrap()
            .into_iter()
            .map(|highlight| highlight.kind)
            .collect::<Vec<_>>(),
        [
            DocumentHighlightKind::Text,
            DocumentHighlightKind::Read,
            DocumentHighlightKind::Write,
        ]
    );
    assert!(matches!(
        database.type_definition_at(literal_position).unwrap(),
        Some(DefinitionTarget::Language(_))
    ));

    let global_token = database.token_at(min_position).unwrap().unwrap();
    assert_eq!(
        &source[global_token.span.start..global_token.span.end],
        "global"
    );
    let trivia_position = source.find("global.min").unwrap() - 1;
    assert!(database.token_at(trivia_position).unwrap().is_none());
}

#[test]
fn source_reference_queries_cover_all_declaration_kinds() {
    use splitscript::tooling::database::CompilerDatabase;

    fn spellings(database: &mut CompilerDatabase, source: &str, offset: usize) -> Vec<String> {
        database
            .references_at(offset, true)
            .unwrap()
            .into_iter()
            .map(|span| source[span.start..span.end].to_owned())
            .collect()
    }

    let source = r#"
        struct Point { x: i32 }
        enum Mode { Active }
        let total = 0
        state "game.exe" {
            point: Point = process.read(0)
        }
        settings { "General" { "Enabled" => enabled: true } }
        fn observe(point: Point, mode: Mode) {
            total += point.x
            if mode == Mode.Active && settings.enabled {
                print(total as String)
            }
        }
        whileAttached {
            let point = Point { x: 1 }
            observe(point, Mode.Active)
            if current.point.x == 1 {}
        }
    "#;
    let mut database = CompilerDatabase::new(source);
    database.check().expect("navigation fixture should check");

    for (needle, expected, count) in [
        ("struct Point", "Point", 4),
        ("x: i32", "x", 4),
        ("enum Mode", "Mode", 4),
        ("Active }", "Active", 3),
        ("let total", "total", 3),
        ("=> enabled", "enabled", 2),
        ("fn observe", "observe", 2),
        ("current.point", "point", 2),
    ] {
        let offset = source.find(needle).unwrap() + needle.rfind(expected).unwrap();
        let references = spellings(&mut database, source, offset);
        assert_eq!(
            references,
            vec![expected; count],
            "references for {expected}"
        );
    }

    let parameter = source.find("point.x").unwrap();
    assert_eq!(
        spellings(&mut database, source, parameter),
        ["point", "point"]
    );
    let local = source.find("observe(point").unwrap() + "observe(".len();
    assert_eq!(spellings(&mut database, source, local), ["point", "point"]);

    let call = source.rfind("observe").unwrap();
    assert_eq!(
        database.references_at(call, false).unwrap().len(),
        1,
        "excluding the declaration leaves only the call"
    );
}

#[test]
fn managed_class_types_use_the_source_symbol_graph() {
    use splitscript::tooling::database::{CompilerDatabase, SourceDefinitionId};

    let source = r#"
        state "game.exe" {}
        image "Assembly-CSharp" {
            class Player {
                f32 health;
            }
            class GameManager {
                static Player player;
                /// The most recently captured point total.
                i32 points;
            }
        }
        fn observe(player: Player.Ref) {}
        fn inspectSnapshot(manager: GameManager) -> i32 { return manager.points }
    "#;
    let mut database = CompilerDatabase::new(source);
    database.check().expect("managed type fixture should check");

    let declaration = source.find("class Player").unwrap() + "class ".len();
    let field_type = source.find("static Player").unwrap() + "static ".len();
    let parameter_type = source.rfind("Player").unwrap();

    for reference in [field_type, parameter_type] {
        assert!(matches!(
            database.definition_at(reference).unwrap(),
            Some(splitscript::tooling::database::DefinitionTarget::Source(definition))
                if matches!(definition.id, SourceDefinitionId::ManagedClass(_))
                    && definition.span.start == declaration
        ));
    }

    let references = database.references_at(declaration, true).unwrap();
    assert_eq!(references.len(), 3);
    assert!(
        references
            .iter()
            .all(|span| &source[span.start..span.end] == "Player")
    );

    let plan = database.rename_at(parameter_type, "PlayerState").unwrap();
    assert_eq!(plan.edits.len(), 4);
    assert!(plan.edits.iter().any(|edit| {
        edit.span.start == declaration + "Player".len()
            && edit.span.start == edit.span.end
            && edit.replacement == " from \"Player\""
    }));

    let field_declaration = source.find("i32 points").unwrap() + "i32 ".len();
    let field_access = source.rfind("manager.points").unwrap() + "manager.".len();
    assert!(matches!(
        database.definition_at(field_access).unwrap(),
        Some(splitscript::tooling::database::DefinitionTarget::Source(definition))
            if matches!(definition.id, SourceDefinitionId::ManagedField(_))
                && definition.span.start == field_declaration
    ));
    assert_eq!(database.references_at(field_access, true).unwrap().len(), 2);
}

#[test]
fn managed_schema_owners_share_source_identity_hover_and_rename() {
    use splitscript::tooling::database::{CompilerDatabase, SourceDefinitionId};

    let source = r#"
        state Unity ["game.exe"] {}
        /// The game's primary managed image.
        image "Assembly-CSharp" {
            /// Gameplay-owned metadata.
            namespace Game {
                /// A live player and its immutable snapshot shape.
                class Player {
                    /// The player's current health.
                    u32 health;
                }
            }
        }
        fn observe(player: Player) -> u32 { return player.health }
    "#;
    let mut database = CompilerDatabase::new(source);
    database
        .check()
        .expect("managed schema fixture should check");

    for (needle, expected_id, documentation) in [
        ("Assembly-CSharp", "image", "game's primary managed image"),
        ("Game {", "namespace", "Gameplay-owned metadata"),
        ("Player {", "class", "live player"),
        ("health;", "field", "current health"),
    ] {
        let offset = source.find(needle).expect("fixture spelling exists");
        let definition = database
            .definition_at(offset)
            .unwrap()
            .expect("schema declaration has source identity");
        let splitscript::tooling::database::DefinitionTarget::Source(definition) = definition
        else {
            panic!("managed declarations navigate to source")
        };
        assert!(
            matches!(
                (expected_id, definition.id),
                ("image", SourceDefinitionId::ManagedImage(_))
                    | ("namespace", SourceDefinitionId::ManagedNamespace(_))
                    | ("class", SourceDefinitionId::ManagedClass(_))
                    | ("field", SourceDefinitionId::ManagedField(_))
            ),
            "unexpected identity for {expected_id}: {:?}",
            definition.id
        );
        assert!(
            database
                .hover(offset)
                .unwrap()
                .expect("schema declaration has hover")
                .markdown
                .contains(documentation)
        );
    }

    let class_use = source.rfind("Player").expect("parameter uses the class");
    let class_edits = database.rename_at(class_use, "PlayerState").unwrap();
    assert_eq!(
        class_edits.edits.len(),
        3,
        "class declaration and use are renamed while metadata identity is preserved"
    );
    assert!(class_edits.edits.iter().any(|edit| {
        edit.span.start == source.find("Player {").unwrap() + "Player".len()
            && edit.span.start == edit.span.end
            && edit.replacement == " from \"Player\""
    }));

    let field_use = source.rfind("health").expect("return reads the field");
    let field_edits = database.rename_at(field_use, "hitPoints").unwrap();
    assert_eq!(
        field_edits.edits.len(),
        3,
        "field declaration and use are renamed while metadata identity is preserved"
    );
    assert!(field_edits.edits.iter().any(|edit| {
        edit.span.start == source.find("health;").unwrap() + "health".len()
            && edit.span.start == edit.span.end
            && edit.replacement == " from [\"health\", \"<health>k__BackingField\"]"
    }));
}

#[test]
fn managed_rename_keeps_explicit_metadata_names_unchanged() {
    use splitscript::tooling::database::CompilerDatabase;

    let source = r#"
        image "Assembly-CSharp" {
            class Player from "Game.Player" {
                static Player instance from "Instance";
                u32 score from ["_score", "<Score>k__BackingField"];
            }
        }
        state Unity ["game.exe"] {
            score: u32 = Player.instance?.score?
        }
    "#;
    let mut database = CompilerDatabase::new(source);
    database
        .check()
        .expect("managed rename fixture should check");

    let class_use = source.rfind("Player.instance").unwrap();
    let class_plan = database.rename_at(class_use, "Actor").unwrap();
    assert_eq!(class_plan.edits.len(), 3);
    assert!(
        class_plan
            .edits
            .iter()
            .all(|edit| edit.span.start != edit.span.end)
    );

    let field_use = source.rfind("score?").unwrap();
    let field_plan = database.rename_at(field_use, "points").unwrap();
    assert_eq!(field_plan.edits.len(), 2);
    assert!(
        field_plan
            .edits
            .iter()
            .all(|edit| edit.span.start != edit.span.end)
    );
}

#[test]
fn setting_rename_preserves_the_host_key() {
    use splitscript::tooling::database::CompilerDatabase;

    let source = r#"
        state "game.exe" {}
        settings {
            "Automatic splitting" => autoSplit: true,
            "Automatic reset" => autoReset key "reset-enabled": true,
        }
        whileAttached {
            print(settings.autoSplit)
            print(settings.autoReset)
        }
    "#;
    let mut database = CompilerDatabase::new(source);
    database
        .check()
        .expect("setting rename fixture should check");

    let auto_split_use = source.rfind("autoSplit").unwrap();
    let auto_split = database.rename_at(auto_split_use, "splitEnabled").unwrap();
    assert_eq!(auto_split.edits.len(), 3);
    assert!(auto_split.edits.iter().any(|edit| {
        edit.span.start == source.find("autoSplit:").unwrap() + "autoSplit".len()
            && edit.span.start == edit.span.end
            && edit.replacement == " key \"autoSplit\""
    }));

    let auto_reset_use = source.rfind("autoReset").unwrap();
    let auto_reset = database.rename_at(auto_reset_use, "resetEnabled").unwrap();
    assert_eq!(auto_reset.edits.len(), 2);
    assert!(
        auto_reset
            .edits
            .iter()
            .all(|edit| edit.span.start != edit.span.end)
    );
}

#[test]
fn live_managed_paths_use_schema_definitions_and_documentation() {
    use splitscript::tooling::database::{CompilerDatabase, SourceDefinitionId};

    let source = r#"
        image "Assembly-CSharp" {
            class Player {
                /// The current point total.
                u32 score;
            }
            class GameManager {
                /// The active manager singleton.
                static GameManager instance;
                /// The active player object.
                Player player;
            }
        }
        state Unity ["game.exe"] {
            score: u32 = GameManager.instance?.player?.score?
        }
    "#;
    let mut database = CompilerDatabase::new(source);
    database
        .check()
        .expect("live managed path fixture should check");

    let expression = source.rfind("GameManager.instance").unwrap();
    assert!(matches!(
        database.definition_at(expression).unwrap(),
        Some(splitscript::tooling::database::DefinitionTarget::Source(definition))
            if matches!(definition.id, SourceDefinitionId::ManagedClass(_))
    ));
    for (name, expected) in [
        ("instance", "active manager singleton"),
        ("player", "active player object"),
        ("score", "current point total"),
    ] {
        let offset = source.rfind(name).unwrap();
        assert!(matches!(
            database.definition_at(offset).unwrap(),
            Some(splitscript::tooling::database::DefinitionTarget::Source(definition))
                if matches!(definition.id, SourceDefinitionId::ManagedField(_))
        ));
        assert!(
            database
                .hover(offset)
                .unwrap()
                .expect("managed field should have hover information")
                .markdown
                .contains(expected)
        );
    }
}

#[test]
fn managed_schema_metadata_aliases_reject_ambiguous_field_bindings() {
    let source = r#"
        image "Assembly-CSharp" {
            class GameManager {
                i32 points from "score";
                i32 resets from ["deaths", "score"];
            }
        }
        state "game.exe" {}
    "#;

    let diagnostics = splitscript::compile(source).expect_err("ambiguous metadata must fail");
    let diagnostic = diagnostics
        .iter()
        .find(|diagnostic| {
            diagnostic
                .message
                .contains("claimed by both `points` and `resets`")
        })
        .expect("the collision should identify both source fields");
    assert_eq!(diagnostic.labels.len(), 2);
    assert!(
        diagnostic
            .labels
            .iter()
            .any(|label| matches!(label.style, splitscript::DiagnosticLabelStyle::Primary))
    );
    assert!(
        diagnostic
            .labels
            .iter()
            .any(|label| matches!(label.style, splitscript::DiagnosticLabelStyle::Secondary))
    );

    let backing_collision = r#"
        image "Assembly-CSharp" {
            class GameManager {
                i32 points;
                i32 pointsStorage from "<points>k__BackingField";
            }
        }
        state "game.exe" {}
    "#;
    let diagnostics = splitscript::compile(backing_collision)
        .expect_err("implicit backing-field lookup must participate in ambiguity checking");
    assert!(diagnostics.iter().any(|diagnostic| {
        diagnostic
            .message
            .contains("claimed by both `points` and `pointsStorage`")
            && diagnostic.message.contains("<points>k__BackingField")
    }));
}

#[test]
fn rename_queries_validate_identifiers_reservations_and_binding_identity() {
    use splitscript::tooling::database::{CompilerDatabase, RenameError};

    let source = r#"
        let total = 1
        state "game.exe" {}
        fn show(value: i32) {
            print(total as String)
            print(value as String)
        }
        whileAttached { show(total) }
    "#;
    let mut database = CompilerDatabase::new(source);

    let parameter = source.find("value: i32").unwrap();
    let plan = database.rename_at(parameter, "amount").unwrap();
    assert_eq!(plan.edits.len(), 2);
    assert!(
        plan.edits
            .iter()
            .all(|edit| &source[edit.span.start..edit.span.end] == "value")
    );
    let target = database.rename_target_at(parameter).unwrap().unwrap();
    assert_eq!(target.name, "value");
    assert_eq!(&source[target.span.start..target.span.end], "value");

    assert!(matches!(
        database.rename_at(parameter, "2amount"),
        Err(RenameError::InvalidIdentifier)
    ));
    assert!(matches!(
        database.rename_at(parameter, "$amount"),
        Err(RenameError::InvalidIdentifier)
    ));
    assert!(matches!(
        database.rename_at(parameter, "if"),
        Err(RenameError::ReservedIdentifier)
    ));
    assert!(matches!(
        database.rename_at(parameter, "total"),
        Err(RenameError::ConflictingBinding)
    ));

    let print = source.find("print").unwrap();
    assert!(database.rename_target_at(print).unwrap().is_none());
    assert!(matches!(
        database.rename_at(print, "write"),
        Err(RenameError::NotRenameable)
    ));
}

#[test]
fn struct_shorthand_renames_expand_to_preserve_field_and_value_identity() {
    use splitscript::tooling::database::CompilerDatabase;

    let source = r#"
        struct Point { x: u32 }
        state "game.exe" {}
        fn point(x: u32) -> Point { return Point { x } }
    "#;

    let field_declaration = source.find("x: u32 }").unwrap();
    let local_declaration = source.rfind("x: u32").unwrap();
    let shorthand = source.rfind("{ x }").unwrap() + 2;

    let mut field_database = CompilerDatabase::new(source);
    let field_plan = field_database
        .rename_at(field_declaration, "horizontal")
        .expect("renaming a field should expand its shorthand references");
    assert!(
        field_plan
            .edits
            .iter()
            .any(|edit| { edit.span.start == shorthand && edit.replacement == "horizontal: x" }),
        "{field_plan:#?}"
    );

    let mut local_database = CompilerDatabase::new(source);
    let local_plan = local_database
        .rename_at(local_declaration, "coordinate")
        .expect("renaming a local should expand its shorthand references");
    assert!(
        local_plan
            .edits
            .iter()
            .any(|edit| { edit.span.start == shorthand && edit.replacement == "x: coordinate" }),
        "{local_plan:#?}"
    );

    let mut shorthand_database = CompilerDatabase::new(source);
    let shorthand_target = shorthand_database
        .rename_target_at(shorthand)
        .unwrap()
        .expect("the shorthand token should remain renameable");
    assert_eq!(shorthand_target.name, "x");
}

#[test]
fn struct_shorthand_tooling_uses_operation_specific_identity() {
    use splitscript::tooling::database::{CompilerDatabase, DefinitionTarget, SourceDefinitionId};

    let source = r#"
        struct Point { x: u32 }
        state "game.exe" {}
        fn point(value: u32) -> Point {
            let x = value
            return Point { x }
        }
    "#;
    let shorthand = source.rfind("{ x }").unwrap() + 2;

    let mut database = CompilerDatabase::new(source);
    assert!(matches!(
        database.definition_at(shorthand).unwrap(),
        Some(DefinitionTarget::Source(definition))
            if matches!(definition.id, SourceDefinitionId::StructField(_))
    ));

    let hover = database
        .hover(shorthand)
        .unwrap()
        .expect("a shorthand field should have hover information");
    assert!(
        hover.markdown.contains("Point.x: u32"),
        "{}",
        hover.markdown
    );
    assert!(
        hover
            .markdown
            .contains("**Value represented by the shorthand**"),
        "{}",
        hover.markdown
    );
    assert!(hover.markdown.contains("let x: u32"), "{}", hover.markdown);
    assert!(
        hover.markdown.contains("Local variable"),
        "{}",
        hover.markdown
    );

    let rename = database
        .rename_at(shorthand, "coordinate")
        .expect("rename on a shorthand token should select its local value");
    assert!(
        rename
            .edits
            .iter()
            .any(|edit| { edit.span.start == shorthand && edit.replacement == "x: coordinate" }),
        "{rename:#?}"
    );
    assert!(
        !rename
            .edits
            .iter()
            .any(|edit| { edit.span.start == shorthand && edit.replacement == "coordinate: x" })
    );
}

#[test]
fn struct_pattern_tooling_preserves_field_and_binding_identities() {
    use splitscript::tooling::database::{CompilerDatabase, DefinitionTarget, SourceDefinitionId};

    let source = r#"
        struct Point { x: u32, y: u32 }
        state "game.exe" {}
        fn observe(point: Point) -> u32 {
            return match point {
                Point { x, y: 4 } => x,
                _ => 0,
            }
        }
    "#;
    let pattern = source.rfind("Point { x, y: 4 }").unwrap();
    let shorthand = pattern + "Point { ".len();
    let explicit_field = pattern + "Point { x, ".len();
    let field_declaration = source.find("x: u32").unwrap();

    let mut database = CompilerDatabase::new(source);
    assert!(matches!(
        database.definition_at(pattern).unwrap(),
        Some(DefinitionTarget::Source(definition))
            if matches!(definition.id, SourceDefinitionId::Struct(_))
    ));
    assert!(matches!(
        database.definition_at(shorthand).unwrap(),
        Some(DefinitionTarget::Source(definition))
            if matches!(definition.id, SourceDefinitionId::StructField(_))
    ));
    assert!(matches!(
        database.definition_at(explicit_field).unwrap(),
        Some(DefinitionTarget::Source(definition))
            if matches!(definition.id, SourceDefinitionId::StructField(_))
    ));

    let hover = database
        .hover(shorthand)
        .unwrap()
        .expect("a shorthand pattern field should expose both identities");
    assert!(
        hover.markdown.contains("Point.x: u32"),
        "{}",
        hover.markdown
    );
    assert!(
        hover
            .markdown
            .contains("**Value represented by the shorthand**"),
        "{}",
        hover.markdown
    );
    assert!(hover.markdown.contains("x: u32"), "{}", hover.markdown);

    let rename_binding = database
        .rename_at(shorthand, "coordinate")
        .expect("renaming at a pattern shorthand should select its binding");
    assert!(
        rename_binding
            .edits
            .iter()
            .any(|edit| { edit.span.start == shorthand && edit.replacement == "x: coordinate" }),
        "{rename_binding:#?}"
    );

    let mut database = CompilerDatabase::new(source);
    let rename_field = database
        .rename_at(field_declaration, "horizontal")
        .expect("renaming a field should expand pattern shorthand safely");
    assert!(
        rename_field
            .edits
            .iter()
            .any(|edit| { edit.span.start == shorthand && edit.replacement == "horizontal: x" }),
        "{rename_field:#?}"
    );
}

#[test]
fn anonymous_struct_pattern_tooling_resolves_contextual_field_identities() {
    use splitscript::tooling::database::{CompilerDatabase, DefinitionTarget, SourceDefinitionId};

    let source = r#"
        struct Point { x: u32, y: u32 }
        state "game.exe" {}
        fn observe({ x, y: _ }: Point) -> u32 {
            return x
        }
    "#;
    let pattern = source.rfind("{ x, y: _ }").unwrap();
    let shorthand = pattern + "{ ".len();
    let explicit_field = pattern + "{ x, ".len();
    let field_declaration = source.find("x: u32").unwrap();

    let mut database = CompilerDatabase::new(source);
    assert!(matches!(
        database.definition_at(shorthand).unwrap(),
        Some(DefinitionTarget::Source(definition))
            if matches!(definition.id, SourceDefinitionId::StructField(_))
    ));
    assert!(matches!(
        database.definition_at(explicit_field).unwrap(),
        Some(DefinitionTarget::Source(definition))
            if matches!(definition.id, SourceDefinitionId::StructField(_))
    ));

    let hover = database
        .hover(shorthand)
        .unwrap()
        .expect("contextual shorthand should expose both identities");
    assert!(
        hover.markdown.contains("Point.x: u32"),
        "{}",
        hover.markdown
    );
    assert!(hover.markdown.contains("x: u32"), "{}", hover.markdown);

    let rename_binding = database
        .rename_at(shorthand, "coordinate")
        .expect("anonymous shorthand retains its binding identity");
    assert!(
        rename_binding
            .edits
            .iter()
            .any(|edit| { edit.span.start == shorthand && edit.replacement == "x: coordinate" })
    );

    let mut database = CompilerDatabase::new(source);
    let rename_field = database
        .rename_at(field_declaration, "horizontal")
        .expect("anonymous shorthand retains its field identity");
    assert!(
        rename_field
            .edits
            .iter()
            .any(|edit| { edit.span.start == shorthand && edit.replacement == "horizontal: x" })
    );
}

#[test]
fn binding_pattern_shorthand_renames_expand_at_every_declaration_site() {
    use splitscript::tooling::database::CompilerDatabase;

    let source = r#"
        struct Pos { x: u16, y: u16 }
        state "game.exe" {}
        fn parameter(Pos { x, y }: Pos) -> u16 { return x + y }
        fn local(pos: Pos) -> u16 {
            let Pos { x, y } = pos
            return x + y
        }
        fn iterate() {
            for Pos { x, y } in [Pos { x: 1, y: 2 }] {
                print(x + y)
            }
        }
        fn closure(value: Pos) -> u16 {
            let sum = (Pos { x, y }: Pos) => x + y
            return sum(value)
        }
        fn literal(x: u16, y: u16) -> Pos { return Pos { x, y } }
    "#;
    let field_declaration = source.find("x: u16").unwrap();
    let parameter_shorthand =
        source.find("fn parameter(Pos { x, y }").unwrap() + "fn parameter(Pos { ".len();
    let local_pattern = source.find("let Pos { x, y }").unwrap() + "let Pos { ".len();
    let for_pattern = source.find("for Pos { x, y }").unwrap() + "for Pos { ".len();
    let closure_pattern = source.find("(Pos { x, y }: Pos) =>").unwrap() + "(Pos { ".len();
    let literal_shorthand = source.find("return Pos { x, y }").unwrap() + "return Pos { ".len();

    let mut database = CompilerDatabase::new(source);
    let rename = database
        .rename_at(field_declaration, "horizontal")
        .expect("renaming a struct field should expand every binding shorthand");
    for shorthand in [
        parameter_shorthand,
        local_pattern,
        for_pattern,
        closure_pattern,
        literal_shorthand,
    ] {
        assert!(
            rename
                .edits
                .iter()
                .any(|edit| edit.span.start == shorthand && edit.replacement == "horizontal: x"),
            "missing shorthand expansion at {shorthand}: {rename:#?}"
        );
    }

    let mut database = CompilerDatabase::new(source);
    let rename = database
        .rename_at(parameter_shorthand, "horizontal")
        .expect("renaming the parameter binding should split its shorthand");
    assert!(rename.edits.iter().any(|edit| {
        edit.span.start == parameter_shorthand && edit.replacement == "x: horizontal"
    }));
}

#[test]
fn unrelated_renames_preserve_both_identities_of_unchanged_struct_shorthand() {
    use splitscript::tooling::database::CompilerDatabase;

    let source = r#"
        struct Point { x: u32, y: u32 }
        state "game.exe" {}
        fn point(x: u32) -> Point { return Point { x, y: 0 } }
        fn observe(point: Point) { print(point.x) }
        whileAttached { observe(point(1)) }
    "#;
    let y_declaration = source.find("y: u32").unwrap();

    let mut rename_database = CompilerDatabase::new(source);
    let rename = rename_database
        .rename_at(y_declaration, "vertical")
        .expect("an unrelated shorthand must retain both of its identities");
    assert_eq!(rename.new_name, "vertical");
    assert_eq!(rename.edits.len(), 2);

    let mut suppression_database = CompilerDatabase::new(source);
    let suppression = suppression_database
        .underscore_suppression_at(y_declaration)
        .unwrap()
        .expect("unused-field suppression must terminate with a validated rename");
    assert_eq!(suppression.new_name, "_y");
    assert_eq!(suppression.edits.len(), 2);
}

#[test]
fn semantic_queries_use_exact_tokens_before_end_of_word_fallbacks() {
    use splitscript::tooling::database::{CompilerDatabase, DefinitionTarget};

    let source = concat!(
        "state \"game.exe\" {}\n",
        "fn observe(value: i32) { print(value) }\n",
        "whileAttached {\n",
        "    observe(1)\n",
        "    observe (2)\n",
        "}\n",
    );
    let mut database = CompilerDatabase::new(source);
    database.check().expect("navigation fixture should check");

    let declaration = source.find("observe").unwrap();
    let adjacent = source.find("observe(1)").unwrap();
    let adjacent_boundary = adjacent + "observe".len();

    let hover = database
        .hover(adjacent_boundary)
        .unwrap()
        .expect("call punctuation should expose the enclosing expression type");
    assert_eq!(hover.markdown, "```splitscript\nNone\n```");
    assert_eq!(&source[hover.span.start..hover.span.end], "observe(1)");
    assert!(matches!(
        database.definition_at(adjacent_boundary).unwrap(),
        Some(DefinitionTarget::Source(definition)) if definition.span.start == declaration
    ));
    assert_eq!(
        database
            .references_at(adjacent_boundary, true)
            .unwrap()
            .len(),
        3
    );
    assert!(
        database
            .rename_target_at(adjacent_boundary)
            .unwrap()
            .is_some()
    );

    let inside = adjacent_boundary - 1;
    assert!(database.hover(inside).unwrap().is_some());
    assert!(matches!(
        database.definition_at(inside).unwrap(),
        Some(DefinitionTarget::Source(definition)) if definition.span.start == declaration
    ));
    assert_eq!(database.references_at(inside, true).unwrap().len(), 3);
    assert!(database.rename_target_at(inside).unwrap().is_some());

    let gap_boundary = source.rfind("observe").unwrap() + "observe".len();
    assert!(database.hover(gap_boundary).unwrap().is_some());
    assert!(matches!(
        database.definition_at(gap_boundary).unwrap(),
        Some(DefinitionTarget::Source(definition)) if definition.span.start == declaration
    ));
    assert_eq!(database.references_at(gap_boundary, true).unwrap().len(), 3);
    assert!(database.rename_target_at(gap_boundary).unwrap().is_some());

    let opening_parenthesis = gap_boundary + 1;
    let hover = database
        .hover(opening_parenthesis)
        .unwrap()
        .expect("separated call punctuation should expose the expression type");
    assert_eq!(hover.markdown, "```splitscript\nNone\n```");
    assert_eq!(&source[hover.span.start..hover.span.end], "observe (2)");
    assert!(
        database
            .definition_at(opening_parenthesis)
            .unwrap()
            .is_none()
    );
    assert!(
        database
            .rename_target_at(opening_parenthesis)
            .unwrap()
            .is_none()
    );
}

#[test]
fn underscore_suppression_reuses_validated_identity_renames() {
    use splitscript::tooling::database::CompilerDatabase;

    let source = r#"
        struct Snapshot {
            used: i32,
            unusedField: i32
        }

        let deadGlobal = 1
        let _deadGlobal = 2
        state "game.exe" {}

        fn deadHelper() {
            print(deadGlobal)
        }

        fn snapshot() -> Snapshot {
            return Snapshot { used: 1, unusedField: 2 }
        }

        whileAttached {
            snapshot().used
        }
    "#;
    let mut database = CompilerDatabase::new(source);
    database.check().expect("warnings do not reject the source");

    let global_offset = source.find("deadGlobal =").unwrap();
    let global = database
        .underscore_suppression_at(global_offset)
        .unwrap()
        .expect("the unused global is renameable");
    assert_eq!(global.new_name, "__deadGlobal");
    assert_eq!(global.edits.len(), 2);
    assert!(
        global
            .edits
            .iter()
            .all(|edit| &source[edit.span.start..edit.span.end] == "deadGlobal")
    );

    let field_offset = source.find("unusedField: i32").unwrap();
    let field = database
        .underscore_suppression_at(field_offset)
        .unwrap()
        .expect("the unread field is renameable");
    assert_eq!(field.new_name, "_unusedField");
    assert_eq!(field.edits.len(), 2);
    assert!(
        field
            .edits
            .iter()
            .all(|edit| &source[edit.span.start..edit.span.end] == "unusedField")
    );
}

#[test]
fn document_symbols_preserve_source_order_and_domain_hierarchy() {
    use std::sync::Arc;

    use splitscript::{tooling::database::CompilerDatabase, tooling::symbols::DocumentSymbolKind};

    let source = r#"
        struct Point { x: i32 }
        let global = 1
        state "game.exe" { level = process.read<i32>(0) }
        settings {
            "General" {
                "Timing" {
                    "Enabled" => enabled: true
                }
            }
        }
        enum Mode { Active }
        fn Point.isOrigin() { return self.x == 0 }
        whileAttached {}
    "#;
    let mut database = CompilerDatabase::new(source);
    let symbols = database.document_symbols().unwrap();
    assert!(Arc::ptr_eq(&symbols, &database.document_symbols().unwrap()));
    assert_eq!(
        symbols
            .iter()
            .map(|symbol| symbol.name.as_str())
            .collect::<Vec<_>>(),
        [
            "Point",
            "global",
            "state",
            "settings",
            "Mode",
            "isOrigin",
            "whileAttached"
        ]
    );

    let structure = &symbols[0];
    assert_eq!(structure.kind, DocumentSymbolKind::Struct);
    assert_eq!(structure.children[0].name, "x");
    assert_eq!(structure.children[0].kind, DocumentSymbolKind::Field);

    let state = &symbols[2];
    assert_eq!(state.kind, DocumentSymbolKind::Namespace);
    assert_eq!(state.children[0].name, "level");

    let settings = &symbols[3];
    assert_eq!(settings.children[0].name, "General");
    assert_eq!(settings.children[0].children[0].name, "Timing");
    assert_eq!(settings.children[0].children[0].children[0].name, "enabled");
    assert_eq!(
        settings.children[0].children[0].children[0].kind,
        DocumentSymbolKind::Property
    );

    assert_eq!(symbols[5].kind, DocumentSymbolKind::Method);
    assert_eq!(symbols[6].kind, DocumentSymbolKind::Event);
    for symbol in symbols.iter() {
        assert!(symbol.range.start <= symbol.selection_range.start);
        assert!(symbol.selection_range.end <= symbol.range.end);
    }
}

#[test]
fn document_symbols_expose_each_destructured_global_binding() {
    use splitscript::tooling::database::CompilerDatabase;

    let source = r#"
        struct Point { x: i32, y: i32 }
        let Point { x: globalX, y: globalY } = Point { x: 1, y: 2 }
        state "game.exe" {}
    "#;
    let mut database = CompilerDatabase::new(source);
    let symbols = database.document_symbols().unwrap();
    let names = symbols
        .iter()
        .map(|symbol| symbol.name.as_str())
        .collect::<Vec<_>>();

    assert_eq!(names, ["Point", "globalX", "globalY", "state"]);

    let lowered = database.lower().unwrap();
    let declarations = lowered
        .hir()
        .declarations()
        .filter_map(|declaration| {
            matches!(
                declaration.id,
                splitscript::compiler::hir::DeclarationId::Global(_)
            )
            .then_some(declaration.name.as_str())
        })
        .collect::<Vec<_>>();
    assert_eq!(declarations, ["globalX", "globalY"]);
}

#[test]
fn compiler_database_preserves_semantics_around_type_errors() {
    use std::sync::Arc;

    use splitscript::{
        DiagnosticCode,
        compiler::ast::Stmt,
        compiler::hir::ExpressionResolution,
        compiler::semantic::ResolvedCall,
        compiler::types::{BuiltinType, TypeKind},
        tooling::database::{CompilerDatabase, DefinitionTarget, SourceDefinitionId},
    };

    let source = r#"
        struct Counter { value: i32 }
        state "game.exe" {}

        fn readCounter(counter: Counter) -> i32 {
            return counter.value
        }

        whileAttached {
            let counter = Counter { value: 7 }
            let answer = readCounter(counter)
            let broken: bool = 42
            print(answer as String)
        }
    "#;
    let mut database = CompilerDatabase::new(source);

    let strict_errors = database.check().unwrap_err();
    assert_eq!(strict_errors.len(), 1);
    assert_eq!(strict_errors[0].code, DiagnosticCode::Type);

    let recovered = database.recovering_check().unwrap();
    assert!(Arc::ptr_eq(
        &recovered,
        &database.recovering_check().unwrap()
    ));
    assert_eq!(recovered.diagnostics().len(), 1);

    let function = recovered.syntax().functions[0].id;
    let Stmt::Variable(answer) = &recovered.syntax().actions[0].body.statements[1] else {
        panic!("expected the unaffected answer declaration");
    };
    let answer_type = database.value_type(answer.id).unwrap().unwrap();
    assert_eq!(
        database.type_kind(answer_type).unwrap(),
        Some(TypeKind::Builtin(BuiltinType::I32))
    );
    assert!(matches!(
        database.resolved_call(answer.value.as_ref().unwrap().id).unwrap(),
        Some(ResolvedCall::UserFunction {
            function: target, ..
        }) if target == function
    ));

    let call_position = source.rfind("readCounter").unwrap() + 1;
    let analysis = database.analysis_at(call_position).unwrap().unwrap();
    assert_eq!(analysis.type_kind, TypeKind::Builtin(BuiltinType::I32));
    assert!(matches!(
        analysis.resolution,
        Some(ExpressionResolution::Call(ResolvedCall::UserFunction {
            function: target,
            ..
        })) if target == function
    ));
    assert!(matches!(
        database.definition_at(call_position).unwrap(),
        Some(DefinitionTarget::Source(definition))
            if definition.id == SourceDefinitionId::Function(function)
    ));

    let argument_position = source.rfind("counter)").unwrap() + 1;
    let Stmt::Variable(counter) = &recovered.syntax().actions[0].body.statements[0] else {
        panic!("expected the counter declaration");
    };
    assert!(matches!(
        database.definition_at(argument_position).unwrap(),
        Some(DefinitionTarget::Source(definition))
            if definition.id == SourceDefinitionId::Value(counter.id)
    ));
}

#[test]
fn compiler_database_shares_lowering_before_and_after_semantic_errors() {
    use std::sync::Arc;

    use splitscript::tooling::database::CompilerDatabase;

    let base = "// Unicode 🦊\nstate \"game.exe\" { level: u32 at 0x100 }\n";
    for (suffix, valid) in [
        ("whileAttached { print(current.level) }", true),
        ("fn broken() { return missingName }", false),
        ("onDetach { print(current.level) }", false),
        ("fn broken(value: Mystery) {}", false),
    ] {
        let source = format!("{base}{suffix}");
        for recovery_first in [false, true] {
            let mut database = CompilerDatabase::with_source_name("sharing.split", &source);
            let first = if recovery_first {
                database.recovering_lower().unwrap()
            } else {
                database.lower().unwrap()
            };
            assert_eq!(first.source_name(), "sharing.split");
            assert!(Arc::ptr_eq(&first, &database.lower().unwrap()));
            assert!(Arc::ptr_eq(&first, &database.recovering_lower().unwrap()));
            assert_eq!(database.check().is_ok(), valid, "{suffix}");
            let recovered = database.recovering_check().unwrap();
            assert!(Arc::ptr_eq(&first, &database.recovering_lower().unwrap()));
            let direct = splitscript::check_recovering((*first).clone());
            assert_eq!(
                format!("{:?}", recovered.diagnostics()),
                format!("{:?}", direct.diagnostics()),
                "sharing must preserve diagnostic content, order, and offsets"
            );
        }
    }
}

#[test]
fn shared_lowering_survives_warning_policy_changes_but_not_source_edits() {
    use std::sync::Arc;

    use splitscript::{
        DiagnosticCode, DiagnosticSeverity, WarningLevel, WarningPolicy,
        tooling::database::CompilerDatabase,
    };

    let source = "state \"game.exe\" {} whileAttached { let unread = 1 }";
    let mut database = CompilerDatabase::new(source);
    let original = database.recovering_lower().unwrap();
    let mut policy = WarningPolicy::default();
    for level in [WarningLevel::Deny, WarningLevel::Allow] {
        assert!(policy.set(DiagnosticCode::UnusedBinding, level));
        assert!(database.set_warning_policy(policy));
        let diagnostics = database.diagnostics();
        if level == WarningLevel::Deny {
            assert_eq!(diagnostics[0].severity, DiagnosticSeverity::Error);
        } else {
            assert!(diagnostics.is_empty());
        }
        assert!(database.check().is_ok());
        assert!(Arc::ptr_eq(&original, &database.lower().unwrap()));
        assert!(Arc::ptr_eq(
            &original,
            &database.recovering_lower().unwrap()
        ));
    }

    assert!(
        database.set_source("state \"game.exe\" {}\nfn retained() { return 7 }\nlet broken = +")
    );
    let errors = database.lower().unwrap_err();
    let partial = database.recovering_lower().unwrap();
    assert!(!Arc::ptr_eq(&original, &partial));
    assert!(
        partial
            .hir()
            .declarations_named("retained")
            .next()
            .is_some()
    );
    assert!(Arc::ptr_eq(&errors, &database.lower().unwrap_err()));
    assert!(
        database.check().is_err(),
        "recovered syntax must stay out of strict checking"
    );

    assert!(database.set_source(source));
    let repaired = database.lower().unwrap();
    assert!(!Arc::ptr_eq(&original, &repaired));
    assert!(!Arc::ptr_eq(&partial, &repaired));
    assert!(Arc::ptr_eq(
        &repaired,
        &database.recovering_lower().unwrap()
    ));
    assert!(database.check().is_ok());
}

#[test]
fn compiler_database_lowers_recovered_declarations_after_syntax_errors() {
    use std::sync::Arc;

    use splitscript::{compiler::hir::DeclarationId, tooling::database::CompilerDatabase};

    let source = r#"
        let broken = +
        state "game.exe" {}
        fn retained() { return 1 }
        whileAttached { print("retained") }
    "#;
    let mut database = CompilerDatabase::new(source);

    let lowered = database.recovering_lower().unwrap();
    assert!(Arc::ptr_eq(&lowered, &database.recovering_lower().unwrap()));
    let declarations = lowered.hir().declarations().collect::<Vec<_>>();
    assert!(declarations.iter().any(|declaration| {
        declaration.name == "broken" && matches!(declaration.id, DeclarationId::Global(_))
    }));
    assert!(declarations.iter().any(|declaration| {
        declaration.name == "retained" && matches!(declaration.id, DeclarationId::Function(_))
    }));
    assert!(declarations.iter().any(|declaration| {
        declaration.name == "whileAttached" && matches!(declaration.id, DeclarationId::Action(_))
    }));

    let strict_errors = database.lower().unwrap_err();
    assert_eq!(strict_errors.len(), 1);
    assert_eq!(lowered.syntax().globals.len(), 1);
    assert_eq!(lowered.syntax().functions.len(), 1);
    assert_eq!(lowered.syntax().actions.len(), 1);
}

#[test]
fn compiler_database_resolves_expression_segments_to_definitions() {
    use std::sync::Arc;

    use splitscript::{
        compiler::stdlib::StdlibItemId,
        tooling::database::{CompilerDatabase, DefinitionTarget, SourceDefinitionId},
    };

    let source = r#"
        struct Counter { value: i32 }
        enum Mode {
            Idle,
            Active
        }
        let global = 1
        state "game.exe" {}

        fn bump(value) {
            return value.value + global
        }

        whileAttached {
            let counter = Counter { value: 1 }
            let mode = Mode.Active
            let result = bump(counter)
            let special = f32.NaN.isNaN()
            print(result as String)
        }
    "#;
    let mut database = CompilerDatabase::new(source);
    let checked = database.check().unwrap();
    let global = checked.syntax().globals[0].id;
    let function = checked.syntax().functions[0].id;
    let parameter = checked.syntax().functions[0].params[0].id;
    let field = checked.syntax().structs[0].fields[0].id;
    let enumeration = checked.syntax().enums[0].id;
    let active = checked.syntax().enums[0].variants[1].id;

    let definitions = database.definition_index().unwrap();
    assert!(Arc::ptr_eq(
        &definitions,
        &database.definition_index().unwrap()
    ));
    assert_eq!(
        definitions
            .get(SourceDefinitionId::Value(global))
            .map(|definition| &source[definition.span.start..definition.span.end]),
        Some("global")
    );

    let value_path = source.find("value.value").unwrap();
    assert!(matches!(
        database.definition_at(value_path + 1).unwrap(),
        Some(DefinitionTarget::Source(definition))
            if definition.id == SourceDefinitionId::Value(parameter)
    ));
    assert!(matches!(
        database.definition_at(value_path + "value.".len()).unwrap(),
        Some(DefinitionTarget::Source(definition))
            if definition.id == SourceDefinitionId::StructField(field)
    ));

    let global_path = source.find("+ global").unwrap() + 2;
    assert!(matches!(
        database.definition_at(global_path + 1).unwrap(),
        Some(DefinitionTarget::Source(definition))
            if definition.id == SourceDefinitionId::Value(global)
    ));

    let enum_path = source.find("Mode.Active").unwrap();
    assert!(matches!(
        database.definition_at(enum_path + 1).unwrap(),
        Some(DefinitionTarget::Source(definition))
            if definition.id == SourceDefinitionId::Enum(enumeration)
    ));
    assert!(matches!(
        database.definition_at(enum_path + "Mode.".len()).unwrap(),
        Some(DefinitionTarget::Source(definition))
            if definition.id == SourceDefinitionId::EnumVariant(active)
    ));

    let bump_call = source.find("bump(counter)").unwrap();
    assert!(matches!(
        database.definition_at(bump_call + 1).unwrap(),
        Some(DefinitionTarget::Source(definition))
            if definition.id == SourceDefinitionId::Function(function)
    ));
    let print_call = source.find("print(result").unwrap();
    assert_eq!(
        database.definition_at(print_call + 1).unwrap(),
        Some(DefinitionTarget::StandardLibrary(StdlibItemId::Print))
    );
    let constant = source.find("f32.NaN.isNaN()").unwrap();
    assert_eq!(
        database.definition_at(constant + "f32.".len()).unwrap(),
        Some(DefinitionTarget::StandardLibrary(StdlibItemId::F32NaN))
    );
    assert_eq!(
        database.definition_at(constant + "f32.NaN.".len()).unwrap(),
        Some(DefinitionTarget::StandardLibrary(StdlibItemId::FloatIsNaN))
    );
    assert_eq!(
        database
            .definition_at(source.find("whileAttached").unwrap())
            .unwrap(),
        Some(DefinitionTarget::Language(
            splitscript::tooling::language::LanguageItemId::WhileAttached
        ))
    );
}

#[test]
fn postfix_expression_receivers_preserve_member_and_method_navigation() {
    use splitscript::tooling::database::{CompilerDatabase, DefinitionTarget, SourceDefinitionId};

    let source = r#"
        struct Counter { value: i32 }
        struct Wrapper { counter: Counter }
        state "game.exe" {}

        fn wrapper() -> Wrapper {
            return Wrapper { counter: Counter { value: 41 } }
        }

        fn Counter.increment() -> i32 {
            return self.value + 1
        }

        whileAttached {
            print(wrapper().counter.increment() as String)
        }
    "#;
    let mut database = CompilerDatabase::new(source);
    let checked = database.check().expect("postfix receiver should check");
    let counter_field = checked.syntax().structs[1].fields[0].id;
    let increment = checked.syntax().functions[1].id;
    let call = source.find("wrapper().counter.increment()").unwrap();
    let counter = call + "wrapper().".len();
    let method = counter + "counter.".len();

    assert!(matches!(
        database.definition_at(counter).unwrap(),
        Some(DefinitionTarget::Source(definition))
            if definition.id == SourceDefinitionId::StructField(counter_field)
    ));
    assert!(matches!(
        database.definition_at(method).unwrap(),
        Some(DefinitionTarget::Source(definition))
            if definition.id == SourceDefinitionId::Function(increment)
    ));
}

#[test]
fn choice_setting_enum_paths_use_resolved_source_identities() {
    use splitscript::tooling::database::{CompilerDatabase, DefinitionTarget, SourceDefinitionId};

    let source = r#"
        enum CaptureMode {
            WindowTitle,
            ExecutableName
        }
        state "game.exe" {}
        settings {
            "Capture Source" => captureMode: choice {
                "Window Title" => CaptureMode.WindowTitle,
                "Executable Name" => CaptureMode.ExecutableName default
            }
        }
    "#;
    let mut database = CompilerDatabase::new(source);
    database.check().expect("choice setting should check");

    let option = source.find("CaptureMode.WindowTitle").unwrap();
    let variant = option + "CaptureMode.".len();
    assert!(matches!(
        database.definition_at(option).unwrap(),
        Some(DefinitionTarget::Source(definition))
            if matches!(definition.id, SourceDefinitionId::Enum(_))
    ));
    assert!(matches!(
        database.definition_at(variant).unwrap(),
        Some(DefinitionTarget::Source(definition))
            if matches!(definition.id, SourceDefinitionId::EnumVariant(_))
    ));
    assert_eq!(database.references_at(option, true).unwrap().len(), 3);
    assert_eq!(database.references_at(variant, true).unwrap().len(), 2);
}

#[test]
fn for_binding_references_navigate_to_the_loop_header() {
    use splitscript::{
        compiler::ast::Stmt,
        tooling::database::{CompilerDatabase, DefinitionTarget, SourceDefinitionId},
    };

    let source = r#"state "game.exe" {}
whileAttached {
    for element in [1, 2] {
        print(element as String)
    }
}"#;
    let mut database = CompilerDatabase::new(source);
    let checked = database.check().unwrap();
    let Stmt::For { binding, .. } = &checked.syntax().actions[0].body.statements[0] else {
        panic!("expected a for loop")
    };
    let use_offset = source.rfind("element").unwrap();
    assert!(matches!(
        database.definition_at(use_offset).unwrap(),
        Some(DefinitionTarget::Source(definition))
            if definition.id == SourceDefinitionId::Value(binding.id)
                && &source[definition.span.start..definition.span.end] == "element"
    ));
    assert_eq!(database.references_at(use_offset, true).unwrap().len(), 2);
}

#[test]
fn compiler_database_resolves_type_struct_and_pattern_syntax() {
    use splitscript::tooling::database::{CompilerDatabase, DefinitionTarget, SourceDefinitionId};

    let source = r#"
        struct Counter { value: i32 }
        struct Wrapper { counter: Counter }
        enum Mode {
            Idle,
            Active
        }
        enum Event {
            Counter(Counter)
        }
        state "game.exe" {}

        fn identity(value: Counter) -> Counter {
            return value
        }

        fn modeText(mode: Mode) -> String {
            return match mode {
                Mode.Idle => "idle",
                Mode.Active => "active"
            }
        }

        fn eventValue(event: Event) -> i32 {
            return match event {
                Event.Counter(counter) => counter.value
            }
        }

        whileAttached {
            let counter: Counter = Counter { value: 1 }
            print(modeText(Mode.Active))
        }
    "#;
    let mut database = CompilerDatabase::new(source);
    let checked = database.check().unwrap();
    let counter = checked.syntax().structs[0].id;
    let value_field = checked.syntax().structs[0].fields[0].id;
    let mode = checked.syntax().enums[0].id;
    let idle = checked.syntax().enums[0].variants[0].id;

    let wrapper_type = source.find("counter: Counter").unwrap() + "counter: ".len();
    assert!(matches!(
        database.definition_at(wrapper_type).unwrap(),
        Some(DefinitionTarget::Source(definition))
            if definition.id == SourceDefinitionId::Struct(counter)
    ));

    let parameter_type = source.find("value: Counter").unwrap() + "value: ".len();
    assert!(matches!(
        database.definition_at(parameter_type).unwrap(),
        Some(DefinitionTarget::Source(definition))
            if definition.id == SourceDefinitionId::Struct(counter)
    ));
    let return_type = source.find("-> Counter").unwrap() + "-> ".len();
    assert!(matches!(
        database.definition_at(return_type).unwrap(),
        Some(DefinitionTarget::Source(definition))
            if definition.id == SourceDefinitionId::Struct(counter)
    ));
    let payload_type = source.find("Counter(Counter)").unwrap() + "Counter(".len();
    assert!(matches!(
        database.definition_at(payload_type).unwrap(),
        Some(DefinitionTarget::Source(definition))
            if definition.id == SourceDefinitionId::Struct(counter)
    ));

    let pattern = source.find("Mode.Idle =>").unwrap();
    assert!(matches!(
        database.definition_at(pattern + 1).unwrap(),
        Some(DefinitionTarget::Source(definition))
            if definition.id == SourceDefinitionId::Enum(mode)
    ));
    assert!(matches!(
        database.definition_at(pattern + "Mode.".len()).unwrap(),
        Some(DefinitionTarget::Source(definition))
            if definition.id == SourceDefinitionId::EnumVariant(idle)
    ));

    let binding = source.find("Event.Counter(counter)").unwrap() + "Event.Counter(".len();
    let binding_definition = database.definition_at(binding).unwrap().unwrap();
    let binding_use = source.find("counter.value").unwrap();
    assert_eq!(
        database.definition_at(binding_use).unwrap(),
        Some(binding_definition)
    );

    let literal = source.find("Counter { value: 1 }").unwrap();
    assert!(matches!(
        database.definition_at(literal + 1).unwrap(),
        Some(DefinitionTarget::Source(definition))
            if definition.id == SourceDefinitionId::Struct(counter)
    ));
    assert!(matches!(
        database
            .definition_at(literal + "Counter { ".len())
            .unwrap(),
        Some(DefinitionTarget::Source(definition))
            if definition.id == SourceDefinitionId::StructField(value_field)
    ));
}

#[test]
fn setting_runtime_keys_are_nonempty_and_unique() {
    let duplicate = r#"
        state "game.exe" {}
        settings {
            "First" => first key "shared": true,
            "Second" => shared: false
        }
    "#;
    let diagnostics = splitscript::compile(duplicate)
        .expect_err("an explicit host key must not collide with an identifier key");
    assert!(
        diagnostics
            .iter()
            .any(|diagnostic| { diagnostic.message == "duplicate runtime setting key `shared`" })
    );

    let empty = r#"
        state "game.exe" {}
        settings {
            "Empty" => empty key "": true
        }
    "#;
    let diagnostics =
        splitscript::compile(empty).expect_err("the host settings map cannot use an empty key");
    assert!(
        diagnostics
            .iter()
            .any(|diagnostic| diagnostic.message == "a setting key cannot be empty")
    );
}

#[test]
fn setting_family_bindings_navigate_without_exposing_generated_members() {
    let source = r#"
        state "game.exe" {}
        settings {
            for level in 2..=4 {
                `Level {level}` key `{level}`: true,
            },
        }
    "#;
    let mut database = splitscript::tooling::database::CompilerDatabase::new(source);
    let checked = database.check().expect("settings family should check");
    let family = &checked.syntax().setting_families[0];
    let use_offset = source.find("{level}").unwrap() + 1;
    assert!(matches!(
        database.definition_at(use_offset).unwrap(),
        Some(splitscript::tooling::database::DefinitionTarget::Source(definition))
            if definition.id == splitscript::tooling::database::SourceDefinitionId::Value(family.binding_id)
                && definition.span == family.binding_span
    ));
    let for_offset = source.find("for level").unwrap();
    assert_eq!(
        database.definition_at(for_offset).unwrap(),
        Some(splitscript::tooling::database::DefinitionTarget::Language(
            splitscript::tooling::language::LanguageItemId::SettingFamily
        ))
    );
    let in_offset = source.find(" in 2").unwrap() + 1;
    assert_eq!(
        database.definition_at(in_offset).unwrap(),
        Some(splitscript::tooling::database::DefinitionTarget::Language(
            splitscript::tooling::language::LanguageItemId::SettingFamily
        ))
    );
}
