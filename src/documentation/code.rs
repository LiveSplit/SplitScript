//! Semantic code fragments for the generated Markdown reference.

use crate::{
    catalog::Example,
    database::{CompilerDatabase, DefinitionTarget},
    highlight::{LanguageTokenContext, SemanticTokenKind, language_identifier_kind},
    language::LanguageCatalog,
    lexer::{Lexeme, TokenKind, TriviaKind, lex_lossless},
    stdlib::{StandardLibrary, StdlibSymbolId},
};

use super::reference::{core_type_uri, language_item_uri, relative_document_link, symbol_uri};

#[derive(Debug, Clone)]
struct Annotation {
    start: usize,
    end: usize,
    kind: SemanticTokenKind,
    target: Option<String>,
}

pub(super) fn signature(
    source: &str,
    current_uri: &str,
    primary: Option<StdlibSymbolId>,
    library: &StandardLibrary,
) -> String {
    let annotations = lexical_annotations(source, current_uri, primary, library);
    render(source, &annotations)
}

pub(super) fn example(example: Example, current_uri: &str, library: &StandardLibrary) -> String {
    let annotations = semantic_example_annotations(example, current_uri, library)
        .unwrap_or_else(|| lexical_annotations(example.source, current_uri, None, library));
    render(example.source, &annotations)
}

/// Fast graph-validation rendering. Focused tests cover semantic example
/// links; exhaustive page validation need not type-check every example merely
/// to establish page identities and prose-link correctness.
pub(super) fn lexical_example(
    example: Example,
    current_uri: &str,
    library: &StandardLibrary,
) -> String {
    render(
        example.source,
        &lexical_annotations(example.source, current_uri, None, library),
    )
}

pub(super) fn checked_fragment(
    source: &str,
    validation_program: &str,
    current_uri: &str,
    library: &StandardLibrary,
    semantic: bool,
) -> String {
    let annotations = semantic
        .then(|| semantic_fragment_annotations(source, validation_program, current_uri, library))
        .flatten()
        .unwrap_or_else(|| lexical_annotations(source, current_uri, None, library));
    render(source, &annotations)
}

fn semantic_example_annotations(
    example: Example,
    current_uri: &str,
    library: &StandardLibrary,
) -> Option<Vec<Annotation>> {
    let program = example.validation_program();
    semantic_fragment_annotations(example.source, &program, current_uri, library)
}

fn semantic_fragment_annotations(
    source: &str,
    validation_program: &str,
    current_uri: &str,
    library: &StandardLibrary,
) -> Option<Vec<Annotation>> {
    let mappings = source_mappings(source, validation_program)?;
    let context = crate::CompilerContext::default().without_standard_library_bodies();
    let mut database = CompilerDatabase::with_context_and_source_name(
        context,
        "guide-example.split",
        validation_program.to_owned(),
    );
    let highlights = database.semantic_highlights().ok()?;
    let mut annotations = Vec::new();
    for highlight in highlights.highlights() {
        let Some(mapping) = mappings.iter().find(|mapping| {
            mapping.validation_start <= highlight.span.start
                && highlight.span.end <= mapping.validation_end
        }) else {
            continue;
        };
        let target = is_linkable(highlight.kind)
            .then(|| database.definition_at(highlight.span.start).ok().flatten())
            .flatten()
            .and_then(|target| target_uri(target, current_uri, library));
        annotations.push(Annotation {
            start: mapping.visible_start + highlight.span.start - mapping.validation_start,
            end: mapping.visible_start + highlight.span.end - mapping.validation_start,
            kind: highlight.kind,
            target,
        });
    }
    Some(annotations)
}

#[derive(Debug, Clone, Copy)]
struct SourceMapping {
    validation_start: usize,
    validation_end: usize,
    visible_start: usize,
}

