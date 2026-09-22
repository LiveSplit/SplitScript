//! Resolved declaration index and typed body HIR.
//!
//! Lowering first establishes an inspectable declaration index. After checking,
//! typed expressions and blocks own body shape, child identities, types, and
//! type-directed resolutions without attaching them to syntax nodes.

use std::collections::{HashMap, HashSet};
use std::sync::Arc;

use crate::{
    ast::{
        ActionKind, AssignmentId, BinaryOp, Block, EnumId, EnumVariantId, Expr, ExprId, ExprKind,
        FunctionId, InterpolatedPart, ManagedClassId, ManagedFieldId, ManagedImageId,
        ManagedItemDecl, ManagedNamespaceId, MatchArm, MatchPattern, PatternBinding, PatternId,
        Program as SyntaxProgram, SettingChoiceOptionId, SettingKind, Span, Stmt, StructId,
        SuspensionMode, TypeRef, UnaryOp, ValueId,
    },
    semantic::{
        DynamicCallCallee, FunctionInstance, ResolvedCall, ResolvedEnumVariantId, ResolvedMember,
        ResolvedStructFieldId, ResolvedStructId, ResolvedValue, ResolvedWrapperPattern,
        SemanticModel, ValueConversion,
    },
    stdlib::{Implementation, StandardLibrary, StdlibItemId, StdlibTypeId},
    types::{EnumTypeId, TypeId, TypeKind},
    visit::{self, Visitor as SyntaxVisitor},
};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum DeclarationId {
    StateField(ValueId),
    Setting(ValueId),
    Global(ValueId),
    Struct(StructId),
    Enum(EnumId),
    ManagedImage(ManagedImageId),
    ManagedNamespace(ManagedNamespaceId),
    ManagedClass(ManagedClassId),
    ManagedField(ManagedFieldId),
    Function(FunctionId),
    Action(ActionKind),
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Declaration {
    pub id: DeclarationId,
    pub owner: Option<DeclarationId>,
    pub name: String,
    pub span: Span,
}

#[derive(Debug, Clone, Default)]
pub struct DeclarationIndex {
    declarations: Vec<Declaration>,
    by_name: HashMap<String, Vec<usize>>,
}

impl DeclarationIndex {
    pub(crate) fn lower(syntax: &SyntaxProgram) -> Self {
        let mut program = Self::default();
        if let Some(state) = &syntax.state {
            for field in state.all_fields() {
                program.push(
                    DeclarationId::StateField(field.id),
                    None,
                    &field.name,
                    field.span,
                );
            }
            if let (Some(value), Some(enumeration)) = (state.provider_value, &state.provider_enum) {
                program.push(
                    DeclarationId::Global(value),
                    None,
                    state
                        .refinement_value_name()
                        .expect("a refinement value has a source name"),
                    state.span,
                );
                program.push(
                    DeclarationId::Enum(enumeration.id),
                    None,
                    &enumeration.name,
                    enumeration.span,
                );
            }
        }
        for setting in &syntax.settings {
            if setting.source_visible {
                program.push(
                    DeclarationId::Setting(setting.id),
                    None,
                    &setting.name,
                    setting.span,
                );
            }
        }
        for global in &syntax.globals {
            let mut seen = HashSet::new();
            global.binding.visit_bindings(&mut |binding| {
                if seen.insert(binding.id) {
                    program.push(
                        DeclarationId::Global(binding.id),
                        None,
                        &binding.name,
                        binding.name_span,
                    );
                }
            });
        }
        for structure in &syntax.structs {
            program.push(
                DeclarationId::Struct(structure.id),
                None,
                &structure.name,
                structure.span,
            );
        }
        for enumeration in &syntax.enums {
            program.push(
                DeclarationId::Enum(enumeration.id),
                None,
                &enumeration.name,
                enumeration.span,
            );
        }
        for image in &syntax.managed_images {
            program.push(
                DeclarationId::ManagedImage(image.id),
                None,
                &image.name,
                image.span,
            );
            program.lower_managed_items(&image.items, DeclarationId::ManagedImage(image.id));
        }
        for function in &syntax.functions {
            program.push(
                DeclarationId::Function(function.id),
                None,
                &function.name,
                function.span,
            );
        }
        for action in &syntax.actions {
            program.push(
                DeclarationId::Action(action.kind),
                None,
                action.kind.name(),
                action.span,
            );
        }
        program
    }

    fn lower_managed_items(&mut self, items: &[ManagedItemDecl], owner: DeclarationId) {
        for item in items {
            match item {
                ManagedItemDecl::Namespace(namespace) => {
                    self.push(
                        DeclarationId::ManagedNamespace(namespace.id),
                        Some(owner),
                        &namespace.name,
                        namespace.span,
                    );
                    self.lower_managed_items(
                        &namespace.items,
                        DeclarationId::ManagedNamespace(namespace.id),
                    );
                }
                ManagedItemDecl::Class(class) => {
                    self.push(
                        DeclarationId::ManagedClass(class.id),
                        Some(owner),
                        &class.name,
                        class.span,
                    );
                    for field in class.all_fields() {
                        self.push(
                            DeclarationId::ManagedField(field.id),
                            Some(DeclarationId::ManagedClass(class.id)),
                            &field.name,
                            field.span,
                        );
                    }
                }
            }
        }
    }

    fn push(&mut self, id: DeclarationId, owner: Option<DeclarationId>, name: &str, span: Span) {
        let index = self.declarations.len();
        self.declarations.push(Declaration {
            id,
            owner,
            name: name.to_owned(),
            span,
        });
        self.by_name.entry(name.to_owned()).or_default().push(index);
    }

    pub fn declarations(&self) -> impl Iterator<Item = &Declaration> {
        self.declarations.iter()
    }

    pub fn declarations_named<'a>(
        &'a self,
        name: &'a str,
    ) -> impl Iterator<Item = &'a Declaration> + 'a {
        self.by_name
            .get(name)
            .into_iter()
            .flatten()
            .map(|index| &self.declarations[*index])
    }

    pub fn declaration(&self, id: DeclarationId) -> Option<&Declaration> {
        self.declarations
            .iter()
            .find(|declaration| declaration.id == id)
    }

    pub fn owner(&self, id: DeclarationId) -> Option<&Declaration> {
        let owner = self.declaration(id)?.owner?;
        self.declaration(owner)
    }

    pub fn children(&self, id: DeclarationId) -> impl Iterator<Item = &Declaration> {
        self.declarations
            .iter()
            .filter(move |declaration| declaration.owner == Some(id))
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ExpressionResolution {
    ValuePath {
        root: Option<ResolvedValue>,
        members: Vec<ResolvedMember>,
    },
    Member {
        members: Vec<ResolvedMember>,
    },
    Call(ResolvedCall),
    DynamicCall(DynamicCallCallee),
    FunctionValue(FunctionInstance),
    StructLiteral {
        structure: ResolvedStructId,
        fields: Vec<ResolvedStructFieldId>,
    },
    EnumConstructor {
        variant: ResolvedEnumVariantId,
    },
}

#[derive(Debug, Clone)]
pub enum TypedInterpolatedPart {
    Text(String),
    Expression {
        expression: ExprId,
        conversion: Option<ImplicitConversion>,
    },
}

/// A conversion inserted by type checking rather than written with `as`.
///
/// This conversion is specific to interpolation operands. General value
/// conversions such as lifting `T` into `T?` or `T!` are recorded on
/// [`TypedExpression::conversion`] with semantic source and target types.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ImplicitConversion {
    ToString { source: TypeId },
}

#[derive(Debug, Clone)]
pub struct TypedMatchArm {
    pub pattern: TypedPattern,
    pub resolution: ResolvedPattern,
    pub guard: Option<ExprId>,
    pub value: ExprId,
    pub span: Span,
}

#[derive(Debug, Clone)]
pub struct TypedPatternNode {
    pub pattern: TypedPattern,
    pub resolution: ResolvedPattern,
}

#[derive(Debug, Clone)]
pub struct TypedBindingPattern {
    /// The single incoming value supplied by the declaration site.
    pub value: ValueId,
    pub pattern: TypedPatternNode,
}

#[derive(Debug, Clone)]
pub struct TypedStructPatternField {
    pub field: ResolvedStructFieldId,
    pub pattern: TypedPatternNode,
}

#[derive(Debug, Clone)]
pub enum TypedPattern {
    Struct {
        structure: ResolvedStructId,
        fields: Vec<TypedStructPatternField>,
    },
    Enum {
        enumeration: EnumTypeId,
        variant: String,
        payload: Option<Box<TypedPatternNode>>,
    },
    Bool(bool),
    Char(char),
    String(String),
    Int {
        value: u64,
        negative: bool,
        suffix: Option<TypeRef>,
    },
    IntRange {
        start: u64,
        start_negative: bool,
        start_suffix: Option<TypeRef>,
        end: u64,
        end_negative: bool,
        end_suffix: Option<TypeRef>,
        kind: crate::ast::RangeKind,
    },
    FileVersion([u16; 4]),
    None,
    OptionSome(Box<TypedPatternNode>),
    IteratorEnd,
    IteratorItem(Box<TypedPatternNode>),
    ResultSuccess(Box<TypedPatternNode>),
    ResultError(Box<TypedPatternNode>),
    Array {
        prefix: Vec<TypedPatternNode>,
        rest: bool,
        suffix: Vec<TypedPatternNode>,
    },
    Alternation(Vec<TypedPatternNode>),
    Binding(PatternBinding),
    Wildcard,
}

impl TypedPattern {
    pub fn visit_bindings(&self, visitor: &mut impl FnMut(&PatternBinding)) {
        match self {
            Self::Struct { fields, .. } => {
                for field in fields {
                    field.pattern.pattern.visit_bindings(visitor);
                }
            }
            Self::Enum {
                payload: Some(payload),
                ..
            }
            | Self::OptionSome(payload)
            | Self::IteratorItem(payload)
            | Self::ResultSuccess(payload)
            | Self::ResultError(payload) => payload.pattern.visit_bindings(visitor),
            Self::Array { prefix, suffix, .. } => {
                for element in prefix.iter().chain(suffix) {
                    element.pattern.visit_bindings(visitor);
                }
            }
            Self::Alternation(alternatives) => {
                for alternative in alternatives {
                    alternative.pattern.visit_bindings(visitor);
                }
            }
            Self::Binding(binding) => visitor(binding),
            Self::Enum { payload: None, .. }
            | Self::Bool(_)
            | Self::Char(_)
            | Self::String(_)
            | Self::Int { .. }
            | Self::IntRange { .. }
            | Self::FileVersion(_)
            | Self::None
            | Self::IteratorEnd
            | Self::Wildcard => {}
        }
    }
}

#[derive(Debug, Clone)]
pub enum TypedExpressionKind {
    None,
    IteratorEnd,
    Bool(bool),
    Int {
        value: u64,
        negative: bool,
        suffix: Option<TypeRef>,
    },
    Float(crate::ast::FloatLiteral),
    Char(char),
    String(String),
    InterpolatedString(Vec<TypedInterpolatedPart>),
    Signature(String),
    Array(Vec<ExprId>),
    Range {
        start: ExprId,
        end: ExprId,
        kind: crate::ast::RangeKind,
    },
    Block {
        statements: TypedBlock,
        value: Option<ExprId>,
    },
    Loop {
        body: TypedBlock,
    },
    Struct {
        structure: ResolvedStructId,
        fields: Vec<(String, ExprId)>,
    },
    Enum {
        enumeration: EnumTypeId,
        variant: String,
        payload: Option<ExprId>,
    },
    Match {
        value: ExprId,
        arms: Vec<TypedMatchArm>,
    },
    Is {
        value: ExprId,
        pattern: TypedPatternNode,
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
    Suspend {
        mode: SuspensionMode,
        destination: ValueId,
        value: ExprId,
    },
    /// Unwraps a result or transfers its error to the nearest failure boundary.
    Propagate {
        value: ExprId,
        target: FailureTarget,
    },
    Path(Vec<String>),
    Member {
        receiver: ExprId,
        name: String,
        name_span: Span,
    },
    Index {
        receiver: ExprId,
        index: ExprId,
    },
    Unary {
        op: UnaryOp,
        expression: ExprId,
    },
    Cast {
        expression: ExprId,
        target: TypeRef,
    },
    Binary {
        op: BinaryOp,
        left: ExprId,
        right: ExprId,
    },
    Call {
        source_path: Vec<String>,
        receiver: Option<ExprId>,
        arguments: Vec<ExprId>,
    },
    Invoke {
        callee: ExprId,
        arguments: Vec<ExprId>,
    },
    Closure {
        parameters: Vec<TypedBindingPattern>,
        body: ExprId,
    },
}

/// The control-flow destination selected for postfix `?` and `throw`.
///
/// Return boundaries complete a function or state poll with an error. Retry
/// boundaries instead abort only the current synchronous attempt and leave its
/// enclosing async poll pending for the next tick.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum FailureTarget {
    Return(TypeId),
    Retry { expression: ExprId, result: TypeId },
}

impl FailureTarget {
    pub fn result(self) -> TypeId {
        match self {
            Self::Return(result) | Self::Retry { result, .. } => result,
        }
    }
}

#[derive(Debug, Clone)]
pub struct TypedExpression {
    pub id: ExprId,
    pub ty: TypeId,
    pub kind: TypedExpressionKind,
    pub resolution: Option<ExpressionResolution>,
    pub conversion: Option<ValueConversion>,
    pub span: Span,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ResolvedAssignment {
    pub id: AssignmentId,
    pub target: ValueId,
    pub operator: Option<ResolvedCall>,
    pub span: Span,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ResolvedIndexAssignment {
    pub id: AssignmentId,
    pub operator: ResolvedCall,
    pub setter: Option<ResolvedCall>,
    pub span: Span,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ResolvedPattern {
    pub id: PatternId,
    pub variant: Option<ResolvedEnumVariantId>,
    pub wrapper: Option<ResolvedWrapperPattern>,
    pub span: Span,
}

#[derive(Debug, Clone)]
pub enum TypedStatementKind {
    Variable {
        binding: TypedBindingPattern,
        initializer: ExprId,
    },
    Assign {
        assignment: ResolvedAssignment,
        op: Option<BinaryOp>,
        value: ExprId,
    },
    StateAssign {
        assignment: ResolvedAssignment,
        target: ExprId,
        op: Option<BinaryOp>,
        value: ExprId,
    },
    IndexAssign {
        assignment: ResolvedIndexAssignment,
        target: ExprId,
        op: BinaryOp,
        value: ExprId,
    },
    If {
        condition: ExprId,
        then_block: TypedBlock,
        else_block: Option<TypedBlock>,
    },
    While {
        condition: ExprId,
        body: TypedBlock,
    },
    For {
        binding: TypedBindingPattern,
        iterable_value: ValueId,
        index_value: ValueId,
        version_value: ValueId,
        iterable: ExprId,
        body: TypedBlock,
    },
    Suspend {
        mode: SuspensionMode,
        binding: Option<ValueId>,
        returns: bool,
        value: ExprId,
    },
    Yield {
        value: ExprId,
    },
    Expression(ExprId),
}

#[derive(Debug, Clone)]
pub struct TypedStatement {
    pub kind: TypedStatementKind,
    pub debug_only: bool,
    pub span: Span,
}

#[derive(Debug, Clone)]
pub struct TypedBlock {
    pub statements: Vec<TypedStatement>,
    pub span: Span,
}

#[derive(Debug, Clone)]
pub struct FunctionBody {
    pub function: FunctionInstance,
    pub debug_only: bool,
    pub parameters: Vec<TypedBindingPattern>,
    pub body: TypedBlock,
}

#[derive(Debug, Clone)]
pub struct ActionBody {
    pub action: ActionKind,
    pub body: TypedBlock,
}

#[derive(Debug, Clone)]
pub struct GlobalInitializer {
    pub binding: TypedBindingPattern,
    pub expression: ExprId,
    pub debug_only: bool,
}

#[derive(Debug, Clone, Copy)]
pub struct StateTransform {
    pub field: ValueId,
    pub value: ValueId,
    pub expression: ExprId,
}

#[derive(Debug, Clone)]
pub struct TypedProgram {
    standard_library: StandardLibrary,
    declarations: Arc<DeclarationIndex>,
    expressions: Vec<TypedExpression>,
    expression_positions: Vec<usize>,
    assignments: Vec<ResolvedAssignment>,
    patterns: Vec<ResolvedPattern>,
    function_bodies: Vec<FunctionBody>,
    action_bodies: Vec<ActionBody>,
    global_initializers: Vec<GlobalInitializer>,
    /// Top-level declarations without a source initializer. A later scoped
    /// initialization analysis classifies each one by the lifecycle action
    /// that definitely assigns it.
    bare_globals: Vec<(ValueId, bool)>,
    state_sources: Vec<(ValueId, ExprId)>,
    state_transforms: Vec<StateTransform>,
    setting_choice_defaults: HashMap<ValueId, EnumVariantId>,
    setting_choice_options: HashMap<SettingChoiceOptionId, EnumVariantId>,
    visible_expression_count: usize,
    visible_function_count: usize,
    library_bodies_expected: bool,
    library_functions: HashMap<StdlibItemId, Vec<FunctionId>>,
}

pub(crate) fn visible_expression_count(program: &SyntaxProgram) -> usize {
    #[derive(Default)]
    struct Counter {
        count: usize,
    }

    impl<'ast> SyntaxVisitor<'ast> for Counter {
        fn visit_expr(&mut self, expression: &'ast Expr) {
            self.count = self.count.max(expression.id.index() + 1);
            visit::walk_expr(self, expression);
        }
    }

    let mut counter = Counter::default();
    counter.visit_program(program);
    counter.count
}

impl TypedProgram {
    pub(crate) fn build(
        declarations: Arc<DeclarationIndex>,
        syntax: &SyntaxProgram,
        semantics: &SemanticModel,
        standard_library: StandardLibrary,
        library_bodies_expected: bool,
        visible_expression_count: usize,
        visible_function_count: usize,
    ) -> Self {
        let mut builder = TypedBodyBuilder {
            semantics,
            syntax,
            standard_library: &standard_library,
            expressions: HashMap::new(),
            assignments: HashMap::new(),
            patterns: HashMap::new(),
        };
        builder.visit_program(syntax);
        let mut expressions = builder.expressions.into_values().collect::<Vec<_>>();
        // Sort compact keys first so comparisons do not repeatedly move large HIR records.
        expressions.sort_by_cached_key(|expression| expression.id.index());
        let expression_positions =
            crate::expression_index::expression_positions(&expressions, |expression| expression.id);
        let mut assignments = builder.assignments.into_values().collect::<Vec<_>>();
        assignments.sort_by_key(|assignment| assignment.id.index());
        let mut patterns = builder.patterns.into_values().collect::<Vec<_>>();
        patterns.sort_by_key(|pattern| pattern.id.index());
        let function_bodies = syntax
            .functions
            .iter()
            .map(|function| FunctionBody {
                function: FunctionInstance::monomorphic(function.id),
                debug_only: function.debug_only,
                parameters: function
                    .params
                    .iter()
                    .map(|parameter| TypedBindingPattern {
                        value: parameter.id,
                        pattern: lower_pattern_node(
                            &parameter.binding.pattern,
                            semantics,
                            syntax,
                            &standard_library,
                        ),
                    })
                    .collect(),
                body: lower_block(&function.body, semantics, syntax, &standard_library),
            })
            .collect();
        let action_bodies = syntax
            .actions
            .iter()
            .map(|action| ActionBody {
                action: action.kind,
                body: lower_block(&action.body, semantics, syntax, &standard_library),
            })
            .collect();
        let global_initializers = syntax
            .globals
            .iter()
            .filter_map(|global| {
                global.value.as_ref().map(|value| GlobalInitializer {
                    binding: TypedBindingPattern {
                        value: global.id,
                        pattern: lower_pattern_node(
                            &global.binding.pattern,
                            semantics,
                            syntax,
                            &standard_library,
                        ),
                    },
                    expression: value.id,
                    debug_only: global.debug_only,
                })
            })
            .collect();
        let bare_globals = syntax
            .globals
            .iter()
            .filter(|global| global.value.is_none())
            .map(|global| (global.id, global.debug_only))
            .collect();
        let state_sources = syntax
            .state
            .iter()
            .flat_map(|state| state.all_fields())
            .filter_map(|field| match &field.source {
                crate::ast::StateSource::Expression(expression) => Some((field.id, expression.id)),
                crate::ast::StateSource::Pointer(_) => None,
            })
            .collect();
        let state_transforms = syntax
            .state
            .iter()
            .flat_map(|state| state.all_fields())
            .filter_map(|field| {
                field.transform.as_ref().map(|transform| StateTransform {
                    field: field.id,
                    value: transform.value,
                    expression: transform.expression.id,
                })
            })
            .collect();

        let mut setting_choice_defaults = HashMap::new();
        let mut setting_choice_options = HashMap::new();
        for setting in &syntax.settings {
            if let SettingKind::Choice { options, .. } = &setting.kind {
                setting_choice_defaults.insert(
                    setting.id,
                    semantics
                        .setting_choice_default(setting.id)
                        .expect("checked choice settings have resolved defaults"),
                );
                for option in options {
                    setting_choice_options.insert(
                        option.id,
                        semantics
                            .setting_choice_option(option.id)
                            .expect("checked choice options have resolved variants"),
                    );
                }
            }
        }

        // After the full catalog bootstrap, augmentation injects only the
        // conservative dependency closure for this compilation.
        let all_library_bodies_expected =
            library_bodies_expected && !standard_library.source_body_operations_are_initialized();
        let functions_by_name = syntax
            .functions
            .iter()
            .map(|function| (function.name.as_str(), function.id))
            .collect::<HashMap<_, _>>();
        let library_functions = standard_library
            .all_items()
            .iter()
            .filter_map(|item| {
                let function_names = match item.implementation {
                    Implementation::Intrinsic(_) | Implementation::CapabilityRequirement => {
                        return None;
                    }
                    Implementation::LibraryBody { function_name, .. } => vec![function_name],
                    Implementation::LibraryOverloads { cases, .. } => {
                        cases.iter().map(|case| case.function_name).collect()
                    }
                };
                let functions = function_names
                    .into_iter()
                    .map(|function_name| {
                        let declaration = functions_by_name.get(function_name).copied();
                        if all_library_bodies_expected {
                            Some(
                                declaration
                                    .expect("injected library bodies have parsed declarations"),
                            )
                        } else {
                            declaration
                        }
                    })
                    .collect::<Option<Vec<_>>>()?;
                Some((item.id, functions))
            })
            .collect();

        Self {
            standard_library,
            declarations,
            expressions,
            expression_positions,
            assignments,
            patterns,
            function_bodies,
            action_bodies,
            global_initializers,
            bare_globals,
            state_sources,
            state_transforms,
            setting_choice_defaults,
            setting_choice_options,
            visible_expression_count,
            visible_function_count,
            library_bodies_expected,
            library_functions,
        }
    }

    pub fn declarations(&self) -> &DeclarationIndex {
        &self.declarations
    }

    pub(crate) fn declarations_arc(&self) -> Arc<DeclarationIndex> {
        Arc::clone(&self.declarations)
    }

    pub fn standard_library(&self) -> &StandardLibrary {
        &self.standard_library
    }

    pub(crate) fn expects_library_bodies(&self) -> bool {
        self.library_bodies_expected
    }

    /// Resolves a catalog-owned source implementation to its inferred hidden
    /// function template. Concrete calls instantiate this declaration through
    /// the same `FunctionInstance` path as user-authored generic functions.
    pub fn library_function(&self, item: StdlibItemId) -> Option<FunctionId> {
        self.library_functions
            .get(&item)
            .and_then(|functions| (functions.len() == 1).then_some(functions[0]))
    }

    pub fn library_overload_function(&self, item: StdlibItemId, case: usize) -> Option<FunctionId> {
        self.library_functions
            .get(&item)
            .and_then(|functions| functions.get(case))
            .copied()
    }

    pub fn library_functions(&self, item: StdlibItemId) -> impl Iterator<Item = FunctionId> + '_ {
        self.library_functions
            .get(&item)
            .into_iter()
            .flat_map(|functions| functions.iter().copied())
    }

    pub fn expression(&self, id: ExprId) -> Option<&TypedExpression> {
        if !self.expression_positions.is_empty() {
            return self
                .expression_positions
                .get(id.index())
                .and_then(|&position| self.expressions.get(position));
        }
        self.expressions
            .binary_search_by_key(&id.index(), |expression| expression.id.index())
            .ok()
            .map(|index| &self.expressions[index])
    }

    pub fn expressions(&self) -> impl Iterator<Item = &TypedExpression> {
        self.expressions
            .iter()
            .filter(|expression| expression.id.index() < self.visible_expression_count)
    }

    pub(crate) fn all_expressions(&self) -> impl Iterator<Item = &TypedExpression> {
        self.expressions.iter()
    }

    pub(crate) fn visible_expression_count(&self) -> usize {
        self.visible_expression_count
    }

    pub(crate) fn visible_function_count(&self) -> usize {
        self.visible_function_count
    }

    pub fn call(&self, id: ExprId) -> Option<&ResolvedCall> {
        match &self.expression(id)?.resolution {
            Some(ExpressionResolution::Call(call)) => Some(call),
            _ => None,
        }
    }

    pub fn value_path(&self, id: ExprId) -> Option<(Option<ResolvedValue>, &[ResolvedMember])> {
        match &self.expression(id)?.resolution {
            Some(ExpressionResolution::ValuePath { root, members }) => {
                Some((*root, members.as_slice()))
            }
            _ => None,
        }
    }

    pub fn struct_literal_fields(&self, id: ExprId) -> Option<&[ResolvedStructFieldId]> {
        match &self.expression(id)?.resolution {
            Some(ExpressionResolution::StructLiteral { fields, .. }) => Some(fields),
            _ => None,
        }
    }

    pub fn enum_variant(&self, id: ExprId) -> Option<ResolvedEnumVariantId> {
        match &self.expression(id)?.resolution {
            Some(ExpressionResolution::EnumConstructor { variant }) => Some(*variant),
            _ => None,
        }
    }

    pub fn assignment(&self, id: AssignmentId) -> Option<ResolvedAssignment> {
        self.assignments
            .binary_search_by_key(&id.index(), |assignment| assignment.id.index())
            .ok()
            .map(|index| self.assignments[index].clone())
    }

    pub fn pattern(&self, id: PatternId) -> Option<ResolvedPattern> {
        self.patterns
            .binary_search_by_key(&id.index(), |pattern| pattern.id.index())
            .ok()
            .map(|index| self.patterns[index])
    }

    pub fn patterns(&self) -> impl Iterator<Item = ResolvedPattern> + '_ {
        self.patterns.iter().copied()
    }

    pub fn function_body(&self, function: FunctionId) -> Option<&TypedBlock> {
        self.function_bodies
            .iter()
            .find(|body| {
                body.function.function == function && body.function.type_arguments.is_empty()
            })
            .map(|body| &body.body)
    }

    pub fn function_instance_body(&self, function: &FunctionInstance) -> Option<&TypedBlock> {
        self.function_bodies
            .iter()
            .find(|body| &body.function == function)
            .map(|body| &body.body)
    }

    pub fn action_body(&self, action: ActionKind) -> Option<&TypedBlock> {
        self.action_bodies
            .iter()
            .find(|body| body.action == action)
            .map(|body| &body.body)
    }

    pub fn function_bodies(&self) -> impl Iterator<Item = &FunctionBody> {
        self.function_bodies
            .iter()
            .filter(|body| body.function.function.index() < self.visible_function_count)
    }

    pub(crate) fn all_function_bodies(&self) -> impl Iterator<Item = &FunctionBody> {
        self.function_bodies.iter()
    }

    pub fn action_bodies(&self) -> impl Iterator<Item = &ActionBody> {
        self.action_bodies.iter()
    }

    pub fn global_initializers(&self) -> impl Iterator<Item = &GlobalInitializer> + '_ {
        self.global_initializers.iter()
    }

    /// Globals whose storage is initialized by a successful `onAttach` run
    /// and cleared when that attachment ends.
    pub fn bare_globals(&self) -> impl Iterator<Item = ValueId> + '_ {
        self.bare_globals.iter().map(|(value, _)| *value)
    }

    pub(crate) fn bare_globals_with_debug(&self) -> impl Iterator<Item = (ValueId, bool)> + '_ {
        self.bare_globals.iter().copied()
    }

    pub fn state_sources(&self) -> impl Iterator<Item = (ValueId, ExprId)> + '_ {
        self.state_sources.iter().copied()
    }

    pub fn state_transforms(&self) -> impl Iterator<Item = StateTransform> + '_ {
        self.state_transforms.iter().copied()
    }

    pub fn setting_choice_default(&self, setting: ValueId) -> Option<EnumVariantId> {
        self.setting_choice_defaults.get(&setting).copied()
    }

    pub fn setting_choice_option(&self, option: SettingChoiceOptionId) -> Option<EnumVariantId> {
        self.setting_choice_options.get(&option).copied()
    }
}

