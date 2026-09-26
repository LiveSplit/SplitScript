//! Lazy structural and opaque `Debug` body generation used by `Display` fallback.

use crate::codegen::GC_NULL_HEAP_TYPE;
use std::collections::HashMap;

use wasm_encoder::{BlockType, Function, Instruction, ValType};

use crate::{
    ast::{Program, RangeKind, ValueId},
    capabilities::DerivedDebugKind,
    intrinsic_registry::RuntimeHelperId,
    semantic::{FunctionInstance, SemanticModel},
    stdlib::{StdlibTypeConstructorId, StdlibTypeId},
    structural::{StructuralMemberId, StructuralType, StructuralTypeId, StructuralTypes},
    types::{ResolvedArrayType, TypeId, TypeKind},
};

use super::{
    DisplayFunctions, GcLayout, RuntimeHelperPlan, STATE_TYPE, Type, array_value, emit_array_get,
    emit_string_literal, emit_typed_struct_get, enum_variant_payload,
    function_plan::UserFunctionPlan, managed_snapshot_field_type, semantic_type, struct_field_type,
    try_array_element_type,
};

pub(super) struct DisplayInputs<'a> {
    pub program: &'a Program,
    pub structural: &'a StructuralTypes,
    pub arrays: &'a [ResolvedArrayType],
    pub semantics: &'a SemanticModel,
    pub displays: &'a DisplayFunctions,
    pub users: &'a HashMap<FunctionInstance, UserFunctionPlan>,
    pub helpers: &'a RuntimeHelperPlan,
    pub debug_depth: u32,
    pub globals: &'a HashMap<ValueId, u32>,
    pub selected_provider: Option<u32>,
    pub gc: &'a GcLayout,
}

pub(super) fn compile(inputs: &DisplayInputs<'_>) -> Vec<Function> {
    inputs
        .displays
        .derived
        .iter()
        .map(|(&ty, derived)| {
            if matches!(inputs.semantics.types().kind(ty), TypeKind::StateSnapshot) {
                return compile_state_snapshot(inputs);
            }
            if derived.kind == DerivedDebugKind::Opaque {
                return compile_opaque(ty, inputs);
            }
            if let Some(structural) = inputs.structural.get(ty) {
                return match structural.id {
                    StructuralTypeId::Struct(_) => compile_struct(structural, inputs),
                    StructuralTypeId::Enum(_) => compile_enum(structural, inputs),
                    StructuralTypeId::ManagedClass(_) => compile_managed_class(structural, inputs),
                };
            }
            match inputs.semantics.types().kind(ty) {
                TypeKind::Array {
                    layout, element, ..
                } => compile_array(*layout, *element, inputs),
                TypeKind::Set {
                    layout,
                    element,
                    backing,
                } => compile_set(*layout, *element, *backing, inputs),
                TypeKind::Option { layout, value } => compile_option(*layout, *value, inputs),
                TypeKind::Result { layout, value } => compile_result(*layout, *value, inputs),
                TypeKind::Range {
                    layout,
                    bound,
                    kind,
                } => compile_range(*layout, *bound, *kind, inputs),
                TypeKind::Application {
                    layout,
                    constructor: StdlibTypeConstructorId::IteratorStep,
                    arguments,
                } => compile_iterator_step(*layout, arguments[0], inputs),
                TypeKind::Application {
                    layout,
                    constructor: StdlibTypeConstructorId::Map,
                    arguments,
                } => compile_map(*layout, arguments, inputs),
                TypeKind::Application {
                    layout,
                    constructor,
                    arguments,
                } if inputs.gc.standard_library.type_constructor_has_capability(
                    *constructor,
                    crate::stdlib::StdlibCapabilityId::Debug,
                ) =>
                {
                    compile_catalog_struct(*layout, *constructor, arguments, inputs)
                }
                kind => unreachable!("derived Debug implementation for {kind:?}"),
            }
        })
        .collect()
}

fn compile_state_snapshot(inputs: &DisplayInputs<'_>) -> Function {
    let mut function = Function::new([]);
    begin_recursion_guard(&mut function, inputs);
    emit_string_literal(&mut function, "StateSnapshot {\n", inputs.gc);

    let state = inputs
        .program
        .state
        .as_ref()
        .expect("checked programs have a state declaration");
    let fields = state.all_fields().collect::<Vec<_>>();
    for field in &fields {
        let conditional =
            if let Some(predicate) = inputs.semantics.state_field_shape_predicate(field.id) {
                super::update::emit_shape_predicate(
                    &mut function,
                    inputs.program,
                    predicate,
                    inputs.semantics,
                    inputs.gc,
                    inputs.globals,
                    super::update::PredicateState::Local(0),
                );
                true
            } else if let Some((provider_index, _)) = state
                .provider_variant_fields()
                .enumerate()
                .find(|(_, (_, fields))| fields.iter().any(|candidate| candidate.id == field.id))
            {
                let selected = inputs
                    .selected_provider
                    .expect("provider alternatives have a selected-provider global");
                let enumeration = state
                    .provider_enum
                    .as_ref()
                    .expect("provider alternatives generate a typed enum");
                function
                    .instruction(&Instruction::GlobalGet(selected))
                    .instruction(&Instruction::StructGet {
                        struct_type_index: inputs.gc.index(Type::Enum(enumeration.id)),
                        field_index: 0,
                    })
                    .instruction(&Instruction::I32Const(provider_index as i32))
                    .instruction(&Instruction::I32Eq);
                true
            } else {
                false
            };

        if conditional {
            function.instruction(&Instruction::If(BlockType::Result(
                inputs.gc.val_type(Type::Standard(StdlibTypeId::String)),
            )));
        }
        emit_state_field_segment(&mut function, field, inputs);
        if conditional {
            function.instruction(&Instruction::Else);
            emit_string_literal(&mut function, "", inputs.gc);
            function.instruction(&Instruction::End);
        }
    }

    emit_string_literal(&mut function, "}", inputs.gc);
    join_pieces(&mut function, 2 + fields.len() as u32, inputs);
    finish_recursion_guard(&mut function, inputs);
    function
}

