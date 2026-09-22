use super::conversion::{diagnostic_json, offset_at_position, position, position_parts};
use super::*;
use crate::{Diagnostic, highlight::SemanticTokenKind};

fn notification(method: &str, params: Value) -> Value {
    json!({ "jsonrpc": "2.0", "method": method, "params": params })
}

fn initialize(server: &mut LanguageServer) {
    server.handle(json!({
        "jsonrpc": "2.0",
        "id": 0,
        "method": "initialize",
        "params": {}
    }));
}

#[test]
fn serves_compiler_owned_documentation_index_and_markdown_pages() {
    let mut server = LanguageServer::default();
    initialize(&mut server);
    let reference = DocumentationReference::default();

    let index = server.handle(json!({
        "jsonrpc": "2.0",
        "id": 40,
        "method": "splitscript/documentation/index",
        "params": {}
    }));
    let entries = index[0]["result"].as_array().unwrap();
    assert_eq!(
        index[0]["result"],
        serde_json::to_value(reference.index()).unwrap(),
        "the LSP index must be the compiler-owned reference without an adapter projection"
    );
    assert!(entries.iter().any(|entry| {
        entry["title"] == "Duration"
            && entry["uri"] == "/stdlib/types/Duration/index.md"
            && entry["kind"] == "struct"
    }));
    assert!(entries.iter().any(|entry| {
        entry["signature"] == "asl.lifecycle.update"
            && entry["uri"] == "/migration/asl/lifecycle/update.md"
            && entry["kind"] == "migration concept"
    }));

    let search = server.handle(json!({
        "jsonrpc": "2.0",
        "id": 44,
        "method": "splitscript/documentation/search",
        "params": { "query": "multiple processes" }
    }));
    assert_eq!(
        search[0]["result"][0]["uri"],
        "/migration/asl/state/attachment.md"
    );
    assert_eq!(search[0]["result"][0]["kind"], "migration concept");

    let page = server.handle(json!({
        "jsonrpc": "2.0",
        "id": 41,
        "method": "splitscript/documentation/page",
        "params": { "uri": "/stdlib/types/Duration/index.md" }
    }));
    assert_eq!(page[0]["result"]["title"], "Duration");
    assert!(
        page[0]["result"]["markdown"]
            .as_str()
            .unwrap()
            .contains("[fromSeconds](methods/fromSeconds.md)")
    );

    let migration = server.handle(json!({
        "jsonrpc": "2.0",
        "id": 43,
        "method": "splitscript/documentation/page",
        "params": { "uri": "/migration/asl/lifecycle/update.md" }
    }));
    assert_eq!(migration[0]["result"]["title"], "update lifecycle block");
    assert!(
        migration[0]["result"]["markdown"]
            .as_str()
            .unwrap()
            .contains("[whileAttached](../../../language/while-attached.md)")
    );

    let missing = server.handle(json!({
        "jsonrpc": "2.0",
        "id": 42,
        "method": "splitscript/documentation/page",
        "params": { "uri": "/missing.md" }
    }));
    assert_eq!(missing[0]["error"]["code"], -32602);

    assert_eq!(
        page[0]["result"],
        serde_json::to_value(
            reference
                .page("/stdlib/types/Duration/index.md")
                .expect("the representative reference page exists")
        )
        .unwrap(),
        "the LSP must return the compiler-owned page without an adapter projection",
    );
}

#[test]
fn advertises_full_sync_diagnostics_formatting_and_semantic_tokens() {
    let mut server = LanguageServer::default();
    let response = server.handle(json!({
        "jsonrpc": "2.0",
        "id": 1,
        "method": "initialize",
        "params": {}
    }));
    assert_eq!(response[0]["id"], 1);
    assert_eq!(
        response[0]["result"]["capabilities"]["textDocumentSync"]["change"],
        1
    );
    assert_eq!(
        response[0]["result"]["capabilities"]["documentFormattingProvider"],
        true
    );
    assert_eq!(
        response[0]["result"]["capabilities"]["documentSymbolProvider"],
        true
    );
    assert_eq!(
        response[0]["result"]["capabilities"]["codeActionProvider"]["codeActionKinds"][0],
        "quickfix"
    );
    assert_eq!(
        response[0]["result"]["capabilities"]["codeActionProvider"]["codeActionKinds"][1],
        "refactor.extract"
    );
    assert_eq!(
        response[0]["result"]["capabilities"]["codeActionProvider"]["codeActionKinds"][2],
        "refactor.rewrite"
    );
    assert_eq!(
        response[0]["result"]["capabilities"]["semanticTokensProvider"]["full"],
        true
    );
    assert_eq!(
        response[0]["result"]["capabilities"]["completionProvider"]["triggerCharacters"][0],
        "."
    );
    assert_eq!(response[0]["result"]["capabilities"]["hoverProvider"], true);
    assert_eq!(
        response[0]["result"]["capabilities"]["inlayHintProvider"],
        true
    );
    assert_eq!(
        response[0]["result"]["capabilities"]["selectionRangeProvider"],
        true
    );
    assert_eq!(
        response[0]["result"]["capabilities"]["definitionProvider"],
        true
    );
    assert_eq!(
        response[0]["result"]["capabilities"]["typeDefinitionProvider"],
        true
    );
    assert_eq!(
        response[0]["result"]["capabilities"]["referencesProvider"],
        true
    );
    assert_eq!(
        response[0]["result"]["capabilities"]["documentHighlightProvider"],
        true
    );
    assert_eq!(
        response[0]["result"]["capabilities"]["renameProvider"]["prepareProvider"],
        true
    );
    assert_eq!(
        response[0]["result"]["capabilities"]["signatureHelpProvider"]["triggerCharacters"],
        json!(["(", ","])
    );
    assert_eq!(
        response[0]["result"]["capabilities"]["semanticTokensProvider"]["legend"]["tokenTypes"]
            [SemanticTokenKind::StateField.index() as usize],
        "stateField"
    );

    let diagnostics = server.handle(notification(
        "textDocument/didOpen",
        json!({
            "textDocument": {
                "uri": "file:///game.split",
                "languageId": "splitscript",
                "version": 4,
                "text": "state \"game.exe\" {"
            }
        }),
    ));
    assert_eq!(diagnostics[0]["method"], "textDocument/publishDiagnostics");
    assert_eq!(diagnostics[0]["params"]["version"], 4);
    assert_eq!(diagnostics[0]["params"]["diagnostics"][0]["code"], "SS0002");
}

#[test]
fn changes_reuse_the_document_database_and_formatting_ignores_type_errors() {
    let mut server = LanguageServer::default();
    initialize(&mut server);
    server.handle(notification(
        "textDocument/didOpen",
        json!({
            "textDocument": {
                "uri": "file:///game.split",
                "version": 1,
                "text": "state \"game.exe\" {}"
            }
        }),
    ));
    let diagnostics = server.handle(notification(
        "textDocument/didChange",
        json!({
            "textDocument": { "uri": "file:///game.split", "version": 2 },
            "contentChanges": [{
                "text": "state \"game.exe\"{}\nwhileAttached{let broken:bool=42}"
            }]
        }),
    ));
    assert_eq!(diagnostics[0]["params"]["version"], 2);
    assert_eq!(diagnostics[0]["params"]["diagnostics"][0]["code"], "SS0003");

    let formatting = server.handle(json!({
        "jsonrpc": "2.0",
        "id": "format",
        "method": "textDocument/formatting",
        "params": {
            "textDocument": { "uri": "file:///game.split" },
            "options": { "tabSize": 4, "insertSpaces": true }
        }
    }));
    assert_eq!(formatting[0]["id"], "format");
    assert_eq!(formatting[0]["result"].as_array().unwrap().len(), 1);
    assert!(
        formatting[0]["result"][0]["newText"]
            .as_str()
            .unwrap()
            .contains("let broken: bool = 42")
    );
}

#[test]
fn formatting_applies_editorconfig_and_explicit_editor_overrides() {
    let mut server = LanguageServer::default();
    initialize(&mut server);
    server.handle(notification(
        "textDocument/didOpen",
        json!({
            "textDocument": {
                "uri": "file:///project/game.split",
                "version": 1,
                "text": "state \"game.exe\"{}\nwhileAttached{if true{print(1)}}"
            }
        }),
    ));

    let formatting = server.handle(json!({
        "jsonrpc": "2.0",
        "id": "configured-format",
        "method": "textDocument/formatting",
        "params": {
            "textDocument": { "uri": "file:///project/game.split" },
            "options": {
                "tabSize": 8,
                "insertSpaces": false,
                "splitscript": {
                    "documentLineEnding": "lf",
                    "filesInsertFinalNewline": true,
                    "editorConfig": [{
                        "relativePath": "game.split",
                        "source": "root = true\n[*.split]\nindent_style = space\nindent_size = 2\nend_of_line = crlf\ninsert_final_newline = false"
                    }],
                    "indentStyle": "tabs",
                    "lineEnding": "lf",
                    "insertFinalNewline": true
                }
            }
        }
    }));
    let formatted = formatting[0]["result"][0]["newText"].as_str().unwrap();
    assert!(formatted.contains("\n\tif true {\n\t\tprint(1)"));
    assert!(!formatted.contains("\r\n"));
    assert!(formatted.ends_with('\n'));
}

#[test]
fn hover_survives_length_preserving_parser_repairs() {
    let mut server = LanguageServer::default();
    initialize(&mut server);
    let source = r#"state GBA {
    pos at 0x100,
}

fn bar() {
    return current.pos.x > old.pos.y
}
"#;
    let diagnostics = server.handle(notification(
        "textDocument/didOpen",
        json!({
            "textDocument": {
                "uri": "file:///showcase.split",
                "languageId": "splitscript",
                "version": 1,
                "text": source
            }
        }),
    ));
    assert_eq!(
        diagnostics[0]["params"]["diagnostics"][0]["message"],
        "expected `;` between state fields"
    );

    let hover = server.handle(json!({
        "jsonrpc": "2.0",
        "id": 91,
        "method": "textDocument/hover",
        "params": {
            "textDocument": { "uri": "file:///showcase.split" },
            "position": { "line": 0, "character": 7 }
        }
    }));
    assert!(
        hover[0]["result"]["contents"]["value"]
            .as_str()
            .unwrap()
            .contains("state GBA")
    );
}

#[test]
fn hover_survives_type_and_parser_errors_elsewhere_in_the_document() {
    let mut server = LanguageServer::default();
    initialize(&mut server);
    let source = r#"fn retained(value: i32) -> i32 {
    return value
}

