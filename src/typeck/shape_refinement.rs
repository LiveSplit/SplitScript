//! Symbolic refinement for finite attachment-shape dimensions.
//!
//! Shape conditions are ordinary boolean expressions in the syntax tree,
//! but declarations deliberately accept only predicates the compiler can
//! prove statically. Keeping the canonical facts here lets state fields,
//! managed metadata, function effects, and code generation share one model.

use std::collections::HashMap;

use crate::{
    Diagnostic,
    ast::{BinaryOp, ConditionalFieldsDecl, Expr, ExprKind, MatchPattern, UnaryOp},
    types::ResolvedTypeRef,
};

use super::{
    Checker,
    declarations::{ShapeConstraint, ShapeDimension, ShapePredicate},
};

impl Checker {
    pub(super) fn is_attachment_shape_global(&self, value: crate::ast::ValueId) -> bool {
        self.shape_dimensions
            .contains(&ShapeDimension::Global(value))
    }

    /// Extracts a conjunction of enum-variant facts from an arbitrary
    /// condition. Returning `None` merely means that the expression does not
    /// refine shape-dependent declarations.
    pub(super) fn shape_constraints(&self, expression: &Expr) -> Option<Vec<ShapeConstraint>> {
        let mut constraints = Vec::new();
        self.collect_shape_constraints(expression, &mut constraints)?;
        let mut dimensions = HashMap::new();
        for constraint in &constraints {
            if dimensions
                .insert(constraint.dimension, constraint.variant)
                .is_some_and(|previous| previous != constraint.variant)
            {
                return None;
            }
        }
        constraints.sort_by_key(|constraint| shape_dimension_sort_key(constraint.dimension));
        constraints.dedup();
        Some(constraints)
    }

    pub(super) fn shape_match_constraints(
        &self,
        value: &Expr,
        pattern: &MatchPattern,
    ) -> Option<Vec<ShapeConstraint>> {
        self.shape_is_constraint_atom(value, pattern)
            .map(|constraint| vec![constraint])
    }

    /// Derives the facts established by the false branch when that complement
    /// is itself one exact shape assignment. At present this is possible for
    /// a single equality over a two-variant enum. Broader predicates would
    /// require a disjunction rather than the canonical conjunction represented
    /// by [`ShapeConstraint`].
    pub(super) fn inverse_shape_constraints(
        &self,
        expression: &Expr,
    ) -> Option<Vec<ShapeConstraint>> {
        let constraints = self.shape_constraints(expression)?;
        let [constraint] = constraints.as_slice() else {
            return None;
        };
        let enum_id = self.shape_dimension_enum(constraint.dimension)?;
        let enumeration = self
            .declarations
            .enums
            .iter()
            .find(|enumeration| enumeration.id == enum_id)?;
        if enumeration.variants.len() != 2 {
            return None;
        }
        let variant = enumeration
            .variants
            .iter()
            .find(|variant| variant.id != constraint.variant)?;
        Some(vec![ShapeConstraint {
            dimension: constraint.dimension,
            variant: variant.id,
        }])
    }

    /// Returns shape facts that are necessarily true whenever `expression`
    /// is true. Unlike declaration predicates, ordinary boolean expressions
    /// may contain unrelated conditions; a conjunction still preserves every
    /// shape fact contributed by either side.
    pub(super) fn truthy_shape_constraints(&self, expression: &Expr) -> Vec<ShapeConstraint> {
        let mut candidates = Vec::new();
        self.collect_truthy_shape_constraints(expression, &mut candidates);
        canonical_constraints(candidates)
    }

    /// Returns shape facts that are necessarily true whenever `expression`
    /// is false. A disjunction must have every operand false, so exact
    /// two-variant complements remain available through an entire `||` chain.
    pub(super) fn falsy_shape_constraints(&self, expression: &Expr) -> Vec<ShapeConstraint> {
        let mut candidates = Vec::new();
        self.collect_falsy_shape_constraints(expression, &mut candidates);
        canonical_constraints(candidates)
    }