#[cfg(test)]
mod lookup_tests {
    use super::*;

    #[test]
    fn expression_index_preserves_ids_and_holes() {
        let checked =
            crate::check(crate::lower(crate::parse("state \"game.exe\" {}").unwrap())).unwrap();
        let hir = checked.typed_hir();
        assert!(!hir.expression_positions.is_empty());
        for index in 0..hir.expression_positions.len() + 2 {
            let id = ExprId::from_index(index as u32);
            let expected = hir
                .expressions
                .binary_search_by_key(&index, |expression| expression.id.index())
                .ok()
                .map(|position| hir.expressions[position].id);
            assert_eq!(hir.expression(id).map(|expression| expression.id), expected);
        }
    }

    #[test]
    fn sparse_and_empty_expression_arenas_keep_bounded_lookups() {
        let checked =
            crate::check(crate::lower(crate::parse("state \"game.exe\" {}").unwrap())).unwrap();
        let mut hir = checked.typed_hir().clone();
        hir.expressions.truncate(1);
        let id = ExprId::from_index(u32::MAX);
        hir.expressions[0].id = id;
        hir.expression_positions =
            crate::expression_index::expression_positions(&hir.expressions, |expression| {
                expression.id
            });
        assert!(hir.expression_positions.is_empty());
        assert_eq!(hir.expression(id).map(|expression| expression.id), Some(id));
        assert!(hir.expression(ExprId::from_index(0)).is_none());
        hir.expressions.clear();
        hir.expression_positions =
            crate::expression_index::expression_positions(&hir.expressions, |expression| {
                expression.id
            });
        assert!(hir.expression(id).is_none());
    }
}

