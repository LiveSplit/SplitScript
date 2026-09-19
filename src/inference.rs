//! Temporary types and reusable whole-program type inference.
//!
//! Source syntax never contains these types. The checker records constraints
//! here and publishes only fully resolved [`crate::types::TypeId`] values.

use std::{collections::HashMap, fmt, ops::BitOr};

use crate::{
    ast::{
        ArrayTypeId, AsyncTypeId, CallableTypeId, ConstructedTypeIdAllocator, IteratorTypeId,
        OptionTypeId, RangeKind, RangeTypeId, ResultTypeId, TypeApplicationId,
    },
    stdlib::{
        CapabilityBehavior, CoreTypeId, StandardLibrary, StdlibCapabilityId,
        StdlibTypeConstructorId, StdlibTypeId, TypeRef as CatalogTypeRef,
    },
    types::{BuiltinType, ResolvedTypeRef, TypeId, TypeKind, TypeStore},
};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub(crate) enum Type {
    Known(TypeId),
    Array(ArrayTypeId),
    Option(OptionTypeId),
    Result(ResultTypeId),
    Async(AsyncTypeId),
    Iterator(IteratorTypeId),
    Callable(CallableTypeId),
    Range(RangeTypeId),
    Set(TypeApplicationId),
    Application(TypeApplicationId),
    Variable(u32),
}

impl Type {
    /// Returns the source-level identity of a type when inference has resolved
    /// far enough to have one.
    pub(crate) fn try_to_ref(self, types: &TypeStore) -> Option<ResolvedTypeRef> {
        if matches!(self, Self::Variable(_)) {
            None
        } else {
            Some(self.to_ref(types))
        }
    }

    pub(crate) fn to_ref(self, types: &TypeStore) -> ResolvedTypeRef {
        match self {
            Self::Known(id) => match types.kind(id) {
                TypeKind::Error => ResolvedTypeRef::Error,
                TypeKind::Builtin(builtin) => ResolvedTypeRef::Core(*builtin),
                TypeKind::Standard(standard) => ResolvedTypeRef::Standard(*standard),
                TypeKind::StateSnapshot => ResolvedTypeRef::StateSnapshot,
                TypeKind::SettingsView => ResolvedTypeRef::SettingsView,
                TypeKind::Struct(structure) => ResolvedTypeRef::Struct(*structure),
                TypeKind::Enum(enumeration) => ResolvedTypeRef::Enum(*enumeration),
                TypeKind::ManagedClass(class) => ResolvedTypeRef::ManagedClass(*class),
                TypeKind::ManagedReference(class) => ResolvedTypeRef::ManagedReference(*class),
                TypeKind::GenericParameter { .. } => ResolvedTypeRef::GenericParameter(id),
                TypeKind::Array { layout, .. } => ResolvedTypeRef::Array(*layout),
                TypeKind::Option { layout, .. } => ResolvedTypeRef::Option(*layout),
                TypeKind::Result { layout, .. } => ResolvedTypeRef::Result(*layout),
                TypeKind::Async { layout, .. } => ResolvedTypeRef::Async(*layout),
                TypeKind::Iterator { layout, .. } => ResolvedTypeRef::Iterator(*layout),
                TypeKind::Callable { layout, .. } => ResolvedTypeRef::Callable(*layout),
                TypeKind::Range { layout, .. } => ResolvedTypeRef::Range(*layout),
                TypeKind::Set { layout, .. } => ResolvedTypeRef::Set(*layout),
                TypeKind::Application { layout, .. } => ResolvedTypeRef::Application(*layout),
            },
            Self::Array(id) => ResolvedTypeRef::Array(id),
            Self::Option(id) => ResolvedTypeRef::Option(id),
            Self::Result(id) => ResolvedTypeRef::Result(id),
            Self::Async(id) => ResolvedTypeRef::Async(id),
            Self::Iterator(id) => ResolvedTypeRef::Iterator(id),
            Self::Callable(id) => ResolvedTypeRef::Callable(id),
            Self::Range(id) => ResolvedTypeRef::Range(id),
            Self::Set(id) => ResolvedTypeRef::Set(id),
            Self::Application(id) => ResolvedTypeRef::Application(id),
            Self::Variable(variable) => {
                unreachable!("inference variable ?{variable} cannot become a source type reference")
            }
        }
    }
}

impl fmt::Display for Type {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Known(id) => write!(formatter, "type#{}", id.index()),
            Self::Array(id) => write!(formatter, "[T]#{id}"),
            Self::Option(id) => write!(formatter, "T?#{id}"),
            Self::Result(id) => write!(formatter, "T!#{id}"),
            Self::Async(id) => write!(formatter, "Async#{id}"),
            Self::Iterator(id) => write!(formatter, "Iterator#{id}"),
            Self::Callable(id) => write!(formatter, "Callable#{id}"),
            Self::Range(id) => write!(formatter, "Range#{id}"),
            Self::Set(id) => write!(formatter, "Set#{id}"),
            Self::Application(id) => write!(formatter, "Application#{id}"),
            Self::Variable(id) => write!(formatter, "?{id}"),
        }
    }
}

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub(crate) struct Requirements(Vec<StdlibCapabilityId>);

impl Requirements {
    pub(crate) fn none() -> Self {
        Self::default()
    }

    pub(crate) fn capability(capability: StdlibCapabilityId) -> Self {
        Self(vec![capability])
    }

    pub(crate) fn capabilities(capabilities: impl IntoIterator<Item = StdlibCapabilityId>) -> Self {
        let mut requirements = Self::none();
        for capability in capabilities {
            if !requirements.0.contains(&capability) {
                requirements.0.push(capability);
            }
        }
        requirements
    }

    pub(crate) fn is_empty(&self) -> bool {
        self.0.is_empty()
    }

    pub(crate) fn as_slice(&self) -> &[StdlibCapabilityId] {
        &self.0
    }

    fn contains(&self, capability: StdlibCapabilityId) -> bool {
        self.0.contains(&capability)
    }
}

impl BitOr for Requirements {
    type Output = Self;

    fn bitor(mut self, rhs: Self) -> Self::Output {
        for capability in rhs.0 {
            if !self.0.contains(&capability) {
                self.0.push(capability);
            }
        }
        self
    }
}

#[derive(Debug, Clone)]
pub(crate) enum InferenceError {
    Message(String),
    TypeMismatch {
        left: Type,
        right: Type,
    },
    UnsupportedOperation {
        ty: Type,
        requirements: Requirements,
    },
    UnsatisfiedConstraints {
        ty: Type,
        requirements: Requirements,
    },
    IntegerLiteralOutOfRange(Type),
}

#[derive(Debug, Clone)]
struct Variable {
    parent: u32,
    binding: Option<Type>,
    requirements: Requirements,
    largest_literal: Option<u64>,
    largest_negative_literal: Option<u64>,
    literal_default: Option<LiteralDefault>,
}

/// Equality between an associated type and a normal inference variable.
///
/// Keeping the projected value as an ordinary variable lets all existing
/// bidirectional inference continue to constrain it. The receiver relation is
/// retained separately so generic functions can instantiate `T.Item` from a
/// concrete `T` at each call site.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) struct AssociatedProjection {
    pub(crate) receiver: u32,
    pub(crate) output: u32,
    pub(crate) capability: StdlibCapabilityId,
    pub(crate) name: &'static str,
}

/// A projection whose receiver was already a concrete source-defined type.
///
/// Catalog types resolve their associated types immediately. Source types are
/// proven structurally after user method bodies have been inferred, so these
/// projections retain the ordinary output variable until that proof is
/// available.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct ConcreteAssociatedProjection {
    receiver: TypeId,
    output: u32,
    capability: StdlibCapabilityId,
    name: &'static str,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) struct SourceAssociatedType {
    pub(crate) receiver: TypeId,
    pub(crate) capability: StdlibCapabilityId,
    pub(crate) name: &'static str,
    pub(crate) value: Type,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum LiteralDefault {
    Integer,
    Float,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum LiteralKind {
    Integer,
    Float,
}

impl LiteralDefault {
    fn merge(left: Option<Self>, right: Option<Self>) -> Option<Self> {
        match (left, right) {
            (Some(Self::Float), _) | (_, Some(Self::Float)) => Some(Self::Float),
            (Some(Self::Integer), _) | (_, Some(Self::Integer)) => Some(Self::Integer),
            (None, None) => None,
        }
    }
}

#[derive(Debug, Clone, Copy)]
pub(crate) struct ArrayLayout {
    pub(crate) id: ArrayTypeId,
    pub(crate) element: Type,
    pub(crate) shape: ArrayShape,
}

/// The statically known shape of an array value.
///
/// Shape variables are deliberately separate from ordinary type variables:
/// an array can have a fully known element type while its exact-length versus
/// growable refinement is still being inferred through a function signature.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub(crate) enum ArrayShape {
    Growable,
    Exact(u32),
    Variable(u32),
}

#[derive(Debug, Clone, Copy)]
struct ArrayShapeVariable {
    parent: u32,
    binding: Option<ArrayShape>,
}

#[derive(Debug, Clone, Copy)]
pub(crate) struct OptionLayout {
    pub(crate) id: OptionTypeId,
    pub(crate) value: Type,
}

#[derive(Debug, Clone, Copy)]
pub(crate) struct ResultLayout {
    pub(crate) id: ResultTypeId,
    pub(crate) value: Type,
}

#[derive(Debug, Clone, Copy)]
pub(crate) struct AsyncLayout {
    pub(crate) id: AsyncTypeId,
    pub(crate) value: Type,
}

#[derive(Debug, Clone, Copy)]
pub(crate) struct IteratorLayout {
    pub(crate) id: IteratorTypeId,
    pub(crate) item: Type,
}

#[derive(Debug, Clone)]
pub(crate) struct CallableLayout {
    pub(crate) id: CallableTypeId,
    pub(crate) parameters: Vec<Type>,
    pub(crate) result: Type,
}

#[derive(Debug, Clone, Copy)]
pub(crate) struct RangeLayout {
    pub(crate) id: RangeTypeId,
    pub(crate) lower: Type,
    pub(crate) upper: Type,
    pub(crate) kind: RangeKind,
}

#[derive(Debug, Clone, Copy)]
pub(crate) struct SetLayout {
    pub(crate) id: TypeApplicationId,
    pub(crate) element: Type,
    pub(crate) backing: Option<ArrayTypeId>,
}

#[derive(Debug, Clone)]
pub(crate) struct ApplicationLayout {
    pub(crate) id: TypeApplicationId,
    pub(crate) constructor: StdlibTypeConstructorId,
    pub(crate) arguments: Vec<Type>,
}

#[derive(Default)]
pub(crate) struct ConstructedLayouts {
    pub(crate) arrays: Vec<ArrayLayout>,
    pub(crate) options: Vec<OptionLayout>,
    pub(crate) results: Vec<ResultLayout>,
    pub(crate) asyncs: Vec<AsyncLayout>,
    pub(crate) iterators: Vec<IteratorLayout>,
    pub(crate) callables: Vec<CallableLayout>,
    pub(crate) ranges: Vec<RangeLayout>,
    pub(crate) sets: Vec<SetLayout>,
    pub(crate) applications: Vec<ApplicationLayout>,
}

pub(crate) struct InferenceContext {
    standard_library: StandardLibrary,
    types: TypeStore,
    variables: Vec<Variable>,
    array_shape_variables: Vec<ArrayShapeVariable>,
    associated_projections: Vec<AssociatedProjection>,
    concrete_associated_projections: Vec<ConcreteAssociatedProjection>,
    source_associated_types: HashMap<(TypeId, StdlibCapabilityId, &'static str), Type>,
    arrays: Vec<ArrayLayout>,
    options: Vec<OptionLayout>,
    results: Vec<ResultLayout>,
    asyncs: Vec<AsyncLayout>,
    iterators: Vec<IteratorLayout>,
    callables: Vec<CallableLayout>,
    ranges: Vec<RangeLayout>,
    sets: Vec<SetLayout>,
    applications: Vec<ApplicationLayout>,
    canonical_arrays: HashMap<ArrayTypeId, ArrayTypeId>,
    canonical_options: HashMap<OptionTypeId, OptionTypeId>,
    canonical_results: HashMap<ResultTypeId, ResultTypeId>,
    canonical_asyncs: HashMap<AsyncTypeId, AsyncTypeId>,
    canonical_iterators: HashMap<IteratorTypeId, IteratorTypeId>,
    canonical_callables: HashMap<CallableTypeId, CallableTypeId>,
    canonical_ranges: HashMap<RangeTypeId, RangeTypeId>,
    canonical_sets: HashMap<TypeApplicationId, TypeApplicationId>,
    canonical_applications: HashMap<TypeApplicationId, TypeApplicationId>,
    constructed_types: HashMap<Type, TypeId>,
    constructed_type_ids: ConstructedTypeIdAllocator,
}

impl InferenceContext {
    pub(crate) fn new(
        standard_library: StandardLibrary,
        types: TypeStore,
        first_constructed_type_index: u32,
        layouts: ConstructedLayouts,
    ) -> Self {
        let ConstructedLayouts {
            mut arrays,
            options,
            results,
            asyncs,
            iterators,
            callables,
            ranges,
            mut sets,
            applications,
        } = layouts;
        let next_constructed_type_index = arrays
            .iter()
            .map(|layout| layout.id.index() as u32 + 1)
            .chain(options.iter().map(|layout| layout.id.index() as u32 + 1))
            .chain(results.iter().map(|layout| layout.id.index() as u32 + 1))
            .chain(asyncs.iter().map(|layout| layout.id.index() as u32 + 1))
            .chain(iterators.iter().map(|layout| layout.id.index() as u32 + 1))
            .chain(callables.iter().map(|layout| layout.id.index() as u32 + 1))
            .chain(ranges.iter().map(|layout| layout.id.index() as u32 + 1))
            .chain(sets.iter().map(|layout| layout.id.index() as u32 + 1))
            .chain(
                applications
                    .iter()
                    .map(|layout| layout.id.index() as u32 + 1),
            )
            .fold(first_constructed_type_index, u32::max);
        let mut constructed_type_ids =
            ConstructedTypeIdAllocator::starting_at(next_constructed_type_index);
        for set in &mut sets {
            let backing = arrays
                .iter()
                .find(|array| array.element == set.element && array.shape == ArrayShape::Growable)
                .map(|array| array.id)
                .unwrap_or_else(|| {
                    let id = constructed_type_ids.array();
                    arrays.push(ArrayLayout {
                        id,
                        element: set.element,
                        shape: ArrayShape::Growable,
                    });
                    id
                });
            set.backing = Some(backing);
        }
        Self {
            standard_library,
            types,
            variables: Vec::new(),
            array_shape_variables: Vec::new(),
            associated_projections: Vec::new(),
            concrete_associated_projections: Vec::new(),
            source_associated_types: HashMap::new(),
            arrays,
            options,
            results,
            asyncs,
            iterators,
            callables,
            ranges,
            sets,
            applications,
            canonical_arrays: HashMap::new(),
            canonical_options: HashMap::new(),
            canonical_results: HashMap::new(),
            canonical_asyncs: HashMap::new(),
            canonical_iterators: HashMap::new(),
            canonical_callables: HashMap::new(),
            canonical_ranges: HashMap::new(),
            canonical_sets: HashMap::new(),
            canonical_applications: HashMap::new(),
            constructed_types: HashMap::new(),
            constructed_type_ids,
        }
    }

