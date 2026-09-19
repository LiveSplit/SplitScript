//! Deterministic WebAssembly GC type and layout planning.

use wasm_encoder::{
    AbstractHeapType, ArrayType, CompositeInnerType, CompositeType, FieldType, FuncType, HeapType,
    RefType, StorageType, StructType, SubType, TypeSection, ValType,
};

use crate::{
    ast::{EnumDecl, Program},
    semantic::SemanticModel,
    stdlib::{
        DeclaredTypeRef, RuntimeRepresentation, StandardLibrary, StdlibTypeConstructorId,
        StdlibTypeId, TypeRef,
    },
    types::{
        ResolvedApplicationType, ResolvedArrayType, ResolvedAsyncType, ResolvedCallableType,
        ResolvedIteratorType, ResolvedOptionType, ResolvedRangeType, ResolvedResultType,
        ResolvedSetType, TypeId, TypeKind,
    },
};

use super::{
    GcLayout, Type, array_element_type, async_frame::AsyncFrameLayouts, enum_variant_payload,
    managed_snapshot_field_type, option_value_type, reachability, result_value_type, semantic_type,
    standard_field_type, struct_field_type, value_type,
};

pub(super) struct EncodedTypes {
    pub section: TypeSection,
    pub next_type_index: u32,
    pub layout: GcLayout,
}

pub(super) struct Inputs<'a> {
    pub standard_library: &'a StandardLibrary,
    pub program: &'a Program,
    pub wasm_ir: &'a crate::wasm_ir::Program,
    pub semantics: &'a SemanticModel,
    pub async_frames: &'a AsyncFrameLayouts,
    pub enums: &'a [EnumDecl],
    pub array_types: &'a [ResolvedArrayType],
    pub option_types: &'a [ResolvedOptionType],
    pub result_types: &'a [ResolvedResultType],
    pub async_types: &'a [ResolvedAsyncType],
    pub iterator_types: &'a [ResolvedIteratorType],
    pub callable_types: &'a [ResolvedCallableType],
    pub set_types: &'a [ResolvedSetType],
    pub application_types: &'a [ResolvedApplicationType],
    pub range_types: &'a [ResolvedRangeType],
    pub reachability: &'a reachability::Reachability,
}