struct TypedBodyBuilder<'a> {
    semantics: &'a SemanticModel,
    syntax: &'a SyntaxProgram,
    standard_library: &'a StandardLibrary,
    expressions: HashMap<ExprId, TypedExpression>,
    assignments: HashMap<AssignmentId, ResolvedAssignment>,
    patterns: HashMap<PatternId, ResolvedPattern>,
}

impl TypedBodyBuilder<'_> {
    fn insert_pattern(&mut self, pattern: &MatchPattern, id: PatternId, span: Span) {
        self.patterns.insert(
            id,
            ResolvedPattern {
                id,
                variant: self.semantics.pattern_variant(id),
                wrapper: self.semantics.wrapper_pattern(id),
                span,
            },
        );
        match pattern {
            MatchPattern::Struct { fields, .. } => {
                for field in fields {
                    self.insert_pattern(&field.pattern.kind, field.pattern.id, field.pattern.span);
                }
            }
            MatchPattern::Enum {
                payload: Some(payload),
                ..
            }
            | MatchPattern::OptionSome(payload)
            | MatchPattern::IteratorItem(payload)
            | MatchPattern::ResultSuccess(payload)
            | MatchPattern::ResultError(payload) => {
                self.insert_pattern(&payload.kind, payload.id, payload.span);
            }
            MatchPattern::Array(array) => {
                for element in array.elements() {
                    self.insert_pattern(&element.kind, element.id, element.span);
                }
            }
            MatchPattern::Alternation(elements) => {
                for element in elements {
                    self.insert_pattern(&element.kind, element.id, element.span);
                }
            }
            _ => {}
        }
    }
}

