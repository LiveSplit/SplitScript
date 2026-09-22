//! Language Server Protocol state backed by [`crate::database::CompilerDatabase`].
//!
//! Transport framing lives in the `splitls` binary. Keeping message handling
//! here makes the protocol behavior directly testable and leaves compiler
//! queries independent of JSON-RPC.

use serde_json::{Value, json};

mod conversion;
mod documents;
mod protocol;

use conversion::{
    completion_list_json, diagnostic_json, document_highlight_json, document_symbol_json,
    hover_json, inlay_hint_json, location_json, offset_at_position, position, selection_range_json,
    semantic_token_data, signature_help_json,
};
use documents::{Document, DocumentStore};
use protocol::{
    CodeActionParams, DidChangeParams, DidOpenParams, DocumentationPageParams,
    DocumentationSearchParams, FormattingParams, IncomingMessage, InlayHintParams, ReferenceParams,
    RenameParams, SelectionRangeParams, TextDocumentParams, TextDocumentPositionParams, decode,
};

use crate::{
    DiagnosticCode, DiagnosticFix, FixApplicability,
    database::DefinitionTarget,
    documentation::DocumentationReference,
    highlight::{SEMANTIC_TOKEN_MODIFIERS, SemanticTokenKind},
};

/// Stateful single-process LSP handler for open SplitScript documents.
#[derive(Default)]
pub struct LanguageServer {
    documents: DocumentStore,
    documentation: DocumentationReference,
    initialized: bool,
    shutdown_requested: bool,
    should_exit: bool,
}

