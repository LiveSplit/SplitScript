//! Declarative standard-library surface shared by compiler and tooling.
//!
//! Source names, callable shapes, type schemes, effects, and documentation live
//! here. Type checking resolves calls to stable item IDs. Backends only receive
//! stable intrinsic IDs and concrete inferred type arguments.

mod catalog;
mod declarations;
mod graph;
mod ids;
mod intrinsics;
mod library_bodies;
mod schema;
mod validation;

pub use ids::{
    IntrinsicId, StdlibCapabilityId, StdlibFieldId, StdlibItemId, StdlibNamespaceId,
    StdlibStateProviderId, StdlibTypeConstructorId, StdlibTypeId, StdlibVariantId,
};
pub use schema::{
    Availability, CancellationKind, Deprecation, Effect, EffectSet, Implementation,
    IntrinsicContext, ItemKind, ItemVisibility, OperationMetadata, OperationSemantics, Parameter,
    ParameterRule, Signature, StandardBinaryOperator, StandardUnaryOperator, StdlibItem,
    SuspensionKind, TypeParameter, TypeRef,
};

pub use declarations::{
    CapabilityBehavior, CoreType, CoreTypeId, DeclaredTypeRef, FieldVisibility,
    ManagedRuntimeBackend, RuntimeRepresentation, STATE_PROVIDER_MAPPING_LIFECYCLE_NOTE,
    ScalarMemoryLayout, StateProviderAttachment, StateProviderContext, StateProviderMemoryRange,
    StateProviderProcesses, StdlibAssociatedType, StdlibAssociatedTypeDefinition, StdlibCapability,
    StdlibField, StdlibNamespace, StdlibOwner, StdlibStateProvider, StdlibSymbolId, StdlibType,
    StdlibTypeConstructor, StdlibTypeKind, StdlibVariant, TypeConstructorSyntax, TypeVisibility,
    ValueUsage,
};

use catalog::{
    CAPABILITIES, FIELDS, ITEMS, NAMESPACES, STATE_PROVIDERS, TYPE_CONSTRUCTORS, TYPES, VARIANTS,
};
use declarations::CORE_TYPES;
pub(crate) use declarations::with_core_types;
pub(crate) use library_bodies::{
    MANAGED_ARRAY_TYPE_FIELD, MANAGED_BATCH_MIN_FIELDS, MANAGED_KEYED_VERIFY_FIELD,
    MANAGED_LIST_LAYOUT_FIELD, MANAGED_MAP_READ_FIELD, MANAGED_OBJECT_TYPE_FIELD,
    MANAGED_POINTER_SIZE_FIELD, MANAGED_SET_READ_FIELD, PROVIDER_BINDINGS_TYPE,
    PROVIDER_PREPARATION_FUNCTION, RESERVED_FUNCTION_PREFIX, augment_program_with_library_bodies,
    managed_class_address_name, managed_field_offset_name, managed_field_presence_name,
    managed_instance_header_name, managed_static_field_address_name, provider_context_field_name,
};

use std::{collections::HashSet, sync::Arc};

use graph::{StandardLibraryGraph, default_standard_library_graph};

use crate::{catalog::Documentation, intrinsic_registry};

impl TypeRef {
    fn render(self, library: &StandardLibrary) -> String {
        self.render_with(library, &[])
    }

    fn render_with(self, library: &StandardLibrary, substitutions: &[(&str, String)]) -> String {
        match self {
            Self::Core(ty) => ty.to_string(),
            Self::Standard(ty) => library.type_decl(ty).name.to_owned(),
            Self::Parameter(name) => substitutions
                .iter()
                .find_map(|(parameter, ty)| (*parameter == name).then(|| ty.clone()))
                .unwrap_or_else(|| name.to_owned()),
            Self::Associated(name) => name.to_owned(),
            Self::Async(value) => {
                format!("async {}", value.render_with(library, substitutions))
            }
            Self::Iterator(item) => {
                format!("iterator {}", item.render_with(library, substitutions))
            }
            Self::Application {
                constructor,
                arguments,
            } => {
                let rendered = arguments
                    .iter()
                    .map(|argument| argument.render_with(library, substitutions))
                    .collect::<Vec<_>>();
                let declaration = library.type_constructor(constructor);
                let arguments = rendered.join(", ");
                match declaration.syntax {
                    TypeConstructorSyntax::Named => {
                        format!("{}<{arguments}>", declaration.name)
                    }
                    TypeConstructorSyntax::Array => format!("[{arguments}]"),
                    TypeConstructorSyntax::Optional => format!("{arguments}?"),
                    TypeConstructorSyntax::Fallible => format!("{arguments}!"),
                    TypeConstructorSyntax::ExclusiveRange => {
                        format!("{arguments}..<{arguments}")
                    }
                    TypeConstructorSyntax::InclusiveRange => {
                        format!("{arguments}..={arguments}")
                    }
                }
            }
            Self::FixedArray { element, length } => format!(
                "[{}; {length}]",
                element.render_with(library, substitutions)
            ),
            Self::Callable { parameters, result } => format!(
                "({}) -> {}",
                parameters
                    .iter()
                    .map(|parameter| parameter.render_with(library, substitutions))
                    .collect::<Vec<_>>()
                    .join(", "),
                result.render_with(library, substitutions),
            ),
        }
    }
}
#[derive(Debug, Clone)]
pub struct StandardLibrary {
    graph: Arc<StandardLibraryGraph>,
}

impl PartialEq for StandardLibrary {
    fn eq(&self, other: &Self) -> bool {
        Arc::ptr_eq(&self.graph, &other.graph)
    }
}

impl Eq for StandardLibrary {}

impl std::hash::Hash for StandardLibrary {
    fn hash<H: std::hash::Hasher>(&self, state: &mut H) {
        Arc::as_ptr(&self.graph).hash(state);
    }
}

impl Default for StandardLibrary {
    fn default() -> Self {
        Self::new()
    }
}

impl StandardLibrary {
    pub fn new() -> Self {
        let library = Self {
            graph: default_standard_library_graph(),
        };
        library.initialize_source_body_operations();
        library
    }

    #[cfg(test)]
    pub(crate) fn isolated_bundled() -> Self {
        let library = Self {
            graph: Arc::new(StandardLibraryGraph::build().unwrap_or_else(|errors| {
                panic!(
                    "the isolated bundled standard-library graph is invalid:\n{}",
                    errors.join("\n")
                )
            })),
        };
        library.initialize_source_body_operations();
        library
    }

    fn initialize_source_body_operations(&self) {
        self.graph.initialize_source_body_operations_with(|| {
            let operations =
                crate::derive_standard_library_operation_metadata(self.clone()).unwrap_or_else(
                    |diagnostics| {
                        panic!(
                            "source-defined standard-library operation analysis failed:\n{diagnostics:#?}"
                        )
                    },
                );
            for item in self
                .all_items()
                .iter()
                .filter(|item| item.kind == ItemKind::Constant)
            {
                let operation = operations
                    .get(&item.id)
                    .expect("source-defined constants have operation metadata");
                if operation.availability != Availability::Everywhere
                    || operation.effects != EffectSet::one(Effect::Pure)
                {
                    panic!(
                        "standard-library constant `{}` is not a pure, synchronous, context-independent value",
                        item.qualified_name
                    )
                }
            }
            operations
        });
    }

    fn rendered_library_bodies(&self) -> &library_bodies::RenderedLibraryBodies {
        self.graph
            .rendered_library_bodies
            .get_or_init(|| library_bodies::render_library_bodies(self))
    }

    pub(crate) fn source_body_operations_are_initialized(&self) -> bool {
        self.graph.source_body_operations_are_initialized()
    }

