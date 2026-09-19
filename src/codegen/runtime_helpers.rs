//! Descriptor-driven orchestration for compiler-generated runtime helpers.

use std::collections::HashMap;

use wasm_encoder::Function;

use crate::{
    ast::{Program, ValueId},
    intrinsic_registry::RuntimeHelperId,
    stdlib::StdlibTypeId,
    types::ResolvedArrayType,
};

use super::data_plan::FloatFormatData;
use super::data_plan::StringPool;
use super::imports::Abi;
use super::memory_plan::LinearMemoryLayout;
use super::runtime_helper_registry;
use super::{GcLayout, RuntimeHelperPlan, SettingStorage, Type, settings, try_array_element_type};

mod decimal_conversion;
mod equality;
mod file;
pub(super) mod float_format;
mod float_parse;
mod gba;
mod gcn;
mod genesis;
mod managed_context;
mod md5;
mod process;
mod provider;
mod ps1;
mod ps2;
mod sms;
mod strings;
mod wii;

pub(super) use equality::{compile_equality, emit_value_equality};

pub(super) fn build_enter_managed_object(inputs: &RuntimeHelperInputs<'_>) -> Function {
    managed_context::enter(inputs.gc)
}

pub(super) fn build_charge_managed_work(inputs: &RuntimeHelperInputs<'_>) -> Function {
    managed_context::charge_work(inputs.gc)
}

pub(super) struct RuntimeHelperInputs<'a> {
    pub abi: &'a Abi,
    pub strings: &'a StringPool,
    pub plan: &'a RuntimeHelperPlan,
    pub arrays: &'a [ResolvedArrayType],
    pub program: &'a Program,
    pub semantics: &'a crate::semantic::SemanticModel,
    pub settings: &'a settings::SettingsContext<'a>,
    pub settings_map: &'a HashMap<ValueId, SettingStorage>,
    pub gc: &'a GcLayout,
    pub failure_payloads: &'a super::failure_payload::FailurePayloadDemand,
    pub memory: LinearMemoryLayout,
    pub float_format: Option<&'a FloatFormatData>,
}

pub(super) fn compile_runtime(
    plan: &RuntimeHelperPlan,
    inputs: &RuntimeHelperInputs<'_>,
) -> Vec<Function> {
    plan.entries()
        .map(|helper| (runtime_helper_registry::descriptor(helper).build_body)(inputs))
        .collect()
}

fn array_layouts(inputs: &RuntimeHelperInputs<'_>, element: Type) -> (u32, u32) {
    let array = inputs
        .arrays
        .iter()
        .find(|array| try_array_element_type(array.id, inputs.semantics) == Some(element))
        .expect("runtime helper has its required reachable array layout")
        .id;
    let storage = super::array_value::storage_id(array, inputs.arrays, inputs.semantics);
    (
        inputs.gc.index(Type::Array(array)),
        inputs.gc.index(Type::ArrayStorage(storage)),
    )
}

pub(super) fn build_print_string(inputs: &RuntimeHelperInputs<'_>) -> Function {
    strings::compile_print_string(inputs.abi, inputs.gc, inputs.memory.scratch())
}

pub(super) fn build_timer_set_variable(inputs: &RuntimeHelperInputs<'_>) -> Function {
    strings::compile_timer_set_variable(inputs.abi, inputs.gc, inputs.memory.scratch())
}

pub(super) fn build_format_i64(inputs: &RuntimeHelperInputs<'_>) -> Function {
    strings::compile_format_i64(inputs.gc)
}

pub(super) fn build_zmij_mul128(_inputs: &RuntimeHelperInputs<'_>) -> Function {
    float_format::compile_mul128()
}

pub(super) fn build_zmij_mul192_hi128(inputs: &RuntimeHelperInputs<'_>) -> Function {
    float_format::compile_mul192_hi128(inputs.plan.function(RuntimeHelperId::ZmijMul128))
}

pub(super) fn build_zmij_decimal_f32(inputs: &RuntimeHelperInputs<'_>) -> Function {
    float_format::compile_decimal_f32(
        inputs
            .float_format
            .expect("float helpers require their static data")
            .pow10_significands,
        inputs.plan.function(RuntimeHelperId::ZmijMul128),
    )
}