fn emit_state_field_segment(
    function: &mut Function,
    field: &crate::ast::StateField,
    inputs: &DisplayInputs<'_>,
) {
    emit_string_literal(function, &format!("    {}: ", field.name), inputs.gc);
    let storage = inputs
        .semantics
        .state_storage_field(field.id)
        .expect("checked state fields have physical snapshot storage");
    let field_index = inputs
        .semantics
        .state_storage_fields()
        .iter()
        .position(|candidate| *candidate == storage)
        .expect("state field storage belongs to the snapshot") as u32;
    let field_type_id = inputs
        .semantics
        .value_type(storage)
        .expect("checked state fields have semantic types");
    let field_type = semantic_type(field_type_id, inputs.semantics);
    function
        .instruction(&Instruction::LocalGet(0))
        .instruction(&Instruction::RefAsNonNull);
    emit_typed_struct_get(function, STATE_TYPE, field_index, field_type);
    emit_value(function, field_type_id, field_type, inputs);
    function.instruction(&Instruction::Call(
        inputs.helpers.function(RuntimeHelperId::IndentDisplay),
    ));
    emit_string_literal(function, ",\n", inputs.gc);
    join_pieces(function, 3, inputs);
}

fn compile_opaque(ty: TypeId, inputs: &DisplayInputs<'_>) -> Function {
    let mut function = Function::new([]);
    let backend = semantic_type(ty, inputs.semantics);
    debug_assert!(backend.has_runtime_value());
    function
        .instruction(&Instruction::LocalGet(0))
        .instruction(&Instruction::Drop);
    let text = match inputs.semantics.types().kind(ty) {
        TypeKind::Standard(standard) => format!(
            "{} {{ .. }}",
            inputs.gc.standard_library.type_decl(*standard).name
        ),
        TypeKind::StateSnapshot => "StateSnapshot { .. }".to_owned(),
        TypeKind::SettingsView => "SettingsView { .. }".to_owned(),
        TypeKind::ManagedReference(_) => "<managed class reference>".to_owned(),
        TypeKind::Array { .. } => "array { .. }".to_owned(),
        TypeKind::Option { .. } => "optional value { .. }".to_owned(),
        TypeKind::Result { .. } => "fallible value { .. }".to_owned(),
        TypeKind::Async { .. } => "<future>".to_owned(),
        TypeKind::Iterator { .. } => "<iterator>".to_owned(),
        TypeKind::Callable { .. } => "<closure>".to_owned(),
        TypeKind::Range { .. } => "range { .. }".to_owned(),
        TypeKind::Set { .. } => "Set { .. }".to_owned(),
        TypeKind::Application { constructor, .. } => {
            format!(
                "{} {{ .. }}",
                inputs
                    .gc
                    .standard_library
                    .type_constructor(*constructor)
                    .name
            )
        }
        TypeKind::Error
        | TypeKind::Builtin(_)
        | TypeKind::GenericParameter { .. }
        | TypeKind::Struct(_)
        | TypeKind::Enum(_)
        | TypeKind::ManagedClass(_) => unreachable!("opaque Debug received a non-opaque type"),
    };
    emit_string_literal(&mut function, &text, inputs.gc);
    function.instruction(&Instruction::End);
    function
}

fn catalog_type_variables(
    constructor: StdlibTypeConstructorId,
    arguments: &[TypeId],
    inputs: &DisplayInputs<'_>,
) -> HashMap<&'static str, TypeId> {
    inputs
        .gc
        .standard_library
        .type_constructor(constructor)
        .parameters
        .iter()
        .zip(arguments)
        .map(|(parameter, argument)| (parameter.name, *argument))
        .collect()
}