impl LanguageServer {
    /// Handles one decoded JSON-RPC message and returns zero or more outgoing
    /// responses and notifications.
    pub fn handle(&mut self, message: Value) -> Vec<Value> {
        let fallback_id = message.get("id").cloned();
        let IncomingMessage { id, method, params } = match decode(message) {
            Ok(message) => message,
            Err(error) => {
                return fallback_id
                    .map_or_else(Vec::new, |id| vec![error_response(id, -32600, error)]);
            }
        };

        if !self.initialized && !matches!(method.as_str(), "initialize" | "exit") {
            return id.map_or_else(Vec::new, |id| {
                vec![error_response(id, -32002, "server is not initialized")]
            });
        }

        if self.shutdown_requested && method != "exit" {
            return id.map_or_else(Vec::new, |id| {
                vec![error_response(id, -32600, "server has shut down")]
            });
        }

        match method.as_str() {
            "initialize" => {
                self.initialized = true;
                id.map_or_else(Vec::new, |id| {
                    vec![response(
                        id,
                        json!({
                            "capabilities": {
                                "positionEncoding": "utf-16",
                                "textDocumentSync": {
                                    "openClose": true,
                                    "change": 1
                                },
                                "documentFormattingProvider": true,
                                "documentSymbolProvider": true,
                                "codeActionProvider": {
                                    "codeActionKinds": ["quickfix", "refactor.extract", "refactor.rewrite"],
                                    "resolveProvider": false
                                },
                                "completionProvider": {
                                    "resolveProvider": false,
                                    "triggerCharacters": ["."]
                                },
                                "hoverProvider": true,
                                "inlayHintProvider": true,
                                "selectionRangeProvider": true,
                                "definitionProvider": true,
                                "typeDefinitionProvider": true,
                                "referencesProvider": true,
                                "documentHighlightProvider": true,
                                "renameProvider": {
                                    "prepareProvider": true
                                },
                                "signatureHelpProvider": {
                                    "triggerCharacters": ["(", ","],
                                    "retriggerCharacters": [","]
                                },
                                "semanticTokensProvider": {
                                    "legend": {
                                        "tokenTypes": SemanticTokenKind::ALL
                                            .iter()
                                            .map(|kind| kind.name())
                                            .collect::<Vec<_>>(),
                                        "tokenModifiers": SEMANTIC_TOKEN_MODIFIERS
                                    },
                                    "full": true
                                }
                            },
                            "serverInfo": {
                                "name": "splitls",
                                "version": crate::COMPILER_VERSION_TEXT
                            }
                        }),
                    )]
                })
            }
            "initialized" => Vec::new(),
            "shutdown" => {
                self.shutdown_requested = true;
                id.map_or_else(Vec::new, |id| vec![response(id, Value::Null)])
            }
            "exit" => {
                self.should_exit = true;
                Vec::new()
            }
            "textDocument/didOpen" => self.did_open(params),
            "textDocument/didChange" => self.did_change(params),
            "textDocument/didClose" => self.did_close(params),
            "textDocument/formatting" => {
                id.map_or_else(Vec::new, |id| vec![self.formatting_response(id, params)])
            }
            "textDocument/documentSymbol" => id.map_or_else(Vec::new, |id| {
                vec![self.document_symbol_response(id, params)]
            }),
            "textDocument/codeAction" => {
                id.map_or_else(Vec::new, |id| vec![self.code_action_response(id, params)])
            }
            "textDocument/semanticTokens/full" => id.map_or_else(Vec::new, |id| {
                vec![self.semantic_tokens_response(id, params)]
            }),
            "textDocument/completion" => {
                id.map_or_else(Vec::new, |id| vec![self.completion_response(id, params)])
            }
            "textDocument/hover" => {
                id.map_or_else(Vec::new, |id| vec![self.hover_response(id, params)])
            }
            "textDocument/inlayHint" => {
                id.map_or_else(Vec::new, |id| vec![self.inlay_hint_response(id, params)])
            }
            "textDocument/selectionRange" => id.map_or_else(Vec::new, |id| {
                vec![self.selection_range_response(id, params)]
            }),
            "textDocument/definition" => {
                id.map_or_else(Vec::new, |id| vec![self.definition_response(id, params)])
            }
            "textDocument/typeDefinition" => id.map_or_else(Vec::new, |id| {
                vec![self.type_definition_response(id, params)]
            }),
            "textDocument/references" => {
                id.map_or_else(Vec::new, |id| vec![self.references_response(id, params)])
            }
            "textDocument/documentHighlight" => id.map_or_else(Vec::new, |id| {
                vec![self.document_highlight_response(id, params)]
            }),
            "textDocument/prepareRename" => id.map_or_else(Vec::new, |id| {
                vec![self.prepare_rename_response(id, params)]
            }),
            "textDocument/rename" => {
                id.map_or_else(Vec::new, |id| vec![self.rename_response(id, params)])
            }
            "textDocument/signatureHelp" => id.map_or_else(Vec::new, |id| {
                vec![self.signature_help_response(id, params)]
            }),
            "splitscript/documentation/index" => id.map_or_else(Vec::new, |id| {
                vec![response(id, json!(self.documentation.index()))]
            }),
            "splitscript/documentation/search" => id.map_or_else(Vec::new, |id| {
                vec![self.documentation_search_response(id, params)]
            }),
            "splitscript/documentation/page" => id.map_or_else(Vec::new, |id| {
                vec![self.documentation_page_response(id, params)]
            }),
            _ if id.is_some() => vec![error_response(
                id.unwrap(),
                -32601,
                format!("method `{method}` is not supported"),
            )],
            _ => Vec::new(),
        }
    }

    pub const fn should_exit(&self) -> bool {
        self.should_exit
    }

    fn documentation_page_response(&self, id: Value, params: Value) -> Value {
        let params = match decode_request::<DocumentationPageParams>(&id, params) {
            Ok(params) => params,
            Err(response) => return response,
        };
        match self.documentation.page(&params.uri) {
            Some(page) => response(id, json!(page)),
            None => error_response(
                id,
                -32602,
                format!("unknown SplitScript documentation page `{}`", params.uri),
            ),
        }
    }

    fn documentation_search_response(&self, id: Value, params: Value) -> Value {
        let params = match decode_request::<DocumentationSearchParams>(&id, params) {
            Ok(params) => params,
            Err(response) => return response,
        };
        response(id, json!(self.documentation.search(&params.query)))
    }