    /// Resolves flat parser branches into exact, mutually exclusive shape
    /// alternatives. A branch introduced by `else` starts with the shapes
    /// left unmatched by the preceding branches in the same chain.
    pub(super) fn shape_branch_predicates<Field>(
        &mut self,
        groups: &[ConditionalFieldsDecl<Field>],
    ) -> Vec<ShapePredicate> {
        for condition in groups.iter().filter_map(|group| group.condition.as_ref()) {
            self.collect_declared_shape_dimensions(condition);
        }
        let universe = self.shape_assignments();
        if universe.is_empty() && !groups.is_empty() {
            self.errors.push(
                Diagnostic::type_error(
                    "conditional fields require a bounded attachment shape",
                    groups[0].keyword_span,
                )
                .with_primary_label("this conditional declaration needs exact shape branches")
                .with_note(format!(
                    "conditional declarations support at most {} shape combinations",
                    crate::shape_selection::MAX_ENUMERATED_SHAPE_COMBINATIONS,
                )),
            );
        }
        let mut remaining = universe.clone();
        let mut predicates = Vec::with_capacity(groups.len());
        for group in groups {
            if group.else_span.is_none() {
                remaining = universe.clone();
            }
            let selected = if let Some(condition) = &group.condition {
                let mut understood = true;
                let selected = remaining
                    .iter()
                    .filter(|assignment| {
                        self.evaluate_shape_condition(condition, assignment)
                            .unwrap_or_else(|| {
                                understood = false;
                                false
                            })
                    })
                    .cloned()
                    .collect::<Vec<_>>();
                if !understood {
                    self.errors.push(
                        Diagnostic::type_error(
                            "conditional fields need a statically decidable shape predicate",
                            condition.span,
                        )
                        .with_primary_label(
                            "test shape variables with enum variants using `is`, `==`, or `!=` and combine them with `&&`, `||`, or `!`",
                        ),
                    );
                }
                selected
            } else {
                remaining.clone()
            };
            remaining.retain(|assignment| !selected.contains(assignment));
            predicates.push(ShapePredicate {
                alternatives: selected,
            });
        }
        predicates
    }

    pub(super) fn with_shape_constraints<T>(
        &mut self,
        constraints: Option<&[ShapeConstraint]>,
        operation: impl FnOnce(&mut Self) -> T,
    ) -> T {
        let predicate = constraints.map(|constraints| ShapePredicate {
            alternatives: self
                .shape_assignments()
                .into_iter()
                .filter(|assignment| assignment_satisfies_constraints(assignment, constraints))
                .collect(),
        });
        self.with_shape_predicate(predicate.as_ref(), operation)
    }

    pub(super) fn with_shape_predicate<T>(
        &mut self,
        predicate: Option<&ShapePredicate>,
        operation: impl FnOnce(&mut Self) -> T,
    ) -> T {
        let previous = self.active_shapes.clone();
        if let Some(predicate) = predicate {
            let active = self.active_shapes.as_ref().map_or_else(
                || self.shape_assignments(),
                |active| active.alternatives.clone(),
            );
            self.active_shapes = Some(ShapePredicate {
                alternatives: active
                    .into_iter()
                    .filter(|assignment| predicate_matches_assignment(predicate, assignment))
                    .collect(),
            });
        }
        let output = operation(self);
        self.active_shapes = previous;
        output
    }

    pub(super) fn shape_predicate_satisfied(&self, required: &ShapePredicate) -> bool {
        self.active_shape_assignments()
            .iter()
            .all(|assignment| predicate_matches_assignment(required, assignment))
    }

