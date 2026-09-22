//! Lifecycle-scoped global lifetime and definite-initialization analysis.
//!
//! A bare top-level `let` is backed by ordinary mutable Wasm storage, but its
//! source lifetime is inferred from the lifecycle action that definitely
//! initializes it. `onAttach` owns attachment-scoped values and `onStart`
//! owns attempt-scoped values. This pass is the semantic authority for those
//! lifetimes, shape-dependent attachment availability, and the viral helper
//! requirements they induce. Backend defaults must never become observable
//! source values.

use std::{
    cell::RefCell,
    collections::{HashMap, HashSet},
};

use crate::{
    Diagnostic,
    ast::{ActionKind, EnumVariantId, FunctionId, Program, Span, ValueId},
    hir::{TypedBlock, TypedExpressionKind, TypedProgram, TypedStatementKind},
    semantic::{
        ResolvedCall, ResolvedEnumVariantId, ResolvedShapeConstraint, ResolvedShapeDimension,
        ResolvedValue, SemanticModel,
    },
    stdlib::CoreTypeId,
    types::TypeKind,
};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum AttachmentShape {
    Single,
    Shape(u16),
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum GlobalLifetime {
    Attachment,
    Attempt,
}

/// Lifecycle actions whose invocation is either the attempt initializer itself
/// or is guarded by the generated attempt-readiness state.
pub const fn action_has_attempt_scope(action: ActionKind) -> bool {
    matches!(
        action,
        ActionKind::OnStart
            | ActionKind::OnReset
            | ActionKind::Split
            | ActionKind::Reset
            | ActionKind::IsLoading
            | ActionKind::GameTime
    )
}

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct ScopedGlobalAnalysis {
    lifetimes: HashMap<ValueId, GlobalLifetime>,
    compiler_initialized: HashSet<ValueId>,
    shapes: Vec<AttachmentShape>,
    shape_facts: HashMap<AttachmentShape, Vec<ResolvedShapeConstraint>>,
    available_in: HashMap<ValueId, HashSet<AttachmentShape>>,
    function_shapes: HashMap<FunctionId, HashSet<AttachmentShape>>,
    function_requires_attempt: HashSet<FunctionId>,
    action_requires_attempt: HashSet<ActionKind>,
}

impl ScopedGlobalAnalysis {
    pub fn lifetime(&self, value: ValueId) -> Option<GlobalLifetime> {
        self.lifetimes.get(&value).copied()
    }

    pub fn is_scoped_global(&self, value: ValueId) -> bool {
        self.lifetimes.contains_key(&value)
    }

    pub fn is_attachment_global(&self, value: ValueId) -> bool {
        self.lifetime(value) == Some(GlobalLifetime::Attachment)
    }

    pub fn is_compiler_initialized(&self, value: ValueId) -> bool {
        self.compiler_initialized.contains(&value)
    }

    pub fn is_attempt_global(&self, value: ValueId) -> bool {
        self.lifetime(value) == Some(GlobalLifetime::Attempt)
    }

    pub fn attachment_globals(&self) -> impl Iterator<Item = ValueId> + '_ {
        self.lifetimes.iter().filter_map(|(value, lifetime)| {
            (*lifetime == GlobalLifetime::Attachment).then_some(*value)
        })
    }

    pub fn attempt_globals(&self) -> impl Iterator<Item = ValueId> + '_ {
        self.lifetimes.iter().filter_map(|(value, lifetime)| {
            (*lifetime == GlobalLifetime::Attempt).then_some(*value)
        })
    }

    pub fn shapes(&self) -> &[AttachmentShape] {
        &self.shapes
    }

    fn shape_has_fact(
        &self,
        shape: AttachmentShape,
        dimension: ResolvedShapeDimension,
        variant: EnumVariantId,
    ) -> bool {
        self.shape_facts.get(&shape).is_some_and(|facts| {
            facts
                .iter()
                .any(|fact| fact.dimension == dimension && fact.variant == variant)
        })
    }

    fn shape_satisfies(
        &self,
        shape: AttachmentShape,
        predicate: &crate::semantic::ResolvedShapePredicate,
    ) -> bool {
        predicate.alternatives.iter().any(|alternative| {
            alternative.iter().all(|constraint| {
                self.shape_has_fact(shape, constraint.dimension, constraint.variant)
            })
        })
    }

    pub fn is_available_in(&self, value: ValueId, shape: AttachmentShape) -> bool {
        self.available_in
            .get(&value)
            .is_some_and(|shapes| shapes.contains(&shape))
    }

    pub fn available_shapes(&self, value: ValueId) -> impl Iterator<Item = AttachmentShape> + '_ {
        self.available_in.get(&value).into_iter().flatten().copied()
    }

    /// Shapes in which a helper's attachment-global preconditions hold.
    /// An empty result means the helper has no valid attached call context.
    pub fn function_shapes(
        &self,
        function: FunctionId,
    ) -> impl Iterator<Item = AttachmentShape> + '_ {
        self.function_shapes
            .get(&function)
            .into_iter()
            .flatten()
            .copied()
    }

    pub fn function_requires_attempt(&self, function: FunctionId) -> bool {
        self.function_requires_attempt.contains(&function)
    }

    pub fn shape_label(&self, shape: AttachmentShape, syntax: &Program) -> String {
        let Some(facts) = self.shape_facts.get(&shape) else {
            return "the attachment".to_owned();
        };
        let labels = facts
            .iter()
            .filter_map(|fact| {
                let ResolvedShapeDimension::Global(value) = fact.dimension else {
                    return None;
                };
                let global = syntax
                    .globals
                    .iter()
                    .filter_map(|global| global.binding.simple_binding())
                    .find(|binding| binding.id == value)?;
                let enumeration = syntax.enums.iter().find(|enumeration| {
                    enumeration
                        .variants
                        .iter()
                        .any(|variant| variant.id == fact.variant)
                })?;
                let variant = enumeration
                    .variants
                    .iter()
                    .find(|variant| variant.id == fact.variant)?;
                Some(format!(
                    "{} = {}.{}",
                    global.name, enumeration.name, variant.name
                ))
            })
            .collect::<Vec<_>>();
        if labels.is_empty() {
            "the attachment".to_owned()
        } else {
            labels.join(", ")
        }
    }

    pub fn action_requires_attempt(&self, action: ActionKind) -> bool {
        self.action_requires_attempt.contains(&action)
    }
}

#[derive(Clone)]
struct EvalPath {
    assigned: HashSet<ValueId>,
    shape_facts: HashMap<ValueId, EnumVariantId>,
}

impl EvalPath {
    fn new(assigned: HashSet<ValueId>) -> Self {
        Self {
            assigned,
            shape_facts: HashMap::new(),
        }
    }
}

#[derive(Default)]
struct Flow {
    normal: Vec<EvalPath>,
    returned: Vec<EvalPath>,
}

struct Initializer<'a> {
    hir: &'a TypedProgram,
    semantics: &'a SemanticModel,
    globals: &'a HashSet<ValueId>,
    debug_globals: HashSet<ValueId>,
    shape_globals: HashSet<ValueId>,
    requirements: &'a HashMap<FunctionId, FunctionRequirements>,
    uninitialized_reads: RefCell<HashSet<(ValueId, Span)>>,
}