    fn did_open(&mut self, params: Value) -> Vec<Value> {
        let Ok(params) = decode::<DidOpenParams>(params) else {
            return Vec::new();
        };
        let uri = params.text_document.uri;
        self.documents.open(
            uri.clone(),
            params.text_document.version,
            params.text_document.text,
        );
        vec![self.diagnostics_notification(&uri)]
    }

    fn did_change(&mut self, params: Value) -> Vec<Value> {
        let Ok(mut params) = decode::<DidChangeParams>(params) else {
            return Vec::new();
        };
        let Some(change) = params.content_changes.pop() else {
            return Vec::new();
        };
        let uri = params.text_document.uri;
        if !self
            .documents
            .change(&uri, params.text_document.version, change.text)
        {
            return Vec::new();
        }
        vec![self.diagnostics_notification(&uri)]
    }

    fn did_close(&mut self, params: Value) -> Vec<Value> {
        let Ok(params) = decode::<TextDocumentParams>(params) else {
            return Vec::new();
        };
        let uri = params.text_document.uri;
        self.documents.close(&uri);
        vec![json!({
            "jsonrpc": "2.0",
            "method": "textDocument/publishDiagnostics",
            "params": {
                "uri": uri,
                "diagnostics": []
            }
        })]
    }

    fn formatting_response(&mut self, id: Value, params: Value) -> Value {
        let params = match decode_request::<FormattingParams>(&id, params) {
            Ok(params) => params,
            Err(response) => return response,
        };
        let Some(document) = self.documents.get_mut(&params.text_document.uri) else {
            return error_response(id, -32602, "text document is not open");
        };
        let source = document.database.source().to_owned();
        let mut options = crate::FormatOptions {
            indent_style: if params.options.insert_spaces {
                crate::IndentStyle::Spaces
            } else {
                crate::IndentStyle::Tabs
            },
            indent_width: params.options.tab_size.max(1),
            ..crate::FormatOptions::default()
        };
        let configured = params.options.splitscript;
        if let Some(line_ending) = configured
            .document_line_ending
            .as_deref()
            .and_then(parse_line_ending)
        {
            options.line_ending = line_ending;
        }
        if let Some(insert) = configured.files_insert_final_newline {
            options.insert_final_newline = insert;
        }
        let layers = configured
            .editor_config
            .into_iter()
            .map(|layer| crate::EditorConfigLayer {
                relative_path: layer.relative_path,
                source: layer.source,
            })
            .collect::<Vec<_>>();
        options = crate::resolve_editorconfig(options, &layers);
        if let Some(width) = configured.max_line_width.filter(|width| *width > 0) {
            options.max_line_width = width;
        }
        if let Some(style) = configured.indent_style.as_deref() {
            options.indent_style = match style {
                "spaces" => crate::IndentStyle::Spaces,
                "tabs" => crate::IndentStyle::Tabs,
                _ => options.indent_style,
            };
        }
        if let Some(width) = configured.indent_width.filter(|width| *width > 0) {
            options.indent_width = width;
        }
        if let Some(line_ending) = configured
            .line_ending
            .as_deref()
            .and_then(parse_line_ending)
        {
            options.line_ending = line_ending;
        }
        if let Some(insert) = configured.insert_final_newline {
            options.insert_final_newline = insert;
        }
        let Ok(formatted) = document.database.format_with_options(options) else {
            return response(id, json!([]));
        };
        if *formatted == source {
            return response(id, json!([]));
        }
        response(
            id,
            json!([{
                "range": {
                    "start": { "line": 0, "character": 0 },
                    "end": position(&source, source.len())
                },
                "newText": &*formatted
            }]),
        )
    }

    fn document_symbol_response(&mut self, id: Value, params: Value) -> Value {
        let params = match decode_request::<TextDocumentParams>(&id, params) {
            Ok(params) => params,
            Err(response) => return response,
        };
        let Some(document) = self.documents.get_mut(&params.text_document.uri) else {
            return error_response(id, -32602, "text document is not open");
        };
        let source = document.database.source().to_owned();
        let Ok(symbols) = document.database.document_symbols() else {
            return response(id, json!([]));
        };
        response(
            id,
            Value::Array(
                symbols
                    .iter()
                    .map(|symbol| document_symbol_json(&source, symbol))
                    .collect(),
            ),
        )
    }