pub(super) fn build_zmij_decimal_f64(inputs: &RuntimeHelperInputs<'_>) -> Function {
    float_format::compile_decimal_f64(
        inputs
            .float_format
            .expect("float helpers require their static data")
            .pow10_significands,
        inputs.plan.function(RuntimeHelperId::ZmijMul192Hi128),
    )
}

pub(super) fn build_format_f32(inputs: &RuntimeHelperInputs<'_>) -> Function {
    float_format::compile_format_f32(
        inputs.plan.function(RuntimeHelperId::ZmijDecimalF32),
        inputs.plan.function(RuntimeHelperId::StringFromMemory),
        inputs.memory.scratch().float_format,
        inputs.gc,
    )
}

pub(super) fn build_format_f64(inputs: &RuntimeHelperInputs<'_>) -> Function {
    float_format::compile_format_f64(
        inputs.plan.function(RuntimeHelperId::ZmijDecimalF64),
        inputs.plan.function(RuntimeHelperId::StringFromMemory),
        inputs.memory.scratch().float_format,
        inputs.gc,
    )
}

pub(super) fn build_format_char(inputs: &RuntimeHelperInputs<'_>) -> Function {
    strings::compile_format_char(inputs.gc)
}

pub(super) fn build_quote_debug_string(inputs: &RuntimeHelperInputs<'_>) -> Function {
    strings::compile_quote_debug_string(inputs.gc)
}

pub(super) fn build_string_equality(inputs: &RuntimeHelperInputs<'_>) -> Function {
    equality::compile_string_eq(inputs.gc)
}

pub(super) fn build_string_match(inputs: &RuntimeHelperInputs<'_>) -> Function {
    strings::compile_string_match(inputs.gc)
}

pub(super) fn build_string_find(inputs: &RuntimeHelperInputs<'_>) -> Function {
    strings::compile_string_find(inputs.gc)
}

pub(super) fn build_string_rfind(inputs: &RuntimeHelperInputs<'_>) -> Function {
    strings::compile_string_rfind(inputs.gc)
}

pub(super) fn build_string_ascii_case(inputs: &RuntimeHelperInputs<'_>) -> Function {
    strings::compile_string_ascii_case(inputs.gc)
}

pub(super) fn build_string_replace_all(inputs: &RuntimeHelperInputs<'_>) -> Function {
    strings::compile_string_replace_all(
        inputs.plan.function(RuntimeHelperId::StringFind),
        inputs.gc,
    )
}

pub(super) fn build_string_split(inputs: &RuntimeHelperInputs<'_>) -> Function {
    let (array, storage) = array_layouts(inputs, Type::Standard(StdlibTypeId::String));
    strings::compile_string_split(
        inputs.plan.function(RuntimeHelperId::StringFind),
        array,
        storage,
        inputs.gc,
    )
}

pub(super) fn build_string_parse_integer(inputs: &RuntimeHelperInputs<'_>) -> Function {
    strings::compile_string_parse_integer(inputs.gc)
}

pub(super) fn build_string_parse_float(inputs: &RuntimeHelperInputs<'_>) -> Function {
    let scratch = inputs.memory.scratch();
    float_parse::compile_string_parse_float(
        inputs.plan.function(RuntimeHelperId::DecimalLeftShift),
        inputs.plan.function(RuntimeHelperId::DecimalRightShift),
        inputs.plan.function(RuntimeHelperId::DecimalRound),
        scratch.float_parse_digits,
        inputs.gc,
    )
}

pub(super) fn build_decimal_left_shift(inputs: &RuntimeHelperInputs<'_>) -> Function {
    let scratch = inputs.memory.scratch();
    decimal_conversion::compile_decimal_left_shift(
        scratch.float_parse_digits,
        scratch.float_parse_temp,
    )
}

pub(super) fn build_decimal_right_shift(inputs: &RuntimeHelperInputs<'_>) -> Function {
    decimal_conversion::compile_decimal_right_shift(inputs.memory.scratch().float_parse_digits)
}