    pub(crate) fn known_standard(&self, standard: StdlibTypeId) -> Type {
        Type::Known(self.types.id_for_standard(standard))
    }

    pub(crate) fn known_core(&self, core: CoreTypeId) -> Type {
        Type::Known(self.types.id_for_core(core))
    }

    pub(crate) fn known_builtin(&self, builtin: BuiltinType) -> Type {
        Type::Known(self.types.id_for_builtin(builtin))
    }

    pub(crate) fn type_store(&self) -> &TypeStore {
        &self.types
    }

    /// Produces the semantic poison type used after an already-diagnosed
    /// expression failure. It preserves declaration identity for later editor
    /// queries without making the failed program eligible for code generation.
    pub(crate) fn error_type(&mut self) -> Type {
        Type::Known(self.types.id_for_error())
    }

    pub(crate) fn is_error_type(&mut self, ty: Type) -> bool {
        matches!(
            self.shallow(ty),
            Type::Known(id) if matches!(self.types.kind(id), TypeKind::Error)
        )
    }

    pub(crate) fn is_never_type(&mut self, ty: Type) -> bool {
        matches!(
            self.shallow(ty),
            Type::Known(id)
                if matches!(
                    self.types.kind(id),
                    TypeKind::Builtin(CoreTypeId::Never)
                )
        )
    }

    pub(crate) fn standard_type(&self, ty: Type) -> Option<StdlibTypeId> {
        let Type::Known(id) = ty else {
            return None;
        };
        match self.types.kind(id) {
            TypeKind::Standard(standard) => Some(*standard),
            TypeKind::SettingsView => Some(StdlibTypeId::SettingsView),
            _ => None,
        }
    }

    pub(crate) fn type_may_have_capability(
        &self,
        ty: Type,
        capability: StdlibCapabilityId,
    ) -> bool {
        type_may_have_capability(&self.standard_library, &self.types, ty, capability)
    }

    pub(crate) fn is_integer(&self, ty: Type) -> bool {
        self.type_may_have_capability(ty, StdlibCapabilityId::Integer)
    }

    pub(crate) fn is_numeric(&self, ty: Type) -> bool {
        self.type_may_have_capability(ty, StdlibCapabilityId::Numeric)
    }

    pub(crate) fn fits_unsigned_literal(&self, value: u64, ty: Type) -> bool {
        fits_unsigned_literal(&self.types, value, ty)
    }

    pub(crate) fn fits_negative_literal(&self, value: u64, ty: Type) -> bool {
        fits_negative_literal(&self.types, value, ty)
    }

    pub(crate) fn known_type_name(&self, id: TypeId) -> String {
        match self.types.kind(id) {
            TypeKind::Error => "<unknown>".into(),
            TypeKind::Builtin(builtin) => builtin.to_string(),
            TypeKind::Standard(standard) => self.standard_library.type_decl(*standard).name.into(),
            TypeKind::StateSnapshot => "StateSnapshot".into(),
            TypeKind::SettingsView => "SettingsView".into(),
            TypeKind::Struct(structure) => format!("struct#{structure}"),
            TypeKind::Enum(enumeration) => format!("enum#{enumeration}"),
            TypeKind::ManagedClass(class) => format!("class#{class}"),
            TypeKind::ManagedReference(class) => format!("class#{class}.Ref"),
            TypeKind::GenericParameter { index, .. } => {
                crate::types::generic_parameter_name(*index)
            }
            TypeKind::Array {
                element, length, ..
            } => match length {
                Some(length) => format!("[{}; {length}]", self.known_type_name(*element)),
                None => format!("[{}]", self.known_type_name(*element)),
            },
            TypeKind::Set { element, .. } => {
                format!("Set<{}>", self.known_type_name(*element))
            }
            TypeKind::Application {
                constructor,
                arguments,
                ..
            } => {
                let name = self.standard_library.type_constructor(*constructor).name;
                let arguments = arguments
                    .iter()
                    .map(|argument| self.known_type_name(*argument))
                    .collect::<Vec<_>>()
                    .join(", ");
                format!("{name}<{arguments}>")
            }
            TypeKind::Option { value, .. } => format!("{}?", self.known_type_name(*value)),
            TypeKind::Result { value, .. } => format!("{}!", self.known_type_name(*value)),
            TypeKind::Async { value, .. } => format!("async {}", self.known_type_name(*value)),
            TypeKind::Iterator { item, .. } => {
                format!("iterator {}", self.known_type_name(*item))
            }
            TypeKind::Callable {
                parameters, result, ..
            } => {
                let parameters = parameters
                    .iter()
                    .map(|parameter| self.known_type_name(*parameter))
                    .collect::<Vec<_>>()
                    .join(", ");
                format!("({parameters}) -> {}", self.known_type_name(*result))
            }
            TypeKind::Range { bound, kind, .. } => {
                let bound = self.known_type_name(*bound);
                format!("{bound}{}{bound}", kind.operator())
            }
        }
    }

    pub(crate) fn fresh(
        &mut self,
        requirements: Requirements,
        largest_literal: Option<u64>,
    ) -> Type {
        let literal_default = largest_literal.map(|_| LiteralDefault::Integer);
        self.fresh_with_literal_default(requirements, largest_literal, None, literal_default)
    }

    pub(crate) fn fresh_negative_integer_literal(
        &mut self,
        requirements: Requirements,
        magnitude: u64,
    ) -> Type {
        self.fresh_with_literal_default(
            requirements,
            None,
            Some(magnitude),
            Some(LiteralDefault::Integer),
        )
    }

    pub(crate) fn fresh_float_literal(&mut self) -> Type {
        self.fresh_with_literal_default(
            Requirements::capability(StdlibCapabilityId::Float),
            None,
            None,
            Some(LiteralDefault::Float),
        )
    }

    fn fresh_with_literal_default(
        &mut self,
        requirements: Requirements,
        largest_literal: Option<u64>,
        largest_negative_literal: Option<u64>,
        literal_default: Option<LiteralDefault>,
    ) -> Type {
        let requirements = Requirements(
            self.standard_library
                .minimal_capabilities(requirements.as_slice()),
        );
        let id = self.variables.len() as u32;
        self.variables.push(Variable {
            parent: id,
            binding: None,
            requirements,
            largest_literal,
            largest_negative_literal,
            literal_default,
        });
        Type::Variable(id)
    }

    pub(crate) fn shallow(&mut self, ty: Type) -> Type {
        let Type::Variable(id) = ty else {
            return ty;
        };
        let root = self.root(id);
        if let Some(binding) = self.variables[root as usize].binding {
            self.shallow(binding)
        } else {
            Type::Variable(root)
        }
    }

    /// Resolves the current outer inference binding without path compression.
    ///
    /// Read-only semantic passes may need to recognize a type that has already
    /// been constrained while inference is still in progress. An unbound
    /// variable remains a variable and must not be converted into a source
    /// type reference.
    pub(crate) fn shallow_readonly(&self, ty: Type) -> Type {
        let Type::Variable(id) = ty else {
            return ty;
        };
        let root = self.root_without_compression(id);
        if let Some(binding) = self.variables[root as usize].binding {
            self.shallow_readonly(binding)
        } else {
            Type::Variable(root)
        }
    }

    /// Projects one associated type from a value constrained by `capability`.
    /// Concrete built-in constructors resolve immediately. An unresolved
    /// generic receiver retains an equality relation that is solved when a
    /// call site supplies its concrete iterable type.
    pub(crate) fn associated_type(
        &mut self,
        receiver: Type,
        capability: StdlibCapabilityId,
        name: &'static str,
    ) -> Type {
        let receiver = self.shallow(receiver);
        let value = if let Some(value) = self.concrete_associated_type(receiver, capability, name) {
            value
        } else {
            match receiver {
                Type::Variable(receiver) => {
                    let receiver = self.root(receiver);
                    if let Some(projection) =
                        self.associated_projections.iter().find(|projection| {
                            projection.receiver == receiver
                                && projection.capability == capability
                                && projection.name == name
                        })
                    {
                        Type::Variable(self.root_without_compression(projection.output))
                    } else {
                        let Type::Variable(output) = self.fresh(Requirements::none(), None) else {
                            unreachable!("fresh inference values are variables")
                        };
                        self.associated_projections.push(AssociatedProjection {
                            receiver,
                            output,
                            capability,
                            name,
                        });
                        Type::Variable(output)
                    }
                }
                Type::Known(receiver)
                    if matches!(
                        self.types.kind(receiver),
                        TypeKind::Struct(_) | TypeKind::Enum(_)
                    ) =>
                {
                    if let Some(projection) =
                        self.concrete_associated_projections
                            .iter()
                            .find(|projection| {
                                projection.receiver == receiver
                                    && projection.capability == capability
                                    && projection.name == name
                            })
                    {
                        Type::Variable(self.root_without_compression(projection.output))
                    } else {
                        let Type::Variable(output) = self.fresh(Requirements::none(), None) else {
                            unreachable!("fresh inference values are variables")
                        };
                        self.concrete_associated_projections
                            .push(ConcreteAssociatedProjection {
                                receiver,
                                output,
                                capability,
                                name,
                            });
                        Type::Variable(output)
                    }
                }
                _ => return self.error_type(),
            }
        };
        let constraints = self
            .standard_library
            .capability(capability)
            .associated_types
            .iter()
            .find(|associated| associated.name == name)
            .expect("resolved associated types are declared by their capability")
            .constraints;
        let requirements = constraints
            .iter()
            .fold(Requirements::none(), |requirements, constraint| {
                requirements | Requirements::capability(*constraint)
            });
        if self.require(value, requirements).is_err() {
            // Catalog validation guarantees that concrete definitions satisfy
            // their declared associated-type constraints. Preserve editor
            // analysis if a future structural implementation is incomplete.
            return self.error_type();
        }
        value
    }

    /// Records one associated type proven from a source type's complete method
    /// contract. The value remains an inference type until ordinary
    /// finalization so return-type and wrapper inference can participate.
    pub(crate) fn define_source_associated_type(
        &mut self,
        receiver: TypeId,
        capability: StdlibCapabilityId,
        name: &'static str,
        value: Type,
    ) -> Result<(), InferenceError> {
        let key = (receiver, capability, name);
        if let Some(previous) = self.source_associated_types.get(&key).copied() {
            self.unify_deferred(previous, value)?;
        } else {
            self.source_associated_types.insert(key, value);
        }
        self.solve_associated_projections()
    }

    pub(crate) fn source_associated_types(
        &self,
    ) -> impl Iterator<Item = SourceAssociatedType> + '_ {
        self.source_associated_types
            .iter()
            .map(
                |(&(receiver, capability, name), &value)| SourceAssociatedType {
                    receiver,
                    capability,
                    name,
                    value,
                },
            )
    }

    /// Resolves every projection whose source implementation was proven and
    /// poisons only the outputs of missing implementations. Semantic
    /// capability validation subsequently reports the missing or mismatched
    /// method at the actual use site; inference recovery must not replace that
    /// precise diagnostic with a span-less unbound-variable error.
    pub(crate) fn finish_associated_projections(&mut self) {
        let _ = self.solve_associated_projections();
        let error = Type::Known(self.types.id_for_error());
        for projection in self.concrete_associated_projections.clone() {
            if self
                .concrete_associated_type(
                    Type::Known(projection.receiver),
                    projection.capability,
                    projection.name,
                )
                .is_none()
            {
                let output = self.root(projection.output);
                self.variables[output as usize].binding = Some(error);
            }
        }
        // A generic projection can receive the output of a concrete source
        // projection (for example `Iterable.Iterator.Item`). Resolve the
        // concrete layer first so recovery propagates through the chain.
        for _ in 0..=self.associated_projections.len() {
            let mut changed = false;
            for projection in self.associated_projections.clone() {
                let receiver = self.shallow(Type::Variable(projection.receiver));
                if !matches!(receiver, Type::Variable(_))
                    && self
                        .concrete_associated_type(receiver, projection.capability, projection.name)
                        .is_none()
                {
                    let output = self.root(projection.output);
                    if self.variables[output as usize].binding.is_none() {
                        self.variables[output as usize].binding = Some(error);
                        changed = true;
                    }
                }
            }
            if !changed {
                break;
            }
        }
    }

