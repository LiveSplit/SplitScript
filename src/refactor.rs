//! Selection-based source refactorings shared by every editor host.

use std::collections::{HashMap, HashSet};

use splitscript_syntax::{SyntaxMode, TokenKind, lex};

use crate::{
    TextEdit,
    ast::{Block, Expr, ExprId, ExprKind, MatchPattern, Program, Span, Stmt, ValueId},
    database::{CompilerDatabase, SemanticQueryResult, SemanticSnapshot},
    semantic::ResolvedValue,
    type_display::display_type,
    types::{TypeId, TypeKind},
    visit::{self, Visitor},
};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RefactoringKind {
    ExtractVariable,
    ExtractFunction,
    InspectExpression,
    RemoveInspection,
}

impl RefactoringKind {
    pub const fn lsp_kind(self) -> &'static str {
        match self {
            Self::ExtractVariable => "refactor.extract.variable",
            Self::ExtractFunction => "refactor.extract.function",
            Self::InspectExpression => "refactor.rewrite.inspect.add",
            Self::RemoveInspection => "refactor.rewrite.inspect.remove",
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Refactoring {
    pub title: String,
    pub kind: RefactoringKind,
    pub edits: Vec<TextEdit>,
}

pub(crate) fn extract_refactorings(
    database: &mut CompilerDatabase,
    selection: Span,
) -> SemanticQueryResult<Vec<Refactoring>> {
    let source = database.source().to_owned();
    let selection = trim_selection(&source, selection);
    let is_cursor = selection.start == selection.end;

    // These edits are advertised as machine-applicable. A recovered syntax
    // tree may have missing nodes that change scope or control flow, so only
    // offer refactorings for a strictly checked document.
    let checked = database.check()?;
    let snapshot = SemanticSnapshot::Checked(checked);
    let context = database.context();
    let mut refactorings = Vec::new();
    let selected_expression = if is_cursor {
        expression_at_cursor(snapshot.syntax(), selection.start)
    } else {
        exact_expression(snapshot.syntax(), selection)
    };
    if let Some(expression) = selected_expression {
        if is_cursor {
            let expression =
                enclosing_inspection(snapshot.syntax(), selection.start).unwrap_or(expression);
            let refactoring = inspection_refactoring(&source, expression);
            if validates(&context, &source, &refactoring.edits) {
                refactorings.push(refactoring);
            }
            return Ok(refactorings);
        }
        if let Some(refactoring) = extract_variable(&source, snapshot.syntax(), expression)
            && validates(&context, &source, &refactoring.edits)
        {
            refactorings.push(refactoring);
        }
        if let Some(refactoring) = extract_function(&source, &snapshot, expression)
            && validates(&context, &source, &refactoring.edits)
        {
            refactorings.push(refactoring);
        }
        let refactoring = inspection_refactoring(&source, expression);
        if validates(&context, &source, &refactoring.edits) {
            refactorings.push(refactoring);
        }
    }
    if !refactorings
        .iter()
        .any(|refactoring| refactoring.kind == RefactoringKind::ExtractFunction)
        && let Some(statements) = exact_statement_selection(snapshot.syntax(), selection)
        && let Some(refactoring) = extract_statements_function(&source, &snapshot, statements)
        && validates(&context, &source, &refactoring.edits)
    {
        refactorings.push(refactoring);
    }
    Ok(refactorings)
}

fn inspection_refactoring(source: &str, expression: &Expr) -> Refactoring {
    if let ExprKind::Inspect {
        value,
        keyword_span,
        ..
    } = &expression.kind
    {
        let inner = inspection_inner_source(source, expression, *keyword_span);
        let replacement = if inspection_parentheses_are_redundant(value) {
            inner.to_owned()
        } else {
            format!("({inner})")
        };
        return Refactoring {
            title: "Remove inspection".to_owned(),
            kind: RefactoringKind::RemoveInspection,
            edits: vec![TextEdit {
                span: expression.span,
                replacement,
            }],
        };
    }

    Refactoring {
        title: "Inspect expression".to_owned(),
        kind: RefactoringKind::InspectExpression,
        edits: vec![TextEdit {
            span: expression.span,
            replacement: format!(
                "inspect({})",
                strip_redundant_outer_parentheses(
                    &source[expression.span.start..expression.span.end]
                )
            ),
        }],
    }
}

fn strip_redundant_outer_parentheses(mut source: &str) -> &str {
    source = source.trim();
    loop {
        let Ok(tokens) = lex(source, SyntaxMode::Program) else {
            return source;
        };
        let tokens = tokens
            .iter()
            .filter(|token| !matches!(token.kind, TokenKind::Eof))
            .collect::<Vec<_>>();
        let (Some(first), Some(last)) = (tokens.first(), tokens.last()) else {
            return source;
        };
        if !matches!(first.kind, TokenKind::LParen) || !matches!(last.kind, TokenKind::RParen) {
            return source;
        }

        let mut depth = 0usize;
        let mut closes_at_end = false;
        for (index, token) in tokens.iter().enumerate() {
            match token.kind {
                TokenKind::LParen => depth += 1,
                TokenKind::RParen => {
                    depth = depth.saturating_sub(1);
                    if depth == 0 {
                        closes_at_end = index + 1 == tokens.len();
                        break;
                    }
                }
                _ => {}
            }
        }
        if !closes_at_end {
            return source;
        }
        source = source[first.span.end..last.span.start].trim();
    }
}

fn inspection_inner_source<'a>(source: &'a str, expression: &Expr, keyword_span: Span) -> &'a str {
    let after_keyword = &source[keyword_span.end..expression.span.end];
    let opening = after_keyword.find('(').unwrap_or(0) + keyword_span.end;
    let closing = source[opening + 1..expression.span.end]
        .rfind(')')
        .map_or(expression.span.end, |offset| opening + 1 + offset);
    source[opening + 1..closing].trim()
}