    pub fn core_types(&self) -> &'static [CoreType] {
        CORE_TYPES
    }

    pub fn state_providers(&self) -> &'static [StdlibStateProvider] {
        STATE_PROVIDERS
    }

    pub fn state_provider(&self, id: StdlibStateProviderId) -> &'static StdlibStateProvider {
        self.graph
            .state_providers
            .get(&id)
            .copied()
            .expect("every standard-library state provider ID must have a declaration")
    }

    pub fn state_provider_by_name(&self, name: &str) -> Option<&'static StdlibStateProvider> {
        self.graph.state_providers_by_name.get(name).copied()
    }

    pub fn default_state_provider(&self) -> Option<&'static StdlibStateProvider> {
        self.state_providers()
            .iter()
            .find(|provider| provider.default)
    }

    /// Returns the compiler-owned preparation callable selected by one state
    /// declaration. A configured selector replaces the provider's automatic
    /// preparation; providers without either hook need no preparation phase.
    pub(crate) fn state_provider_preparation(
        &self,
        provider: StdlibStateProviderId,
        selector: Option<usize>,
    ) -> Option<StdlibItemId> {
        let provider = self.state_provider(provider);
        selector
            .map(|selector| {
                provider
                    .selectors
                    .get(selector)
                    .expect("resolved provider selector belongs to its provider")
                    .preparation
            })
            .or(provider.preparation)
    }

    pub fn core_type(&self, id: CoreTypeId) -> &'static CoreType {
        self.graph
            .core_types
            .get(&id)
            .copied()
            .expect("every core type ID must have a declaration")
    }

    pub fn core_type_has_capability(&self, ty: CoreTypeId, capability: StdlibCapabilityId) -> bool {
        self.capabilities_satisfy(self.core_type(ty).capabilities, capability)
    }

    pub fn capabilities(&self) -> &'static [StdlibCapability] {
        CAPABILITIES
    }

    pub fn capability(&self, id: StdlibCapabilityId) -> &'static StdlibCapability {
        self.graph
            .capabilities
            .get(&id)
            .copied()
            .expect("every standard-library capability ID must have a declaration")
    }

    /// Finds the capability in `capability`'s hierarchy that declares an
    /// associated type visible under `name`.
    ///
    /// Catalog validation rejects ambiguous inherited names, so a successful
    /// lookup always identifies one semantic owner even through a diamond.
    pub fn associated_type_owner(
        &self,
        capability: StdlibCapabilityId,
        name: &str,
    ) -> Option<StdlibCapabilityId> {
        self.capability_associated_types(capability)
            .into_iter()
            .find_map(|(owner, associated)| (associated.name == name).then_some(owner))
    }

    /// Returns all associated types visible through a capability, preserving
    /// the declaration-first hierarchy order and reporting each declaring
    /// capability only once through diamonds.
    pub fn capability_associated_types(
        &self,
        capability: StdlibCapabilityId,
    ) -> Vec<(StdlibCapabilityId, &'static StdlibAssociatedType)> {
        fn collect(
            library: &StandardLibrary,
            capability: StdlibCapabilityId,
            visited: &mut Vec<StdlibCapabilityId>,
            output: &mut Vec<(StdlibCapabilityId, &'static StdlibAssociatedType)>,
        ) {
            if visited.contains(&capability) {
                return;
            }
            visited.push(capability);
            let declaration = library.capability(capability);
            output.extend(
                declaration
                    .associated_types
                    .iter()
                    .map(|associated| (capability, associated)),
            );
            for super_capability in declaration.super_capabilities {
                collect(library, *super_capability, visited, output);
            }
        }

        let mut output = Vec::new();
        collect(self, capability, &mut Vec::new(), &mut output);
        output
    }

    /// Whether providing `capability` also provides `required`, following the
    /// hierarchy authored in the standard-library source.
    pub fn capability_implies(
        &self,
        capability: StdlibCapabilityId,
        required: StdlibCapabilityId,
    ) -> bool {
        capability == required || self.graph.implied_capabilities[&capability].contains(&required)
    }

    /// Whether every concrete runtime value satisfies `required` through the
    /// compiler-provided opaque `Debug` representation (and its super
    /// capabilities).
    pub fn has_universal_debug_fallback(&self, required: StdlibCapabilityId) -> bool {
        self.capability_implies(StdlibCapabilityId::Debug, required)
    }

    /// Whether any directly provided capability transitively provides the
    /// requested capability.
    pub fn capabilities_satisfy(
        &self,
        capabilities: &[StdlibCapabilityId],
        required: StdlibCapabilityId,
    ) -> bool {
        capabilities
            .iter()
            .any(|capability| self.capability_implies(*capability, required))
    }

    /// Removes duplicate and transitively implied constraints while preserving
    /// the declaration order of the strongest remaining capabilities.
    pub fn minimal_capabilities(
        &self,
        capabilities: &[StdlibCapabilityId],
    ) -> Vec<StdlibCapabilityId> {
        let mut unique = Vec::new();
        for capability in capabilities {
            if !unique.contains(capability) {
                unique.push(*capability);
            }
        }
        unique
            .iter()
            .copied()
            .filter(|candidate| {
                !unique
                    .iter()
                    .any(|other| other != candidate && self.capability_implies(*other, *candidate))
            })
            .collect()
    }

    pub fn type_constructors(&self) -> &'static [StdlibTypeConstructor] {
        TYPE_CONSTRUCTORS
    }

    pub fn public_type_constructors(&self) -> impl Iterator<Item = &'static StdlibTypeConstructor> {
        self.type_constructors()
            .iter()
            .filter(|constructor| constructor.public)
    }

    pub fn type_constructor(&self, id: StdlibTypeConstructorId) -> &'static StdlibTypeConstructor {
        self.graph
            .type_constructors
            .get(&id)
            .copied()
            .expect("every standard-library type-constructor ID must have a declaration")
    }

    pub fn type_constructor_has_capability(
        &self,
        constructor: StdlibTypeConstructorId,
        capability: StdlibCapabilityId,
    ) -> bool {
        self.capabilities_satisfy(self.type_constructor(constructor).capabilities, capability)
    }

    /// Looks up constructors that are actually written as source identifiers.
    /// Structural forms such as `[T]`, `T?`, and `T!` deliberately have no
    /// identifier lookup path.
    pub fn named_type_constructor_by_name(
        &self,
        name: &str,
    ) -> Option<&'static StdlibTypeConstructor> {
        self.public_type_constructors().find(|constructor| {
            constructor.syntax == TypeConstructorSyntax::Named && constructor.name == name
        })
    }

    pub(crate) fn named_type_constructor_by_name_including_private(
        &self,
        name: &str,
    ) -> Option<&'static StdlibTypeConstructor> {
        self.type_constructors().iter().find(|constructor| {
            constructor.syntax == TypeConstructorSyntax::Named && constructor.name == name
        })
    }

    pub fn render_type_constructor(&self, id: StdlibTypeConstructorId) -> String {
        let constructor = self.type_constructor(id);
        let parameters = constructor
            .parameters
            .iter()
            .map(|parameter| parameter.name)
            .collect::<Vec<_>>()
            .join(", ");
        match constructor.syntax {
            TypeConstructorSyntax::Named => format!("{}<{parameters}>", constructor.name),
            TypeConstructorSyntax::Array => format!("[{parameters}]"),
            TypeConstructorSyntax::Optional => format!("{parameters}?"),
            TypeConstructorSyntax::Fallible => format!("{parameters}!"),
            TypeConstructorSyntax::ExclusiveRange => format!("{parameters}..<{parameters}"),
            TypeConstructorSyntax::InclusiveRange => format!("{parameters}..={parameters}"),
        }
    }

    pub fn render_field_owner(&self, owner: StdlibOwner) -> String {
        match owner {
            StdlibOwner::Type(owner) => self.type_decl(owner).name.to_owned(),
            StdlibOwner::TypeConstructor(owner) => self.render_type_constructor(owner),
            _ => unreachable!("standard-library fields belong to types or type constructors"),
        }
    }

    pub fn namespaces(&self) -> &'static [StdlibNamespace] {
        NAMESPACES
    }

    pub fn namespace(&self, id: StdlibNamespaceId) -> &'static StdlibNamespace {
        self.graph
            .namespaces
            .get(&id)
            .copied()
            .expect("every standard-library namespace ID must have a declaration")
    }

    pub fn namespace_by_name(&self, name: &str) -> Option<&'static StdlibNamespace> {
        self.graph.namespaces_by_name.get(name).copied()
    }

    pub fn namespace_by_path(&self, path: &[&str]) -> Option<&'static StdlibNamespace> {
        self.graph.namespaces_by_path.get(path).copied()
    }

    pub fn types(&self) -> impl Iterator<Item = &'static StdlibType> {
        TYPES
            .iter()
            .filter(|ty| ty.visibility == TypeVisibility::Public)
    }

    pub(crate) fn all_types(&self) -> &'static [StdlibType] {
        TYPES
    }

    pub fn type_decl(&self, id: StdlibTypeId) -> &'static StdlibType {
        self.graph
            .types
            .get(&id)
            .copied()
            .expect("every standard-library type ID must have a declaration")
    }

    pub fn type_by_name(&self, name: &str) -> Option<&'static StdlibType> {
        self.graph.types_by_name.get(name).copied()
    }

    pub(crate) fn type_by_name_including_private(&self, name: &str) -> Option<&'static StdlibType> {
        self.graph.all_types_by_name.get(name).copied()
    }

    pub fn type_has_capability(&self, ty: StdlibTypeId, capability: StdlibCapabilityId) -> bool {
        self.capabilities_satisfy(self.type_decl(ty).capabilities, capability)
    }

    /// Returns the catalog-owned conversion used when a standard type
    /// implements `Display` with its own source or intrinsic body.
    pub fn display_implementation(&self, ty: StdlibTypeId) -> Option<&'static StdlibItem> {
        self.type_decl(ty).display.map(|item| self.item(item))
    }

    /// Returns every catalog method bound to the requested binary syntax.
    /// Applicability to a concrete receiver is decided by the semantic adapter.
    pub fn binary_operator_items(
        &self,
        operator: StandardBinaryOperator,
    ) -> impl Iterator<Item = &'static StdlibItem> + '_ {
        self.graph
            .binary_operators
            .get(&operator)
            .into_iter()
            .flat_map(|items| items.iter().copied())
    }

    /// Returns every catalog method bound to the requested unary syntax.
    pub fn unary_operator_items(
        &self,
        operator: StandardUnaryOperator,
    ) -> impl Iterator<Item = &'static StdlibItem> + '_ {
        self.graph
            .unary_operators
            .get(&operator)
            .into_iter()
            .flat_map(|items| items.iter().copied())
    }

    pub fn render_declared_type(&self, ty: DeclaredTypeRef) -> &'static str {
        match ty {
            DeclaredTypeRef::Core(core) => self.core_type(core).name,
            DeclaredTypeRef::Standard(standard) => self.type_decl(standard).name,
        }
    }

    pub fn render_type(&self, ty: TypeRef) -> String {
        ty.render(self)
    }

    pub fn fields(&self) -> &'static [StdlibField] {
        FIELDS
    }

    pub fn field(&self, id: StdlibFieldId) -> &'static StdlibField {
        self.graph
            .fields
            .get(&id)
            .copied()
            .expect("every standard-library field ID must have a declaration")
    }

    pub fn fields_of(&self, owner: StdlibTypeId) -> impl Iterator<Item = &'static StdlibField> {
        self.fields_of_owner(StdlibOwner::Type(owner))
    }

    pub fn fields_of_constructor(
        &self,
        owner: StdlibTypeConstructorId,
    ) -> impl Iterator<Item = &'static StdlibField> {
        self.fields_of_owner(StdlibOwner::TypeConstructor(owner))
    }

    pub fn fields_of_owner(
        &self,
        owner: StdlibOwner,
    ) -> impl Iterator<Item = &'static StdlibField> {
        self.graph
            .fields_by_owner
            .get(&owner)
            .into_iter()
            .flat_map(|fields| fields.iter().copied())
    }

    pub fn public_field(&self, owner: StdlibTypeId, name: &str) -> Option<&'static StdlibField> {
        self.public_field_of_owner(StdlibOwner::Type(owner), name)
    }

    pub fn public_constructor_field(
        &self,
        owner: StdlibTypeConstructorId,
        name: &str,
    ) -> Option<&'static StdlibField> {
        self.public_field_of_owner(StdlibOwner::TypeConstructor(owner), name)
    }

    fn public_field_of_owner(
        &self,
        owner: StdlibOwner,
        name: &str,
    ) -> Option<&'static StdlibField> {
        self.graph.public_fields.get(&(owner, name)).copied()
    }

    pub fn public_fields(&self, owner: StdlibTypeId) -> impl Iterator<Item = &'static StdlibField> {
        let owner_is_public = self.type_decl(owner).visibility == TypeVisibility::Public;
        self.fields_of(owner)
            .filter(move |field| owner_is_public && field.visibility == FieldVisibility::Public)
    }

    pub fn public_constructor_fields(
        &self,
        owner: StdlibTypeConstructorId,
    ) -> impl Iterator<Item = &'static StdlibField> {
        self.fields_of_constructor(owner)
            .filter(|field| field.visibility == FieldVisibility::Public)
    }

    pub fn variants(&self) -> &'static [StdlibVariant] {
        VARIANTS
    }

    pub fn public_variants(&self) -> impl Iterator<Item = &'static StdlibVariant> {
        VARIANTS
            .iter()
            .filter(|variant| self.type_decl(variant.owner).visibility == TypeVisibility::Public)
    }

    pub fn variant(&self, id: StdlibVariantId) -> &'static StdlibVariant {
        self.graph
            .variants
            .get(&id)
            .copied()
            .expect("every standard-library variant ID must have a declaration")
    }

    pub fn variants_of(&self, owner: StdlibTypeId) -> impl Iterator<Item = &'static StdlibVariant> {
        self.graph
            .variants_by_owner
            .get(&owner)
            .into_iter()
            .flat_map(|variants| variants.iter().copied())
    }

    pub fn items(&self) -> impl Iterator<Item = &'static StdlibItem> {
        ITEMS
            .iter()
            .filter(|item| item.visibility == ItemVisibility::Public)
    }

    pub(crate) fn all_items(&self) -> &'static [StdlibItem] {
        ITEMS
    }

    pub fn methods(&self) -> impl Iterator<Item = &'static StdlibItem> {
        self.graph.methods.iter().copied()
    }

    pub fn method_items_named(&self, name: &str) -> impl Iterator<Item = &'static StdlibItem> + '_ {
        self.graph
            .methods_by_name
            .get(name)
            .into_iter()
            .flat_map(|items| items.iter().copied())
    }

    pub fn item(&self, id: StdlibItemId) -> &'static StdlibItem {
        self.graph
            .items
            .get(&id)
            .copied()
            .expect("every standard-library ID must have a catalog entry")
    }

    /// Returns the use obligation for a callable's result.
    ///
    /// Catalog declarations can provide a more specific explanation, but the
    /// default follows the language-wide value semantics: discarding a
    /// returned value is a mistake unless the operation mutates its receiver.
    pub fn must_use(&self, id: StdlibItemId) -> Option<&'static str> {
        const VALUE_MUST_BE_USED: &str =
            "This operation only produces a value; use or explicitly store its returned value.";

        let item = self.item(id);
        item.must_use.or_else(|| {
            (item.signature.result != TypeRef::Core(CoreTypeId::None)
                && !self.effects(id).contains(&Effect::MutatesValue))
            .then_some(VALUE_MUST_BE_USED)
        })
    }

    /// Returns the authoritative effects and availability for a catalog item.
    /// Intrinsics use their trusted declarations; source bodies use the
    /// compiler-derived result cached in this library graph.
    pub fn operation_metadata(&self, id: StdlibItemId) -> OperationMetadata {
        let item = self.item(id);
        match item.implementation {
            Implementation::CapabilityRequirement => OperationMetadata {
                effects: EffectSet::one(Effect::Pure),
                availability: Availability::Everywhere,
            },
            Implementation::Intrinsic(intrinsic) => {
                let contract = intrinsic_registry::contract(intrinsic);
                let declared = item
                    .intrinsic_context
                    .expect("intrinsic items declare privileged context metadata");
                let effects = [
                    Effect::RequiresAttachedProcess,
                    Effect::RequiresStateSnapshots,
                    Effect::Suspends,
                    Effect::CancelsOnProcessClose,
                ]
                .into_iter()
                .fold(contract.effects, EffectSet::without);
                let effects = if item.signature.result_is_async {
                    effects.with(Effect::Suspends)
                } else {
                    effects
                };
                OperationMetadata {
                    effects: declared
                        .effects()
                        .iter()
                        .fold(effects, |effects, effect| effects.with(*effect)),
                    availability: declared.availability,
                }
            }
            Implementation::LibraryBody { .. } | Implementation::LibraryOverloads { .. } => self
                .graph
                .source_body_operation(id)
                // During one-time bootstrap, calls to other source bodies are
                // followed through their injected function identities by the
                // ordinary operation analysis. A direct metadata query only
                // needs a neutral provisional value.
                .unwrap_or(OperationMetadata {
                    effects: EffectSet::one(Effect::Pure),
                    availability: Availability::Everywhere,
                }),
        }
    }

    pub fn effects(&self, id: StdlibItemId) -> EffectSet {
        self.operation_metadata(id).effects
    }

    pub fn operation_semantics(&self, id: StdlibItemId) -> OperationSemantics {
        self.operation_metadata(id).semantics()
    }

    pub fn item_by_name(&self, qualified_name: &str) -> Option<&'static StdlibItem> {
        self.graph.items_by_name.get(qualified_name).copied()
    }

    pub(crate) fn item_by_name_including_private(
        &self,
        qualified_name: &str,
    ) -> Option<&'static StdlibItem> {
        self.graph.all_items_by_name.get(qualified_name).copied()
    }

    pub(crate) fn source_body_item_by_function_name(
        &self,
        function_name: &str,
    ) -> Option<&'static StdlibItem> {
        self.graph
            .source_body_items_by_function_name
            .get(function_name)
            .copied()
    }

    pub(crate) fn method_items_named_including_private(
        &self,
        name: &str,
    ) -> impl Iterator<Item = &'static StdlibItem> + '_ {
        self.graph
            .all_methods_by_name
            .get(name)
            .into_iter()
            .flat_map(|items| items.iter().copied())
    }

    pub fn children_of(&self, owner: StdlibOwner) -> impl Iterator<Item = StdlibSymbolId> + '_ {
        self.graph
            .children_by_owner
            .get(&owner)
            .into_iter()
            .flat_map(|children| children.iter().copied())
    }

    pub fn item_path(&self, item: &StdlibItem) -> Option<Vec<&'static str>> {
        self.graph.item_path(item)
    }

    /// Public items whose complete source path has exactly this parent prefix.
    /// Includes static functions, methods, and constants in catalog order.
    pub(crate) fn items_with_path_prefix<'a>(
        &'a self,
        prefix: &[&'a str],
    ) -> impl Iterator<Item = &'static StdlibItem> + 'a {
        self.graph
            .items_by_path_prefix
            .get(prefix)
            .into_iter()
            .flat_map(|items| items.iter().copied())
    }

    pub fn render_signature(&self, id: StdlibItemId) -> String {
        self.render_signature_with(id, &[])
    }

    /// Renders a catalog signature after replacing named type parameters with
    /// semantic types inferred at one call site.
    pub fn render_signature_with(
        &self,
        id: StdlibItemId,
        substitutions: &[(&str, String)],
    ) -> String {
        let item = self.item(id);
        let signature = item.signature;
        let mut rendered = match item.kind {
            ItemKind::Function => item.qualified_name.to_owned(),
            ItemKind::Method { receiver } => format!(
                "{}.{}",
                receiver.render_with(self, substitutions),
                item.name
            ),
            ItemKind::Constant => {
                return format!(
                    "{}: {}",
                    item.qualified_name,
                    signature.result.render_with(self, substitutions)
                );
            }
        };
        if signature.explicit_type_parameters != 0 {
            rendered.push('<');
            for (index, parameter) in signature
                .type_parameters
                .iter()
                .take(signature.explicit_type_parameters)
                .enumerate()
            {
                if index != 0 {
                    rendered.push_str(", ");
                }
                rendered.push_str(
                    substitutions
                        .iter()
                        .find(|(name, _)| *name == parameter.name)
                        .map_or(parameter.name, |(_, replacement)| replacement),
                );
            }
            rendered.push('>');
        }
        rendered.push('(');
        for (index, parameter) in signature.parameters.iter().enumerate() {
            if index != 0 {
                rendered.push_str(", ");
            }
            rendered.push_str(parameter.name);
            rendered.push_str(": ");
            rendered.push_str(&parameter.ty.render_with(self, substitutions));
        }
        rendered.push_str(") -> ");
        if signature.result_is_async {
            rendered.push_str("async ");
        }
        rendered.push_str(&signature.result.render_with(self, substitutions));
        let unresolved = signature
            .type_parameters
            .iter()
            .filter(|parameter| {
                !parameter.constraints.is_empty()
                    && !substitutions
                        .iter()
                        .any(|(name, _)| *name == parameter.name)
            })
            .collect::<Vec<_>>();
        if !unresolved.is_empty() {
            rendered.push_str(" where ");
            for (index, parameter) in unresolved.into_iter().enumerate() {
                if index != 0 {
                    rendered.push_str(", ");
                }
                rendered.push_str(parameter.name);
                if !parameter.constraints.is_empty() {
                    rendered.push_str(": ");
                }
                for (constraint_index, constraint) in self
                    .minimal_capabilities(parameter.constraints)
                    .iter()
                    .enumerate()
                {
                    if constraint_index != 0 {
                        rendered.push_str(" + ");
                    }
                    rendered.push_str(self.capability(*constraint).name);
                }
            }
        }
        rendered
    }

    pub fn render_operation_semantics(&self, id: StdlibItemId) -> String {
        let semantics = self.operation_semantics(id);
        let mut facts = vec![match (
            semantics.availability,
            semantics.requires_state_snapshots,
            semantics.requires_attached_process,
        ) {
            (Availability::OnAttach, _, _) => "available in suspending attachment code",
            (Availability::Everywhere, true, _) => {
                "available after old and current state snapshots are initialized"
            }
            (Availability::Everywhere, false, true) => {
                "available while a process is attached, except in onDetach"
            }
            (Availability::Everywhere, false, false) => "available everywhere",
        }];
        facts.push(match semantics.suspension {
            SuspensionKind::None => "synchronous",
            SuspensionKind::Suspends => "suspends",
        });
        if semantics.cancellation == CancellationKind::ProcessClose {
            facts.push("cancels when the process closes");
        }
        facts.join("; ")
    }

    pub fn validate(&self) -> Vec<String> {
        let mut errors = validation::validate(
            CAPABILITIES,
            NAMESPACES,
            TYPE_CONSTRUCTORS,
            TYPES,
            FIELDS,
            VARIANTS,
        );
        validate_named_declarations(
            "capability",
            CAPABILITIES,
            |value| (value.id, value.name, value.documentation),
            &mut errors,
        );
        validate_capability_hierarchy(CAPABILITIES, &mut errors);
        validate_named_declarations(
            "type constructor",
            TYPE_CONSTRUCTORS,
            |value| (value.id, value.name, value.documentation),
            &mut errors,
        );
        for constructor in TYPE_CONSTRUCTORS {
            let mut parameters = HashSet::new();
            for parameter in constructor.parameters {
                if parameter.name.trim().is_empty() {
                    errors.push(format!(
                        "type constructor `{}` has an empty parameter name",
                        constructor.name
                    ));
                } else if !parameters.insert(parameter.name) {
                    errors.push(format!(
                        "type constructor `{}` repeats parameter `{}`",
                        constructor.name, parameter.name
                    ));
                }
            }
        }
        let mut ids = HashSet::new();
        let mut intrinsics = HashSet::new();
        let mut qualified_names = HashSet::new();
        let mut call_shapes = HashSet::new();
        let mut example_sources = HashSet::new();
        for namespace in NAMESPACES {
            struct_example_sources(
                "namespace",
                namespace.name,
                namespace.documentation,
                &mut example_sources,
                &mut errors,
            );
        }
        for capability in CAPABILITIES {
            struct_example_sources(
                "capability",
                capability.name,
                capability.documentation,
                &mut example_sources,
                &mut errors,
            );
        }
        for constructor in TYPE_CONSTRUCTORS {
            struct_example_sources(
                "type constructor",
                constructor.name,
                constructor.documentation,
                &mut example_sources,
                &mut errors,
            );
        }
        for ty in TYPES
            .iter()
            .filter(|ty| ty.visibility == TypeVisibility::Public)
        {
            struct_example_sources(
                "type",
                ty.name,
                ty.documentation,
                &mut example_sources,
                &mut errors,
            );
        }
        for field in FIELDS.iter().filter(|field| {
            field.visibility == FieldVisibility::Public
                && match field.owner {
                    StdlibOwner::Type(owner) => {
                        self.type_decl(owner).visibility == TypeVisibility::Public
                    }
                    StdlibOwner::TypeConstructor(_) => true,
                    _ => false,
                }
        }) {
            struct_example_sources(
                "field",
                field.name,
                field.documentation,
                &mut example_sources,
                &mut errors,
            );
        }
        for variant in self.public_variants() {
            struct_example_sources(
                "variant",
                variant.name,
                variant.documentation,
                &mut example_sources,
                &mut errors,
            );
        }
        let mut provider_names = HashSet::new();
        let mut default_state_provider = None;
        for provider in STATE_PROVIDERS {
            if provider.name.trim().is_empty() {
                errors.push("state provider has an empty name".to_owned());
            } else if !provider_names.insert(provider.name) {
                errors.push(format!("duplicate state provider `{}`", provider.name));
            }
            if provider.value_name.trim().is_empty() {
                errors.push(format!(
                    "state provider `{}` has an empty value name",
                    provider.name
                ));
            }
            if !TYPES.iter().any(|ty| ty.id == provider.process_type) {
                errors.push(format!(
                    "state provider `{}` references unknown process type `{:?}`",
                    provider.name, provider.process_type
                ));
            }
            if matches!(provider.processes, StateProviderProcesses::Declared(processes) if processes.is_empty())
            {
                errors.push(format!(
                    "state provider `{}` declares no process names",
                    provider.name
                ));
            }
            if provider.default {
                if provider.processes != StateProviderProcesses::SourceState {
                    errors.push(format!(
                        "default state provider `{}` does not use source process names",
                        provider.name
                    ));
                }
                if default_state_provider.replace(provider.name).is_some() {
                    errors.push("multiple default state providers are declared".to_owned());
                }
            }
            let mut context_names = HashSet::new();
            for context in provider.contexts {
                let qualified = format!("{}.{}", provider.name, context.name);
                if context.name.trim().is_empty() {
                    errors.push(format!(
                        "state provider `{}` has an empty context value name",
                        provider.name
                    ));
                } else if context.name == provider.value_name {
                    errors.push(format!(
                        "state provider `{}` uses `{}` for both its primary and context value",
                        provider.name, context.name
                    ));
                } else if !context_names.insert(context.name) {
                    errors.push(format!(
                        "state provider `{}` repeats context value `{}`",
                        provider.name, context.name
                    ));
                }
                if context.documentation.summary.trim().is_empty()
                    || context.documentation.details.trim().is_empty()
                    || context.documentation.examples.is_empty()
                {
                    errors.push(format!(
                        "state-provider context `{qualified}` has incomplete documentation"
                    ));
                }
                struct_example_sources(
                    "state-provider context",
                    &qualified,
                    context.documentation,
                    &mut example_sources,
                    &mut errors,
                );
                if self.type_decl(context.ty).visibility != TypeVisibility::Public {
                    errors.push(format!(
                        "state-provider context `{qualified}` exposes private type `{}`",
                        self.type_decl(context.ty).name
                    ));
                }
                let preparation = self.item(context.preparation);
                if preparation.kind != ItemKind::Function
                    || !preparation.signature.type_parameters.is_empty()
                    || !preparation.signature.parameters.is_empty()
                    || !preparation.signature.result_is_async
                    || preparation.signature.result != TypeRef::Standard(context.ty)
                    || !matches!(
                        preparation.implementation,
                        Implementation::LibraryBody { .. }
                    )
                {
                    errors.push(format!(
                        "state-provider context `{qualified}` has incompatible preparation callable `{:?}`",
                        context.preparation
                    ));
                }
                if self.source_body_operations_are_initialized() {
                    let operation = self.operation_metadata(context.preparation);
                    if !operation.effects.contains(&Effect::Suspends)
                        || !operation.effects.contains(&Effect::RequiresAttachedProcess)
                    {
                        errors.push(format!(
                            "state-provider context `{qualified}` preparation `{}` must suspend and require an attached process",
                            preparation.qualified_name
                        ));
                    }
                }
            }
            let mut selector_names = HashSet::new();
            for selector in provider.selectors {
                let qualified = format!("{}.{}", provider.name, selector.name);
                if selector.name.trim().is_empty() {
                    errors.push(format!(
                        "state provider `{}` has an empty selector name",
                        provider.name
                    ));
                } else if !selector_names.insert(selector.name) {
                    errors.push(format!(
                        "state provider `{}` repeats selector `{}`",
                        provider.name, selector.name
                    ));
                }
                if selector.documentation.summary.trim().is_empty()
                    || selector.documentation.details.trim().is_empty()
                {
                    errors.push(format!(
                        "state-provider selector `{qualified}` has incomplete documentation"
                    ));
                }
                let mut parameter_names = HashSet::new();
                for parameter in selector.parameters {
                    if parameter.name.trim().is_empty() {
                        errors.push(format!(
                            "state-provider selector `{qualified}` has an empty parameter name"
                        ));
                    } else if !parameter_names.insert(parameter.name) {
                        errors.push(format!(
                            "state-provider selector `{qualified}` repeats parameter `{}`",
                            parameter.name
                        ));
                    }
                    validate_catalog_type_ref(parameter.ty, &[], &qualified, &mut errors);
                }
            }
            let direct_read = ITEMS.iter().find(|item| item.id == provider.direct_read);
            match direct_read {
                Some(item)
                    if item.owner == StdlibOwner::Type(provider.process_type)
                        && matches!(
                            item.kind,
                            ItemKind::Method {
                                receiver: TypeRef::Standard(receiver)
                            } if receiver == provider.process_type
                        )
                        && item.signature.parameters.len() == 1
                        && matches!(
                            item.signature.parameters[0].ty,
                            TypeRef::Core(CoreTypeId::U32 | CoreTypeId::Address)
                        )
                        && item.signature.type_parameters.len() == 1
                        && item.signature.type_parameters[0]
                            .constraints
                            .contains(&StdlibCapabilityId::MemoryReadable)
                        && matches!(
                            item.signature.result,
                            TypeRef::Application {
                                constructor: StdlibTypeConstructorId::Result,
                                arguments: [TypeRef::Parameter(_)]
                            }
                        ) => {}
                Some(item) => errors.push(format!(
                    "state provider `{}` has incompatible direct-read operation `{}`",
                    provider.name, item.qualified_name
                )),
                None => errors.push(format!(
                    "state provider `{}` references missing direct-read item `{:?}`",
                    provider.name, provider.direct_read
                )),
            }
            if provider.documentation.summary.trim().is_empty()
                || provider.documentation.details.trim().is_empty()
            {
                errors.push(format!(
                    "state provider `{}` has incomplete documentation",
                    provider.name
                ));
            }
            if provider.documentation.examples.is_empty() {
                errors.push(format!(
                    "state provider `{}` has no examples",
                    provider.name
                ));
            }
            for example in provider.documentation.examples {
                if example.title.trim().is_empty()
                    || example.source.trim().is_empty()
                    || !example.has_validation_source()
                    || !example.validation_includes_source()
                {
                    errors.push(format!(
                        "state provider `{}` has an incomplete example",
                        provider.name
                    ));
                }
                if !example.source.contains(provider.value_name)
                    && !example.source.contains(&format!("state {}", provider.name))
                {
                    errors.push(format!(
                        "example for state provider `{}` demonstrates neither `state {}` nor `{}`",
                        provider.name, provider.name, provider.value_name
                    ));
                }
                if !example_sources.insert(example.source) {
                    errors.push(format!(
                        "state provider `{}` reuses another symbol's visible example",
                        provider.name
                    ));
                }
            }
            match provider.attachment {
                StateProviderAttachment::Identity => {
                    if !matches!(
                        self.type_decl(provider.process_type).representation,
                        RuntimeRepresentation::Scalar {
                            storage: CoreTypeId::I64
                        }
                    ) {
                        errors.push(format!(
                            "identity state provider `{}` must expose an i64 scalar handle",
                            provider.name
                        ));
                    }
                    if provider.validation.is_some() {
                        errors.push(format!(
                            "identity state provider `{}` cannot declare mapping validation",
                            provider.name
                        ));
                    }
                }
                StateProviderAttachment::Callable(attachment_id) => {
                    let attachment = self.item(attachment_id);
                    let direct_result =
                        attachment.signature.result == TypeRef::Standard(provider.process_type);
                    if attachment.kind != ItemKind::Function
                        || !attachment.signature.type_parameters.is_empty()
                        || !attachment.signature.parameters.is_empty()
                        || !direct_result
                        || !matches!(
                            attachment.implementation,
                            Implementation::LibraryBody { .. }
                        )
                    {
                        errors.push(format!(
                            "state provider `{}` has incompatible attachment callable `{:?}`",
                            provider.name, attachment_id
                        ));
                    }
                    if self.source_body_operations_are_initialized() {
                        let operation = self.operation_metadata(attachment_id);
                        if !operation.effects.contains(&Effect::Suspends)
                            || !operation.effects.contains(&Effect::RequiresAttachedProcess)
                        {
                            errors.push(format!(
                                "state provider `{}` attachment `{}` must suspend and require an attached process",
                                provider.name, attachment.qualified_name
                            ));
                        }
                    }
                }
            }

            if let Some(validation_id) = provider.validation {
                let validation = self.item(validation_id);
                if validation.owner != StdlibOwner::Type(provider.process_type)
                    || !matches!(
                        validation.kind,
                        ItemKind::Method {
                            receiver: TypeRef::Standard(receiver)
                        } if receiver == provider.process_type
                    )
                    || !validation.signature.type_parameters.is_empty()
                    || !validation.signature.parameters.is_empty()
                    || validation.signature.result_is_async
                    || validation.signature.result != TypeRef::Core(CoreTypeId::Bool)
                    || !matches!(
                        validation.implementation,
                        Implementation::LibraryBody { .. }
                    )
                {
                    errors.push(format!(
                        "state provider `{}` has incompatible mapping validation `{:?}`",
                        provider.name, validation_id
                    ));
                }
                if self.source_body_operations_are_initialized() {
                    let operation = self.operation_metadata(validation_id);
                    if operation.effects.contains(&Effect::Suspends)
                        || !operation.effects.contains(&Effect::RequiresAttachedProcess)
                    {
                        errors.push(format!(
                            "state provider `{}` mapping validation `{}` must be synchronous and require an attached process",
                            provider.name, validation.qualified_name
                        ));
                    }
                }
            }

            let preparations = provider
                .preparation
                .map(|item| {
                    (
                        format!("state provider `{}`", provider.name),
                        item,
                        &[][..],
                        false,
                    )
                })
                .into_iter()
                .chain(provider.selectors.iter().map(|selector| {
                    (
                        format!(
                            "state-provider selector `{}.{}`",
                            provider.name, selector.name
                        ),
                        selector.preparation,
                        selector.parameters,
                        selector.managed_backend.is_some(),
                    )
                }))
                .collect::<Vec<_>>();
            if !provider.selectors.is_empty() && provider.preparation.is_none() {
                errors.push(format!(
                    "configured state provider `{}` has no automatic preparation callable",
                    provider.name
                ));
            }
            let mut preparation_result = None;
            for (owner, preparation_id, expected_parameters, specialized_backend) in preparations {
                let preparation = self.item(preparation_id);
                let parameters_match = preparation.signature.parameters.len()
                    == expected_parameters.len()
                    && preparation
                        .signature
                        .parameters
                        .iter()
                        .zip(expected_parameters)
                        .all(|(actual, expected)| actual.ty == expected.ty);
                if preparation.kind != ItemKind::Function
                    || !preparation.signature.type_parameters.is_empty()
                    || !parameters_match
                    || !preparation.signature.result_is_async
                    || !matches!(
                        preparation.implementation,
                        Implementation::LibraryBody { .. }
                    )
                {
                    errors.push(format!(
                        "{owner} has incompatible preparation callable `{:?}`",
                        preparation_id
                    ));
                }
                if let Some(expected) = preparation_result {
                    if !specialized_backend && expected != preparation.signature.result {
                        errors.push(format!(
                            "{owner} preparation returns a different runtime context type"
                        ));
                    }
                } else {
                    preparation_result = Some(preparation.signature.result);
                }
                if self.source_body_operations_are_initialized() {
                    let operation = self.operation_metadata(preparation_id);
                    if !operation.effects.contains(&Effect::Suspends)
                        || !operation.effects.contains(&Effect::RequiresAttachedProcess)
                    {
                        errors.push(format!(
                            "{owner} preparation `{}` must suspend and require an attached process",
                            preparation.qualified_name
                        ));
                    }
                }
            }
        }
        if default_state_provider.is_none() {
            errors.push("the standard library has no source-state process provider".to_owned());
        }
        for item in ITEMS {
            if !ids.insert(item.id) {
                errors.push(format!("duplicate standard-library ID `{:?}`", item.id));
            }
            match item.implementation {
                Implementation::CapabilityRequirement => {
                    let valid_owner = matches!(
                        item.owner,
                        StdlibOwner::Capability(capability)
                            if self.capability(capability).behavior
                                == CapabilityBehavior::StructuralMethods
                    );
                    if !valid_owner || !matches!(item.kind, ItemKind::Method { .. }) {
                        errors.push(format!(
                            "`{}` is a capability requirement outside a structural-method capability",
                            item.qualified_name
                        ));
                    }
                    if item.intrinsic_context.is_some() {
                        errors.push(format!(
                            "`{}` capability requirement declares intrinsic context",
                            item.qualified_name
                        ));
                    }
                }
                Implementation::Intrinsic(intrinsic) => {
                    if !intrinsics.insert(intrinsic) {
                        errors.push(format!(
                            "intrinsic `{:?}` is bound by more than one standard-library item",
                            intrinsic
                        ));
                    }
                    let contract = intrinsic_registry::contract(intrinsic);
                    if !contract.accepts(item.kind) {
                        errors.push(format!(
                            "`{}` has a callable kind incompatible with intrinsic `{intrinsic:?}`",
                            item.qualified_name
                        ));
                    }
                    if !contract.signature.matches(item.kind, item.signature) {
                        errors.push(format!(
                            "`{}` has a signature incompatible with intrinsic `{intrinsic:?}`",
                            item.qualified_name,
                        ));
                    }
                    if let intrinsic_registry::FailureChannelPolicy::ReuseAsyncArgument(index) =
                        contract.failure_channel
                    {
                        let argument_is_async = item
                            .signature
                            .parameters
                            .get(usize::from(index))
                            .is_some_and(|parameter| matches!(parameter.ty, TypeRef::Async(_)));
                        let result_is_fallible = matches!(
                            item.signature.result,
                            TypeRef::Application {
                                constructor: StdlibTypeConstructorId::Result,
                                ..
                            }
                        );
                        if !item.signature.result_is_async
                            || !argument_is_async
                            || !result_is_fallible
                        {
                            errors.push(format!(
                                "`{}` has failure-channel metadata that requires an async argument and fallible async result",
                                item.qualified_name,
                            ));
                        }
                    }
                    let Some(declared) = item.intrinsic_context else {
                        errors.push(format!(
                            "`{}` has no source-declared intrinsic context metadata",
                            item.qualified_name
                        ));
                        continue;
                    };
                    let contract_is_async =
                        contract.lowering == intrinsic_registry::LoweringClass::Suspension;
                    if item.signature.result_is_async != contract_is_async {
                        errors.push(format!(
                            "`{}` declares result asyncness inconsistently with intrinsic `{intrinsic:?}`",
                            item.qualified_name
                        ));
                    }
                    let contract_operation = contract.effects;
                    if declared.availability != contract.availability
                        || declared.requires_attached_process
                            != contract_operation.contains(&Effect::RequiresAttachedProcess)
                        || declared.requires_state_snapshots
                            != contract_operation.contains(&Effect::RequiresStateSnapshots)
                        || (declared.cancellation == CancellationKind::ProcessClose)
                            != contract_operation.contains(&Effect::CancelsOnProcessClose)
                    {
                        errors.push(format!(
                            "`{}` source-declared context does not match intrinsic `{intrinsic:?}`",
                            item.qualified_name
                        ));
                    }
                    if contract.lowering == intrinsic_registry::LoweringClass::Suspension
                        && !contract.effects.contains(&Effect::Suspends)
                    {
                        errors.push(format!(
                            "intrinsic `{intrinsic:?}` uses suspension lowering without a suspension effect"
                        ));
                    }
                }
                Implementation::LibraryBody {
                    function_name,
                    body,
                } => {
                    if !function_name.starts_with("__splitscript_stdlib_") {
                        errors.push(format!(
                            "`{}` has an invalid generated library-function name",
                            item.qualified_name
                        ));
                    }
                    if body.trim().is_empty() {
                        errors.push(format!(
                            "`{}` has an empty source implementation",
                            item.qualified_name
                        ));
                    }
                    if self.graph.source_body_operation(item.id).is_none() {
                        errors.push(format!(
                            "`{}` has no compiler-derived operation metadata",
                            item.qualified_name
                        ));
                    }
                }
                Implementation::LibraryOverloads {
                    dispatch_parameter,
                    cases,
                } => {
                    if cases.is_empty() {
                        errors.push(format!(
                            "`{}` has no source implementation cases",
                            item.qualified_name
                        ));
                    }
                    for case in cases {
                        if !case.function_name.starts_with("__splitscript_stdlib_") {
                            errors.push(format!(
                                "`{}` has an invalid generated overload-function name",
                                item.qualified_name
                            ));
                        }
                        if case.body.trim().is_empty() {
                            errors.push(format!(
                                "`{}` has an empty overload implementation",
                                item.qualified_name
                            ));
                        }
                    }
                    let Some(parameter) = item.signature.type_parameters.get(dispatch_parameter)
                    else {
                        errors.push(format!(
                            "`{}` dispatches on a missing type parameter",
                            item.qualified_name
                        ));
                        continue;
                    };
                    if parameter.constraints != [StdlibCapabilityId::Numeric] {
                        errors.push(format!(
                            "`{}` capability-directed implementation must expose one `Numeric` dispatch parameter",
                            item.qualified_name
                        ));
                    }
                    for core in self.core_types().iter().filter(|core| {
                        self.core_type_has_capability(core.id, StdlibCapabilityId::Numeric)
                    }) {
                        let matching = cases
                            .iter()
                            .filter(|case| self.core_type_has_capability(core.id, case.capability))
                            .count();
                        if matching != 1 {
                            errors.push(format!(
                                "`{}` has {matching} implementation cases for numeric type `{}`; exactly one is required",
                                item.qualified_name, core.name
                            ));
                        }
                    }
                    if self.graph.source_body_operation(item.id).is_none() {
                        errors.push(format!(
                            "`{}` has no compiler-derived operation metadata",
                            item.qualified_name
                        ));
                    }
                }
            }
            if !qualified_names.insert(item.qualified_name) {
                errors.push(format!(
                    "duplicate standard-library name `{}`",
                    item.qualified_name
                ));
            }
            let path = self.item_path(item);
            let call_shape =
                (item.implementation != Implementation::CapabilityRequirement).then(|| match item
                    .kind
                {
                    ItemKind::Function => format!(
                        "function {}",
                        path.as_ref()
                            .expect("functions have source paths")
                            .join(".")
                    ),
                    ItemKind::Method { receiver } => {
                        format!("method {}.{}", receiver.render(self), item.name)
                    }
                    ItemKind::Constant => format!(
                        "constant {}",
                        path.as_ref()
                            .expect("constants have source paths")
                            .join(".")
                    ),
                });
            if let Some(path) = &path
                && path.join(".") != item.qualified_name
            {
                errors.push(format!(
                    "`{}` disagrees with its declared owner and name `{}`",
                    item.qualified_name,
                    path.join(".")
                ));
            }
            if let Some(call_shape) = call_shape
                && !call_shapes.insert(call_shape.clone())
            {
                errors.push(format!(
                    "duplicate standard-library call shape `{call_shape}`"
                ));
            }
            let mut type_parameters = HashSet::new();
            for parameter in item.signature.type_parameters {
                if !type_parameters.insert(parameter.name) {
                    errors.push(format!(
                        "`{}` repeats type parameter `{}`",
                        item.qualified_name, parameter.name
                    ));
                }
                for constraint in parameter.constraints {
                    if !CAPABILITIES
                        .iter()
                        .any(|candidate| candidate.id == *constraint)
                    {
                        errors.push(format!(
                            "`{}` references unknown capability `{constraint:?}`",
                            item.qualified_name
                        ));
                    }
                }
            }
            if let ItemKind::Method { receiver } = item.kind {
                validate_catalog_type_ref(
                    receiver,
                    item.signature.type_parameters,
                    item.qualified_name,
                    &mut errors,
                );
            }
            for parameter in item.signature.parameters {
                validate_catalog_type_ref(
                    parameter.ty,
                    item.signature.type_parameters,
                    item.qualified_name,
                    &mut errors,
                );
            }
            validate_catalog_type_ref(
                item.signature.result,
                item.signature.type_parameters,
                item.qualified_name,
                &mut errors,
            );
            let public = item.visibility == ItemVisibility::Public;
            if public && item.documentation.summary.trim().is_empty() {
                errors.push(format!(
                    "`{}` has no documentation summary",
                    item.qualified_name
                ));
            }
            if public && item.documentation.details.trim().is_empty() {
                errors.push(format!(
                    "`{}` has no documentation details",
                    item.qualified_name
                ));
            }
            if public && item.documentation.examples.is_empty() {
                errors.push(format!("`{}` has no examples", item.qualified_name));
            }
            let example_call = match item.kind {
                ItemKind::Function => item.qualified_name.to_owned(),
                ItemKind::Method { .. } => format!(".{}", item.name),
                ItemKind::Constant => item.qualified_name.to_owned(),
            };
            for example in item.documentation.examples {
                if example.title.trim().is_empty()
                    || example.source.trim().is_empty()
                    || !example.has_validation_source()
                    || !example.validation_includes_source()
                {
                    errors.push(format!(
                        "`{}` has an incomplete example",
                        item.qualified_name
                    ));
                }
                let demonstrates_operator = item.binary_operator.is_some_and(|operator| {
                    example.source.contains(match operator {
                        StandardBinaryOperator::Add => " + ",
                        StandardBinaryOperator::Subtract => " - ",
                        StandardBinaryOperator::Multiply => " * ",
                        StandardBinaryOperator::Divide => " / ",
                        StandardBinaryOperator::Remainder => " % ",
                        StandardBinaryOperator::BitOr => " | ",
                        StandardBinaryOperator::BitXor => " ^ ",
                        StandardBinaryOperator::BitAnd => " & ",
                        StandardBinaryOperator::ShiftLeft => " << ",
                        StandardBinaryOperator::ShiftRight => " >> ",
                        StandardBinaryOperator::Equal => " == ",
                        StandardBinaryOperator::NotEqual => " != ",
                        StandardBinaryOperator::LessThan => " < ",
                        StandardBinaryOperator::LessThanOrEqual => " <= ",
                        StandardBinaryOperator::GreaterThan => " > ",
                        StandardBinaryOperator::GreaterThanOrEqual => " >= ",
                    })
                }) || item.unary_operator.is_some_and(|operator| {
                    example.source.contains(match operator {
                        StandardUnaryOperator::Not => "!",
                        StandardUnaryOperator::Negate => "-",
                    })
                });
                if !example.source.contains(&example_call) && !demonstrates_operator {
                    errors.push(format!(
                        "example for `{}` does not demonstrate `{example_call}`",
                        item.qualified_name
                    ));
                }
                if !example_sources.insert(example.source) {
                    errors.push(format!(
                        "`{}` reuses another symbol's visible example",
                        item.qualified_name
                    ));
                }
            }
            let effects = self.effects(item.id);
            let semantics = self.operation_semantics(item.id);
            if effects.is_empty() {
                errors.push(format!("`{}` declares no effects", item.qualified_name));
            }
            if effects.contains(&Effect::Pure) && effects.iter().count() != 1 {
                errors.push(format!(
                    "`{}` declares `pure` together with observable effects",
                    item.qualified_name
                ));
            }
            if semantics.cancellation != CancellationKind::None
                && !semantics.suspension.is_awaitable()
            {
                errors.push(format!(
                    "`{}` is cancellable but not awaitable",
                    item.qualified_name
                ));
            }
            if semantics.cancellation == CancellationKind::ProcessClose
                && !semantics.requires_attached_process
            {
                errors.push(format!(
                    "`{}` cancels on process close but does not require a process",
                    item.qualified_name
                ));
            }
            if effects.contains(&Effect::ReadsProcess) && !semantics.requires_attached_process {
                errors.push(format!(
                    "`{}` reads process state but does not require an attached process",
                    item.qualified_name
                ));
            }
            if semantics.availability == Availability::OnAttach
                && !semantics.suspension.is_awaitable()
            {
                errors.push(format!(
                    "`{}` is onAttach-only but is not awaitable",
                    item.qualified_name
                ));
            }
            for parameter in item.signature.parameters {
                if !public {
                    continue;
                }
                if parameter.documentation.trim().is_empty() {
                    errors.push(format!(
                        "parameter `{}.{}` has no documentation",
                        item.qualified_name, parameter.name
                    ));
                }
            }
            for related in item.documentation.related {
                if !stdlib_symbol_exists(*related) {
                    errors.push(format!(
                        "`{}` links to missing standard-library symbol `{:?}`",
                        item.qualified_name, related
                    ));
                }
            }
            if let Some(replacement) = item
                .deprecation
                .and_then(|deprecation| deprecation.replacement)
                && !ITEMS.iter().any(|candidate| candidate.id == replacement)
            {
                errors.push(format!(
                    "`{}` has missing replacement `{:?}`",
                    item.qualified_name, replacement
                ));
            }
        }
        for intrinsic in IntrinsicId::ALL {
            if !intrinsics.contains(intrinsic) {
                errors.push(format!(
                    "intrinsic `{intrinsic:?}` has no public standard-library binding"
                ));
            }
        }
        errors
    }
}

