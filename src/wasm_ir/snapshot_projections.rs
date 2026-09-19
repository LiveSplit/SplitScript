//! Conservative reuse of immutable managed-snapshot projections.
//!
//! This is deliberately separate from general common-subexpression
//! elimination. Host process memory can mutate concurrently; only values that
//! have already entered a compiler-created state snapshot are stable here.

use std::collections::{HashMap, HashSet};

use crate::{
    ast::ValueId,
    semantic::{ResolvedMember, ResolvedReceiver, ResolvedValue, SemanticModel},
    types::TypeKind,
};

use super::{
    Block, CallTarget, Expression, ExpressionKind, Local, LocalId, LocalPurpose, Program,
    SnapshotProjection, SnapshotRoot, Visitor, walk_expression,
};

/// Adds profitable projection locals to one synchronous body's existing local
/// plan. Callers must not invoke this for a body that can suspend across ticks.
pub(super) fn plan(
    block: &Block,
    program: &Program,
    semantics: &SemanticModel,
    mutated_values: &HashSet<ValueId>,
    locals: &mut Vec<Local>,
) {
    let mut collector = Collector {
        semantics,
        mutated_values,
        counts: HashMap::new(),
    };
    Visitor::visit_block(&mut collector, block, program);
    let mut projections = collector
        .counts
        .into_iter()
        // Loading a complete state field needs three uses to repay its local
        // declaration and initialization. A direct managed field eliminates
        // the additional GC field projection as well, making two uses the
        // first profitable case in the Wasm binary encoding.
        .filter_map(|(projection, count)| {
            let minimum_uses = if projection.member.is_some() { 2 } else { 3 };
            (count >= minimum_uses).then_some(projection)
        })
        .collect::<Vec<_>>();
    projections.sort_by_key(|projection| {
        (
            matches!(projection.root, SnapshotRoot::Old),
            projection.field.index(),
            projection.member.is_some(),
            projection.member.map_or(0, |member| member.index()),
        )
    });
    for projection in projections {
        let ty = projection.member.map_or_else(
            || {
                semantics
                    .value_type(projection.field)
                    .expect("checked state fields have semantic types")
            },
            |member| {
                semantics
                    .managed_field_snapshot_type(member)
                    .expect("checked managed fields have semantic value types")
            },
        );
        let id = LocalId(locals.len());
        locals.push(Local {
            id,
            ty,
            purpose: LocalPurpose::SnapshotProjection(projection),
        });
    }
}

struct Collector<'a> {
    semantics: &'a SemanticModel,
    mutated_values: &'a HashSet<ValueId>,
    counts: HashMap<SnapshotProjection, usize>,
}

impl Collector<'_> {
    fn record_value(&mut self, value: ResolvedValue, members: &[ResolvedMember]) {
        let (root, field) = match value {
            ResolvedValue::CurrentState(field) => (SnapshotRoot::Current, field),
            ResolvedValue::OldState(field) => (SnapshotRoot::Old, field),
            _ => return,
        };
        if root == SnapshotRoot::Current && self.mutated_values.contains(&field) {
            return;
        }
        let Some(ty) = self.semantics.value_type(field) else {
            return;
        };
        if !matches!(self.semantics.types().kind(ty), TypeKind::ManagedClass(_)) {
            return;
        }
        *self
            .counts
            .entry(SnapshotProjection {
                root,
                field,
                member: None,
            })
            .or_default() += 1;
        if let Some(ResolvedMember::ManagedField(member)) = members.first() {
            *self
                .counts
                .entry(SnapshotProjection {
                    root,
                    field,
                    member: Some(*member),
                })
                .or_default() += 1;
        }
    }

    fn record_receiver(&mut self, receiver: &ResolvedReceiver) {
        if let ResolvedReceiver::Path { root, members } = receiver {
            self.record_value(*root, members);
        }
    }
}

