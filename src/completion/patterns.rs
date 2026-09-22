//! Type-directed completion for patterns.
//!
//! Pattern completion deliberately lives beside the other compiler-owned
//! completion grammars. Editor frontends receive already-filtered candidates;
//! they do not need to duplicate the pattern grammar or semantic type model.

use std::collections::{BTreeSet, HashMap};

use crate::{
    ast::{Expr, ExprKind},
    database::{CompilerDatabase, SemanticQueryResult, SemanticSnapshot},
    documentation::{language_item_uri, symbol_uri},
    language::{LanguageCatalog, LanguageItemId},
    lexer::{Token, TokenKind},
    stdlib::{
        CoreTypeId, FieldVisibility, StandardLibrary, StdlibCapabilityId, StdlibSymbolId,
        StdlibTypeConstructorId, StdlibTypeId,
    },
    type_display::display_type,
    types::{TypeId, TypeKind},
    visit::{self, Visitor},
};

use super::{
    CompletionBuilder, CompletionItem, CompletionKind, CompletionList, CompletionRequest,
    matching_closing_brace, render_documentation,
};

pub(super) fn complete_pattern(
    database: &mut CompilerDatabase,
    request: &CompletionRequest<'_>,
    library: &StandardLibrary,
) -> SemanticQueryResult<Option<CompletionList>> {
    let snapshot = database.semantic_snapshot()?;
    if let Some(completions) = complete_pattern_from_snapshot(request, library, &snapshot) {
        return Ok(Some(completions));
    }

    let has_is = request.tokens.iter().any(|token| {
        token.span.end <= request.replacement.start
            && matches!(&token.kind, TokenKind::Ident(name) if name == "is")
    });
    if !has_is {
        return Ok(None);
    }
    let mut probe_source = request.source.to_owned();
    probe_source.replace_range(request.replacement.start..request.replacement.end, "_");
    let mut probe = CompilerDatabase::with_context(database.context(), probe_source);
    let snapshot = probe.semantic_snapshot()?;
    Ok(complete_pattern_from_snapshot(request, library, &snapshot))
}

fn complete_pattern_from_snapshot(
    request: &CompletionRequest<'_>,
    library: &StandardLibrary,
    snapshot: &SemanticSnapshot,
) -> Option<CompletionList> {
    let (segment, expected, binding_pattern) =
        pattern_context(snapshot, &request.tokens, request.replacement.start)?;
    let site = analyze_pattern_prefix(&segment, expected, snapshot)?;

    let prefix = request.source[request.replacement.start..request.offset].to_owned();
    let mut builder = CompletionBuilder::new(prefix, request.replacement);
    match site {
        PatternSite::Value {
            expected,
            qualified_enum,
        } => add_value_patterns(
            &mut builder,
            expected,
            qualified_enum,
            binding_pattern,
            snapshot,
            library,
        ),
        PatternSite::ArrayElement {
            expected,
            rest_available,
        } => {
            add_value_patterns(
                &mut builder,
                expected,
                false,
                binding_pattern,
                snapshot,
                library,
            );
            if rest_available {
                add_array_rest_pattern(&mut builder, "array rest", "..");
            }
        }
        PatternSite::StructFields { structure, used } => {
            add_struct_fields(&mut builder, structure, &used, binding_pattern, snapshot)
        }
    }
    Some(builder.finish())
}

fn pattern_context<'tokens>(
    snapshot: &SemanticSnapshot,
    tokens: &'tokens [&'tokens Token],
    offset: usize,
) -> Option<(Vec<&'tokens Token>, TypeId, bool)> {
    let syntax = snapshot.syntax();
    if let Some((open, value)) = enclosing_match(syntax, tokens, offset)
        && let Some(segment) = current_pattern_segment(tokens, open, offset)
    {
        return Some((
            segment.to_vec(),
            snapshot.semantics().expression_type(value.id)?,
            false,
        ));
    }
    if let Some((segment, value)) = enclosing_is(syntax, tokens, offset) {
        return Some((
            segment,
            snapshot.semantics().expression_type(value.id)?,
            false,
        ));
    }
    let (segment, binding) = enclosing_binding_pattern(syntax, tokens, offset)?;
    Some((segment, snapshot.semantics().value_type(binding.id)?, true))
}

fn enclosing_binding_pattern<'ast, 'tokens>(
    syntax: &'ast crate::ast::Program,
    tokens: &'tokens [&'tokens Token],
    offset: usize,
) -> Option<(Vec<&'tokens Token>, &'ast crate::ast::BindingPattern)> {
    struct Finder<'ast> {
        offset: usize,
        found: Option<&'ast crate::ast::BindingPattern>,
    }

    impl<'ast> Visitor<'ast> for Finder<'ast> {
        fn visit_binding_pattern(&mut self, binding: &'ast crate::ast::BindingPattern) {
            let span = binding.pattern.span;
            if span.start <= self.offset
                && self.offset <= span.end
                && self.found.is_none_or(|previous| {
                    span.end - span.start < previous.pattern.span.end - previous.pattern.span.start
                })
            {
                self.found = Some(binding);
            }
        }
    }

    let mut finder = Finder {
        offset,
        found: None,
    };
    finder.visit_program(syntax);
    let binding = finder.found?;
    let cursor = tokens
        .iter()
        .position(|token| token.span.start >= offset)
        .unwrap_or(tokens.len());
    let start = tokens[..cursor]
        .iter()
        .position(|token| token.span.start >= binding.pattern.span.start)?;
    Some((tokens[start..cursor].to_vec(), binding))
}