/// These expressions bind at least as tightly as the postfix-shaped
/// `inspect(...)` expression in every surrounding expression context. More
/// involved operands deliberately retain parentheses when inspection is
/// removed so a refactoring can never silently change precedence.
fn inspection_parentheses_are_redundant(value: &Expr) -> bool {
    matches!(
        value.kind,
        ExprKind::None
            | ExprKind::IteratorEnd
            | ExprKind::Bool(_)
            | ExprKind::Char(_)
            | ExprKind::Int { .. }
            | ExprKind::Float(_)
            | ExprKind::String(_)
            | ExprKind::InterpolatedString(_)
            | ExprKind::Signature(_)
            | ExprKind::Array(_)
            | ExprKind::Path(_)
            | ExprKind::Member { .. }
            | ExprKind::Index { .. }
            | ExprKind::Call { .. }
            | ExprKind::Invoke { .. }
    )
}

fn trim_selection(source: &str, mut span: Span) -> Span {
    span.start = span.start.min(source.len());
    span.end = span.end.min(source.len()).max(span.start);
    while span.start < span.end && source.as_bytes()[span.start].is_ascii_whitespace() {
        span.start += 1;
    }
    while span.start < span.end && source.as_bytes()[span.end - 1].is_ascii_whitespace() {
        span.end -= 1;
    }
    span
}

fn exact_expression(program: &Program, selection: Span) -> Option<&Expr> {
    struct Finder<'ast> {
        selection: Span,
        found: Option<&'ast Expr>,
    }
    impl<'ast> Visitor<'ast> for Finder<'ast> {
        fn visit_expr(&mut self, expression: &'ast Expr) {
            if expression.span == self.selection {
                self.found = Some(expression);
            }
            visit::walk_expr(self, expression);
        }
    }
    let mut finder = Finder {
        selection,
        found: None,
    };
    finder.visit_program(program);
    finder.found
}

fn expression_at_cursor(program: &Program, cursor: usize) -> Option<&Expr> {
    struct Finder<'ast> {
        cursor: usize,
        found: Option<&'ast Expr>,
    }
    impl<'ast> Visitor<'ast> for Finder<'ast> {
        fn visit_expr(&mut self, expression: &'ast Expr) {
            if expression.span.start <= self.cursor && self.cursor <= expression.span.end {
                if self.found.is_none_or(|found| {
                    expression.span.end - expression.span.start < found.span.end - found.span.start
                }) {
                    self.found = Some(expression);
                }
                visit::walk_expr(self, expression);
            }
        }
    }
    let mut finder = Finder {
        cursor,
        found: None,
    };
    finder.visit_program(program);
    finder.found
}

fn enclosing_inspection(program: &Program, cursor: usize) -> Option<&Expr> {
    struct Finder<'ast> {
        cursor: usize,
        found: Option<&'ast Expr>,
    }
    impl<'ast> Visitor<'ast> for Finder<'ast> {
        fn visit_expr(&mut self, expression: &'ast Expr) {
            if expression.span.start <= self.cursor && self.cursor <= expression.span.end {
                if matches!(expression.kind, ExprKind::Inspect { .. })
                    && self.found.is_none_or(|found| {
                        expression.span.end - expression.span.start
                            < found.span.end - found.span.start
                    })
                {
                    self.found = Some(expression);
                }
                visit::walk_expr(self, expression);
            }
        }
    }
    let mut finder = Finder {
        cursor,
        found: None,
    };
    finder.visit_program(program);
    finder.found
}

#[derive(Debug, Clone, Copy)]
struct StatementSelection<'ast> {
    statements: &'ast [Stmt],
    span: Span,
}

fn exact_statement_selection(program: &Program, selection: Span) -> Option<StatementSelection<'_>> {
    fn in_block(block: &Block, selection: Span) -> Option<StatementSelection<'_>> {
        for statement in &block.statements {
            let nested = match statement {
                Stmt::Debug { statement, .. } => match statement.as_ref() {
                    Stmt::If {
                        then_block,
                        else_block,
                        ..
                    } => in_block(then_block, selection).or_else(|| {
                        else_block
                            .as_ref()
                            .and_then(|block| in_block(block, selection))
                    }),
                    Stmt::While { body, .. } | Stmt::For { body, .. } => in_block(body, selection),
                    _ => None,
                },
                Stmt::If {
                    then_block,
                    else_block,
                    ..
                } => in_block(then_block, selection).or_else(|| {
                    else_block
                        .as_ref()
                        .and_then(|block| in_block(block, selection))
                }),
                Stmt::While { body, .. } | Stmt::For { body, .. } => in_block(body, selection),
                _ => None,
            };
            if nested.is_some() {
                return nested;
            }
        }

        for start in 0..block.statements.len() {
            if statement_span(&block.statements[start]).start != selection.start {
                continue;
            }
            for end in start..block.statements.len() {
                if statement_span(&block.statements[end]).end == selection.end {
                    return Some(StatementSelection {
                        statements: &block.statements[start..=end],
                        span: selection,
                    });
                }
                if statement_span(&block.statements[end]).end > selection.end {
                    break;
                }
            }
        }
        None
    }

    program
        .functions
        .iter()
        .find_map(|function| in_block(&function.body, selection))
        .or_else(|| {
            program
                .actions
                .iter()
                .find_map(|action| in_block(&action.body, selection))
        })
}

fn extract_variable(source: &str, program: &Program, expression: &Expr) -> Option<Refactoring> {
    let statement = enclosing_statement(program, expression.span)?;
    if !can_hoist_from_statement(statement, expression.id) {
        return None;
    }
    let statement_start = statement_span(statement).start;
    let line_start = source[..statement_start]
        .rfind('\n')
        .map_or(0, |index| index + 1);
    let indentation = &source[line_start..statement_start];
    if !indentation.chars().all(char::is_whitespace) {
        return None;
    }

    let occupied = source_identifiers(source);
    let name = unique_name("value", &occupied);
    let expression_source = &source[expression.span.start..expression.span.end];
    Some(Refactoring {
        title: format!("Extract into variable `{name}`"),
        kind: RefactoringKind::ExtractVariable,
        edits: vec![
            TextEdit {
                span: Span {
                    start: line_start,
                    end: line_start,
                },
                replacement: format!("{indentation}let {name} = {expression_source}\n"),
            },
            TextEdit {
                span: expression.span,
                replacement: name,
            },
        ],
    })
}

