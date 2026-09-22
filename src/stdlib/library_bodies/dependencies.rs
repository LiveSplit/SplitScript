//! Conservative source-body selection before user receiver types are known.
//!
//! User identifiers retain all callable candidates with that name. This
//! includes function values, calls inside closures/interpolations, and bodies
//! of unused user functions. Extra candidates cost time but preserve ordinary
//! inference and diagnostics. Within the library, bootstrap-resolved calls
//! select precise dependencies. Hidden compiler calls are rooted separately.

use std::{
    collections::{HashMap, HashSet},
    sync::OnceLock,
};

use super::{RenderedLibraryBodies, lex_tokens};
use crate::{
    ast::{Expr, FunctionId, Program},
    lexer::{Token, TokenKind},
    semantic::{ResolvedCall, ResolvedValue, SemanticModel},
    stdlib::{Implementation, ItemKind, StandardLibrary, StateProviderAttachment, StdlibItemId},
    visit::{self, Visitor},
};

#[derive(Debug)]
pub(super) struct BodyDependencies {
    names: HashMap<&'static str, Vec<usize>>,
    items: HashMap<StdlibItemId, Vec<usize>>,
    generated_edges: Vec<Vec<usize>>,
    resolved_edges: OnceLock<Vec<Vec<usize>>>,
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
        // The parser expands `v"1.2.3.4"` to FileVersion.fromParts; its
        // constructor name is absent from the authored token stream.
        names.entry("v").or_default().extend(
            items
                .get(&StdlibItemId::FileVersionFromParts)
                .into_iter()
                .flatten()
                .copied(),
        );
        let mut result = Self {
            names,
            generated_edges: vec![Vec::new(); token_ranges.len()],
            resolved_edges: OnceLock::new(),
            items,
            roots,
            token_ranges,
        };
        // Provider specialization rewrites the generic preparation call after
        // parsing. Keep both possible replacements in its dependency closure.
        if let Some(generic) = library.item_by_name_including_private("Unity.providerIl2cpp") {
            for name in ["Unity.providerIl2cpp32", "Unity.providerIl2cpp64"] {
                if let Some(specialized) = library.item_by_name_including_private(name) {
                    for &body in result.items.get(&generic.id).into_iter().flatten() {
                        result.generated_edges[body].extend(
                            result
                                .items
                                .get(&specialized.id)
                                .into_iter()
                                .flatten()
                                .copied(),
                        );
                    }
                }
            }
        }
        Some(result)
    }

    /// The bootstrap already resolved the library in isolation. Cache only
    /// catalog body indices, never its compilation-owned expression/type IDs.
    pub(super) fn record_resolved(&self, program: &Program, semantics: &SemanticModel) {
        self.resolved_edges.get_or_init(|| {
            let functions = program
                .functions
                .iter()
                .filter_map(|function| {
                    self.names
                        .get(function.name.as_str())
                        .map(|bodies| (function.id, bodies.as_slice()))
                })
                .collect::<HashMap<_, _>>();
            let mut edges = self.generated_edges.clone();
            for function in &program.functions {
                let Some(owners) = functions.get(&function.id) else {
                    continue;
                };
                let mut collector = ResolvedDependencies {
                    index: self,
                    semantics,
                    functions: &functions,
                    bodies: Vec::new(),
                };
                collector.visit_function(function);
                for &owner in *owners {
                    edges[owner].extend(&collector.bodies);
                }
            }
            for dependencies in &mut edges {
                dependencies.sort_unstable();
                dependencies.dedup();
            }
            edges
        });
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
        let edges = self.resolved_edges.get()?;
        // Preserve the ordinary recovery path when a fragment is malformed.
        let mut pending = self.references(&lex_tokens(user).ok()?);
        pending.extend(self.references(&lex_tokens(generated).ok()?));
        pending.extend(&self.roots);
        let mut selected = vec![false; edges.len()];
        while let Some(index) = pending.pop() {
            if std::mem::replace(&mut selected[index], true) {
                continue;
            }
            pending.extend(&edges[index]);
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

struct ResolvedDependencies<'a> {
    index: &'a BodyDependencies,
    semantics: &'a SemanticModel,
    functions: &'a HashMap<FunctionId, &'a [usize]>,
    bodies: Vec<usize>,
}

impl ResolvedDependencies<'_> {
    fn item(&mut self, item: StdlibItemId) {
        self.bodies
            .extend(self.index.items.get(&item).into_iter().flatten().copied());
    }

    fn function(&mut self, function: FunctionId) {
        self.bodies.extend(
            self.functions
                .get(&function)
                .into_iter()
                .flat_map(|bodies| bodies.iter())
                .copied(),
        );
    }
}

impl<'ast> Visitor<'ast> for ResolvedDependencies<'_> {
    fn visit_expr(&mut self, expression: &'ast Expr) {
        if let Some(call) = self.semantics.call(expression.id) {
            match call {
                ResolvedCall::StandardLibrary { item, .. } => self.item(*item),
                ResolvedCall::UserFunction { function, .. }
                | ResolvedCall::UserMethod { function, .. } => self.function(*function),
                ResolvedCall::ManagedComponent { .. } => {
                    self.item(StdlibItemId::UnityGameObjectComponentAddress)
                }
                ResolvedCall::ManagedSnapshot { .. }
                | ResolvedCall::ManagedInstances { .. }
                | ResolvedCall::ResultError { .. }
                | ResolvedCall::OptionSome { .. }
                | ResolvedCall::IteratorItem { .. }
                | ResolvedCall::ResultSuccess { .. } => {}
            }
        }
        if let Some(function) = self.semantics.function_value(expression.id) {
            self.function(function.function);
        }
        if let Some(ResolvedValue::StandardLibraryConstant(item)) =
            self.semantics.value(expression.id)
        {
            self.item(item);
        }
        // Includes nested closures and interpolation expressions. Implicit
        // capability/formatting implementations remain unconditional roots.
        visit::walk_expr(self, expression);
    }
}