pub(super) fn build_decimal_round(inputs: &RuntimeHelperInputs<'_>) -> Function {
    decimal_conversion::compile_decimal_round(inputs.memory.scratch().float_parse_digits)
}

pub(super) fn build_string_inspect(inputs: &RuntimeHelperInputs<'_>) -> Function {
    strings::compile_string_inspect(inputs.gc)
}

pub(super) fn build_string_slice(inputs: &RuntimeHelperInputs<'_>) -> Function {
    strings::compile_string_slice(inputs.gc)
}

pub(super) fn build_string_trim_ascii_whitespace(inputs: &RuntimeHelperInputs<'_>) -> Function {
    strings::compile_string_trim_ascii_whitespace(
        inputs.plan.function(RuntimeHelperId::StringSlice),
        inputs.gc,
    )
}

pub(super) fn build_string_is_blank(inputs: &RuntimeHelperInputs<'_>) -> Function {
    strings::compile_string_is_blank(inputs.plan.function(RuntimeHelperId::StringInspect))
}

pub(super) fn build_string_pad(inputs: &RuntimeHelperInputs<'_>) -> Function {
    strings::compile_string_pad(inputs.gc)
}

pub(super) fn build_wrap_debug_entry(inputs: &RuntimeHelperInputs<'_>) -> Function {
    let (array, storage) = array_layouts(inputs, Type::Standard(StdlibTypeId::String));
    strings::compile_wrap_debug_entry(
        inputs.plan.function(RuntimeHelperId::IndentDisplay),
        inputs.plan.function(RuntimeHelperId::JoinStrings),
        array,
        storage,
        inputs.gc,
    )
}

pub(super) fn build_wrap_debug_variant(inputs: &RuntimeHelperInputs<'_>) -> Function {
    let (array, storage) = array_layouts(inputs, Type::Standard(StdlibTypeId::String));
    strings::compile_wrap_debug_variant(
        inputs.plan.function(RuntimeHelperId::WrapDebugEntry),
        inputs.plan.function(RuntimeHelperId::JoinStrings),
        array,
        storage,
        inputs.gc,
    )
}

pub(super) fn build_scan_process_range(inputs: &RuntimeHelperInputs<'_>) -> Function {
    process::compile_scan_process_range(inputs.abi, inputs.memory.scratch().scan)
}

pub(super) fn build_scan_aligned_pointer_range(inputs: &RuntimeHelperInputs<'_>) -> Function {
    process::compile_scan_aligned_pointer_range(inputs.abi, inputs.memory.scratch().scan)
}

pub(super) fn build_scan_relative32_target_range(inputs: &RuntimeHelperInputs<'_>) -> Function {
    process::compile_scan_relative32_target_range(
        inputs.plan.function(RuntimeHelperId::ScanProcessRange),
        inputs.plan.function(RuntimeHelperId::ReadRelative32),
    )
}

pub(super) fn build_read_relative32(inputs: &RuntimeHelperInputs<'_>) -> Function {
    process::compile_read_relative32(inputs.abi, inputs.memory.scratch().abi_read)
}

pub(super) fn build_read_utf8_string(inputs: &RuntimeHelperInputs<'_>) -> Function {
    process::compile_read_utf8_string(
        inputs.abi,
        inputs.plan.function(RuntimeHelperId::Utf8StringFromMemory),
        inputs.gc,
        inputs.memory.scratch().native_utf8,
    )
}

pub(super) fn build_utf8_string_from_memory(inputs: &RuntimeHelperInputs<'_>) -> Function {
    process::compile_utf8_string_from_memory(
        inputs.plan.function(RuntimeHelperId::StringFromMemory),
        inputs.gc,
        inputs.memory.scratch().native_utf8,
    )
}

pub(super) fn build_utf16_string_from_memory(inputs: &RuntimeHelperInputs<'_>) -> Function {
    let scratch = inputs.memory.scratch();
    process::compile_utf16_string_from_memory(inputs.gc, scratch.utf16_input, scratch.utf16_output)
}

