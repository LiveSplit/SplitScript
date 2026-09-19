//! Structural equality body generation for runtime and source aggregates.

use wasm_encoder::{BlockType, Function, Instruction, ValType};

use crate::{
    ast::{ArrayTypeId, OptionTypeId, ResultTypeId},
    intrinsic_registry::RuntimeHelperId,
    semantic::SemanticModel,
    stdlib::{DeclaredTypeRef, RuntimeRepresentation, StdlibTypeId},
    structural::{StructuralMemberId, StructuralType, StructuralTypeId, StructuralTypes},
    types::{ResolvedArrayType, ResolvedOptionType, ResolvedResultType},
};

use super::super::{
    EqualityFunctions, GcLayout, RuntimeHelperPlan, Type, array_value, emit_array_get,
    emit_typed_struct_get, enum_variant_payload, managed_snapshot_field_type, option_value_type,
    result_value_type, semantic_type, standard_field_type, struct_field_type,
    try_array_element_type,
};
#[allow(clippy::too_many_arguments)]
pub(in crate::codegen) fn compile_equality(
    plan: &RuntimeHelperPlan,
    structural: &StructuralTypes,
    arrays: &[ResolvedArrayType],
    options: &[ResolvedOptionType],
    results: &[ResolvedResultType],
    semantics: &SemanticModel,
    equality_functions: &EqualityFunctions,
    gc: &GcLayout,
) -> Vec<Function> {
    let mut equality = Vec::new();
    // Payload-less/numeric-only structural equality never emits this call.
    // The sentinel is therefore only observed by the body builder when no
    // String edge exists, while dependency analysis requires the real helper
    // for every source-declared aggregate conservatively.
    let string_equality = plan
        .optional_function(RuntimeHelperId::StringEquality)
        .unwrap_or(0);
    for structure in gc.standard_library.all_types() {
        if equality_functions
            .standard_structs
            .contains_key(&structure.id)
        {
            equality.push(compile_standard_struct_equality(
                structure.id,
                semantics,
                equality_functions,
                string_equality,
                gc,
            ));
        }
    }
    for (_, structure) in structural.structs() {
        let StructuralTypeId::Struct(struct_id) = structure.id else {
            unreachable!()
        };
        if equality_functions.structs.contains_key(&struct_id) {
            equality.push(compile_struct_equality(
                structure,
                semantics,
                equality_functions,
                string_equality,
                gc,
            ));
        }
    }
    for (_, class) in structural.managed_classes() {
        let StructuralTypeId::ManagedClass(class_id) = class.id else {
            unreachable!()
        };
        if equality_functions.managed_classes.contains_key(&class_id) {
            equality.push(compile_managed_class_equality(
                class,
                semantics,
                equality_functions,
                string_equality,
                gc,
            ));
        }
    }
    for (_, enumeration) in structural.enums() {
        let StructuralTypeId::Enum(enum_id) = enumeration.id else {
            unreachable!()
        };
        if equality_functions.enums.contains_key(&enum_id) {
            equality.push(compile_enum_equality(
                enumeration,
                semantics,
                equality_functions,
                string_equality,
                gc,
            ));
        }
    }
    for array in arrays {
        if equality_functions.arrays.contains_key(&array.id) {
            equality.push(compile_array_equality(
                array.id,
                arrays,
                semantics,
                equality_functions,
                string_equality,
                gc,
            ));
        }
    }
    for option in options {
        if equality_functions.options.contains_key(&option.id) {
            equality.push(compile_option_equality(
                option.id,
                semantics,
                equality_functions,
                string_equality,
                gc,
            ));
        }
    }
    for result in results {
        if equality_functions.results.contains_key(&result.id) {
            equality.push(compile_result_equality(
                result.id,
                semantics,
                equality_functions,
                string_equality,
                gc,
            ));
        }
    }

    equality
}