fn source_mappings(source: &str, validation_program: &str) -> Option<Vec<SourceMapping>> {
    let mut mappings = Vec::<SourceMapping>::new();
    let mut validation_cursor = 0;
    let mut visible_cursor = 0;
    for line in source.split_inclusive('\n') {
        let line_without_newline = line.strip_suffix('\n').unwrap_or(line);
        let content = line_without_newline.trim_start();
        let visible_indent = line_without_newline.len() - content.len();
        if content.is_empty() {
            visible_cursor += line.len();
            continue;
        }
        let offset = validation_program[validation_cursor..].find(content)?;
        let validation_start = validation_cursor + offset;
        let validation_end = validation_start + content.len();
        let visible_start = visible_cursor + visible_indent;
        if let Some(previous) = mappings.last_mut()
            && previous.validation_end == validation_start
            && previous.visible_start + (previous.validation_end - previous.validation_start)
                == visible_start
        {
            previous.validation_end = validation_end;
        } else {
            mappings.push(SourceMapping {
                validation_start,
                validation_end,
                visible_start,
            });
        }
        validation_cursor = validation_end;
        visible_cursor += line.len();
    }
    Some(mappings)
}

fn lexical_annotations(
    source: &str,
    current_uri: &str,
    primary: Option<StdlibSymbolId>,
    library: &StandardLibrary,
) -> Vec<Annotation> {
    let Ok(lexed) = lex_lossless(source) else {
        return Vec::new();
    };
    let lexemes = lexed.into_lexemes();
    let mut annotations = Vec::new();
    for (index, lexeme) in lexemes.iter().enumerate() {
        let (span, kind, target) = match lexeme {
            Lexeme::Trivia(trivia)
                if matches!(
                    trivia.kind,
                    TriviaKind::LineComment | TriviaKind::BlockComment
                ) =>
            {
                (trivia.span, SemanticTokenKind::Comment, None)
            }
            Lexeme::Trivia(_) => continue,
            Lexeme::Token(token) => {
                let previous = previous_token(&lexemes, index);
                let next = next_token(&lexemes, index);
                let Some(kind) = lexical_kind(&token.kind, previous, next, primary, library) else {
                    continue;
                };
                let target = match &token.kind {
                    TokenKind::Ident(name) => {
                        lexical_target(name, previous, next, primary, current_uri, library)
                    }
                    _ => None,
                };
                (token.span, kind, target)
            }
        };
        if span.start != span.end {
            annotations.push(Annotation {
                start: span.start,
                end: span.end,
                kind,
                target,
            });
        }
    }
    annotations
}

fn previous_token(lexemes: &[Lexeme], index: usize) -> Option<&TokenKind> {
    lexemes[..index]
        .iter()
        .rev()
        .find_map(|lexeme| match lexeme {
            Lexeme::Token(token) if token.kind != TokenKind::Eof => Some(&token.kind),
            _ => None,
        })
}

fn next_token(lexemes: &[Lexeme], index: usize) -> Option<&TokenKind> {
    lexemes[index + 1..].iter().find_map(|lexeme| match lexeme {
        Lexeme::Token(token) if token.kind != TokenKind::Eof => Some(&token.kind),
        _ => None,
    })
}

