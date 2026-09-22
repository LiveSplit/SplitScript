//! Completion for the declaration-only module grammar and state headers.

use super::{
    CompletionBuilder, CompletionItem, CompletionKind, CompletionList, CompletionRequest,
    catalog_language_completion, identifier_span, render_documentation,
};
use crate::{
    ast::{ActionKind, Program},
    documentation::symbol_uri,
    language::{LanguageCatalog, LanguageItemId},
    lexer::TokenKind,
    stdlib::{StandardLibrary, StateProviderProcesses, StdlibSymbolId},
};

pub(super) fn complete_top_level(source: &str, syntax: &Program, offset: usize) -> CompletionList {
    let replacement = identifier_span(source, offset);
    let prefix = source[replacement.start..offset].to_owned();
    let mut builder = CompletionBuilder::new(prefix, replacement);
    let language = LanguageCatalog::new();

    if syntax.state.is_none() {
        add_catalog_snippet(
            &mut builder,
            language.item(LanguageItemId::State),
            "state \"${1:game.exe}\" {\n\t$0\n}",
        );
    }
    if syntax.tick_rate.is_none() {
        add_catalog_snippet(
            &mut builder,
            language.item(LanguageItemId::TickRate),
            "tickRate {\n\tattached: ${1:120},\n\tdetached: ${2:1},\n}",
        );
    }
    if syntax.settings_span.is_none() {
        add_catalog_snippet(
            &mut builder,
            language.item(LanguageItemId::Settings),
            "settings {\n\t$0\n}",
        );
    }

    add_catalog_snippet(
        &mut builder,
        language.item(LanguageItemId::Function),
        "fn ${1:name}(${2}) {\n\t$0\n}",
    );
    add_catalog_snippet(
        &mut builder,
        language.item(LanguageItemId::Struct),
        "struct ${1:Name} {\n\t${2:field}: ${3:Type},\n}",
    );
    add_catalog_snippet(
        &mut builder,
        language.item(LanguageItemId::Enum),
        "enum ${1:Name} {\n\t${2:Variant},\n}",
    );
    add_catalog_snippet(
        &mut builder,
        language.item(LanguageItemId::Let),
        "let ${1:name} = ${2:value}",
    );

    add_plain_snippet(
        &mut builder,
        "debug fn",
        "development-only function",
        "debug fn ${1:name}(${2}) {\n\t$0\n}",
        "Declares a function that is checked in every build and erased from release output.",
    );
    add_plain_snippet(
        &mut builder,
        "debug let",
        "development-only global",
        "debug let ${1:name} = ${2:value}",
        "Declares a global that is checked in every build and erased from release output.",
    );

    for action in all_actions() {
        if syntax
            .actions
            .iter()
            .any(|existing| existing.kind == action)
        {
            continue;
        }
        let item = language.action(action);
        add_catalog_snippet(&mut builder, item, &format!("{} {{\n\t$0\n}}", item.name));
    }

    builder.finish()
}

