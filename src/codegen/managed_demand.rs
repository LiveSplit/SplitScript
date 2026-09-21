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
    let snapshots = reachable.managed_snapshots().collect::<HashSet<_>>();
    for class in snapshots.iter().copied() {
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
    let reference_classes = reachable
        .managed_references()
        .filter_map(|ty| super::managed_references::class(ty, semantics))
        .collect::<HashSet<_>>();
    if snapshots.is_empty() && reference_classes.is_empty() {
        remove.insert(crate::stdlib::MANAGED_OBJECT_TYPE_FIELD.to_owned());
        remove.insert("__object_type_cache".to_owned());
    }
    use crate::managed_read::{ManagedDecoderKind as D, typed_collection_binding};
    let mut collection_bindings = HashSet::new();
    for ty in reachable.managed_decoders() {
        match capabilities.managed_decoder(ty).unwrap().kind {
            D::Array { element } | D::List { element } => {
                collection_bindings.insert(typed_collection_binding(
                    crate::stdlib::MANAGED_ARRAY_TYPE_FIELD,
                    &[element],
                    capabilities,
                ));
                if matches!(
                    capabilities.managed_decoder(ty).unwrap().kind,
                    D::List { .. }
                ) {
                    collection_bindings.insert(typed_collection_binding(
                        crate::stdlib::MANAGED_LIST_LAYOUT_FIELD,
                        &[element],
                        capabilities,
                    ));
                }
            }
            D::Map { key, value } => {
                collection_bindings.insert(typed_collection_binding(
                    crate::stdlib::MANAGED_MAP_READ_FIELD,
                    &[key, value],
                    capabilities,
                ));
            }
            D::Set { element } => {
                collection_bindings.insert(typed_collection_binding(
                    crate::stdlib::MANAGED_SET_READ_FIELD,
                    &[element],
                    capabilities,
                ));
            }
            _ => {}
        }
    }
    for (base, cache) in [
        (
            crate::stdlib::MANAGED_ARRAY_TYPE_FIELD,
            "__array_layout_cache",
        ),
        (
            crate::stdlib::MANAGED_LIST_LAYOUT_FIELD,
            "__list_layout_cache",
        ),
        (crate::stdlib::MANAGED_MAP_READ_FIELD, "__map_layout_cache"),
        (crate::stdlib::MANAGED_SET_READ_FIELD, "__set_layout_cache"),
    ] {
        let mut needed = false;
        for suffix in ["", "_class", "_schema"] {
            let binding = format!("{base}{suffix}");
            if collection_bindings.contains(&binding) {
                needed = true
            } else {
                remove.insert(binding);
            }
        }
        if !needed {
            remove.insert(cache.to_owned());
        }
    }
    let contracts =
        crate::managed_read::schema_contracts(reachable.managed_decoders(), capabilities);
    let schema_bindings = contracts
        .iter()
        .map(|ty| {
            crate::managed_read::schema_binding(capabilities.managed_decoder(*ty).unwrap().kind)
        })
        .collect::<HashSet<_>>();
    for name in [
        "__schema_storage",
        "__schema_class",
        "__schema_array",
        "__schema_list",
        "__schema_map",
        "__schema_set",
    ] {
        if !schema_bindings.contains(name) {
            remove.insert(name.to_owned());
        }
    }
    if contracts.is_empty() {
        remove.insert("__schema_proof".to_owned());
        remove.insert("__schema_proof_cache".to_owned());
    }
    if !collection_bindings
        .iter()
        .any(|binding| binding.ends_with("_class"))
        && !schema_bindings.contains("__schema_class")
    {
        remove.insert("__class_contract".to_owned());
        remove.insert("__class_contract_cache".to_owned());
    }
    if remove.contains("__map_layout_cache") && remove.contains("__set_layout_cache") {
        remove.insert(crate::stdlib::MANAGED_KEYED_VERIFY_FIELD.to_owned());
        remove.insert("__keyed_array".to_owned());
        remove.insert("__keyed_array_cache".to_owned());
    }
    let mut images = HashMap::new();
    let mut needed_images = HashSet::new();
    let mut batches = HashMap::new();
    let mut batch_inputs = HashMap::new();
    for class in &managed.classes {
        if class.fields.len() >= crate::stdlib::MANAGED_BATCH_MIN_FIELDS {
            let selected = class
                .fields
                .iter()
                .map(|field| fields.contains(&field.id))
                .collect::<Vec<_>>();
            let grouped = selected.iter().filter(|selected| **selected).count()
                >= crate::stdlib::MANAGED_BATCH_MIN_FIELDS;
            let name = format!("__class_{}_fields", class.id.index());
            if grouped {
                batch_inputs.insert(name, selected.clone());
            } else {
                remove.insert(name);
            }
            let mut slot = 0;
            for (field, selected) in class.fields.iter().zip(selected) {
                let index = (grouped && selected).then_some(slot);
                batches.insert(managed_field_offset_name(field.id.index()), index);
                batches.insert(managed_static_field_address_name(field.id.index()), index);
                slot += usize::from(selected);
            }
        }
        let next = images.len();
        let image = *images.entry(&class.image_name).or_insert(next);
        let needed = reference_classes.contains(&class.id)
            || snapshots.contains(&class.id)
            || headers.contains(&class.id)
            || class.all_fields().any(|f| fields.contains(&f.id));
        if needed {
            needed_images.insert(image);
        } else {
            remove.insert(format!("__class_{}", class.id.index()));
        }
        if !snapshots.contains(&class.id) && !reference_classes.contains(&class.id) {
            remove.insert(crate::stdlib::managed_class_address_name(class.id.index()));
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
    if batches.is_empty()
        && removed_fields.is_empty()
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
    prune_block(&mut preparation.body, &remove, &batches, &batch_inputs);
    semantics.remove_generated_struct_literal_fields(&removed_fields);
    Some(pruned)
}

fn prune_block(
    block: &mut Block,
    remove: &HashSet<String>,
    batches: &HashMap<String, Option<usize>>,
    batch_inputs: &HashMap<String, Vec<bool>>,
) {
    block.statements.retain_mut(|statement| {
        match statement {
            Stmt::Variable(variable) => {
                if remove.contains(&variable.name) {
                    return false;
                }
                if let Some(selected) = batch_inputs.get(&variable.name) {
                    let ExprKind::Suspend { value, .. } =
                        &mut variable.value.as_mut().unwrap().kind
                    else {
                        unreachable!("generated group awaits its reader")
                    };
                    let ExprKind::Call { args, .. } = &mut value.kind else {
                        unreachable!("generated group calls its reader")
                    };
                    for argument in args {
                        let ExprKind::Array(elements) = &mut argument.kind else {
                            unreachable!("generated group arguments are arrays")
                        };
                        assert_eq!(elements.len(), selected.len());
                        let mut selection = selected.iter();
                        elements.retain(|_| *selection.next().unwrap());
                    }
                }
                if let Some(batch) = batches.get(&variable.name) {
                    let value = variable
                        .value
                        .as_mut()
                        .expect("generated binding has a value");
                    let ExprKind::If {
                        then_expr,
                        else_expr,
                        ..
                    } = &value.kind
                    else {
                        unreachable!("groupable field bindings have checked alternatives")
                    };
                    *value = if batch.is_some() {
                        *then_expr.clone()
                    } else {
                        *else_expr.clone()
                    };
                    if let Some(index) = batch {
                        reindex_binding(value, *index);
                    }
                }
            }
            Stmt::Suspend {
                binding: Some(binding),
                ..
            } => return !remove.contains(&binding.name),
            Stmt::Expression(expression) => {
                prune_expression(expression, remove, batches, batch_inputs)
            }
            _ => {}
        }
        true
    });
}

fn reindex_binding(expression: &mut Expr, slot: usize) {
    match &mut expression.kind {
        ExprKind::Block(block) => {
            let Some(Stmt::Expression(value)) = block.statements.last_mut() else {
                unreachable!("generated field branch ends with its value")
            };
            reindex_binding(value, slot);
        }
        ExprKind::Cast { expr, .. } => reindex_binding(expr, slot),
        ExprKind::Index { index, .. } => {
            let ExprKind::Int { value, .. } = &mut index.kind else {
                unreachable!("generated field uses a literal slot")
            };
            *value = slot as u64;
        }
        _ => unreachable!("generated field reads a group slot"),
    }
}

fn prune_expression(
    expression: &mut Expr,
    remove: &HashSet<String>,
    batches: &HashMap<String, Option<usize>>,
    batch_inputs: &HashMap<String, Vec<bool>>,
) {
    match &mut expression.kind {
        ExprKind::Return(Some(value)) => prune_expression(value, remove, batches, batch_inputs),
        ExprKind::Block(block) => prune_block(block, remove, batches, batch_inputs),
        ExprKind::Struct { name, fields, .. } if name == PROVIDER_BINDINGS_TYPE => {
            fields.retain(|field| !remove.contains(&field.name));
        }
        _ => {}
    }
}
