//! Exercise private metadata adapters through a compiler-owned probe function.
//! The public language intentionally exposes managed schemas, not these adapters.

use crate::*;

#[test]
fn managed_collection_layout_runtime() {
    for (backend, source) in [
        ("mono", include_str!("../tests/mono_list_layout.split")),
        ("il2cpp", include_str!("../tests/il2cpp_list_layout.split")),
    ] {
        let mut parsed = parse(source).unwrap();
        // This fixture deliberately occupies the same reserved namespace as
        // generated binders. No user-source restriction changes in production.
        assert!(!parsed.resolution_diagnostics.is_empty());
        for diagnostic in &parsed.resolution_diagnostics {
            assert!(diagnostic.message.contains("reserved"), "{diagnostic:?}");
        }
        parsed.resolution_diagnostics.clear();
        let checked = check(parsed).unwrap_or_else(|errors| panic!("{errors:#?}"));
        for profile in [BuildProfile::Debug, BuildProfile::Release] {
            let wasm = codegen_with_options(
                &checked,
                CompilerOptions {
                    profile,
                    ..Default::default()
                },
            );
            wasmparser::Validator::new_with_features(wasmparser::WasmFeatures::all())
                .validate_all(&wasm)
                .unwrap();
            let output = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
                .join("target")
                .join(format!("{backend}-list-layout-{profile:?}.wasm"));
            std::fs::write(&output, wasm).unwrap();
            let result = std::process::Command::new("node")
                .current_dir(env!("CARGO_MANIFEST_DIR"))
                .arg("tests/managed_list_layout_runtime.mjs")
                .arg(output)
                .arg(backend)
                .output()
                .expect("managed runtime fixtures require Node.js");
            assert!(
                result.status.success(),
                "{}\n{}",
                String::from_utf8_lossy(&result.stdout),
                String::from_utf8_lossy(&result.stderr)
            );
            println!("{}", String::from_utf8_lossy(&result.stdout));
        }
    }
}
