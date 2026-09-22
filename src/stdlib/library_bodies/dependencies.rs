//! Conservative source-body dependencies, before receiver types are known.
//!
//! Every identifier retains all callable candidates with that name. This
//! includes function values, calls inside closures/interpolations, and bodies
//! of unused user functions. Extra candidates cost time but preserve ordinary
//! inference and diagnostics. Hidden compiler calls are rooted separately.

use std::collections::{HashMap, HashSet};

use super::{RenderedLibraryBodies, lex_tokens};
use crate::{
    lexer::{Token, TokenKind},
    stdlib::{Implementation, ItemKind, StandardLibrary, StateProviderAttachment, StdlibItemId},
};

#[derive(Debug)]
pub(super) struct BodyDependencies {
    names: HashMap<&'static str, Vec<usize>>,
    edges: Vec<Vec<usize>>,
    roots: Vec<usize>,
    token_ranges: Vec<std::ops::Range<usize>>,
}

impl BodyDependencies {
    pub(super) fn build(
        library: &StandardLibrary,
        rendered: &RenderedLibraryBodies,
    ) -> Option<Self> {
        let tokens = rendered.tokens.as_ref().ok()?;
        let mut items = HashMap::<_, Vec<_>>::new();
        let mut token_ranges = Vec::new();
        for (index, (range, name)) in rendered.body_ranges.iter().enumerate() {
            let item = library.item_by_name_including_private(name)?;
            items.entry(item.id).or_default().push(index);
            token_ranges.push(
                tokens.partition_point(|token| token.span.start < range.start)
                    ..tokens.partition_point(|token| token.span.start < range.end),
            );
        }
        let mut names = HashMap::<_, Vec<_>>::new();
        for item in library.all_items() {
            let Some(bodies) = items.get(&item.id) else {
                continue;
            };
            names.entry(item.name).or_default().extend(bodies);
            match item.implementation {
                Implementation::LibraryBody { function_name, .. } => {
                    names.entry(function_name).or_default().extend(bodies);
                }
                Implementation::LibraryOverloads { cases, .. } => {
                    for case in cases {
                        // Always retain an overload family together.
                        names.entry(case.function_name).or_default().extend(bodies);
                    }
                }
                _ => unreachable!(),
            }
        }
        // Provider calls need not appear in authored expressions. Recognizing
        // all provider names also covers state-provider alternatives.
        let mut roots = Vec::new();
        for provider in library.state_providers() {
            let mut dependencies = Vec::new();
            if let StateProviderAttachment::Callable(item) = provider.attachment {
                dependencies.extend(items.get(&item).into_iter().flatten().copied());
            }
            for item in provider
                .validation
                .into_iter()
                .chain([provider.direct_read])
            {
                dependencies.extend(items.get(&item).into_iter().flatten().copied());
            }
            if provider.default {
                roots.extend(&dependencies);
            }
            names.entry(provider.name).or_default().extend(dependencies);
        }
        // Capability calls can be implicit (operators, formatting, for loops,
        // indexing) or become concrete only during generic specialization.
        let capability_names = library
            .all_items()
            .iter()
            .filter(|item| item.implementation == Implementation::CapabilityRequirement)
            .map(|item| item.name)
            .collect::<HashSet<_>>();
        for item in library.all_items() {
            if item.kind == ItemKind::Constant
                || item.binary_operator.is_some()
                || item.unary_operator.is_some()
                || capability_names.contains(item.name)
            {
                roots.extend(items.get(&item.id).into_iter().flatten().copied());
            }
        }
        for ty in library.all_types() {
            if let Some(display) = ty.display {
                roots.extend(items.get(&display).into_iter().flatten().copied());
            }
        }
        // `component<T>` is a language-generated managed call, not a catalog
        // call to the traversal helper by its source name.
        names.entry("component").or_default().extend(
            items
                .get(&StdlibItemId::UnityGameObjectComponentAddress)
                .into_iter()
                .flatten()
                .copied(),
        );
        let mut result = Self {
            names,
            edges: Vec::new(),
            roots,
            token_ranges,
        };
        result.edges = result
            .token_ranges
            .iter()
            .map(|range| result.references(&tokens[range.clone()]))
            .collect();
        // Provider specialization rewrites the generic preparation call after
        // parsing. Keep both possible replacements in its dependency closure.
        if let Some(generic) = library.item_by_name_including_private("Unity.providerIl2cpp") {
            for name in ["Unity.providerIl2cpp32", "Unity.providerIl2cpp64"] {
                if let Some(specialized) = library.item_by_name_including_private(name) {
                    for &body in items.get(&generic.id).into_iter().flatten() {
                        result.edges[body]
                            .extend(items.get(&specialized.id).into_iter().flatten().copied());
                    }
                }
            }
        }
        Some(result)
    }

    fn references(&self, tokens: &[Token]) -> Vec<usize> {
        let mut references = Vec::new();
        for token in tokens {
            if let TokenKind::Ident(name) = &token.kind {
                references.extend(self.names.get(name.as_str()).into_iter().flatten().copied());
            }
        }
        references.sort_unstable();
        references.dedup();
        references
    }

    pub(super) fn select(
        &self,
        rendered: &RenderedLibraryBodies,
        user: &str,
        generated: &str,
    ) -> Option<RenderedLibraryBodies> {
        // Preserve the ordinary recovery path when a fragment is malformed.
        let mut pending = self.references(&lex_tokens(user).ok()?);
        pending.extend(self.references(&lex_tokens(generated).ok()?));
        pending.extend(&self.roots);
        let mut selected = vec![false; self.edges.len()];
        while let Some(index) = pending.pop() {
            if std::mem::replace(&mut selected[index], true) {
                continue;
            }
            pending.extend(&self.edges[index]);
        }
        let original_tokens = rendered.tokens.as_ref().ok()?;
        let mut source = String::new();
        let mut body_ranges = Vec::new();
        let mut tokens = Vec::new();
        for (index, (range, name)) in rendered.body_ranges.iter().enumerate() {
            if !selected[index] {
                continue;
            }
            let start = source.len();
            source.push_str(&rendered.source[range.clone()]);
            body_ranges.push((start..source.len(), *name));
            source.push('\n');
            tokens.extend(
                original_tokens[self.token_ranges[index].clone()]
                    .iter()
                    .map(|token| {
                        let mut token = token.clone();
                        token.span.start = start + token.span.start - range.start;
                        token.span.end = start + token.span.end - range.start;
                        token
                    }),
            );
        }
        if source.ends_with('\n') {
            source.pop();
        }
        let mut eof = original_tokens.last()?.clone();
        eof.span.start = source.len();
        eof.span.end = source.len();
        tokens.push(eof);
        Some(RenderedLibraryBodies {
            source,
            body_ranges,
            tokens: Ok(tokens),
            dependencies: None,
        })
    }
}