state GBA {}
settings {
    "Foo" => foo: true,
}
split {
    let broken = 0b100
    let result = retained(1)
    return 0b100 || settings.foo
}
"#;
    let diagnostics = server.handle(notification(
        "textDocument/didOpen",
        json!({
            "textDocument": {
                "uri": "file:///recovery.split",
                "languageId": "splitscript",
                "version": 1,
                "text": source
            }
        }),
    ));
    assert!(
        diagnostics[0]["params"]["diagnostics"]
            .as_array()
            .unwrap()
            .iter()
            .any(|diagnostic| diagnostic["message"] == "expected `bool`, found an integer literal")
    );

    let setting = source.rfind("foo").unwrap() + 1;
    let (line, character) = position_parts(source, setting);
    let hover = server.handle(json!({
        "jsonrpc": "2.0",
        "id": "type-error",
        "method": "textDocument/hover",
        "params": {
            "textDocument": { "uri": "file:///recovery.split" },
            "position": { "line": line, "character": character }
        }
    }));
    assert!(
        hover[0]["result"]["contents"]["value"]
            .as_str()
            .is_some_and(|markdown| markdown.contains("foo: bool")),
        "{hover:#?}"
    );

    let broken_source = source.replacen("let broken = 0b100", "let broken = 0b102", 1);
    let diagnostics = server.handle(notification(
        "textDocument/didChange",
        json!({
            "textDocument": {
                "uri": "file:///recovery.split",
                "version": 2
            },
            "contentChanges": [{ "text": broken_source }]
        }),
    ));
    assert!(
        diagnostics[0]["params"]["diagnostics"]
            .as_array()
            .unwrap()
            .iter()
            .any(|diagnostic| diagnostic["message"]
                == "digit `2` is not valid in a binary integer literal")
    );

    let retained_call = broken_source.rfind("retained").unwrap() + 1;
    let (line, character) = position_parts(&broken_source, retained_call);
    let hover = server.handle(json!({
        "jsonrpc": "2.0",
        "id": "parser-error-hover",
        "method": "textDocument/hover",
        "params": {
            "textDocument": { "uri": "file:///recovery.split" },
            "position": { "line": line, "character": character }
        }
    }));
    assert!(
        hover[0]["result"]["contents"]["value"]
            .as_str()
            .is_some_and(|markdown| markdown.contains("fn retained(value: i32) -> i32")),
        "{hover:#?}"
    );

    let definition = server.handle(json!({
        "jsonrpc": "2.0",
        "id": "parser-error-definition",
        "method": "textDocument/definition",
        "params": {
            "textDocument": { "uri": "file:///recovery.split" },
            "position": { "line": line, "character": character }
        }
    }));
    assert_eq!(
        definition[0]["result"]["range"]["start"],
        position(&broken_source, broken_source.find("retained").unwrap())
    );

    let references = server.handle(json!({
        "jsonrpc": "2.0",
        "id": "parser-error-references",
        "method": "textDocument/references",
        "params": {
            "textDocument": { "uri": "file:///recovery.split" },
            "position": { "line": line, "character": character },
            "context": { "includeDeclaration": true }
        }
    }));
    assert_eq!(references[0]["result"].as_array().unwrap().len(), 2);

    let highlights = server.handle(json!({
        "jsonrpc": "2.0",
        "id": "parser-error-highlights",
        "method": "textDocument/semanticTokens/full",
        "params": { "textDocument": { "uri": "file:///recovery.split" } }
    }));
    assert!(
        !highlights[0]["result"]["data"]
            .as_array()
            .unwrap()
            .is_empty(),
        "{highlights:#?}"
    );

    let lexical_source = source.replacen("let broken = 0b100", "let broken = \"unfinished", 1);
    let diagnostics = server.handle(notification(
        "textDocument/didChange",
        json!({
            "textDocument": {
                "uri": "file:///recovery.split",
                "version": 3
            },
            "contentChanges": [{ "text": lexical_source }]
        }),
    ));
    assert!(
        diagnostics[0]["params"]["diagnostics"]
            .as_array()
            .unwrap()
            .iter()
            .any(|diagnostic| diagnostic["message"] == "unterminated string literal")
    );
    let retained_call = lexical_source.rfind("retained").unwrap() + 1;
    let (line, character) = position_parts(&lexical_source, retained_call);
    let hover = server.handle(json!({
        "jsonrpc": "2.0",
        "id": "lexical-error-hover",
        "method": "textDocument/hover",
        "params": {
            "textDocument": { "uri": "file:///recovery.split" },
            "position": { "line": line, "character": character }
        }
    }));
    assert!(
        hover[0]["result"]["contents"]["value"]
            .as_str()
            .is_some_and(|markdown| markdown.contains("fn retained(value: i32) -> i32")),
        "{hover:#?}"
    );

    let definition = server.handle(json!({
        "jsonrpc": "2.0",
        "id": "lexical-error-definition",
        "method": "textDocument/definition",
        "params": {
            "textDocument": { "uri": "file:///recovery.split" },
            "position": { "line": line, "character": character }
        }
    }));
    assert_eq!(
        definition[0]["result"]["range"]["start"],
        position(&lexical_source, lexical_source.find("retained").unwrap())
    );

    let references = server.handle(json!({
        "jsonrpc": "2.0",
        "id": "lexical-error-references",
        "method": "textDocument/references",
        "params": {
            "textDocument": { "uri": "file:///recovery.split" },
            "position": { "line": line, "character": character },
            "context": { "includeDeclaration": true }
        }
    }));
    assert_eq!(references[0]["result"].as_array().unwrap().len(), 2);

    let highlights = server.handle(json!({
        "jsonrpc": "2.0",
        "id": "lexical-error-highlights",
        "method": "textDocument/semanticTokens/full",
        "params": { "textDocument": { "uri": "file:///recovery.split" } }
    }));
    assert!(
        !highlights[0]["result"]["data"]
            .as_array()
            .unwrap()
            .is_empty(),
        "{highlights:#?}"
    );
}

#[test]
fn showcase_declaration_recovery_does_not_panic_the_language_server() {
    let mut server = LanguageServer::default();
    initialize(&mut server);
    let source = r#"state GBA {
    pos: Pos at 0x100;
}

struct Pos {
    x: f32,
}

fn bar() {
    if true {
        return 5
    }
    return false
}

fn other()

debug fn foo(x: u32, pos: Pos) -> TimerState {
    if x > 0gxb101 || settings.foo {
        return TimerState.Paused
    }
    return TimerState.NotRunning
}

settings {
    "Foo" => bar: true,
    "Yay" {
        "Some more" => foo: true,
        "Label" => aFile: file {
            "Files" => "*.*",
        },
        "Label" => okok: true,
    },
}
"#;
    let diagnostics = server.handle(notification(
        "textDocument/didOpen",
        json!({
            "textDocument": {
                "uri": "file:///showcase.split",
                "languageId": "splitscript",
                "version": 1,
                "text": source
            }
        }),
    ));
    let diagnostics = diagnostics[0]["params"]["diagnostics"]
        .as_array()
        .expect("opening the showcase publishes diagnostics");
    assert!(
        diagnostics
            .iter()
            .any(|diagnostic| { diagnostic["message"] == "unknown integer type suffix `gxb101`" })
    );
    assert!(diagnostics.iter().any(|diagnostic| {
        diagnostic["message"].as_str().is_some_and(|message| {
            message.starts_with("type `bool` does not satisfy the required `Numeric` capability")
        })
    }));

    let offset = source.find("TimerState.Paused").unwrap();
    let (line, character) = position_parts(source, offset);
    let hover = server.handle(json!({
        "jsonrpc": "2.0",
        "id": "showcase-hover",
        "method": "textDocument/hover",
        "params": {
            "textDocument": { "uri": "file:///showcase.split" },
            "position": { "line": line, "character": character }
        }
    }));
    assert_eq!(hover[0]["id"], "showcase-hover");

    let highlights = server.handle(json!({
        "jsonrpc": "2.0",
        "id": "showcase-highlights",
        "method": "textDocument/semanticTokens/full",
        "params": { "textDocument": { "uri": "file:///showcase.split" } }
    }));
    assert_eq!(highlights[0]["id"], "showcase-highlights");
}

#[test]
fn publishes_unused_binding_warnings_without_rejecting_the_document() {
    let mut server = LanguageServer::default();
    initialize(&mut server);
    let diagnostics = server.handle(notification(
        "textDocument/didOpen",
        json!({
            "textDocument": {
                "uri": "file:///game.split",
                "version": 1,
                "text": "state \"game.exe\" {} whileAttached { let unused = 1 }"
            }
        }),
    ));
    let warning = &diagnostics[0]["params"]["diagnostics"][0];
    assert_eq!(warning["severity"], 2);
    assert_eq!(warning["code"], "SS1002");
    assert!(
        warning["message"]
            .as_str()
            .unwrap()
            .contains("unused variable")
    );
    assert_eq!(
        warning["data"]["fixes"][0]["applicability"],
        "machine-applicable"
    );
}

#[test]
fn debug_only_use_warning_offers_a_debug_modifier_quick_fix() {
    let source =
        "state \"game.exe\" {}\nwhileAttached {\n    let detail = 1\n    debug print(detail)\n}";
    let uri = "file:///debug-only-use.split";
    let mut server = LanguageServer::default();
    initialize(&mut server);
    let diagnostics = server.handle(notification(
        "textDocument/didOpen",
        json!({
            "textDocument": {
                "uri": uri,
                "version": 1,
                "text": source
            }
        }),
    ));
    let published = diagnostics[0]["params"]["diagnostics"].as_array().unwrap();
    let warning = published
        .iter()
        .find(|diagnostic| diagnostic["code"] == "SS1009")
        .expect("debug-only use warning should be published");
    assert_eq!(warning["severity"], 2);

    let actions = server.handle(json!({
        "jsonrpc": "2.0",
        "id": 240,
        "method": "textDocument/codeAction",
        "params": {
            "textDocument": { "uri": uri },
            "range": warning["range"],
            "context": {
                "diagnostics": [warning],
                "only": ["quickfix"]
            }
        }
    }));
    let quick_fixes = actions[0]["result"].as_array().unwrap();
    assert_eq!(quick_fixes.len(), 1, "{quick_fixes:#?}");
    assert_eq!(quick_fixes[0]["title"], "mark `detail` as debug-only");
    assert_eq!(quick_fixes[0]["isPreferred"], true);
    assert_eq!(
        quick_fixes[0]["edit"]["changes"][uri][0]["newText"],
        "debug "
    );
}

