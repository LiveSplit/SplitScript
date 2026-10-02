use super::GcStringLiterals;
use wasm_encoder::{
    CodeSection, DataCountSection, DataSection, Function, FunctionSection, Instruction as I,
    Module, StorageType, TypeSection,
};

fn literal_module(values: &[&str], enabled: bool, preceding_segments: u32) -> Vec<u8> {
    let mut literals = GcStringLiterals::default();
    if enabled {
        literals.enable(preceding_segments);
    }
    let mut types = TypeSection::new();
    types.ty().array(&StorageType::I8, true);
    types.ty().function([], []);
    let mut functions = FunctionSection::new();
    functions.function(1);
    let mut function = Function::new([]);
    for value in values {
        if let Some((segment, offset)) = literals.intern(value) {
            function
                .instruction(&I::I32Const(offset as i32))
                .instruction(&I::I32Const(value.len() as i32))
                .instruction(&I::ArrayNewData {
                    array_type_index: 0,
                    array_data_index: segment,
                });
        } else {
            for byte in value.bytes() {
                function.instruction(&I::I32Const(i32::from(byte)));
            }
            function.instruction(&I::ArrayNewFixed {
                array_type_index: 0,
                array_size: value.len() as u32,
            });
        }
        function.instruction(&I::Drop);
    }
    function.instruction(&I::End);
    let mut code = CodeSection::new();
    code.function(&function);
    let mut data = DataSection::new();
    for _ in 0..preceding_segments {
        data.passive([]);
    }
    let has_literals = literals.append_to(&mut data);
    let mut module = Module::new();
    module.section(&types).section(&functions);
    if has_literals {
        module.section(&DataCountSection { count: data.len() });
    }
    module.section(&code).section(&data);
    let bytes = module.finish();
    wasmparser::Validator::new_with_features(wasmparser::WasmFeatures::all())
        .validate_all(&bytes)
        .unwrap();
    bytes
}

#[test]
fn literal_costs_account_for_headers_utf8_and_leb_boundaries() {
    // Check actual complete encoded modules, including all section and body
    // size prefixes, at both signed-constant and unsigned-index boundaries.
    for segments in [0, 1, 127, 128] {
        for character in ['\0', '?', '@', 'é', '🦊'] {
            for length in 0..40 {
                let value = character.to_string().repeat(length);
                for values in [vec![value.as_str()], vec![value.as_str(); 3]] {
                    let fixed = literal_module(&values, false, segments);
                    let pooled = literal_module(&values, true, segments);
                    assert!(
                        pooled.len() <= fixed.len(),
                        "{segments}/{character:?}/{length}"
                    );
                }
            }
        }
        for padding in [63, 64, 127, 128, 8191, 8192] {
            let prefix = "p".repeat(padding);
            let values = [
                prefix.as_str(),
                "metadata field",
                "metadata field",
                "12345678",
            ];
            assert!(
                literal_module(&values, true, segments).len()
                    < literal_module(&values, false, segments).len()
            );
        }
    }
}

#[test]
fn profitable_short_literals_share_bytes_without_pooling_tiny_values() {
    let mut literals = GcStringLiterals::default();
    assert_eq!(literals.intern("metadata field"), None);
    literals.enable(1);
    assert_eq!(literals.intern(""), None);
    assert_eq!(literals.intern("short"), None);
    assert_eq!(literals.intern("metadata field"), Some((1, 0)));
    assert_eq!(literals.intern("metadata field"), Some((1, 0)));
    assert_eq!(literals.pool.borrow().bytes, b"metadata field");
}