fn enclosing_is<'ast, 'tokens>(
    syntax: &'ast crate::ast::Program,
    tokens: &'tokens [&'tokens Token],
    offset: usize,
) -> Option<(Vec<&'tokens Token>, &'ast Expr)> {
    let cursor = tokens
        .iter()
        .position(|token| token.span.start >= offset)
        .unwrap_or(tokens.len());
    let (keyword_index, keyword) = tokens[..cursor]
        .iter()
        .enumerate()
        .rev()
        .find(|(_, token)| matches!(&token.kind, TokenKind::Ident(name) if name == "is"))?;
    let segment = &tokens[keyword_index + 1..cursor];
    if top_level_token(segment, TokenKind::AndAnd).is_some()
        || top_level_token(segment, TokenKind::OrOr).is_some()
        || top_level_token(segment, TokenKind::FatArrow).is_some()
    {
        return None;
    }

    struct Finder<'ast> {
        keyword: crate::ast::Span,
        preceding_end: usize,
        parsed: Option<&'ast Expr>,
        recovered: Option<&'ast Expr>,
    }

    impl<'ast> Visitor<'ast> for Finder<'ast> {
        fn visit_expr(&mut self, expression: &'ast Expr) {
            if let ExprKind::Is {
                value,
                keyword_span,
                ..
            } = &expression.kind
                && *keyword_span == self.keyword
            {
                self.parsed = Some(value);
            }
            if expression.span.end == self.preceding_end
                && self
                    .recovered
                    .is_none_or(|candidate| expression.span.start < candidate.span.start)
            {
                self.recovered = Some(expression);
            }
            visit::walk_expr(self, expression);
        }
    }

    let preceding_end = tokens.get(keyword_index.checked_sub(1)?)?.span.end;
    let mut finder = Finder {
        keyword: keyword.span,
        preceding_end,
        parsed: None,
        recovered: None,
    };
    finder.visit_program(syntax);
    Some((segment.to_vec(), finder.parsed.or(finder.recovered)?))
}

#[derive(Debug)]
enum PatternSite {
    Value {
        expected: TypeId,
        qualified_enum: bool,
    },
    ArrayElement {
        expected: TypeId,
        rest_available: bool,
    },
    StructFields {
        structure: TypeId,
        used: BTreeSet<String>,
    },
}

struct PatternStructField {
    name: String,
    ty: TypeId,
    documentation: Option<String>,
    documentation_uri: Option<String>,
}

struct PatternStruct {
    name: String,
    documentation: Option<String>,
    fields: Vec<PatternStructField>,
}

fn contextual_struct(expected: TypeId, snapshot: &SemanticSnapshot) -> Option<PatternStruct> {
    match snapshot.semantics().types().kind(expected) {
        TypeKind::Struct(structure) => {
            let declaration = snapshot
                .syntax()
                .structs
                .iter()
                .find(|candidate| candidate.id == *structure)?;
            let fields = declaration
                .fields
                .iter()
                .map(|field| {
                    Some(PatternStructField {
                        name: field.name.clone(),
                        ty: snapshot.semantics().struct_field_type(field.id)?,
                        documentation: field.documentation.clone(),
                        documentation_uri: None,
                    })
                })
                .collect::<Option<Vec<_>>>()?;
            Some(PatternStruct {
                name: declaration.name.clone(),
                documentation: declaration.documentation.clone(),
                fields,
            })
        }
        TypeKind::Application {
            constructor,
            arguments,
            ..
        } => {
            let library = snapshot.context().standard_library();
            let declaration = library.type_constructor(*constructor);
            let variables = declaration
                .parameters
                .iter()
                .zip(arguments)
                .map(|(parameter, argument)| (parameter.name, *argument))
                .collect::<HashMap<_, _>>();
            let fields = library
                .fields_of_constructor(*constructor)
                .filter(|field| field.visibility == FieldVisibility::Public)
                .map(|field| {
                    Some(PatternStructField {
                        name: field.name.to_owned(),
                        ty: snapshot
                            .semantics()
                            .instantiated_catalog_type(field.ty, &variables)?,
                        documentation: Some(render_documentation(&field.documentation)),
                        documentation_uri: Some(symbol_uri(
                            StdlibSymbolId::Field(field.id),
                            &library,
                        )),
                    })
                })
                .collect::<Option<Vec<_>>>()?;
            (!fields.is_empty()).then(|| PatternStruct {
                name: declaration.name.to_owned(),
                documentation: Some(render_documentation(&declaration.documentation)),
                fields,
            })
        }
        _ => None,
    }
}

