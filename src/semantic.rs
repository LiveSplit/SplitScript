//! Semantic facts produced by type checking and consumed by later stages.

use std::collections::HashMap;

use crate::{
    ast::{
        ActionKind, ArrayTypeId, AssignmentId, EnumId, EnumVariantId, ExprId, FunctionId,
        ManagedClassId, ManagedFieldId, OptionTypeId, PatternId, ResultTypeId,
        SettingChoiceOptionId, StructFieldId, StructId, TypeApplicationId, ValueId,
    },
    inference::Type,
    stdlib::{
        StandardLibrary, StdlibFieldId, StdlibItemId, StdlibStateProviderId,
        StdlibTypeConstructorId, StdlibTypeId, StdlibVariantId, TypeRef as CatalogTypeRef,
    },
    types::{
        ResolvedArrayType, ResolvedCallableType, ResolvedConstructedTypes,
        ResolvedConstructedTypesMut, ResolvedOptionType, ResolvedRangeType, ResolvedResultType,
        ResolvedSetType, TypeId, TypeKind, TypeStore,
    },
};

/// A concrete instantiation of a source function.
///
/// Monomorphic functions use an empty argument vector. Generic instances also
/// retain their exact concrete parameter/result signature because nominal GC
/// layouts are not recoverable from type arguments alone. This identity lives
/// at the semantic boundary so typed HIR, reachability, and Wasm emission agree
/// on every concrete body without inventing backend-only function IDs.
#[derive(Debug, Clone, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct FunctionInstance {
    pub function: FunctionId,
    pub type_arguments: Vec<TypeId>,
    pub signature: Vec<TypeId>,
}

/// A source function materialized as a value of one concrete callable type.
///
/// The target function and callable layout are both part of the identity: the
/// backend needs a small captureless adapter because ordinary source functions
/// do not receive the environment parameter used by callable GC objects. `ty`
/// is the fully resolved callable type, including its nominal GC layout.
#[derive(Debug, Clone, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct FunctionValueInstance {
    pub function: FunctionInstance,
    pub ty: TypeId,
}

/// A closure expression as instantiated inside one concrete generic function.
///
/// The same source closure can have different parameter, result, capture, and
/// frame layouts for different function specializations. Top-level action and
/// state-expression closures have no function owner.
#[derive(Debug, Clone, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct ClosureInstance {
    pub owner: Option<FunctionInstance>,
    pub expression: ExprId,
}

impl ClosureInstance {
    pub fn new(owner: Option<FunctionInstance>, expression: ExprId) -> Self {
        Self { owner, expression }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct FunctionAssociatedProjection {
    pub receiver: TypeId,
    pub output: TypeId,
    pub capability: crate::stdlib::StdlibCapabilityId,
    pub name: &'static str,
}

impl FunctionInstance {
    pub fn monomorphic(function: FunctionId) -> Self {
        Self {
            function,
            type_arguments: Vec::new(),
            signature: Vec::new(),
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ResolvedCall {
    UserFunction {
        function: FunctionId,
        type_arguments: Vec<TypeId>,
        signature: Vec<TypeId>,
    },
    UserMethod {
        function: FunctionId,
        type_arguments: Vec<TypeId>,
        signature: Vec<TypeId>,
        receiver: ResolvedReceiver,
        receiver_type: TypeId,
    },
    StandardLibrary {
        item: StdlibItemId,
        type_arguments: Vec<TypeId>,
        /// Concrete receiver/parameter types followed by the declared result.
        /// Library source bodies use this to instantiate their inferred hidden
        /// function template without reconstructing catalog types downstream.
        signature: Vec<TypeId>,
        receiver: Option<ResolvedReceiver>,
        receiver_type: Option<TypeId>,
    },
    /// Schema-derived transactional read of every instance field declared by
    /// one live managed reference.
    ManagedSnapshot {
        class: ManagedClassId,
        result: crate::ast::ResultTypeId,
        receiver: ResolvedReceiver,
        receiver_type: TypeId,
    },
    /// Schema-derived lookup of one declared managed class on a native Unity
    /// GameObject. Native hierarchy traversal remains source-defined; this
    /// call supplies the class's bound runtime header and nominal `T.Ref!`
    /// result type.
    ManagedComponent {
        class: ManagedClassId,
        result: crate::ast::ResultTypeId,
        receiver: ResolvedReceiver,
        receiver_type: TypeId,
    },
    /// Compiler-provided cooperative discovery of every live instance of a
    /// declared managed class.
    ManagedInstances {
        class: ManagedClassId,
    },
    ResultError {
        result: crate::ast::ResultTypeId,
    },
    OptionSome {
        option: crate::ast::OptionTypeId,
    },
    IteratorItem {
        step: TypeApplicationId,
    },
    ResultSuccess {
        result: crate::ast::ResultTypeId,
    },
}

fn resolved_option_layout(ty: Type, types: &TypeStore) -> OptionTypeId {
    match ty {
        Type::Option(layout) => layout,
        Type::Known(id) => match types.kind(id) {
            TypeKind::Option { layout, .. } => *layout,
            kind => unreachable!("Some/None resolved to non-Option type `{kind:?}`"),
        },
        ty => unreachable!("Some/None resolved to non-Option inference term `{ty}`"),
    }
}

fn resolved_result_layout(ty: Type, types: &TypeStore) -> ResultTypeId {
    match ty {
        Type::Result(layout) => layout,
        Type::Known(id) => match types.kind(id) {
            TypeKind::Result { layout, .. } => *layout,
            kind => unreachable!("result constructor resolved to non-Result type `{kind:?}`"),
        },
        ty => unreachable!("result constructor resolved to non-Result inference term `{ty}`"),
    }
}

fn resolved_application_layout(ty: Type, types: &TypeStore) -> TypeApplicationId {
    match ty {
        Type::Application(layout) => layout,
        Type::Known(id) => match types.kind(id) {
            TypeKind::Application { layout, .. } => *layout,
            kind => unreachable!("named application resolved to `{kind:?}`"),
        },
        ty => unreachable!("named application resolved to inference term `{ty}`"),
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ValueConversionKind {
    NoneToOptional,
    NoneToDomainNullable,
    LiftOption,
    LiftResult,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ValueConversion {
    pub kind: ValueConversionKind,
    pub source: TypeId,
    pub target: TypeId,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ResolvedValue {
    /// A catalog-declared value whose source implementation is evaluated on
    /// demand. Constants participate in ordinary value paths; the backend's
    /// hidden zero-argument function is an implementation detail rather than
    /// a source-level call.
    StandardLibraryConstant(StdlibItemId),
    ProviderValue(StdlibStateProviderId),
    /// An additional attachment-scoped value declared by the selected state
    /// provider. The index addresses the provider's catalog context list.
    ProviderContext {
        provider: StdlibStateProviderId,
        context: u32,
    },
    /// A live static managed field. Provider preparation caches its class
    /// storage table and metadata offset; evaluation performs the fallible
    /// process-memory read.
    ManagedStatic {
        class: ManagedClassId,
        field: ManagedFieldId,
    },
    Variable(ValueId),
    CurrentSnapshot,
    OldSnapshot,
    SettingsView,
    OldSettingsView,
    CurrentState(ValueId),
    OldState(ValueId),
    /// A sibling state field read from the candidate snapshot currently being
    /// assembled. Unlike `CurrentState`, this never falls back to the last
    /// committed snapshot when its dependency fails.
    StateCandidate(ValueId),
    Setting(ValueId),
    OldSetting(ValueId),
}

impl ResolvedValue {
    /// Returns the source value identity read by this resolution. Provider
    /// values are compiler/catalog-owned roots and have no source declaration.
    pub fn source_value(self) -> Option<ValueId> {
        match self {
            Self::StandardLibraryConstant(_)
            | Self::ProviderValue(_)
            | Self::ProviderContext { .. }
            | Self::ManagedStatic { .. }
            | Self::CurrentSnapshot
            | Self::OldSnapshot
            | Self::SettingsView
            | Self::OldSettingsView => None,
            Self::Variable(value)
            | Self::CurrentState(value)
            | Self::OldState(value)
            | Self::StateCandidate(value)
            | Self::Setting(value)
            | Self::OldSetting(value) => Some(value),
        }
    }
}

impl ResolvedCall {
    /// Returns the receiver resolution for method-shaped calls.
    pub fn receiver(&self) -> Option<&ResolvedReceiver> {
        match self {
            Self::UserMethod { receiver, .. } => Some(receiver),
            Self::StandardLibrary { receiver, .. } => receiver.as_ref(),
            Self::ManagedSnapshot { receiver, .. } | Self::ManagedComponent { receiver, .. } => {
                Some(receiver)
            }
            Self::UserFunction { .. }
            | Self::ManagedInstances { .. }
            | Self::ResultError { .. }
            | Self::OptionSome { .. }
            | Self::IteratorItem { .. }
            | Self::ResultSuccess { .. } => None,
        }
    }
}

/// The value on which a method is invoked.
///
/// Plain source paths retain their declaration root and resolved fields for
/// navigation and direct lowering. General postfix calls instead retain the
/// receiver expression, which is evaluated exactly once before the explicit
/// arguments.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ResolvedReceiver {
    Path {
        root: ResolvedValue,
        members: Vec<ResolvedMember>,
    },
    Expression {
        expression: ExprId,
        members: Vec<ResolvedMember>,
    },
}

impl ResolvedReceiver {
    pub fn path(&self) -> Option<(ResolvedValue, &[ResolvedMember])> {
        match self {
            Self::Path { root, members } => Some((*root, members)),
            Self::Expression { .. } => None,
        }
    }

    pub fn expression(&self) -> Option<ExprId> {
        match self {
            Self::Expression { expression, .. } => Some(*expression),
            Self::Path { .. } => None,
        }
    }

    pub fn members(&self) -> &[ResolvedMember] {
        match self {
            Self::Path { members, .. } | Self::Expression { members, .. } => members,
        }
    }
}

/// A field selected after the root of a resolved value path.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum ResolvedMember {
    StateField(ValueId),
    SettingField(ValueId),
    StructField(StructFieldId),
    ManagedField(ManagedFieldId),
    StandardField(StdlibFieldId),
}

/// Stable identity of an enum variant selected by checked source.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum ResolvedEnumVariantId {
    Source(EnumVariantId),
    Standard(StdlibVariantId),
}

/// Nominal struct selected by a checked struct literal.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum ResolvedStructId {
    Source(StructId),
    Standard(StdlibTypeId),
    StandardConstructor(TypeApplicationId),
}

impl std::fmt::Display for ResolvedStructId {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Source(structure) => structure.fmt(formatter),
            Self::Standard(structure) => write!(formatter, "{structure:?}"),
            Self::StandardConstructor(structure) => write!(formatter, "{structure:?}"),
        }
    }
}

/// Field selected by a checked struct literal.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum ResolvedStructFieldId {
    Source(StructFieldId),
    Standard(StdlibFieldId),
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ResolvedWrapperPattern {
    OptionNone(OptionTypeId),
    OptionSome(OptionTypeId),
    ResultSuccess(ResultTypeId),
    ResultError(ResultTypeId),
    IteratorEnd(TypeApplicationId),
    IteratorItem(TypeApplicationId),
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DynamicCallCallee {
    Expression(ExprId),
    Value(ValueId),
}

/// The checked source value that selects one finite declaration shape.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum ResolvedShapeDimension {
    Global(ValueId),
    StateField(ValueId),
}

/// One statically proven fact about a finite declaration-shape discriminator.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct ResolvedShapeConstraint {
    pub dimension: ResolvedShapeDimension,
    pub variant: EnumVariantId,
}

/// Exact shape alternatives under which a conditional declaration exists.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct ResolvedShapePredicate {
    pub alternatives: Vec<Vec<ResolvedShapeConstraint>>,
}

#[derive(Debug, Clone, Default)]
pub struct SemanticModel {
    types: TypeStore,
    state_provider: Option<StdlibStateProviderId>,
    state_provider_selector: Option<usize>,
    state_provider_alternatives: HashMap<EnumVariantId, (StdlibStateProviderId, Option<usize>)>,
    state_field_providers: HashMap<ValueId, StdlibStateProviderId>,
    expression_types: HashMap<ExprId, TypeId>,
    calls: HashMap<ExprId, ResolvedCall>,
    dynamic_calls: HashMap<ExprId, DynamicCallCallee>,
    function_values: HashMap<ExprId, FunctionInstance>,
    values: HashMap<ExprId, ResolvedValue>,
    value_types: HashMap<ValueId, TypeId>,
    action_results: HashMap<ActionKind, TypeId>,
    function_results: HashMap<FunctionId, TypeId>,
    function_completions: HashMap<FunctionId, TypeId>,
    function_parameter_types: HashMap<FunctionId, Vec<TypeId>>,
    function_type_parameters: HashMap<FunctionId, Vec<TypeId>>,
    function_associated_projections: HashMap<FunctionId, Vec<FunctionAssociatedProjection>>,
    source_associated_types:
        HashMap<(TypeId, crate::stdlib::StdlibCapabilityId, &'static str), TypeId>,
    generic_parameter_constraints: HashMap<TypeId, Vec<crate::stdlib::StdlibCapabilityId>>,
    specialized_types: HashMap<(FunctionInstance, TypeId), TypeId>,
    struct_field_types: HashMap<StructFieldId, TypeId>,
    managed_field_types: HashMap<ManagedFieldId, TypeId>,
    standard_field_types: HashMap<StdlibFieldId, TypeId>,
    enum_variant_payloads: HashMap<EnumVariantId, Option<TypeId>>,
    enum_representations: HashMap<EnumId, TypeId>,
    array_element_types: HashMap<ArrayTypeId, TypeId>,
    state_storage_fields: Vec<ValueId>,
    state_storage_field_by_declaration: HashMap<ValueId, ValueId>,
    state_provider_fields: HashMap<EnumVariantId, Vec<ValueId>>,
    conditional_state_fields: HashMap<ValueId, ResolvedShapePredicate>,
    conditional_managed_fields: HashMap<ManagedFieldId, ResolvedShapePredicate>,
    state_poll_results: HashMap<ValueId, TypeId>,
    state_dependencies: HashMap<ValueId, Vec<ValueId>>,
    propagation_targets: HashMap<ExprId, TypeId>,
    propagation_retry_boundaries: HashMap<ExprId, ExprId>,
    path_members: HashMap<ExprId, Vec<ResolvedMember>>,
    struct_literals: HashMap<ExprId, ResolvedStructId>,
    struct_literal_fields: HashMap<ExprId, Vec<ResolvedStructFieldId>>,
    enum_variants: HashMap<ExprId, ResolvedEnumVariantId>,
    pattern_variants: HashMap<PatternId, ResolvedEnumVariantId>,
    wrapper_patterns: HashMap<PatternId, ResolvedWrapperPattern>,
    struct_patterns: HashMap<PatternId, ResolvedStructId>,
    struct_pattern_fields: HashMap<PatternId, Vec<ResolvedStructFieldId>>,
    setting_choice_defaults: HashMap<ValueId, EnumVariantId>,
    setting_choice_options: HashMap<SettingChoiceOptionId, EnumVariantId>,
    assignments: HashMap<AssignmentId, ValueId>,
    assignment_calls: HashMap<AssignmentId, ResolvedCall>,
    index_assignment_setters: HashMap<AssignmentId, ResolvedCall>,
    value_conversions: HashMap<ExprId, ValueConversion>,
    visible_expression_count: Option<usize>,
}

impl SemanticModel {
    pub fn types(&self) -> &TypeStore {
        &self.types
    }

