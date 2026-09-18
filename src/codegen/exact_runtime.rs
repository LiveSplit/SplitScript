//! Exact runtime representations retained behind erased source types.
//!
//! `iterator T`, `async T`, and callable types deliberately remain erased in
//! the language. This backend-only analysis records when one expression or
//! local nevertheless has exactly one concrete implementation. Emitters may
//! then call that implementation directly; a conflicting definition,
//! heterogeneous aggregate, or unknown invocation monotonically degrades the
//! fact to dynamic dispatch.

use std::collections::HashMap;

use crate::{
    ast::{ExprId, Program, ValueId},
    semantic::{
        ClosureInstance, DynamicCallCallee, FunctionInstance, FunctionValueInstance,
        ResolvedMember, ResolvedReceiver, ResolvedValue, SemanticModel,
    },
    types::{TypeId, TypeKind},
    wasm_ir::{self, BodyAbi, BodyOwner, LoweredPattern, TemporaryId},
};

use super::{
    async_frame::{AsyncFrameLayouts, LeafFutureInstance},
    reachability::Reachability,
};

const MAX_FIXPOINT_ROUNDS: usize = 64;

/// One concrete implementation of an erased callable value.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub(super) enum CallableProducer {
    Closure(ClosureInstance),
    Function(FunctionValueInstance),
}

/// One concrete implementation of an erased resumable value.
///
/// Functions and closures cover both generators and async state machines.
/// Leaf futures are compiler-generated frames for async intrinsics.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub(super) enum ContinuationProducer {
    Function(FunctionInstance),
    Closure(ClosureInstance),
    Leaf(LeafFutureInstance),
}

#[derive(Debug, Clone, PartialEq, Eq, Hash)]
enum ExactRepresentation {
    Callable(CallableProducer),
    Continuation(ContinuationProducer),
}

/// Monotone fact used while solving recursive forwarding relationships.
#[derive(Debug, Clone, PartialEq, Eq)]
enum Fact {
    /// No producer has reached this definition yet.
    Bottom,
    Exact(ExactRepresentation),
    /// More than one producer, an aggregate projection, or an unknown escape.
    Dynamic,
}

impl Fact {
    /// Joins independent definitions of one storage location.
    fn join_definition(&self, incoming: Self) -> Self {
        match (self, incoming) {
            (Self::Bottom, value) => value,
            (value, Self::Bottom) => value.clone(),
            (Self::Exact(left), Self::Exact(right)) if left == &right => self.clone(),
            (Self::Dynamic, _) | (_, Self::Dynamic) | (Self::Exact(_), Self::Exact(_)) => {
                Self::Dynamic
            }
        }
    }

    /// Joins mutually exclusive result paths. A not-yet-solved path delays the
    /// result instead of making an optimistic exact claim.
    fn join_alternatives(values: impl IntoIterator<Item = Self>) -> Self {
        let mut result: Option<Self> = None;
        for value in values {
            if value == Self::Bottom {
                return Self::Bottom;
            }
            result = Some(match result {
                None => value,
                Some(current) => current.join_definition(value),
            });
        }
        result.unwrap_or(Self::Dynamic)
    }

    fn continuation(&self) -> Option<&ContinuationProducer> {
        match self {
            Self::Exact(ExactRepresentation::Continuation(producer)) => Some(producer),
            Self::Bottom | Self::Exact(ExactRepresentation::Callable(_)) | Self::Dynamic => None,
        }
    }

    fn callable(&self) -> Option<&CallableProducer> {
        match self {
            Self::Exact(ExactRepresentation::Callable(producer)) => Some(producer),
            Self::Bottom | Self::Exact(ExactRepresentation::Continuation(_)) | Self::Dynamic => {
                None
            }
        }
    }
}

type Owner = Option<FunctionInstance>;

/// Proven exact implementations at the few dynamic-dispatch boundaries.
#[derive(Debug, Default)]
pub(super) struct ExactRuntimeRepresentations {
    expressions: HashMap<(Owner, ExprId), ExactRepresentation>,
    call_receivers: HashMap<(Owner, ExprId), ContinuationProducer>,
    invocations: HashMap<(Owner, ExprId), CallableProducer>,
}