fn compile_array_equality(
    array: ArrayTypeId,
    arrays: &[ResolvedArrayType],
    semantics: &SemanticModel,
    equality_functions: &EqualityFunctions,
    string_eq: u32,
    gc: &GcLayout,
) -> Function {
    let mut function = Function::new([(2, ValType::I32)]);
    let length = 2;
    let index = 3;
    let storage = array_value::storage_id(array, arrays, semantics);
    let element = try_array_element_type(array, semantics)
        .expect("reachable array equality has a lowerable element type");

    function.instruction(&Instruction::LocalGet(0));
    array_value::emit_length(&mut function, gc, array);
    function
        .instruction(&Instruction::LocalTee(length))
        .instruction(&Instruction::LocalGet(1));
    array_value::emit_length(&mut function, gc, array);
    function
        .instruction(&Instruction::I32Ne)
        .instruction(&Instruction::If(BlockType::Empty))
        .instruction(&Instruction::I32Const(0))
        .instruction(&Instruction::Return)
        .instruction(&Instruction::End)
        .instruction(&Instruction::Block(BlockType::Empty))
        .instruction(&Instruction::Loop(BlockType::Empty))
        .instruction(&Instruction::LocalGet(index))
        .instruction(&Instruction::LocalGet(length))
        .instruction(&Instruction::I32GeU)
        .instruction(&Instruction::BrIf(1))
        .instruction(&Instruction::LocalGet(0));
    array_value::emit_backing(&mut function, gc, array);
    function.instruction(&Instruction::LocalGet(index));
    emit_array_get(
        &mut function,
        gc.index(Type::ArrayStorage(storage)),
        element,
        gc,
    );
    function.instruction(&Instruction::LocalGet(1));
    array_value::emit_backing(&mut function, gc, array);
    function.instruction(&Instruction::LocalGet(index));
    emit_array_get(
        &mut function,
        gc.index(Type::ArrayStorage(storage)),
        element,
        gc,
    );
    emit_value_equality(&mut function, element, equality_functions, string_eq);
    function
        .instruction(&Instruction::I32Eqz)
        .instruction(&Instruction::If(BlockType::Empty))
        .instruction(&Instruction::I32Const(0))
        .instruction(&Instruction::Return)
        .instruction(&Instruction::End)
        .instruction(&Instruction::LocalGet(index))
        .instruction(&Instruction::I32Const(1))
        .instruction(&Instruction::I32Add)
        .instruction(&Instruction::LocalSet(index))
        .instruction(&Instruction::Br(0))
        .instruction(&Instruction::End)
        .instruction(&Instruction::End)
        .instruction(&Instruction::I32Const(1))
        .instruction(&Instruction::End);
    function
}

pub(in crate::codegen::runtime_helpers) fn compile_string_eq(gc: &GcLayout) -> Function {
    let mut function = Function::new([(2, ValType::I32)]);
    let left = 0;
    let right = 1;
    let len = 2;
    let index = 3;

    function
        .instruction(&Instruction::LocalGet(left))
        .instruction(&Instruction::ArrayLen)
        .instruction(&Instruction::LocalTee(len))
        .instruction(&Instruction::LocalGet(right))
        .instruction(&Instruction::ArrayLen)
        .instruction(&Instruction::I32Ne)
        .instruction(&Instruction::If(BlockType::Empty))
        .instruction(&Instruction::I32Const(0))
        .instruction(&Instruction::Return)
        .instruction(&Instruction::End)
        .instruction(&Instruction::Block(BlockType::Empty))
        .instruction(&Instruction::Loop(BlockType::Empty))
        .instruction(&Instruction::LocalGet(index))
        .instruction(&Instruction::LocalGet(len))
        .instruction(&Instruction::I32GeU)
        .instruction(&Instruction::BrIf(1))
        .instruction(&Instruction::LocalGet(left))
        .instruction(&Instruction::RefAsNonNull)
        .instruction(&Instruction::LocalGet(index))
        .instruction(&Instruction::ArrayGetU(
            gc.standard_index(StdlibTypeId::String),
        ))
        .instruction(&Instruction::LocalGet(right))
        .instruction(&Instruction::RefAsNonNull)
        .instruction(&Instruction::LocalGet(index))
        .instruction(&Instruction::ArrayGetU(
            gc.standard_index(StdlibTypeId::String),
        ))
        .instruction(&Instruction::I32Ne)
        .instruction(&Instruction::If(BlockType::Empty))
        .instruction(&Instruction::I32Const(0))
        .instruction(&Instruction::Return)
        .instruction(&Instruction::End)
        .instruction(&Instruction::LocalGet(index))
        .instruction(&Instruction::I32Const(1))
        .instruction(&Instruction::I32Add)
        .instruction(&Instruction::LocalSet(index))
        .instruction(&Instruction::Br(0))
        .instruction(&Instruction::End)
        .instruction(&Instruction::End)
        .instruction(&Instruction::I32Const(1))
        .instruction(&Instruction::End);
    function
}