fn attachment_shapes(
    syntax: &Program,
    semantics: &SemanticModel,
) -> (
    Vec<AttachmentShape>,
    HashMap<AttachmentShape, Vec<ResolvedShapeConstraint>>,
) {
    let mut dimensions = Vec::new();
    let predicates = syntax
        .state
        .iter()
        .flat_map(|state| state.all_fields())
        .filter_map(|field| semantics.state_field_shape_predicate(field.id))
        .chain(
            syntax
                .managed_class_declarations()
                .into_iter()
                .flat_map(|class| class.all_fields())
                .filter_map(|field| semantics.managed_field_shape_predicate(field.id)),
        );
    for predicate in predicates {
        for constraint in predicate.alternatives.iter().flatten() {
            if matches!(constraint.dimension, ResolvedShapeDimension::Global(_))
                && !dimensions.contains(&constraint.dimension)
            {
                dimensions.push(constraint.dimension);
            }
        }
    }
    dimensions.sort_by_key(|dimension| match dimension {
        ResolvedShapeDimension::Global(value) => value.index(),
        ResolvedShapeDimension::StateField(_) => {
            unreachable!("attachment shapes only contain global dimensions")
        }
    });
    if dimensions.is_empty() {
        return (
            vec![AttachmentShape::Single],
            HashMap::from([(AttachmentShape::Single, Vec::new())]),
        );
    }

    let mut assignments = vec![Vec::new()];
    for dimension in dimensions {
        let ResolvedShapeDimension::Global(value) = dimension else {
            unreachable!()
        };
        let Some(ty) = semantics.value_type(value) else {
            continue;
        };
        let TypeKind::Enum(enumeration) = semantics.types().kind(ty) else {
            continue;
        };
        let Some(declaration) = syntax.enum_declaration(*enumeration) else {
            continue;
        };
        let mut expanded = Vec::new();
        for assignment in &assignments {
            for variant in &declaration.variants {
                let mut candidate = assignment.clone();
                candidate.push(ResolvedShapeConstraint {
                    dimension,
                    variant: variant.id,
                });
                expanded.push(candidate);
            }
        }
        assignments = expanded;
    }
    if assignments.is_empty()
        || assignments.len() > crate::shape_selection::MAX_ENUMERATED_SHAPE_COMBINATIONS
    {
        return (
            vec![AttachmentShape::Single],
            HashMap::from([(AttachmentShape::Single, Vec::new())]),
        );
    }
    let shapes = (0..assignments.len())
        .map(|index| AttachmentShape::Shape(index as u16))
        .collect::<Vec<_>>();
    let facts = shapes.iter().copied().zip(assignments).collect();
    (shapes, facts)
}

fn path_matches_shape(
    path: &EvalPath,
    shape: AttachmentShape,
    facts: &HashMap<AttachmentShape, Vec<ResolvedShapeConstraint>>,
) -> bool {
    facts.get(&shape).into_iter().flatten().all(|constraint| {
        let ResolvedShapeDimension::Global(value) = constraint.dimension else {
            return true;
        };
        path.shape_facts
            .get(&value)
            .is_none_or(|variant| *variant == constraint.variant)
    })
}

pub(crate) fn analyze(
    syntax: &Program,
    hir: &TypedProgram,
    semantics: &SemanticModel,
) -> (ScopedGlobalAnalysis, Vec<Diagnostic>) {
    let bare_globals = hir.bare_globals().collect::<HashSet<_>>();
    let (shapes, shape_facts) = attachment_shapes(syntax, semantics);
    let mut analysis = ScopedGlobalAnalysis {
        lifetimes: HashMap::new(),
        compiler_initialized: HashSet::new(),
        shapes,
        shape_facts,
        available_in: HashMap::new(),
        function_shapes: HashMap::new(),
        function_requires_attempt: HashSet::new(),
        action_requires_attempt: HashSet::new(),
    };
    if bare_globals.is_empty() {
        return (analysis, Vec::new());
    }

    let automatic_shape = if crate::shape_selection::has_explicit_shape_selection(syntax) {
        crate::shape_selection::AutomaticShapeSelection::NotDeclared
    } else {
        crate::shape_selection::automatic_shape_selection(syntax, semantics)
    };
    let compiler_initialized = match automatic_shape {
        crate::shape_selection::AutomaticShapeSelection::Available(plan)
            if plan.evidence_fields.is_empty()
                || semantics.state_provider()
                    == Some(crate::stdlib::StdlibStateProviderId::Unity) =>
        {
            plan.dimensions
                .into_iter()
                .filter_map(|dimension| match dimension.dimension {
                    crate::semantic::ResolvedShapeDimension::Global(value) => Some(value),
                    crate::semantic::ResolvedShapeDimension::StateField(_) => None,
                })
                .collect::<HashSet<_>>()
        }
        _ => HashSet::new(),
    };
    analysis.compiler_initialized = compiler_initialized.clone();
    let mut assigned_by_attach = assignments_in_action(hir, ActionKind::OnAttach, &bare_globals);
    assigned_by_attach.extend(compiler_initialized.iter().copied());
    let assigned_by_start = assignments_in_action(hir, ActionKind::OnStart, &bare_globals);
    let declarations = syntax
        .globals
        .iter()
        .filter(|global| global.value.is_none())
        .map(|global| (global.id, global))
        .collect::<HashMap<_, _>>();
    let mut diagnostics = Vec::new();
    for value in &bare_globals {
        let in_attach = assigned_by_attach.contains(value);
        let in_start = assigned_by_start.contains(value);
        match (in_attach, in_start) {
            (true, false) => {
                analysis
                    .lifetimes
                    .insert(*value, GlobalLifetime::Attachment);
            }
            (false, true) => {
                analysis.lifetimes.insert(*value, GlobalLifetime::Attempt);
            }
            (true, true) => {
                let Some(global) = declarations.get(value) else {
                    continue;
                };
                let mut diagnostic = Diagnostic::semantic(
                    format!(
                        "bare global `{}` has both attachment and attempt initializers",
                        global.name
                    ),
                    global.name_span,
                )
                .with_primary_label(
                    "a bare global must have exactly one lifecycle initialization boundary",
                );
                if let Some(action) = syntax
                    .actions
                    .iter()
                    .find(|action| action.kind == ActionKind::OnAttach)
                {
                    diagnostic = diagnostic.with_secondary_label(
                        action.span,
                        "attachment initialization happens here",
                    );
                }
                if let Some(action) = syntax
                    .actions
                    .iter()
                    .find(|action| action.kind == ActionKind::OnStart)
                {
                    diagnostic = diagnostic
                        .with_secondary_label(action.span, "attempt initialization happens here");
                }
                diagnostics.push(diagnostic);
            }
            (false, false) => {
                let Some(global) = declarations.get(value) else {
                    continue;
                };
                diagnostics.push(
                    Diagnostic::semantic(
                        format!(
                            "bare global `{}` has no direct lifecycle initializer",
                            global.name
                        ),
                        global.name_span,
                    )
                    .with_primary_label(
                        "assign this global directly in exactly one `onAttach` or `onStart` block",
                    )
                    .with_note(
                        "`onAttach` creates attachment-scoped state; `onStart` creates attempt-scoped state",
                    )
                    .with_note(
                        "assignments performed by called helpers do not establish a bare global's lifetime",
                    ),
                );
            }
        }
    }

    let requirements = infer_function_requirements(syntax, hir, &analysis);
    let attachment_globals = analysis.attachment_globals().collect::<HashSet<_>>();
    let attempt_globals = analysis.attempt_globals().collect::<HashSet<_>>();
    let shape_globals = analysis
        .shape_facts
        .values()
        .flatten()
        .filter_map(|constraint| match constraint.dimension {
            ResolvedShapeDimension::Global(value) => Some(value),
            ResolvedShapeDimension::StateField(_) => None,
        })
        .collect();
    if let Some(on_attach) = hir.action_body(ActionKind::OnAttach)
        && !attachment_globals.is_empty()
    {
        let initializer = Initializer {
            hir,
            semantics,
            globals: &attachment_globals,
            debug_globals: debug_globals(hir, &attachment_globals),
            shape_globals,
            requirements: &requirements,
            uninitialized_reads: RefCell::new(HashSet::new()),
        };
        let initial = if compiler_initialized.is_empty() {
            vec![EvalPath::new(HashSet::new())]
        } else {
            analysis
                .shapes
                .iter()
                .copied()
                .map(|shape| {
                    let mut path = EvalPath::new(compiler_initialized.clone());
                    for constraint in analysis.shape_facts.get(&shape).into_iter().flatten() {
                        if let ResolvedShapeDimension::Global(value) = constraint.dimension
                            && compiler_initialized.contains(&value)
                        {
                            path.shape_facts.insert(value, constraint.variant);
                        }
                    }
                    path
                })
                .collect()
        };
        let mut flow = initializer.eval_block(on_attach, initial);
        flow.returned.append(&mut flow.normal);
        let mut reachable_shapes = Vec::new();
        for shape in analysis.shapes.iter().copied() {
            let paths = flow
                .returned
                .iter()
                .filter(|path| path_matches_shape(path, shape, &analysis.shape_facts))
                .cloned()
                .collect::<Vec<_>>();
            if !paths.is_empty() {
                reachable_shapes.push(shape);
                record_availability(
                    &mut analysis.available_in,
                    shape,
                    &paths,
                    &attachment_globals,
                );
            }
        }
        analysis.shapes = reachable_shapes;
        diagnostics.extend(initialization_diagnostics(
            &attachment_globals,
            &initializer,
            &declarations,
            on_attach.span,
            GlobalLifetime::Attachment,
            |value| {
                analysis
                    .available_in
                    .get(&value)
                    .is_some_and(|shapes| !shapes.is_empty())
            },
        ));
    } else if !compiler_initialized.is_empty() {
        for value in compiler_initialized {
            analysis
                .available_in
                .entry(value)
                .or_default()
                .extend(analysis.shapes.iter().copied());
        }
    }

    if let Some(on_start) = hir.action_body(ActionKind::OnStart)
        && !attempt_globals.is_empty()
    {
        let initializer = Initializer {
            hir,
            semantics,
            globals: &attempt_globals,
            debug_globals: debug_globals(hir, &attempt_globals),
            shape_globals: HashSet::new(),
            requirements: &requirements,
            uninitialized_reads: RefCell::new(HashSet::new()),
        };
        let mut flow = initializer.eval_block(on_start, vec![EvalPath::new(HashSet::new())]);
        flow.returned.append(&mut flow.normal);
        let definitely_initialized = intersection_of_assignments(&flow.returned, &attempt_globals);
        diagnostics.extend(initialization_diagnostics(
            &attempt_globals,
            &initializer,
            &declarations,
            on_start.span,
            GlobalLifetime::Attempt,
            |value| definitely_initialized.contains(&value),
        ));
    }

    analysis.function_shapes = requirements
        .iter()
        .map(|(function, facts)| {
            let valid = analysis
                .shapes
                .iter()
                .copied()
                .filter(|shape| {
                    facts
                        .attachment_globals
                        .iter()
                        .all(|global| analysis.is_available_in(*global, *shape))
                })
                .collect();
            (*function, valid)
        })
        .collect();
    analysis.function_requires_attempt = requirements
        .iter()
        .filter_map(|(function, facts)| (!facts.attempt_globals.is_empty()).then_some(*function))
        .collect();
    analysis.action_requires_attempt =
        infer_action_attempt_requirements(hir, &analysis, &requirements);
    diagnostics.extend(validate_uses(
        syntax,
        hir,
        semantics,
        &analysis,
        &requirements,
    ));
    (analysis, diagnostics)
}