impl<'ast> SyntaxVisitor<'ast> for TypedBodyBuilder<'_> {
    fn visit_parameter(&mut self, parameter: &'ast crate::ast::Parameter) {
        self.insert_pattern(
            &parameter.binding.pattern.kind,
            parameter.binding.pattern.id,
            parameter.binding.pattern.span,
        );
        visit::walk_parameter(self, parameter);
    }

    fn visit_variable(&mut self, variable: &'ast crate::ast::VariableDecl) {
        self.insert_pattern(
            &variable.binding.pattern.kind,
            variable.binding.pattern.id,
            variable.binding.pattern.span,
        );
        visit::walk_variable(self, variable);
    }

    fn visit_for_binding(&mut self, binding: &'ast crate::ast::ForBinding) {
        self.insert_pattern(
            &binding.binding.pattern.kind,
            binding.binding.pattern.id,
            binding.binding.pattern.span,
        );
        visit::Visitor::visit_pattern(self, &binding.binding.pattern.kind);
    }

    fn visit_stmt(&mut self, statement: &'ast Stmt) {
        if let Stmt::Assign { id, span, .. } | Stmt::StateAssign { id, span, .. } = statement {
            self.assignments.insert(
                *id,
                ResolvedAssignment {
                    id: *id,
                    target: self
                        .semantics
                        .assignment_target(*id)
                        .expect("checked assignments have resolved targets"),
                    operator: self.semantics.assignment_call(*id).cloned(),
                    span: *span,
                },
            );
        }
        visit::walk_stmt(self, statement);
    }

    fn visit_expr(&mut self, expression: &'ast Expr) {
        if let ExprKind::Is { pattern, .. } = &expression.kind {
            self.insert_pattern(&pattern.kind, pattern.id, pattern.span);
        }
        let resolution = if let Some(variant) = self.semantics.enum_variant(expression.id) {
            Some(ExpressionResolution::EnumConstructor { variant })
        } else if let Some(call) = self.semantics.call(expression.id) {
            Some(ExpressionResolution::Call(call.clone()))
        } else if let Some(callee) = self.semantics.dynamic_call_callee(expression.id) {
            Some(ExpressionResolution::DynamicCall(callee))
        } else if let Some(function) = self.semantics.function_value(expression.id) {
            Some(ExpressionResolution::FunctionValue(function.clone()))
        } else {
            match &expression.kind {
                ExprKind::Path(_) => Some(ExpressionResolution::ValuePath {
                    root: self.semantics.value(expression.id),
                    members: self
                        .semantics
                        .path_members(expression.id)
                        .unwrap_or_default()
                        .to_vec(),
                }),
                ExprKind::Member { .. } => Some(ExpressionResolution::Member {
                    members: self
                        .semantics
                        .path_members(expression.id)
                        .unwrap_or_default()
                        .to_vec(),
                }),
                ExprKind::Call { .. } => None,
                ExprKind::Struct { .. } => Some(ExpressionResolution::StructLiteral {
                    structure: self
                        .semantics
                        .struct_literal(expression.id)
                        .expect("checked struct literals have resolved nominal identities"),
                    fields: self
                        .semantics
                        .struct_literal_fields(expression.id)
                        .expect("checked struct literals have resolved fields")
                        .to_vec(),
                }),
                _ => None,
            }
        };
        self.expressions.insert(
            expression.id,
            TypedExpression {
                id: expression.id,
                ty: self
                    .semantics
                    .expression_type(expression.id)
                    .expect("checked expressions have resolved types"),
                kind: lower_expression_kind(
                    expression,
                    self.semantics,
                    self.syntax,
                    self.standard_library,
                ),
                resolution,
                conversion: self.semantics.value_conversion(expression.id),
                span: expression.span,
            },
        );
        visit::walk_expr(self, expression);
    }

    fn visit_match_arm(&mut self, arm: &'ast MatchArm) {
        self.insert_pattern(&arm.pattern, arm.pattern_id, arm.span);
        visit::walk_match_arm(self, arm);
    }
}