pub(super) fn build_utf16_le_string_from_memory(inputs: &RuntimeHelperInputs<'_>) -> Function {
    process::compile_utf16_le_string_from_memory(
        inputs.plan.function(RuntimeHelperId::Utf16StringFromMemory),
        inputs.gc,
        inputs.memory.scratch().utf16_input,
    )
}

pub(super) fn build_read_utf16_le_string(inputs: &RuntimeHelperInputs<'_>) -> Function {
    process::compile_read_utf16_le_string(
        inputs.abi,
        inputs
            .plan
            .function(RuntimeHelperId::Utf16LeStringFromMemory),
        inputs.gc,
        inputs.memory.scratch().utf16_input,
    )
}

pub(super) fn build_managed_field_address(_inputs: &RuntimeHelperInputs<'_>) -> Function {
    process::compile_managed_field_address()
}

pub(super) fn build_read_managed_memory(inputs: &RuntimeHelperInputs<'_>) -> Function {
    process::compile_read_managed_memory(inputs.abi)
}

pub(super) fn build_charge_managed_bytes(inputs: &RuntimeHelperInputs<'_>) -> Function {
    managed_context::charge_bytes(inputs.gc)
}

pub(super) fn build_read_managed_string(inputs: &RuntimeHelperInputs<'_>) -> Function {
    let scratch = inputs.memory.scratch();
    process::compile_read_managed_string(
        inputs.abi,
        inputs.gc,
        scratch.abi_read,
        scratch.utf16_input,
        scratch.utf16_output,
        inputs.plan.function(RuntimeHelperId::ChargeManagedBytes),
    )
}

pub(super) fn build_read_managed_string_field(inputs: &RuntimeHelperInputs<'_>) -> Function {
    build_managed_string_field(inputs, false)
}

pub(super) fn build_read_optional_managed_string_field(
    inputs: &RuntimeHelperInputs<'_>,
) -> Function {
    build_managed_string_field(inputs, true)
}

fn build_managed_string_field(inputs: &RuntimeHelperInputs<'_>, optional: bool) -> Function {
    let Type::Result(result) =
        runtime_helper_registry::managed_string_result_type(inputs.semantics, optional)
    else {
        unreachable!("managed string field helpers return Result values")
    };
    let option = optional.then(|| {
        let string = inputs
            .semantics
            .types()
            .id_for_standard(StdlibTypeId::String);
        inputs
            .semantics
            .types()
            .iter()
            .find_map(|(_, kind)| match kind {
                crate::types::TypeKind::Option { layout, value } if *value == string => {
                    Some(*layout)
                }
                _ => None,
            })
            .expect("optional managed strings have an Option layout")
    });
    process::compile_read_managed_string_field(
        inputs.abi,
        inputs.plan.function(RuntimeHelperId::ReadManagedString),
        inputs.gc,
        inputs.memory.scratch().abi_read,
        result,
        option,
        inputs.failure_payloads,
    )
}

pub(super) fn build_module_path(inputs: &RuntimeHelperInputs<'_>) -> Function {
    process::compile_module_path(
        inputs.abi,
        inputs.plan.function(RuntimeHelperId::StringFromMemory),
        inputs.gc,
        inputs.memory.scratch(),
    )
}

pub(super) fn build_loaded_module(inputs: &RuntimeHelperInputs<'_>) -> Function {
    process::compile_loaded_module(inputs.abi, inputs.gc, inputs.memory.scratch())
}

pub(super) fn build_process_path(inputs: &RuntimeHelperInputs<'_>) -> Function {
    process::compile_process_path(
        inputs.abi,
        inputs.plan.function(RuntimeHelperId::StringFromMemory),
        inputs.gc,
        inputs.memory.scratch(),
    )
}

pub(super) fn build_runtime_operating_system(inputs: &RuntimeHelperInputs<'_>) -> Function {
    process::compile_runtime_metadata(
        inputs.abi,
        crate::abi::AbiImportId::RuntimeGetOs,
        inputs.plan.function(RuntimeHelperId::StringFromMemory),
        inputs.gc,
        inputs.memory.scratch(),
    )
}