    fn code_action_response(&mut self, id: Value, params: Value) -> Value {
        let params = match decode_request::<CodeActionParams>(&id, params) {
            Ok(params) => params,
            Err(response) => return response,
        };
        let uri = params.text_document.uri;
        let permits = |requested: &str| {
            params.context.only.as_ref().is_none_or(|kinds| {
                kinds.is_empty()
                    || kinds.iter().any(|kind| {
                        requested == kind
                            || requested.starts_with(&format!("{kind}."))
                            || kind.starts_with(&format!("{requested}."))
                    })
            })
        };
        let permits_quick_fixes = permits("quickfix");
        let permits_extractions = permits("refactor.extract");
        let permits_rewrites = permits("refactor.rewrite");
        if !permits_quick_fixes && !permits_extractions && !permits_rewrites {
            return response(id, json!([]));
        }
        let Some(document) = self.documents.get_mut(&uri) else {
            return error_response(id, -32602, "text document is not open");
        };
        let source = document.database.source().to_owned();
        let (Some(start), Some(end)) = (
            offset_at_position(
                &source,
                params.range.start.line,
                params.range.start.character,
            ),
            offset_at_position(&source, params.range.end.line, params.range.end.character),
        ) else {
            return error_response(id, -32602, "invalid code-action range");
        };
        let mut actions = Vec::new();
        if permits_quick_fixes {
            let diagnostics = document.database.diagnostics();
            for diagnostic in diagnostics
                .iter()
                .filter(|diagnostic| diagnostic.span.start <= end && start <= diagnostic.span.end)
            {
                let mut fixes = diagnostic.fixes.clone();
                if fixes.is_empty()
                    && matches!(
                        diagnostic.code,
                        DiagnosticCode::UnusedDeclaration | DiagnosticCode::UnusedMember
                    )
                    && let Ok(Some(plan)) = document
                        .database
                        .underscore_suppression_at(diagnostic.span.start)
                {
                    let original = &source[diagnostic.span.start..diagnostic.span.end];
                    fixes.push(DiagnosticFix {
                        title: format!("rename `{original}` to `{}`", plan.new_name),
                        applicability: FixApplicability::MachineApplicable,
                        edits: plan.edits,
                    });
                }
                for fix in fixes.iter() {
                    let edits = fix
                        .edits
                        .iter()
                        .map(|edit| {
                            json!({
                                "range": {
                                    "start": position(&source, edit.span.start),
                                    "end": position(&source, edit.span.end)
                                },
                                "newText": edit.replacement
                            })
                        })
                        .collect::<Vec<_>>();
                    let mut changes = serde_json::Map::new();
                    changes.insert(uri.clone(), Value::Array(edits));
                    actions.push(json!({
                        "title": fix.title,
                        "kind": "quickfix",
                        "diagnostics": [diagnostic_json(&uri, &source, diagnostic)],
                        "isPreferred": fix.applicability == FixApplicability::MachineApplicable,
                        "edit": { "changes": changes },
                        "data": { "applicability": fix.applicability.to_string() }
                    }));
                }
            }
        }
        if (permits_extractions || permits_rewrites)
            && let Ok(refactorings) = document
                .database
                .refactorings(crate::ast::Span { start, end })
        {
            for refactoring in refactorings
                .into_iter()
                .filter(|refactoring| permits(refactoring.kind.lsp_kind()))
            {
                let edits = refactoring
                    .edits
                    .iter()
                    .map(|edit| {
                        json!({
                            "range": {
                                "start": position(&source, edit.span.start),
                                "end": position(&source, edit.span.end)
                            },
                            "newText": edit.replacement
                        })
                    })
                    .collect::<Vec<_>>();
                let mut changes = serde_json::Map::new();
                changes.insert(uri.clone(), Value::Array(edits));
                actions.push(json!({
                    "title": refactoring.title,
                    "kind": refactoring.kind.lsp_kind(),
                    "edit": { "changes": changes }
                }));
            }
        }
        response(id, Value::Array(actions))
    }