    /// The catalog provider selected by `state ProviderName`, if present.
    pub fn state_provider(&self) -> Option<StdlibStateProviderId> {
        self.state_provider
    }

    pub fn state_provider_selector(&self) -> Option<usize> {
        self.state_provider_selector
    }

    pub fn state_provider_alternative(
        &self,
        variant: EnumVariantId,
    ) -> Option<(StdlibStateProviderId, Option<usize>)> {
        self.state_provider_alternatives.get(&variant).copied()
    }

    pub fn state_provider_alternatives(
        &self,
    ) -> impl Iterator<Item = (EnumVariantId, StdlibStateProviderId, Option<usize>)> + '_ {
        self.state_provider_alternatives
            .iter()
            .map(|(variant, (provider, selector))| (*variant, *provider, *selector))
    }

    pub fn state_field_provider(&self, field: ValueId) -> Option<StdlibStateProviderId> {
        self.state_field_providers
            .get(&field)
            .copied()
            .or(self.state_provider)
    }

    pub fn expression_type(&self, expression: ExprId) -> Option<TypeId> {
        self.expression_types.get(&expression).copied()
    }

    pub fn expression_types(&self) -> impl Iterator<Item = (ExprId, TypeId)> + '_ {
        self.expression_types
            .iter()
            .filter(|(expression, _)| {
                self.visible_expression_count
                    .is_none_or(|count| expression.index() < count)
            })
            .map(|(expression, ty)| (*expression, *ty))
    }

    pub fn dynamic_call_callee(&self, expression: ExprId) -> Option<DynamicCallCallee> {
        self.dynamic_calls.get(&expression).copied()
    }

    pub fn function_value(&self, expression: ExprId) -> Option<&FunctionInstance> {
        self.function_values.get(&expression)
    }

    pub fn call(&self, expression: ExprId) -> Option<&ResolvedCall> {
        self.calls.get(&expression)
    }

    pub fn value(&self, expression: ExprId) -> Option<ResolvedValue> {
        self.values.get(&expression).copied()
    }