    pub(super) fn shape_predicates_cover_all<'a>(
        &self,
        predicates: impl IntoIterator<Item = &'a ShapePredicate>,
    ) -> bool {
        let predicates = predicates.into_iter().collect::<Vec<_>>();
        self.shape_assignments().iter().all(|assignment| {
            predicates
                .iter()
                .any(|predicate| predicate_matches_assignment(predicate, assignment))
        })
    }

    fn collect_shape_constraints(
        &self,
        expression: &Expr,
        output: &mut Vec<ShapeConstraint>,
    ) -> Option<()> {
        match &expression.kind {
            ExprKind::Inspect { value, .. } => self.collect_shape_constraints(value, output),
            ExprKind::Binary {
                op: BinaryOp::And,
                left,
                right,
            } => {
                self.collect_shape_constraints(left, output)?;
                self.collect_shape_constraints(right, output)
            }
            ExprKind::Binary {
                op: BinaryOp::Eq,
                left,
                right,
            } => self
                .shape_constraint_atom(left, right)
                .or_else(|| self.shape_constraint_atom(right, left))
                .map(|constraint| output.push(constraint)),
            ExprKind::Is { value, pattern, .. } => self
                .shape_is_constraint_atom(value, &pattern.kind)
                .map(|constraint| output.push(constraint)),
            _ => None,
        }
    }

    fn collect_truthy_shape_constraints(
        &self,
        expression: &Expr,
        output: &mut Vec<ShapeConstraint>,
    ) {
        match &expression.kind {
            ExprKind::Inspect { value, .. } => {
                self.collect_truthy_shape_constraints(value, output);
            }
            ExprKind::Binary {
                op: BinaryOp::And,
                left,
                right,
            } => {
                self.collect_truthy_shape_constraints(left, output);
                self.collect_truthy_shape_constraints(right, output);
            }
            ExprKind::Binary {
                op: BinaryOp::Eq,
                left,
                right,
            } => {
                if let Some(constraint) = self
                    .shape_constraint_atom(left, right)
                    .or_else(|| self.shape_constraint_atom(right, left))
                {
                    output.push(constraint);
                }
            }
            ExprKind::Is { value, pattern, .. } => {
                if let Some(constraint) = self.shape_is_constraint_atom(value, &pattern.kind) {
                    output.push(constraint);
                }
            }
            _ => {}
        }
    }

    fn collect_falsy_shape_constraints(
        &self,
        expression: &Expr,
        output: &mut Vec<ShapeConstraint>,
    ) {
        if let ExprKind::Inspect { value, .. } = &expression.kind {
            self.collect_falsy_shape_constraints(value, output);
        } else if let ExprKind::Binary {
            op: BinaryOp::Or,
            left,
            right,
        } = &expression.kind
        {
            self.collect_falsy_shape_constraints(left, output);
            self.collect_falsy_shape_constraints(right, output);
        } else if let Some(constraints) = self.inverse_shape_constraints(expression) {
            output.extend(constraints);
        }
    }

    fn shape_constraint_atom(&self, dimension: &Expr, variant: &Expr) -> Option<ShapeConstraint> {
        let dimension_path = expression_path(dimension)?;
        let variant_path = expression_path(variant)?;
        let [enum_name, variant_name] = variant_path.as_slice() else {
            return None;
        };
        let (dimension, enum_id) = self.resolve_shape_dimension(&dimension_path)?;
        let enumeration = self
            .declarations
            .enums
            .iter()
            .find(|enumeration| enumeration.id == enum_id && enumeration.name == *enum_name)?;
        let variant = enumeration
            .variants
            .iter()
            .find(|variant| variant.name == *variant_name)?;
        Some(ShapeConstraint {
            dimension,
            variant: variant.id,
        })
    }

    fn shape_is_constraint_atom(
        &self,
        dimension: &Expr,
        pattern: &MatchPattern,
    ) -> Option<ShapeConstraint> {
        let MatchPattern::Enum {
            enumeration,
            variant,
            payload: None,
        } = pattern
        else {
            return None;
        };
        let dimension_path = expression_path(dimension)?;
        let (dimension, enum_id) = self.resolve_shape_dimension(&dimension_path)?;
        let enumeration_decl = self
            .declarations
            .enums
            .iter()
            .find(|candidate| candidate.id == enum_id && candidate.name == enumeration.name)?;
        let variant = enumeration_decl
            .variants
            .iter()
            .find(|candidate| candidate.name == *variant)?;
        Some(ShapeConstraint {
            dimension,
            variant: variant.id,
        })
    }

    fn resolve_shape_dimension(
        &self,
        path: &[&str],
    ) -> Option<(ShapeDimension, crate::ast::EnumId)> {
        match path {
            [name] => {
                if let Some(binding) = self.declarations.globals.get(*name) {
                    let value = binding.id?;
                    let ty = self.inference.shallow_readonly(binding.ty);
                    let ResolvedTypeRef::Enum(enum_id) =
                        ty.try_to_ref(self.inference.type_store())?
                    else {
                        return None;
                    };
                    return Some((ShapeDimension::Global(value), enum_id));
                }
                let (value, ty) = self.declarations.state_fields.get(*name).copied()?;
                let ty = self.inference.shallow_readonly(ty);
                let ResolvedTypeRef::Enum(enum_id) = ty.try_to_ref(self.inference.type_store())?
                else {
                    return None;
                };
                Some((ShapeDimension::StateField(value), enum_id))
            }
            ["current", name] => {
                let (value, ty) = self.declarations.state_fields.get(*name).copied()?;
                let ty = self.inference.shallow_readonly(ty);
                let ResolvedTypeRef::Enum(enum_id) = ty.try_to_ref(self.inference.type_store())?
                else {
                    return None;
                };
                Some((ShapeDimension::StateField(value), enum_id))
            }
            _ => None,
        }
    }

    fn shape_dimension_enum(&self, dimension: ShapeDimension) -> Option<crate::ast::EnumId> {
        match dimension {
            ShapeDimension::Global(value) => {
                let ty = self
                    .declarations
                    .globals
                    .values()
                    .find_map(|binding| (binding.id == Some(value)).then_some(binding.ty))?;
                let ty = self.inference.shallow_readonly(ty);
                let ResolvedTypeRef::Enum(enum_id) = ty.try_to_ref(self.inference.type_store())?
                else {
                    return None;
                };
                Some(enum_id)
            }
            ShapeDimension::StateField(value) => {
                let ty = self.declarations.state_fields_by_id.get(&value).copied()?;
                let ty = self.inference.shallow_readonly(ty);
                let ResolvedTypeRef::Enum(enum_id) = ty.try_to_ref(self.inference.type_store())?
                else {
                    return None;
                };
                Some(enum_id)
            }
        }
    }

    fn collect_declared_shape_dimensions(&mut self, expression: &Expr) {
        let mut paths = Vec::new();
        collect_expression_paths(expression, &mut paths);
        for path in paths {
            let Some((dimension, _)) = self.resolve_shape_dimension(&path) else {
                continue;
            };
            if !self.shape_dimensions.contains(&dimension) {
                self.shape_dimensions.push(dimension);
            }
        }
    }

    fn shape_assignments(&self) -> Vec<Vec<ShapeConstraint>> {
        let mut assignments = vec![Vec::new()];
        for dimension in &self.shape_dimensions {
            let Some(enum_id) = self.shape_dimension_enum(*dimension) else {
                return Vec::new();
            };
            let Some(enumeration) = self
                .declarations
                .enums
                .iter()
                .find(|enumeration| enumeration.id == enum_id)
            else {
                return Vec::new();
            };
            if assignments
                .len()
                .checked_mul(enumeration.variants.len())
                .is_none_or(|count| {
                    count > crate::shape_selection::MAX_ENUMERATED_SHAPE_COMBINATIONS
                })
            {
                return Vec::new();
            }
            assignments = assignments
                .into_iter()
                .flat_map(|assignment| {
                    enumeration.variants.iter().map(move |variant| {
                        let mut assignment = assignment.clone();
                        assignment.push(ShapeConstraint {
                            dimension: *dimension,
                            variant: variant.id,
                        });
                        assignment
                    })
                })
                .collect();
        }
        assignments
    }

    fn active_shape_assignments(&self) -> Vec<Vec<ShapeConstraint>> {
        self.active_shapes.as_ref().map_or_else(
            || self.shape_assignments(),
            |active| active.alternatives.clone(),
        )
    }

    fn evaluate_shape_condition(
        &self,
        expression: &Expr,
        assignment: &[ShapeConstraint],
    ) -> Option<bool> {
        match &expression.kind {
            ExprKind::Inspect { value, .. } => self.evaluate_shape_condition(value, assignment),
            ExprKind::Bool(value) => Some(*value),
            ExprKind::Unary {
                op: UnaryOp::Not,
                expr,
            } => Some(!self.evaluate_shape_condition(expr, assignment)?),
            ExprKind::Binary { op, left, right } => match op {
                BinaryOp::And => Some(
                    self.evaluate_shape_condition(left, assignment)?
                        && self.evaluate_shape_condition(right, assignment)?,
                ),
                BinaryOp::Or => Some(
                    self.evaluate_shape_condition(left, assignment)?
                        || self.evaluate_shape_condition(right, assignment)?,
                ),
                BinaryOp::Eq | BinaryOp::Ne => {
                    let constraint = self
                        .shape_constraint_atom(left, right)
                        .or_else(|| self.shape_constraint_atom(right, left))?;
                    let equal = assignment.contains(&constraint);
                    Some(if *op == BinaryOp::Eq { equal } else { !equal })
                }
                _ => None,
            },
            ExprKind::Is { value, pattern, .. } => {
                let constraint = self.shape_is_constraint_atom(value, &pattern.kind)?;
                Some(assignment.contains(&constraint))
            }
            _ => None,
        }
    }
}

