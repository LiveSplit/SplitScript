use std::borrow::Cow;

use wasm_encoder::{
    CodeSection, CustomSection, DataCountSection, ElementSection, Elements, ExportKind,
    ExportSection, FunctionSection, GlobalSection, ImportSection, MemorySection, MemoryType,
    Module, TypeSection,
};

use super::{
    data_plan::{GcStringLiterals, StaticData},
    debug_artifacts::DebugArtifactPlan,
};

pub(super) struct Sections {
    pub types: TypeSection,
    pub imports: ImportSection,
    pub functions: FunctionSection,
    pub globals: GlobalSection,
    pub referenced_functions: Vec<u32>,
    pub codes: CodeSection,
}

pub(super) fn finish(
    sections: Sections,
    data: &StaticData,
    string_literals: &GcStringLiterals,
    start_function: u32,
    update_function: u32,
    debug: Option<&DebugArtifactPlan>,
) -> Vec<u8> {
    let Sections {
        types,
        imports,
        functions,
        globals,
        referenced_functions,
        codes,
    } = sections;
    let mut memories = MemorySection::new();
    memories.memory(MemoryType {
        minimum: data.layout().minimum_pages(),
        maximum: None,
        memory64: false,
        shared: false,
        page_size_log2: None,
    });
    let mut exports = ExportSection::new();
    exports.export("memory", ExportKind::Memory, 0);
    exports.export("_start", ExportKind::Func, start_function);
    exports.export("update", ExportKind::Func, update_function);
    let mut data = data.encode();
    let has_passive_strings = string_literals.append_to(&mut data);
    let mut elements = ElementSection::new();
    if !referenced_functions.is_empty() {
        elements.declared(Elements::Functions(Cow::Owned(referenced_functions)));
    }

    let mut module = Module::new();
    module.section(&types);
    module.section(&imports);
    module.section(&functions);
    module.section(&memories);
    module.section(&globals);
    module.section(&exports);
    if !elements.is_empty() {
        module.section(&elements);
    }
    if has_passive_strings {
        module.section(&DataCountSection { count: data.len() });
    }
    module.section(&codes);
    module.section(&data);
    if let Some(debug) = debug {
        module.section(debug.names());
        for section in debug.dwarf() {
            module.section(&CustomSection {
                name: Cow::Borrowed(section.name),
                data: Cow::Borrowed(&section.data),
            });
        }
    }
    module.section(&CustomSection {
        name: Cow::Borrowed("splitscript"),
        data: Cow::Owned(crate::build_identity::module_metadata()),
    });
    module.finish()
}
