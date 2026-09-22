//! Navigable standard-library reference pages over compiler-owned catalogs.

use std::{
    collections::{HashMap, HashSet},
    sync::{Arc, Mutex, OnceLock},
};

use crate::{
    catalog::Documentation,
    language::{LanguageCatalog, LanguageItem, LanguageItemId, LanguageItemKind},
    migration::{
        ForeignSpellingReplacement, MigrationCatalog, MigrationConcept, MigrationConceptId,
        MigrationNavigationGroup, MigrationSupport, MigrationTarget, migration_navigation_group,
    },
    stdlib::{
        CoreTypeId, FieldVisibility, ItemKind, StandardLibrary, StdlibOwner, StdlibSymbolId,
        StdlibTypeKind, TypeRef,
    },
};

use super::{
    STATE_PROVIDER_INDEX_URI, StandardLibraryDocumentation, bundled, code, escape_markdown_symbol,
    intra_doc,
};

#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize)]
#[serde(rename_all = "camelCase")]
pub struct DocumentationIndexEntry {
    /// Stable virtual-document path, such as `/stdlib/types/Duration/index.md`.
    pub uri: String,
    pub title: String,
    pub kind: &'static str,
    pub summary: String,
    #[serde(skip)]
    pub(crate) raw_summary: &'static str,
    #[serde(skip)]
    pub(crate) search_text: String,
    pub signature: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize)]
#[serde(rename_all = "camelCase")]
pub struct DocumentationPage {
    pub uri: String,
    pub title: String,
    pub markdown: String,
}

struct DocumentationMemberGroup {
    title: &'static str,
    members: Vec<DocumentationMember>,
}