impl ExactRuntimeRepresentations {
    pub(super) fn analyze(
        syntax: &Program,
        program: &wasm_ir::Program,
        semantics: &SemanticModel,
        reachability: &Reachability,
        async_frames: &AsyncFrameLayouts,
    ) -> Self {
        let mut builder = Builder {
            syntax,
            program,
            semantics,
            reachability,
            async_frames,
            expression_facts: HashMap::new(),
            value_facts: HashMap::new(),
            temporary_facts: HashMap::new(),
            receiver_facts: HashMap::new(),
            invocation_facts: HashMap::new(),
            function_returns: HashMap::new(),
            closure_returns: HashMap::new(),
            changed: false,
        };
        builder.solve();
        builder.finish()
    }

    pub(super) fn expression_continuation(
        &self,
        owner: Option<&FunctionInstance>,
        expression: ExprId,
    ) -> Option<&ContinuationProducer> {
        match self.expressions.get(&(owner.cloned(), expression)) {
            Some(ExactRepresentation::Continuation(producer)) => Some(producer),
            Some(ExactRepresentation::Callable(_)) | None => None,
        }
    }

    pub(super) fn call_receiver(
        &self,
        owner: Option<&FunctionInstance>,
        expression: ExprId,
    ) -> Option<&ContinuationProducer> {
        self.call_receivers.get(&(owner.cloned(), expression))
    }

    pub(super) fn invocation(
        &self,
        owner: Option<&FunctionInstance>,
        expression: ExprId,
    ) -> Option<&CallableProducer> {
        self.invocations.get(&(owner.cloned(), expression))
    }
}

struct Builder<'a> {
    syntax: &'a Program,
    program: &'a wasm_ir::Program,
    semantics: &'a SemanticModel,
    reachability: &'a Reachability,
    async_frames: &'a AsyncFrameLayouts,
    expression_facts: HashMap<(Owner, ExprId), Fact>,
    value_facts: HashMap<(Owner, ValueId), Fact>,
    temporary_facts: HashMap<(Owner, TemporaryId), Fact>,
    receiver_facts: HashMap<(Owner, ExprId), Fact>,
    invocation_facts: HashMap<(Owner, ExprId), Fact>,
    function_returns: HashMap<FunctionInstance, Fact>,
    closure_returns: HashMap<ClosureInstance, Fact>,
    changed: bool,
}

