//! Wasm-oriented control-flow and storage plans lowered from typed HIR.
//!
//! This IR deliberately remains close to structured WebAssembly. It owns
//! block terminators, suspension continuations, user/scratch locals, and the
//! complete expression plan consumed by WebAssembly emission. Expression
//! nodes retain semantic IDs and type/conversion edges without depending on
//! source-shaped typed HIR during backend encoding.

use std::collections::{BTreeSet, HashMap, HashSet};

mod snapshot_projections;
mod visit;
pub use visit::{
    Visitor, visit_expression_children, walk_expression, walk_statement, walk_terminator,
};

use crate::{
    ast::{
        ActionKind, BinaryOp, ExprId, ManagedClassId, ManagedFieldId, OptionTypeId, PatternId,
        ResultTypeId, Span, SuspensionMode, UnaryOp, ValueId,
    },
    effects::OperationAnalysis,
    hir::{
        self, ExpressionResolution, FailureTarget, ImplicitConversion, TypedExpression,
        TypedExpressionKind, TypedInterpolatedPart, TypedPattern, TypedProgram, TypedStatementKind,
    },
    intrinsic_registry::{self, ScratchPolicy, ScratchType},
    semantic::{
        FunctionInstance, ResolvedCall, ResolvedEnumVariantId, ResolvedMember, ResolvedReceiver,
        ResolvedStructFieldId, ResolvedStructId, ResolvedValue, ResolvedWrapperPattern,
        SemanticModel, ValueConversion,
    },
    stdlib::{CancellationKind, Implementation, IntrinsicId, StandardLibrary, SuspensionKind},
    types::{EnumTypeId, TypeId, TypeKind},
};

#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub enum BodyOwner {
    Function(FunctionInstance),
    Action(ActionKind),
}

#[derive(Debug, Clone, Copy)]
struct SourceProvenance {
    profile: crate::BuildProfile,
    visible: bool,
}