#[test]
fn value_block_tail_warning_survives_analysis_and_offers_its_parser_fix() {
    let source = "state \"game.exe\" {} setup { let value: u32 = { 1; }; print(value) }";
    let uri = "file:///value-block-warning.split";
    let mut server = LanguageServer::default();
    initialize(&mut server);
    let diagnostics = server.handle(notification(
        "textDocument/didOpen",
        json!({
            "textDocument": {
                "uri": uri,
                "version": 1,
                "text": source
            }
        }),
    ));
    let published = diagnostics[0]["params"]["diagnostics"].as_array().unwrap();
    assert_eq!(published.len(), 1, "{published:#?}");
    assert_eq!(published[0]["code"], "SS1005");
    assert_eq!(published[0]["severity"], 2);

    let actions = server.handle(json!({
        "jsonrpc": "2.0",
        "id": 24,
        "method": "textDocument/codeAction",
        "params": {
            "textDocument": { "uri": uri },
            "range": {
                "start": { "line": 0, "character": 0 },
                "end": position(source, source.len())
            },
            "context": {
                "diagnostics": published,
                "only": ["quickfix"]
            }
        }
    }));
    let quick_fixes = actions[0]["result"].as_array().unwrap();
    assert_eq!(quick_fixes.len(), 1, "{quick_fixes:#?}");
    assert_eq!(quick_fixes[0]["title"], "remove the trailing semicolon");
    assert_eq!(quick_fixes[0]["isPreferred"], true);
    assert_eq!(quick_fixes[0]["edit"]["changes"][uri][0]["newText"], "");
}

#[test]
fn retry_fallback_warning_preserves_editor_analysis_and_offers_both_groupings() {
    let source = concat!(
        "state \"game.exe\" {}\n",
        "fn choose(value: i32?!) { return retry value else 0 }\n",
        "whileAttached {\n",
        "    let number: i32 = 4\n",
        "    let bounded = number.clamp(0, 7)\n",
        "}\n"
    );
    let uri = "file:///retry-fallback-warning.split";
    let mut server = LanguageServer::default();
    initialize(&mut server);
    let diagnostics = server.handle(notification(
        "textDocument/didOpen",
        json!({
            "textDocument": {
                "uri": uri,
                "version": 1,
                "text": source
            }
        }),
    ));
    let published = diagnostics[0]["params"]["diagnostics"].as_array().unwrap();
    let warning = published
        .iter()
        .find(|diagnostic| diagnostic["code"] == "SS1006")
        .expect("the retry/fallback ambiguity warning should be published");
    assert_eq!(warning["severity"], 2);
    let warning_range = warning["range"].clone();

    let actions = server.handle(json!({
        "jsonrpc": "2.0",
        "id": 25,
        "method": "textDocument/codeAction",
        "params": {
            "textDocument": { "uri": uri },
            "range": warning_range,
            "context": {
                "diagnostics": [warning],
                "only": ["quickfix"]
            }
        }
    }));
    let quick_fixes = actions[0]["result"].as_array().unwrap();
    assert_eq!(quick_fixes.len(), 2, "{quick_fixes:#?}");
    assert_eq!(
        quick_fixes[0]["title"],
        "make the current retry boundary explicit"
    );
    assert_eq!(quick_fixes[0]["isPreferred"], true);
    assert_eq!(quick_fixes[1]["title"], "retry the complete fallback chain");
    assert_eq!(quick_fixes[1]["isPreferred"], false);

    let hover_offset = source.find("clamp").unwrap() + 2;
    let (hover_line, hover_character) = position_parts(source, hover_offset);
    let hover = server.handle(json!({
        "jsonrpc": "2.0",
        "id": 26,
        "method": "textDocument/hover",
        "params": {
            "textDocument": { "uri": uri },
            "position": { "line": hover_line, "character": hover_character }
        }
    }));
    let markdown = hover[0]["result"]["contents"]["value"]
        .as_str()
        .expect("catalog hover should survive the parser warning");
    assert!(markdown.contains("i32.clamp"), "{markdown}");
}

#[test]
fn positions_use_utf16_code_units_and_close_clears_diagnostics() {
    assert_eq!(
        position("🦊x", "🦊".len()),
        json!({ "line": 0, "character": 2 })
    );

    let mut server = LanguageServer::default();
    initialize(&mut server);
    server.handle(notification(
        "textDocument/didOpen",
        json!({
            "textDocument": {
                "uri": "file:///game.split",
                "version": 1,
                "text": "state \"game.exe\" {"
            }
        }),
    ));
    let closed = server.handle(notification(
        "textDocument/didClose",
        json!({ "textDocument": { "uri": "file:///game.split" } }),
    ));
    assert!(
        closed[0]["params"]["diagnostics"]
            .as_array()
            .unwrap()
            .is_empty()
    );
}

#[test]
fn diagnostic_conversion_preserves_notes_labels_and_fixes() {
    use crate::ast::Span;

    let diagnostic = Diagnostic::new("bad value", Span { start: 5, end: 6 })
        .with_secondary_label(Span { start: 0, end: 4 }, "declared here")
        .with_note("values must agree")
        .with_machine_applicable_fix("replace it", Span { start: 5, end: 6 }, "0");
    let converted = diagnostic_json("file:///game.split", "🦊\nvalue", &diagnostic);

    assert!(
        converted["message"]
            .as_str()
            .unwrap()
            .contains("note: values must agree")
    );
    assert_eq!(
        converted["relatedInformation"][0]["message"],
        "declared here"
    );
    assert_eq!(converted["data"]["fixes"][0]["title"], "replace it");
    assert_eq!(converted["data"]["fixes"][0]["edits"][0]["newText"], "0");
}

#[test]
fn migration_diagnostics_link_to_their_compiler_owned_reference_page() {
    use crate::ast::Span;

    let diagnostic = Diagnostic::new("legacy lifecycle", Span { start: 0, end: 6 })
        .with_migration_topic("asl.lifecycle.update");
    let converted = diagnostic_json("file:///game.split", "update", &diagnostic);

    assert_eq!(
        converted["codeDescription"]["href"],
        "splitscript-docs:/migration/asl/lifecycle/update.md"
    );
    assert_eq!(converted["data"]["migrationTopic"], "asl.lifecycle.update");
}

#[test]
fn diagnostics_can_link_to_non_migration_reference_pages() {
    use crate::ast::Span;

    let diagnostic = Diagnostic::new("unknown provider", Span { start: 0, end: 5 })
        .with_documentation_uri("/stdlib/state-providers/index.md");
    let converted = diagnostic_json("file:///game.split", "Untiy", &diagnostic);

    assert_eq!(
        converted["codeDescription"]["href"],
        "splitscript-docs:/stdlib/state-providers/index.md"
    );
    assert_eq!(
        converted["data"]["documentationUri"],
        "/stdlib/state-providers/index.md"
    );
}

#[test]
fn shutdown_requires_a_following_exit_notification() {
    let mut server = LanguageServer::default();
    initialize(&mut server);
    let response = server.handle(json!({
        "jsonrpc": "2.0",
        "id": 9,
        "method": "shutdown"
    }));
    assert_eq!(response[0]["result"], Value::Null);
    assert!(!server.should_exit());
    server.handle(notification("exit", Value::Null));
    assert!(server.should_exit());
}

#[test]
fn malformed_request_parameters_return_consistent_protocol_errors() {
    let mut server = LanguageServer::default();
    initialize(&mut server);
    let response = server.handle(json!({
        "jsonrpc": "2.0",
        "id": 91,
        "method": "textDocument/hover",
        "params": {
            "textDocument": { "uri": 42 },
            "position": { "line": "zero", "character": 0 }
        }
    }));
    assert_eq!(response[0]["error"]["code"], -32602);
    assert!(
        response[0]["error"]["message"]
            .as_str()
            .unwrap()
            .starts_with("invalid request parameters:")
    );

    let invalid_envelope = server.handle(json!({
        "jsonrpc": "2.0",
        "id": 92,
        "method": 7,
        "params": {}
    }));
    assert_eq!(invalid_envelope[0]["error"]["code"], -32600);
}

#[test]
fn semantic_tokens_cover_language_domains_and_use_utf16_deltas() {
    let source = concat!(
        "// 🦊\n",
        "enum Mode { Active }\n",
        "state \"game.exe\" { level = process.read<i32>(0) }\n",
        "settings { \"General\" { \"Enabled\" => enabled: true } }\n",
        "debug fn observe(mode: Mode) { debug print(mode as String) }\n",
        "whileAttached {\n",
        "    let marker = await process.scan(0, 1, sig\"48 ??\")\n",
        "    if current.level == 1 { observe(Mode.Active) }\n",
        "}\n"
    );
    let mut server = LanguageServer::default();
    initialize(&mut server);
    server.handle(notification(
        "textDocument/didOpen",
        json!({
            "textDocument": {
                "uri": "file:///semantic.split",
                "version": 1,
                "text": source
            }
        }),
    ));
    let response = server.handle(json!({
        "jsonrpc": "2.0",
        "id": 7,
        "method": "textDocument/semanticTokens/full",
        "params": { "textDocument": { "uri": "file:///semantic.split" } }
    }));
    let data = response[0]["result"]["data"]
        .as_array()
        .expect("semantic token data");
    let (tokens, remainder) = data.as_chunks::<5>();
    assert!(remainder.is_empty());
    let kinds = tokens
        .iter()
        .map(|token| token[3].as_u64().unwrap() as u32)
        .collect::<Vec<_>>();
    for expected in [
        SemanticTokenKind::SettingTitle,
        SemanticTokenKind::Setting,
        SemanticTokenKind::StateField,
        SemanticTokenKind::Lifecycle,
        SemanticTokenKind::Enum,
        SemanticTokenKind::EnumMember,
        SemanticTokenKind::Signature,
        SemanticTokenKind::Debug,
    ] {
        assert!(
            kinds.contains(&expected.index()),
            "missing {expected:?} semantic token"
        );
    }
    assert!(tokens.iter().any(|token| {
        token[4].as_u64().unwrap() as u32 & crate::highlight::MODIFIER_DEBUG != 0
    }));

    assert_eq!(position_parts("🦊x", "🦊".len()), (0, 2));
}

#[test]
fn inlay_hints_show_inferred_types_and_respect_explicit_annotations() {
    let source = concat!(
        "// 🦊\n",
        "state \"game.exe\" {}\n",
        "let global = 7\n",
        "fn identity(value) { return value }\n",
        "whileAttached {\n",
        "    let local = identity(global)\n",
        "    let explicit: i32 = local\n",
        "}\n"
    );
    let uri = "file:///inlay.split";
    let mut server = LanguageServer::default();
    initialize(&mut server);
    server.handle(notification(
        "textDocument/didOpen",
        json!({
            "textDocument": {
                "uri": uri,
                "version": 1,
                "text": source
            }
        }),
    ));
    let response = server.handle(json!({
        "jsonrpc": "2.0",
        "id": 8,
        "method": "textDocument/inlayHint",
        "params": {
            "textDocument": { "uri": uri },
            "range": {
                "start": { "line": 0, "character": 0 },
                "end": position(source, source.len())
            }
        }
    }));
    let hints = response[0]["result"].as_array().unwrap();
    assert_eq!(
        hints
            .iter()
            .map(|hint| hint["label"].as_str().unwrap())
            .collect::<Vec<_>>(),
        [": i32", ": T", " -> T", ": i32"]
    );
    assert!(hints.iter().all(|hint| hint["kind"] == 1));
    assert_eq!(
        hints[0]["position"],
        position(source, source.find("global").unwrap() + "global".len())
    );
    assert_eq!(
        hints[1]["position"],
        position(source, source.find("value").unwrap() + "value".len())
    );
    assert_eq!(
        hints[2]["position"],
        position(
            source,
            source.find("identity(value)").unwrap() + "identity(value)".len()
        )
    );
    assert_eq!(
        hints[3]["position"],
        position(source, source.find("local").unwrap() + "local".len())
    );
}