fn compile_struct_equality(
    structure: &StructuralType,
    semantics: &SemanticModel,
    equality_functions: &EqualityFunctions,
    string_eq: u32,
    gc: &GcLayout,
) -> Function {
    let mut function = Function::new([]);
    let StructuralTypeId::Struct(struct_id) = structure.id else {
        unreachable!()
    };
    let type_index = gc.index(Type::Struct(struct_id));

    function.instruction(&Instruction::I32Const(1));
    for (field_index, field) in structure.members.iter().enumerate() {
        let StructuralMemberId::StructField(field_id) = field.source else {
            unreachable!()
        };
        let ty = struct_field_type(field_id, semantics);
        function
            .instruction(&Instruction::LocalGet(0))
            .instruction(&Instruction::RefAsNonNull);
        emit_typed_struct_get(&mut function, type_index, field_index as u32, ty);
        function
            .instruction(&Instruction::LocalGet(1))
            .instruction(&Instruction::RefAsNonNull);
        emit_typed_struct_get(&mut function, type_index, field_index as u32, ty);
        emit_value_equality(&mut function, ty, equality_functions, string_eq);
        function.instruction(&Instruction::I32And);
    }
    function.instruction(&Instruction::End);
    function
}

fn compile_managed_class_equality(
    class: &StructuralType,
    semantics: &SemanticModel,
    equality_functions: &EqualityFunctions,
    string_eq: u32,
    gc: &GcLayout,
) -> Function {
    let mut function = Function::new([]);
    let StructuralTypeId::ManagedClass(class_id) = class.id else {
        unreachable!()
    };
    let type_index = gc.index(Type::ManagedClass(class_id));
    for (field_index, field) in class.members.iter().enumerate() {
        let StructuralMemberId::ManagedField(field_id) = field.source else {
            unreachable!()
        };
        let ty = managed_snapshot_field_type(field_id, semantics);
        let get = |function: &mut Function, object| {
            function
                .instruction(&Instruction::LocalGet(object))
                .instruction(&Instruction::RefAsNonNull);
            emit_typed_struct_get(function, type_index, field_index as u32, ty);
        };
        // Inactive conditional fields contain defaults. Reference defaults are
        // null, including normally non-null strings and arrays. Compare the
        // saved slots without consulting the current attachment's shape.
        let conditional_reference = semantics.managed_field_shape_predicate(field_id).is_some()
            && matches!(gc.val_type(ty), ValType::Ref(_));
        if conditional_reference {
            get(&mut function, 0);
            function.instruction(&Instruction::RefIsNull);
            get(&mut function, 1);
            function
                .instruction(&Instruction::RefIsNull)
                .instruction(&Instruction::I32Or)
                .instruction(&Instruction::If(BlockType::Result(ValType::I32)));
            get(&mut function, 0);
            get(&mut function, 1);
            function
                .instruction(&Instruction::RefEq)
                .instruction(&Instruction::Else);
        }
        get(&mut function, 0);
        get(&mut function, 1);
        emit_value_equality(&mut function, ty, equality_functions, string_eq);
        if conditional_reference {
            function.instruction(&Instruction::End);
        }
        function
            .instruction(&Instruction::I32Eqz)
            .instruction(&Instruction::If(BlockType::Empty))
            .instruction(&Instruction::I32Const(0))
            .instruction(&Instruction::Return)
            .instruction(&Instruction::End);
    }
    function
        .instruction(&Instruction::I32Const(1))
        .instruction(&Instruction::End);
    function
}

fn compile_standard_struct_equality(
    structure: StdlibTypeId,
    semantics: &SemanticModel,
    equality_functions: &EqualityFunctions,
    string_eq: u32,
    gc: &GcLayout,
) -> Function {
    let mut function = Function::new([]);
    let ty = Type::Standard(structure);
    let type_index = gc.index(ty);

    function.instruction(&Instruction::I32Const(1));
    for (field_index, field) in gc.standard_library.fields_of(structure).enumerate() {
        let field_ty = standard_field_type(field.id, semantics);
        function
            .instruction(&Instruction::LocalGet(0))
            .instruction(&Instruction::RefAsNonNull);
        emit_typed_struct_get(&mut function, type_index, field_index as u32, field_ty);
        function
            .instruction(&Instruction::LocalGet(1))
            .instruction(&Instruction::RefAsNonNull);
        emit_typed_struct_get(&mut function, type_index, field_index as u32, field_ty);
        emit_value_equality(&mut function, field_ty, equality_functions, string_eq);
        function.instruction(&Instruction::I32And);
    }
    function.instruction(&Instruction::End);
    function
}