impl DocumentationMemberGroup {
    fn symbols(title: &'static str, members: impl IntoIterator<Item = StdlibSymbolId>) -> Self {
        Self {
            title,
            members: members
                .into_iter()
                .map(DocumentationMember::Symbol)
                .collect(),
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum DocumentationMember {
    Symbol(StdlibSymbolId),
    CapabilitySymbol {
        symbol: StdlibSymbolId,
        capability: crate::stdlib::StdlibCapabilityId,
    },
    CoreType(CoreTypeId),
}

/// Compiler-owned documentation reference consumed by editor integrations.
///
/// The reference deliberately returns Markdown rather than VS Code-specific
/// HTML. Editors can render it with their native Markdown UI, expose it as
/// plain read-only text, or transform the same pages for another frontend.
#[derive(Debug, Clone)]
pub struct DocumentationReference {
    library: StandardLibrary,
    semantic_examples: bool,
}

type CachedPage = Arc<OnceLock<Option<DocumentationPage>>>;

impl Default for DocumentationReference {
    fn default() -> Self {
        Self {
            library: StandardLibrary::default(),
            semantic_examples: true,
        }
    }
}

/// Documentation examples are semantically checked while their pages are
/// rendered. The catalogs are immutable for the lifetime of the compiler, so
/// every `DocumentationReference` describes the same canonical page for a URI.
/// Sharing one lazily initialized cell per URI prevents full-reference tests,
/// LSP requests, and parallel clients from compiling the same examples again.
/// Distinct pages retain independent cells and can still render concurrently.
static PAGE_ROUTES: OnceLock<HashSet<String>> = OnceLock::new();
static PAGE_CACHE: OnceLock<Mutex<HashMap<(bool, String), CachedPage>>> = OnceLock::new();

impl DocumentationReference {
    pub(super) fn with_lexical_examples(&self) -> Self {
        Self {
            library: self.library.clone(),
            semantic_examples: false,
        }
    }
    /// Validates catalogs and every rendered page in the documentation graph.
    ///
    /// This checks the same graph consumed by native tools and editor clients,
    /// including canonical page identities, local links, and heading anchors.
    pub fn validate(&self) -> Vec<String> {
        super::validation::validate(self, &self.library)
    }

    /// Returns the stable virtual page for a language-catalog item.
    pub fn language_item_uri(&self, item: LanguageItemId) -> String {
        language_item_uri(item)
    }

    /// Returns the stable virtual page for a standard-library symbol.
    pub fn standard_library_symbol_uri(&self, symbol: StdlibSymbolId) -> String {
        symbol_uri(symbol, &self.library)
    }

    pub fn index(&self) -> Vec<DocumentationIndexEntry> {
        let mut entries = vec![
            DocumentationIndexEntry {
                uri: "/language/index.md".to_owned(),
                title: "Language".to_owned(),
                kind: "guide",
                summary: "Syntax, declarations, lifecycle blocks, and contextual values."
                    .to_owned(),
                raw_summary: "Syntax, declarations, lifecycle blocks, and contextual values.",
                search_text: "syntax declarations lifecycle blocks contextual values".to_owned(),
                signature: None,
            },
            DocumentationIndexEntry {
                uri: "/migration/index.md".to_owned(),
                title: "Migration".to_owned(),
                kind: "guide",
                summary: "Guidance from ASL and familiar languages to canonical SplitScript."
                    .to_owned(),
                raw_summary: "Guidance from ASL and familiar languages to canonical SplitScript.",
                search_text: "ASL C# JavaScript Rust porting migration guidance".to_owned(),
                signature: None,
            },
            DocumentationIndexEntry {
                uri: STATE_PROVIDER_INDEX_URI.to_owned(),
                title: "State providers".to_owned(),
                kind: "index",
                summary: "Process and emulator environments available to state declarations."
                    .to_owned(),
                raw_summary: "Process and emulator environments available to state declarations.",
                search_text: "state providers process emulator environment".to_owned(),
                signature: None,
            },
        ];

        entries.extend(
            LanguageCatalog::new()
                .items()
                .filter(|item| !matches!(item.kind, LanguageItemKind::BuiltinType(_)))
                .map(|item| DocumentationIndexEntry {
                    uri: language_item_uri(item.id),
                    title: item.name.to_owned(),
                    kind: language_item_kind_label(item.kind),
                    summary: compact_prose(item.documentation.summary),
                    raw_summary: item.documentation.summary,
                    search_text: format!(
                        "{} {} {}",
                        item.documentation.summary, item.documentation.details, item.form
                    ),
                    signature: Some(item.form.to_owned()),
                }),
        );

        let migration = MigrationCatalog::default();
        entries.extend(
            migration
                .concepts()
                .iter()
                .map(|concept| DocumentationIndexEntry {
                    uri: migration_concept_uri(concept.id),
                    title: concept.name.to_owned(),
                    kind: "migration concept",
                    summary: compact_prose(concept.summary),
                    raw_summary: concept.summary,
                    search_text: migration_search_text(concept, &migration),
                    signature: Some(concept.id.as_str().to_owned()),
                }),
        );
        entries.extend(bundled::index());

        entries.extend(
            self.library
                .namespaces()
                .iter()
                .map(|namespace| DocumentationIndexEntry {
                    uri: symbol_uri(StdlibSymbolId::Namespace(namespace.id), &self.library),
                    title: namespace.path.join("."),
                    kind: "namespace",
                    summary: compact_prose(namespace.documentation.summary),
                    raw_summary: namespace.documentation.summary,
                    search_text: format!(
                        "{} {}",
                        namespace.documentation.summary, namespace.documentation.details
                    ),
                    signature: None,
                }),
        );
        entries.extend(self.library.core_types().iter().filter_map(|ty| {
            let language = LanguageCatalog::new().builtin_type(ty.id)?;
            Some(DocumentationIndexEntry {
                uri: core_type_uri(ty.id, &self.library),
                title: ty.name.to_owned(),
                kind: "built-in type",
                summary: compact_prose(language.documentation.summary),
                raw_summary: language.documentation.summary,
                search_text: format!(
                    "{} {}",
                    language.documentation.summary, language.documentation.details
                ),
                signature: Some(ty.name.to_owned()),
            })
        }));
        entries.extend(self.library.capabilities().iter().map(|capability| {
            DocumentationIndexEntry {
                uri: symbol_uri(StdlibSymbolId::Capability(capability.id), &self.library),
                title: capability.name.to_owned(),
                kind: "capability",
                summary: compact_prose(capability.documentation.summary),
                raw_summary: capability.documentation.summary,
                search_text: format!(
                    "{} {}",
                    capability.documentation.summary, capability.documentation.details
                ),
                signature: Some(format!("capability {}", capability.name)),
            }
        }));
        entries.extend(self.library.public_type_constructors().map(|constructor| {
            let title = self.library.render_type_constructor(constructor.id);
            DocumentationIndexEntry {
                uri: symbol_uri(
                    StdlibSymbolId::TypeConstructor(constructor.id),
                    &self.library,
                ),
                title,
                kind: "type constructor",
                summary: compact_prose(constructor.documentation.summary),
                raw_summary: constructor.documentation.summary,
                search_text: format!(
                    "{} {}",
                    constructor.documentation.summary, constructor.documentation.details
                ),
                signature: Some(render_constrained_type_constructor(
                    constructor,
                    &self.library,
                )),
            }
        }));
        entries.extend(self.library.types().map(|ty| DocumentationIndexEntry {
            uri: symbol_uri(StdlibSymbolId::Type(ty.id), &self.library),
            title: ty.name.to_owned(),
            kind: match ty.kind {
                StdlibTypeKind::Intrinsic => "type",
                StdlibTypeKind::Struct => "struct",
                StdlibTypeKind::Enum => "enum",
            },
            summary: compact_prose(ty.documentation.summary),
            raw_summary: ty.documentation.summary,
            search_text: format!("{} {}", ty.documentation.summary, ty.documentation.details),
            signature: Some(render_type_declaration(ty)),
        }));
        entries.extend(
            self.library
                .fields()
                .iter()
                .filter(|field| field.visibility == FieldVisibility::Public)
                .map(|field| {
                    let owner = self.library.render_field_owner(field.owner);
                    DocumentationIndexEntry {
                        uri: symbol_uri(StdlibSymbolId::Field(field.id), &self.library),
                        title: format!("{owner}.{}", field.name),
                        kind: "field",
                        summary: compact_prose(field.documentation.summary),
                        raw_summary: field.documentation.summary,
                        search_text: format!(
                            "{} {}",
                            field.documentation.summary, field.documentation.details
                        ),
                        signature: Some(format!(
                            "{}.{}: {}",
                            owner,
                            field.name,
                            self.library.render_type(field.ty)
                        )),
                    }
                }),
        );
        entries.extend(self.library.public_variants().map(|variant| {
            let owner = self.library.type_decl(variant.owner);
            DocumentationIndexEntry {
                uri: symbol_uri(StdlibSymbolId::Variant(variant.id), &self.library),
                title: format!("{}.{}", owner.name, variant.name),
                kind: "enum variant",
                summary: compact_prose(variant.documentation.summary),
                raw_summary: variant.documentation.summary,
                search_text: format!(
                    "{} {}",
                    variant.documentation.summary, variant.documentation.details
                ),
                signature: Some(format!("{}.{}", owner.name, variant.name)),
            }
        }));
        entries.extend(self.library.state_providers().iter().map(|provider| {
            let contexts = provider
                .contexts
                .iter()
                .flat_map(|context| {
                    [
                        context.name,
                        context.documentation.summary,
                        context.documentation.details,
                    ]
                })
                .collect::<Vec<_>>()
                .join(" ");
            DocumentationIndexEntry {
                uri: symbol_uri(StdlibSymbolId::StateProvider(provider.id), &self.library),
                title: provider.name.to_owned(),
                kind: "state provider",
                summary: compact_prose(provider.documentation.summary),
                raw_summary: provider.documentation.summary,
                search_text: format!(
                    "{} {} {contexts}",
                    provider.documentation.summary, provider.documentation.details,
                ),
                signature: Some(format!("state {}", provider.name)),
            }
        }));
        entries.extend(self.library.items().map(|item| DocumentationIndexEntry {
            uri: symbol_uri(StdlibSymbolId::Item(item.id), &self.library),
            title: render_item_name(item, &self.library),
            kind: if is_operator(item) {
                "operator"
            } else {
                match item.kind {
                    ItemKind::Function => "function",
                    ItemKind::Method { .. } => "method",
                    ItemKind::Constant => "constant",
                }
            },
            summary: compact_prose(item.documentation.summary),
            raw_summary: item.documentation.summary,
            search_text: format!(
                "{} {}",
                item.documentation.summary, item.documentation.details
            ),
            signature: Some(self.library.render_signature(item.id)),
        }));

        entries.sort_by(|left, right| {
            left.title
                .to_ascii_lowercase()
                .cmp(&right.title.to_ascii_lowercase())
                .then_with(|| left.kind.cmp(right.kind))
        });
        entries
    }

    /// Searches the complete compiler-owned documentation index.
    ///
    /// Ranking considers canonical titles and signatures first, then migration
    /// identities, foreign spellings, summaries, details, and guide prose. The
    /// returned entries remain the same stable identities consumed by editor
    /// navigation and [`Self::page`].
    pub fn search(&self, query: &str) -> Vec<DocumentationIndexEntry> {
        let query = SearchText::new(query);
        if query.original.is_empty() {
            return self.index();
        }

        let exact_aliases = self.exact_migration_aliases(&query.original);
        let mut matches = self
            .index()
            .into_iter()
            .filter_map(|entry| {
                let alias = exact_aliases.iter().any(|uri| uri == &entry.uri);
                search_score(&entry, &query, alias).map(|score| (score, entry))
            })
            .collect::<Vec<_>>();
        matches.sort_by(|(left_score, left), (right_score, right)| {
            right_score
                .cmp(left_score)
                .then_with(|| {
                    left.title
                        .to_ascii_lowercase()
                        .cmp(&right.title.to_ascii_lowercase())
                })
                .then_with(|| left.kind.cmp(right.kind))
        });
        matches.into_iter().map(|(_, entry)| entry).collect()
    }

    /// Resolves a user-facing documentation topic to its canonical page.
    ///
    /// Native tools accept the stable migration identity printed by a
    /// diagnostic, an exact reference title such as `Process.read`, a virtual
    /// document path, or one unambiguous catalogued foreign spelling. Broader
    /// queries remain search results rather than silently selected pages.
    pub fn topic(&self, topic: &str) -> Option<DocumentationPage> {
        let topic = topic.trim();
        if topic.is_empty() {
            return self.page("/index.md");
        }

        let uri = if topic.starts_with('/') {
            topic.to_owned()
        } else {
            format!("/{topic}")
        };
        if let Some(page) = self.page(&uri) {
            return Some(page);
        }
        let migration_uri = migration_topic_uri(topic);
        if let Some(page) = self.page(&migration_uri) {
            return Some(page);
        }

        let exact = self
            .index()
            .into_iter()
            .filter(|entry| {
                entry.title.eq_ignore_ascii_case(topic)
                    || entry
                        .signature
                        .as_deref()
                        .is_some_and(|signature| signature.eq_ignore_ascii_case(topic))
            })
            .collect::<Vec<_>>();
        if let [entry] = exact.as_slice() {
            return self.page(&entry.uri);
        }

        let generic_aliases = self.exact_generic_symbol_aliases(topic);
        if let [uri] = generic_aliases.as_slice() {
            return self.page(uri);
        }

        let aliases = self.exact_migration_aliases(topic);
        if let [uri] = aliases.as_slice() {
            return self.page(uri);
        }
        None
    }

    /// Resolves the concise spelling people can write without inventing names
    /// for a generic owner's type parameters. Display labels retain the full
    /// `MapEntry<K, V>.key` form, while exact lookup also accepts the
    /// catalog-owned `MapEntry.key` path. Construct aliases from semantic
    /// ownership rather than stripping angle brackets from rendered Markdown;
    /// this keeps operators and future type-form syntax unambiguous.
    fn exact_generic_symbol_aliases(&self, query: &str) -> Vec<String> {
        let query = query.trim();
        let member_query = query.rsplit_once('.');
        let mut uris = self
            .library
            .public_type_constructors()
            .filter(|constructor| constructor.name.eq_ignore_ascii_case(query))
            .map(|constructor| {
                symbol_uri(
                    StdlibSymbolId::TypeConstructor(constructor.id),
                    &self.library,
                )
            })
            .chain(self.library.fields().iter().filter_map(|field| {
                let StdlibOwner::TypeConstructor(owner) = field.owner else {
                    return None;
                };
                let (owner_query, field_query) = member_query?;
                (self
                    .library
                    .type_constructor(owner)
                    .name
                    .eq_ignore_ascii_case(owner_query)
                    && field.name.eq_ignore_ascii_case(field_query))
                .then(|| symbol_uri(StdlibSymbolId::Field(field.id), &self.library))
            }))
            .chain(self.library.items().filter_map(|item| {
                let StdlibOwner::TypeConstructor(owner) = item.owner else {
                    return None;
                };
                let (owner_query, item_query) = member_query?;
                (self
                    .library
                    .type_constructor(owner)
                    .name
                    .eq_ignore_ascii_case(owner_query)
                    && item.name.eq_ignore_ascii_case(item_query))
                .then(|| symbol_uri(StdlibSymbolId::Item(item.id), &self.library))
            }))
            .collect::<Vec<_>>();
        uris.sort();
        uris.dedup();
        uris
    }

    fn exact_migration_aliases(&self, query: &str) -> Vec<String> {
        let query = query.trim();
        let migration = MigrationCatalog::default();
        let mut uris = migration
            .concepts()
            .iter()
            .filter(|concept| {
                concept
                    .spellings
                    .iter()
                    .any(|spelling| spelling.spelling.eq_ignore_ascii_case(query))
            })
            .map(|concept| migration_concept_uri(concept.id))
            .collect::<Vec<_>>();
        uris.extend(
            migration
                .query_aliases()
                .iter()
                .filter(|aliases| {
                    aliases
                        .queries
                        .iter()
                        .any(|alias| alias.eq_ignore_ascii_case(query))
                })
                .map(|aliases| migration_concept_uri(aliases.concept)),
        );
        uris.sort();
        uris.dedup();
        uris
    }

    pub fn page(&self, uri: &str) -> Option<DocumentationPage> {
        if !self.page_routes().contains(uri) {
            return None;
        }
        let page = {
            let mut cache = PAGE_CACHE
                .get_or_init(|| Mutex::new(HashMap::new()))
                .lock()
                .expect("documentation page cache should not be poisoned");
            cache
                .entry((self.semantic_examples, uri.to_owned()))
                .or_insert_with(|| Arc::new(OnceLock::new()))
                .clone()
        };
        page.get_or_init(|| self.render_page(uri)).clone()
    }

    fn page_routes(&self) -> &'static HashSet<String> {
        PAGE_ROUTES.get_or_init(|| {
            let mut routes = self
                .index()
                .into_iter()
                .map(|entry| entry.uri)
                .collect::<HashSet<_>>();
            routes.insert("/index.md".to_owned());
            routes
        })
    }

    fn render_page(&self, uri: &str) -> Option<DocumentationPage> {
        if uri == "/index.md" {
            return Some(self.index_page());
        }

        if uri == "/language/index.md" {
            return Some(self.language_index_page());
        }

        if uri == "/migration/index.md" {
            return Some(self.migration_index_page());
        }

        if uri == STATE_PROVIDER_INDEX_URI {
            return Some(self.state_provider_index_page());
        }

        if let Some(mut page) = bundled::page(uri, &self.library, self.semantic_examples) {
            page.markdown = intra_doc::render_links(&page.markdown, uri, &self.library);
            return Some(page);
        }

        if let Some(item) = LanguageCatalog::new().items().find(|item| {
            !matches!(item.kind, LanguageItemKind::BuiltinType(_))
                && language_item_uri(item.id) == uri
        }) {
            return Some(self.language_item_page(item));
        }

        let migration = MigrationCatalog::default();
        if let Some(concept) = migration
            .concepts()
            .iter()
            .find(|concept| migration_concept_uri(concept.id) == uri)
        {
            return Some(self.migration_concept_page(concept, &migration));
        }

        if let Some(ty) = self
            .library
            .core_types()
            .iter()
            .find(|ty| core_type_uri(ty.id, &self.library) == uri)
            && let Some(language) = LanguageCatalog::new().builtin_type(ty.id)
        {
            return Some(self.core_type_page(ty.id, &language.documentation));
        }

        self.library
            .namespaces()
            .iter()
            .find(|value| symbol_uri(StdlibSymbolId::Namespace(value.id), &self.library) == uri)
            .map(|value| {
                self.declaration_page(
                    StdlibSymbolId::Namespace(value.id),
                    value.path.join("."),
                    "namespace",
                    None,
                    &value.documentation,
                    vec![
                        DocumentationMemberGroup::symbols(
                            "Namespaces",
                            self.child_namespaces(value.id),
                        ),
                        DocumentationMemberGroup::symbols(
                            "Functions",
                            self.owned_non_operators(StdlibOwner::Namespace(value.id)),
                        ),
                    ],
                )
            })
            .or_else(|| {
                self.library
                    .capabilities()
                    .iter()
                    .find(|value| {
                        symbol_uri(StdlibSymbolId::Capability(value.id), &self.library) == uri
                    })
                    .map(|value| {
                        self.declaration_page(
                            StdlibSymbolId::Capability(value.id),
                            value.name.to_owned(),
                            "capability",
                            Some(format!("capability {}", value.name)),
                            &value.documentation,
                            vec![
                                DocumentationMemberGroup::symbols(
                                    "Requires",
                                    value
                                        .super_capabilities
                                        .iter()
                                        .map(|id| StdlibSymbolId::Capability(*id)),
                                ),
                                DocumentationMemberGroup {
                                    title: "Implemented by",
                                    members: self.capability_types(value.id),
                                },
                                DocumentationMemberGroup::symbols(
                                    "Operators",
                                    self.owned_operators(StdlibOwner::Capability(value.id)),
                                ),
                                DocumentationMemberGroup::symbols(
                                    "Methods",
                                    self.owned_non_operators(StdlibOwner::Capability(value.id)),
                                ),
                            ],
                        )
                    })
            })
            .or_else(|| {
                self.library
                    .public_type_constructors()
                    .find(|value| {
                        symbol_uri(StdlibSymbolId::TypeConstructor(value.id), &self.library) == uri
                    })
                    .map(|value| {
                        self.declaration_page(
                            StdlibSymbolId::TypeConstructor(value.id),
                            self.library.render_type_constructor(value.id),
                            "type constructor",
                            Some(render_constrained_type_constructor(value, &self.library)),
                            &value.documentation,
                            vec![
                                DocumentationMemberGroup::symbols(
                                    "Operators",
                                    self.owned_operators(StdlibOwner::TypeConstructor(value.id)),
                                ),
                                DocumentationMemberGroup::symbols(
                                    "Methods",
                                    self.owned_non_operators(StdlibOwner::TypeConstructor(
                                        value.id,
                                    )),
                                ),
                            ],
                        )
                    })
            })
            .or_else(|| {
                self.library
                    .types()
                    .find(|value| symbol_uri(StdlibSymbolId::Type(value.id), &self.library) == uri)
                    .map(|value| {
                        self.declaration_page(
                            StdlibSymbolId::Type(value.id),
                            value.name.to_owned(),
                            match value.kind {
                                StdlibTypeKind::Intrinsic => "type",
                                StdlibTypeKind::Struct => "struct",
                                StdlibTypeKind::Enum => "enum",
                            },
                            Some(render_type_declaration(value)),
                            &value.documentation,
                            vec![
                                DocumentationMemberGroup::symbols(
                                    "Capabilities",
                                    value
                                        .capabilities
                                        .iter()
                                        .map(|id| StdlibSymbolId::Capability(*id)),
                                ),
                                DocumentationMemberGroup::symbols(
                                    "Fields",
                                    self.library
                                        .public_fields(value.id)
                                        .map(|field| StdlibSymbolId::Field(field.id)),
                                ),
                                DocumentationMemberGroup::symbols(
                                    "Variants",
                                    self.library
                                        .variants_of(value.id)
                                        .map(|variant| StdlibSymbolId::Variant(variant.id)),
                                ),
                                DocumentationMemberGroup {
                                    title: "Operators",
                                    members: self.available_members(
                                        StdlibOwner::Type(value.id),
                                        value.capabilities,
                                        true,
                                    ),
                                },
                                DocumentationMemberGroup {
                                    title: "Methods",
                                    members: self.available_members(
                                        StdlibOwner::Type(value.id),
                                        value.capabilities,
                                        false,
                                    ),
                                },
                            ],
                        )
                    })
            })
            .or_else(|| {
                self.library
                    .fields()
                    .iter()
                    .filter(|value| value.visibility == FieldVisibility::Public)
                    .find(|value| symbol_uri(StdlibSymbolId::Field(value.id), &self.library) == uri)
                    .map(|value| {
                        let owner = self.library.render_field_owner(value.owner);
                        self.declaration_page(
                            StdlibSymbolId::Field(value.id),
                            format!("{owner}.{}", value.name),
                            "field",
                            Some(format!(
                                "{}.{}: {}",
                                owner,
                                value.name,
                                self.library.render_type(value.ty)
                            )),
                            &value.documentation,
                            Vec::new(),
                        )
                    })
            })
            .or_else(|| {
                self.library
                    .public_variants()
                    .find(|value| {
                        symbol_uri(StdlibSymbolId::Variant(value.id), &self.library) == uri
                    })
                    .map(|value| {
                        let owner = self.library.type_decl(value.owner);
                        self.declaration_page(
                            StdlibSymbolId::Variant(value.id),
                            format!("{}.{}", owner.name, value.name),
                            "enum variant",
                            Some(format!("{}.{}", owner.name, value.name)),
                            &value.documentation,
                            Vec::new(),
                        )
                    })
            })
            .or_else(|| {
                self.library
                    .state_providers()
                    .iter()
                    .find(|value| {
                        symbol_uri(StdlibSymbolId::StateProvider(value.id), &self.library) == uri
                    })
                    .map(|value| {
                        let mut page = self.declaration_page(
                            StdlibSymbolId::StateProvider(value.id),
                            value.name.to_owned(),
                            "state provider",
                            Some(format!("state {}", value.name)),
                            &value.documentation,
                            vec![DocumentationMemberGroup::symbols(
                                "Value type",
                                [StdlibSymbolId::Type(value.process_type)],
                            )],
                        );
                        self.append_state_provider_ranges(&mut page.markdown, value);
                        if value.validation.is_some() {
                            page.markdown.push_str("\n\n## Mapping lifecycle\n\n");
                            page.markdown.push_str(&intra_doc::render_links(
                                crate::stdlib::STATE_PROVIDER_MAPPING_LIFECYCLE_NOTE,
                                &page.uri,
                                &self.library,
                            ));
                        }
                        self.append_state_provider_contexts(&mut page.markdown, &page.uri, value);
                        page
                    })
            })
            .or_else(|| {
                self.library
                    .items()
                    .find(|value| symbol_uri(StdlibSymbolId::Item(value.id), &self.library) == uri)
                    .map(|value| {
                        let title = render_item_name(value, &self.library);
                        let documentation = StandardLibraryDocumentation::generate_with_library(
                            &self.library,
                            value.id,
                            &[],
                        );
                        let details = intra_doc::render_links(
                            &documentation.reference_details_markdown(),
                            uri,
                            &self.library,
                        );
                        let mut markdown = format!(
                            "{}\n\n# {}\n\n_{}_\n\n{}\n\n{}",
                            self.symbol_breadcrumb(StdlibSymbolId::Item(value.id), uri),
                            title,
                            if is_operator(value) {
                                "Operator"
                            } else {
                                match value.kind {
                                    ItemKind::Function => "Function",
                                    ItemKind::Method { .. } => "Method",
                                    ItemKind::Constant => "Constant",
                                }
                            },
                            self.render_signature(
                                &documentation.signature,
                                uri,
                                Some(StdlibSymbolId::Item(value.id)),
                            ),
                            details,
                        );
                        append_examples(
                            &mut markdown,
                            uri,
                            &self.library,
                            documentation.examples,
                            self.semantic_examples,
                        );
                        if let Some(provider) = self
                            .library
                            .state_providers()
                            .iter()
                            .find(|provider| provider.direct_read == value.id)
                        {
                            self.append_state_provider_ranges(&mut markdown, provider);
                        }
                        append_related(
                            &mut markdown,
                            uri,
                            &self.library,
                            value.documentation.related,
                        );
                        DocumentationPage {
                            uri: uri.to_owned(),
                            title,
                            markdown,
                        }
                    })
            })
    }

    fn owned_operators(&self, owner: StdlibOwner) -> Vec<StdlibSymbolId> {
        self.library
            .items()
            .filter(|item| item.owner == owner && is_operator(item))
            .map(|item| StdlibSymbolId::Item(item.id))
            .collect()
    }

    fn owned_non_operators(&self, owner: StdlibOwner) -> Vec<StdlibSymbolId> {
        self.library
            .items()
            .filter(|item| item.owner == owner && !is_operator(item))
            .map(|item| StdlibSymbolId::Item(item.id))
            .collect()
    }

    fn available_members(
        &self,
        direct_owner: StdlibOwner,
        capabilities: &[crate::stdlib::StdlibCapabilityId],
        operators: bool,
    ) -> Vec<DocumentationMember> {
        self.library
            .items()
            .filter(|item| item.owner == direct_owner && is_operator(item) == operators)
            .map(|item| DocumentationMember::Symbol(StdlibSymbolId::Item(item.id)))
            .chain(self.library.items().filter_map(|item| {
                let StdlibOwner::Capability(capability) = item.owner else {
                    return None;
                };
                (is_operator(item) == operators
                    && self.library.capabilities_satisfy(capabilities, capability))
                .then_some(DocumentationMember::CapabilitySymbol {
                    symbol: StdlibSymbolId::Item(item.id),
                    capability,
                })
            }))
            .collect()
    }

    fn capability_types(
        &self,
        capability: crate::stdlib::StdlibCapabilityId,
    ) -> Vec<DocumentationMember> {
        self.library
            .core_types()
            .iter()
            .filter(|ty| {
                self.library
                    .capabilities_satisfy(ty.capabilities, capability)
            })
            .map(|ty| DocumentationMember::CoreType(ty.id))
            .chain(self.library.types().filter_map(|ty| {
                self.library
                    .capabilities_satisfy(ty.capabilities, capability)
                    .then_some(DocumentationMember::Symbol(StdlibSymbolId::Type(ty.id)))
            }))
            .collect()
    }

    fn child_namespaces(&self, parent: crate::stdlib::StdlibNamespaceId) -> Vec<StdlibSymbolId> {
        let parent = self.library.namespace(parent);
        self.library
            .namespaces()
            .iter()
            .filter(|namespace| {
                namespace.path.len() == parent.path.len() + 1
                    && namespace.path.starts_with(parent.path)
            })
            .map(|namespace| StdlibSymbolId::Namespace(namespace.id))
            .collect()
    }

    fn core_type_page<Id>(
        &self,
        ty: CoreTypeId,
        documentation: &Documentation<Id>,
    ) -> DocumentationPage {
        let ty = self.library.core_type(ty);
        let uri = core_type_uri(ty.id, &self.library);
        let mut markdown = format!(
            "{}\n\n# {}\n\n_Built-in type_\n\n{}",
            breadcrumb(&uri, Vec::new(), ty.name),
            ty.name,
            self.render_signature(ty.name, &uri, None),
        );
        append_documentation(
            &mut markdown,
            &uri,
            &self.library,
            documentation,
            self.semantic_examples,
        );
        self.append_member_groups(
            &mut markdown,
            &uri,
            vec![
                DocumentationMemberGroup::symbols(
                    "Capabilities",
                    ty.capabilities
                        .iter()
                        .map(|id| StdlibSymbolId::Capability(*id)),
                ),
                DocumentationMemberGroup {
                    title: "Operators",
                    members: self.available_members(
                        StdlibOwner::Core(ty.id),
                        ty.capabilities,
                        true,
                    ),
                },
                DocumentationMemberGroup {
                    title: "Methods",
                    members: self.available_members(
                        StdlibOwner::Core(ty.id),
                        ty.capabilities,
                        false,
                    ),
                },
            ],
        );
        DocumentationPage {
            uri,
            title: ty.name.to_owned(),
            markdown,
        }
    }

    fn declaration_page(
        &self,
        symbol: StdlibSymbolId,
        title: String,
        kind: &str,
        signature: Option<String>,
        documentation: &Documentation<StdlibSymbolId>,
        member_groups: Vec<DocumentationMemberGroup>,
    ) -> DocumentationPage {
        let uri = symbol_uri(symbol, &self.library);
        let mut markdown = format!(
            "{}\n\n# {}\n\n_{kind}_",
            self.symbol_breadcrumb(symbol, &uri),
            escape_markdown_symbol(&title),
        );
        if let Some(signature) = signature {
            markdown.push_str("\n\n");
            markdown.push_str(&self.render_signature(&signature, &uri, Some(symbol)));
        }
        append_documentation(
            &mut markdown,
            &uri,
            &self.library,
            documentation,
            self.semantic_examples,
        );
        self.append_member_groups(&mut markdown, &uri, member_groups);
        append_related(&mut markdown, &uri, &self.library, documentation.related);
        DocumentationPage {
            uri,
            title,
            markdown,
        }
    }

    fn append_member_groups(
        &self,
        markdown: &mut String,
        uri: &str,
        member_groups: Vec<DocumentationMemberGroup>,
    ) {
        for mut group in member_groups {
            group
                .members
                .sort_by_key(|member| member_label(*member, &self.library));
            group.members.dedup();
            if group.members.is_empty() {
                continue;
            }
            markdown.push_str(&format!("\n\n## {}\n", group.title));
            append_member_table(markdown, uri, &group.members, &self.library);
        }
    }

    fn append_state_provider_contexts(
        &self,
        markdown: &mut String,
        uri: &str,
        provider: &crate::stdlib::StdlibStateProvider,
    ) {
        if provider.contexts.is_empty() {
            return;
        }

        markdown.push_str("\n\n## Contextual values\n");
        append_reference_table_header(markdown, &["Value", "Description"]);
        for context in provider.contexts {
            let ty_uri = symbol_uri(StdlibSymbolId::Type(context.ty), &self.library);
            let ty = self.library.render_type(TypeRef::Standard(context.ty));
            let value = format!(
                "`{}`: [`{ty}`]({})",
                context.name,
                relative_document_link(uri, &ty_uri),
            );
            markdown.push_str(&format!(
                "\n| {value} | {} |",
                table_prose(context.documentation.summary, uri, &self.library),
            ));
        }
    }

    fn append_state_provider_ranges(
        &self,
        markdown: &mut String,
        provider: &crate::stdlib::StdlibStateProvider,
    ) {
        if provider.readable_ranges.is_empty() {
            return;
        }
        markdown.push_str("\n\n## Readable guest memory\n");
        for range in provider.readable_ranges {
            markdown.push_str(&format!(
                "\n- `0x{:08x}..<0x{:08x}` (exclusive upper bound)",
                range.start, range.end
            ));
        }
        markdown.push_str(
            "\n\nA read must fit entirely inside one range. Literal addresses are checked during compilation; computed addresses remain fallible and are checked at runtime.",
        );
    }

    fn language_index_page(&self) -> DocumentationPage {
        let uri = "/language/index.md";
        let mut markdown = format!(
            "{}\n\n# Language\n\nCompiler-owned syntax, declarations, lifecycle blocks, and contextual values.\n",
            reference_breadcrumb(uri, Vec::new(), "Language")
        );
        let groups = [
            ("Declarations", "declaration"),
            ("Lifecycle blocks", "lifecycle block"),
            ("Keywords", "keyword"),
            ("Syntax", "syntax"),
            ("Contextual values", "contextual value"),
        ];
        let language = LanguageCatalog::new();
        for (title, kind) in groups {
            let mut items = language
                .items()
                .filter(|item| language_item_kind_label(item.kind) == kind)
                .collect::<Vec<_>>();
            items.sort_by_key(|item| item.name.to_ascii_lowercase());
            if items.is_empty() {
                continue;
            }
            markdown.push_str(&format!("\n## {title}\n"));
            append_reference_table_header(&mut markdown, &["Symbol", "Description"]);
            for item in items {
                markdown.push_str(&format!(
                    "\n| [{}]({}) | {} |",
                    escape_markdown_table_cell(item.name),
                    relative_document_link(uri, &language_item_uri(item.id)),
                    table_prose(item.documentation.summary, uri, &self.library),
                ));
            }
        }
        DocumentationPage {
            uri: uri.to_owned(),
            title: "Language".to_owned(),
            markdown,
        }
    }

    fn language_item_page(&self, item: &LanguageItem) -> DocumentationPage {
        let uri = language_item_uri(item.id);
        let summary = intra_doc::render_links(item.documentation.summary, &uri, &self.library);
        let details = intra_doc::render_links(item.documentation.details, &uri, &self.library);
        let prose = super::prose_markdown(&summary, &details);
        let mut markdown = format!(
            "{}\n\n# {}\n\n_{}_\n\n{}\n\n{}",
            reference_breadcrumb(
                &uri,
                vec![("Language".to_owned(), "/language/index.md".to_owned())],
                item.name,
            ),
            item.name,
            language_item_kind_label(item.kind),
            self.render_signature(item.form, &uri, None),
            prose,
        );
        append_examples(
            &mut markdown,
            &uri,
            &self.library,
            item.documentation.examples,
            self.semantic_examples,
        );
        if !item.documentation.related.is_empty() {
            markdown.push_str("\n\n## Related\n");
            append_reference_table_header(&mut markdown, &["Symbol", "Description"]);
            for related in item.documentation.related {
                let related = LanguageCatalog::new().item(*related);
                markdown.push_str(&format!(
                    "\n| [{}]({}) | {} |",
                    escape_markdown_table_cell(related.name),
                    relative_document_link(&uri, &language_item_uri(related.id)),
                    table_prose(related.documentation.summary, &uri, &self.library),
                ));
            }
        }
        DocumentationPage {
            uri,
            title: item.name.to_owned(),
            markdown,
        }
    }

    fn render_signature(
        &self,
        source: &str,
        current_uri: &str,
        primary: Option<StdlibSymbolId>,
    ) -> String {
        code::signature(source, current_uri, primary, &self.library)
    }

    fn migration_index_page(&self) -> DocumentationPage {
        let uri = "/migration/index.md";
        let migration = MigrationCatalog::default();
        let mut markdown = format!(
            "{}\n\n# Migration\n\nChoose the source language first, then the task or source concept. Stable catalog identities remain searchable, but the navigation follows the way a porter approaches the work.\n",
            reference_breadcrumb(uri, Vec::new(), "Migration")
        );
        let mut current_source = None;
        for group in MigrationNavigationGroup::ALL {
            let mut concepts = migration
                .concepts()
                .iter()
                .filter(|concept| migration_navigation_group(concept) == *group)
                .collect::<Vec<_>>();
            if concepts.is_empty() {
                continue;
            }
            concepts.sort_by_key(|concept| concept.name.to_ascii_lowercase());
            if current_source != Some(group.source()) {
                current_source = Some(group.source());
                markdown.push_str(&format!("\n\n## {}", group.source().name()));
                if group.source() == crate::migration::SourceLanguage::Asl {
                    markdown.push_str(&format!(
                        "\n\nFor lifecycle and semantic context, start with the [complete ASL porting guide]({}).",
                        relative_document_link(uri, "/guides/asl-porting.md"),
                    ));
                }
            }
            if let Some(task) = group.task() {
                markdown.push_str(&format!("\n\n### {task}"));
            }
            for concept in concepts {
                markdown.push_str(&format!(
                    "\n\n- [{}]({}): {} _{}_",
                    concept.name,
                    relative_document_link(uri, &migration_concept_uri(concept.id)),
                    intra_doc::render_links(concept.summary, uri, &self.library),
                    concept.support.label(),
                ));
            }
        }
        DocumentationPage {
            uri: uri.to_owned(),
            title: "Migration".to_owned(),
            markdown,
        }
    }

    fn migration_concept_page(
        &self,
        concept: &MigrationConcept,
        migration: &MigrationCatalog,
    ) -> DocumentationPage {
        let uri = migration_concept_uri(concept.id);
        let sources = concept
            .sources
            .iter()
            .map(|source| source.name())
            .collect::<Vec<_>>()
            .join(", ");
        let summary = intra_doc::render_links(concept.summary, &uri, &self.library);
        let mut markdown = format!(
            "{}\n\n# {}\n\n_Migration from {sources}_",
            reference_breadcrumb(
                &uri,
                vec![("Migration".to_owned(), "/migration/index.md".to_owned())],
                concept.name,
            ),
            concept.name,
        );

        markdown.push_str("\n\n## Source pattern");
        if concept.spellings.is_empty() {
            markdown.push_str(&format!(
                "\n\nLook for the {} concept **{}** in the source. The semantic difference below is more important than a token-for-token rewrite.",
                sources, concept.name,
            ));
        } else {
            for spelling in concept.spellings {
                markdown.push_str(&format!(
                    "\n\n- {}: {}",
                    spelling.source.name(),
                    migration_source_code(spelling.spelling),
                ));
            }
        }

        markdown.push_str("\n\n## Canonical SplitScript");
        let replacements = concept
            .spellings
            .iter()
            .filter(|spelling| spelling.replacement != ForeignSpellingReplacement::Remove)
            .collect::<Vec<_>>();
        if !replacements.is_empty() {
            for spelling in replacements {
                markdown.push_str(&format!(
                    "\n\n- {} becomes {}.",
                    migration_source_code(spelling.spelling),
                    migration_source_code(spelling.replacement.text()),
                ));
            }
        }
        if concept.targets.is_empty() {
            markdown
                .push_str("\n\nThere is no currently supported canonical API for this behavior.");
        } else {
            markdown.push_str("\n\nUse the canonical language and API symbols linked below.");
        }

        markdown.push_str(&format!("\n\n## Semantic difference\n\n{summary}"));

        markdown.push_str("\n\n## Supported hosts\n\n");
        match concept.support {
            MigrationSupport::Direct | MigrationSupport::TypedPattern
                if concept
                    .targets
                    .iter()
                    .any(|target| matches!(target, MigrationTarget::StateProvider(_))) =>
            {
                markdown.push_str(
                    "The linked state-provider page is the canonical list of supported runtimes or emulators. The compiler rejects unsupported provider forms.",
                );
            }
            MigrationSupport::Direct | MigrationSupport::TypedPattern => markdown.push_str(
                "This uses the standard SplitScript runtime contract. Any provider-specific requirements are documented by the linked canonical API.",
            ),
            MigrationSupport::Planned => markdown.push_str(
                "No supported host currently exposes the complete behavior required by this source pattern.",
            ),
            MigrationSupport::SandboxNonGoal => markdown.push_str(
                "SplitScript intentionally does not expose this behavior through its sandboxed host contract.",
            ),
        }

        markdown.push_str("\n\n## Related reference");
        for target in concept.targets {
            let label = migration.target_display(*target);
            let rendered = migration_target_uri(*target, &self.library).map_or_else(
                || format!("`{label}`"),
                |target_uri| format!("[{}]({})", label, relative_document_link(&uri, &target_uri),),
            );
            markdown.push_str(&format!(
                "\n\n- {rendered} ({})",
                migration_target_kind(*target),
            ));
        }
        if let Some(anchor) = concept.cookbook_anchor {
            markdown.push_str(&format!(
                "\n\n- [Complete porting recipe]({}#{anchor})",
                relative_document_link(&uri, "/guides/asl-porting.md"),
            ));
        }
        if concept.targets.is_empty() && concept.cookbook_anchor.is_none() {
            markdown.push_str("\n\nNo additional reference page is available yet.");
        }
        if concept
            .spellings
            .iter()
            .any(|spelling| spelling.replacement == ForeignSpellingReplacement::Remove)
        {
            markdown.push_str("\n\nThe compiler also recognizes source-only syntax that should be removed rather than replaced.");
            for spelling in concept
                .spellings
                .iter()
                .filter(|spelling| spelling.replacement == ForeignSpellingReplacement::Remove)
            {
                markdown.push_str(&format!(
                    "\n\n- Remove {}.",
                    migration_source_code(spelling.spelling)
                ));
            }
        }
        DocumentationPage {
            uri,
            title: concept.name.to_owned(),
            markdown,
        }
    }

    fn index_page(&self) -> DocumentationPage {
        let entries = self.index();
        let mut markdown = String::from(
            "# SplitScript reference\n\n\
             This reference is generated from the same compiler-owned catalogs used by parsing, type checking, migration diagnostics, completion, and hover.\n",
        );
        let sections: [(&str, Vec<String>); 6] = [
            (
                "Guides",
                entries
                    .iter()
                    .filter(|entry| entry.kind == "guide")
                    .map(|entry| entry.uri.clone())
                    .collect(),
            ),
            (
                "Namespaces",
                self.library
                    .namespaces()
                    .iter()
                    .filter(|namespace| namespace.path.len() == 1)
                    .map(|namespace| {
                        symbol_uri(StdlibSymbolId::Namespace(namespace.id), &self.library)
                    })
                    .collect(),
            ),
            (
                "Types",
                self.library
                    .core_types()
                    .iter()
                    .map(|ty| core_type_uri(ty.id, &self.library))
                    .chain(
                        self.library
                            .types()
                            .map(|ty| symbol_uri(StdlibSymbolId::Type(ty.id), &self.library)),
                    )
                    .chain(self.library.public_type_constructors().map(|constructor| {
                        symbol_uri(
                            StdlibSymbolId::TypeConstructor(constructor.id),
                            &self.library,
                        )
                    }))
                    .collect(),
            ),
            (
                "Capabilities",
                self.library
                    .capabilities()
                    .iter()
                    .map(|capability| {
                        symbol_uri(StdlibSymbolId::Capability(capability.id), &self.library)
                    })
                    .collect(),
            ),
            (
                "State providers",
                self.library
                    .state_providers()
                    .iter()
                    .map(|provider| {
                        symbol_uri(StdlibSymbolId::StateProvider(provider.id), &self.library)
                    })
                    .collect(),
            ),
            (
                "Functions",
                self.owned_non_operators(StdlibOwner::Root)
                    .into_iter()
                    .map(|item| symbol_uri(item, &self.library))
                    .collect(),
            ),
        ];
        for (title, uris) in sections {
            let mut section = uris
                .iter()
                .filter_map(|uri| entries.iter().find(|entry| entry.uri == *uri))
                .collect::<Vec<_>>();
            section.sort_by_key(|entry| entry.title.to_ascii_lowercase());
            if section.is_empty() {
                continue;
            }
            markdown.push_str(&format!("\n## {title}\n"));
            append_reference_table_header(&mut markdown, &["Symbol", "Description"]);
            for entry in section {
                markdown.push_str(&format!(
                    "\n| [{}]({}) | {} |",
                    escape_markdown_symbol(&entry.title),
                    relative_document_link("/index.md", &entry.uri),
                    table_prose(entry.raw_summary, "/index.md", &self.library)
                ));
            }
        }
        DocumentationPage {
            uri: "/index.md".to_owned(),
            title: "SplitScript reference".to_owned(),
            markdown,
        }
    }

    fn state_provider_index_page(&self) -> DocumentationPage {
        let entries = self.index();
        let mut providers = entries
            .iter()
            .filter(|entry| entry.kind == "state provider")
            .collect::<Vec<_>>();
        providers.sort_by_key(|entry| entry.title.to_ascii_lowercase());

        let mut markdown = String::from(
            "# State providers\n\n\
             A [`state`](../../language/state.md) declaration selects one of these compiler-supported process or emulator environments.\n",
        );
        append_reference_table_header(&mut markdown, &["Provider", "Description"]);
        for entry in providers {
            markdown.push_str(&format!(
                "\n| [{}]({}) | {} |",
                escape_markdown_symbol(&entry.title),
                relative_document_link(STATE_PROVIDER_INDEX_URI, &entry.uri),
                table_prose(entry.raw_summary, STATE_PROVIDER_INDEX_URI, &self.library)
            ));
        }
        DocumentationPage {
            uri: STATE_PROVIDER_INDEX_URI.to_owned(),
            title: "State providers".to_owned(),
            markdown,
        }
    }

    fn symbol_breadcrumb(&self, symbol: StdlibSymbolId, uri: &str) -> String {
        match symbol {
            StdlibSymbolId::Namespace(id) => {
                let namespace = self.library.namespace(id);
                let ancestors = namespace
                    .path
                    .iter()
                    .enumerate()
                    .take(namespace.path.len().saturating_sub(1))
                    .filter_map(|(index, name)| {
                        let path = &namespace.path[..=index];
                        self.library
                            .namespaces()
                            .iter()
                            .find(|candidate| candidate.path == path)
                            .map(|ancestor| {
                                (
                                    (*name).to_owned(),
                                    symbol_uri(
                                        StdlibSymbolId::Namespace(ancestor.id),
                                        &self.library,
                                    ),
                                )
                            })
                    })
                    .collect();
                breadcrumb(uri, ancestors, namespace.name)
            }
            StdlibSymbolId::Field(id) => {
                let field = self.library.field(id);
                let owner = self.library.render_field_owner(field.owner);
                breadcrumb(
                    uri,
                    vec![(
                        owner,
                        symbol_uri(field_owner_symbol(field.owner), &self.library),
                    )],
                    field.name,
                )
            }
            StdlibSymbolId::Variant(id) => {
                let variant = self.library.variant(id);
                let owner = self.library.type_decl(variant.owner);
                breadcrumb(
                    uri,
                    vec![(
                        owner.name.to_owned(),
                        symbol_uri(StdlibSymbolId::Type(owner.id), &self.library),
                    )],
                    variant.name,
                )
            }
            StdlibSymbolId::Item(id) => {
                let item = self.library.item(id);
                breadcrumb(uri, self.owner_breadcrumb(item.owner), item.name)
            }
            _ => breadcrumb(uri, Vec::new(), &symbol_local_label(symbol, &self.library)),
        }
    }

    fn owner_breadcrumb(&self, owner: StdlibOwner) -> Vec<(String, String)> {
        match owner {
            StdlibOwner::Root => Vec::new(),
            StdlibOwner::Namespace(id) => {
                let namespace = self.library.namespace(id);
                namespace
                    .path
                    .iter()
                    .enumerate()
                    .filter_map(|(index, name)| {
                        let path = &namespace.path[..=index];
                        self.library
                            .namespaces()
                            .iter()
                            .find(|candidate| candidate.path == path)
                            .map(|ancestor| {
                                (
                                    (*name).to_owned(),
                                    symbol_uri(
                                        StdlibSymbolId::Namespace(ancestor.id),
                                        &self.library,
                                    ),
                                )
                            })
                    })
                    .collect()
            }
            StdlibOwner::Type(id) => vec![(
                self.library.type_decl(id).name.to_owned(),
                symbol_uri(StdlibSymbolId::Type(id), &self.library),
            )],
            StdlibOwner::Core(id) => vec![(
                self.library.core_type(id).name.to_owned(),
                core_type_uri(id, &self.library),
            )],
            StdlibOwner::Capability(id) => vec![(
                self.library.capability(id).name.to_owned(),
                symbol_uri(StdlibSymbolId::Capability(id), &self.library),
            )],
            StdlibOwner::TypeConstructor(id) => vec![(
                self.library.render_type_constructor(id),
                symbol_uri(StdlibSymbolId::TypeConstructor(id), &self.library),
            )],
        }
    }
}

fn render_constrained_type_constructor(
    constructor: &crate::stdlib::StdlibTypeConstructor,
    library: &StandardLibrary,
) -> String {
    if constructor.parameters.is_empty() {
        return constructor.name.to_owned();
    }
    let parameters = constructor
        .parameters
        .iter()
        .map(|parameter| {
            let capabilities = library.minimal_capabilities(parameter.constraints);
            if capabilities.is_empty() {
                parameter.name.to_owned()
            } else {
                format!(
                    "{} where {}",
                    parameter.name,
                    capabilities
                        .iter()
                        .map(|capability| library.capability(*capability).name)
                        .collect::<Vec<_>>()
                        .join(" + ")
                )
            }
        })
        .collect::<Vec<_>>()
        .join(", ");
    match constructor.syntax {
        crate::stdlib::TypeConstructorSyntax::Named => {
            format!("{}<{parameters}>", constructor.name)
        }
        crate::stdlib::TypeConstructorSyntax::Array => format!("[{parameters}]"),
        crate::stdlib::TypeConstructorSyntax::Optional => format!("{parameters}?"),
        crate::stdlib::TypeConstructorSyntax::Fallible => format!("{parameters}!"),
        crate::stdlib::TypeConstructorSyntax::ExclusiveRange => {
            format!("{parameters}..<{parameters}")
        }
        crate::stdlib::TypeConstructorSyntax::InclusiveRange => {
            format!("{parameters}..={parameters}")
        }
    }
}

fn type_constructor_slug(constructor: &crate::stdlib::StdlibTypeConstructor) -> &'static str {
    match constructor.syntax {
        crate::stdlib::TypeConstructorSyntax::Named => constructor.name,
        crate::stdlib::TypeConstructorSyntax::Array => "array",
        crate::stdlib::TypeConstructorSyntax::Optional => "optional",
        crate::stdlib::TypeConstructorSyntax::Fallible => "fallible",
        crate::stdlib::TypeConstructorSyntax::ExclusiveRange => "exclusive-range",
        crate::stdlib::TypeConstructorSyntax::InclusiveRange => "inclusive-range",
    }
}

fn render_type_declaration(ty: &crate::stdlib::StdlibType) -> String {
    match ty.kind {
        StdlibTypeKind::Intrinsic => ty.name.to_owned(),
        StdlibTypeKind::Struct => format!("struct {}", ty.name),
        StdlibTypeKind::Enum => format!("enum {}", ty.name),
    }
}

fn render_item_name(item: &crate::stdlib::StdlibItem, library: &StandardLibrary) -> String {
    match item.owner {
        StdlibOwner::TypeConstructor(owner) => {
            format!("{}.{}", library.render_type_constructor(owner), item.name)
        }
        _ => item.qualified_name.to_owned(),
    }
}

fn append_documentation<Id>(
    markdown: &mut String,
    current_uri: &str,
    library: &StandardLibrary,
    documentation: &Documentation<Id>,
    semantic_examples: bool,
) {
    let summary = intra_doc::render_links(documentation.summary, current_uri, library);
    let details = intra_doc::render_links(documentation.details, current_uri, library);
    markdown.push_str("\n\n");
    markdown.push_str(&super::prose_markdown(&summary, &details));
    append_examples(
        markdown,
        current_uri,
        library,
        documentation.examples,
        semantic_examples,
    );
}

fn append_examples(
    markdown: &mut String,
    current_uri: &str,
    library: &StandardLibrary,
    examples: &[crate::catalog::Example],
    semantic_examples: bool,
) {
    if examples.is_empty() {
        return;
    }
    markdown.push_str("\n\n## Examples");
    for example in examples {
        markdown.push_str(&format!("\n\n_{}_\n\n", example.title));
        markdown.push_str(&if semantic_examples {
            code::example(*example, current_uri, library)
        } else {
            code::lexical_example(*example, current_uri, library)
        });
    }
}

fn append_related(
    markdown: &mut String,
    current_uri: &str,
    library: &StandardLibrary,
    related: &[StdlibSymbolId],
) {
    if related.is_empty() {
        return;
    }
    markdown.push_str("\n\n## Related\n");
    let members = related
        .iter()
        .copied()
        .map(DocumentationMember::Symbol)
        .collect::<Vec<_>>();
    append_member_table(markdown, current_uri, &members, library);
}

fn append_member_table(
    markdown: &mut String,
    current_uri: &str,
    members: &[DocumentationMember],
    library: &StandardLibrary,
) {
    append_reference_table_header(markdown, &["Member", "Description", "Available through"]);
    for member in members {
        let (member_link, description, available_through) =
            member_markdown(current_uri, *member, library);
        markdown.push_str(&format!(
            "\n| {member_link} | {description} | {} |",
            available_through
                .map(|capability| format!("Available through {capability}"))
                .unwrap_or_default()
        ));
    }
}

fn member_markdown(
    current_uri: &str,
    member: DocumentationMember,
    library: &StandardLibrary,
) -> (String, String, Option<String>) {
    let (label, target, available_through) = match member {
        DocumentationMember::Symbol(symbol) => (
            symbol_local_label(symbol, library),
            symbol_uri(symbol, library),
            None,
        ),
        DocumentationMember::CapabilitySymbol { symbol, capability } => (
            symbol_local_label(symbol, library),
            symbol_uri(symbol, library),
            Some(capability),
        ),
        DocumentationMember::CoreType(ty) => (
            library.core_type(ty).name.to_owned(),
            core_type_uri(ty, library),
            None,
        ),
    };
    let label = escape_markdown_symbol(&label);
    let member_link = format!(
        "[{label}]({})",
        relative_document_link(current_uri, &target)
    );
    let available_through = available_through.map(|capability| {
        let capability = StdlibSymbolId::Capability(capability);
        format!(
            "[{}]({})",
            symbol_local_label(capability, library),
            relative_document_link(current_uri, &symbol_uri(capability, library))
        )
    });
    (
        member_link,
        table_prose(member_summary(member, library), current_uri, library),
        available_through,
    )
}

fn member_summary(member: DocumentationMember, library: &StandardLibrary) -> &'static str {
    let symbol = match member {
        DocumentationMember::Symbol(symbol)
        | DocumentationMember::CapabilitySymbol { symbol, .. } => symbol,
        DocumentationMember::CoreType(ty) => {
            return LanguageCatalog::new()
                .builtin_type(ty)
                .expect("every documented core type has a language-catalog entry")
                .documentation
                .summary;
        }
    };
    match symbol {
        StdlibSymbolId::StateProvider(id) => library.state_provider(id).documentation.summary,
        StdlibSymbolId::Namespace(id) => library.namespace(id).documentation.summary,
        StdlibSymbolId::Capability(id) => library.capability(id).documentation.summary,
        StdlibSymbolId::TypeConstructor(id) => library.type_constructor(id).documentation.summary,
        StdlibSymbolId::Type(id) => library.type_decl(id).documentation.summary,
        StdlibSymbolId::Field(id) => library.field(id).documentation.summary,
        StdlibSymbolId::Variant(id) => library.variant(id).documentation.summary,
        StdlibSymbolId::Item(id) => library.item(id).documentation.summary,
    }
}