fn canonical_constraints(candidates: Vec<ShapeConstraint>) -> Vec<ShapeConstraint> {
    let mut dimensions = HashMap::new();
    for constraint in candidates {
        dimensions
            .entry(constraint.dimension)
            .and_modify(|variant: &mut Option<_>| {
                if *variant != Some(constraint.variant) {
                    *variant = None;
                }
            })
            .or_insert(Some(constraint.variant));
    }
    let mut constraints = dimensions
        .into_iter()
        .filter_map(|(dimension, variant)| {
            variant.map(|variant| ShapeConstraint { dimension, variant })
        })
        .collect::<Vec<_>>();
    constraints.sort_by_key(|constraint| shape_dimension_sort_key(constraint.dimension));
    constraints
}

fn shape_dimension_sort_key(dimension: ShapeDimension) -> (u8, usize) {
    match dimension {
        ShapeDimension::Global(value) => (0, value.index()),
        ShapeDimension::StateField(value) => (1, value.index()),
    }
}

fn predicate_matches_assignment(
    predicate: &ShapePredicate,
    assignment: &[ShapeConstraint],
) -> bool {
    predicate.alternatives.iter().any(|alternative| {
        alternative
            .iter()
            .all(|constraint| assignment.contains(constraint))
    })
}

fn collect_expression_paths<'a>(expression: &'a Expr, output: &mut Vec<Vec<&'a str>>) {
    match &expression.kind {
        ExprKind::Inspect { value, .. } => collect_expression_paths(value, output),
        ExprKind::Unary { expr, .. } => collect_expression_paths(expr, output),
        ExprKind::Binary { left, right, .. } => {
            collect_expression_paths(left, output);
            collect_expression_paths(right, output);
        }
        ExprKind::Is { value, .. } => collect_expression_paths(value, output),
        _ => {
            if let Some(path) = expression_path(expression) {
                output.push(path);
            }
        }
    }
}

fn assignment_satisfies_constraints(
    assignment: &[ShapeConstraint],
    constraints: &[ShapeConstraint],
) -> bool {
    constraints
        .iter()
        .all(|constraint| assignment.contains(constraint))
}

fn expression_path(expression: &Expr) -> Option<Vec<&str>> {
    match &expression.kind {
        ExprKind::Inspect { value, .. } => expression_path(value),
        ExprKind::Path(path) => Some(path.iter().map(String::as_str).collect()),
        ExprKind::Member { receiver, name, .. } => {
            let mut path = expression_path(receiver)?;
            path.push(name);
            Some(path)
        }
        _ => None,
    }
}