    pub(crate) fn associated_projections_for(
        &mut self,
        receivers: &[u32],
    ) -> Vec<AssociatedProjection> {
        let receiver_roots = receivers
            .iter()
            .map(|receiver| self.root(*receiver))
            .collect::<Vec<_>>();
        self.associated_projections
            .clone()
            .into_iter()
            .filter_map(|mut projection| {
                projection.receiver = self.root(projection.receiver);
                projection.output = self.root(projection.output);
                receiver_roots
                    .contains(&projection.receiver)
                    .then_some(projection)
            })
            .collect()
    }

    fn root_without_compression(&self, mut id: u32) -> u32 {
        loop {
            let parent = self.variables[id as usize].parent;
            if parent == id {
                return id;
            }
            id = parent;
        }
    }

    fn concrete_associated_type(
        &mut self,
        receiver: Type,
        capability: StdlibCapabilityId,
        name: &'static str,
    ) -> Option<Type> {
        let receiver = self.shallow(receiver);
        if let Type::Known(receiver) = receiver
            && let Some(value) = self
                .source_associated_types
                .get(&(receiver, capability, name))
                .copied()
        {
            return Some(value);
        }
        if let Type::Iterator(iterator) = receiver {
            return match (capability, name) {
                (StdlibCapabilityId::Iterator, "Item") => Some(self.iterator_item(iterator)),
                (StdlibCapabilityId::Iterable, "Iterator") => Some(Type::Iterator(iterator)),
                _ => None,
            };
        }
        if let Some(standard) = self.standard_type(receiver) {
            if !self
                .standard_library
                .type_has_capability(standard, capability)
            {
                return None;
            }
            let value = self
                .standard_library
                .type_decl(standard)
                .associated_types
                .iter()
                .find(|associated| associated.name == name)?
                .value;
            return Some(self.catalog_type(value, &HashMap::new()));
        }
        let (constructor, arguments) = self.type_constructor_arguments(receiver)?;
        if !self
            .standard_library
            .type_constructor_has_capability(constructor, capability)
        {
            return None;
        }
        let declaration = self.standard_library.type_constructor(constructor);
        let value = declaration
            .associated_types
            .iter()
            .find(|associated| associated.name == name)?
            .value;
        let variables = declaration
            .parameters
            .iter()
            .zip(arguments)
            .map(|(parameter, argument)| (parameter.name, argument))
            .collect::<HashMap<_, _>>();
        Some(self.catalog_type(value, &variables))
    }

    fn type_constructor_arguments(
        &mut self,
        receiver: Type,
    ) -> Option<(StdlibTypeConstructorId, Vec<Type>)> {
        match self.shallow(receiver) {
            Type::Array(array) => Some((
                StdlibTypeConstructorId::Array,
                vec![self.array_element(array)],
            )),
            Type::Option(option) => Some((
                StdlibTypeConstructorId::Option,
                vec![self.option_value(option)],
            )),
            Type::Result(result) => Some((
                StdlibTypeConstructorId::Result,
                vec![self.result_value(result)],
            )),
            Type::Set(set) => Some((StdlibTypeConstructorId::Set, vec![self.set_element(set)])),
            Type::Range(range) => Some((
                match self.range_kind(range) {
                    RangeKind::Exclusive => StdlibTypeConstructorId::ExclusiveRange,
                    RangeKind::Inclusive => StdlibTypeConstructorId::InclusiveRange,
                },
                vec![self.range_bound(range)],
            )),
            Type::Application(application) => Some((
                self.application_constructor(application),
                self.application_arguments(application).to_vec(),
            )),
            Type::Known(_)
            | Type::Async(_)
            | Type::Iterator(_)
            | Type::Callable(_)
            | Type::Variable(_) => None,
        }
    }

    fn catalog_type(
        &mut self,
        ty: CatalogTypeRef,
        variables: &HashMap<&'static str, Type>,
    ) -> Type {
        match ty {
            CatalogTypeRef::Core(core) => self.known_core(core),
            CatalogTypeRef::Standard(standard) => self.known_standard(standard),
            CatalogTypeRef::Parameter(name) | CatalogTypeRef::Associated(name) => variables[name],
            CatalogTypeRef::Async(value) => {
                let value = self.catalog_type(*value, variables);
                Type::Async(self.async_type(value))
            }
            CatalogTypeRef::Iterator(item) => {
                let item = self.catalog_type(*item, variables);
                Type::Iterator(self.iterator_type(item))
            }
            CatalogTypeRef::FixedArray { element, length } => {
                let element = self.catalog_type(*element, variables);
                Type::Array(self.array_type_with_length(element, Some(length)))
            }
            CatalogTypeRef::Callable { parameters, result } => {
                let parameters = parameters
                    .iter()
                    .map(|parameter| self.catalog_type(*parameter, variables))
                    .collect();
                let result = self.catalog_type(*result, variables);
                Type::Callable(self.callable_type(parameters, result))
            }
            CatalogTypeRef::Application {
                constructor,
                arguments,
            } => {
                let arguments = arguments
                    .iter()
                    .map(|argument| self.catalog_type(*argument, variables))
                    .collect::<Vec<_>>();
                self.catalog_application_type(constructor, arguments)
            }
        }
    }

    fn catalog_application_type(
        &mut self,
        constructor: StdlibTypeConstructorId,
        arguments: Vec<Type>,
    ) -> Type {
        let [value] = arguments.as_slice() else {
            return Type::Application(self.application_type(constructor, arguments));
        };
        let value = *value;
        match constructor {
            // Standard-library array operations are shape-polymorphic by
            // default. Structural mutators add the growable constraint during
            // call resolution; read-only operations remain usable on `[T; N]`.
            StdlibTypeConstructorId::Array => Type::Array(self.inferred_array_type(value)),
            StdlibTypeConstructorId::Option => Type::Option(self.option_type(value)),
            StdlibTypeConstructorId::Result => Type::Result(self.result_type(value)),
            StdlibTypeConstructorId::Set => Type::Set(self.set_type(value)),
            StdlibTypeConstructorId::ExclusiveRange => {
                Type::Range(self.range_type(value, RangeKind::Exclusive))
            }
            StdlibTypeConstructorId::InclusiveRange => {
                Type::Range(self.range_type(value, RangeKind::Inclusive))
            }
            _ => Type::Application(self.application_type(constructor, arguments)),
        }
    }

    /// Instantiates one type from a generalized function signature.
    ///
    /// Every generalized root receives one fresh variable per call. Reusing
    /// `substitutions` across all parameters and the result preserves equality
    /// relationships such as `(T, T) -> T`, including when `T` is nested in a
    /// constructed GC type.
    #[cfg(test)]
    pub(crate) fn instantiate_type(
        &mut self,
        ty: Type,
        generalized: &[u32],
        substitutions: &mut HashMap<u32, Type>,
    ) -> Type {
        self.instantiate_type_with_array_shapes(
            ty,
            generalized,
            &[],
            substitutions,
            &mut HashMap::new(),
        )
    }

    pub(crate) fn instantiate_type_with_array_shapes(
        &mut self,
        ty: Type,
        generalized: &[u32],
        generalized_shapes: &[u32],
        substitutions: &mut HashMap<u32, Type>,
        shape_substitutions: &mut HashMap<u32, ArrayShape>,
    ) -> Type {
        match self.shallow(ty) {
            Type::Variable(variable) if generalized.contains(&variable) => {
                if let Some(substitution) = substitutions.get(&variable) {
                    return *substitution;
                }
                let template = self.variables[variable as usize].clone();
                let substitution = self.fresh_with_literal_default(
                    template.requirements,
                    template.largest_literal,
                    template.largest_negative_literal,
                    template.literal_default,
                );
                substitutions.insert(variable, substitution);
                substitution
            }
            variable @ Type::Variable(_) => variable,
            Type::Array(array) => {
                let element = self.array_element(array);
                let instantiated = self.instantiate_type_with_array_shapes(
                    element,
                    generalized,
                    generalized_shapes,
                    substitutions,
                    shape_substitutions,
                );
                let shape = self.array_shape(array);
                let instantiated_shape = match shape {
                    ArrayShape::Variable(variable) if generalized_shapes.contains(&variable) => {
                        *shape_substitutions
                            .entry(variable)
                            .or_insert_with(|| self.fresh_array_shape())
                    }
                    shape => shape,
                };
                if instantiated == element && instantiated_shape == shape {
                    Type::Array(array)
                } else {
                    Type::Array(self.array_type_with_shape(instantiated, instantiated_shape))
                }
            }
            Type::Set(set) => {
                let element = self.set_element(set);
                let instantiated = self.instantiate_type_with_array_shapes(
                    element,
                    generalized,
                    generalized_shapes,
                    substitutions,
                    shape_substitutions,
                );
                if instantiated == element {
                    Type::Set(set)
                } else {
                    Type::Set(self.set_type(instantiated))
                }
            }
            Type::Application(application) => {
                let constructor = self.application_constructor(application);
                let arguments = self.application_arguments(application).to_vec();
                let instantiated = arguments
                    .iter()
                    .map(|argument| {
                        self.instantiate_type_with_array_shapes(
                            *argument,
                            generalized,
                            generalized_shapes,
                            substitutions,
                            shape_substitutions,
                        )
                    })
                    .collect::<Vec<_>>();
                if instantiated == arguments {
                    Type::Application(application)
                } else {
                    Type::Application(self.application_type(constructor, instantiated))
                }
            }
            Type::Option(option) => {
                let value = self.option_value(option);
                let instantiated = self.instantiate_type_with_array_shapes(
                    value,
                    generalized,
                    generalized_shapes,
                    substitutions,
                    shape_substitutions,
                );
                if instantiated == value {
                    Type::Option(option)
                } else {
                    Type::Option(self.option_type(instantiated))
                }
            }
            Type::Result(result) => {
                let value = self.result_value(result);
                let instantiated = self.instantiate_type_with_array_shapes(
                    value,
                    generalized,
                    generalized_shapes,
                    substitutions,
                    shape_substitutions,
                );
                if instantiated == value {
                    Type::Result(result)
                } else {
                    Type::Result(self.result_type(instantiated))
                }
            }
            Type::Async(future) => {
                let value = self.async_value(future);
                let instantiated = self.instantiate_type_with_array_shapes(
                    value,
                    generalized,
                    generalized_shapes,
                    substitutions,
                    shape_substitutions,
                );
                if instantiated == value {
                    Type::Async(future)
                } else {
                    Type::Async(self.async_type(instantiated))
                }
            }
            Type::Iterator(iterator) => {
                let item = self.iterator_item(iterator);
                let instantiated = self.instantiate_type_with_array_shapes(
                    item,
                    generalized,
                    generalized_shapes,
                    substitutions,
                    shape_substitutions,
                );
                if instantiated == item {
                    Type::Iterator(iterator)
                } else {
                    Type::Iterator(self.iterator_type(instantiated))
                }
            }
            Type::Callable(callable) => {
                let parameters = self.callable_parameters(callable).to_vec();
                let result = self.callable_result(callable);
                let instantiated_parameters = parameters
                    .iter()
                    .map(|parameter| {
                        self.instantiate_type_with_array_shapes(
                            *parameter,
                            generalized,
                            generalized_shapes,
                            substitutions,
                            shape_substitutions,
                        )
                    })
                    .collect::<Vec<_>>();
                let instantiated_result = self.instantiate_type_with_array_shapes(
                    result,
                    generalized,
                    generalized_shapes,
                    substitutions,
                    shape_substitutions,
                );
                if instantiated_parameters == parameters && instantiated_result == result {
                    Type::Callable(callable)
                } else {
                    Type::Callable(self.callable_type(instantiated_parameters, instantiated_result))
                }
            }
            Type::Range(range) => {
                let bound = self.range_bound(range);
                let instantiated = self.instantiate_type_with_array_shapes(
                    bound,
                    generalized,
                    generalized_shapes,
                    substitutions,
                    shape_substitutions,
                );
                if instantiated == bound {
                    Type::Range(range)
                } else {
                    Type::Range(self.range_type(instantiated, self.range_kind(range)))
                }
            }
            known @ Type::Known(_) => known,
        }
    }

    pub(crate) fn unbound_variables_in(
        &mut self,
        types: impl IntoIterator<Item = Type>,
    ) -> Vec<u32> {
        let mut variables = Vec::new();
        for ty in types {
            self.collect_unbound_variables(ty, &mut variables);
        }
        variables
    }