pub trait TypedVisitor: Sized {
    fn visit_program(&mut self, program: &TypedProgram) {
        walk_typed_program(self, program);
    }

    fn visit_block(&mut self, block: &TypedBlock, program: &TypedProgram) {
        walk_typed_block(self, block, program);
    }

    fn visit_statement(&mut self, statement: &TypedStatement, program: &TypedProgram) {
        walk_typed_statement(self, statement, program);
    }

    fn visit_expression(&mut self, expression: &TypedExpression, program: &TypedProgram) {
        walk_typed_expression(self, expression, program);
    }

    fn visit_match_arm(&mut self, arm: &TypedMatchArm, program: &TypedProgram) {
        walk_typed_match_arm(self, arm, program);
    }
}

pub fn walk_typed_program<V: TypedVisitor>(visitor: &mut V, program: &TypedProgram) {
    for initializer in program.global_initializers() {
        visitor.visit_expression(
            program
                .expression(initializer.expression)
                .expect("global initializer belongs to typed HIR"),
            program,
        );
    }
    for (_, expression) in program.state_sources() {
        visitor.visit_expression(
            program
                .expression(expression)
                .expect("state source belongs to typed HIR"),
            program,
        );
    }
    for transform in program.state_transforms() {
        visitor.visit_expression(
            program
                .expression(transform.expression)
                .expect("state transform belongs to typed HIR"),
            program,
        );
    }
    for function in program.function_bodies() {
        visitor.visit_block(&function.body, program);
    }
    for action in program.action_bodies() {
        visitor.visit_block(&action.body, program);
    }
}

pub fn walk_typed_block<V: TypedVisitor>(
    visitor: &mut V,
    block: &TypedBlock,
    program: &TypedProgram,
) {
    for statement in &block.statements {
        visitor.visit_statement(statement, program);
    }
}

pub fn walk_typed_statement<V: TypedVisitor>(
    visitor: &mut V,
    statement: &TypedStatement,
    program: &TypedProgram,
) {
    let mut visit_expression = |id| {
        visitor.visit_expression(
            program
                .expression(id)
                .expect("statement expression belongs to typed HIR"),
            program,
        );
    };
    match &statement.kind {
        TypedStatementKind::Variable { initializer, .. } => visit_expression(*initializer),
        TypedStatementKind::Assign { value, .. } => visit_expression(*value),
        TypedStatementKind::StateAssign { target, value, .. } => {
            visit_expression(*target);
            visit_expression(*value);
        }
        TypedStatementKind::IndexAssign { target, value, .. } => {
            visit_expression(*target);
            visit_expression(*value);
        }
        TypedStatementKind::If {
            condition,
            then_block,
            else_block,
        } => {
            visit_expression(*condition);
            visitor.visit_block(then_block, program);
            if let Some(else_block) = else_block {
                visitor.visit_block(else_block, program);
            }
        }
        TypedStatementKind::While { condition, body } => {
            visit_expression(*condition);
            visitor.visit_block(body, program);
        }
        TypedStatementKind::For { iterable, body, .. } => {
            visit_expression(*iterable);
            visitor.visit_block(body, program);
        }
        TypedStatementKind::Suspend { value, .. }
        | TypedStatementKind::Yield { value }
        | TypedStatementKind::Expression(value) => {
            visit_expression(*value);
        }
    }
}

pub fn walk_typed_expression<V: TypedVisitor>(
    visitor: &mut V,
    expression: &TypedExpression,
    program: &TypedProgram,
) {
    let mut visit_expression = |id| {
        visitor.visit_expression(
            program
                .expression(id)
                .expect("child expression belongs to typed HIR"),
            program,
        );
    };
    match &expression.kind {
        TypedExpressionKind::InterpolatedString(parts) => {
            for part in parts {
                if let TypedInterpolatedPart::Expression { expression, .. } = part {
                    visit_expression(*expression);
                }
            }
        }
        TypedExpressionKind::Array(elements) => {
            for element in elements {
                visit_expression(*element);
            }
        }
        TypedExpressionKind::Range { start, end, .. } => {
            visit_expression(*start);
            visit_expression(*end);
        }
        TypedExpressionKind::Block { statements, value } => {
            visitor.visit_block(statements, program);
            if let Some(value) = value {
                visitor.visit_expression(
                    program
                        .expression(*value)
                        .expect("value-block tail belongs to typed HIR"),
                    program,
                );
            }
        }
        TypedExpressionKind::Loop { body } => visitor.visit_block(body, program),
        TypedExpressionKind::Struct { fields, .. } => {
            for (_, value) in fields {
                visit_expression(*value);
            }
        }
        TypedExpressionKind::Enum { payload, .. } => {
            if let Some(payload) = payload {
                visit_expression(*payload);
            }
        }
        TypedExpressionKind::Match { value, arms } => {
            visit_expression(*value);
            for arm in arms {
                visitor.visit_match_arm(arm, program);
            }
        }
        TypedExpressionKind::Is { value, .. } => visit_expression(*value),
        TypedExpressionKind::If {
            condition,
            then_expr,
            else_expr,
        } => {
            visit_expression(*condition);
            visit_expression(*then_expr);
            visit_expression(*else_expr);
        }
        TypedExpressionKind::Fallback { value, fallback } => {
            visit_expression(*value);
            visit_expression(*fallback);
        }
        TypedExpressionKind::Break(Some(value))
        | TypedExpressionKind::Return(Some(value))
        | TypedExpressionKind::Throw { error: value, .. }
        | TypedExpressionKind::Inspect { value, .. }
        | TypedExpressionKind::Suspend { value, .. }
        | TypedExpressionKind::Propagate { value, .. } => visit_expression(*value),
        TypedExpressionKind::Member { receiver, .. } => visit_expression(*receiver),
        TypedExpressionKind::Index { receiver, index } => {
            visit_expression(*receiver);
            visit_expression(*index);
        }
        TypedExpressionKind::Unary { expression, .. }
        | TypedExpressionKind::Cast { expression, .. } => visit_expression(*expression),
        TypedExpressionKind::Binary { left, right, .. } => {
            visit_expression(*left);
            visit_expression(*right);
        }
        TypedExpressionKind::Call {
            receiver,
            arguments,
            ..
        } => {
            if let Some(receiver) = receiver {
                visit_expression(*receiver);
            }
            for argument in arguments {
                visit_expression(*argument);
            }
        }
        TypedExpressionKind::Invoke { callee, arguments } => {
            visit_expression(*callee);
            for argument in arguments {
                visit_expression(*argument);
            }
        }
        TypedExpressionKind::Closure { body, .. } => visit_expression(*body),
        TypedExpressionKind::None
        | TypedExpressionKind::IteratorEnd
        | TypedExpressionKind::Break(None)
        | TypedExpressionKind::Continue
        | TypedExpressionKind::Return(None)
        | TypedExpressionKind::Bool(_)
        | TypedExpressionKind::Int { .. }
        | TypedExpressionKind::Float(_)
        | TypedExpressionKind::Char(_)
        | TypedExpressionKind::String(_)
        | TypedExpressionKind::Signature(_)
        | TypedExpressionKind::Path(_) => {}
    }
}

