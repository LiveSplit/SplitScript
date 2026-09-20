//! Repository-wide verification orchestration.

#[path = "xtask/documentation_site.rs"]
mod documentation_site;

use std::{
    collections::{BTreeMap, BTreeSet},
    env, fs,
    path::{Path, PathBuf},
    process::{Command, ExitCode},
};

#[derive(Clone, Copy)]
struct RuntimeFixture {
    source: &'static str,
    output: &'static str,
    profile: &'static str,
    harness: &'static str,
    extra_arguments: &'static [&'static str],
}

#[derive(Clone, Copy)]
struct CompileFixture {
    source: &'static str,
    output: &'static str,
    profile: &'static str,
}

const COMPILE_FIXTURES: &[CompileFixture] = &[
    CompileFixture {
        source: "examples/lunistice.split",
        output: "lunistice.release.wasm",
        profile: "release",
    },
    CompileFixture {
        source: "tests/debug_profile.split",
        output: "debug_profile.debug.wasm",
        profile: "debug",
    },
    CompileFixture {
        source: "tests/debug_profile.split",
        output: "debug_profile.release.wasm",
        profile: "release",
    },
];

const RUNTIME_FIXTURES: &[RuntimeFixture] = &[
    RuntimeFixture {
        source: "tests/managed_metadata_budget.split",
        output: "managed_metadata_budget_debug.wasm",
        profile: "debug",
        harness: "tests/managed_metadata_budget_runtime.mjs",
        extra_arguments: &[],
    },
    RuntimeFixture {
        source: "tests/managed_metadata_budget.split",
        output: "managed_metadata_budget_release.wasm",
        profile: "release",
        harness: "tests/managed_metadata_budget_runtime.mjs",
        extra_arguments: &[],
    },
    RuntimeFixture {
        source: "tests/process_selection.split",
        output: "process_selection.wasm",
        profile: "release",
        harness: "tests/process_selection_runtime.mjs",
        extra_arguments: &[],
    },
    RuntimeFixture {
        source: "examples/neon_white.split",
        output: "neon_white.wasm",
        profile: "release",
        harness: "tests/neon_white_runtime.mjs",
        extra_arguments: &[],
    },
    RuntimeFixture {
        source: "examples/axiom_verge.split",
        output: "axiom_verge.wasm",
        profile: "release",
        harness: "tests/axiom_verge_runtime.mjs",
        extra_arguments: &[],
    },
    RuntimeFixture {
        source: "examples/a_hat_in_time.split",
        output: "a_hat_in_time.wasm",
        profile: "release",
        harness: "tests/a_hat_in_time_runtime.mjs",
        extra_arguments: &[],
    },
    RuntimeFixture {
        source: "examples/akibas_trip.split",
        output: "akibas_trip.wasm",
        profile: "release",
        harness: "tests/akibas_trip_runtime.mjs",
        extra_arguments: &[],
    },
    RuntimeFixture {
        source: "examples/hello_lunistice.split",
        output: "hello_lunistice.wasm",
        profile: "release",
        harness: "tests/runtime_smoke.mjs",
        extra_arguments: &[],
    },
    RuntimeFixture {
        source: "examples/abzu.split",
        output: "abzu.wasm",
        profile: "release",
        harness: "tests/abzu_runtime.mjs",
        extra_arguments: &[],
    },
    RuntimeFixture {
        source: "examples/abzu.split",
        output: "abzu.wasm",
        profile: "release",
        harness: "tests/abzu_runtime.mjs",
        extra_arguments: &["epic"],
    },
    RuntimeFixture {
        source: "examples/abzu.split",
        output: "abzu.wasm",
        profile: "release",
        harness: "tests/abzu_runtime.mjs",
        extra_arguments: &["unknown"],
    },
    RuntimeFixture {
        source: "examples/state_layouts.split",
        output: "state_layouts.wasm",
        profile: "release",
        harness: "tests/state_layouts_runtime.mjs",
        extra_arguments: &[],
    },
    RuntimeFixture {
        source: "examples/state_layouts.split",
        output: "state_layouts.wasm",
        profile: "release",
        harness: "tests/state_layouts_runtime.mjs",
        extra_arguments: &["gog"],
    },
    RuntimeFixture {
        source: "examples/state_layouts.split",
        output: "state_layouts.wasm",
        profile: "release",
        harness: "tests/state_layouts_runtime.mjs",
        extra_arguments: &["unknown"],
    },
    RuntimeFixture {
        source: "examples/ronin.split",
        output: "ronin.wasm",
        profile: "release",
        harness: "tests/ronin_runtime.mjs",
        extra_arguments: &["v8"],
    },
    RuntimeFixture {
        source: "examples/ronin.split",
        output: "ronin.wasm",
        profile: "release",
        harness: "tests/ronin_runtime.mjs",
        extra_arguments: &["v9"],
    },
    RuntimeFixture {
        source: "examples/ronin.split",
        output: "ronin.wasm",
        profile: "release",
        harness: "tests/ronin_runtime.mjs",
        extra_arguments: &["unknown"],
    },
    RuntimeFixture {
        source: "examples/martha_is_dead.split",
        output: "martha_is_dead.wasm",
        profile: "release",
        harness: "tests/martha_is_dead_runtime.mjs",
        extra_arguments: &[],
    },
    RuntimeFixture {
        source: "examples/martha_is_dead.split",
        output: "martha_is_dead.wasm",
        profile: "release",
        harness: "tests/martha_is_dead_runtime.mjs",
        extra_arguments: &["v427"],
    },
    RuntimeFixture {
        source: "examples/martha_is_dead.split",
        output: "martha_is_dead.wasm",
        profile: "release",
        harness: "tests/martha_is_dead_runtime.mjs",
        extra_arguments: &["v1040101"],
    },
    RuntimeFixture {
        source: "examples/martha_is_dead.split",
        output: "martha_is_dead.wasm",
        profile: "release",
        harness: "tests/martha_is_dead_runtime.mjs",
        extra_arguments: &["unknown"],
    },
    RuntimeFixture {
        source: "examples/a_plague_tale_innocence.split",
        output: "a_plague_tale_innocence.wasm",
        profile: "release",
        harness: "tests/a_plague_tale_innocence_runtime.mjs",
        extra_arguments: &[],
    },
    RuntimeFixture {
        source: "examples/a_plague_tale_innocence.split",
        output: "a_plague_tale_innocence.wasm",
        profile: "release",
        harness: "tests/a_plague_tale_innocence_runtime.mjs",
        extra_arguments: &["epic"],
    },
    RuntimeFixture {
        source: "examples/a_plague_tale_innocence.split",
        output: "a_plague_tale_innocence.wasm",
        profile: "release",
        harness: "tests/a_plague_tale_innocence_runtime.mjs",
        extra_arguments: &["xbox"],
    },
    RuntimeFixture {
        source: "examples/a_plague_tale_innocence.split",
        output: "a_plague_tale_innocence.wasm",
        profile: "release",
        harness: "tests/a_plague_tale_innocence_runtime.mjs",
        extra_arguments: &["unknown"],
    },
    RuntimeFixture {
        source: "examples/arietta_of_spirits.split",
        output: "arietta_of_spirits.wasm",
        profile: "release",
        harness: "tests/arietta_of_spirits_runtime.mjs",
        extra_arguments: &[],
    },
    RuntimeFixture {
        source: "examples/operation_matriarchy.split",
        output: "operation_matriarchy.wasm",
        profile: "release",
        harness: "tests/operation_matriarchy_runtime.mjs",
        extra_arguments: &[],
    },
    RuntimeFixture {
        source: "examples/borderlands.split",
        output: "borderlands.wasm",
        profile: "release",
        harness: "tests/borderlands_runtime.mjs",
        extra_arguments: &[],
    },
    RuntimeFixture {
        source: "examples/borderlands.split",
        output: "borderlands.wasm",
        profile: "release",
        harness: "tests/borderlands_runtime.mjs",
        extra_arguments: &["v150"],
    },
    RuntimeFixture {
        source: "examples/borderlands.split",
        output: "borderlands.wasm",
        profile: "release",
        harness: "tests/borderlands_runtime.mjs",
        extra_arguments: &["unknown"],
    },
    RuntimeFixture {
        source: "examples/alan_wake.split",
        output: "alan_wake.wasm",
        profile: "release",
        harness: "tests/alan_wake_runtime.mjs",
        extra_arguments: &[],
    },
    RuntimeFixture {
        source: "examples/alan_wake.split",
        output: "alan_wake.wasm",
        profile: "release",
        harness: "tests/alan_wake_runtime.mjs",
        extra_arguments: &["gog"],
    },
    RuntimeFixture {
        source: "examples/alan_wake.split",
        output: "alan_wake.wasm",
        profile: "release",
        harness: "tests/alan_wake_runtime.mjs",
        extra_arguments: &["epic"],
    },
    RuntimeFixture {
        source: "examples/alan_wake.split",
        output: "alan_wake.wasm",
        profile: "release",
        harness: "tests/alan_wake_runtime.mjs",
        extra_arguments: &["unknown"],
    },
    RuntimeFixture {
        source: "examples/lunistice.split",
        output: "lunistice.debug.wasm",
        profile: "debug",
        harness: "tests/lunistice_runtime.mjs",
        extra_arguments: &[],
    },
    RuntimeFixture {
        source: "examples/lunistice.split",
        output: "lunistice.debug.wasm",
        profile: "debug",
        harness: "tests/lunistice_runtime.mjs",
        extra_arguments: &["--dlc"],
    },
    RuntimeFixture {
        source: "examples/lunistice.split",
        output: "lunistice.debug.wasm",
        profile: "debug",
        harness: "tests/lunistice_runtime.mjs",
        extra_arguments: &["--ambiguous-class"],
    },
    RuntimeFixture {
        source: "examples/lunistice.split",
        output: "lunistice.debug.wasm",
        profile: "debug",
        harness: "tests/lunistice_runtime.mjs",
        extra_arguments: &["--missing-class"],
    },
    RuntimeFixture {
        source: "examples/lunistice.split",
        output: "lunistice.debug.wasm",
        profile: "debug",
        harness: "tests/lunistice_runtime.mjs",
        extra_arguments: &["--ambiguous-field"],
    },
    RuntimeFixture {
        source: "examples/lunistice.split",
        output: "lunistice.debug.wasm",
        profile: "debug",
        harness: "tests/lunistice_runtime.mjs",
        extra_arguments: &["--missing-field"],
    },
    RuntimeFixture {
        source: "examples/lunistice.split",
        output: "lunistice.debug.wasm",
        profile: "debug",
        harness: "tests/lunistice_runtime.mjs",
        extra_arguments: &["--transient-binding-read"],
    },
    RuntimeFixture {
        source: "examples/lunistice.split",
        output: "lunistice.debug.wasm",
        profile: "debug",
        harness: "tests/lunistice_runtime.mjs",
        extra_arguments: &["--mixed-layout"],
    },
    RuntimeFixture {
        source: "examples/lunistice.split",
        output: "lunistice.debug.wasm",
        profile: "debug",
        harness: "tests/lunistice_runtime.mjs",
        extra_arguments: &["--inherited-field"],
    },
    RuntimeFixture {
        source: "examples/lso_desktop_settings.split",
        output: "settings.wasm",
        profile: "release",
        harness: "tests/settings_runtime.mjs",
        extra_arguments: &[],
    },
    RuntimeFixture {
        source: "examples/minish_cap.split",
        output: "minish_cap.wasm",
        profile: "release",
        harness: "tests/minish_cap_runtime.mjs",
        extra_arguments: &[],
    },
    RuntimeFixture {
        source: "examples/minish_cap.split",
        output: "minish_cap.wasm",
        profile: "release",
        harness: "tests/minish_cap_runtime.mjs",
        extra_arguments: &["vba"],
    },
    RuntimeFixture {
        source: "examples/aquanox.split",
        output: "aquanox.wasm",
        profile: "release",
        harness: "tests/aquanox_runtime.mjs",
        extra_arguments: &[],
    },
    RuntimeFixture {
        source: "examples/cancellation.split",
        output: "cancellation.wasm",
        profile: "release",
        harness: "tests/cancellation_runtime.mjs",
        extra_arguments: &[],
    },
    RuntimeFixture {
        source: "tests/module_path.split",
        output: "module_path.wasm",
        profile: "release",
        harness: "tests/module_path_runtime.mjs",
        extra_arguments: &[],
    },
    RuntimeFixture {
        source: "tests/pe_export.split",
        output: "pe_export.wasm",
        profile: "release",
        harness: "tests/pe_export_runtime.mjs",
        extra_arguments: &[],
    },
    RuntimeFixture {
        source: "tests/pe_debug_id.split",
        output: "pe_debug_id.wasm",
        profile: "release",
        harness: "tests/pe_debug_id_runtime.mjs",
        extra_arguments: &[],
    },
    RuntimeFixture {
        source: "tests/async_failure.split",
        output: "async_failure-debug.wasm",
        profile: "debug",
        harness: "tests/async_failure_runtime.mjs",
        extra_arguments: &[],
    },
    RuntimeFixture {
        source: "tests/async_failure.split",
        output: "async_failure-release.wasm",
        profile: "release",
        harness: "tests/async_failure_runtime.mjs",
        extra_arguments: &[],
    },
    RuntimeFixture {
        source: "tests/elf_export.split",
        output: "elf_export-debug.wasm",
        profile: "debug",
        harness: "tests/elf_export_runtime.mjs",
        extra_arguments: &[],
    },
    RuntimeFixture {
        source: "tests/elf_export.split",
        output: "elf_export-release.wasm",
        profile: "release",
        harness: "tests/elf_export_runtime.mjs",
        extra_arguments: &[],
    },
    RuntimeFixture {
        source: "tests/elf_build_id.split",
        output: "elf_build_id.wasm",
        profile: "release",
        harness: "tests/unix_identity_runtime.mjs",
        extra_arguments: &["elf"],
    },
    RuntimeFixture {
        source: "tests/mach_export.split",
        output: "mach-export-debug.wasm",
        profile: "debug",
        harness: "tests/mach_export_runtime.mjs",
        extra_arguments: &[],
    },
    RuntimeFixture {
        source: "tests/mach_export.split",
        output: "mach-export-release.wasm",
        profile: "release",
        harness: "tests/mach_export_runtime.mjs",
        extra_arguments: &[],
    },
    RuntimeFixture {
        source: "tests/mach_uuid.split",
        output: "mach_uuid.wasm",
        profile: "release",
        harness: "tests/unix_identity_runtime.mjs",
        extra_arguments: &["mach"],
    },
    RuntimeFixture {
        source: "examples/artificial.split",
        output: "artificial.wasm",
        profile: "release",
        harness: "tests/artificial_runtime.mjs",
        extra_arguments: &[],
    },
    RuntimeFixture {
        source: "examples/himno.split",
        output: "himno.wasm",
        profile: "release",
        harness: "tests/himno_runtime.mjs",
        extra_arguments: &[],
    },
    RuntimeFixture {
        source: "tests/loaded_module.split",
        output: "loaded_module.wasm",
        profile: "release",
        harness: "tests/loaded_module_runtime.mjs",
        extra_arguments: &[],
    },
    RuntimeFixture {
        source: "tests/action_defaults.split",
        output: "action_defaults.wasm",
        profile: "release",
        harness: "tests/action_defaults_runtime.mjs",
        extra_arguments: &[],
    },
    RuntimeFixture {
        source: "tests/tick_rate_lifecycle.split",
        output: "tick_rate_lifecycle.wasm",
        profile: "release",
        harness: "tests/tick_rate_lifecycle_runtime.mjs",
        extra_arguments: &[],
    },
    RuntimeFixture {
        source: "tests/on_state_ready.split",
        output: "on_state_ready.wasm",
        profile: "release",
        harness: "tests/on_state_ready_runtime.mjs",
        extra_arguments: &[],
    },
    RuntimeFixture {
        source: "tests/while_attached_control.split",
        output: "while_attached_control.wasm",
        profile: "release",
        harness: "tests/while_attached_control_runtime.mjs",
        extra_arguments: &[],
    },
    RuntimeFixture {
        source: "tests/process_exit_lifecycle.split",
        output: "process_exit_lifecycle.wasm",
        profile: "release",
        harness: "tests/process_exit_lifecycle_runtime.mjs",
        extra_arguments: &[],
    },
    RuntimeFixture {
        source: "tests/timer_split_index.split",
        output: "timer_split_index.wasm",
        profile: "release",
        harness: "tests/timer_split_index_runtime.mjs",
        extra_arguments: &[],
    },
    RuntimeFixture {
        source: "tests/host_metadata.split",
        output: "host_metadata.wasm",
        profile: "release",
        harness: "tests/host_metadata_runtime.mjs",
        extra_arguments: &[],
    },
    RuntimeFixture {
        source: "tests/async_loop.split",
        output: "async_loop.wasm",
        profile: "release",
        harness: "tests/async_loop_runtime.mjs",
        extra_arguments: &[],
    },
    RuntimeFixture {
        source: "tests/fallible_process_operations.split",
        output: "fallible_process_operations.wasm",
        profile: "release",
        harness: "tests/fallible_process_operations_runtime.mjs",
        extra_arguments: &[],
    },
    RuntimeFixture {
        source: "tests/signed_pointer_offsets.split",
        output: "signed_pointer_offsets.wasm",
        profile: "release",
        harness: "tests/signed_pointer_offsets_runtime.mjs",
        extra_arguments: &[],
    },
    RuntimeFixture {
        source: "tests/subnormal_float_literals.split",
        output: "subnormal_float_literals.wasm",
        profile: "release",
        harness: "tests/subnormal_float_literals_runtime.mjs",
        extra_arguments: &[],
    },
    RuntimeFixture {
        source: "tests/float_helpers.split",
        output: "float_helpers.wasm",
        profile: "release",
        harness: "tests/float_helpers_runtime.mjs",
        extra_arguments: &[],
    },
    RuntimeFixture {
        source: "tests/duration_helpers.split",
        output: "duration_helpers.wasm",
        profile: "release",
        harness: "tests/duration_helpers_runtime.mjs",
        extra_arguments: &[],
    },
    RuntimeFixture {
        source: "tests/instant_helpers.split",
        output: "instant_helpers.wasm",
        profile: "release",
        harness: "tests/instant_helpers_runtime.mjs",
        extra_arguments: &[],
    },
    RuntimeFixture {
        source: "tests/for_loop.split",
        output: "for_loop.wasm",
        profile: "release",
        harness: "tests/for_loop_runtime.mjs",
        extra_arguments: &[],
    },
    RuntimeFixture {
        source: "tests/collection_iteration_runtime.split",
        output: "collection_iteration_runtime.wasm",
        profile: "release",
        harness: "tests/collection_iteration_runtime.mjs",
        extra_arguments: &[],
    },
    RuntimeFixture {
        source: "tests/array_push_runtime.split",
        output: "array_push_runtime.wasm",
        profile: "release",
        harness: "tests/array_push_runtime.mjs",
        extra_arguments: &[],
    },
    RuntimeFixture {
        source: "tests/set_runtime.split",
        output: "set_runtime.wasm",
        profile: "release",
        harness: "tests/set_runtime.mjs",
        extra_arguments: &[],
    },
    RuntimeFixture {
        source: "tests/map_runtime.split",
        output: "map_runtime.wasm",
        profile: "release",
        harness: "tests/map_runtime.mjs",
        extra_arguments: &[],
    },
    RuntimeFixture {
        source: "examples/openjk_speed.split",
        output: "openjk_speed.wasm",
        profile: "release",
        harness: "tests/openjk_speed_runtime.mjs",
        extra_arguments: &[],
    },
    RuntimeFixture {
        source: "examples/battlefront_ii.split",
        output: "battlefront_ii.wasm",
        profile: "release",
        harness: "tests/battlefront_ii_runtime.mjs",
        extra_arguments: &[],
    },
    RuntimeFixture {
        source: "examples/battlefront_ii.split",
        output: "battlefront_ii.wasm",
        profile: "release",
        harness: "tests/battlefront_ii_runtime.mjs",
        extra_arguments: &["gc"],
    },
    RuntimeFixture {
        source: "examples/dark_sasi.split",
        output: "dark_sasi.wasm",
        profile: "release",
        harness: "tests/dark_sasi_runtime.mjs",
        extra_arguments: &[],
    },
    RuntimeFixture {
        source: "examples/dark_sasi.split",
        output: "dark_sasi.wasm",
        profile: "release",
        harness: "tests/dark_sasi_runtime.mjs",
        extra_arguments: &["skipped"],
    },
    RuntimeFixture {
        source: "examples/nioh_rta_no_load.split",
        output: "nioh_rta_no_load.wasm",
        profile: "release",
        harness: "tests/nioh_rta_no_load_runtime.mjs",
        extra_arguments: &[],
    },
    RuntimeFixture {
        source: "examples/nioh_rta_no_load.split",
        output: "nioh_rta_no_load.wasm",
        profile: "release",
        harness: "tests/nioh_rta_no_load_runtime.mjs",
        extra_arguments: &["v12105"],
    },
    RuntimeFixture {
        source: "examples/nioh_rta_no_load.split",
        output: "nioh_rta_no_load.wasm",
        profile: "release",
        harness: "tests/nioh_rta_no_load_runtime.mjs",
        extra_arguments: &["v12106"],
    },
    RuntimeFixture {
        source: "examples/nioh_rta_no_load.split",
        output: "nioh_rta_no_load.wasm",
        profile: "release",
        harness: "tests/nioh_rta_no_load_runtime.mjs",
        extra_arguments: &["unknown"],
    },
    RuntimeFixture {
        source: "tests/process_results.split",
        output: "process_results.wasm",
        profile: "release",
        harness: "tests/process_results_runtime.mjs",
        extra_arguments: &[],
    },
    RuntimeFixture {
        source: "tests/process_readable_enums.split",
        output: "process_readable_enums.wasm",
        profile: "release",
        harness: "tests/process_readable_enums_runtime.mjs",
        extra_arguments: &[],
    },
    RuntimeFixture {
        source: "tests/process_scan_memory.split",
        output: "process_scan_memory.wasm",
        profile: "release",
        harness: "tests/process_scan_memory_runtime.mjs",
        extra_arguments: &[],
    },
    RuntimeFixture {
        source: "tests/process_scan_memory_any.split",
        output: "process_scan_memory_any.wasm",
        profile: "release",
        harness: "tests/process_scan_memory_any_runtime.mjs",
        extra_arguments: &[],
    },
    RuntimeFixture {
        source: "tests/process_memory_ranges.split",
        output: "process_memory_ranges.wasm",
        profile: "release",
        harness: "tests/process_memory_ranges_runtime.mjs",
        extra_arguments: &[],
    },
    RuntimeFixture {
        source: "tests/process_find_memory_range.split",
        output: "process_find_memory_range.wasm",
        profile: "release",
        harness: "tests/process_find_memory_range_runtime.mjs",
        extra_arguments: &[],
    },
    RuntimeFixture {
        source: "tests/managed_instances_runtime.split",
        output: "managed_instances_runtime.wasm",
        profile: "release",
        harness: "tests/managed_instances_runtime.mjs",
        extra_arguments: &[],
    },
    RuntimeFixture {
        source: "tests/managed_instances_mono_runtime.split",
        output: "managed_instances_mono_runtime.wasm",
        profile: "release",
        harness: "tests/managed_instances_mono_runtime.mjs",
        extra_arguments: &[],
    },
    RuntimeFixture {
        source: "tests/managed_array_freeze.split",
        output: "managed_array_freeze.wasm",
        profile: "release",
        harness: "tests/managed_array_freeze_runtime.mjs",
        extra_arguments: &[],
    },
    RuntimeFixture {
        source: "tests/managed_array_freeze.split",
        output: "managed_array_freeze_debug.wasm",
        profile: "debug",
        harness: "tests/managed_array_freeze_runtime.mjs",
        extra_arguments: &[],
    },
    RuntimeFixture {
        source: "tests/managed_arrays.split",
        output: "managed_arrays.wasm",
        profile: "release",
        harness: "tests/managed_arrays_runtime.mjs",
        extra_arguments: &[],
    },
    RuntimeFixture {
        source: "tests/managed_arrays.split",
        output: "managed_arrays_debug.wasm",
        profile: "debug",
        harness: "tests/managed_arrays_runtime.mjs",
        extra_arguments: &[],
    },
    RuntimeFixture {
        source: "tests/managed_map_class_arrays.split",
        output: "managed_map_class_arrays.wasm",
        profile: "release",
        harness: "tests/managed_keyed_classes_runtime.mjs",
        extra_arguments: &["map", "--array-elements"],
    },
    RuntimeFixture {
        source: "tests/managed_map_class_arrays.split",
        output: "managed_map_class_arrays_debug.wasm",
        profile: "debug",
        harness: "tests/managed_keyed_classes_runtime.mjs",
        extra_arguments: &["map", "--array-elements"],
    },
    RuntimeFixture {
        source: "tests/managed_set_class_arrays.split",
        output: "managed_set_class_arrays.wasm",
        profile: "release",
        harness: "tests/managed_keyed_classes_runtime.mjs",
        extra_arguments: &["set", "--array-elements"],
    },
    RuntimeFixture {
        source: "tests/managed_set_class_arrays.split",
        output: "managed_set_class_arrays_debug.wasm",
        profile: "debug",
        harness: "tests/managed_keyed_classes_runtime.mjs",
        extra_arguments: &["set", "--array-elements"],
    },
    RuntimeFixture {
        source: "tests/managed_map_classes.split",
        output: "managed_map_classes.wasm",
        profile: "release",
        harness: "tests/managed_keyed_classes_runtime.mjs",
        extra_arguments: &["map"],
    },
    RuntimeFixture {
        source: "tests/managed_map_classes.split",
        output: "managed_map_classes_debug.wasm",
        profile: "debug",
        harness: "tests/managed_keyed_classes_runtime.mjs",
        extra_arguments: &["map"],
    },
    RuntimeFixture {
        source: "tests/managed_set_classes.split",
        output: "managed_set_classes.wasm",
        profile: "release",
        harness: "tests/managed_keyed_classes_runtime.mjs",
        extra_arguments: &["set"],
    },
    RuntimeFixture {
        source: "tests/managed_set_classes.split",
        output: "managed_set_classes_debug.wasm",
        profile: "debug",
        harness: "tests/managed_keyed_classes_runtime.mjs",
        extra_arguments: &["set"],
    },
    RuntimeFixture {
        source: "tests/managed_collection_contracts.split",
        output: "managed_collection_contracts.wasm",
        profile: "release",
        harness: "tests/managed_collection_type_runtime.mjs",
        extra_arguments: &["mono", "--public"],
    },
    RuntimeFixture {
        source: "tests/managed_collection_contracts.split",
        output: "managed_collection_contracts.wasm",
        profile: "release",
        harness: "tests/managed_collection_type_runtime.mjs",
        extra_arguments: &["il2cpp", "--public"],
    },
    RuntimeFixture {
        source: "tests/managed_collection_contracts.split",
        output: "managed_collection_contracts_debug.wasm",
        profile: "debug",
        harness: "tests/managed_collection_type_runtime.mjs",
        extra_arguments: &["mono", "--public"],
    },
    RuntimeFixture {
        source: "tests/managed_collection_contracts.split",
        output: "managed_collection_contracts_debug.wasm",
        profile: "debug",
        harness: "tests/managed_collection_type_runtime.mjs",
        extra_arguments: &["il2cpp", "--public"],
    },
    RuntimeFixture {
        source: "tests/managed_map_lists.split",
        output: "managed_map_lists.wasm",
        profile: "release",
        harness: "tests/managed_keyed_lists_runtime.mjs",
        extra_arguments: &["map"],
    },
    RuntimeFixture {
        source: "tests/managed_map_lists.split",
        output: "managed_map_lists_debug.wasm",
        profile: "debug",
        harness: "tests/managed_keyed_lists_runtime.mjs",
        extra_arguments: &["map"],
    },
    RuntimeFixture {
        source: "tests/managed_set_lists.split",
        output: "managed_set_lists.wasm",
        profile: "release",
        harness: "tests/managed_keyed_lists_runtime.mjs",
        extra_arguments: &["set"],
    },
    RuntimeFixture {
        source: "tests/managed_set_lists.split",
        output: "managed_set_lists_debug.wasm",
        profile: "debug",
        harness: "tests/managed_keyed_lists_runtime.mjs",
        extra_arguments: &["set"],
    },
    RuntimeFixture {
        source: "tests/managed_nested_map.split",
        output: "managed_nested_map.wasm",
        profile: "release",
        harness: "tests/managed_nested_keyed_runtime.mjs",
        extra_arguments: &["map"],
    },
    RuntimeFixture {
        source: "tests/managed_nested_map.split",
        output: "managed_nested_map_debug.wasm",
        profile: "debug",
        harness: "tests/managed_nested_keyed_runtime.mjs",
        extra_arguments: &["map"],
    },
    RuntimeFixture {
        source: "tests/managed_nested_set.split",
        output: "managed_nested_set.wasm",
        profile: "release",
        harness: "tests/managed_nested_keyed_runtime.mjs",
        extra_arguments: &["set"],
    },
    RuntimeFixture {
        source: "tests/managed_nested_set.split",
        output: "managed_nested_set_debug.wasm",
        profile: "debug",
        harness: "tests/managed_nested_keyed_runtime.mjs",
        extra_arguments: &["set"],
    },
    RuntimeFixture {
        source: "tests/keyed_equality.split",
        output: "keyed_equality.wasm",
        profile: "release",
        harness: "tests/keyed_equality_runtime.mjs",
        extra_arguments: &[],
    },
    RuntimeFixture {
        source: "tests/keyed_equality.split",
        output: "keyed_equality_debug.wasm",
        profile: "debug",
        harness: "tests/keyed_equality_runtime.mjs",
        extra_arguments: &[],
    },
    RuntimeFixture {
        source: "tests/managed_sets.split",
        output: "managed_sets.wasm",
        profile: "release",
        harness: "tests/managed_sets_runtime.mjs",
        extra_arguments: &[],
    },
    RuntimeFixture {
        source: "tests/managed_sets.split",
        output: "managed_sets_debug.wasm",
        profile: "debug",
        harness: "tests/managed_sets_runtime.mjs",
        extra_arguments: &[],
    },
    RuntimeFixture {
        source: "tests/managed_set_scalars.split",
        output: "managed_set_scalars.wasm",
        profile: "release",
        harness: "tests/managed_set_scalars_runtime.mjs",
        extra_arguments: &[],
    },
    RuntimeFixture {
        source: "tests/managed_set_scalars.split",
        output: "managed_set_scalars_debug.wasm",
        profile: "debug",
        harness: "tests/managed_set_scalars_runtime.mjs",
        extra_arguments: &[],
    },
    RuntimeFixture {
        source: "tests/managed_set_inline_enum.split",
        output: "managed_set_inline_enum.wasm",
        profile: "release",
        harness: "tests/managed_set_inline_runtime.mjs",
        extra_arguments: &["--enum"],
    },
    RuntimeFixture {
        source: "tests/managed_set_inline_enum.split",
        output: "managed_set_inline_enum_debug.wasm",
        profile: "debug",
        harness: "tests/managed_set_inline_runtime.mjs",
        extra_arguments: &["--enum"],
    },
    RuntimeFixture {
        source: "tests/managed_set_inline.split",
        output: "managed_set_inline.wasm",
        profile: "release",
        harness: "tests/managed_set_inline_runtime.mjs",
        extra_arguments: &[],
    },
    RuntimeFixture {
        source: "tests/managed_set_inline.split",
        output: "managed_set_inline_debug.wasm",
        profile: "debug",
        harness: "tests/managed_set_inline_runtime.mjs",
        extra_arguments: &[],
    },
    RuntimeFixture {
        source: "tests/managed_maps.split",
        output: "managed_maps.wasm",
        profile: "release",
        harness: "tests/managed_maps_runtime.mjs",
        extra_arguments: &[],
    },
    RuntimeFixture {
        source: "tests/managed_map_inline.split",
        output: "managed_map_inline.wasm",
        profile: "release",
        harness: "tests/managed_map_inline_runtime.mjs",
        extra_arguments: &[],
    },
    RuntimeFixture {
        source: "tests/managed_map_inline.split",
        output: "managed_map_inline_debug.wasm",
        profile: "debug",
        harness: "tests/managed_map_inline_runtime.mjs",
        extra_arguments: &[],
    },
    RuntimeFixture {
        source: "tests/managed_maps.split",
        output: "managed_maps_debug.wasm",
        profile: "debug",
        harness: "tests/managed_maps_runtime.mjs",
        extra_arguments: &[],
    },
    RuntimeFixture {
        source: "tests/managed_lists.split",
        output: "managed_lists.wasm",
        profile: "release",
        harness: "tests/managed_lists_runtime.mjs",
        extra_arguments: &[],
    },
    RuntimeFixture {
        source: "tests/managed_lists.split",
        output: "managed_lists_debug.wasm",
        profile: "debug",
        harness: "tests/managed_lists_runtime.mjs",
        extra_arguments: &[],
    },
    RuntimeFixture {
        source: "tests/managed_array_tree.split",
        output: "managed_array_tree.wasm",
        profile: "release",
        harness: "tests/managed_array_tree_runtime.mjs",
        extra_arguments: &[],
    },
    RuntimeFixture {
        source: "tests/managed_array_tree.split",
        output: "managed_array_tree_debug.wasm",
        profile: "debug",
        harness: "tests/managed_array_tree_runtime.mjs",
        extra_arguments: &[],
    },
    RuntimeFixture {
        source: "tests/managed_live_identity.split",
        output: "managed_live_identity.wasm",
        profile: "release",
        harness: "tests/managed_snapshot_identity_runtime.mjs",
        extra_arguments: &["--live"],
    },
    RuntimeFixture {
        source: "tests/managed_live_identity.split",
        output: "managed_live_identity_debug.wasm",
        profile: "debug",
        harness: "tests/managed_snapshot_identity_runtime.mjs",
        extra_arguments: &["--live"],
    },
    RuntimeFixture {
        source: "tests/managed_snapshot_identity.split",
        output: "managed_snapshot_identity.wasm",
        profile: "release",
        harness: "tests/managed_snapshot_identity_runtime.mjs",
        extra_arguments: &[],
    },
    RuntimeFixture {
        source: "tests/managed_snapshot_identity.split",
        output: "managed_snapshot_identity_debug.wasm",
        profile: "debug",
        harness: "tests/managed_snapshot_identity_runtime.mjs",
        extra_arguments: &[],
    },
    RuntimeFixture {
        source: "tests/managed_deep_snapshot.split",
        output: "managed_deep_snapshot.wasm",
        profile: "release",
        harness: "tests/managed_deep_snapshot_runtime.mjs",
        extra_arguments: &[],
    },
    RuntimeFixture {
        source: "tests/managed_slot_bounds.split",
        output: "managed_slot_bounds.wasm",
        profile: "release",
        harness: "tests/managed_slot_bounds_runtime.mjs",
        extra_arguments: &[],
    },
    RuntimeFixture {
        source: "tests/managed_slot_bounds.split",
        output: "managed_slot_bounds_debug.wasm",
        profile: "debug",
        harness: "tests/managed_slot_bounds_runtime.mjs",
        extra_arguments: &[],
    },
    RuntimeFixture {
        source: "tests/managed_deep_snapshot.split",
        output: "managed_deep_snapshot_debug.wasm",
        profile: "debug",
        harness: "tests/managed_deep_snapshot_runtime.mjs",
        extra_arguments: &[],
    },
    RuntimeFixture {
        source: "tests/managed_optional_snapshot.split",
        output: "managed_optional_snapshot.wasm",
        profile: "release",
        harness: "tests/managed_deep_snapshot_runtime.mjs",
        extra_arguments: &["--optional-left"],
    },
    RuntimeFixture {
        source: "tests/managed_optional_snapshot.split",
        output: "managed_optional_snapshot_debug.wasm",
        profile: "debug",
        harness: "tests/managed_deep_snapshot_runtime.mjs",
        extra_arguments: &["--optional-left"],
    },
    RuntimeFixture {
        source: "tests/managed_snapshot_work_budget.split",
        output: "managed_snapshot_work_budget.wasm",
        profile: "release",
        harness: "tests/managed_recursive_snapshot_runtime.mjs",
        extra_arguments: &["--work-budget"],
    },
    RuntimeFixture {
        source: "tests/managed_snapshot_work_budget.split",
        output: "managed_snapshot_work_budget_debug.wasm",
        profile: "debug",
        harness: "tests/managed_recursive_snapshot_runtime.mjs",
        extra_arguments: &["--work-budget"],
    },
    RuntimeFixture {
        source: "tests/managed_error_path_bound.split",
        output: "managed_error_path_bound.wasm",
        profile: "release",
        harness: "tests/managed_recursive_snapshot_runtime.mjs",
        extra_arguments: &["--bounded-paths"],
    },
    RuntimeFixture {
        source: "tests/managed_error_path_bound.split",
        output: "managed_error_path_bound_debug.wasm",
        profile: "debug",
        harness: "tests/managed_recursive_snapshot_runtime.mjs",
        extra_arguments: &["--bounded-paths"],
    },
    RuntimeFixture {
        source: "tests/managed_recursive_snapshot.split",
        output: "managed_recursive_snapshot.wasm",
        profile: "release",
        harness: "tests/managed_recursive_snapshot_runtime.mjs",
        extra_arguments: &[],
    },
    RuntimeFixture {
        source: "tests/managed_recursive_snapshot.split",
        output: "managed_recursive_snapshot_debug.wasm",
        profile: "debug",
        harness: "tests/managed_recursive_snapshot_runtime.mjs",
        extra_arguments: &[],
    },
    RuntimeFixture {
        source: "tests/managed_string_errors.split",
        output: "managed_string_errors.wasm",
        profile: "release",
        harness: "tests/managed_string_width_runtime.mjs",
        extra_arguments: &["--observe-errors"],
    },
    RuntimeFixture {
        source: "tests/managed_string_errors.split",
        output: "managed_string_errors_debug.wasm",
        profile: "debug",
        harness: "tests/managed_string_width_runtime.mjs",
        extra_arguments: &["--observe-errors"],
    },
    RuntimeFixture {
        source: "tests/managed_string_width.split",
        output: "managed_string_width.wasm",
        profile: "release",
        harness: "tests/managed_string_width_runtime.mjs",
        extra_arguments: &[],
    },
    RuntimeFixture {
        source: "tests/managed_string_width.split",
        output: "managed_string_width_debug.wasm",
        profile: "debug",
        harness: "tests/managed_string_width_runtime.mjs",
        extra_arguments: &[],
    },
    RuntimeFixture {
        source: "tests/unity_field_cursor.split",
        output: "unity_field_cursor.wasm",
        profile: "release",
        harness: "tests/unity_field_cursor_runtime.mjs",
        extra_arguments: &[],
    },
    RuntimeFixture {
        source: "tests/unity_field_cursor.split",
        output: "unity_field_cursor_debug.wasm",
        profile: "debug",
        harness: "tests/unity_field_cursor_runtime.mjs",
        extra_arguments: &[],
    },
    RuntimeFixture {
        source: "tests/unity_nested_metadata.split",
        output: "unity_nested_metadata.wasm",
        profile: "release",
        harness: "tests/unity_nested_metadata_runtime.mjs",
        extra_arguments: &[],
    },
    RuntimeFixture {
        source: "tests/il2cpp_profiles.split",
        output: "il2cpp_profiles.wasm",
        profile: "release",
        harness: "tests/il2cpp_profiles_runtime.mjs",
        extra_arguments: &[],
    },
    RuntimeFixture {
        source: "tests/il2cpp_profile_explicit_x64.split",
        output: "il2cpp_profile_explicit_x64.wasm",
        profile: "release",
        harness: "tests/il2cpp_profiles_runtime.mjs",
        extra_arguments: &["64"],
    },
    RuntimeFixture {
        source: "tests/il2cpp_profile_explicit_x86.split",
        output: "il2cpp_profile_explicit_x86.wasm",
        profile: "release",
        harness: "tests/il2cpp_profiles_runtime.mjs",
        extra_arguments: &["32"],
    },
    RuntimeFixture {
        source: "tests/il2cpp_profile_custom.split",
        output: "il2cpp_profile_custom.wasm",
        profile: "release",
        harness: "tests/il2cpp_profiles_runtime.mjs",
        extra_arguments: &["64"],
    },
    RuntimeFixture {
        source: "tests/il2cpp_profile_invalid.split",
        output: "il2cpp_profile_invalid.wasm",
        profile: "release",
        harness: "tests/il2cpp_profiles_runtime.mjs",
        extra_arguments: &["invalid"],
    },
    RuntimeFixture {
        source: "tests/mono_profiles.split",
        output: "mono_profiles.wasm",
        profile: "release",
        harness: "tests/mono_generic_metadata_runtime.mjs",
        extra_arguments: &[],
    },
    RuntimeFixture {
        source: "tests/mono_profiles.split",
        output: "mono_profiles.wasm",
        profile: "release",
        harness: "tests/mono_metadata_cursor_runtime.mjs",
        extra_arguments: &[],
    },
    RuntimeFixture {
        source: "tests/mono_profiles.split",
        output: "mono_profiles.wasm",
        profile: "release",
        harness: "tests/mono_profiles_runtime.mjs",
        extra_arguments: &[],
    },
    RuntimeFixture {
        source: "tests/mono_profiles.split",
        output: "mono_profiles-debug.wasm",
        profile: "debug",
        harness: "tests/mono_linux_runtime.mjs",
        extra_arguments: &[],
    },
    RuntimeFixture {
        source: "tests/mono_profiles.split",
        output: "mono_profiles.wasm",
        profile: "release",
        harness: "tests/mono_linux_runtime.mjs",
        extra_arguments: &[],
    },
    RuntimeFixture {
        source: "tests/mono_profiles.split",
        output: "mono_profiles-debug.wasm",
        profile: "debug",
        harness: "tests/mono_mac_runtime.mjs",
        extra_arguments: &[],
    },
    RuntimeFixture {
        source: "tests/mono_profiles.split",
        output: "mono_profiles.wasm",
        profile: "release",
        harness: "tests/mono_mac_runtime.mjs",
        extra_arguments: &[],
    },
    RuntimeFixture {
        source: "tests/mono_linux_V1.split",
        output: "mono_linux_V1_debug.wasm",
        profile: "debug",
        harness: "tests/mono_explicit_unix_runtime.mjs",
        extra_arguments: &["linux", "V1"],
    },
    RuntimeFixture {
        source: "tests/mono_linux_V1.split",
        output: "mono_linux_V1_release.wasm",
        profile: "release",
        harness: "tests/mono_explicit_unix_runtime.mjs",
        extra_arguments: &["linux", "V1"],
    },
    RuntimeFixture {
        source: "tests/mono_linux_V1Cattrs.split",
        output: "mono_linux_V1Cattrs_debug.wasm",
        profile: "debug",
        harness: "tests/mono_explicit_unix_runtime.mjs",
        extra_arguments: &["linux", "V1Cattrs"],
    },
    RuntimeFixture {
        source: "tests/mono_linux_V1Cattrs.split",
        output: "mono_linux_V1Cattrs_release.wasm",
        profile: "release",
        harness: "tests/mono_explicit_unix_runtime.mjs",
        extra_arguments: &["linux", "V1Cattrs"],
    },
    RuntimeFixture {
        source: "tests/mono_linux_V2.split",
        output: "mono_linux_V2_debug.wasm",
        profile: "debug",
        harness: "tests/mono_explicit_unix_runtime.mjs",
        extra_arguments: &["linux", "V2"],
    },
    RuntimeFixture {
        source: "tests/mono_linux_V2.split",
        output: "mono_linux_V2_release.wasm",
        profile: "release",
        harness: "tests/mono_explicit_unix_runtime.mjs",
        extra_arguments: &["linux", "V2"],
    },
    RuntimeFixture {
        source: "tests/mono_linux_V3.split",
        output: "mono_linux_V3_debug.wasm",
        profile: "debug",
        harness: "tests/mono_explicit_unix_runtime.mjs",
        extra_arguments: &["linux", "V3"],
    },
    RuntimeFixture {
        source: "tests/mono_linux_V3.split",
        output: "mono_linux_V3_release.wasm",
        profile: "release",
        harness: "tests/mono_explicit_unix_runtime.mjs",
        extra_arguments: &["linux", "V3"],
    },
    RuntimeFixture {
        source: "tests/mono_mac_V1.split",
        output: "mono_mac_V1_debug.wasm",
        profile: "debug",
        harness: "tests/mono_explicit_unix_runtime.mjs",
        extra_arguments: &["mac", "V1"],
    },
    RuntimeFixture {
        source: "tests/mono_mac_V1.split",
        output: "mono_mac_V1_release.wasm",
        profile: "release",
        harness: "tests/mono_explicit_unix_runtime.mjs",
        extra_arguments: &["mac", "V1"],
    },
    RuntimeFixture {
        source: "tests/mono_mac_V1Cattrs.split",
        output: "mono_mac_V1Cattrs_debug.wasm",
        profile: "debug",
        harness: "tests/mono_explicit_unix_runtime.mjs",
        extra_arguments: &["mac", "V1Cattrs"],
    },
    RuntimeFixture {
        source: "tests/mono_mac_V1Cattrs.split",
        output: "mono_mac_V1Cattrs_release.wasm",
        profile: "release",
        harness: "tests/mono_explicit_unix_runtime.mjs",
        extra_arguments: &["mac", "V1Cattrs"],
    },
    RuntimeFixture {
        source: "tests/mono_mac_V2.split",
        output: "mono_mac_V2_debug.wasm",
        profile: "debug",
        harness: "tests/mono_explicit_unix_runtime.mjs",
        extra_arguments: &["mac", "V2"],
    },
    RuntimeFixture {
        source: "tests/mono_mac_V2.split",
        output: "mono_mac_V2_release.wasm",
        profile: "release",
        harness: "tests/mono_explicit_unix_runtime.mjs",
        extra_arguments: &["mac", "V2"],
    },
    RuntimeFixture {
        source: "tests/mono_mac_V3.split",
        output: "mono_mac_V3_debug.wasm",
        profile: "debug",
        harness: "tests/mono_explicit_unix_runtime.mjs",
        extra_arguments: &["mac", "V3"],
    },
    RuntimeFixture {
        source: "tests/mono_mac_V3.split",
        output: "mono_mac_V3_release.wasm",
        profile: "release",
        harness: "tests/mono_explicit_unix_runtime.mjs",
        extra_arguments: &["mac", "V3"],
    },
    RuntimeFixture {
        source: "tests/mono_profiles_V1.split",
        output: "mono_profiles_V1.wasm",
        profile: "release",
        harness: "tests/mono_profiles_runtime.mjs",
        extra_arguments: &["V1"],
    },
    RuntimeFixture {
        source: "tests/mono_profiles_V1Cattrs.split",
        output: "mono_profiles_V1Cattrs.wasm",
        profile: "release",
        harness: "tests/mono_profiles_runtime.mjs",
        extra_arguments: &["V1Cattrs"],
    },
    RuntimeFixture {
        source: "tests/mono_profiles_V2.split",
        output: "mono_profiles_V2.wasm",
        profile: "release",
        harness: "tests/mono_profiles_runtime.mjs",
        extra_arguments: &["V2"],
    },
    RuntimeFixture {
        source: "tests/mono_profiles_V3.split",
        output: "mono_profiles_V3.wasm",
        profile: "release",
        harness: "tests/mono_profiles_runtime.mjs",
        extra_arguments: &["V3"],
    },
    RuntimeFixture {
        source: "tests/managed_inherited_static_runtime.split",
        output: "managed_inherited_static_runtime.wasm",
        profile: "release",
        harness: "tests/managed_inherited_static_runtime.mjs",
        extra_arguments: &[],
    },
    RuntimeFixture {
        source: "tests/managed_unused_metadata_runtime.split",
        output: "managed_unused_metadata_runtime.wasm",
        profile: "release",
        harness: "tests/managed_inherited_static_runtime.mjs",
        extra_arguments: &[],
    },
    RuntimeFixture {
        source: "tests/postfix_calls.split",
        output: "postfix_calls.wasm",
        profile: "release",
        harness: "tests/postfix_calls_runtime.mjs",
        extra_arguments: &[],
    },
    RuntimeFixture {
        source: "tests/string_predicates.split",
        output: "string_predicates.wasm",
        profile: "release",
        harness: "tests/string_predicates_runtime.mjs",
        extra_arguments: &[],
    },
    RuntimeFixture {
        source: "tests/string_parsing.split",
        output: "string_parsing.wasm",
        profile: "release",
        harness: "tests/string_parsing_runtime.mjs",
        extra_arguments: &[],
    },
    RuntimeFixture {
        source: "examples/tiberian_sun.split",
        output: "tiberian_sun.wasm",
        profile: "release",
        harness: "tests/tiberian_sun_runtime.mjs",
        extra_arguments: &[],
    },
    RuntimeFixture {
        source: "examples/dds.split",
        output: "dds.wasm",
        profile: "release",
        harness: "tests/dds_runtime.mjs",
        extra_arguments: &[],
    },
    RuntimeFixture {
        source: "tests/while_loop.split",
        output: "while_loop.wasm",
        profile: "release",
        harness: "tests/while_loop_runtime.mjs",
        extra_arguments: &[],
    },
    RuntimeFixture {
        source: "tests/wrapper_equality.split",
        output: "wrapper_equality.wasm",
        profile: "release",
        harness: "tests/wrapper_equality_runtime.mjs",
        extra_arguments: &[],
    },
    RuntimeFixture {
        source: "tests/wrapper_fallbacks.split",
        output: "wrapper_fallbacks.wasm",
        profile: "release",
        harness: "tests/wrapper_fallbacks_runtime.mjs",
        extra_arguments: &[],
    },
    RuntimeFixture {
        source: "tests/wrapper_match.split",
        output: "wrapper_match.wasm",
        profile: "release",
        harness: "tests/wrapper_match_runtime.mjs",
        extra_arguments: &[],
    },
];