fn debug_globals(hir: &TypedProgram, globals: &HashSet<ValueId>) -> HashSet<ValueId> {
    hir.bare_globals_with_debug()
        .filter_map(|(value, debug_only)| (debug_only && globals.contains(&value)).then_some(value))
        .collect()
}

fn assignments_in_action(
    hir: &TypedProgram,
    action: ActionKind,
    globals: &HashSet<ValueId>,
) -> HashSet<ValueId> {
    struct Collector<'a> {
        globals: &'a HashSet<ValueId>,
        assigned: HashSet<ValueId>,
    }

    impl ScopedUseVisitor for Collector<'_> {
        fn visit_closure_bodies(&self) -> bool {
            false
        }

        fn global(&mut self, _value: ValueId, _span: Span, _refined: Option<AttachmentShape>) {}

        fn global_write(&mut self, value: ValueId, _span: Span, _refined: Option<AttachmentShape>) {
            if self.globals.contains(&value) {
                self.assigned.insert(value);
            }
        }

        fn call(&mut self, _function: FunctionId, _span: Span, _refined: Option<AttachmentShape>) {}
    }

    let mut collector = Collector {
        globals,
        assigned: HashSet::new(),
    };
    if let Some(body) = hir.action_body(action) {
        walk_block(&mut collector, body, hir, None, None);
    }
    collector.assigned
}

fn intersection_of_assignments(paths: &[EvalPath], globals: &HashSet<ValueId>) -> HashSet<ValueId> {
    let mut assigned = globals.clone();
    for path in paths {
        assigned.retain(|value| path.assigned.contains(value));
    }
    assigned
}

fn initialization_diagnostics(
    globals: &HashSet<ValueId>,
    initializer: &Initializer<'_>,
    declarations: &HashMap<ValueId, &crate::ast::VariableDecl>,
    action_span: Span,
    lifetime: GlobalLifetime,
    is_initialized: impl Fn(ValueId) -> bool,
) -> Vec<Diagnostic> {
    let (scope_name, action_name, completion_label) = match lifetime {
        GlobalLifetime::Attachment => (
            "attachment-scoped",
            "onAttach",
            "assign this value on every path that completes the corresponding attachment shape",
        ),
        GlobalLifetime::Attempt => (
            "attempt-scoped",
            "onStart",
            "assign this value on every path that completes attempt initialization",
        ),
    };
    let mut diagnostics = globals
        .iter()
        .filter(|value| !is_initialized(**value))
        .filter_map(|value| declarations.get(value))
        .map(|global| {
            // Lifetime classification already proved that the initializer
            // contains a direct assignment. Reaching here therefore means the
            // assignment is conditional, not absent.
            let message = format!(
                "{scope_name} global `{}` is not initialized on every `{action_name}` path",
                global.name
            );
            Diagnostic::semantic(message, global.name_span)
                .with_primary_label(completion_label)
                .with_secondary_label(
                    action_span,
                    format!("{scope_name} initialization happens here"),
                )
        })
        .collect::<Vec<_>>();
    for (value, span) in initializer.uninitialized_reads.borrow().iter().copied() {
        let Some(global) = declarations.get(&value) else {
            continue;
        };
        diagnostics.push(
            Diagnostic::semantic(
                format!(
                    "{scope_name} global `{}` may be read before it is initialized",
                    global.name
                ),
                span,
            )
            .with_primary_label("this path has not assigned the value yet")
            .with_secondary_label(
                global.name_span,
                format!("the {scope_name} global is declared here"),
            ),
        );
    }
    diagnostics
}

fn record_availability(
    availability: &mut HashMap<ValueId, HashSet<AttachmentShape>>,
    shape: AttachmentShape,
    paths: &[EvalPath],
    globals: &HashSet<ValueId>,
) {
    let mut definitely_assigned = globals.clone();
    for path in paths {
        definitely_assigned.retain(|global| path.assigned.contains(global));
    }
    for global in definitely_assigned {
        availability.entry(global).or_default().insert(shape);
    }
}