#[test]
fn inlay_hints_render_complete_closure_signatures() {
    let source = concat!(
        "state \"game.exe\" {}\n",
        "whileAttached {\n",
        "    let unary: (u16) -> u32 = value => value as u32\n",
        "    let binary: (u16, u16) -> u16 = (x, y) => x + y\n",
        "}\n"
    );
    let uri = "file:///closure-inlays.split";
    let mut server = LanguageServer::default();
    initialize(&mut server);
    server.handle(notification(
        "textDocument/didOpen",
        json!({
            "textDocument": {
                "uri": uri,
                "version": 1,
                "text": source
            }
        }),
    ));
    let response = server.handle(json!({
        "jsonrpc": "2.0",
        "id": 81,
        "method": "textDocument/inlayHint",
        "params": {
            "textDocument": { "uri": uri },
            "range": {
                "start": { "line": 0, "character": 0 },
                "end": position(source, source.len())
            }
        }
    }));
    let labels = response[0]["result"]
        .as_array()
        .unwrap()
        .iter()
        .map(|hint| hint["label"].as_str().unwrap())
        .collect::<Vec<_>>();
    assert_eq!(
        labels,
        ["(", ": u16", ") -> u32", ": u16", ": u16", " -> u16"]
    );
}

#[test]
fn completion_uses_inferred_members_catalog_docs_and_utf16_text_edits() {
    let source = concat!(
        "// 🦊\n",
        "state \"game.exe\" {}\n",
        "whileAttached {\n",
        "    let number: i32 = 4\n",
        "    number.cl\n",
        "}\n"
    );
    let offset = source.find("number.cl").unwrap() + "number.cl".len();
    let (line, character) = position_parts(source, offset);
    let mut server = LanguageServer::default();
    initialize(&mut server);
    server.handle(notification(
        "textDocument/didOpen",
        json!({
            "textDocument": {
                "uri": "file:///completion.split",
                "version": 1,
                "text": source
            }
        }),
    ));
    let response = server.handle(json!({
        "jsonrpc": "2.0",
        "id": 8,
        "method": "textDocument/completion",
        "params": {
            "textDocument": { "uri": "file:///completion.split" },
            "position": { "line": line, "character": character }
        }
    }));
    let items = response[0]["result"]["items"]
        .as_array()
        .expect("completion items");
    let clamp = items
        .iter()
        .find(|item| item["label"] == "clamp")
        .expect("numeric clamp completion");
    assert_eq!(clamp["kind"], 2);
    assert_eq!(clamp["insertTextFormat"], 2);
    assert_eq!(
        clamp["textEdit"]["newText"],
        "clamp(${1:minimum}, ${2:maximum})"
    );
    assert_eq!(
        clamp["textEdit"]["range"]["start"],
        json!({ "line": line, "character": character - 2 })
    );
    assert!(
        clamp["documentation"]["value"]
            .as_str()
            .unwrap()
            .contains("smaller")
            || clamp["documentation"]["value"]
                .as_str()
                .unwrap()
                .contains("inclusive range")
    );

    assert_eq!(offset_at_position("🦊x", 0, 2), Some("🦊".len()));
    assert_eq!(offset_at_position("🦊x", 0, 1), None);
}

#[test]
fn completion_replaces_only_the_setting_key_string_contents() {
    let source = concat!(
        "state \"game.exe\" {}\n",
        "settings {\n",
        "    /// Splits at the boss.\n",
        "    \"Boss\" => boss key \"split-boss\": true,\n",
        "}\n",
        "whileAttached { let enabled = settings.enabled(\"spl\") }\n",
    );
    let offset = source.find("settings.enabled(\"spl").unwrap() + "settings.enabled(\"spl".len();
    let (line, character) = position_parts(source, offset);
    let mut server = LanguageServer::default();
    initialize(&mut server);
    server.handle(notification(
        "textDocument/didOpen",
        json!({
            "textDocument": {
                "uri": "file:///setting-key-completion.split",
                "version": 1,
                "text": source
            }
        }),
    ));
    let response = server.handle(json!({
        "jsonrpc": "2.0",
        "id": 81,
        "method": "textDocument/completion",
        "params": {
            "textDocument": { "uri": "file:///setting-key-completion.split" },
            "position": { "line": line, "character": character }
        }
    }));
    let item = response[0]["result"]["items"]
        .as_array()
        .unwrap()
        .iter()
        .find(|item| item["label"] == "split-boss")
        .expect("the declared key is completed");

    assert_eq!(item["kind"], 10);
    assert_eq!(item["textEdit"]["newText"], "split-boss");
    assert_eq!(
        item["textEdit"]["range"]["start"],
        json!({ "line": line, "character": character - 3 })
    );
    assert_eq!(
        item["textEdit"]["range"]["end"],
        json!({ "line": line, "character": character })
    );
    assert_eq!(item["documentation"]["value"], "Splits at the boss.");
}

#[test]
fn hover_and_signature_help_preserve_resolved_catalog_information() {
    let source = concat!(
        "// 🦊\n",
        "state \"game.exe\" {}\n",
        "whileAttached {\n",
        "    let number: i32 = 4\n",
        "    let bounded = number.clamp(0, 7)\n",
        "}\n"
    );
    let mut server = LanguageServer::default();
    initialize(&mut server);
    server.handle(notification(
        "textDocument/didOpen",
        json!({
            "textDocument": {
                "uri": "file:///insight.split",
                "version": 1,
                "text": source
            }
        }),
    ));

    let hover_offset = source.find("clamp").unwrap() + 2;
    let (hover_line, hover_character) = position_parts(source, hover_offset);
    let hover = server.handle(json!({
        "jsonrpc": "2.0",
        "id": 9,
        "method": "textDocument/hover",
        "params": {
            "textDocument": { "uri": "file:///insight.split" },
            "position": { "line": hover_line, "character": hover_character }
        }
    }));
    let markdown = hover[0]["result"]["contents"]["value"]
        .as_str()
        .expect("hover markdown");
    assert!(markdown.contains("i32.clamp"));
    assert!(markdown.contains("Self = i32"));
    assert!(markdown.contains("Runtime behavior"));
    assert!(markdown.contains("Examples"));

    let value_offset = source.rfind("number").unwrap() + "number".len() - 1;
    let (value_line, value_character) = position_parts(source, value_offset);
    let value_hover = server.handle(json!({
        "jsonrpc": "2.0",
        "id": 90,
        "method": "textDocument/hover",
        "params": {
            "textDocument": { "uri": "file:///insight.split" },
            "position": { "line": value_line, "character": value_character }
        }
    }));
    assert!(
        value_hover[0]["result"]["contents"]["value"]
            .as_str()
            .unwrap()
            .contains("let number: i32")
    );
    assert_eq!(
        value_hover[0]["result"]["range"]["start"],
        position(source, source.rfind("number").unwrap())
    );

    // The exact token at a boundary wins: the cursor after `number` is on the
    // dot, so it reports the enclosing call expression rather than selecting
    // the receiver variable.
    let dot_offset = source.rfind("number").unwrap() + "number".len();
    let (dot_line, dot_character) = position_parts(source, dot_offset);
    let dot_hover = server.handle(json!({
        "jsonrpc": "2.0",
        "id": 91,
        "method": "textDocument/hover",
        "params": {
            "textDocument": { "uri": "file:///insight.split" },
            "position": { "line": dot_line, "character": dot_character }
        }
    }));
    assert_eq!(
        dot_hover[0]["result"]["contents"]["value"],
        "```splitscript\ni32\n```"
    );
    assert_eq!(
        dot_hover[0]["result"]["range"]["start"],
        position(source, source.rfind("number").unwrap())
    );
    assert_eq!(
        dot_hover[0]["result"]["range"]["end"],
        position(
            source,
            source.find("clamp(0, 7)").unwrap() + "clamp(0, 7)".len()
        )
    );

    let parameter_offset = source.find(", 7").unwrap() + 2;
    let (parameter_line, parameter_character) = position_parts(source, parameter_offset);
    let signature = server.handle(json!({
        "jsonrpc": "2.0",
        "id": 10,
        "method": "textDocument/signatureHelp",
        "params": {
            "textDocument": { "uri": "file:///insight.split" },
            "position": { "line": parameter_line, "character": parameter_character }
        }
    }));
    assert_eq!(signature[0]["result"]["activeParameter"], 1);
    assert!(
        signature[0]["result"]["signatures"][0]["label"]
            .as_str()
            .unwrap()
            .starts_with("i32.clamp")
    );
    assert_eq!(
        signature[0]["result"]["signatures"][0]["parameters"][1]["label"],
        "maximum"
    );
}

#[test]
fn selection_ranges_follow_recovered_syntax_and_preserve_utf16_positions() {
    let source = concat!(
        "// 🦊\n",
        "state \"game.exe\" {}\n",
        "fn calculate(value: i32) -> i32 {\n",
        "    let result = value * (1 + 2)\n",
        "    return result\n",
        "}\n"
    );
    let mut server = LanguageServer::default();
    initialize(&mut server);
    server.handle(notification(
        "textDocument/didOpen",
        json!({
            "textDocument": {
                "uri": "file:///selection.split",
                "version": 1,
                "text": source
            }
        }),
    ));

    let offset = source.find("1 + 2").unwrap();
    let second_offset = source.rfind("result").unwrap();
    let (line, character) = position_parts(source, offset);
    let (second_line, second_character) = position_parts(source, second_offset);
    let response = server.handle(json!({
        "jsonrpc": "2.0",
        "id": 92,
        "method": "textDocument/selectionRange",
        "params": {
            "textDocument": { "uri": "file:///selection.split" },
            "positions": [
                { "line": line, "character": character },
                { "line": second_line, "character": second_character }
            ]
        }
    }));

    assert_eq!(response[0]["result"].as_array().unwrap().len(), 2);
    let mut node = &response[0]["result"][0];
    let mut ranges = Vec::new();
    while !node.is_null() {
        ranges.push(node["range"].clone());
        node = &node["parent"];
    }
    let expected = |start: usize, end: usize| {
        json!({
            "start": position(source, start),
            "end": position(source, end)
        })
    };
    let range_of = |text: &str| {
        let start = source.find(text).unwrap();
        expected(start, start + text.len())
    };

    assert_eq!(ranges.first(), Some(&expected(offset, offset)));
    for range in [
        range_of("1"),
        range_of("(1 + 2)"),
        range_of("value * (1 + 2)"),
        range_of("let result = value * (1 + 2)"),
        range_of(
            "fn calculate(value: i32) -> i32 {\n    let result = value * (1 + 2)\n    return result\n}",
        ),
        expected(0, source.len()),
    ] {
        assert!(ranges.contains(&range), "missing selection range {range}");
    }
}