fn can_hoist_from_statement(statement: &Stmt, target: ExprId) -> bool {
    let (root, repeated) = match statement {
        Stmt::Variable(variable) => (
            variable
                .value
                .as_ref()
                .expect("local variables have initializers"),
            false,
        ),
        Stmt::Assign { value, .. } | Stmt::Yield { value, .. } | Stmt::Expression(value) => {
            (value, false)
        }
        Stmt::StateAssign { .. } | Stmt::IndexAssign { .. } => return false,
        Stmt::If { condition, .. } => (condition, false),
        Stmt::While { condition, .. } => (condition, true),
        Stmt::For { iterable, .. } => (iterable, false),
        Stmt::Suspend { value, .. } => (value, false),
        Stmt::Debug { statement, .. } => return can_hoist_from_statement(statement, target),
    };
    let mut finder = ConditionalFinder {
        target,
        conditional: repeated,
        result: None,
    };
    finder.visit_expr(root);
    finder.result == Some(false)
}

struct ConditionalFinder {
    target: ExprId,
    conditional: bool,
    result: Option<bool>,
}

impl ConditionalFinder {
    fn under_condition(&mut self, expression: &Expr) {
        let previous = self.conditional;
        self.conditional = true;
        self.visit_expr(expression);
        self.conditional = previous;
    }
}

impl<'ast> Visitor<'ast> for ConditionalFinder {
    fn visit_expr(&mut self, expression: &'ast Expr) {
        if expression.id == self.target {
            self.result = Some(self.conditional);
            return;
        }
        match &expression.kind {
            ExprKind::Match { value, arms } => {
                self.visit_expr(value);
                for arm in arms {
                    if let Some(guard) = &arm.guard {
                        self.under_condition(guard);
                    }
                    self.under_condition(&arm.value);
                }
            }
            ExprKind::If {
                condition,
                then_expr,
                else_expr,
            } => {
                self.visit_expr(condition);
                self.under_condition(then_expr);
                self.under_condition(else_expr);
            }
            ExprKind::Fallback { value, fallback } => {
                self.visit_expr(value);
                self.under_condition(fallback);
            }
            ExprKind::Binary {
                op: crate::ast::BinaryOp::And | crate::ast::BinaryOp::Or,
                left,
                right,
            } => {
                self.visit_expr(left);
                self.under_condition(right);
            }
            _ => visit::walk_expr(self, expression),
        }
    }
}

#[derive(Debug)]
struct Parameter {
    name: String,
    ty: TypeId,
    argument: String,
    replacements: Vec<Span>,
    first_use: usize,
}

fn extract_function(
    source: &str,
    snapshot: &SemanticSnapshot,
    expression: &Expr,
) -> Option<Refactoring> {
    let facts = ExpressionFacts::collect(expression, snapshot);
    if facts.has_escaping_control_flow {
        return None;
    }
    let parameters = extraction_parameters(source, snapshot, &facts)?;

    let occupied = source_identifiers(source);
    let function_name = unique_name("extracted", &occupied);
    let mut body = source[expression.span.start..expression.span.end].to_owned();
    let mut replacements = parameters
        .iter()
        .flat_map(|parameter| {
            parameter.replacements.iter().map(|span| {
                (
                    Span {
                        start: span.start - expression.span.start,
                        end: span.end - expression.span.start,
                    },
                    parameter.name.as_str(),
                )
            })
        })
        .collect::<Vec<_>>();
    replacements.sort_by_key(|(span, _)| span.start);
    for (span, replacement) in replacements.into_iter().rev() {
        body.replace_range(span.start..span.end, replacement);
    }

    let signature = parameters
        .iter()
        .map(|parameter| {
            format!(
                "{}: {}",
                parameter.name,
                display_type(parameter.ty, snapshot)
            )
        })
        .collect::<Vec<_>>()
        .join(", ");
    let arguments = parameters
        .iter()
        .map(|parameter| parameter.argument.as_str())
        .collect::<Vec<_>>()
        .join(", ");
    let result = if facts.has_propagation {
        let mut completion = snapshot.semantics().expression_type(expression.id)?;
        if let TypeKind::Result { value, .. } = snapshot.semantics().types().kind(completion) {
            completion = *value;
        }
        if contains_generic_parameter(completion, snapshot) {
            return None;
        }
        let completion = display_type(completion, snapshot);
        if facts.has_suspension {
            format!(" -> async {completion}!")
        } else {
            format!(" -> {completion}!")
        }
    } else {
        String::new()
    };
    let indented_body = body.replace('\n', "\n    ");
    let declaration =
        format!("\n\nfn {function_name}({signature}){result} {{\n    return {indented_body}\n}}\n");

    let mut call = format!("{function_name}({arguments})");
    if facts.has_suspension {
        call = format!("(await {call})");
    }
    if facts.has_propagation {
        call = format!("({call}?)");
    }

    Some(Refactoring {
        title: format!("Extract into function `{function_name}`"),
        kind: RefactoringKind::ExtractFunction,
        edits: vec![
            TextEdit {
                span: expression.span,
                replacement: call,
            },
            TextEdit {
                span: Span {
                    start: source.len(),
                    end: source.len(),
                },
                replacement: declaration,
            },
        ],
    })
}

fn extract_statements_function(
    source: &str,
    snapshot: &SemanticSnapshot,
    selection: StatementSelection<'_>,
) -> Option<Refactoring> {
    let facts = ExpressionFacts::collect_statements(selection.statements, snapshot);
    if facts.has_escaping_control_flow || facts.has_propagation {
        return None;
    }

    let mut safety = StatementSafety {
        snapshot,
        unsafe_control_flow: false,
        assignments: Vec::new(),
    };
    for statement in selection.statements {
        safety.visit_stmt(statement);
    }
    if safety.unsafe_control_flow {
        return None;
    }
    let globals = global_values(snapshot.syntax());
    if safety
        .assignments
        .iter()
        .any(|target| !globals.contains(target) && !facts.internal_bindings.contains(target))
        || selected_bindings_escape(snapshot, selection.span, &facts.internal_bindings)
    {
        return None;
    }

    let parameters = extraction_parameters(source, snapshot, &facts)?;
    let occupied = source_identifiers(source);
    let function_name = unique_name("extracted", &occupied);
    let body = rewritten_fragment(source, selection.span, &parameters);
    let line_start = source[..selection.span.start]
        .rfind('\n')
        .map_or(0, |index| index + 1);
    let indentation = &source[line_start..selection.span.start];
    if !indentation.chars().all(char::is_whitespace) {
        return None;
    }
    let body = indent_statement_fragment(&body, indentation);
    let signature = parameter_signature(&parameters, snapshot);
    let declaration = format!("\n\nfn {function_name}({signature}) {{\n{body}\n}}\n");
    let arguments = parameter_arguments(&parameters);
    let call = if facts.has_suspension {
        format!("await {function_name}({arguments})")
    } else {
        format!("{function_name}({arguments})")
    };

    Some(Refactoring {
        title: format!("Extract into function `{function_name}`"),
        kind: RefactoringKind::ExtractFunction,
        edits: vec![
            TextEdit {
                span: selection.span,
                replacement: call,
            },
            TextEdit {
                span: Span {
                    start: source.len(),
                    end: source.len(),
                },
                replacement: declaration,
            },
        ],
    })
}