fn table_prose(value: &str, current_uri: &str, library: &StandardLibrary) -> String {
    escape_markdown_table_cell(&intra_doc::render_links(value, current_uri, library))
}

fn compact_prose(value: &str) -> String {
    intra_doc::strip_links(value)
}

fn migration_source_code(value: &str) -> String {
    format!(
        "<code>{}</code>",
        value
            .replace('&', "&amp;")
            .replace('<', "&lt;")
            .replace('>', "&gt;")
    )
}

fn migration_search_text(concept: &MigrationConcept, migration: &MigrationCatalog) -> String {
    let mut parts = vec![
        concept.id.as_str().to_owned(),
        concept.name.to_owned(),
        concept.summary.to_owned(),
        concept.support.label().to_owned(),
    ];
    parts.extend(
        concept
            .sources
            .iter()
            .map(|source| source.name().to_owned()),
    );
    parts.extend(
        concept
            .targets
            .iter()
            .map(|target| migration.target_display(*target)),
    );
    for spelling in concept.spellings {
        parts.push(spelling.spelling.to_owned());
        parts.push(spelling.message.to_owned());
        parts.push(spelling.primary_label.to_owned());
    }
    parts.extend(
        migration
            .query_aliases()
            .iter()
            .filter(|aliases| aliases.concept == concept.id)
            .flat_map(|aliases| aliases.queries.iter().map(|query| (*query).to_owned())),
    );
    for diagnostic in migration
        .diagnostics()
        .iter()
        .filter(|diagnostic| diagnostic.concept == concept.id)
    {
        parts.push(diagnostic.id.as_str().to_owned());
        parts.push(diagnostic.message.to_owned());
        parts.push(diagnostic.primary_label.to_owned());
        parts.extend(diagnostic.notes.iter().map(|note| (*note).to_owned()));
    }
    parts.join(" ")
}