fn stdlib_symbol_exists(symbol: StdlibSymbolId) -> bool {
    match symbol {
        StdlibSymbolId::StateProvider(id) => STATE_PROVIDERS.iter().any(|value| value.id == id),
        StdlibSymbolId::Namespace(id) => NAMESPACES.iter().any(|value| value.id == id),
        StdlibSymbolId::Capability(id) => CAPABILITIES.iter().any(|value| value.id == id),
        StdlibSymbolId::TypeConstructor(id) => TYPE_CONSTRUCTORS.iter().any(|value| value.id == id),
        StdlibSymbolId::Type(id) => TYPES.iter().any(|value| value.id == id),
        StdlibSymbolId::Field(id) => FIELDS.iter().any(|value| value.id == id),
        StdlibSymbolId::Variant(id) => VARIANTS.iter().any(|value| value.id == id),
        StdlibSymbolId::Item(id) => ITEMS.iter().any(|value| value.id == id),
    }
}

fn validate_catalog_type_ref(
    ty: TypeRef,
    parameters: &[TypeParameter],
    item: &str,
    errors: &mut Vec<String>,
) {
    match ty {
        TypeRef::Core(core) => {
            if !CORE_TYPES.iter().any(|candidate| candidate.id == core) {
                errors.push(format!("`{item}` references unknown core type `{core:?}`"));
            }
        }
        TypeRef::Standard(standard) => {
            if !TYPES.iter().any(|candidate| candidate.id == standard) {
                errors.push(format!(
                    "`{item}` references unknown standard type `{standard:?}`"
                ));
            }
        }
        TypeRef::Parameter(parameter) => {
            if !parameters
                .iter()
                .any(|candidate| candidate.name == parameter)
            {
                errors.push(format!(
                    "`{item}` references undeclared type parameter `{parameter}`"
                ));
            }
        }
        TypeRef::Associated(_) => {}
        TypeRef::Async(value) | TypeRef::Iterator(value) => {
            validate_catalog_type_ref(*value, parameters, item, errors);
        }
        TypeRef::Application {
            constructor,
            arguments,
        } => {
            let Some(declaration) = TYPE_CONSTRUCTORS
                .iter()
                .find(|candidate| candidate.id == constructor)
            else {
                errors.push(format!(
                    "`{item}` references unknown type constructor `{constructor:?}`"
                ));
                return;
            };
            if declaration.parameters.len() != arguments.len() {
                errors.push(format!(
                    "`{item}` applies `{}` to {} type arguments instead of {}",
                    declaration.name,
                    arguments.len(),
                    declaration.parameters.len()
                ));
            }
            for argument in arguments {
                validate_catalog_type_ref(*argument, parameters, item, errors);
            }
        }
        TypeRef::FixedArray { element, .. } => {
            validate_catalog_type_ref(*element, parameters, item, errors);
        }
        TypeRef::Callable {
            parameters: callable_parameters,
            result,
        } => {
            for parameter in callable_parameters {
                validate_catalog_type_ref(*parameter, parameters, item, errors);
            }
            validate_catalog_type_ref(*result, parameters, item, errors);
        }
    }
}

