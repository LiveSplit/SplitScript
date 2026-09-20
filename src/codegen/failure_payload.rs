//! Whole-program demand analysis for fallible-value error payloads.
//!
//! A `T!` always keeps one stable physical layout. This pass only determines
//! whether its error field must contain a materialized payload. Demand is
//! joined per existing result layout, never per call site, so a function is
//! not cloned merely because one caller observes its error and another does
//! not.

use std::collections::{BTreeMap, BTreeSet};

use crate::{
    ast::{ExprId, ResultTypeId},
    hir::FailureTarget,
    semantic::{FunctionInstance, SemanticModel},
    types::{TypeId, TypeKind},
    wasm_ir::{self, BodyOwner, Visitor},
};

use super::reachability::Reachability;

#[derive(Debug, Default)]
pub(super) struct FailurePayloadDemand {
    demanded: BTreeSet<ResultTypeId>,
}

impl FailurePayloadDemand {
    pub(super) fn analyze(
        semantics: &SemanticModel,
        program: &wasm_ir::Program,
        reachability: &Reachability,
        managed: &crate::managed::ManagedBindingPlan,
        capabilities: &crate::capabilities::CapabilityAnalysis,
    ) -> Self {
        let mut demanded = BTreeSet::new();
        let mut dependencies = BTreeMap::<ResultTypeId, BTreeSet<ResultTypeId>>::new();

        // Equality and derived formatting both inspect the error payload.
        for (_, kind) in semantics.types().iter() {
            if let TypeKind::Result { layout, .. } = kind
                && reachability.requires_result_equality(*layout)
            {
                demanded.insert(*layout);
            }
        }
        for ty in reachability.derived_debugs() {
            if let TypeKind::Result { layout, .. } = semantics.types().kind(ty) {
                demanded.insert(*layout);
            }
        }

        for (owner, id) in reachability.expression_instances() {
            let expression = program
                .expression(id)
                .expect("reachable expressions belong to Wasm IR");
            match &expression.kind {
                wasm_ir::ExpressionKind::Match { value, arms } => {
                    if arms.iter().any(|arm| arm.pattern.binds_result_error())
                        && let Some(result) =
                            expression_result(*value, owner.as_ref(), program, semantics)
                    {
                        demanded.insert(result);
                    }
                }
                wasm_ir::ExpressionKind::Propagate { value, target } => {
                    if let FailureTarget::Return(target) = target
                        && let (Some(target), Some(source)) = (
                            type_result(*target, owner.as_ref(), semantics),
                            expression_result(*value, owner.as_ref(), program, semantics),
                        )
                    {
                        dependencies.entry(target).or_default().insert(source);
                    }
                }
                wasm_ir::ExpressionKind::Call {
                    target: wasm_ir::CallTarget::ManagedComponent { helper_result, .. },
                    ..
                } => {
                    if let (Some(target), Some(source)) = (
                        type_result(expression.ty, owner.as_ref(), semantics),
                        type_result(*helper_result, owner.as_ref(), semantics),
                    ) {
                        dependencies.entry(target).or_default().insert(source);
                    }
                }
                _ => {}
            }
        }

        // Statement-form matches carry their patterns in blocks rather than
        // expression nodes, so inspect every reachable body with its concrete
        // generic owner.
        for body in program.bodies() {
            if matches!(body.owner, BodyOwner::Action(_)) {
                PatternDemandVisitor::new(None, semantics, &mut demanded)
                    .visit_block(&body.entry, program);
            }
        }
        for owner in reachability.functions() {
            let body = program
                .body(BodyOwner::Function(owner.clone()))
                .expect("reachable functions have Wasm IR bodies");
            PatternDemandVisitor::new(Some(owner), semantics, &mut demanded)
                .visit_block(&body.entry, program);
        }
        for state in program.state_expressions() {
            PatternDemandVisitor::new(None, semantics, &mut demanded)
                .visit_block(&state.entry, program);
        }
        for transform in program.state_transforms() {
            PatternDemandVisitor::new(None, semantics, &mut demanded)
                .visit_block(&transform.entry, program);
        }
        for initializer in program.global_initializer_plans() {
            PatternDemandVisitor::new(None, semantics, &mut demanded)
                .visit_block(&initializer.entry, program);
        }
        for instance in reachability.closure_instances() {
            let closure = program
                .closure(instance.expression)
                .expect("reachable closures have Wasm IR bodies");
            PatternDemandVisitor::new(instance.owner.as_ref(), semantics, &mut demanded)
                .visit_block(&closure.entry, program);
        }

        // Snapshot readers are generated directly rather than represented by
        // `?` expressions in Wasm IR. Their field failures still flow into the
        // outer Result, and must retain messages when that Result is observed.
        // Inferred and generated uses can have different Result layout IDs
        // for the same value type. Include each alias: generated field readers
        // select a canonical layout independently of the observing call site.
        let mut results = BTreeMap::<TypeId, BTreeSet<ResultTypeId>>::new();
        for (_, kind) in semantics.types().iter() {
            if let TypeKind::Result { layout, value } = kind {
                results.entry(*value).or_default().insert(*layout);
            }
        }
        for class in reachability.managed_snapshots() {
            let targets = &results[&semantics.types().id_for_managed_class(class)];
            for target in targets {
                dependencies.entry(*target).or_default().extend(targets);
            }
            let binding = managed
                .classes
                .iter()
                .find(|binding| binding.id == class)
                .expect("reachable snapshots have managed bindings");
            for field in binding
                .all_fields()
                .filter(|field| field.kind == crate::managed::ManagedFieldKind::Instance)
            {
                for target in targets {
                    dependencies
                        .entry(*target)
                        .or_default()
                        .extend(&results[&field.value_type]);
                    dependencies
                        .entry(*target)
                        .or_default()
                        .extend(&results[&field.snapshot_type]);
                    if let TypeKind::Option { value, .. } =
                        semantics.types().kind(field.snapshot_type)
                        && matches!(semantics.types().kind(*value), TypeKind::ManagedClass(_))
                    {
                        for optional in &results[&field.snapshot_type] {
                            dependencies
                                .entry(*optional)
                                .or_default()
                                .extend(&results[value]);
                        }
                    }
                }
            }
        }

        for ty in reachability.managed_decoders() {
            let plan = capabilities.managed_decoder(ty).unwrap();
            if matches!(
                plan.kind,
                crate::managed_read::ManagedDecoderKind::Array { .. }
                    | crate::managed_read::ManagedDecoderKind::List { .. }
            ) {
                let address = semantics
                    .types()
                    .id_for_core(crate::stdlib::CoreTypeId::Address);
                for target in &results[&plan.output] {
                    dependencies
                        .entry(*target)
                        .or_default()
                        .extend(&results[&address]);
                }
            }
            if matches!(
                plan.kind,
                crate::managed_read::ManagedDecoderKind::List { .. }
            ) {
                let layout = semantics
                    .types()
                    .id_for_standard(crate::stdlib::StdlibTypeId::UnityListLayout);
                for target in &results[&plan.output] {
                    dependencies
                        .entry(*target)
                        .or_default()
                        .extend(&results[&layout]);
                }
            }
            if matches!(
                plan.kind,
                crate::managed_read::ManagedDecoderKind::Map { .. }
                    | crate::managed_read::ManagedDecoderKind::Set { .. }
            ) {
                for dependency in [
                    semantics
                        .types()
                        .id_for_standard(crate::stdlib::StdlibTypeId::UnityKeyedRead),
                    semantics
                        .types()
                        .id_for_core(crate::stdlib::CoreTypeId::Bool),
                ] {
                    for target in &results[&plan.output] {
                        dependencies
                            .entry(*target)
                            .or_default()
                            .extend(&results[&dependency]);
                    }
                }
            }
            for child in plan.kind.children() {
                let child = capabilities.managed_decoder(child).unwrap().output;
                for target in &results[&plan.output] {
                    dependencies
                        .entry(*target)
                        .or_default()
                        .extend(&results[&child]);
                }
            }
        }
        // If an outer error is observable, every payload forwarded into it is
        // observable too. Iterate to a fixed point for chains of `?` calls.
        let mut pending = demanded.iter().copied().collect::<Vec<_>>();
        while let Some(target) = pending.pop() {
            for source in dependencies.get(&target).into_iter().flatten() {
                if demanded.insert(*source) {
                    pending.push(*source);
                }
            }
        }

        Self { demanded }
    }