struct StatementSafety<'a> {
    snapshot: &'a SemanticSnapshot,
    unsafe_control_flow: bool,
    assignments: Vec<ValueId>,
}

impl<'ast> Visitor<'ast> for StatementSafety<'_> {
    fn visit_stmt(&mut self, statement: &'ast Stmt) {
        match statement {
            Stmt::Assign { id, .. } => {
                if let Some(target) = self.snapshot.semantics().assignment_target(*id) {
                    self.assignments.push(target);
                }
                visit::walk_stmt(self, statement);
            }
            _ => visit::walk_stmt(self, statement),
        }
    }

    fn visit_expr(&mut self, expression: &'ast Expr) {
        if matches!(
            expression.kind,
            ExprKind::Return(_) | ExprKind::Break(_) | ExprKind::Continue | ExprKind::Throw(_)
        ) {
            self.unsafe_control_flow = true;
        }
        visit::walk_expr(self, expression);
    }
}

fn selected_bindings_escape(
    snapshot: &SemanticSnapshot,
    selection: Span,
    bindings: &HashSet<ValueId>,
) -> bool {
    struct Finder<'a> {
        snapshot: &'a SemanticSnapshot,
        selection: Span,
        bindings: &'a HashSet<ValueId>,
        escaped: bool,
    }
    impl<'ast> Visitor<'ast> for Finder<'_> {
        fn visit_expr(&mut self, expression: &'ast Expr) {
            if (expression.span.start < self.selection.start
                || self.selection.end < expression.span.end)
                && self
                    .snapshot
                    .semantics()
                    .value(expression.id)
                    .is_some_and(|value| {
                        matches!(value, ResolvedValue::Variable(id) if self.bindings.contains(&id))
                    })
            {
                self.escaped = true;
            }
            visit::walk_expr(self, expression);
        }

        fn visit_stmt(&mut self, statement: &'ast Stmt) {
            if let Stmt::Assign { id, span, .. } = statement
                && (span.start < self.selection.start || self.selection.end < span.end)
                && self
                    .snapshot
                    .semantics()
                    .assignment_target(*id)
                    .is_some_and(|target| self.bindings.contains(&target))
            {
                self.escaped = true;
            }
            visit::walk_stmt(self, statement);
        }
    }
    let mut finder = Finder {
        snapshot,
        selection,
        bindings,
        escaped: false,
    };
    finder.visit_program(snapshot.syntax());
    finder.escaped
}

fn rewritten_fragment(source: &str, span: Span, parameters: &[Parameter]) -> String {
    let mut body = source[span.start..span.end].to_owned();
    let mut replacements = parameters
        .iter()
        .flat_map(|parameter| {
            parameter.replacements.iter().map(|replacement| {
                (
                    Span {
                        start: replacement.start - span.start,
                        end: replacement.end - span.start,
                    },
                    parameter.name.as_str(),
                )
            })
        })
        .collect::<Vec<_>>();
    replacements.sort_by_key(|(span, _)| span.start);
    for (replacement, name) in replacements.into_iter().rev() {
        body.replace_range(replacement.start..replacement.end, name);
    }
    body
}

fn indent_statement_fragment(fragment: &str, original_indentation: &str) -> String {
    fragment
        .lines()
        .enumerate()
        .map(|(index, line)| {
            let line = if index == 0 {
                line
            } else {
                line.strip_prefix(original_indentation).unwrap_or(line)
            };
            format!("    {line}")
        })
        .collect::<Vec<_>>()
        .join("\n")
}

fn parameter_signature(parameters: &[Parameter], snapshot: &SemanticSnapshot) -> String {
    parameters
        .iter()
        .map(|parameter| {
            format!(
                "{}: {}",
                parameter.name,
                display_type(parameter.ty, snapshot)
            )
        })
        .collect::<Vec<_>>()
        .join(", ")
}

fn parameter_arguments(parameters: &[Parameter]) -> String {
    parameters
        .iter()
        .map(|parameter| parameter.argument.as_str())
        .collect::<Vec<_>>()
        .join(", ")
}