#[test]
fn catalog_docs_completion_and_hover_stay_in_sync() {
    use crate::{
        DocumentationReference,
        documentation::StandardLibraryDocumentation,
        stdlib::{StdlibItemId, StdlibSymbolId},
    };

    let incomplete = concat!(
        "state \"game.exe\" {}\n",
        "whileAttached {\n",
        "    let number: i32 = 4\n",
        "    number.cl\n",
        "}\n"
    );
    let generic = StandardLibraryDocumentation::generate(StdlibItemId::NumericClamp, &[]);
    let mut server = LanguageServer::default();
    initialize(&mut server);
    server.handle(notification(
        "textDocument/didOpen",
        json!({
            "textDocument": {
                "uri": "file:///catalog-sync.split",
                "version": 1,
                "text": incomplete
            }
        }),
    ));
    let completion_offset = incomplete.find("number.cl").unwrap() + "number.cl".len();
    let (line, character) = position_parts(incomplete, completion_offset);
    let completion = server.handle(json!({
        "jsonrpc": "2.0",
        "id": 11,
        "method": "textDocument/completion",
        "params": {
            "textDocument": { "uri": "file:///catalog-sync.split" },
            "position": { "line": line, "character": character }
        }
    }));
    let clamp = completion[0]["result"]["items"]
        .as_array()
        .unwrap()
        .iter()
        .find(|item| item["label"] == "clamp")
        .expect("clamp completion");
    assert_eq!(clamp["detail"], generic.signature);
    let documentation_uri = DocumentationReference::default()
        .standard_library_symbol_uri(StdlibSymbolId::Item(StdlibItemId::NumericClamp));
    let completion_markdown = clamp["documentation"]["value"]
        .as_str()
        .expect("completion Markdown");
    assert!(completion_markdown.starts_with(&generic.summary_markdown()));
    assert!(completion_markdown.contains("Open full documentation"));
    assert!(completion_markdown.contains("splitscript.openDocumentation"));
    assert!(completion_markdown.contains(&format!(
        "%5B%22{}%22%5D",
        documentation_uri.replace('/', "%2F")
    )));

    let complete = incomplete.replace("number.cl\n", "number.clamp(0, 7)\n");
    server.handle(notification(
        "textDocument/didChange",
        json!({
            "textDocument": { "uri": "file:///catalog-sync.split", "version": 2 },
            "contentChanges": [{ "text": complete }]
        }),
    ));
    let hover_offset = complete.find("clamp").unwrap() + 2;
    let (line, character) = position_parts(&complete, hover_offset);
    let hover = server.handle(json!({
        "jsonrpc": "2.0",
        "id": 12,
        "method": "textDocument/hover",
        "params": {
            "textDocument": { "uri": "file:///catalog-sync.split" },
            "position": { "line": line, "character": character }
        }
    }));
    let resolved = StandardLibraryDocumentation::generate(
        StdlibItemId::NumericClamp,
        &[("Self", "i32".to_owned())],
    );
    let hover_markdown = hover[0]["result"]["contents"]["value"]
        .as_str()
        .expect("hover Markdown");
    assert!(hover_markdown.starts_with(&resolved.hover_markdown()));
    assert!(hover_markdown.contains("Open full documentation"));
    assert!(hover_markdown.contains(&format!(
        "%5B%22{}%22%5D",
        documentation_uri.replace('/', "%2F")
    )));
    assert_eq!(generic.summary_markdown(), resolved.summary_markdown());

    let definition = server.handle(json!({
        "jsonrpc": "2.0",
        "id": 13,
        "method": "textDocument/definition",
        "params": {
            "textDocument": { "uri": "file:///catalog-sync.split" },
            "position": { "line": line, "character": character }
        }
    }));
    assert_eq!(
        definition[0]["result"]["uri"],
        format!("splitscript-docs:{documentation_uri}")
    );
    assert_eq!(
        definition[0]["result"]["range"],
        json!({
            "start": { "line": 0, "character": 0 },
            "end": { "line": 0, "character": 0 }
        })
    );
}

#[test]
fn definition_and_references_use_source_identities_and_utf16_ranges() {
    let source = concat!(
        "// 🦊\n",
        "state \"game.exe\" {}\n",
        "fn observe(value: i32) { print(value as String) }\n",
        "whileAttached {\n",
        "    observe(1)\n",
        "    observe (2)\n",
        "}\n"
    );
    let uri = "file:///navigation.split";
    let mut server = LanguageServer::default();
    initialize(&mut server);
    server.handle(notification(
        "textDocument/didOpen",
        json!({
            "textDocument": {
                "uri": uri,
                "version": 1,
                "text": source
            }
        }),
    ));

    // A token gap permits the editor-friendly end-of-word fallback.
    let call = source.rfind("observe").unwrap() + "observe".len();
    let (line, character) = position_parts(source, call);
    let definition = server.handle(json!({
        "jsonrpc": "2.0",
        "id": 13,
        "method": "textDocument/definition",
        "params": {
            "textDocument": { "uri": uri },
            "position": { "line": line, "character": character }
        }
    }));
    let declaration = source.find("observe").unwrap();
    assert_eq!(definition[0]["result"]["uri"], uri);
    assert_eq!(
        definition[0]["result"]["range"]["start"],
        position(source, declaration)
    );
    assert_eq!(
        definition[0]["result"]["range"]["end"],
        position(source, declaration + "observe".len())
    );

    let references = server.handle(json!({
        "jsonrpc": "2.0",
        "id": 14,
        "method": "textDocument/references",
        "params": {
            "textDocument": { "uri": uri },
            "position": { "line": line, "character": character },
            "context": { "includeDeclaration": false }
        }
    }));
    assert_eq!(references[0]["result"].as_array().unwrap().len(), 2);
    assert_eq!(
        references[0]["result"][0]["range"]["start"],
        position(source, source.find("observe(1)").unwrap())
    );

    let with_declaration = server.handle(json!({
        "jsonrpc": "2.0",
        "id": 15,
        "method": "textDocument/references",
        "params": {
            "textDocument": { "uri": uri },
            "position": { "line": line, "character": character },
            "context": { "includeDeclaration": true }
        }
    }));
    assert_eq!(with_declaration[0]["result"].as_array().unwrap().len(), 3);

    // Navigation receives a caret position rather than a hovered character,
    // so the identifier ending at an adjacent opening parenthesis remains the
    // target.
    let adjacent = source.find("observe(1)").unwrap() + "observe".len();
    let (adjacent_line, adjacent_character) = position_parts(source, adjacent);
    let definition = server.handle(json!({
        "jsonrpc": "2.0",
        "id": 16,
        "method": "textDocument/definition",
        "params": {
            "textDocument": { "uri": uri },
            "position": { "line": adjacent_line, "character": adjacent_character }
        }
    }));
    assert_eq!(definition[0]["result"]["uri"], uri);
    assert_eq!(
        definition[0]["result"]["range"]["start"],
        position(source, declaration)
    );

    let references = server.handle(json!({
        "jsonrpc": "2.0",
        "id": 17,
        "method": "textDocument/references",
        "params": {
            "textDocument": { "uri": uri },
            "position": { "line": adjacent_line, "character": adjacent_character },
            "context": { "includeDeclaration": true }
        }
    }));
    assert_eq!(references[0]["result"].as_array().unwrap().len(), 3);
}

#[test]
fn document_highlights_classify_reads_and_writes() {
    let source = concat!(
        "let total = 0\n",
        "state \"game.exe\" {}\n",
        "whileAttached {\n",
        "    total += 1\n",
        "    print(total)\n",
        "}\n"
    );
    let uri = "file:///highlights.split";
    let mut server = LanguageServer::default();
    initialize(&mut server);
    server.handle(notification(
        "textDocument/didOpen",
        json!({
            "textDocument": {
                "uri": uri,
                "version": 1,
                "text": source
            }
        }),
    ));

    let use_offset = source.rfind("total").unwrap();
    let (line, character) = position_parts(source, use_offset);
    let highlights = server.handle(json!({
        "jsonrpc": "2.0",
        "id": 18,
        "method": "textDocument/documentHighlight",
        "params": {
            "textDocument": { "uri": uri },
            "position": { "line": line, "character": character }
        }
    }));
    let highlights = highlights[0]["result"].as_array().unwrap();
    assert_eq!(highlights.len(), 3);
    assert_eq!(highlights[0]["kind"], 1, "the declaration is text");
    assert_eq!(highlights[1]["kind"], 3, "the assignment is a write");
    assert_eq!(highlights[2]["kind"], 2, "the print argument is a read");
}

#[test]
fn type_definition_navigates_inferred_source_and_catalog_types() {
    let source = concat!(
        "struct Point { x: i32, }\n",
        "state \"game.exe\" {}\n",
        "whileAttached {\n",
        "    let point = Point { x: 1 }\n",
        "    let copy = point\n",
        "    let count = 2\n",
        "    print(copy.x + count)\n",
        "}\n"
    );
    let uri = "file:///type-definition.split";
    let mut server = LanguageServer::default();
    initialize(&mut server);
    server.handle(notification(
        "textDocument/didOpen",
        json!({
            "textDocument": {
                "uri": uri,
                "version": 1,
                "text": source
            }
        }),
    ));

    let point_use = source.rfind("= point").unwrap() + 2;
    let (line, character) = position_parts(source, point_use);
    let structure = server.handle(json!({
        "jsonrpc": "2.0",
        "id": 19,
        "method": "textDocument/typeDefinition",
        "params": {
            "textDocument": { "uri": uri },
            "position": { "line": line, "character": character }
        }
    }));
    let declaration = source.find("Point").unwrap();
    assert_eq!(structure[0]["result"]["uri"], uri);
    assert_eq!(
        structure[0]["result"]["range"]["start"],
        position(source, declaration)
    );

    let literal = source.find("= 2").unwrap() + 2;
    let (line, character) = position_parts(source, literal);
    let builtin = server.handle(json!({
        "jsonrpc": "2.0",
        "id": 20,
        "method": "textDocument/typeDefinition",
        "params": {
            "textDocument": { "uri": uri },
            "position": { "line": line, "character": character }
        }
    }));
    assert_eq!(
        builtin[0]["result"]["uri"],
        "splitscript-docs:/stdlib/types/i32/index.md"
    );
}

