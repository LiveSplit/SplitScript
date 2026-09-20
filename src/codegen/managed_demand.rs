//! Remove unused compiler-generated metadata lookups before async lowering.
//!
//! Demand comes from resolved, profile-filtered operations. The preparation
//! function is deliberately not a root: its own lookups cannot create demand.
//! User declarations and user-authored effects are never removed here.

use std::collections::{HashMap, HashSet};

use crate::{
    ast::{Block, Expr, ExprKind, Program, Stmt},
    semantic::{ResolvedMember, ResolvedValue, SemanticModel},
    stdlib::{
        PROVIDER_BINDINGS_TYPE, PROVIDER_PREPARATION_FUNCTION, managed_field_offset_name,
        managed_field_presence_name, managed_instance_header_name,
        managed_static_field_address_name,
    },
    wasm_ir,
};

pub(super) fn prune(
    program: &Program,
    semantics: &mut SemanticModel,
    wasm: &wasm_ir::Program,
    capabilities: &crate::capabilities::CapabilityAnalysis,
) -> Option<Program> {
    let bindings = program
        .structs
        .iter()
        .find(|s| s.name == PROVIDER_BINDINGS_TYPE)?;
    let reachable = super::reachability::Reachability::analyze(
        program,
        semantics,
        wasm,
        wasm.standard_library(),
        capabilities,
        [],
    );
    let mut fields = HashSet::new();
    let mut headers = HashSet::new();
    for (owner, id) in reachable.expression_instances() {
        let expression = wasm.expression(id).expect("reachable expression exists");
        let members = match &expression.kind {
            wasm_ir::ExpressionKind::Path { root, members } => {
                if let Some(ResolvedValue::ManagedStatic { field, .. }) = root {
                    fields.insert(*field);
                }
                members.as_slice()
            }
            wasm_ir::ExpressionKind::Member { members, .. } => members.as_slice(),
            wasm_ir::ExpressionKind::Call { target, .. } => {
                let target = reachable.resolved_call_target(owner.as_ref(), id, target);
                match target {
                    wasm_ir::CallTarget::ManagedInstances { class }
                    | wasm_ir::CallTarget::ManagedComponent { class, .. } => {
                        headers.insert(*class);
                    }
                    _ => {}
                }
                if let Some((receiver, _)) = target.receiver_with_type() {
                    if let Some((ResolvedValue::ManagedStatic { field, .. }, _)) = receiver.path() {
                        fields.insert(field);
                    }
                    receiver.members()
                } else {
                    &[]
                }
            }
            _ => &[],
        };
        for member in members {
            if let ResolvedMember::ManagedField(field) = member {
                fields.insert(*field);
            }
        }
    }
    let managed = crate::managed::ManagedBindingPlan::build(program, semantics);
    for class in reachable.managed_snapshots() {
        let class = managed
            .classes
            .iter()
            .find(|candidate| candidate.id == class)
            .unwrap();
        fields.extend(
            class
                .all_fields()
                .filter(|field| field.kind == crate::managed::ManagedFieldKind::Instance)
                .map(|field| field.id),
        );
    }
    // Evidence is an attachment operation even if no script expression reads it.
    if !crate::shape_selection::has_explicit_shape_selection(program)
        && let Some(shape) = &managed.automatic_shape
    {
        fields.extend(&shape.evidence_fields);
    }
    let mut remove = HashSet::new();
    if !reachable.managed_decoders().any(|ty| {
        matches!(
            capabilities.managed_decoder(ty).unwrap().kind,
            crate::managed_read::ManagedDecoderKind::Array { .. }
        )
    }) {
        remove.insert(crate::stdlib::MANAGED_ARRAY_TYPE_FIELD.to_owned());
        remove.insert("__array_layout_cache".to_owned());
    }
    if !reachable.managed_decoders().any(|ty| {
        matches!(
            capabilities.managed_decoder(ty).unwrap().kind,
            crate::managed_read::ManagedDecoderKind::List { .. }
        )
    }) {
        remove.insert(crate::stdlib::MANAGED_LIST_LAYOUT_FIELD.to_owned());
        remove.insert("__list_layout_cache".to_owned());
    }
    if !reachable.managed_decoders().any(|ty| {
        matches!(
            capabilities.managed_decoder(ty).unwrap().kind,
            crate::managed_read::ManagedDecoderKind::Map { .. }
        )
    }) {
        remove.insert(crate::stdlib::MANAGED_MAP_READ_FIELD.to_owned());
        remove.insert("__map_layout_cache".to_owned());
    }
    if !reachable.managed_decoders().any(|ty| {
        matches!(
            capabilities.managed_decoder(ty).unwrap().kind,
            crate::managed_read::ManagedDecoderKind::Set { .. }
        )
    }) {
        remove.insert(crate::stdlib::MANAGED_SET_READ_FIELD.to_owned());
        remove.insert("__set_layout_cache".to_owned());
    }
    if remove.contains(crate::stdlib::MANAGED_MAP_READ_FIELD)
        && remove.contains(crate::stdlib::MANAGED_SET_READ_FIELD)
    {
        remove.insert(crate::stdlib::MANAGED_KEYED_VERIFY_FIELD.to_owned());
    }
    let mut images = HashMap::new();
    let mut needed_images = HashSet::new();
    for class in &managed.classes {
        let next = images.len();
        let image = *images.entry(&class.image_name).or_insert(next);
        let needed =
            headers.contains(&class.id) || class.all_fields().any(|f| fields.contains(&f.id));
        if needed {
            needed_images.insert(image);
        } else {
            remove.insert(format!("__class_{}", class.id.index()));
        }
        if !headers.contains(&class.id) {
            remove.insert(managed_instance_header_name(class.id.index()));
        }
        for field in class.all_fields().filter(|f| !fields.contains(&f.id)) {
            remove.extend([
                managed_field_offset_name(field.id.index()),
                managed_static_field_address_name(field.id.index()),
                managed_field_presence_name(field.id.index()),
                format!("__field_{}_conditional_probe", field.id.index()),
            ]);
        }
    }
    for image in 0..images.len() {
        if !needed_images.contains(&image) {
            remove.insert(format!("__image_{image}"));
        }
    }
    let removed_fields = bindings
        .fields
        .iter()
        .filter(|f| remove.contains(&f.name))
        .map(|f| f.id)
        .collect::<HashSet<_>>();
    // An empty, unused class still has a lookup even without binding fields.
    if removed_fields.is_empty()
        && needed_images.len() == images.len()
        && managed
            .classes
            .iter()
            .all(|c| !remove.contains(&format!("__class_{}", c.id.index())))
    {
        return None;
    }
    let mut pruned = program.clone();
    pruned
        .structs
        .iter_mut()
        .find(|s| s.id == bindings.id)
        .unwrap()
        .fields
        .retain(|f| !removed_fields.contains(&f.id));
    let preparation = pruned
        .functions
        .iter_mut()
        .find(|f| f.name == PROVIDER_PREPARATION_FUNCTION)
        .expect("bindings have a preparation function");
    prune_block(&mut preparation.body, &remove);
    semantics.remove_generated_struct_literal_fields(&removed_fields);
    Some(pruned)
}

fn prune_block(block: &mut Block, remove: &HashSet<String>) {
    block.statements.retain_mut(|statement| {
        match statement {
            Stmt::Variable(variable) => return !remove.contains(&variable.name),
            Stmt::Suspend {
                binding: Some(binding),
                ..
            } => return !remove.contains(&binding.name),
            Stmt::Expression(expression) => prune_expression(expression, remove),
            _ => {}
        }
        true
    });
}

fn prune_expression(expression: &mut Expr, remove: &HashSet<String>) {
    match &mut expression.kind {
        ExprKind::Return(Some(value)) => prune_expression(value, remove),
        ExprKind::Block(block) => prune_block(block, remove),
        ExprKind::Struct { name, fields, .. } if name == PROVIDER_BINDINGS_TYPE => {
            fields.retain(|field| !remove.contains(&field.name));
        }
        _ => {}
    }
}