fn compile_catalog_struct(
    application: crate::ast::TypeApplicationId,
    constructor: StdlibTypeConstructorId,
    arguments: &[TypeId],
    inputs: &DisplayInputs<'_>,
) -> Function {
    let mut function = Function::new([]);
    begin_recursion_guard(&mut function, inputs);
    let declaration = inputs.gc.standard_library.type_constructor(constructor);
    let variables = catalog_type_variables(constructor, arguments, inputs);
    let fields = inputs
        .gc
        .standard_library
        .fields_of_constructor(constructor)
        .enumerate()
        .filter(|(_, field)| field.visibility == crate::stdlib::FieldVisibility::Public)
        .collect::<Vec<_>>();
    let has_hidden_fields = inputs
        .gc
        .standard_library
        .fields_of_constructor(constructor)
        .count()
        != fields.len();
    if fields.is_empty() && has_hidden_fields {
        emit_string_literal(
            &mut function,
            &format!("{} {{ .. }}", declaration.name),
            inputs.gc,
        );
        finish_recursion_guard(&mut function, inputs);
        return function;
    }
    emit_string_literal(
        &mut function,
        &format!("{} {{\n", declaration.name),
        inputs.gc,
    );
    for (field_index, field) in &fields {
        emit_string_literal(&mut function, &format!("    {}: ", field.name), inputs.gc);
        let field_type_id = inputs
            .semantics
            .instantiated_catalog_type(field.ty, &variables)
            .expect("concrete catalog struct fields have semantic layouts");
        let field_type = semantic_type(field_type_id, inputs.semantics);
        function
            .instruction(&Instruction::LocalGet(0))
            .instruction(&Instruction::RefAsNonNull);
        emit_typed_struct_get(
            &mut function,
            inputs.gc.index(Type::Application(application)),
            *field_index as u32,
            field_type,
        );
        emit_value(&mut function, field_type_id, field_type, inputs);
        function.instruction(&Instruction::Call(
            inputs.helpers.function(RuntimeHelperId::IndentDisplay),
        ));
        emit_string_literal(&mut function, ",\n", inputs.gc);
    }
    if has_hidden_fields {
        emit_string_literal(&mut function, "    ..\n", inputs.gc);
    }
    emit_string_literal(&mut function, "}", inputs.gc);
    join_pieces(
        &mut function,
        2 + fields.len() as u32 * 3 + u32::from(has_hidden_fields),
        inputs,
    );
    finish_recursion_guard(&mut function, inputs);
    function
}

fn compile_struct(structure: &StructuralType, inputs: &DisplayInputs<'_>) -> Function {
    let mut function = Function::new([]);
    begin_recursion_guard(&mut function, inputs);
    let StructuralTypeId::Struct(struct_id) = structure.id else {
        unreachable!()
    };
    let type_index = inputs.gc.index(Type::Struct(struct_id));
    emit_string_literal(
        &mut function,
        &format!("{} {{\n", structure.name),
        inputs.gc,
    );
    for (field_index, field) in structure.members.iter().enumerate() {
        emit_string_literal(&mut function, &format!("    {}: ", field.name), inputs.gc);
        let StructuralMemberId::StructField(field_id) = field.source else {
            unreachable!()
        };
        let field_type_id = field.ty.expect("struct fields have semantic types");
        let field_type = struct_field_type(field_id, inputs.semantics);
        function
            .instruction(&Instruction::LocalGet(0))
            .instruction(&Instruction::RefAsNonNull);
        emit_typed_struct_get(&mut function, type_index, field_index as u32, field_type);
        emit_value(&mut function, field_type_id, field_type, inputs);
        function.instruction(&Instruction::Call(
            inputs.helpers.function(RuntimeHelperId::IndentDisplay),
        ));
        emit_string_literal(&mut function, ",\n", inputs.gc);
    }
    emit_string_literal(&mut function, "}", inputs.gc);
    join_pieces(
        &mut function,
        2 + structure.members.len() as u32 * 3,
        inputs,
    );
    finish_recursion_guard(&mut function, inputs);
    function
}

fn compile_managed_class(structure: &StructuralType, inputs: &DisplayInputs<'_>) -> Function {
    let mut function = Function::new([]);
    begin_recursion_guard(&mut function, inputs);
    let StructuralTypeId::ManagedClass(class) = structure.id else {
        unreachable!()
    };
    let type_index = inputs.gc.index(Type::ManagedClass(class));
    emit_string_literal(
        &mut function,
        &format!("{} {{\n", structure.name),
        inputs.gc,
    );
    for (field_index, field) in structure.members.iter().enumerate() {
        let StructuralMemberId::ManagedField(field_id) = field.source else {
            unreachable!()
        };
        if let Some(predicate) = inputs.semantics.managed_field_shape_predicate(field_id) {
            super::update::emit_shape_predicate(
                &mut function,
                inputs.program,
                predicate,
                inputs.semantics,
                inputs.gc,
                inputs.globals,
                super::update::PredicateState::Unavailable,
            );
            function.instruction(&Instruction::If(BlockType::Result(
                inputs.gc.val_type(Type::Standard(StdlibTypeId::String)),
            )));
            emit_managed_field_segment(
                &mut function,
                type_index,
                field_index as u32,
                field,
                field_id,
                inputs,
            );
            function.instruction(&Instruction::Else);
            emit_string_literal(&mut function, "", inputs.gc);
            function.instruction(&Instruction::End);
        } else {
            emit_managed_field_segment(
                &mut function,
                type_index,
                field_index as u32,
                field,
                field_id,
                inputs,
            );
        }
    }
    emit_string_literal(&mut function, "}", inputs.gc);
    join_pieces(&mut function, 2 + structure.members.len() as u32, inputs);
    finish_recursion_guard(&mut function, inputs);
    function
}

