use std::collections::HashMap;

use wasm_encoder::{ConstExpr, GlobalSection, GlobalType, HeapType, RefType, ValType};

use crate::{
    ast::{ActionKind, EnumVariantId, Program, ValueId},
    managed::ManagedBindingPlan,
    semantic::{FunctionInstance, SemanticModel},
    stdlib::{
        CoreTypeId, RuntimeRepresentation, StandardLibrary, StateProviderAttachment,
        StdlibStateProviderId, StdlibTypeId,
    },
    wasm_ir,
};

use super::{
    GcLayout, STATE_TYPE, Type, constant, is_wasm_global_constant,
    managed_state_reads::{self, ManagedStateReadCache},
    semantic_type, value_type,
};

/// User attachment initialization completed and normal state polling may run.
pub(super) const ATTACH_READY: i32 = 1;
/// This process was rejected after acquisition and remains held only until it
/// closes, preventing the discovery loop from immediately selecting it again.
pub(super) const ATTACH_REJECTED: i32 = 2;
/// Provider preparation and automatic shape selection completed; user
/// `onAttach` is still pending.
pub(super) const ATTACH_PREPARED: i32 = 3;

pub(super) struct GlobalPlan {
    pub section: GlobalSection,
    pub runtime: RuntimeGlobals,
    pub variables: HashMap<ValueId, u32>,
    pub variable_types: HashMap<ValueId, Type>,
    pub settings: HashMap<ValueId, SettingStorage>,
    pub managed_state_reads: ManagedStateReadCache,
    pub provider_values: HashMap<StdlibStateProviderId, u32>,
    pub provider_attachment_frames: HashMap<EnumVariantId, u32>,
}

#[derive(Clone, Copy)]
pub(super) struct SettingStorage {
    pub current: u32,
    pub old: u32,
    pub ty: Type,
}

#[derive(Debug, Clone, Copy)]
pub(super) struct RuntimeGlobals {
    pub process: u32,
    /// Index of the source-declared process name that successfully attached.
    /// `-1` means no process is attached.
    pub process_name: u32,
    /// Physical native pointer width in bytes, detected once per attachment.
    /// Storage is omitted when no reachable operation depends on it.
    pub process_pointer_size: Option<u32>,
    pub provider_value: Option<u32>,
    /// Pending source-defined state-provider attachment future.
    pub provider_attachment_frame: Option<u32>,
    /// Opaque provider-specific context produced before user attachment code.
    pub provider_preparation_value: Option<u32>,
    /// Pending source-defined state-provider preparation future.
    pub provider_preparation_frame: Option<u32>,
    /// Whether preparation completed for the current process attachment.
    pub provider_prepared: Option<u32>,
    /// The typed discriminator for a multi-provider state.
    pub selected_provider: Option<u32>,
    pub current: u32,
    pub old: u32,
    pub attach_ready: u32,
    /// Whether this attachment has committed at least one complete snapshot.
    pub state_ready: u32,
    /// Last timer state observed by the module-wide lifecycle monitor. `-1`
    /// means the loaded script has not established its baseline yet.
    /// Storage exists only when a timer lifecycle observer is declared.
    pub observed_timer_state: Option<u32>,
    /// Whether an observed start transition has completed `onStart` for the
    /// current timer attempt. Storage exists only for attempt-scoped globals.
    pub attempt_ready: Option<u32>,
    /// Current compiler-derived structural formatting depth. This bounds
    /// recursive container graphs without allocating traversal state.
    pub debug_depth: u32,
    /// Monotonically wrapping identity of the current host update. First-class
    /// future dispatch uses it to prevent aliased handles from advancing more
    /// than once during one update.
    pub future_poll_epoch: u32,
    pub async_frame: u32,
    /// Boolean completion of a suspending `whileAttached` invocation. The
    /// action's Wasm return remains the poll readiness flag.
    pub while_attached_result: Option<u32>,
}

pub(super) struct Inputs<'a> {
    pub program: &'a Program,
    pub semantics: &'a SemanticModel,
    pub gc: &'a GcLayout,
    pub wasm_ir: &'a wasm_ir::Program,
    pub managed: &'a ManagedBindingPlan,
    pub reachability: &'a super::reachability::Reachability,
    pub provider_attachment: Option<&'a FunctionInstance>,
    pub provider_alternatives: &'a [(EnumVariantId, StdlibStateProviderId, FunctionInstance)],
    pub provider_preparation: Option<&'a FunctionInstance>,
    pub needs_native_pointer_size: bool,
}