impl Builder<'_> {
    fn solve(&mut self) {
        for _ in 0..MAX_FIXPOINT_ROUNDS {
            self.changed = false;

            // Statements define locals and temporaries; returns define the
            // reusable summaries that preserve exact producers through normal
            // forwarding functions without specializing bodies per call site.
            let functions = self.reachability.functions().cloned().collect::<Vec<_>>();
            for instance in functions {
                let body = self
                    .program
                    .body(BodyOwner::Function(instance.clone()))
                    .expect("reachable functions have Wasm IR bodies");
                let mut returns = Vec::new();
                self.block(Some(&instance), &body.entry, &mut returns);
                if matches!(body.abi, BodyAbi::Direct) {
                    let returned = Fact::join_alternatives(returns);
                    self.merge_function_return(instance, returned);
                }
            }

            let closures = self
                .reachability
                .closure_instances()
                .cloned()
                .collect::<Vec<_>>();
            for instance in closures {
                let body = self
                    .program
                    .closure(instance.expression)
                    .expect("reachable closures have Wasm IR bodies");
                let mut returns = Vec::new();
                self.block(instance.owner.as_ref(), &body.entry, &mut returns);
                if matches!(body.abi, BodyAbi::Direct) {
                    let returned = Fact::join_alternatives(returns);
                    self.merge_closure_return(instance, returned);
                }
            }

            let action_blocks = self
                .program
                .bodies()
                .filter_map(|body| {
                    matches!(body.owner, BodyOwner::Action(_)).then_some(&body.entry)
                })
                .collect::<Vec<_>>();
            for block in action_blocks {
                self.block(None, block, &mut Vec::new());
            }
            let initializer_blocks = self
                .program
                .global_initializer_plans()
                .map(|plan| &plan.entry)
                .collect::<Vec<_>>();
            for block in initializer_blocks {
                self.block(None, block, &mut Vec::new());
            }
            let state_blocks = self
                .program
                .state_expressions()
                .map(|plan| &plan.entry)
                .chain(self.program.state_transforms().map(|plan| &plan.entry))
                .collect::<Vec<_>>();
            for block in state_blocks {
                self.block(None, block, &mut Vec::new());
            }

            if !self.changed {
                break;
            }
        }
    }

    fn finish(self) -> ExactRuntimeRepresentations {
        ExactRuntimeRepresentations {
            expressions: self
                .expression_facts
                .into_iter()
                .filter_map(|(key, fact)| match fact {
                    Fact::Exact(producer) => Some((key, producer)),
                    Fact::Bottom | Fact::Dynamic => None,
                })
                .collect(),
            call_receivers: self
                .receiver_facts
                .into_iter()
                .filter_map(|(key, fact)| fact.continuation().cloned().map(|value| (key, value)))
                .collect(),
            invocations: self
                .invocation_facts
                .into_iter()
                .filter_map(|(key, fact)| fact.callable().cloned().map(|value| (key, value)))
                .collect(),
        }
    }

    fn expression(&mut self, owner: Option<&FunctionInstance>, id: ExprId) -> Fact {
        let expression = self
            .program
            .expression(id)
            .expect("reachable expressions belong to Wasm IR");
        let fact = match &expression.kind {
            wasm_ir::ExpressionKind::Closure { closure, .. } => {
                Fact::Exact(ExactRepresentation::Callable(CallableProducer::Closure(
                    ClosureInstance::new(owner.cloned(), *closure),
                )))
            }
            wasm_ir::ExpressionKind::FunctionValue { function } => {
                let function = self.called_instance(owner, function);
                let ty = self.specialize(owner, expression.ty);
                Fact::Exact(ExactRepresentation::Callable(CallableProducer::Function(
                    FunctionValueInstance { function, ty },
                )))
            }
            wasm_ir::ExpressionKind::Path { root, members } => {
                self.path_fact(owner, *root, members, expression.ty)
            }
            wasm_ir::ExpressionKind::Member { receiver, members } => {
                self.expression(owner, *receiver);
                if members.is_empty() {
                    self.expression_fact(owner, *receiver)
                } else {
                    self.dynamic_for(owner, expression.ty)
                }
            }
            wasm_ir::ExpressionKind::Temporary(temporary) => self
                .temporary_facts
                .get(&(owner.cloned(), *temporary))
                .cloned()
                .unwrap_or(Fact::Bottom),
            wasm_ir::ExpressionKind::Cast { value } => {
                let source = self.expression(owner, *value);
                let source_ty = self
                    .program
                    .expression(*value)
                    .expect("cast operands belong to Wasm IR")
                    .ty;
                if self.same_erased_family(owner, source_ty, expression.ty) {
                    source
                } else {
                    self.dynamic_for(owner, expression.ty)
                }
            }
            wasm_ir::ExpressionKind::If {
                condition,
                then_expr,
                else_expr,
            } => {
                self.expression(owner, *condition);
                Fact::join_alternatives([
                    self.expression(owner, *then_expr),
                    self.expression(owner, *else_expr),
                ])
            }
            wasm_ir::ExpressionKind::Match { value, arms } => {
                self.expression(owner, *value);
                Fact::join_alternatives(
                    arms.iter()
                        .map(|arm| {
                            if let Some(guard) = arm.guard {
                                self.expression(owner, guard);
                            }
                            self.expression(owner, arm.value)
                        })
                        .collect::<Vec<_>>(),
                )
            }
            wasm_ir::ExpressionKind::Call { target, arguments } => {
                self.call(owner, id, expression.ty, target, arguments)
            }
            wasm_ir::ExpressionKind::Invoke { callee, arguments } => {
                self.invoke(owner, id, expression.ty, callee, arguments)
            }
            wasm_ir::ExpressionKind::Unary { operand, .. } => {
                self.expression(owner, *operand);
                self.dynamic_for(owner, expression.ty)
            }
            wasm_ir::ExpressionKind::Fallback { value, fallback } => {
                self.expression(owner, *value);
                self.expression(owner, *fallback);
                self.dynamic_for(owner, expression.ty)
            }
            kind => {
                wasm_ir::visit_expression_children(kind, |child| {
                    self.expression(owner, child);
                });
                self.dynamic_for(owner, expression.ty)
            }
        };
        let fact = if expression.conversion.is_some() {
            self.dynamic_for(owner, expression.ty)
        } else {
            fact
        };
        self.merge_expression(owner, id, fact)
    }

    fn call(
        &mut self,
        owner: Option<&FunctionInstance>,
        expression: ExprId,
        result_type: TypeId,
        original: &wasm_ir::CallTarget,
        arguments: &[ExprId],
    ) -> Fact {
        let capability_call = matches!(original, wasm_ir::CallTarget::CapabilityRequirement { .. });
        let target = self
            .reachability
            .resolved_call_target(owner, expression, original);
        let receiver = target
            .receiver_with_type()
            .map(|(receiver, receiver_type)| self.receiver(owner, receiver, receiver_type));
        if let Some(receiver) = receiver.clone() {
            self.merge_receiver(owner, expression, receiver);
        }
        let argument_facts = arguments
            .iter()
            .map(|argument| self.expression(owner, *argument))
            .collect::<Vec<_>>();

        if matches!(target, wasm_ir::CallTarget::IteratorIdentity { .. }) {
            return receiver.unwrap_or(Fact::Dynamic);
        }

        let called = match target {
            wasm_ir::CallTarget::UserFunction { function }
            | wasm_ir::CallTarget::UserMethod { function, .. } => Some(if capability_call {
                function.clone()
            } else {
                self.called_instance(owner, function)
            }),
            wasm_ir::CallTarget::LibraryOverload { .. } => wasm_ir::resolve_library_overload(
                target,
                owner,
                self.semantics,
                self.program.standard_library(),
            ),
            _ => None,
        };
        if let Some(called) = called {
            let inputs = receiver
                .into_iter()
                .chain(argument_facts)
                .collect::<Vec<_>>();
            self.feed_function(&called, &inputs);
            return self.function_call_result(&called, result_type, owner);
        }

        let result = self.specialize(owner, result_type);
        if matches!(self.semantics.types().kind(result), TypeKind::Async { .. })
            && matches!(
                target,
                wasm_ir::CallTarget::Intrinsic { .. }
                    | wasm_ir::CallTarget::ManagedInstances { .. }
            )
        {
            let leaf = LeafFutureInstance {
                owner: owner.cloned(),
                expression,
            };
            if self.async_frames.leaf(&leaf).is_some() {
                return Fact::Exact(ExactRepresentation::Continuation(
                    ContinuationProducer::Leaf(leaf),
                ));
            }
        }
        self.dynamic_for(owner, result_type)
    }

    fn invoke(
        &mut self,
        owner: Option<&FunctionInstance>,
        expression: ExprId,
        result_type: TypeId,
        callee: &DynamicCallCallee,
        arguments: &[ExprId],
    ) -> Fact {
        let callee_fact = match callee {
            DynamicCallCallee::Expression(callee) => self.expression(owner, *callee),
            DynamicCallCallee::Value(value) => self
                .value_facts
                .get(&(owner.cloned(), *value))
                .cloned()
                .unwrap_or(Fact::Bottom),
        };
        self.merge_invocation(owner, expression, callee_fact.clone());
        let arguments = arguments
            .iter()
            .map(|argument| self.expression(owner, *argument))
            .collect::<Vec<_>>();
        match callee_fact {
            Fact::Exact(ExactRepresentation::Callable(CallableProducer::Closure(instance))) => {
                self.feed_closure(&instance, &arguments);
                self.closure_call_result(&instance, result_type, owner)
            }
            Fact::Exact(ExactRepresentation::Callable(CallableProducer::Function(instance))) => {
                self.feed_function(&instance.function, &arguments);
                self.function_call_result(&instance.function, result_type, owner)
            }
            Fact::Dynamic => {
                let callee_type = match callee {
                    DynamicCallCallee::Expression(callee) => {
                        self.program
                            .expression(*callee)
                            .expect("dynamic callees belong to Wasm IR")
                            .ty
                    }
                    DynamicCallCallee::Value(value) => self
                        .semantics
                        .value_type(*value)
                        .expect("checked callable values have types"),
                };
                self.poison_callable_inputs(owner, callee_type);
                self.dynamic_for(owner, result_type)
            }
            Fact::Bottom | Fact::Exact(ExactRepresentation::Continuation(_)) => Fact::Bottom,
        }
    }

    fn function_call_result(
        &self,
        function: &FunctionInstance,
        result_type: TypeId,
        owner: Option<&FunctionInstance>,
    ) -> Fact {
        let body = self
            .program
            .body(BodyOwner::Function(function.clone()))
            .expect("reachable calls have Wasm IR bodies");
        match body.abi {
            BodyAbi::Generator(_) | BodyAbi::AsyncFunction(_) => Fact::Exact(
                ExactRepresentation::Continuation(ContinuationProducer::Function(function.clone())),
            ),
            BodyAbi::Direct => self
                .function_returns
                .get(function)
                .cloned()
                .unwrap_or(Fact::Bottom),
            BodyAbi::AsyncAction => self.dynamic_for(owner, result_type),
        }
    }

    fn closure_call_result(
        &self,
        closure: &ClosureInstance,
        result_type: TypeId,
        owner: Option<&FunctionInstance>,
    ) -> Fact {
        let body = self
            .program
            .closure(closure.expression)
            .expect("reachable closure calls have Wasm IR bodies");
        match body.abi {
            BodyAbi::Generator(_) | BodyAbi::AsyncFunction(_) => Fact::Exact(
                ExactRepresentation::Continuation(ContinuationProducer::Closure(closure.clone())),
            ),
            BodyAbi::Direct => self
                .closure_returns
                .get(closure)
                .cloned()
                .unwrap_or(Fact::Bottom),
            BodyAbi::AsyncAction => self.dynamic_for(owner, result_type),
        }
    }

    fn feed_function(&mut self, function: &FunctionInstance, inputs: &[Fact]) {
        let declaration = self
            .syntax
            .functions
            .iter()
            .find(|candidate| candidate.id == function.function)
            .expect("function instances have declarations");
        for (parameter, input) in declaration.params.iter().zip(inputs) {
            self.merge_value(Some(function), parameter.id, input.clone());
        }
    }

    fn feed_closure(&mut self, closure: &ClosureInstance, inputs: &[Fact]) {
        let parameters = self
            .program
            .closure(closure.expression)
            .expect("closure instances have bodies")
            .parameters
            .clone();
        for (parameter, input) in parameters.into_iter().zip(inputs) {
            self.merge_value(closure.owner.as_ref(), parameter, input.clone());
        }
    }

    fn poison_callable_inputs(&mut self, owner: Option<&FunctionInstance>, ty: TypeId) {
        let ty = self.specialize(owner, ty);
        let closures = self
            .reachability
            .closure_instances()
            .filter(|instance| {
                let candidate = self
                    .program
                    .expression(instance.expression)
                    .expect("reachable closures have expressions")
                    .ty;
                self.specialize(instance.owner.as_ref(), candidate) == ty
            })
            .cloned()
            .collect::<Vec<_>>();
        for closure in closures {
            let count = self
                .program
                .closure(closure.expression)
                .expect("reachable closures have bodies")
                .parameters
                .len();
            self.feed_closure(&closure, &vec![Fact::Dynamic; count]);
        }
        let functions = self
            .reachability
            .function_value_instances()
            .filter(|instance| instance.ty == ty)
            .cloned()
            .collect::<Vec<_>>();
        for function in functions {
            let count = self
                .syntax
                .functions
                .iter()
                .find(|candidate| candidate.id == function.function.function)
                .expect("function values have declarations")
                .params
                .len();
            self.feed_function(&function.function, &vec![Fact::Dynamic; count]);
        }
    }

    fn receiver(
        &mut self,
        owner: Option<&FunctionInstance>,
        receiver: &ResolvedReceiver,
        receiver_type: TypeId,
    ) -> Fact {
        match receiver {
            ResolvedReceiver::Path { root, members } => {
                self.path_fact(owner, Some(*root), members, receiver_type)
            }
            ResolvedReceiver::Expression {
                expression,
                members,
            } if members.is_empty() => self.expression(owner, *expression),
            ResolvedReceiver::Expression { expression, .. } => {
                self.expression(owner, *expression);
                self.dynamic_for(owner, receiver_type)
            }
        }
    }

    fn path_fact(
        &self,
        owner: Option<&FunctionInstance>,
        root: Option<ResolvedValue>,
        members: &[ResolvedMember],
        result_type: TypeId,
    ) -> Fact {
        if !members.is_empty() {
            return self.dynamic_for(owner, result_type);
        }
        let Some(value) = root.and_then(ResolvedValue::source_value) else {
            return self.dynamic_for(owner, result_type);
        };
        self.value_facts
            .get(&(owner.cloned(), value))
            .cloned()
            .unwrap_or(Fact::Bottom)
    }

    fn block(
        &mut self,
        owner: Option<&FunctionInstance>,
        block: &wasm_ir::Block,
        returns: &mut Vec<Fact>,
    ) {
        for statement in &block.statements {
            match statement {
                wasm_ir::Statement::DebugLocation(_) => {}
                wasm_ir::Statement::Store {
                    target,
                    operation,
                    value,
                    ..
                } => {
                    let fact = self.expression(owner, *value);
                    self.merge_value(
                        owner,
                        *target,
                        if operation.is_some() {
                            Fact::Dynamic
                        } else {
                            fact
                        },
                    );
                }
                wasm_ir::Statement::BindPattern { value, pattern } => {
                    let source = self
                        .value_facts
                        .get(&(owner.cloned(), *value))
                        .cloned()
                        .unwrap_or(Fact::Bottom);
                    self.bind_pattern(owner, pattern, source);
                }
                wasm_ir::Statement::StateStore { value, .. } => {
                    self.expression(owner, *value);
                }
                wasm_ir::Statement::StoreTemporary { target, value } => {
                    let fact = self.expression(owner, *value);
                    self.merge_temporary(owner, *target, fact);
                }
                wasm_ir::Statement::IndexStore { target, value, .. } => {
                    self.expression(owner, *target);
                    self.expression(owner, *value);
                }
                wasm_ir::Statement::Evaluate { expression, .. } => {
                    self.expression(owner, *expression);
                }
                wasm_ir::Statement::If {
                    condition,
                    then_block,
                    else_block,
                } => {
                    self.expression(owner, *condition);
                    self.block(owner, then_block, returns);
                    self.block(owner, else_block, returns);
                }
                wasm_ir::Statement::Match { value, arms, .. } => {
                    self.expression(owner, *value);
                    for arm in arms {
                        if let Some(guard) = arm.guard {
                            self.expression(owner, guard);
                        }
                        self.block(owner, &arm.block, returns);
                    }
                }
                wasm_ir::Statement::Fallback {
                    value,
                    fallback_block,
                    success_block,
                    ..
                } => {
                    self.expression(owner, *value);
                    self.block(owner, fallback_block, returns);
                    self.block(owner, success_block, returns);
                }
                wasm_ir::Statement::While {
                    condition, body, ..
                } => {
                    self.expression(owner, *condition);
                    self.block(owner, body, returns);
                }
                wasm_ir::Statement::For {
                    binding,
                    pattern,
                    iterable_value,
                    iterable,
                    iterator_step,
                    body,
                    ..
                } => {
                    let iterable = self.expression(owner, *iterable);
                    self.merge_value(owner, *iterable_value, iterable);
                    self.bind_pattern(owner, pattern, Fact::Dynamic);
                    self.merge_value(owner, *binding, Fact::Dynamic);
                    if let Some(step) = iterator_step {
                        self.expression(owner, *step);
                    }
                    self.block(owner, body, returns);
                }
                wasm_ir::Statement::ForInit {
                    binding,
                    pattern,
                    iterable_value,
                    iterable,
                    iterator_step,
                    ..
                } => {
                    let iterable = self.expression(owner, *iterable);
                    self.merge_value(owner, *iterable_value, iterable);
                    self.bind_pattern(owner, pattern, Fact::Dynamic);
                    self.merge_value(owner, *binding, Fact::Dynamic);
                    if let Some(step) = iterator_step {
                        self.expression(owner, *step);
                    }
                }
            }
        }
        match &block.terminator {
            wasm_ir::Terminator::Fallthrough | wasm_ir::Terminator::Continue => {}
            wasm_ir::Terminator::Break(value) => {
                if let Some(value) = value {
                    self.expression(owner, *value);
                }
            }
            wasm_ir::Terminator::Return(value) => {
                returns.push(value.map_or(Fact::Dynamic, |value| self.expression(owner, value)))
            }
            wasm_ir::Terminator::Throw { error, .. } => {
                self.expression(owner, *error);
            }
            wasm_ir::Terminator::AsyncWhile {
                header,
                continuation,
                ..
            } => {
                self.block(owner, header, returns);
                self.block(owner, continuation, returns);
            }
            wasm_ir::Terminator::AsyncWhileCondition {
                condition, body, ..
            } => {
                self.expression(owner, *condition);
                self.block(owner, body, returns);
            }
            wasm_ir::Terminator::AsyncFor {
                binding,
                pattern,
                iterable_value,
                iterator_step,
                body,
                continuation,
                ..
            } => {
                self.bind_pattern(owner, pattern, Fact::Dynamic);
                self.merge_value(owner, *binding, Fact::Dynamic);
                // The async-for initialization block defines the cursor. Keep
                // an existing exact fact rather than replacing it here.
                if !self
                    .value_facts
                    .contains_key(&(owner.cloned(), *iterable_value))
                {
                    self.merge_value(owner, *iterable_value, Fact::Bottom);
                }
                if let Some(step) = iterator_step {
                    self.expression(owner, *step);
                }
                self.block(owner, body, returns);
                self.block(owner, continuation, returns);
            }
            wasm_ir::Terminator::Retry {
                attempt,
                continuation,
                ..
            } => {
                self.block(owner, attempt, returns);
                self.block(owner, continuation, returns);
            }
            wasm_ir::Terminator::RetryComplete { value, .. } => {
                self.expression(owner, *value);
            }
            wasm_ir::Terminator::Suspend {
                destination,
                value,
                continuation,
                ..
            } => {
                self.expression(owner, *value);
                if let Some(destination) = destination.source_value() {
                    self.merge_value(owner, destination, Fact::Dynamic);
                }
                self.block(owner, continuation, returns);
            }
            wasm_ir::Terminator::Yield {
                value,
                continuation,
                ..
            } => {
                self.expression(owner, *value);
                self.block(owner, continuation, returns);
            }
        }
    }

    fn bind_pattern(
        &mut self,
        owner: Option<&FunctionInstance>,
        pattern: &LoweredPattern,
        source: Fact,
    ) {
        if let LoweredPattern::Binding(value) = pattern {
            self.merge_value(owner, *value, source);
            return;
        }
        pattern.visit_bindings(&mut |value| self.merge_value(owner, value, Fact::Dynamic));
    }

    fn same_erased_family(
        &self,
        owner: Option<&FunctionInstance>,
        left: TypeId,
        right: TypeId,
    ) -> bool {
        matches!(
            (
                self.semantics.types().kind(self.specialize(owner, left)),
                self.semantics.types().kind(self.specialize(owner, right)),
            ),
            (TypeKind::Callable { .. }, TypeKind::Callable { .. })
                | (TypeKind::Iterator { .. }, TypeKind::Iterator { .. })
                | (TypeKind::Async { .. }, TypeKind::Async { .. })
        )
    }

    fn dynamic_for(&self, owner: Option<&FunctionInstance>, ty: TypeId) -> Fact {
        match self.semantics.types().kind(self.specialize(owner, ty)) {
            TypeKind::Callable { .. } | TypeKind::Iterator { .. } | TypeKind::Async { .. } => {
                Fact::Dynamic
            }
            _ => Fact::Bottom,
        }
    }

    fn specialize(&self, owner: Option<&FunctionInstance>, ty: TypeId) -> TypeId {
        owner.map_or(ty, |owner| self.semantics.specialize_type(owner, ty))
    }

    fn called_instance(
        &self,
        owner: Option<&FunctionInstance>,
        function: &FunctionInstance,
    ) -> FunctionInstance {
        owner.map_or_else(
            || function.clone(),
            |owner| self.semantics.specialize_function_instance(owner, function),
        )
    }

    fn expression_fact(&self, owner: Option<&FunctionInstance>, expression: ExprId) -> Fact {
        self.expression_facts
            .get(&(owner.cloned(), expression))
            .cloned()
            .unwrap_or(Fact::Bottom)
    }

    fn merge_expression(
        &mut self,
        owner: Option<&FunctionInstance>,
        expression: ExprId,
        fact: Fact,
    ) -> Fact {
        let key = (owner.cloned(), expression);
        self.changed |= merge_fact(&mut self.expression_facts, key.clone(), fact);
        self.expression_facts
            .get(&key)
            .cloned()
            .unwrap_or(Fact::Bottom)
    }

    fn merge_value(&mut self, owner: Option<&FunctionInstance>, value: ValueId, fact: Fact) {
        self.changed |= merge_fact(&mut self.value_facts, (owner.cloned(), value), fact);
    }

    fn merge_temporary(
        &mut self,
        owner: Option<&FunctionInstance>,
        temporary: TemporaryId,
        fact: Fact,
    ) {
        self.changed |= merge_fact(&mut self.temporary_facts, (owner.cloned(), temporary), fact);
    }

    fn merge_receiver(&mut self, owner: Option<&FunctionInstance>, expression: ExprId, fact: Fact) {
        self.changed |= merge_fact(&mut self.receiver_facts, (owner.cloned(), expression), fact);
    }

    fn merge_invocation(
        &mut self,
        owner: Option<&FunctionInstance>,
        expression: ExprId,
        fact: Fact,
    ) {
        self.changed |= merge_fact(
            &mut self.invocation_facts,
            (owner.cloned(), expression),
            fact,
        );
    }

    fn merge_function_return(&mut self, function: FunctionInstance, fact: Fact) {
        let current = self
            .function_returns
            .get(&function)
            .cloned()
            .unwrap_or(Fact::Bottom);
        let merged = current.join_definition(fact);
        if merged != current {
            self.function_returns.insert(function, merged);
            self.changed = true;
        }
    }

    fn merge_closure_return(&mut self, closure: ClosureInstance, fact: Fact) {
        let current = self
            .closure_returns
            .get(&closure)
            .cloned()
            .unwrap_or(Fact::Bottom);
        let merged = current.join_definition(fact);
        if merged != current {
            self.closure_returns.insert(closure, merged);
            self.changed = true;
        }
    }
}