/// Concrete value types whose formatting is implicit in this expression.
///
/// Keeping this recognition on typed HIR gives effects, reachability, and
/// diagnostics one definition of interpolation, string casts, and runtime
/// text-output calls. Capability analysis remains responsible for deciding
/// how each returned type implements `Display`.
pub(crate) fn implicit_display_types(
    expression: &TypedExpression,
    program: &TypedProgram,
    semantics: &SemanticModel,
) -> Vec<TypeId> {
    let mut types = Vec::new();
    match &expression.kind {
        TypedExpressionKind::InterpolatedString(parts) => {
            types.extend(parts.iter().filter_map(|part| match part {
                TypedInterpolatedPart::Expression {
                    conversion: Some(ImplicitConversion::ToString { source }),
                    ..
                } => Some(*source),
                TypedInterpolatedPart::Text(_)
                | TypedInterpolatedPart::Expression {
                    conversion: None, ..
                } => None,
            }));
        }
        TypedExpressionKind::Cast {
            expression: value, ..
        } if matches!(
            semantics.types().kind(expression.ty),
            TypeKind::Standard(StdlibTypeId::String)
        ) =>
        {
            types.push(
                program
                    .expression(*value)
                    .expect("cast operands belong to typed HIR")
                    .ty,
            );
        }
        TypedExpressionKind::Inspect { value, .. } => {
            types.push(
                program
                    .expression(*value)
                    .expect("inspect operands belong to typed HIR")
                    .ty,
            );
        }
        _ => {}
    }
    if let Some(ResolvedCall::StandardLibrary { item, .. }) = program.call(expression.id)
        && let TypedExpressionKind::Call { arguments, .. } = &expression.kind
    {
        let converted = match *item {
            StdlibItemId::Print => arguments.first(),
            StdlibItemId::SetVariable => arguments.get(1),
            _ => None,
        };
        if let Some(argument) = converted {
            types.push(
                program
                    .expression(*argument)
                    .expect("resolved call arguments belong to typed HIR")
                    .ty,
            );
        }
    }
    types.sort_by_key(|ty| ty.index());
    types.dedup();
    types
}

pub fn walk_typed_match_arm<V: TypedVisitor>(
    visitor: &mut V,
    arm: &TypedMatchArm,
    program: &TypedProgram,
) {
    if let Some(guard) = arm.guard {
        visitor.visit_expression(
            program
                .expression(guard)
                .expect("match guard belongs to typed HIR"),
            program,
        );
    }
    visitor.visit_expression(
        program
            .expression(arm.value)
            .expect("match value belongs to typed HIR"),
        program,
    );
}

fn lower_pattern(
    pattern: &MatchPattern,
    id: crate::ast::PatternId,
    semantics: &SemanticModel,
    syntax: &SyntaxProgram,
    standard_library: &StandardLibrary,
) -> TypedPattern {
    match pattern {
        MatchPattern::Struct { fields, .. } => {
            let structure = semantics
                .struct_pattern(id)
                .expect("typed struct patterns have resolved structures");
            let resolved_fields = semantics
                .struct_pattern_fields(id)
                .expect("typed struct patterns have resolved fields");
            TypedPattern::Struct {
                structure,
                fields: fields
                    .iter()
                    .zip(resolved_fields)
                    .map(|(field, resolved)| TypedStructPatternField {
                        field: *resolved,
                        pattern: lower_pattern_node(
                            &field.pattern,
                            semantics,
                            syntax,
                            standard_library,
                        ),
                    })
                    .collect(),
            }
        }
        MatchPattern::Enum {
            variant, payload, ..
        } => TypedPattern::Enum {
            enumeration: enum_type_for_variant(
                semantics
                    .pattern_variant(id)
                    .expect("typed enum patterns have resolved variants"),
                syntax,
                standard_library,
            ),
            variant: variant.clone(),
            payload: payload.as_ref().map(|payload| {
                Box::new(lower_pattern_node(
                    payload,
                    semantics,
                    syntax,
                    standard_library,
                ))
            }),
        },
        MatchPattern::Bool(value) => TypedPattern::Bool(*value),
        MatchPattern::Char(value) => TypedPattern::Char(*value),
        MatchPattern::String(value) => TypedPattern::String(value.clone()),
        MatchPattern::Int {
            value,
            negative,
            suffix,
        } => TypedPattern::Int {
            value: *value,
            negative: *negative,
            suffix: *suffix,
        },
        MatchPattern::IntRange {
            start,
            start_negative,
            start_suffix,
            end,
            end_negative,
            end_suffix,
            kind,
            ..
        } => TypedPattern::IntRange {
            start: *start,
            start_negative: *start_negative,
            start_suffix: *start_suffix,
            end: *end,
            end_negative: *end_negative,
            end_suffix: *end_suffix,
            kind: *kind,
        },
        MatchPattern::FileVersion(components) => TypedPattern::FileVersion(*components),
        MatchPattern::None => TypedPattern::None,
        MatchPattern::OptionSome(payload) => TypedPattern::OptionSome(Box::new(
            lower_pattern_node(payload, semantics, syntax, standard_library),
        )),
        MatchPattern::IteratorEnd => TypedPattern::IteratorEnd,
        MatchPattern::IteratorItem(payload) => TypedPattern::IteratorItem(Box::new(
            lower_pattern_node(payload, semantics, syntax, standard_library),
        )),
        MatchPattern::ResultSuccess(payload) => TypedPattern::ResultSuccess(Box::new(
            lower_pattern_node(payload, semantics, syntax, standard_library),
        )),
        MatchPattern::ResultError(payload) => TypedPattern::ResultError(Box::new(
            lower_pattern_node(payload, semantics, syntax, standard_library),
        )),
        MatchPattern::Array(array) => TypedPattern::Array {
            prefix: array
                .prefix
                .iter()
                .map(|element| lower_pattern_node(element, semantics, syntax, standard_library))
                .collect(),
            rest: array.rest.is_some(),
            suffix: array
                .suffix
                .iter()
                .map(|element| lower_pattern_node(element, semantics, syntax, standard_library))
                .collect(),
        },
        MatchPattern::Alternation(alternatives) => TypedPattern::Alternation(
            alternatives
                .iter()
                .map(|alternative| {
                    lower_pattern_node(alternative, semantics, syntax, standard_library)
                })
                .collect(),
        ),
        MatchPattern::Binding(binding) => TypedPattern::Binding(binding.clone()),
        MatchPattern::Wildcard => TypedPattern::Wildcard,
    }
}

fn lower_pattern_node(
    node: &crate::ast::PatternNode,
    semantics: &SemanticModel,
    syntax: &SyntaxProgram,
    standard_library: &StandardLibrary,
) -> TypedPatternNode {
    TypedPatternNode {
        pattern: lower_pattern(&node.kind, node.id, semantics, syntax, standard_library),
        resolution: ResolvedPattern {
            id: node.id,
            variant: semantics.pattern_variant(node.id),
            wrapper: semantics.wrapper_pattern(node.id),
            span: node.span,
        },
    }
}