pub(super) fn encode(inputs: Inputs<'_>) -> GlobalPlan {
    let Inputs {
        program,
        semantics,
        gc,
        wasm_ir,
        managed,
        reachability,
        provider_attachment,
        provider_alternatives,
        provider_preparation,
        needs_native_pointer_size,
    } = inputs;
    let mut section = GlobalSection::new();
    let process = section.len();
    section.global(
        GlobalType {
            val_type: ValType::I64,
            mutable: true,
            shared: false,
        },
        &ConstExpr::i64_const(0),
    );
    let process_name = section.len();
    section.global(
        GlobalType {
            val_type: ValType::I32,
            mutable: true,
            shared: false,
        },
        &ConstExpr::i32_const(-1),
    );
    let process_pointer_size = needs_native_pointer_size.then(|| {
        let index = section.len();
        section.global(
            GlobalType {
                val_type: ValType::I32,
                mutable: true,
                shared: false,
            },
            &ConstExpr::i32_const(0),
        );
        index
    });
    let provider_value = semantics.state_provider().and_then(|provider| {
        let ty = wasm_ir
            .standard_library()
            .state_provider(provider)
            .process_type;
        if wasm_ir
            .standard_library()
            .state_provider(provider)
            .attachment
            == StateProviderAttachment::Identity
        {
            return None;
        }
        let index = section.len();
        let initial = match wasm_ir.standard_library().type_decl(ty).representation {
            RuntimeRepresentation::Scalar {
                storage: CoreTypeId::I64,
            } => ConstExpr::i64_const(0),
            RuntimeRepresentation::GcStruct { .. }
            | RuntimeRepresentation::GcArray { .. }
            | RuntimeRepresentation::Enum { .. } => {
                ConstExpr::ref_null(HeapType::Concrete(gc.standard_index(ty)))
            }
            representation => {
                unreachable!("unsupported state-provider representation: {representation:?}")
            }
        };
        section.global(
            GlobalType {
                val_type: gc.val_type(Type::Standard(ty)),
                mutable: true,
                shared: false,
            },
            &initial,
        );
        Some(index)
    });
    let mut provider_values = HashMap::new();
    for (_, provider, _) in provider_alternatives {
        let declaration = wasm_ir.standard_library().state_provider(*provider);
        if declaration.attachment == StateProviderAttachment::Identity {
            continue;
        }
        let ty = declaration.process_type;
        let index = section.len();
        let initial = match wasm_ir.standard_library().type_decl(ty).representation {
            RuntimeRepresentation::Scalar {
                storage: CoreTypeId::I64,
            } => ConstExpr::i64_const(0),
            RuntimeRepresentation::GcStruct { .. }
            | RuntimeRepresentation::GcArray { .. }
            | RuntimeRepresentation::Enum { .. } => {
                ConstExpr::ref_null(HeapType::Concrete(gc.standard_index(ty)))
            }
            representation => {
                unreachable!("unsupported state-provider representation: {representation:?}")
            }
        };
        section.global(
            GlobalType {
                val_type: gc.val_type(Type::Standard(ty)),
                mutable: true,
                shared: false,
            },
            &initial,
        );
        provider_values.insert(*provider, index);
    }
    let provider_attachment_frame = provider_attachment.map(|attachment| {
        let index = section.len();
        let frame_type = gc.function_frame_index(attachment);
        section.global(
            GlobalType {
                val_type: ValType::Ref(RefType {
                    nullable: true,
                    heap_type: HeapType::Concrete(frame_type),
                }),
                mutable: true,
                shared: false,
            },
            &ConstExpr::ref_null(HeapType::Concrete(frame_type)),
        );
        index
    });
    let mut provider_attachment_frames = HashMap::new();
    for (variant, _, attachment) in provider_alternatives {
        let index = section.len();
        let frame_type = gc.function_frame_index(attachment);
        section.global(
            GlobalType {
                val_type: ValType::Ref(RefType {
                    nullable: true,
                    heap_type: HeapType::Concrete(frame_type),
                }),
                mutable: true,
                shared: false,
            },
            &ConstExpr::ref_null(HeapType::Concrete(frame_type)),
        );
        provider_attachment_frames.insert(*variant, index);
    }
    let provider_preparation_value = provider_preparation.map(|preparation| {
        let completion = semantic_type(
            semantics.specialize_type(
                preparation,
                semantics
                    .function_completion(preparation.function)
                    .expect("checked provider preparation has a completion type"),
            ),
            semantics,
        );
        let index = section.len();
        section.global(
            GlobalType {
                val_type: gc.val_type(completion),
                mutable: true,
                shared: false,
            },
            &default_const_expr(gc.val_type(completion)),
        );
        index
    });
    let provider_preparation_frame = provider_preparation.map(|preparation| {
        let index = section.len();
        let frame_type = gc.function_frame_index(preparation);
        section.global(
            GlobalType {
                val_type: ValType::Ref(RefType {
                    nullable: true,
                    heap_type: HeapType::Concrete(frame_type),
                }),
                mutable: true,
                shared: false,
            },
            &ConstExpr::ref_null(HeapType::Concrete(frame_type)),
        );
        index
    });
    let provider_prepared = provider_preparation.map(|_| {
        let index = section.len();
        section.global(
            GlobalType {
                val_type: ValType::I32,
                mutable: true,
                shared: false,
            },
            &ConstExpr::i32_const(0),
        );
        index
    });

    let managed_state_reads =
        managed_state_reads::encode(&mut section, semantics, gc, wasm_ir, managed, reachability);
    let selected_provider_type = program
        .state
        .as_ref()
        .and_then(|state| state.provider_value)
        .map(|value| value_type(value, semantics));
    let selected_provider = selected_provider_type.map(|provider_type| {
        let selected = section.len();
        let mut val_type = gc.val_type(provider_type);
        if let ValType::Ref(reference) = &mut val_type {
            reference.nullable = true;
        }
        section.global(
            GlobalType {
                val_type,
                mutable: true,
                shared: false,
            },
            &default_const_expr(val_type),
        );
        selected
    });
    let nullable_state = ValType::Ref(RefType {
        nullable: true,
        heap_type: HeapType::Concrete(STATE_TYPE),
    });
    let current = section.len();
    section.global(
        GlobalType {
            val_type: nullable_state,
            mutable: true,
            shared: false,
        },
        &ConstExpr::ref_null(HeapType::Concrete(STATE_TYPE)),
    );
    let old = section.len();
    section.global(
        GlobalType {
            val_type: nullable_state,
            mutable: true,
            shared: false,
        },
        &ConstExpr::ref_null(HeapType::Concrete(STATE_TYPE)),
    );
    let attach_ready = section.len();
    section.global(
        GlobalType {
            val_type: ValType::I32,
            mutable: true,
            shared: false,
        },
        &ConstExpr::i32_const(0),
    );
    let state_ready = section.len();
    section.global(
        GlobalType {
            val_type: ValType::I32,
            mutable: true,
            shared: false,
        },
        &ConstExpr::i32_const(0),
    );
    let observed_timer_state = program
        .actions
        .iter()
        .any(|action| matches!(action.kind, ActionKind::OnStart | ActionKind::OnReset))
        .then(|| {
            let index = section.len();
            section.global(
                GlobalType {
                    val_type: ValType::I32,
                    mutable: true,
                    shared: false,
                },
                &ConstExpr::i32_const(-1),
            );
            index
        });
    let attempt_ready = wasm_ir.attempt_globals().next().is_some().then(|| {
        let index = section.len();
        section.global(
            GlobalType {
                val_type: ValType::I32,
                mutable: true,
                shared: false,
            },
            &ConstExpr::i32_const(0),
        );
        index
    });
    let debug_depth = section.len();
    section.global(
        GlobalType {
            val_type: ValType::I32,
            mutable: true,
            shared: false,
        },
        &ConstExpr::i32_const(0),
    );
    let future_poll_epoch = section.len();
    section.global(
        GlobalType {
            val_type: ValType::I64,
            mutable: true,
            shared: false,
        },
        &ConstExpr::i64_const(0),
    );
    let async_frame = section.len();
    section.global(
        GlobalType {
            val_type: ValType::Ref(RefType {
                nullable: true,
                heap_type: HeapType::Concrete(gc.async_frame_index()),
            }),
            mutable: true,
            shared: false,
        },
        &ConstExpr::ref_null(HeapType::Concrete(gc.async_frame_index())),
    );
    let while_attached_result = wasm_ir
        .body(wasm_ir::BodyOwner::Action(ActionKind::WhileAttached))
        .is_some_and(|body| matches!(body.abi, wasm_ir::BodyAbi::AsyncAction))
        .then(|| {
            let index = section.len();
            section.global(
                GlobalType {
                    val_type: ValType::I32,
                    mutable: true,
                    shared: false,
                },
                &ConstExpr::i32_const(1),
            );
            index
        });

    let mut variables = HashMap::new();
    let mut variable_types = HashMap::new();
    if let Some(state) = &program.state
        && let (Some(value), Some(global), Some(ty)) = (
            state.provider_value,
            selected_provider,
            selected_provider_type,
        )
    {
        variables.insert(value, global);
        variable_types.insert(value, ty);
    }
    for variable in &program.globals {
        let mut bindings = Vec::new();
        variable
            .binding
            .visit_bindings(&mut |binding| bindings.push(binding.id));
        let simple = variable.binding.simple_binding().is_some();
        for binding in bindings
            .into_iter()
            .filter(|binding| wasm_ir.contains_global(*binding))
        {
            let ty = value_type(binding, semantics);
            if !ty.has_runtime_value() {
                variables.insert(binding, u32::MAX);
                variable_types.insert(binding, ty);
                continue;
            }
            let index = section.len();
            let mut val_type = gc.val_type(ty);
            let runtime_initialized = variable
                .value
                .as_ref()
                .is_some_and(|value| !simple || !is_wasm_global_constant(value.id, wasm_ir));
            if (variable.value.is_none() || runtime_initialized)
                && let ValType::Ref(reference) = &mut val_type
            {
                // The source value is non-null whenever user code may observe it,
                // but Wasm storage needs a null placeholder before module-start or
                // lifecycle initialization has populated the value.
                reference.nullable = true;
            }
            let global_type = GlobalType {
                val_type,
                mutable: variable.mutable,
                shared: false,
            };
            if let Type::Option(option) = ty {
                section.global(
                    global_type,
                    &ConstExpr::ref_null(HeapType::Concrete(gc.index(Type::Option(option)))),
                );
            } else if let ValType::Ref(reference) = gc.val_type(ty) {
                section.global(global_type, &ConstExpr::ref_null(reference.heap_type));
            } else if simple
                && let Some(value) = &variable.value
                && is_wasm_global_constant(value.id, wasm_ir)
            {
                section.global(global_type, &constant(value.id, wasm_ir, ty));
            } else {
                section.global(global_type, &default_const_expr(gc.val_type(ty)));
            }
            variables.insert(binding, index);
            variable_types.insert(binding, ty);
        }
    }

    let mut settings = HashMap::new();
    for setting in &program.settings {
        let Some(ty) = semantics.value_type(setting.id) else {
            continue;
        };
        let ty = semantic_type(ty, semantics);
        let current = section.len();
        emit_setting_global(&mut section, ty, gc, wasm_ir.standard_library());
        let old = section.len();
        emit_setting_global(&mut section, ty, gc, wasm_ir.standard_library());
        settings.insert(setting.id, SettingStorage { current, old, ty });
    }

    GlobalPlan {
        section,
        runtime: RuntimeGlobals {
            process,
            process_name,
            process_pointer_size,
            provider_value,
            provider_attachment_frame,
            provider_preparation_value,
            provider_preparation_frame,
            provider_prepared,
            selected_provider,
            current,
            old,
            attach_ready,
            state_ready,
            observed_timer_state,
            attempt_ready,
            debug_depth,
            future_poll_epoch,
            async_frame,
            while_attached_result,
        },
        variables,
        variable_types,
        settings,
        managed_state_reads,
        provider_values,
        provider_attachment_frames,
    }
}

