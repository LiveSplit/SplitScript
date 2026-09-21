//! Deterministic assignment of semantic aggregate types to Wasm GC indices.

use std::collections::HashMap;

use wasm_encoder::{AbstractHeapType, HeapType, RefType, StorageType, ValType};

use crate::{
    ast::{AsyncTypeId, EnumDecl, IteratorTypeId, Program},
    semantic::{ClosureInstance, FunctionInstance, ResolvedEnumVariantId},
    stdlib::{
        DeclaredTypeRef, RuntimeRepresentation, StandardLibrary, StdlibFieldId, StdlibTypeId,
    },
    types::{
        EnumTypeId, ResolvedApplicationType, ResolvedArrayType, ResolvedAsyncType,
        ResolvedCallableType, ResolvedIteratorType, ResolvedOptionType, ResolvedRangeType,
        ResolvedResultType, ResolvedSetType, ResolvedTypeRef,
    },
};

use super::{
    STATE_TYPE, Type,
    async_frame::{AsyncFrameLayouts, LeafFutureInstance},
    reachability,
};

pub(super) struct GcLayout {
    pub standard_library: StandardLibrary,
    pub string_literals: super::data_plan::GcStringLiterals,
    standard: HashMap<StdlibTypeId, u32>,
    standard_fields: HashMap<StdlibFieldId, u32>,
    async_frame: u32,
    async_values: HashMap<AsyncTypeId, u32>,
    iterator_values: HashMap<IteratorTypeId, u32>,
    callable_functions: HashMap<crate::ast::CallableTypeId, u32>,
    capture_cells: HashMap<Type, u32>,
    ordered_capture_cells: Vec<Type>,
    closure_environments: HashMap<ClosureInstance, u32>,
    function_frames: HashMap<FunctionInstance, u32>,
    function_frame_tags: HashMap<FunctionInstance, u32>,
    closure_frames: HashMap<ClosureInstance, u32>,
    closure_frame_tags: HashMap<ClosureInstance, u32>,
    leaf_frames: HashMap<LeafFutureInstance, u32>,
    leaf_frame_tags: HashMap<LeafFutureInstance, u32>,
    dynamic: HashMap<Type, u32>,
    ordered: Vec<Type>,
    pub type_count: u32,
}

pub(super) struct Inputs<'a> {
    pub standard_library: StandardLibrary,
    pub program: &'a Program,
    pub wasm_ir: &'a crate::wasm_ir::Program,
    pub enums: &'a [EnumDecl],
    pub semantics: &'a crate::semantic::SemanticModel,
    pub arrays: &'a [ResolvedArrayType],
    pub options: &'a [ResolvedOptionType],
    pub results: &'a [ResolvedResultType],
    pub asyncs: &'a [ResolvedAsyncType],
    pub iterators: &'a [ResolvedIteratorType],
    pub callables: &'a [ResolvedCallableType],
    pub sets: &'a [ResolvedSetType],
    pub applications: &'a [ResolvedApplicationType],
    pub ranges: &'a [ResolvedRangeType],
    pub async_frames: &'a AsyncFrameLayouts,
    pub reachability: &'a reachability::Reachability,
}