    fn diagnostics_notification(&mut self, uri: &str) -> Value {
        let Some(document) = self.documents.get_mut(uri) else {
            return Value::Null;
        };
        let diagnostics = document.database.diagnostics();
        let source = document.database.source();
        let diagnostics = diagnostics
            .iter()
            .map(|diagnostic| diagnostic_json(uri, source, diagnostic))
            .collect::<Vec<_>>();
        let mut params = json!({
            "uri": uri,
            "diagnostics": diagnostics
        });
        if let Some(version) = document.version {
            params["version"] = json!(version);
        }
        json!({
            "jsonrpc": "2.0",
            "method": "textDocument/publishDiagnostics",
            "params": params
        })
    }

    fn semantic_tokens_response(&mut self, id: Value, params: Value) -> Value {
        let params = match decode_request::<TextDocumentParams>(&id, params) {
            Ok(params) => params,
            Err(response) => return response,
        };
        let Some(document) = self.documents.get_mut(&params.text_document.uri) else {
            return error_response(id, -32602, "text document is not open");
        };
        let Ok(highlights) = document.database.semantic_highlights() else {
            return response(id, json!({ "data": [] }));
        };
        response(
            id,
            json!({
                "data": semantic_token_data(
                    document.database.source(),
                    highlights.highlights()
                )
            }),
        )
    }

    fn completion_response(&mut self, id: Value, params: Value) -> Value {
        let params = match decode_request::<TextDocumentPositionParams>(&id, params) {
            Ok(params) => params,
            Err(response) => return response,
        };
        let Some(document) = self.documents.get_mut(&params.text_document.uri) else {
            return error_response(id, -32602, "text document is not open");
        };
        let source = document.database.source().to_owned();
        let Some(offset) =
            offset_at_position(&source, params.position.line, params.position.character)
        else {
            return error_response(id, -32602, "completion position is outside the document");
        };
        let Ok(completions) = document.database.completions(offset) else {
            return response(id, json!({ "isIncomplete": false, "items": [] }));
        };
        response(id, completion_list_json(&source, &completions))
    }

    fn hover_response(&mut self, id: Value, params: Value) -> Value {
        let params = match decode_request::<TextDocumentPositionParams>(&id, params) {
            Ok(params) => params,
            Err(response) => return response,
        };
        let Some((source, offset, document)) = self.document_at_position(&params) else {
            return error_response(id, -32602, "invalid hover document or position");
        };
        let Ok(hover) = document.database.hover(offset) else {
            return response(id, Value::Null);
        };
        response(
            id,
            hover.map_or(Value::Null, |hover| hover_json(&source, &hover)),
        )
    }

    fn inlay_hint_response(&mut self, id: Value, params: Value) -> Value {
        let params = match decode_request::<InlayHintParams>(&id, params) {
            Ok(params) => params,
            Err(response) => return response,
        };
        let Some(document) = self.documents.get_mut(&params.text_document.uri) else {
            return error_response(id, -32602, "text document is not open");
        };
        let source = document.database.source().to_owned();
        let (Some(start), Some(end)) = (
            offset_at_position(
                &source,
                params.range.start.line,
                params.range.start.character,
            ),
            offset_at_position(&source, params.range.end.line, params.range.end.character),
        ) else {
            return error_response(id, -32602, "invalid inlay-hint range");
        };
        let Ok(hints) = document
            .database
            .inlay_hints(crate::ast::Span { start, end })
        else {
            return response(id, json!([]));
        };
        response(
            id,
            Value::Array(
                hints
                    .iter()
                    .map(|hint| inlay_hint_json(&source, hint))
                    .collect(),
            ),
        )
    }