impl Initializer<'_> {
    fn eval_block(&self, block: &TypedBlock, mut normal: Vec<EvalPath>) -> Flow {
        let mut returned = Vec::new();
        for statement in &block.statements {
            if normal.is_empty() {
                break;
            }
            if statement.debug_only {
                // Debug statements disappear in release builds. Their writes
                // may establish debug-only attachment globals, but must never
                // establish a release-visible value or alter release control
                // flow. Reads are still analyzed against the incoming path.
                let debug_flow = self.eval_statement(statement, normal.clone());
                let debug_normal = collapse_paths(debug_flow.normal);
                if let Some(debug_path) = debug_normal.first() {
                    for path in &mut normal {
                        path.assigned.extend(
                            debug_path
                                .assigned
                                .iter()
                                .filter(|value| self.debug_globals.contains(value))
                                .copied(),
                        );
                    }
                }
                continue;
            }
            let flow = self.eval_statement(statement, normal);
            normal = collapse_paths(flow.normal);
            returned.extend(flow.returned);
        }
        Flow { normal, returned }
    }

    fn eval_statement(&self, statement: &crate::hir::TypedStatement, input: Vec<EvalPath>) -> Flow {
        match &statement.kind {
            TypedStatementKind::Variable { initializer, .. }
            | TypedStatementKind::Yield { value: initializer }
            | TypedStatementKind::Expression(initializer) => self.eval_expr(*initializer, input),
            TypedStatementKind::Assign {
                assignment,
                op,
                value,
            } => {
                if op.is_some() && self.globals.contains(&assignment.target) {
                    self.record_uninitialized(assignment.target, assignment.span, &input);
                }
                let mut flow = self.eval_expr(*value, input);
                if op.is_none() && self.globals.contains(&assignment.target) {
                    for path in &mut flow.normal {
                        path.assigned.insert(assignment.target);
                    }
                }
                if op.is_none()
                    && self.shape_globals.contains(&assignment.target)
                    && let Some(ResolvedEnumVariantId::Source(variant)) =
                        self.hir.enum_variant(*value)
                {
                    for path in &mut flow.normal {
                        path.shape_facts.insert(assignment.target, variant);
                    }
                }
                flow
            }
            TypedStatementKind::StateAssign { target, value, .. }
            | TypedStatementKind::IndexAssign { target, value, .. } => {
                self.eval_sequence(&[*target, *value], input)
            }
            TypedStatementKind::If {
                condition,
                then_block,
                else_block,
            } => {
                let condition = self.eval_expr(*condition, input);
                let mut returned = condition.returned;
                let mut normal = Vec::new();
                for path in condition.normal {
                    let then_flow = self.eval_block(then_block, vec![path.clone()]);
                    normal.extend(then_flow.normal);
                    returned.extend(then_flow.returned);
                    if let Some(else_block) = else_block {
                        let else_flow = self.eval_block(else_block, vec![path]);
                        normal.extend(else_flow.normal);
                        returned.extend(else_flow.returned);
                    } else {
                        normal.push(path);
                    }
                }
                Flow { normal, returned }
            }
            TypedStatementKind::While { condition, body } => {
                let condition = self.eval_expr(*condition, input);
                let mut returned = condition.returned;
                for path in &condition.normal {
                    returned.extend(self.eval_block(body, vec![path.clone()]).returned);
                }
                Flow {
                    normal: condition.normal,
                    returned,
                }
            }
            TypedStatementKind::For { iterable, body, .. } => {
                let iterable = self.eval_expr(*iterable, input);
                let mut returned = iterable.returned;
                for path in &iterable.normal {
                    returned.extend(self.eval_block(body, vec![path.clone()]).returned);
                }
                Flow {
                    normal: iterable.normal,
                    returned,
                }
            }
            TypedStatementKind::Suspend { value, returns, .. } => {
                let mut flow = self.eval_expr(*value, input);
                if *returns {
                    flow.returned.append(&mut flow.normal);
                }
                flow
            }
        }
    }

    fn eval_expr(&self, id: crate::ast::ExprId, input: Vec<EvalPath>) -> Flow {
        let expression = self
            .hir
            .expression(id)
            .expect("attachment analysis only visits typed expressions");
        if let Some((Some(ResolvedValue::Variable(value)), _)) = self.hir.value_path(id)
            && self.globals.contains(&value)
        {
            self.record_uninitialized(value, expression.span, &input);
        }
        if let Some(call) = self.hir.call(id) {
            if let Some(value) = call
                .receiver()
                .and_then(|receiver| receiver.path().map(|(root, _)| root))
                .and_then(ResolvedValue::source_value)
                && self.globals.contains(&value)
            {
                self.record_uninitialized(value, expression.span, &input);
            }
            if let ResolvedCall::UserFunction { function, .. }
            | ResolvedCall::UserMethod { function, .. } = call
                && let Some(requirements) = self.requirements.get(function)
            {
                for value in requirements
                    .attachment_globals
                    .iter()
                    .chain(&requirements.attempt_globals)
                    .filter(|value| self.globals.contains(value))
                {
                    self.record_uninitialized(*value, expression.span, &input);
                }
            }
        }
        let mut flow = match &expression.kind {
            TypedExpressionKind::Enum { payload, .. } => {
                if let Some(payload) = payload {
                    self.eval_expr(*payload, input)
                } else {
                    Flow {
                        normal: input,
                        returned: Vec::new(),
                    }
                }
            }
            TypedExpressionKind::Block { statements, value } => {
                let mut flow = self.eval_block(statements, input);
                if let Some(value) = value {
                    let tail = self.eval_expr(*value, flow.normal);
                    flow.normal = tail.normal;
                    flow.returned.extend(tail.returned);
                }
                flow
            }
            TypedExpressionKind::Match { value, arms } => {
                let subject = self.eval_expr(*value, input);
                let mut returned = subject.returned;
                let mut normal = Vec::new();
                for path in subject.normal {
                    for arm in arms {
                        let guard = arm.guard.map_or_else(
                            || Flow {
                                normal: vec![path.clone()],
                                returned: Vec::new(),
                            },
                            |guard| self.eval_expr(guard, vec![path.clone()]),
                        );
                        returned.extend(guard.returned);
                        let arm_flow = self.eval_expr(arm.value, guard.normal);
                        normal.extend(arm_flow.normal);
                        returned.extend(arm_flow.returned);
                    }
                }
                Flow { normal, returned }
            }
            TypedExpressionKind::If {
                condition,
                then_expr,
                else_expr,
            } => {
                let condition = self.eval_expr(*condition, input);
                let mut returned = condition.returned;
                let mut normal = Vec::new();
                for path in condition.normal {
                    let then_flow = self.eval_expr(*then_expr, vec![path.clone()]);
                    normal.extend(then_flow.normal);
                    returned.extend(then_flow.returned);
                    let else_flow = self.eval_expr(*else_expr, vec![path]);
                    normal.extend(else_flow.normal);
                    returned.extend(else_flow.returned);
                }
                Flow { normal, returned }
            }
            TypedExpressionKind::Fallback { value, fallback } => {
                let value_flow = self.eval_expr(*value, input.clone());
                let fallback_flow = self.eval_expr(*fallback, input);
                Flow {
                    normal: value_flow
                        .normal
                        .into_iter()
                        .chain(fallback_flow.normal)
                        .collect(),
                    returned: value_flow
                        .returned
                        .into_iter()
                        .chain(fallback_flow.returned)
                        .collect(),
                }
            }
            TypedExpressionKind::Return(value) => {
                let mut flow = if let Some(value) = value {
                    self.eval_expr(*value, input)
                } else {
                    Flow {
                        normal: input,
                        returned: Vec::new(),
                    }
                };
                flow.returned.append(&mut flow.normal);
                flow
            }
            TypedExpressionKind::Break(_)
            | TypedExpressionKind::Continue
            | TypedExpressionKind::Throw { .. } => {
                let children = match &expression.kind {
                    TypedExpressionKind::Break(Some(value)) => vec![*value],
                    TypedExpressionKind::Throw { error, .. } => vec![*error],
                    _ => Vec::new(),
                };
                let mut flow = self.eval_sequence(&children, input);
                flow.normal.clear();
                flow
            }
            TypedExpressionKind::Loop { body } => {
                let mut returned = Vec::new();
                for path in &input {
                    returned.extend(self.eval_block(body, vec![path.clone()]).returned);
                }
                // Conservative until value-producing breaks are explicit CFG
                // edges in typed HIR.
                Flow {
                    normal: input,
                    returned,
                }
            }
            // Constructing a closure does not execute its body. Any scoped
            // requirements of that body are enforced when the callable is
            // invoked, rather than pretending its assignments initialized the
            // surrounding lifecycle action.
            TypedExpressionKind::Closure { .. } => Flow {
                normal: input,
                returned: Vec::new(),
            },
            _ => self.eval_sequence(&expression_children(&expression.kind), input),
        };
        if matches!(
            self.semantics.types().kind(expression.ty),
            TypeKind::Builtin(CoreTypeId::Never)
        ) {
            // `await process.closed()`, a divergent value block, and any
            // future source-defined `Never` expression terminate this path.
            // Keep this semantic rather than enumerating terminal syntax.
            flow.normal.clear();
        }
        flow
    }

    fn eval_sequence(&self, children: &[crate::ast::ExprId], input: Vec<EvalPath>) -> Flow {
        let mut normal = input;
        let mut returned = Vec::new();
        for child in children {
            let flow = self.eval_expr(*child, normal);
            normal = flow.normal;
            returned.extend(flow.returned);
            if normal.is_empty() {
                break;
            }
        }
        Flow { normal, returned }
    }

    fn record_uninitialized(&self, value: ValueId, span: Span, paths: &[EvalPath]) {
        if self.globals.contains(&value) && paths.iter().any(|path| !path.assigned.contains(&value))
        {
            self.uninitialized_reads.borrow_mut().insert((value, span));
        }
    }
}