impl GcLayout {
    pub(super) fn plan(inputs: Inputs<'_>) -> Self {
        let Inputs {
            standard_library,
            program,
            wasm_ir,
            enums,
            semantics,
            arrays,
            options,
            results,
            asyncs,
            iterators,
            callables,
            sets,
            applications,
            ranges,
            async_frames,
            reachability,
        } = inputs;
        let standard = standard_library
            .all_types()
            .iter()
            .filter(|declaration| reachability.contains_standard_type(declaration.id))
            .filter(|declaration| {
                matches!(
                    declaration.representation,
                    RuntimeRepresentation::GcArray { .. }
                        | RuntimeRepresentation::GcStruct { .. }
                        | RuntimeRepresentation::Enum { .. }
                )
            })
            .enumerate()
            .map(|(position, declaration)| (declaration.id, STATE_TYPE + 1 + position as u32))
            .collect::<HashMap<_, _>>();
        let standard_fields = standard_library
            .all_types()
            .iter()
            .filter(|declaration| {
                matches!(
                    declaration.representation,
                    RuntimeRepresentation::GcStruct { .. }
                )
            })
            .flat_map(|declaration| {
                standard_library
                    .fields_of(declaration.id)
                    .enumerate()
                    .map(|(position, field)| (field.id, position as u32))
            })
            .collect::<HashMap<_, _>>();
        let async_frame = STATE_TYPE + 1 + standard.len() as u32;
        let mut next = async_frame + 1;
        let mut dynamic = HashMap::new();
        let mut callable_functions = HashMap::new();
        let mut ordered = Vec::new();

        for structure in program
            .structs
            .iter()
            .filter(|structure| reachability.contains_struct_type(structure.id))
        {
            dynamic.insert(Type::Struct(structure.id), next);
            ordered.push(Type::Struct(structure.id));
            next += 1;
        }
        for class in program
            .managed_class_declarations()
            .into_iter()
            .filter(|class| reachability.contains_managed_class_type(class.id))
        {
            dynamic.insert(Type::ManagedClass(class.id), next);
            ordered.push(Type::ManagedClass(class.id));
            next += 1;
        }
        for enumeration in enums
            .iter()
            .filter(|enumeration| reachability.contains_enum_type(enumeration.id))
        {
            dynamic.insert(Type::Enum(enumeration.id), next);
            ordered.push(Type::Enum(enumeration.id));
            next += 1;
        }
        let reachable_arrays = arrays
            .iter()
            .filter(|array| reachability.contains_array_type(array.id))
            .collect::<Vec<_>>();
        let mut canonical_arrays = Vec::new();
        for reachable in &reachable_arrays {
            let element = super::try_array_element_type(reachable.id, semantics)
                .expect("reachable arrays have backend-representable element types");
            let general = arrays.iter().find(|array| {
                array.length.is_none()
                    && super::try_array_element_type(array.id, semantics) == Some(element)
            });
            let representation_length = general.is_none().then_some(reachable.length);
            if canonical_arrays.iter().any(|array: &&ResolvedArrayType| {
                let candidate_general = arrays.iter().any(|candidate| {
                    candidate.length.is_none()
                        && super::try_array_element_type(candidate.id, semantics) == Some(element)
                });
                super::try_array_element_type(array.id, semantics) == Some(element)
                    && (!candidate_general).then_some(array.length) == representation_length
            }) {
                continue;
            }
            let canonical = general.unwrap_or(reachable);
            canonical_arrays.push(canonical);
        }
        canonical_arrays.sort_by_key(|array| array.id.index());
        for array in &canonical_arrays {
            let ty = Type::Array(array.id);
            dynamic.insert(ty, next);
            ordered.push(ty);
            next += 1;
        }
        for array in reachable_arrays {
            let element = super::try_array_element_type(array.id, semantics)
                .expect("reachable arrays have backend-representable element types");
            let has_general = arrays.iter().any(|candidate| {
                candidate.length.is_none()
                    && super::try_array_element_type(candidate.id, semantics) == Some(element)
            });
            let representation_length = (!has_general).then_some(array.length);
            let canonical = canonical_arrays
                .iter()
                .find(|candidate| {
                    super::try_array_element_type(candidate.id, semantics) == Some(element)
                        && (!has_general).then_some(candidate.length) == representation_length
                })
                .expect("every reachable array element has a canonical wrapper layout");
            let index = dynamic[&Type::Array(canonical.id)];
            dynamic.insert(Type::Array(array.id), index);
        }

        let mut reachable_storage = arrays
            .iter()
            .filter(|array| reachability.contains_array_storage(array.id))
            .collect::<Vec<_>>();
        let storage_sized_elements = reachable_storage
            .iter()
            .filter(|array| array.length.is_some())
            .filter_map(|array| super::try_array_element_type(array.id, semantics))
            .collect::<Vec<_>>();
        let reachable_storage_ids = reachable_storage
            .iter()
            .map(|array| array.id)
            .collect::<Vec<_>>();
        reachable_storage.extend(arrays.iter().filter(|array| {
            array.length.is_none()
                && super::try_array_element_type(array.id, semantics)
                    .is_some_and(|element| storage_sized_elements.contains(&element))
                && !reachable_storage_ids.contains(&array.id)
        }));
        let mut canonical_storage = Vec::new();
        for reachable in &reachable_storage {
            let element = super::try_array_element_type(reachable.id, semantics)
                .expect("reachable array storage has a backend-representable element type");
            if canonical_storage.iter().any(|array: &&ResolvedArrayType| {
                array.length == reachable.length
                    && super::try_array_element_type(array.id, semantics) == Some(element)
            }) {
                continue;
            }
            canonical_storage.push(*reachable);
        }
        canonical_storage.sort_by_key(|array| (array.length.is_some(), array.id.index()));
        for array in &canonical_storage {
            let ty = Type::ArrayStorage(array.id);
            dynamic.insert(ty, next);
            ordered.push(ty);
            next += 1;
        }
        // Wrapper/storage selection can name an earlier, unused declaration
        // of the same physical array shape. Alias it to already-emitted
        // storage without making that declaration or its readers reachable.
        for array in arrays {
            let Some(element) = super::try_array_element_type(array.id, semantics) else {
                continue;
            };
            let Some(canonical) = canonical_storage.iter().find(|candidate| {
                candidate.length == array.length
                    && super::try_array_element_type(candidate.id, semantics) == Some(element)
            }) else {
                continue;
            };
            let index = dynamic[&Type::ArrayStorage(canonical.id)];
            dynamic.insert(Type::ArrayStorage(array.id), index);
        }

        for set in sets
            .iter()
            .filter(|set| reachability.contains_set_type(set.id))
        {
            let ty = Type::Set(set.id);
            dynamic.insert(ty, next);
            ordered.push(ty);
            next += 1;
        }

        for application in applications
            .iter()
            .filter(|application| reachability.contains_application_type(application.id))
        {
            let ty = Type::Application(application.id);
            dynamic.insert(ty, next);
            ordered.push(ty);
            next += 1;
        }

        for callable in callables
            .iter()
            .filter(|callable| reachability.contains_callable_type(callable.id))
        {
            let ty = Type::Callable(callable.id);
            callable_functions.insert(callable.id, next);
            next += 1;
            dynamic.insert(ty, next);
            ordered.push(ty);
            next += 1;
        }

        // Generic source-defined methods can introduce template range shapes.
        // Only their materialized integer instantiations have a physical Wasm
        // representation; the generic template itself is erased.
        for range in ranges
            .iter()
            .filter(|range| matches!(range.bound, ResolvedTypeRef::Core(_)))
            .filter(|range| reachability.contains_range_type(range.id))
        {
            let ty = Type::Range(range.id);
            dynamic.insert(ty, next);
            ordered.push(ty);
            next += 1;
        }

        let mut constructed = options
            .iter()
            .filter(|option| reachability.contains_option_type(option.id))
            .map(|option| (option.id.index(), Type::Option(option.id)))
            .chain(
                results
                    .iter()
                    .filter(|result| reachability.contains_result_type(result.id))
                    .map(|result| (result.id.index(), Type::Result(result.id))),
            )
            .collect::<Vec<_>>();
        constructed.sort_by_key(|(id, _)| *id);
        for (_, ty) in constructed {
            dynamic.insert(ty, next);
            ordered.push(ty);
            next += 1;
        }

        let mut capture_cells = HashMap::new();
        let mut ordered_capture_cells = Vec::new();
        for instance in reachability.closure_instances() {
            let closure = wasm_ir
                .closure(instance.expression)
                .expect("reachable closure instances have bodies");
            for capture in closure.captures.iter().filter(|capture| capture.mutable) {
                let ty = instance.owner.as_ref().map_or_else(
                    || super::value_type(capture.value, semantics),
                    |owner| {
                        super::semantic_type(
                            semantics.specialize_type(
                                owner,
                                semantics
                                    .value_type(capture.value)
                                    .expect("checked captures have types"),
                            ),
                            semantics,
                        )
                    },
                );
                if !ty.has_runtime_value() || capture_cells.contains_key(&ty) {
                    continue;
                }
                capture_cells.insert(ty, next);
                ordered_capture_cells.push(ty);
                next += 1;
            }
        }

        let mut closure_environments = HashMap::new();
        for instance in reachability.closure_instances().filter(|instance| {
            wasm_ir
                .closure(instance.expression)
                .is_some_and(|closure| !closure.captures.is_empty())
        }) {
            closure_environments.insert(instance.clone(), next);
            next += 1;
        }

        let mut async_values = HashMap::new();
        for future in asyncs
            .iter()
            .filter(|future| reachability.contains_async_type(future.id))
        {
            async_values.insert(future.id, next);
            next += 1;
        }
        let mut iterator_values = HashMap::new();
        for iterator in iterators
            .iter()
            .filter(|iterator| reachability.contains_iterator_type(iterator.id))
        {
            iterator_values.insert(iterator.id, next);
            next += 1;
        }
        let mut function_frames = HashMap::new();
        let mut function_frame_tags = HashMap::new();
        for (tag, (instance, _)) in async_frames.functions().enumerate() {
            function_frames.insert(instance.clone(), next);
            function_frame_tags.insert(instance.clone(), tag as u32 + 1);
            next += 1;
        }
        let mut closure_frames = HashMap::new();
        let mut closure_frame_tags = HashMap::new();
        let first_closure_tag = function_frames.len() as u32 + 1;
        for (position, (instance, _)) in async_frames.closures().enumerate() {
            closure_frames.insert(instance.clone(), next);
            closure_frame_tags.insert(instance.clone(), first_closure_tag + position as u32);
            next += 1;
        }
        let first_intrinsic_tag = first_closure_tag + closure_frames.len() as u32;
        let mut leaf_frames = HashMap::new();
        let mut leaf_frame_tags = HashMap::new();
        for (position, (instance, _)) in async_frames.leaves().enumerate() {
            leaf_frames.insert(instance.clone(), next);
            leaf_frame_tags.insert(instance.clone(), first_intrinsic_tag + position as u32);
            next += 1;
        }

        Self {
            standard_library,
            string_literals: Default::default(),
            standard,
            standard_fields,
            async_frame,
            async_values,
            iterator_values,
            callable_functions,
            capture_cells,
            ordered_capture_cells,
            closure_environments,
            function_frames,
            function_frame_tags,
            closure_frames,
            closure_frame_tags,
            leaf_frames,
            leaf_frame_tags,
            dynamic,
            ordered,
            type_count: next,
        }
    }