fn merge_fact<K: std::hash::Hash + Eq>(map: &mut HashMap<K, Fact>, key: K, incoming: Fact) -> bool {
    let current = map.get(&key).cloned().unwrap_or(Fact::Bottom);
    let merged = current.join_definition(incoming);
    if merged == current {
        return false;
    }
    map.insert(key, merged);
    true
}

#[cfg(test)]
mod tests {
    use super::ExactRuntimeRepresentations;
    use crate::{
        ast::ExprId,
        codegen::{async_frame::AsyncFrameLayouts, reachability::Reachability},
        semantic::SemanticModel,
        types::TypeKind,
        wasm_ir,
    };

    fn analyze(source: &str) -> (wasm_ir::Program, SemanticModel, ExactRuntimeRepresentations) {
        let checked = crate::check(crate::lower(crate::parse(source).unwrap()))
            .expect("exact-representation fixture should check");
        let backend = crate::lower_wasm(&checked);
        let reachability = Reachability::analyze(
            &backend.program,
            &backend.semantics,
            &backend.wasm_ir,
            &backend.standard_library,
            backend.capabilities,
            std::iter::empty(),
        );
        let frames = AsyncFrameLayouts::plan(
            &backend.program,
            &backend.wasm_ir,
            &backend.semantics,
            &reachability,
        );
        let exact = ExactRuntimeRepresentations::analyze(
            &backend.program,
            &backend.wasm_ir,
            &backend.semantics,
            &reachability,
            &frames,
        );
        (backend.wasm_ir, backend.semantics, exact)
    }