fn main() -> ExitCode {
    let arguments = env::args().skip(1).collect::<Vec<_>>();
    match arguments.as_slice() {
        [command] if command == "check" => match check() {
            Ok(()) => ExitCode::SUCCESS,
            Err(error) => {
                eprintln!("verification failed: {error}");
                ExitCode::FAILURE
            }
        },
        [command] if command == "conformance" => match conformance() {
            Ok(()) => ExitCode::SUCCESS,
            Err(error) => {
                eprintln!("conformance verification failed: {error}");
                ExitCode::FAILURE
            }
        },
        [command, arguments @ ..] if command == "unity-baseline" => {
            let root = PathBuf::from(env!("CARGO_MANIFEST_DIR"));
            let mut args = vec![
                "run",
                "--quiet",
                "--profile",
                "max-opt",
                "--example",
                "unity_baseline",
                "--",
            ];
            args.extend(arguments.iter().map(String::as_str));
            match run(&root, "cargo", &args) {
                Ok(()) => ExitCode::SUCCESS,
                Err(error) => {
                    eprintln!("Unity baseline failed: {error}");
                    ExitCode::FAILURE
                }
            }
        }
        [command] if command == "docs" => {
            match documentation_site::generate(Some(Path::new("target/generated-docs"))) {
                Ok(()) => ExitCode::SUCCESS,
                Err(error) => {
                    eprintln!("documentation site generation failed: {error}");
                    ExitCode::FAILURE
                }
            }
        }
        [command, mode] if command == "docs" && mode == "--check" => {
            match documentation_site::generate(None) {
                Ok(()) => ExitCode::SUCCESS,
                Err(error) => {
                    eprintln!("documentation site validation failed: {error}");
                    ExitCode::FAILURE
                }
            }
        }
        [command, destination] if command == "docs" => {
            match documentation_site::generate(Some(Path::new(destination))) {
                Ok(()) => ExitCode::SUCCESS,
                Err(error) => {
                    eprintln!("documentation site generation failed: {error}");
                    ExitCode::FAILURE
                }
            }
        }
        _ => {
            eprintln!(
                "usage: cargo xtask <check | conformance | unity-baseline [--record | --compare REPORT.json] | docs [OUTPUT | --check]>"
            );
            ExitCode::FAILURE
        }
    }
}

