use std::collections::HashMap;

use wasm_encoder::{FunctionSection, HeapType, RefType, TypeSection, ValType};

use crate::{
    ast::{ActionKind, EnumDecl, ManagedClassId, ManagedFieldId, Program},
    capabilities::CapabilityAnalysis,
    equality::EqualityCapabilities,
    semantic::{ClosureInstance, FunctionInstance, FunctionValueInstance, SemanticModel},
    stdlib::{IntrinsicId, RuntimeRepresentation, StandardLibrary, StdlibTypeConstructorId},
    structural::{StructuralTypeId, StructuralTypes},
    types::{ResolvedArrayType, ResolvedOptionType, ResolvedResultType, ResolvedSetType, TypeKind},
};

use super::{
    ArrayFunctions, DisplayFunctions, EqualityFunctions, GcLayout, RuntimeHelperPlan, STATE_TYPE,
    Type, action_result_val_type,
    async_frame::LeafFutureInstance,
    dependencies::BackendDependencies,
    display_plan::DerivedDebugFunction,
    function_types::FunctionTypes,
    reachability, runtime_helper_registry, semantic_type, set_element_type,
    set_functions::{SetFunctionPlan, SetFunctions},
};

/// The complete, deterministic assignment of generated function signatures
/// and indices. Body generation consumes this plan but cannot mutate the
/// shared type or function index spaces.
pub(super) struct FunctionPlan<'a> {
    pub section: FunctionSection,
    pub runtime_helpers: RuntimeHelperPlan,
    pub equality: EqualityFunctions,
    pub managed_equality: EqualityFunctions,
    pub array_functions: ArrayFunctions,
    pub sets: SetFunctions,
    pub users: HashMap<FunctionInstance, UserFunctionPlan>,
    pub closures: HashMap<ClosureInstance, u32>,
    pub function_values: HashMap<FunctionValueInstance, u32>,
    pub closure_resumes: HashMap<ClosureInstance, u32>,
    pub leaf_futures: HashMap<LeafFutureInstance, u32>,
    pub displays: DisplayFunctions,
    pub managed_state_reads: HashMap<ManagedFieldId, u32>,
    pub managed_snapshots: HashMap<ManagedClassId, u32>,
    pub managed_freezers: HashMap<crate::types::TypeId, u32>,
    pub managed_decoders: HashMap<crate::types::TypeId, u32>,
    pub reads: Vec<u32>,
    pub transforms: Vec<Option<u32>>,
    pub actions: HashMap<ActionKind, u32>,
    pub start: u32,
    pub update: u32,
    pub arrays: &'a [ResolvedArrayType],
    /// Names assigned alongside final indices for debug builds or an opt-in
    /// compiler sidecar report. Reporting does not emit a Wasm name section.
    pub debug_names: Vec<(u32, String)>,
}

struct FunctionDeclarations<'a> {
    types: &'a mut TypeSection,
    signatures: &'a mut FunctionTypes,
    section: FunctionSection,
    imported_functions: u32,
    debug_names: Option<Vec<(u32, String)>>,
}

impl<'a> FunctionDeclarations<'a> {
    fn new(
        types: &'a mut TypeSection,
        signatures: &'a mut FunctionTypes,
        imported_functions: u32,
        profile: crate::BuildProfile,
    ) -> Self {
        Self {
            types,
            signatures,
            section: FunctionSection::new(),
            imported_functions,
            debug_names: (profile == crate::BuildProfile::Debug).then(Vec::new),
        }
    }

    fn declare(
        &mut self,
        name: impl FnOnce() -> String,
        params: Vec<ValType>,
        results: Vec<ValType>,
    ) -> u32 {
        let type_index = self.signatures.intern(self.types, params, results);
        self.declare_type(name, type_index)
    }

    fn declare_type(&mut self, name: impl FnOnce() -> String, type_index: u32) -> u32 {
        let function_index = self.imported_functions + self.section.len();
        self.section.function(type_index);
        if let Some(names) = &mut self.debug_names {
            names.push((function_index, name()));
        }
        function_index
    }
}

#[derive(Clone, Copy, Debug)]
pub(super) struct UserFunctionPlan {
    pub call: u32,
    pub resume: Option<u32>,
}