impl SourceProvenance {
    const fn emits_debug_locations(self) -> bool {
        self.visible && matches!(self.profile, crate::BuildProfile::Debug)
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct LocalId(usize);

impl LocalId {
    pub const fn index(self) -> usize {
        self.0
    }
}

/// Identity of a compiler-owned expression temporary.
///
/// Async normalization uses these to preserve source evaluation order without
/// fabricating a user-visible [`ValueId`].
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct TemporaryId(u32);

impl TemporaryId {
    pub const fn index(self) -> u32 {
        self.0
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct AsyncStateId(u32);

impl AsyncStateId {
    pub const ENTRY: Self = Self(0);

    pub const fn index(self) -> u32 {
        self.0
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum LocalPurpose {
    Value(ValueId),
    Temporary(TemporaryId),
    SnapshotProjection(SnapshotProjection),
    MatchValue(ExprId),
    FallbackValue(ExprId),
    IntrinsicScratch { expression: ExprId, slot: u8 },
    SuspensionScratch(ExprId),
}

/// A compiler-owned local that reuses one immutable managed snapshot stored in
/// the current or old state snapshot. This deliberately does not represent a
/// general process-memory read: only values already materialized by the state
/// transaction are stable for the duration of a synchronous body invocation.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct SnapshotProjection {
    pub root: SnapshotRoot,
    pub field: ValueId,
    /// One direct field of the immutable managed snapshot, or `None` for the
    /// state field containing the managed snapshot itself.
    pub member: Option<ManagedFieldId>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum SnapshotRoot {
    Current,
    Old,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Local {
    pub id: LocalId,
    pub ty: TypeId,
    pub purpose: LocalPurpose,
}

#[derive(Debug, Clone)]
pub struct Expression {
    pub id: ExprId,
    pub ty: TypeId,
    pub kind: ExpressionKind,
    /// User-source origin retained for debugger line tables. Expressions from
    /// injected library bodies and purely generated scaffolding have no
    /// source location in the autosplitter file.
    pub source: Option<Span>,
    /// Type-checker-inserted wrapper lift on this expression edge.
    pub conversion: Option<ValueConversion>,
}

#[derive(Debug, Clone)]
pub enum ExpressionKind {
    None,
    IteratorEnd,
    Bool(bool),
    Int {
        value: u64,
        negative: bool,
    },
    Float(crate::ast::FloatLiteral),
    Char(char),
    String(String),
    InterpolatedString(Vec<InterpolatedPart>),
    Signature(String),
    Array(Vec<ExprId>),
    Range {
        start: ExprId,
        end: ExprId,
        kind: crate::ast::RangeKind,
    },
    /// Marker lowered through statement-aware expression normalization before
    /// code generation. Its body remains in typed HIR so it can preserve
    /// lexical control-flow and suspension boundaries.
    ValueBlock,
    /// Statement-aware marker for a value-producing infinite loop.
    Loop,
    Struct {
        structure: ResolvedStructId,
        fields: Vec<(ResolvedStructFieldId, ExprId)>,
    },
    Enum {
        enumeration: EnumTypeId,
        variant: ResolvedEnumVariantId,
        payload: Option<ExprId>,
    },
    Path {
        root: Option<ResolvedValue>,
        members: Vec<ResolvedMember>,
    },
    Member {
        receiver: ExprId,
        members: Vec<ResolvedMember>,
    },
    Index {
        receiver: ExprId,
        index: ExprId,
    },
    Unary {
        op: UnaryOp,
        operand: ExprId,
    },
    Cast {
        value: ExprId,
    },
    Binary {
        op: BinaryOp,
        left: ExprId,
        right: ExprId,
    },
    Call {
        target: CallTarget,
        arguments: Vec<ExprId>,
    },
    Invoke {
        callee: crate::semantic::DynamicCallCallee,
        arguments: Vec<ExprId>,
    },
    FunctionValue {
        function: crate::semantic::FunctionInstance,
    },
    Closure {
        closure: ExprId,
        parameters: Vec<ValueId>,
        body: ExprId,
    },
    If {
        condition: ExprId,
        then_expr: ExprId,
        else_expr: ExprId,
    },
    Fallback {
        value: ExprId,
        fallback: ExprId,
    },
    Break(Option<ExprId>),
    Continue,
    Return(Option<ExprId>),
    Throw {
        error: ExprId,
        target: FailureTarget,
    },
    Inspect {
        label: String,
        value: ExprId,
    },
    /// A source expression whose wrapper has no release-runtime behavior.
    /// Keeping the source expression identity while forwarding its operand
    /// preserves conversions, control flow, and editor/debug metadata.
    Transparent {
        value: ExprId,
    },
    Suspend {
        mode: SuspensionMode,
        destination: ValueId,
        value: ExprId,
    },
    Temporary(TemporaryId),
    FallbackSuccess {
        source: ExprId,
    },
    Propagate {
        value: ExprId,
        target: FailureTarget,
    },
    Match {
        value: ExprId,
        arms: Vec<MatchArm>,
    },
    Is {
        value: ExprId,
        pattern_id: PatternId,
        pattern: LoweredPattern,
    },
}

#[derive(Debug, Clone)]
pub enum CallTarget {
    UserFunction {
        function: FunctionInstance,
    },
    UserMethod {
        function: FunctionInstance,
        receiver: ResolvedReceiver,
        receiver_type: TypeId,
    },
    Intrinsic {
        item: crate::stdlib::StdlibItemId,
        intrinsic: IntrinsicId,
        type_arguments: Vec<TypeId>,
        receiver: Option<ResolvedReceiver>,
        receiver_type: Option<TypeId>,
    },
    LibraryOverload {
        item: crate::stdlib::StdlibItemId,
        dispatch_type: TypeId,
        cases: Vec<(crate::stdlib::StdlibCapabilityId, FunctionInstance)>,
        receiver: Option<ResolvedReceiver>,
        receiver_type: Option<TypeId>,
    },
    /// A structural capability method whose concrete implementation depends
    /// on the surrounding generic function instance.
    CapabilityRequirement {
        item: crate::stdlib::StdlibItemId,
        /// Explicit callable type arguments, distinct from any type arguments
        /// carried by the eventual concrete receiver implementation.
        type_arguments: Vec<TypeId>,
        signature: Vec<TypeId>,
        receiver: ResolvedReceiver,
        receiver_type: TypeId,
    },
    /// A compiler-provided `Display.toString` or `Debug.debugString` fallback,
    /// selected after generic capability dispatch reaches a concrete value.
    DefaultFormatting {
        mode: FormattingMode,
        receiver: ResolvedReceiver,
        receiver_type: TypeId,
    },
    GeneratorNext {
        receiver: ResolvedReceiver,
        receiver_type: TypeId,
    },
    IteratorIdentity {
        receiver: ResolvedReceiver,
        receiver_type: TypeId,
    },
    ManagedSnapshot {
        class: ManagedClassId,
        result: ResultTypeId,
        receiver: ResolvedReceiver,
        receiver_type: TypeId,
    },
    ManagedComponent {
        class: ManagedClassId,
        result: ResultTypeId,
        receiver: ResolvedReceiver,
        receiver_type: TypeId,
        helper: FunctionInstance,
        helper_result: TypeId,
    },
    ManagedInstances {
        class: ManagedClassId,
    },
    ResultError {
        result: ResultTypeId,
    },
    OptionSome {
        option: OptionTypeId,
    },
    IteratorItem {
        step: crate::ast::TypeApplicationId,
    },
    ResultSuccess {
        result: ResultTypeId,
    },
}

impl CallTarget {
    pub(crate) fn receiver_with_type(&self) -> Option<(&ResolvedReceiver, TypeId)> {
        match self {
            Self::UserMethod {
                receiver,
                receiver_type,
                ..
            }
            | Self::CapabilityRequirement {
                receiver,
                receiver_type,
                ..
            }
            | Self::DefaultFormatting {
                receiver,
                receiver_type,
                ..
            }
            | Self::GeneratorNext {
                receiver,
                receiver_type,
            }
            | Self::IteratorIdentity {
                receiver,
                receiver_type,
            }
            | Self::ManagedSnapshot {
                receiver,
                receiver_type,
                ..
            }
            | Self::ManagedComponent {
                receiver,
                receiver_type,
                ..
            } => Some((receiver, *receiver_type)),
            Self::Intrinsic {
                receiver: Some(receiver),
                receiver_type: Some(receiver_type),
                ..
            }
            | Self::LibraryOverload {
                receiver: Some(receiver),
                receiver_type: Some(receiver_type),
                ..
            } => Some((receiver, *receiver_type)),
            Self::Intrinsic { .. }
            | Self::LibraryOverload { .. }
            | Self::UserFunction { .. }
            | Self::ManagedInstances { .. }
            | Self::ResultError { .. }
            | Self::OptionSome { .. }
            | Self::IteratorItem { .. }
            | Self::ResultSuccess { .. } => None,
        }
    }

    fn receiver_with_type_mut(&mut self) -> Option<(&mut ResolvedReceiver, TypeId)> {
        match self {
            Self::UserMethod {
                receiver,
                receiver_type,
                ..
            }
            | Self::CapabilityRequirement {
                receiver,
                receiver_type,
                ..
            }
            | Self::DefaultFormatting {
                receiver,
                receiver_type,
                ..
            }
            | Self::GeneratorNext {
                receiver,
                receiver_type,
            }
            | Self::IteratorIdentity {
                receiver,
                receiver_type,
            }
            | Self::ManagedSnapshot {
                receiver,
                receiver_type,
                ..
            }
            | Self::ManagedComponent {
                receiver,
                receiver_type,
                ..
            } => Some((receiver, *receiver_type)),
            Self::Intrinsic {
                receiver: Some(receiver),
                receiver_type: Some(receiver_type),
                ..
            }
            | Self::LibraryOverload {
                receiver: Some(receiver),
                receiver_type: Some(receiver_type),
                ..
            } => Some((receiver, *receiver_type)),
            Self::Intrinsic { .. }
            | Self::LibraryOverload { .. }
            | Self::UserFunction { .. }
            | Self::ManagedInstances { .. }
            | Self::ResultError { .. }
            | Self::OptionSome { .. }
            | Self::IteratorItem { .. }
            | Self::ResultSuccess { .. } => None,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum FormattingMode {
    Display,
    Debug,
}

#[derive(Debug, Clone)]
pub enum InterpolatedPart {
    Text(String),
    Expression {
        expression: ExprId,
        string_conversion_source: Option<TypeId>,
    },
}

#[derive(Debug, Clone)]
pub struct MatchArm {
    pub pattern_id: PatternId,
    pub pattern: LoweredPattern,
    pub guard: Option<ExprId>,
    pub value: ExprId,
}

#[derive(Debug, Clone)]
pub enum LoweredPattern {
    Struct {
        structure: crate::semantic::ResolvedStructId,
        fields: Vec<(crate::semantic::ResolvedStructFieldId, LoweredPattern)>,
    },
    Enum {
        enumeration: EnumTypeId,
        variant: ResolvedEnumVariantId,
        payload: Option<Box<LoweredPattern>>,
    },
    Bool(bool),
    Char(char),
    String(String),
    Int {
        value: u64,
        negative: bool,
    },
    IntRange {
        start: u64,
        start_negative: bool,
        end: u64,
        end_negative: bool,
        kind: crate::ast::RangeKind,
    },
    FileVersion([u16; 4]),
    OptionNone(OptionTypeId),
    OptionSome {
        option: OptionTypeId,
        payload: Box<LoweredPattern>,
    },
    IteratorEnd(crate::ast::TypeApplicationId),
    IteratorItem {
        step: crate::ast::TypeApplicationId,
        payload: Box<LoweredPattern>,
    },
    ResultSuccess {
        result: ResultTypeId,
        payload: Box<LoweredPattern>,
    },
    ResultError {
        result: ResultTypeId,
        payload: Box<LoweredPattern>,
    },
    Array {
        prefix: Vec<LoweredPattern>,
        rest: bool,
        suffix: Vec<LoweredPattern>,
    },
    Alternation(Vec<LoweredPattern>),
    Binding(ValueId),
    Wildcard,
}

impl LoweredPattern {
    pub fn visit_bindings(&self, visitor: &mut impl FnMut(ValueId)) {
        match self {
            Self::Binding(binding) => visitor(*binding),
            Self::Struct { fields, .. } => {
                for (_, pattern) in fields {
                    pattern.visit_bindings(visitor);
                }
            }
            Self::Enum {
                payload: Some(payload),
                ..
            }
            | Self::OptionSome { payload, .. }
            | Self::IteratorItem { payload, .. }
            | Self::ResultSuccess { payload, .. }
            | Self::ResultError { payload, .. } => payload.visit_bindings(visitor),
            Self::Array { prefix, suffix, .. } => {
                for element in prefix.iter().chain(suffix) {
                    element.visit_bindings(visitor);
                }
            }
            Self::Alternation(elements) => {
                for element in elements {
                    element.visit_bindings(visitor);
                }
            }
            _ => {}
        }
    }

    pub fn contains_string(&self) -> bool {
        matches!(self, Self::String(_))
            || matches!(self, Self::Struct { fields, .. } if fields.iter().any(|(_, pattern)| pattern.contains_string()))
            || matches!(self, Self::Enum { payload: Some(payload), .. } if payload.contains_string())
            || matches!(self, Self::OptionSome { payload, .. } | Self::IteratorItem { payload, .. } | Self::ResultSuccess { payload, .. } | Self::ResultError { payload, .. } if payload.contains_string())
            || matches!(self, Self::Array { prefix, suffix, .. } if prefix.iter().chain(suffix).any(Self::contains_string))
            || matches!(self, Self::Alternation(elements) if elements.iter().any(Self::contains_string))
    }

    pub fn binds_result_error(&self) -> bool {
        matches!(self, Self::ResultError { payload, .. } if payload.demands_value())
            || matches!(self, Self::Struct { fields, .. } if fields.iter().any(|(_, pattern)| pattern.binds_result_error()))
            || matches!(self, Self::Enum { payload: Some(payload), .. } if payload.binds_result_error())
            || matches!(self, Self::OptionSome { payload, .. } | Self::IteratorItem { payload, .. } | Self::ResultSuccess { payload, .. } if payload.binds_result_error())
            || matches!(self, Self::Array { prefix, suffix, .. } if prefix.iter().chain(suffix).any(Self::binds_result_error))
            || matches!(self, Self::Alternation(elements) if elements.iter().any(Self::binds_result_error))
    }

    fn demands_value(&self) -> bool {
        match self {
            Self::Wildcard => false,
            Self::Alternation(alternatives) => alternatives.iter().any(Self::demands_value),
            _ => true,
        }
    }
}

#[derive(Debug, Clone)]
pub struct Body {
    pub owner: BodyOwner,
    /// Wasm-facing call contract. Direct bodies retain the ordinary function
    /// ABI; async functions are initialized into a typed continuation frame
    /// and polled separately.
    pub abi: BodyAbi,
    pub entry: Block,
    pub locals: Vec<Local>,
    /// Source locals retained by at least one poll or continuation state.
    /// Only these values need storage in the async continuation frame.
    pub frame_values: Vec<ValueId>,
    /// Compiler-generated expression values retained by the continuation
    /// frame. These remain distinct from source locals for tooling and DWARF.
    pub frame_temporaries: Vec<TemporaryId>,
    /// Structured lifetime that owns every cancellable suspension in this body.
    pub cancellation_region: Option<CancellationRegion>,
    /// Entry, poll, and continuation states in this async body.
    pub async_state_count: u32,
}

/// Stable status returned by every generated poll entry point.
///
/// Keeping this independent of the completed value avoids inventing a `T`
/// while an `async T` is pending. The ready value lives in the continuation
/// frame described by [`AsyncFunctionAbi`].
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[repr(i32)]
pub enum PollStatus {
    Pending = 0,
    Ready = 1,
}

impl PollStatus {
    pub const fn wasm_value(self) -> i32 {
        self as i32
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct AsyncFunctionAbi {
    /// The semantic completion type. Non-unit values are retained in the
    /// function's typed frame; `None` is represented physically by `Ready`
    /// alone. For `T!`, the frame stores the whole Result value; failure is
    /// ordinary readiness, not a third poll status.
    pub completion: TypeId,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct GeneratorAbi {
    pub item: TypeId,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum BodyAbi {
    /// Ordinary parameters and result, unchanged from the synchronous Wasm
    /// calling convention.
    Direct,
    /// Host-facing lifecycle action poll contract. The action stores its
    /// source result separately and returns whether this invocation is ready.
    AsyncAction,
    /// A source function is initialized with its ordinary parameters, then
    /// polled through a typed frame. Dropping that frame at the body's
    /// cancellation boundary cancels the computation without producing a
    /// Ready value.
    AsyncFunction(AsyncFunctionAbi),
    /// A synchronous source generator. Calling it allocates the continuation
    /// frame without executing the body; advancing the returned iterator
    /// resumes the frame until its next `yield` or permanent completion.
    Generator(GeneratorAbi),
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum CancellationRegion {
    AttachmentLifetime,
}

/// Where a completed suspension deposits its value.
///
/// This is deliberately independent of the syntax that introduced the
/// suspension. Async normalization decides the destination from the
/// continuation graph; code generation only has to honor that decision.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SuspensionDestination {
    Discard,
    SourceValue(ValueId),
    Temporary(TemporaryId),
    BodyResult,
}

impl SuspensionDestination {
    pub const fn source_value(self) -> Option<ValueId> {
        match self {
            Self::SourceValue(value) => Some(value),
            Self::Discard | Self::Temporary(_) | Self::BodyResult => None,
        }
    }
}

#[derive(Debug, Clone, Default)]
pub struct Block {
    pub statements: Vec<Statement>,
    pub terminator: Terminator,
}

#[derive(Debug, Clone)]
pub enum Statement {
    /// Source-level statement boundary retained solely for debugger line
    /// tables. It emits no WebAssembly instruction by itself.
    DebugLocation(Span),
    Store {
        target: ValueId,
        /// Whether this store introduces the body's local rather than assigning
        /// an existing local/global. Local planning consumes this semantic
        /// distinction instead of re-reading typed HIR.
        declaration: bool,
        operation: Option<AssignmentOperation>,
        value: ExprId,
    },
    BindPattern {
        value: ValueId,
        pattern: LoweredPattern,
    },
    StateStore {
        target: ValueId,
        operation: Option<AssignmentOperation>,
        value: ExprId,
    },
    StoreTemporary {
        target: TemporaryId,
        value: ExprId,
    },
    IndexStore {
        target: ExprId,
        operation: AssignmentOperation,
        value: ExprId,
    },
    Evaluate {
        expression: ExprId,
        discard_result: bool,
    },
    If {
        condition: ExprId,
        then_block: Block,
        else_block: Block,
    },
    Match {
        expression: ExprId,
        value: ExprId,
        arms: Vec<MatchStatementArm>,
    },
    Fallback {
        expression: ExprId,
        value: ExprId,
        fallback_block: Block,
        success_block: Block,
    },
    While {
        condition: ExprId,
        body: Block,
        result: Option<TemporaryId>,
    },
    For {
        binding: ValueId,
        pattern: LoweredPattern,
        iterable_value: ValueId,
        index_value: ValueId,
        version_value: ValueId,
        iterable: ExprId,
        /// Generated ordinary `Iterator.next` call for cursor-consuming loops.
        iterator_step: Option<ExprId>,
        body: Block,
    },
    ForInit {
        binding: ValueId,
        pattern: LoweredPattern,
        iterable_value: ValueId,
        index_value: ValueId,
        version_value: ValueId,
        iterable: ExprId,
        iterator_step: Option<ExprId>,
    },
}

#[derive(Debug, Clone)]
pub struct MatchStatementArm {
    pub pattern_id: PatternId,
    pub pattern: LoweredPattern,
    pub guard: Option<ExprId>,
    pub block: Block,
}

#[derive(Debug, Clone)]
pub enum AssignmentOperation {
    Primitive(BinaryOp),
    Call(CallTarget),
}

#[derive(Debug, Clone, Default)]
pub enum Terminator {
    #[default]
    Fallthrough,
    Break(Option<ExprId>),
    Continue,
    AsyncWhile {
        header: Box<Block>,
        continuation: Box<Block>,
        header_state: AsyncStateId,
        exit_state: AsyncStateId,
        result: Option<TemporaryId>,
    },
    AsyncWhileCondition {
        condition: ExprId,
        body: Box<Block>,
        header_state: AsyncStateId,
        exit_state: AsyncStateId,
    },
    AsyncFor {
        binding: ValueId,
        pattern: LoweredPattern,
        iterable_value: ValueId,
        index_value: ValueId,
        version_value: ValueId,
        iterator_step: Option<ExprId>,
        body: Box<Block>,
        continuation: Box<Block>,
        header_state: AsyncStateId,
        exit_state: AsyncStateId,
    },
    Return(Option<ExprId>),
    Throw {
        error: ExprId,
        target: FailureTarget,
    },
    /// Enters a synchronous fallible attempt. The attempt is evaluated in the
    /// poll state on every tick until `RetryComplete` observes success.
    Retry {
        attempt: Box<Block>,
        continuation: Box<Block>,
        source: Option<Span>,
        poll_state: AsyncStateId,
        resume_state: AsyncStateId,
        cancellation: Option<CancellationRegion>,
        live_values: Vec<ValueId>,
    },
    /// Completes the enclosing retry attempt with its `T!` value.
    RetryComplete {
        value: ExprId,
        destination: SuspensionDestination,
        resume_state: AsyncStateId,
    },
    Suspend {
        mode: SuspensionMode,
        destination: SuspensionDestination,
        value: ExprId,
        /// User-source origin of the suspension. Generated library/runtime
        /// suspensions deliberately remain locationless.
        source: Option<Span>,
        /// State retried while the awaited operation remains pending.
        poll_state: AsyncStateId,
        /// State entered after the awaited operation succeeds.
        resume_state: AsyncStateId,
        /// Region whose cancellation discards this suspended computation.
        cancellation: Option<CancellationRegion>,
        /// Source locals needed to retry this poll or execute its continuation.
        /// Kept in declaration order so lowering output remains deterministic.
        live_values: Vec<ValueId>,
        continuation: Box<Block>,
    },
    Yield {
        value: ExprId,
        resume_state: AsyncStateId,
        live_values: Vec<ValueId>,
        continuation: Box<Block>,
    },
}

#[derive(Debug, Clone)]
pub struct StateExpression {
    pub field: ValueId,
    pub entry: Block,
    pub locals: Vec<Local>,
}

/// One closed source initializer lowered into module-start control flow.
///
/// Its terminal store is part of the plan, so value blocks, loops, fallback,
/// and future expression forms share ordinary statement lowering instead of
/// requiring a second expression evaluator in the backend.
#[derive(Debug, Clone)]
pub struct GlobalInitializerPlan {
    pub value: ValueId,
    pub expression: ExprId,
    pub destructures: bool,
    pub entry: Block,
    pub locals: Vec<Local>,
}

#[derive(Debug, Clone)]
pub struct StateTransform {
    pub field: ValueId,
    pub value: ValueId,
    pub entry: Block,
    pub locals: Vec<Local>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ClosureCapture {
    pub value: ValueId,
    pub mutable: bool,
}

/// A source closure lowered into an independently callable body.
///
/// Capture discovery belongs to this IR boundary: later backend phases consume
/// the same ordered capture list when planning environments, allocating cells,
/// and emitting the closure function.
#[derive(Debug, Clone)]
pub struct ClosureBody {
    pub expression: ExprId,
    pub parameters: Vec<ValueId>,
    pub captures: Vec<ClosureCapture>,
    pub entry: Block,
    pub locals: Vec<Local>,
    /// Values and temporaries that survive a suspension in this closure.
    pub frame_values: Vec<ValueId>,
    pub frame_temporaries: Vec<TemporaryId>,
    /// Calling convention selected from the closure's result type. Async and
    /// generator closures allocate lazy continuation frames; direct closures
    /// execute immediately.
    pub abi: BodyAbi,
    pub async_state_count: u32,
}

#[derive(Debug, Clone, Default)]
pub struct Program {
    standard_library: StandardLibrary,
    profile: crate::BuildProfile,
    bodies: Vec<Body>,
    body_indices: HashMap<BodyOwner, usize>,
    global_initializers: Vec<(hir::TypedBindingPattern, ExprId)>,
    global_initializer_plans: Vec<GlobalInitializerPlan>,
    attachment_globals: Vec<ValueId>,
    attempt_globals: Vec<ValueId>,
    state_expressions: Vec<StateExpression>,
    state_transforms: Vec<StateTransform>,
    closures: Vec<ClosureBody>,
    closure_captures: HashMap<ExprId, Vec<ClosureCapture>>,
    mutably_captured_values: HashSet<ValueId>,
    expressions: Vec<Expression>,
    /// Source-defined constants are values in the language and hidden
    /// zero-argument functions only in the backend. Keep that lowering map in
    /// one place so value paths, including method receivers, share it.
    constant_functions: HashMap<crate::stdlib::StdlibItemId, FunctionInstance>,
    temporary_types: std::collections::HashMap<TemporaryId, TypeId>,
    next_generated_expression: u32,
    next_temporary: u32,
}

impl Program {
    pub(crate) fn lower(
        typed_hir: &TypedProgram,
        semantics: &SemanticModel,
        effects: &OperationAnalysis,
        capabilities: &crate::capabilities::CapabilityAnalysis,
        scoped_globals: &crate::scoped_globals::ScopedGlobalAnalysis,
        profile: crate::BuildProfile,
    ) -> Self {
        let expressions = typed_hir
            .all_expressions()
            .map(|expression| lower_expression(expression, typed_hir, semantics, profile))
            .collect::<Vec<_>>();
        let constant_functions = typed_hir
            .standard_library()
            .all_items()
            .iter()
            .filter(|item| item.kind == crate::stdlib::ItemKind::Constant)
            .map(|item| {
                let function = typed_hir
                    .library_function(item.id)
                    .expect("source-defined constants have injected function bodies");
                let result = semantics
                    .function_result(function)
                    .expect("checked constant bodies have result types");
                (item.id, semantics.function_instance(function, vec![result]))
            })
            .collect();
        let next_generated_expression = expressions
            .iter()
            .map(|expression| expression.id.index() as u32)
            .max()
            .map_or(0, |index| index + 1);
        let global_initializers = typed_hir
            .global_initializers()
            .filter(|initializer| !initializer.debug_only || profile == crate::BuildProfile::Debug)
            .map(|initializer| (initializer.binding.clone(), initializer.expression))
            .collect();
        let bare_globals = typed_hir
            .bare_globals_with_debug()
            .filter(|(_, debug_only)| !*debug_only || profile == crate::BuildProfile::Debug)
            .map(|(value, _)| value)
            .collect::<Vec<_>>();
        let attachment_globals = bare_globals
            .iter()
            .copied()
            .filter(|value| scoped_globals.is_attachment_global(*value))
            .collect();
        let attempt_globals = bare_globals
            .iter()
            .copied()
            .filter(|value| scoped_globals.is_attempt_global(*value))
            .collect();
        let mut program = Self {
            standard_library: typed_hir.standard_library().clone(),
            profile,
            bodies: Vec::new(),
            body_indices: HashMap::new(),
            global_initializers,
            global_initializer_plans: Vec::new(),
            attachment_globals,
            attempt_globals,
            state_expressions: Vec::new(),
            state_transforms: Vec::new(),
            closures: Vec::new(),
            closure_captures: HashMap::new(),
            mutably_captured_values: HashSet::new(),
            expressions,
            constant_functions,
            temporary_types: std::collections::HashMap::new(),
            next_generated_expression,
            next_temporary: 0,
        };
        let global_initializers = program.global_initializers.clone();
        for (binding, expression) in global_initializers {
            let normalized =
                normalize_expression_suspensions(expression, typed_hir, semantics, &mut program);
            let destructures = binding_destructures(&binding);
            let mut statements = vec![Statement::Store {
                target: binding.value,
                // A simple global stores directly into its Wasm global. A
                // destructuring declaration alone needs a temporary local for
                // the single incoming aggregate before projecting its leaves.
                declaration: destructures,
                operation: None,
                value: normalized.value,
            }];
            if destructures {
                statements.push(Statement::BindPattern {
                    value: binding.value,
                    pattern: lower_pattern(&binding.pattern.pattern, &binding.pattern.resolution),
                });
            }
            let continuation = Block {
                statements,
                terminator: Terminator::Fallthrough,
            };
            let entry = wrap_async_expression_steps(
                normalized.steps,
                continuation,
                typed_hir,
                semantics,
                SourceProvenance {
                    profile,
                    visible: expression.index() < typed_hir.visible_expression_count(),
                },
                &mut program,
            );
            let locals = plan_block(&entry, &program, semantics, capabilities);
            program
                .global_initializer_plans
                .push(GlobalInitializerPlan {
                    value: binding.value,
                    expression,
                    destructures,
                    entry,
                    locals,
                });
        }
        let mutated_values = mutated_values(typed_hir);
        let globals = program
            .global_initializers
            .iter()
            .flat_map(|(binding, _)| pattern_bindings(&binding.pattern.pattern))
            .chain(program.attachment_globals.iter().copied())
            .chain(program.attempt_globals.iter().copied())
            .collect::<HashSet<_>>();
        let closures = typed_hir
            .all_expressions()
            .filter_map(|expression| match &expression.kind {
                TypedExpressionKind::Closure { parameters, body } => {
                    Some((expression.id, parameters.clone(), *body))
                }
                _ => None,
            })
            .collect::<Vec<_>>();
        for (expression, parameters, body) in &closures {
            let declared_parameters = parameters
                .iter()
                .flat_map(|parameter| {
                    std::iter::once(parameter.value)
                        .chain(pattern_bindings(&parameter.pattern.pattern))
                })
                .collect::<Vec<_>>();
            let captures = closure_captures(
                *body,
                &declared_parameters,
                typed_hir,
                &globals,
                &mutated_values,
            );
            program.mutably_captured_values.extend(
                captures
                    .iter()
                    .filter(|capture| capture.mutable)
                    .map(|capture| capture.value),
            );
            program.closure_captures.insert(*expression, captures);
        }
        for function in typed_hir.all_function_bodies() {
            if function.debug_only && profile == crate::BuildProfile::Release {
                continue;
            }
            let body = lower_body(
                BodyOwner::Function(function.function.clone()),
                &function.body,
                &function.parameters,
                typed_hir,
                semantics,
                effects,
                capabilities,
                SourceProvenance {
                    profile,
                    visible: function.function.function.index()
                        < typed_hir.visible_function_count(),
                },
                &mutated_values,
                &mut program,
            );
            program.push_body(body);
        }
        for action in typed_hir.action_bodies() {
            let body = lower_body(
                BodyOwner::Action(action.action),
                &action.body,
                &[],
                typed_hir,
                semantics,
                effects,
                capabilities,
                SourceProvenance {
                    profile,
                    visible: true,
                },
                &mutated_values,
                &mut program,
            );
            program.push_body(body);
        }
        let state_sources = typed_hir.state_sources().collect::<Vec<_>>();
        for (field, expression) in state_sources {
            let entry = lower_expression_body(
                expression,
                typed_hir,
                semantics,
                SourceProvenance {
                    profile,
                    visible: true,
                },
                &mut program,
            );
            let locals = plan_block(&entry, &program, semantics, capabilities);
            program.state_expressions.push(StateExpression {
                field,
                entry,
                locals,
            });
        }
        let state_transforms = typed_hir.state_transforms().collect::<Vec<_>>();
        for transform in state_transforms {
            let entry = lower_expression_body(
                transform.expression,
                typed_hir,
                semantics,
                SourceProvenance {
                    profile,
                    visible: true,
                },
                &mut program,
            );
            let locals = plan_block(&entry, &program, semantics, capabilities);
            program.state_transforms.push(StateTransform {
                field: transform.field,
                value: transform.value,
                entry,
                locals,
            });
        }
        for (expression, parameters, body) in closures {
            let captures = program.closure_captures[&expression].clone();
            let is_generator = hir::typed_expression_contains_yield(body, typed_hir);
            let mut entry = lower_expression_body(
                body,
                typed_hir,
                semantics,
                SourceProvenance {
                    profile,
                    visible: expression.index() < typed_hir.visible_expression_count(),
                },
                &mut program,
            );
            for parameter in parameters.iter().rev() {
                if !binding_destructures(parameter) {
                    continue;
                }
                entry.statements.insert(
                    0,
                    Statement::BindPattern {
                        value: parameter.value,
                        pattern: lower_pattern(
                            &parameter.pattern.pattern,
                            &parameter.pattern.resolution,
                        ),
                    },
                );
            }
            let mut next_async_state = 1;
            assign_async_states(&mut entry, &mut next_async_state);
            let locals = plan_block(&entry, &program, semantics, capabilities);
            let frame_values = plan_frame_values(&mut entry, &locals, &program);
            let frame_temporaries = locals
                .iter()
                .filter_map(|local| match local.purpose {
                    LocalPurpose::Temporary(temporary) => Some(temporary),
                    _ => None,
                })
                .collect();
            let callable = program
                .expression(expression)
                .expect("closure expressions belong to Wasm IR")
                .ty;
            let TypeKind::Callable { result, .. } = semantics.types().kind(callable) else {
                unreachable!("checked closure expressions have callable types")
            };
            let abi = match semantics.types().kind(*result) {
                TypeKind::Async { value, .. } => {
                    BodyAbi::AsyncFunction(AsyncFunctionAbi { completion: *value })
                }
                TypeKind::Iterator { item, .. } if is_generator => {
                    BodyAbi::Generator(GeneratorAbi { item: *item })
                }
                _ => BodyAbi::Direct,
            };
            program.closures.push(ClosureBody {
                expression,
                parameters: parameters.iter().map(|parameter| parameter.value).collect(),
                captures,
                entry,
                locals,
                frame_values,
                frame_temporaries,
                abi,
                async_state_count: next_async_state,
            });
        }
        program
    }

    pub fn profile(&self) -> crate::BuildProfile {
        self.profile
    }

    pub fn standard_library(&self) -> &StandardLibrary {
        &self.standard_library
    }

    pub fn constant_function(
        &self,
        item: crate::stdlib::StdlibItemId,
    ) -> Option<&FunctionInstance> {
        self.constant_functions.get(&item)
    }

    /// Whether this source value must use shared GC-cell storage because a
    /// closure can both retain and mutate it after its declaring scope moves
    /// on. All backend storage paths consult this one decision so locals,
    /// continuation frames, and nested closure environments keep aliasing.
    pub fn is_mutably_captured(&self, value: ValueId) -> bool {
        self.mutably_captured_values.contains(&value)
    }

    pub fn mutably_captured_values(&self) -> impl Iterator<Item = ValueId> + '_ {
        self.mutably_captured_values.iter().copied()
    }

    pub fn global_initializers(&self) -> impl Iterator<Item = (ValueId, ExprId)> + '_ {
        self.global_initializers
            .iter()
            .map(|(binding, expression)| (binding.value, *expression))
    }

    pub fn global_initializer_plans(&self) -> impl Iterator<Item = &GlobalInitializerPlan> {
        self.global_initializer_plans.iter()
    }

    pub fn contains_global(&self, value: ValueId) -> bool {
        self.global_initializers
            .iter()
            .any(|(binding, _)| pattern_bindings(&binding.pattern.pattern).contains(&value))
            || self.attachment_globals.contains(&value)
            || self.attempt_globals.contains(&value)
    }

    pub fn attachment_globals(&self) -> impl Iterator<Item = ValueId> + '_ {
        self.attachment_globals.iter().copied()
    }

    pub fn is_attachment_global(&self, value: ValueId) -> bool {
        self.attachment_globals.contains(&value)
    }

    pub fn attempt_globals(&self) -> impl Iterator<Item = ValueId> + '_ {
        self.attempt_globals.iter().copied()
    }

    pub fn is_attempt_global(&self, value: ValueId) -> bool {
        self.attempt_globals.contains(&value)
    }

    pub fn is_scoped_global(&self, value: ValueId) -> bool {
        self.is_attachment_global(value) || self.is_attempt_global(value)
    }

    pub fn bodies(&self) -> impl Iterator<Item = &Body> {
        self.bodies.iter()
    }

    pub fn body(&self, owner: BodyOwner) -> Option<&Body> {
        self.body_indices
            .get(&owner)
            .or_else(|| match owner {
                BodyOwner::Function(instance)
                    if !instance.type_arguments.is_empty() || !instance.signature.is_empty() =>
                {
                    self.body_indices
                        .get(&BodyOwner::Function(FunctionInstance::monomorphic(
                            instance.function,
                        )))
                }
                BodyOwner::Function(_) | BodyOwner::Action(_) => None,
            })
            .map(|index| &self.bodies[*index])
    }

    fn push_body(&mut self, body: Body) {
        let previous = self
            .body_indices
            .insert(body.owner.clone(), self.bodies.len());
        assert!(previous.is_none(), "Wasm body owners must be unique");
        self.bodies.push(body);
    }

    pub fn state_expressions(&self) -> impl Iterator<Item = &StateExpression> {
        self.state_expressions.iter()
    }

    pub fn state_expression(&self, field: ValueId) -> Option<&StateExpression> {
        self.state_expressions
            .iter()
            .find(|expression| expression.field == field)
    }

    pub fn state_transforms(&self) -> impl Iterator<Item = &StateTransform> {
        self.state_transforms.iter()
    }

    pub fn state_transform(&self, field: ValueId) -> Option<&StateTransform> {
        self.state_transforms
            .iter()
            .find(|transform| transform.field == field)
    }

    pub fn closures(&self) -> impl Iterator<Item = &ClosureBody> {
        self.closures.iter()
    }

    pub fn closure(&self, expression: ExprId) -> Option<&ClosureBody> {
        self.closures
            .iter()
            .find(|closure| closure.expression == expression)
    }

    pub fn closure_captures(&self, expression: ExprId) -> &[ClosureCapture] {
        self.closure_captures
            .get(&expression)
            .map(Vec::as_slice)
            .unwrap_or(&[])
    }

    pub fn expression(&self, id: ExprId) -> Option<&Expression> {
        self.expressions
            .binary_search_by_key(&id.index(), |expression| expression.id.index())
            .ok()
            .map(|index| &self.expressions[index])
    }

    pub fn expressions(&self) -> impl ExactSizeIterator<Item = &Expression> {
        self.expressions.iter()
    }

    fn push_generated_expression(
        &mut self,
        ty: TypeId,
        kind: ExpressionKind,
        conversion: Option<ValueConversion>,
        source: Option<Span>,
    ) -> ExprId {
        let id = ExprId::from_index(self.next_generated_expression);
        self.next_generated_expression += 1;
        self.expressions.push(Expression {
            id,
            ty,
            kind,
            source,
            conversion,
        });
        id
    }

    fn temporary(&mut self, ty: TypeId) -> (TemporaryId, ExprId) {
        self.temporary_read(ty, ty, None)
    }

    fn temporary_read(
        &mut self,
        storage_ty: TypeId,
        expression_ty: TypeId,
        conversion: Option<ValueConversion>,
    ) -> (TemporaryId, ExprId) {
        let temporary = TemporaryId(self.next_temporary);
        self.next_temporary += 1;
        let previous = self.temporary_types.insert(temporary, storage_ty);
        debug_assert!(previous.is_none(), "temporary identities are unique");
        let expression = self.push_generated_expression(
            expression_ty,
            ExpressionKind::Temporary(temporary),
            conversion,
            None,
        );
        // The read expression may apply an edge conversion, while the local
        // itself stores the value produced before that conversion. Local
        // planning obtains this type from stores/suspension destinations.
        debug_assert!(storage_ty == expression_ty || conversion.is_some());
        (temporary, expression)
    }

    pub(crate) fn effective_expression_type(&self, expression: ExprId) -> TypeId {
        let expression = self
            .expression(expression)
            .expect("lowered expression belongs to Wasm IR");
        expression
            .conversion
            .map_or(expression.ty, |conversion| conversion.target)
    }

    fn temporary_type(&self, temporary: TemporaryId) -> TypeId {
        self.temporary_types[&temporary]
    }
}

fn lower_pattern(pattern: &TypedPattern, resolution: &hir::ResolvedPattern) -> LoweredPattern {
    match pattern {
        TypedPattern::Struct { structure, fields } => LoweredPattern::Struct {
            structure: *structure,
            fields: fields
                .iter()
                .map(|field| {
                    (
                        field.field,
                        lower_pattern(&field.pattern.pattern, &field.pattern.resolution),
                    )
                })
                .collect(),
        },
        TypedPattern::Enum {
            enumeration,
            payload,
            ..
        } => LoweredPattern::Enum {
            enumeration: *enumeration,
            variant: resolution
                .variant
                .expect("checked enum patterns have resolved variants"),
            payload: payload
                .as_ref()
                .map(|payload| Box::new(lower_pattern(&payload.pattern, &payload.resolution))),
        },
        TypedPattern::Bool(value) => LoweredPattern::Bool(*value),
        TypedPattern::Char(value) => LoweredPattern::Char(*value),
        TypedPattern::String(value) => LoweredPattern::String(value.clone()),
        TypedPattern::Int {
            value, negative, ..
        } => LoweredPattern::Int {
            value: *value,
            negative: *negative,
        },
        TypedPattern::IntRange {
            start,
            start_negative,
            end,
            end_negative,
            kind,
            ..
        } => LoweredPattern::IntRange {
            start: *start,
            start_negative: *start_negative,
            end: *end,
            end_negative: *end_negative,
            kind: *kind,
        },
        TypedPattern::FileVersion(components) => LoweredPattern::FileVersion(*components),
        TypedPattern::None => {
            let Some(ResolvedWrapperPattern::OptionNone(option)) = resolution.wrapper else {
                unreachable!("checked None patterns resolve to Options")
            };
            LoweredPattern::OptionNone(option)
        }
        TypedPattern::OptionSome(payload) => {
            let Some(ResolvedWrapperPattern::OptionSome(option)) = resolution.wrapper else {
                unreachable!("checked Some patterns resolve to Options")
            };
            LoweredPattern::OptionSome {
                option,
                payload: Box::new(lower_pattern(&payload.pattern, &payload.resolution)),
            }
        }
        TypedPattern::IteratorEnd => {
            let Some(ResolvedWrapperPattern::IteratorEnd(step)) = resolution.wrapper else {
                unreachable!("checked End patterns resolve to IteratorStep")
            };
            LoweredPattern::IteratorEnd(step)
        }
        TypedPattern::IteratorItem(payload) => {
            let Some(ResolvedWrapperPattern::IteratorItem(step)) = resolution.wrapper else {
                unreachable!("checked Item patterns resolve to IteratorStep")
            };
            LoweredPattern::IteratorItem {
                step,
                payload: Box::new(lower_pattern(&payload.pattern, &payload.resolution)),
            }
        }
        TypedPattern::ResultSuccess(payload) => {
            let Some(ResolvedWrapperPattern::ResultSuccess(result)) = resolution.wrapper else {
                unreachable!("checked Ok patterns resolve to Results")
            };
            LoweredPattern::ResultSuccess {
                result,
                payload: Box::new(lower_pattern(&payload.pattern, &payload.resolution)),
            }
        }
        TypedPattern::ResultError(payload) => {
            let Some(ResolvedWrapperPattern::ResultError(result)) = resolution.wrapper else {
                unreachable!("checked Err patterns resolve to Results")
            };
            LoweredPattern::ResultError {
                result,
                payload: Box::new(lower_pattern(&payload.pattern, &payload.resolution)),
            }
        }
        TypedPattern::Array {
            prefix,
            rest,
            suffix,
        } => LoweredPattern::Array {
            prefix: prefix
                .iter()
                .map(|element| lower_pattern(&element.pattern, &element.resolution))
                .collect(),
            rest: *rest,
            suffix: suffix
                .iter()
                .map(|element| lower_pattern(&element.pattern, &element.resolution))
                .collect(),
        },
        TypedPattern::Alternation(alternatives) => LoweredPattern::Alternation(
            alternatives
                .iter()
                .map(|alternative| lower_pattern(&alternative.pattern, &alternative.resolution))
                .collect(),
        ),
        TypedPattern::Binding(binding) => LoweredPattern::Binding(binding.id),
        TypedPattern::Wildcard => LoweredPattern::Wildcard,
    }
}

fn lower_expression(
    expression: &TypedExpression,
    typed_hir: &TypedProgram,
    semantics: &SemanticModel,
    profile: crate::BuildProfile,
) -> Expression {
    let kind = match &expression.kind {
        TypedExpressionKind::None => ExpressionKind::None,
        TypedExpressionKind::IteratorEnd => ExpressionKind::IteratorEnd,
        TypedExpressionKind::Bool(value) => ExpressionKind::Bool(*value),
        TypedExpressionKind::Int {
            value, negative, ..
        } => ExpressionKind::Int {
            value: *value,
            negative: *negative,
        },
        TypedExpressionKind::Float(value) => ExpressionKind::Float(value.clone()),
        TypedExpressionKind::Char(value) => ExpressionKind::Char(*value),
        TypedExpressionKind::String(value) => ExpressionKind::String(value.clone()),
        TypedExpressionKind::InterpolatedString(parts) => ExpressionKind::InterpolatedString(
            parts
                .iter()
                .map(|part| match part {
                    TypedInterpolatedPart::Text(value) => InterpolatedPart::Text(value.clone()),
                    TypedInterpolatedPart::Expression {
                        expression,
                        conversion,
                    } => InterpolatedPart::Expression {
                        expression: *expression,
                        string_conversion_source: conversion.map(|conversion| match conversion {
                            ImplicitConversion::ToString { source } => source,
                        }),
                    },
                })
                .collect(),
        ),
        TypedExpressionKind::Signature(value) => ExpressionKind::Signature(value.clone()),
        TypedExpressionKind::Array(elements) => ExpressionKind::Array(elements.clone()),
        TypedExpressionKind::Range { start, end, kind } => ExpressionKind::Range {
            start: *start,
            end: *end,
            kind: *kind,
        },
        TypedExpressionKind::Block { .. } => ExpressionKind::ValueBlock,
        TypedExpressionKind::Loop { .. } => ExpressionKind::Loop,
        TypedExpressionKind::Struct { structure, fields } => {
            let Some(ExpressionResolution::StructLiteral {
                structure: resolved_struct,
                fields: resolved_fields,
            }) = &expression.resolution
            else {
                unreachable!("checked struct literals have resolved field IDs")
            };
            debug_assert_eq!(structure, resolved_struct);
            debug_assert_eq!(fields.len(), resolved_fields.len());
            ExpressionKind::Struct {
                structure: *resolved_struct,
                fields: resolved_fields
                    .iter()
                    .copied()
                    .zip(fields.iter().map(|(_, value)| *value))
                    .collect(),
            }
        }
        TypedExpressionKind::Enum {
            enumeration,
            payload,
            ..
        } => {
            let Some(ExpressionResolution::EnumConstructor { variant }) = &expression.resolution
            else {
                unreachable!("checked enum constructors have resolved variant IDs")
            };
            ExpressionKind::Enum {
                enumeration: *enumeration,
                variant: *variant,
                payload: *payload,
            }
        }
        TypedExpressionKind::Path(_) => {
            if let Some(ExpressionResolution::FunctionValue(function)) = &expression.resolution {
                ExpressionKind::FunctionValue {
                    function: function.clone(),
                }
            } else {
                let Some(ExpressionResolution::ValuePath { root, members }) =
                    &expression.resolution
                else {
                    unreachable!("checked value paths have a typed-HIR resolution")
                };
                ExpressionKind::Path {
                    root: *root,
                    members: members.clone(),
                }
            }
        }
        TypedExpressionKind::Member { receiver, .. } => {
            let Some(ExpressionResolution::Member { members }) = &expression.resolution else {
                unreachable!("checked member expressions have a typed-HIR resolution")
            };
            ExpressionKind::Member {
                receiver: *receiver,
                members: members.clone(),
            }
        }
        TypedExpressionKind::Index { receiver, index } => {
            if let Some(ExpressionResolution::Call(target)) = &expression.resolution {
                ExpressionKind::Call {
                    target: lower_call_target(target, typed_hir, semantics),
                    arguments: vec![*index],
                }
            } else {
                ExpressionKind::Index {
                    receiver: *receiver,
                    index: *index,
                }
            }
        }
        TypedExpressionKind::Unary {
            op,
            expression: operand,
        } => {
            if let Some(ExpressionResolution::Call(target)) = &expression.resolution {
                ExpressionKind::Call {
                    target: lower_call_target(target, typed_hir, semantics),
                    arguments: Vec::new(),
                }
            } else {
                ExpressionKind::Unary {
                    op: *op,
                    operand: *operand,
                }
            }
        }
        TypedExpressionKind::Cast {
            expression: value, ..
        } => ExpressionKind::Cast { value: *value },
        TypedExpressionKind::Binary { op, left, right } => {
            if let Some(ExpressionResolution::Call(target)) = &expression.resolution {
                ExpressionKind::Call {
                    target: lower_call_target(target, typed_hir, semantics),
                    arguments: vec![*right],
                }
            } else {
                ExpressionKind::Binary {
                    op: *op,
                    left: *left,
                    right: *right,
                }
            }
        }
        TypedExpressionKind::Call { arguments, .. } => match &expression.resolution {
            Some(ExpressionResolution::Call(target)) => ExpressionKind::Call {
                target: lower_call_target(target, typed_hir, semantics),
                arguments: arguments.clone(),
            },
            Some(ExpressionResolution::DynamicCall(callee)) => ExpressionKind::Invoke {
                callee: *callee,
                arguments: arguments.clone(),
            },
            _ => unreachable!("checked calls have a resolved target"),
        },
        TypedExpressionKind::Invoke { arguments, .. } => {
            let Some(ExpressionResolution::DynamicCall(callee)) = expression.resolution else {
                unreachable!("checked callable invocations have a dynamic target")
            };
            ExpressionKind::Invoke {
                callee,
                arguments: arguments.clone(),
            }
        }
        TypedExpressionKind::Closure { parameters, body } => ExpressionKind::Closure {
            closure: expression.id,
            parameters: parameters.iter().map(|parameter| parameter.value).collect(),
            body: *body,
        },
        TypedExpressionKind::If {
            condition,
            then_expr,
            else_expr,
        } => ExpressionKind::If {
            condition: *condition,
            then_expr: *then_expr,
            else_expr: *else_expr,
        },
        TypedExpressionKind::Fallback { value, fallback } => ExpressionKind::Fallback {
            value: *value,
            fallback: *fallback,
        },
        TypedExpressionKind::Break(value) => ExpressionKind::Break(*value),
        TypedExpressionKind::Continue => ExpressionKind::Continue,
        TypedExpressionKind::Return(value) => ExpressionKind::Return(*value),
        TypedExpressionKind::Throw { error, target } => ExpressionKind::Throw {
            error: *error,
            target: *target,
        },
        TypedExpressionKind::Inspect { label, value } => {
            if profile == crate::BuildProfile::Debug {
                ExpressionKind::Inspect {
                    label: label.clone(),
                    value: *value,
                }
            } else {
                ExpressionKind::Transparent { value: *value }
            }
        }
        TypedExpressionKind::Suspend {
            mode,
            destination,
            value,
        } => ExpressionKind::Suspend {
            mode: *mode,
            destination: *destination,
            value: *value,
        },
        TypedExpressionKind::Propagate { value, target } => ExpressionKind::Propagate {
            value: *value,
            target: *target,
        },
        TypedExpressionKind::Match { value, arms } => ExpressionKind::Match {
            value: *value,
            arms: arms
                .iter()
                .map(|arm| MatchArm {
                    pattern_id: arm.resolution.id,
                    pattern: lower_pattern(&arm.pattern, &arm.resolution),
                    guard: arm.guard,
                    value: arm.value,
                })
                .collect(),
        },
        TypedExpressionKind::Is { value, pattern } => ExpressionKind::Is {
            value: *value,
            pattern_id: pattern.resolution.id,
            pattern: lower_pattern(&pattern.pattern, &pattern.resolution),
        },
    };
    Expression {
        id: expression.id,
        ty: expression.ty,
        kind,
        source: (expression.id.index() < typed_hir.visible_expression_count())
            .then_some(expression.span),
        conversion: expression.conversion,
    }
}

fn lower_call_target(
    target: &ResolvedCall,
    typed_hir: &TypedProgram,
    semantics: &SemanticModel,
) -> CallTarget {
    match target {
        ResolvedCall::UserFunction {
            function,
            type_arguments,
            signature,
        } => CallTarget::UserFunction {
            function: FunctionInstance {
                function: *function,
                type_arguments: type_arguments.clone(),
                signature: signature.clone(),
            },
        },
        ResolvedCall::UserMethod {
            function,
            type_arguments,
            signature,
            receiver,
            receiver_type,
        } => CallTarget::UserMethod {
            function: FunctionInstance {
                function: *function,
                type_arguments: type_arguments.clone(),
                signature: signature.clone(),
            },
            receiver: receiver.clone(),
            receiver_type: *receiver_type,
        },
        ResolvedCall::StandardLibrary {
            item,
            type_arguments,
            signature,
            receiver,
            receiver_type,
        } => match typed_hir.standard_library().item(*item).implementation {
            Implementation::CapabilityRequirement => {
                let explicit = typed_hir
                    .standard_library()
                    .item(*item)
                    .signature
                    .explicit_type_parameters;
                CallTarget::CapabilityRequirement {
                    item: *item,
                    type_arguments: type_arguments[type_arguments.len() - explicit..].to_vec(),
                    signature: signature.clone(),
                    receiver: receiver
                        .clone()
                        .expect("capability requirements are receiver methods"),
                    receiver_type: receiver_type
                        .expect("capability requirements have receiver types"),
                }
            }
            Implementation::Intrinsic(intrinsic) => CallTarget::Intrinsic {
                item: *item,
                intrinsic,
                type_arguments: type_arguments.clone(),
                receiver: receiver.clone(),
                receiver_type: *receiver_type,
            },
            Implementation::LibraryBody { .. } => {
                let function = typed_hir
                    .library_function(*item)
                    .expect("catalog source bodies have injected functions");
                let function = semantics.function_instance(function, signature.clone());
                if receiver.is_some() {
                    CallTarget::UserMethod {
                        function,
                        receiver: receiver.clone().expect("method calls have receivers"),
                        receiver_type: receiver_type.expect("method calls have receiver types"),
                    }
                } else {
                    CallTarget::UserFunction { function }
                }
            }
            Implementation::LibraryOverloads {
                dispatch_parameter,
                cases,
            } => {
                let dispatch_type = type_arguments[dispatch_parameter];
                let cases = cases
                    .iter()
                    .enumerate()
                    .map(|(index, case)| {
                        let function = typed_hir
                            .library_overload_function(*item, index)
                            .expect("catalog overload cases have injected functions");
                        (
                            case.capability,
                            semantics.function_instance(function, signature.clone()),
                        )
                    })
                    .collect();
                CallTarget::LibraryOverload {
                    item: *item,
                    dispatch_type,
                    cases,
                    receiver: receiver.clone(),
                    receiver_type: *receiver_type,
                }
            }
        },
        ResolvedCall::ManagedSnapshot {
            class,
            result,
            receiver,
            receiver_type,
        } => CallTarget::ManagedSnapshot {
            class: *class,
            result: *result,
            receiver: receiver.clone(),
            receiver_type: *receiver_type,
        },
        ResolvedCall::ManagedComponent {
            class,
            result,
            receiver,
            receiver_type,
        } => {
            let function = typed_hir
                .library_function(crate::stdlib::StdlibItemId::UnityGameObjectComponentAddress)
                .expect("Unity component lookup has a source-defined traversal helper");
            let helper_result = semantics
                .function_result(function)
                .expect("checked Unity component helper has a result type");
            let signature = semantics
                .function_parameter_types(function)
                .iter()
                .copied()
                .chain([helper_result])
                .collect();
            CallTarget::ManagedComponent {
                class: *class,
                result: *result,
                receiver: receiver.clone(),
                receiver_type: *receiver_type,
                helper: semantics.function_instance(function, signature),
                helper_result,
            }
        }
        ResolvedCall::ManagedInstances { class } => CallTarget::ManagedInstances { class: *class },
        ResolvedCall::ResultError { result } => CallTarget::ResultError { result: *result },
        ResolvedCall::OptionSome { option } => CallTarget::OptionSome { option: *option },
        ResolvedCall::IteratorItem { step } => CallTarget::IteratorItem { step: *step },
        ResolvedCall::ResultSuccess { result } => CallTarget::ResultSuccess { result: *result },
    }
}

pub(crate) fn resolve_library_overload(
    target: &CallTarget,
    owner: Option<&FunctionInstance>,
    semantics: &SemanticModel,
    library: &crate::stdlib::StandardLibrary,
) -> Option<FunctionInstance> {
    let CallTarget::LibraryOverload {
        dispatch_type,
        cases,
        ..
    } = target
    else {
        return None;
    };
    let dispatch_type = owner.map_or(*dispatch_type, |owner| {
        semantics.specialize_type(owner, *dispatch_type)
    });
    let satisfies = |capability| match semantics.types().kind(dispatch_type) {
        crate::types::TypeKind::Builtin(core) => {
            library.core_type_has_capability(*core, capability)
        }
        crate::types::TypeKind::Standard(ty) => library.type_has_capability(*ty, capability),
        _ => false,
    };
    let (_, function) = cases
        .iter()
        .find(|(capability, _)| satisfies(*capability))
        .unwrap_or_else(|| {
            panic!(
                "checked library overload has no implementation for concrete type {:?}",
                semantics.types().kind(dispatch_type)
            )
        });
    Some(owner.map_or_else(
        || function.clone(),
        |owner| semantics.specialize_function_instance(owner, function),
    ))
}

pub(crate) fn resolve_capability_requirement(
    target: &CallTarget,
    owner: Option<&FunctionInstance>,
    program: &crate::ast::Program,
    semantics: &SemanticModel,
    library: &crate::stdlib::StandardLibrary,
    capabilities: &crate::capabilities::CapabilityAnalysis,
) -> Option<CallTarget> {
    let CallTarget::CapabilityRequirement {
        item,
        type_arguments,
        signature,
        receiver,
        receiver_type,
    } = target
    else {
        return None;
    };
    let specialize = |ty| owner.map_or(ty, |owner| semantics.specialize_type(owner, ty));
    let receiver_type = specialize(*receiver_type);
    let signature = signature
        .iter()
        .copied()
        .map(specialize)
        .collect::<Vec<_>>();
    let explicit_type_arguments = type_arguments
        .iter()
        .copied()
        .map(specialize)
        .collect::<Vec<_>>();
    let implementation =
        capabilities.resolve_method_requirement(receiver_type, *item, semantics)?;
    match implementation {
        crate::capabilities::CapabilityMethodImplementation::Source(function) => {
            Some(CallTarget::UserMethod {
                function: semantics.function_instance(function, signature),
                receiver: receiver.clone(),
                receiver_type,
            })
        }
        crate::capabilities::CapabilityMethodImplementation::Standard(item) => {
            let declaration = library.item(item);
            let mut type_arguments = match semantics.types().kind(receiver_type) {
                TypeKind::Array { element, .. } => vec![*element],
                TypeKind::Option { value, .. } | TypeKind::Result { value, .. } => vec![*value],
                TypeKind::Set { element, .. } => vec![*element],
                TypeKind::Range { bound, .. } => vec![*bound],
                TypeKind::Application { arguments, .. } => arguments.clone(),
                TypeKind::Builtin(_) | TypeKind::Standard(_) | TypeKind::SettingsView => Vec::new(),
                kind => unreachable!(
                    "capability dispatch selected a standard implementation for `{kind:?}`"
                ),
            };
            type_arguments.extend(explicit_type_arguments);
            match declaration.implementation {
                Implementation::Intrinsic(intrinsic) => Some(CallTarget::Intrinsic {
                    item,
                    intrinsic,
                    type_arguments,
                    receiver: Some(receiver.clone()),
                    receiver_type: Some(receiver_type),
                }),
                Implementation::LibraryBody { function_name, .. } => {
                    let function = program
                        .functions
                        .iter()
                        .find(|function| function.name == function_name)
                        .expect("standard-library capability bodies are injected");
                    Some(CallTarget::UserMethod {
                        function: semantics.function_instance(function.id, signature),
                        receiver: receiver.clone(),
                        receiver_type,
                    })
                }
                Implementation::LibraryOverloads {
                    dispatch_parameter,
                    cases,
                } => Some(CallTarget::LibraryOverload {
                    item,
                    dispatch_type: type_arguments[dispatch_parameter],
                    cases: cases
                        .iter()
                        .map(|case| {
                            let function_name = case.function_name;
                            let function = program
                                .functions
                                .iter()
                                .find(|function| function.name == function_name)
                                .expect("standard-library overload bodies are injected");
                            (
                                case.capability,
                                semantics.function_instance(function.id, signature.clone()),
                            )
                        })
                        .collect(),
                    receiver: Some(receiver.clone()),
                    receiver_type: Some(receiver_type),
                }),
                Implementation::CapabilityRequirement => {
                    unreachable!("capability dispatch must select an implementation")
                }
            }
        }
        crate::capabilities::CapabilityMethodImplementation::GeneratorNext => {
            Some(CallTarget::GeneratorNext {
                receiver: receiver.clone(),
                receiver_type,
            })
        }
        crate::capabilities::CapabilityMethodImplementation::IteratorIdentity => {
            Some(CallTarget::IteratorIdentity {
                receiver: receiver.clone(),
                receiver_type,
            })
        }
        crate::capabilities::CapabilityMethodImplementation::DefaultDisplay => {
            Some(CallTarget::DefaultFormatting {
                mode: FormattingMode::Display,
                receiver: receiver.clone(),
                receiver_type,
            })
        }
        crate::capabilities::CapabilityMethodImplementation::DefaultDebug => {
            Some(CallTarget::DefaultFormatting {
                mode: FormattingMode::Debug,
                receiver: receiver.clone(),
                receiver_type,
            })
        }
    }
}

fn lower_assignment_operation(
    assignment: &hir::ResolvedAssignment,
    op: Option<BinaryOp>,
    typed_hir: &TypedProgram,
    semantics: &SemanticModel,
) -> Option<AssignmentOperation> {
    assignment
        .operator
        .as_ref()
        .map(|call| AssignmentOperation::Call(lower_call_target(call, typed_hir, semantics)))
        .or_else(|| op.map(AssignmentOperation::Primitive))
}

fn lower_index_assignment_operation(
    assignment: &hir::ResolvedIndexAssignment,
    target: ExprId,
    typed_hir: &TypedProgram,
    semantics: &SemanticModel,
) -> AssignmentOperation {
    let mut call = lower_call_target(&assignment.operator, typed_hir, semantics);
    let receiver = ResolvedReceiver::Expression {
        expression: target,
        members: Vec::new(),
    };
    replace_call_receiver(&mut call, receiver);
    AssignmentOperation::Call(call)
}

fn replace_call_receiver(call: &mut CallTarget, receiver: ResolvedReceiver) {
    let (call_receiver, _) = call
        .receiver_with_type_mut()
        .expect("receiver-bound compiler-generated calls retain a receiver");
    *call_receiver = receiver;
}

/// Builds the ordinary protocol call used when `for` consumes an existing
/// iterator cursor. Collection loops return `None` and retain their optimized
/// direct lowering; iterator loops then flow through the same call target,
/// reachability, specialization, and scratch planning as source `next()`.
fn generated_iterator_step_call(
    iterable_value: ValueId,
    index_value: ValueId,
    semantics: &SemanticModel,
    wasm_ir: &mut Program,
) -> Option<ExprId> {
    let receiver_type = semantics
        .value_type(iterable_value)
        .expect("checked for-loop storage has a type");
    let step_type = semantics
        .value_type(index_value)
        .expect("checked for-loop cursor state has a type");
    let TypeKind::Application { constructor, .. } = semantics.types().kind(step_type) else {
        return None;
    };
    if *constructor != crate::stdlib::StdlibTypeConstructorId::IteratorStep {
        return None;
    }
    Some(wasm_ir.push_generated_expression(
        step_type,
        ExpressionKind::Call {
            target: CallTarget::CapabilityRequirement {
                item: crate::stdlib::StdlibItemId::IteratorNext,
                type_arguments: Vec::new(),
                signature: vec![receiver_type, step_type],
                receiver: ResolvedReceiver::Path {
                    root: crate::semantic::ResolvedValue::Variable(iterable_value),
                    members: Vec::new(),
                },
                receiver_type,
            },
            arguments: Vec::new(),
        },
        None,
        None,
    ))
}

/// Converts a generic `Iterable` operand into the cursor stored by a lowered
/// `for` loop. Concrete arrays, sets, and ranges retain their specialized
/// allocation-free lowering; only a loop whose source and storage types differ
/// needs to dispatch through the protocol. Iterator cursors implement
/// `Iterable.iterator` as an identity operation, so this representation also
/// remains correct when a generic iterable is instantiated with a cursor.
fn generated_iterable_iterator_call(
    iterable: ExprId,
    iterable_value: ValueId,
    index_value: ValueId,
    semantics: &SemanticModel,
    wasm_ir: &mut Program,
) -> ExprId {
    let source_type = wasm_ir
        .expression(iterable)
        .expect("lowered for-loop operands have expressions")
        .ty;
    let storage_type = semantics
        .value_type(iterable_value)
        .expect("checked for-loop storage has a type");
    let step_type = semantics
        .value_type(index_value)
        .expect("checked for-loop cursor state has a type");
    let is_cursor_loop = matches!(
        semantics.types().kind(step_type),
        TypeKind::Application {
            constructor: crate::stdlib::StdlibTypeConstructorId::IteratorStep,
            ..
        }
    );
    if !is_cursor_loop || source_type == storage_type {
        return iterable;
    }
    wasm_ir.push_generated_expression(
        storage_type,
        ExpressionKind::Call {
            target: CallTarget::CapabilityRequirement {
                item: crate::stdlib::StdlibItemId::IterableIterator,
                type_arguments: Vec::new(),
                signature: vec![source_type, storage_type],
                receiver: ResolvedReceiver::Expression {
                    expression: iterable,
                    members: Vec::new(),
                },
                receiver_type: source_type,
            },
            arguments: Vec::new(),
        },
        None,
        None,
    )
}

fn mutated_values(program: &TypedProgram) -> HashSet<ValueId> {
    #[derive(Default)]
    struct Collector {
        values: HashSet<ValueId>,
    }

    impl hir::TypedVisitor for Collector {
        fn visit_statement(&mut self, statement: &hir::TypedStatement, program: &TypedProgram) {
            if let TypedStatementKind::Assign { assignment, .. }
            | TypedStatementKind::StateAssign { assignment, .. } = &statement.kind
            {
                self.values.insert(assignment.target);
            }
            hir::walk_typed_statement(self, statement, program);
        }
    }

    let mut collector = Collector::default();
    hir::TypedVisitor::visit_program(&mut collector, program);
    collector.values
}

fn pattern_bindings(pattern: &hir::TypedPattern) -> Vec<ValueId> {
    fn collect(pattern: &hir::TypedPattern, output: &mut Vec<ValueId>) {
        match pattern {
            hir::TypedPattern::Binding(binding) => {
                if !output.contains(&binding.id) {
                    output.push(binding.id);
                }
            }
            hir::TypedPattern::Struct { fields, .. } => {
                for field in fields {
                    collect(&field.pattern.pattern, output);
                }
            }
            hir::TypedPattern::Enum {
                payload: Some(payload),
                ..
            }
            | hir::TypedPattern::OptionSome(payload)
            | hir::TypedPattern::IteratorItem(payload)
            | hir::TypedPattern::ResultSuccess(payload)
            | hir::TypedPattern::ResultError(payload) => collect(&payload.pattern, output),
            hir::TypedPattern::Array { prefix, suffix, .. } => {
                for element in prefix.iter().chain(suffix) {
                    collect(&element.pattern, output);
                }
            }
            hir::TypedPattern::Alternation(alternatives) => {
                for alternative in alternatives {
                    collect(&alternative.pattern, output);
                }
            }
            _ => {}
        }
    }

    let mut output = Vec::new();
    collect(pattern, &mut output);
    output
}

fn binding_destructures(binding: &hir::TypedBindingPattern) -> bool {
    !matches!(
        &binding.pattern.pattern,
        hir::TypedPattern::Binding(leaf) if leaf.id == binding.value
    )
}

fn closure_captures(
    body: ExprId,
    parameters: &[ValueId],
    program: &TypedProgram,
    globals: &HashSet<ValueId>,
    mutated_values: &HashSet<ValueId>,
) -> Vec<ClosureCapture> {
    struct Collector {
        referenced: BTreeSet<ValueId>,
        declared: HashSet<ValueId>,
    }

    impl Collector {
        fn reference(&mut self, value: ResolvedValue) {
            if let ResolvedValue::Variable(value) = value {
                self.referenced.insert(value);
            }
        }

        fn declare_pattern(&mut self, pattern: &hir::TypedPattern) {
            match pattern {
                hir::TypedPattern::Binding(binding) => {
                    self.declared.insert(binding.id);
                }
                hir::TypedPattern::Struct { fields, .. } => {
                    for field in fields {
                        self.declare_pattern(&field.pattern.pattern);
                    }
                }
                hir::TypedPattern::Enum {
                    payload: Some(payload),
                    ..
                }
                | hir::TypedPattern::OptionSome(payload)
                | hir::TypedPattern::IteratorItem(payload)
                | hir::TypedPattern::ResultSuccess(payload)
                | hir::TypedPattern::ResultError(payload) => {
                    self.declare_pattern(&payload.pattern);
                }
                hir::TypedPattern::Array { prefix, suffix, .. } => {
                    for element in prefix.iter().chain(suffix) {
                        self.declare_pattern(&element.pattern);
                    }
                }
                hir::TypedPattern::Alternation(elements) => {
                    for element in elements {
                        self.declare_pattern(&element.pattern);
                    }
                }
                _ => {}
            }
        }
    }

    impl hir::TypedVisitor for Collector {
        fn visit_statement(&mut self, statement: &hir::TypedStatement, program: &TypedProgram) {
            match &statement.kind {
                TypedStatementKind::Variable { binding, .. } => {
                    self.declared.insert(binding.value);
                    self.declare_pattern(&binding.pattern.pattern);
                }
                TypedStatementKind::Assign { assignment, .. } => {
                    self.referenced.insert(assignment.target);
                }
                TypedStatementKind::For {
                    binding,
                    iterable_value,
                    index_value,
                    version_value,
                    ..
                } => {
                    self.declared.extend([
                        binding.value,
                        *iterable_value,
                        *index_value,
                        *version_value,
                    ]);
                    self.declare_pattern(&binding.pattern.pattern);
                }
                TypedStatementKind::StateAssign { .. }
                | TypedStatementKind::IndexAssign { .. }
                | TypedStatementKind::If { .. }
                | TypedStatementKind::While { .. }
                | TypedStatementKind::Suspend { .. }
                | TypedStatementKind::Yield { .. }
                | TypedStatementKind::Expression(_) => {}
            }
            hir::walk_typed_statement(self, statement, program);
        }

        fn visit_expression(&mut self, expression: &hir::TypedExpression, program: &TypedProgram) {
            if let TypedExpressionKind::Closure { parameters, .. } = &expression.kind {
                for parameter in parameters {
                    self.declared.insert(parameter.value);
                    self.declare_pattern(&parameter.pattern.pattern);
                }
            }
            if let TypedExpressionKind::Is { pattern, .. } = &expression.kind {
                self.declare_pattern(&pattern.pattern);
            }
            match &expression.resolution {
                Some(hir::ExpressionResolution::ValuePath {
                    root: Some(root), ..
                }) => self.reference(*root),
                Some(hir::ExpressionResolution::Call(call)) => {
                    if let Some(ResolvedReceiver::Path { root, .. }) = call.receiver() {
                        self.reference(*root);
                    }
                }
                Some(hir::ExpressionResolution::DynamicCall(
                    crate::semantic::DynamicCallCallee::Value(value),
                )) => {
                    self.referenced.insert(*value);
                }
                Some(hir::ExpressionResolution::ValuePath { root: None, .. })
                | Some(hir::ExpressionResolution::Member { .. })
                | Some(hir::ExpressionResolution::DynamicCall(
                    crate::semantic::DynamicCallCallee::Expression(_),
                ))
                | Some(hir::ExpressionResolution::FunctionValue(_))
                | Some(hir::ExpressionResolution::StructLiteral { .. })
                | Some(hir::ExpressionResolution::EnumConstructor { .. })
                | None => {}
            }
            hir::walk_typed_expression(self, expression, program);
        }

        fn visit_match_arm(&mut self, arm: &hir::TypedMatchArm, program: &TypedProgram) {
            self.declare_pattern(&arm.pattern);
            hir::walk_typed_match_arm(self, arm, program);
        }
    }

    let mut collector = Collector {
        referenced: BTreeSet::new(),
        declared: parameters.iter().copied().collect(),
    };
    hir::TypedVisitor::visit_expression(
        &mut collector,
        program
            .expression(body)
            .expect("closure body belongs to typed HIR"),
        program,
    );
    collector
        .referenced
        .into_iter()
        .filter(|value| !collector.declared.contains(value) && !globals.contains(value))
        .map(|value| ClosureCapture {
            value,
            mutable: mutated_values.contains(&value),
        })
        .collect()
}

#[allow(clippy::too_many_arguments)]
fn lower_body(
    owner: BodyOwner,
    block: &hir::TypedBlock,
    parameters: &[hir::TypedBindingPattern],
    typed_hir: &TypedProgram,
    semantics: &SemanticModel,
    effects: &OperationAnalysis,
    capabilities: &crate::capabilities::CapabilityAnalysis,
    source: SourceProvenance,
    mutated_values: &HashSet<ValueId>,
    wasm_ir: &mut Program,
) -> Body {
    let abi = match &owner {
        BodyOwner::Action(ActionKind::OnAttach) => BodyAbi::AsyncAction,
        BodyOwner::Action(ActionKind::WhileAttached)
            if typed_block_contains_await(block, typed_hir, source.profile) =>
        {
            BodyAbi::AsyncAction
        }
        BodyOwner::Action(_) => BodyAbi::Direct,
        BodyOwner::Function(instance)
            if let Some(result) = semantics.function_result(instance.function)
                && let result = semantics.specialize_type(instance, result)
                && let crate::types::TypeKind::Iterator { item, .. } =
                    semantics.types().kind(result)
                && hir::typed_block_contains_yield(block, typed_hir) =>
        {
            BodyAbi::Generator(GeneratorAbi { item: *item })
        }
        BodyOwner::Function(instance)
            if effects.function(instance.function).suspension == SuspensionKind::Suspends =>
        {
            let result = semantics
                .function_completion(instance.function)
                .expect("checked functions have result types");
            let result = semantics.specialize_type(instance, result);
            BodyAbi::AsyncFunction(AsyncFunctionAbi { completion: result })
        }
        BodyOwner::Function(_) => BodyAbi::Direct,
    };
    let mut entry = if !matches!(abi, BodyAbi::Direct) {
        lower_async_block(block, typed_hir, semantics, source, wasm_ir)
    } else {
        lower_block(block, typed_hir, semantics, source, wasm_ir)
    };
    for parameter in parameters.iter().rev() {
        if !binding_destructures(parameter) {
            continue;
        }
        entry.statements.insert(
            0,
            Statement::BindPattern {
                value: parameter.value,
                pattern: lower_pattern(&parameter.pattern.pattern, &parameter.pattern.resolution),
            },
        );
    }
    let mut next_async_state = 1;
    assign_async_states(&mut entry, &mut next_async_state);
    let mut locals = plan_block(&entry, wasm_ir, semantics, capabilities);
    if matches!(abi, BodyAbi::Direct) {
        snapshot_projections::plan(&entry, wasm_ir, semantics, mutated_values, &mut locals);
    }
    let frame_values = plan_frame_values(&mut entry, &locals, wasm_ir);
    let frame_temporaries = locals
        .iter()
        .filter_map(|local| match local.purpose {
            LocalPurpose::Temporary(temporary) => Some(temporary),
            _ => None,
        })
        .collect();
    let cancellation_region = match &owner {
        BodyOwner::Action(ActionKind::OnAttach | ActionKind::WhileAttached)
            if matches!(abi, BodyAbi::AsyncAction) =>
        {
            Some(CancellationRegion::AttachmentLifetime)
        }
        BodyOwner::Function(instance)
            if effects.function(instance.function).cancellation
                == CancellationKind::ProcessClose =>
        {
            Some(CancellationRegion::AttachmentLifetime)
        }
        BodyOwner::Action(_) | BodyOwner::Function(_) => None,
    };
    Body {
        owner,
        abi,
        entry,
        locals,
        frame_values,
        frame_temporaries,
        cancellation_region,
        async_state_count: next_async_state,
    }
}

fn lower_expression_body(
    expression: ExprId,
    typed_hir: &TypedProgram,
    semantics: &SemanticModel,
    source: SourceProvenance,
    wasm_ir: &mut Program,
) -> Block {
    let normalized = normalize_expression_suspensions(expression, typed_hir, semantics, wasm_ir);
    wrap_async_expression_steps(
        normalized.steps,
        Block {
            statements: Vec::new(),
            terminator: Terminator::Return(Some(normalized.value)),
        },
        typed_hir,
        semantics,
        source,
        wasm_ir,
    )
}

fn plan_frame_values(entry: &mut Block, locals: &[Local], program: &Program) -> Vec<ValueId> {
    let local_values = locals
        .iter()
        .filter_map(|local| match local.purpose {
            LocalPurpose::Value(value) => Some(value),
            _ => None,
        })
        .collect::<HashSet<_>>();
    let mut frame_values = HashSet::new();
    analyze_suspension_liveness(
        entry,
        HashSet::new(),
        &local_values,
        locals,
        program,
        &mut frame_values,
    );
    locals
        .iter()
        .filter_map(|local| match local.purpose {
            LocalPurpose::Value(value) if frame_values.contains(&value) => Some(value),
            _ => None,
        })
        .collect()
}

fn analyze_suspension_liveness(
    block: &mut Block,
    live_after: HashSet<ValueId>,
    local_values: &HashSet<ValueId>,
    ordered_locals: &[Local],
    program: &Program,
    frame_values: &mut HashSet<ValueId>,
) -> HashSet<ValueId> {
    let mut live = match &mut block.terminator {
        Terminator::Yield {
            value,
            live_values,
            continuation,
            ..
        } => {
            let continuation_live = analyze_suspension_liveness(
                continuation,
                live_after,
                local_values,
                ordered_locals,
                program,
                frame_values,
            );
            live_values.clear();
            live_values.extend(
                ordered_locals
                    .iter()
                    .filter_map(|local| match local.purpose {
                        LocalPurpose::Value(value) if continuation_live.contains(&value) => {
                            Some(value)
                        }
                        _ => None,
                    }),
            );
            frame_values.extend(live_values.iter().copied());
            let mut before_yield = continuation_live;
            collect_expression_values(*value, &mut before_yield, local_values, program);
            before_yield
        }
        Terminator::Suspend {
            destination,
            value,
            live_values,
            continuation,
            ..
        } => {
            let continuation_live = analyze_suspension_liveness(
                continuation,
                live_after,
                local_values,
                ordered_locals,
                program,
                frame_values,
            );
            let mut suspension_live = continuation_live.clone();
            collect_expression_values(*value, &mut suspension_live, local_values, program);
            live_values.clear();
            live_values.extend(
                ordered_locals
                    .iter()
                    .filter_map(|local| match local.purpose {
                        LocalPurpose::Value(value) if suspension_live.contains(&value) => {
                            Some(value)
                        }
                        _ => None,
                    }),
            );
            frame_values.extend(live_values.iter().copied());

            let mut before_suspend = continuation_live;
            if let Some(binding) = destination.source_value() {
                before_suspend.remove(&binding);
            }
            collect_expression_values(*value, &mut before_suspend, local_values, program);
            before_suspend
        }
        Terminator::Retry {
            attempt,
            continuation,
            live_values,
            ..
        } => {
            let continuation_live = analyze_suspension_liveness(
                continuation,
                live_after,
                local_values,
                ordered_locals,
                program,
                frame_values,
            );
            let attempt_live = analyze_suspension_liveness(
                attempt,
                HashSet::new(),
                local_values,
                ordered_locals,
                program,
                frame_values,
            );
            let mut suspension_live = continuation_live;
            suspension_live.extend(attempt_live);
            live_values.clear();
            live_values.extend(
                ordered_locals
                    .iter()
                    .filter_map(|local| match local.purpose {
                        LocalPurpose::Value(value) if suspension_live.contains(&value) => {
                            Some(value)
                        }
                        _ => None,
                    }),
            );
            frame_values.extend(live_values.iter().copied());
            suspension_live
        }
        Terminator::RetryComplete { value, .. } => {
            let mut live = HashSet::new();
            collect_expression_values(*value, &mut live, local_values, program);
            live
        }
        Terminator::Return(value) => {
            let mut live = HashSet::new();
            if let Some(value) = value {
                collect_expression_values(*value, &mut live, local_values, program);
            }
            live
        }
        Terminator::Break(value) => {
            let mut live = live_after;
            if let Some(value) = value {
                collect_expression_values(*value, &mut live, local_values, program);
            }
            live
        }
        Terminator::Continue => live_after,
        Terminator::AsyncWhile {
            header,
            continuation,
            ..
        } => {
            let continuation_live = analyze_suspension_liveness(
                continuation,
                live_after,
                local_values,
                ordered_locals,
                program,
                frame_values,
            );
            analyze_suspension_liveness(
                header,
                continuation_live,
                local_values,
                ordered_locals,
                program,
                frame_values,
            )
        }
        Terminator::AsyncWhileCondition {
            condition, body, ..
        } => {
            let mut loop_live = live_after;
            collect_expression_values(*condition, &mut loop_live, local_values, program);
            loop {
                let body_live = analyze_suspension_liveness(
                    body,
                    loop_live.clone(),
                    local_values,
                    ordered_locals,
                    program,
                    frame_values,
                );
                let previous_len = loop_live.len();
                loop_live.extend(body_live);
                if loop_live.len() == previous_len {
                    break;
                }
            }
            loop_live
        }
        Terminator::AsyncFor {
            binding,
            pattern,
            iterable_value,
            index_value,
            version_value,
            body,
            continuation,
            ..
        } => {
            let continuation_live = analyze_suspension_liveness(
                continuation,
                live_after,
                local_values,
                ordered_locals,
                program,
                frame_values,
            );
            let mut loop_live = continuation_live;
            loop_live.insert(*iterable_value);
            loop_live.insert(*index_value);
            loop_live.insert(*version_value);
            loop {
                let body_live = analyze_suspension_liveness(
                    body,
                    loop_live.clone(),
                    local_values,
                    ordered_locals,
                    program,
                    frame_values,
                );
                let previous_len = loop_live.len();
                loop_live.extend(body_live);
                if loop_live.len() == previous_len {
                    break;
                }
            }
            loop_live.remove(binding);
            pattern.visit_bindings(&mut |binding| {
                loop_live.remove(&binding);
            });
            loop_live
        }
        Terminator::Throw { error, .. } => {
            let mut live = HashSet::new();
            collect_expression_values(*error, &mut live, local_values, program);
            live
        }
        Terminator::Fallthrough => live_after,
    };
    analyze_statements_liveness(
        &mut block.statements,
        &mut live,
        local_values,
        ordered_locals,
        program,
        frame_values,
    );
    live
}

fn analyze_statements_liveness(
    statements: &mut [Statement],
    live: &mut HashSet<ValueId>,
    local_values: &HashSet<ValueId>,
    ordered_locals: &[Local],
    program: &Program,
    frame_values: &mut HashSet<ValueId>,
) {
    for statement in statements.iter_mut().rev() {
        match statement {
            Statement::DebugLocation(_) => {}
            Statement::Store {
                target,
                operation,
                value,
                ..
            } => {
                live.remove(target);
                if operation.is_some() && local_values.contains(target) {
                    live.insert(*target);
                }
                collect_expression_values(*value, live, local_values, program);
            }
            Statement::BindPattern { value, pattern } => {
                pattern.visit_bindings(&mut |binding| {
                    live.remove(&binding);
                });
                if local_values.contains(value) {
                    live.insert(*value);
                }
            }
            Statement::StateStore { value, .. } => {
                collect_expression_values(*value, live, local_values, program);
            }
            Statement::Evaluate { expression, .. } => {
                collect_expression_values(*expression, live, local_values, program);
            }
            Statement::StoreTemporary { value, .. } => {
                collect_expression_values(*value, live, local_values, program);
            }
            Statement::IndexStore { target, value, .. } => {
                collect_expression_values(*target, live, local_values, program);
                collect_expression_values(*value, live, local_values, program);
            }
            Statement::If {
                condition,
                then_block,
                else_block,
            } => {
                let mut then_live = analyze_suspension_liveness(
                    then_block,
                    live.clone(),
                    local_values,
                    ordered_locals,
                    program,
                    frame_values,
                );
                let else_live = analyze_suspension_liveness(
                    else_block,
                    live.clone(),
                    local_values,
                    ordered_locals,
                    program,
                    frame_values,
                );
                then_live.extend(else_live);
                collect_expression_values(*condition, &mut then_live, local_values, program);
                *live = then_live;
            }
            Statement::Match { value, arms, .. } => {
                let mut match_live = HashSet::new();
                for arm in arms {
                    let mut arm_live = analyze_suspension_liveness(
                        &mut arm.block,
                        live.clone(),
                        local_values,
                        ordered_locals,
                        program,
                        frame_values,
                    );
                    if let Some(guard) = arm.guard {
                        collect_expression_values(guard, &mut arm_live, local_values, program);
                    }
                    arm.pattern.visit_bindings(&mut |binding| {
                        arm_live.remove(&binding);
                    });
                    match_live.extend(arm_live);
                }
                collect_expression_values(*value, &mut match_live, local_values, program);
                *live = match_live;
            }
            Statement::Fallback {
                value,
                fallback_block,
                success_block,
                ..
            } => {
                let mut branch_live = analyze_suspension_liveness(
                    fallback_block,
                    live.clone(),
                    local_values,
                    ordered_locals,
                    program,
                    frame_values,
                );
                branch_live.extend(analyze_suspension_liveness(
                    success_block,
                    live.clone(),
                    local_values,
                    ordered_locals,
                    program,
                    frame_values,
                ));
                collect_expression_values(*value, &mut branch_live, local_values, program);
                *live = branch_live;
            }
            Statement::While {
                condition, body, ..
            } => {
                let mut body_live = analyze_suspension_liveness(
                    body,
                    live.clone(),
                    local_values,
                    ordered_locals,
                    program,
                    frame_values,
                );
                body_live.extend(live.iter().copied());
                collect_expression_values(*condition, &mut body_live, local_values, program);
                *live = body_live;
            }
            Statement::For {
                binding,
                pattern,
                iterable_value,
                index_value,
                version_value,
                iterable,
                iterator_step,
                body,
            } => {
                let mut body_live = analyze_suspension_liveness(
                    body,
                    live.clone(),
                    local_values,
                    ordered_locals,
                    program,
                    frame_values,
                );
                body_live.extend(live.iter().copied());
                body_live.remove(binding);
                pattern.visit_bindings(&mut |binding| {
                    body_live.remove(&binding);
                });
                body_live.remove(iterable_value);
                body_live.remove(index_value);
                body_live.remove(version_value);
                collect_expression_values(*iterable, &mut body_live, local_values, program);
                if let Some(iterator_step) = iterator_step {
                    collect_expression_values(
                        *iterator_step,
                        &mut body_live,
                        local_values,
                        program,
                    );
                }
                *live = body_live;
            }
            Statement::ForInit {
                binding,
                pattern,
                iterable_value,
                index_value,
                version_value,
                iterable,
                iterator_step,
            } => {
                live.remove(binding);
                pattern.visit_bindings(&mut |binding| {
                    live.remove(&binding);
                });
                live.remove(iterable_value);
                live.remove(index_value);
                live.remove(version_value);
                collect_expression_values(*iterable, live, local_values, program);
                if let Some(iterator_step) = iterator_step {
                    collect_expression_values(*iterator_step, live, local_values, program);
                }
            }
        }
    }
}

fn collect_expression_values(
    expression: ExprId,
    live: &mut HashSet<ValueId>,
    local_values: &HashSet<ValueId>,
    program: &Program,
) {
    struct Collector<'a> {
        live: &'a mut HashSet<ValueId>,
        local_values: &'a HashSet<ValueId>,
    }

    impl Visitor for Collector<'_> {
        fn visit_expression(&mut self, expression: &Expression, program: &Program) {
            if let ExpressionKind::Suspend { destination, .. } = expression.kind
                && self.local_values.contains(&destination)
            {
                self.live.insert(destination);
            }
            let root = match &expression.kind {
                ExpressionKind::Path { root, .. } => *root,
                ExpressionKind::Call {
                    target:
                        CallTarget::UserMethod { receiver, .. }
                        | CallTarget::Intrinsic {
                            receiver: Some(receiver),
                            ..
                        },
                    ..
                } => receiver.path().map(|(root, _)| root),
                _ => None,
            };
            if let Some(ResolvedValue::Variable(value)) = root
                && self.local_values.contains(&value)
            {
                self.live.insert(value);
            }
            if let ExpressionKind::Closure { closure, .. } = expression.kind {
                self.live.extend(
                    program
                        .closure_captures(closure)
                        .iter()
                        .map(|capture| capture.value)
                        .filter(|value| self.local_values.contains(value)),
                );
                // The body executes only when invoked. Closure construction
                // reads exactly its lexical capture set, not its statements.
                return;
            }
            walk_expression(self, expression, program);
        }
    }

    Collector { live, local_values }.visit_expression_id(expression, program);
}

fn lower_block(
    block: &hir::TypedBlock,
    typed_hir: &TypedProgram,
    semantics: &SemanticModel,
    source: SourceProvenance,
    wasm_ir: &mut Program,
) -> Block {
    // The expression normalizer is also responsible for statement-bearing
    // value blocks. Running it for direct bodies keeps synchronous and
    // suspending value blocks on one lowering path.
    lower_async_statements(
        &block.statements,
        Block::default(),
        typed_hir,
        semantics,
        source,
        wasm_ir,
    )
}

fn lower_async_block(
    block: &hir::TypedBlock,
    typed_hir: &TypedProgram,
    semantics: &SemanticModel,
    source: SourceProvenance,
    wasm_ir: &mut Program,
) -> Block {
    lower_async_statements(
        &block.statements,
        Block::default(),
        typed_hir,
        semantics,
        source,
        wasm_ir,
    )
}

#[derive(Debug, Clone)]
enum AsyncExpressionStep {
    Store {
        target: TemporaryId,
        value: ExprId,
    },
    Suspend {
        mode: SuspensionMode,
        destination: SuspensionDestination,
        value: ExprId,
        cancellation: Option<CancellationRegion>,
        source: Option<Span>,
    },
    Retry {
        attempt: Box<NormalizedExpression>,
        destination: SuspensionDestination,
        cancellation: Option<CancellationRegion>,
        source: Option<Span>,
    },
    If {
        condition: ExprId,
        then_expression: Box<NormalizedExpression>,
        else_expression: Box<NormalizedExpression>,
        destination: Option<TemporaryId>,
    },
    Fallback {
        expression: ExprId,
        value: ExprId,
        fallback: Box<NormalizedExpression>,
        destination: TemporaryId,
        success: ExprId,
    },
    Match {
        expression: ExprId,
        value: ExprId,
        arms: Vec<NormalizedMatchArm>,
        destination: Option<TemporaryId>,
    },
    ValueBlock {
        statements: hir::TypedBlock,
        tail: Box<NormalizedExpression>,
        destination: TemporaryId,
    },
    Loop {
        body: hir::TypedBlock,
        destination: TemporaryId,
    },
}

#[derive(Debug, Clone)]
struct NormalizedMatchArm {
    pattern_id: PatternId,
    pattern: LoweredPattern,
    guard: Option<NormalizedExpression>,
    value: NormalizedExpression,
}

impl AsyncExpressionStep {
    fn suspends(&self) -> bool {
        match self {
            Self::Suspend { .. } | Self::Retry { .. } => true,
            Self::Store { .. } => false,
            Self::If {
                then_expression,
                else_expression,
                ..
            } => then_expression
                .steps
                .iter()
                .chain(&else_expression.steps)
                .any(Self::suspends),
            Self::Match { arms, .. } => arms
                .iter()
                .flat_map(|arm| {
                    arm.guard
                        .iter()
                        .flat_map(|guard| &guard.steps)
                        .chain(&arm.value.steps)
                })
                .any(Self::suspends),
            Self::Fallback { fallback, .. } => fallback.steps.iter().any(Self::suspends),
            // A value block is statement-aware lowering work even when it has
            // no suspension. Treat it conservatively when deciding whether a
            // parent expression must capture its other operands.
            Self::ValueBlock { .. } | Self::Loop { .. } => true,
        }
    }
}

#[derive(Debug, Clone)]
struct NormalizedExpression {
    value: ExprId,
    steps: Vec<AsyncExpressionStep>,
}

struct AsyncNormalizationContext<'a> {
    typed_hir: &'a TypedProgram,
    wasm_ir: &'a mut Program,
}

fn normalize_expression_suspensions(
    expression: ExprId,
    typed_hir: &TypedProgram,
    _semantics: &SemanticModel,
    wasm_ir: &mut Program,
) -> NormalizedExpression {
    normalize_expression_suspensions_with(
        expression,
        &mut AsyncNormalizationContext { typed_hir, wasm_ir },
    )
}

fn normalize_expression_suspensions_with(
    expression: ExprId,
    context: &mut AsyncNormalizationContext<'_>,
) -> NormalizedExpression {
    if let Some(TypedExpression {
        kind: TypedExpressionKind::Loop { body },
        ..
    }) = context.typed_hir.expression(expression)
    {
        let original = context
            .wasm_ir
            .expression(expression)
            .expect("normalized expression belongs to Wasm IR")
            .clone();
        let storage_ty = original
            .conversion
            .map_or(original.ty, |conversion| conversion.source);
        let (destination, value) =
            context
                .wasm_ir
                .temporary_read(storage_ty, original.ty, original.conversion);
        return NormalizedExpression {
            value,
            steps: vec![AsyncExpressionStep::Loop {
                body: body.clone(),
                destination,
            }],
        };
    }

    if let Some(TypedExpression {
        kind: TypedExpressionKind::Block { statements, value },
        ..
    }) = context.typed_hir.expression(expression)
    {
        let original = context
            .wasm_ir
            .expression(expression)
            .expect("normalized expression belongs to Wasm IR")
            .clone();
        let tail = if let Some(value) = value {
            normalize_expression_suspensions_with(*value, context)
        } else {
            let value = context.wasm_ir.push_generated_expression(
                original.ty,
                ExpressionKind::None,
                None,
                original.source,
            );
            NormalizedExpression {
                value,
                steps: Vec::new(),
            }
        };
        let storage_ty = original
            .conversion
            .map_or(original.ty, |conversion| conversion.source);
        let (destination, value) =
            context
                .wasm_ir
                .temporary_read(storage_ty, original.ty, original.conversion);
        return NormalizedExpression {
            value,
            steps: vec![AsyncExpressionStep::ValueBlock {
                statements: statements.clone(),
                tail: Box::new(tail),
                destination,
            }],
        };
    }

    let original = context
        .wasm_ir
        .expression(expression)
        .expect("normalized expression belongs to Wasm IR")
        .clone();

    // A closure body is a different function body, not an eagerly evaluated
    // child of the closure-construction expression. Its suspension and block
    // normalization happens independently when `ClosureBody` is lowered.
    if matches!(original.kind, ExpressionKind::Closure { .. }) {
        return NormalizedExpression {
            value: expression,
            steps: Vec::new(),
        };
    }

    if let ExpressionKind::Suspend { mode, value, .. } = original.kind.clone() {
        let operand = normalize_expression_suspensions_with(value, context);
        let cancellation = suspension_cancellation(mode, value, context.typed_hir);
        let storage_ty = original
            .conversion
            .map_or(original.ty, |conversion| conversion.source);
        let (temporary, value) =
            context
                .wasm_ir
                .temporary_read(storage_ty, original.ty, original.conversion);
        // Both operators accept the same arbitrary expression tree, including
        // the ordinary `ExpressionKind::Block`. Their evaluation boundaries
        // differ: retry owns the whole normalized tree so every poll starts a
        // fresh attempt, while await evaluates and captures its operand once
        // before polling the resulting future.
        let steps = if mode == SuspensionMode::Retry {
            vec![AsyncExpressionStep::Retry {
                attempt: Box::new(operand),
                destination: SuspensionDestination::Temporary(temporary),
                cancellation,
                source: original.source,
            }]
        } else {
            let mut steps = operand.steps;
            let operand_value = capture_await_operand(operand.value, &mut steps, context);
            steps.push(AsyncExpressionStep::Suspend {
                mode,
                destination: SuspensionDestination::Temporary(temporary),
                value: operand_value,
                cancellation,
                source: original.source,
            });
            steps
        };
        debug_assert!(steps.last().is_some());
        return NormalizedExpression { value, steps };
    }

    if let ExpressionKind::If {
        condition,
        then_expr,
        else_expr,
    } = original.kind.clone()
    {
        return normalize_if_expression(original, condition, then_expr, else_expr, context);
    }

    if let ExpressionKind::Binary { op, left, right } = original.kind.clone()
        && matches!(op, BinaryOp::And | BinaryOp::Or)
    {
        let short_circuit = context.wasm_ir.push_generated_expression(
            original.ty,
            ExpressionKind::Bool(op == BinaryOp::Or),
            None,
            original.source,
        );
        let (then_expr, else_expr) = if op == BinaryOp::And {
            (right, short_circuit)
        } else {
            (short_circuit, right)
        };
        return normalize_if_expression(original, left, then_expr, else_expr, context);
    }

    if let ExpressionKind::Match { value, arms } = original.kind.clone() {
        return normalize_match_expression(original, value, arms, context);
    }

    if let ExpressionKind::Fallback { value, fallback } = original.kind.clone() {
        return normalize_fallback_expression(original, value, fallback, context);
    }

    let mut children = Vec::new();
    visit_expression_children(&original.kind, |child| children.push(child));
    if children.is_empty() {
        return NormalizedExpression {
            value: expression,
            steps: Vec::new(),
        };
    }

    let normalized = children
        .iter()
        .map(|child| normalize_expression_suspensions_with(*child, context))
        .collect::<Vec<_>>();
    let suspends = normalized
        .iter()
        .map(|child| child.steps.iter().any(AsyncExpressionStep::suspends))
        .collect::<Vec<_>>();
    let mut replacements = Vec::with_capacity(normalized.len());
    let mut steps = Vec::new();
    for (index, child) in normalized.into_iter().enumerate() {
        steps.extend(child.steps);
        let later_suspends = suspends[index + 1..].iter().any(|suspends| *suspends);
        if later_suspends {
            let ty = context.wasm_ir.effective_expression_type(child.value);
            let (temporary, replacement) = context.wasm_ir.temporary(ty);
            steps.push(AsyncExpressionStep::Store {
                target: temporary,
                value: child.value,
            });
            replacements.push(replacement);
        } else {
            replacements.push(child.value);
        }
    }

    let mut replacements = replacements.into_iter();
    let kind = map_expression_children(original.kind, |_| {
        replacements
            .next()
            .expect("every expression child has a normalized replacement")
    });
    let value = context.wasm_ir.push_generated_expression(
        original.ty,
        kind,
        original.conversion,
        original.source,
    );
    NormalizedExpression { value, steps }
}

fn capture_await_operand(
    operand: ExprId,
    steps: &mut Vec<AsyncExpressionStep>,
    context: &mut AsyncNormalizationContext<'_>,
) -> ExprId {
    let expression = context
        .wasm_ir
        .expression(operand)
        .expect("await operand belongs to Wasm IR")
        .clone();
    let source = expression.source;
    let ExpressionKind::Call {
        mut target,
        mut arguments,
    } = expression.kind
    else {
        return operand;
    };

    if let Some((receiver, receiver_type)) = target.receiver_with_type_mut() {
        match receiver.clone() {
            ResolvedReceiver::Expression {
                expression,
                members,
            } => {
                let receiver_expression = if members.is_empty() {
                    expression
                } else {
                    context.wasm_ir.push_generated_expression(
                        receiver_type,
                        ExpressionKind::Member {
                            receiver: expression,
                            members,
                        },
                        None,
                        source,
                    )
                };
                let captured = capture_await_value(receiver_expression, steps, context);
                *receiver = ResolvedReceiver::Expression {
                    expression: captured,
                    members: Vec::new(),
                };
            }
            ResolvedReceiver::Path { root, members }
                if matches!(root, ResolvedValue::Variable(_)) || !members.is_empty() =>
            {
                let receiver_expression = context.wasm_ir.push_generated_expression(
                    receiver_type,
                    ExpressionKind::Path {
                        root: Some(root),
                        members,
                    },
                    None,
                    source,
                );
                let captured = capture_await_value(receiver_expression, steps, context);
                *receiver = ResolvedReceiver::Expression {
                    expression: captured,
                    members: Vec::new(),
                };
            }
            ResolvedReceiver::Path { .. } => {}
        }
    }
    for argument in &mut arguments {
        *argument = capture_await_value(*argument, steps, context);
    }
    context.wasm_ir.push_generated_expression(
        expression.ty,
        ExpressionKind::Call { target, arguments },
        expression.conversion,
        source,
    )
}

fn capture_await_value(
    value: ExprId,
    steps: &mut Vec<AsyncExpressionStep>,
    context: &mut AsyncNormalizationContext<'_>,
) -> ExprId {
    let expression = context
        .wasm_ir
        .expression(value)
        .expect("await capture belongs to Wasm IR");
    if matches!(
        expression.kind,
        ExpressionKind::None
            | ExpressionKind::IteratorEnd
            | ExpressionKind::Bool(_)
            | ExpressionKind::Int { .. }
            | ExpressionKind::Float(_)
            | ExpressionKind::Char(_)
            | ExpressionKind::String(_)
            | ExpressionKind::Signature(_)
            | ExpressionKind::Temporary(_)
            | ExpressionKind::FallbackSuccess { .. }
    ) {
        return value;
    }
    let ty = context.wasm_ir.effective_expression_type(value);
    let (temporary, captured) = context.wasm_ir.temporary(ty);
    steps.push(AsyncExpressionStep::Store {
        target: temporary,
        value,
    });
    captured
}

fn normalize_fallback_expression(
    original: Expression,
    value: ExprId,
    fallback: ExprId,
    context: &mut AsyncNormalizationContext<'_>,
) -> NormalizedExpression {
    let input = normalize_expression_suspensions_with(value, context);
    let fallback = normalize_expression_suspensions_with(fallback, context);
    if fallback.steps.is_empty() {
        let kind = ExpressionKind::Fallback {
            value: input.value,
            fallback: fallback.value,
        };
        let value = context.wasm_ir.push_generated_expression(
            original.ty,
            kind,
            original.conversion,
            original.source,
        );
        return NormalizedExpression {
            value,
            steps: input.steps,
        };
    }

    let storage_ty = original
        .conversion
        .map_or(original.ty, |conversion| conversion.source);
    let (destination, result) =
        context
            .wasm_ir
            .temporary_read(storage_ty, original.ty, original.conversion);
    let success = context.wasm_ir.push_generated_expression(
        storage_ty,
        ExpressionKind::FallbackSuccess {
            source: original.id,
        },
        None,
        original.source,
    );
    let mut steps = input.steps;
    steps.push(AsyncExpressionStep::Fallback {
        expression: original.id,
        value: input.value,
        fallback: Box::new(fallback),
        destination,
        success,
    });
    NormalizedExpression {
        value: result,
        steps,
    }
}

fn normalize_match_expression(
    original: Expression,
    value: ExprId,
    arms: Vec<MatchArm>,
    context: &mut AsyncNormalizationContext<'_>,
) -> NormalizedExpression {
    let input = normalize_expression_suspensions_with(value, context);
    let arms = arms
        .into_iter()
        .map(|arm| NormalizedMatchArm {
            pattern_id: arm.pattern_id,
            pattern: arm.pattern,
            guard: arm
                .guard
                .map(|guard| normalize_expression_suspensions_with(guard, context)),
            value: normalize_expression_suspensions_with(arm.value, context),
        })
        .collect::<Vec<_>>();
    if arms.iter().all(|arm| {
        arm.value.steps.is_empty()
            && arm
                .guard
                .as_ref()
                .is_none_or(|guard| guard.steps.is_empty())
    }) {
        let kind = ExpressionKind::Match {
            value: input.value,
            arms: arms
                .into_iter()
                .map(|arm| MatchArm {
                    pattern_id: arm.pattern_id,
                    pattern: arm.pattern,
                    guard: arm.guard.map(|guard| guard.value),
                    value: arm.value.value,
                })
                .collect(),
        };
        let value = context.wasm_ir.push_generated_expression(
            original.ty,
            kind,
            original.conversion,
            original.source,
        );
        return NormalizedExpression {
            value,
            steps: input.steps,
        };
    }

    let storage_ty = original
        .conversion
        .map_or(original.ty, |conversion| conversion.source);
    let (temporary, value) =
        context
            .wasm_ir
            .temporary_read(storage_ty, original.ty, original.conversion);
    let destination = Some(temporary);
    let guard_suspends = arms.iter().any(|arm| {
        arm.guard
            .as_ref()
            .is_some_and(|guard| !guard.steps.is_empty())
    });
    let mut steps = input.steps;
    let input_value = if guard_suspends {
        let ty = context.wasm_ir.effective_expression_type(input.value);
        let (temporary, value) = context.wasm_ir.temporary(ty);
        steps.push(AsyncExpressionStep::Store {
            target: temporary,
            value: input.value,
        });
        value
    } else {
        input.value
    };
    steps.push(AsyncExpressionStep::Match {
        expression: original.id,
        value: input_value,
        arms,
        destination,
    });
    NormalizedExpression { value, steps }
}

fn normalize_if_expression(
    original: Expression,
    condition: ExprId,
    then_expr: ExprId,
    else_expr: ExprId,
    context: &mut AsyncNormalizationContext<'_>,
) -> NormalizedExpression {
    let condition = normalize_expression_suspensions_with(condition, context);
    let then_expression = normalize_expression_suspensions_with(then_expr, context);
    let else_expression = normalize_expression_suspensions_with(else_expr, context);
    if then_expression.steps.is_empty() && else_expression.steps.is_empty() {
        let kind = ExpressionKind::If {
            condition: condition.value,
            then_expr: then_expression.value,
            else_expr: else_expression.value,
        };
        let value = context.wasm_ir.push_generated_expression(
            original.ty,
            kind,
            original.conversion,
            original.source,
        );
        let mut steps = condition.steps;
        steps.extend(then_expression.steps);
        steps.extend(else_expression.steps);
        return NormalizedExpression { value, steps };
    }

    let storage_ty = original
        .conversion
        .map_or(original.ty, |conversion| conversion.source);
    let (temporary, value) =
        context
            .wasm_ir
            .temporary_read(storage_ty, original.ty, original.conversion);
    let destination = Some(temporary);
    let mut steps = condition.steps;
    steps.push(AsyncExpressionStep::If {
        condition: condition.value,
        then_expression: Box::new(then_expression),
        else_expression: Box::new(else_expression),
        destination,
    });
    NormalizedExpression { value, steps }
}

fn map_expression_children(
    kind: ExpressionKind,
    mut map: impl FnMut(ExprId) -> ExprId,
) -> ExpressionKind {
    match kind {
        ExpressionKind::None
        | ExpressionKind::IteratorEnd
        | ExpressionKind::Bool(_)
        | ExpressionKind::Int { .. }
        | ExpressionKind::Float(_)
        | ExpressionKind::Char(_)
        | ExpressionKind::String(_)
        | ExpressionKind::Signature(_)
        | ExpressionKind::Temporary(_)
        | ExpressionKind::FallbackSuccess { .. }
        | ExpressionKind::Break(None)
        | ExpressionKind::Continue
        | ExpressionKind::Return(None)
        | ExpressionKind::Path { .. }
        | ExpressionKind::FunctionValue { .. } => kind,
        ExpressionKind::ValueBlock => kind,
        ExpressionKind::Loop => kind,
        ExpressionKind::InterpolatedString(parts) => ExpressionKind::InterpolatedString(
            parts
                .into_iter()
                .map(|part| match part {
                    InterpolatedPart::Text(_) => part,
                    InterpolatedPart::Expression {
                        expression,
                        string_conversion_source,
                    } => InterpolatedPart::Expression {
                        expression: map(expression),
                        string_conversion_source,
                    },
                })
                .collect(),
        ),
        ExpressionKind::Array(elements) => {
            ExpressionKind::Array(elements.into_iter().map(&mut map).collect())
        }
        ExpressionKind::Range { start, end, kind } => ExpressionKind::Range {
            start: map(start),
            end: map(end),
            kind,
        },
        ExpressionKind::Struct { structure, fields } => ExpressionKind::Struct {
            structure,
            fields: fields
                .into_iter()
                .map(|(field, value)| (field, map(value)))
                .collect(),
        },
        ExpressionKind::Enum {
            enumeration,
            variant,
            payload,
        } => ExpressionKind::Enum {
            enumeration,
            variant,
            payload: payload.map(&mut map),
        },
        ExpressionKind::Member { receiver, members } => ExpressionKind::Member {
            receiver: map(receiver),
            members,
        },
        ExpressionKind::Index { receiver, index } => ExpressionKind::Index {
            receiver: map(receiver),
            index: map(index),
        },
        ExpressionKind::Unary { op, operand } => ExpressionKind::Unary {
            op,
            operand: map(operand),
        },
        ExpressionKind::Cast { value } => ExpressionKind::Cast { value: map(value) },
        ExpressionKind::Binary { op, left, right } => ExpressionKind::Binary {
            op,
            left: map(left),
            right: map(right),
        },
        ExpressionKind::Call {
            mut target,
            arguments,
        } => {
            if let Some((ResolvedReceiver::Expression { expression, .. }, _)) =
                target.receiver_with_type_mut()
            {
                *expression = map(*expression);
            }
            ExpressionKind::Call {
                target,
                arguments: arguments.into_iter().map(&mut map).collect(),
            }
        }
        ExpressionKind::Invoke { callee, arguments } => ExpressionKind::Invoke {
            callee: match callee {
                crate::semantic::DynamicCallCallee::Expression(callee) => {
                    crate::semantic::DynamicCallCallee::Expression(map(callee))
                }
                callee => callee,
            },
            arguments: arguments.into_iter().map(&mut map).collect(),
        },
        ExpressionKind::Closure {
            closure,
            parameters,
            body,
        } => ExpressionKind::Closure {
            closure,
            parameters,
            body: map(body),
        },
        ExpressionKind::If {
            condition,
            then_expr,
            else_expr,
        } => ExpressionKind::If {
            condition: map(condition),
            then_expr: map(then_expr),
            else_expr: map(else_expr),
        },
        ExpressionKind::Fallback { value, fallback } => ExpressionKind::Fallback {
            value: map(value),
            fallback: map(fallback),
        },
        ExpressionKind::Break(value) => ExpressionKind::Break(value.map(&mut map)),
        ExpressionKind::Return(value) => ExpressionKind::Return(value.map(&mut map)),
        ExpressionKind::Throw { error, target } => ExpressionKind::Throw {
            error: map(error),
            target,
        },
        ExpressionKind::Inspect { label, value } => ExpressionKind::Inspect {
            label,
            value: map(value),
        },
        ExpressionKind::Transparent { value } => ExpressionKind::Transparent { value: map(value) },
        ExpressionKind::Suspend {
            mode,
            destination,
            value,
        } => ExpressionKind::Suspend {
            mode,
            destination,
            value: map(value),
        },
        ExpressionKind::Propagate { value, target } => ExpressionKind::Propagate {
            value: map(value),
            target,
        },
        ExpressionKind::Match { value, arms } => ExpressionKind::Match {
            value: map(value),
            arms: arms
                .into_iter()
                .map(|arm| MatchArm {
                    pattern_id: arm.pattern_id,
                    pattern: arm.pattern,
                    guard: arm.guard.map(&mut map),
                    value: map(arm.value),
                })
                .collect(),
        },
        ExpressionKind::Is {
            value,
            pattern_id,
            pattern,
        } => ExpressionKind::Is {
            value: map(value),
            pattern_id,
            pattern,
        },
    }
}

#[allow(clippy::too_many_arguments)]
fn lower_normalized_match_arms(
    expression: ExprId,
    value: ExprId,
    arms: &[NormalizedMatchArm],
    destination: Option<TemporaryId>,
    continuation: &Block,
    typed_hir: &TypedProgram,
    semantics: &SemanticModel,
    source: SourceProvenance,
    wasm_ir: &mut Program,
) -> Vec<MatchStatementArm> {
    arms.iter()
        .enumerate()
        .map(|(index, arm)| {
            let mut value_tail = continuation.clone();
            value_tail.statements.insert(
                0,
                destination.map_or(
                    Statement::Evaluate {
                        expression: arm.value.value,
                        // With no destination the match itself is in statement
                        // position. Its arm value must therefore be compiled in
                        // the ordinary result-discarding context as well; in
                        // particular, a `None` arm must not leave its erased
                        // unit representation on an empty Wasm branch stack.
                        discard_result: true,
                    },
                    |target| Statement::StoreTemporary {
                        target,
                        value: arm.value.value,
                    },
                ),
            );
            let value_block = wrap_async_expression_steps(
                arm.value.steps.clone(),
                value_tail,
                typed_hir,
                semantics,
                source,
                wasm_ir,
            );
            let (guard, block) = match &arm.guard {
                Some(guard) if !guard.steps.is_empty() => {
                    let remaining = Block {
                        statements: vec![Statement::Match {
                            expression,
                            value,
                            arms: lower_normalized_match_arms(
                                expression,
                                value,
                                &arms[index + 1..],
                                destination,
                                continuation,
                                typed_hir,
                                semantics,
                                source,
                                wasm_ir,
                            ),
                        }],
                        terminator: Terminator::Fallthrough,
                    };
                    let guarded = Block {
                        statements: vec![Statement::If {
                            condition: guard.value,
                            then_block: value_block,
                            else_block: remaining,
                        }],
                        terminator: Terminator::Fallthrough,
                    };
                    (
                        None,
                        wrap_async_expression_steps(
                            guard.steps.clone(),
                            guarded,
                            typed_hir,
                            semantics,
                            source,
                            wasm_ir,
                        ),
                    )
                }
                Some(guard) => (Some(guard.value), value_block),
                None => (None, value_block),
            };
            MatchStatementArm {
                pattern_id: arm.pattern_id,
                pattern: arm.pattern.clone(),
                guard,
                block,
            }
        })
        .collect()
}

fn wrap_async_expression_steps(
    steps: Vec<AsyncExpressionStep>,
    mut continuation: Block,
    typed_hir: &TypedProgram,
    semantics: &SemanticModel,
    source: SourceProvenance,
    wasm_ir: &mut Program,
) -> Block {
    for step in steps.into_iter().rev() {
        match step {
            AsyncExpressionStep::Store { target, value } => continuation
                .statements
                .insert(0, Statement::StoreTemporary { target, value }),
            AsyncExpressionStep::Suspend {
                mode,
                destination,
                value,
                cancellation,
                source,
            } => {
                continuation = Block {
                    statements: Vec::new(),
                    terminator: Terminator::Suspend {
                        mode,
                        destination,
                        value,
                        source,
                        poll_state: AsyncStateId::ENTRY,
                        resume_state: AsyncStateId::ENTRY,
                        cancellation,
                        live_values: Vec::new(),
                        continuation: Box::new(continuation),
                    },
                };
            }
            AsyncExpressionStep::Retry {
                attempt,
                destination,
                cancellation,
                source: retry_source,
            } => {
                let attempt = wrap_async_expression_steps(
                    attempt.steps,
                    Block {
                        statements: Vec::new(),
                        terminator: Terminator::RetryComplete {
                            value: attempt.value,
                            destination,
                            resume_state: AsyncStateId::ENTRY,
                        },
                    },
                    typed_hir,
                    semantics,
                    source,
                    wasm_ir,
                );
                continuation = Block {
                    statements: Vec::new(),
                    terminator: Terminator::Retry {
                        attempt: Box::new(attempt),
                        continuation: Box::new(continuation),
                        source: retry_source,
                        poll_state: AsyncStateId::ENTRY,
                        resume_state: AsyncStateId::ENTRY,
                        cancellation,
                        live_values: Vec::new(),
                    },
                };
            }
            AsyncExpressionStep::If {
                condition,
                then_expression,
                else_expression,
                destination,
            } => {
                let branch_tail = |expression: ExprId, mut continuation: Block| {
                    continuation.statements.insert(
                        0,
                        destination.map_or(
                            Statement::Evaluate {
                                expression,
                                discard_result: false,
                            },
                            |target| Statement::StoreTemporary {
                                target,
                                value: expression,
                            },
                        ),
                    );
                    continuation
                };
                let then_block = wrap_async_expression_steps(
                    then_expression.steps,
                    branch_tail(then_expression.value, continuation.clone()),
                    typed_hir,
                    semantics,
                    source,
                    wasm_ir,
                );
                let else_block = wrap_async_expression_steps(
                    else_expression.steps,
                    branch_tail(else_expression.value, continuation),
                    typed_hir,
                    semantics,
                    source,
                    wasm_ir,
                );
                continuation = Block {
                    statements: vec![Statement::If {
                        condition,
                        then_block,
                        else_block,
                    }],
                    terminator: Terminator::Fallthrough,
                };
            }
            AsyncExpressionStep::Match {
                expression,
                value,
                arms,
                destination,
            } => {
                continuation = Block {
                    statements: vec![Statement::Match {
                        expression,
                        value,
                        arms: lower_normalized_match_arms(
                            expression,
                            value,
                            &arms,
                            destination,
                            &continuation,
                            typed_hir,
                            semantics,
                            source,
                            wasm_ir,
                        ),
                    }],
                    terminator: Terminator::Fallthrough,
                };
            }
            AsyncExpressionStep::Fallback {
                expression,
                value,
                fallback,
                destination,
                success,
            } => {
                let mut fallback_tail = continuation.clone();
                fallback_tail.statements.insert(
                    0,
                    Statement::StoreTemporary {
                        target: destination,
                        value: fallback.value,
                    },
                );
                let fallback_block = wrap_async_expression_steps(
                    fallback.steps,
                    fallback_tail,
                    typed_hir,
                    semantics,
                    source,
                    wasm_ir,
                );
                let mut success_block = continuation;
                success_block.statements.insert(
                    0,
                    Statement::StoreTemporary {
                        target: destination,
                        value: success,
                    },
                );
                continuation = Block {
                    statements: vec![Statement::Fallback {
                        expression,
                        value,
                        fallback_block,
                        success_block,
                    }],
                    terminator: Terminator::Fallthrough,
                };
            }
            AsyncExpressionStep::ValueBlock {
                statements,
                tail,
                destination,
            } => {
                continuation.statements.insert(
                    0,
                    Statement::StoreTemporary {
                        target: destination,
                        value: tail.value,
                    },
                );
                continuation = wrap_async_expression_steps(
                    tail.steps,
                    continuation,
                    typed_hir,
                    semantics,
                    source,
                    wasm_ir,
                );
                continuation = lower_async_statements(
                    &statements.statements,
                    continuation,
                    typed_hir,
                    semantics,
                    source,
                    wasm_ir,
                );
            }
            AsyncExpressionStep::Loop { body, destination } => {
                let condition = wasm_ir.push_generated_expression(
                    semantics
                        .types()
                        .id_for_core(crate::stdlib::CoreTypeId::Bool),
                    ExpressionKind::Bool(true),
                    None,
                    None,
                );
                if typed_block_contains_await(&body, typed_hir, source.profile) {
                    let body = lower_async_statements(
                        &body.statements,
                        Block {
                            statements: Vec::new(),
                            terminator: Terminator::Continue,
                        },
                        typed_hir,
                        semantics,
                        source,
                        wasm_ir,
                    );
                    let header = Block {
                        statements: Vec::new(),
                        terminator: Terminator::AsyncWhileCondition {
                            condition,
                            body: Box::new(body),
                            header_state: AsyncStateId::ENTRY,
                            exit_state: AsyncStateId::ENTRY,
                        },
                    };
                    continuation = Block {
                        statements: Vec::new(),
                        terminator: Terminator::AsyncWhile {
                            header: Box::new(header),
                            continuation: Box::new(continuation),
                            header_state: AsyncStateId::ENTRY,
                            exit_state: AsyncStateId::ENTRY,
                            result: Some(destination),
                        },
                    };
                } else {
                    continuation.statements.insert(
                        0,
                        Statement::While {
                            condition,
                            body: lower_block(&body, typed_hir, semantics, source, wasm_ir),
                            result: Some(destination),
                        },
                    );
                }
            }
        }
    }
    continuation
}

fn lower_async_statements(
    statements: &[hir::TypedStatement],
    tail: Block,
    typed_hir: &TypedProgram,
    semantics: &SemanticModel,
    source: SourceProvenance,
    wasm_ir: &mut Program,
) -> Block {
    enum ControlFlowTerminator {
        Break,
        Continue,
        Return,
        Throw(FailureTarget),
    }

    let mut result = tail;
    for statement in statements.iter().rev() {
        if statement.debug_only && source.profile == crate::BuildProfile::Release {
            continue;
        }
        match &statement.kind {
            TypedStatementKind::Variable {
                binding,
                initializer,
            } => {
                let normalized =
                    normalize_expression_suspensions(*initializer, typed_hir, semantics, wasm_ir);
                if binding_destructures(binding) {
                    result.statements.insert(
                        0,
                        Statement::BindPattern {
                            value: binding.value,
                            pattern: lower_pattern(
                                &binding.pattern.pattern,
                                &binding.pattern.resolution,
                            ),
                        },
                    );
                }
                result.statements.insert(
                    0,
                    Statement::Store {
                        target: binding.value,
                        declaration: true,
                        operation: None,
                        value: normalized.value,
                    },
                );
                result = wrap_async_expression_steps(
                    normalized.steps,
                    result,
                    typed_hir,
                    semantics,
                    source,
                    wasm_ir,
                );
            }
            TypedStatementKind::Assign {
                assignment,
                op,
                value,
            } => {
                let normalized =
                    normalize_expression_suspensions(*value, typed_hir, semantics, wasm_ir);
                result.statements.insert(
                    0,
                    Statement::Store {
                        target: assignment.target,
                        declaration: false,
                        operation: lower_assignment_operation(
                            assignment, *op, typed_hir, semantics,
                        ),
                        value: normalized.value,
                    },
                );
                result = wrap_async_expression_steps(
                    normalized.steps,
                    result,
                    typed_hir,
                    semantics,
                    source,
                    wasm_ir,
                );
            }
            TypedStatementKind::StateAssign {
                assignment,
                op,
                value,
                ..
            } => {
                let normalized =
                    normalize_expression_suspensions(*value, typed_hir, semantics, wasm_ir);
                result.statements.insert(
                    0,
                    Statement::StateStore {
                        target: assignment.target,
                        operation: lower_assignment_operation(
                            assignment, *op, typed_hir, semantics,
                        ),
                        value: normalized.value,
                    },
                );
                result = wrap_async_expression_steps(
                    normalized.steps,
                    result,
                    typed_hir,
                    semantics,
                    source,
                    wasm_ir,
                );
            }
            TypedStatementKind::IndexAssign {
                assignment,
                target,
                value,
                ..
            } => {
                let TypedExpressionKind::Index { receiver, index } = &typed_hir
                    .expression(*target)
                    .expect("indexed assignment target belongs to typed HIR")
                    .kind
                else {
                    unreachable!("checked indexed assignments retain an index target")
                };
                let normalized_receiver =
                    normalize_expression_suspensions(*receiver, typed_hir, semantics, wasm_ir);
                let receiver_type = wasm_ir.effective_expression_type(normalized_receiver.value);
                let (receiver_temporary, receiver_read) = wasm_ir.temporary(receiver_type);

                let normalized_index =
                    normalize_expression_suspensions(*index, typed_hir, semantics, wasm_ir);
                let index_type = wasm_ir.effective_expression_type(normalized_index.value);
                let (index_temporary, index_read) = wasm_ir.temporary(index_type);

                let normalized_value =
                    normalize_expression_suspensions(*value, typed_hir, semantics, wasm_ir);
                let target_type = wasm_ir.effective_expression_type(*target);
                let target_source = wasm_ir.expression(*target).and_then(|target| target.source);
                let lowered_target = if let Some(getter) = semantics.call(*target) {
                    let mut getter = lower_call_target(getter, typed_hir, semantics);
                    replace_call_receiver(
                        &mut getter,
                        ResolvedReceiver::Expression {
                            expression: receiver_read,
                            members: Vec::new(),
                        },
                    );
                    wasm_ir.push_generated_expression(
                        target_type,
                        ExpressionKind::Call {
                            target: getter,
                            arguments: vec![index_read],
                        },
                        None,
                        target_source,
                    )
                } else {
                    wasm_ir.push_generated_expression(
                        target_type,
                        ExpressionKind::Index {
                            receiver: receiver_read,
                            index: index_read,
                        },
                        None,
                        target_source,
                    )
                };

                let operation = lower_index_assignment_operation(
                    assignment,
                    lowered_target,
                    typed_hir,
                    semantics,
                );
                if let Some(setter) = &assignment.setter {
                    let updated = match operation {
                        AssignmentOperation::Call(target) => wasm_ir.push_generated_expression(
                            target_type,
                            ExpressionKind::Call {
                                target,
                                arguments: vec![normalized_value.value],
                            },
                            None,
                            target_source,
                        ),
                        AssignmentOperation::Primitive(op) => wasm_ir.push_generated_expression(
                            target_type,
                            ExpressionKind::Binary {
                                op,
                                left: lowered_target,
                                right: normalized_value.value,
                            },
                            None,
                            target_source,
                        ),
                    };
                    let mut setter = lower_call_target(setter, typed_hir, semantics);
                    replace_call_receiver(
                        &mut setter,
                        ResolvedReceiver::Expression {
                            expression: receiver_read,
                            members: Vec::new(),
                        },
                    );
                    let result_type = assignment
                        .setter
                        .as_ref()
                        .and_then(|call| match call {
                            crate::semantic::ResolvedCall::StandardLibrary {
                                signature, ..
                            }
                            | crate::semantic::ResolvedCall::UserFunction { signature, .. }
                            | crate::semantic::ResolvedCall::UserMethod { signature, .. } => {
                                signature.last().copied()
                            }
                            _ => None,
                        })
                        .expect("Map setter calls have a concrete result type");
                    let setter_call = wasm_ir.push_generated_expression(
                        result_type,
                        ExpressionKind::Call {
                            target: setter,
                            arguments: vec![index_read, updated],
                        },
                        None,
                        target_source,
                    );
                    result.statements.insert(
                        0,
                        Statement::Evaluate {
                            expression: setter_call,
                            discard_result: true,
                        },
                    );
                } else {
                    result.statements.insert(
                        0,
                        Statement::IndexStore {
                            target: lowered_target,
                            operation,
                            value: normalized_value.value,
                        },
                    );
                }
                result = wrap_async_expression_steps(
                    normalized_value.steps,
                    result,
                    typed_hir,
                    semantics,
                    source,
                    wasm_ir,
                );
                result.statements.insert(
                    0,
                    Statement::StoreTemporary {
                        target: index_temporary,
                        value: normalized_index.value,
                    },
                );
                result = wrap_async_expression_steps(
                    normalized_index.steps,
                    result,
                    typed_hir,
                    semantics,
                    source,
                    wasm_ir,
                );
                result.statements.insert(
                    0,
                    Statement::StoreTemporary {
                        target: receiver_temporary,
                        value: normalized_receiver.value,
                    },
                );
                result = wrap_async_expression_steps(
                    normalized_receiver.steps,
                    result,
                    typed_hir,
                    semantics,
                    source,
                    wasm_ir,
                );
            }
            TypedStatementKind::If {
                condition,
                then_block,
                else_block,
            } if typed_block_contains_await(then_block, typed_hir, source.profile)
                || else_block.as_ref().is_some_and(|block| {
                    typed_block_contains_await(block, typed_hir, source.profile)
                }) =>
            {
                let then_block = lower_async_statements(
                    &then_block.statements,
                    result.clone(),
                    typed_hir,
                    semantics,
                    source,
                    wasm_ir,
                );
                let else_block = else_block.as_ref().map_or_else(
                    || result.clone(),
                    |block| {
                        lower_async_statements(
                            &block.statements,
                            result.clone(),
                            typed_hir,
                            semantics,
                            source,
                            wasm_ir,
                        )
                    },
                );
                let normalized =
                    normalize_expression_suspensions(*condition, typed_hir, semantics, wasm_ir);
                result = Block {
                    statements: vec![Statement::If {
                        condition: normalized.value,
                        then_block,
                        else_block,
                    }],
                    terminator: Terminator::Fallthrough,
                };
                result = wrap_async_expression_steps(
                    normalized.steps,
                    result,
                    typed_hir,
                    semantics,
                    source,
                    wasm_ir,
                );
            }
            TypedStatementKind::If {
                condition,
                then_block,
                else_block,
            } => {
                let normalized =
                    normalize_expression_suspensions(*condition, typed_hir, semantics, wasm_ir);
                result.statements.insert(
                    0,
                    Statement::If {
                        condition: normalized.value,
                        then_block: lower_block(then_block, typed_hir, semantics, source, wasm_ir),
                        else_block: else_block.as_ref().map_or_else(Block::default, |block| {
                            lower_block(block, typed_hir, semantics, source, wasm_ir)
                        }),
                    },
                );
                result = wrap_async_expression_steps(
                    normalized.steps,
                    result,
                    typed_hir,
                    semantics,
                    source,
                    wasm_ir,
                );
            }
            TypedStatementKind::While { condition, body } => {
                if typed_expression_contains_suspension(*condition, typed_hir)
                    || typed_block_contains_await(body, typed_hir, source.profile)
                {
                    let normalized =
                        normalize_expression_suspensions(*condition, typed_hir, semantics, wasm_ir);
                    let body = lower_async_statements(
                        &body.statements,
                        Block {
                            statements: Vec::new(),
                            terminator: Terminator::Continue,
                        },
                        typed_hir,
                        semantics,
                        source,
                        wasm_ir,
                    );
                    let header = wrap_async_expression_steps(
                        normalized.steps,
                        Block {
                            statements: Vec::new(),
                            terminator: Terminator::AsyncWhileCondition {
                                condition: normalized.value,
                                body: Box::new(body),
                                header_state: AsyncStateId::ENTRY,
                                exit_state: AsyncStateId::ENTRY,
                            },
                        },
                        typed_hir,
                        semantics,
                        source,
                        wasm_ir,
                    );
                    result = Block {
                        statements: Vec::new(),
                        terminator: Terminator::AsyncWhile {
                            header: Box::new(header),
                            continuation: Box::new(result),
                            header_state: AsyncStateId::ENTRY,
                            exit_state: AsyncStateId::ENTRY,
                            result: None,
                        },
                    };
                    if source.emits_debug_locations() {
                        result
                            .statements
                            .insert(0, Statement::DebugLocation(statement.span));
                    }
                    continue;
                }
                result.statements.insert(
                    0,
                    Statement::While {
                        condition: *condition,
                        body: lower_block(body, typed_hir, semantics, source, wasm_ir),
                        result: None,
                    },
                );
            }
            TypedStatementKind::For {
                binding,
                iterable_value,
                index_value,
                version_value,
                iterable,
                body,
            } => {
                if typed_block_contains_await(body, typed_hir, source.profile) {
                    let normalized =
                        normalize_expression_suspensions(*iterable, typed_hir, semantics, wasm_ir);
                    let iterable = generated_iterable_iterator_call(
                        normalized.value,
                        *iterable_value,
                        *index_value,
                        semantics,
                        wasm_ir,
                    );
                    let iterator_step = generated_iterator_step_call(
                        *iterable_value,
                        *index_value,
                        semantics,
                        wasm_ir,
                    );
                    let body = lower_async_statements(
                        &body.statements,
                        Block {
                            statements: Vec::new(),
                            terminator: Terminator::Continue,
                        },
                        typed_hir,
                        semantics,
                        source,
                        wasm_ir,
                    );
                    result = Block {
                        statements: vec![Statement::ForInit {
                            binding: binding.value,
                            pattern: lower_pattern(
                                &binding.pattern.pattern,
                                &binding.pattern.resolution,
                            ),
                            iterable_value: *iterable_value,
                            index_value: *index_value,
                            version_value: *version_value,
                            iterable,
                            iterator_step,
                        }],
                        terminator: Terminator::AsyncFor {
                            binding: binding.value,
                            pattern: lower_pattern(
                                &binding.pattern.pattern,
                                &binding.pattern.resolution,
                            ),
                            iterable_value: *iterable_value,
                            index_value: *index_value,
                            version_value: *version_value,
                            iterator_step,
                            body: Box::new(body),
                            continuation: Box::new(result),
                            header_state: AsyncStateId::ENTRY,
                            exit_state: AsyncStateId::ENTRY,
                        },
                    };
                    result = wrap_async_expression_steps(
                        normalized.steps,
                        result,
                        typed_hir,
                        semantics,
                        source,
                        wasm_ir,
                    );
                    if source.emits_debug_locations() {
                        result
                            .statements
                            .insert(0, Statement::DebugLocation(statement.span));
                    }
                    continue;
                }
                let normalized =
                    normalize_expression_suspensions(*iterable, typed_hir, semantics, wasm_ir);
                let iterable = generated_iterable_iterator_call(
                    normalized.value,
                    *iterable_value,
                    *index_value,
                    semantics,
                    wasm_ir,
                );
                let iterator_step =
                    generated_iterator_step_call(*iterable_value, *index_value, semantics, wasm_ir);
                result.statements.insert(
                    0,
                    Statement::For {
                        binding: binding.value,
                        pattern: lower_pattern(
                            &binding.pattern.pattern,
                            &binding.pattern.resolution,
                        ),
                        iterable_value: *iterable_value,
                        index_value: *index_value,
                        version_value: *version_value,
                        iterable,
                        iterator_step,
                        body: lower_block(body, typed_hir, semantics, source, wasm_ir),
                    },
                );
                result = wrap_async_expression_steps(
                    normalized.steps,
                    result,
                    typed_hir,
                    semantics,
                    source,
                    wasm_ir,
                );
            }
            TypedStatementKind::Suspend {
                mode,
                binding,
                returns,
                value,
            } => {
                result = Block {
                    statements: Vec::new(),
                    terminator: Terminator::Suspend {
                        mode: *mode,
                        destination: if *returns {
                            SuspensionDestination::BodyResult
                        } else {
                            binding.map_or(
                                SuspensionDestination::Discard,
                                SuspensionDestination::SourceValue,
                            )
                        },
                        value: *value,
                        source: source.emits_debug_locations().then_some(statement.span),
                        poll_state: AsyncStateId::ENTRY,
                        resume_state: AsyncStateId::ENTRY,
                        cancellation: suspension_cancellation(*mode, *value, typed_hir),
                        live_values: Vec::new(),
                        continuation: Box::new(if *returns { Block::default() } else { result }),
                    },
                };
            }
            TypedStatementKind::Yield { value } => {
                result = Block {
                    statements: Vec::new(),
                    terminator: Terminator::Yield {
                        value: *value,
                        resume_state: AsyncStateId::ENTRY,
                        live_values: Vec::new(),
                        continuation: Box::new(result),
                    },
                };
            }
            TypedStatementKind::Expression(expression) => {
                let expression = typed_hir
                    .expression(*expression)
                    .expect("statement expressions belong to typed HIR");
                let control_flow = match &expression.kind {
                    TypedExpressionKind::Break(value) => Some((
                        value.map(|value| {
                            normalize_expression_suspensions(value, typed_hir, semantics, wasm_ir)
                        }),
                        ControlFlowTerminator::Break,
                    )),
                    TypedExpressionKind::Continue => Some((None, ControlFlowTerminator::Continue)),
                    TypedExpressionKind::Return(value) => Some((
                        value.map(|value| {
                            normalize_expression_suspensions(value, typed_hir, semantics, wasm_ir)
                        }),
                        ControlFlowTerminator::Return,
                    )),
                    TypedExpressionKind::Throw { error, target } => Some((
                        Some(normalize_expression_suspensions(
                            *error, typed_hir, semantics, wasm_ir,
                        )),
                        ControlFlowTerminator::Throw(*target),
                    )),
                    _ => None,
                };
                if let Some((normalized, terminator)) = control_flow {
                    let normalized_value = normalized.as_ref().map(|value| value.value);
                    result = Block {
                        statements: Vec::new(),
                        terminator: match terminator {
                            ControlFlowTerminator::Break => Terminator::Break(normalized_value),
                            ControlFlowTerminator::Continue => Terminator::Continue,
                            ControlFlowTerminator::Return => Terminator::Return(normalized_value),
                            ControlFlowTerminator::Throw(target) => Terminator::Throw {
                                error: normalized_value.expect("throw has an error expression"),
                                target,
                            },
                        },
                    };
                    if let Some(normalized) = normalized {
                        result = wrap_async_expression_steps(
                            normalized.steps,
                            result,
                            typed_hir,
                            semantics,
                            source,
                            wasm_ir,
                        );
                    }
                } else {
                    let normalized = normalize_expression_suspensions(
                        expression.id,
                        typed_hir,
                        semantics,
                        wasm_ir,
                    );
                    result.statements.insert(
                        0,
                        Statement::Evaluate {
                            expression: normalized.value,
                            discard_result: true,
                        },
                    );
                    result = wrap_async_expression_steps(
                        normalized.steps,
                        result,
                        typed_hir,
                        semantics,
                        source,
                        wasm_ir,
                    );
                }
            }
        }
        if source.emits_debug_locations() {
            result
                .statements
                .insert(0, Statement::DebugLocation(statement.span));
        }
    }
    result
}

fn typed_block_contains_await(
    block: &hir::TypedBlock,
    typed_hir: &TypedProgram,
    profile: crate::BuildProfile,
) -> bool {
    block.statements.iter().any(|statement| {
        if statement.debug_only && profile == crate::BuildProfile::Release {
            return false;
        }
        match &statement.kind {
            TypedStatementKind::Suspend { .. } | TypedStatementKind::Yield { .. } => true,
            TypedStatementKind::Variable { initializer, .. } => {
                typed_expression_contains_suspension(*initializer, typed_hir)
            }
            TypedStatementKind::Assign { value, .. }
            | TypedStatementKind::StateAssign { value, .. }
            | TypedStatementKind::Expression(value) => {
                typed_expression_contains_suspension(*value, typed_hir)
            }
            TypedStatementKind::If {
                condition,
                then_block,
                else_block,
                ..
            } => {
                typed_expression_contains_suspension(*condition, typed_hir)
                    || typed_block_contains_await(then_block, typed_hir, profile)
                    || else_block
                        .as_ref()
                        .is_some_and(|block| typed_block_contains_await(block, typed_hir, profile))
            }
            TypedStatementKind::While { condition, body } => {
                typed_expression_contains_suspension(*condition, typed_hir)
                    || typed_block_contains_await(body, typed_hir, profile)
            }
            TypedStatementKind::For { iterable, body, .. } => {
                typed_expression_contains_suspension(*iterable, typed_hir)
                    || typed_block_contains_await(body, typed_hir, profile)
            }
            _ => false,
        }
    })
}

fn typed_expression_contains_suspension(expression: ExprId, typed_hir: &TypedProgram) -> bool {
    struct Finder(bool);

    impl hir::TypedVisitor for Finder {
        fn visit_expression(&mut self, expression: &hir::TypedExpression, program: &TypedProgram) {
            if matches!(expression.kind, TypedExpressionKind::Suspend { .. }) {
                self.0 = true;
                return;
            }
            hir::walk_typed_expression(self, expression, program);
        }
    }

    let mut finder = Finder(false);
    let expression = typed_hir
        .expression(expression)
        .expect("checked expression belongs to typed HIR");
    hir::TypedVisitor::visit_expression(&mut finder, expression, typed_hir);
    finder.0
}

fn assign_async_states(block: &mut Block, next: &mut u32) {
    for statement in &mut block.statements {
        match statement {
            Statement::If {
                then_block,
                else_block,
                ..
            } => {
                assign_async_states(then_block, next);
                assign_async_states(else_block, next);
            }
            Statement::Match { arms, .. } => {
                for arm in arms {
                    assign_async_states(&mut arm.block, next);
                }
            }
            Statement::Fallback {
                fallback_block,
                success_block,
                ..
            } => {
                assign_async_states(fallback_block, next);
                assign_async_states(success_block, next);
            }
            Statement::While { body, .. } | Statement::For { body, .. } => {
                assign_async_states(body, next)
            }
            Statement::Store { .. }
            | Statement::BindPattern { .. }
            | Statement::StateStore { .. }
            | Statement::DebugLocation(_)
            | Statement::StoreTemporary { .. }
            | Statement::IndexStore { .. }
            | Statement::Evaluate { .. }
            | Statement::ForInit { .. } => {}
        }
    }
    if let Terminator::Suspend {
        poll_state,
        resume_state,
        continuation,
        ..
    } = &mut block.terminator
    {
        *poll_state = AsyncStateId(*next);
        *next += 1;
        *resume_state = AsyncStateId(*next);
        *next += 1;
        assign_async_states(continuation, next);
    } else if let Terminator::Yield {
        resume_state,
        continuation,
        ..
    } = &mut block.terminator
    {
        *resume_state = AsyncStateId(*next);
        *next += 1;
        assign_async_states(continuation, next);
    } else if let Terminator::Retry {
        attempt,
        continuation,
        poll_state,
        resume_state,
        ..
    } = &mut block.terminator
    {
        *poll_state = AsyncStateId(*next);
        *next += 1;
        *resume_state = AsyncStateId(*next);
        *next += 1;
        set_retry_complete_state(attempt, *resume_state);
        assign_async_states(attempt, next);
        assign_async_states(continuation, next);
    } else if let Terminator::AsyncWhile {
        header,
        continuation,
        header_state,
        exit_state,
        ..
    } = &mut block.terminator
    {
        *header_state = AsyncStateId(*next);
        *next += 1;
        *exit_state = AsyncStateId(*next);
        *next += 1;
        set_async_while_targets(header, *header_state, *exit_state);
        assign_async_states(header, next);
        assign_async_states(continuation, next);
    } else if let Terminator::AsyncWhileCondition { body, .. } = &mut block.terminator {
        assign_async_states(body, next);
    } else if let Terminator::AsyncFor {
        body,
        continuation,
        header_state,
        exit_state,
        ..
    } = &mut block.terminator
    {
        *header_state = AsyncStateId(*next);
        *next += 1;
        *exit_state = AsyncStateId(*next);
        *next += 1;
        assign_async_states(body, next);
        assign_async_states(continuation, next);
    }
}

fn set_retry_complete_state(block: &mut Block, resume_state: AsyncStateId) {
    for statement in &mut block.statements {
        match statement {
            Statement::If {
                then_block,
                else_block,
                ..
            } => {
                set_retry_complete_state(then_block, resume_state);
                set_retry_complete_state(else_block, resume_state);
            }
            Statement::Match { arms, .. } => {
                for arm in arms {
                    set_retry_complete_state(&mut arm.block, resume_state);
                }
            }
            Statement::Fallback {
                fallback_block,
                success_block,
                ..
            } => {
                set_retry_complete_state(fallback_block, resume_state);
                set_retry_complete_state(success_block, resume_state);
            }
            Statement::While { body, .. } | Statement::For { body, .. } => {
                set_retry_complete_state(body, resume_state);
            }
            Statement::Store { .. }
            | Statement::BindPattern { .. }
            | Statement::StateStore { .. }
            | Statement::DebugLocation(_)
            | Statement::StoreTemporary { .. }
            | Statement::IndexStore { .. }
            | Statement::Evaluate { .. }
            | Statement::ForInit { .. } => {}
        }
    }
    match &mut block.terminator {
        Terminator::RetryComplete {
            resume_state: target,
            ..
        } => *target = resume_state,
        Terminator::Retry { .. } => {
            unreachable!("a retry operand cannot contain another retry")
        }
        Terminator::Suspend { .. } => {
            unreachable!("a retry operand cannot contain await")
        }
        Terminator::Yield { .. } => {
            unreachable!("a retry operand cannot contain yield")
        }
        Terminator::AsyncWhile {
            header,
            continuation,
            ..
        } => {
            set_retry_complete_state(header, resume_state);
            set_retry_complete_state(continuation, resume_state);
        }
        Terminator::AsyncWhileCondition { body, .. } | Terminator::AsyncFor { body, .. } => {
            set_retry_complete_state(body, resume_state)
        }
        Terminator::Fallthrough
        | Terminator::Break(_)
        | Terminator::Continue
        | Terminator::Return(_)
        | Terminator::Throw { .. } => {}
    }
}

fn set_async_while_targets(
    block: &mut Block,
    header_state: AsyncStateId,
    exit_state: AsyncStateId,
) {
    for statement in &mut block.statements {
        match statement {
            Statement::If {
                then_block,
                else_block,
                ..
            } => {
                set_async_while_targets(then_block, header_state, exit_state);
                set_async_while_targets(else_block, header_state, exit_state);
            }
            Statement::Match { arms, .. } => {
                for arm in arms {
                    set_async_while_targets(&mut arm.block, header_state, exit_state);
                }
            }
            Statement::Fallback {
                fallback_block,
                success_block,
                ..
            } => {
                set_async_while_targets(fallback_block, header_state, exit_state);
                set_async_while_targets(success_block, header_state, exit_state);
            }
            Statement::While { body, .. } | Statement::For { body, .. } => {
                set_async_while_targets(body, header_state, exit_state);
            }
            Statement::Store { .. }
            | Statement::BindPattern { .. }
            | Statement::StateStore { .. }
            | Statement::DebugLocation(_)
            | Statement::StoreTemporary { .. }
            | Statement::IndexStore { .. }
            | Statement::Evaluate { .. }
            | Statement::ForInit { .. } => {}
        }
    }
    match &mut block.terminator {
        Terminator::AsyncWhileCondition {
            header_state: condition_header,
            exit_state: condition_exit,
            ..
        } => {
            *condition_header = header_state;
            *condition_exit = exit_state;
        }
        Terminator::Suspend { continuation, .. } | Terminator::Yield { continuation, .. } => {
            set_async_while_targets(continuation, header_state, exit_state);
        }
        Terminator::Retry {
            attempt,
            continuation,
            ..
        } => {
            set_async_while_targets(attempt, header_state, exit_state);
            set_async_while_targets(continuation, header_state, exit_state);
        }
        Terminator::Fallthrough
        | Terminator::Break(_)
        | Terminator::Continue
        | Terminator::AsyncWhile { .. }
        | Terminator::AsyncFor { .. }
        | Terminator::Return(_)
        | Terminator::RetryComplete { .. }
        | Terminator::Throw { .. } => {}
    }
}

fn suspension_cancellation(
    mode: SuspensionMode,
    expression: ExprId,
    typed_hir: &TypedProgram,
) -> Option<CancellationRegion> {
    if mode == SuspensionMode::Retry {
        return Some(CancellationRegion::AttachmentLifetime);
    }
    let ResolvedCall::StandardLibrary { item, .. } = typed_hir.call(expression)? else {
        return None;
    };
    (typed_hir
        .standard_library()
        .operation_semantics(*item)
        .cancellation
        == CancellationKind::ProcessClose)
        .then_some(CancellationRegion::AttachmentLifetime)
}

fn plan_block(
    block: &Block,
    program: &Program,
    semantics: &SemanticModel,
    capabilities: &crate::capabilities::CapabilityAnalysis,
) -> Vec<Local> {
    let mut planner = LocalPlanner::new(semantics, capabilities);
    Visitor::visit_block(&mut planner, block, program);
    planner.locals
}

struct LocalPlanner<'a> {
    semantics: &'a SemanticModel,
    capabilities: &'a crate::capabilities::CapabilityAnalysis,
    locals: Vec<Local>,
}

#[derive(Clone, Copy)]
struct IntrinsicScratchSource<'a> {
    expression: ExprId,
    expression_ty: TypeId,
    receiver_ty: Option<TypeId>,
    arguments: &'a [ExprId],
    program: &'a Program,
}

impl<'a> LocalPlanner<'a> {
    fn new(
        semantics: &'a SemanticModel,
        capabilities: &'a crate::capabilities::CapabilityAnalysis,
    ) -> Self {
        Self {
            semantics,
            capabilities,
            locals: Vec::new(),
        }
    }

    fn push(&mut self, ty: TypeId, purpose: LocalPurpose) {
        if self.locals.iter().any(|local| local.purpose == purpose) {
            return;
        }
        let id = LocalId(self.locals.len());
        self.locals.push(Local { id, ty, purpose });
    }

    fn value(&mut self, value: ValueId) {
        let ty = self
            .semantics
            .value_type(value)
            .expect("checked local values have semantic types");
        self.push(ty, LocalPurpose::Value(value));
    }

    fn push_intrinsic_scratch(
        &mut self,
        source: IntrinsicScratchSource<'_>,
        slot_base: u8,
        policy: ScratchPolicy,
    ) {
        let ty = match policy.ty {
            ScratchType::Core(core) => self.semantics.types().id_for_core(core),
            ScratchType::Standard(standard) => self.semantics.types().id_for_standard(standard),
            ScratchType::Expression => source.expression_ty,
            ScratchType::ResultValue => {
                let TypeKind::Result { value, .. } =
                    self.semantics.types().kind(source.expression_ty)
                else {
                    unreachable!("result-value scratch requires a Result expression")
                };
                *value
            }
            ScratchType::AsyncArgumentValue(index) => {
                let argument = source
                    .arguments
                    .get(index as usize)
                    .and_then(|argument| source.program.expression(*argument))
                    .expect("async-argument scratch requires a declared call argument");
                let TypeKind::Async { value, .. } = self.semantics.types().kind(argument.ty) else {
                    unreachable!("async-argument scratch requires an async argument")
                };
                *value
            }
            ScratchType::Receiver => source
                .receiver_ty
                .expect("receiver scratch requires a method-shaped intrinsic"),
        };
        for slot in 0..policy.slots {
            self.push(
                ty,
                LocalPurpose::IntrinsicScratch {
                    expression: source.expression,
                    slot: slot_base + slot,
                },
            );
        }
    }

    fn awaited_value_type(&self, future: TypeId) -> TypeId {
        let TypeKind::Async { value, .. } = self.semantics.types().kind(future) else {
            unreachable!("await scratch planning requires an async expression")
        };
        *value
    }
}

/// Plans the scratch locals required by one compiler-provided async operation's
/// standalone poll function. Unlike a source body, this generated body has no
/// syntax-owned locals of its own.
pub(crate) fn leaf_future_locals(
    expression: ExprId,
    program: &Program,
    semantics: &SemanticModel,
    capabilities: &crate::capabilities::CapabilityAnalysis,
) -> Vec<Local> {
    let lowered = program
        .expression(expression)
        .expect("leaf future expressions belong to Wasm IR");
    let mut planner = LocalPlanner::new(semantics, capabilities);
    if let Some(intrinsic) = resolved_intrinsic(program, expression) {
        let ExpressionKind::Call { arguments, .. } = &lowered.kind else {
            unreachable!("intrinsic expressions are calls")
        };
        let completion = planner.awaited_value_type(lowered.ty);
        let mut slot_base = 0;
        for policy in intrinsic_registry::contract(intrinsic).async_scratch {
            planner.push_intrinsic_scratch(
                IntrinsicScratchSource {
                    expression,
                    expression_ty: completion,
                    receiver_ty: None,
                    arguments,
                    program,
                },
                slot_base,
                *policy,
            );
            slot_base += policy.slots;
        }
    }
    planner.locals
}

impl Visitor for LocalPlanner<'_> {
    fn visit_statement(&mut self, statement: &Statement, program: &Program) {
        match statement {
            Statement::StoreTemporary { target, .. } => {
                let ty = program.temporary_type(*target);
                self.push(ty, LocalPurpose::Temporary(*target));
            }
            Statement::Store {
                target,
                declaration: true,
                ..
            } => self.value(*target),
            Statement::BindPattern { pattern, .. } => {
                pattern.visit_bindings(&mut |binding| {
                    if !program.contains_global(binding) {
                        self.value(binding);
                    }
                });
            }
            Statement::Match {
                expression,
                value,
                arms,
            } => {
                let value_type = program
                    .expression(*value)
                    .expect("match input belongs to Wasm IR")
                    .ty;
                self.push(value_type, LocalPurpose::MatchValue(*expression));
                for arm in arms {
                    arm.pattern
                        .visit_bindings(&mut |binding| self.value(binding));
                }
            }
            Statement::Fallback {
                expression, value, ..
            } => {
                let value_type = program
                    .expression(*value)
                    .expect("fallback input belongs to Wasm IR")
                    .ty;
                self.push(value_type, LocalPurpose::FallbackValue(*expression));
            }
            Statement::While {
                result: Some(result),
                ..
            } => {
                self.push(
                    program.temporary_type(*result),
                    LocalPurpose::Temporary(*result),
                );
            }
            Statement::For {
                binding,
                pattern,
                iterable_value,
                index_value,
                version_value,
                ..
            }
            | Statement::ForInit {
                binding,
                pattern,
                iterable_value,
                index_value,
                version_value,
                ..
            } => {
                self.value(*binding);
                pattern.visit_bindings(&mut |binding| self.value(binding));
                self.value(*iterable_value);
                self.value(*index_value);
                self.value(*version_value);
            }
            Statement::Store { .. }
            | Statement::StateStore { .. }
            | Statement::DebugLocation(_)
            | Statement::IndexStore { .. }
            | Statement::Evaluate { .. }
            | Statement::If { .. }
            | Statement::While { result: None, .. } => {}
        }
        walk_statement(self, statement, program);
    }

    fn visit_terminator(&mut self, terminator: &Terminator, program: &Program) {
        if let Terminator::AsyncWhile {
            result: Some(result),
            ..
        } = terminator
        {
            self.push(
                program.temporary_type(*result),
                LocalPurpose::Temporary(*result),
            );
        }
        if let Terminator::Suspend {
            mode,
            destination,
            value,
            ..
        } = terminator
        {
            if let Some(binding) = destination.source_value() {
                self.value(binding);
            }
            let operand_type = program
                .expression(*value)
                .expect("suspended expression belongs to Wasm IR")
                .ty;
            if let SuspensionDestination::Temporary(temporary) = destination {
                let completion_type = match mode {
                    SuspensionMode::Await => self.awaited_value_type(operand_type),
                    SuspensionMode::Retry => {
                        let TypeKind::Result { value, .. } =
                            self.semantics.types().kind(operand_type)
                        else {
                            unreachable!("retry temporary requires a Result expression")
                        };
                        *value
                    }
                };
                self.push(completion_type, LocalPurpose::Temporary(*temporary));
            }
            if *mode == SuspensionMode::Retry {
                self.push(operand_type, LocalPurpose::SuspensionScratch(*value));
            } else if let Some(intrinsic) = resolved_intrinsic(program, *value) {
                let completion_type = self.awaited_value_type(operand_type);
                let ExpressionKind::Call { arguments, .. } = &program
                    .expression(*value)
                    .expect("intrinsic suspension operands belong to Wasm IR")
                    .kind
                else {
                    unreachable!("intrinsic expressions are calls")
                };
                let mut slot_base = 0;
                for policy in intrinsic_registry::contract(intrinsic).async_scratch {
                    self.push_intrinsic_scratch(
                        IntrinsicScratchSource {
                            expression: *value,
                            expression_ty: completion_type,
                            receiver_ty: None,
                            arguments,
                            program,
                        },
                        slot_base,
                        *policy,
                    );
                    slot_base += policy.slots;
                }
            }
        } else if let Terminator::RetryComplete {
            destination, value, ..
        } = terminator
        {
            if let Some(binding) = destination.source_value() {
                self.value(binding);
            }
            let operand_type = program
                .expression(*value)
                .expect("retried expression belongs to Wasm IR")
                .ty;
            let TypeKind::Result {
                value: completion_type,
                ..
            } = self.semantics.types().kind(operand_type)
            else {
                unreachable!("retry completion requires a Result expression")
            };
            if let SuspensionDestination::Temporary(temporary) = destination {
                self.push(*completion_type, LocalPurpose::Temporary(*temporary));
            }
            self.push(operand_type, LocalPurpose::SuspensionScratch(*value));
        }
        walk_terminator(self, terminator, program);
    }

    fn visit_expression(&mut self, expression: &Expression, program: &Program) {
        if let ExpressionKind::Inspect { value, .. } = &expression.kind {
            self.push(
                program.effective_expression_type(*value),
                LocalPurpose::IntrinsicScratch {
                    expression: expression.id,
                    slot: 0,
                },
            );
        }
        if let ExpressionKind::Invoke { callee, .. } = &expression.kind {
            let callee_type = match callee {
                crate::semantic::DynamicCallCallee::Expression(callee) => {
                    program
                        .expression(*callee)
                        .expect("dynamic callees belong to Wasm IR")
                        .ty
                }
                crate::semantic::DynamicCallCallee::Value(value) => self
                    .semantics
                    .value_type(*value)
                    .expect("dynamic callee values have checked types"),
            };
            self.push(
                callee_type,
                LocalPurpose::IntrinsicScratch {
                    expression: expression.id,
                    slot: 0,
                },
            );
        }
        if let ExpressionKind::Match { value, arms } = &expression.kind {
            self.visit_expression_id(*value, program);
            let value_type = program
                .expression(*value)
                .expect("match input belongs to Wasm IR")
                .ty;
            self.push(value_type, LocalPurpose::MatchValue(expression.id));
            for arm in arms {
                arm.pattern
                    .visit_bindings(&mut |binding| self.value(binding));
                if let Some(guard) = arm.guard {
                    self.visit_expression_id(guard, program);
                }
                self.visit_expression_id(arm.value, program);
            }
            return;
        }

        if let ExpressionKind::Is { value, pattern, .. } = &expression.kind {
            self.visit_expression_id(*value, program);
            let value_type = program
                .expression(*value)
                .expect("`is` input belongs to Wasm IR")
                .ty;
            self.push(value_type, LocalPurpose::MatchValue(expression.id));
            pattern.visit_bindings(&mut |binding| self.value(binding));
            return;
        }

        if let ExpressionKind::Fallback { value, fallback } = &expression.kind {
            self.visit_expression_id(*value, program);
            let value_type = program
                .expression(*value)
                .expect("fallback input belongs to Wasm IR")
                .ty;
            self.push(value_type, LocalPurpose::FallbackValue(expression.id));
            self.visit_expression_id(*fallback, program);
            return;
        }

        if let ExpressionKind::Propagate { value, .. } = expression.kind {
            self.visit_expression_id(value, program);
            let input_type = program
                .expression(value)
                .expect("propagated input belongs to Wasm IR")
                .ty;
            self.push(input_type, LocalPurpose::FallbackValue(expression.id));
            return;
        }

        walk_expression(self, expression, program);
        if let ExpressionKind::Call {
            target: CallTarget::GeneratorNext { receiver_type, .. },
            ..
        } = &expression.kind
        {
            self.push(
                *receiver_type,
                LocalPurpose::IntrinsicScratch {
                    expression: expression.id,
                    slot: 0,
                },
            );
        }
        if let ExpressionKind::Call {
            target:
                CallTarget::CapabilityRequirement {
                    item,
                    receiver_type,
                    ..
                },
            ..
        } = &expression.kind
            && self
                .capabilities
                .resolve_method_requirement(*receiver_type, *item, self.semantics)
                == Some(crate::capabilities::CapabilityMethodImplementation::GeneratorNext)
        {
            self.push(
                *receiver_type,
                LocalPurpose::IntrinsicScratch {
                    expression: expression.id,
                    slot: 0,
                },
            );
        }
        if let ExpressionKind::Call {
            target: CallTarget::ManagedComponent { helper_result, .. },
            ..
        } = &expression.kind
        {
            self.push(
                *helper_result,
                LocalPurpose::IntrinsicScratch {
                    expression: expression.id,
                    slot: 0,
                },
            );
        }
        let intrinsic = match &expression.kind {
            ExpressionKind::Call {
                target:
                    CallTarget::Intrinsic {
                        intrinsic,
                        receiver_type,
                        ..
                    },
                ..
            } => Some((*intrinsic, *receiver_type)),
            ExpressionKind::Call {
                target:
                    CallTarget::CapabilityRequirement {
                        item,
                        receiver_type,
                        ..
                    },
                ..
            } => match self.capabilities.resolve_method_requirement(
                *receiver_type,
                *item,
                self.semantics,
            ) {
                Some(crate::capabilities::CapabilityMethodImplementation::Standard(item)) => {
                    match program.standard_library().item(item).implementation {
                        Implementation::Intrinsic(intrinsic) => {
                            Some((intrinsic, Some(*receiver_type)))
                        }
                        Implementation::CapabilityRequirement
                        | Implementation::LibraryBody { .. }
                        | Implementation::LibraryOverloads { .. } => None,
                    }
                }
                Some(
                    crate::capabilities::CapabilityMethodImplementation::Source(_)
                    | crate::capabilities::CapabilityMethodImplementation::DefaultDisplay
                    | crate::capabilities::CapabilityMethodImplementation::DefaultDebug
                    | crate::capabilities::CapabilityMethodImplementation::GeneratorNext
                    | crate::capabilities::CapabilityMethodImplementation::IteratorIdentity,
                )
                | None => None,
            },
            _ => None,
        };
        if let Some((intrinsic, receiver_ty)) = intrinsic {
            let ExpressionKind::Call { arguments, .. } = &expression.kind else {
                unreachable!("intrinsic expressions are calls")
            };
            let source = IntrinsicScratchSource {
                expression: expression.id,
                expression_ty: expression.ty,
                receiver_ty,
                arguments,
                program,
            };
            let mut slot_base = 0;
            for policy in intrinsic_registry::contract(intrinsic).synchronous_scratch {
                self.push_intrinsic_scratch(source, slot_base, *policy);
                slot_base += policy.slots;
            }
        }
    }
}

fn resolved_intrinsic(program: &Program, expression: ExprId) -> Option<IntrinsicId> {
    let ExpressionKind::Call {
        target: CallTarget::Intrinsic { intrinsic, .. },
        ..
    } = &program.expression(expression)?.kind
    else {
        return None;
    };
    Some(*intrinsic)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn async_function_bodies_use_a_typed_poll_contract() {
        let source = r#"
state "game.exe" {}
fn synchronous() -> i32 { return 42 }
fn loadModule() -> async Module {
    let module = await process.module("game.dll")
    return module
}
onAttach {
    let module = await process.module("game.dll")
    print(module.address)
}
"#;
        let checked = crate::check(crate::lower(crate::parse(source).unwrap())).unwrap();
        let backend = crate::lower_wasm(&checked);
        let function = |name: &str| {
            checked
                .syntax()
                .functions
                .iter()
                .find(|function| function.name == name)
                .unwrap()
                .id
        };

        let synchronous = backend
            .wasm_ir()
            .body(BodyOwner::Function(FunctionInstance::monomorphic(
                function("synchronous"),
            )))
            .unwrap();
        assert_eq!(synchronous.abi, BodyAbi::Direct);

        let asynchronous = backend
            .wasm_ir()
            .body(BodyOwner::Function(FunctionInstance::monomorphic(
                function("loadModule"),
            )))
            .unwrap();
        let BodyAbi::AsyncFunction(abi) = asynchronous.abi else {
            panic!("expected an async source-function ABI")
        };
        let result = abi.completion;
        assert!(matches!(
            checked.semantics().types().kind(result),
            TypeKind::Standard(_)
        ));
        assert_eq!(
            asynchronous.cancellation_region,
            Some(CancellationRegion::AttachmentLifetime)
        );
        assert!(asynchronous.async_state_count > 1);
        assert_eq!(PollStatus::Pending.wasm_value(), 0);
        assert_eq!(PollStatus::Ready.wasm_value(), 1);

        #[derive(Default)]
        struct DebugLocations(Vec<Span>);

        impl Visitor for DebugLocations {
            fn visit_statement(&mut self, statement: &Statement, program: &Program) {
                if let Statement::DebugLocation(span) = statement {
                    self.0.push(*span);
                }
                walk_statement(self, statement, program);
            }
        }

        let expected = checked
            .hir
            .function_body(function("loadModule"))
            .unwrap()
            .statements
            .iter()
            .map(|statement| statement.span)
            .collect::<Vec<_>>();
        let mut debug_locations = DebugLocations::default();
        debug_locations.visit_block(&asynchronous.entry, backend.wasm_ir());
        for span in expected {
            assert!(
                debug_locations.0.contains(&span),
                "async normalization dropped statement location {span:?}"
            );
        }

        let release = crate::lower_wasm_with_options(
            &checked,
            crate::CompilerOptions {
                profile: crate::BuildProfile::Release,
                ..crate::CompilerOptions::default()
            },
        );
        let release_body = release
            .wasm_ir()
            .body(BodyOwner::Function(FunctionInstance::monomorphic(
                function("loadModule"),
            )))
            .unwrap();
        let mut release_locations = DebugLocations::default();
        release_locations.visit_block(&release_body.entry, release.wasm_ir());
        assert!(release_locations.0.is_empty());
    }
}