fn check() -> Result<(), String> {
    let root = PathBuf::from(env!("CARGO_MANIFEST_DIR"));
    run(&root, "cargo", &["fmt", "--all", "--", "--check"])?;
    run(
        &root,
        "cargo",
        &["clippy", "--all-targets", "--", "-D", "warnings"],
    )?;
    documentation_site::generate(None)?;
    compiler_runtime_conformance(&root)?;
    verify_extension(&root)?;
    println!("repository verification passed");
    Ok(())
}

fn conformance() -> Result<(), String> {
    let root = PathBuf::from(env!("CARGO_MANIFEST_DIR"));
    compiler_runtime_conformance(&root)?;
    println!("compiler and runtime conformance passed");
    Ok(())
}

fn compiler_runtime_conformance(root: &Path) -> Result<(), String> {
    run(
        root,
        "cargo",
        &[
            "test",
            "--package",
            "splitscript-syntax",
            "--package",
            "splitscript-stdlib-loader",
        ],
    )?;
    // Do not ask Cargo to build the currently running xtask as a test harness:
    // Windows cannot replace its locked executable. These targets are the
    // complete repository test surface owned by the product.
    run(
        root,
        "cargo",
        &[
            "test",
            "--lib",
            "--bin",
            "splitc",
            "--bin",
            "splitls",
            "--test",
            "compiler",
            "--examples",
        ],
    )?;
    run(
        root,
        "cargo",
        &[
            "build",
            "--profile",
            "max-opt",
            "--bin",
            "splitc",
            "--bin",
            "splitls",
        ],
    )?;
    let outputs = root.join("target/verify");
    fs::create_dir_all(&outputs)
        .map_err(|error| format!("could not create {}: {error}", outputs.display()))?;
    let compiler = root.join(if cfg!(windows) {
        "target/max-opt/splitc.exe"
    } else {
        "target/max-opt/splitc"
    });
    let artifacts = verification_artifacts(RUNTIME_FIXTURES, COMPILE_FIXTURES)?;
    println!(
        "compiling {} unique verification artifacts for {} runtime scenarios",
        artifacts.len(),
        RUNTIME_FIXTURES.len()
    );
    for fixture in &artifacts {
        compile_once(
            root,
            &compiler,
            fixture.source,
            &outputs.join(fixture.output),
            fixture.profile,
        )?;
    }
    let debug = outputs.join("debug_profile.debug.wasm");
    let release = outputs.join("debug_profile.release.wasm");

    for module in artifacts.iter().map(|fixture| outputs.join(fixture.output)) {
        run(
            root,
            "wasm-tools",
            &["validate", "--features", "all", path_text(&module)?],
        )?;
    }

    for fixture in RUNTIME_FIXTURES {
        let module = outputs.join(fixture.output);
        let mut arguments = vec![fixture.harness, path_text(&module)?];
        arguments.extend_from_slice(fixture.extra_arguments);
        run(root, "node", &arguments)?;
    }
    run(
        root,
        "node",
        &[
            "tests/debug_profile_runtime.mjs",
            path_text(&debug)?,
            path_text(&release)?,
        ],
    )?;
    run(
        root,
        "cargo",
        &[
            "run",
            "--quiet",
            "--profile",
            "max-opt",
            "--example",
            "unity_baseline",
        ],
    )?;
    Ok(())
}