    pub(super) fn is_demanded(&self, result: ResultTypeId) -> bool {
        self.demanded.contains(&result)
    }
}

fn expression_result(
    expression: ExprId,
    owner: Option<&FunctionInstance>,
    program: &wasm_ir::Program,
    semantics: &SemanticModel,
) -> Option<ResultTypeId> {
    type_result(
        program
            .expression(expression)
            .expect("lowered expressions belong to Wasm IR")
            .ty,
        owner,
        semantics,
    )
}

fn type_result(
    ty: TypeId,
    owner: Option<&FunctionInstance>,
    semantics: &SemanticModel,
) -> Option<ResultTypeId> {
    let ty = owner.map_or(ty, |owner| semantics.specialize_type(owner, ty));
    match semantics.types().kind(ty) {
        TypeKind::Result { layout, .. } => Some(*layout),
        _ => None,
    }
}

struct PatternDemandVisitor<'a> {
    owner: Option<&'a FunctionInstance>,
    semantics: &'a SemanticModel,
    demanded: &'a mut BTreeSet<ResultTypeId>,
}

impl<'a> PatternDemandVisitor<'a> {
    fn new(
        owner: Option<&'a FunctionInstance>,
        semantics: &'a SemanticModel,
        demanded: &'a mut BTreeSet<ResultTypeId>,
    ) -> Self {
        Self {
            owner,
            semantics,
            demanded,
        }
    }
}

impl Visitor for PatternDemandVisitor<'_> {
    fn visit_statement(&mut self, statement: &wasm_ir::Statement, program: &wasm_ir::Program) {
        if let wasm_ir::Statement::Match { value, arms, .. } = statement
            && arms.iter().any(|arm| arm.pattern.binds_result_error())
            && let Some(result) = expression_result(*value, self.owner, program, self.semantics)
        {
            self.demanded.insert(result);
        }
        wasm_ir::walk_statement(self, statement, program);
    }

    // Only block-owned patterns matter here. Reachable expression instances
    // were already handled above, without revisiting their shared DAGs.
    fn visit_expression(&mut self, _expression: &wasm_ir::Expression, _program: &wasm_ir::Program) {
    }
}