fn lower_expression_kind(
    expression: &Expr,
    semantics: &SemanticModel,
    syntax: &SyntaxProgram,
    standard_library: &StandardLibrary,
) -> TypedExpressionKind {
    if let Some(resolved_variant) = semantics.enum_variant(expression.id) {
        let (variant, payload) = match &expression.kind {
            ExprKind::Path(path) => {
                let variant = path
                    .last()
                    .expect("resolved enum paths retain a variant segment");
                (variant.clone(), None)
            }
            ExprKind::Call {
                callee,
                receiver,
                args,
                ..
            } => {
                debug_assert!(receiver.is_none());
                let variant = callee
                    .last()
                    .expect("resolved enum constructors retain a variant segment");
                (variant.clone(), args.first().map(|payload| payload.id))
            }
            _ => unreachable!("only enum-shaped syntax resolves an enum variant"),
        };
        return TypedExpressionKind::Enum {
            // The expression's published type may be the target of an implicit
            // wrapper conversion (for example, returning `Chapter.Village`
            // from a `Chapter?` function). The resolved variant remains the
            // canonical identity of the enum value being constructed.
            enumeration: enum_type_for_variant(resolved_variant, syntax, standard_library),
            variant,
            payload,
        };
    }
    match &expression.kind {
        ExprKind::Error => unreachable!("recovery expressions cannot reach typed HIR"),
        ExprKind::None => TypedExpressionKind::None,
        ExprKind::IteratorEnd => TypedExpressionKind::IteratorEnd,
        ExprKind::Bool(value) => TypedExpressionKind::Bool(*value),
        ExprKind::Int {
            value,
            negative,
            suffix,
        } => TypedExpressionKind::Int {
            value: *value,
            negative: *negative,
            suffix: *suffix,
        },
        ExprKind::Float(literal) => TypedExpressionKind::Float(literal.clone()),
        ExprKind::Char(value) => TypedExpressionKind::Char(*value),
        ExprKind::String(value) => TypedExpressionKind::String(value.clone()),
        ExprKind::InterpolatedString(parts) => TypedExpressionKind::InterpolatedString(
            parts
                .iter()
                .map(|part| match part {
                    InterpolatedPart::Text(value) => TypedInterpolatedPart::Text(value.clone()),
                    InterpolatedPart::Expr(expression) => {
                        let source = semantics
                            .expression_type(expression.id)
                            .expect("interpolation operands have resolved types");
                        let conversion = (!matches!(
                            semantics.types().kind(source),
                            TypeKind::Standard(StdlibTypeId::String)
                        ))
                        .then_some(ImplicitConversion::ToString { source });
                        TypedInterpolatedPart::Expression {
                            expression: expression.id,
                            conversion,
                        }
                    }
                })
                .collect(),
        ),
        ExprKind::Signature(value) => TypedExpressionKind::Signature(value.clone()),
        ExprKind::Array(elements) => {
            TypedExpressionKind::Array(elements.iter().map(|element| element.id).collect())
        }
        ExprKind::Range {
            start, end, kind, ..
        } => TypedExpressionKind::Range {
            start: start.id,
            end: end.id,
            kind: *kind,
        },
        ExprKind::Block(block) => {
            let value = block
                .statements
                .last()
                .and_then(|statement| match statement {
                    Stmt::Expression(expression) => Some(expression.id),
                    _ => None,
                });
            let prefix_len = block.statements.len() - usize::from(value.is_some());
            let statements = Block {
                statements: block.statements[..prefix_len].to_vec(),
                span: block.span,
                trailing_semicolon: None,
            };
            TypedExpressionKind::Block {
                statements: lower_block(&statements, semantics, syntax, standard_library),
                value,
            }
        }
        ExprKind::Loop(block) => TypedExpressionKind::Loop {
            body: lower_block(block, semantics, syntax, standard_library),
        },
        ExprKind::Struct { fields, .. } => TypedExpressionKind::Struct {
            structure: semantics
                .struct_literal(expression.id)
                .expect("checked struct literals resolve their nominal declaration"),
            fields: fields
                .iter()
                .map(|field| (field.name.clone(), field.value.id))
                .collect(),
        },
        ExprKind::Match { value, arms } => TypedExpressionKind::Match {
            value: value.id,
            arms: arms
                .iter()
                .map(|arm| TypedMatchArm {
                    pattern: lower_pattern(
                        &arm.pattern,
                        arm.pattern_id,
                        semantics,
                        syntax,
                        standard_library,
                    ),
                    resolution: ResolvedPattern {
                        id: arm.pattern_id,
                        variant: semantics.pattern_variant(arm.pattern_id),
                        wrapper: semantics.wrapper_pattern(arm.pattern_id),
                        span: arm.span,
                    },
                    guard: arm.guard.as_ref().map(|guard| guard.id),
                    value: arm.value.id,
                    span: arm.span,
                })
                .collect(),
        },
        ExprKind::Is { value, pattern, .. } => TypedExpressionKind::Is {
            value: value.id,
            pattern: lower_pattern_node(pattern, semantics, syntax, standard_library),
        },
        ExprKind::If {
            condition,
            then_expr,
            else_expr,
        } => TypedExpressionKind::If {
            condition: condition.id,
            then_expr: then_expr.id,
            else_expr: else_expr.id,
        },
        ExprKind::Fallback { value, fallback } => TypedExpressionKind::Fallback {
            value: value.id,
            fallback: fallback.id,
        },
        ExprKind::Break(value) => TypedExpressionKind::Break(value.as_ref().map(|value| value.id)),
        ExprKind::Continue => TypedExpressionKind::Continue,
        ExprKind::Return(value) => {
            TypedExpressionKind::Return(value.as_ref().map(|value| value.id))
        }
        ExprKind::Throw(error) => TypedExpressionKind::Throw {
            error: error.id,
            target: failure_target_for_propagation(semantics, expression.id),
        },
        ExprKind::Inspect { label, value, .. } => TypedExpressionKind::Inspect {
            label: label.clone(),
            value: value.id,
        },
        ExprKind::Suspend {
            mode,
            destination,
            value,
        } => TypedExpressionKind::Suspend {
            mode: *mode,
            destination: *destination,
            value: value.id,
        },
        ExprKind::Propagate(value) => TypedExpressionKind::Propagate {
            value: value.id,
            target: failure_target_for_propagation(semantics, expression.id),
        },
        ExprKind::Path(path) => TypedExpressionKind::Path(path.clone()),
        ExprKind::Member {
            receiver,
            name,
            name_span,
        } => TypedExpressionKind::Member {
            receiver: receiver.id,
            name: name.clone(),
            name_span: *name_span,
        },
        ExprKind::Index {
            receiver, index, ..
        } => TypedExpressionKind::Index {
            receiver: receiver.id,
            index: index.id,
        },
        ExprKind::Unary { op, expr } => TypedExpressionKind::Unary {
            op: *op,
            expression: expr.id,
        },
        ExprKind::Cast { expr, target } => TypedExpressionKind::Cast {
            expression: expr.id,
            target: *target,
        },
        ExprKind::Binary { op, left, right } => TypedExpressionKind::Binary {
            op: *op,
            left: left.id,
            right: right.id,
        },
        ExprKind::Call {
            callee,
            receiver,
            args,
            ..
        } => TypedExpressionKind::Call {
            source_path: callee.clone(),
            receiver: receiver.as_ref().map(|receiver| receiver.id),
            arguments: args.iter().map(|argument| argument.id).collect(),
        },
        ExprKind::Invoke { callee, args } => TypedExpressionKind::Invoke {
            callee: callee.id,
            arguments: args.iter().map(|argument| argument.id).collect(),
        },
        ExprKind::Closure { params, body, .. } => TypedExpressionKind::Closure {
            parameters: params
                .iter()
                .map(|parameter| TypedBindingPattern {
                    value: parameter.id,
                    pattern: lower_pattern_node(
                        &parameter.binding.pattern,
                        semantics,
                        syntax,
                        standard_library,
                    ),
                })
                .collect(),
            body: body.id,
        },
    }
}