fn validate_capability_hierarchy(capabilities: &[StdlibCapability], errors: &mut Vec<String>) {
    for capability in capabilities {
        let mut supers = HashSet::new();
        for super_capability in capability.super_capabilities {
            if !supers.insert(*super_capability) {
                errors.push(format!(
                    "capability `{}` repeats super capability `{:?}`",
                    capability.name, super_capability
                ));
            }
            if !capabilities
                .iter()
                .any(|candidate| candidate.id == *super_capability)
            {
                errors.push(format!(
                    "capability `{}` references unknown super capability `{:?}`",
                    capability.name, super_capability
                ));
            }
        }
    }

    let mut completed = HashSet::new();
    for capability in capabilities {
        let mut active = HashSet::new();
        if generated_capability_hierarchy_has_cycle(
            capability.id,
            capabilities,
            &mut active,
            &mut completed,
        ) {
            errors.push(format!(
                "capability hierarchy contains a cycle through `{}`",
                capability.name
            ));
            break;
        }
    }
}

fn generated_capability_hierarchy_has_cycle(
    capability: StdlibCapabilityId,
    capabilities: &[StdlibCapability],
    active: &mut HashSet<StdlibCapabilityId>,
    completed: &mut HashSet<StdlibCapabilityId>,
) -> bool {
    if completed.contains(&capability) {
        return false;
    }
    if !active.insert(capability) {
        return true;
    }
    let cyclic = capabilities
        .iter()
        .find(|candidate| candidate.id == capability)
        .into_iter()
        .flat_map(|candidate| candidate.super_capabilities.iter().copied())
        .any(|super_capability| {
            generated_capability_hierarchy_has_cycle(
                super_capability,
                capabilities,
                active,
                completed,
            )
        });
    active.remove(&capability);
    completed.insert(capability);
    cyclic
}