fn collapse_paths(paths: Vec<EvalPath>) -> Vec<EvalPath> {
    let mut groups: Vec<EvalPath> = Vec::new();
    for path in paths {
        if let Some(existing) = groups
            .iter_mut()
            .find(|existing| existing.shape_facts == path.shape_facts)
        {
            existing
                .assigned
                .retain(|global| path.assigned.contains(global));
        } else {
            groups.push(path);
        }
    }
    groups
}

#[derive(Clone, Default)]
struct FunctionRequirements {
    attachment_globals: HashSet<ValueId>,
    attempt_globals: HashSet<ValueId>,
    callees: HashSet<FunctionId>,
}

trait ScopedUseVisitor {
    /// Closure bodies execute only when invoked, not when the closure value is
    /// created. Lifecycle initializer classification therefore opts out while
    /// dependency analysis retains the conservative body traversal.
    fn visit_closure_bodies(&self) -> bool {
        true
    }

    fn global(&mut self, value: ValueId, span: Span, refined: Option<AttachmentShape>);
    fn global_write(&mut self, value: ValueId, span: Span, refined: Option<AttachmentShape>);
    fn call(&mut self, function: FunctionId, span: Span, refined: Option<AttachmentShape>);
}

fn infer_function_requirements(
    _syntax: &Program,
    hir: &TypedProgram,
    analysis: &ScopedGlobalAnalysis,
) -> HashMap<FunctionId, FunctionRequirements> {
    struct Collector<'a> {
        analysis: &'a ScopedGlobalAnalysis,
        facts: FunctionRequirements,
    }
    impl ScopedUseVisitor for Collector<'_> {
        fn global(&mut self, value: ValueId, _span: Span, refined: Option<AttachmentShape>) {
            if refined.is_none() {
                if self.analysis.is_attachment_global(value) {
                    self.facts.attachment_globals.insert(value);
                } else if self.analysis.is_attempt_global(value) {
                    self.facts.attempt_globals.insert(value);
                }
            }
        }

        fn global_write(&mut self, value: ValueId, _span: Span, refined: Option<AttachmentShape>) {
            if refined.is_none() {
                if self.analysis.is_attachment_global(value) {
                    self.facts.attachment_globals.insert(value);
                } else if self.analysis.is_attempt_global(value) {
                    self.facts.attempt_globals.insert(value);
                }
            }
        }

        fn call(&mut self, function: FunctionId, _span: Span, refined: Option<AttachmentShape>) {
            if refined.is_none() {
                self.facts.callees.insert(function);
            }
        }
    }

    let mut requirements = HashMap::new();
    for function in hir.function_bodies() {
        let mut collector = Collector {
            analysis,
            facts: FunctionRequirements::default(),
        };
        walk_block(&mut collector, &function.body, hir, Some(analysis), None);
        requirements.insert(function.function.function, collector.facts);
    }

    // A helper inherits every attachment value needed by a helper it calls.
    // This is the same viral shape as process effects, but retains the exact
    // globals so `onAttach` can eventually prove call-site initialization.
    loop {
        let previous = requirements.clone();
        let mut changed = false;
        for facts in requirements.values_mut() {
            for callee in facts.callees.clone() {
                if let Some(callee) = previous.get(&callee) {
                    let old_attachment_len = facts.attachment_globals.len();
                    let old_attempt_len = facts.attempt_globals.len();
                    facts
                        .attachment_globals
                        .extend(callee.attachment_globals.iter().copied());
                    facts
                        .attempt_globals
                        .extend(callee.attempt_globals.iter().copied());
                    changed |= facts.attachment_globals.len() != old_attachment_len
                        || facts.attempt_globals.len() != old_attempt_len;
                }
            }
        }
        if !changed {
            break;
        }
    }
    requirements
}

fn infer_action_attempt_requirements(
    hir: &TypedProgram,
    analysis: &ScopedGlobalAnalysis,
    requirements: &HashMap<FunctionId, FunctionRequirements>,
) -> HashSet<ActionKind> {
    struct Collector<'a> {
        analysis: &'a ScopedGlobalAnalysis,
        requirements: &'a HashMap<FunctionId, FunctionRequirements>,
        requires_attempt: bool,
    }

    impl ScopedUseVisitor for Collector<'_> {
        fn global(&mut self, value: ValueId, _span: Span, _refined: Option<AttachmentShape>) {
            self.requires_attempt |= self.analysis.is_attempt_global(value);
        }

        fn global_write(&mut self, value: ValueId, _span: Span, _refined: Option<AttachmentShape>) {
            self.requires_attempt |= self.analysis.is_attempt_global(value);
        }

        fn call(&mut self, function: FunctionId, _span: Span, _refined: Option<AttachmentShape>) {
            self.requires_attempt |= self
                .requirements
                .get(&function)
                .is_some_and(|facts| !facts.attempt_globals.is_empty());
        }
    }

    hir.action_bodies()
        .filter_map(|action| {
            let mut collector = Collector {
                analysis,
                requirements,
                requires_attempt: false,
            };
            walk_block(&mut collector, &action.body, hir, Some(analysis), None);
            collector.requires_attempt.then_some(action.action)
        })
        .collect()
}