fn verify_extension(root: &Path) -> Result<(), String> {
    run(
        root,
        "cargo",
        &[
            "build",
            "--profile",
            "max-opt",
            "--target",
            "wasm32-unknown-unknown",
            "--package",
            "splitscript-vscode-wasm",
        ],
    )?;

    let extension = root.join("editors/vscode");
    let npm = if cfg!(windows) { "npm.cmd" } else { "npm" };
    run(&extension, npm, &["run", "check"])?;
    run(&extension, npm, &["test"])?;
    let verification_extension = root.join("target/vscode-check/dist");
    run(
        &extension,
        "node",
        &[
            "scripts/build.mjs",
            "--output",
            path_text(&verification_extension)?,
        ],
    )?;
    let embedded_compiler =
        root.join("target/wasm32-unknown-unknown/max-opt/splitscript_vscode_wasm.wasm");
    let packaged_compiler = verification_extension.join("splitscript_vscode_wasm.wasm");
    run(
        root,
        "wasm-tools",
        &["validate", path_text(&embedded_compiler)?],
    )?;
    run(
        root,
        "node",
        &[
            "tests/embedded_compiler_runtime.mjs",
            path_text(&embedded_compiler)?,
        ],
    )?;
    let embedded_worker = verification_extension.join("embeddedCompilerNodeWorker.js");
    run(
        root,
        "node",
        &[
            "tests/embedded_compiler_worker_runtime.mjs",
            path_text(&embedded_worker)?,
            path_text(&packaged_compiler)?,
            path_text(&verification_extension.join("embeddedCompilerWorkerClient.js"))?,
        ],
    )?;
    run(
        root,
        "node",
        &[
            "tests/web_worker_bundles_runtime.mjs",
            path_text(&verification_extension.join("web/extension.js"))?,
            path_text(&verification_extension.join("web/embeddedCompilerWorker.js"))?,
            path_text(&verification_extension.join("web/embeddedLanguageServerWorker.js"))?,
            path_text(&packaged_compiler)?,
        ],
    )?;
    run(
        &extension,
        "node",
        &[
            "scripts/test-web-host.mjs",
            path_text(&verification_extension)?,
        ],
    )?;
    let embedded_language_server_worker =
        verification_extension.join("embeddedLanguageServerNodeWorker.js");
    run(
        root,
        "node",
        &[
            "tests/embedded_language_server_worker_runtime.mjs",
            path_text(&embedded_language_server_worker)?,
            path_text(&packaged_compiler)?,
        ],
    )?;

    // Build the exact production tree that VSCE packages outside `dist`, which
    // may be held open by an Extension Development Host on Windows. Audit its
    // manifest and construct an isolated VSIX without mutating those outputs.
    let production_extension = root.join("target/vscode-package/dist");
    run(
        &extension,
        "node",
        &[
            "scripts/build.mjs",
            "--production",
            "--output",
            path_text(&production_extension)?,
        ],
    )?;
    run(
        &extension,
        "node",
        &[
            "scripts/probe-package.mjs",
            path_text(&production_extension)?,
        ],
    )?;
    Ok(())
}