fn lexical_kind(
    token: &TokenKind,
    previous: Option<&TokenKind>,
    next: Option<&TokenKind>,
    primary: Option<StdlibSymbolId>,
    library: &StandardLibrary,
) -> Option<SemanticTokenKind> {
    match token {
        TokenKind::Ident(name)
            if name == "None"
                && !matches!(previous, Some(TokenKind::Colon | TokenKind::Gt))
                && !matches!(next, Some(TokenKind::Dot))
                && !(previous.is_none() && next.is_none()) =>
        {
            language_identifier_kind(name, LanguageTokenContext::DocumentationFragment)
        }
        TokenKind::Ident(name)
            if library
                .capabilities()
                .iter()
                .any(|capability| capability.name == name) =>
        {
            Some(SemanticTokenKind::Capability)
        }
        TokenKind::Ident(name)
            if primary.is_some_and(|symbol| {
                primary_name(symbol, library) == name && matches!(symbol, StdlibSymbolId::Item(_))
            }) =>
        {
            let StdlibSymbolId::Item(item) = primary.unwrap() else {
                unreachable!()
            };
            Some(match library.item(item).kind {
                crate::stdlib::ItemKind::Function => SemanticTokenKind::Function,
                crate::stdlib::ItemKind::Method { .. } => SemanticTokenKind::Method,
                crate::stdlib::ItemKind::Constant => SemanticTokenKind::Constant,
            })
        }
        TokenKind::Ident(name)
            if is_type_name(name, library)
                && (name != "address"
                    || matches!(previous, Some(TokenKind::Colon | TokenKind::Gt))
                    || matches!(next, Some(TokenKind::Dot))
                    || (previous.is_none() && next.is_none())) =>
        {
            Some(SemanticTokenKind::Type)
        }
        TokenKind::Ident(name)
            if language_identifier_kind(name, LanguageTokenContext::DocumentationFragment)
                .is_some() =>
        {
            language_identifier_kind(name, LanguageTokenContext::DocumentationFragment)
        }
        TokenKind::Ident(_) if matches!(previous, Some(TokenKind::Ident(name)) if name == "class") => {
            Some(SemanticTokenKind::Struct)
        }
        TokenKind::Ident(_) if matches!(previous, Some(TokenKind::Ident(name)) if name == "namespace") => {
            Some(SemanticTokenKind::Namespace)
        }
        TokenKind::Ident(_) if matches!(previous, Some(TokenKind::Ident(name)) if name == "struct") => {
            Some(SemanticTokenKind::Struct)
        }
        TokenKind::Ident(_) if matches!(previous, Some(TokenKind::Ident(name)) if name == "enum") => {
            Some(SemanticTokenKind::Enum)
        }
        TokenKind::Ident(_) if matches!(next, Some(TokenKind::Colon)) => {
            Some(SemanticTokenKind::Parameter)
        }
        TokenKind::Ident(_) if matches!(next, Some(TokenKind::LParen)) => {
            Some(if matches!(previous, Some(TokenKind::Dot)) {
                SemanticTokenKind::Method
            } else {
                SemanticTokenKind::Function
            })
        }
        TokenKind::Ident(_) if matches!(previous, Some(TokenKind::Dot)) => {
            Some(SemanticTokenKind::Property)
        }
        TokenKind::Ident(_) => Some(SemanticTokenKind::Variable),
        TokenKind::Char(_) | TokenKind::String(_) => Some(SemanticTokenKind::String),
        TokenKind::TemplateStart | TokenKind::TemplateChunk(_) | TokenKind::TemplateEnd => {
            Some(SemanticTokenKind::TemplateString)
        }
        TokenKind::DocComment(_) => Some(SemanticTokenKind::Comment),
        TokenKind::Int(_) | TokenKind::Float(_) => Some(SemanticTokenKind::Number),
        kind if is_operator(kind) => Some(SemanticTokenKind::Operator),
        _ => None,
    }
}

fn is_linkable(kind: SemanticTokenKind) -> bool {
    matches!(
        kind,
        SemanticTokenKind::Keyword
            | SemanticTokenKind::Type
            | SemanticTokenKind::Capability
            | SemanticTokenKind::Struct
            | SemanticTokenKind::Enum
            | SemanticTokenKind::EnumMember
            | SemanticTokenKind::Function
            | SemanticTokenKind::Method
            | SemanticTokenKind::Property
            | SemanticTokenKind::Namespace
            | SemanticTokenKind::Lifecycle
    )
}

