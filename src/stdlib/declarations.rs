//! Nominal standard-library declarations shared by semantics, code generation,
//! documentation, and editor tooling.
//!
//! This module deliberately contains no parser, inference, or WebAssembly
//! encoder types. It describes public symbols and backend-neutral runtime
//! shapes; consumers resolve those identities into their own stage-specific
//! representations.

use crate::catalog::Documentation;
pub use splitscript_syntax::PrimitiveType as CoreTypeId;

use super::ids::{
    StdlibCapabilityId, StdlibFieldId, StdlibItemId, StdlibNamespaceId, StdlibStateProviderId,
    StdlibTypeConstructorId, StdlibTypeId, StdlibVariantId,
};
use super::schema::{TypeParameter, TypeRef};

/// Canonical user-facing lifecycle note for providers whose private memory
/// mapping can disappear while their host process remains open.
pub const STATE_PROVIDER_MAPPING_LIFECYCLE_NOTE: &str = "The provider validates its private memory mapping before each state poll. If that mapping disappears while the host process remains open, the current logical attachment ends: attachment-scoped state and pending continuations are cleared, and [`onDetach`] runs if [`onAttach`] had completed. Discovery then continues cooperatively against the same host process. A replacement mapping starts a new attachment, runs [`onAttach`] again, and establishes a fresh [`old`] / [`current`] baseline.";

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum StdlibOwner {
    Root,
    Namespace(StdlibNamespaceId),
    Type(StdlibTypeId),
    Core(CoreTypeId),
    Capability(StdlibCapabilityId),
    TypeConstructor(StdlibTypeConstructorId),
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub enum StdlibSymbolId {
    StateProvider(StdlibStateProviderId),
    Namespace(StdlibNamespaceId),
    Capability(StdlibCapabilityId),
    TypeConstructor(StdlibTypeConstructorId),
    Type(StdlibTypeId),
    Field(StdlibFieldId),
    Variant(StdlibVariantId),
    Item(StdlibItemId),
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct StdlibStateProvider {
    pub id: StdlibStateProviderId,
    pub name: &'static str,
    pub value_name: &'static str,
    pub processes: StateProviderProcesses,
    /// Whether an unqualified `state "..."` declaration selects this
    /// provider. Exactly one source-process provider has this role.
    pub default: bool,
    pub process_type: StdlibTypeId,
    pub attachment: StateProviderAttachment,
    /// Optional synchronous provider-owned validation performed before each
    /// state poll. A failed validation ends the logical attachment while
    /// retaining the host process for a fresh provider attachment attempt.
    pub validation: Option<StdlibItemId>,
    /// Optional asynchronous work performed after the provider value becomes
    /// available and before the user's `onAttach` action runs.
    pub preparation: Option<StdlibItemId>,
    pub direct_read: StdlibItemId,
    /// Statically known guest-memory domains accepted by this provider's
    /// direct read. An empty slice means the catalog cannot prove a bounded
    /// domain and runtime validation remains authoritative.
    pub readable_ranges: &'static [StateProviderMemoryRange],
    pub contexts: &'static [StateProviderContext],
    pub selectors: &'static [StateProviderSelector],
    pub documentation: Documentation<StdlibSymbolId>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct StateProviderMemoryRange {
    pub start: u64,
    pub end: u64,
}

impl StateProviderMemoryRange {
    pub const fn contains(self, address: u64, size: u32) -> bool {
        address >= self.start
            && match address.checked_add(size as u64) {
                Some(end) => end <= self.end,
                None => false,
            }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct StateProviderContext {
    pub name: &'static str,
    pub ty: StdlibTypeId,
    /// Parameterless asynchronous source function that constructs the value
    /// once for each process attachment where the context is referenced.
    pub preparation: StdlibItemId,
    pub documentation: Documentation<StdlibSymbolId>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct StateProviderSelector {
    pub name: &'static str,
    pub parameters: &'static [StateProviderSelectorParameter],
    /// Selector-specific preparation callable. Its parameters exactly match
    /// this selector. A managed backend specialization may return its own
    /// context; otherwise the result matches the default preparation.
    pub preparation: StdlibItemId,
    /// Optional managed-runtime specialization known from this selector.
    ///
    /// This is privileged catalog metadata rather than public language syntax.
    /// Managed-schema generation uses it to omit backend branches that cannot
    /// be reached for an explicitly configured provider.
    pub managed_backend: Option<ManagedRuntimeBackend>,
    pub documentation: Documentation<StdlibSymbolId>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ManagedRuntimeBackend {
    Il2Cpp,
    Mono,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct StateProviderSelectorParameter {
    pub name: &'static str,
    pub ty: TypeRef,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum StateProviderProcesses {
    /// Process names are declared by the provider itself.
    Declared(&'static [&'static str]),
    /// Process names come from the source-level `state "..."` declaration.
    SourceState,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum StateProviderAttachment {
    /// The provider value is the raw attached-process handle.
    Identity,
    /// A standard-library callable derives the provider value from the raw
    /// attached-process handle. Its implementation may be intrinsic or an
    /// ordinary source-defined async function.
    Callable(StdlibItemId),
}

macro_rules! with_core_types {
    ($consumer:ident) => {
        $consumer! {
            Never => { name: "Never", capabilities: &[], memory_layout: None },
            None => { name: "None", capabilities: &[StdlibCapabilityId::Debug, StdlibCapabilityId::Equatable], memory_layout: None },
            Bool => { name: "bool", capabilities: BOOL_CAPABILITIES, memory_layout: Some(ScalarMemoryLayout { size: 1, alignment: 1 }) },
            Char => { name: "char", capabilities: CHAR_CAPABILITIES, memory_layout: None },
            I8 => { name: "i8", capabilities: SIGNED_INTEGER_CAPABILITIES, memory_layout: Some(ScalarMemoryLayout { size: 1, alignment: 1 }) },
            U8 => { name: "u8", capabilities: UNSIGNED_INTEGER_CAPABILITIES, memory_layout: Some(ScalarMemoryLayout { size: 1, alignment: 1 }) },
            I16 => { name: "i16", capabilities: SIGNED_INTEGER_CAPABILITIES, memory_layout: Some(ScalarMemoryLayout { size: 2, alignment: 2 }) },
            U16 => { name: "u16", capabilities: UNSIGNED_INTEGER_CAPABILITIES, memory_layout: Some(ScalarMemoryLayout { size: 2, alignment: 2 }) },
            I32 => { name: "i32", capabilities: SIGNED_INTEGER_CAPABILITIES, memory_layout: Some(ScalarMemoryLayout { size: 4, alignment: 4 }) },
            U32 => { name: "u32", capabilities: UNSIGNED_INTEGER_CAPABILITIES, memory_layout: Some(ScalarMemoryLayout { size: 4, alignment: 4 }) },
            I64 => { name: "i64", capabilities: SIGNED_INTEGER_CAPABILITIES, memory_layout: Some(ScalarMemoryLayout { size: 8, alignment: 8 }) },
            U64 => { name: "u64", capabilities: UNSIGNED_INTEGER_CAPABILITIES, memory_layout: Some(ScalarMemoryLayout { size: 8, alignment: 8 }) },
            Address => { name: "address", capabilities: UNSIGNED_INTEGER_CAPABILITIES, memory_layout: Some(ScalarMemoryLayout { size: 8, alignment: 8 }) },
            F32 => { name: "f32", capabilities: FLOAT_CAPABILITIES, memory_layout: Some(ScalarMemoryLayout { size: 4, alignment: 4 }) },
            F64 => { name: "f64", capabilities: FLOAT_CAPABILITIES, memory_layout: Some(ScalarMemoryLayout { size: 8, alignment: 8 }) }
        }
    };
}

pub(crate) use with_core_types;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum DeclaredTypeRef {
    Core(CoreTypeId),
    Standard(StdlibTypeId),
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct CoreType {
    pub id: CoreTypeId,
    pub name: &'static str,
    pub capabilities: &'static [StdlibCapabilityId],
    pub memory_layout: Option<ScalarMemoryLayout>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ScalarMemoryLayout {
    pub size: u32,
    pub alignment: u32,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum StdlibTypeKind {
    Intrinsic,
    Struct,
    Enum,
}

/// Backend-neutral storage requirements for a standard-library type.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RuntimeRepresentation {
    Scalar {
        storage: CoreTypeId,
    },
    GcArray {
        element: CoreTypeId,
        mutable: bool,
        nullable: bool,
    },
    GcStruct {
        nullable: bool,
    },
    Enum {
        nullable: bool,
    },
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum FieldVisibility {
    Public,
    RuntimePrivate,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ValueUsage {
    pub struct_field: bool,
    pub enum_payload: bool,
    pub state_field: bool,
    pub local_variable: bool,
    pub global_variable: bool,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct StdlibNamespace {
    pub id: StdlibNamespaceId,
    pub name: &'static str,
    pub path: &'static [&'static str],
    pub documentation: Documentation<StdlibSymbolId>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CapabilityBehavior {
    Declared,
    StructuralEquality,
    StructuralMemoryLayout,
    /// Fixed-layout base cases and source-owned high-level managed decoders.
    StructuralManagedRead,
    /// User-defined nominal types satisfy the capability by declaring every
    /// method contract owned by it with the required signature.
    StructuralMethods,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct StdlibCapability {
    pub id: StdlibCapabilityId,
    pub name: &'static str,
    /// Capabilities guaranteed by this capability.
    pub super_capabilities: &'static [StdlibCapabilityId],
    pub behavior: CapabilityBehavior,
    pub associated_types: &'static [StdlibAssociatedType],
    pub documentation: Documentation<StdlibSymbolId>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct StdlibTypeConstructor {
    pub id: StdlibTypeConstructorId,
    pub public: bool,
    pub syntax: TypeConstructorSyntax,
    pub name: &'static str,
    pub parameters: &'static [TypeParameter],
    pub capabilities: &'static [StdlibCapabilityId],
    pub must_use: Option<&'static str>,
    pub associated_types: &'static [StdlibAssociatedTypeDefinition],
    pub documentation: Documentation<StdlibSymbolId>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct StdlibAssociatedType {
    pub name: &'static str,
    pub constraints: &'static [StdlibCapabilityId],
    pub documentation: Documentation<StdlibSymbolId>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct StdlibAssociatedTypeDefinition {
    pub name: &'static str,
    pub value: super::TypeRef,
    pub documentation: Documentation<StdlibSymbolId>,
}

/// Whether a nominal type belongs to the authored language surface or only to
/// trusted standard-library implementation source.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TypeVisibility {
    Public,
    LibraryPrivate,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TypeConstructorSyntax {
    Named,
    Array,
    Optional,
    Fallible,
    ExclusiveRange,
    InclusiveRange,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct StdlibType {
    pub id: StdlibTypeId,
    pub name: &'static str,
    pub visibility: TypeVisibility,
    pub kind: StdlibTypeKind,
    pub capabilities: &'static [StdlibCapabilityId],
    /// Concrete associated-type definitions used by declared capabilities.
    pub associated_types: &'static [StdlibAssociatedTypeDefinition],
    /// Catalog method used for user-facing string conversion, when this type
    /// supplies a source- or intrinsic-defined `Display` implementation.
    pub display: Option<super::StdlibItemId>,
    pub representation: RuntimeRepresentation,
    pub value_usage: ValueUsage,
    /// Whether user code may construct this complete value with a struct literal.
    pub public_construction: bool,
    pub documentation: Documentation<StdlibSymbolId>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct StdlibField {
    pub id: StdlibFieldId,
    pub owner: StdlibOwner,
    pub name: &'static str,
    pub ty: TypeRef,
    pub visibility: FieldVisibility,
    pub documentation: Documentation<StdlibSymbolId>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct StdlibVariant {
    pub id: StdlibVariantId,
    pub owner: StdlibTypeId,
    pub name: &'static str,
    pub documentation: Documentation<StdlibSymbolId>,
}

const BOOL_CAPABILITIES: &[StdlibCapabilityId] = &[
    StdlibCapabilityId::Debug,
    StdlibCapabilityId::Equatable,
    StdlibCapabilityId::Display,
    StdlibCapabilityId::MemoryReadable,
];
const CHAR_CAPABILITIES: &[StdlibCapabilityId] = &[
    StdlibCapabilityId::Debug,
    StdlibCapabilityId::Equatable,
    StdlibCapabilityId::Display,
];
const SIGNED_INTEGER_CAPABILITIES: &[StdlibCapabilityId] = &[
    StdlibCapabilityId::Debug,
    StdlibCapabilityId::Integer,
    StdlibCapabilityId::Signed,
    StdlibCapabilityId::MemoryReadable,
];
const UNSIGNED_INTEGER_CAPABILITIES: &[StdlibCapabilityId] = &[
    StdlibCapabilityId::Debug,
    StdlibCapabilityId::Integer,
    StdlibCapabilityId::MemoryReadable,
];
const FLOAT_CAPABILITIES: &[StdlibCapabilityId] = &[
    StdlibCapabilityId::Float,
    StdlibCapabilityId::MemoryReadable,
];

macro_rules! define_core_type_table {
    ($($id:ident => {
        name: $name:literal,
        capabilities: $capabilities:expr,
        memory_layout: $memory_layout:expr
    }),* $(,)?) => {
        pub(super) const CORE_TYPES: &[CoreType] = &[
            $(CoreType {
                id: CoreTypeId::$id,
                name: $name,
                capabilities: $capabilities,
                memory_layout: $memory_layout,
            }),*
        ];
    };
}

with_core_types!(define_core_type_table);

#[cfg(test)]
mod tests {
    use wasmparser::{Validator, WasmFeatures};

    use crate::{
        completion::CompletionKind,
        database::{CompilerDatabase, DefinitionTarget},
        memory::{MemoryFieldId, MemoryTypeLayout},
        stdlib::{StandardLibrary, StdlibSymbolId},
    };

    use super::{StdlibFieldId, StdlibTypeId};

    #[test]
    fn ordinary_catalog_struct_flows_through_compiler_and_tooling_generically() {
        let source = r#"
            state "probe.exe" {}

            whileAttached {
                let probe: CatalogStructProbe = process.read(0x100) else return
                if probe == probe {
                    print(probe.value as String)
                }
            }
        "#;
        let library = StandardLibrary::new();
        let structure = library
            .type_by_name("CatalogStructProbe")
            .expect("the test struct should resolve by its sole catalog name");
        assert_eq!(structure.id, StdlibTypeId::CatalogStructProbe);
        let field = library
            .public_field(structure.id, "value")
            .expect("the test struct's declared field should be discoverable");
        assert_eq!(field.id, StdlibFieldId::CatalogStructProbeValue);
        assert_eq!(library.validate(), Vec::<String>::new());

        let mut database = CompilerDatabase::new(source);
        let checked = database
            .check()
            .expect("catalog names and fields should type-check without special cases");
        let ty = checked
            .semantics()
            .types()
            .id_for_standard(StdlibTypeId::CatalogStructProbe);
        let MemoryTypeLayout::Struct(memory) = checked
            .memory_layouts()
            .layout(
                ty,
                checked.semantics(),
                crate::memory::MemoryAddressWidth::Bit64,
            )
            .expect("the declared capability should produce a generic memory layout")
        else {
            panic!("the catalog fixture should have a struct memory layout")
        };
        assert_eq!((memory.size, memory.alignment), (4, 4));
        assert_eq!(
            memory.fields[0].field,
            MemoryFieldId::Standard(StdlibFieldId::CatalogStructProbeValue)
        );

        let type_offset = source.find("CatalogStructProbe").unwrap();
        assert_eq!(
            database.definition_at(type_offset).unwrap(),
            Some(DefinitionTarget::StandardLibrarySymbol(
                StdlibSymbolId::Type(StdlibTypeId::CatalogStructProbe)
            ))
        );
        let type_hover = database
            .hover(type_offset)
            .unwrap()
            .expect("catalog type documentation should power hover");
        assert!(
            type_hover
                .markdown
                .contains("Exercises the generic standard-library struct pipeline")
        );
        let type_completion = database
            .completions(type_offset + "CatalogStruct".len())
            .unwrap();
        assert!(type_completion.items.iter().any(|item| {
            item.label == "CatalogStructProbe"
                && item.kind == CompletionKind::Struct
                && item.documentation.as_deref().is_some_and(|documentation| {
                    documentation.contains("generic standard-library struct pipeline")
                })
        }));

        let field_offset = source.find("probe.value").unwrap() + "probe.".len();
        assert_eq!(
            database.definition_at(field_offset).unwrap(),
            Some(DefinitionTarget::StandardLibrarySymbol(
                StdlibSymbolId::Field(StdlibFieldId::CatalogStructProbeValue)
            ))
        );
        let hover = database
            .hover(field_offset)
            .unwrap()
            .expect("catalog field documentation should power hover");
        assert!(hover.markdown.contains("CatalogStructProbe.value: u32"));
        assert!(hover.markdown.contains("Returns the probe value"));

        let completion_offset = field_offset + "va".len();
        let completion = database.completions(completion_offset).unwrap();
        let value = completion
            .items
            .iter()
            .find(|item| item.label == "value")
            .expect("catalog fields should power receiver completion");
        assert_eq!(value.kind, CompletionKind::Property);
        assert!(
            value
                .documentation
                .as_deref()
                .is_some_and(|documentation| documentation.contains("Returns the probe value"))
        );

        let wasm = crate::compile(source)
            .expect("generic process-memory and field lowering should compile the catalog struct");
        Validator::new_with_features(WasmFeatures::all())
            .validate_all(&wasm)
            .expect("the catalog representation should produce a valid Wasm GC struct");
    }

    #[test]
    fn constructed_catalog_fields_share_semantics_tooling_and_gc_layouts() {
        let source = r#"
            state "probe.exe" {}

            whileAttached {
                let seed: CatalogStructProbe = process.read(0x100) else return
                let constructed = seed.constructed([3, 5], Some(8))
                let pair = seed.fixedPair([13, 21])
                let memory: CatalogFixedMemoryProbe = process.read(0x200) else return
                let fallback = constructed.fallback else 0
                print(`{constructed.values.length()}:{pair.length()}:{memory.values.length()}:{fallback}`)
            }
        "#;
        let library = StandardLibrary::new();
        let structure = library
            .type_by_name("CatalogConstructedFieldProbe")
            .expect("the constructed-field fixture should be generated");
        let values = library
            .public_field(structure.id, "values")
            .expect("the array field should be discoverable");
        assert_eq!(library.render_type(values.ty), "[u64; 2]");

        let mut database = CompilerDatabase::new(source);
        database
            .check()
            .expect("constructed standard fields should use ordinary type inference");

        let offset = source.find("constructed.values").unwrap() + "constructed.".len();
        let hover = database
            .hover(offset)
            .unwrap()
            .expect("constructed standard fields should provide hover");
        assert!(
            hover
                .markdown
                .contains("CatalogConstructedFieldProbe.values: [u64; 2]")
        );

        let wasm = crate::compile(source)
            .expect("constructed standard fields should lower through ordinary Wasm GC layouts");
        Validator::new_with_features(WasmFeatures::all())
            .validate_all(&wasm)
            .expect("constructed standard fields should produce valid Wasm GC");
    }
}