fn enclosing_match<'ast>(
    syntax: &'ast crate::ast::Program,
    tokens: &[&Token],
    offset: usize,
) -> Option<(usize, &'ast Expr)> {
    struct Finder<'ast, 'tokens> {
        tokens: &'tokens [&'tokens Token],
        offset: usize,
        found: Option<(usize, &'ast Expr)>,
    }

    impl<'ast> Visitor<'ast> for Finder<'ast, '_> {
        fn visit_expr(&mut self, expression: &'ast Expr) {
            if let ExprKind::Match { value, .. } = &expression.kind
                && let Some(open) = self.tokens.iter().position(|token| {
                    token.span.start >= value.span.end && token.kind == TokenKind::LBrace
                })
            {
                let close = matching_closing_brace(self.tokens, open)
                    .map_or(usize::MAX, |close| self.tokens[close].span.end);
                if self.tokens[open].span.end <= self.offset
                    && self.offset <= close
                    && self.found.as_ref().is_none_or(|(previous, _)| {
                        self.tokens[*previous].span.start < self.tokens[open].span.start
                    })
                {
                    self.found = Some((open, value));
                }
            }
            visit::walk_expr(self, expression);
        }
    }

    let mut finder = Finder {
        tokens,
        offset,
        found: None,
    };
    finder.visit_program(syntax);
    finder.found
}

/// Returns only the tokens belonging to the current arm's pattern. A top-level
/// `=>` or guard switches back to ordinary expression completion, while commas
/// nested in struct and array patterns remain part of this segment.
fn current_pattern_segment<'a>(
    tokens: &'a [&'a Token],
    open: usize,
    replacement_start: usize,
) -> Option<&'a [&'a Token]> {
    let cursor = tokens
        .iter()
        .position(|token| token.span.start >= replacement_start)
        .unwrap_or(tokens.len());
    let mut start = open + 1;
    let mut parentheses = 0usize;
    let mut brackets = 0usize;
    let mut braces = 0usize;
    let mut in_value = false;

    for (index, token) in tokens.iter().enumerate().take(cursor).skip(open + 1) {
        let top_level = parentheses == 0 && brackets == 0 && braces == 0;
        if top_level {
            match &token.kind {
                TokenKind::Comma => {
                    start = index + 1;
                    in_value = false;
                    continue;
                }
                TokenKind::FatArrow => in_value = true,
                TokenKind::Ident(name) if name == "if" && !in_value => return None,
                _ => {}
            }
        }
        update_depths(&token.kind, &mut parentheses, &mut brackets, &mut braces);
    }

    (!in_value).then_some(&tokens[start..cursor])
}

fn update_depths(
    token: &TokenKind,
    parentheses: &mut usize,
    brackets: &mut usize,
    braces: &mut usize,
) {
    match token {
        TokenKind::LParen => *parentheses += 1,
        TokenKind::RParen => *parentheses = parentheses.saturating_sub(1),
        TokenKind::LBracket => *brackets += 1,
        TokenKind::RBracket => *brackets = brackets.saturating_sub(1),
        TokenKind::LBrace => *braces += 1,
        TokenKind::RBrace => *braces = braces.saturating_sub(1),
        _ => {}
    }
}

fn analyze_pattern_prefix(
    tokens: &[&Token],
    expected: TypeId,
    snapshot: &SemanticSnapshot,
) -> Option<PatternSite> {
    let tokens = after_last_top_level(tokens, TokenKind::Or);
    if tokens.is_empty() {
        return Some(PatternSite::Value {
            expected,
            qualified_enum: false,
        });
    }

    let unmatched = unmatched_openings(tokens);
    let Some(&open) = unmatched.first() else {
        let qualified_enum = matches!(tokens.last().map(|token| &token.kind), Some(TokenKind::Dot))
            && enum_qualifier_matches(&tokens[..tokens.len() - 1], expected, snapshot);
        return qualified_enum.then_some(PatternSite::Value {
            expected,
            qualified_enum: true,
        });
    };

    match tokens[open].kind {
        TokenKind::LParen => {
            let payload = constructor_payload_type(&tokens[..open], expected, snapshot)?;
            analyze_pattern_prefix(&tokens[open + 1..], payload, snapshot)
        }
        TokenKind::LBracket => {
            let TypeKind::Array { element, .. } = snapshot.semantics().types().kind(expected)
            else {
                return None;
            };
            let contents = &tokens[open + 1..];
            let tail = after_last_top_level(contents, TokenKind::Comma);
            if tail.is_empty() {
                Some(PatternSite::ArrayElement {
                    expected: *element,
                    rest_available: top_level_token(contents, TokenKind::DotDot).is_none(),
                })
            } else {
                analyze_pattern_prefix(tail, *element, snapshot)
            }
        }
        TokenKind::LBrace => {
            let declaration = contextual_struct(expected, snapshot)?;
            if let Some(token) = tokens[..open].last() {
                let TokenKind::Ident(name) = &token.kind else {
                    return None;
                };
                if name.as_str() != declaration.name.as_str() {
                    return None;
                }
            }

            let contents = &tokens[open + 1..];
            let chunks = top_level_chunks(contents, TokenKind::Comma);
            let mut used = BTreeSet::new();
            for chunk in chunks.iter().take(chunks.len().saturating_sub(1)) {
                if let Some(TokenKind::Ident(name)) = chunk.first().map(|token| &token.kind) {
                    used.insert(name.clone());
                }
            }
            let tail = chunks.last().copied().unwrap_or_default();
            if let Some(colon) = top_level_token(tail, TokenKind::Colon) {
                let name = tail[..colon].iter().find_map(|token| match &token.kind {
                    TokenKind::Ident(name) => Some(name.as_str()),
                    _ => None,
                })?;
                let field = declaration.fields.iter().find(|field| field.name == name)?;
                analyze_pattern_prefix(&tail[colon + 1..], field.ty, snapshot)
            } else {
                if let Some(TokenKind::Ident(name)) = tail.first().map(|token| &token.kind) {
                    used.insert(name.clone());
                }
                Some(PatternSite::StructFields {
                    structure: expected,
                    used,
                })
            }
        }
        _ => None,
    }
}