pub(super) fn complete_state_header(
    request: &CompletionRequest<'_>,
    standard_library: &StandardLibrary,
) -> Option<CompletionList> {
    let source = request.source;
    let offset = request.offset;
    let replacement = request.replacement;
    let tokens = &request.tokens;
    let state = top_level_state_header(tokens, offset)?;
    let tail = tokens[state + 1..]
        .iter()
        .copied()
        .take_while(|token| token.span.start < offset)
        .collect::<Vec<_>>();
    if tail
        .iter()
        .any(|token| matches!(token.kind, TokenKind::LBrace))
    {
        return None;
    }
    // Selector arguments are ordinary SplitScript expressions. Once the
    // cursor enters them, let the shared expression/member completion paths
    // handle types, constants, static methods, and nested calls instead of
    // returning an empty state-header result.
    if has_unclosed_parenthesis(&tail) {
        return None;
    }

    let prefix = source[replacement.start..offset].to_owned();
    let mut builder = CompletionBuilder::new(prefix, replacement);
    if tail.is_empty()
        || tail.len() == 1
            && tail[0].span == replacement
            && matches!(tail[0].kind, TokenKind::Ident(_))
    {
        add_plain_snippet(
            &mut builder,
            "\"game.exe\"",
            "process executable",
            "\"${1:game.exe}\" {\n\t$0\n}",
            "Attaches to one executable name and exposes the native `process` value.",
        );
        add_plain_snippet(
            &mut builder,
            "[\"game.exe\", ...]",
            "process executable list",
            "[\"${1:game.exe}\", \"${2:demo.exe}\"] {\n\t$0\n}",
            "Attaches to any executable in the list and exposes the native `process` value.",
        );
        for provider in standard_library
            .state_providers()
            .iter()
            .filter(|provider| !provider.default)
        {
            let insert_text = match provider.processes {
                StateProviderProcesses::Declared(_) => format!("{} {{\n\t$0\n}}", provider.name),
                StateProviderProcesses::SourceState => {
                    format!("{} [\"${{1:game.exe}}\"] {{\n\t$0\n}}", provider.name)
                }
            };
            builder.add(CompletionItem {
                label: provider.name.to_owned(),
                kind: CompletionKind::Snippet,
                detail: Some("standard-library state provider".to_owned()),
                documentation: Some(render_documentation(&provider.documentation)),
                documentation_uri: Some(symbol_uri(
                    StdlibSymbolId::StateProvider(provider.id),
                    standard_library,
                )),
                insert_text,
                is_snippet: true,
            });
        }
        return Some(builder.finish());
    }

    if let [provider_token, dot, rest @ ..] = tail.as_slice()
        && matches!(dot.kind, TokenKind::Dot)
        && let TokenKind::Ident(provider_name) = &provider_token.kind
        && let Some(provider) = standard_library.state_provider_by_name(provider_name)
        && (rest.is_empty()
            || matches!(rest, [token] if token.span == replacement && matches!(token.kind, TokenKind::Ident(_))))
    {
        for selector in provider.selectors {
            let mut next_placeholder = 1;
            let arguments = selector
                .parameters
                .iter()
                .map(|parameter| {
                    let placeholder = format!("${{{next_placeholder}:{}}}", parameter.name);
                    next_placeholder += 1;
                    placeholder
                })
                .collect::<Vec<_>>()
                .join(", ");
            let process_suffix = match provider.processes {
                StateProviderProcesses::Declared(_) => String::new(),
                StateProviderProcesses::SourceState => {
                    let suffix = format!(" [\"${{{next_placeholder}:game.exe}}\"]");
                    suffix
                }
            };
            let parameters = selector
                .parameters
                .iter()
                .map(|parameter| {
                    format!(
                        "{}: {}",
                        parameter.name,
                        standard_library.render_type(parameter.ty)
                    )
                })
                .collect::<Vec<_>>()
                .join(", ");
            builder.add(CompletionItem {
                label: selector.name.to_owned(),
                kind: CompletionKind::Snippet,
                detail: Some(format!("state-provider selector ({parameters})")),
                documentation: Some(render_documentation(&selector.documentation)),
                documentation_uri: Some(symbol_uri(
                    StdlibSymbolId::StateProvider(provider.id),
                    standard_library,
                )),
                insert_text: format!(
                    "{}({arguments}){process_suffix} {{\n\t$0\n}}",
                    selector.name
                ),
                is_snippet: true,
            });
        }
        return Some(builder.finish());
    }

    let complete_target = matches!(tail.as_slice(), [token] if matches!(token.kind, TokenKind::String(_)))
        || is_complete_process_list(&tail)
        || matches!(tail.as_slice(), [token]
            if matches!(&token.kind, TokenKind::Ident(name)
                if standard_library.state_provider_by_name(name).is_some_and(|provider|
                    matches!(provider.processes, StateProviderProcesses::Declared(_)))));
    if complete_target && replacement.start == offset {
        add_plain_snippet(
            &mut builder,
            "{",
            "state body",
            "{\n\t$0\n}",
            "Begins the persistent watched-state declaration.",
        );
    }
    Some(builder.finish())
}

fn has_unclosed_parenthesis(tokens: &[&crate::lexer::Token]) -> bool {
    let mut depth = 0usize;
    for token in tokens {
        match token.kind {
            TokenKind::LParen => depth += 1,
            TokenKind::RParen => depth = depth.saturating_sub(1),
            _ => {}
        }
    }
    depth > 0
}

fn top_level_state_header(tokens: &[&crate::lexer::Token], offset: usize) -> Option<usize> {
    let mut brace_depth = 0_u32;
    let mut candidate = None;
    for (index, token) in tokens.iter().enumerate() {
        if token.span.start >= offset {
            break;
        }
        match token.kind {
            TokenKind::LBrace => brace_depth += 1,
            TokenKind::RBrace => brace_depth = brace_depth.saturating_sub(1),
            TokenKind::Ident(ref name) if brace_depth == 0 && name == "state" => {
                candidate = Some(index);
            }
            _ => {}
        }
    }
    candidate
}

fn is_complete_process_list(tokens: &[&crate::lexer::Token]) -> bool {
    if !matches!(
        tokens.first().map(|token| &token.kind),
        Some(TokenKind::LBracket)
    ) || !matches!(
        tokens.last().map(|token| &token.kind),
        Some(TokenKind::RBracket)
    ) {
        return false;
    }
    let mut expects_string = true;
    for token in &tokens[1..tokens.len() - 1] {
        match (&token.kind, expects_string) {
            (TokenKind::String(_), true) => expects_string = false,
            (TokenKind::Comma, false) => expects_string = true,
            _ => return false,
        }
    }
    !expects_string
}

fn all_actions() -> [ActionKind; 13] {
    [
        ActionKind::Setup,
        ActionKind::SelectProcess,
        ActionKind::OnDetach,
        ActionKind::OnAttach,
        ActionKind::OnStateReady,
        ActionKind::OnStart,
        ActionKind::OnReset,
        ActionKind::WhileAttached,
        ActionKind::Start,
        ActionKind::Split,
        ActionKind::Reset,
        ActionKind::IsLoading,
        ActionKind::GameTime,
    ]
}

fn add_catalog_snippet(
    builder: &mut CompletionBuilder,
    item: &crate::language::LanguageItem,
    insert_text: &str,
) {
    builder.add(catalog_language_completion(
        item.name,
        CompletionKind::Snippet,
        item,
        insert_text.to_owned(),
        true,
    ));
}

fn add_plain_snippet(
    builder: &mut CompletionBuilder,
    label: &str,
    detail: &str,
    insert_text: &str,
    documentation: &str,
) {
    builder.add(CompletionItem {
        label: label.to_owned(),
        kind: CompletionKind::Snippet,
        detail: Some(detail.to_owned()),
        documentation: Some(documentation.to_owned()),
        documentation_uri: None,
        insert_text: insert_text.to_owned(),
        is_snippet: true,
    });
}