    #[test]
    fn one_analysis_tracks_callables_generators_and_futures() {
        let (program, semantics, exact) = analyze(
            r#"
                state "game.exe" {}

                fn values() -> iterator u32 {
                    yield 1
                }

                fn later() -> async u32 {
                    await nextTick()
                    return 2
                }

                onAttach {
                    let transform = value => value + 1
                    print(transform(3))
                    let iterator = values()
                    print(iterator.next())
                    print(await later())
                }
            "#,
        );

        let mut callable = false;
        let mut iterator = false;
        let mut future = false;
        for expression in program.expressions() {
            match &expression.kind {
                wasm_ir::ExpressionKind::Invoke { .. } => {
                    callable |= exact.invocation(None, expression.id).is_some();
                }
                wasm_ir::ExpressionKind::Call { .. } => {
                    iterator |= exact.call_receiver(None, expression.id).is_some();
                    if matches!(
                        semantics.types().kind(expression.ty),
                        TypeKind::Async { .. }
                    ) {
                        future |= exact.expression_continuation(None, expression.id).is_some();
                    }
                }
                _ => {}
            }
        }
        assert!(
            callable,
            "the closure invocation should have one exact target"
        );
        assert!(iterator, "the generator cursor should have one exact frame");
        assert!(future, "the async call should have one exact frame");
    }