pub(super) struct Inputs<'a> {
    pub standard_library: &'a StandardLibrary,
    pub program: &'a Program,
    pub semantics: &'a SemanticModel,
    pub enums: &'a [EnumDecl],
    pub arrays: &'a [ResolvedArrayType],
    pub options: &'a [ResolvedOptionType],
    pub results: &'a [ResolvedResultType],
    pub sets: &'a [ResolvedSetType],
    pub equality: &'a EqualityCapabilities,
    pub capabilities: &'a CapabilityAnalysis,
    pub structural: &'a StructuralTypes,
    pub dependencies: &'a BackendDependencies,
    pub reachability: &'a reachability::Reachability,
    pub gc: &'a GcLayout,
    pub wasm_ir: &'a crate::wasm_ir::Program,
    pub async_frames: &'a super::async_frame::AsyncFrameLayouts,
    pub managed_state_reads: &'a super::managed_state_reads::ManagedStateReadCache,
    pub pointer_prefixes: &'a super::pointer_prefixes::PointerPrefixPlan,
}

pub(super) fn encode<'a>(
    types: &mut TypeSection,
    signatures: &mut FunctionTypes,
    imported_functions: u32,
    report_names: bool,
    inputs: Inputs<'a>,
) -> FunctionPlan<'a> {
    let Inputs {
        standard_library,
        program,
        semantics,
        enums,
        arrays,
        options,
        results,
        sets,
        equality: equality_capabilities,
        capabilities,
        structural,
        dependencies,
        reachability,
        gc,
        wasm_ir,
        async_frames,
        managed_state_reads,
        pointer_prefixes,
    } = inputs;
    let mut declarations =
        FunctionDeclarations::new(types, signatures, imported_functions, wasm_ir.profile());
    if report_names && declarations.debug_names.is_none() {
        declarations.debug_names = Some(Vec::new());
    }

    let mut helper_functions = HashMap::new();
    let ordered_helpers = dependencies.helpers().collect::<Vec<_>>();
    for helper in ordered_helpers.iter().copied() {
        let descriptor = runtime_helper_registry::descriptor(helper);
        let (params, results) =
            runtime_helper_registry::resolve_signature(descriptor.signature, arrays, semantics, gc);
        helper_functions.insert(
            helper,
            declarations.declare(
                || format!("__splitscript::runtime::{helper:?}"),
                params,
                results,
            ),
        );
    }

    let mut equality = EqualityFunctions {
        standard_library: standard_library.clone(),
        ..EqualityFunctions::default()
    };
    for structure in standard_library.all_types().iter().filter(|structure| {
        reachability.requires_standard_struct_equality(structure.id)
            && matches!(
                structure.representation,
                RuntimeRepresentation::GcStruct { .. }
            )
    }) {
        let struct_type = gc.val_type(Type::Standard(structure.id));
        let function = declarations.declare(
            || format!("__splitscript::equals::{}", structure.name),
            vec![struct_type, struct_type],
            vec![ValType::I32],
        );
        equality.standard_structs.insert(structure.id, function);
    }
    for (_, structure) in structural.structs() {
        let StructuralTypeId::Struct(struct_id) = structure.id else {
            unreachable!()
        };
        if reachability.requires_struct_equality(struct_id)
            && equality_capabilities.structure(struct_id).is_ok()
        {
            let struct_type = gc.val_type(Type::Struct(struct_id));
            let function = declarations.declare(
                || format!("__splitscript::equals::{}", structure.name),
                vec![struct_type, struct_type],
                vec![ValType::I32],
            );
            equality.structs.insert(struct_id, function);
        }
    }
    for (_, class) in structural.managed_classes() {
        let StructuralTypeId::ManagedClass(class_id) = class.id else {
            unreachable!()
        };
        if reachability.requires_managed_class_equality(class_id)
            && equality_capabilities.managed_class(class_id).is_ok()
        {
            let class_type = gc.val_type(Type::ManagedClass(class_id));
            let function = declarations.declare(
                || format!("__splitscript::equals::{}", class.name),
                vec![class_type, class_type],
                vec![ValType::I32],
            );
            equality.managed_classes.insert(class_id, function);
        }
    }
    for (_, enumeration) in structural.enums() {
        let StructuralTypeId::Enum(enum_id) = enumeration.id else {
            unreachable!()
        };
        if reachability.requires_enum_equality(enum_id)
            && equality_capabilities.enumeration(enum_id).is_ok()
        {
            let enum_type = gc.val_type(Type::Enum(enum_id));
            let function = declarations.declare(
                || format!("__splitscript::equals::{}", enumeration.name),
                vec![enum_type, enum_type],
                vec![ValType::I32],
            );
            equality.enums.insert(enum_id, function);
        }
    }
    for array in arrays {
        if reachability.requires_array_equality(array.id) {
            let array_type = gc.val_type(Type::Array(array.id));
            let function = declarations.declare(
                || format!("__splitscript::equals::array#{}", array.id.index()),
                vec![array_type, array_type],
                vec![ValType::I32],
            );
            equality.arrays.insert(array.id, function);
        }
    }
    for option in options {
        if reachability.requires_option_equality(option.id) {
            let option_type = gc.val_type(Type::Option(option.id));
            let function = declarations.declare(
                || format!("__splitscript::equals::option#{}", option.id.index()),
                vec![option_type, option_type],
                vec![ValType::I32],
            );
            equality.options.insert(option.id, function);
        }
    }
    for result in results {
        if reachability.requires_result_equality(result.id) {
            let result_type = gc.val_type(Type::Result(result.id));
            let function = declarations.declare(
                || format!("__splitscript::equals::result#{}", result.id.index()),
                vec![result_type, result_type],
                vec![ValType::I32],
            );
            equality.results.insert(result.id, function);
        }
    }

    for (_, kind) in semantics.types().iter() {
        let (layout, ty, name, functions) = match kind {
            TypeKind::Set { layout, .. } if reachability.requires_set_equality(*layout) => {
                (*layout, Type::Set(*layout), "set", &mut equality.sets)
            }
            TypeKind::Application {
                layout,
                constructor: StdlibTypeConstructorId::Map,
                ..
            } if reachability.requires_map_equality(*layout) => (
                *layout,
                Type::Application(*layout),
                "map",
                &mut equality.maps,
            ),
            _ => continue,
        };
        let value_type = gc.val_type(ty);
        let function = declarations.declare(
            || format!("__splitscript::equals::{name}#{}", layout.index()),
            vec![value_type, value_type],
            vec![ValType::I32],
        );
        functions.insert(layout, function);
    }

    let mut managed_equality = EqualityFunctions {
        standard_library: standard_library.clone(),
        ..Default::default()
    };
    if reachability.managed_equality().next().is_some() {
        managed_equality.budget = Some(super::equality_plan::ComparisonBudget {
            charge: helper_functions
                [&crate::intrinsic_registry::RuntimeHelperId::ChargeManagedWork],
            context_type: gc.standard_index(crate::stdlib::StdlibTypeId::ManagedReadContext),
        });
    }
    for ty in reachability.managed_equality() {
        let value = semantic_type(ty, semantics);
        let needs_function = match value {
            Type::Standard(crate::stdlib::StdlibTypeId::String) => true,
            Type::Standard(standard) => matches!(
                standard_library.type_decl(standard).representation,
                RuntimeRepresentation::GcStruct { .. }
            ),
            Type::ManagedClass(_)
            | Type::Struct(_)
            | Type::Enum(_)
            | Type::Array(_)
            | Type::Option(_)
            | Type::Result(_)
            | Type::Set(_)
            | Type::Application(_) => true,
            _ => false,
        };
        if !needs_function {
            continue;
        }
        let index = declarations.declare(
            || format!("__splitscript::managed_equals::type#{}", ty.index()),
            vec![
                gc.val_type(value),
                gc.val_type(value),
                gc.val_type(Type::Standard(
                    crate::stdlib::StdlibTypeId::ManagedReadContext,
                )),
            ],
            vec![ValType::I32],
        );
        match value {
            Type::Standard(crate::stdlib::StdlibTypeId::String) => {
                managed_equality.string = Some(index);
            }
            Type::Standard(id) => {
                managed_equality.standard_structs.insert(id, index);
            }
            Type::ManagedClass(id) => {
                managed_equality.managed_classes.insert(id, index);
            }
            Type::Struct(id) => {
                managed_equality.structs.insert(id, index);
            }
            Type::Enum(id) => {
                managed_equality.enums.insert(id, index);
            }
            Type::Array(id) => {
                managed_equality.arrays.insert(id, index);
            }
            Type::Option(id) => {
                managed_equality.options.insert(id, index);
            }
            Type::Result(id) => {
                managed_equality.results.insert(id, index);
            }
            Type::Set(id) => {
                managed_equality.sets.insert(id, index);
            }
            Type::Application(id) => {
                managed_equality.maps.insert(id, index);
            }
            _ => unreachable!(),
        }
    }

    let mut displays = DisplayFunctions {
        custom: reachability
            .display_functions()
            .map(|(ty, function)| (ty, function.clone()))
            .collect(),
        custom_debug: reachability
            .debug_functions()
            .map(|(ty, function)| (ty, function.clone()))
            .collect(),
        ..DisplayFunctions::default()
    };
    for ty in reachability.derived_debugs() {
        let source_type = super::semantic_type(ty, semantics);
        displays.derived.insert(
            ty,
            DerivedDebugFunction {
                function: declarations.declare(
                    || {
                        let name = structural
                            .get(ty)
                            .map_or_else(|| format!("type#{}", ty.index()), |ty| ty.name.clone());
                        format!("__splitscript::debug::{name}")
                    },
                    vec![gc.val_type(source_type)],
                    vec![gc.val_type(Type::Standard(crate::stdlib::StdlibTypeId::String))],
                ),
                kind: capabilities
                    .derived_debug_kind(ty, semantics)
                    .expect("reachable derived Debug types have a formatting policy"),
            },
        );
    }

    let runtime_helpers = RuntimeHelperPlan {
        ordered: ordered_helpers,
        functions: helper_functions,
    };

    let mut array_functions = ArrayFunctions::default();
    array_functions.freezing = reachability.managed_decoders().next().is_some();
    for array in arrays.iter().filter(|array| {
        reachability.requires_array_set(array.id)
            || reachability.requires_array_push(array.id)
            || reachability.requires_array_remove_at(array.id)
            || reachability.requires_array_clear(array.id)
    }) {
        let array_type = gc.val_type(Type::Array(array.id));
        if reachability.requires_array_set(array.id) {
            let element = gc.val_type(super::try_array_element_type(array.id, semantics).unwrap());
            array_functions.insert_set(
                array.id,
                declarations.declare(
                    || format!("__splitscript::array#{}::set", array.id.index()),
                    vec![array_type, ValType::I32, element],
                    vec![],
                ),
            );
        }
        if reachability.requires_array_push(array.id) {
            let element_type = gc.val_type(
                super::try_array_element_type(array.id, semantics)
                    .expect("reachable arrays have lowerable element types"),
            );
            array_functions.insert_push(
                array.id,
                declarations.declare(
                    || format!("__splitscript::array#{}::push", array.id.index()),
                    vec![array_type, element_type],
                    vec![],
                ),
            );
        }
        if reachability.requires_array_remove_at(array.id) {
            array_functions.insert_remove_at(
                array.id,
                declarations.declare(
                    || format!("__splitscript::array#{}::removeAt", array.id.index()),
                    vec![array_type, ValType::I32],
                    vec![],
                ),
            );
        }
        if reachability.requires_array_clear(array.id) {
            array_functions.insert_clear(
                array.id,
                declarations.declare(
                    || format!("__splitscript::array#{}::clear", array.id.index()),
                    vec![array_type],
                    vec![],
                ),
            );
        }
    }

    let mut set_functions = SetFunctions::default();
    for set in sets
        .iter()
        .filter(|set| reachability.contains_set_type(set.id))
    {
        let set_type = gc.val_type(Type::Set(set.id));
        let element_type = gc.val_type(set_element_type(set.id, semantics));
        let mut declare_operation = |intrinsic, name, params, results| {
            reachability
                .requires_set_operation(set.id, intrinsic)
                .then(|| {
                    declarations.declare(
                        || super::debug_artifacts::set_function_name(set.id, name),
                        params,
                        results,
                    )
                })
        };
        set_functions.insert(
            set.id,
            SetFunctionPlan {
                new: declare_operation(IntrinsicId::SetNew, "new", vec![], vec![set_type]),
                length: declare_operation(
                    IntrinsicId::SetLength,
                    "length",
                    vec![set_type],
                    vec![ValType::I32],
                ),
                contains: declare_operation(
                    IntrinsicId::SetContains,
                    "contains",
                    vec![set_type, element_type],
                    vec![ValType::I32],
                ),
                insert: declare_operation(
                    IntrinsicId::SetInsert,
                    "insert",
                    vec![set_type, element_type],
                    vec![ValType::I32],
                ),
                remove: declare_operation(
                    IntrinsicId::SetRemove,
                    "remove",
                    vec![set_type, element_type],
                    vec![ValType::I32],
                ),
                clear: declare_operation(IntrinsicId::SetClear, "clear", vec![set_type], vec![]),
            },
        );
    }

    let mut managed_state_read_functions = HashMap::new();
    for storage in managed_state_reads.entries() {
        managed_state_read_functions.insert(
            storage.field,
            declarations.declare(
                || {
                    format!(
                        "__splitscript::managed::class#{}::field#{}::read",
                        storage.class.index(),
                        storage.field.index(),
                    )
                },
                vec![],
                vec![gc.val_type(Type::Result(storage.result))],
            ),
        );
    }

    let mut managed_snapshot_functions = HashMap::new();
    for class in reachability.managed_snapshots() {
        let snapshot = semantics.types().id_for_managed_class(class);
        let result = semantics
            .types()
            .iter()
            .find_map(|(_, kind)| match kind {
                crate::types::TypeKind::Result { layout, value } if *value == snapshot => {
                    Some(*layout)
                }
                _ => None,
            })
            .expect("reachable managed snapshot calls have Result layouts");
        let mut parameters = vec![ValType::I64];
        if runtime_helpers
            .optional_function(crate::intrinsic_registry::RuntimeHelperId::EnterManagedObject)
            .is_some()
        {
            parameters.push(gc.val_type(Type::Standard(
                crate::stdlib::StdlibTypeId::ManagedReadContext,
            )));
        }
        managed_snapshot_functions.insert(
            class,
            declarations.declare(
                || {
                    let class_name = &program
                        .managed_class(class)
                        .expect("reachable managed snapshot classes have declarations")
                        .name;
                    format!("__splitscript::managed::{class_name}::snapshot")
                },
                parameters,
                vec![gc.val_type(Type::Result(result))],
            ),
        );
    }

    let mut managed_decoder_functions = HashMap::new();
    for value in reachability.managed_decoders() {
        let output = capabilities.managed_decoder(value).unwrap().output;
        let result = semantics
            .types()
            .iter()
            .find_map(|(_, kind)| match kind {
                crate::types::TypeKind::Result {
                    value: candidate,
                    layout,
                } if *candidate == output => Some(*layout),
                _ => None,
            })
            .expect("managed child reads have Result layouts");
        managed_decoder_functions.insert(
            value,
            declarations.declare(
                || format!("__splitscript::managed::decode::{value:?}"),
                vec![
                    ValType::I64,
                    ValType::I64,
                    ValType::I32,
                    gc.val_type(Type::Standard(
                        crate::stdlib::StdlibTypeId::ManagedReadContext,
                    )),
                    ValType::I32,
                ],
                vec![gc.val_type(Type::Result(result))],
            ),
        );
    }

    let mut managed_freezers = HashMap::new();
    for value in super::managed_freezers::required(reachability, capabilities, semantics) {
        managed_freezers.insert(
            value,
            declarations.declare(
                || format!("__splitscript::managed::freeze::{value:?}"),
                vec![gc.val_type(semantic_type(value, semantics))],
                vec![],
            ),
        );
    }
    let functions_by_id = program
        .functions
        .iter()
        .map(|function| (function.id, function))
        .collect::<HashMap<_, _>>();
    let mut users = HashMap::new();
    for instance in reachability.functions() {
        let function = functions_by_id
            .get(&instance.function)
            .expect("reachable function instances have source declarations");
        let result = semantic_type(
            semantics.specialize_type(
                instance,
                semantics
                    .function_completion(instance.function)
                    .expect("checked functions have result types"),
            ),
            semantics,
        );
        let params = function
            .params
            .iter()
            .filter_map(|parameter| {
                let ty = semantic_type(
                    semantics.specialize_type(
                        instance,
                        semantics
                            .value_type(parameter.id)
                            .expect("checked parameters have types"),
                    ),
                    semantics,
                );
                ty.has_runtime_value().then(|| gc.val_type(ty))
            })
            .collect::<Vec<_>>();
        let body = wasm_ir
            .body(crate::wasm_ir::BodyOwner::Function(instance.clone()))
            .expect("reachable functions have Wasm IR bodies");
        let source_name = declarations.debug_names.as_ref().map(|_| {
            super::debug_artifacts::user_function_name(
                &function.name,
                instance,
                program,
                semantics,
                standard_library,
                enums,
            )
        });
        let plan = if matches!(
            body.abi,
            crate::wasm_ir::BodyAbi::AsyncFunction(_) | crate::wasm_ir::BodyAbi::Generator(_)
        ) {
            let frame = ValType::Ref(RefType {
                nullable: false,
                heap_type: HeapType::Concrete(gc.function_frame_index(instance)),
            });
            UserFunctionPlan {
                call: declarations.declare(
                    || {
                        format!(
                            "{}::init",
                            source_name
                                .as_deref()
                                .expect("debug builds have source names")
                        )
                    },
                    params,
                    vec![frame],
                ),
                resume: match body.abi {
                    crate::wasm_ir::BodyAbi::AsyncFunction(_) => Some(declarations.declare(
                        || {
                            format!(
                                "{}::poll",
                                source_name
                                    .as_deref()
                                    .expect("debug builds have source names")
                            )
                        },
                        vec![frame],
                        vec![ValType::I32],
                    )),
                    crate::wasm_ir::BodyAbi::Generator(generator) => {
                        let item = semantics.specialize_type(instance, generator.item);
                        iterator_step_type(item, semantics).map(|step| {
                            declarations.declare(
                                || {
                                    format!(
                                        "{}::next",
                                        source_name
                                            .as_deref()
                                            .expect("debug builds have source names")
                                    )
                                },
                                vec![frame],
                                vec![gc.val_type(step)],
                            )
                        })
                    }
                    crate::wasm_ir::BodyAbi::Direct | crate::wasm_ir::BodyAbi::AsyncAction => {
                        unreachable!("continuation function planning has a continuation ABI")
                    }
                },
            }
        } else {
            UserFunctionPlan {
                call: declarations.declare(
                    || source_name.expect("debug builds have source names"),
                    params,
                    result
                        .has_runtime_value()
                        .then(|| gc.val_type(result))
                        .into_iter()
                        .collect(),
                ),
                resume: None,
            }
        };
        users.insert(instance.clone(), plan);
    }

    let mut leaf_futures = HashMap::new();
    for (instance, _) in async_frames.leaves() {
        let frame = ValType::Ref(RefType {
            nullable: false,
            heap_type: HeapType::Concrete(gc.leaf_frame_index(instance)),
        });
        leaf_futures.insert(
            instance.clone(),
            declarations.declare(
                || {
                    format!(
                        "__splitscript::future::expr{}::poll",
                        instance.expression.index()
                    )
                },
                vec![frame],
                vec![ValType::I32],
            ),
        );
    }

    let state_ref = ValType::Ref(RefType {
        nullable: false,
        heap_type: HeapType::Concrete(STATE_TYPE),
    });
    let mut reads = Vec::with_capacity(
        program
            .state
            .as_ref()
            .map_or(0, |state| state.all_fields().count()),
    );
    let mut transforms = Vec::with_capacity(reads.capacity());
    if let Some(state) = &program.state {
        for field in state.all_fields() {
            let poll_result = semantic_type(
                semantics
                    .state_poll_result(field.id)
                    .expect("checked state fields have poll-result types"),
                semantics,
            );
            let mut parameters = vec![ValType::I64];
            let has_dependencies = !semantics.state_dependencies(field.id).is_empty();
            if has_dependencies {
                parameters.push(state_ref);
            }
            if pointer_prefixes.field(field.id).is_some() {
                parameters.extend([ValType::I64, ValType::I32]);
            }
            reads.push(declarations.declare(
                || format!("state::{}::read", field.name),
                parameters,
                vec![gc.val_type(poll_result)],
            ));
            transforms.push(field.transform.as_ref().map(|_| {
                let ty = semantic_type(
                    semantics
                        .value_type(field.id)
                        .expect("checked state fields have types"),
                    semantics,
                );
                let value = gc.val_type(ty);
                declarations.declare(
                    || format!("state::{}::transform", field.name),
                    if has_dependencies {
                        vec![value, state_ref]
                    } else {
                        vec![value]
                    },
                    vec![gc.val_type(poll_result)],
                )
            }));
        }
    }

    let mut actions = HashMap::new();
    for action in &program.actions {
        let (params, results) = match action.kind {
            ActionKind::Setup
            | ActionKind::SelectProcess
            | ActionKind::OnDetach
            | ActionKind::OnStart
            | ActionKind::OnReset => {
                let results = (action.kind == ActionKind::SelectProcess)
                    .then(|| action_result_val_type(action.kind, semantics, gc))
                    .into_iter()
                    .collect();
                (vec![], results)
            }
            ActionKind::OnAttach => (vec![ValType::I64], vec![ValType::I32]),
            action => (
                vec![state_ref, state_ref],
                (!matches!(
                    action,
                    ActionKind::OnDetach
                        | ActionKind::OnStateReady
                        | ActionKind::OnStart
                        | ActionKind::OnReset
                ))
                .then(|| action_result_val_type(action, semantics, gc))
                .into_iter()
                .collect(),
            ),
        };
        actions.insert(
            action.kind,
            declarations.declare(|| action.kind.name().to_owned(), params, results),
        );
    }

    let start = declarations.declare(|| "_start".to_owned(), vec![], vec![]);
    let update = declarations.declare(|| "update".to_owned(), vec![], vec![]);
    let mut closure_resumes = HashMap::new();
    for (instance, _) in async_frames.closures() {
        let frame = ValType::Ref(RefType {
            nullable: false,
            heap_type: HeapType::Concrete(gc.closure_frame_index(instance)),
        });
        let closure = wasm_ir
            .closure(instance.expression)
            .expect("continuation-backed closures have Wasm IR bodies");
        let result = match closure.abi {
            crate::wasm_ir::BodyAbi::AsyncFunction(_) => Some(vec![ValType::I32]),
            crate::wasm_ir::BodyAbi::Generator(generator) => iterator_step_type(
                instance.owner.as_ref().map_or(generator.item, |owner| {
                    semantics.specialize_type(owner, generator.item)
                }),
                semantics,
            )
            .map(|step| vec![gc.val_type(step)]),
            crate::wasm_ir::BodyAbi::Direct | crate::wasm_ir::BodyAbi::AsyncAction => {
                unreachable!("planned closure frames have continuation ABIs")
            }
        };
        let Some(result) = result else {
            // A constructed but never consumed generator needs its frame and
            // initializer, but no `next` entry point or IteratorStep layout.
            continue;
        };
        closure_resumes.insert(
            instance.clone(),
            declarations.declare(
                || {
                    format!(
                        "__splitscript::closure::expr{}::{}",
                        instance.expression.index(),
                        match closure.abi {
                            crate::wasm_ir::BodyAbi::AsyncFunction(_) => "poll",
                            crate::wasm_ir::BodyAbi::Generator(_) => "next",
                            crate::wasm_ir::BodyAbi::Direct
                            | crate::wasm_ir::BodyAbi::AsyncAction => unreachable!(),
                        }
                    )
                },
                vec![frame],
                result,
            ),
        );
    }
    let mut closures = HashMap::new();
    for instance in reachability.closure_instances() {
        let closure = wasm_ir
            .closure(instance.expression)
            .expect("reachable closure instances have bodies");
        let expression = wasm_ir
            .expression(closure.expression)
            .expect("closure expressions belong to Wasm IR");
        let ty = instance.owner.as_ref().map_or(expression.ty, |owner| {
            semantics.specialize_type(owner, expression.ty)
        });
        let crate::types::TypeKind::Callable { layout, .. } = semantics.types().kind(ty) else {
            unreachable!("checked closure expressions have callable types")
        };
        let function_index = declarations.declare_type(
            || format!("__splitscript::closure::expr{}", closure.expression.index()),
            gc.callable_function_index(*layout),
        );
        closures.insert(instance.clone(), function_index);
    }
    let mut function_values = HashMap::new();
    for instance in reachability.function_value_instances() {
        let crate::types::TypeKind::Callable { layout, .. } = semantics.types().kind(instance.ty)
        else {
            unreachable!("function-value adapters have callable layouts")
        };
        let function_index = declarations.declare_type(
            || {
                let function = functions_by_id
                    .get(&instance.function.function)
                    .expect("reachable function values have source declarations");
                format!("__splitscript::function-value::{}", function.name)
            },
            gc.callable_function_index(*layout),
        );
        function_values.insert(instance.clone(), function_index);
    }

    FunctionPlan {
        section: declarations.section,
        runtime_helpers,
        equality,
        managed_equality,
        array_functions,
        sets: set_functions,
        users,
        closures,
        function_values,
        closure_resumes,
        leaf_futures,
        displays,
        managed_state_reads: managed_state_read_functions,
        managed_snapshots: managed_snapshot_functions,
        managed_decoders: managed_decoder_functions,
        managed_freezers,
        reads,
        transforms,
        actions,
        start,
        update,
        arrays,
        debug_names: declarations.debug_names.unwrap_or_default(),
    }
}