#[test]
fn domain_roots_navigate_to_their_blocks_and_providers_hover() {
    let source = concat!(
        "state GBA { room: u8 at 0x03000010 }\n",
        "settings { \"Enabled\" => enabled: true }\n",
        "whileAttached {\n",
        "    let emulator = gba\n",
        "    let stateChanged = current.room != old.room\n",
        "    let settingChanged = settings.enabled != oldSettings.enabled\n",
        "}\n"
    );
    let uri = "file:///domain-navigation.split";
    let mut server = LanguageServer::default();
    initialize(&mut server);
    server.handle(notification(
        "textDocument/didOpen",
        json!({
            "textDocument": {
                "uri": uri,
                "version": 1,
                "text": source
            }
        }),
    ));

    for (root, declaration) in [
        ("gba\n", source.find("state").unwrap()),
        ("current.room", source.find("state").unwrap()),
        ("old.room", source.find("state").unwrap()),
        ("settings.enabled", source.find("settings").unwrap()),
        ("oldSettings.enabled", source.find("settings").unwrap()),
    ] {
        let offset = source.find(root).unwrap();
        let (line, character) = position_parts(source, offset);
        let definition = server.handle(json!({
            "jsonrpc": "2.0",
            "id": 130,
            "method": "textDocument/definition",
            "params": {
                "textDocument": { "uri": uri },
                "position": { "line": line, "character": character }
            }
        }));
        assert_eq!(
            definition[0]["result"]["uri"], uri,
            "missing definition for {root}"
        );
        assert_eq!(
            definition[0]["result"]["range"]["start"],
            position(source, declaration),
            "wrong declaration for {root}"
        );
    }

    let provider = source.find("gba\n").unwrap() + 1;
    let (line, character) = position_parts(source, provider);
    let hover = server.handle(json!({
        "jsonrpc": "2.0",
        "id": 131,
        "method": "textDocument/hover",
        "params": {
            "textDocument": { "uri": uri },
            "position": { "line": line, "character": character }
        }
    }));
    let markdown = hover[0]["result"]["contents"]["value"]
        .as_str()
        .expect("provider hover markdown");
    assert!(markdown.contains("state GBA { ... }"));
    assert!(markdown.contains("gba: GBAEmulator"));

    let native_source = concat!(
        "state \"game.exe\" {}\n",
        "onAttach { let executable = process.name() }\n"
    );
    let native_uri = "file:///native-domain-navigation.split";
    server.handle(notification(
        "textDocument/didOpen",
        json!({
            "textDocument": {
                "uri": native_uri,
                "version": 1,
                "text": native_source
            }
        }),
    ));
    let process = native_source.rfind("process").unwrap();
    let (line, character) = position_parts(native_source, process);
    let definition = server.handle(json!({
        "jsonrpc": "2.0",
        "id": 132,
        "method": "textDocument/definition",
        "params": {
            "textDocument": { "uri": native_uri },
            "position": { "line": line, "character": character }
        }
    }));
    assert_eq!(definition[0]["result"]["uri"], native_uri);
    assert_eq!(
        definition[0]["result"]["range"]["start"],
        position(native_source, native_source.find("state").unwrap())
    );
}

#[test]
fn prepare_rename_and_rename_emit_validated_workspace_edits() {
    let source = concat!(
        "// \u{1f98a}\n",
        "state \"game.exe\" {}\n",
        "fn observe(value: i32) { print(value as String) }\n",
        "whileAttached { observe(1) }\n"
    );
    let uri = "file:///rename.split";
    let call = source.rfind("observe").unwrap();
    let (line, character) = position_parts(source, call + "observe".len());
    let mut server = LanguageServer::default();
    initialize(&mut server);
    server.handle(notification(
        "textDocument/didOpen",
        json!({
            "textDocument": {
                "uri": uri,
                "version": 1,
                "text": source
            }
        }),
    ));

    let prepared = server.handle(json!({
        "jsonrpc": "2.0",
        "id": 16,
        "method": "textDocument/prepareRename",
        "params": {
            "textDocument": { "uri": uri },
            "position": { "line": line, "character": character }
        }
    }));
    assert_eq!(prepared[0]["result"]["placeholder"], "observe");
    assert_eq!(
        prepared[0]["result"]["range"]["start"],
        position(source, call)
    );

    let renamed = server.handle(json!({
        "jsonrpc": "2.0",
        "id": 17,
        "method": "textDocument/rename",
        "params": {
            "textDocument": { "uri": uri },
            "position": { "line": line, "character": character },
            "newName": "examine"
        }
    }));
    let edits = renamed[0]["result"]["changes"][uri]
        .as_array()
        .expect("workspace edits for the open URI");
    assert_eq!(edits.len(), 2);
    assert!(edits.iter().all(|edit| edit["newText"] == "examine"));

    let reserved = server.handle(json!({
        "jsonrpc": "2.0",
        "id": 18,
        "method": "textDocument/rename",
        "params": {
            "textDocument": { "uri": uri },
            "position": { "line": line, "character": character },
            "newName": "while"
        }
    }));
    assert_eq!(reserved[0]["error"]["code"], -32602);
    assert!(
        reserved[0]["error"]["message"]
            .as_str()
            .unwrap()
            .contains("reserved")
    );

    let print = source.find("print").unwrap();
    let (line, character) = position_parts(source, print + 1);
    let catalog = server.handle(json!({
        "jsonrpc": "2.0",
        "id": 19,
        "method": "textDocument/prepareRename",
        "params": {
            "textDocument": { "uri": uri },
            "position": { "line": line, "character": character }
        }
    }));
    assert_eq!(catalog[0]["result"], Value::Null);
}

#[test]
fn managed_rename_emits_external_identity_preservation_edits() {
    let source = r#"image "Assembly-CSharp" {
    class Player {
        static Player instance;
        u32 score;
    }
}
state Unity ["game.exe"] {
    score: u32 = Player.instance?.score?
}
"#;
    let uri = "file:///managed-rename.split";
    let mut server = LanguageServer::default();
    initialize(&mut server);
    server.handle(notification(
        "textDocument/didOpen",
        json!({
            "textDocument": {
                "uri": uri,
                "version": 1,
                "text": source
            }
        }),
    ));

    let score = source.rfind("score?").unwrap();
    let (line, character) = position_parts(source, score + 1);
    let renamed = server.handle(json!({
        "jsonrpc": "2.0",
        "id": 171,
        "method": "textDocument/rename",
        "params": {
            "textDocument": { "uri": uri },
            "position": { "line": line, "character": character },
            "newName": "points"
        }
    }));
    let edits = renamed[0]["result"]["changes"][uri]
        .as_array()
        .expect("managed field rename edits");
    assert_eq!(edits.len(), 3);
    assert_eq!(
        edits
            .iter()
            .filter_map(|edit| edit["newText"].as_str())
            .collect::<Vec<_>>(),
        [
            "points",
            " from [\"score\", \"<score>k__BackingField\"]",
            "points",
        ]
    );
}

#[test]
fn setting_rename_emits_stable_key_preservation_edit() {
    let source = r#"state "game.exe" {}
settings {
    "Automatic splitting" => autoSplit: true,
}
whileAttached {
    print(settings.autoSplit)
}
"#;
    let uri = "file:///setting-rename.split";
    let mut server = LanguageServer::default();
    initialize(&mut server);
    server.handle(notification(
        "textDocument/didOpen",
        json!({
            "textDocument": {
                "uri": uri,
                "version": 1,
                "text": source
            }
        }),
    ));

    let use_site = source.rfind("autoSplit").unwrap();
    let (line, character) = position_parts(source, use_site + 1);
    let renamed = server.handle(json!({
        "jsonrpc": "2.0",
        "id": 172,
        "method": "textDocument/rename",
        "params": {
            "textDocument": { "uri": uri },
            "position": { "line": line, "character": character },
            "newName": "splitEnabled"
        }
    }));
    let edits = renamed[0]["result"]["changes"][uri]
        .as_array()
        .expect("setting rename edits");
    assert_eq!(edits.len(), 3);
    assert_eq!(
        edits
            .iter()
            .filter_map(|edit| edit["newText"].as_str())
            .collect::<Vec<_>>(),
        ["splitEnabled", " key \"autoSplit\"", "splitEnabled"]
    );
}

#[test]
fn document_symbols_and_code_actions_preserve_compiler_structure() {
    let symbols_source = concat!(
        "struct Point { x: i32 }\n",
        "state \"game.exe\" { level = process.read<i32>(0) }\n",
        "settings { \"General\" { \"Enabled\" => enabled: true } }\n",
        "whileAttached {}\n"
    );
    let symbols_uri = "file:///symbols.split";
    let mut server = LanguageServer::default();
    initialize(&mut server);
    server.handle(notification(
        "textDocument/didOpen",
        json!({
            "textDocument": {
                "uri": symbols_uri,
                "version": 1,
                "text": symbols_source
            }
        }),
    ));
    let symbols = server.handle(json!({
        "jsonrpc": "2.0",
        "id": 20,
        "method": "textDocument/documentSymbol",
        "params": { "textDocument": { "uri": symbols_uri } }
    }));
    let outline = symbols[0]["result"].as_array().unwrap();
    assert_eq!(
        outline
            .iter()
            .map(|symbol| symbol["name"].as_str().unwrap())
            .collect::<Vec<_>>(),
        ["Point", "state", "settings", "whileAttached"]
    );
    assert_eq!(outline[0]["kind"], 23);
    assert_eq!(outline[0]["children"][0]["name"], "x");
    assert_eq!(outline[2]["children"][0]["name"], "General");
    assert_eq!(outline[2]["children"][0]["children"][0]["name"], "enabled");

    let broken = "state \"game.exe\" {}\nwhileAttached { let value: i32?? = None }\n";
    let broken_uri = "file:///fix.split";
    let diagnostics = server.handle(notification(
        "textDocument/didOpen",
        json!({
            "textDocument": {
                "uri": broken_uri,
                "version": 1,
                "text": broken
            }
        }),
    ));
    assert_eq!(
        diagnostics[0]["params"]["diagnostics"][0]["data"]["fixes"][0]["title"],
        "remove the duplicate wrapper"
    );
    let actions = server.handle(json!({
        "jsonrpc": "2.0",
        "id": 21,
        "method": "textDocument/codeAction",
        "params": {
            "textDocument": { "uri": broken_uri },
            "range": {
                "start": { "line": 1, "character": 0 },
                "end": position(broken, broken.len())
            },
            "context": {
                "diagnostics": diagnostics[0]["params"]["diagnostics"],
                "only": ["quickfix"]
            }
        }
    }));
    let quick_fixes = actions[0]["result"].as_array().unwrap();
    assert_eq!(quick_fixes.len(), 1);
    assert_eq!(quick_fixes[0]["title"], "remove the duplicate wrapper");
    assert_eq!(quick_fixes[0]["kind"], "quickfix");
    assert_eq!(quick_fixes[0]["isPreferred"], true);
    assert_eq!(
        quick_fixes[0]["edit"]["changes"][broken_uri][0]["newText"],
        ""
    );

    let unrelated = server.handle(json!({
        "jsonrpc": "2.0",
        "id": 22,
        "method": "textDocument/codeAction",
        "params": {
            "textDocument": { "uri": broken_uri },
            "range": {
                "start": { "line": 0, "character": 0 },
                "end": position(broken, broken.len())
            },
            "context": { "diagnostics": [], "only": ["source"] }
        }
    }));
    assert!(unrelated[0]["result"].as_array().unwrap().is_empty());
}