    fn collect_unbound_variables(&mut self, ty: Type, output: &mut Vec<u32>) {
        match self.shallow(ty) {
            Type::Variable(variable) => {
                if !output.contains(&variable) {
                    output.push(variable);
                }
            }
            Type::Array(array) => self.collect_unbound_variables(self.array_element(array), output),
            Type::Set(set) => self.collect_unbound_variables(self.set_element(set), output),
            Type::Application(application) => {
                for argument in self.application_arguments(application).to_vec() {
                    self.collect_unbound_variables(argument, output);
                }
            }
            Type::Option(option) => {
                self.collect_unbound_variables(self.option_value(option), output)
            }
            Type::Result(result) => {
                self.collect_unbound_variables(self.result_value(result), output)
            }
            Type::Async(future) => self.collect_unbound_variables(self.async_value(future), output),
            Type::Iterator(iterator) => {
                self.collect_unbound_variables(self.iterator_item(iterator), output)
            }
            Type::Callable(callable) => {
                for parameter in self.callable_parameters(callable).to_vec() {
                    self.collect_unbound_variables(parameter, output);
                }
                self.collect_unbound_variables(self.callable_result(callable), output);
            }
            Type::Range(range) => self.collect_unbound_variables(self.range_bound(range), output),
            Type::Known(_) => {}
        }
    }

    pub(crate) fn unbound_array_shapes_in(
        &mut self,
        types: impl IntoIterator<Item = Type>,
    ) -> Vec<u32> {
        let mut shapes = Vec::new();
        for ty in types {
            self.collect_unbound_array_shapes(ty, &mut shapes);
        }
        shapes
    }

    fn collect_unbound_array_shapes(&mut self, ty: Type, output: &mut Vec<u32>) {
        match self.shallow(ty) {
            Type::Array(array) => {
                if let ArrayShape::Variable(variable) = self.array_shape(array)
                    && !output.contains(&variable)
                {
                    output.push(variable);
                }
                self.collect_unbound_array_shapes(self.array_element(array), output);
            }
            Type::Set(set) => self.collect_unbound_array_shapes(self.set_element(set), output),
            Type::Application(application) => {
                for argument in self.application_arguments(application).to_vec() {
                    self.collect_unbound_array_shapes(argument, output);
                }
            }
            Type::Option(option) => {
                self.collect_unbound_array_shapes(self.option_value(option), output)
            }
            Type::Result(result) => {
                self.collect_unbound_array_shapes(self.result_value(result), output)
            }
            Type::Async(future) => {
                self.collect_unbound_array_shapes(self.async_value(future), output)
            }
            Type::Iterator(iterator) => {
                self.collect_unbound_array_shapes(self.iterator_item(iterator), output)
            }
            Type::Callable(callable) => {
                for parameter in self.callable_parameters(callable).to_vec() {
                    self.collect_unbound_array_shapes(parameter, output);
                }
                self.collect_unbound_array_shapes(self.callable_result(callable), output);
            }
            Type::Range(range) => {
                self.collect_unbound_array_shapes(self.range_bound(range), output)
            }
            Type::Known(_) | Type::Variable(_) => {}
        }
    }

    /// Replaces every omitted array shape nested in a source annotation with
    /// a fresh shape variable. The parser interns source type syntax, so this
    /// elaboration step is what prevents unrelated annotations from sharing a
    /// single inference decision.
    pub(crate) fn freshen_omitted_array_shapes(&mut self, ty: Type) -> Type {
        match self.shallow(ty) {
            Type::Array(array) => {
                let element = self.freshen_omitted_array_shapes(self.array_element(array));
                let shape = match self.array_shape(array) {
                    ArrayShape::Growable => self.fresh_array_shape(),
                    shape => shape,
                };
                Type::Array(self.array_type_with_shape(element, shape))
            }
            Type::Option(option) => {
                let value = self.freshen_omitted_array_shapes(self.option_value(option));
                Type::Option(self.option_type(value))
            }
            Type::Result(result) => {
                let value = self.freshen_omitted_array_shapes(self.result_value(result));
                Type::Result(self.result_type(value))
            }
            Type::Async(future) => {
                let value = self.freshen_omitted_array_shapes(self.async_value(future));
                Type::Async(self.async_type(value))
            }
            Type::Iterator(iterator) => {
                let item = self.freshen_omitted_array_shapes(self.iterator_item(iterator));
                Type::Iterator(self.iterator_type(item))
            }
            Type::Callable(callable) => {
                let parameters = self.callable_parameters(callable).to_vec();
                let result = self.callable_result(callable);
                let parameters = parameters
                    .into_iter()
                    .map(|parameter| self.freshen_omitted_array_shapes(parameter))
                    .collect();
                let result = self.freshen_omitted_array_shapes(result);
                Type::Callable(self.callable_type(parameters, result))
            }
            Type::Set(set) => {
                let element = self.freshen_omitted_array_shapes(self.set_element(set));
                Type::Set(self.set_type(element))
            }
            Type::Range(range) => {
                let bound = self.freshen_omitted_array_shapes(self.range_bound(range));
                Type::Range(self.range_type(bound, self.range_kind(range)))
            }
            Type::Application(application) => {
                let constructor = self.application_constructor(application);
                let arguments = self.application_arguments(application).to_vec();
                let arguments = arguments
                    .into_iter()
                    .map(|argument| self.freshen_omitted_array_shapes(argument))
                    .collect();
                Type::Application(self.application_type(constructor, arguments))
            }
            ty @ (Type::Known(_) | Type::Variable(_)) => ty,
        }
    }

    pub(crate) fn bind_generic_parameter(&mut self, variable: u32, parameter: TypeId) {
        let root = self.root(variable);
        debug_assert_eq!(root, variable, "generalized variables are canonical roots");
        debug_assert!(
            self.variables[root as usize].binding.is_none(),
            "generalized variables remain unbound until semantic publication"
        );
        self.variables[root as usize].binding = Some(Type::Known(parameter));
    }

    pub(crate) fn variable_requirements(&mut self, variable: u32) -> Requirements {
        let root = self.root(variable);
        self.variables[root as usize].requirements.clone()
    }

    pub(crate) fn literal_kind(&mut self, ty: Type) -> Option<LiteralKind> {
        let Type::Variable(variable) = self.shallow(ty) else {
            return None;
        };
        let root = self.root(variable);
        match self.variables[root as usize].literal_default {
            Some(LiteralDefault::Integer) => Some(LiteralKind::Integer),
            Some(LiteralDefault::Float) => Some(LiteralKind::Float),
            None => None,
        }
    }

    pub(crate) fn intern_generic_parameter(
        &mut self,
        owner: crate::ast::FunctionId,
        index: u32,
    ) -> TypeId {
        self.types.intern_generic_parameter(owner, index)
    }

    pub(crate) fn is_unbound_without_default(&mut self, ty: Type) -> bool {
        let Type::Variable(variable) = self.shallow(ty) else {
            return false;
        };
        self.default_builtin(variable).is_none()
    }

    pub(crate) fn unify(&mut self, left: Type, right: Type) -> Result<Type, InferenceError> {
        let unified = self.unify_inner(left, right)?;
        self.solve_associated_projections()?;
        Ok(self.shallow(unified))
    }

    /// Equates two parts of a freshly instantiated generic signature without
    /// solving unrelated call-site projections that are still pending.
    ///
    /// Associated-type declarations are replayed before arguments constrain
    /// the fresh receiver variables. Running the global solver at that point
    /// can revisit an earlier invalid call while a later call is merely being
    /// instantiated, turning an ordinary source diagnostic into an internal
    /// panic. Ordinary source unification still runs the complete solver.
    pub(crate) fn unify_deferred(
        &mut self,
        left: Type,
        right: Type,
    ) -> Result<Type, InferenceError> {
        let unified = self.unify_inner(left, right)?;
        Ok(self.shallow(unified))
    }

    fn unify_inner(&mut self, left: Type, right: Type) -> Result<Type, InferenceError> {
        let left = self.shallow(left);
        let right = self.shallow(right);
        if self.is_error_type(left) || self.is_error_type(right) {
            return Ok(self.error_type());
        }
        match (left, right) {
            (Type::Variable(left), Type::Variable(right)) => self.unify_variables(left, right),
            (Type::Variable(variable), ty) | (ty, Type::Variable(variable)) => {
                self.bind(variable, ty)
            }
            (Type::Array(left), Type::Array(right)) => {
                if self
                    .unify_array_shapes(self.array_shape(left), self.array_shape(right))
                    .is_err()
                {
                    return Err(InferenceError::TypeMismatch {
                        left: Type::Array(left),
                        right: Type::Array(right),
                    });
                }
                let left_element = self.array_element(left);
                let right_element = self.array_element(right);
                self.unify_inner(left_element, right_element)?;
                Ok(Type::Array(left))
            }
            (Type::Set(left), Type::Set(right)) => {
                let left_element = self.set_element(left);
                let right_element = self.set_element(right);
                self.unify_inner(left_element, right_element)?;
                Ok(Type::Set(left))
            }
            (Type::Application(left), Type::Application(right)) => {
                if self.application_constructor(left) != self.application_constructor(right) {
                    return Err(InferenceError::TypeMismatch {
                        left: Type::Application(left),
                        right: Type::Application(right),
                    });
                }
                let left_arguments = self.application_arguments(left).to_vec();
                let right_arguments = self.application_arguments(right).to_vec();
                if left_arguments.len() != right_arguments.len() {
                    return Err(InferenceError::TypeMismatch {
                        left: Type::Application(left),
                        right: Type::Application(right),
                    });
                }
                for (left, right) in left_arguments.into_iter().zip(right_arguments) {
                    self.unify_inner(left, right)?;
                }
                Ok(Type::Application(left))
            }
            (Type::Option(left), Type::Option(right)) => {
                let left_value = self.option_value(left);
                let right_value = self.option_value(right);
                self.unify_inner(left_value, right_value)?;
                Ok(Type::Option(left))
            }
            (Type::Result(left), Type::Result(right)) => {
                let left_value = self.result_value(left);
                let right_value = self.result_value(right);
                self.unify_inner(left_value, right_value)?;
                Ok(Type::Result(left))
            }
            (Type::Async(left), Type::Async(right)) => {
                let left_value = self.async_value(left);
                let right_value = self.async_value(right);
                self.unify_inner(left_value, right_value)?;
                Ok(Type::Async(left))
            }
            (Type::Iterator(left), Type::Iterator(right)) => {
                let left_item = self.iterator_item(left);
                let right_item = self.iterator_item(right);
                self.unify_inner(left_item, right_item)?;
                Ok(Type::Iterator(left))
            }
            (Type::Callable(left), Type::Callable(right)) => {
                let left_parameters = self.callable_parameters(left).to_vec();
                let right_parameters = self.callable_parameters(right).to_vec();
                if left_parameters.len() != right_parameters.len() {
                    return Err(InferenceError::TypeMismatch {
                        left: Type::Callable(left),
                        right: Type::Callable(right),
                    });
                }
                for (left, right) in left_parameters.into_iter().zip(right_parameters) {
                    self.unify_inner(left, right)?;
                }
                self.unify_inner(self.callable_result(left), self.callable_result(right))?;
                Ok(Type::Callable(left))
            }
            (Type::Range(left), Type::Range(right)) => {
                if self.range_kind(left) != self.range_kind(right) {
                    return Err(InferenceError::TypeMismatch {
                        left: Type::Range(left),
                        right: Type::Range(right),
                    });
                }
                let left_bound = self.range_bound(left);
                let right_bound = self.range_bound(right);
                self.unify_inner(left_bound, right_bound)?;
                Ok(Type::Range(left))
            }
            (left, right) if left == right => Ok(left),
            (left, right) => Err(InferenceError::TypeMismatch { left, right }),
        }
    }

    fn solve_associated_projections(&mut self) -> Result<(), InferenceError> {
        let projections = self.associated_projections.clone();
        for (index, projection) in projections.iter().enumerate() {
            let receiver = self.root(projection.receiver);
            for previous in &projections[..index] {
                if self.root(previous.receiver) == receiver
                    && previous.capability == projection.capability
                    && previous.name == projection.name
                {
                    self.unify_inner(
                        Type::Variable(previous.output),
                        Type::Variable(projection.output),
                    )?;
                }
            }
            let receiver = self.shallow(Type::Variable(receiver));
            if matches!(receiver, Type::Variable(_)) {
                continue;
            }
            let value = match self.concrete_associated_type(
                receiver,
                projection.capability,
                projection.name,
            ) {
                Some(value) => value,
                None => continue,
            };
            self.unify_inner(Type::Variable(projection.output), value)?;
        }
        let concrete = self.concrete_associated_projections.clone();
        for projection in concrete {
            let value = match self.concrete_associated_type(
                Type::Known(projection.receiver),
                projection.capability,
                projection.name,
            ) {
                Some(value) => value,
                None => continue,
            };
            self.unify_inner(Type::Variable(projection.output), value)?;
        }
        Ok(())
    }

    pub(crate) fn require(
        &mut self,
        ty: Type,
        requirements: Requirements,
    ) -> Result<(), InferenceError> {
        let ty = self.shallow(ty);
        if self.is_error_type(ty) {
            return Ok(());
        }
        match ty {
            Type::Variable(variable) => {
                let variable = self.root(variable);
                let combined =
                    self.variables[variable as usize].requirements.clone() | requirements;
                let combined = Requirements(
                    self.standard_library
                        .minimal_capabilities(combined.as_slice()),
                );
                if !requirements_are_possible(&self.standard_library, &self.types, &combined) {
                    return Err(error("incompatible type constraints"));
                }
                self.variables[variable as usize].requirements = combined;
                Ok(())
            }
            concrete
                if type_meets_requirements(
                    &self.standard_library,
                    &self.types,
                    concrete,
                    &requirements,
                ) =>
            {
                Ok(())
            }
            concrete => Err(InferenceError::UnsupportedOperation {
                ty: concrete,
                requirements,
            }),
        }
    }