fn unmatched_openings(tokens: &[&Token]) -> Vec<usize> {
    let mut stack: Vec<(TokenKind, usize)> = Vec::new();
    for (index, token) in tokens.iter().enumerate() {
        match token.kind {
            TokenKind::LParen | TokenKind::LBracket | TokenKind::LBrace => {
                stack.push((token.kind.clone(), index));
            }
            TokenKind::RParen => close_delimiter(&mut stack, TokenKind::LParen),
            TokenKind::RBracket => close_delimiter(&mut stack, TokenKind::LBracket),
            TokenKind::RBrace => close_delimiter(&mut stack, TokenKind::LBrace),
            _ => {}
        }
    }
    stack.into_iter().map(|(_, index)| index).collect()
}

fn close_delimiter(stack: &mut Vec<(TokenKind, usize)>, opening: TokenKind) {
    if stack.last().is_some_and(|(kind, _)| *kind == opening) {
        stack.pop();
    }
}

fn after_last_top_level<'a>(tokens: &'a [&'a Token], separator: TokenKind) -> &'a [&'a Token] {
    top_level_chunks(tokens, separator)
        .last()
        .copied()
        .unwrap_or_default()
}

fn top_level_chunks<'a>(tokens: &'a [&'a Token], separator: TokenKind) -> Vec<&'a [&'a Token]> {
    let mut chunks = Vec::new();
    let mut start = 0;
    let mut parentheses = 0usize;
    let mut brackets = 0usize;
    let mut braces = 0usize;
    for (index, token) in tokens.iter().enumerate() {
        if parentheses == 0 && brackets == 0 && braces == 0 && token.kind == separator {
            chunks.push(&tokens[start..index]);
            start = index + 1;
            continue;
        }
        update_depths(&token.kind, &mut parentheses, &mut brackets, &mut braces);
    }
    chunks.push(&tokens[start..]);
    chunks
}

fn top_level_token(tokens: &[&Token], searched: TokenKind) -> Option<usize> {
    let mut parentheses = 0usize;
    let mut brackets = 0usize;
    let mut braces = 0usize;
    for (index, token) in tokens.iter().enumerate() {
        if parentheses == 0 && brackets == 0 && braces == 0 && token.kind == searched {
            return Some(index);
        }
        update_depths(&token.kind, &mut parentheses, &mut brackets, &mut braces);
    }
    None
}

fn constructor_payload_type(
    head: &[&Token],
    expected: TypeId,
    snapshot: &SemanticSnapshot,
) -> Option<TypeId> {
    let name = head.iter().rev().find_map(|token| match &token.kind {
        TokenKind::Ident(name) => Some(name.as_str()),
        _ => None,
    })?;
    match (name, snapshot.semantics().types().kind(expected)) {
        ("Some", TypeKind::Option { value, .. }) | ("Ok", TypeKind::Result { value, .. }) => {
            Some(*value)
        }
        ("Err", TypeKind::Result { .. }) => Some(
            snapshot
                .semantics()
                .types()
                .id_for_standard(StdlibTypeId::String),
        ),
        (
            "Item",
            TypeKind::Application {
                constructor,
                arguments,
                ..
            },
        ) if *constructor == StdlibTypeConstructorId::IteratorStep => arguments.first().copied(),
        (variant, TypeKind::Enum(enumeration)) => {
            let declaration = snapshot
                .enum_types()
                .iter()
                .find(|candidate| candidate.id == *enumeration)?;
            let variant = declaration
                .variants
                .iter()
                .find(|candidate| candidate.name == variant)?;
            snapshot.semantics().enum_variant_payload(variant.id)
        }
        _ => None,
    }
}

fn enum_qualifier_matches(head: &[&Token], expected: TypeId, snapshot: &SemanticSnapshot) -> bool {
    let Some(TokenKind::Ident(name)) = head.last().map(|token| &token.kind) else {
        return false;
    };
    match snapshot.semantics().types().kind(expected) {
        TypeKind::Enum(enumeration) => snapshot
            .enum_types()
            .iter()
            .any(|candidate| candidate.id == *enumeration && candidate.name == *name),
        TypeKind::Standard(standard) => {
            snapshot
                .context()
                .standard_library()
                .type_decl(*standard)
                .name
                == name
        }
        _ => false,
    }
}

fn add_value_patterns(
    builder: &mut CompletionBuilder,
    expected: TypeId,
    qualified_enum: bool,
    binding_pattern: bool,
    snapshot: &SemanticSnapshot,
    library: &StandardLibrary,
) {
    let detail = format!("pattern for `{}`", display_type(expected, snapshot));
    if !qualified_enum {
        builder.add(pattern_item("_", "_", false, &detail, None, None));
    }
    match snapshot.semantics().types().kind(expected) {
        TypeKind::Builtin(CoreTypeId::Bool) => {
            add_plain(builder, "false", &detail);
            add_plain(builder, "true", &detail);
        }
        TypeKind::Builtin(CoreTypeId::Char) => builder.add(pattern_item(
            "character literal",
            "'${1:x}'",
            true,
            &detail,
            Some("Matches one character value.".to_owned()),
            None,
        )),
        TypeKind::Builtin(core)
            if library.core_type_has_capability(*core, StdlibCapabilityId::Integer) =>
        {
            builder.add(pattern_item(
                "integer literal",
                "${1:0}",
                true,
                &detail,
                Some("Matches one integer value, including a negative signed value.".to_owned()),
                None,
            ));
            builder.add(pattern_item(
                "exclusive integer range",
                "${1:0}..<${2:10}",
                true,
                &detail,
                Some(
                    "Matches integers from the lower bound up to, but excluding, the upper bound."
                        .to_owned(),
                ),
                None,
            ));
            builder.add(pattern_item(
                "inclusive integer range",
                "${1:0}..=${2:10}",
                true,
                &detail,
                Some("Matches integers from the lower bound through the upper bound.".to_owned()),
                None,
            ));
        }
        TypeKind::Standard(StdlibTypeId::String) => builder.add(pattern_item(
            "string literal",
            "\"${1:value}\"",
            true,
            &detail,
            Some("Matches one string value.".to_owned()),
            None,
        )),
        TypeKind::Standard(StdlibTypeId::FileVersion) => builder.add(pattern_item(
            "file-version literal",
            "v\"${1:1.0.0.0}\"",
            true,
            &detail,
            Some("Matches one four-component file version.".to_owned()),
            None,
        )),
        TypeKind::Standard(standard) => {
            let owner = library.type_decl(*standard);
            for variant in library.variants_of(*standard) {
                let label = if qualified_enum {
                    variant.name.to_owned()
                } else {
                    format!("{}.{}", owner.name, variant.name)
                };
                builder.add(pattern_item(
                    &label,
                    &label,
                    false,
                    &detail,
                    Some(render_documentation(&variant.documentation)),
                    Some(symbol_uri(StdlibSymbolId::Variant(variant.id), library)),
                ));
            }
        }
        TypeKind::Option { .. } => {
            add_plain(builder, "None", &detail);
            add_snippet(builder, "Some", "Some(${1:_})", &detail);
        }
        TypeKind::Result { .. } => {
            add_snippet(builder, "Err", "Err(${1:_})", &detail);
            add_snippet(builder, "Ok", "Ok(${1:_})", &detail);
        }
        TypeKind::Application { constructor, .. }
            if *constructor == StdlibTypeConstructorId::IteratorStep =>
        {
            add_plain(builder, "End", &detail);
            add_snippet(builder, "Item", "Item(${1:_})", &detail);
        }
        TypeKind::Enum(enumeration) => {
            let Some(declaration) = snapshot
                .enum_types()
                .iter()
                .find(|candidate| candidate.id == *enumeration)
            else {
                return;
            };
            for variant in &declaration.variants {
                let label = if qualified_enum {
                    variant.name.clone()
                } else {
                    format!("{}.{}", declaration.name, variant.name)
                };
                let insert = if variant.payload.is_some() {
                    format!("{label}(${{1:_}})")
                } else {
                    label.clone()
                };
                builder.add(pattern_item(
                    &label,
                    &insert,
                    variant.payload.is_some(),
                    &detail,
                    variant.documentation.clone(),
                    None,
                ));
            }
        }
        TypeKind::Struct(_) => {
            let Some(declaration) = contextual_struct(expected, snapshot) else {
                return;
            };
            let fields = declaration
                .fields
                .iter()
                .enumerate()
                .map(|(index, field)| {
                    if binding_pattern {
                        field.name.clone()
                    } else {
                        format!("{}: ${{{}:_}}", field.name, index + 1)
                    }
                })
                .collect::<Vec<_>>()
                .join(", ");
            let anonymous_insert = if declaration.fields.is_empty() {
                "{}".to_owned()
            } else {
                format!("{{ {fields} }}")
            };
            builder.add(pattern_item(
                "struct fields",
                &anonymous_insert,
                !binding_pattern && !declaration.fields.is_empty(),
                &detail,
                Some(format!(
                    "Destructures `{}` using the struct type supplied by this context.",
                    declaration.name
                )),
                None,
            ));

            let named_insert = if declaration.fields.is_empty() {
                format!("{} {{}}", declaration.name)
            } else {
                format!("{} {{ {fields} }}", declaration.name)
            };
            builder.add(pattern_item(
                &declaration.name,
                &named_insert,
                !binding_pattern && !declaration.fields.is_empty(),
                &detail,
                declaration.documentation.clone(),
                None,
            ));
        }
        TypeKind::Application { .. } => {
            let Some(declaration) = contextual_struct(expected, snapshot) else {
                return;
            };
            let fields = declaration
                .fields
                .iter()
                .enumerate()
                .map(|(index, field)| {
                    if binding_pattern {
                        field.name.clone()
                    } else {
                        format!("{}: ${{{}:_}}", field.name, index + 1)
                    }
                })
                .collect::<Vec<_>>()
                .join(", ");
            let anonymous_insert = format!("{{ {fields} }}");
            builder.add(pattern_item(
                "struct fields",
                &anonymous_insert,
                !binding_pattern,
                &detail,
                Some(format!(
                    "Destructures `{}` using the struct type supplied by this context.",
                    declaration.name
                )),
                None,
            ));
            let named_insert = format!("{} {{ {fields} }}", declaration.name);
            builder.add(pattern_item(
                &declaration.name,
                &named_insert,
                !binding_pattern,
                &detail,
                declaration.documentation,
                None,
            ));
        }
        TypeKind::Array {
            length: Some(length),
            ..
        } if *length <= 32 => {
            let elements = (0..*length)
                .map(|index| format!("${{{}:_}}", index + 1))
                .collect::<Vec<_>>()
                .join(", ");
            builder.add(pattern_item(
                "fixed-array pattern",
                &format!("[{elements}]"),
                *length != 0,
                &detail,
                Some(format!("Matches all {length} elements of the fixed array.")),
                None,
            ));
            add_array_rest_pattern(builder, "array rest pattern", "[..]");
        }
        TypeKind::Array { .. } => {
            add_array_rest_pattern(builder, "array rest pattern", "[..]");
        }
        _ => {}
    }
}

fn add_array_rest_pattern(builder: &mut CompletionBuilder, label: &str, insert: &str) {
    let item = LanguageCatalog::new().item(LanguageItemId::ArrayRestPattern);
    builder.add(CompletionItem {
        label: label.to_owned(),
        kind: CompletionKind::Keyword,
        detail: Some(item.form.to_owned()),
        documentation: Some(render_documentation(&item.documentation)),
        documentation_uri: Some(language_item_uri(item.id)),
        insert_text: insert.to_owned(),
        is_snippet: false,
    });
}

fn add_struct_fields(
    builder: &mut CompletionBuilder,
    structure: TypeId,
    used: &BTreeSet<String>,
    binding_pattern: bool,
    snapshot: &SemanticSnapshot,
) {
    let Some(declaration) = contextual_struct(structure, snapshot) else {
        return;
    };
    for field in &declaration.fields {
        if used.contains(&field.name) {
            continue;
        }
        let detail = format!("{}: {}", field.name, display_type(field.ty, snapshot));
        let insert = if binding_pattern {
            field.name.clone()
        } else {
            format!("{}: ${{1:_}}", field.name)
        };
        builder.add(pattern_item(
            &field.name,
            &insert,
            !binding_pattern,
            &detail,
            field.documentation.clone(),
            field.documentation_uri.clone(),
        ));
    }
}

fn add_plain(builder: &mut CompletionBuilder, label: &str, detail: &str) {
    builder.add(pattern_item(label, label, false, detail, None, None));
}

fn add_snippet(builder: &mut CompletionBuilder, label: &str, insert: &str, detail: &str) {
    builder.add(pattern_item(label, insert, true, detail, None, None));
}

fn pattern_item(
    label: &str,
    insert_text: &str,
    is_snippet: bool,
    detail: &str,
    documentation: Option<String>,
    documentation_uri: Option<String>,
) -> CompletionItem {
    CompletionItem {
        label: label.to_owned(),
        kind: if is_snippet {
            CompletionKind::Snippet
        } else if label == "_" {
            CompletionKind::Variable
        } else {
            CompletionKind::EnumMember
        },
        detail: Some(detail.to_owned()),
        documentation,
        documentation_uri,
        insert_text: insert_text.to_owned(),
        is_snippet,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn completions(source: &str) -> CompletionList {
        let offset = source.find("<|>").expect("completion marker exists");
        let source = source.replacen("<|>", "", 1);
        CompilerDatabase::new(source)
            .completions(offset)
            .expect("completion succeeds")
    }

    fn labels(source: &str) -> Vec<String> {
        completions(source)
            .items
            .into_iter()
            .map(|item| item.label)
            .collect()
    }

    #[test]
    fn root_pattern_completion_is_type_directed_and_suppresses_expressions() {
        let source = r#"
state "game.exe" {}
fn observe(value: bool) {
    return match value {
        <|>true => 1,
        false => 0,
    }
}
"#;
        let labels = labels(source);
        for expected in ["_", "false", "true"] {
            assert!(labels.contains(&expected.to_owned()), "{labels:#?}");
        }
        for expression in ["print", "return", "String", "while"] {
            assert!(!labels.contains(&expression.to_owned()), "{labels:#?}");
        }
    }

    #[test]
    fn is_pattern_completion_is_type_directed_even_while_recovering() {
        let root = labels(
            r#"
state "game.exe" {}
fn observe(value: bool?) {
    if value is <|> {
        print("matched")
    }
}
"#,
        );
        for expected in ["_", "None", "Some"] {
            assert!(root.contains(&expected.to_owned()), "{root:#?}");
        }
        for expression in ["print", "return", "String", "while"] {
            assert!(!root.contains(&expression.to_owned()), "{root:#?}");
        }

        let nested = labels(
            r#"
state "game.exe" {}
fn observe(value: bool?) {
    if value is Some(<|>) {
        print("matched")
    }
}
"#,
        );
        assert!(nested.contains(&"true".to_owned()), "{nested:#?}");
        assert!(nested.contains(&"false".to_owned()), "{nested:#?}");
        assert!(!nested.contains(&"Some".to_owned()), "{nested:#?}");
    }

    #[test]
    fn integer_pattern_completion_offers_both_explicit_range_shapes() {
        let items = completions(
            r#"
state "game.exe" {}
fn observe(value: i16) {
    return match value {
        <|>
    }
}
"#,
        )
        .items;
        for (label, insert) in [
            ("exclusive integer range", "${1:0}..<${2:10}"),
            ("inclusive integer range", "${1:0}..=${2:10}"),
        ] {
            let item = items
                .iter()
                .find(|item| item.label == label)
                .unwrap_or_else(|| panic!("missing `{label}` in {items:#?}"));
            assert_eq!(item.insert_text, insert);
            assert!(item.is_snippet);
        }
    }

    #[test]
    fn completion_survives_an_empty_recovered_match_arm() {
        let source = r#"
state "game.exe" {}
fn observe(value: bool) {
    return match value {
        <|>
    }
}
"#;
        let labels = labels(source);
        assert!(labels.contains(&"true".to_owned()), "{labels:#?}");
        assert!(labels.contains(&"false".to_owned()), "{labels:#?}");
        assert!(!labels.contains(&"print".to_owned()));
    }

    #[test]
    fn enum_completion_supports_qualified_and_payload_variants() {
        let source = r#"
enum Mode { Idle, Active(bool) }
state "game.exe" {}
fn observe(mode: Mode) {
    return match mode {
        <|>Mode.Idle => false,
        Mode.Active(value) => value,
    }
}
"#;
        let completion = completions(source);
        let active = completion
            .items
            .iter()
            .find(|item| item.label == "Mode.Active")
            .expect("payload variant is completed");
        assert_eq!(active.insert_text, "Mode.Active(${1:_})");
        assert!(active.is_snippet);

        let qualified = labels(&source.replace("<|>Mode.Idle", "Mode.<|>Idle"));
        assert!(qualified.contains(&"Idle".to_owned()), "{qualified:#?}");
        assert!(qualified.contains(&"Active".to_owned()), "{qualified:#?}");
        assert!(!qualified.contains(&"Mode.Idle".to_owned()));
    }

    #[test]
    fn standard_enum_completion_uses_the_same_qualification_rules() {
        let source = r#"
state "game.exe" {}
fn observe(value: TimerState) {
    return match value {
        <|>TimerState.Running => true,
        _ => false,
    }
}
"#;
        let root_labels = labels(source);
        assert!(
            root_labels.contains(&"TimerState.Running".to_owned()),
            "{root_labels:#?}"
        );
        assert!(!root_labels.contains(&"Running".to_owned()));

        let qualified = labels(&source.replace("<|>TimerState.Running", "TimerState.<|>Running"));
        assert!(qualified.contains(&"Running".to_owned()), "{qualified:#?}");
        assert!(!qualified.contains(&"TimerState.Running".to_owned()));
    }

    #[test]
    fn wrapper_and_array_payloads_complete_recursively() {
        let option = r#"
state "game.exe" {}
fn observe(value: bool?) {
    return match value {
        Some(<|>_) => true,
        None => false,
    }
}
"#;
        let option_labels = labels(option);
        assert!(
            option_labels.contains(&"true".to_owned()),
            "{option_labels:#?}"
        );
        assert!(
            option_labels.contains(&"false".to_owned()),
            "{option_labels:#?}"
        );
        assert!(!option_labels.contains(&"Some".to_owned()));

        let array = r#"
state "game.exe" {}
fn observe(value: [bool; 2]) {
    return match value {
        [true, <|>_] => true,
        _ => false,
    }
}
"#;
        let labels = labels(array);
        assert!(labels.contains(&"true".to_owned()), "{labels:#?}");
        assert!(labels.contains(&"false".to_owned()), "{labels:#?}");
        assert!(!labels.contains(&"fixed-array pattern".to_owned()));
        assert!(labels.contains(&"array rest".to_owned()), "{labels:#?}");
    }

    #[test]
    fn array_rest_completion_is_offered_only_once_per_array_pattern() {
        let source = r#"
state "game.exe" {}
fn observe(value: [bool]) {
    return match value {
        [true, <|>_] => true,
        _ => false,
    }
}
"#;
        let initial_labels = labels(source);
        assert!(
            initial_labels.contains(&"array rest".to_owned()),
            "{initial_labels:#?}"
        );

        let after_rest = labels(&source.replace("[true, <|>_]", "[true, .., <|>_]"));
        assert!(
            !after_rest.contains(&"array rest".to_owned()),
            "{after_rest:#?}"
        );
        assert!(after_rest.contains(&"true".to_owned()), "{after_rest:#?}");
    }

    #[test]
    fn result_and_iterator_step_completion_offer_only_their_constructors() {
        let result = r#"
state "game.exe" {}
fn observe(value: bool!) {
    return match value {
        <|>Ok(_) => true,
        Err(_) => false,
    }
}
"#;
        let result_labels = labels(result);
        for expected in ["_", "Err", "Ok"] {
            assert!(
                result_labels.contains(&expected.to_owned()),
                "{result_labels:#?}"
            );
        }
        assert!(!result_labels.contains(&"Some".to_owned()));

        let step = r#"
state "game.exe" {}
fn observe(value: IteratorStep<bool>) {
    return match value {
        <|>Item(_) => true,
        End => false,
    }
}
"#;
        let labels = labels(step);
        for expected in ["_", "End", "Item"] {
            assert!(labels.contains(&expected.to_owned()), "{labels:#?}");
        }
        assert!(!labels.contains(&"None".to_owned()));
    }

    #[test]
    fn completion_survives_an_unclosed_wrapper_pattern() {
        let source = r#"
state "game.exe" {}
fn observe(value: bool?) {
    return match value {
        Some(<|>
    }
}
"#;
        let labels = labels(source);
        assert!(labels.contains(&"true".to_owned()), "{labels:#?}");
        assert!(labels.contains(&"false".to_owned()), "{labels:#?}");
        assert!(!labels.contains(&"Some".to_owned()));
    }

    #[test]
    fn struct_patterns_complete_only_remaining_fields_and_field_patterns() {
        let source = r#"
struct Position { visible: bool, grounded: bool }
state "game.exe" {}
fn observe(position: Position) {
    return match position {
        Position { visible: _, <|>grounded: _ } => true,
        _ => false,
    }
}
"#;
        let completion = completions(source);
        assert_eq!(
            completion
                .items
                .iter()
                .map(|item| item.label.as_str())
                .collect::<Vec<_>>(),
            ["grounded"]
        );
        assert_eq!(completion.items[0].insert_text, "grounded: ${1:_}");

        let field = source.replace("<|>grounded: _", "grounded: <|>_");
        let labels = labels(&field);
        assert!(labels.contains(&"true".to_owned()), "{labels:#?}");
        assert!(labels.contains(&"false".to_owned()), "{labels:#?}");

        let anonymous = source.replace(
            "Position { visible: _, <|>grounded: _ }",
            "{ visible: _, <|>grounded: _ }",
        );
        let completion = completions(&anonymous);
        assert_eq!(
            completion
                .items
                .iter()
                .map(|item| item.label.as_str())
                .collect::<Vec<_>>(),
            ["grounded"]
        );
    }

    #[test]
    fn declaration_destructuring_uses_type_directed_pattern_completion() {
        let parameter = r#"
struct Position { x: u16, y: u16 }
state "game.exe" {}
fn observe({ x, <|> }: Position) -> u16 {
    return x
}
"#;
        let completion = completions(parameter);
        assert_eq!(
            completion
                .items
                .iter()
                .map(|item| item.label.as_str())
                .collect::<Vec<_>>(),
            ["y"]
        );
        assert_eq!(completion.items[0].insert_text, "y");
        assert!(!completion.items[0].is_snippet);

        let inferred_parameter = parameter
            .replace("{ x, <|> }", "Position { x, <|> }")
            .replace(": Position) -> u16", ") -> u16");
        let completion = completions(&inferred_parameter);
        assert_eq!(
            completion
                .items
                .iter()
                .map(|item| item.label.as_str())
                .collect::<Vec<_>>(),
            ["y"]
        );

        for declaration in [
            r#"
struct Position { x: u16, y: u16 }
state "game.exe" {}
fn observe(position: Position) -> u16 {
    let { x, <|> } = position
    return x
}
"#,
            r#"
struct Position { x: u16, y: u16 }
state "game.exe" {}
fn observe(positions: [Position]) {
    for { x, <|> } in positions { print(x) }
}
"#,
            r#"
struct Position { x: u16, y: u16 }
state "game.exe" {}
fn observe(position: Position) -> u16 {
    let read = ({ x, <|> }: Position) => x
    return read(position)
}
"#,
        ] {
            let completion = completions(declaration);
            assert_eq!(
                completion
                    .items
                    .iter()
                    .map(|item| item.label.as_str())
                    .collect::<Vec<_>>(),
                ["y"]
            );
        }
    }

    #[test]
    fn contextual_struct_completion_offers_anonymous_and_named_shapes() {
        let completion = completions(
            r#"
struct Position { x: u16, y: u16 }
state "game.exe" {}
fn observe(position: Position) -> u16 {
    return match position {
        <|>
    }
}
"#,
        );
        let anonymous = completion
            .items
            .iter()
            .find(|item| item.label == "struct fields")
            .expect("contextual struct pattern completion exists");
        assert_eq!(anonymous.insert_text, "{ x: ${1:_}, y: ${2:_} }");
        assert!(anonymous.is_snippet);
        assert!(completion.items.iter().any(|item| item.label == "Position"));
    }

    #[test]
    fn fixed_array_completion_provides_the_exact_shape() {
        let source = r#"
state "game.exe" {}
fn observe(value: [bool; 2]) {
    return match value {
        <|>_ => true,
    }
}
"#;
        let completion = completions(source);
        let array = completion
            .items
            .iter()
            .find(|item| item.label == "fixed-array pattern")
            .expect("fixed array shape is completed");
        assert_eq!(array.insert_text, "[${1:_}, ${2:_}]");
        assert!(array.is_snippet);
    }

    #[test]
    fn match_arm_values_keep_ordinary_expression_completion() {
        let source = r#"
state "game.exe" {}
fn observe(value: bool) {
    return match value {
        true => <|>false,
        false => false,
    }
}
"#;
        let labels = labels(source);
        assert!(labels.contains(&"print".to_owned()), "{labels:#?}");
        assert!(!labels.contains(&"fixed-array pattern".to_owned()));
    }
}
