//! Snapshot-transaction planning for managed static field reads.
//!
//! State fields are emitted as separate Wasm functions, so ordinary local
//! common-subexpression elimination cannot share a singleton lookup between
//! them. This plan gives all reachable managed-read bodies access to nullable
//! cache slots and an explicit transaction flag. `update` clears and activates
//! the slots while assembling a candidate state; calls outside that boundary
//! always perform a fresh read.

use std::collections::{HashMap, HashSet};

use wasm_encoder::{ConstExpr, GlobalSection, GlobalType, RefType, ValType};

use crate::{
    ast::ManagedFieldId,
    managed::{ManagedBindingPlan, ManagedFieldKind},
    semantic::{ResolvedReceiver, ResolvedValue, SemanticModel},
    wasm_ir,
};

use super::{GcLayout, Type};

#[derive(Debug, Default)]
pub(super) struct ManagedStateReadCache {
    active: Option<u32>,
    entries: Vec<ManagedStateReadStorage>,
}

#[derive(Debug, Clone, Copy)]
pub(super) struct ManagedStateReadStorage {
    pub class: crate::ast::ManagedClassId,
    pub field: ManagedFieldId,
    pub global: u32,
    pub result: crate::ast::ResultTypeId,
}

impl ManagedStateReadCache {
    pub fn active(&self) -> Option<u32> {
        self.active
    }

    pub fn entries(&self) -> impl Iterator<Item = ManagedStateReadStorage> + '_ {
        self.entries.iter().copied()
    }

    pub fn get(&self, field: ManagedFieldId) -> Option<ManagedStateReadStorage> {
        self.entries
            .iter()
            .find(|entry| entry.field == field)
            .copied()
    }
}

pub(super) fn encode(
    section: &mut GlobalSection,
    semantics: &SemanticModel,
    gc: &GcLayout,
    wasm_ir: &wasm_ir::Program,
    managed: &ManagedBindingPlan,
    reachability: &super::reachability::Reachability,
) -> ManagedStateReadCache {
    // Collect every referenced static rather than only direct state-expression
    // paths. A helper called by a state expression is emitted once and must be
    // able to join the transaction too; the runtime `active` flag determines
    // whether its read is cached at a particular call site.
    let mut collector = ManagedStaticCollector {
        fields: HashSet::new(),
        reachability,
    };
    wasm_ir::Visitor::visit_program(&mut collector, wasm_ir);

    let static_fields = managed
        .classes
        .iter()
        .flat_map(|class| {
            class
                .fields
                .iter()
                .chain(
                    class
                        .conditional_fields
                        .iter()
                        .flat_map(|group| &group.fields),
                )
                .map(|field| (class.id, field))
        })
        .filter(|(_, field)| field.kind == ManagedFieldKind::Static)
        .map(|(class, field)| (field.id, (class, field.value_type)))
        .collect::<HashMap<_, _>>();

    let mut fields = collector.fields.into_iter().collect::<Vec<_>>();
    fields.sort_by_key(|field| field.index());
    let active = (!fields.is_empty()).then(|| {
        let global = section.len();
        section.global(
            GlobalType {
                val_type: ValType::I32,
                mutable: true,
                shared: false,
            },
            &ConstExpr::i32_const(0),
        );
        global
    });
    let entries = fields
        .into_iter()
        .filter_map(|field| {
            let (class, value) = static_fields.get(&field).copied()?;
            let result = semantics.types().iter().find_map(|(_, kind)| match kind {
                crate::types::TypeKind::Result {
                    layout,
                    value: candidate,
                } if *candidate == value => Some(*layout),
                _ => None,
            })?;
            let global = section.len();
            let ValType::Ref(reference) = gc.val_type(Type::Result(result)) else {
                unreachable!("managed reads use GC-backed Result values")
            };
            section.global(
                GlobalType {
                    val_type: ValType::Ref(RefType {
                        nullable: true,
                        heap_type: reference.heap_type,
                    }),
                    mutable: true,
                    shared: false,
                },
                &ConstExpr::ref_null(reference.heap_type),
            );
            Some(ManagedStateReadStorage {
                class,
                field,
                global,
                result,
            })
        })
        .collect();
    ManagedStateReadCache { active, entries }
}

struct ManagedStaticCollector<'a> {
    fields: HashSet<ManagedFieldId>,
    reachability: &'a super::reachability::Reachability,
}

impl wasm_ir::Visitor for ManagedStaticCollector<'_> {
    fn visit_expression(&mut self, expression: &wasm_ir::Expression, program: &wasm_ir::Program) {
        if !self.reachability.contains_expression(expression.id) {
            return;
        }
        if let wasm_ir::ExpressionKind::Path {
            root: Some(ResolvedValue::ManagedStatic { field, .. }),
            ..
        } = &expression.kind
        {
            self.fields.insert(*field);
        }
        if let wasm_ir::ExpressionKind::Call { target, .. } = &expression.kind {
            let receiver = match target {
                wasm_ir::CallTarget::UserMethod { receiver, .. }
                | wasm_ir::CallTarget::ManagedSnapshot { receiver, .. }
                | wasm_ir::CallTarget::ManagedComponent { receiver, .. }
                | wasm_ir::CallTarget::CapabilityRequirement { receiver, .. }
                | wasm_ir::CallTarget::DefaultFormatting { receiver, .. }
                | wasm_ir::CallTarget::GeneratorNext { receiver, .. }
                | wasm_ir::CallTarget::IteratorIdentity { receiver, .. } => Some(receiver),
                wasm_ir::CallTarget::Intrinsic { receiver, .. }
                | wasm_ir::CallTarget::LibraryOverload { receiver, .. } => receiver.as_ref(),
                wasm_ir::CallTarget::UserFunction { .. }
                | wasm_ir::CallTarget::ManagedInstances { .. }
                | wasm_ir::CallTarget::ResultError { .. }
                | wasm_ir::CallTarget::OptionSome { .. }
                | wasm_ir::CallTarget::IteratorItem { .. }
                | wasm_ir::CallTarget::ResultSuccess { .. } => None,
            };
            if let Some(ResolvedReceiver::Path {
                root: ResolvedValue::ManagedStatic { field, .. },
                ..
            }) = receiver
            {
                self.fields.insert(*field);
            }
        }
        wasm_ir::walk_expression(self, expression, program);
    }
}