impl Visitor for Collector<'_> {
    fn visit_expression(&mut self, expression: &Expression, program: &Program) {
        match &expression.kind {
            ExpressionKind::Path {
                root: Some(root),
                members,
            } => self.record_value(*root, members),
            ExpressionKind::Call { target, .. } => {
                let receiver = match target {
                    CallTarget::UserMethod { receiver, .. }
                    | CallTarget::CapabilityRequirement { receiver, .. }
                    | CallTarget::DefaultFormatting { receiver, .. }
                    | CallTarget::GeneratorNext { receiver, .. }
                    | CallTarget::IteratorIdentity { receiver, .. }
                    | CallTarget::ManagedSnapshot { receiver, .. }
                    | CallTarget::ManagedComponent { receiver, .. } => Some(receiver),
                    CallTarget::Intrinsic { receiver, .. }
                    | CallTarget::LibraryOverload { receiver, .. } => receiver.as_ref(),
                    CallTarget::UserFunction { .. }
                    | CallTarget::ManagedInstances { .. }
                    | CallTarget::ResultError { .. }
                    | CallTarget::OptionSome { .. }
                    | CallTarget::IteratorItem { .. }
                    | CallTarget::ResultSuccess { .. } => None,
                };
                if let Some(receiver) = receiver {
                    self.record_receiver(receiver);
                }
            }
            _ => {}
        }
        walk_expression(self, expression, program);
    }
}

#[cfg(test)]
mod tests {
    use crate::{
        ast::ActionKind,
        wasm_ir::{BodyOwner, LocalPurpose, SnapshotProjection, SnapshotRoot},
    };

    #[test]
    fn direct_bodies_reuse_repeated_immutable_managed_snapshot_roots() {
        let source = r#"
image "Assembly-CSharp" {
    class Manager {
        static Manager instance;
        u32 points;
        u32 deaths;
    }
}

state Unity.il2cpp(Il2CppProfile.unity2021_3_11f1X64()) "game.exe" {
    manager: Manager = Manager.instance?.snapshot()?;
}

whileAttached {
    print(current.manager.points)
    print(current.manager.deaths)
    print(current.manager.points)
}

split {
    return old.manager.points != current.manager.points
        || old.manager.deaths != current.manager.deaths
        || old.manager.points > current.manager.points
}
"#;
        let checked = crate::check(crate::lower(crate::parse(source).unwrap())).unwrap();
        let backend = crate::lower_wasm(&checked);
        let manager = checked.syntax().state.as_ref().unwrap().fields[0].id;

        let while_attached = backend
            .wasm_ir()
            .body(BodyOwner::Action(ActionKind::WhileAttached))
            .unwrap();
        assert!(while_attached.locals.iter().any(|local| {
            local.purpose
                == LocalPurpose::SnapshotProjection(SnapshotProjection {
                    root: SnapshotRoot::Current,
                    field: manager,
                    member: None,
                })
        }));
        assert!(while_attached.locals.iter().any(|local| {
            matches!(
                local.purpose,
                LocalPurpose::SnapshotProjection(SnapshotProjection {
                    root: SnapshotRoot::Current,
                    field,
                    member: Some(_),
                }) if field == manager
            )
        }));

        let split = backend
            .wasm_ir()
            .body(BodyOwner::Action(ActionKind::Split))
            .unwrap();
        assert!(split.locals.iter().any(|local| {
            local.purpose
                == LocalPurpose::SnapshotProjection(SnapshotProjection {
                    root: SnapshotRoot::Current,
                    field: manager,
                    member: None,
                })
        }));
        assert!(split.locals.iter().any(|local| {
            local.purpose
                == LocalPurpose::SnapshotProjection(SnapshotProjection {
                    root: SnapshotRoot::Old,
                    field: manager,
                    member: None,
                })
        }));
    }

    #[test]
    fn assignable_current_managed_snapshot_roots_are_not_reused() {
        let source = r#"
image "Assembly-CSharp" {
    class Manager {
        static Manager instance;
        u32 points;
    }
}

state Unity.il2cpp(Il2CppProfile.unity2021_3_11f1X64()) "game.exe" {
    manager: Manager = Manager.instance?.snapshot()?;
}

whileAttached {
    current.manager = old.manager
    print(current.manager.points)
    print(current.manager.points)
    print(current.manager.points)
}
"#;
        let checked = crate::check(crate::lower(crate::parse(source).unwrap())).unwrap();
        let backend = crate::lower_wasm(&checked);
        let manager = checked.syntax().state.as_ref().unwrap().fields[0].id;
        let body = backend
            .wasm_ir()
            .body(BodyOwner::Action(ActionKind::WhileAttached))
            .unwrap();
        assert!(!body.locals.iter().any(|local| {
            matches!(
                local.purpose,
                LocalPurpose::SnapshotProjection(SnapshotProjection {
                    root: SnapshotRoot::Current,
                    field,
                    ..
                }) if field == manager
            )
        }));
    }
}