fn validate_uses(
    syntax: &Program,
    hir: &TypedProgram,
    semantics: &SemanticModel,
    analysis: &ScopedGlobalAnalysis,
    requirements: &HashMap<FunctionId, FunctionRequirements>,
) -> Vec<Diagnostic> {
    struct Validator<'a> {
        syntax: &'a Program,
        analysis: &'a ScopedGlobalAnalysis,
        requirements: &'a HashMap<FunctionId, FunctionRequirements>,
        base: Vec<AttachmentShape>,
        detached_action: Option<ActionKind>,
        validate_attachment: bool,
        attempt_available: bool,
        attempt_context: String,
        diagnostics: Vec<Diagnostic>,
        seen: HashSet<(usize, usize, Option<ValueId>, Option<FunctionId>)>,
    }

    impl Validator<'_> {
        fn active(&self, refined: Option<AttachmentShape>) -> Vec<AttachmentShape> {
            refined.map_or_else(|| self.base.clone(), |shape| vec![shape])
        }

        fn global_name(&self, value: ValueId) -> &str {
            self.syntax
                .globals
                .iter()
                .find(|global| global.id == value)
                .map_or("attachment value", |global| global.name.as_str())
        }

        fn global_declaration_span(&self, value: ValueId) -> Option<Span> {
            self.syntax
                .globals
                .iter()
                .find(|global| global.id == value)
                .map(|global| global.name_span)
        }

        fn invalid_shapes(
            &self,
            value: ValueId,
            refined: Option<AttachmentShape>,
        ) -> Vec<AttachmentShape> {
            self.active(refined)
                .into_iter()
                .filter(|shape| !self.analysis.is_available_in(value, *shape))
                .collect()
        }

        fn shape_names(&self, shapes: &[AttachmentShape]) -> String {
            let names = shapes
                .iter()
                .map(|shape| {
                    let Some(facts) = self.analysis.shape_facts.get(shape) else {
                        return "the attachment".to_owned();
                    };
                    if facts.is_empty() {
                        return "the attachment".to_owned();
                    }
                    facts
                        .iter()
                        .filter_map(|fact| {
                            let ResolvedShapeDimension::Global(value) = fact.dimension else {
                                return None;
                            };
                            let global = self
                                .syntax
                                .globals
                                .iter()
                                .filter_map(|global| global.binding.simple_binding())
                                .find(|binding| binding.id == value)?;
                            let enumeration = self.syntax.enums.iter().find(|enumeration| {
                                enumeration
                                    .variants
                                    .iter()
                                    .any(|variant| variant.id == fact.variant)
                            })?;
                            let variant = enumeration
                                .variants
                                .iter()
                                .find(|variant| variant.id == fact.variant)?;
                            Some(format!(
                                "{} = {}.{}",
                                global.name, enumeration.name, variant.name
                            ))
                        })
                        .collect::<Vec<_>>()
                        .join(", ")
                })
                .collect::<Vec<_>>();
            match names.as_slice() {
                [] => String::new(),
                [one] => one.clone(),
                _ => names.join(", "),
            }
        }

        fn validate_global_access(
            &mut self,
            value: ValueId,
            span: Span,
            refined: Option<AttachmentShape>,
            access: &str,
        ) {
            if self.analysis.is_attempt_global(value) {
                if self.attempt_available
                    || !self.seen.insert((span.start, span.end, Some(value), None))
                {
                    return;
                }
                let name = self.global_name(value).to_owned();
                let mut diagnostic = Diagnostic::semantic(
                    format!(
                        "attempt-scoped global `{name}` is unavailable in {}",
                        self.attempt_context
                    ),
                    span,
                )
                .with_primary_label(format!(
                    "this {access} can occur before `onStart` initializes the value"
                ));
                if let Some(declaration) = self.global_declaration_span(value) {
                    diagnostic = diagnostic.with_secondary_label(
                        declaration,
                        "the attempt-scoped global is declared here",
                    );
                }
                self.diagnostics.push(diagnostic);
                return;
            }
            if !self.analysis.is_attachment_global(value) {
                return;
            }
            if !self.validate_attachment {
                return;
            }
            if let Some(action) = self.detached_action {
                if self.seen.insert((span.start, span.end, Some(value), None)) {
                    let name = self.global_name(value).to_owned();
                    let mut diagnostic = Diagnostic::semantic(
                        format!(
                            "attachment-scoped global `{name}` is unavailable in `{}`",
                            action.name()
                        ),
                        span,
                    )
                    .with_primary_label(format!(
                        "this {access} occurs without an attached process"
                    ));
                    if let Some(declaration) = self.global_declaration_span(value) {
                        diagnostic = diagnostic.with_secondary_label(
                            declaration,
                            "the attachment-scoped global is declared here",
                        );
                    }
                    self.diagnostics.push(diagnostic);
                }
                return;
            }
            let invalid = self.invalid_shapes(value, refined);
            if invalid.is_empty() || !self.seen.insert((span.start, span.end, Some(value), None)) {
                return;
            }
            let name = self.global_name(value).to_owned();
            let mut diagnostic = Diagnostic::semantic(
                format!(
                    "attachment-scoped global `{name}` is not initialized for {}",
                    self.shape_names(&invalid)
                ),
                span,
            )
            .with_primary_label(format!(
                "this {access} is not valid on every path that reaches it"
            ));
            if let Some(declaration) = self.global_declaration_span(value) {
                diagnostic = diagnostic.with_secondary_label(
                    declaration,
                    "the attachment-scoped global is declared here",
                );
            }
            self.diagnostics.push(diagnostic);
        }
    }

    impl ScopedUseVisitor for Validator<'_> {
        fn global(&mut self, value: ValueId, span: Span, refined: Option<AttachmentShape>) {
            self.validate_global_access(value, span, refined, "read");
        }

        fn global_write(&mut self, value: ValueId, span: Span, refined: Option<AttachmentShape>) {
            self.validate_global_access(value, span, refined, "write");
        }

        fn call(&mut self, function: FunctionId, span: Span, refined: Option<AttachmentShape>) {
            let Some(facts) = self.requirements.get(&function) else {
                return;
            };
            if !facts.attempt_globals.is_empty()
                && !self.attempt_available
                && self
                    .seen
                    .insert((span.start, span.end, None, Some(function)))
            {
                let name = self
                    .syntax
                    .functions
                    .iter()
                    .find(|candidate| candidate.id == function)
                    .map_or("function", |candidate| candidate.name.as_str());
                let mut diagnostic = Diagnostic::semantic(
                    format!(
                        "`{name}` requires attempt state unavailable in {}",
                        self.attempt_context
                    ),
                    span,
                )
                .with_primary_label(
                    "this helper may only be called after `onStart` initializes the attempt",
                );
                if let Some(declaration) = self
                    .syntax
                    .functions
                    .iter()
                    .find(|candidate| candidate.id == function)
                {
                    diagnostic = diagnostic.with_secondary_label(
                        declaration.name_span,
                        "the helper's attempt requirement originates here",
                    );
                }
                self.diagnostics.push(diagnostic);
            }
            let invalid = self
                .active(refined)
                .into_iter()
                .filter(|shape| {
                    facts
                        .attachment_globals
                        .iter()
                        .any(|global| !self.analysis.is_available_in(*global, *shape))
                })
                .collect::<Vec<_>>();
            if !self.validate_attachment {
                return;
            }
            if invalid.is_empty()
                || !self
                    .seen
                    .insert((span.start, span.end, None, Some(function)))
            {
                return;
            }
            let name = self
                .syntax
                .functions
                .iter()
                .find(|candidate| candidate.id == function)
                .map_or("function", |candidate| candidate.name.as_str());
            let mut diagnostic = Diagnostic::semantic(
                format!(
                    "`{name}` requires attachment values unavailable for {}",
                    self.shape_names(&invalid)
                ),
                span,
            )
            .with_primary_label("refine the attachment shape before calling this helper");
            if let Some(declaration) = self
                .syntax
                .functions
                .iter()
                .find(|candidate| candidate.id == function)
            {
                diagnostic = diagnostic.with_secondary_label(
                    declaration.name_span,
                    "the helper's attachment requirements originate here",
                );
            }
            self.diagnostics.push(diagnostic);
        }
    }

    let mut diagnostics = Vec::new();
    for action in hir.action_bodies() {
        // Candidate selection owns a temporary process handle, but it runs
        // before any attachment-scoped global has been initialized.
        let detached_action = (action.action == ActionKind::SelectProcess
            || !crate::effects::action_has_attached_process(action.action))
        .then_some(action.action);
        let mut validator = Validator {
            syntax,
            analysis,
            requirements,
            base: analysis.shapes.clone(),
            detached_action,
            validate_attachment: action.action != ActionKind::OnAttach,
            attempt_available: action_has_attempt_scope(action.action),
            attempt_context: format!("`{}`", action.action.name()),
            diagnostics: Vec::new(),
            seen: HashSet::new(),
        };
        walk_block(&mut validator, &action.body, hir, Some(analysis), None);
        diagnostics.extend(validator.diagnostics);
    }

    for function in hir.function_bodies() {
        let mut validator = Validator {
            syntax,
            analysis,
            requirements,
            base: analysis
                .function_shapes(function.function.function)
                .collect(),
            detached_action: None,
            validate_attachment: true,
            attempt_available: true,
            attempt_context: "this helper".to_owned(),
            diagnostics: Vec::new(),
            seen: HashSet::new(),
        };
        walk_block(&mut validator, &function.body, hir, Some(analysis), None);
        diagnostics.extend(validator.diagnostics);
    }

    if syntax.state.is_some() {
        for (field, expression) in hir.state_sources() {
            let shapes = semantics.state_field_shape_predicate(field).map_or_else(
                || analysis.shapes.clone(),
                |predicate| {
                    analysis
                        .shapes
                        .iter()
                        .copied()
                        .filter(|shape| analysis.shape_satisfies(*shape, predicate))
                        .collect()
                },
            );
            for shape in shapes {
                let mut validator = Validator {
                    syntax,
                    analysis,
                    requirements,
                    base: vec![shape],
                    detached_action: None,
                    validate_attachment: true,
                    attempt_available: false,
                    attempt_context: "state polling".to_owned(),
                    diagnostics: Vec::new(),
                    seen: HashSet::new(),
                };
                walk_expression(&mut validator, expression, hir, Some(analysis), Some(shape));
                diagnostics.extend(validator.diagnostics);
            }
        }
        for transform in hir.state_transforms() {
            let shapes = semantics
                .state_field_shape_predicate(transform.field)
                .map_or_else(
                    || analysis.shapes.clone(),
                    |predicate| {
                        analysis
                            .shapes
                            .iter()
                            .copied()
                            .filter(|shape| analysis.shape_satisfies(*shape, predicate))
                            .collect()
                    },
                );
            for shape in shapes {
                let mut validator = Validator {
                    syntax,
                    analysis,
                    requirements,
                    base: vec![shape],
                    detached_action: None,
                    validate_attachment: true,
                    attempt_available: false,
                    attempt_context: "a state transform".to_owned(),
                    diagnostics: Vec::new(),
                    seen: HashSet::new(),
                };
                walk_expression(
                    &mut validator,
                    transform.expression,
                    hir,
                    Some(analysis),
                    Some(shape),
                );
                diagnostics.extend(validator.diagnostics);
            }
        }
    }
    diagnostics
}