    #[test]
    fn shared_function_bodies_devirtualize_only_when_all_callers_agree() {
        fn next_expression(program: &wasm_ir::Program) -> ExprId {
            program
                .expressions()
                .find_map(|expression| match &expression.kind {
                    wasm_ir::ExpressionKind::Call {
                        target:
                            wasm_ir::CallTarget::CapabilityRequirement {
                                item: crate::stdlib::StdlibItemId::IteratorNext,
                                ..
                            },
                        ..
                    } if expression.source.is_some() => Some(expression.id),
                    _ => None,
                })
                .expect("the source helper calls Iterator.next")
        }

        let (program, _, exact) = analyze(
            r#"
                state "game.exe" {}

                fn first() -> iterator u32 { yield 1 }
                fn inspect(values: iterator u32) { print(values.next()) }

                whileAttached {
                    let values = first()
                    inspect(values)
                }
            "#,
        );
        let next = next_expression(&program);
        assert!(
            exact
                .call_receivers
                .keys()
                .any(|(_, expression)| *expression == next),
            "an exact producer should flow through a local and helper parameter"
        );

        let (program, _, exact) = analyze(
            r#"
                state "game.exe" {}

                fn first() -> iterator u32 { yield 1 }
                fn second() -> iterator u32 { yield 2 }
                fn inspect(values: iterator u32) { print(values.next()) }

                whileAttached {
                    inspect(first())
                    inspect(second())
                }
            "#,
        );
        let next = next_expression(&program);
        assert!(
            !exact
                .call_receivers
                .keys()
                .any(|(_, expression)| *expression == next),
            "one shared body receiving different frames must stay dynamically dispatched"
        );
    }
}