#[derive(Debug)]
struct SearchText {
    original: String,
    lowercase: String,
    normalized: String,
    compact: String,
    words: Vec<String>,
}

impl SearchText {
    fn new(value: &str) -> Self {
        let original = value.trim().to_owned();
        let lowercase = original.to_lowercase();
        let normalized = normalize_search_text(&original);
        let words = normalized.split_whitespace().map(str::to_owned).collect();
        let compact = normalized
            .chars()
            .filter(|character| *character != ' ')
            .collect();
        Self {
            original,
            lowercase,
            normalized,
            compact,
            words,
        }
    }
}

fn normalize_search_text(value: &str) -> String {
    let mut normalized = String::new();
    let mut separator = false;
    for character in value.chars().flat_map(char::to_lowercase) {
        if character.is_alphanumeric() {
            if separator && !normalized.is_empty() {
                normalized.push(' ');
            }
            separator = false;
            normalized.push(character);
        } else {
            separator = true;
        }
    }
    normalized
        .split_whitespace()
        .map(|word| {
            let suffix = word.strip_prefix("string").unwrap_or_default();
            if !suffix.is_empty() && suffix.chars().all(|character| character.is_ascii_digit()) {
                "stringn"
            } else {
                word
            }
        })
        .collect::<Vec<_>>()
        .join(" ")
}