fn emit_managed_field_segment(
    function: &mut Function,
    type_index: u32,
    field_index: u32,
    field: &crate::structural::StructuralMember,
    field_id: crate::ast::ManagedFieldId,
    inputs: &DisplayInputs<'_>,
) {
    emit_string_literal(function, &format!("    {}: ", field.name), inputs.gc);
    let field_type_id = field
        .ty
        .expect("managed snapshot fields have semantic types");
    let field_type = managed_snapshot_field_type(field_id, inputs.semantics);
    function
        .instruction(&Instruction::LocalGet(0))
        .instruction(&Instruction::RefAsNonNull);
    emit_typed_struct_get(function, type_index, field_index, field_type);
    emit_value(function, field_type_id, field_type, inputs);
    function.instruction(&Instruction::Call(
        inputs.helpers.function(RuntimeHelperId::IndentDisplay),
    ));
    emit_string_literal(function, ",\n", inputs.gc);
    join_pieces(function, 3, inputs);
}

fn compile_enum(enumeration: &StructuralType, inputs: &DisplayInputs<'_>) -> Function {
    let mut function = Function::new([(1, ValType::I32)]);
    begin_recursion_guard(&mut function, inputs);
    let tag = 1;
    let StructuralTypeId::Enum(enum_id) = enumeration.id else {
        unreachable!()
    };
    let type_index = inputs.gc.index(Type::Enum(enum_id));
    function
        .instruction(&Instruction::LocalGet(0))
        .instruction(&Instruction::RefAsNonNull);
    emit_typed_struct_get(&mut function, type_index, 0, Type::I32);
    function.instruction(&Instruction::LocalSet(tag));

    for (variant_index, variant) in enumeration.members.iter().enumerate() {
        function
            .instruction(&Instruction::LocalGet(tag))
            .instruction(&Instruction::I32Const(variant_index as i32))
            .instruction(&Instruction::I32Eq)
            .instruction(&Instruction::If(BlockType::Empty));
        if let Some(payload_type) = variant
            .ty
            .filter(|ty| semantic_type(*ty, inputs.semantics).has_runtime_value())
        {
            emit_string_literal(
                &mut function,
                &format!("{}.{}(\n    ", enumeration.name, variant.name),
                inputs.gc,
            );
            function
                .instruction(&Instruction::LocalGet(0))
                .instruction(&Instruction::RefAsNonNull);
            let StructuralMemberId::EnumVariant(variant_id) = variant.source else {
                unreachable!()
            };
            let payload = enum_variant_payload(variant_id, inputs.semantics)
                .expect("payload variants have backend types");
            emit_typed_struct_get(&mut function, type_index, variant_index as u32 + 1, payload);
            emit_value(&mut function, payload_type, payload, inputs);
            function.instruction(&Instruction::Call(
                inputs.helpers.function(RuntimeHelperId::IndentDisplay),
            ));
            emit_string_literal(&mut function, ",\n)", inputs.gc);
            join_pieces(&mut function, 3, inputs);
        } else {
            emit_string_literal(
                &mut function,
                &format!("{}.{}", enumeration.name, variant.name),
                inputs.gc,
            );
        }
        decrement_debug_depth(&mut function, inputs);
        function
            .instruction(&Instruction::Return)
            .instruction(&Instruction::End);
    }
    emit_string_literal(&mut function, &enumeration.name, inputs.gc);
    finish_recursion_guard(&mut function, inputs);
    function
}

fn compile_array(
    array: crate::ast::ArrayTypeId,
    element: TypeId,
    inputs: &DisplayInputs<'_>,
) -> Function {
    let storage = array_value::storage_id(array, inputs.arrays, inputs.semantics);
    compile_sequence(
        "[\n",
        "]",
        element,
        Type::ArrayStorage(storage),
        |function| {
            function.instruction(&Instruction::LocalGet(0));
            array_value::emit_length(function, inputs.gc, array);
        },
        |function| {
            function.instruction(&Instruction::LocalGet(0));
            array_value::emit_backing(function, inputs.gc, array);
        },
        inputs,
    )
}

fn compile_set(
    set: crate::ast::TypeApplicationId,
    element: TypeId,
    backing: crate::ast::ArrayTypeId,
    inputs: &DisplayInputs<'_>,
) -> Function {
    compile_sequence(
        "Set {\n",
        "}",
        element,
        Type::ArrayStorage(backing),
        |function| {
            function
                .instruction(&Instruction::LocalGet(0))
                .instruction(&Instruction::StructGet {
                    struct_type_index: inputs.gc.index(Type::Set(set)),
                    field_index: super::set_functions::LENGTH_FIELD,
                });
        },
        |function| {
            function
                .instruction(&Instruction::LocalGet(0))
                .instruction(&Instruction::StructGet {
                    struct_type_index: inputs.gc.index(Type::Set(set)),
                    field_index: super::set_functions::BACKING_FIELD,
                })
                .instruction(&Instruction::RefAsNonNull);
        },
        inputs,
    )
}