fn enum_type_for_variant(
    variant: ResolvedEnumVariantId,
    syntax: &SyntaxProgram,
    standard_library: &StandardLibrary,
) -> EnumTypeId {
    match variant {
        ResolvedEnumVariantId::Source(variant) => EnumTypeId::Source(
            syntax
                .enum_declarations()
                .find(|enumeration| {
                    enumeration
                        .variants
                        .iter()
                        .any(|candidate| candidate.id == variant)
                })
                .expect("resolved source variants belong to a source enum")
                .id,
        ),
        ResolvedEnumVariantId::Standard(variant) => {
            EnumTypeId::Standard(standard_library.variant(variant).owner)
        }
    }
}

fn lower_block(
    block: &Block,
    semantics: &SemanticModel,
    syntax: &SyntaxProgram,
    standard_library: &StandardLibrary,
) -> TypedBlock {
    TypedBlock {
        statements: block
            .statements
            .iter()
            .map(|source_statement| {
                let (statement, debug_only, span) = match source_statement {
                    Stmt::Debug { statement, span } => (statement.as_ref(), true, *span),
                    statement => {
                        let span = match statement {
                            Stmt::Variable(variable) => variable.span,
                            Stmt::Assign { span, .. }
                            | Stmt::StateAssign { span, .. }
                            | Stmt::IndexAssign { span, .. }
                            | Stmt::If { span, .. }
                            | Stmt::While { span, .. }
                            | Stmt::For { span, .. }
                            | Stmt::Suspend { span, .. }
                            | Stmt::Yield { span, .. } => *span,
                            Stmt::Expression(expression) => expression.span,
                            Stmt::Debug { .. } => {
                                unreachable!("nested debug modifiers are rejected during checking")
                            }
                        };
                        (statement, statement.is_implicitly_debug_only(), span)
                    }
                };
                TypedStatement {
                    kind: match statement {
                        Stmt::Debug { .. } => {
                            unreachable!(
                                "debug modifiers are unwrapped into typed statement metadata"
                            )
                        }
                        Stmt::Variable(variable) => TypedStatementKind::Variable {
                            binding: TypedBindingPattern {
                                value: variable.id,
                                pattern: lower_pattern_node(
                                    &variable.binding.pattern,
                                    semantics,
                                    syntax,
                                    standard_library,
                                ),
                            },
                            initializer: variable
                                .value
                                .as_ref()
                                .expect("local variables have initializers")
                                .id,
                        },
                        Stmt::Assign {
                            id,
                            op,
                            value,
                            span,
                            ..
                        } => TypedStatementKind::Assign {
                            assignment: ResolvedAssignment {
                                id: *id,
                                target: semantics
                                    .assignment_target(*id)
                                    .expect("checked assignments have resolved targets"),
                                operator: semantics.assignment_call(*id).cloned(),
                                span: *span,
                            },
                            op: *op,
                            value: value.id,
                        },
                        Stmt::StateAssign {
                            id,
                            target,
                            op,
                            value,
                            span,
                        } => TypedStatementKind::StateAssign {
                            assignment: ResolvedAssignment {
                                id: *id,
                                target: semantics
                                    .assignment_target(*id)
                                    .expect("checked state assignments have resolved targets"),
                                operator: semantics.assignment_call(*id).cloned(),
                                span: *span,
                            },
                            target: target.id,
                            op: *op,
                            value: value.id,
                        },
                        Stmt::IndexAssign {
                            id,
                            target,
                            op,
                            value,
                            span,
                        } => TypedStatementKind::IndexAssign {
                            assignment: ResolvedIndexAssignment {
                                id: *id,
                                operator: semantics
                                    .assignment_call(*id)
                                    .expect("checked indexed assignments have resolved operators")
                                    .clone(),
                                setter: semantics.index_assignment_setter(*id).cloned(),
                                span: *span,
                            },
                            target: target.id,
                            op: *op,
                            value: value.id,
                        },
                        Stmt::If {
                            condition,
                            then_block,
                            else_block,
                            ..
                        } => TypedStatementKind::If {
                            condition: condition.id,
                            then_block: lower_block(
                                then_block,
                                semantics,
                                syntax,
                                standard_library,
                            ),
                            else_block: else_block.as_ref().map(|block| {
                                lower_block(block, semantics, syntax, standard_library)
                            }),
                        },
                        Stmt::While {
                            condition, body, ..
                        } => TypedStatementKind::While {
                            condition: condition.id,
                            body: lower_block(body, semantics, syntax, standard_library),
                        },
                        Stmt::For {
                            binding,
                            iterable_value,
                            index_value,
                            version_value,
                            iterable,
                            body,
                            ..
                        } => TypedStatementKind::For {
                            binding: TypedBindingPattern {
                                value: binding.id,
                                pattern: lower_pattern_node(
                                    &binding.binding.pattern,
                                    semantics,
                                    syntax,
                                    standard_library,
                                ),
                            },
                            iterable_value: *iterable_value,
                            index_value: *index_value,
                            version_value: *version_value,
                            iterable: iterable.id,
                            body: lower_block(body, semantics, syntax, standard_library),
                        },
                        Stmt::Suspend {
                            mode,
                            binding,
                            returns,
                            value,
                            ..
                        } => TypedStatementKind::Suspend {
                            mode: *mode,
                            binding: binding.as_ref().map(|binding| binding.id),
                            returns: *returns,
                            value: value.id,
                        },
                        Stmt::Yield { value, .. } => TypedStatementKind::Yield { value: value.id },
                        Stmt::Expression(expression) => {
                            TypedStatementKind::Expression(expression.id)
                        }
                    },
                    debug_only,
                    span,
                }
            })
            .collect(),
        span: block.span,
    }
}

fn failure_target_for_propagation(semantics: &SemanticModel, expression: ExprId) -> FailureTarget {
    let result = semantics
        .propagation_target(expression)
        .expect("checked propagation expressions have a failure boundary");
    semantics.propagation_retry_boundary(expression).map_or(
        FailureTarget::Return(result),
        |retry| FailureTarget::Retry {
            expression: retry,
            result,
        },
    )
}

/// Returns whether a typed block contains a `yield` belonging to that block's
/// callable body. Yields inside nested closures are deliberately excluded.
pub fn typed_block_contains_yield(block: &TypedBlock, program: &TypedProgram) -> bool {
    block
        .statements
        .iter()
        .any(|statement| match &statement.kind {
            TypedStatementKind::Yield { .. } => true,
            TypedStatementKind::If {
                condition,
                then_block,
                else_block,
            } => {
                typed_expression_contains_yield(*condition, program)
                    || typed_block_contains_yield(then_block, program)
                    || else_block
                        .as_ref()
                        .is_some_and(|block| typed_block_contains_yield(block, program))
            }
            TypedStatementKind::While { condition, body } => {
                typed_expression_contains_yield(*condition, program)
                    || typed_block_contains_yield(body, program)
            }
            TypedStatementKind::For { iterable, body, .. } => {
                typed_expression_contains_yield(*iterable, program)
                    || typed_block_contains_yield(body, program)
            }
            TypedStatementKind::Variable { initializer, .. }
            | TypedStatementKind::Assign {
                value: initializer, ..
            }
            | TypedStatementKind::Suspend {
                value: initializer, ..
            }
            | TypedStatementKind::Expression(initializer) => {
                typed_expression_contains_yield(*initializer, program)
            }
            TypedStatementKind::StateAssign { target, value, .. }
            | TypedStatementKind::IndexAssign { target, value, .. } => {
                typed_expression_contains_yield(*target, program)
                    || typed_expression_contains_yield(*value, program)
            }
        })
}

/// Returns whether a typed expression contains a `yield` belonging to its
/// surrounding callable body. Yields inside nested closures are excluded.
pub fn typed_expression_contains_yield(expression: ExprId, program: &TypedProgram) -> bool {
    let Some(expression) = program.expression(expression) else {
        return false;
    };
    match &expression.kind {
        TypedExpressionKind::Block { statements, value } => {
            typed_block_contains_yield(statements, program)
                || value.is_some_and(|value| typed_expression_contains_yield(value, program))
        }
        TypedExpressionKind::Loop { body } => typed_block_contains_yield(body, program),
        TypedExpressionKind::If {
            then_expr,
            else_expr,
            ..
        } => {
            typed_expression_contains_yield(*then_expr, program)
                || typed_expression_contains_yield(*else_expr, program)
        }
        TypedExpressionKind::Match { arms, .. } => arms
            .iter()
            .any(|arm| typed_expression_contains_yield(arm.value, program)),
        TypedExpressionKind::Closure { .. } => false,
        _ => false,
    }
}