pub(super) fn iterator_step_type(
    item: crate::types::TypeId,
    semantics: &SemanticModel,
) -> Option<Type> {
    semantics.types().iter().find_map(|(_, kind)| match kind {
        crate::types::TypeKind::Application {
            layout,
            constructor: StdlibTypeConstructorId::IteratorStep,
            arguments,
        } if arguments.as_slice() == [item] => Some(Type::Application(*layout)),
        _ => None,
    })
}

#[cfg(test)]
mod tests {
    use std::cell::Cell;

    use super::FunctionDeclarations;
    use crate::{
        BuildProfile,
        codegen::{code_bodies::CodeBodies, function_types::FunctionTypes},
    };
    use wasm_encoder::{Encode, Function, Instruction, TypeSection};

    #[test]
    fn release_skips_name_builders_without_changing_function_indices_or_counts() {
        let mut encoded_sections = Vec::new();
        for profile in [BuildProfile::Debug, BuildProfile::Release] {
            let mut types = TypeSection::new();
            let mut signatures = FunctionTypes::new(0);
            let mut declarations =
                FunctionDeclarations::new(&mut types, &mut signatures, 7, profile);
            let names_built = Cell::new(0);
            for ordinal in 0..128 {
                assert_eq!(
                    declarations.declare(
                        || {
                            names_built.set(names_built.get() + 1);
                            format!("function#{ordinal}")
                        },
                        vec![],
                        vec![]
                    ),
                    7 + ordinal
                );
            }
            assert_eq!(
                declarations.declare_type(
                    || {
                        names_built.set(names_built.get() + 1);
                        "adapter".to_owned()
                    },
                    0
                ),
                135
            );
            assert_eq!(declarations.section.len(), 129);
            match profile {
                BuildProfile::Debug => {
                    let names = declarations.debug_names.as_ref().unwrap();
                    assert_eq!(names_built.get(), 129);
                    assert_eq!(names.len(), 129);
                    assert_eq!(names[0], (7, "function#0".to_owned()));
                    assert_eq!(names[128], (135, "adapter".to_owned()));
                }
                BuildProfile::Release => {
                    assert_eq!(names_built.get(), 0);
                    assert!(declarations.debug_names.is_none());
                }
            }
            let mut bodies = CodeBodies::new(7, declarations.section.len() as usize, None);
            for _ in 0..129 {
                let mut function = Function::new([]);
                function.instruction(&Instruction::End);
                bodies.push(&function);
            }
            assert_eq!(bodies.finish().len(), 129);
            let mut encoded = Vec::new();
            declarations.section.encode(&mut encoded);
            encoded_sections.push(encoded);
        }
        assert_eq!(encoded_sections[0], encoded_sections[1]);
    }
}