fn verification_artifacts(
    runtime_fixtures: &[RuntimeFixture],
    compile_fixtures: &[CompileFixture],
) -> Result<Vec<CompileFixture>, String> {
    let mut scenarios = BTreeSet::new();
    for fixture in runtime_fixtures {
        if !scenarios.insert((
            fixture.source,
            fixture.output,
            fixture.profile,
            fixture.harness,
            fixture.extra_arguments,
        )) {
            return Err(format!(
                "duplicate runtime scenario for `{}` with harness `{}` and arguments {:?}",
                fixture.output, fixture.harness, fixture.extra_arguments
            ));
        }
    }

    let mut artifacts = BTreeMap::new();
    for fixture in runtime_fixtures.iter().map(|fixture| CompileFixture {
        source: fixture.source,
        output: fixture.output,
        profile: fixture.profile,
    }) {
        insert_artifact(&mut artifacts, fixture)?;
    }
    for &fixture in compile_fixtures {
        insert_artifact(&mut artifacts, fixture)?;
    }
    Ok(artifacts.into_values().collect())
}

fn insert_artifact(
    artifacts: &mut BTreeMap<&'static str, CompileFixture>,
    fixture: CompileFixture,
) -> Result<(), String> {
    if let Some(previous) = artifacts.get(fixture.output) {
        if previous.source != fixture.source || previous.profile != fixture.profile {
            return Err(format!(
                "conflicting artifact `{}`: `{}` ({}) and `{}` ({})",
                fixture.output, previous.source, previous.profile, fixture.source, fixture.profile
            ));
        }
    } else {
        artifacts.insert(fixture.output, fixture);
    }
    Ok(())
}

fn compile_once(
    root: &Path,
    compiler: &Path,
    source: &str,
    output: &Path,
    profile: &str,
) -> Result<(), String> {
    run(
        root,
        path_text(compiler)?,
        &[source, "-o", path_text(output)?, "--profile", profile],
    )
}

fn path_text(path: &Path) -> Result<&str, String> {
    path.to_str()
        .ok_or_else(|| format!("path is not valid UTF-8: {}", path.display()))
}

fn run(directory: &Path, command: &str, arguments: &[&str]) -> Result<(), String> {
    println!("> {command} {}", arguments.join(" "));
    let status = Command::new(command)
        .args(arguments)
        .current_dir(directory)
        .status()
        .map_err(|error| format!("could not launch `{command}`: {error}"))?;
    if status.success() {
        Ok(())
    } else {
        Err(format!("`{command}` exited with {status}"))
    }
}