fn lexical_target(
    name: &str,
    previous: Option<&TokenKind>,
    next: Option<&TokenKind>,
    primary: Option<StdlibSymbolId>,
    current_uri: &str,
    library: &StandardLibrary,
) -> Option<String> {
    if let Some(primary) = primary
        && primary_name(primary, library) == name
        && (matches!(previous, Some(TokenKind::Dot))
            || !matches!(previous, Some(TokenKind::LParen | TokenKind::Comma)))
    {
        return Some(relative_document_link(
            current_uri,
            &symbol_uri(primary, library),
        ));
    }
    if let Some(ty) = library.type_by_name(name) {
        return Some(relative_document_link(
            current_uri,
            &symbol_uri(StdlibSymbolId::Type(ty.id), library),
        ));
    }
    if let Some(capability) = library
        .capabilities()
        .iter()
        .find(|capability| capability.name == name)
    {
        return Some(relative_document_link(
            current_uri,
            &symbol_uri(StdlibSymbolId::Capability(capability.id), library),
        ));
    }
    if let Some(namespace) = library.namespace_by_name(name) {
        return Some(relative_document_link(
            current_uri,
            &symbol_uri(StdlibSymbolId::Namespace(namespace.id), library),
        ));
    }
    if let Some(constructor) = library.named_type_constructor_by_name(name) {
        return Some(relative_document_link(
            current_uri,
            &symbol_uri(StdlibSymbolId::TypeConstructor(constructor.id), library),
        ));
    }
    let type_position = matches!(previous, Some(TokenKind::Colon | TokenKind::Gt))
        || matches!(next, Some(TokenKind::Dot))
        || (previous.is_none() && next.is_none());
    if type_position && let Some(ty) = library.core_types().iter().find(|ty| ty.name == name) {
        return Some(relative_document_link(
            current_uri,
            &core_type_uri(ty.id, library),
        ));
    }
    if let Some(item) = LanguageCatalog::new().item_for_source_token(name) {
        return Some(relative_document_link(
            current_uri,
            &language_item_uri(item.id),
        ));
    }
    None
}

fn target_uri(
    target: DefinitionTarget,
    current_uri: &str,
    library: &StandardLibrary,
) -> Option<String> {
    let target = match target {
        DefinitionTarget::StandardLibrary(item) => symbol_uri(StdlibSymbolId::Item(item), library),
        DefinitionTarget::StandardLibrarySymbol(symbol) => symbol_uri(symbol, library),
        DefinitionTarget::Language(id) => language_item_uri(id),
        DefinitionTarget::Source(_) => return None,
    };
    Some(relative_document_link(current_uri, &target))
}

fn primary_name(symbol: StdlibSymbolId, library: &StandardLibrary) -> &'static str {
    match symbol {
        StdlibSymbolId::StateProvider(id) => library.state_provider(id).name,
        StdlibSymbolId::Namespace(id) => library.namespace(id).name,
        StdlibSymbolId::Capability(id) => library.capability(id).name,
        StdlibSymbolId::TypeConstructor(id) => library.type_constructor(id).name,
        StdlibSymbolId::Type(id) => library.type_decl(id).name,
        StdlibSymbolId::Field(id) => library.field(id).name,
        StdlibSymbolId::Variant(id) => library.variant(id).name,
        StdlibSymbolId::Item(id) => library.item(id).name,
    }
}

fn is_type_name(name: &str, library: &StandardLibrary) -> bool {
    library.core_types().iter().any(|ty| ty.name == name)
        || library.type_by_name(name).is_some()
        || library.named_type_constructor_by_name(name).is_some()
        || library
            .capabilities()
            .iter()
            .any(|capability| capability.name == name)
}

fn is_operator(kind: &TokenKind) -> bool {
    matches!(
        kind,
        TokenKind::FatArrow
            | TokenKind::Assign
            | TokenKind::PlusAssign
            | TokenKind::MinusAssign
            | TokenKind::StarAssign
            | TokenKind::SlashAssign
            | TokenKind::PercentAssign
            | TokenKind::OrAssign
            | TokenKind::AndAssign
            | TokenKind::CaretAssign
            | TokenKind::ShlAssign
            | TokenKind::ShrAssign
            | TokenKind::Plus
            | TokenKind::Minus
            | TokenKind::Star
            | TokenKind::Slash
            | TokenKind::Percent
            | TokenKind::Bang
            | TokenKind::Question
            | TokenKind::Tilde
            | TokenKind::Or
            | TokenKind::And
            | TokenKind::Caret
            | TokenKind::OrOr
            | TokenKind::AndAnd
            | TokenKind::EqEq
            | TokenKind::BangEq
            | TokenKind::Lt
            | TokenKind::Le
            | TokenKind::Gt
            | TokenKind::Ge
            | TokenKind::Shl
            | TokenKind::Shr
            | TokenKind::Dot
            | TokenKind::DotDotEq
    )
}