    pub(crate) fn default_unbound(&mut self) -> Vec<InferenceError> {
        let mut errors = Vec::new();
        for id in 0..self.variables.len() as u32 {
            let root = self.root(id);
            if root != id || self.variables[root as usize].binding.is_some() {
                continue;
            }
            let Some(default) = self.default_builtin(root) else {
                errors.push(error(format!(
                    "cannot infer type variable `?{root}` without more context"
                )));
                continue;
            };
            let default = self.known_builtin(default);
            match self.validate_binding(root, default) {
                Ok(()) => self.variables[root as usize].binding = Some(default),
                Err(error) => errors.push(error),
            }
        }
        errors
    }

    fn default_builtin(&self, variable: u32) -> Option<BuiltinType> {
        let variable = &self.variables[variable as usize];
        // A numeric default is an ergonomic source-language choice. It must
        // never silently become a process-memory layout choice: memory reads
        // need an explicit concrete representation from an annotation or an
        // otherwise fixed type.
        if variable
            .requirements
            .contains(StdlibCapabilityId::MemoryReadable)
            || variable
                .requirements
                .contains(StdlibCapabilityId::ManagedReadable)
        {
            return None;
        }
        if variable.requirements.contains(StdlibCapabilityId::Float) {
            return Some(BuiltinType::F64);
        }
        if variable.requirements.contains(StdlibCapabilityId::Integer) {
            return Some(BuiltinType::I32);
        }
        match variable.literal_default {
            Some(LiteralDefault::Integer) => Some(BuiltinType::I32),
            Some(LiteralDefault::Float) => Some(BuiltinType::F64),
            None => None,
        }
    }

    /// Gives every still-unbound variable a recovery-only representation so
    /// editor queries can retain independent semantic facts after diagnostics.
    /// Strict checking never observes these fallback bindings.
    pub(crate) fn recover_unbound(&mut self) {
        let error = self.types.id_for_error();
        for id in 0..self.variables.len() as u32 {
            let root = self.root(id);
            if root == id && self.variables[root as usize].binding.is_none() {
                self.variables[root as usize].binding = Some(Type::Known(error));
            }
        }
    }

    pub(crate) fn recover_unbound_type(&mut self, ty: Type) {
        let variables = self.unbound_variables_in([ty]);
        if variables.is_empty() {
            return;
        }
        let error = Type::Known(self.types.id_for_error());
        for variable in variables {
            let root = self.root(variable);
            if self.variables[root as usize].binding.is_none() {
                self.variables[root as usize].binding = Some(error);
            }
        }
    }

    pub(crate) fn resolve(&mut self, ty: Type) -> Type {
        let ty = match self.shallow(ty) {
            Type::Variable(variable) => {
                panic!("unresolved type variable ?{variable} after inference")
            }
            Type::Array(array) => {
                Type::Array(self.canonical_arrays.get(&array).copied().unwrap_or(array))
            }
            Type::Option(option) => Type::Option(
                self.canonical_options
                    .get(&option)
                    .copied()
                    .unwrap_or(option),
            ),
            Type::Result(result) => Type::Result(
                self.canonical_results
                    .get(&result)
                    .copied()
                    .unwrap_or(result),
            ),
            Type::Async(future) => Type::Async(
                self.canonical_asyncs
                    .get(&future)
                    .copied()
                    .unwrap_or(future),
            ),
            Type::Iterator(iterator) => Type::Iterator(
                self.canonical_iterators
                    .get(&iterator)
                    .copied()
                    .unwrap_or(iterator),
            ),
            Type::Callable(callable) => Type::Callable(
                self.canonical_callables
                    .get(&callable)
                    .copied()
                    .unwrap_or(callable),
            ),
            Type::Range(range) => {
                Type::Range(self.canonical_ranges.get(&range).copied().unwrap_or(range))
            }
            Type::Set(set) => Type::Set(self.canonical_sets.get(&set).copied().unwrap_or(set)),
            Type::Application(application) => Type::Application(
                self.canonical_applications
                    .get(&application)
                    .copied()
                    .unwrap_or(application),
            ),
            ty => ty,
        };
        self.constructed_types
            .get(&ty)
            .copied()
            .map_or(ty, Type::Known)
    }

    pub(crate) fn intern_resolved_constructed_types(&mut self) {
        let arrays = self
            .arrays
            .iter()
            .map(|layout| Type::Array(layout.id))
            .collect::<Vec<_>>();
        let options = self
            .options
            .iter()
            .map(|layout| Type::Option(layout.id))
            .collect::<Vec<_>>();
        let results = self
            .results
            .iter()
            .map(|layout| Type::Result(layout.id))
            .collect::<Vec<_>>();
        let asyncs = self
            .asyncs
            .iter()
            .map(|layout| Type::Async(layout.id))
            .collect::<Vec<_>>();
        let iterators = self
            .iterators
            .iter()
            .map(|layout| Type::Iterator(layout.id))
            .collect::<Vec<_>>();
        let callables = self
            .callables
            .iter()
            .map(|layout| Type::Callable(layout.id))
            .collect::<Vec<_>>();
        let ranges = self
            .ranges
            .iter()
            .map(|layout| Type::Range(layout.id))
            .collect::<Vec<_>>();
        let sets = self
            .sets
            .iter()
            .map(|layout| Type::Set(layout.id))
            .collect::<Vec<_>>();
        let applications = self
            .applications
            .iter()
            .map(|layout| Type::Application(layout.id))
            .collect::<Vec<_>>();
        for ty in arrays
            .into_iter()
            .chain(options)
            .chain(results)
            .chain(asyncs)
            .chain(iterators)
            .chain(callables)
            .chain(ranges)
            .chain(sets)
            .chain(applications)
        {
            self.intern_resolved_type(ty);
        }
    }

    pub(crate) fn array_type(&mut self, element: Type) -> ArrayTypeId {
        self.array_type_with_shape(element, ArrayShape::Growable)
    }

    pub(crate) fn inferred_array_type(&mut self, element: Type) -> ArrayTypeId {
        let shape = self.fresh_array_shape();
        self.array_type_with_shape(element, shape)
    }

    pub(crate) fn array_type_with_length(
        &mut self,
        element: Type,
        length: Option<u32>,
    ) -> ArrayTypeId {
        self.array_type_with_shape(
            element,
            length.map_or(ArrayShape::Growable, ArrayShape::Exact),
        )
    }

    fn array_type_with_shape(&mut self, element: Type, shape: ArrayShape) -> ArrayTypeId {
        if let Some(array) = self
            .arrays
            .iter()
            .find(|array| array.element == element && array.shape == shape)
        {
            return array.id;
        }
        let id = self.constructed_type_ids.array();
        self.arrays.push(ArrayLayout { id, element, shape });
        id
    }

    pub(crate) fn array_element(&self, id: ArrayTypeId) -> Type {
        self.arrays
            .iter()
            .find(|array| array.id == id)
            .expect("checked array type has a layout")
            .element
    }

    pub(crate) fn array_length(&self, id: ArrayTypeId) -> Option<u32> {
        match self.array_shape(id) {
            ArrayShape::Exact(length) => Some(length),
            ArrayShape::Growable | ArrayShape::Variable(_) => None,
        }
    }

    pub(crate) fn array_shape(&self, id: ArrayTypeId) -> ArrayShape {
        let shape = self
            .arrays
            .iter()
            .find(|array| array.id == id)
            .expect("checked array type has a layout")
            .shape;
        self.shallow_array_shape(shape)
    }

    pub(crate) fn require_growable_array(&mut self, id: ArrayTypeId) -> Result<(), ()> {
        self.unify_array_shapes(self.array_shape(id), ArrayShape::Growable)
    }

    fn fresh_array_shape(&mut self) -> ArrayShape {
        let id = self.array_shape_variables.len() as u32;
        self.array_shape_variables.push(ArrayShapeVariable {
            parent: id,
            binding: None,
        });
        ArrayShape::Variable(id)
    }

    fn array_shape_root(&self, mut variable: u32) -> u32 {
        loop {
            let parent = self.array_shape_variables[variable as usize].parent;
            if parent == variable {
                return variable;
            }
            variable = parent;
        }
    }

    fn shallow_array_shape(&self, shape: ArrayShape) -> ArrayShape {
        let ArrayShape::Variable(variable) = shape else {
            return shape;
        };
        let root = self.array_shape_root(variable);
        self.array_shape_variables[root as usize]
            .binding
            .map_or(ArrayShape::Variable(root), |binding| {
                self.shallow_array_shape(binding)
            })
    }

    fn unify_array_shapes(&mut self, left: ArrayShape, right: ArrayShape) -> Result<(), ()> {
        let left = self.shallow_array_shape(left);
        let right = self.shallow_array_shape(right);
        match (left, right) {
            (ArrayShape::Variable(left), ArrayShape::Variable(right)) if left == right => Ok(()),
            (ArrayShape::Variable(left), ArrayShape::Variable(right)) => {
                self.array_shape_variables[right as usize].parent = left;
                Ok(())
            }
            (ArrayShape::Variable(variable), shape) | (shape, ArrayShape::Variable(variable)) => {
                self.array_shape_variables[variable as usize].binding = Some(shape);
                Ok(())
            }
            (ArrayShape::Growable, ArrayShape::Growable) => Ok(()),
            (ArrayShape::Exact(left), ArrayShape::Exact(right)) if left == right => Ok(()),
            _ => Err(()),
        }
    }

    pub(crate) fn option_value(&self, id: OptionTypeId) -> Type {
        self.options
            .iter()
            .find(|option| option.id == id)
            .expect("checked option type has a layout")
            .value
    }

    pub(crate) fn result_value(&self, id: ResultTypeId) -> Type {
        self.results
            .iter()
            .find(|result| result.id == id)
            .expect("checked result type has a layout")
            .value
    }

    pub(crate) fn async_value(&self, id: AsyncTypeId) -> Type {
        self.asyncs
            .iter()
            .find(|future| future.id == id)
            .expect("checked async type has a declaration")
            .value
    }

    pub(crate) fn iterator_item(&self, id: IteratorTypeId) -> Type {
        self.iterators
            .iter()
            .find(|iterator| iterator.id == id)
            .expect("checked iterator type has a declaration")
            .item
    }

    pub(crate) fn callable_parameters(&self, id: CallableTypeId) -> &[Type] {
        &self
            .callables
            .iter()
            .find(|callable| callable.id == id)
            .expect("checked callable type has a declaration")
            .parameters
    }

    pub(crate) fn callable_result(&self, id: CallableTypeId) -> Type {
        self.callables
            .iter()
            .find(|callable| callable.id == id)
            .expect("checked callable type has a declaration")
            .result
    }

    pub(crate) fn option_type(&mut self, value: Type) -> OptionTypeId {
        if let Some(option) = self.options.iter().find(|option| option.value == value) {
            return option.id;
        }
        let id = self.constructed_type_ids.option();
        self.options.push(OptionLayout { id, value });
        id
    }

    pub(crate) fn result_type(&mut self, value: Type) -> ResultTypeId {
        if let Some(result) = self.results.iter().find(|result| result.value == value) {
            return result.id;
        }
        let id = self.constructed_type_ids.result();
        self.results.push(ResultLayout { id, value });
        id
    }

    pub(crate) fn async_type(&mut self, value: Type) -> AsyncTypeId {
        if let Some(future) = self.asyncs.iter().find(|future| future.value == value) {
            return future.id;
        }
        let id = self.constructed_type_ids.async_value();
        self.asyncs.push(AsyncLayout { id, value });
        id
    }

    pub(crate) fn iterator_type(&mut self, item: Type) -> IteratorTypeId {
        if let Some(iterator) = self.iterators.iter().find(|iterator| iterator.item == item) {
            return iterator.id;
        }
        let id = self.constructed_type_ids.iterator();
        self.iterators.push(IteratorLayout { id, item });
        id
    }

    pub(crate) fn callable_type(&mut self, parameters: Vec<Type>, result: Type) -> CallableTypeId {
        if let Some(callable) = self
            .callables
            .iter()
            .find(|callable| callable.parameters == parameters && callable.result == result)
        {
            return callable.id;
        }
        let id = self.constructed_type_ids.callable();
        self.callables.push(CallableLayout {
            id,
            parameters,
            result,
        });
        id
    }

    pub(crate) fn range_type(&mut self, bound: Type, kind: RangeKind) -> RangeTypeId {
        if let Some(range) = self
            .ranges
            .iter()
            .find(|range| range.lower == bound && range.upper == bound && range.kind == kind)
        {
            return range.id;
        }
        let id = self.constructed_type_ids.range();
        self.ranges.push(RangeLayout {
            id,
            lower: bound,
            upper: bound,
            kind,
        });
        id
    }

    pub(crate) fn range_bound(&self, id: RangeTypeId) -> Type {
        self.ranges
            .iter()
            .find(|range| range.id == id)
            .expect("checked range type has a layout")
            .lower
    }

    pub(crate) fn range_kind(&self, id: RangeTypeId) -> RangeKind {
        self.ranges
            .iter()
            .find(|range| range.id == id)
            .expect("checked range type has a layout")
            .kind
    }

    pub(crate) fn set_type(&mut self, element: Type) -> TypeApplicationId {
        if let Some(set) = self.sets.iter().find(|set| set.element == element) {
            return set.id;
        }
        let backing = self.array_type(element);
        let id = self.constructed_type_ids.application();
        self.sets.push(SetLayout {
            id,
            element,
            backing: Some(backing),
        });
        id
    }

    pub(crate) fn set_element(&self, id: TypeApplicationId) -> Type {
        self.sets
            .iter()
            .find(|set| set.id == id)
            .expect("checked set type has a layout")
            .element
    }