fn compile_map(
    map: crate::ast::TypeApplicationId,
    arguments: &[TypeId],
    inputs: &DisplayInputs<'_>,
) -> Function {
    let variables = catalog_type_variables(StdlibTypeConstructorId::Map, arguments, inputs);
    let entries_field = inputs
        .gc
        .standard_library
        .fields_of_constructor(StdlibTypeConstructorId::Map)
        .next()
        .expect("Map has its private entries field");
    let entries_type = inputs
        .semantics
        .instantiated_catalog_type(entries_field.ty, &variables)
        .expect("Map.entries has a concrete semantic layout");
    let TypeKind::Array {
        layout: entries_array,
        element: entry_type,
        ..
    } = inputs.semantics.types().kind(entries_type)
    else {
        unreachable!("Map.entries is an array")
    };
    let TypeKind::Application {
        layout: entry_layout,
        constructor: StdlibTypeConstructorId::MapEntry,
        arguments: entry_arguments,
    } = inputs.semantics.types().kind(*entry_type)
    else {
        unreachable!("Map.entries contains MapEntry values")
    };
    let key = entry_arguments[0];
    let value = entry_arguments[1];
    let key_backend = semantic_type(key, inputs.semantics);
    let value_backend = semantic_type(value, inputs.semantics);
    let (strings, string_storage) = string_array(inputs);
    let mut function = Function::new([
        (2, ValType::I32),
        (1, inputs.gc.val_type(Type::ArrayStorage(string_storage))),
        (1, inputs.gc.val_type(Type::Application(*entry_layout))),
    ]);
    let index = 1;
    let length = 2;
    let pieces = 3;
    let entry = 4;
    let map_index = inputs.gc.index(Type::Application(map));
    let entry_index = inputs.gc.index(Type::Application(*entry_layout));

    begin_recursion_guard(&mut function, inputs);
    function
        .instruction(&Instruction::LocalGet(0))
        .instruction(&Instruction::RefAsNonNull);
    emit_typed_struct_get(&mut function, map_index, 0, Type::Array(*entries_array));
    array_value::emit_length(&mut function, inputs.gc, *entries_array);
    function
        .instruction(&Instruction::LocalSet(length))
        .instruction(&Instruction::LocalGet(length))
        .instruction(&Instruction::I32Const(2))
        .instruction(&Instruction::I32Add)
        .instruction(&Instruction::ArrayNewDefault(
            inputs.gc.index(Type::ArrayStorage(string_storage)),
        ))
        .instruction(&Instruction::LocalSet(pieces));
    set_piece_literal(&mut function, pieces, 0, "Map {\n", inputs);
    function
        .instruction(&Instruction::Block(BlockType::Empty))
        .instruction(&Instruction::Loop(BlockType::Empty))
        .instruction(&Instruction::LocalGet(index))
        .instruction(&Instruction::LocalGet(length))
        .instruction(&Instruction::I32GeU)
        .instruction(&Instruction::BrIf(1))
        .instruction(&Instruction::LocalGet(0))
        .instruction(&Instruction::RefAsNonNull);
    emit_typed_struct_get(&mut function, map_index, 0, Type::Array(*entries_array));
    array_value::emit_backing(&mut function, inputs.gc, *entries_array);
    function.instruction(&Instruction::LocalGet(index));
    emit_array_get(
        &mut function,
        inputs.gc.index(Type::ArrayStorage(*entries_array)),
        Type::Application(*entry_layout),
        inputs.gc,
    );
    function.instruction(&Instruction::LocalSet(entry));

    function
        .instruction(&Instruction::LocalGet(pieces))
        .instruction(&Instruction::RefAsNonNull)
        .instruction(&Instruction::LocalGet(index))
        .instruction(&Instruction::I32Const(1))
        .instruction(&Instruction::I32Add)
        .instruction(&Instruction::LocalGet(entry))
        .instruction(&Instruction::RefAsNonNull);
    emit_typed_struct_get(&mut function, entry_index, 0, key_backend);
    emit_value(&mut function, key, key_backend, inputs);
    emit_string_literal(&mut function, ": ", inputs.gc);
    function
        .instruction(&Instruction::LocalGet(entry))
        .instruction(&Instruction::RefAsNonNull);
    emit_typed_struct_get(&mut function, entry_index, 1, value_backend);
    emit_value(&mut function, value, value_backend, inputs);
    join_pieces(&mut function, 3, inputs);
    function
        .instruction(&Instruction::Call(
            inputs.helpers.function(RuntimeHelperId::WrapDebugEntry),
        ))
        .instruction(&Instruction::ArraySet(
            inputs.gc.index(Type::ArrayStorage(string_storage)),
        ))
        .instruction(&Instruction::LocalGet(index))
        .instruction(&Instruction::I32Const(1))
        .instruction(&Instruction::I32Add)
        .instruction(&Instruction::LocalSet(index))
        .instruction(&Instruction::Br(0))
        .instruction(&Instruction::End)
        .instruction(&Instruction::End)
        .instruction(&Instruction::LocalGet(pieces))
        .instruction(&Instruction::RefAsNonNull)
        .instruction(&Instruction::LocalGet(length))
        .instruction(&Instruction::I32Const(1))
        .instruction(&Instruction::I32Add);
    emit_string_literal(&mut function, "}", inputs.gc);
    function
        .instruction(&Instruction::ArraySet(
            inputs.gc.index(Type::ArrayStorage(string_storage)),
        ))
        .instruction(&Instruction::LocalGet(pieces))
        .instruction(&Instruction::LocalGet(length))
        .instruction(&Instruction::I32Const(2))
        .instruction(&Instruction::I32Add);
    array_value::emit_wrap_loaded(&mut function, inputs.gc.index(Type::Array(strings)));
    function
        .instruction(&Instruction::RefNull(GC_NULL_HEAP_TYPE))
        .instruction(&Instruction::Call(
            inputs.helpers.function(RuntimeHelperId::JoinStrings),
        ));
    finish_recursion_guard(&mut function, inputs);
    function
}