fn walk_block(
    visitor: &mut impl ScopedUseVisitor,
    block: &TypedBlock,
    hir: &TypedProgram,
    shapes: Option<&ScopedGlobalAnalysis>,
    refined: Option<AttachmentShape>,
) {
    for statement in &block.statements {
        match &statement.kind {
            TypedStatementKind::Variable { initializer, .. }
            | TypedStatementKind::Yield { value: initializer }
            | TypedStatementKind::Expression(initializer) => {
                walk_expression(visitor, *initializer, hir, shapes, refined)
            }
            TypedStatementKind::Assign {
                assignment,
                op,
                value,
            } => {
                if op.is_some() {
                    visitor.global(assignment.target, assignment.span, refined);
                } else {
                    visitor.global_write(assignment.target, assignment.span, refined);
                }
                walk_expression(visitor, *value, hir, shapes, refined);
            }
            TypedStatementKind::StateAssign { target, value, .. }
            | TypedStatementKind::IndexAssign { target, value, .. } => {
                walk_expression(visitor, *target, hir, shapes, refined);
                walk_expression(visitor, *value, hir, shapes, refined);
            }
            TypedStatementKind::If {
                condition,
                then_block,
                else_block,
            } => {
                walk_expression(visitor, *condition, hir, shapes, refined);
                walk_conditional_blocks(
                    visitor,
                    *condition,
                    then_block,
                    else_block.as_ref(),
                    hir,
                    shapes,
                    refined,
                );
            }
            TypedStatementKind::While { condition, body } => {
                walk_expression(visitor, *condition, hir, shapes, refined);
                walk_block(visitor, body, hir, shapes, refined);
            }
            TypedStatementKind::For { iterable, body, .. } => {
                walk_expression(visitor, *iterable, hir, shapes, refined);
                walk_block(visitor, body, hir, shapes, refined);
            }
            TypedStatementKind::Suspend { value, .. } => {
                walk_expression(visitor, *value, hir, shapes, refined)
            }
        }
    }
}

fn walk_expression(
    visitor: &mut impl ScopedUseVisitor,
    id: crate::ast::ExprId,
    hir: &TypedProgram,
    shapes: Option<&ScopedGlobalAnalysis>,
    refined: Option<AttachmentShape>,
) {
    let expression = hir
        .expression(id)
        .expect("attachment use analysis only visits typed expressions");
    if let Some((Some(ResolvedValue::Variable(value)), _)) = hir.value_path(id) {
        visitor.global(value, expression.span, refined);
    }
    if let Some(call) = hir.call(id) {
        if let Some(value) = call
            .receiver()
            .and_then(|receiver| receiver.path().map(|(root, _)| root))
            .and_then(ResolvedValue::source_value)
        {
            visitor.global(value, expression.span, refined);
        }
        if let ResolvedCall::UserFunction { function, .. }
        | ResolvedCall::UserMethod { function, .. } = call
        {
            visitor.call(*function, expression.span, refined);
        }
    }

    match &expression.kind {
        TypedExpressionKind::Block { statements, value } => {
            walk_block(visitor, statements, hir, shapes, refined);
            if let Some(value) = value {
                walk_expression(visitor, *value, hir, shapes, refined);
            }
        }
        TypedExpressionKind::Loop { body } => walk_block(visitor, body, hir, shapes, refined),
        TypedExpressionKind::Match { value, arms } => {
            walk_expression(visitor, *value, hir, shapes, refined);
            let dimension = hir
                .value_path(*value)
                .and_then(|(root, _)| root)
                .and_then(ResolvedValue::source_value)
                .map(ResolvedShapeDimension::Global);
            let mut remaining = active_shapes(shapes, refined);
            for arm in arms {
                if let Some(guard) = arm.guard {
                    walk_expression(visitor, guard, hir, shapes, refined);
                }
                let selected = match (
                    shapes,
                    dimension,
                    arm.resolution.variant,
                    arm.guard.is_none(),
                ) {
                    (
                        Some(shapes),
                        Some(dimension),
                        Some(ResolvedEnumVariantId::Source(variant)),
                        _,
                    ) => remaining
                        .iter()
                        .copied()
                        .filter(|shape| shapes.shape_has_fact(*shape, dimension, variant))
                        .collect::<Vec<_>>(),
                    (Some(_), Some(_), None, true) => remaining.clone(),
                    _ => Vec::new(),
                };
                if selected.is_empty() {
                    walk_expression(visitor, arm.value, hir, shapes, refined);
                } else {
                    for shape in &selected {
                        walk_expression(visitor, arm.value, hir, shapes, Some(*shape));
                    }
                    if arm.guard.is_none() {
                        remaining.retain(|shape| !selected.contains(shape));
                    }
                }
            }
        }
        TypedExpressionKind::If {
            condition,
            then_expr,
            else_expr,
        } => {
            walk_expression(visitor, *condition, hir, shapes, refined);
            walk_conditional_expressions(
                visitor, *condition, *then_expr, *else_expr, hir, shapes, refined,
            );
        }
        TypedExpressionKind::Fallback { value, fallback } => {
            walk_expression(visitor, *value, hir, shapes, refined);
            walk_expression(visitor, *fallback, hir, shapes, refined);
        }
        TypedExpressionKind::Closure { body, .. } => {
            if visitor.visit_closure_bodies() {
                walk_expression(visitor, *body, hir, shapes, refined);
            }
        }
        _ => {
            for child in expression_children(&expression.kind) {
                walk_expression(visitor, child, hir, shapes, refined);
            }
        }
    }
}