    pub(crate) fn set_backing(&self, id: TypeApplicationId) -> ArrayTypeId {
        self.sets
            .iter()
            .find(|set| set.id == id)
            .expect("checked set type has a layout")
            .backing
            .expect("set backing arrays are assigned during inference initialization")
    }

    pub(crate) fn application_type(
        &mut self,
        constructor: StdlibTypeConstructorId,
        arguments: Vec<Type>,
    ) -> TypeApplicationId {
        if let Some(application) = self.applications.iter().find(|application| {
            application.constructor == constructor && application.arguments == arguments
        }) {
            return application.id;
        }
        let id = self.constructed_type_ids.application();
        self.applications.push(ApplicationLayout {
            id,
            constructor,
            arguments,
        });
        id
    }

    pub(crate) fn application_constructor(&self, id: TypeApplicationId) -> StdlibTypeConstructorId {
        self.applications
            .iter()
            .find(|application| application.id == id)
            .expect("checked named type application has a layout")
            .constructor
    }

    pub(crate) fn application_arguments(&self, id: TypeApplicationId) -> &[Type] {
        &self
            .applications
            .iter()
            .find(|application| application.id == id)
            .expect("checked named type application has a layout")
            .arguments
    }

    pub(crate) fn finalize_arrays(&mut self) {
        // Shape-polymorphic signatures have served their purpose once every
        // call has been checked. The semantic product and backend only need a
        // concrete representation; an unconstrained omitted shape defaults to
        // the ordinary growable array representation.
        for id in 0..self.array_shape_variables.len() as u32 {
            let root = self.array_shape_root(id);
            if root == id && self.array_shape_variables[root as usize].binding.is_none() {
                self.array_shape_variables[root as usize].binding = Some(ArrayShape::Growable);
            }
        }
        let shapes = self
            .arrays
            .iter()
            .map(|array| self.shallow_array_shape(array.shape))
            .collect::<Vec<_>>();
        for (array, shape) in self.arrays.iter_mut().zip(shapes) {
            array.shape = shape;
        }

        let unresolved = self
            .arrays
            .iter()
            .map(|array| array.element)
            .collect::<Vec<_>>();
        let resolved = unresolved
            .into_iter()
            .map(|element| self.resolve(element))
            .collect::<Vec<_>>();
        for (array, element) in self.arrays.iter_mut().zip(resolved) {
            array.element = element;
        }

        // Array layouts can be allocated while their element is still an
        // inference variable. Distinct provisional layouts may consequently
        // resolve to the same `[T]` or `[T; N]` type. Collapse those layouts
        // before semantic publication so Wasm GC never receives two nominal
        // identities for one source type.
        loop {
            let previous = self.canonical_arrays.clone();
            self.canonical_arrays.clear();
            let mut representatives = Vec::<(Type, ArrayShape, ArrayTypeId)>::new();
            for array in &self.arrays {
                let element = canonical_constructed_type(
                    array.element,
                    &previous,
                    &self.canonical_options,
                    &self.canonical_results,
                    &self.canonical_asyncs,
                    &self.canonical_iterators,
                    &self.canonical_sets,
                );
                let canonical = representatives
                    .iter()
                    .find_map(|(candidate, shape, id)| {
                        (*candidate == element && *shape == array.shape).then_some(*id)
                    })
                    .unwrap_or_else(|| {
                        representatives.push((element, array.shape, array.id));
                        array.id
                    });
                self.canonical_arrays.insert(array.id, canonical);
            }
            if self.canonical_arrays == previous {
                break;
            }
        }

        let canonical_arrays = self.canonical_arrays.clone();
        for array in &mut self.arrays {
            array.element = canonical_constructed_type(
                array.element,
                &canonical_arrays,
                &self.canonical_options,
                &self.canonical_results,
                &self.canonical_asyncs,
                &self.canonical_iterators,
                &self.canonical_sets,
            );
        }
    }

    pub(crate) fn finalize_wrappers(&mut self) {
        let option_values = self
            .options
            .iter()
            .map(|option| option.value)
            .collect::<Vec<_>>();
        let option_values = option_values
            .into_iter()
            .map(|value| self.resolve(value))
            .collect::<Vec<_>>();
        for (option, value) in self.options.iter_mut().zip(option_values) {
            option.value = value;
        }

        let result_values = self
            .results
            .iter()
            .map(|result| result.value)
            .collect::<Vec<_>>();
        let result_values = result_values
            .into_iter()
            .map(|value| self.resolve(value))
            .collect::<Vec<_>>();
        for (result, value) in self.results.iter_mut().zip(result_values) {
            result.value = value;
        }

        let async_values = self
            .asyncs
            .iter()
            .map(|future| future.value)
            .collect::<Vec<_>>();
        let async_values = async_values
            .into_iter()
            .map(|value| self.resolve(value))
            .collect::<Vec<_>>();
        for (future, value) in self.asyncs.iter_mut().zip(async_values) {
            future.value = value;
        }

        let iterator_items = self
            .iterators
            .iter()
            .map(|iterator| iterator.item)
            .collect::<Vec<_>>();
        let iterator_items = iterator_items
            .into_iter()
            .map(|item| self.resolve(item))
            .collect::<Vec<_>>();
        for (iterator, item) in self.iterators.iter_mut().zip(iterator_items) {
            iterator.item = item;
        }

        // Constructors can allocate a provisional wrapper before later uses
        // constrain its value type. Once all inference variables resolve,
        // collapse layouts with identical value types to the first stable ID.
        // This keeps WebAssembly GC's nominal type identities aligned with the
        // structural unification performed above.
        loop {
            let previous_options = self.canonical_options.clone();
            let previous_results = self.canonical_results.clone();
            let previous_asyncs = self.canonical_asyncs.clone();
            let previous_iterators = self.canonical_iterators.clone();
            self.canonical_options.clear();
            self.canonical_results.clear();
            self.canonical_asyncs.clear();
            self.canonical_iterators.clear();

            let mut option_representatives = Vec::<(Type, OptionTypeId)>::new();
            for option in &self.options {
                let value = canonical_constructed_type(
                    option.value,
                    &self.canonical_arrays,
                    &previous_options,
                    &previous_results,
                    &previous_asyncs,
                    &previous_iterators,
                    &self.canonical_sets,
                );
                let canonical = option_representatives
                    .iter()
                    .find_map(|(candidate, id)| (*candidate == value).then_some(*id))
                    .unwrap_or_else(|| {
                        option_representatives.push((value, option.id));
                        option.id
                    });
                self.canonical_options.insert(option.id, canonical);
            }

            let mut result_representatives = Vec::<(Type, ResultTypeId)>::new();
            for result in &self.results {
                let value = canonical_constructed_type(
                    result.value,
                    &self.canonical_arrays,
                    &previous_options,
                    &previous_results,
                    &previous_asyncs,
                    &previous_iterators,
                    &self.canonical_sets,
                );
                let canonical = result_representatives
                    .iter()
                    .find_map(|(candidate, id)| (*candidate == value).then_some(*id))
                    .unwrap_or_else(|| {
                        result_representatives.push((value, result.id));
                        result.id
                    });
                self.canonical_results.insert(result.id, canonical);
            }

            let mut async_representatives = Vec::<(Type, AsyncTypeId)>::new();
            for future in &self.asyncs {
                let value = canonical_constructed_type(
                    future.value,
                    &self.canonical_arrays,
                    &previous_options,
                    &previous_results,
                    &previous_asyncs,
                    &previous_iterators,
                    &self.canonical_sets,
                );
                let canonical = async_representatives
                    .iter()
                    .find_map(|(candidate, id)| (*candidate == value).then_some(*id))
                    .unwrap_or_else(|| {
                        async_representatives.push((value, future.id));
                        future.id
                    });
                self.canonical_asyncs.insert(future.id, canonical);
            }

            let mut iterator_representatives = Vec::<(Type, IteratorTypeId)>::new();
            for iterator in &self.iterators {
                let item = canonical_constructed_type(
                    iterator.item,
                    &self.canonical_arrays,
                    &previous_options,
                    &previous_results,
                    &previous_asyncs,
                    &previous_iterators,
                    &self.canonical_sets,
                );
                let canonical = iterator_representatives
                    .iter()
                    .find_map(|(candidate, id)| (*candidate == item).then_some(*id))
                    .unwrap_or_else(|| {
                        iterator_representatives.push((item, iterator.id));
                        iterator.id
                    });
                self.canonical_iterators.insert(iterator.id, canonical);
            }

            if self.canonical_options == previous_options
                && self.canonical_results == previous_results
                && self.canonical_asyncs == previous_asyncs
                && self.canonical_iterators == previous_iterators
            {
                break;
            }
        }

        let canonical_options = self.canonical_options.clone();
        let canonical_results = self.canonical_results.clone();
        let canonical_asyncs = self.canonical_asyncs.clone();
        let canonical_iterators = self.canonical_iterators.clone();
        for option in &mut self.options {
            option.value = canonical_constructed_type(
                option.value,
                &self.canonical_arrays,
                &canonical_options,
                &canonical_results,
                &canonical_asyncs,
                &canonical_iterators,
                &self.canonical_sets,
            );
        }
        for result in &mut self.results {
            result.value = canonical_constructed_type(
                result.value,
                &self.canonical_arrays,
                &canonical_options,
                &canonical_results,
                &canonical_asyncs,
                &canonical_iterators,
                &self.canonical_sets,
            );
        }
        for future in &mut self.asyncs {
            future.value = canonical_constructed_type(
                future.value,
                &self.canonical_arrays,
                &canonical_options,
                &canonical_results,
                &canonical_asyncs,
                &canonical_iterators,
                &self.canonical_sets,
            );
        }
        for iterator in &mut self.iterators {
            iterator.item = canonical_constructed_type(
                iterator.item,
                &self.canonical_arrays,
                &canonical_options,
                &canonical_results,
                &canonical_asyncs,
                &canonical_iterators,
                &self.canonical_sets,
            );
        }
    }

    pub(crate) fn finalize_sets(&mut self) {
        let unresolved = self.sets.iter().map(|set| set.element).collect::<Vec<_>>();
        let resolved = unresolved
            .into_iter()
            .map(|element| self.resolve(element))
            .collect::<Vec<_>>();
        for (set, element) in self.sets.iter_mut().zip(resolved) {
            set.element = element;
            let backing = set
                .backing
                .expect("set backing arrays are assigned during inference initialization");
            set.backing = Some(
                self.canonical_arrays
                    .get(&backing)
                    .copied()
                    .unwrap_or(backing),
            );
        }

        let mut representatives = Vec::<(Type, TypeApplicationId)>::new();
        self.canonical_sets.clear();
        for set in &self.sets {
            let canonical = representatives
                .iter()
                .find_map(|(candidate, id)| (*candidate == set.element).then_some(*id))
                .unwrap_or_else(|| {
                    representatives.push((set.element, set.id));
                    set.id
                });
            self.canonical_sets.insert(set.id, canonical);
        }
    }

    pub(crate) fn finalize_applications(&mut self) {
        let unresolved = self
            .applications
            .iter()
            .map(|application| application.arguments.clone())
            .collect::<Vec<_>>();
        let resolved = unresolved
            .into_iter()
            .map(|arguments| {
                arguments
                    .into_iter()
                    .map(|argument| self.resolve(argument))
                    .collect::<Vec<_>>()
            })
            .collect::<Vec<_>>();
        for (application, arguments) in self.applications.iter_mut().zip(resolved) {
            application.arguments = arguments;
        }

        let mut representatives =
            Vec::<(StdlibTypeConstructorId, Vec<Type>, TypeApplicationId)>::new();
        self.canonical_applications.clear();
        for application in &self.applications {
            let canonical = representatives
                .iter()
                .find_map(|(constructor, arguments, id)| {
                    (*constructor == application.constructor && *arguments == application.arguments)
                        .then_some(*id)
                })
                .unwrap_or_else(|| {
                    representatives.push((
                        application.constructor,
                        application.arguments.clone(),
                        application.id,
                    ));
                    application.id
                });
            self.canonical_applications
                .insert(application.id, canonical);
        }
    }

    pub(crate) fn finalize_ranges(&mut self) {
        let bounds = self
            .ranges
            .iter()
            .map(|range| (range.lower, range.upper))
            .collect::<Vec<_>>();
        let bounds = bounds
            .into_iter()
            .map(|(lower, upper)| (self.resolve(lower), self.resolve(upper)))
            .collect::<Vec<_>>();
        for (range, (lower, upper)) in self.ranges.iter_mut().zip(bounds) {
            range.lower = lower;
            range.upper = upper;
        }

        let mut representatives = Vec::<(Type, RangeKind, RangeTypeId)>::new();
        self.canonical_ranges.clear();
        for range in &self.ranges {
            let canonical = representatives
                .iter()
                .find_map(|(bound, kind, id)| {
                    (*bound == range.lower && *kind == range.kind).then_some(*id)
                })
                .unwrap_or_else(|| {
                    representatives.push((range.lower, range.kind, range.id));
                    range.id
                });
            self.canonical_ranges.insert(range.id, canonical);
        }
    }