    fn selection_range_response(&mut self, id: Value, params: Value) -> Value {
        let params = match decode_request::<SelectionRangeParams>(&id, params) {
            Ok(params) => params,
            Err(response) => return response,
        };
        let Some(document) = self.documents.get_mut(&params.text_document.uri) else {
            return error_response(id, -32602, "text document is not open");
        };
        let source = document.database.source().to_owned();
        let mut selections = Vec::with_capacity(params.positions.len());
        for position_value in params.positions {
            let Some(offset) =
                offset_at_position(&source, position_value.line, position_value.character)
            else {
                return error_response(id, -32602, "selection position is outside the document");
            };
            let Ok(ranges) = document.database.selection_ranges(offset) else {
                return response(id, Value::Null);
            };
            selections.push(selection_range_json(&source, &ranges));
        }
        response(id, Value::Array(selections))
    }

    fn definition_response(&mut self, id: Value, params: Value) -> Value {
        let params = match decode_request::<TextDocumentPositionParams>(&id, params) {
            Ok(params) => params,
            Err(response) => return response,
        };
        let uri = params.text_document.uri.clone();
        let Some((source, offset, document)) = self.document_at_position(&params) else {
            return error_response(id, -32602, "invalid definition document or position");
        };
        let Ok(definition) = document.database.definition_at(offset) else {
            return response(id, Value::Null);
        };
        let location = definition.map_or(Value::Null, |definition| {
            definition_target_location_json(definition, &uri, &source, &self.documentation)
        });
        response(id, location)
    }

    fn type_definition_response(&mut self, id: Value, params: Value) -> Value {
        let params = match decode_request::<TextDocumentPositionParams>(&id, params) {
            Ok(params) => params,
            Err(response) => return response,
        };
        let uri = params.text_document.uri.clone();
        let Some((source, offset, document)) = self.document_at_position(&params) else {
            return error_response(id, -32602, "invalid type-definition document or position");
        };
        let Ok(definition) = document.database.type_definition_at(offset) else {
            return response(id, Value::Null);
        };
        response(
            id,
            definition.map_or(Value::Null, |definition| {
                definition_target_location_json(definition, &uri, &source, &self.documentation)
            }),
        )
    }

    fn references_response(&mut self, id: Value, params: Value) -> Value {
        let params = match decode_request::<ReferenceParams>(&id, params) {
            Ok(params) => params,
            Err(response) => return response,
        };
        let uri = params.position.text_document.uri.clone();
        let Some((source, offset, document)) = self.document_at_position(&params.position) else {
            return error_response(id, -32602, "invalid references document or position");
        };
        let Ok(references) = document
            .database
            .references_at(offset, params.context.include_declaration)
        else {
            return response(id, json!([]));
        };
        response(
            id,
            Value::Array(
                references
                    .into_iter()
                    .map(|span| location_json(&uri, &source, span))
                    .collect(),
            ),
        )
    }

    fn document_highlight_response(&mut self, id: Value, params: Value) -> Value {
        let params = match decode_request::<TextDocumentPositionParams>(&id, params) {
            Ok(params) => params,
            Err(response) => return response,
        };
        let Some((source, offset, document)) = self.document_at_position(&params) else {
            return error_response(id, -32602, "invalid highlight document or position");
        };
        let Ok(highlights) = document.database.document_highlights_at(offset) else {
            return response(id, json!([]));
        };
        response(
            id,
            Value::Array(
                highlights
                    .into_iter()
                    .map(|highlight| document_highlight_json(&source, highlight))
                    .collect(),
            ),
        )
    }

    fn prepare_rename_response(&mut self, id: Value, params: Value) -> Value {
        let params = match decode_request::<TextDocumentPositionParams>(&id, params) {
            Ok(params) => params,
            Err(response) => return response,
        };
        let Some((source, offset, document)) = self.document_at_position(&params) else {
            return error_response(id, -32602, "invalid rename document or position");
        };
        let Ok(target) = document.database.rename_target_at(offset) else {
            return response(id, Value::Null);
        };
        response(
            id,
            target.map_or(Value::Null, |target| {
                json!({
                    "range": {
                        "start": position(&source, target.span.start),
                        "end": position(&source, target.span.end)
                    },
                    "placeholder": target.name
                })
            }),
        )
    }

