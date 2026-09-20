//! Exercise private metadata adapters through a compiler-owned probe function.
//! The public language intentionally exposes managed schemas, not these adapters.

use crate::*;

#[test]
fn managed_collection_layout_runtime() {
    run_layout_fixtures(
        "list",
        "tests/managed_list_layout_runtime.mjs",
        [
            ("mono", include_str!("../tests/mono_list_layout.split")),
            ("il2cpp", include_str!("../tests/il2cpp_list_layout.split")),
        ],
    );
}

#[test]
fn managed_keyed_collection_layout_runtime() {
    run_layout_fixtures(
        "keyed",
        "tests/managed_keyed_layout_runtime.mjs",
        [
            ("mono", include_str!("../tests/mono_keyed_layout.split")),
            ("il2cpp", include_str!("../tests/il2cpp_keyed_layout.split")),
        ],
    );
}

#[test]
fn managed_keyed_collection_slots_runtime() {
    run_layout_fixtures(
        "slots",
        "tests/managed_keyed_slots_runtime.mjs",
        [
            ("mono", include_str!("../tests/mono_keyed_reads.split")),
            ("il2cpp", include_str!("../tests/il2cpp_keyed_reads.split")),
        ],
    );
}

#[test]
fn mono_static_field_paths_preserve_declaring_owners() {
    run_layout_fixtures(
        "static-path",
        "tests/mono_static_path_runtime.mjs",
        [("mono", include_str!("../tests/mono_static_path.split"))],
    );
}

#[test]
fn il2cpp_plain_storage_profiles() {
    run_layout_fixtures(
        "plain-storage",
        "tests/il2cpp_plain_storage_runtime.mjs",
        [(
            "il2cpp",
            include_str!("../tests/il2cpp_plain_storage.split"),
        )],
    );
}

#[test]
fn il2cpp_generic_storage_profiles() {
    run_layout_fixtures(
        "generic-storage",
        "tests/il2cpp_generic_storage_runtime.mjs",
        [(
            "il2cpp",
            include_str!("../tests/il2cpp_generic_storage.split"),
        )],
    );
}

fn run_layout_fixtures<const N: usize>(kind: &str, harness: &str, fixtures: [(&str, &str); N]) {
    for (backend, source) in fixtures {
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
                .join(format!("{backend}-{kind}-layout-{profile:?}.wasm"));
            std::fs::write(&output, wasm).unwrap();
            let result = std::process::Command::new("node")
                .current_dir(env!("CARGO_MANIFEST_DIR"))
                .arg(harness)
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