fn render(source: &str, annotations: &[Annotation]) -> String {
    let mut html = String::from("<pre class=\"hljs splitscript-code\"><code>");
    let mut cursor = 0;
    for annotation in annotations {
        if annotation.start < cursor
            || annotation.start > annotation.end
            || annotation.end > source.len()
        {
            continue;
        }
        escape_html_into(&mut html, &source[cursor..annotation.start]);
        if let Some(target) = &annotation.target {
            html.push_str("<a href=\"");
            escape_html_attribute_into(&mut html, target);
            html.push_str("\">");
        }
        html.push_str("<span data-splitscript-token=\"");
        html.push_str(annotation.kind.name());
        html.push_str("\" class=\"");
        html.push_str(css_class(annotation.kind));
        html.push_str("\">");
        escape_html_into(&mut html, &source[annotation.start..annotation.end]);
        html.push_str("</span>");
        if annotation.target.is_some() {
            html.push_str("</a>");
        }
        cursor = annotation.end;
    }
    escape_html_into(&mut html, &source[cursor..]);
    html.push_str("</code></pre>");
    html
}

fn css_class(kind: SemanticTokenKind) -> &'static str {
    match kind {
        SemanticTokenKind::Keyword | SemanticTokenKind::Debug => "hljs-keyword",
        SemanticTokenKind::Type | SemanticTokenKind::Struct | SemanticTokenKind::Enum => {
            "hljs-type"
        }
        SemanticTokenKind::Capability => "hljs-type",
        SemanticTokenKind::EnumMember | SemanticTokenKind::Constant => "hljs-literal",
        SemanticTokenKind::Function | SemanticTokenKind::Method | SemanticTokenKind::Lifecycle => {
            "hljs-title function_"
        }
        SemanticTokenKind::Variable | SemanticTokenKind::Parameter => "hljs-variable",
        SemanticTokenKind::Property
        | SemanticTokenKind::Setting
        | SemanticTokenKind::SettingTitle
        | SemanticTokenKind::StateField => "hljs-attr",
        SemanticTokenKind::Namespace => "hljs-built_in",
        SemanticTokenKind::String
        | SemanticTokenKind::TemplateString
        | SemanticTokenKind::Signature
        | SemanticTokenKind::Version => "hljs-string",
        SemanticTokenKind::Number => "hljs-number",
        SemanticTokenKind::Operator => "splitscript-token-operator",
        SemanticTokenKind::Comment => "hljs-comment",
    }
}

fn escape_html_into(output: &mut String, source: &str) {
    for character in source.chars() {
        match character {
            '&' => output.push_str("&amp;"),
            '<' => output.push_str("&lt;"),
            '>' => output.push_str("&gt;"),
            '"' => output.push_str("&quot;"),
            _ => output.push(character),
        }
    }
}