    pub(super) fn dynamic_types(&self) -> impl ExactSizeIterator<Item = Type> + '_ {
        self.ordered.iter().copied()
    }

    pub(super) fn standard_index(&self, ty: StdlibTypeId) -> u32 {
        *self
            .standard
            .get(&ty)
            .unwrap_or_else(|| panic!("standard type `{ty:?}` has no static GC layout"))
    }

    pub(super) fn standard_field_index(&self, field: StdlibFieldId) -> u32 {
        self.standard_fields
            .get(&field)
            .copied()
            .expect("every standard field belongs to its owner's declared slots")
    }

    pub(super) fn enum_variant_index(
        &self,
        enumeration: EnumTypeId,
        variant: ResolvedEnumVariantId,
        enums: &[EnumDecl],
    ) -> usize {
        match (enumeration, variant) {
            (EnumTypeId::Source(enumeration), ResolvedEnumVariantId::Source(variant)) => enums
                .iter()
                .find(|declaration| declaration.id == enumeration)
                .and_then(|declaration| {
                    declaration
                        .variants
                        .iter()
                        .position(|declared| declared.id == variant)
                })
                .expect("checked source enum variants belong to their declaration"),
            (EnumTypeId::Standard(enumeration), ResolvedEnumVariantId::Standard(variant)) => self
                .standard_library
                .variants_of(enumeration)
                .position(|declared| declared.id == variant)
                .expect("checked standard enum variants belong to their declaration"),
            _ => unreachable!("checked enum and variant identities have the same owner"),
        }
    }

    pub(super) fn async_frame_index(&self) -> u32 {
        self.async_frame
    }

    pub(super) fn callable_function_index(&self, callable: crate::ast::CallableTypeId) -> u32 {
        self.callable_functions[&callable]
    }

    pub(super) fn closure_environment_index(&self, instance: &ClosureInstance) -> Option<u32> {
        self.closure_environments.get(instance).copied()
    }

    pub(super) fn capture_cell_index(&self, ty: Type) -> u32 {
        self.capture_cells[&ty]
    }

    pub(super) fn capture_cell_types(&self) -> impl Iterator<Item = Type> + '_ {
        self.ordered_capture_cells.iter().copied()
    }

    pub(super) fn capture_cell_val_type(&self, ty: Type) -> ValType {
        ValType::Ref(RefType {
            nullable: true,
            heap_type: HeapType::Concrete(self.capture_cell_index(ty)),
        })
    }

    pub(super) fn capture_cell_storage_type(&self, ty: Type) -> StorageType {
        StorageType::Val(self.capture_cell_val_type(ty))
    }

    pub(super) fn function_frame_index(&self, instance: &FunctionInstance) -> u32 {
        self.function_frames
            .get(instance)
            .copied()
            .expect("suspending function instances have planned GC frames")
    }

    pub(super) fn function_frame_tag(&self, instance: &FunctionInstance) -> u32 {
        self.function_frame_tags
            .get(instance)
            .copied()
            .expect("suspending function instances have runtime tags")
    }

    pub(super) fn closure_frame_index(&self, instance: &ClosureInstance) -> u32 {
        self.closure_frames
            .get(instance)
            .copied()
            .expect("suspending closures have planned GC frames")
    }

    pub(super) fn closure_frame_tag(&self, instance: &ClosureInstance) -> u32 {
        self.closure_frame_tags
            .get(instance)
            .copied()
            .expect("suspending closures have runtime tags")
    }

    pub(super) fn leaf_frame_index(&self, instance: &LeafFutureInstance) -> u32 {
        self.leaf_frames
            .get(instance)
            .copied()
            .expect("reachable intrinsic futures have planned GC frames")
    }

    pub(super) fn leaf_frame_tag(&self, instance: &LeafFutureInstance) -> u32 {
        self.leaf_frame_tags
            .get(instance)
            .copied()
            .expect("reachable intrinsic futures have runtime tags")
    }

    pub(super) fn index(&self, ty: Type) -> u32 {
        match ty {
            Type::StateSnapshot => STATE_TYPE,
            Type::Async(future) => *self
                .async_values
                .get(&future)
                .expect("reachable async values have erased GC headers"),
            Type::Iterator(iterator) => *self
                .iterator_values
                .get(&iterator)
                .expect("reachable iterator values have erased GC headers"),
            Type::Standard(standard) => self.standard_index(standard),
            Type::Struct(_)
            | Type::ManagedClass(_)
            | Type::Enum(_)
            | Type::ArrayStorage(_)
            | Type::Array(_)
            | Type::Option(_)
            | Type::Result(_)
            | Type::Callable(_)
            | Type::Range(_)
            | Type::Set(_)
            | Type::Application(_) => *self
                .dynamic
                .get(&ty)
                .unwrap_or_else(|| panic!("dynamic GC type `{ty:?}` was not marked reachable")),
            _ => unreachable!("scalar types have no GC heap index"),
        }
    }

    pub(super) fn val_type(&self, ty: Type) -> ValType {
        match ty {
            Type::Never => unreachable!("the Never type has no WebAssembly value representation"),
            Type::None => ValType::Ref(RefType {
                nullable: true,
                heap_type: HeapType::Abstract {
                    shared: false,
                    ty: AbstractHeapType::None,
                },
            }),
            Type::Bool
            | Type::Char
            | Type::I8
            | Type::U8
            | Type::I16
            | Type::U16
            | Type::I32
            | Type::U32
            | Type::SettingsView => ValType::I32,
            Type::I64 | Type::U64 | Type::Address => ValType::I64,
            Type::F32 => ValType::F32,
            Type::F64 => ValType::F64,
            Type::StateSnapshot => ValType::Ref(RefType {
                nullable: true,
                heap_type: HeapType::Concrete(STATE_TYPE),
            }),
            Type::Standard(standard) => {
                match self.standard_library.type_decl(standard).representation {
                    RuntimeRepresentation::Scalar { storage } => {
                        self.val_type(Type::from_declared(DeclaredTypeRef::Core(storage)))
                    }
                    RuntimeRepresentation::GcArray { nullable, .. }
                    | RuntimeRepresentation::GcStruct { nullable, .. }
                    | RuntimeRepresentation::Enum { nullable } => ValType::Ref(RefType {
                        nullable,
                        heap_type: HeapType::Concrete(self.index(ty)),
                    }),
                }
            }
            Type::Struct(_)
            | Type::ManagedClass(_)
            | Type::Enum(_)
            | Type::ArrayStorage(_)
            | Type::Array(_)
            | Type::Option(_)
            | Type::Result(_)
            | Type::Async(_)
            | Type::Iterator(_)
            | Type::Callable(_)
            | Type::Range(_)
            | Type::Set(_)
            | Type::Application(_) => ValType::Ref(RefType {
                nullable: true,
                heap_type: HeapType::Concrete(self.index(ty)),
            }),
        }
    }

    pub(super) fn storage_type(&self, ty: Type) -> StorageType {
        match ty {
            Type::Bool | Type::I8 | Type::U8 => StorageType::I8,
            Type::I16 | Type::U16 => StorageType::I16,
            // Standalone `Never` values are erased, but aggregate shapes such
            // as `Never?` still need a legal, uninhabited payload field. Wasm's
            // bottom reference type is an exact physical representation for
            // that unreachable slot.
            Type::Never => StorageType::Val(ValType::Ref(RefType {
                nullable: true,
                heap_type: HeapType::Abstract {
                    shared: false,
                    ty: AbstractHeapType::None,
                },
            })),
            _ => StorageType::Val(self.val_type(ty)),
        }
    }

    /// Continuation frames are allocated before every live value has been
    /// produced. Their reference slots therefore need a nullable physical
    /// representation even when the source type is non-null; frame reads
    /// restore the source guarantee after control-flow proves initialization.
    pub(super) fn frame_storage_type(&self, ty: Type) -> StorageType {
        match self.storage_type(ty) {
            StorageType::Val(ValType::Ref(mut reference)) => {
                reference.nullable = true;
                StorageType::Val(ValType::Ref(reference))
            }
            storage => storage,
        }
    }

    /// Returns defaultable storage for an array element.
    ///
    /// Growable arrays allocate spare capacity with `array.new_default`.
    /// Standard-library GC values may be non-null at the source level, so their
    /// physical backing slots must be nullable until the logical length reaches
    /// them. Array reads restore the source-level non-null guarantee.
    pub(super) fn array_element_storage_type(&self, ty: Type) -> StorageType {
        match self.storage_type(ty) {
            StorageType::Val(ValType::Ref(mut reference)) => {
                reference.nullable = true;
                StorageType::Val(ValType::Ref(reference))
            }
            storage => storage,
        }
    }
}