fn compile_sequence(
    opening: &str,
    closing: &str,
    element: TypeId,
    source_storage: Type,
    emit_length: impl Fn(&mut Function),
    emit_backing: impl Fn(&mut Function),
    inputs: &DisplayInputs<'_>,
) -> Function {
    let (strings, string_storage) = string_array(inputs);
    let mut function = Function::new([
        (2, ValType::I32),
        (1, inputs.gc.val_type(Type::ArrayStorage(string_storage))),
    ]);
    let index = 1;
    let length = 2;
    let pieces = 3;
    begin_recursion_guard(&mut function, inputs);
    emit_length(&mut function);
    function
        .instruction(&Instruction::LocalSet(length))
        .instruction(&Instruction::LocalGet(length))
        .instruction(&Instruction::I32Const(2))
        .instruction(&Instruction::I32Add)
        .instruction(&Instruction::ArrayNewDefault(
            inputs.gc.index(Type::ArrayStorage(string_storage)),
        ))
        .instruction(&Instruction::LocalSet(pieces));
    set_piece_literal(&mut function, pieces, 0, opening, inputs);
    function
        .instruction(&Instruction::Block(BlockType::Empty))
        .instruction(&Instruction::Loop(BlockType::Empty))
        .instruction(&Instruction::LocalGet(index))
        .instruction(&Instruction::LocalGet(length))
        .instruction(&Instruction::I32GeU)
        .instruction(&Instruction::BrIf(1))
        .instruction(&Instruction::LocalGet(pieces))
        .instruction(&Instruction::RefAsNonNull)
        .instruction(&Instruction::LocalGet(index))
        .instruction(&Instruction::I32Const(1))
        .instruction(&Instruction::I32Add);
    emit_backing(&mut function);
    function.instruction(&Instruction::LocalGet(index));
    let backend = super::semantic_type(element, inputs.semantics);
    emit_array_get(
        &mut function,
        inputs.gc.index(source_storage),
        backend,
        inputs.gc,
    );
    emit_value(&mut function, element, backend, inputs);
    function
        .instruction(&Instruction::Call(
            inputs.helpers.function(RuntimeHelperId::WrapDebugEntry),
        ))
        .instruction(&Instruction::ArraySet(
            inputs.gc.index(Type::ArrayStorage(string_storage)),
        ))
        .instruction(&Instruction::LocalGet(index))
        .instruction(&Instruction::I32Const(1))
        .instruction(&Instruction::I32Add)
        .instruction(&Instruction::LocalSet(index))
        .instruction(&Instruction::Br(0))
        .instruction(&Instruction::End)
        .instruction(&Instruction::End)
        .instruction(&Instruction::LocalGet(pieces))
        .instruction(&Instruction::RefAsNonNull)
        .instruction(&Instruction::LocalGet(length))
        .instruction(&Instruction::I32Const(1))
        .instruction(&Instruction::I32Add);
    emit_string_literal(&mut function, closing, inputs.gc);
    function
        .instruction(&Instruction::ArraySet(
            inputs.gc.index(Type::ArrayStorage(string_storage)),
        ))
        .instruction(&Instruction::LocalGet(pieces))
        .instruction(&Instruction::LocalGet(length))
        .instruction(&Instruction::I32Const(2))
        .instruction(&Instruction::I32Add);
    array_value::emit_wrap_loaded(&mut function, inputs.gc.index(Type::Array(strings)));
    function
        .instruction(&Instruction::RefNull(GC_NULL_HEAP_TYPE))
        .instruction(&Instruction::Call(
            inputs.helpers.function(RuntimeHelperId::JoinStrings),
        ));
    finish_recursion_guard(&mut function, inputs);
    function
}

fn compile_option(
    option: crate::ast::OptionTypeId,
    value: TypeId,
    inputs: &DisplayInputs<'_>,
) -> Function {
    let mut function = Function::new([]);
    begin_recursion_guard(&mut function, inputs);
    function
        .instruction(&Instruction::LocalGet(0))
        .instruction(&Instruction::RefIsNull)
        .instruction(&Instruction::If(BlockType::Result(
            inputs.gc.val_type(Type::Standard(StdlibTypeId::String)),
        )));
    emit_string_literal(&mut function, "None", inputs.gc);
    function
        .instruction(&Instruction::Else)
        .instruction(&Instruction::LocalGet(0))
        .instruction(&Instruction::RefAsNonNull);
    let backend = super::semantic_type(value, inputs.semantics);
    emit_typed_struct_get(
        &mut function,
        inputs.gc.index(Type::Option(option)),
        0,
        backend,
    );
    emit_unary("Some", value, backend, &mut function, inputs);
    function.instruction(&Instruction::End);
    finish_recursion_guard(&mut function, inputs);
    function
}