fn search_score(
    entry: &DocumentationIndexEntry,
    query: &SearchText,
    exact_alias: bool,
) -> Option<u32> {
    if exact_alias {
        return Some(20_000);
    }

    let title = normalize_search_text(&entry.title);
    let signature = entry
        .signature
        .as_deref()
        .map(normalize_search_text)
        .unwrap_or_default();
    let body = normalize_search_text(&format!("{} {}", entry.summary, entry.search_text));
    let raw_title = entry.title.to_lowercase();
    let raw_signature = entry
        .signature
        .as_deref()
        .unwrap_or_default()
        .to_lowercase();
    let raw_body = format!("{} {}", entry.summary, entry.search_text).to_lowercase();
    let title_compact = title.replace(' ', "");
    let signature_compact = signature.replace(' ', "");
    let body_compact = body.replace(' ', "");

    if raw_title == query.lowercase || raw_signature == query.lowercase {
        return Some(19_000);
    }
    if !query.normalized.is_empty() && (title == query.normalized || signature == query.normalized)
    {
        return Some(19_000);
    }
    if !query.compact.is_empty()
        && (title_compact == query.compact || signature_compact == query.compact)
    {
        return Some(18_000);
    }

    let mut score = 0;
    if contains_search_literal(&raw_title, &query.lowercase)
        || contains_search_literal(&raw_signature, &query.lowercase)
        || contains_search_literal(&raw_body, &query.lowercase)
    {
        score += 9_000;
    }
    if !query.normalized.is_empty() && title.starts_with(&query.normalized) {
        score += 8_000;
    } else if !query.normalized.is_empty()
        && (title.contains(&query.normalized) || title_compact.contains(&query.compact))
    {
        score += 6_000;
    }
    if !query.normalized.is_empty() && signature.starts_with(&query.normalized) {
        score += 5_000;
    } else if !query.normalized.is_empty()
        && (signature.contains(&query.normalized) || signature_compact.contains(&query.compact))
    {
        score += 4_000;
    }
    if !query.normalized.is_empty()
        && (body.contains(&query.normalized) || body_compact.contains(&query.compact))
    {
        score += 2_000;
    }

    let mut matched_words = 0;
    for word in &query.words {
        if title.split_whitespace().any(|candidate| candidate == word) {
            score += 500;
            matched_words += 1;
        } else if title.contains(word) {
            score += 350;
            matched_words += 1;
        } else if signature.contains(word) {
            score += 250;
            matched_words += 1;
        } else if body.contains(word) {
            score += 100;
            matched_words += 1;
        }
    }
    if matched_words == query.words.len() {
        score += 1_000;
    }
    (score != 0).then_some(score)
}

fn contains_search_literal(haystack: &str, needle: &str) -> bool {
    if needle.is_empty() {
        return false;
    }
    haystack.match_indices(needle).any(|(start, _)| {
        let end = start + needle.len();
        let starts_with_word = needle.chars().next().is_some_and(char::is_alphanumeric);
        let ends_with_word = needle
            .chars()
            .next_back()
            .is_some_and(char::is_alphanumeric);
        let left_boundary = !starts_with_word
            || haystack[..start]
                .chars()
                .next_back()
                .is_none_or(|character| !character.is_alphanumeric());
        let right_boundary = !ends_with_word
            || haystack[end..]
                .chars()
                .next()
                .is_none_or(|character| !character.is_alphanumeric());
        left_boundary && right_boundary
    })
}