fn compile_enum_equality(
    enumeration: &StructuralType,
    semantics: &SemanticModel,
    equality_functions: &EqualityFunctions,
    string_eq: u32,
    gc: &GcLayout,
) -> Function {
    let mut function = Function::new([(1, ValType::I32)]);
    let tag = 2;
    let StructuralTypeId::Enum(enum_id) = enumeration.id else {
        unreachable!()
    };
    let type_index = gc.index(Type::Enum(enum_id));

    function
        .instruction(&Instruction::LocalGet(0))
        .instruction(&Instruction::RefAsNonNull);
    emit_typed_struct_get(&mut function, type_index, 0, Type::I32);
    function
        .instruction(&Instruction::LocalTee(tag))
        .instruction(&Instruction::LocalGet(1))
        .instruction(&Instruction::RefAsNonNull);
    emit_typed_struct_get(&mut function, type_index, 0, Type::I32);
    function
        .instruction(&Instruction::I32Ne)
        .instruction(&Instruction::If(BlockType::Empty))
        .instruction(&Instruction::I32Const(0))
        .instruction(&Instruction::Return)
        .instruction(&Instruction::End);

    for (variant_index, variant) in enumeration.members.iter().enumerate() {
        function
            .instruction(&Instruction::LocalGet(tag))
            .instruction(&Instruction::I32Const(variant_index as i32))
            .instruction(&Instruction::I32Eq)
            .instruction(&Instruction::If(BlockType::Empty));
        if variant
            .ty
            .is_some_and(|ty| semantic_type(ty, semantics).has_runtime_value())
        {
            let StructuralMemberId::EnumVariant(variant_id) = variant.source else {
                unreachable!()
            };
            let ty = enum_variant_payload(variant_id, semantics)
                .expect("payload variants have backend types");
            function
                .instruction(&Instruction::LocalGet(0))
                .instruction(&Instruction::RefAsNonNull);
            emit_typed_struct_get(&mut function, type_index, variant_index as u32 + 1, ty);
            function
                .instruction(&Instruction::LocalGet(1))
                .instruction(&Instruction::RefAsNonNull);
            emit_typed_struct_get(&mut function, type_index, variant_index as u32 + 1, ty);
            emit_value_equality(&mut function, ty, equality_functions, string_eq);
        } else {
            function.instruction(&Instruction::I32Const(1));
        }
        function
            .instruction(&Instruction::Return)
            .instruction(&Instruction::End);
    }
    function
        .instruction(&Instruction::I32Const(0))
        .instruction(&Instruction::End);
    function
}

fn compile_option_equality(
    option: OptionTypeId,
    semantics: &SemanticModel,
    equality_functions: &EqualityFunctions,
    string_eq: u32,
    gc: &GcLayout,
) -> Function {
    let mut function = Function::new([]);
    let type_index = gc.index(Type::Option(option));
    let value_type = option_value_type(option, semantics);

    function
        .instruction(&Instruction::LocalGet(0))
        .instruction(&Instruction::RefIsNull)
        .instruction(&Instruction::If(BlockType::Empty))
        .instruction(&Instruction::LocalGet(1))
        .instruction(&Instruction::RefIsNull)
        .instruction(&Instruction::Return)
        .instruction(&Instruction::End)
        .instruction(&Instruction::LocalGet(1))
        .instruction(&Instruction::RefIsNull)
        .instruction(&Instruction::If(BlockType::Empty))
        .instruction(&Instruction::I32Const(0))
        .instruction(&Instruction::Return)
        .instruction(&Instruction::End)
        .instruction(&Instruction::LocalGet(0))
        .instruction(&Instruction::RefAsNonNull);
    emit_typed_struct_get(&mut function, type_index, 0, value_type);
    function
        .instruction(&Instruction::LocalGet(1))
        .instruction(&Instruction::RefAsNonNull);
    emit_typed_struct_get(&mut function, type_index, 0, value_type);
    emit_value_equality(&mut function, value_type, equality_functions, string_eq);
    function.instruction(&Instruction::End);
    function
}