fn compile_result(
    result: crate::ast::ResultTypeId,
    value: TypeId,
    inputs: &DisplayInputs<'_>,
) -> Function {
    let mut function = Function::new([]);
    begin_recursion_guard(&mut function, inputs);
    let type_index = inputs.gc.index(Type::Result(result));
    function
        .instruction(&Instruction::LocalGet(0))
        .instruction(&Instruction::RefAsNonNull);
    emit_typed_struct_get(&mut function, type_index, 1, Type::I32);
    function.instruction(&Instruction::If(BlockType::Result(
        inputs.gc.val_type(Type::Standard(StdlibTypeId::String)),
    )));
    function
        .instruction(&Instruction::LocalGet(0))
        .instruction(&Instruction::RefAsNonNull);
    emit_typed_struct_get(
        &mut function,
        type_index,
        2,
        Type::Standard(StdlibTypeId::String),
    );
    let string = inputs
        .semantics
        .types()
        .id_for_standard(StdlibTypeId::String);
    emit_unary(
        "Err",
        string,
        Type::Standard(StdlibTypeId::String),
        &mut function,
        inputs,
    );
    function
        .instruction(&Instruction::Else)
        .instruction(&Instruction::LocalGet(0))
        .instruction(&Instruction::RefAsNonNull);
    let backend = super::semantic_type(value, inputs.semantics);
    emit_typed_struct_get(&mut function, type_index, 0, backend);
    emit_unary("Ok", value, backend, &mut function, inputs);
    function.instruction(&Instruction::End);
    finish_recursion_guard(&mut function, inputs);
    function
}

fn compile_range(
    range: crate::ast::RangeTypeId,
    bound: TypeId,
    kind: RangeKind,
    inputs: &DisplayInputs<'_>,
) -> Function {
    let mut function = Function::new([]);
    begin_recursion_guard(&mut function, inputs);
    let backend = super::semantic_type(bound, inputs.semantics);
    let type_index = inputs.gc.index(Type::Range(range));
    function
        .instruction(&Instruction::LocalGet(0))
        .instruction(&Instruction::RefAsNonNull);
    emit_typed_struct_get(&mut function, type_index, 0, backend);
    emit_value(&mut function, bound, backend, inputs);
    emit_string_literal(
        &mut function,
        match kind {
            RangeKind::Exclusive => "..<",
            RangeKind::Inclusive => "..=",
        },
        inputs.gc,
    );
    function
        .instruction(&Instruction::LocalGet(0))
        .instruction(&Instruction::RefAsNonNull);
    emit_typed_struct_get(&mut function, type_index, 1, backend);
    emit_value(&mut function, bound, backend, inputs);
    join_pieces(&mut function, 3, inputs);
    finish_recursion_guard(&mut function, inputs);
    function
}

fn compile_iterator_step(
    step: crate::ast::TypeApplicationId,
    value: TypeId,
    inputs: &DisplayInputs<'_>,
) -> Function {
    let mut function = Function::new([]);
    begin_recursion_guard(&mut function, inputs);
    function
        .instruction(&Instruction::LocalGet(0))
        .instruction(&Instruction::RefIsNull)
        .instruction(&Instruction::If(BlockType::Result(
            inputs.gc.val_type(Type::Standard(StdlibTypeId::String)),
        )));
    emit_string_literal(&mut function, "End", inputs.gc);
    function
        .instruction(&Instruction::Else)
        .instruction(&Instruction::LocalGet(0))
        .instruction(&Instruction::RefAsNonNull);
    let backend = super::semantic_type(value, inputs.semantics);
    emit_typed_struct_get(
        &mut function,
        inputs.gc.index(Type::Application(step)),
        0,
        backend,
    );
    emit_unary("Item", value, backend, &mut function, inputs);
    function.instruction(&Instruction::End);
    finish_recursion_guard(&mut function, inputs);
    function
}

fn begin_recursion_guard(function: &mut Function, inputs: &DisplayInputs<'_>) {
    function
        .instruction(&Instruction::GlobalGet(inputs.debug_depth))
        .instruction(&Instruction::I32Const(64))
        .instruction(&Instruction::I32GeU)
        .instruction(&Instruction::If(BlockType::Result(
            inputs.gc.val_type(Type::Standard(StdlibTypeId::String)),
        )));
    emit_string_literal(function, "<cycle>", inputs.gc);
    function.instruction(&Instruction::Else);
    increment_debug_depth(function, inputs);
}

fn increment_debug_depth(function: &mut Function, inputs: &DisplayInputs<'_>) {
    function
        .instruction(&Instruction::GlobalGet(inputs.debug_depth))
        .instruction(&Instruction::I32Const(1))
        .instruction(&Instruction::I32Add)
        .instruction(&Instruction::GlobalSet(inputs.debug_depth));
}

fn decrement_debug_depth(function: &mut Function, inputs: &DisplayInputs<'_>) {
    function
        .instruction(&Instruction::GlobalGet(inputs.debug_depth))
        .instruction(&Instruction::I32Const(1))
        .instruction(&Instruction::I32Sub)
        .instruction(&Instruction::GlobalSet(inputs.debug_depth));
}