pub(super) fn append_reference_table_header(markdown: &mut String, columns: &[&str]) {
    markdown.push_str("\n<div class=\"splitscript-reference-table\"></div>\n\n|");
    for column in columns {
        markdown.push_str(&format!(" {column} |"));
    }
    markdown.push_str("\n|");
    for _ in columns {
        markdown.push_str(" --- |");
    }
}

pub(super) fn escape_markdown_table_cell(value: &str) -> String {
    value.replace('|', "\\|")
}

/// Produces an ordinary relative Markdown link so VS Code's Markdown preview
/// keeps navigation inside the current virtual-document scheme. Absolute
/// custom-scheme links are treated as external resources and are not routed
/// back through `TextDocumentContentProvider`.
pub(super) fn relative_document_link(current_uri: &str, target_uri: &str) -> String {
    let mut current = current_uri
        .trim_start_matches('/')
        .split('/')
        .collect::<Vec<_>>();
    current.pop();
    let target = target_uri
        .trim_start_matches('/')
        .split('/')
        .collect::<Vec<_>>();

    let common = current
        .iter()
        .zip(&target)
        .take_while(|(left, right)| left == right)
        .count();
    let mut relative = "../".repeat(current.len() - common);
    relative.push_str(&target[common..].join("/"));
    relative
}

fn breadcrumb(uri: &str, ancestors: Vec<(String, String)>, current: &str) -> String {
    reference_breadcrumb(uri, ancestors, current)
}

fn reference_breadcrumb(uri: &str, ancestors: Vec<(String, String)>, current: &str) -> String {
    let mut markdown = format!(
        "[SplitScript reference]({})",
        relative_document_link(uri, "/index.md")
    );
    for (label, target) in ancestors {
        markdown.push_str(&format!(
            " / [{}]({})",
            escape_markdown_symbol(&label),
            relative_document_link(uri, &target)
        ));
    }
    markdown.push_str(&format!(" / {}", escape_markdown_symbol(current)));
    markdown
}

fn symbol_label(symbol: StdlibSymbolId, library: &StandardLibrary) -> String {
    match symbol {
        StdlibSymbolId::StateProvider(id) => library.state_provider(id).name.to_owned(),
        StdlibSymbolId::Namespace(id) => library.namespace(id).path.join("."),
        StdlibSymbolId::Capability(id) => library.capability(id).name.to_owned(),
        StdlibSymbolId::TypeConstructor(id) => library.render_type_constructor(id),
        StdlibSymbolId::Type(id) => library.type_decl(id).name.to_owned(),
        StdlibSymbolId::Field(id) => {
            let field = library.field(id);
            format!("{}.{}", library.render_field_owner(field.owner), field.name)
        }
        StdlibSymbolId::Variant(id) => {
            let variant = library.variant(id);
            format!("{}.{}", library.type_decl(variant.owner).name, variant.name)
        }
        StdlibSymbolId::Item(id) => render_item_name(library.item(id), library),
    }
}

fn member_label(member: DocumentationMember, library: &StandardLibrary) -> String {
    match member {
        DocumentationMember::Symbol(symbol)
        | DocumentationMember::CapabilitySymbol { symbol, .. } => symbol_label(symbol, library),
        DocumentationMember::CoreType(ty) => library.core_type(ty).name.to_owned(),
    }
}

fn symbol_local_label(symbol: StdlibSymbolId, library: &StandardLibrary) -> String {
    match symbol {
        StdlibSymbolId::StateProvider(id) => library.state_provider(id).name.to_owned(),
        StdlibSymbolId::Namespace(id) => library.namespace(id).name.to_owned(),
        StdlibSymbolId::Capability(id) => library.capability(id).name.to_owned(),
        StdlibSymbolId::TypeConstructor(id) => library.render_type_constructor(id),
        StdlibSymbolId::Type(id) => library.type_decl(id).name.to_owned(),
        StdlibSymbolId::Field(id) => library.field(id).name.to_owned(),
        StdlibSymbolId::Variant(id) => library.variant(id).name.to_owned(),
        StdlibSymbolId::Item(id) => {
            let item = library.item(id);
            operator_symbol(item).unwrap_or(item.name).to_owned()
        }
    }
}

fn field_owner_symbol(owner: StdlibOwner) -> StdlibSymbolId {
    match owner {
        StdlibOwner::Type(owner) => StdlibSymbolId::Type(owner),
        StdlibOwner::TypeConstructor(owner) => StdlibSymbolId::TypeConstructor(owner),
        _ => unreachable!("fields have type or type-constructor owners"),
    }
}

fn is_operator(item: &crate::stdlib::StdlibItem) -> bool {
    item.binary_operator.is_some() || item.unary_operator.is_some()
}

fn operator_symbol(item: &crate::stdlib::StdlibItem) -> Option<&'static str> {
    item.binary_operator
        .map(crate::stdlib::StandardBinaryOperator::symbol)
        .or_else(|| {
            item.unary_operator
                .map(crate::stdlib::StandardUnaryOperator::symbol)
        })
}

pub(super) fn core_type_uri(id: CoreTypeId, library: &StandardLibrary) -> String {
    format!("/stdlib/types/{}/index.md", library.core_type(id).name)
}

pub(crate) fn language_item_uri(id: LanguageItemId) -> String {
    let language = LanguageCatalog::new();
    let item = language.item(id);
    if matches!(item.kind, LanguageItemKind::BuiltinType(_)) {
        return format!("/stdlib/types/{}/index.md", item.name);
    }
    let slug = match item.name {
        "==" => "equality".to_owned(),
        "!=" => "inequality".to_owned(),
        "T?" => "optional-type".to_owned(),
        "T!" => "fallible-type".to_owned(),
        "?" => "error-propagation".to_owned(),
        "[T; N]" => "array-type".to_owned(),
        "///" => "documentation-comment".to_owned(),
        name => documentation_slug(name),
    };
    format!("/language/{slug}.md")
}

pub fn migration_concept_uri(id: MigrationConceptId) -> String {
    migration_topic_uri(id.as_str())
}

pub(crate) fn migration_topic_uri(topic: &str) -> String {
    format!("/migration/{}.md", topic.replace('.', "/"))
}

fn documentation_slug(name: &str) -> String {
    let mut slug = String::new();
    let mut separator = false;
    let mut previous_was_lowercase = false;
    for character in name.chars() {
        if character.is_ascii_alphanumeric() {
            if (separator || (character.is_ascii_uppercase() && previous_was_lowercase))
                && !slug.is_empty()
            {
                slug.push('-');
            }
            separator = false;
            slug.push(character.to_ascii_lowercase());
            previous_was_lowercase = character.is_ascii_lowercase();
        } else {
            separator = true;
            previous_was_lowercase = false;
        }
    }
    slug
}

fn language_item_kind_label(kind: LanguageItemKind) -> &'static str {
    match kind {
        LanguageItemKind::Keyword => "keyword",
        LanguageItemKind::Declaration => "declaration",
        LanguageItemKind::Syntax => "syntax",
        LanguageItemKind::BuiltinType(_) => "built-in type",
        LanguageItemKind::SnapshotRoot => "contextual value",
        LanguageItemKind::Action(_) => "lifecycle block",
    }
}

pub(super) fn migration_target_uri(
    target: MigrationTarget,
    library: &StandardLibrary,
) -> Option<String> {
    match target {
        MigrationTarget::Language(name) => {
            LanguageCatalog::new()
                .item_for_source_token(name)
                .map(|item| match item.kind {
                    LanguageItemKind::BuiltinType(ty) => core_type_uri(ty, library),
                    _ => language_item_uri(item.id),
                })
        }
        MigrationTarget::StandardLibraryType(name) => library
            .type_by_name(name)
            .map(|ty| symbol_uri(StdlibSymbolId::Type(ty.id), library))
            .or_else(|| {
                library
                    .core_types()
                    .iter()
                    .find(|ty| ty.name == name)
                    .map(|ty| core_type_uri(ty.id, library))
            }),
        MigrationTarget::StandardLibraryItem(name) => library
            .item_by_name(name)
            .map(|item| symbol_uri(StdlibSymbolId::Item(item.id), library)),
        MigrationTarget::StateProvider(name) => library
            .state_provider_by_name(name)
            .map(|provider| symbol_uri(StdlibSymbolId::StateProvider(provider.id), library)),
    }
}

fn migration_target_kind(target: MigrationTarget) -> &'static str {
    match target {
        MigrationTarget::Language(_) => "language",
        MigrationTarget::StandardLibraryType(_) => "type",
        MigrationTarget::StandardLibraryItem(_) => "standard library",
        MigrationTarget::StateProvider(_) => "state provider",
    }
}