fn extraction_parameters(
    source: &str,
    snapshot: &SemanticSnapshot,
    facts: &ExpressionFacts<'_>,
) -> Option<Vec<Parameter>> {
    let globals = global_values(snapshot.syntax());
    let names = value_names(snapshot.syntax());
    // Keep lexical names unchanged in the copied source. Reserve them up front
    // so generated names for snapshot/setting arguments cannot shadow one of
    // the caller-local parameters.
    let mut occupied_parameters = facts
        .references
        .iter()
        .filter_map(|reference| match reference.value {
            ResolvedValue::Variable(value)
                if !globals.contains(&value) && !facts.internal_bindings.contains(&value) =>
            {
                names.get(&value).cloned()
            }
            _ => None,
        })
        .collect::<HashSet<_>>();
    let mut parameters = Vec::new();

    for reference in &facts.references {
        if is_contextual(reference.value)
            && facts.references.iter().any(|candidate| {
                is_contextual(candidate.value)
                    && candidate.span != reference.span
                    && candidate.span.start <= reference.span.start
                    && reference.span.end <= candidate.span.end
            })
        {
            continue;
        }
        match reference.value {
            ResolvedValue::Variable(value)
                if !globals.contains(&value) && !facts.internal_bindings.contains(&value) =>
            {
                let original = names.get(&value)?.clone();
                if parameters.iter().any(|parameter: &Parameter| {
                    parameter.replacements.is_empty() && parameter.argument == original
                }) {
                    continue;
                }
                parameters.push(Parameter {
                    name: original.clone(),
                    ty: snapshot.semantics().value_type(value)?,
                    argument: original,
                    replacements: Vec::new(),
                    first_use: reference.span.start,
                });
            }
            ResolvedValue::CurrentSnapshot
            | ResolvedValue::OldSnapshot
            | ResolvedValue::SettingsView
            | ResolvedValue::OldSettingsView
            | ResolvedValue::CurrentState(_)
            | ResolvedValue::OldState(_)
            | ResolvedValue::Setting(_)
            | ResolvedValue::OldSetting(_) => {
                let argument = source[reference.span.start..reference.span.end].to_owned();
                if let Some(parameter) = parameters
                    .iter_mut()
                    .find(|parameter| parameter.argument == argument)
                {
                    parameter.replacements.push(reference.span);
                    continue;
                }
                let base = contextual_parameter_name(reference.value, &argument);
                let name = unique_name(&base, &occupied_parameters);
                occupied_parameters.insert(name.clone());
                parameters.push(Parameter {
                    name,
                    ty: snapshot.semantics().expression_type(reference.expression)?,
                    argument,
                    replacements: vec![reference.span],
                    first_use: reference.span.start,
                });
            }
            ResolvedValue::ProviderValue(_)
            | ResolvedValue::ProviderContext { .. }
            | ResolvedValue::Variable(_)
            | ResolvedValue::StateCandidate(_) => {}
            ResolvedValue::ManagedStatic { .. } => {}
            ResolvedValue::StandardLibraryConstant(_) => {}
        }
    }
    parameters.sort_by_key(|parameter| parameter.first_use);

    // A generalized type cannot yet be written on a standalone generated
    // function without transferring the surrounding function's constraints.
    (!parameters
        .iter()
        .any(|parameter| contains_generic_parameter(parameter.ty, snapshot)))
    .then_some(parameters)
}

fn global_values(program: &Program) -> HashSet<ValueId> {
    program
        .globals
        .iter()
        .map(|variable| variable.id)
        .chain(
            program
                .state
                .as_ref()
                .and_then(|state| state.provider_value),
        )
        .collect()
}

fn is_contextual(value: ResolvedValue) -> bool {
    matches!(
        value,
        ResolvedValue::CurrentSnapshot
            | ResolvedValue::OldSnapshot
            | ResolvedValue::SettingsView
            | ResolvedValue::OldSettingsView
            | ResolvedValue::CurrentState(_)
            | ResolvedValue::OldState(_)
            | ResolvedValue::Setting(_)
            | ResolvedValue::OldSetting(_)
    )
}

#[derive(Debug, Clone, Copy)]
struct ValueReference {
    expression: ExprId,
    value: ResolvedValue,
    span: Span,
}

struct ExpressionFacts<'a> {
    snapshot: &'a SemanticSnapshot,
    references: Vec<ValueReference>,
    internal_bindings: HashSet<ValueId>,
    has_suspension: bool,
    has_propagation: bool,
    has_escaping_control_flow: bool,
}

impl<'a> ExpressionFacts<'a> {
    fn collect(expression: &Expr, snapshot: &'a SemanticSnapshot) -> Self {
        let mut facts = Self {
            snapshot,
            references: Vec::new(),
            internal_bindings: HashSet::new(),
            has_suspension: false,
            has_propagation: false,
            has_escaping_control_flow: false,
        };
        facts.visit_expr(expression);
        facts
    }

    fn collect_statements(statements: &[Stmt], snapshot: &'a SemanticSnapshot) -> Self {
        let mut facts = Self {
            snapshot,
            references: Vec::new(),
            internal_bindings: HashSet::new(),
            has_suspension: false,
            has_propagation: false,
            has_escaping_control_flow: false,
        };
        for statement in statements {
            facts.visit_stmt(statement);
        }
        facts
    }
}

impl<'ast> Visitor<'ast> for ExpressionFacts<'_> {
    fn visit_variable(&mut self, variable: &'ast crate::ast::VariableDecl) {
        self.internal_bindings.insert(variable.id);
        visit::walk_variable(self, variable);
    }

    fn visit_suspension_binding(&mut self, binding: &'ast crate::ast::SuspensionBinding) {
        self.internal_bindings.insert(binding.id);
    }

    fn visit_for_binding(&mut self, binding: &'ast crate::ast::ForBinding) {
        self.internal_bindings.insert(binding.id);
    }

    fn visit_expr(&mut self, expression: &'ast Expr) {
        if let Some(value) = self.snapshot.semantics().value(expression.id) {
            self.references.push(ValueReference {
                expression: expression.id,
                value,
                span: expression.span,
            });
        }
        match &expression.kind {
            ExprKind::Suspend { .. } => self.has_suspension = true,
            ExprKind::Propagate(_) => self.has_propagation = true,
            ExprKind::Return(_) | ExprKind::Break(_) | ExprKind::Continue | ExprKind::Throw(_) => {
                self.has_escaping_control_flow = true
            }
            _ => {}
        }
        visit::walk_expr(self, expression);
    }

    fn visit_pattern(&mut self, pattern: &'ast MatchPattern) {
        let binding = match pattern {
            MatchPattern::Bool(_)
            | MatchPattern::Char(_)
            | MatchPattern::String(_)
            | MatchPattern::Int { .. }
            | MatchPattern::IntRange { .. }
            | MatchPattern::FileVersion(_)
            | MatchPattern::None
            | MatchPattern::IteratorEnd
            | MatchPattern::Wildcard
            | MatchPattern::Struct { .. }
            | MatchPattern::Enum { .. }
            | MatchPattern::OptionSome(_)
            | MatchPattern::IteratorItem(_)
            | MatchPattern::ResultSuccess(_)
            | MatchPattern::ResultError(_)
            | MatchPattern::Array(_)
            | MatchPattern::Alternation(_) => None,
            MatchPattern::Binding(binding) => Some(binding),
        };
        if let Some(binding) = binding {
            self.internal_bindings.insert(binding.id);
        }
        visit::walk_pattern(self, pattern);
    }
}