fn escape_html_attribute_into(output: &mut String, source: &str) {
    escape_html_into(output, source);
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{
        language::{LanguageItemId, LanguageItemKind},
        stdlib::StdlibItemId,
    };

    #[test]
    fn signatures_highlight_and_link_catalog_symbols() {
        let library = StandardLibrary::new();
        let item = library.item(StdlibItemId::ProcessRead);
        let source = library.render_signature(item.id);
        let html = signature(
            &source,
            "/stdlib/types/Process/methods/read.md",
            Some(StdlibSymbolId::Item(item.id)),
            &library,
        );
        assert!(
            html.contains("data-splitscript-token=\"type\" class=\"hljs-type\">Process</span>"),
            "{html}"
        );
        assert!(
            html.contains(
                "href=\"read.md\"><span data-splitscript-token=\"method\" class=\"hljs-title function_\">read"
            ),
            "{html}"
        );
        assert!(
            html.contains("capabilities/MemoryReadable/index.md"),
            "{html}"
        );
        assert!(
            html.contains(
                "data-splitscript-token=\"interface\" class=\"hljs-type\">MemoryReadable"
            ),
            "{html}"
        );
    }

    #[test]
    fn language_signatures_follow_catalogued_compiler_roles() {
        let library = StandardLibrary::new();
        for item in LanguageCatalog::new().items() {
            let expected = match item.kind {
                LanguageItemKind::Keyword | LanguageItemKind::Declaration => {
                    Some(if item.name == "debug" {
                        SemanticTokenKind::Debug
                    } else {
                        SemanticTokenKind::Keyword
                    })
                }
                LanguageItemKind::Action(_) => Some(SemanticTokenKind::Lifecycle),
                _ => None,
            };
            let Some(expected) = expected else {
                continue;
            };
            let html = signature(item.form, "/language/test.md", None, &library);
            assert!(
                html.contains(&format!("data-splitscript-token=\"{}\"", expected.name())),
                "`{}` signature did not contain its compiler role `{}`: {html}",
                item.name,
                expected.name(),
            );
        }
    }

    #[test]
    fn fragment_highlighting_covers_contextual_syntax_and_wrapper_variants() {
        let library = StandardLibrary::new();
        let managed = signature(
            "image \"Assembly-CSharp\" { namespace Game { class Player from \"RuntimePlayer\" { static String name; } } }",
            "/language/image.md",
            None,
            &library,
        );
        for keyword in ["image", "namespace", "class", "from", "static"] {
            assert!(
                managed.contains(&format!(
                    "data-splitscript-token=\"keyword\" class=\"hljs-keyword\">{keyword}</span>"
                )),
                "missing managed keyword `{keyword}` in {managed}",
            );
        }
        assert!(
            managed.contains(
                "data-splitscript-token=\"namespace\" class=\"hljs-built_in\">Game</span>"
            ),
            "{managed}"
        );
        assert!(
            managed.contains("data-splitscript-token=\"struct\" class=\"hljs-type\">Player</span>"),
            "{managed}"
        );

        let wrappers = signature(
            "let value: i32? = Some(Ok(1)) else None",
            "/language/test.md",
            None,
            &library,
        );
        for variant in ["Some", "Ok", "None"] {
            assert!(
                wrappers.contains(&format!(
                    "data-splitscript-token=\"enumMember\" class=\"hljs-literal\">{variant}</span>"
                )),
                "wrapper `{variant}` did not match compiler highlighting: {wrappers}",
            );
        }
    }

    #[test]
    fn managed_image_example_uses_semantic_highlighting() {
        let library = StandardLibrary::new();
        let item = LanguageCatalog::new().item(LanguageItemId::ManagedImage);
        let html = example(
            item.documentation.examples[0],
            "/language/image.md",
            &library,
        );
        for keyword in ["image", "class", "from", "static"] {
            assert!(
                html.contains(&format!(
                    "data-splitscript-token=\"keyword\" class=\"hljs-keyword\">{keyword}</span>"
                )),
                "missing semantic managed keyword `{keyword}` in {html}",
            );
        }
        assert!(
            html.contains("data-splitscript-token=\"property\" class=\"hljs-attr\">score</span>"),
            "managed field should use the compiler's semantic role: {html}"
        );
    }

    #[test]
    fn semantic_examples_link_resolved_methods_but_not_fixture_locals() {
        let library = StandardLibrary::new();
        let documentation_example = Example::checked(
            "Read a value",
            "let health = process.read<i32>(player.offset(0x20)) else 0",
            "state \"game.exe\" {}\nonAttach {\nlet player: address = 0x1000\nlet health = process.read<i32>(player.offset(0x20)) else 0\nprint(health)\n}",
        );
        let html = example(
            documentation_example,
            "/stdlib/types/Process/methods/read.md",
            &library,
        );
        assert!(html.contains("href=\"read.md"), "{html}");
        assert!(html.contains("address/methods/offset.md"), "{html}");
        assert!(!html.contains("href=\"player"));
    }

    #[test]
    fn semantic_examples_link_language_keywords() {
        let library = StandardLibrary::new();
        let documentation_example = Example::complete_program(
            "Load asynchronously",
            "fn load() -> async Module {\n    let module = await process.module(\"game.dll\")\n    return module\n}",
        );
        let html = example(documentation_example, "/language/async.md", &library);
        for keyword in ["fn", "async", "let", "await", "return"] {
            assert!(
                html.contains(&format!("href=\"{keyword}.md\"")),
                "missing link for `{keyword}` in {html}"
            );
        }
    }
}