pub(crate) fn symbol_uri(symbol: StdlibSymbolId, library: &StandardLibrary) -> String {
    match symbol {
        StdlibSymbolId::StateProvider(id) => format!(
            "/stdlib/state-providers/{}.md",
            library.state_provider(id).name
        ),
        StdlibSymbolId::Namespace(id) => format!(
            "/stdlib/namespaces/{}/index.md",
            library.namespace(id).path.join("/")
        ),
        StdlibSymbolId::Capability(id) => format!(
            "/stdlib/capabilities/{}/index.md",
            library.capability(id).name
        ),
        StdlibSymbolId::TypeConstructor(id) => format!(
            "/stdlib/type-forms/{}/index.md",
            type_constructor_slug(library.type_constructor(id))
        ),
        StdlibSymbolId::Type(id) => {
            format!("/stdlib/types/{}/index.md", library.type_decl(id).name)
        }
        StdlibSymbolId::Field(id) => {
            let field = library.field(id);
            match field.owner {
                StdlibOwner::Type(owner) => format!(
                    "/stdlib/types/{}/fields/{}.md",
                    library.type_decl(owner).name,
                    field.name
                ),
                StdlibOwner::TypeConstructor(owner) => format!(
                    "/stdlib/type-forms/{}/fields/{}.md",
                    type_constructor_slug(library.type_constructor(owner)),
                    field.name
                ),
                _ => unreachable!("fields have type or type-constructor owners"),
            }
        }
        StdlibSymbolId::Variant(id) => {
            let variant = library.variant(id);
            format!(
                "/stdlib/types/{}/variants/{}.md",
                library.type_decl(variant.owner).name,
                variant.name
            )
        }
        StdlibSymbolId::Item(id) => {
            let item = library.item(id);
            match item.owner {
                StdlibOwner::Root => format!("/stdlib/functions/{}.md", item.name),
                StdlibOwner::Namespace(owner) => format!(
                    "/stdlib/namespaces/{}/{}.md",
                    library.namespace(owner).path.join("/"),
                    item.name
                ),
                StdlibOwner::Type(owner) => format!(
                    "/stdlib/types/{}/{}/{}.md",
                    library.type_decl(owner).name,
                    if is_operator(item) {
                        "operators"
                    } else {
                        "methods"
                    },
                    item.name
                ),
                StdlibOwner::Core(owner) => format!(
                    "/stdlib/types/{}/{}/{}.md",
                    library.core_type(owner).name,
                    if is_operator(item) {
                        "operators"
                    } else {
                        "methods"
                    },
                    item.name
                ),
                StdlibOwner::Capability(owner) => format!(
                    "/stdlib/capabilities/{}/{}/{}.md",
                    library.capability(owner).name,
                    if is_operator(item) {
                        "operators"
                    } else {
                        "methods"
                    },
                    item.name
                ),
                StdlibOwner::TypeConstructor(owner) => format!(
                    "/stdlib/type-forms/{}/{}/{}.md",
                    type_constructor_slug(library.type_constructor(owner)),
                    if is_operator(item) {
                        "operators"
                    } else {
                        "methods"
                    },
                    item.name
                ),
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn reference_indexes_and_renders_all_standard_library_symbol_kinds() {
        let reference = DocumentationReference::default();
        let index = reference.index();
        assert!(
            index
                .iter()
                .all(|entry| !entry.summary.contains("[`") && !entry.summary.contains("`]")),
            "searchable index summaries must not expose intra-doc authoring markup"
        );
        for kind in [
            "namespace",
            "built-in type",
            "capability",
            "type constructor",
            "type",
            "struct",
            "enum",
            "field",
            "enum variant",
            "state provider",
            "function",
            "method",
            "operator",
        ] {
            assert!(index.iter().any(|entry| entry.kind == kind), "{kind}");
        }

        let duration = index
            .iter()
            .find(|entry| entry.title == "Duration")
            .expect("Duration is indexed");
        assert!(
            index
                .iter()
                .all(|entry| !entry.title.contains("MonoLayout")),
            "private standard-library types and their members must not be indexed"
        );
        assert!(
            reference
                .page("/stdlib/types/MonoLayout/index.md")
                .is_none()
        );
        assert_eq!(duration.uri, "/stdlib/types/Duration/index.md");
        let page = reference.page(&duration.uri).expect("Duration has a page");
        assert!(page.markdown.contains("\n\n# Duration\n"));
        assert!(
            page.markdown
                .starts_with("[SplitScript reference](../../../index.md) / Duration")
        );
        assert!(
            page.markdown
                .contains("[fromSeconds](methods/fromSeconds.md)")
        );
        let from_milliseconds = reference
            .page("/stdlib/types/Duration/methods/fromMilliseconds.md")
            .expect("Duration.fromMilliseconds has a page");
        assert!(
            from_milliseconds
                .markdown
                .contains("[`fromSeconds`](fromSeconds.md)"),
            "standard-library prose should render navigable intra-doc references"
        );
        let module_address = reference
            .page("/stdlib/types/Module/fields/address.md")
            .expect("Module.address has a page");
        assert_eq!(
            module_address
                .markdown
                .matches("Returns the module base address.")
                .count(),
            1
        );
        assert!(
            module_address
                .markdown
                .contains("Relative virtual addresses within the image")
        );
        assert!(!page.markdown.contains("splitscript-docs:"));
    }

    #[test]
    fn reference_root_only_contains_top_level_declarations() {
        let reference = DocumentationReference::default();
        let page = reference.page("/index.md").expect("root page exists");
        assert_eq!(page.title, "SplitScript reference");
        assert!(page.markdown.contains("# SplitScript reference"));
        assert!(page.markdown.contains("[Language](language/index.md)"));
        assert!(page.markdown.contains("[Migration](migration/index.md)"));
        assert!(
            page.markdown
                .contains("[Duration](stdlib/types/Duration/index.md)")
        );
        assert!(page.markdown.contains(
            "<div class=\"splitscript-reference-table\"></div>\n\n\
             | Symbol | Description |"
        ));
        assert!(!page.markdown.contains("Duration.fromSeconds"));
        assert!(!page.markdown.contains("FileVersion.major"));
        assert!(!page.markdown.contains("TimerState.Running"));
        assert!(reference.page("/missing.md").is_none());
    }

    #[test]
    fn invalid_documentation_routes_never_enter_the_page_cache() {
        let reference = DocumentationReference::default();
        let routes = reference.page_routes();
        assert_eq!(routes.len(), reference.index().len() + 1);

        let invalid = (0..1_000)
            .map(|index| format!("/invalid-cache-probe-{index}.md"))
            .collect::<Vec<_>>();
        for uri in &invalid {
            assert!(reference.page(uri).is_none());
        }
        for uri in [
            "index.md",
            "//index.md",
            "/index.md?query",
            "/guides/../index.md",
        ] {
            assert!(
                reference.page(uri).is_none(),
                "accepted invalid route `{uri}`"
            );
        }

        let cache = PAGE_CACHE
            .get_or_init(|| Mutex::new(HashMap::new()))
            .lock()
            .expect("documentation page cache should not be poisoned");
        assert!(cache.keys().all(|(_, uri)| !invalid.contains(uri)));
    }

    #[test]
    fn owned_symbols_are_nested_under_their_declaring_types() {
        let reference = DocumentationReference::default();

        let method = reference
            .page("/stdlib/types/Duration/methods/fromSeconds.md")
            .expect("Duration.fromSeconds has a page");
        assert!(method.markdown.starts_with(
            "[SplitScript reference](../../../../index.md) / [Duration](../index.md) / fromSeconds"
        ));

        let field = reference
            .page("/stdlib/types/FileVersion/fields/major.md")
            .expect("FileVersion.major has a page");
        assert!(field.markdown.starts_with(
            "[SplitScript reference](../../../../index.md) / [FileVersion](../index.md) / major"
        ));

        let variant = reference
            .page("/stdlib/types/TimerState/variants/Running.md")
            .expect("TimerState.Running has a page");
        assert!(variant.markdown.starts_with(
            "[SplitScript reference](../../../../index.md) / [TimerState](../index.md) / Running"
        ));
    }

    #[test]
    fn callable_pages_contain_semantic_code_and_navigable_examples() {
        let reference = DocumentationReference::default();
        let page = reference
            .page("/stdlib/types/Process/methods/read.md")
            .expect("Process.read has a page");

        assert!(
            page.markdown
                .contains("<pre class=\"hljs splitscript-code\">")
        );
        assert!(
            page.markdown
                .contains("href=\"../../../capabilities/MemoryReadable/index.md\"")
        );
        assert!(
            page.markdown
                .contains("href=\"../../address/methods/offset.md\"")
        );
        assert!(!page.markdown.contains("# state \"game.exe\""));
        assert!(!page.markdown.contains("# let player"));

        let byte_at = reference
            .page("/stdlib/types/String/methods/byteAt.md")
            .expect("String.byteAt has a page");
        assert!(
            byte_at
                .markdown
                .contains("\n    <a href=\"../../../functions/print.md\">")
        );
    }

    #[test]
    fn language_and_migration_catalogs_are_navigable_reference_pages() {
        let reference = DocumentationReference::default();
        let index = reference.index();

        let while_attached = index
            .iter()
            .find(|entry| entry.title == "whileAttached" && entry.kind == "lifecycle block")
            .expect("language lifecycle blocks are searchable");
        assert_eq!(while_attached.uri, "/language/while-attached.md");
        let language_page = reference
            .page(&while_attached.uri)
            .expect("language item has a page");
        assert!(language_page.markdown.starts_with(
            "[SplitScript reference](../index.md) / [Language](index.md) / whileAttached"
        ));
        assert!(language_page.markdown.contains("# whileAttached"));
        assert!(language_page.markdown.contains("_lifecycle block_"));
        assert!(
            language_page
                .markdown
                .contains("<pre class=\"hljs splitscript-code\">")
        );

        let on_attach = reference
            .page("/language/on-attach.md")
            .expect("onAttach has a language page");
        for related in [
            "[state](state.md)",
            "[conditional fields](conditional-fields.md)",
            "[await](await.md)",
            "[retry](retry.md)",
            "[onStateReady](on-state-ready.md)",
        ] {
            assert!(
                on_attach.markdown.contains(related),
                "onAttach is missing related concept `{related}`"
            );
        }

        let async_page = reference
            .page("/language/async.md")
            .expect("async has a language page");
        assert!(async_page.markdown.contains("[`await`](await.md)"));
        assert!(async_page.markdown.contains("[`retry`](retry.md)"));
        for keyword in ["fn", "async", "let", "await", "return"] {
            assert!(
                async_page
                    .markdown
                    .contains(&format!("href=\"{keyword}.md\"")),
                "missing semantic example link for `{keyword}`"
            );
        }

        let update = index
            .iter()
            .find(|entry| entry.signature.as_deref() == Some("asl.lifecycle.update"))
            .expect("migration concepts are searchable by stable identity");
        assert_eq!(update.uri, "/migration/asl/lifecycle/update.md");
        let migration_page = reference.page(&update.uri).expect("migration page exists");
        assert!(migration_page.markdown.contains("# update lifecycle block"));
        for heading in [
            "## Source pattern",
            "## Canonical SplitScript",
            "## Semantic difference",
            "## Supported hosts",
            "## Related reference",
        ] {
            assert!(
                migration_page.markdown.contains(heading),
                "migration page is missing `{heading}`"
            );
        }
        assert!(
            migration_page
                .markdown
                .contains("[whileAttached](../../../language/while-attached.md)")
        );
        assert!(
            migration_page
                .markdown
                .contains("../../../guides/asl-porting.md#legacy-asl-lifecycle-blocks")
        );

        let migration_index = reference
            .page("/migration/index.md")
            .expect("migration index exists");
        for heading in [
            "## ASL",
            "### Attachment and state",
            "### Process and memory",
            "### Lifecycle and timer",
            "### Settings",
            "### Collections and text",
            "### Unity and emulators",
            "### Unsupported host behavior",
            "## C#",
            "## JavaScript",
            "## Rust",
        ] {
            assert!(
                migration_index.markdown.contains(heading),
                "migration index is missing `{heading}`"
            );
        }
        assert!(!migration_index.markdown.contains("| Concept |"));

        let guide = reference
            .page("/guides/asl-porting.md")
            .expect("the canonical porting guide is bundled");
        assert!(guide.markdown.contains("# Porting ASL to SplitScript"));
        assert!(guide.markdown.contains("## Legacy ASL lifecycle blocks"));
        assert!(
            guide
                .markdown
                .contains("[`onAttach`](../language/on-attach.md)"),
            "bundled guide prose should render intra-doc references"
        );
        assert!(!guide.markdown.contains("examples/"));
        assert!(!guide.markdown.contains("AXIOM_VERGE_PORT.md"));
        assert!(
            reference
                .page("/examples/a-plague-tale-innocence.md")
                .is_none()
        );
    }

    #[test]
    fn native_topics_resolve_stable_identities_titles_and_paths_exactly() {
        let reference = DocumentationReference::default();
        assert_eq!(
            reference.topic("").unwrap().uri,
            "/index.md",
            "an omitted CLI topic renders the reference index"
        );
        assert_eq!(
            reference.topic("asl.lifecycle.update").unwrap().uri,
            "/migration/asl/lifecycle/update.md"
        );
        assert_eq!(
            reference.topic("Process.read").unwrap().uri,
            "/stdlib/types/Process/methods/read.md"
        );
        assert_eq!(
            reference.topic("guides/asl-porting.md").unwrap().uri,
            "/guides/asl-porting.md"
        );
        assert!(reference.topic("read").is_none());
    }

    #[test]
    fn concise_generic_topics_resolve_to_canonical_symbol_pages() {
        let reference = DocumentationReference::default();

        for (query, expected_uri) in [
            ("Set", "/stdlib/type-forms/Set/index.md"),
            ("MapEntry.key", "/stdlib/type-forms/MapEntry/fields/key.md"),
        ] {
            assert_eq!(
                reference.topic(query).map(|page| page.uri),
                Some(expected_uri.to_owned()),
                "concise generic topic `{query}`",
            );
        }
        for private in [
            "ArrayIterator",
            "SetIterator",
            "ExclusiveRangeIterator",
            "InclusiveRangeIterator",
            "MapIterator",
            "FilterIterator",
        ] {
            assert!(
                reference.topic(private).is_none(),
                "private iterator implementation `{private}` must not have a documentation topic"
            );
        }
    }

    #[test]
    fn state_provider_index_lists_every_catalog_provider() {
        let reference = DocumentationReference::default();
        let page = reference
            .page(STATE_PROVIDER_INDEX_URI)
            .expect("state providers have a navigable index");

        for provider in reference.library.state_providers() {
            assert!(
                page.markdown.contains(&format!("[{}]", provider.name)),
                "{} should be linked from the state-provider index",
                provider.name
            );
        }
        assert!(page.markdown.contains("[Unity](Unity.md)"));
    }

    #[test]
    fn attachment_and_conditional_field_pages_explain_exact_names_and_build_selection() {
        let reference = DocumentationReference::default();
        let state = reference
            .page("/language/state.md")
            .expect("state has a language page");
        assert!(
            state
                .markdown
                .contains("Windows candidate must include that extension")
        );
        assert!(state.markdown.contains("Try alternate executable names"));
        assert!(state.markdown.contains("Support multiple game builds"));

        let conditional_fields = reference
            .page("/language/conditional-fields.md")
            .expect("conditional fields have a language page");
        assert!(
            conditional_fields
                .markdown
                .contains("Select and refine a supported build")
        );
        assert!(conditional_fields.markdown.contains("ordinary enum value"));

        let native = reference
            .index()
            .into_iter()
            .find(|entry| entry.title == "Native" && entry.kind == "state provider")
            .expect("Native state provider is indexed");
        let native = reference.page(&native.uri).expect("Native has a page");
        assert!(native.markdown.contains("Windows candidates must"));
        assert!(native.markdown.contains("Try alternate executable names"));
    }

    #[test]
    fn state_provider_pages_expose_contextual_values() {
        let reference = DocumentationReference::default();
        let unity = reference
            .index()
            .into_iter()
            .find(|entry| entry.title == "Unity" && entry.kind == "state provider")
            .expect("Unity state provider is indexed");
        let unity = reference.page(&unity.uri).expect("Unity has a page");

        assert!(unity.markdown.contains("## Contextual values"));
        assert!(unity.markdown.contains("`unity`:"));
        assert!(unity.markdown.contains("`UnityContext`"));
        assert!(
            unity
                .markdown
                .contains("Provides attachment-scoped Unity engine facilities")
        );
        assert!(
            unity
                .markdown
                .contains("scripts do not discover runtime images, classes, field offsets")
        );
        assert!(unity.markdown.contains("`PlayerController.Ref`"));
        assert!(
            unity
                .markdown
                .contains("Read one transactional managed snapshot")
        );
        assert!(unity.markdown.contains("Class checks share bounded"));
        assert!(!unity.markdown.contains("`MonoModule`"));
        assert!(!unity.markdown.contains("`UnityClass`"));
    }

    #[test]
    fn provider_readable_ranges_are_rendered_from_catalog_metadata() {
        let reference = DocumentationReference::default();
        for uri in [
            "/stdlib/state-providers/PS2.md",
            "/stdlib/types/PS2Emulator/methods/read.md",
        ] {
            let page = reference.page(uri).expect("PS2 read page should exist");
            assert!(page.markdown.contains("## Readable guest memory"), "{uri}");
            assert!(page.markdown.contains("`0x00100000..<0x02000000`"), "{uri}");
            assert!(page.markdown.contains("computed addresses remain fallible"));
        }

        let ps2 = reference
            .page("/stdlib/state-providers/PS2.md")
            .expect("PS2 provider page should exist");
        assert!(ps2.markdown.contains("## Mapping lifecycle"));
        assert!(ps2.markdown.contains("starts a new attachment"));

        let native = reference
            .page("/stdlib/state-providers/Native.md")
            .expect("Native provider page should exist");
        assert!(!native.markdown.contains("## Mapping lifecycle"));
    }

    #[test]
    fn managed_schema_pages_explain_identity_failure_layout_and_cost() {
        let reference = DocumentationReference::default();

        let image = reference
            .page("/language/image.md")
            .expect("image has a language page");
        assert!(
            image
                .markdown
                .contains("reachable image and class metadata")
        );
        assert!(image.markdown.contains("private implementation detail"));

        let class = reference
            .page("/language/class.md")
            .expect("class has a language page");
        for expected in [
            "`T.Ref` denotes a live remote object reference",
            "no partially populated object escapes",
            "without allocating a GC object",
            "Put build-specific fields in an",
            "retained only when used",
            "metadata traversal remains private",
        ] {
            assert!(class.markdown.contains(expected), "{expected}");
        }

        let metadata_names = reference
            .page("/language/from.md")
            .expect("from has a language page");
        assert!(metadata_names.markdown.contains("one unambiguous match"));
        assert!(
            metadata_names
                .markdown
                .contains("do not create a source-visible shape discriminator")
        );
    }

    #[test]
    fn reference_retains_semantic_code_annotations_for_every_frontend() {
        let reference = DocumentationReference::default();
        let page = reference
            .page("/stdlib/types/Process/methods/read.md")
            .expect("Process.read has a terminal page");

        assert!(
            page.markdown
                .contains("<pre class=\"hljs splitscript-code\">")
        );
        assert!(page.markdown.contains("data-splitscript-token=\"method\""));
        assert!(page.markdown.contains("data-splitscript-token=\"keyword\""));
    }

    #[test]
    fn documentation_search_covers_canonical_and_migration_vocabulary() {
        let reference = DocumentationReference::default();

        let timer = reference
            .topic("timer.CurrentPhase")
            .expect("an unambiguous foreign spelling resolves directly");
        assert_eq!(timer.uri, "/migration/asl/timer/state.md");

        let migration = MigrationCatalog::default();
        for aliases in migration.query_aliases() {
            let expected_uri = migration_concept_uri(aliases.concept);
            for query in aliases.queries {
                assert_eq!(
                    reference.topic(query).map(|page| page.uri),
                    Some(expected_uri.clone()),
                    "exact migration alias `{query}`",
                );
                assert_eq!(
                    reference
                        .search(query)
                        .first()
                        .map(|entry| entry.uri.as_str()),
                    Some(expected_uri.as_str()),
                    "migration search alias `{query}`",
                );
            }
        }

        for (query, expected_uri) in [
            ("modules.First()", "/migration/asl/process/modules.md"),
            ("multiple processes", "/migration/asl/state/attachment.md"),
            (".exe", "/language/provider.md"),
            ("refreshRate", "/migration/asl/runtime/refresh-rate.md"),
            (
                "TimeSpan.FromMilliseconds",
                "/stdlib/types/Duration/methods/fromMilliseconds.md",
            ),
            (
                "MemoryWatcherList",
                "/migration/asl/state/memory-watcher-list.md",
            ),
            ("Task.Run", "/migration/asl/async/task-run.md"),
            (
                "run scoped variables",
                "/migration/asl/state/attempt-scoped.md",
            ),
            (
                "settings Add loop",
                "/migration/asl/settings/registration.md",
            ),
            ("UnityASL", "/migration/asl/unity/managed-schema.md"),
            ("mono.Make", "/migration/asl/unity/managed-schema.md"),
            ("mono.MakeString", "/migration/asl/unity/managed-schema.md"),
            ("Unity.mono", "/migration/asl/unity/managed-schema.md"),
            (
                "settings.ContainsKey",
                "/migration/asl/settings/dynamic-lookup.md",
            ),
            ("string128", "/migration/asl/state/string-n.md"),
        ] {
            let results = reference.search(query);
            assert_eq!(
                results.first().map(|entry| entry.uri.as_str()),
                Some(expected_uri),
                "{query}"
            );
        }

        for (query, expected_provider_uris) in [
            (
                "Dolphin",
                &[
                    "/stdlib/state-providers/GCN.md",
                    "/stdlib/state-providers/Wii.md",
                ][..],
            ),
            (
                "Fusion",
                &[
                    "/stdlib/state-providers/SMS.md",
                    "/stdlib/state-providers/Genesis.md",
                ][..],
            ),
            ("PCSX2", &["/stdlib/state-providers/PS2.md"][..]),
            ("DuckStation", &["/stdlib/state-providers/PS1.md"][..]),
            ("mGBA", &["/stdlib/state-providers/GBA.md"][..]),
            (
                "RetroArch",
                &[
                    "/stdlib/state-providers/GBA.md",
                    "/stdlib/state-providers/PS1.md",
                    "/stdlib/state-providers/PS2.md",
                    "/stdlib/state-providers/SMS.md",
                    "/stdlib/state-providers/Genesis.md",
                    "/stdlib/state-providers/GCN.md",
                    "/stdlib/state-providers/Wii.md",
                ][..],
            ),
            (
                "DeepPointer",
                &[
                    "/stdlib/state-providers/GBA.md",
                    "/stdlib/state-providers/PS1.md",
                    "/stdlib/state-providers/PS2.md",
                    "/stdlib/state-providers/SMS.md",
                    "/stdlib/state-providers/Genesis.md",
                    "/stdlib/state-providers/GCN.md",
                    "/stdlib/state-providers/Wii.md",
                ][..],
            ),
        ] {
            let results = reference.search(query);
            for expected_uri in expected_provider_uris {
                assert!(
                    results
                        .iter()
                        .any(|entry| entry.uri.as_str() == *expected_uri),
                    "`{query}` did not surface `{expected_uri}`; got {results:#?}"
                );
            }
        }
    }

    #[test]
    fn every_searchable_documentation_identity_has_one_page() {
        let reference = DocumentationReference::default().with_lexical_examples();
        let index = reference.index();
        let mut uris = std::collections::HashSet::new();
        for entry in index {
            assert!(
                uris.insert(entry.uri.clone()),
                "duplicate URI `{}`",
                entry.uri
            );
            assert!(
                reference.page(&entry.uri).is_some(),
                "missing page for `{}` ({})",
                entry.title,
                entry.uri,
            );
        }
    }

    #[test]
    fn postfix_question_mark_has_a_stable_documentation_route() {
        let reference = DocumentationReference::default();
        assert!(
            reference
                .index()
                .iter()
                .any(|entry| entry.uri == "/language/error-propagation.md")
        );
    }

    #[test]
    fn documentation_links_are_relative_to_the_current_virtual_page() {
        assert_eq!(
            relative_document_link("/index.md", "/stdlib/types/Duration/index.md"),
            "stdlib/types/Duration/index.md"
        );
        assert_eq!(
            relative_document_link(
                "/stdlib/types/Duration/index.md",
                "/stdlib/types/Duration/methods/fromSeconds.md"
            ),
            "methods/fromSeconds.md"
        );
        assert_eq!(
            relative_document_link(
                "/stdlib/types/Duration/methods/fromSeconds.md",
                "/stdlib/types/Duration/methods/fromMinutes.md"
            ),
            "fromMinutes.md"
        );
    }

    #[test]
    fn operators_are_separate_from_methods_under_their_owner() {
        let reference = DocumentationReference::default();
        let index = reference.index();
        let add = index
            .iter()
            .find(|entry| entry.title == "Numeric.add")
            .expect("Numeric.add is indexed");
        assert_eq!(add.kind, "operator");
        assert_eq!(add.uri, "/stdlib/capabilities/Numeric/operators/add.md");

        let numeric = reference
            .page("/stdlib/capabilities/Numeric/index.md")
            .expect("Numeric has a page");
        assert!(numeric.markdown.contains("## Implemented by"));
        assert!(numeric.markdown.contains("[i32](../../types/i32/index.md)"));
        assert!(
            !numeric
                .markdown
                .contains("[bool](../../types/bool/index.md)")
        );
        assert!(numeric.markdown.contains("## Operators"));
        assert!(numeric.markdown.contains("[+](operators/add.md)"));
        assert!(numeric.markdown.contains("## Methods"));
        assert!(numeric.markdown.contains("[squared](methods/squared.md)"));

        let operator = reference.page(&add.uri).expect("the + operator has a page");
        assert!(operator.markdown.starts_with(
            "[SplitScript reference](../../../../index.md) / [Numeric](../index.md) / add"
        ));
        assert!(operator.markdown.contains("\n\n_Operator_\n\n"));

        let equatable = reference
            .page("/stdlib/capabilities/Equatable/index.md")
            .expect("Equatable has a page");
        assert!(
            equatable
                .markdown
                .contains("[FileVersion](../../types/FileVersion/index.md)")
        );
    }

    #[test]
    fn type_pages_include_members_available_through_transitive_capabilities() {
        let reference = DocumentationReference::default();
        let integer = reference
            .page("/stdlib/types/i32/index.md")
            .expect("i32 has a page");

        assert!(integer.markdown.contains(
            "<div class=\"splitscript-reference-table\"></div>\n\n\
             | Member | Description | Available through |"
        ));
        assert!(integer.markdown.contains(
            "| [+](../../capabilities/Numeric/operators/add.md) | \
             Adds another value of the same numeric type. | \
             Available through [Numeric](../../capabilities/Numeric/index.md) |"
        ));
        assert!(integer.markdown.contains(
            "| [%](../../capabilities/Integer/operators/remainder.md) | \
             Returns the remainder after integer division. | \
             Available through [Integer](../../capabilities/Integer/index.md) |"
        ));
        assert!(integer.markdown.contains(
            "| [\\|](../../capabilities/Integer/operators/bitOr.md) | \
             Combines two integers with a bitwise OR. | \
             Available through [Integer](../../capabilities/Integer/index.md) |"
        ));
        assert!(integer.markdown.contains(
            "| [abs](../../capabilities/Signed/methods/abs.md) | \
             Returns this value without a negative sign. | \
             Available through [Signed](../../capabilities/Signed/index.md) |"
        ));
        assert!(!integer.markdown.contains('—'));

        let duration = reference
            .page("/stdlib/types/Duration/index.md")
            .expect("Duration has a page");
        assert!(
            duration
                .markdown
                .lines()
                .any(|line| {
                    line == "| [+](operators/add.md) | Adds another duration and normalizes the result. |  |"
                })
        );
        assert!(duration.markdown.contains(
            "| [==](../../capabilities/Equatable/operators/equals.md) | \
             Reports whether this value and another value are equal. | \
             Available through [Equatable](../../capabilities/Equatable/index.md) |"
        ));

        let array = reference
            .page("/stdlib/type-forms/array/index.md")
            .expect("Array has a page");
        assert!(array.markdown.contains(
            "| [clear](methods/clear.md) | Removes every element from a growable array. |  |"
        ));
        assert!(!array.markdown.contains("\n- [clear](methods/clear.md)"));

        let type_form_titles = reference
            .index()
            .into_iter()
            .filter(|entry| entry.kind == "type constructor")
            .map(|entry| entry.title)
            .collect::<Vec<_>>();
        assert!(type_form_titles.contains(&"[T]".to_owned()));
        assert!(type_form_titles.contains(&"T?".to_owned()));
        assert!(type_form_titles.contains(&"T!".to_owned()));
        assert!(
            !type_form_titles
                .iter()
                .any(|title| { matches!(title.as_str(), "Array" | "Option" | "Result") })
        );

        let fallible = reference
            .page("/stdlib/type-forms/fallible/index.md")
            .expect("T! has a page");
        assert!(fallible.markdown.contains("# T!"));
        assert!(fallible.markdown.contains("discardError"));
        assert!(!fallible.markdown.contains("Result"));

        let exclusive_range = reference
            .page("/stdlib/type-forms/exclusive-range/index.md")
            .expect("T..<T has a page with a URL-safe path");
        assert!(exclusive_range.markdown.contains("# T..\\<T"));
        assert!(exclusive_range.markdown.contains("where"));
        assert!(exclusive_range.markdown.contains("Integer"));
        assert!(exclusive_range.markdown.contains("upper bound is excluded"));

        let inclusive_range = reference
            .page("/stdlib/type-forms/inclusive-range/index.md")
            .expect("T..=T has a page with a URL-safe path");
        assert!(inclusive_range.markdown.contains("# T..=T"));
        assert!(inclusive_range.markdown.contains("where"));
        assert!(inclusive_range.markdown.contains("Integer"));
        assert!(inclusive_range.markdown.contains("upper bound is included"));

        assert!(
            reference
                .index()
                .iter()
                .filter(|entry| entry.kind == "type constructor")
                .all(|entry| !entry.uri.contains(' ')),
            "documentation routes must not contain raw spaces"
        );
    }
}