pub(super) fn build_runtime_architecture(inputs: &RuntimeHelperInputs<'_>) -> Function {
    process::compile_runtime_metadata(
        inputs.abi,
        crate::abi::AbiImportId::RuntimeGetArch,
        inputs.plan.function(RuntimeHelperId::StringFromMemory),
        inputs.gc,
        inputs.memory.scratch(),
    )
}

pub(super) fn build_join_strings(inputs: &RuntimeHelperInputs<'_>) -> Function {
    let (array, storage) = array_layouts(inputs, Type::Standard(StdlibTypeId::String));
    strings::compile_join_strings(array, storage, inputs.gc)
}

pub(super) fn build_indent_display(inputs: &RuntimeHelperInputs<'_>) -> Function {
    strings::compile_indent_display(inputs.gc)
}

pub(super) fn build_follow_address(inputs: &RuntimeHelperInputs<'_>) -> Function {
    let (array, storage) = array_layouts(inputs, Type::I64);
    process::compile_follow_address(inputs.abi, array, storage, inputs.memory.scratch().abi_read)
}

pub(super) fn build_detect_process_pointer_size(inputs: &RuntimeHelperInputs<'_>) -> Function {
    process::compile_detect_process_pointer_size(inputs.abi, inputs.memory.scratch().abi_read)
}

pub(super) fn build_gba_translate_address(inputs: &RuntimeHelperInputs<'_>) -> Function {
    gba::compile_translate_address(inputs.abi, inputs.gc, inputs.memory.scratch().abi_read)
}

pub(super) fn build_gba_read_memory(inputs: &RuntimeHelperInputs<'_>) -> Function {
    provider::compile_translated_read(
        inputs.abi,
        inputs.plan.function(RuntimeHelperId::GBATranslateAddress),
    )
}

pub(super) fn build_gcn_translate_address(inputs: &RuntimeHelperInputs<'_>) -> Function {
    gcn::compile_translate_address(inputs.abi, inputs.gc, inputs.memory.scratch().abi_read)
}

pub(super) fn build_gcn_read_memory(inputs: &RuntimeHelperInputs<'_>) -> Function {
    provider::compile_translated_read(
        inputs.abi,
        inputs.plan.function(RuntimeHelperId::GCNTranslateAddress),
    )
}

pub(super) fn build_wii_translate_address(inputs: &RuntimeHelperInputs<'_>) -> Function {
    wii::compile_translate_address(inputs.abi, inputs.gc, inputs.memory.scratch().abi_read)
}

pub(super) fn build_wii_read_memory(inputs: &RuntimeHelperInputs<'_>) -> Function {
    provider::compile_translated_read(
        inputs.abi,
        inputs.plan.function(RuntimeHelperId::WiiTranslateAddress),
    )
}

pub(super) fn build_ps2_translate_address(inputs: &RuntimeHelperInputs<'_>) -> Function {
    let provider = inputs
        .gc
        .standard_library
        .state_provider(crate::stdlib::StdlibStateProviderId::Ps2);
    let [range] = provider.readable_ranges else {
        panic!("PS2 runtime translation requires exactly one readable guest-memory range")
    };
    ps2::compile_translate_address(
        inputs.abi,
        inputs.gc,
        inputs.memory.scratch().abi_read,
        *range,
    )
}

pub(super) fn build_ps2_read_memory(inputs: &RuntimeHelperInputs<'_>) -> Function {
    provider::compile_translated_read(
        inputs.abi,
        inputs.plan.function(RuntimeHelperId::Ps2TranslateAddress),
    )
}

pub(super) fn build_ps1_translate_address(inputs: &RuntimeHelperInputs<'_>) -> Function {
    ps1::compile_translate_address(inputs.abi, inputs.gc, inputs.memory.scratch().abi_read)
}

pub(super) fn build_ps1_read_memory(inputs: &RuntimeHelperInputs<'_>) -> Function {
    provider::compile_translated_read(
        inputs.abi,
        inputs.plan.function(RuntimeHelperId::Ps1TranslateAddress),
    )
}