fn default_const_expr(ty: ValType) -> ConstExpr {
    match ty {
        ValType::I32 => ConstExpr::i32_const(0),
        ValType::I64 => ConstExpr::i64_const(0),
        ValType::F32 => ConstExpr::f32_const(0.0.into()),
        ValType::F64 => ConstExpr::f64_const(0.0.into()),
        ValType::Ref(reference) => ConstExpr::ref_null(reference.heap_type),
        ValType::V128 => unreachable!("SplitScript has no v128 source values"),
    }
}

fn emit_setting_global(
    section: &mut GlobalSection,
    ty: Type,
    gc: &GcLayout,
    standard_library: &StandardLibrary,
) {
    let global_type = GlobalType {
        val_type: gc.val_type(ty),
        mutable: true,
        shared: false,
    };
    match ty {
        Type::Bool => section.global(global_type, &ConstExpr::i32_const(0)),
        Type::Standard(StdlibTypeId::String) => section.global(
            global_type,
            &ConstExpr::ref_null(HeapType::Concrete(gc.standard_index(StdlibTypeId::String))),
        ),
        ty if ty.is_enum(standard_library) => section.global(
            global_type,
            &ConstExpr::ref_null(HeapType::Concrete(gc.index(ty))),
        ),
        _ => unreachable!(),
    };
}