#[test]
fn code_actions_extract_selected_expressions() {
    let source = concat!(
        "state \"game.exe\" {}\n",
        "fn score(offset: i32) {\n",
        "    return offset + 1\n",
        "}\n"
    );
    let uri = "file:///refactor.split";
    let mut server = LanguageServer::default();
    initialize(&mut server);
    server.handle(notification(
        "textDocument/didOpen",
        json!({
            "textDocument": {
                "uri": uri,
                "version": 1,
                "text": source
            }
        }),
    ));

    let start = source.find("offset + 1").unwrap();
    let end = start + "offset + 1".len();
    let (start_line, start_character) = position_parts(source, start);
    let (end_line, end_character) = position_parts(source, end);
    let actions = server.handle(json!({
        "jsonrpc": "2.0",
        "id": 24,
        "method": "textDocument/codeAction",
        "params": {
            "textDocument": { "uri": uri },
            "range": {
                "start": { "line": start_line, "character": start_character },
                "end": { "line": end_line, "character": end_character }
            },
            "context": { "diagnostics": [], "only": ["refactor.extract"] }
        }
    }));
    let actions = actions[0]["result"].as_array().unwrap();
    assert_eq!(actions.len(), 2, "{actions:#?}");
    assert_eq!(actions[0]["kind"], "refactor.extract.variable");
    assert_eq!(actions[1]["kind"], "refactor.extract.function");
    assert_eq!(
        actions[1]["edit"]["changes"][uri][0]["newText"],
        "extracted(offset)"
    );

    let function_only = server.handle(json!({
        "jsonrpc": "2.0",
        "id": 25,
        "method": "textDocument/codeAction",
        "params": {
            "textDocument": { "uri": uri },
            "range": {
                "start": { "line": start_line, "character": start_character },
                "end": { "line": end_line, "character": end_character }
            },
            "context": {
                "diagnostics": [],
                "only": ["refactor.extract.function"]
            }
        }
    }));
    let function_only = function_only[0]["result"].as_array().unwrap();
    assert_eq!(function_only.len(), 1);
    assert_eq!(function_only[0]["kind"], "refactor.extract.function");

    let quick_fixes = server.handle(json!({
        "jsonrpc": "2.0",
        "id": 26,
        "method": "textDocument/codeAction",
        "params": {
            "textDocument": { "uri": uri },
            "range": {
                "start": { "line": start_line, "character": start_character },
                "end": { "line": end_line, "character": end_character }
            },
            "context": { "diagnostics": [], "only": ["quickfix"] }
        }
    }));
    assert!(quick_fixes[0]["result"].as_array().unwrap().is_empty());

    let statement_source = concat!(
        "state \"game.exe\" {}\n",
        "fn report(value: i32) {\n",
        "    print(value)\n",
        "    print(value + 1)\n",
        "}\n"
    );
    let statement_uri = "file:///statement-refactor.split";
    server.handle(notification(
        "textDocument/didOpen",
        json!({
            "textDocument": {
                "uri": statement_uri,
                "version": 1,
                "text": statement_source
            }
        }),
    ));
    let statement_start = statement_source.find("print(value)").unwrap();
    let statement_end =
        statement_source.find("print(value + 1)").unwrap() + "print(value + 1)".len();
    let (statement_start_line, statement_start_character) =
        position_parts(statement_source, statement_start);
    let (statement_end_line, statement_end_character) =
        position_parts(statement_source, statement_end);
    let statements = server.handle(json!({
        "jsonrpc": "2.0",
        "id": 27,
        "method": "textDocument/codeAction",
        "params": {
            "textDocument": { "uri": statement_uri },
            "range": {
                "start": {
                    "line": statement_start_line,
                    "character": statement_start_character
                },
                "end": {
                    "line": statement_end_line,
                    "character": statement_end_character
                }
            },
            "context": {
                "diagnostics": [],
                "only": ["refactor.extract.function"]
            }
        }
    }));
    let statements = statements[0]["result"].as_array().unwrap();
    assert_eq!(statements.len(), 1, "{statements:#?}");
    assert_eq!(
        statements[0]["edit"]["changes"][statement_uri][0]["newText"],
        "extracted(value)"
    );
}

#[test]
fn code_actions_add_and_remove_expression_inspection() {
    let source = "state \"game.exe\" {}\nfn score(offset: i32) { return offset + 1 }\n";
    let uri = "file:///inspect-refactor.split";
    let mut server = LanguageServer::default();
    initialize(&mut server);
    server.handle(notification(
        "textDocument/didOpen",
        json!({
            "textDocument": {
                "uri": uri,
                "version": 1,
                "text": source
            }
        }),
    ));

    let start = source.find("offset + 1").unwrap();
    let end = start + "offset + 1".len();
    let actions = server.handle(json!({
        "jsonrpc": "2.0",
        "id": 27,
        "method": "textDocument/codeAction",
        "params": {
            "textDocument": { "uri": uri },
            "range": {
                "start": position(source, start),
                "end": position(source, end)
            },
            "context": { "diagnostics": [], "only": ["refactor.rewrite"] }
        }
    }));
    let actions = actions[0]["result"].as_array().unwrap();
    assert_eq!(actions.len(), 1, "{actions:#?}");
    assert_eq!(actions[0]["title"], "Inspect expression");
    assert_eq!(actions[0]["kind"], "refactor.rewrite.inspect.add");
    assert_eq!(
        actions[0]["edit"]["changes"][uri][0]["newText"],
        "inspect(offset + 1)"
    );

    let inspected = "state \"game.exe\" {}\nfn score(offset: i32) { return inspect(offset + 1) }\n";
    server.handle(notification(
        "textDocument/didChange",
        json!({
            "textDocument": { "uri": uri, "version": 2 },
            "contentChanges": [{ "text": inspected }]
        }),
    ));
    let cursor = inspected.find("inspect(offset + 1)").unwrap() + 2;
    let actions = server.handle(json!({
        "jsonrpc": "2.0",
        "id": 28,
        "method": "textDocument/codeAction",
        "params": {
            "textDocument": { "uri": uri },
            "range": {
                "start": position(inspected, cursor),
                "end": position(inspected, cursor)
            },
            "context": { "diagnostics": [], "only": ["refactor.rewrite"] }
        }
    }));
    let actions = actions[0]["result"].as_array().unwrap();
    assert_eq!(actions.len(), 1, "{actions:#?}");
    assert_eq!(actions[0]["title"], "Remove inspection");
    assert_eq!(actions[0]["kind"], "refactor.rewrite.inspect.remove");
    assert_eq!(
        actions[0]["edit"]["changes"][uri][0]["newText"],
        "(offset + 1)"
    );

    server.handle(notification(
        "textDocument/didChange",
        json!({
            "textDocument": { "uri": uri, "version": 3 },
            "contentChanges": [{ "text": source }]
        }),
    ));
    let cursor = source.find("offset + 1").unwrap() + 2;
    let actions = server.handle(json!({
        "jsonrpc": "2.0",
        "id": 29,
        "method": "textDocument/codeAction",
        "params": {
            "textDocument": { "uri": uri },
            "range": {
                "start": position(source, cursor),
                "end": position(source, cursor)
            },
            "context": { "diagnostics": [], "only": ["refactor.rewrite"] }
        }
    }));
    let actions = actions[0]["result"].as_array().unwrap();
    assert_eq!(actions.len(), 1, "{actions:#?}");
    assert_eq!(actions[0]["title"], "Inspect expression");
    assert_eq!(
        actions[0]["edit"]["changes"][uri][0]["newText"],
        "inspect(offset)"
    );
}

#[test]
fn unused_member_code_actions_apply_validated_multi_edit_suppressions() {
    let source = concat!(
        "struct Pair {\n",
        "    used: i32,\n",
        "    unused: i32,\n",
        "}\n",
        "state \"game.exe\" {}\n",
        "fn pair() -> Pair { return Pair { used: 1, unused: 2 } }\n",
        "whileAttached { pair().used }\n"
    );
    let uri = "file:///unused-member.split";
    let mut server = LanguageServer::default();
    initialize(&mut server);
    let diagnostics = server.handle(notification(
        "textDocument/didOpen",
        json!({
            "textDocument": {
                "uri": uri,
                "version": 1,
                "text": source
            }
        }),
    ));
    let published = diagnostics[0]["params"]["diagnostics"].as_array().unwrap();
    assert_eq!(published.len(), 1, "{published:#?}");
    assert_eq!(published[0]["code"], "SS1004", "{published:#?}");

    let actions = server.handle(json!({
        "jsonrpc": "2.0",
        "id": 23,
        "method": "textDocument/codeAction",
        "params": {
            "textDocument": { "uri": uri },
            "range": {
                "start": { "line": 0, "character": 0 },
                "end": position(source, source.len())
            },
            "context": {
                "diagnostics": published,
                "only": ["quickfix"]
            }
        }
    }));
    let quick_fixes = actions[0]["result"].as_array().unwrap();
    assert_eq!(quick_fixes.len(), 1, "{quick_fixes:#?}");
    assert_eq!(quick_fixes[0]["title"], "rename `unused` to `_unused`");
    assert_eq!(quick_fixes[0]["isPreferred"], true);
    let edits = quick_fixes[0]["edit"]["changes"][uri].as_array().unwrap();
    assert_eq!(edits.len(), 2);
    assert!(edits.iter().all(|edit| edit["newText"] == "_unused"));
}