    fn rename_response(&mut self, id: Value, params: Value) -> Value {
        let params = match decode_request::<RenameParams>(&id, params) {
            Ok(params) => params,
            Err(response) => return response,
        };
        let uri = params.position.text_document.uri.clone();
        let Some((source, offset, document)) = self.document_at_position(&params.position) else {
            return error_response(id, -32602, "invalid rename document or position");
        };
        let plan = match document.database.rename_at(offset, &params.new_name) {
            Ok(plan) => plan,
            Err(error) => return error_response(id, -32602, error.to_string()),
        };
        let edits = plan
            .edits
            .into_iter()
            .map(|edit| {
                json!({
                    "range": {
                        "start": position(&source, edit.span.start),
                        "end": position(&source, edit.span.end)
                    },
                    "newText": edit.replacement
                })
            })
            .collect();
        let mut changes = serde_json::Map::new();
        changes.insert(uri, Value::Array(edits));
        response(id, json!({ "changes": changes }))
    }

    fn signature_help_response(&mut self, id: Value, params: Value) -> Value {
        let params = match decode_request::<TextDocumentPositionParams>(&id, params) {
            Ok(params) => params,
            Err(response) => return response,
        };
        let Some((_source, offset, document)) = self.document_at_position(&params) else {
            return error_response(id, -32602, "invalid signature-help document or position");
        };
        let Ok(help) = document.database.signature_help(offset) else {
            return response(id, Value::Null);
        };
        response(
            id,
            help.map_or(Value::Null, |help| signature_help_json(&help)),
        )
    }

    fn document_at_position<'a>(
        &'a mut self,
        params: &TextDocumentPositionParams,
    ) -> Option<(String, usize, &'a mut Document)> {
        let document = self.documents.get_mut(&params.text_document.uri)?;
        let source = document.database.source().to_owned();
        let offset = offset_at_position(&source, params.position.line, params.position.character)?;
        Some((source, offset, document))
    }
}

fn documentation_location_json(path: &str) -> Value {
    json!({
        "uri": format!("splitscript-docs:{path}"),
        "range": {
            "start": { "line": 0, "character": 0 },
            "end": { "line": 0, "character": 0 }
        }
    })
}

fn parse_line_ending(value: &str) -> Option<crate::LineEnding> {
    match value {
        "lf" => Some(crate::LineEnding::Lf),
        "crlf" => Some(crate::LineEnding::CrLf),
        "cr" => Some(crate::LineEnding::Cr),
        _ => None,
    }
}

fn definition_target_location_json(
    target: DefinitionTarget,
    uri: &str,
    source: &str,
    documentation: &DocumentationReference,
) -> Value {
    match target {
        DefinitionTarget::Source(definition) => location_json(uri, source, definition.span),
        DefinitionTarget::StandardLibrary(item) => documentation_location_json(
            &documentation.standard_library_symbol_uri(crate::stdlib::StdlibSymbolId::Item(item)),
        ),
        DefinitionTarget::StandardLibrarySymbol(symbol) => {
            documentation_location_json(&documentation.standard_library_symbol_uri(symbol))
        }
        DefinitionTarget::Language(item) => {
            documentation_location_json(&documentation.language_item_uri(item))
        }
    }
}

fn decode_request<T: serde::de::DeserializeOwned>(id: &Value, params: Value) -> Result<T, Value> {
    decode(params).map_err(|error| error_response(id.clone(), -32602, error))
}

fn response(id: Value, result: Value) -> Value {
    json!({
        "jsonrpc": "2.0",
        "id": id,
        "result": result
    })
}

fn error_response(id: Value, code: i64, message: impl Into<String>) -> Value {
    json!({
        "jsonrpc": "2.0",
        "id": id,
        "error": {
            "code": code,
            "message": message.into()
        }
    })
}

#[cfg(test)]
mod tests;