pub(super) fn encode(inputs: Inputs<'_>) -> EncodedTypes {
    let Inputs {
        standard_library,
        program,
        wasm_ir,
        semantics,
        async_frames,
        enums,
        array_types,
        option_types,
        result_types,
        async_types,
        iterator_types,
        callable_types,
        set_types,
        application_types,
        range_types,
        reachability,
    } = inputs;
    let layout = GcLayout::plan(super::gc_layout::Inputs {
        standard_library: standard_library.clone(),
        program,
        wasm_ir,
        enums,
        semantics,
        arrays: array_types,
        options: option_types,
        results: result_types,
        asyncs: async_types,
        iterators: iterator_types,
        callables: callable_types,
        sets: set_types,
        applications: application_types,
        ranges: range_types,
        async_frames,
        reachability,
    });
    let mut recursive_types = vec![SubType {
        is_final: true,
        supertype_idx: None,
        composite_type: CompositeType {
            inner: CompositeInnerType::Struct(StructType {
                fields: semantics
                    .state_storage_fields()
                    .iter()
                    .map(|field| FieldType {
                        element_type: layout.storage_type(value_type(*field, semantics)),
                        mutable: true,
                    })
                    .collect(),
            }),
            shared: false,
            descriptor: None,
            describes: None,
        },
    }];
    for declaration in standard_library.all_types() {
        if !reachability.contains_standard_type(declaration.id) {
            continue;
        }
        let inner = match declaration.representation {
            RuntimeRepresentation::Scalar { .. } => continue,
            RuntimeRepresentation::GcArray {
                element, mutable, ..
            } => CompositeInnerType::Array(ArrayType(FieldType {
                element_type: layout
                    .storage_type(Type::from_declared(DeclaredTypeRef::Core(element))),
                mutable,
            })),
            RuntimeRepresentation::GcStruct { .. } => CompositeInnerType::Struct(StructType {
                fields: standard_library
                    .fields_of(declaration.id)
                    .map(|field| FieldType {
                        element_type: layout.storage_type(standard_field_type(field.id, semantics)),
                        mutable: false,
                    })
                    .collect(),
            }),
            RuntimeRepresentation::Enum { .. } => CompositeInnerType::Struct(StructType {
                fields: std::iter::once(FieldType {
                    element_type: StorageType::Val(ValType::I32),
                    mutable: false,
                })
                .chain(
                    standard_library
                        .variants_of(declaration.id)
                        .map(|_| FieldType {
                            element_type: StorageType::Val(ValType::I32),
                            mutable: false,
                        }),
                )
                .collect(),
            }),
        };
        recursive_types.push(SubType {
            is_final: true,
            supertype_idx: None,
            composite_type: CompositeType {
                inner,
                shared: false,
                descriptor: None,
                describes: None,
            },
        });
    }
    let mut fields = Vec::with_capacity(
        1 + async_frames
            .actions()
            .map(|(_, layout)| layout.types.len())
            .sum::<usize>(),
    );
    fields.push(FieldType {
        element_type: StorageType::Val(ValType::I32),
        mutable: true,
    });
    for (_, frame_layout) in async_frames.actions() {
        debug_assert!(
            frame_layout.types.iter().all(|ty| *ty != Type::Never),
            "async frame contains a `Never` field: {frame_layout:?}"
        );
        fields.extend(frame_layout.types.iter().enumerate().map(|(position, ty)| {
            let field = frame_layout.base_fields + position as u32;
            FieldType {
                element_type: if frame_layout.capture_cell_fields.contains(&field) {
                    layout.capture_cell_storage_type(*ty)
                } else {
                    layout.frame_storage_type(*ty)
                },
                mutable: true,
            }
        }));
    }
    recursive_types.push(SubType {
        is_final: true,
        supertype_idx: None,
        composite_type: CompositeType {
            inner: CompositeInnerType::Struct(StructType {
                fields: fields.into(),
            }),
            shared: false,
            descriptor: None,
            describes: None,
        },
    });
    for ty in layout.dynamic_types() {
        let (inner, is_final, supertype_idx) = match ty {
            Type::Struct(id) => {
                let structure = program
                    .structs
                    .iter()
                    .find(|structure| structure.id == id)
                    .expect("reachable struct layouts have declarations");
                (
                    CompositeInnerType::Struct(StructType {
                        fields: structure
                            .fields
                            .iter()
                            .map(|field| FieldType {
                                element_type: layout
                                    .storage_type(struct_field_type(field.id, semantics)),
                                mutable: false,
                            })
                            .collect(),
                    }),
                    true,
                    None,
                )
            }
            Type::ManagedClass(id) => {
                let class = program
                    .managed_class(id)
                    .expect("reachable managed class shapes have declarations");
                (
                    CompositeInnerType::Struct(StructType {
                        fields: class
                            .all_fields()
                            .filter(|field| !field.is_static)
                            .map(|field| FieldType {
                                element_type: layout
                                    .storage_type(managed_snapshot_field_type(field.id, semantics)),
                                mutable: false,
                            })
                            .collect(),
                    }),
                    true,
                    None,
                )
            }
            Type::Enum(id) => {
                let enumeration = enums
                    .iter()
                    .find(|enumeration| enumeration.id == id)
                    .expect("reachable enum layouts have declarations");
                (
                    CompositeInnerType::Struct(StructType {
                        fields: std::iter::once(FieldType {
                            element_type: StorageType::Val(ValType::I32),
                            mutable: false,
                        })
                        .chain(enumeration.variants.iter().map(|variant| FieldType {
                            element_type: enum_variant_payload(variant.id, semantics).map_or(
                                StorageType::Val(ValType::I32),
                                |ty| {
                                    if ty.has_runtime_value() {
                                        layout.storage_type(ty)
                                    } else {
                                        StorageType::Val(ValType::I32)
                                    }
                                },
                            ),
                            mutable: false,
                        }))
                        .collect(),
                    }),
                    true,
                    None,
                )
            }
            Type::Array(id) => {
                let declaration = array_types
                    .iter()
                    .find(|array| array.id == id)
                    .expect("reachable arrays have resolved declarations");
                let backing = array_types
                    .iter()
                    .find(|array| {
                        array.length.is_none()
                            && super::try_array_element_type(array.id, semantics)
                                == super::try_array_element_type(declaration.id, semantics)
                    })
                    .unwrap_or(declaration);
                (
                    CompositeInnerType::Struct(StructType {
                        fields: vec![
                            FieldType {
                                element_type: layout.storage_type(Type::ArrayStorage(backing.id)),
                                mutable: true,
                            },
                            FieldType {
                                element_type: StorageType::Val(ValType::I32),
                                mutable: true,
                            },
                            FieldType {
                                element_type: StorageType::Val(ValType::I32),
                                mutable: true,
                            },
                        ]
                        .into(),
                    }),
                    true,
                    None,
                )
            }
            Type::ArrayStorage(id) => {
                let declaration = array_types
                    .iter()
                    .find(|array| array.id == id)
                    .expect("reachable array storage has a resolved declaration");
                let supertype_idx = declaration.length.and_then(|_| {
                    array_types
                        .iter()
                        .find(|array| {
                            array.length.is_none()
                                && super::try_array_element_type(array.id, semantics)
                                    == super::try_array_element_type(declaration.id, semantics)
                        })
                        .map(|array| layout.index(Type::ArrayStorage(array.id)))
                });
                (
                    CompositeInnerType::Array(ArrayType(FieldType {
                        element_type: layout
                            .array_element_storage_type(array_element_type(id, semantics)),
                        mutable: true,
                    })),
                    declaration.length.is_some(),
                    supertype_idx,
                )
            }
            Type::Option(id) => (
                CompositeInnerType::Struct(StructType {
                    fields: vec![FieldType {
                        element_type: layout.storage_type(option_value_type(id, semantics)),
                        mutable: false,
                    }]
                    .into(),
                }),
                true,
                None,
            ),
            Type::Result(id) => (
                CompositeInnerType::Struct(StructType {
                    fields: vec![
                        FieldType {
                            element_type: layout.storage_type(result_value_type(id, semantics)),
                            mutable: false,
                        },
                        FieldType {
                            element_type: StorageType::Val(ValType::I32),
                            mutable: false,
                        },
                        FieldType {
                            element_type: StorageType::Val(
                                layout.val_type(Type::Standard(StdlibTypeId::String)),
                            ),
                            mutable: false,
                        },
                    ]
                    .into(),
                }),
                true,
                None,
            ),
            Type::Set(id) => {
                let set = set_types
                    .iter()
                    .find(|set| set.id == id)
                    .expect("reachable sets have resolved declarations");
                (
                    CompositeInnerType::Struct(StructType {
                        fields: vec![
                            FieldType {
                                element_type: layout.storage_type(Type::ArrayStorage(set.backing)),
                                mutable: true,
                            },
                            FieldType {
                                element_type: StorageType::Val(ValType::I32),
                                mutable: true,
                            },
                            FieldType {
                                element_type: StorageType::Val(ValType::I32),
                                mutable: true,
                            },
                        ]
                        .into(),
                    }),
                    true,
                    None,
                )
            }
            Type::Range(id) => {
                let range = range_types
                    .iter()
                    .find(|range| range.id == id)
                    .expect("reachable ranges have resolved declarations");
                let crate::types::ResolvedTypeRef::Core(bound) = range.bound else {
                    unreachable!("range bounds are concrete integer types")
                };
                let bound = Type::from_core(bound);
                let owner = match range.kind {
                    crate::ast::RangeKind::Exclusive => StdlibTypeConstructorId::ExclusiveRange,
                    crate::ast::RangeKind::Inclusive => StdlibTypeConstructorId::InclusiveRange,
                };
                let fields = standard_library
                    .fields_of_constructor(owner)
                    .map(|field| {
                        let ty = match field.ty {
                            TypeRef::Parameter(_) => bound,
                            TypeRef::Core(core) => Type::from_core(core),
                            TypeRef::Standard(standard) => Type::from_standard(standard),
                            _ => unreachable!(
                                "constructed GC fields currently use direct declared types"
                            ),
                        };
                        FieldType {
                            element_type: layout.storage_type(ty),
                            mutable: false,
                        }
                    })
                    .collect();
                (
                    CompositeInnerType::Struct(StructType { fields }),
                    true,
                    None,
                )
            }
            Type::Application(id) => {
                let application = application_types
                    .iter()
                    .find(|application| application.id == id)
                    .expect("reachable named applications have resolved declarations");
                let declaration = standard_library.type_constructor(application.constructor);
                let arguments = semantics
                    .types()
                    .iter()
                    .find_map(|(_, kind)| match kind {
                        TypeKind::Application {
                            layout, arguments, ..
                        } if *layout == id => Some(arguments.as_slice()),
                        _ => None,
                    })
                    .expect("reachable named applications have semantic argument types");
                let variables = declaration
                    .parameters
                    .iter()
                    .zip(arguments)
                    .map(|(parameter, argument)| (parameter.name, *argument))
                    .collect::<std::collections::HashMap<_, _>>();
                (
                    CompositeInnerType::Struct(StructType {
                        fields: standard_library
                            .fields_of_constructor(application.constructor)
                            .map(|field| FieldType {
                                element_type: layout.storage_type(semantic_type(
                                    instantiated_catalog_type(field.ty, &variables, semantics),
                                    semantics,
                                )),
                                // These generic structs currently back
                                // compiler-owned iterator cursors. Their
                                // storage is source-private, while `next()`
                                // advances the cursor in place.
                                mutable: true,
                            })
                            .collect(),
                    }),
                    true,
                    None,
                )
            }
            Type::Callable(id) => {
                let TypeKind::Callable {
                    parameters, result, ..
                } = semantics
                    .types()
                    .iter()
                    .find_map(|(_, kind)| {
                        matches!(kind, TypeKind::Callable { layout: candidate, .. } if *candidate == id)
                            .then_some(kind)
                    })
                    .expect("reachable callables have semantic signatures")
                else {
                    unreachable!()
                };
                debug_assert_eq!(
                    layout.callable_function_index(id),
                    recursive_types.len() as u32
                );
                let params = std::iter::once(ValType::Ref(RefType {
                    nullable: true,
                    heap_type: HeapType::Abstract {
                        shared: false,
                        ty: AbstractHeapType::Any,
                    },
                }))
                .chain(parameters.iter().filter_map(|parameter| {
                    let ty = semantic_type(*parameter, semantics);
                    ty.has_runtime_value().then(|| layout.val_type(ty))
                }))
                .collect::<Vec<_>>();
                let result_ty = semantic_type(*result, semantics);
                let results = result_ty
                    .has_runtime_value()
                    .then(|| layout.val_type(result_ty))
                    .into_iter()
                    .collect::<Vec<_>>();
                recursive_types.push(SubType {
                    is_final: true,
                    supertype_idx: None,
                    composite_type: CompositeType {
                        inner: CompositeInnerType::Func(FuncType::new(params, results)),
                        shared: false,
                        descriptor: None,
                        describes: None,
                    },
                });
                (
                    CompositeInnerType::Struct(StructType {
                        fields: vec![
                            FieldType {
                                element_type: StorageType::Val(ValType::Ref(RefType {
                                    nullable: false,
                                    heap_type: HeapType::Concrete(
                                        layout.callable_function_index(id),
                                    ),
                                })),
                                mutable: false,
                            },
                            FieldType {
                                element_type: StorageType::Val(ValType::Ref(RefType {
                                    nullable: true,
                                    heap_type: HeapType::Abstract {
                                        shared: false,
                                        ty: AbstractHeapType::Any,
                                    },
                                })),
                                mutable: false,
                            },
                        ]
                        .into(),
                    }),
                    true,
                    None,
                )
            }
            _ => unreachable!("only dynamic GC types are ordered by GcLayout"),
        };
        recursive_types.push(SubType {
            is_final,
            supertype_idx,
            composite_type: CompositeType {
                inner,
                shared: false,
                descriptor: None,
                describes: None,
            },
        });
    }
    for ty in layout.capture_cell_types() {
        debug_assert_eq!(layout.capture_cell_index(ty), recursive_types.len() as u32);
        recursive_types.push(SubType {
            is_final: true,
            supertype_idx: None,
            composite_type: CompositeType {
                inner: CompositeInnerType::Struct(StructType {
                    fields: [FieldType {
                        element_type: layout.storage_type(ty),
                        mutable: true,
                    }]
                    .into(),
                }),
                shared: false,
                descriptor: None,
                describes: None,
            },
        });
    }
    for instance in reachability.closure_instances() {
        let closure = wasm_ir
            .closure(instance.expression)
            .expect("reachable closure instances have bodies");
        if closure.captures.is_empty() {
            continue;
        }
        debug_assert_eq!(
            layout
                .closure_environment_index(instance)
                .expect("capturing closures have environment layouts"),
            recursive_types.len() as u32
        );
        recursive_types.push(SubType {
            is_final: true,
            supertype_idx: None,
            composite_type: CompositeType {
                inner: CompositeInnerType::Struct(StructType {
                    fields: closure
                        .captures
                        .iter()
                        .map(|capture| {
                            let ty = instance.owner.as_ref().map_or_else(
                                || value_type(capture.value, semantics),
                                |owner| {
                                    super::semantic_type(
                                        semantics.specialize_type(
                                            owner,
                                            semantics
                                                .value_type(capture.value)
                                                .expect("checked captures have types"),
                                        ),
                                        semantics,
                                    )
                                },
                            );
                            FieldType {
                                element_type: if capture.mutable && ty.has_runtime_value() {
                                    layout.capture_cell_storage_type(ty)
                                } else {
                                    layout.storage_type(ty)
                                },
                                mutable: false,
                            }
                        })
                        .collect(),
                }),
                shared: false,
                descriptor: None,
                describes: None,
            },
        });
    }
    for future in async_types
        .iter()
        .filter(|future| reachability.contains_async_type(future.id))
    {
        debug_assert_eq!(
            layout.index(Type::Async(future.id)),
            recursive_types.len() as u32
        );
        recursive_types.push(SubType {
            is_final: false,
            supertype_idx: None,
            composite_type: CompositeType {
                inner: CompositeInnerType::Struct(StructType {
                    fields: vec![
                        FieldType {
                            element_type: StorageType::Val(ValType::I32),
                            mutable: true,
                        },
                        FieldType {
                            element_type: StorageType::Val(ValType::I32),
                            mutable: false,
                        },
                        FieldType {
                            element_type: StorageType::Val(ValType::I64),
                            mutable: true,
                        },
                    ]
                    .into(),
                }),
                shared: false,
                descriptor: None,
                describes: None,
            },
        });
    }
    for iterator in iterator_types
        .iter()
        .filter(|iterator| reachability.contains_iterator_type(iterator.id))
    {
        debug_assert_eq!(
            layout.index(Type::Iterator(iterator.id)),
            recursive_types.len() as u32
        );
        recursive_types.push(SubType {
            is_final: false,
            supertype_idx: None,
            composite_type: CompositeType {
                inner: CompositeInnerType::Struct(StructType {
                    fields: vec![
                        FieldType {
                            element_type: StorageType::Val(ValType::I32),
                            mutable: true,
                        },
                        FieldType {
                            element_type: StorageType::Val(ValType::I32),
                            mutable: false,
                        },
                        FieldType {
                            element_type: StorageType::Val(ValType::I64),
                            mutable: true,
                        },
                    ]
                    .into(),
                }),
                shared: false,
                descriptor: None,
                describes: None,
            },
        });
    }
    for (instance, frame) in async_frames.functions() {
        let result = semantics.specialize_type(
            instance,
            semantics
                .function_result(instance.function)
                .expect("checked functions have result types"),
        );
        let result = super::semantic_type(result, semantics);
        let supertype = match result {
            Type::Async(future) => layout.index(Type::Async(future)),
            Type::Iterator(iterator) => layout.index(Type::Iterator(iterator)),
            _ => unreachable!("continuation-backed functions return async or iterator values"),
        };
        let frame_index = layout.function_frame_index(instance);
        debug_assert_eq!(frame_index, recursive_types.len() as u32);
        recursive_types.push(SubType {
            is_final: true,
            supertype_idx: Some(supertype),
            composite_type: CompositeType {
                inner: CompositeInnerType::Struct(StructType {
                    fields: [
                        FieldType {
                            element_type: StorageType::Val(ValType::I32),
                            mutable: true,
                        },
                        FieldType {
                            element_type: StorageType::Val(ValType::I32),
                            mutable: false,
                        },
                        FieldType {
                            element_type: StorageType::Val(ValType::I64),
                            mutable: true,
                        },
                    ]
                    .into_iter()
                    .chain(frame.types.iter().enumerate().map(|(position, ty)| {
                        let field = frame.base_fields + position as u32;
                        FieldType {
                            element_type: if frame.capture_cell_fields.contains(&field) {
                                layout.capture_cell_storage_type(*ty)
                            } else {
                                layout.frame_storage_type(*ty)
                            },
                            mutable: true,
                        }
                    }))
                    .collect(),
                }),
                shared: false,
                descriptor: None,
                describes: None,
            },
        });
    }
    for (instance, frame) in async_frames.closures() {
        let closure_type = wasm_ir
            .expression(instance.expression)
            .expect("reachable closure expressions belong to Wasm IR")
            .ty;
        let closure_type = instance.owner.as_ref().map_or(closure_type, |owner| {
            semantics.specialize_type(owner, closure_type)
        });
        let TypeKind::Callable { result, .. } = semantics.types().kind(closure_type) else {
            unreachable!("checked closure expressions have callable types")
        };
        let result = super::semantic_type(*result, semantics);
        let supertype = match result {
            Type::Async(future) => layout.index(Type::Async(future)),
            Type::Iterator(iterator) => layout.index(Type::Iterator(iterator)),
            _ => unreachable!("continuation-backed closures return async or iterator values"),
        };
        let frame_index = layout.closure_frame_index(instance);
        debug_assert_eq!(frame_index, recursive_types.len() as u32);
        recursive_types.push(SubType {
            is_final: true,
            supertype_idx: Some(supertype),
            composite_type: CompositeType {
                inner: CompositeInnerType::Struct(StructType {
                    fields: [
                        FieldType {
                            element_type: StorageType::Val(ValType::I32),
                            mutable: true,
                        },
                        FieldType {
                            element_type: StorageType::Val(ValType::I32),
                            mutable: false,
                        },
                        FieldType {
                            element_type: StorageType::Val(ValType::I64),
                            mutable: true,
                        },
                    ]
                    .into_iter()
                    .chain(frame.types.iter().enumerate().map(|(position, ty)| {
                        let field = frame.base_fields + position as u32;
                        FieldType {
                            element_type: if frame.capture_cell_fields.contains(&field) {
                                layout.capture_cell_storage_type(*ty)
                            } else {
                                layout.frame_storage_type(*ty)
                            },
                            mutable: true,
                        }
                    }))
                    .collect(),
                }),
                shared: false,
                descriptor: None,
                describes: None,
            },
        });
    }
    for (instance, frame) in async_frames.leaves() {
        let Type::Async(future) = frame.future else {
            unreachable!("leaf future layouts have async value types")
        };
        let frame_index = layout.leaf_frame_index(instance);
        debug_assert_eq!(frame_index, recursive_types.len() as u32);
        recursive_types.push(SubType {
            is_final: true,
            supertype_idx: Some(layout.index(Type::Async(future))),
            composite_type: CompositeType {
                inner: CompositeInnerType::Struct(StructType {
                    fields: [
                        FieldType {
                            element_type: StorageType::Val(ValType::I32),
                            mutable: true,
                        },
                        FieldType {
                            element_type: StorageType::Val(ValType::I32),
                            mutable: false,
                        },
                        FieldType {
                            element_type: StorageType::Val(ValType::I64),
                            mutable: true,
                        },
                    ]
                    .into_iter()
                    .chain(frame.types.iter().map(|ty| FieldType {
                        element_type: layout.frame_storage_type(*ty),
                        mutable: true,
                    }))
                    .collect(),
                }),
                shared: false,
                descriptor: None,
                describes: None,
            },
        });
    }
    let mut types = TypeSection::new();
    types.ty().rec(recursive_types);

    // `TypeSection::len` counts encoded entries, while a recursive group can
    // contain multiple indexed subtypes. State, Duration, String, the attach
    // Module, the attach continuation frame, and then user structs occupy the
    // first indices.
    EncodedTypes {
        section: types,
        next_type_index: layout.type_count,
        layout,
    }
}

pub(super) fn instantiated_catalog_type(
    ty: TypeRef,
    variables: &std::collections::HashMap<&'static str, TypeId>,
    semantics: &SemanticModel,
) -> TypeId {
    semantics
        .instantiated_catalog_type(ty, variables)
        .expect("instantiated catalog types have semantic layouts")
}