fn active_shapes(
    shapes: Option<&ScopedGlobalAnalysis>,
    refined: Option<AttachmentShape>,
) -> Vec<AttachmentShape> {
    refined.map_or_else(
        || shapes.map_or_else(Vec::new, |shapes| shapes.shapes.clone()),
        |shape| vec![shape],
    )
}

fn condition_value(
    hir: &TypedProgram,
    shapes: &ScopedGlobalAnalysis,
    expression: crate::ast::ExprId,
    shape: AttachmentShape,
) -> Option<bool> {
    let expression = hir.expression(expression)?;
    match &expression.kind {
        TypedExpressionKind::Is { value, pattern } => {
            let dimension = hir
                .value_path(*value)
                .and_then(|(root, _)| root)
                .and_then(ResolvedValue::source_value)
                .map(ResolvedShapeDimension::Global)?;
            let ResolvedEnumVariantId::Source(variant) = pattern.resolution.variant? else {
                return None;
            };
            Some(shapes.shape_has_fact(shape, dimension, variant))
        }
        TypedExpressionKind::Binary { op, left, right }
            if matches!(op, crate::ast::BinaryOp::Eq | crate::ast::BinaryOp::Ne) =>
        {
            let atom = |value, variant_expression| {
                let dimension = hir
                    .value_path(value)
                    .and_then(|(root, _)| root)
                    .and_then(ResolvedValue::source_value)
                    .map(ResolvedShapeDimension::Global)?;
                let ResolvedEnumVariantId::Source(variant) =
                    hir.enum_variant(variant_expression)?
                else {
                    return None;
                };
                Some(shapes.shape_has_fact(shape, dimension, variant))
            };
            let equal = atom(*left, *right).or_else(|| atom(*right, *left))?;
            Some(if *op == crate::ast::BinaryOp::Eq {
                equal
            } else {
                !equal
            })
        }
        TypedExpressionKind::Binary { op, left, right }
            if matches!(op, crate::ast::BinaryOp::And | crate::ast::BinaryOp::Or) =>
        {
            let left = condition_value(hir, shapes, *left, shape)?;
            let right = condition_value(hir, shapes, *right, shape)?;
            Some(if *op == crate::ast::BinaryOp::And {
                left && right
            } else {
                left || right
            })
        }
        TypedExpressionKind::Unary {
            op: crate::ast::UnaryOp::Not,
            expression,
        } => condition_value(hir, shapes, *expression, shape).map(|value| !value),
        _ => None,
    }
}

fn conditional_shapes(
    hir: &TypedProgram,
    shapes: Option<&ScopedGlobalAnalysis>,
    condition: crate::ast::ExprId,
    refined: Option<AttachmentShape>,
    expected: bool,
) -> Option<Vec<AttachmentShape>> {
    let shapes = shapes?;
    let active = active_shapes(Some(shapes), refined);
    let mut understood = false;
    let selected = active
        .into_iter()
        .filter(|shape| {
            condition_value(hir, shapes, condition, *shape).is_none_or(|value| {
                understood = true;
                value == expected
            })
        })
        .collect();
    understood.then_some(selected)
}

fn walk_conditional_blocks(
    visitor: &mut impl ScopedUseVisitor,
    condition: crate::ast::ExprId,
    then_block: &TypedBlock,
    else_block: Option<&TypedBlock>,
    hir: &TypedProgram,
    shapes: Option<&ScopedGlobalAnalysis>,
    refined: Option<AttachmentShape>,
) {
    let Some(then_shapes) = conditional_shapes(hir, shapes, condition, refined, true) else {
        walk_block(visitor, then_block, hir, shapes, refined);
        if let Some(else_block) = else_block {
            walk_block(visitor, else_block, hir, shapes, refined);
        }
        return;
    };
    for shape in then_shapes {
        walk_block(visitor, then_block, hir, shapes, Some(shape));
    }
    if let Some(else_block) = else_block {
        for shape in conditional_shapes(hir, shapes, condition, refined, false).unwrap_or_default()
        {
            walk_block(visitor, else_block, hir, shapes, Some(shape));
        }
    }
}

fn walk_conditional_expressions(
    visitor: &mut impl ScopedUseVisitor,
    condition: crate::ast::ExprId,
    then_expression: crate::ast::ExprId,
    else_expression: crate::ast::ExprId,
    hir: &TypedProgram,
    shapes: Option<&ScopedGlobalAnalysis>,
    refined: Option<AttachmentShape>,
) {
    let Some(then_shapes) = conditional_shapes(hir, shapes, condition, refined, true) else {
        walk_expression(visitor, then_expression, hir, shapes, refined);
        walk_expression(visitor, else_expression, hir, shapes, refined);
        return;
    };
    for shape in then_shapes {
        walk_expression(visitor, then_expression, hir, shapes, Some(shape));
    }
    for shape in conditional_shapes(hir, shapes, condition, refined, false).unwrap_or_default() {
        walk_expression(visitor, else_expression, hir, shapes, Some(shape));
    }
}

fn expression_children(kind: &TypedExpressionKind) -> Vec<crate::ast::ExprId> {
    match kind {
        TypedExpressionKind::InterpolatedString(parts) => parts
            .iter()
            .filter_map(|part| match part {
                crate::hir::TypedInterpolatedPart::Expression { expression, .. } => {
                    Some(*expression)
                }
                crate::hir::TypedInterpolatedPart::Text(_) => None,
            })
            .collect(),
        TypedExpressionKind::Array(values) => values.clone(),
        TypedExpressionKind::Range { start, end, .. } => vec![*start, *end],
        TypedExpressionKind::Struct { fields, .. } => {
            fields.iter().map(|(_, value)| *value).collect()
        }
        TypedExpressionKind::Enum { payload, .. } => payload.iter().copied().collect(),
        TypedExpressionKind::Is { value, .. } => vec![*value],
        TypedExpressionKind::Break(value) | TypedExpressionKind::Return(value) => {
            value.iter().copied().collect()
        }
        TypedExpressionKind::Throw { error, .. }
        | TypedExpressionKind::Inspect { value: error, .. }
        | TypedExpressionKind::Suspend { value: error, .. }
        | TypedExpressionKind::Propagate { value: error, .. }
        | TypedExpressionKind::Unary {
            expression: error, ..
        }
        | TypedExpressionKind::Cast {
            expression: error, ..
        } => vec![*error],
        TypedExpressionKind::Member { receiver, .. } => vec![*receiver],
        TypedExpressionKind::Index { receiver, index } => vec![*receiver, *index],
        TypedExpressionKind::Binary { left, right, .. } => vec![*left, *right],
        TypedExpressionKind::Call {
            receiver,
            arguments,
            ..
        } => receiver
            .iter()
            .copied()
            .chain(arguments.iter().copied())
            .collect(),
        TypedExpressionKind::Invoke { callee, arguments } => std::iter::once(*callee)
            .chain(arguments.iter().copied())
            .collect(),
        TypedExpressionKind::Closure { .. } => {
            unreachable!("closure bodies are handled before child collection")
        }
        TypedExpressionKind::None
        | TypedExpressionKind::IteratorEnd
        | TypedExpressionKind::Bool(_)
        | TypedExpressionKind::Int { .. }
        | TypedExpressionKind::Float(_)
        | TypedExpressionKind::Char(_)
        | TypedExpressionKind::String(_)
        | TypedExpressionKind::Signature(_)
        | TypedExpressionKind::Path(_)
        | TypedExpressionKind::Continue => Vec::new(),
        TypedExpressionKind::Block { .. }
        | TypedExpressionKind::Loop { .. }
        | TypedExpressionKind::Match { .. }
        | TypedExpressionKind::If { .. }
        | TypedExpressionKind::Fallback { .. } => {
            unreachable!("control-flow expressions are handled before child collection")
        }
    }
}