    pub(crate) fn finalize_callables(&mut self) {
        let signatures = self
            .callables
            .iter()
            .map(|callable| (callable.parameters.clone(), callable.result))
            .collect::<Vec<_>>();
        let signatures = signatures
            .into_iter()
            .map(|(parameters, result)| {
                (
                    parameters
                        .into_iter()
                        .map(|parameter| self.resolve(parameter))
                        .collect::<Vec<_>>(),
                    self.resolve(result),
                )
            })
            .collect::<Vec<_>>();
        for (callable, (parameters, result)) in self.callables.iter_mut().zip(signatures) {
            callable.parameters = parameters;
            callable.result = result;
        }

        let mut representatives = Vec::<(Vec<Type>, Type, CallableTypeId)>::new();
        self.canonical_callables.clear();
        for callable in &self.callables {
            let canonical = representatives
                .iter()
                .find_map(|(parameters, result, id)| {
                    (*parameters == callable.parameters && *result == callable.result)
                        .then_some(*id)
                })
                .unwrap_or_else(|| {
                    representatives.push((
                        callable.parameters.clone(),
                        callable.result,
                        callable.id,
                    ));
                    callable.id
                });
            self.canonical_callables.insert(callable.id, canonical);
        }
    }

    pub(crate) fn arrays(&self) -> &[ArrayLayout] {
        &self.arrays
    }

    pub(crate) fn options(&self) -> &[OptionLayout] {
        &self.options
    }

    pub(crate) fn results(&self) -> &[ResultLayout] {
        &self.results
    }

    pub(crate) fn asyncs(&self) -> &[AsyncLayout] {
        &self.asyncs
    }

    pub(crate) fn iterators(&self) -> &[IteratorLayout] {
        &self.iterators
    }

    pub(crate) fn callables(&self) -> &[CallableLayout] {
        &self.callables
    }

    pub(crate) fn ranges(&self) -> &[RangeLayout] {
        &self.ranges
    }

    pub(crate) fn sets(&self) -> &[SetLayout] {
        &self.sets
    }

    pub(crate) fn applications(&self) -> &[ApplicationLayout] {
        &self.applications
    }

    fn intern_resolved_type(&mut self, ty: Type) -> TypeId {
        let ty = self.resolve(ty);
        if let Type::Known(id) = ty {
            return id;
        }
        if let Some(id) = self.constructed_types.get(&ty) {
            return *id;
        }
        let kind = match ty {
            Type::Array(layout) => {
                let element = self.array_element(layout);
                let element = self.intern_resolved_type(element);
                TypeKind::Array {
                    layout,
                    element,
                    length: self.array_length(layout),
                }
            }
            Type::Option(layout) => {
                let value = self.option_value(layout);
                let value = self.intern_resolved_type(value);
                TypeKind::Option { layout, value }
            }
            Type::Result(layout) => {
                let value = self.result_value(layout);
                let value = self.intern_resolved_type(value);
                TypeKind::Result { layout, value }
            }
            Type::Async(layout) => {
                let value = self.async_value(layout);
                let value = self.intern_resolved_type(value);
                TypeKind::Async { layout, value }
            }
            Type::Iterator(layout) => {
                let item = self.iterator_item(layout);
                let item = self.intern_resolved_type(item);
                TypeKind::Iterator { layout, item }
            }
            Type::Callable(layout) => {
                let parameters = self.callable_parameters(layout).to_vec();
                let result = self.callable_result(layout);
                let parameters = parameters
                    .into_iter()
                    .map(|parameter| self.intern_resolved_type(parameter))
                    .collect();
                let result = self.intern_resolved_type(result);
                TypeKind::Callable {
                    layout,
                    parameters,
                    result,
                }
            }
            Type::Range(layout) => {
                let bound = self.range_bound(layout);
                let bound = self.intern_resolved_type(bound);
                TypeKind::Range {
                    layout,
                    bound,
                    kind: self.range_kind(layout),
                }
            }
            Type::Set(layout) => {
                let element = self.set_element(layout);
                let element = self.intern_resolved_type(element);
                TypeKind::Set {
                    layout,
                    element,
                    backing: self.set_backing(layout),
                }
            }
            Type::Application(layout) => {
                let constructor = self.application_constructor(layout);
                let arguments = self.application_arguments(layout).to_vec();
                let arguments = arguments
                    .into_iter()
                    .map(|argument| self.intern_resolved_type(argument))
                    .collect();
                TypeKind::Application {
                    layout,
                    constructor,
                    arguments,
                }
            }
            Type::Variable(variable) => {
                unreachable!("unresolved type variable ?{variable} reached semantic interning")
            }
            Type::Known(_) => unreachable!("known types return before constructed interning"),
        };
        let id = self.types.intern(kind);
        self.constructed_types.insert(ty, id);
        id
    }

    fn root(&mut self, id: u32) -> u32 {
        let parent = self.variables[id as usize].parent;
        if parent == id {
            id
        } else {
            let root = self.root(parent);
            self.variables[id as usize].parent = root;
            root
        }
    }

    fn unify_variables(&mut self, left: u32, right: u32) -> Result<Type, InferenceError> {
        let left = self.root(left);
        let right = self.root(right);
        if left == right {
            return Ok(Type::Variable(left));
        }

        let right_variable = self.variables[right as usize].clone();
        let left_variable = self.variables[left as usize].clone();
        self.variables[right as usize].parent = left;
        let requirements = left_variable.requirements.clone() | right_variable.requirements.clone();
        self.variables[left as usize].requirements = Requirements(
            self.standard_library
                .minimal_capabilities(requirements.as_slice()),
        );
        self.variables[left as usize].largest_literal = left_variable
            .largest_literal
            .max(right_variable.largest_literal);
        self.variables[left as usize].largest_negative_literal = left_variable
            .largest_negative_literal
            .max(right_variable.largest_negative_literal);
        self.variables[left as usize].literal_default = LiteralDefault::merge(
            left_variable.literal_default,
            right_variable.literal_default,
        );
        self.variables[left as usize].binding = None;

        let binding = match (left_variable.binding, right_variable.binding) {
            (Some(left), Some(right)) => Some(self.unify_inner(left, right)?),
            (Some(binding), None) | (None, Some(binding)) => Some(binding),
            (None, None) => None,
        };
        if let Some(binding) = binding {
            self.validate_binding(left, binding)?;
            self.variables[left as usize].binding = Some(binding);
            Ok(self.shallow(Type::Variable(left)))
        } else if requirements_are_possible(
            &self.standard_library,
            &self.types,
            &self.variables[left as usize].requirements,
        ) {
            Ok(Type::Variable(left))
        } else {
            Err(error("incompatible type constraints"))
        }
    }

    fn bind(&mut self, variable: u32, ty: Type) -> Result<Type, InferenceError> {
        let variable = self.root(variable);
        let ty = self.shallow(ty);
        if ty == Type::Variable(variable) {
            return Ok(ty);
        }
        if let Some(binding) = self.variables[variable as usize].binding {
            return self.unify_inner(binding, ty);
        }
        if self.occurs_in(variable, ty, &mut Vec::new()) {
            return Err(InferenceError::Message(
                "polymorphic recursion is not supported because it would require an infinite recursive type"
                    .to_owned(),
            ));
        }
        self.validate_binding(variable, ty)?;
        self.variables[variable as usize].binding = Some(ty);
        Ok(ty)
    }

    fn occurs_in(&mut self, variable: u32, ty: Type, visited: &mut Vec<Type>) -> bool {
        let ty = self.shallow(ty);
        if !visited.contains(&ty) {
            visited.push(ty);
        } else {
            return false;
        }
        match ty {
            Type::Variable(candidate) => self.root(candidate) == variable,
            Type::Array(array) => self.occurs_in(variable, self.array_element(array), visited),
            Type::Set(set) => self.occurs_in(variable, self.set_element(set), visited),
            Type::Application(application) => self
                .application_arguments(application)
                .to_vec()
                .into_iter()
                .any(|argument| self.occurs_in(variable, argument, visited)),
            Type::Option(option) => self.occurs_in(variable, self.option_value(option), visited),
            Type::Result(result) => self.occurs_in(variable, self.result_value(result), visited),
            Type::Async(future) => self.occurs_in(variable, self.async_value(future), visited),
            Type::Iterator(iterator) => {
                self.occurs_in(variable, self.iterator_item(iterator), visited)
            }
            Type::Callable(callable) => {
                self.callable_parameters(callable)
                    .to_vec()
                    .into_iter()
                    .any(|parameter| self.occurs_in(variable, parameter, visited))
                    || self.occurs_in(variable, self.callable_result(callable), visited)
            }
            Type::Range(range) => self.occurs_in(variable, self.range_bound(range), visited),
            Type::Known(_) => false,
        }
    }

    fn validate_binding(&self, variable: u32, ty: Type) -> Result<(), InferenceError> {
        let inference = &self.variables[variable as usize];
        if !type_meets_requirements(
            &self.standard_library,
            &self.types,
            ty,
            &inference.requirements,
        ) {
            return Err(InferenceError::UnsatisfiedConstraints {
                ty,
                requirements: inference.requirements.clone(),
            });
        }
        if let Some(literal) = inference.largest_literal
            && !fits_unsigned_literal(&self.types, literal, ty)
        {
            return Err(InferenceError::IntegerLiteralOutOfRange(ty));
        }
        if let Some(literal) = inference.largest_negative_literal
            && !fits_negative_literal(&self.types, literal, ty)
        {
            return Err(InferenceError::IntegerLiteralOutOfRange(ty));
        }
        Ok(())
    }
}

fn fits_unsigned_literal(types: &TypeStore, value: u64, ty: Type) -> bool {
    let Type::Known(id) = ty else {
        return false;
    };
    match types.kind(id) {
        TypeKind::Error => false,
        TypeKind::Builtin(BuiltinType::Bool) => value <= 1,
        TypeKind::Builtin(BuiltinType::U8) => u8::try_from(value).is_ok(),
        TypeKind::Builtin(BuiltinType::I8) => value <= i8::MAX as u64,
        TypeKind::Builtin(BuiltinType::U16) => u16::try_from(value).is_ok(),
        TypeKind::Builtin(BuiltinType::I16) => value <= i16::MAX as u64,
        TypeKind::Builtin(BuiltinType::U32) => u32::try_from(value).is_ok(),
        TypeKind::Builtin(BuiltinType::I32) => value <= i32::MAX as u64,
        TypeKind::Builtin(BuiltinType::U64 | BuiltinType::Address) => true,
        TypeKind::Builtin(BuiltinType::I64) => value <= i64::MAX as u64,
        TypeKind::Builtin(BuiltinType::F32) => integer_is_exact_at_precision(value, 24),
        TypeKind::Builtin(BuiltinType::F64) => integer_is_exact_at_precision(value, 53),
        _ => false,
    }
}

fn fits_negative_literal(types: &TypeStore, value: u64, ty: Type) -> bool {
    let Type::Known(id) = ty else {
        return false;
    };
    match types.kind(id) {
        TypeKind::Builtin(BuiltinType::I8) => value <= (i8::MAX as u64) + 1,
        TypeKind::Builtin(BuiltinType::I16) => value <= (i16::MAX as u64) + 1,
        TypeKind::Builtin(BuiltinType::I32) => value <= (i32::MAX as u64) + 1,
        TypeKind::Builtin(BuiltinType::I64) => value <= (i64::MAX as u64) + 1,
        TypeKind::Builtin(BuiltinType::F32) => integer_is_exact_at_precision(value, 24),
        TypeKind::Builtin(BuiltinType::F64) => integer_is_exact_at_precision(value, 53),
        _ => false,
    }
}

fn integer_is_exact_at_precision(value: u64, precision: u32) -> bool {
    let significant_bits = u64::BITS - value.leading_zeros();
    significant_bits <= precision || value.trailing_zeros() >= significant_bits - precision
}

fn type_meets_requirements(
    standard_library: &StandardLibrary,
    types: &TypeStore,
    ty: Type,
    requirements: &Requirements,
) -> bool {
    if matches!(ty, Type::Variable(_)) {
        return true;
    }
    if matches!(
        ty,
        Type::Known(id) if matches!(types.kind(id), TypeKind::Error)
    ) {
        return true;
    }
    requirements
        .0
        .iter()
        .all(|capability| type_may_have_capability(standard_library, types, ty, *capability))
}