    pub fn values(&self) -> impl Iterator<Item = (ExprId, ResolvedValue)> + '_ {
        self.values
            .iter()
            .filter(|(expression, _)| {
                self.visible_expression_count
                    .is_none_or(|count| expression.index() < count)
            })
            .map(|(expression, value)| (*expression, *value))
    }

    pub fn value_type(&self, value: ValueId) -> Option<TypeId> {
        self.value_types.get(&value).copied()
    }

    pub fn value_types(&self) -> impl Iterator<Item = (ValueId, TypeId)> + '_ {
        self.value_types.iter().map(|(value, ty)| (*value, *ty))
    }

    /// The checked ABI result of a lifecycle action.
    pub fn action_result(&self, action: ActionKind) -> Option<TypeId> {
        self.action_results.get(&action).copied()
    }

    /// Physical fields in the generated StateSnapshot GC structure.
    pub fn state_storage_fields(&self) -> &[ValueId] {
        &self.state_storage_fields
    }

    pub fn state_field_shape_predicate(&self, field: ValueId) -> Option<&ResolvedShapePredicate> {
        self.conditional_state_fields.get(&field)
    }

    pub fn managed_field_shape_predicate(
        &self,
        field: ManagedFieldId,
    ) -> Option<&ResolvedShapePredicate> {
        self.conditional_managed_fields.get(&field)
    }

    /// Maps a concrete field declaration to the physical snapshot field that
    /// stores it. Compatible provider or conditional alternatives project to
    /// the first declaration's slot.
    pub fn state_storage_field(&self, field: ValueId) -> Option<ValueId> {
        self.state_storage_field_by_declaration.get(&field).copied()
    }

    /// Concrete declarations read when a provider alternative is active.
    pub fn state_provider_fields(&self, variant: EnumVariantId) -> &[ValueId] {
        self.state_provider_fields
            .get(&variant)
            .map(Vec::as_slice)
            .unwrap_or_default()
    }

    pub fn function_result(&self, function: FunctionId) -> Option<TypeId> {
        self.function_results.get(&function).copied()
    }

    /// The value produced when an async function completes. Synchronous
    /// functions have the same result and completion type.
    pub fn function_completion(&self, function: FunctionId) -> Option<TypeId> {
        self.function_completions.get(&function).copied()
    }

    pub fn function_parameter_types(&self, function: FunctionId) -> &[TypeId] {
        self.function_parameter_types
            .get(&function)
            .map(Vec::as_slice)
            .unwrap_or_default()
    }

    pub fn function_type_parameters(&self, function: FunctionId) -> &[TypeId] {
        self.function_type_parameters
            .get(&function)
            .map(Vec::as_slice)
            .unwrap_or_default()
    }

    pub fn function_associated_projections(
        &self,
        function: FunctionId,
    ) -> &[FunctionAssociatedProjection] {
        self.function_associated_projections
            .get(&function)
            .map(Vec::as_slice)
            .unwrap_or_default()
    }

    pub fn associated_projection_for_output(
        &self,
        output: TypeId,
    ) -> Option<FunctionAssociatedProjection> {
        self.function_associated_projections
            .values()
            .flatten()
            .find(|projection| projection.output == output)
            .copied()
    }

    /// Returns an associated type inferred from the method contract of a
    /// source-defined nominal type.
    pub fn source_associated_type(
        &self,
        receiver: TypeId,
        capability: crate::stdlib::StdlibCapabilityId,
        name: &'static str,
    ) -> Option<TypeId> {
        self.source_associated_types
            .get(&(receiver, capability, name))
            .copied()
    }

    /// Constructs the canonical concrete identity for an inferred function
    /// template from its exact call signature. This is also used when a
    /// catalog call targets a hidden source body: the catalog owns the public
    /// signature, while the hidden declaration may infer a differently shaped
    /// but equivalent set of generalized roots.
    pub fn function_instance(
        &self,
        function: FunctionId,
        signature: Vec<TypeId>,
    ) -> FunctionInstance {
        let templates = self
            .function_parameter_types(function)
            .iter()
            .copied()
            .chain(self.function_result(function))
            .collect::<Vec<_>>();
        debug_assert_eq!(templates.len(), signature.len());
        let type_arguments = self
            .function_type_parameters(function)
            .iter()
            .map(|parameter| {
                templates
                    .iter()
                    .copied()
                    .zip(signature.iter().copied())
                    .find_map(|(template, concrete)| {
                        self.specialize_signature_node(template, concrete, *parameter)
                    })
                    .expect("generalized function roots occur in their signature")
            })
            .collect();
        FunctionInstance {
            function,
            type_arguments,
            signature,
        }
    }

    pub fn generic_parameter_constraints(
        &self,
        parameter: TypeId,
    ) -> &[crate::stdlib::StdlibCapabilityId] {
        self.generic_parameter_constraints
            .get(&parameter)
            .map(Vec::as_slice)
            .unwrap_or_default()
    }

    /// Substitutes a function template's type parameters for one concrete
    /// instance. Constructed instantiations are already interned by call-site
    /// inference, so specialization preserves the checked program's canonical
    /// type and layout identities.
    pub fn specialize_type(&self, instance: &FunctionInstance, ty: TypeId) -> TypeId {
        if let Some(specialized) = self.specialized_types.get(&(instance.clone(), ty)) {
            return *specialized;
        }
        if let Some(specialized) = self.direct_specialization(instance, ty) {
            return specialized;
        }
        let specialized_child = match self.types.kind(ty) {
            TypeKind::Error => None,
            TypeKind::Array { element, .. } => Some((0, self.specialize_type(instance, *element))),
            TypeKind::Option { value, .. } => Some((1, self.specialize_type(instance, *value))),
            TypeKind::Result { value, .. } => Some((2, self.specialize_type(instance, *value))),
            TypeKind::Async { value, .. } => Some((3, self.specialize_type(instance, *value))),
            TypeKind::Iterator { item, .. } => Some((6, self.specialize_type(instance, *item))),
            // Callable signatures are specialized by the closure/callable
            // monomorphization pass because they contain more than one child.
            TypeKind::Callable { .. } => None,
            TypeKind::Set { element, .. } => Some((4, self.specialize_type(instance, *element))),
            TypeKind::Range { bound, .. } => Some((5, self.specialize_type(instance, *bound))),
            TypeKind::Application { .. } => None,
            TypeKind::Builtin(_)
            | TypeKind::Standard(_)
            | TypeKind::StateSnapshot
            | TypeKind::SettingsView
            | TypeKind::Struct(_)
            | TypeKind::ManagedClass(_)
            | TypeKind::ManagedReference(_)
            | TypeKind::Enum(_)
            | TypeKind::GenericParameter { .. } => None,
        };
        let Some((constructor, child)) = specialized_child else {
            return ty;
        };
        let original_array_length = match self.types.kind(ty) {
            TypeKind::Array { length, .. } => *length,
            _ => None,
        };
        let original_child = match self.types.kind(ty) {
            TypeKind::Array { element, .. } => *element,
            TypeKind::Option { value, .. }
            | TypeKind::Result { value, .. }
            | TypeKind::Async { value, .. } => *value,
            TypeKind::Iterator { item, .. } => *item,
            TypeKind::Set { element, .. } => *element,
            TypeKind::Range { bound, .. } => *bound,
            TypeKind::Application { .. } => unreachable!(
                "named type applications are specialized through their complete argument list"
            ),
            _ => unreachable!(),
        };
        if child == original_child {
            return ty;
        }
        self.types
            .iter()
            .find_map(|(candidate, kind)| match (constructor, kind) {
                (
                    0,
                    TypeKind::Array {
                        element, length, ..
                    },
                ) if *element == child && *length == original_array_length => Some(candidate),
                (1, TypeKind::Option { value, .. }) if *value == child => Some(candidate),
                (2, TypeKind::Result { value, .. }) if *value == child => Some(candidate),
                (3, TypeKind::Async { value, .. }) if *value == child => Some(candidate),
                (4, TypeKind::Set { element, .. }) if *element == child => Some(candidate),
                (5, TypeKind::Range { bound, .. }) if *bound == child => Some(candidate),
                (6, TypeKind::Iterator { item, .. }) if *item == child => Some(candidate),
                _ => None,
            })
            .unwrap_or_else(|| {
                panic!(
                    "backend specialization did not materialize a concrete instance of {:?}",
                    self.types.kind(ty)
                )
            })
    }

    fn direct_specialization(&self, instance: &FunctionInstance, ty: TypeId) -> Option<TypeId> {
        let parameters = self.function_type_parameters(instance.function);
        if let Some(index) = parameters.iter().position(|parameter| *parameter == ty) {
            return instance.type_arguments.get(index).copied();
        }
        let templates = self
            .function_parameter_types(instance.function)
            .iter()
            .copied()
            .chain(self.function_result(instance.function));
        for (template, concrete) in templates.zip(&instance.signature) {
            if let Some(specialized) = self.specialize_signature_node(template, *concrete, ty) {
                return Some(specialized);
            }
        }
        None
    }

    pub(crate) fn materialize_specialized_type(
        &mut self,
        instance: &FunctionInstance,
        ty: TypeId,
        ids: &mut crate::ast::ConstructedTypeIdAllocator,
        constructed: &mut ResolvedConstructedTypesMut<'_>,
    ) -> TypeId {
        if let Some(specialized) = self.specialized_types.get(&(instance.clone(), ty)) {
            return *specialized;
        }
        if let Some(specialized) = self.direct_specialization(instance, ty) {
            self.specialized_types
                .insert((instance.clone(), ty), specialized);
            return specialized;
        }
        let kind = self.types.kind(ty).clone();
        let specialized = match kind {
            TypeKind::Error => ty,
            TypeKind::Array {
                element, length, ..
            } => {
                let element =
                    self.materialize_specialized_type(instance, element, ids, constructed);
                if element
                    == match self.types.kind(ty) {
                        TypeKind::Array { element, .. } => *element,
                        _ => unreachable!(),
                    }
                {
                    ty
                } else if let Some(existing) = self.types.iter().find_map(|(id, kind)| {
                    matches!(kind, TypeKind::Array { element: candidate, length: candidate_length, .. }
                        if *candidate == element && *candidate_length == length)
                    .then_some(id)
                }) {
                    existing
                } else {
                    let layout = ids.array();
                    constructed.arrays.push(ResolvedArrayType {
                        id: layout,
                        element: self.resolved_type_ref(element),
                        length,
                    });
                    self.array_element_types.insert(layout, element);
                    self.types.intern(TypeKind::Array {
                        layout,
                        element,
                        length,
                    })
                }
            }
            TypeKind::Option { value, .. } => {
                let value = self.materialize_specialized_type(instance, value, ids, constructed);
                if value
                    == match self.types.kind(ty) {
                        TypeKind::Option { value, .. } => *value,
                        _ => unreachable!(),
                    }
                {
                    ty
                } else {
                    let layout = ids.option();
                    constructed.options.push(ResolvedOptionType {
                        id: layout,
                        value: self.resolved_type_ref(value),
                    });
                    self.types.intern(TypeKind::Option { layout, value })
                }
            }
            TypeKind::Result { value, .. } => {
                let value = self.materialize_specialized_type(instance, value, ids, constructed);
                if value
                    == match self.types.kind(ty) {
                        TypeKind::Result { value, .. } => *value,
                        _ => unreachable!(),
                    }
                {
                    ty
                } else {
                    let layout = ids.result();
                    constructed.results.push(ResolvedResultType {
                        id: layout,
                        value: self.resolved_type_ref(value),
                    });
                    self.types.intern(TypeKind::Result { layout, value })
                }
            }
            TypeKind::Async { value, .. } => {
                let value = self.materialize_specialized_type(instance, value, ids, constructed);
                if value
                    == match self.types.kind(ty) {
                        TypeKind::Async { value, .. } => *value,
                        _ => unreachable!(),
                    }
                {
                    ty
                } else {
                    let layout = ids.async_value();
                    constructed.asyncs.push(crate::types::ResolvedAsyncType {
                        id: layout,
                        value: self.resolved_type_ref(value),
                    });
                    self.types.intern(TypeKind::Async { layout, value })
                }
            }
            TypeKind::Iterator { item, .. } => {
                let item = self.materialize_specialized_type(instance, item, ids, constructed);
                if item
                    == match self.types.kind(ty) {
                        TypeKind::Iterator { item, .. } => *item,
                        _ => unreachable!(),
                    }
                {
                    ty
                } else {
                    let layout = ids.iterator();
                    constructed
                        .iterators
                        .push(crate::types::ResolvedIteratorType {
                            id: layout,
                            item: self.resolved_type_ref(item),
                        });
                    self.types.intern(TypeKind::Iterator { layout, item })
                }
            }
            TypeKind::Set { element, .. } => {
                let element =
                    self.materialize_specialized_type(instance, element, ids, constructed);
                if element
                    == match self.types.kind(ty) {
                        TypeKind::Set { element, .. } => *element,
                        _ => unreachable!(),
                    }
                {
                    ty
                } else {
                    let backing = constructed
                        .arrays
                        .iter()
                        .find(|array| {
                            array.length.is_none()
                                && self.array_element_types.get(&array.id) == Some(&element)
                        })
                        .map(|array| array.id)
                        .unwrap_or_else(|| {
                            let backing = ids.array();
                            constructed.arrays.push(ResolvedArrayType {
                                id: backing,
                                element: self.resolved_type_ref(element),
                                length: None,
                            });
                            self.array_element_types.insert(backing, element);
                            self.types.intern(TypeKind::Array {
                                layout: backing,
                                element,
                                length: None,
                            });
                            backing
                        });
                    let layout = ids.application();
                    constructed.sets.push(ResolvedSetType {
                        id: layout,
                        element: self.resolved_type_ref(element),
                        backing,
                    });
                    self.types.intern(TypeKind::Set {
                        layout,
                        element,
                        backing,
                    })
                }
            }
            TypeKind::Range { bound, kind, .. } => {
                let bound = self.materialize_specialized_type(instance, bound, ids, constructed);
                if bound
                    == match self.types.kind(ty) {
                        TypeKind::Range { bound, .. } => *bound,
                        _ => unreachable!(),
                    }
                {
                    ty
                } else {
                    let layout = ids.range();
                    constructed.ranges.push(ResolvedRangeType {
                        id: layout,
                        bound: self.resolved_type_ref(bound),
                        kind,
                    });
                    self.types.intern(TypeKind::Range {
                        layout,
                        bound,
                        kind,
                    })
                }
            }
            TypeKind::Application {
                constructor,
                arguments,
                ..
            } => {
                let specialized_arguments = arguments
                    .iter()
                    .map(|argument| {
                        self.materialize_specialized_type(instance, *argument, ids, constructed)
                    })
                    .collect::<Vec<_>>();
                if specialized_arguments == arguments {
                    ty
                } else if let Some(existing) = self.types.iter().find_map(|(id, kind)| {
                    matches!(kind, TypeKind::Application {
                        constructor: candidate,
                        arguments: candidate_arguments,
                        ..
                    } if *candidate == constructor && *candidate_arguments == specialized_arguments)
                    .then_some(id)
                }) {
                    existing
                } else {
                    let layout = ids.application();
                    constructed
                        .applications
                        .push(crate::types::ResolvedApplicationType {
                            id: layout,
                            constructor,
                            arguments: specialized_arguments
                                .iter()
                                .map(|argument| self.resolved_type_ref(*argument))
                                .collect(),
                        });
                    self.types.intern(TypeKind::Application {
                        layout,
                        constructor,
                        arguments: specialized_arguments,
                    })
                }
            }
            TypeKind::Callable {
                parameters, result, ..
            } => {
                let specialized_parameters = parameters
                    .iter()
                    .map(|parameter| {
                        self.materialize_specialized_type(instance, *parameter, ids, constructed)
                    })
                    .collect::<Vec<_>>();
                let specialized_result =
                    self.materialize_specialized_type(instance, result, ids, constructed);
                if specialized_parameters == parameters && specialized_result == result {
                    ty
                } else {
                    let resolved_parameters = specialized_parameters
                        .iter()
                        .map(|parameter| self.resolved_type_ref(*parameter))
                        .collect::<Vec<_>>();
                    let resolved_result = self.resolved_type_ref(specialized_result);
                    let layout = constructed
                        .callables
                        .iter()
                        .find(|callable| {
                            callable.parameters == resolved_parameters
                                && callable.result == resolved_result
                        })
                        .map(|callable| callable.id)
                        .unwrap_or_else(|| {
                            let layout = ids.callable();
                            constructed.callables.push(ResolvedCallableType {
                                id: layout,
                                parameters: resolved_parameters,
                                result: resolved_result,
                            });
                            layout
                        });
                    self.types.intern(TypeKind::Callable {
                        layout,
                        parameters: specialized_parameters,
                        result: specialized_result,
                    })
                }
            }
            TypeKind::Builtin(_)
            | TypeKind::Standard(_)
            | TypeKind::StateSnapshot
            | TypeKind::SettingsView
            | TypeKind::Struct(_)
            | TypeKind::ManagedClass(_)
            | TypeKind::ManagedReference(_)
            | TypeKind::Enum(_)
            | TypeKind::GenericParameter { .. } => ty,
        };
        self.specialized_types
            .insert((instance.clone(), ty), specialized);
        specialized
    }

    /// Materializes the concrete semantic layout named by a catalog type.
    ///
    /// Generic standard-library structs may themselves contain constructed
    /// fields (for example, `Map<K, V>` owns `[MapEntry<K, V>]`). Type checking
    /// only needs the outer declaration, while the GC backend needs every
    /// nested nominal layout. This is the single post-inference constructor for
    /// those nested shapes.
    pub(crate) fn materialize_catalog_type(
        &mut self,
        ty: CatalogTypeRef,
        variables: &HashMap<&'static str, TypeId>,
        ids: &mut crate::ast::ConstructedTypeIdAllocator,
        constructed: &mut ResolvedConstructedTypesMut<'_>,
        library: &StandardLibrary,
    ) -> TypeId {
        match ty {
            CatalogTypeRef::Core(core) => self.types.id_for_core(core),
            CatalogTypeRef::Standard(standard) => self.types.id_for_standard(standard),
            CatalogTypeRef::Parameter(name) | CatalogTypeRef::Associated(name) => variables[&name],
            CatalogTypeRef::Async(value) => {
                let value =
                    self.materialize_catalog_type(*value, variables, ids, constructed, library);
                if let Some(existing) = self.types.iter().find_map(|(id, kind)| {
                    matches!(kind, TypeKind::Async { value: candidate, .. } if *candidate == value)
                        .then_some(id)
                }) {
                    return existing;
                }
                let layout = ids.async_value();
                constructed.asyncs.push(crate::types::ResolvedAsyncType {
                    id: layout,
                    value: self.resolved_type_ref(value),
                });
                self.types.intern(TypeKind::Async { layout, value })
            }
            CatalogTypeRef::Iterator(item) => {
                let item =
                    self.materialize_catalog_type(*item, variables, ids, constructed, library);
                if let Some(existing) = self.types.iter().find_map(|(id, kind)| {
                    matches!(kind, TypeKind::Iterator { item: candidate, .. } if *candidate == item)
                        .then_some(id)
                }) {
                    return existing;
                }
                let layout = ids.iterator();
                constructed
                    .iterators
                    .push(crate::types::ResolvedIteratorType {
                        id: layout,
                        item: self.resolved_type_ref(item),
                    });
                self.types.intern(TypeKind::Iterator { layout, item })
            }
            CatalogTypeRef::FixedArray { element, length } => {
                let element =
                    self.materialize_catalog_type(*element, variables, ids, constructed, library);
                self.materialize_catalog_array(element, Some(length), ids, constructed)
            }
            CatalogTypeRef::Callable { parameters, result } => {
                let parameters = parameters
                    .iter()
                    .map(|parameter| {
                        self.materialize_catalog_type(
                            *parameter,
                            variables,
                            ids,
                            constructed,
                            library,
                        )
                    })
                    .collect::<Vec<_>>();
                let result =
                    self.materialize_catalog_type(*result, variables, ids, constructed, library);
                if let Some(existing) = self.types.iter().find_map(|(id, kind)| {
                    matches!(kind, TypeKind::Callable { parameters: candidate_parameters, result: candidate_result, .. }
                        if *candidate_parameters == parameters && *candidate_result == result)
                    .then_some(id)
                }) {
                    return existing;
                }
                let layout = ids.callable();
                constructed.callables.push(ResolvedCallableType {
                    id: layout,
                    parameters: parameters
                        .iter()
                        .map(|parameter| self.resolved_type_ref(*parameter))
                        .collect(),
                    result: self.resolved_type_ref(result),
                });
                self.types.intern(TypeKind::Callable {
                    layout,
                    parameters,
                    result,
                })
            }
            CatalogTypeRef::Application {
                constructor,
                arguments,
            } => {
                let arguments = arguments
                    .iter()
                    .map(|argument| {
                        self.materialize_catalog_type(
                            *argument,
                            variables,
                            ids,
                            constructed,
                            library,
                        )
                    })
                    .collect::<Vec<_>>();
                if let Some(existing) = self.types.iter().find_map(|(id, kind)| {
                    let matches = match kind {
                        TypeKind::Array {
                            element,
                            length: None,
                            ..
                        } => {
                            constructor == StdlibTypeConstructorId::Array
                                && arguments.as_slice() == [*element]
                        }
                        TypeKind::Option { value, .. } => {
                            constructor == StdlibTypeConstructorId::Option
                                && arguments.as_slice() == [*value]
                        }
                        TypeKind::Result { value, .. } => {
                            constructor == StdlibTypeConstructorId::Result
                                && arguments.as_slice() == [*value]
                        }
                        TypeKind::Set { element, .. } => {
                            constructor == StdlibTypeConstructorId::Set
                                && arguments.as_slice() == [*element]
                        }
                        TypeKind::Range { bound, kind, .. } => {
                            let expected = match kind {
                                crate::ast::RangeKind::Exclusive => {
                                    StdlibTypeConstructorId::ExclusiveRange
                                }
                                crate::ast::RangeKind::Inclusive => {
                                    StdlibTypeConstructorId::InclusiveRange
                                }
                            };
                            constructor == expected && arguments.as_slice() == [*bound]
                        }
                        TypeKind::Application {
                            constructor: candidate,
                            arguments: candidate_arguments,
                            ..
                        } => *candidate == constructor && *candidate_arguments == arguments,
                        _ => false,
                    };
                    matches.then_some(id)
                }) {
                    return existing;
                }
                match constructor {
                    StdlibTypeConstructorId::Array => {
                        self.materialize_catalog_array(arguments[0], None, ids, constructed)
                    }
                    StdlibTypeConstructorId::Option => {
                        let layout = ids.option();
                        let value = arguments[0];
                        constructed.options.push(ResolvedOptionType {
                            id: layout,
                            value: self.resolved_type_ref(value),
                        });
                        self.types.intern(TypeKind::Option { layout, value })
                    }
                    StdlibTypeConstructorId::Result => {
                        let layout = ids.result();
                        let value = arguments[0];
                        constructed.results.push(ResolvedResultType {
                            id: layout,
                            value: self.resolved_type_ref(value),
                        });
                        self.types.intern(TypeKind::Result { layout, value })
                    }
                    StdlibTypeConstructorId::Set => {
                        let element = arguments[0];
                        let backing_type =
                            self.materialize_catalog_array(element, None, ids, constructed);
                        let backing = match self.types.kind(backing_type) {
                            TypeKind::Array { layout, .. } => *layout,
                            _ => unreachable!(),
                        };
                        let layout = ids.application();
                        constructed.sets.push(ResolvedSetType {
                            id: layout,
                            element: self.resolved_type_ref(element),
                            backing,
                        });
                        self.types.intern(TypeKind::Set {
                            layout,
                            element,
                            backing,
                        })
                    }
                    StdlibTypeConstructorId::ExclusiveRange
                    | StdlibTypeConstructorId::InclusiveRange => {
                        let bound = arguments[0];
                        let kind = if constructor == StdlibTypeConstructorId::ExclusiveRange {
                            crate::ast::RangeKind::Exclusive
                        } else {
                            crate::ast::RangeKind::Inclusive
                        };
                        let layout = ids.range();
                        constructed.ranges.push(ResolvedRangeType {
                            id: layout,
                            bound: self.resolved_type_ref(bound),
                            kind,
                        });
                        self.types.intern(TypeKind::Range {
                            layout,
                            bound,
                            kind,
                        })
                    }
                    _ => {
                        let layout = ids.application();
                        constructed
                            .applications
                            .push(crate::types::ResolvedApplicationType {
                                id: layout,
                                constructor,
                                arguments: arguments
                                    .iter()
                                    .map(|argument| self.resolved_type_ref(*argument))
                                    .collect(),
                            });
                        let result = self.types.intern(TypeKind::Application {
                            layout,
                            constructor,
                            arguments: arguments.clone(),
                        });

                        // Materialize fields immediately so nested applications
                        // are complete before reachability asks for their GC
                        // dependencies.
                        let declaration = library.type_constructor(constructor);
                        let field_variables = declaration
                            .parameters
                            .iter()
                            .zip(&arguments)
                            .map(|(parameter, argument)| (parameter.name, *argument))
                            .collect::<HashMap<_, _>>();
                        for field in library.fields_of_constructor(constructor) {
                            self.materialize_catalog_type(
                                field.ty,
                                &field_variables,
                                ids,
                                constructed,
                                library,
                            );
                        }
                        result
                    }
                }
            }
        }
    }

    fn materialize_catalog_array(
        &mut self,
        element: TypeId,
        length: Option<u32>,
        ids: &mut crate::ast::ConstructedTypeIdAllocator,
        constructed: &mut ResolvedConstructedTypesMut<'_>,
    ) -> TypeId {
        if let Some(existing) = self.types.iter().find_map(|(id, kind)| {
            matches!(kind, TypeKind::Array { element: candidate, length: candidate_length, .. }
                if *candidate == element && *candidate_length == length)
            .then_some(id)
        }) {
            return existing;
        }
        let layout = ids.array();
        constructed.arrays.push(ResolvedArrayType {
            id: layout,
            element: self.resolved_type_ref(element),
            length,
        });
        self.array_element_types.insert(layout, element);
        self.types.intern(TypeKind::Array {
            layout,
            element,
            length,
        })
    }

    fn resolved_type_ref(&self, ty: TypeId) -> crate::types::ResolvedTypeRef {
        match self.types.kind(ty) {
            TypeKind::Error => crate::types::ResolvedTypeRef::Error,
            TypeKind::Builtin(core) => crate::types::ResolvedTypeRef::Core(*core),
            TypeKind::Standard(standard) => crate::types::ResolvedTypeRef::Standard(*standard),
            TypeKind::StateSnapshot => crate::types::ResolvedTypeRef::StateSnapshot,
            TypeKind::SettingsView => crate::types::ResolvedTypeRef::SettingsView,
            TypeKind::Struct(structure) => crate::types::ResolvedTypeRef::Struct(*structure),
            TypeKind::Enum(enumeration) => crate::types::ResolvedTypeRef::Enum(*enumeration),
            TypeKind::ManagedClass(class) => crate::types::ResolvedTypeRef::ManagedClass(*class),
            TypeKind::ManagedReference(class) => {
                crate::types::ResolvedTypeRef::ManagedReference(*class)
            }
            TypeKind::GenericParameter { .. } => {
                crate::types::ResolvedTypeRef::GenericParameter(ty)
            }
            TypeKind::Array { layout, .. } => crate::types::ResolvedTypeRef::Array(*layout),
            TypeKind::Option { layout, .. } => crate::types::ResolvedTypeRef::Option(*layout),
            TypeKind::Result { layout, .. } => crate::types::ResolvedTypeRef::Result(*layout),
            TypeKind::Async { layout, .. } => crate::types::ResolvedTypeRef::Async(*layout),
            TypeKind::Iterator { layout, .. } => crate::types::ResolvedTypeRef::Iterator(*layout),
            TypeKind::Callable { layout, .. } => crate::types::ResolvedTypeRef::Callable(*layout),
            TypeKind::Set { layout, .. } => crate::types::ResolvedTypeRef::Set(*layout),
            TypeKind::Range { layout, .. } => crate::types::ResolvedTypeRef::Range(*layout),
            TypeKind::Application { layout, .. } => {
                crate::types::ResolvedTypeRef::Application(*layout)
            }
        }
    }

    fn specialize_signature_node(
        &self,
        template: TypeId,
        concrete: TypeId,
        searched: TypeId,
    ) -> Option<TypeId> {
        if template == searched {
            return Some(concrete);
        }
        match (self.types.kind(template), self.types.kind(concrete)) {
            (
                TypeKind::Array {
                    element: template, ..
                },
                TypeKind::Array {
                    element: concrete, ..
                },
            )
            | (
                TypeKind::Option {
                    value: template, ..
                },
                TypeKind::Option {
                    value: concrete, ..
                },
            )
            | (
                TypeKind::Result {
                    value: template, ..
                },
                TypeKind::Result {
                    value: concrete, ..
                },
            )
            | (
                TypeKind::Async {
                    value: template, ..
                },
                TypeKind::Async {
                    value: concrete, ..
                },
            )
            | (
                TypeKind::Iterator { item: template, .. },
                TypeKind::Iterator { item: concrete, .. },
            ) => self.specialize_signature_node(*template, *concrete, searched),
            (
                TypeKind::Range {
                    bound: template, ..
                },
                TypeKind::Range {
                    bound: concrete, ..
                },
            ) => self.specialize_signature_node(*template, *concrete, searched),
            (
                TypeKind::Set {
                    element: template, ..
                },
                TypeKind::Set {
                    element: concrete, ..
                },
            ) => self.specialize_signature_node(*template, *concrete, searched),
            (
                TypeKind::Application {
                    constructor: template_constructor,
                    arguments: template_arguments,
                    ..
                },
                TypeKind::Application {
                    constructor: concrete_constructor,
                    arguments: concrete_arguments,
                    ..
                },
            ) if template_constructor == concrete_constructor
                && template_arguments.len() == concrete_arguments.len() =>
            {
                template_arguments.iter().zip(concrete_arguments).find_map(
                    |(template, concrete)| {
                        self.specialize_signature_node(*template, *concrete, searched)
                    },
                )
            }
            (
                TypeKind::Callable {
                    parameters: template_parameters,
                    result: template_result,
                    ..
                },
                TypeKind::Callable {
                    parameters: concrete_parameters,
                    result: concrete_result,
                    ..
                },
            ) if template_parameters.len() == concrete_parameters.len() => template_parameters
                .iter()
                .zip(concrete_parameters)
                .find_map(|(template, concrete)| {
                    self.specialize_signature_node(*template, *concrete, searched)
                })
                .or_else(|| {
                    self.specialize_signature_node(*template_result, *concrete_result, searched)
                }),
            _ => None,
        }
    }

    pub fn specialize_function_instance(
        &self,
        owner: &FunctionInstance,
        called: &FunctionInstance,
    ) -> FunctionInstance {
        FunctionInstance {
            function: called.function,
            type_arguments: called
                .type_arguments
                .iter()
                .map(|ty| self.specialize_type(owner, *ty))
                .collect(),
            signature: called
                .signature
                .iter()
                .map(|ty| self.specialize_type(owner, *ty))
                .collect(),
        }
    }

    pub(crate) fn set_function_type_parameters(
        &mut self,
        parameters: HashMap<FunctionId, Vec<TypeId>>,
        constraints: HashMap<TypeId, Vec<crate::stdlib::StdlibCapabilityId>>,
        associated_projections: HashMap<FunctionId, Vec<FunctionAssociatedProjection>>,
    ) {
        self.function_type_parameters = parameters;
        self.generic_parameter_constraints = constraints;
        self.function_associated_projections = associated_projections;
    }

    pub(crate) fn set_function_parameter_types(
        &mut self,
        parameters: HashMap<FunctionId, Vec<TypeId>>,
    ) {
        self.function_parameter_types = parameters;
    }

    pub(crate) fn set_source_associated_types(
        &mut self,
        associated_types: HashMap<
            (TypeId, crate::stdlib::StdlibCapabilityId, &'static str),
            TypeId,
        >,
    ) {
        self.source_associated_types = associated_types;
    }

    pub fn struct_field_type(&self, field: StructFieldId) -> Option<TypeId> {
        self.struct_field_types.get(&field).copied()
    }

    pub fn struct_field_types(&self) -> impl Iterator<Item = (StructFieldId, TypeId)> + '_ {
        self.struct_field_types
            .iter()
            .map(|(field, ty)| (*field, *ty))
    }

    pub fn enum_representation(&self, enumeration: EnumId) -> Option<TypeId> {
        self.enum_representations.get(&enumeration).copied()
    }

    pub fn managed_field_type(&self, field: ManagedFieldId) -> Option<TypeId> {
        self.managed_field_types.get(&field).copied()
    }

    /// Returns the source value produced by reading a managed field.
    ///
    /// A class name in a managed schema describes the metadata field's class,
    /// while its runtime value is a live managed reference. Other storage
    /// schemas produce their recursively owned value, such as arrays for lists.
    pub fn managed_field_value_type(&self, field: ManagedFieldId) -> Option<TypeId> {
        let declared = self.managed_field_type(field)?;
        Some(match self.types.kind(declared) {
            TypeKind::ManagedClass(class) => self.types.id_for_managed_reference(*class),
            TypeKind::Option { value, .. }
                if matches!(self.types.kind(*value), TypeKind::ManagedClass(_)) =>
            {
                let TypeKind::ManagedClass(class) = self.types.kind(*value) else {
                    unreachable!()
                };
                let live = self.types.id_for_managed_reference(*class);
                self.types
                    .iter()
                    .find_map(|(id, kind)| {
                        matches!(kind, TypeKind::Option { value, .. } if *value == live)
                            .then_some(id)
                    })
                    .expect(
                        "nullable managed reference projections are materialized during checking",
                    )
            }
            _ => self.managed_owned_type(declared),
        })
    }

    pub fn managed_field_types(&self) -> impl Iterator<Item = (ManagedFieldId, TypeId)> + '_ {
        self.managed_field_types
            .iter()
            .map(|(field, ty)| (*field, *ty))
    }

    /// The owned value stored in a class snapshot. A declared child class
    /// remains a class value here; only live access projects it to `C.Ref`.
    pub fn managed_field_snapshot_type(&self, field: ManagedFieldId) -> Option<TypeId> {
        self.managed_field_type(field)
            .map(|ty| self.managed_owned_type(ty))
    }

    /// Projects a remote storage schema to its recursively owned value type.
    /// The corresponding layouts are materialized during type checking.
    pub(crate) fn managed_owned_type(&self, ty: TypeId) -> TypeId {
        self.try_managed_owned_type(ty)
            .expect("managed field projections are materialized during checking")
    }

    pub(crate) fn try_managed_owned_type(&self, ty: TypeId) -> Option<TypeId> {
        if let TypeKind::Application {
            constructor,
            arguments,
            ..
        } = self.types.kind(ty)
            && *constructor == crate::stdlib::StdlibTypeConstructorId::Map
        {
            let owned = arguments
                .iter()
                .map(|arg| self.try_managed_owned_type(*arg))
                .collect::<Option<Vec<_>>>()?;
            if &owned == arguments {
                return Some(ty);
            }
            return self.types.iter().find_map(|(id, kind)| {
                matches!(kind, TypeKind::Application { constructor: candidate, arguments, .. }
                        if candidate == constructor && arguments == &owned)
                .then_some(id)
            });
        }
        let shape = match self.types.kind(ty) {
            TypeKind::Application {
                constructor,
                arguments,
                ..
            } if *constructor == crate::stdlib::StdlibTypeConstructorId::List => {
                Some((self.try_managed_owned_type(arguments[0])?, None))
            }
            TypeKind::Array {
                element, length, ..
            } => {
                let owned = self.try_managed_owned_type(*element)?;
                if owned == *element {
                    return Some(ty);
                }
                Some((owned, *length))
            }
            TypeKind::Option { value, .. } => {
                let owned = self.try_managed_owned_type(*value)?;
                if owned == *value {
                    return Some(ty);
                }
                return self.types.iter().find_map(|(id, kind)| {
                    matches!(kind, TypeKind::Option { value, .. } if *value == owned).then_some(id)
                });
            }
            _ => None,
        };
        let Some((element, length)) = shape else {
            return Some(ty);
        };
        self.types.iter().find_map(|(id, kind)| {
            matches!(kind, TypeKind::Array { element: candidate, length: size, .. }
                if *candidate == element && *size == length)
            .then_some(id)
        })
    }

    pub fn standard_field_type(&self, field: StdlibFieldId) -> Option<TypeId> {
        self.standard_field_types.get(&field).copied()
    }

    /// Resolves a catalog type after substituting concrete constructor or
    /// function type arguments. All aggregate layouts are materialized during
    /// semantic finalization, so later compiler stages and editor tooling can
    /// share this read-only lookup instead of each reimplementing catalog
    /// substitution.
    pub fn instantiated_catalog_type(
        &self,
        ty: CatalogTypeRef,
        variables: &HashMap<&'static str, TypeId>,
    ) -> Option<TypeId> {
        match ty {
            CatalogTypeRef::Core(core) => Some(self.types.id_for_core(core)),
            CatalogTypeRef::Standard(standard) => Some(self.types.id_for_standard(standard)),
            CatalogTypeRef::Parameter(name) | CatalogTypeRef::Associated(name) => {
                variables.get(name).copied()
            }
            CatalogTypeRef::Async(value) => {
                let value = self.instantiated_catalog_type(*value, variables)?;
                self.types.iter().find_map(|(id, kind)| {
                    matches!(kind, TypeKind::Async { value: candidate, .. } if *candidate == value)
                        .then_some(id)
                })
            }
            CatalogTypeRef::Iterator(item) => {
                let item = self.instantiated_catalog_type(*item, variables)?;
                self.types.iter().find_map(|(id, kind)| {
                    matches!(kind, TypeKind::Iterator { item: candidate, .. } if *candidate == item)
                        .then_some(id)
                })
            }
            CatalogTypeRef::FixedArray { element, length } => {
                let element = self.instantiated_catalog_type(*element, variables)?;
                self.types.iter().find_map(|(id, kind)| {
                    matches!(kind, TypeKind::Array {
                        element: candidate,
                        length: candidate_length,
                        ..
                    } if *candidate == element && *candidate_length == Some(length))
                    .then_some(id)
                })
            }
            CatalogTypeRef::Callable { parameters, result } => {
                let parameters = parameters
                    .iter()
                    .map(|parameter| self.instantiated_catalog_type(*parameter, variables))
                    .collect::<Option<Vec<_>>>()?;
                let result = self.instantiated_catalog_type(*result, variables)?;
                self.types.iter().find_map(|(id, kind)| {
                    matches!(kind, TypeKind::Callable {
                        parameters: candidate_parameters,
                        result: candidate_result,
                        ..
                    } if *candidate_parameters == parameters && *candidate_result == result)
                    .then_some(id)
                })
            }
            CatalogTypeRef::Application {
                constructor,
                arguments,
            } => {
                let arguments = arguments
                    .iter()
                    .map(|argument| self.instantiated_catalog_type(*argument, variables))
                    .collect::<Option<Vec<_>>>()?;
                self.types.iter().find_map(|(id, kind)| {
                    let matches = match kind {
                        TypeKind::Array { element, .. } => {
                            constructor == StdlibTypeConstructorId::Array
                                && arguments.as_slice() == [*element]
                        }
                        TypeKind::Option { value, .. } => {
                            constructor == StdlibTypeConstructorId::Option
                                && arguments.as_slice() == [*value]
                        }
                        TypeKind::Result { value, .. } => {
                            constructor == StdlibTypeConstructorId::Result
                                && arguments.as_slice() == [*value]
                        }
                        TypeKind::Set { element, .. } => {
                            constructor == StdlibTypeConstructorId::Set
                                && arguments.as_slice() == [*element]
                        }
                        TypeKind::Range { bound, kind, .. } => {
                            let expected = match kind {
                                crate::ast::RangeKind::Exclusive => {
                                    StdlibTypeConstructorId::ExclusiveRange
                                }
                                crate::ast::RangeKind::Inclusive => {
                                    StdlibTypeConstructorId::InclusiveRange
                                }
                            };
                            constructor == expected && arguments.as_slice() == [*bound]
                        }
                        TypeKind::Application {
                            constructor: candidate,
                            arguments: candidate_arguments,
                            ..
                        } => *candidate == constructor && *candidate_arguments == arguments,
                        _ => false,
                    };
                    matches.then_some(id)
                })
            }
        }
    }

    pub fn enum_variant_payload(&self, variant: EnumVariantId) -> Option<TypeId> {
        self.enum_variant_payloads.get(&variant).copied().flatten()
    }

    pub fn enum_variant_payloads(
        &self,
    ) -> impl Iterator<Item = (EnumVariantId, Option<TypeId>)> + '_ {
        self.enum_variant_payloads
            .iter()
            .map(|(variant, payload)| (*variant, *payload))
    }

    pub fn array_element_type(&self, array: ArrayTypeId) -> Option<TypeId> {
        self.array_element_types.get(&array).copied()
    }

    pub fn array_element_types(&self) -> impl Iterator<Item = (ArrayTypeId, TypeId)> + '_ {
        self.array_element_types
            .iter()
            .map(|(array, element)| (*array, *element))
    }

    pub fn state_poll_result(&self, field: ValueId) -> Option<TypeId> {
        self.state_poll_results.get(&field).copied()
    }

    /// Direct candidate-state dependencies of one physical field declaration.
    /// The order is stable by first source occurrence and contains no duplicates.
    pub fn state_dependencies(&self, field: ValueId) -> &[ValueId] {
        self.state_dependencies
            .get(&field)
            .map(Vec::as_slice)
            .unwrap_or(&[])
    }

    /// The result type produced by the nearest failure boundary for `value?`.
    pub fn propagation_target(&self, expression: ExprId) -> Option<TypeId> {
        self.propagation_targets.get(&expression).copied()
    }

    /// The retry expression which catches this propagation, when it does not
    /// return from the enclosing result-producing body.
    pub fn propagation_retry_boundary(&self, expression: ExprId) -> Option<ExprId> {
        self.propagation_retry_boundaries.get(&expression).copied()
    }

    pub fn path_members(&self, expression: ExprId) -> Option<&[ResolvedMember]> {
        self.path_members.get(&expression).map(Vec::as_slice)
    }

    /// Keep generated binding literals aligned with their pruned declarations.
    /// This only mutates the backend's private semantic model.
    pub(crate) fn remove_generated_struct_literal_fields(
        &mut self,
        removed: &std::collections::HashSet<StructFieldId>,
    ) {
        for fields in self.struct_literal_fields.values_mut() {
            fields.retain(
                |field| !matches!(field, ResolvedStructFieldId::Source(id) if removed.contains(id)),
            );
        }
    }

    pub fn struct_literal_fields(&self, expression: ExprId) -> Option<&[ResolvedStructFieldId]> {
        self.struct_literal_fields
            .get(&expression)
            .map(Vec::as_slice)
    }

    pub fn struct_literal(&self, expression: ExprId) -> Option<ResolvedStructId> {
        self.struct_literals.get(&expression).copied()
    }

    pub fn struct_pattern(&self, pattern: PatternId) -> Option<ResolvedStructId> {
        self.struct_patterns.get(&pattern).copied()
    }

    pub fn struct_pattern_fields(&self, pattern: PatternId) -> Option<&[ResolvedStructFieldId]> {
        self.struct_pattern_fields.get(&pattern).map(Vec::as_slice)
    }

    pub fn enum_variant(&self, expression: ExprId) -> Option<ResolvedEnumVariantId> {
        self.enum_variants.get(&expression).copied()
    }

    pub fn pattern_variant(&self, pattern: PatternId) -> Option<ResolvedEnumVariantId> {
        self.pattern_variants.get(&pattern).copied()
    }

    pub fn wrapper_pattern(&self, pattern: PatternId) -> Option<ResolvedWrapperPattern> {
        self.wrapper_patterns.get(&pattern).copied()
    }

    pub fn setting_choice_default(&self, setting: ValueId) -> Option<EnumVariantId> {
        self.setting_choice_defaults.get(&setting).copied()
    }

    pub fn setting_choice_option(&self, option: SettingChoiceOptionId) -> Option<EnumVariantId> {
        self.setting_choice_options.get(&option).copied()
    }

    pub fn assignment_target(&self, assignment: AssignmentId) -> Option<ValueId> {
        self.assignments.get(&assignment).copied()
    }

    pub fn assignment_call(&self, assignment: AssignmentId) -> Option<&ResolvedCall> {
        self.assignment_calls.get(&assignment)
    }

    pub fn index_assignment_setter(&self, assignment: AssignmentId) -> Option<&ResolvedCall> {
        self.index_assignment_setters.get(&assignment)
    }

    pub fn assignment_targets(&self) -> impl Iterator<Item = (AssignmentId, ValueId)> + '_ {
        self.assignments
            .iter()
            .map(|(assignment, target)| (*assignment, *target))
    }

    pub fn calls(&self) -> impl Iterator<Item = (ExprId, &ResolvedCall)> {
        self.calls
            .iter()
            .filter(|(expression, _)| {
                self.visible_expression_count
                    .is_none_or(|count| expression.index() < count)
            })
            .map(|(expression, call)| (*expression, call))
    }

    pub(crate) fn set_visible_expression_count(&mut self, count: usize) {
        self.visible_expression_count = Some(count);
    }

    pub fn value_conversion(&self, expression: ExprId) -> Option<ValueConversion> {
        self.value_conversions.get(&expression).copied()
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum PendingResolvedCall {
    UserFunction {
        function: FunctionId,
        type_arguments: Vec<Type>,
        signature: Vec<Type>,
    },
    UserMethod {
        function: FunctionId,
        type_arguments: Vec<Type>,
        signature: Vec<Type>,
        receiver: ResolvedReceiver,
        receiver_type: Type,
    },
    StandardLibrary {
        item: StdlibItemId,
        type_arguments: Vec<Type>,
        signature: Vec<Type>,
        receiver: Option<ResolvedReceiver>,
        receiver_type: Option<Type>,
    },
    ManagedSnapshot {
        class: ManagedClassId,
        result: crate::ast::ResultTypeId,
        receiver: ResolvedReceiver,
        receiver_type: Type,
    },
    ManagedComponent {
        class: ManagedClassId,
        result: crate::ast::ResultTypeId,
        receiver: ResolvedReceiver,
        receiver_type: Type,
    },
    ManagedInstances {
        class: ManagedClassId,
    },
    ResultError {
        result: crate::ast::ResultTypeId,
    },
    OptionSome {
        option: crate::ast::OptionTypeId,
    },
    IteratorItem {
        step: TypeApplicationId,
    },
    ResultSuccess {
        result: crate::ast::ResultTypeId,
    },
}

#[derive(Debug, Clone)]
pub(crate) struct PendingFunctionValue {
    pub function: FunctionId,
    pub type_arguments: Vec<Type>,
    pub signature: Vec<Type>,
}

#[derive(Debug, Clone, Copy)]
struct PendingValueConversion {
    kind: ValueConversionKind,
    source: Type,
    target: Type,
}

#[derive(Debug, Clone, Default)]
pub(crate) struct SemanticBuilder {
    state_provider: Option<StdlibStateProviderId>,
    state_provider_selector: Option<usize>,
    state_provider_alternatives: HashMap<EnumVariantId, (StdlibStateProviderId, Option<usize>)>,
    state_field_providers: HashMap<ValueId, StdlibStateProviderId>,
    expression_types: HashMap<ExprId, Type>,
    calls: HashMap<ExprId, PendingResolvedCall>,
    dynamic_calls: HashMap<ExprId, DynamicCallCallee>,
    function_values: HashMap<ExprId, PendingFunctionValue>,
    values: HashMap<ExprId, ResolvedValue>,
    value_types: HashMap<ValueId, Type>,
    action_results: HashMap<ActionKind, Type>,
    function_results: HashMap<FunctionId, Type>,
    function_completions: HashMap<FunctionId, Type>,
    struct_field_types: HashMap<StructFieldId, Type>,
    managed_field_types: HashMap<ManagedFieldId, Type>,
    standard_field_types: HashMap<StdlibFieldId, Type>,
    enum_variant_payloads: HashMap<EnumVariantId, Option<Type>>,
    enum_representations: HashMap<EnumId, Type>,
    array_element_types: HashMap<ArrayTypeId, Type>,
    state_storage_fields: Vec<ValueId>,
    state_storage_field_by_declaration: HashMap<ValueId, ValueId>,
    state_provider_fields: HashMap<EnumVariantId, Vec<ValueId>>,
    conditional_state_fields: HashMap<ValueId, ResolvedShapePredicate>,
    conditional_managed_fields: HashMap<ManagedFieldId, ResolvedShapePredicate>,
    state_poll_results: HashMap<ValueId, Type>,
    state_dependencies: HashMap<ValueId, Vec<ValueId>>,
    propagation_targets: HashMap<ExprId, Type>,
    propagation_retry_boundaries: HashMap<ExprId, ExprId>,
    path_members: HashMap<ExprId, Vec<ResolvedMember>>,
    struct_literals: HashMap<ExprId, ResolvedStructId>,
    struct_literal_fields: HashMap<ExprId, Vec<ResolvedStructFieldId>>,
    enum_variants: HashMap<ExprId, ResolvedEnumVariantId>,
    pattern_variants: HashMap<PatternId, ResolvedEnumVariantId>,
    wrapper_patterns: HashMap<PatternId, ResolvedWrapperPattern>,
    struct_patterns: HashMap<PatternId, ResolvedStructId>,
    struct_pattern_fields: HashMap<PatternId, Vec<ResolvedStructFieldId>>,
    setting_choice_defaults: HashMap<ValueId, EnumVariantId>,
    setting_choice_options: HashMap<SettingChoiceOptionId, EnumVariantId>,
    assignments: HashMap<AssignmentId, ValueId>,
    assignment_calls: HashMap<AssignmentId, PendingResolvedCall>,
    index_assignment_setters: HashMap<AssignmentId, PendingResolvedCall>,
    value_conversions: HashMap<ExprId, PendingValueConversion>,
}

impl SemanticBuilder {
    pub(crate) fn resolved_value(&self, expression: ExprId) -> Option<ResolvedValue> {
        self.values.get(&expression).copied()
    }

    pub(crate) fn resolved_assignment(&self, assignment: AssignmentId) -> Option<ValueId> {
        self.assignments.get(&assignment).copied()
    }

    pub(crate) fn inferred_expression_type(&self, expression: ExprId) -> Option<Type> {
        self.expression_types.get(&expression).copied()
    }

    pub(crate) fn call_is_provisionally_awaitable(
        &self,
        expression: ExprId,
        standard_library: &crate::stdlib::StandardLibrary,
    ) -> bool {
        match self.calls.get(&expression) {
            Some(PendingResolvedCall::StandardLibrary { item, .. }) => {
                standard_library
                    .operation_semantics(*item)
                    .suspension
                    .is_awaitable()
                    || matches!(
                        standard_library.item(*item).implementation,
                        crate::stdlib::Implementation::LibraryBody { .. }
                            | crate::stdlib::Implementation::LibraryOverloads { .. }
                    )
            }
            // Function effects are derived only after every body has been
            // checked. Accept the operand provisionally and let the
            // whole-program suspension validator reject synchronous callees.
            Some(
                PendingResolvedCall::UserFunction { .. }
                | PendingResolvedCall::UserMethod { .. }
                | PendingResolvedCall::ManagedInstances { .. },
            ) => true,
            Some(
                PendingResolvedCall::ResultError { .. }
                | PendingResolvedCall::OptionSome { .. }
                | PendingResolvedCall::IteratorItem { .. }
                | PendingResolvedCall::ResultSuccess { .. }
                | PendingResolvedCall::ManagedSnapshot { .. }
                | PendingResolvedCall::ManagedComponent { .. },
            )
            | None => false,
        }
    }

    pub(crate) fn resolve_recursive_call_type_arguments(
        &mut self,
        functions: &HashMap<FunctionId, Vec<Type>>,
    ) {
        for call in self.calls.values_mut() {
            match call {
                PendingResolvedCall::UserFunction {
                    function,
                    type_arguments,
                    ..
                }
                | PendingResolvedCall::UserMethod {
                    function,
                    type_arguments,
                    ..
                } if functions.contains_key(function) && type_arguments.is_empty() => {
                    *type_arguments = functions[function].clone();
                }
                _ => {}
            }
        }
    }
    pub(crate) fn with_state_provider(
        state_provider: Option<StdlibStateProviderId>,
        state_provider_selector: Option<usize>,
        state_provider_alternatives: HashMap<EnumVariantId, (StdlibStateProviderId, Option<usize>)>,
    ) -> Self {
        Self {
            state_provider,
            state_provider_selector,
            state_provider_alternatives,
            ..Self::default()
        }
    }

    pub(crate) fn resolve_state_field_provider(
        &mut self,
        field: ValueId,
        provider: StdlibStateProviderId,
    ) {
        let previous = self.state_field_providers.insert(field, provider);
        debug_assert!(previous.is_none(), "state-field providers are unique");
    }

    pub(crate) fn resolve_expression_type(&mut self, expression: ExprId, ty: Type) {
        let previous = self.expression_types.insert(expression, ty);
        debug_assert!(previous.is_none(), "expression IDs must be unique");
    }

    pub(crate) fn resolve_call(&mut self, expression: ExprId, call: PendingResolvedCall) {
        let previous = self.calls.insert(expression, call);
        debug_assert!(previous.is_none(), "call expression IDs must be unique");
    }

    pub(crate) fn resolve_dynamic_call(&mut self, expression: ExprId, callee: DynamicCallCallee) {
        let previous = self.dynamic_calls.insert(expression, callee);
        debug_assert!(
            previous.is_none(),
            "dynamic call expression IDs must be unique"
        );
    }

    pub(crate) fn resolve_function_value(
        &mut self,
        expression: ExprId,
        function: PendingFunctionValue,
    ) {
        let previous = self.function_values.insert(expression, function);
        debug_assert!(
            previous.is_none(),
            "function-value expression IDs must be unique"
        );
    }

    pub(crate) fn resolve_assignment_call(
        &mut self,
        assignment: AssignmentId,
        call: PendingResolvedCall,
    ) {
        let previous = self.assignment_calls.insert(assignment, call);
        debug_assert!(
            previous.is_none(),
            "assignment operator call must be unique"
        );
    }

    pub(crate) fn resolve_index_assignment_setter(
        &mut self,
        assignment: AssignmentId,
        call: PendingResolvedCall,
    ) {
        let previous = self.index_assignment_setters.insert(assignment, call);
        debug_assert!(
            previous.is_none(),
            "indexed-assignment setter call must be unique"
        );
    }

    pub(crate) fn resolve_value(&mut self, expression: ExprId, value: ResolvedValue) {
        let previous = self.values.insert(expression, value);
        debug_assert!(previous.is_none(), "path expression IDs must be unique");
    }

    pub(crate) fn resolve_value_type(&mut self, value: ValueId, ty: Type) {
        let previous = self.value_types.insert(value, ty);
        debug_assert!(previous.is_none(), "value IDs must be unique");
    }

    pub(crate) fn pending_value_type(&self, value: ValueId) -> Option<Type> {
        self.value_types.get(&value).copied()
    }

    pub(crate) fn resolve_action_result(&mut self, action: ActionKind, ty: Type) {
        let previous = self.action_results.insert(action, ty);
        debug_assert!(previous.is_none(), "action kinds must be unique");
    }

    pub(crate) fn resolve_state_storage(
        &mut self,
        storage_fields: Vec<ValueId>,
        storage_field_by_declaration: HashMap<ValueId, ValueId>,
        provider_fields: HashMap<EnumVariantId, Vec<ValueId>>,
    ) {
        debug_assert!(self.state_storage_fields.is_empty());
        self.state_storage_fields = storage_fields;
        self.state_storage_field_by_declaration = storage_field_by_declaration;
        self.state_provider_fields = provider_fields;
    }

    pub(crate) fn resolve_conditional_state_field(
        &mut self,
        field: ValueId,
        predicate: ResolvedShapePredicate,
    ) {
        let previous = self.conditional_state_fields.insert(field, predicate);
        debug_assert!(previous.is_none(), "conditional state fields are unique");
    }

    pub(crate) fn resolve_conditional_managed_field(
        &mut self,
        field: ManagedFieldId,
        predicate: ResolvedShapePredicate,
    ) {
        let previous = self.conditional_managed_fields.insert(field, predicate);
        debug_assert!(previous.is_none(), "conditional managed fields are unique");
    }

    pub(crate) fn resolve_function_result(&mut self, function: FunctionId, ty: Type) {
        let previous = self.function_results.insert(function, ty);
        debug_assert!(previous.is_none(), "function IDs must be unique");
    }

    pub(crate) fn resolve_function_completion(&mut self, function: FunctionId, ty: Type) {
        let previous = self.function_completions.insert(function, ty);
        debug_assert!(previous.is_none(), "function IDs must be unique");
    }

    pub(crate) fn resolve_struct_field_type(&mut self, field: StructFieldId, ty: Type) {
        let previous = self.struct_field_types.insert(field, ty);
        debug_assert!(previous.is_none(), "struct field IDs must be unique");
    }

    pub(crate) fn resolve_managed_field_type(&mut self, field: ManagedFieldId, ty: Type) {
        let previous = self.managed_field_types.insert(field, ty);
        debug_assert!(previous.is_none(), "managed field IDs must be unique");
    }

    pub(crate) fn resolve_standard_field_type(&mut self, field: StdlibFieldId, ty: Type) {
        let previous = self.standard_field_types.insert(field, ty);
        debug_assert!(previous.is_none(), "standard field IDs must be unique");
    }

    pub(crate) fn resolve_enum_variant_payload(
        &mut self,
        variant: EnumVariantId,
        payload: Option<Type>,
    ) {
        let previous = self.enum_variant_payloads.insert(variant, payload);
        debug_assert!(previous.is_none(), "enum variant IDs must be unique");
    }

    pub(crate) fn resolve_enum_representation(&mut self, enumeration: EnumId, ty: Type) {
        let previous = self.enum_representations.insert(enumeration, ty);
        debug_assert!(previous.is_none(), "enum IDs must be unique");
    }

    pub(crate) fn resolve_array_element_type(&mut self, array: ArrayTypeId, element: Type) {
        let previous = self.array_element_types.insert(array, element);
        debug_assert!(previous.is_none(), "array type IDs must be unique");
    }

    pub(crate) fn resolve_state_poll_result(&mut self, field: ValueId, result: Type) {
        let previous = self.state_poll_results.insert(field, result);
        debug_assert!(previous.is_none(), "state poll result must be unique");
    }

    pub(crate) fn resolve_state_dependency(&mut self, field: ValueId, dependency: ValueId) {
        let dependencies = self.state_dependencies.entry(field).or_default();
        if !dependencies.contains(&dependency) {
            dependencies.push(dependency);
        }
    }

    pub(crate) fn state_dependencies(&self, field: ValueId) -> &[ValueId] {
        self.state_dependencies
            .get(&field)
            .map(Vec::as_slice)
            .unwrap_or(&[])
    }

    pub(crate) fn resolve_propagation_target(
        &mut self,
        expression: ExprId,
        result: Type,
        retry: Option<ExprId>,
    ) {
        let previous = self.propagation_targets.insert(expression, result);
        debug_assert!(
            previous.is_none(),
            "propagation expression IDs must be unique"
        );
        if let Some(retry) = retry {
            let previous = self.propagation_retry_boundaries.insert(expression, retry);
            debug_assert!(
                previous.is_none(),
                "propagation boundary IDs must be unique"
            );
        }
    }

    pub(crate) fn resolve_path_members(
        &mut self,
        expression: ExprId,
        members: Vec<ResolvedMember>,
    ) {
        let previous = self.path_members.insert(expression, members);
        debug_assert!(previous.is_none(), "path expression IDs must be unique");
    }

    pub(crate) fn resolve_struct_literal_fields(
        &mut self,
        expression: ExprId,
        fields: Vec<ResolvedStructFieldId>,
    ) {
        let previous = self.struct_literal_fields.insert(expression, fields);
        debug_assert!(previous.is_none(), "struct expression IDs must be unique");
    }

    pub(crate) fn resolve_struct_literal(
        &mut self,
        expression: ExprId,
        structure: ResolvedStructId,
    ) {
        let previous = self.struct_literals.insert(expression, structure);
        debug_assert!(previous.is_none(), "struct expression IDs must be unique");
    }

    pub(crate) fn resolve_struct_pattern(
        &mut self,
        pattern: PatternId,
        structure: ResolvedStructId,
    ) {
        let previous = self.struct_patterns.insert(pattern, structure);
        debug_assert!(previous.is_none(), "struct pattern IDs must be unique");
    }

    pub(crate) fn resolve_struct_pattern_fields(
        &mut self,
        pattern: PatternId,
        fields: Vec<ResolvedStructFieldId>,
    ) {
        let previous = self.struct_pattern_fields.insert(pattern, fields);
        debug_assert!(previous.is_none(), "struct pattern IDs must be unique");
    }

    pub(crate) fn resolve_enum_variant(
        &mut self,
        expression: ExprId,
        variant: ResolvedEnumVariantId,
    ) {
        let previous = self.enum_variants.insert(expression, variant);
        debug_assert!(previous.is_none(), "enum expression IDs must be unique");
    }

    pub(crate) fn resolve_pattern_variant(
        &mut self,
        pattern: PatternId,
        variant: ResolvedEnumVariantId,
    ) {
        let previous = self.pattern_variants.insert(pattern, variant);
        debug_assert!(previous.is_none(), "pattern IDs must be unique");
    }

    pub(crate) fn resolve_wrapper_pattern(
        &mut self,
        pattern: PatternId,
        wrapper: ResolvedWrapperPattern,
    ) {
        let previous = self.wrapper_patterns.insert(pattern, wrapper);
        debug_assert!(previous.is_none(), "pattern IDs must be unique");
    }

    pub(crate) fn resolve_setting_choice_default(
        &mut self,
        setting: ValueId,
        variant: EnumVariantId,
    ) {
        let previous = self.setting_choice_defaults.insert(setting, variant);
        debug_assert!(previous.is_none(), "setting IDs must be unique");
    }

    pub(crate) fn resolve_setting_choice_option(
        &mut self,
        option: SettingChoiceOptionId,
        variant: EnumVariantId,
    ) {
        let previous = self.setting_choice_options.insert(option, variant);
        debug_assert!(previous.is_none(), "choice option IDs must be unique");
    }

    pub(crate) fn resolve_assignment(&mut self, assignment: AssignmentId, target: ValueId) {
        let previous = self.assignments.insert(assignment, target);
        debug_assert!(previous.is_none(), "assignment IDs must be unique");
    }

    pub(crate) fn resolve_value_conversion(
        &mut self,
        expression: ExprId,
        kind: ValueConversionKind,
        source: Type,
        target: Type,
    ) {
        let previous = self.value_conversions.insert(
            expression,
            PendingValueConversion {
                kind,
                source,
                target,
            },
        );
        debug_assert!(previous.is_none(), "expression conversion must be unique");
    }

    pub(crate) fn finish(
        self,
        mut types: TypeStore,
        constructed: ResolvedConstructedTypes<'_>,
        mut resolve: impl FnMut(Type) -> Type,
    ) -> SemanticModel {
        let ResolvedConstructedTypes {
            arrays,
            options,
            results,
            asyncs,
            iterators,
            callables,
            sets,
            applications,
        } = constructed;
        let constructed = ResolvedConstructedTypes {
            arrays,
            options,
            results,
            asyncs,
            iterators,
            callables,
            sets,
            applications,
        };
        let Self {
            state_provider,
            state_provider_selector,
            state_provider_alternatives,
            state_field_providers,
            expression_types,
            calls,
            dynamic_calls,
            function_values,
            values,
            value_types,
            action_results,
            function_results,
            function_completions,
            struct_field_types,
            managed_field_types,
            standard_field_types,
            enum_variant_payloads,
            enum_representations,
            array_element_types,
            state_storage_fields,
            state_storage_field_by_declaration,
            state_provider_fields,
            conditional_state_fields,
            conditional_managed_fields,
            state_poll_results,
            state_dependencies,
            propagation_targets,
            propagation_retry_boundaries,
            path_members,
            struct_literals,
            struct_literal_fields,
            enum_variants,
            pattern_variants,
            wrapper_patterns,
            struct_patterns,
            struct_pattern_fields,
            setting_choice_defaults,
            setting_choice_options,
            assignments,
            assignment_calls,
            index_assignment_setters,
            value_conversions,
        } = self;
        // WebAssembly GC layouts are nominal. Keep every allocated constructed
        // layout queryable even when inference created it only as a boundary
        // and no source declaration ultimately names it.
        for array in arrays {
            types.intern_inferred(Type::Array(array.id), constructed);
        }
        for option in options {
            types.intern_inferred(Type::Option(option.id), constructed);
        }
        for result in results {
            types.intern_inferred(Type::Result(result.id), constructed);
        }
        for future in asyncs {
            types.intern_inferred(Type::Async(future.id), constructed);
        }
        for callable in callables {
            types.intern_inferred(Type::Callable(callable.id), constructed);
        }
        for set in sets {
            types.intern_inferred(Type::Set(set.id), constructed);
        }
        for application in applications {
            types.intern_inferred(Type::Application(application.id), constructed);
        }
        let (calls, assignment_calls, index_assignment_setters) = {
            let mut finish_call = |call| match call {
                PendingResolvedCall::UserFunction {
                    function,
                    type_arguments,
                    signature,
                } => ResolvedCall::UserFunction {
                    function,
                    type_arguments: type_arguments
                        .into_iter()
                        .map(|ty| types.intern_inferred(resolve(ty), constructed))
                        .collect(),
                    signature: signature
                        .into_iter()
                        .map(|ty| types.intern_inferred(resolve(ty), constructed))
                        .collect(),
                },
                PendingResolvedCall::UserMethod {
                    function,
                    type_arguments,
                    signature,
                    receiver,
                    receiver_type,
                } => ResolvedCall::UserMethod {
                    function,
                    type_arguments: type_arguments
                        .into_iter()
                        .map(|ty| types.intern_inferred(resolve(ty), constructed))
                        .collect(),
                    signature: signature
                        .into_iter()
                        .map(|ty| types.intern_inferred(resolve(ty), constructed))
                        .collect(),
                    receiver,
                    receiver_type: types.intern_inferred(resolve(receiver_type), constructed),
                },
                PendingResolvedCall::StandardLibrary {
                    item,
                    type_arguments,
                    signature,
                    receiver,
                    receiver_type,
                } => ResolvedCall::StandardLibrary {
                    item,
                    type_arguments: type_arguments
                        .into_iter()
                        .map(|ty| types.intern_inferred(resolve(ty), constructed))
                        .collect(),
                    signature: signature
                        .into_iter()
                        .map(|ty| types.intern_inferred(resolve(ty), constructed))
                        .collect(),
                    receiver,
                    receiver_type: receiver_type
                        .map(|ty| types.intern_inferred(resolve(ty), constructed)),
                },
                PendingResolvedCall::ManagedSnapshot {
                    class,
                    result,
                    receiver,
                    receiver_type,
                } => ResolvedCall::ManagedSnapshot {
                    class,
                    result: resolved_result_layout(resolve(Type::Result(result)), &types),
                    receiver,
                    receiver_type: types.intern_inferred(resolve(receiver_type), constructed),
                },
                PendingResolvedCall::ManagedComponent {
                    class,
                    result,
                    receiver,
                    receiver_type,
                } => ResolvedCall::ManagedComponent {
                    class,
                    result: resolved_result_layout(resolve(Type::Result(result)), &types),
                    receiver,
                    receiver_type: types.intern_inferred(resolve(receiver_type), constructed),
                },
                PendingResolvedCall::ManagedInstances { class } => {
                    ResolvedCall::ManagedInstances { class }
                }
                PendingResolvedCall::ResultError { result } => {
                    let result = resolved_result_layout(resolve(Type::Result(result)), &types);
                    ResolvedCall::ResultError { result }
                }
                PendingResolvedCall::OptionSome { option } => {
                    let option = resolved_option_layout(resolve(Type::Option(option)), &types);
                    ResolvedCall::OptionSome { option }
                }
                PendingResolvedCall::IteratorItem { step } => {
                    let step =
                        resolved_application_layout(resolve(Type::Application(step)), &types);
                    ResolvedCall::IteratorItem { step }
                }
                PendingResolvedCall::ResultSuccess { result } => {
                    let result = resolved_result_layout(resolve(Type::Result(result)), &types);
                    ResolvedCall::ResultSuccess { result }
                }
            };
            let calls = calls
                .into_iter()
                .map(|(expression, call)| (expression, finish_call(call)))
                .collect();
            let assignment_calls = assignment_calls
                .into_iter()
                .map(|(assignment, call)| (assignment, finish_call(call)))
                .collect();
            let index_assignment_setters = index_assignment_setters
                .into_iter()
                .map(|(assignment, call)| (assignment, finish_call(call)))
                .collect();
            (calls, assignment_calls, index_assignment_setters)
        };
        let function_values = function_values
            .into_iter()
            .map(|(expression, function)| {
                let type_arguments = function
                    .type_arguments
                    .into_iter()
                    .map(|ty| types.intern_inferred(resolve(ty), constructed))
                    .collect();
                let signature = function
                    .signature
                    .into_iter()
                    .map(|ty| types.intern_inferred(resolve(ty), constructed))
                    .collect();
                (
                    expression,
                    FunctionInstance {
                        function: function.function,
                        type_arguments,
                        signature,
                    },
                )
            })
            .collect();
        let expression_types = expression_types
            .into_iter()
            .map(|(expression, ty)| (expression, types.intern_inferred(resolve(ty), constructed)))
            .collect();
        let value_types = value_types
            .into_iter()
            .map(|(value, ty)| (value, types.intern_inferred(resolve(ty), constructed)))
            .collect();
        let action_results = action_results
            .into_iter()
            .map(|(action, ty)| (action, types.intern_inferred(resolve(ty), constructed)))
            .collect();
        let function_results = function_results
            .into_iter()
            .map(|(function, ty)| (function, types.intern_inferred(resolve(ty), constructed)))
            .collect();
        let function_completions = function_completions
            .into_iter()
            .map(|(function, ty)| (function, types.intern_inferred(resolve(ty), constructed)))
            .collect();
        let struct_field_types = struct_field_types
            .into_iter()
            .map(|(field, ty)| (field, types.intern_inferred(resolve(ty), constructed)))
            .collect();
        let managed_field_types = managed_field_types
            .into_iter()
            .map(|(field, ty)| (field, types.intern_inferred(resolve(ty), constructed)))
            .collect();
        let standard_field_types = standard_field_types
            .into_iter()
            .map(|(field, ty)| (field, types.intern_inferred(resolve(ty), constructed)))
            .collect();
        let enum_variant_payloads = enum_variant_payloads
            .into_iter()
            .map(|(variant, payload)| {
                (
                    variant,
                    payload.map(|ty| types.intern_inferred(resolve(ty), constructed)),
                )
            })
            .collect();
        let enum_representations = enum_representations
            .into_iter()
            .map(|(enumeration, ty)| (enumeration, types.intern_inferred(resolve(ty), constructed)))
            .collect();
        let array_element_types = array_element_types
            .into_iter()
            .map(|(array, element)| (array, types.intern_inferred(resolve(element), constructed)))
            .collect();
        let state_poll_results = state_poll_results
            .into_iter()
            .map(|(field, result)| (field, types.intern_inferred(resolve(result), constructed)))
            .collect();
        let propagation_targets = propagation_targets
            .into_iter()
            .map(|(expression, result)| {
                (
                    expression,
                    types.intern_inferred(resolve(result), constructed),
                )
            })
            .collect();
        let value_conversions = value_conversions
            .into_iter()
            .map(|(expression, conversion)| {
                (
                    expression,
                    ValueConversion {
                        kind: conversion.kind,
                        source: types.intern_inferred(resolve(conversion.source), constructed),
                        target: types.intern_inferred(resolve(conversion.target), constructed),
                    },
                )
            })
            .collect();
        let wrapper_patterns = wrapper_patterns
            .into_iter()
            .map(|(pattern, wrapper)| {
                let wrapper = match wrapper {
                    ResolvedWrapperPattern::OptionNone(option) => {
                        let option = resolved_option_layout(resolve(Type::Option(option)), &types);
                        ResolvedWrapperPattern::OptionNone(option)
                    }
                    ResolvedWrapperPattern::OptionSome(option) => {
                        let option = resolved_option_layout(resolve(Type::Option(option)), &types);
                        ResolvedWrapperPattern::OptionSome(option)
                    }
                    ResolvedWrapperPattern::ResultSuccess(result) => {
                        let result = resolved_result_layout(resolve(Type::Result(result)), &types);
                        ResolvedWrapperPattern::ResultSuccess(result)
                    }
                    ResolvedWrapperPattern::ResultError(result) => {
                        let result = resolved_result_layout(resolve(Type::Result(result)), &types);
                        ResolvedWrapperPattern::ResultError(result)
                    }
                    ResolvedWrapperPattern::IteratorEnd(step) => {
                        let step =
                            resolved_application_layout(resolve(Type::Application(step)), &types);
                        ResolvedWrapperPattern::IteratorEnd(step)
                    }
                    ResolvedWrapperPattern::IteratorItem(step) => {
                        let step =
                            resolved_application_layout(resolve(Type::Application(step)), &types);
                        ResolvedWrapperPattern::IteratorItem(step)
                    }
                };
                (pattern, wrapper)
            })
            .collect();
        SemanticModel {
            types,
            state_provider,
            state_provider_selector,
            state_provider_alternatives,
            state_field_providers,
            expression_types,
            calls,
            dynamic_calls,
            function_values,
            values,
            value_types,
            action_results,
            function_results,
            function_completions,
            function_parameter_types: HashMap::new(),
            function_type_parameters: HashMap::new(),
            function_associated_projections: HashMap::new(),
            source_associated_types: HashMap::new(),
            generic_parameter_constraints: HashMap::new(),
            specialized_types: HashMap::new(),
            struct_field_types,
            managed_field_types,
            standard_field_types,
            enum_variant_payloads,
            enum_representations,
            array_element_types,
            state_storage_fields,
            state_storage_field_by_declaration,
            state_provider_fields,
            conditional_state_fields,
            conditional_managed_fields,
            state_poll_results,
            state_dependencies,
            propagation_targets,
            propagation_retry_boundaries,
            path_members,
            struct_literals,
            struct_literal_fields,
            enum_variants,
            pattern_variants,
            wrapper_patterns,
            struct_patterns,
            struct_pattern_fields,
            setting_choice_defaults,
            setting_choice_options,
            assignments,
            assignment_calls,
            index_assignment_setters,
            value_conversions,
            visible_expression_count: None,
        }
    }
}