fn validate_named_declarations<T, I>(
    kind: &str,
    values: &[T],
    project: impl Fn(&T) -> (I, &'static str, Documentation<StdlibSymbolId>),
    errors: &mut Vec<String>,
) where
    I: Copy + std::fmt::Debug + Eq + std::hash::Hash,
{
    let mut ids = HashSet::new();
    let mut names = HashSet::new();
    for value in values {
        let (id, name, documentation) = project(value);
        if !ids.insert(id) {
            errors.push(format!("duplicate {kind} ID `{:?}`", id));
        }
        if !names.insert(name) {
            errors.push(format!("duplicate {kind} name `{name}`"));
        }
        if documentation.summary.trim().is_empty() {
            errors.push(format!("{kind} `{name}` has incomplete documentation"));
        }
        if documentation.examples.len() != 1 {
            errors.push(format!(
                "{kind} `{name}` must have exactly one focused documentation example"
            ));
        }
        for example in documentation.examples {
            if example.title.trim().is_empty()
                || example.source.trim().is_empty()
                || !example.has_validation_source()
                || !example.validation_includes_source()
            {
                errors.push(format!(
                    "{kind} `{name}` has an incomplete documentation example"
                ));
            }
        }
    }
}

fn struct_example_sources(
    kind: &str,
    name: &str,
    documentation: Documentation<StdlibSymbolId>,
    example_sources: &mut HashSet<&'static str>,
    errors: &mut Vec<String>,
) {
    for example in documentation.examples {
        if !example_sources.insert(example.source) {
            errors.push(format!(
                "{kind} `{name}` reuses another symbol's visible example"
            ));
        }
    }
}