fn finish_recursion_guard(function: &mut Function, inputs: &DisplayInputs<'_>) {
    // The formatted String stays below the depth update on the operand stack.
    decrement_debug_depth(function, inputs);
    function
        .instruction(&Instruction::End)
        .instruction(&Instruction::End);
}

fn emit_unary(
    name: &str,
    value: TypeId,
    backend: Type,
    function: &mut Function,
    inputs: &DisplayInputs<'_>,
) {
    emit_value(function, value, backend, inputs);
    emit_string_literal(function, name, inputs.gc);
    function.instruction(&Instruction::Call(
        inputs.helpers.function(RuntimeHelperId::WrapDebugVariant),
    ));
}

fn string_array(inputs: &DisplayInputs<'_>) -> (crate::ast::ArrayTypeId, crate::ast::ArrayTypeId) {
    let array = inputs
        .arrays
        .iter()
        .find(|array| {
            try_array_element_type(array.id, inputs.semantics)
                == Some(Type::Standard(StdlibTypeId::String))
        })
        .expect("derived Debug requires the runtime String array layout")
        .id;
    let storage = array_value::storage_id(array, inputs.arrays, inputs.semantics);
    (array, storage)
}

fn set_piece_literal(
    function: &mut Function,
    pieces: u32,
    index: u32,
    value: &str,
    inputs: &DisplayInputs<'_>,
) {
    let (_, storage) = string_array(inputs);
    function
        .instruction(&Instruction::LocalGet(pieces))
        .instruction(&Instruction::RefAsNonNull)
        .instruction(&Instruction::I32Const(index as i32));
    emit_string_literal(function, value, inputs.gc);
    function.instruction(&Instruction::ArraySet(
        inputs.gc.index(Type::ArrayStorage(storage)),
    ));
}

fn join_pieces(function: &mut Function, count: u32, inputs: &DisplayInputs<'_>) {
    let strings = inputs
        .arrays
        .iter()
        .find(|array| {
            try_array_element_type(array.id, inputs.semantics)
                == Some(Type::Standard(StdlibTypeId::String))
        })
        .expect("derived Display requires the runtime String array layout");
    array_value::emit_new_fixed(function, inputs.gc, strings.id, count);
    function
        .instruction(&Instruction::RefNull(GC_NULL_HEAP_TYPE))
        .instruction(&Instruction::Call(
            inputs.helpers.function(RuntimeHelperId::JoinStrings),
        ));
}

fn emit_value(function: &mut Function, ty: TypeId, backend: Type, inputs: &DisplayInputs<'_>) {
    if backend == Type::Standard(StdlibTypeId::String) {
        function
            .instruction(&Instruction::I32Const(b'"' as i32))
            .instruction(&Instruction::Call(
                inputs.helpers.function(RuntimeHelperId::QuoteDebugString),
            ));
        return;
    }
    if backend == Type::None {
        function.instruction(&Instruction::Drop);
        emit_string_literal(function, "None", inputs.gc);
        return;
    }
    if backend == Type::Bool {
        function.instruction(&Instruction::If(BlockType::Result(
            inputs.gc.val_type(Type::Standard(StdlibTypeId::String)),
        )));
        emit_string_literal(function, "true", inputs.gc);
        function.instruction(&Instruction::Else);
        emit_string_literal(function, "false", inputs.gc);
        function.instruction(&Instruction::End);
        return;
    }
    if backend == Type::Char {
        function
            .instruction(&Instruction::Call(
                inputs.helpers.function(RuntimeHelperId::FormatChar),
            ))
            .instruction(&Instruction::I32Const(b'\'' as i32))
            .instruction(&Instruction::Call(
                inputs.helpers.function(RuntimeHelperId::QuoteDebugString),
            ));
        return;
    }
    if let Some(debug) = inputs.displays.custom_debug.get(&ty) {
        function.instruction(&Instruction::Call(inputs.users[debug].call));
        return;
    }
    if let Some(display) = inputs.displays.derived.get(&ty) {
        function.instruction(&Instruction::Call(display.function));
        return;
    }
    if backend == Type::F32 {
        function.instruction(&Instruction::Call(
            inputs.helpers.function(RuntimeHelperId::FormatF32),
        ));
        return;
    }
    if backend == Type::F64 {
        function.instruction(&Instruction::Call(
            inputs.helpers.function(RuntimeHelperId::FormatF64),
        ));
        return;
    }
    emit_integer_to_i64(function, backend);
    function
        .instruction(&Instruction::I32Const(10))
        .instruction(&Instruction::I32Const(backend.is_signed() as i32))
        .instruction(&Instruction::Call(
            inputs.helpers.function(RuntimeHelperId::FormatI64),
        ));
}

fn emit_integer_to_i64(function: &mut Function, source: Type) {
    if matches!(source, Type::I8 | Type::I16 | Type::I32) {
        function.instruction(&Instruction::I64ExtendI32S);
    } else if matches!(source, Type::U8 | Type::U16 | Type::U32) {
        function.instruction(&Instruction::I64ExtendI32U);
    } else {
        debug_assert!(matches!(source, Type::I64 | Type::U64 | Type::Address));
    }
}