#[test]
fn unused_struct_field_fix_survives_save_undo_and_save() {
    let unformatted = concat!(
        "state \"explorer.exe\" {}\n",
        "struct Pos {\n",
        "    x: u16,\n",
        "    y: u16,\n",
        "}\n",
        "fn Pos.toString() { return `({self.x}, {self.y})` }\n",
        "setup {\n",
        "    let x = 5\n",
        "    observe([Pos { x, y: 2 }, Pos { x: 3, y: 4 }])\n",
        "}\n",
        "fn observe(values) { print(values) }\n",
    );
    let source = crate::database::CompilerDatabase::new(unformatted)
        .format()
        .expect("fixture formatting")
        .to_string();
    let source = source.as_str();
    let uri = "file:///unused-struct-field-undo.split";
    let mut server = LanguageServer::default();
    initialize(&mut server);
    let diagnostics = server.handle(notification(
        "textDocument/didOpen",
        json!({
            "textDocument": {
                "uri": uri,
                "version": 1,
                "text": source
            }
        }),
    ));
    let published = diagnostics[0]["params"]["diagnostics"].as_array().unwrap();
    let x_offset = source.find("x: u16").unwrap();
    let (x_line, x_character) = position_parts(source, x_offset);
    let actions = server.handle(json!({
        "jsonrpc": "2.0",
        "id": 25,
        "method": "textDocument/codeAction",
        "params": {
            "textDocument": { "uri": uri },
            "range": {
                "start": { "line": x_line, "character": x_character },
                "end": { "line": x_line, "character": x_character + 1 }
            },
            "context": {
                "diagnostics": published,
                "only": ["quickfix"]
            }
        }
    }));
    let action = &actions[0]["result"][0];
    assert_eq!(action["title"], "rename `x` to `_x`");

    let mut fixed = source.to_owned();
    let mut edits = action["edit"]["changes"][uri]
        .as_array()
        .unwrap()
        .iter()
        .map(|edit| {
            let start = &edit["range"]["start"];
            let end = &edit["range"]["end"];
            let start = offset_at_position(
                source,
                start["line"].as_u64().unwrap() as u32,
                start["character"].as_u64().unwrap() as u32,
            )
            .unwrap();
            let end = offset_at_position(
                source,
                end["line"].as_u64().unwrap() as u32,
                end["character"].as_u64().unwrap() as u32,
            )
            .unwrap();
            (start, end, edit["newText"].as_str().unwrap().to_owned())
        })
        .collect::<Vec<_>>();
    edits.sort_by_key(|(start, end, _)| (*start, *end));
    for (start, end, replacement) in edits.into_iter().rev() {
        fixed.replace_range(start..end, &replacement);
    }
    assert!(fixed.contains("_x: u16"));

    server.handle(notification(
        "textDocument/didChange",
        json!({
            "textDocument": { "uri": uri, "version": 2 },
            "contentChanges": [{ "text": fixed }]
        }),
    ));
    let first_save = server.handle(json!({
        "jsonrpc": "2.0",
        "id": 26,
        "method": "textDocument/formatting",
        "params": {
            "textDocument": { "uri": uri },
            "options": { "tabSize": 4, "insertSpaces": true }
        }
    }));
    assert_eq!(first_save[0]["result"], json!([]));

    // Undo restores the exact earlier source but with a monotonically newer
    // LSP document version. A second save must not strand any compiler query.
    let undo_diagnostics = server.handle(notification(
        "textDocument/didChange",
        json!({
            "textDocument": { "uri": uri, "version": 3 },
            "contentChanges": [{ "text": source }]
        }),
    ));
    let republished = undo_diagnostics[0]["params"]["diagnostics"]
        .as_array()
        .unwrap();
    let second_save = server.handle(json!({
        "jsonrpc": "2.0",
        "id": 27,
        "method": "textDocument/formatting",
        "params": {
            "textDocument": { "uri": uri },
            "options": { "tabSize": 4, "insertSpaces": true }
        }
    }));
    assert_eq!(second_save[0]["result"], json!([]));

    let y_offset = source.find("y: u16").unwrap();
    let (y_line, y_character) = position_parts(source, y_offset);
    let actions = server.handle(json!({
        "jsonrpc": "2.0",
        "id": 28,
        "method": "textDocument/codeAction",
        "params": {
            "textDocument": { "uri": uri },
            "range": {
                "start": { "line": y_line, "character": y_character },
                "end": { "line": y_line, "character": y_character + 1 }
            },
            "context": { "diagnostics": republished, "only": ["quickfix"] }
        }
    }));
    assert_eq!(actions[0]["result"][0]["title"], "rename `y` to `_y`");
    let hover = server.handle(json!({
        "jsonrpc": "2.0",
        "id": 29,
        "method": "textDocument/hover",
        "params": {
            "textDocument": { "uri": uri },
            "position": { "line": y_line, "character": y_character }
        }
    }));
    assert!(!hover[0]["result"].is_null());
}

#[test]
fn unused_shared_state_field_has_one_validated_multi_layout_suppression() {
    let source = concat!(
        "enum Build { Steam, GOG }\n",
        "let build: Build\n",
        "state \"game.exe\" {\n",
        "    if build == Build.Steam { level: u32 at 0x100; spare: u32 at 0x104; }\n",
        "    else { level: u32 at 0x200; spare: u32 at 0x204; }\n",
        "}\n",
        "onAttach { build = Build.Steam }\n",
        "split { return current.level != old.level }\n"
    );
    let uri = "file:///unused-state-field.split";
    let mut server = LanguageServer::default();
    initialize(&mut server);
    let diagnostics = server.handle(notification(
        "textDocument/didOpen",
        json!({
            "textDocument": {
                "uri": uri,
                "version": 1,
                "text": source
            }
        }),
    ));
    let published = diagnostics[0]["params"]["diagnostics"].as_array().unwrap();
    assert_eq!(published.len(), 1, "{published:#?}");
    assert_eq!(published[0]["code"], "SS1004", "{published:#?}");
    assert!(
        published[0]["message"]
            .as_str()
            .unwrap()
            .starts_with("unused state field `spare`"),
        "{published:#?}"
    );
    assert_eq!(
        published[0]["relatedInformation"].as_array().unwrap().len(),
        2,
        "{published:#?}"
    );

    let actions = server.handle(json!({
        "jsonrpc": "2.0",
        "id": 24,
        "method": "textDocument/codeAction",
        "params": {
            "textDocument": { "uri": uri },
            "range": {
                "start": { "line": 0, "character": 0 },
                "end": position(source, source.len())
            },
            "context": {
                "diagnostics": published,
                "only": ["quickfix"]
            }
        }
    }));
    let quick_fixes = actions[0]["result"].as_array().unwrap();
    assert_eq!(quick_fixes.len(), 1, "{quick_fixes:#?}");
    assert_eq!(quick_fixes[0]["title"], "rename `spare` to `_spare`");
    assert_eq!(quick_fixes[0]["isPreferred"], true);
    let edits = quick_fixes[0]["edit"]["changes"][uri].as_array().unwrap();
    assert_eq!(edits.len(), 2, "{edits:#?}");
    assert!(edits.iter().all(|edit| edit["newText"] == "_spare"));
}

#[test]
fn struct_field_shorthand_fixes_and_renames_preserve_both_identities() {
    let source = concat!(
        "struct Point { x: u32 }\n",
        "state \"game.exe\" {}\n",
        "fn point(x: u32) -> Point { return Point { x: x } }\n",
        "setup { print(point(1).x) }\n"
    );
    let uri = "file:///struct-shorthand.split";
    let mut server = LanguageServer::default();
    initialize(&mut server);
    let diagnostics = server.handle(notification(
        "textDocument/didOpen",
        json!({
            "textDocument": {
                "uri": uri,
                "version": 1,
                "text": source
            }
        }),
    ));
    let published = diagnostics[0]["params"]["diagnostics"].as_array().unwrap();
    let shorthand = published
        .iter()
        .find(|diagnostic| diagnostic["code"] == "SS1010")
        .expect("the repeated field should publish shorthand guidance");
    let actions = server.handle(json!({
        "jsonrpc": "2.0",
        "id": 25,
        "method": "textDocument/codeAction",
        "params": {
            "textDocument": { "uri": uri },
            "range": shorthand["range"],
            "context": {
                "diagnostics": [shorthand],
                "only": ["quickfix"]
            }
        }
    }));
    let quick_fixes = actions[0]["result"].as_array().unwrap();
    assert_eq!(quick_fixes.len(), 1, "{quick_fixes:#?}");
    assert_eq!(quick_fixes[0]["title"], "shorten to `x`");
    assert_eq!(quick_fixes[0]["isPreferred"], true);
    assert_eq!(quick_fixes[0]["edit"]["changes"][uri][0]["newText"], "");

    let shorthand_source = source.replacen("x: x", "x", 1);
    let shorthand_uri = "file:///struct-shorthand-rename.split";
    server.handle(notification(
        "textDocument/didOpen",
        json!({
            "textDocument": {
                "uri": shorthand_uri,
                "version": 1,
                "text": shorthand_source
            }
        }),
    ));
    let field = shorthand_source.find("x: u32 }").unwrap();
    let (line, character) = position_parts(&shorthand_source, field);
    let renamed = server.handle(json!({
        "jsonrpc": "2.0",
        "id": 26,
        "method": "textDocument/rename",
        "params": {
            "textDocument": { "uri": shorthand_uri },
            "position": { "line": line, "character": character },
            "newName": "horizontal"
        }
    }));
    let edits = renamed[0]["result"]["changes"][shorthand_uri]
        .as_array()
        .expect("field rename edits");
    assert!(edits.iter().any(|edit| edit["newText"] == "horizontal: x"));

    let pattern_source = concat!(
        "struct Pos { x: u16, y: u16 }\n",
        "state \"game.exe\" {}\n",
        "fn use(Pos { x, y: y }) -> u16 { return x + y }\n"
    );
    let pattern_uri = "file:///binding-pattern-shorthand-rename.split";
    let diagnostics = server.handle(notification(
        "textDocument/didOpen",
        json!({
            "textDocument": {
                "uri": pattern_uri,
                "version": 1,
                "text": pattern_source
            }
        }),
    ));
    let shorthand = diagnostics[0]["params"]["diagnostics"]
        .as_array()
        .unwrap()
        .iter()
        .find(|diagnostic| diagnostic["code"] == "SS1010")
        .expect("the repeated pattern binding should publish shorthand guidance");
    let actions = server.handle(json!({
        "jsonrpc": "2.0",
        "id": 27,
        "method": "textDocument/codeAction",
        "params": {
            "textDocument": { "uri": pattern_uri },
            "range": shorthand["range"],
            "context": {
                "diagnostics": [shorthand],
                "only": ["quickfix"]
            }
        }
    }));
    let quick_fixes = actions[0]["result"].as_array().unwrap();
    assert_eq!(quick_fixes[0]["title"], "shorten to `y`");
    assert_eq!(quick_fixes[0]["isPreferred"], true);
    assert_eq!(
        quick_fixes[0]["edit"]["changes"][pattern_uri][0]["newText"],
        ""
    );

    let field = pattern_source.find("x: u16").unwrap();
    let (line, character) = position_parts(pattern_source, field);
    let renamed = server.handle(json!({
        "jsonrpc": "2.0",
        "id": 28,
        "method": "textDocument/rename",
        "params": {
            "textDocument": { "uri": pattern_uri },
            "position": { "line": line, "character": character },
            "newName": "horizontal"
        }
    }));
    let edits = renamed[0]["result"]["changes"][pattern_uri]
        .as_array()
        .expect("binding-pattern field rename edits");
    assert!(edits.iter().any(|edit| edit["newText"] == "horizontal: x"));
}