fn compile_result_equality(
    result: ResultTypeId,
    semantics: &SemanticModel,
    equality_functions: &EqualityFunctions,
    string_eq: u32,
    gc: &GcLayout,
) -> Function {
    let mut function = Function::new([(1, ValType::I32)]);
    let tag = 2;
    let type_index = gc.index(Type::Result(result));
    let value_type = result_value_type(result, semantics);

    function
        .instruction(&Instruction::LocalGet(0))
        .instruction(&Instruction::RefAsNonNull);
    emit_typed_struct_get(&mut function, type_index, 1, Type::I32);
    function
        .instruction(&Instruction::LocalTee(tag))
        .instruction(&Instruction::LocalGet(1))
        .instruction(&Instruction::RefAsNonNull);
    emit_typed_struct_get(&mut function, type_index, 1, Type::I32);
    function
        .instruction(&Instruction::I32Ne)
        .instruction(&Instruction::If(BlockType::Empty))
        .instruction(&Instruction::I32Const(0))
        .instruction(&Instruction::Return)
        .instruction(&Instruction::End)
        .instruction(&Instruction::LocalGet(tag))
        .instruction(&Instruction::If(BlockType::Empty))
        .instruction(&Instruction::LocalGet(0))
        .instruction(&Instruction::RefAsNonNull);
    emit_typed_struct_get(
        &mut function,
        type_index,
        2,
        Type::Standard(StdlibTypeId::String),
    );
    function
        .instruction(&Instruction::LocalGet(1))
        .instruction(&Instruction::RefAsNonNull);
    emit_typed_struct_get(
        &mut function,
        type_index,
        2,
        Type::Standard(StdlibTypeId::String),
    );
    emit_value_equality(
        &mut function,
        Type::Standard(StdlibTypeId::String),
        equality_functions,
        string_eq,
    );
    function
        .instruction(&Instruction::Return)
        .instruction(&Instruction::End)
        .instruction(&Instruction::LocalGet(0))
        .instruction(&Instruction::RefAsNonNull);
    emit_typed_struct_get(&mut function, type_index, 0, value_type);
    function
        .instruction(&Instruction::LocalGet(1))
        .instruction(&Instruction::RefAsNonNull);
    emit_typed_struct_get(&mut function, type_index, 0, value_type);
    emit_value_equality(&mut function, value_type, equality_functions, string_eq);
    function.instruction(&Instruction::End);
    function
}

pub(in crate::codegen) fn emit_value_equality(
    function: &mut Function,
    ty: Type,
    equality_functions: &EqualityFunctions,
    string_eq: u32,
) {
    let instruction = match ty {
        Type::Standard(StdlibTypeId::String) => Instruction::Call(string_eq),
        Type::Standard(standard) => match equality_functions
            .standard_library
            .type_decl(standard)
            .representation
        {
            RuntimeRepresentation::Scalar { storage } => {
                emit_value_equality(
                    function,
                    Type::from_declared(DeclaredTypeRef::Core(storage)),
                    equality_functions,
                    string_eq,
                );
                return;
            }
            RuntimeRepresentation::GcStruct { .. } => {
                Instruction::Call(equality_functions.standard_structs[&standard])
            }
            RuntimeRepresentation::GcArray { .. } | RuntimeRepresentation::Enum { .. } => {
                unreachable!("catalog validation rejected unsupported equality for `{standard:?}`")
            }
        },
        Type::ManagedClass(class) => Instruction::Call(equality_functions.managed_classes[&class]),
        Type::Struct(structure) => Instruction::Call(equality_functions.structs[&structure]),
        Type::Enum(enumeration) => Instruction::Call(equality_functions.enums[&enumeration]),
        Type::Array(array) => Instruction::Call(equality_functions.arrays[&array]),
        Type::Option(option) => Instruction::Call(equality_functions.options[&option]),
        Type::Result(result) => Instruction::Call(equality_functions.results[&result]),
        Type::None => Instruction::RefEq,
        Type::F32 => Instruction::F32Eq,
        Type::F64 => Instruction::F64Eq,
        Type::I64 | Type::U64 | Type::Address => Instruction::I64Eq,
        Type::Bool | Type::I8 | Type::U8 | Type::I16 | Type::U16 | Type::I32 | Type::U32 => {
            Instruction::I32Eq
        }
        _ => unreachable!("type checking rejected structural equality for `{ty:?}`"),
    };
    function.instruction(&instruction);
}