fn contains_generic_parameter(ty: TypeId, snapshot: &SemanticSnapshot) -> bool {
    match snapshot.semantics().types().kind(ty) {
        TypeKind::Error => false,
        TypeKind::GenericParameter { .. } => true,
        TypeKind::Array { element, .. }
        | TypeKind::Set { element, .. }
        | TypeKind::Option { value: element, .. }
        | TypeKind::Result { value: element, .. }
        | TypeKind::Async { value: element, .. }
        | TypeKind::Iterator { item: element, .. }
        | TypeKind::Range { bound: element, .. } => contains_generic_parameter(*element, snapshot),
        TypeKind::Application { arguments, .. } => arguments
            .iter()
            .any(|argument| contains_generic_parameter(*argument, snapshot)),
        TypeKind::Builtin(_)
        | TypeKind::Standard(_)
        | TypeKind::StateSnapshot
        | TypeKind::SettingsView
        | TypeKind::Struct(_)
        | TypeKind::ManagedClass(_)
        | TypeKind::ManagedReference(_)
        | TypeKind::Enum(_) => false,
        TypeKind::Callable { .. } => false,
    }
}

fn contextual_parameter_name(value: ResolvedValue, source: &str) -> String {
    let leaf = source
        .split(|character: char| !character.is_ascii_alphanumeric() && character != '_')
        .rfind(|segment| !segment.is_empty())
        .unwrap_or("value");
    let prefix = match value {
        ResolvedValue::CurrentSnapshot | ResolvedValue::CurrentState(_) => "current",
        ResolvedValue::OldSnapshot | ResolvedValue::OldState(_) => "old",
        ResolvedValue::SettingsView | ResolvedValue::Setting(_) => "setting",
        ResolvedValue::OldSettingsView | ResolvedValue::OldSetting(_) => "oldSetting",
        ResolvedValue::ProviderValue(_)
        | ResolvedValue::ProviderContext { .. }
        | ResolvedValue::Variable(_)
        | ResolvedValue::StateCandidate(_) => "value",
        ResolvedValue::ManagedStatic { .. } => "value",
        ResolvedValue::StandardLibraryConstant(_) => {
            unreachable!("standard-library constants are not contextual values")
        }
    };
    if leaf == prefix {
        return prefix.to_owned();
    }
    let mut characters = leaf.chars();
    let first = characters.next().unwrap_or('v').to_ascii_uppercase();
    format!("{prefix}{first}{}", characters.as_str())
}

fn value_names(program: &Program) -> HashMap<ValueId, String> {
    struct Collector {
        names: HashMap<ValueId, String>,
    }
    impl<'ast> Visitor<'ast> for Collector {
        fn visit_state_field(&mut self, field: &'ast crate::ast::StateField) {
            self.names.insert(field.id, field.name.clone());
            visit::walk_state_field(self, field);
        }
        fn visit_setting(&mut self, setting: &'ast crate::ast::SettingDecl) {
            self.names.insert(setting.id, setting.name.clone());
        }
        fn visit_variable(&mut self, variable: &'ast crate::ast::VariableDecl) {
            self.names.insert(variable.id, variable.name.clone());
            visit::walk_variable(self, variable);
        }
        fn visit_parameter(&mut self, parameter: &'ast crate::ast::Parameter) {
            self.names.insert(parameter.id, parameter.name.clone());
            visit::walk_parameter(self, parameter);
        }
        fn visit_suspension_binding(&mut self, binding: &'ast crate::ast::SuspensionBinding) {
            self.names.insert(binding.id, binding.name.clone());
        }
        fn visit_for_binding(&mut self, binding: &'ast crate::ast::ForBinding) {
            self.names.insert(binding.id, binding.name.clone());
        }
        fn visit_pattern(&mut self, pattern: &'ast MatchPattern) {
            let binding = match pattern {
                MatchPattern::Binding(binding) => Some(binding),
                _ => None,
            };
            if let Some(binding) = binding {
                self.names.insert(binding.id, binding.name.clone());
            }
            visit::walk_pattern(self, pattern);
        }
    }
    let mut collector = Collector {
        names: HashMap::new(),
    };
    if let Some(state) = &program.state
        && let Some(provider) = state.provider_value
    {
        collector.names.insert(provider, "provider".to_owned());
    }
    collector.visit_program(program);
    collector.names
}

fn enclosing_statement(program: &Program, target: Span) -> Option<&Stmt> {
    fn in_block(block: &Block, target: Span) -> Option<&Stmt> {
        for statement in &block.statements {
            let span = statement_span(statement);
            if span.start <= target.start && target.end <= span.end {
                let nested = match statement {
                    Stmt::Debug { statement, .. } => {
                        let nested_span = statement_span(statement);
                        (nested_span.start <= target.start && target.end <= nested_span.end)
                            .then_some(statement.as_ref())
                    }
                    Stmt::If {
                        then_block,
                        else_block,
                        ..
                    } => in_block(then_block, target).or_else(|| {
                        else_block
                            .as_ref()
                            .and_then(|block| in_block(block, target))
                    }),
                    Stmt::While { body, .. } | Stmt::For { body, .. } => in_block(body, target),
                    _ => None,
                };
                return nested.or(Some(statement));
            }
        }
        None
    }
    program
        .functions
        .iter()
        .find_map(|function| in_block(&function.body, target))
        .or_else(|| {
            program
                .actions
                .iter()
                .find_map(|action| in_block(&action.body, target))
        })
}

fn statement_span(statement: &Stmt) -> Span {
    match statement {
        Stmt::Debug { span, .. }
        | Stmt::Assign { span, .. }
        | Stmt::StateAssign { span, .. }
        | Stmt::IndexAssign { span, .. }
        | Stmt::If { span, .. }
        | Stmt::While { span, .. }
        | Stmt::For { span, .. }
        | Stmt::Suspend { span, .. }
        | Stmt::Yield { span, .. } => *span,
        Stmt::Variable(variable) => variable.span,
        Stmt::Expression(expression) => expression.span,
    }
}