/// Conservative pre-semantic admissibility check. Derived capabilities are
/// proven recursively by `CapabilityAnalysis` once inference produces a
/// semantic TypeId.
pub(crate) fn type_may_have_capability(
    library: &StandardLibrary,
    types: &TypeStore,
    ty: Type,
    capability: StdlibCapabilityId,
) -> bool {
    if library.has_universal_debug_fallback(capability) {
        return match ty {
            Type::Known(id) => !matches!(
                types.kind(id),
                TypeKind::Builtin(BuiltinType::Never) | TypeKind::GenericParameter { .. }
            ),
            Type::Variable(_) => false,
            Type::Option(_)
            | Type::Result(_)
            | Type::Async(_)
            | Type::Iterator(_)
            | Type::Callable(_)
            | Type::Range(_)
            | Type::Array(_)
            | Type::Set(_)
            | Type::Application(_) => true,
        };
    }
    let behavior = library.capability(capability).behavior;
    if behavior == CapabilityBehavior::StructuralManagedRead {
        return type_may_have_capability(library, types, ty, StdlibCapabilityId::MemoryReadable)
            || match ty {
                Type::Known(id) => matches!(
                    types.kind(id),
                    TypeKind::Standard(crate::stdlib::StdlibTypeId::String)
                        | TypeKind::Option { .. }
                ),
                Type::Option(_) => true,
                _ => false,
            };
    }
    match ty {
        Type::Known(id) => match types.kind(id) {
            TypeKind::Error => false,
            TypeKind::Builtin(builtin) => library.core_type_has_capability(*builtin, capability),
            TypeKind::Standard(standard) => library.type_has_capability(*standard, capability),
            TypeKind::StateSnapshot | TypeKind::SettingsView => false,
            TypeKind::Struct(_) => matches!(
                behavior,
                CapabilityBehavior::StructuralEquality
                    | CapabilityBehavior::StructuralMemoryLayout
                    | CapabilityBehavior::StructuralMethods
            ),
            TypeKind::Enum(_) => matches!(
                behavior,
                CapabilityBehavior::StructuralEquality
                    | CapabilityBehavior::StructuralMemoryLayout
                    | CapabilityBehavior::StructuralMethods
            ),
            TypeKind::ManagedClass(_) => false,
            TypeKind::ManagedReference(_) => false,
            TypeKind::Option { .. } => {
                behavior == CapabilityBehavior::StructuralEquality
                    || library.type_constructor_has_capability(
                        StdlibTypeConstructorId::Option,
                        capability,
                    )
            }
            TypeKind::Result { .. } => {
                behavior == CapabilityBehavior::StructuralEquality
                    || library.type_constructor_has_capability(
                        StdlibTypeConstructorId::Result,
                        capability,
                    )
            }
            TypeKind::Async { .. } | TypeKind::Callable { .. } => false,
            TypeKind::Iterator { .. } => matches!(
                capability,
                StdlibCapabilityId::Iterator
                    | StdlibCapabilityId::Iterable
                    | StdlibCapabilityId::Debug
                    | StdlibCapabilityId::Display
            ),
            TypeKind::Range { kind, .. } => library.type_constructor_has_capability(
                match kind {
                    RangeKind::Exclusive => StdlibTypeConstructorId::ExclusiveRange,
                    RangeKind::Inclusive => StdlibTypeConstructorId::InclusiveRange,
                },
                capability,
            ),
            TypeKind::Array { length, .. } => {
                behavior == CapabilityBehavior::StructuralEquality
                    || behavior == CapabilityBehavior::StructuralMemoryLayout && length.is_some()
                    || library
                        .type_constructor_has_capability(StdlibTypeConstructorId::Array, capability)
            }
            TypeKind::Set { .. } => {
                library.type_constructor_has_capability(StdlibTypeConstructorId::Set, capability)
            }
            TypeKind::Application { constructor, .. } => {
                library.type_constructor_has_capability(*constructor, capability)
            }
            TypeKind::GenericParameter { .. } => false,
        },
        Type::Option(_) => {
            behavior == CapabilityBehavior::StructuralEquality
                || library
                    .type_constructor_has_capability(StdlibTypeConstructorId::Option, capability)
        }
        Type::Result(_) => {
            behavior == CapabilityBehavior::StructuralEquality
                || library
                    .type_constructor_has_capability(StdlibTypeConstructorId::Result, capability)
        }
        Type::Async(_) | Type::Callable(_) => false,
        Type::Iterator(_) => matches!(
            capability,
            StdlibCapabilityId::Iterator
                | StdlibCapabilityId::Iterable
                | StdlibCapabilityId::Debug
                | StdlibCapabilityId::Display
        ),
        Type::Range(_) => [
            StdlibTypeConstructorId::ExclusiveRange,
            StdlibTypeConstructorId::InclusiveRange,
        ]
        .into_iter()
        .any(|constructor| library.type_constructor_has_capability(constructor, capability)),
        Type::Array(_) => {
            behavior == CapabilityBehavior::StructuralEquality
                || behavior == CapabilityBehavior::StructuralMemoryLayout
                || library
                    .type_constructor_has_capability(StdlibTypeConstructorId::Array, capability)
        }
        Type::Set(_) => {
            library.type_constructor_has_capability(StdlibTypeConstructorId::Set, capability)
        }
        Type::Application(application) => library.type_constructor_has_capability(
            // Inference owns the constructor mapping; unresolved applications
            // are conservatively admitted and validated semantically later.
            match types.iter().find_map(|(_, kind)| match kind {
                TypeKind::Application {
                    layout,
                    constructor,
                    ..
                } if *layout == application => Some(*constructor),
                _ => None,
            }) {
                Some(constructor) => constructor,
                None => return true,
            },
            capability,
        ),
        Type::Variable(_) => false,
    }
}

fn canonical_constructed_type(
    ty: Type,
    arrays: &HashMap<ArrayTypeId, ArrayTypeId>,
    options: &HashMap<OptionTypeId, OptionTypeId>,
    results: &HashMap<ResultTypeId, ResultTypeId>,
    asyncs: &HashMap<AsyncTypeId, AsyncTypeId>,
    iterators: &HashMap<IteratorTypeId, IteratorTypeId>,
    sets: &HashMap<TypeApplicationId, TypeApplicationId>,
) -> Type {
    match ty {
        Type::Array(array) => Type::Array(arrays.get(&array).copied().unwrap_or(array)),
        Type::Option(option) => Type::Option(options.get(&option).copied().unwrap_or(option)),
        Type::Result(result) => Type::Result(results.get(&result).copied().unwrap_or(result)),
        Type::Async(future) => Type::Async(asyncs.get(&future).copied().unwrap_or(future)),
        Type::Iterator(iterator) => {
            Type::Iterator(iterators.get(&iterator).copied().unwrap_or(iterator))
        }
        Type::Set(set) => Type::Set(sets.get(&set).copied().unwrap_or(set)),
        ty => ty,
    }
}

fn requirements_are_possible(
    library: &StandardLibrary,
    types: &TypeStore,
    requirements: &Requirements,
) -> bool {
    library
        .core_types()
        .iter()
        .map(|ty| Type::Known(types.id_for_core(ty.id)))
        .chain(
            library
                .all_types()
                .iter()
                .map(|ty| Type::Known(types.id_for_standard(ty.id))),
        )
        .any(|ty| type_meets_requirements(library, types, ty, requirements))
        || library.type_constructors().iter().any(|constructor| {
            requirements.as_slice().iter().all(|requirement| {
                library.type_constructor_has_capability(constructor.id, *requirement)
            })
        })
}

fn error(message: impl Into<String>) -> InferenceError {
    InferenceError::Message(message.into())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn unifies_bidirectionally_and_checks_literal_bounds() {
        let mut inference = InferenceContext::new(
            StandardLibrary::new(),
            TypeStore::default(),
            0,
            ConstructedLayouts::default(),
        );
        let value = inference.fresh(
            Requirements::capability(StdlibCapabilityId::Integer),
            Some(256),
        );
        let alias = inference.fresh(Requirements::none(), None);
        inference.unify(alias, value).unwrap();
        assert!(
            inference
                .unify(alias, inference.known_builtin(BuiltinType::U8))
                .is_err()
        );
        let u16_type = inference.known_builtin(BuiltinType::U16);
        inference.unify(alias, u16_type).unwrap();
        assert_eq!(inference.resolve(value), u16_type);
    }

    #[test]
    fn broad_capability_constraints_do_not_choose_a_numeric_representation() {
        let mut inference = InferenceContext::new(
            StandardLibrary::new(),
            TypeStore::default(),
            0,
            ConstructedLayouts::default(),
        );
        let value = inference.fresh(Requirements::capability(StdlibCapabilityId::Signed), None);
        inference
            .require(value, Requirements::capability(StdlibCapabilityId::Numeric))
            .unwrap();
        assert_eq!(inference.default_unbound().len(), 1);
        inference.recover_unbound();
        let Type::Known(recovered) = inference.resolve(value) else {
            panic!("recovery must publish a semantic type")
        };
        assert_eq!(inference.type_store().kind(recovered), &TypeKind::Error);
    }

    #[test]
    fn integer_and_float_capabilities_choose_the_language_defaults() {
        let mut inference = InferenceContext::new(
            StandardLibrary::new(),
            TypeStore::default(),
            0,
            ConstructedLayouts::default(),
        );
        let integer = inference.fresh(Requirements::capability(StdlibCapabilityId::Integer), None);
        let float = inference.fresh(Requirements::capability(StdlibCapabilityId::Float), None);
        assert!(inference.default_unbound().is_empty());
        assert_eq!(
            inference.resolve(integer),
            inference.known_builtin(BuiltinType::I32)
        );
        assert_eq!(
            inference.resolve(float),
            inference.known_builtin(BuiltinType::F64)
        );
    }

    #[test]
    fn memory_readable_suppresses_literal_and_capability_defaults() {
        let mut inference = InferenceContext::new(
            StandardLibrary::new(),
            TypeStore::default(),
            0,
            ConstructedLayouts::default(),
        );
        let integer = inference.fresh(
            Requirements::capabilities([
                StdlibCapabilityId::MemoryReadable,
                StdlibCapabilityId::Integer,
            ]),
            Some(1),
        );
        let float = inference.fresh(
            Requirements::capabilities([
                StdlibCapabilityId::MemoryReadable,
                StdlibCapabilityId::Float,
            ]),
            None,
        );
        let errors = inference.default_unbound();
        assert_eq!(errors.len(), 2);
        assert!(inference.is_unbound_without_default(integer));
        assert!(inference.is_unbound_without_default(float));
    }

    #[test]
    fn unsuffixed_literals_use_the_language_numeric_defaults() {
        let mut inference = InferenceContext::new(
            StandardLibrary::new(),
            TypeStore::default(),
            0,
            ConstructedLayouts::default(),
        );
        let integer = inference.fresh(
            Requirements::capability(StdlibCapabilityId::Numeric),
            Some(7),
        );
        let float = inference.fresh_float_literal();
        assert!(inference.default_unbound().is_empty());
        let i32_type = inference.known_builtin(BuiltinType::I32);
        let f64_type = inference.known_builtin(BuiltinType::F64);
        assert_eq!(inference.resolve(integer), i32_type);
        assert_eq!(inference.resolve(float), f64_type);
    }

    #[test]
    fn capability_requirements_are_not_limited_to_well_known_bit_positions() {
        let loaded = StdlibCapabilityId::from_u32(10_000);
        let requirements = Requirements::capabilities([
            StdlibCapabilityId::Numeric,
            loaded,
            StdlibCapabilityId::Numeric,
        ]);

        assert!(requirements.contains(StdlibCapabilityId::Numeric));
        assert!(requirements.contains(loaded));
        assert_eq!(requirements.0.len(), 2);
    }

    #[test]
    fn scheme_instantiation_preserves_constraints_sharing_and_nested_shapes() {
        let mut inference = InferenceContext::new(
            StandardLibrary::new(),
            TypeStore::default(),
            0,
            ConstructedLayouts::default(),
        );
        let template = inference.fresh(Requirements::capability(StdlibCapabilityId::Numeric), None);
        let Type::Variable(root) = template else {
            unreachable!()
        };
        let array = inference.array_type(template);
        let option = inference.option_type(Type::Array(array));
        let result = inference.result_type(Type::Option(option));

        let mut first_substitutions = HashMap::new();
        let first_value = inference.instantiate_type(template, &[root], &mut first_substitutions);
        let first_nested =
            inference.instantiate_type(Type::Result(result), &[root], &mut first_substitutions);
        let Type::Result(first_result) = first_nested else {
            unreachable!()
        };
        let Type::Option(first_option) = inference.result_value(first_result) else {
            unreachable!()
        };
        let Type::Array(first_array) = inference.option_value(first_option) else {
            unreachable!()
        };
        assert_eq!(inference.array_element(first_array), first_value);

        let mut second_substitutions = HashMap::new();
        let second_value = inference.instantiate_type(template, &[root], &mut second_substitutions);
        assert_ne!(first_value, second_value);

        let i64_type = inference.known_builtin(BuiltinType::I64);
        inference.unify(first_value, i64_type).unwrap();
        let string_type = inference.known_standard(StdlibTypeId::String);
        assert!(inference.unify(second_value, string_type).is_err());
    }

    #[test]
    fn interns_resolved_constructed_types_before_semantic_publication() {
        let mut ids = ConstructedTypeIdAllocator::starting_at(0);
        let array = ids.array();
        let option = ids.option();
        let result = ids.result();
        let types = TypeStore::default();
        let u32_type = Type::Known(types.id_for_builtin(BuiltinType::U32));
        let mut inference = InferenceContext::new(
            StandardLibrary::new(),
            types,
            3,
            ConstructedLayouts {
                arrays: vec![ArrayLayout {
                    id: array,
                    element: u32_type,
                    shape: ArrayShape::Growable,
                }],
                options: vec![OptionLayout {
                    id: option,
                    value: Type::Array(array),
                }],
                results: vec![ResultLayout {
                    id: result,
                    value: Type::Option(option),
                }],
                ..ConstructedLayouts::default()
            },
        );

        inference.finalize_arrays();
        inference.finalize_wrappers();
        inference.intern_resolved_constructed_types();

        let Type::Known(array_type) = inference.resolve(Type::Array(array)) else {
            panic!("resolved array did not become a semantic TypeId")
        };
        let Type::Known(option_type) = inference.resolve(Type::Option(option)) else {
            panic!("resolved option did not become a semantic TypeId")
        };
        let Type::Known(result_type) = inference.resolve(Type::Result(result)) else {
            panic!("resolved result did not become a semantic TypeId")
        };

        assert_eq!(
            inference.type_store().kind(array_type),
            &TypeKind::Array {
                layout: array,
                element: match u32_type {
                    Type::Known(id) => id,
                    _ => unreachable!(),
                },
                length: None,
            }
        );
        assert_eq!(
            inference.type_store().kind(option_type),
            &TypeKind::Option {
                layout: option,
                value: array_type,
            }
        );
        assert_eq!(
            inference.type_store().kind(result_type),
            &TypeKind::Result {
                layout: result,
                value: option_type,
            }
        );
    }
}