pub(super) fn build_sms_translate_address(inputs: &RuntimeHelperInputs<'_>) -> Function {
    sms::compile_translate_address(inputs.abi, inputs.gc, inputs.memory.scratch().abi_read)
}

pub(super) fn build_sms_read_memory(inputs: &RuntimeHelperInputs<'_>) -> Function {
    provider::compile_translated_read(
        inputs.abi,
        inputs.plan.function(RuntimeHelperId::SmsTranslateAddress),
    )
}

pub(super) fn build_genesis_read_memory(inputs: &RuntimeHelperInputs<'_>) -> Function {
    genesis::compile_read_memory(inputs.abi, inputs.gc)
}

pub(super) fn build_string_from_memory(inputs: &RuntimeHelperInputs<'_>) -> Function {
    settings::compile_string_from_memory(inputs.gc)
}

pub(super) fn build_file_open_read_only(inputs: &RuntimeHelperInputs<'_>) -> Function {
    file::compile_open_read_only(inputs.abi, inputs.gc, inputs.memory.scratch())
}

pub(super) fn build_file_read_all_storage(inputs: &RuntimeHelperInputs<'_>) -> Function {
    file::compile_read_all_storage(
        inputs.abi,
        inputs.plan.function(RuntimeHelperId::FileOpenReadOnly),
        inputs.gc,
        inputs.memory.scratch(),
    )
}

pub(super) fn build_file_read_all_bytes(inputs: &RuntimeHelperInputs<'_>) -> Function {
    let array = inputs
        .arrays
        .iter()
        .find(|array| try_array_element_type(array.id, inputs.semantics) == Some(Type::U8))
        .expect("File.readAllBytes requires a reachable [u8] layout")
        .id;
    let storage = super::array_value::storage_id(array, inputs.arrays, inputs.semantics);
    file::compile_read_all_bytes(
        inputs.plan.function(RuntimeHelperId::FileReadAllStorage),
        array,
        storage,
        inputs.gc,
    )
}

pub(super) fn build_utf8_string_from_storage(inputs: &RuntimeHelperInputs<'_>) -> Function {
    file::compile_utf8_string_from_storage(inputs.gc)
}

pub(super) fn build_file_read_all_text(inputs: &RuntimeHelperInputs<'_>) -> Function {
    file::compile_read_all_text(
        inputs.plan.function(RuntimeHelperId::FileReadAllStorage),
        inputs.plan.function(RuntimeHelperId::Utf8StringFromStorage),
        inputs.gc,
    )
}

pub(super) fn build_md5_update_blocks(_inputs: &RuntimeHelperInputs<'_>) -> Function {
    md5::compile_update_blocks()
}

pub(super) fn build_md5_format(inputs: &RuntimeHelperInputs<'_>) -> Function {
    md5::compile_format(inputs.gc)
}

pub(super) fn build_module_md5_poll(inputs: &RuntimeHelperInputs<'_>) -> Function {
    md5::compile_module_poll(
        inputs.abi,
        inputs.plan.function(RuntimeHelperId::FileOpenReadOnly),
        inputs.plan.function(RuntimeHelperId::Md5UpdateBlocks),
        inputs.plan.function(RuntimeHelperId::Md5Format),
        inputs.gc,
        inputs.memory.scratch(),
    )
}

pub(super) fn build_refresh_settings(inputs: &RuntimeHelperInputs<'_>) -> Function {
    settings::compile_refresh_settings(
        inputs.program,
        inputs.settings,
        inputs.strings,
        inputs.settings_map,
        inputs
            .plan
            .optional_function(RuntimeHelperId::StringFromMemory)
            .unwrap_or(0),
        inputs
            .plan
            .optional_function(RuntimeHelperId::StringEquality)
            .unwrap_or(0),
        inputs.memory.scratch(),
    )
}

pub(super) fn build_settings_enabled(inputs: &RuntimeHelperInputs<'_>) -> Function {
    settings::compile_settings_enabled(inputs.program, inputs.settings_map, inputs.gc)
}

pub(super) fn build_settings_contains(inputs: &RuntimeHelperInputs<'_>) -> Function {
    settings::compile_settings_contains(inputs.program, inputs.gc)
}