fn source_identifiers(source: &str) -> HashSet<String> {
    splitscript_syntax::lex(source, splitscript_syntax::SyntaxMode::Program)
        .unwrap_or_default()
        .into_iter()
        .filter_map(|token| match token.kind {
            splitscript_syntax::TokenKind::Ident(name) => Some(name),
            _ => None,
        })
        .collect()
}

fn unique_name(base: &str, occupied: &HashSet<String>) -> String {
    if !occupied.contains(base) {
        return base.to_owned();
    }
    (2..)
        .map(|suffix| format!("{base}{suffix}"))
        .find(|candidate| !occupied.contains(candidate))
        .expect("the finite source cannot occupy every numeric suffix")
}

fn validates(context: &crate::CompilerContext, source: &str, edits: &[TextEdit]) -> bool {
    let mut candidate = source.to_owned();
    let mut edits = edits.to_vec();
    edits.sort_by_key(|edit| (edit.span.start, edit.span.end));
    for edit in edits.into_iter().rev() {
        if edit.span.start > edit.span.end || edit.span.end > candidate.len() {
            return false;
        }
        candidate.replace_range(edit.span.start..edit.span.end, &edit.replacement);
    }
    CompilerDatabase::with_context(context.clone(), candidate)
        .check()
        .is_ok()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn selection(source: &str, needle: &str) -> Span {
        let start = source.find(needle).unwrap();
        Span {
            start,
            end: start + needle.len(),
        }
    }

    fn apply(source: &str, refactoring: &Refactoring) -> String {
        let mut output = source.to_owned();
        let mut edits = refactoring.edits.clone();
        edits.sort_by_key(|edit| edit.span.start);
        for edit in edits.into_iter().rev() {
            output.replace_range(edit.span.start..edit.span.end, &edit.replacement);
        }
        output
    }

    #[test]
    fn extracts_an_expression_into_a_local_variable() {
        let source = "state \"game.exe\" {}\nfn score(x: i32) {\n    return x + 1\n}\n";
        let mut database = CompilerDatabase::new(source);
        let actions = database.refactorings(selection(source, "x + 1")).unwrap();
        let action = actions
            .iter()
            .find(|action| action.kind == RefactoringKind::ExtractVariable)
            .unwrap();
        assert_eq!(
            apply(source, action),
            "state \"game.exe\" {}\nfn score(x: i32) {\n    let value = x + 1\n    return value\n}\n"
        );
    }

    #[test]
    fn adds_and_removes_inspection_without_changing_expression_grouping() {
        let source = "state \"game.exe\" {}\nfn score(x: i32) { return x + 1 }\n";
        let mut database = CompilerDatabase::new(source);
        let actions = database.refactorings(selection(source, "x + 1")).unwrap();
        let inspect = actions
            .iter()
            .find(|action| action.kind == RefactoringKind::InspectExpression)
            .unwrap();
        let inspected = apply(source, inspect);
        assert_eq!(
            inspected,
            "state \"game.exe\" {}\nfn score(x: i32) { return inspect(x + 1) }\n"
        );

        let mut database = CompilerDatabase::new(&inspected);
        let actions = database
            .refactorings(selection(&inspected, "inspect(x + 1)"))
            .unwrap();
        let remove = actions
            .iter()
            .find(|action| action.kind == RefactoringKind::RemoveInspection)
            .unwrap();
        assert_eq!(
            apply(&inspected, remove),
            "state \"game.exe\" {}\nfn score(x: i32) { return (x + 1) }\n"
        );

        let atomic = "state \"game.exe\" {}\nfn score(x: i32) { return inspect(x) }\n";
        let cursor = atomic.find("inspect").unwrap() + 2;
        let mut database = CompilerDatabase::new(atomic);
        let actions = database
            .refactorings(Span {
                start: cursor,
                end: cursor,
            })
            .unwrap();
        let remove = actions
            .iter()
            .find(|action| action.kind == RefactoringKind::RemoveInspection)
            .unwrap();
        assert_eq!(
            apply(atomic, remove),
            "state \"game.exe\" {}\nfn score(x: i32) { return x }\n"
        );
    }

    #[test]
    fn inspection_rewrites_target_the_expression_at_a_cursor_and_avoid_double_parentheses() {
        let source = "state \"game.exe\" {}\nfn score(x: i32) { return (x + 1) }\n";
        let mut database = CompilerDatabase::new(source);
        let actions = database.refactorings(selection(source, "(x + 1)")).unwrap();
        let inspect = actions
            .iter()
            .find(|action| action.kind == RefactoringKind::InspectExpression)
            .unwrap();
        assert_eq!(
            apply(source, inspect),
            "state \"game.exe\" {}\nfn score(x: i32) { return inspect(x + 1) }\n"
        );

        let cursor = source.find("x + 1").unwrap();
        let mut database = CompilerDatabase::new(source);
        let actions = database
            .refactorings(Span {
                start: cursor,
                end: cursor,
            })
            .unwrap();
        let inspect = actions
            .iter()
            .find(|action| action.kind == RefactoringKind::InspectExpression)
            .unwrap();
        assert_eq!(
            apply(source, inspect),
            "state \"game.exe\" {}\nfn score(x: i32) { return (inspect(x) + 1) }\n"
        );
    }

    #[test]
    fn extracts_a_function_with_local_and_contextual_parameters() {
        let source = concat!(
            "state \"game.exe\" { level: i32 = 0 }\n",
            "whileAttached {\n",
            "    let offset: i32 = 1\n",
            "    print(current.level + offset)\n",
            "}\n"
        );
        let mut database = CompilerDatabase::new(source);
        let actions = database
            .refactorings(selection(source, "current.level + offset"))
            .unwrap();
        let action = actions
            .iter()
            .find(|action| action.kind == RefactoringKind::ExtractFunction)
            .unwrap();
        let output = apply(source, action);
        assert!(
            output.contains("extracted(current.level, offset)"),
            "{output}"
        );
        assert!(
            output.contains("fn extracted(currentLevel: i32, offset: i32)"),
            "{output}"
        );
        assert!(output.contains("return currentLevel + offset"), "{output}");
    }

    #[test]
    fn extracts_contiguous_statements_into_a_function() {
        let source = concat!(
            "state \"game.exe\" {\n",
            "    points: i32 = 0;\n",
            "    deaths: i32 = 0;\n",
            "    level: i32 = 0;\n",
            "}\n",
            "whileAttached {\n",
            "    setVariable(\"Points\", current.points)\n",
            "    setVariable(\"Deaths\", current.deaths)\n",
            "    setVariable(\"Level\", current.level)\n",
            "}\n"
        );
        let selected = concat!(
            "setVariable(\"Points\", current.points)\n",
            "    setVariable(\"Deaths\", current.deaths)\n",
            "    setVariable(\"Level\", current.level)"
        );
        let mut database = CompilerDatabase::new(source);
        let actions = database.refactorings(selection(source, selected)).unwrap();
        assert_eq!(actions.len(), 1, "{actions:#?}");
        let output = apply(source, &actions[0]);
        assert!(
            output.contains("extracted(current.points, current.deaths, current.level)"),
            "{output}"
        );
        assert!(
            output.contains(concat!(
                "fn extracted(currentPoints: i32, currentDeaths: i32, currentLevel: i32) {\n",
                "    setVariable(\"Points\", currentPoints)\n",
                "    setVariable(\"Deaths\", currentDeaths)\n",
                "    setVariable(\"Level\", currentLevel)\n",
                "}"
            )),
            "{output}"
        );

        let mut database = CompilerDatabase::new(source);
        let actions = database
            .refactorings(selection(source, "setVariable(\"Points\", current.points)"))
            .unwrap();
        assert!(
            actions
                .iter()
                .any(|action| action.kind == RefactoringKind::ExtractFunction),
            "{actions:#?}"
        );
    }

    #[test]
    fn statement_extraction_rejects_escaping_locals_and_caller_mutation() {
        let escaping = concat!(
            "state \"game.exe\" {}\n",
            "fn sample(input: i32) {\n",
            "    let doubled = input * 2\n",
            "    print(doubled)\n",
            "}\n"
        );
        let mut database = CompilerDatabase::new(escaping);
        assert!(
            database
                .refactorings(selection(escaping, "let doubled = input * 2"))
                .unwrap()
                .is_empty()
        );

        let mutation = concat!(
            "state \"game.exe\" {}\n",
            "fn sample() {\n",
            "    let value = 1\n",
            "    value += 1\n",
            "    print(value)\n",
            "}\n"
        );
        let mut database = CompilerDatabase::new(mutation);
        assert!(
            database
                .refactorings(selection(mutation, "value += 1"))
                .unwrap()
                .is_empty()
        );
    }

    #[test]
    fn only_offers_actions_for_exact_expressions() {
        let source = "state \"game.exe\" {}\nfn score(x: i32) { return x + 1 }\n";
        let mut database = CompilerDatabase::new(source);
        let start = source.find("x + 1").unwrap();
        assert!(
            database
                .refactorings(Span {
                    start,
                    end: start + 3,
                })
                .unwrap()
                .is_empty()
        );
    }

    #[test]
    fn extraction_preserves_await_and_propagation_boundaries() {
        let async_source = concat!(
            "state \"game.exe\" {}\n",
            "onAttach {\n",
            "    let module = await process.module(\"game.dll\")\n",
            "}\n"
        );
        let async_expression = "await process.module(\"game.dll\")";
        let mut database = CompilerDatabase::new(async_source);
        let actions = database
            .refactorings(selection(async_source, async_expression))
            .unwrap();
        let extracted = actions
            .iter()
            .find(|action| action.kind == RefactoringKind::ExtractFunction)
            .unwrap();
        let output = apply(async_source, extracted);
        assert!(
            output.contains("let module = (await extracted())"),
            "{output}"
        );
        assert!(
            output.contains("return await process.module(\"game.dll\")"),
            "{output}"
        );

        let result_source = concat!(
            "state \"game.exe\" {}\n",
            "fn readValue(offset: address) -> i32! {\n",
            "    return process.read<i32>(offset)?\n",
            "}\n"
        );
        let result_expression = "process.read<i32>(offset)?";
        let mut database = CompilerDatabase::new(result_source);
        let selected = selection(result_source, result_expression);
        let actions = database.refactorings(selected).unwrap();
        let extracted = actions
            .iter()
            .find(|action| action.kind == RefactoringKind::ExtractFunction)
            .unwrap_or_else(|| {
                let checked = database.check().unwrap();
                let snapshot = SemanticSnapshot::Checked(checked);
                let expression = exact_expression(snapshot.syntax(), selected).unwrap();
                let proposed = extract_function(result_source, &snapshot, expression).unwrap();
                let proposed_source = apply(result_source, &proposed);
                let diagnostics = CompilerDatabase::new(&proposed_source).diagnostics();
                panic!("no extraction was offered:\n{proposed_source}\n{diagnostics:#?}")
            });
        let output = apply(result_source, extracted);
        assert!(output.contains("return (extracted(offset)?)"), "{output}");
        assert!(
            output.contains("return process.read<i32>(offset)?"),
            "{output}"
        );
    }

    #[test]
    fn local_extraction_does_not_hoist_conditionally_evaluated_work() {
        let source = concat!(
            "state \"game.exe\" {}\n",
            "fn check(enabled: bool, value: i32) {\n",
            "    return enabled && value > 0\n",
            "}\n"
        );
        let mut database = CompilerDatabase::new(source);
        let actions = database
            .refactorings(selection(source, "value > 0"))
            .unwrap();
        assert!(
            actions
                .iter()
                .all(|action| action.kind != RefactoringKind::ExtractVariable)
        );
        assert!(
            actions
                .iter()
                .any(|action| action.kind == RefactoringKind::ExtractFunction)
        );

        let source = concat!(
            "state \"game.exe\" {}\n",
            "fn count(limit: i32) {\n",
            "    let index = 0\n",
            "    while index < limit { index += 1 }\n",
            "}\n"
        );
        let mut database = CompilerDatabase::new(source);
        let actions = database
            .refactorings(selection(source, "index < limit"))
            .unwrap();
        assert!(
            actions
                .iter()
                .all(|action| action.kind != RefactoringKind::ExtractVariable)
        );
    }
}
