//! profiles codegen integration tests.

use super::catalogs_types::TypedExpressionCounter;
use super::*;

#[test]
fn explicit_il2cpp_profiles_omit_the_measured_catalog_and_version_lookup() {
    let source = include_str!("../il2cpp_profile_custom.split");
    let (wasm, report) = release_emission(source);
    Validator::new_with_features(WasmFeatures::all())
        .validate_all(&wasm)
        .unwrap();
    assert!(!contains_string_literal(&wasm, b"UnityPlayer.dll"));
    for (_, function) in &report.functions {
        assert!(
            !function.contains("Il2CppProfileSelect") && !function.contains("Il2CppProfileUnity"),
            "custom profile retained catalog: {function}"
        );
    }
}

#[test]
fn explicit_il2cpp_width_omits_opposite_discovery() {
    for (width, constructor) in [
        (64, "Il2CppProfile.unity2022_3_0f1X64()"),
        (32, "Il2CppProfile.unity2022_3_0f1X86()"),
    ] {
        for alias in [false, true] {
            let declaration = if alias {
                format!("fn profile() -> Il2CppProfile {{ return {constructor} }}")
            } else {
                String::new()
            };
            let argument = if alias { "profile()" } else { constructor };
            let source = format!(
                "{declaration}\nimage \"Assembly-CSharp\" {{ class Probe {{ static i32 value; }} }}\n\
                 state Unity.il2cpp({argument}) [\"game.exe\"] {{ value = Probe.value?; }}"
            );
            let (wasm, report) = release_emission(&source);
            Validator::new_with_features(WasmFeatures::all())
                .validate_all(&wasm)
                .unwrap();
            let retained =
                |part: &str| report.functions.iter().any(|(_, name)| name.contains(part));
            assert!(retained(&format!("DiscoverIl2Cpp{width}::poll")));
            assert!(!retained(&format!(
                "DiscoverIl2Cpp{}::poll",
                if width == 64 { 32 } else { 64 }
            )));
            assert_eq!(retained("Il2CppTable32::poll"), width == 32);
            assert_eq!(retained("Il2CppNameReference32::poll"), width == 32);
            assert!(
                !retained("Il2CppProfileIsValid"),
                "Release must omit authoring-only profile validation"
            );
        }
    }
    let custom = include_str!("../il2cpp_profile_custom.split");
    let (_, report) = release_emission(custom);
    assert!(
        !report
            .functions
            .iter()
            .any(|(_, name)| name.contains("Il2CppProfileIsValid")),
        "Release custom profiles must omit authoring-only validation"
    );
    let checked =
        splitscript::check(splitscript::lower(splitscript::parse(custom).unwrap())).unwrap();
    let (_, debug) = splitscript::compiler::codegen_with_report(
        &checked,
        splitscript::CompilerOptions {
            profile: splitscript::BuildProfile::Debug,
            ..Default::default()
        },
    );
    assert!(
        debug
            .functions
            .iter()
            .any(|(_, name)| name.contains("Il2CppProfileIsValid")),
        "Debug custom profiles must retain authoring validation"
    );
    assert!(
        !report
            .functions
            .iter()
            .any(|(_, name)| name.contains("Il2CppTable32::poll"))
    );
}

fn release_emission(source: &str) -> (Vec<u8>, splitscript::compiler::CodegenReport) {
    let checked =
        splitscript::check(splitscript::lower(splitscript::parse(source).unwrap())).unwrap();
    splitscript::compiler::codegen_with_report(
        &checked,
        splitscript::CompilerOptions {
            profile: splitscript::BuildProfile::Release,
            ..Default::default()
        },
    )
}

#[test]
fn static_strings_follow_abi_demand_instead_of_expression_reachability() {
    let source = r#"
        state "game.exe" {}
        fn probe() -> u32! { throw "discarded metadata error" }
        onAttach {
            let module = await process.module("required.dll")
            print("GC-only literal")
            print(`GC-only interpolation {module.address}`)
            print(probe() else 0)
        }
    "#;
    let checked = splitscript::check(splitscript::parse(source).unwrap()).unwrap();
    for profile in [
        splitscript::BuildProfile::Debug,
        splitscript::BuildProfile::Release,
    ] {
        let (wasm, report) = splitscript::compiler::codegen_with_report(
            &checked,
            splitscript::CompilerOptions {
                profile,
                ..Default::default()
            },
        );
        Validator::new_with_features(WasmFeatures::all())
            .validate_all(&wasm)
            .unwrap();
        assert_eq!(
            report.static_data_end - u64::from(report.static_data_start),
            ("game.exe".len() + "required.dll".len()) as u64
        );
        assert!(contains_string_literal(&wasm, b"GC-only literal"));
        assert!(contains_string_literal(&wasm, b"GC-only interpolation "));
        let data: Vec<_> = Parser::new(0)
            .parse_all(&wasm)
            .filter_map(|payload| {
                if let Payload::DataSection(section) = payload.unwrap() {
                    Some(
                        section
                            .into_iter()
                            .flat_map(|segment| segment.unwrap().data.to_vec())
                            .collect::<Vec<_>>(),
                    )
                } else {
                    None
                }
            })
            .flatten()
            .collect();
        assert!(
            data.windows(b"required.dll".len())
                .any(|bytes| bytes == b"required.dll")
        );
        for absent in [b"GC-only".as_slice(), b"discarded metadata error"] {
            assert!(!data.windows(absent.len()).any(|bytes| bytes == absent));
        }
    }
}

#[test]
fn release_long_gc_literals_use_demanded_passive_data() {
    let marker = "metadata literal 🦊 repeated across allocations, long enough for passive data";
    let source = format!(
        r#"
        state "game.exe" {{}}
        fn first() -> String {{ return "{marker}" }}
        fn second() -> String {{ return "{marker}" }}
        fn discarded() -> u32! {{ throw "unused metadata payload that must not reach passive data" }}
        whileAttached {{ print(first()); print(second()); print(discarded() else 0) }}
    "#
    );
    let checked = splitscript::check(splitscript::parse(&source).unwrap()).unwrap();
    for profile in [
        splitscript::BuildProfile::Debug,
        splitscript::BuildProfile::Release,
    ] {
        let (wasm, report) = splitscript::compiler::codegen_with_report(
            &checked,
            splitscript::CompilerOptions {
                profile,
                ..Default::default()
            },
        );
        Validator::new_with_features(WasmFeatures::all())
            .validate_all(&wasm)
            .unwrap();
        let mut passive = Vec::new();
        let mut data_arrays = 0;
        let mut has_data_count = false;
        for payload in Parser::new(0).parse_all(&wasm) {
            match payload.unwrap() {
                Payload::DataSection(section) => {
                    for segment in section {
                        let segment = segment.unwrap();
                        if matches!(segment.kind, wasmparser::DataKind::Passive) {
                            passive.extend_from_slice(segment.data);
                        }
                    }
                }
                Payload::DataCountSection { .. } => has_data_count = true,
                Payload::CodeSectionEntry(body) => {
                    for op in body.get_operators_reader().unwrap() {
                        if matches!(op.unwrap(), wasmparser::Operator::ArrayNewData { .. }) {
                            data_arrays += 1;
                        }
                    }
                }
                _ => {}
            }
        }
        let release = profile == splitscript::BuildProfile::Release;
        assert_eq!(has_data_count, release);
        assert_eq!(data_arrays, if release { 2 } else { 0 });
        assert_eq!(passive, if release { marker.as_bytes() } else { &[] });
        assert_eq!(
            report.static_data_end - u64::from(report.static_data_start),
            "game.exe".len() as u64
        );
    }

    let small = r#"state "game.exe" {} whileAttached { print("short") }"#;
    let (baseline, _) = release_emission(small);
    let unused = format!(r#"{small} fn unused() {{ print("{marker}") }}"#);
    assert_eq!(baseline, release_emission(&unused).0);
    assert!(
        !Parser::new(0)
            .parse_all(&baseline)
            .any(|p| matches!(p.unwrap(), Payload::DataCountSection { .. }))
    );
}

#[test]
fn flat_schema_names_omit_nested_matching_and_unused_nested_declarations() {
    for provider in [
        "Unity",
        "Unity.mono(MonoVersion.V2)",
        "Unity.il2cpp(Il2CppProfile.unity2022_3_0f1X64())",
    ] {
        let source = format!(
            r#"
            image "Assembly-CSharp" {{ class Probe {{ static i32 value; }} }}
            state {provider} ["game.exe"] {{ value = Probe.value?; }}
        "#
        );
        let (wasm, report) = release_emission(&source);
        assert!(
            !report
                .functions
                .iter()
                .any(|(_, name)| name.ends_with("UnityClassNamesMatchesFlat"))
        );
        assert!(
            !report
                .functions
                .iter()
                .any(|(_, name)| name.ends_with("UnityClassNamesMatches"))
        );
        assert!(!contains_string_literal(
            &wasm,
            b"Unity profile lacks nested class metadata"
        ));
        let unused = format!(
            "{source}\nimage \"Unused\" {{ class Nested from \"Other.Outer+Leaf\" {{ static i32 value; }} }}"
        );
        assert_eq!(wasm, release_emission(&unused).0);
        // Either a declaration namespace or a qualified alias needs the flat
        // matcher. Bare aliases keep their existing namespace-agnostic meaning.
        for qualified in [
            source.replace("class Probe {", "class Probe from \"Game.Probe\" {"),
            source.replace("class Probe {", "class Probe from [\"Probe\", \"Game.Probe\"] {"),
            source.replace("image \"Assembly-CSharp\" { class Probe { static i32 value; } }",
                "image \"Assembly-CSharp\" { namespace Game { class Probe { static i32 value; } } }"),
        ] {
            let (wasm, report) = release_emission(&qualified);
            assert!(report.functions.iter().any(|(_, name)| name.ends_with("UnityClassNamesMatchesFlat")));
            Validator::new_with_features(WasmFeatures::all()).validate_all(&wasm).unwrap();
        }
        let nested = source.replace("class Probe {", "class Probe from \"Game.Outer+Probe\" {");
        let (wasm, report) = release_emission(&nested);
        assert!(
            report
                .functions
                .iter()
                .any(|(_, name)| name.ends_with("UnityClassNamesMatches"))
        );
        Validator::new_with_features(WasmFeatures::all())
            .validate_all(&wasm)
            .unwrap();
    }
}

#[test]
fn explicit_mono_families_exclude_build_identity_discovery() {
    for selector in ["mono", "monoLinux", "monoMac"] {
        for family in ["V1", "V1Cattrs", "V2", "V3"] {
            let source = include_str!("../mono_profiles.split").replace(
                "state Unity",
                &format!("state Unity.{selector}(MonoVersion.{family})"),
            );
            let (wasm, report) = release_emission(&source);
            Validator::new_with_features(WasmFeatures::all())
                .validate_all(&wasm)
                .unwrap();
            for (_, name) in &report.functions {
                for excluded in [
                    "MonoLayoutForBuild",
                    "MonoLayoutBuild",
                    "MonoLayoutForLinuxBuild",
                    "MonoLayoutLinuxBuild",
                    "MonoLayoutForMacBuild",
                    "MonoLayoutMacBuild",
                    "ModulePeDebugId",
                    "ModuleElfBuildId",
                    "ModuleMachUuid",
                    "DetectUnixVersion",
                    "DetectMonoVersion",
                    "DetectOldLayout",
                    "Il2Cpp",
                ] {
                    assert!(
                        !name.contains(excluded),
                        "explicit {selector}/{family} retained {name}"
                    );
                }
            }
            for (platform, export, layouts) in [
                ("mono", "ModulePeExport", "MonoLayoutForVersion"),
                ("monoLinux", "ModuleElfExport", "MonoLayoutForLinuxVersion"),
                ("monoMac", "ModuleMachExport", "MonoLayoutForMacVersion"),
            ] {
                for name in [export, layouts] {
                    assert_eq!(
                        report
                            .functions
                            .iter()
                            .any(|(_, function)| function.contains(name)),
                        selector == platform,
                        "wrong platform dependency {name} for {selector}/{family}"
                    );
                }
            }
            for player in [
                "UnityPlayer.dll",
                "UnityPlayer.so",
                "UnityPlayer.dylib",
                "GameAssembly.dll",
            ] {
                assert!(
                    !contains_string_literal(&wasm, player.as_bytes()),
                    "explicit {selector}/{family} retained {player}"
                );
            }
        }
    }
    let (_, report) = release_emission(include_str!("../mono_profiles.split"));
    assert!(
        report
            .functions
            .iter()
            .any(|(_, name)| name.contains("MonoLayoutForBuild"))
    );
}

#[test]
fn binary_identity_readers_follow_the_requested_format() {
    let readers = ["ModulePeDebugId", "ModuleElfBuildId", "ModuleMachUuid"];
    for (source, expected) in [
        ("state \"game.exe\" {}", None),
        (include_str!("../pe_debug_id.split"), Some(readers[0])),
        (include_str!("../elf_build_id.split"), Some(readers[1])),
        (include_str!("../mach_uuid.split"), Some(readers[2])),
    ] {
        let (wasm, report) = release_emission(source);
        Validator::new_with_features(WasmFeatures::all())
            .validate_all(&wasm)
            .unwrap();
        for reader in readers {
            assert_eq!(
                report
                    .functions
                    .iter()
                    .any(|(_, name)| name.ends_with(reader)),
                expected == Some(reader),
                "unexpected demand for {reader}"
            );
        }
    }
}

#[test]
fn managed_metadata_demand_ignores_dead_and_debug_reads() {
    for provider in [
        "Unity.il2cpp(Il2CppProfile.unity2021_3_11f1X64())",
        "Unity.mono(MonoVersion.V2)",
        "Unity",
    ] {
        let source = format!(
            r#"
            image "Assembly-CSharp" {{
                class Probe {{ static String unobservedText; static i32 value; }}
                class UnobservedClass {{ static i32 absent; }}
            }}
            image "UnobservedImage" {{ class AnotherAbsentClass {{ static i32 absent; }} }}
            state {provider} ["game.exe"] {{ value = Probe.value?; }}
            fn dead() {{ return Probe.unobservedText }}
            fn deadInstances() {{ return await UnobservedClass.instances() }}
            whileAttached {{ debug print(Probe.unobservedText) }}
        "#
        );
        let (wasm, report) = release_emission(&source);
        Validator::new_with_features(WasmFeatures::all())
            .validate_all(&wasm)
            .unwrap();
        for name in [
            "unobservedText",
            "UnobservedClass",
            "UnobservedImage",
            "AnotherAbsentClass",
        ] {
            assert!(
                !contains_string_literal(&wasm, name.as_bytes()),
                "retained {name} in {provider}"
            );
        }
        assert!(
            report
                .runtime_helpers
                .iter()
                .all(|name| !name.contains("ReadManagedString"))
        );
        let debug = splitscript::compile_with_options(
            &source,
            splitscript::CompilerOptions {
                profile: splitscript::BuildProfile::Debug,
                ..Default::default()
            },
        )
        .unwrap();
        Validator::new_with_features(WasmFeatures::all())
            .validate_all(&debug)
            .unwrap();
        assert!(
            debug_function_names(&debug)
                .unwrap()
                .1
                .iter()
                .any(|(_, name)| name.contains("ReadManagedString"))
        );
    }
}

#[test]
fn class_verification_follows_reachable_snapshots_and_live_reads() {
    for selector in [
        "Unity.mono(MonoVersion.V2)",
        "Unity.il2cpp(Il2CppProfile.unity2022_3_0f1X64())",
        "Unity",
    ] {
        for (expression, snapshot) in [
            ("Probe.value?", false),
            ("Probe.instance?.value?", true),
            ("Probe.instance?.snapshot()?", true),
            ("Empty.instance?.snapshot()?", true),
        ] {
            let source = format!(
                r#"
                image "Assembly-CSharp" {{
                    class Probe {{ static Probe instance; i32 value; }}
                    class Empty {{ static Empty instance; }}
                    class UnusedSnapshot {{ static UnusedSnapshot instance; }}
                }}
                fn unused() -> UnusedSnapshot! {{ return UnusedSnapshot.instance?.snapshot() }}
                state {selector} ["game.exe"] {{ value = {expression}; }}
                "#
            );
            // The scalar-only case uses a static field instead of a live object.
            let source = if expression == "Probe.value?" {
                source.replace("i32 value;", "static i32 value;")
            } else {
                source
            };
            let (wasm, report) = release_emission(&source);
            Validator::new_with_features(WasmFeatures::all())
                .validate_all(&wasm)
                .unwrap();
            assert_eq!(
                report
                    .functions
                    .iter()
                    .any(|(_, name)| name.contains("ParentClass")),
                snapshot,
                "{selector}: {expression}"
            );
            assert!(!contains_string_literal(&wasm, b"UnusedSnapshot"));
            assert_eq!(
                report
                    .functions
                    .iter()
                    .any(|(_, name)| name.contains("ObjectClass")),
                snapshot,
                "{selector}: {expression}"
            );
        }
    }
}

#[test]
fn managed_snapshot_demand_keeps_unprojected_instance_fields() {
    let source = r#"
        image "Assembly-CSharp" {
            class Probe { static Probe instance; i32 value; String snapshotText; }
        }
        state Unity.il2cpp(Il2CppProfile.unity2021_3_11f1X64()) ["game.exe"] { probe = Probe.instance?.snapshot()?; }
        whileAttached { print(current.probe.value) }
    "#;
    let (wasm, report) = release_emission(source);
    Validator::new_with_features(WasmFeatures::all())
        .validate_all(&wasm)
        .unwrap();
    assert!(contains_string_literal(&wasm, b"snapshotText"));
    assert!(
        report
            .runtime_helpers
            .iter()
            .any(|name| name.contains("ReadManagedString"))
    );
}

#[test]
fn managed_metadata_keeps_automatic_evidence_but_prunes_unused_explicit_shape_fields() {
    let source = r#"
        enum Edition { Base, Demo }
        let edition: Edition
        image "Assembly-CSharp" {
            class Probe {
                static i32 value;
                if edition == Edition.Base { i32 evidenceBase; }
                else { i32 evidenceDemo; }
            }
        }
        state Unity.il2cpp(Il2CppProfile.unity2021_3_11f1X64()) ["game.exe"] { value = Probe.value?; }
    "#;
    for (suffix, present) in [("", true), ("onAttach { edition = Edition.Base }", false)] {
        let (wasm, _) = release_emission(&format!("{source}\n{suffix}"));
        Validator::new_with_features(WasmFeatures::all())
            .validate_all(&wasm)
            .unwrap();
        for name in [b"evidenceBase", b"evidenceDemo"] {
            assert_eq!(contains_string_literal(&wasm, name), present);
        }
    }
}

#[test]
fn scratch_reservations_follow_reachable_operations() {
    let (_, native) = release_emission("state \"game.exe\" {}");
    assert_eq!(native.scratch_bytes, 0);
    assert_eq!(native.abi_read_capacity, 0);
    assert_eq!(native.static_data_start, 0);
    assert_eq!(native.minimum_memory_pages, 1);
    for source in [
        include_str!("../map_runtime.split"),
        include_str!("../set_runtime.split"),
    ] {
        assert_eq!(release_emission(source).1.scratch_bytes, 0);
    }
    let (_, scalar) = release_emission("state \"game.exe\" { value: i32 at 0x1234 }");
    assert_eq!(scalar.scratch_bytes, 16);
    let managed = |field| {
        format!(
            r#"
        image "Assembly-CSharp" {{ class Probe {{ {field} }} }}
        state Unity.il2cpp(Il2CppProfile.unity2021_3_11f1X64()) ["game.exe"] {{ value = Probe.value?; }}
    "#
        )
    };
    assert_eq!(
        release_emission(&managed("static i32 value;"))
            .1
            .scratch_bytes,
        4120
    );
    assert_eq!(
        release_emission(&managed("static String value;"))
            .1
            .scratch_bytes,
        4120 + 6144
    );
}

#[test]
fn unread_memory_layouts_and_dead_reads_do_not_inflate_scratch() {
    let original = "state \"game.exe\" { value: i32 at 0x1234 }";
    let source = format!(
        r#"{original}
        struct Unused {{ bytes: [u8; 4096] }}
        fn unused() {{ return process.read<Unused>(0x5678) }}
    "#
    );
    let (before, baseline) = release_emission(original);
    let (after, report) = release_emission(&source);
    assert_eq!(baseline, report);
    assert!(
        before == after,
        "unused layouts/reads changed Release bytes"
    );
    let used = format!(
        r#"{source} whileAttached {{ let value = unused() else return; print(value.bytes[0]); }}"#
    );
    assert_eq!(release_emission(&used).1.abi_read_capacity, 4096);
}

#[test]
fn scratch_uses_the_readers_pointer_width_and_normalization_headroom() {
    let (_, native) =
        release_emission(r#"state "game.exe" { pointers: [address; 1024] at 0x1234 }"#);
    let (_, gba) = release_emission("state GBA { pointers: [address; 1024] at 0x02000000 }");
    assert_eq!(native.abi_read_capacity, 8192);
    assert_eq!(gba.abi_read_capacity, 4096);
    let (_, genesis) = release_emission("state Genesis { bytes: [u8; 4096] at 0xFF0001 }");
    assert_eq!(genesis.abi_read_capacity, 4098);
}

#[test]
fn codegen_report_leaves_debug_and_release_artifacts_unchanged() {
    use splitscript::{BuildProfile, CompilerOptions, compiler};

    for source in [
        "state \"game.exe\" {}",
        include_str!("../../examples/lunistice.split"),
    ] {
        let checked =
            splitscript::check(splitscript::lower(splitscript::parse(source).unwrap())).unwrap();
        for profile in [BuildProfile::Debug, BuildProfile::Release] {
            let options = CompilerOptions {
                profile,
                ..Default::default()
            };
            let ordinary = compiler::codegen_with_options(&checked, options);
            let (reported, report) = compiler::codegen_with_report(&checked, options);
            // Existing DWARF global order depends on a HashMap. Compare all
            // runtime/name/metadata sections byte-for-byte in Debug as well;
            // Release has no DWARF and is compared as a complete artifact.
            let stable_sections = |wasm: &[u8]| {
                Parser::new(0).parse_all(wasm).filter_map(|payload| {
                    let payload = payload.unwrap();
                    if matches!(&payload, Payload::CustomSection(section) if section.name().starts_with(".debug_")) {
                        return None;
                    }
                    payload.as_section().map(|(id, range)| (id, wasm[range].to_vec()))
                }).collect::<Vec<_>>()
            };
            let unchanged = match profile {
                BuildProfile::Debug => stable_sections(&ordinary) == stable_sections(&reported),
                BuildProfile::Release => ordinary == reported,
            };
            assert!(
                unchanged,
                "reporting changed {profile:?} artifact ({} -> {} bytes; first difference {:?})",
                ordinary.len(),
                reported.len(),
                ordinary.iter().zip(&reported).position(|(a, b)| a != b)
            );
            let mut imported_functions = 0;
            let mut defined_functions = 0;
            for payload in Parser::new(0).parse_all(&reported) {
                match payload.unwrap() {
                    Payload::ImportSection(section) => imported_functions = section.count(),
                    Payload::FunctionSection(section) => defined_functions = section.count(),
                    Payload::MemorySection(section) => {
                        assert_eq!(
                            section.into_iter().next().unwrap().unwrap().initial,
                            report.minimum_memory_pages
                        );
                    }
                    _ => {}
                }
            }
            assert_eq!(report.functions.len(), defined_functions as usize);
            for (position, (index, _)) in report.functions.iter().enumerate() {
                assert_eq!(*index, imported_functions + position as u32);
            }
            assert!(report.scratch_bytes >= u64::from(report.abi_read_capacity));
            assert!(report.static_data_end >= u64::from(report.static_data_start));
            if profile == BuildProfile::Debug {
                let names = debug_function_names(&reported).unwrap().1;
                let defined = names
                    .into_iter()
                    .filter(|(index, _)| *index >= imported_functions)
                    .collect::<Vec<_>>();
                assert_eq!(defined, report.functions);
            } else {
                assert!(debug_function_names(&reported).is_none());
            }
        }
    }
}

#[test]
fn release_managed_report_excludes_unused_strings_and_opposite_backend() {
    use splitscript::{BuildProfile, CompilerOptions, compiler};

    let compile = |provider: &str, declarations: &str, field: &str| {
        let source = format!(
            r#"
            image "Assembly-CSharp" {{
                class Probe {{ static i32 value; {declarations} }}
            }}
            state {provider} ["game.exe"] {{ value = Probe.{field}?; }}
        "#
        );
        let checked =
            splitscript::check(splitscript::lower(splitscript::parse(&source).unwrap())).unwrap();
        compiler::codegen_with_report(
            &checked,
            CompilerOptions {
                profile: BuildProfile::Release,
                ..Default::default()
            },
        )
    };
    for (provider, excluded) in [
        ("Unity.il2cpp(Il2CppProfile.unity2021_3_11f1X64())", "Mono"),
        ("Unity.mono(MonoVersion.V2)", "Il2Cpp"),
    ] {
        let ordinary = compile(provider, "", "value");
        let unused = compile(provider, "static String text;", "value");
        assert!(
            ordinary.0 == unused.0,
            "unused managed metadata changed Release Wasm for {provider}"
        );
        // Unread declarations retain neither metadata binding nor decoders.
        assert_eq!(ordinary.1.runtime_helpers, unused.1.runtime_helpers);
        assert_eq!(ordinary.1.scratch_bytes, unused.1.scratch_bytes);
        assert!(
            unused
                .1
                .functions
                .iter()
                .all(|(_, name)| !name.contains("ReadManagedString"))
        );
        assert!(
            ordinary
                .1
                .functions
                .iter()
                .all(|(_, name)| !name.contains(excluded))
        );
        assert!(
            ordinary
                .1
                .functions
                .iter()
                .all(|(_, name)| !name.contains("ReadManagedString"))
        );
        let used = compile(provider, "static String text;", "text");
        assert!(
            used.1
                .functions
                .iter()
                .any(|(_, name)| name.ends_with("ReadManagedStringField"))
        );
        assert!(used.0.len() > ordinary.0.len());
    }
}

#[test]
fn ordinary_function_types_are_shared_after_the_gc_group() {
    for profile in [
        splitscript::BuildProfile::Debug,
        splitscript::BuildProfile::Release,
    ] {
        let wasm = splitscript::compile_with_options(
            EXAMPLE,
            splitscript::CompilerOptions {
                profile,
                ..Default::default()
            },
        )
        .unwrap();
        Validator::new_with_features(WasmFeatures::all())
            .validate_all(&wasm)
            .unwrap();
        let mut signatures = std::collections::HashSet::new();
        for payload in Parser::new(0).parse_all(&wasm) {
            if let Payload::TypeSection(section) = payload.unwrap() {
                for group in section {
                    let group = group.unwrap();
                    if group.is_explicit_rec_group() {
                        continue;
                    }
                    for ty in group.into_types() {
                        if let wasmparser::CompositeInnerType::Func(function) =
                            ty.composite_type.inner
                        {
                            assert!(
                                signatures.insert((
                                    function.params().to_vec(),
                                    function.results().to_vec()
                                )),
                                "ordinary signatures should be interned across imports and defined functions"
                            );
                        }
                    }
                }
            }
        }
        assert!(!signatures.is_empty());
    }
}

#[test]
fn script_locals_use_adjacent_type_groups() {
    let wasm = splitscript::compile(
        r#"
        state "game.exe" {}
        fn combine(input: i32) {
            let first = input + 1
            let second = input + 2
            let third = input + 3
            return first + second + third
        }
        setup { print(combine(1)) }
    "#,
    )
    .unwrap();
    Validator::new_with_features(WasmFeatures::all())
        .validate_all(&wasm)
        .unwrap();
    let (_, names) = debug_function_names(&wasm).unwrap();
    let index = names.iter().find(|(_, name)| name == "combine").unwrap().0;
    let mut function_index = 0;
    let mut found = false;
    for payload in Parser::new(0).parse_all(&wasm) {
        match payload.unwrap() {
            Payload::ImportSection(section) => function_index = section.count(),
            Payload::CodeSectionEntry(body) => {
                if function_index == index {
                    let locals = body
                        .get_locals_reader()
                        .unwrap()
                        .into_iter()
                        .collect::<Result<Vec<_>, _>>()
                        .unwrap();
                    assert!(locals.iter().map(|(count, _)| count).sum::<u32>() >= 3);
                    assert!(locals.windows(2).all(|pair| pair[0].1 != pair[1].1));
                    found = true;
                }
                function_index += 1;
            }
            _ => {}
        }
    }
    assert!(found);
}

#[test]
fn compiler_stages_expose_lowered_declarations_without_mutating_syntax() {
    let source = r#"
        state "game.exe" {
            level: u16 at 0x1234
        }

        fn identity(value: u16) -> u16 {
            return value
        }

        whileAttached {
            let inferred = [identity(current.level), 2]
            print(`{inferred[0]}`)
        }
    "#;

    let parsed = splitscript::parse(source).unwrap();
    assert!(parsed.syntax().array_types.is_empty());

    let lowered = splitscript::lower(parsed);
    let identity = lowered
        .hir()
        .declarations_named("identity")
        .next()
        .expect("lowering should index functions before type checking");
    assert!(matches!(
        identity.id,
        splitscript::compiler::hir::DeclarationId::Function(_)
    ));
    let identity_id = identity.id;
    assert!(
        lowered
            .hir()
            .declarations_named("whileAttached")
            .any(|declaration| {
                declaration.id
                    == splitscript::compiler::hir::DeclarationId::Action(
                        splitscript::compiler::ast::ActionKind::WhileAttached,
                    )
            })
    );

    let checked = splitscript::check(lowered).unwrap();
    assert!(
        checked.syntax().array_types.is_empty(),
        "type checking must not append inferred layouts to parsed syntax"
    );
    assert!(
        checked
            .semantics()
            .array_element_types()
            .any(|(_, element)| checked.semantics().types().kind(element)
                == &TypeKind::Builtin(BuiltinType::U16))
    );
    assert_eq!(
        checked
            .hir()
            .declarations_named("identity")
            .next()
            .map(|declaration| declaration.id),
        Some(identity_id)
    );
    assert_eq!(
        checked.typed_hir().expressions().count(),
        checked.semantics().expression_types().count()
    );
    assert!(checked.typed_hir().expressions().any(|expression| matches!(
        &expression.resolution,
        Some(splitscript::compiler::hir::ExpressionResolution::Call(_))
    )));
    let action_body = checked
        .typed_hir()
        .action_body(splitscript::compiler::ast::ActionKind::WhileAttached)
        .expect("typed HIR should own action statement shape");
    let splitscript::compiler::hir::TypedStatementKind::Variable { initializer, .. } =
        &action_body.statements[0].kind
    else {
        panic!("expected the inferred variable in typed HIR");
    };
    assert!(matches!(
        &checked.typed_hir().expression(*initializer).unwrap().kind,
        splitscript::compiler::hir::TypedExpressionKind::Array(_)
    ));
    let interpolation = checked
        .typed_hir()
        .expressions()
        .find_map(|expression| match &expression.kind {
            splitscript::compiler::hir::TypedExpressionKind::InterpolatedString(parts) => {
                Some(parts)
            }
            _ => None,
        })
        .expect("typed HIR should retain the interpolated string");
    assert!(matches!(
        interpolation.as_slice(),
        [splitscript::compiler::hir::TypedInterpolatedPart::Expression {
            conversion: Some(splitscript::compiler::hir::ImplicitConversion::ToString { source }),
            ..
        }] if checked.semantics().types().kind(*source)
            == &TypeKind::Builtin(BuiltinType::U16)
    ));
    let mut counter = TypedExpressionCounter::default();
    splitscript::compiler::hir::TypedVisitor::visit_program(&mut counter, checked.typed_hir());
    assert_eq!(counter.0, checked.typed_hir().expressions().count());

    Validator::new_with_features(WasmFeatures::all())
        .validate_all(&splitscript::codegen(&checked))
        .expect("checked inferred layouts should remain available to code generation");
}

#[test]
fn compiler_profiles_flow_through_staged_and_one_shot_compilation() {
    use splitscript::{BuildProfile, CompilerOptions};

    let source = r#"state "game.exe" {} whileAttached { print("profile") }"#;
    let checked = splitscript::check(splitscript::parse(source).unwrap()).unwrap();
    let mut outputs = Vec::new();
    for profile in [BuildProfile::Debug, BuildProfile::Release] {
        let options = CompilerOptions {
            profile,
            ..CompilerOptions::default()
        };
        let lowered = splitscript::lower_wasm_with_options(&checked, options);
        assert_eq!(lowered.profile(), profile);
        let staged = splitscript::codegen_with_options(&checked, options);
        let one_shot = splitscript::compile_with_options(source, options).unwrap();
        assert_eq!(staged, one_shot);
        Validator::new_with_features(WasmFeatures::all())
            .validate_all(&staged)
            .expect("both compiler profiles should produce valid WebAssembly GC");
        outputs.push(staged);
    }

    assert_ne!(outputs[0], outputs[1]);
    assert!(debug_function_names(&outputs[0]).is_some());
    assert!(
        debug_function_names(&outputs[1]).is_none(),
        "release modules must not leak the WebAssembly name section"
    );
}

#[test]
fn reachable_managed_snapshot_types_have_gc_layouts() {
    let source = r#"
        image "Assembly-CSharp" {
            class Player {
                f32 health;
            }
            class GameManager {
                static GameManager instance;
                Player player;
                i32 points;
            }
        }

        state "game.exe" {}

        fn points(manager: GameManager) -> i32 {
            return manager.points
        }

        fn player(manager: GameManager) -> Player {
            return manager.player
        }

        setup {
            let pointsCallback = points
            let playerCallback = player
        }
    "#;

    let wasm = splitscript::compile(source).expect("managed snapshot fixture should compile");
    Validator::new_with_features(WasmFeatures::all())
        .validate_all(&wasm)
        .expect("managed snapshot GC layouts should validate");
}

#[test]
fn unity_hierarchy_components_produce_typed_live_references() {
    let source = r#"
        image "Assembly-CSharp" {
            class PlayerController {
                u32 score;
            }
        }

        state Unity ["game.exe"] {
            player = unity.scenes.active()?
                .find("World/Player")?
                .component<PlayerController>()?
                .snapshot();
        }

        whileAttached {
            print(current.player.score)
        }
    "#;

    let wasm = splitscript::compile(source).expect("typed Unity component lookup should compile");
    Validator::new_with_features(WasmFeatures::all())
        .validate_all(&wasm)
        .expect("typed Unity component lookup should produce valid WebAssembly GC");
}

#[test]
fn managed_reference_snapshot_reads_the_complete_layout_refined_shape() {
    use splitscript::{BuildProfile, CompilerOptions};

    let source = r#"
        enum Edition { Base, Demo }
        let edition: Edition

        image "Assembly-CSharp" {
            class Player {
                f32 health;
            }
            class GameManager {
                static GameManager instance;
                Player player;
                i32 points;
                if edition == Edition.Base {
                    i32 level;
                } else {
                    address scene;
                }
            }
        }

        state Unity.il2cpp(Il2CppProfile.unity2021_3_11f1X64()) ["game.exe"] {
            manager: GameManager = GameManager.instance?.snapshot()?;
        }

        onAttach { edition = Edition.Base }

        whileAttached {
            print(current.manager)
            print(current.manager.points)
            let player = current.manager.player
            let health = player.health
            print(health)
            if edition == Edition.Base {
                print(current.manager.level)
            } else {
                print(current.manager.scene)
            }
        }
    "#;

    let wasm = splitscript::compile_with_options(
        source,
        CompilerOptions {
            profile: BuildProfile::Debug,
            ..CompilerOptions::default()
        },
    )
    .expect("managed snapshots should compile");
    Validator::new_with_features(WasmFeatures::all())
        .validate_all(&wasm)
        .expect("transactional managed snapshot readers should validate");
    let (_, names) = debug_function_names(&wasm).expect("debug names should exist");
    assert!(
        names
            .iter()
            .any(|(_, name)| { name == "__splitscript::managed::GameManager::snapshot" })
    );
    assert!(
        names
            .iter()
            .any(|(_, name)| name == "__splitscript::debug::GameManager"),
        "displaying a managed snapshot should materialize its structural formatter"
    );
    assert!(
        names
            .iter()
            .any(|(_, name)| name == "__splitscript::managed::Player::snapshot"),
        "nested class snapshots must retain their child reader"
    );

    let unused = splitscript::compile_with_options(
        r#"
            image "Assembly-CSharp" {
                class GameManager {
                    static GameManager instance;
                    i32 points;
                }
            }
            state Unity.il2cpp(Il2CppProfile.unity2021_3_11f1X64()) ["game.exe"] {
                points: i32 = GameManager.instance?.points?;
            }
        "#,
        CompilerOptions {
            profile: BuildProfile::Debug,
            ..CompilerOptions::default()
        },
    )
    .expect("unused snapshot readers should not be required");
    let (_, names) = debug_function_names(&unused).expect("debug names should exist");
    assert!(
        names.iter().all(|(_, name)| !name.ends_with("::snapshot")),
        "snapshot readers must be generated only when reachable: {names:#?}"
    );
}

#[test]
fn recursive_managed_classes_have_distinct_live_and_owned_projections() {
    let checked = splitscript::check(
        splitscript::parse(
            r#"
        image "Assembly-CSharp" {
            class Node { Node? next; String text; }
        }
        state Unity ["game.exe"] {}
        fn live(node: Node.Ref) -> Node.Ref?! { return node.next }
        fn owned(node: Node) -> Node? { return node.next }
    "#,
        )
        .unwrap(),
    )
    .expect("live access and owned projections must coexist");
    let class = checked.syntax().managed_class_declarations()[0].id;
    let semantics = checked.semantics();
    let capability = splitscript::compiler::stdlib::StdlibCapabilityId::ManagedReadable;
    assert!(checked.capabilities().has(
        semantics.types().id_for_managed_class(class),
        capability,
        semantics
    ));
    assert!(!checked.capabilities().has(
        semantics.types().id_for_managed_reference(class),
        capability,
        semantics
    ));
}

#[test]
fn managed_snapshots_reject_explicit_live_references_inside_the_owned_value() {
    let errors = splitscript::compile(
        r#"
        image "Assembly-CSharp" {
            class Node { static Node instance; Node.Ref live; }
        }
        state Unity ["game.exe"] {}
        whileAttached { let root = Node.instance else return; let value = root.snapshot(); }
    "#,
    )
    .expect_err("snapshot materialization cannot hide an explicitly live field");
    assert!(
        errors.iter().any(|error| error
            .message
            .contains("snapshot contains a value without a managed decoder")),
        "{errors:#?}"
    );
}

#[test]
fn unused_nested_managed_classes_do_not_retain_object_walk_helpers() {
    let source = r#"
        image "Assembly-CSharp" {
            class Flat { static Flat instance; i32 value; }
        }
        state Unity.il2cpp(Il2CppProfile.unity2022_3_0f1X64()) ["game.exe"] {
            value = Flat.instance?.snapshot()?;
        }
    "#;
    let extended = format!(
        r#"{source}
        image "Unused" {{ class Recursive {{ Recursive? next; }} }}
    "#
    );
    let options = splitscript::CompilerOptions {
        profile: splitscript::BuildProfile::Release,
        ..Default::default()
    };
    assert_eq!(
        splitscript::compile_with_options(source, options).unwrap(),
        splitscript::compile_with_options(&extended, options).unwrap()
    );
    let debug = splitscript::compile(source).unwrap();
    let (_, names) = debug_function_names(&debug).unwrap();
    assert!(
        names
            .iter()
            .all(|(_, name)| !name.contains("EnterManagedObject")),
        "{names:#?}"
    );
    // Even a flat snapshot charges work while validating its runtime class.
    assert!(
        names
            .iter()
            .any(|(_, name)| name.contains("ChargeManagedWork"))
    );
}

#[test]
fn managed_string_decoders_are_retained_only_for_reachable_reads() {
    use splitscript::{BuildProfile, CompilerOptions};

    let compile_names = |state_fields: &str| {
        let source = format!(
            r#"
                image "Assembly-CSharp" {{
                    class GameManager {{
                        static String scene;
                    }}
                }}

                state Unity.il2cpp(Il2CppProfile.unity2021_3_11f1X64()) ["game.exe"] {{
                    {state_fields}
                }}
            "#
        );
        let wasm = splitscript::compile_with_options(
            &source,
            CompilerOptions {
                profile: BuildProfile::Debug,
                ..CompilerOptions::default()
            },
        )
        .expect("managed string reachability fixture should compile");
        debug_function_names(&wasm)
            .expect("debug names should exist")
            .1
    };

    let unused = compile_names("");
    assert!(
        unused
            .iter()
            .all(|(_, name)| !name.contains("ReadManagedString")),
        "an unused managed string declaration must not retain its decoder: {unused:#?}"
    );

    let used = compile_names("scene: String = GameManager.scene?;");
    assert!(
        used.iter()
            .any(|(_, name)| name.ends_with("ReadManagedStringField")),
        "a reachable managed string read must retain its typed decoder: {used:#?}"
    );
}

#[test]
fn explicit_unity_backends_prune_the_unreachable_schema_binder() {
    use splitscript::{BuildProfile, CompilerOptions};

    let compile_names = |state: &str| {
        let source = format!(
            r#"
                image "Assembly-CSharp" {{
                    class GameManager {{
                        static GameManager instance;
                        i32 state;
                    }}
                }}

                {state} {{
                    state: i32 = GameManager.instance?.state?;
                }}
            "#
        );
        let wasm = splitscript::compile_with_options(
            &source,
            CompilerOptions {
                profile: BuildProfile::Debug,
                ..CompilerOptions::default()
            },
        )
        .expect("the managed provider should compile");
        debug_function_names(&wasm)
            .expect("debug names should exist")
            .1
            .into_iter()
            .map(|(_, name)| name)
            .collect::<Vec<_>>()
    };

    let explicit =
        compile_names(r#"state Unity.il2cpp(Il2CppProfile.unity2021_3_11f1X64()) ["game.exe"]"#);
    assert!(explicit.iter().any(|name| name.contains("Il2Cpp")));
    assert!(
        explicit.iter().all(|name| !name.contains("Mono")),
        "an explicit IL2CPP provider must not retain the unreachable Mono binder"
    );

    let automatic = compile_names(r#"state Unity ["game.exe"]"#);
    assert!(automatic.iter().any(|name| name.contains("Il2Cpp")));
    assert!(
        automatic.iter().any(|name| name.contains("Mono")),
        "automatic Unity discovery still needs both schema binders"
    );
}

#[test]
fn managed_schema_declarations_retain_their_logical_hierarchy() {
    use splitscript::compiler::hir::DeclarationId;

    let source = r#"
        enum Edition { Demo }
        let edition: Edition
        state "game.exe" {
        }
        image "Assembly-CSharp" {
            namespace Game {
                class GameManager {
                    i32 points;
                    if edition == Edition.Demo {
                        String scene;
                    }
                }
            }
        }
    "#;
    let lowered = splitscript::lower(splitscript::parse(source).unwrap());
    let declarations = lowered.hir();

    let image = declarations
        .declarations_named("Assembly-CSharp")
        .next()
        .unwrap();
    let namespace = declarations.declarations_named("Game").next().unwrap();
    let class = declarations
        .declarations_named("GameManager")
        .next()
        .unwrap();
    let points = declarations.declarations_named("points").next().unwrap();
    let scene = declarations.declarations_named("scene").next().unwrap();

    assert_eq!(namespace.owner, Some(image.id));
    assert_eq!(class.owner, Some(namespace.id));
    assert_eq!(points.owner, Some(class.id));
    assert_eq!(scene.owner, Some(class.id));
    assert!(matches!(image.id, DeclarationId::ManagedImage(_)));
    assert_eq!(
        declarations
            .children(class.id)
            .map(|child| child.id)
            .collect::<Vec<_>>(),
        vec![points.id, scene.id]
    );
}

#[test]
fn structural_display_helpers_are_materialized_only_when_reachable() {
    use splitscript::{BuildProfile, CompilerOptions};

    let compile_debug = |body: &str| {
        let source = format!(
            r#"
                state "game.exe" {{}}

                struct Point {{
                    x: u32,
                    y: u32,
                }}

                whileAttached {{
                    {body}
                }}
            "#,
        );
        splitscript::compile_with_options(
            &source,
            CompilerOptions {
                profile: BuildProfile::Debug,
                ..CompilerOptions::default()
            },
        )
        .expect("the Display reachability probe should compile")
    };

    let unused = compile_debug("print(\"tick\")");
    let (_, unused_names) = debug_function_names(&unused).expect("debug names should exist");
    assert!(
        unused_names
            .iter()
            .all(|(_, name)| name != "__splitscript::debug::Point"),
        "declaring a displayable struct must not eagerly generate its formatter"
    );

    let displayed = compile_debug("print(Point { x: 1, y: 2 })");
    Validator::new_with_features(WasmFeatures::all())
        .validate_all(&displayed)
        .expect("the lazily generated formatter should be valid WebAssembly GC");
    let (_, displayed_names) = debug_function_names(&displayed).expect("debug names should exist");
    assert!(
        displayed_names
            .iter()
            .any(|(_, name)| name == "__splitscript::debug::Point"),
        "a reachable conversion should materialize the formatter"
    );
}

#[test]
fn opaque_debug_helpers_are_materialized_only_when_reachable() {
    use splitscript::{BuildProfile, CompilerOptions};

    let compile_debug = |display: bool| {
        let display = if display { "print(transform)" } else { "" };
        splitscript::compile_with_options(
            &format!(
                r#"
                    state "game.exe" {{}}

                    setup {{
                        let transform: (u32) -> u32 = value => value + 1
                        {display}
                    }}
                "#,
            ),
            CompilerOptions {
                profile: BuildProfile::Debug,
                ..CompilerOptions::default()
            },
        )
        .expect("the opaque Debug reachability probe should compile")
    };

    let unused = compile_debug(false);
    let (_, unused_names) = debug_function_names(&unused).expect("debug names should exist");
    assert!(
        unused_names
            .iter()
            .all(|(_, name)| !name.starts_with("__splitscript::debug::type#")),
        "merely constructing a closure must not generate its opaque formatter"
    );

    let displayed = compile_debug(true);
    Validator::new_with_features(WasmFeatures::all())
        .validate_all(&displayed)
        .expect("the lazily generated opaque formatter should be valid WebAssembly GC");
    let (_, displayed_names) = debug_function_names(&displayed).expect("debug names should exist");
    assert!(
        displayed_names
            .iter()
            .any(|(_, name)| name.starts_with("__splitscript::debug::type#")),
        "displaying a closure should materialize its opaque formatter"
    );
}

#[test]
fn array_equality_helpers_are_materialized_only_when_compared() {
    use splitscript::{BuildProfile, CompilerOptions};

    let compile_debug = |body: &str| {
        splitscript::compile_with_options(
            &format!(r#"state "game.exe" {{}} setup {{ {body} }}"#),
            CompilerOptions {
                profile: BuildProfile::Debug,
                ..CompilerOptions::default()
            },
        )
        .expect("the array-equality reachability probe should compile")
    };
    let helper_names = |wasm: &[u8]| {
        debug_function_names(wasm)
            .expect("debug names should exist")
            .1
            .into_iter()
            .map(|(_, name)| name)
            .filter(|name| name.starts_with("__splitscript::equals::array#"))
            .collect::<Vec<_>>()
    };

    assert!(helper_names(&compile_debug("let values = [1u8, 2u8]")).is_empty());
    let compared = compile_debug("let values = [1u8, 2u8]; print(values == [1u8, 2u8])");
    Validator::new_with_features(WasmFeatures::all())
        .validate_all(&compared)
        .expect("the lazily generated equality helper should be valid WebAssembly GC");
    assert_eq!(helper_names(&compared).len(), 1);
}

#[test]
fn float_format_helpers_and_tables_are_materialized_by_reachable_width() {
    use splitscript::{BuildProfile, CompilerOptions};

    let compile_debug = |body: &str| {
        splitscript::compile_with_options(
            &format!(r#"state "game.exe" {{}} setup {{ {body} }}"#),
            CompilerOptions {
                profile: BuildProfile::Debug,
                ..CompilerOptions::default()
            },
        )
        .expect("the float-format reachability probe should compile")
    };
    let names = |wasm: &[u8]| {
        debug_function_names(wasm)
            .expect("debug names should exist")
            .1
            .into_iter()
            .map(|(_, name)| name)
            .collect::<Vec<_>>()
    };

    let integer = names(&compile_debug("print(1u32)"));
    assert!(integer.iter().all(|name| !name.contains("FormatF")));
    assert!(integer.iter().all(|name| !name.contains("Zmij")));

    let f32_names = names(&compile_debug("print(1.25 as f32)"));
    assert!(f32_names.iter().any(|name| name.ends_with("::FormatF32")));
    assert!(f32_names.iter().all(|name| !name.ends_with("::FormatF64")));
    assert!(
        f32_names
            .iter()
            .all(|name| !name.ends_with("::ZmijMul192Hi128"))
    );

    let f64_names = names(&compile_debug("print(1.25)"));
    for suffix in ["::FormatF64", "::ZmijDecimalF64", "::ZmijMul192Hi128"] {
        assert!(f64_names.iter().any(|name| name.ends_with(suffix)));
    }

    let custom = splitscript::compile_with_options(
        r#"
            struct Measurement { value: f32 }

            fn Measurement.toString() -> String {
                return "measurement"
            }

            state "game.exe" {}
            setup { print(Measurement { value: 1.25 as f32 }) }
        "#,
        CompilerOptions {
            profile: BuildProfile::Debug,
            ..CompilerOptions::default()
        },
    )
    .expect("a custom Display implementation should compile");
    let custom = names(&custom);
    assert!(
        custom.iter().all(|name| !name.contains("FormatF")),
        "a custom formatter must not retain an unreachable structural float formatter"
    );
    assert!(
        custom.iter().all(|name| !name.contains("Zmij")),
        "a custom formatter must not retain unreachable decimal-conversion helpers"
    );
}

#[test]
fn debug_profiles_name_every_function_while_release_profiles_are_stripped() {
    use splitscript::{BuildProfile, CompilerOptions};

    let source = r#"
        state "game.exe" {
            level: u16 at 0x1234
        }

        enum Phase {
            Ready,
        }

        let tracked: u32 = 7
        let phase = Phase.Ready
        let label = "ready"

        fn identity(value) {
            return value
        }

        fn control(value: bool) {
            while value {
                if value {
                    break
                }
                continue
            }
            return
        }

        onAttach {
            let module = await process.mainModule()
            print(module.address)
            await nextTick()
        }

        whileAttached {
            let visible: u16 = identity(current.level)
            control(visible > 0)
            print(tracked)
            print(visible)
            if phase == Phase.Ready {
                print(label)
            }
        }
    "#;
    let source_name = "P:/debug/fixture.split";
    let compile = |profile| {
        splitscript::compile_named_with_context_and_options_diagnostics(
            splitscript::CompilerContext::default(),
            source_name,
            source,
            CompilerOptions {
                profile,
                ..CompilerOptions::default()
            },
        )
        .map(|(artifact, _)| artifact)
        .expect("debug-name fixture should compile")
    };
    let debug = compile(BuildProfile::Debug);
    let release = compile(BuildProfile::Release);

    let (module_name, function_names) =
        debug_function_names(&debug).expect("debug modules should contain names");
    assert_eq!(module_name, "SplitScript autosplitter");
    assert!(
        function_names
            .iter()
            .any(|(_, name)| name == "env::process_read")
    );
    assert!(
        function_names
            .iter()
            .any(|(_, name)| name.starts_with("identity"))
    );
    for expected in ["state::level::read", "whileAttached", "_start", "update"] {
        assert!(
            function_names.iter().any(|(_, name)| name == expected),
            "missing debug function name `{expected}`: {function_names:#?}"
        );
    }
    let local_names = debug_local_names(&debug);
    let identity = function_names
        .iter()
        .find(|(_, name)| name.starts_with("identity"))
        .map(|(index, _)| *index)
        .expect("the specialized identity function should be named");
    let while_attached = function_names
        .iter()
        .find(|(_, name)| name == "whileAttached")
        .map(|(index, _)| *index)
        .expect("the lifecycle function should be named");
    assert!(
        local_names[&identity]
            .iter()
            .any(|(_, name)| name == "value")
    );
    assert!(
        local_names[&while_attached]
            .iter()
            .any(|(_, name)| name == "visible")
    );
    assert!(
        debug_global_names(&debug)
            .iter()
            .any(|(_, name)| name == "tracked")
    );
    assert_eq!(
        function_names
            .iter()
            .map(|(index, _)| *index)
            .collect::<Vec<_>>(),
        (0..function_names.len() as u32).collect::<Vec<_>>(),
        "imports and definitions should all receive one deterministic name"
    );
    assert!(debug_function_names(&release).is_none());
    let dwarf = debug_dwarf(&debug);
    for required in [".debug_abbrev", ".debug_info", ".debug_line"] {
        assert!(
            dwarf
                .get(required)
                .is_some_and(|section| !section.is_empty()),
            "debug modules should contain {required}"
        );
    }

    let dwarf = gimli::Dwarf::load(|id| {
        Ok::<_, gimli::Error>(gimli::EndianSlice::new(
            dwarf.get(id.name()).copied().unwrap_or_default(),
            gimli::LittleEndian,
        ))
    })
    .expect("generated DWARF sections should load");
    let header = dwarf
        .units()
        .next()
        .expect("generated DWARF should be readable")
        .expect("debug modules should contain a compilation unit");
    let unit = dwarf
        .unit(header)
        .expect("generated DWARF compilation unit should parse");
    let mut rows = unit
        .line_program
        .as_ref()
        .expect("debug compilation units should contain a line program")
        .clone()
        .rows();
    let instruction_boundaries = wasm_instruction_boundaries(&debug);
    let mut source_rows = Vec::new();
    while let Some((_, row)) = rows
        .next_row()
        .expect("generated DWARF line rows should parse")
    {
        if row.end_sequence() {
            continue;
        }
        assert!(
            instruction_boundaries.contains(&row.address()),
            "DWARF address {:#x} is not a Wasm instruction boundary",
            row.address()
        );
        source_rows.push((
            row.line()
                .expect("source-backed rows should have line numbers")
                .get() as usize,
            row.discriminator(),
        ));
    }
    let source_lines = source_rows
        .iter()
        .map(|(line, _)| *line)
        .collect::<Vec<_>>();
    for snippet in [
        "let phase",
        "let label",
        "return value",
        "let visible",
        "print(visible)",
    ] {
        let line = source
            .lines()
            .position(|candidate| candidate.contains(snippet))
            .expect("fixture snippet should exist")
            + 1;
        assert!(
            source_lines.contains(&line),
            "missing line row for `{snippet}` on line {line}: {source_lines:?}"
        );
    }
    for statement in ["break", "continue", "return"] {
        let line = source
            .lines()
            .position(|candidate| candidate.trim() == statement)
            .expect("control-flow fixture statement should exist")
            + 1;
        assert!(
            source_lines.contains(&line),
            "missing statement row for `{statement}` on line {line}: {source_lines:?}"
        );
    }
    for await_snippet in ["await process.mainModule()", "await nextTick()"] {
        let line = source
            .lines()
            .position(|candidate| candidate.contains(await_snippet))
            .expect("async fixture statement should exist")
            + 1;
        for (discriminator, boundary) in [(1, "suspend"), (2, "resume")] {
            assert!(
                source_rows.contains(&(line, discriminator)),
                "missing {boundary} row for `{await_snippet}` on line {line}: {source_rows:?}"
            );
        }
    }

    let mut entries = unit.entries();
    let root = entries
        .next_dfs()
        .expect("generated DWARF entries should parse")
        .expect("debug compilation units should have a root entry");
    let root_name = root
        .attr_value(gimli::DW_AT_name)
        .expect("debug compilation units should identify their source file");
    assert_eq!(
        dwarf
            .attr_string(&unit, root_name)
            .expect("the source identity should be a string")
            .to_string_lossy(),
        source_name
    );
    assert_eq!(
        root.attr_value(gimli::DW_AT_language),
        Some(gimli::AttributeValue::Language(gimli::DW_LANG_C11)),
        "native debuggers need a supported compatibility language to expose names and variables"
    );
    assert!(root.attr_value(gimli::DW_AT_low_pc).is_none());
    assert!(root.attr_value(gimli::DW_AT_high_pc).is_none());
    assert!(
        root.attr_value(gimli::DW_AT_ranges).is_none(),
        "LLDB must derive compilation-unit ownership from Wasmtime's complete subprogram ranges"
    );
    let mut subprograms = Vec::new();
    let mut parameters = Vec::new();
    let mut variables = Vec::new();
    let mut base_types = Vec::new();
    let mut lexical_blocks = 0;
    while let Some(entry) = entries
        .next_dfs()
        .expect("generated DWARF entries should parse")
    {
        let Some(value) = entry.attr_value(gimli::DW_AT_name) else {
            if entry.tag() == gimli::DW_TAG_lexical_block {
                lexical_blocks += 1;
                assert!(entry.attr_value(gimli::DW_AT_low_pc).is_some());
                assert!(entry.attr_value(gimli::DW_AT_high_pc).is_some());
            }
            continue;
        };
        let name = dwarf
            .attr_string(&unit, value)
            .expect("debug names should be strings")
            .to_string_lossy()
            .into_owned();
        match entry.tag() {
            gimli::DW_TAG_subprogram => subprograms.push(name),
            gimli::DW_TAG_formal_parameter => {
                assert!(entry.attr_value(gimli::DW_AT_type).is_some());
                assert_wasm_local_location(entry, unit.encoding());
                parameters.push(name);
            }
            gimli::DW_TAG_variable => {
                assert!(entry.attr_value(gimli::DW_AT_type).is_some());
                if name == "tracked" {
                    assert_wasm_global_location(entry, unit.encoding());
                } else {
                    assert_wasm_local_location(entry, unit.encoding());
                }
                variables.push(name);
            }
            gimli::DW_TAG_base_type => base_types.push(name),
            _ => {}
        }
    }
    assert!(subprograms.iter().any(|name| name.starts_with("identity")));
    assert!(subprograms.iter().any(|name| name == "whileAttached"));
    assert!(parameters.iter().any(|name| name == "value"));
    assert!(variables.iter().any(|name| name == "visible"));
    assert!(variables.iter().any(|name| name == "tracked"));
    assert!(base_types.iter().any(|name| name == "u16"));
    assert!(lexical_blocks >= 1);

    assert!(Parser::new(0).parse_all(&release).all(|payload| {
        !matches!(
            payload.expect("release module should parse"),
            Payload::CustomSection(section)
                if section.name() == "name" || section.name().starts_with(".debug_")
        )
    }));
}

fn debug_dwarf(wasm: &[u8]) -> std::collections::HashMap<String, &[u8]> {
    Parser::new(0)
        .parse_all(wasm)
        .filter_map(|payload| {
            let Payload::CustomSection(section) = payload.ok()? else {
                return None;
            };
            section
                .name()
                .starts_with(".debug_")
                .then(|| (section.name().to_owned(), section.data()))
        })
        .collect()
}

fn assert_wasm_local_location<R: gimli::Reader>(
    entry: &gimli::DebuggingInformationEntry<R>,
    encoding: gimli::Encoding,
) {
    let gimli::AttributeValue::Exprloc(expression) = entry
        .attr_value(gimli::DW_AT_location)
        .expect("source variables should have locations")
    else {
        panic!("source variable location should be an expression")
    };
    let mut operations = expression.operations(encoding);
    assert!(matches!(
        operations.next().expect("location expression should parse"),
        Some(gimli::Operation::WasmLocal { .. })
    ));
    assert!(matches!(
        operations.next().expect("location expression should parse"),
        Some(gimli::Operation::StackValue)
    ));
    assert!(
        operations
            .next()
            .expect("location expression should parse")
            .is_none()
    );
}

fn assert_wasm_global_location<R: gimli::Reader>(
    entry: &gimli::DebuggingInformationEntry<R>,
    encoding: gimli::Encoding,
) {
    let gimli::AttributeValue::Exprloc(expression) = entry
        .attr_value(gimli::DW_AT_location)
        .expect("source globals should have locations")
    else {
        panic!("source global location should be an expression")
    };
    let mut operations = expression.operations(encoding);
    assert!(matches!(
        operations.next().expect("location expression should parse"),
        Some(gimli::Operation::WasmGlobal { .. })
    ));
    assert!(
        operations
            .next()
            .expect("location expression should parse")
            .is_none()
    );
}

fn wasm_instruction_boundaries(wasm: &[u8]) -> std::collections::BTreeSet<u64> {
    let mut code_start = None;
    let mut boundaries = std::collections::BTreeSet::new();
    for payload in Parser::new(0).parse_all(wasm) {
        match payload.expect("generated module should parse") {
            Payload::CodeSectionStart { range, .. } => code_start = Some(range.start),
            Payload::CodeSectionEntry(body) => {
                let code_start = code_start.expect("body entries follow a code section start");
                let mut operators = body
                    .get_operators_reader()
                    .expect("generated function operators should parse");
                while !operators.eof() {
                    boundaries.insert((operators.original_position() - code_start) as u64);
                    operators
                        .read()
                        .expect("generated function operators should parse");
                }
            }
            _ => {}
        }
    }
    boundaries
}

#[test]
fn async_poll_frames_use_their_non_null_parameter_contract() {
    let wasm = splitscript::compile(
        r#"
        state "game.exe" {}
        fn answer() -> async u32 {
            await nextTick()
            return 7
        }
        onAttach {
            let delayed = (value: u32) -> async u32 => {
                let pending = process.module("Game.dll")
                let module = await pending
                return value
            }
            let value = await answer()
            print(await delayed(value))
        }
    "#,
    )
    .unwrap();
    Validator::new_with_features(WasmFeatures::all())
        .validate_all(&wasm)
        .unwrap();
    let (_, names) = debug_function_names(&wasm).unwrap();
    let polls = names
        .into_iter()
        .filter(|(_, name)| name.ends_with("::poll"))
        .collect::<std::collections::HashMap<_, _>>();
    assert!(polls.values().any(|name| name == "answer::poll"));
    assert!(polls.values().any(|name| name.contains("::closure::")));
    assert!(polls.values().any(|name| name.contains("::future::")));
    let mut types = Vec::new();
    let mut functions = Vec::new();
    let mut imports = 0;
    let mut defined = 0;
    let mut checked = 0;
    for payload in Parser::new(0).parse_all(&wasm) {
        match payload.unwrap() {
            Payload::TypeSection(section) => {
                for group in section {
                    types.extend(group.unwrap().into_types());
                }
            }
            Payload::ImportSection(section) => imports = section.count(),
            Payload::FunctionSection(section) => {
                functions.extend(section.into_iter().map(Result::unwrap))
            }
            Payload::CodeSectionEntry(body) => {
                if let Some(name) = polls.get(&(imports + defined as u32)) {
                    let wasmparser::CompositeInnerType::Func(signature) =
                        &types[functions[defined] as usize].composite_type.inner
                    else {
                        panic!("poll signature must be a function type");
                    };
                    assert!(
                        matches!(signature.params(), [wasmparser::ValType::Ref(reference)] if !reference.is_nullable()),
                        "{name}"
                    );
                    assert_eq!(signature.results(), [wasmparser::ValType::I32], "{name}");
                    let operators = body
                        .get_operators_reader()
                        .unwrap()
                        .into_iter()
                        .collect::<Result<Vec<_>, _>>()
                        .unwrap();
                    assert!(
                        !operators.windows(2).any(|pair| matches!(
                            pair,
                            [
                                wasmparser::Operator::LocalGet { local_index: 0 },
                                wasmparser::Operator::RefAsNonNull
                            ]
                        )),
                        "{name}: frame parameter is already non-null"
                    );
                    checked += 1;
                }
                defined += 1;
            }
            _ => {}
        }
    }
    assert_eq!(checked, polls.len());
}

fn debug_function_names(wasm: &[u8]) -> Option<(String, Vec<(u32, String)>)> {
    for payload in Parser::new(0).parse_all(wasm) {
        let Payload::CustomSection(section) = payload.expect("generated module should parse")
        else {
            continue;
        };
        let wasmparser::KnownCustom::Name(reader) = section.as_known() else {
            continue;
        };
        let mut module_name = None;
        let mut functions = Vec::new();
        for subsection in reader {
            match subsection.expect("generated name subsection should parse") {
                wasmparser::Name::Module { name, .. } => module_name = Some(name.to_owned()),
                wasmparser::Name::Function(names) => {
                    functions.extend(names.into_iter().map(|name| {
                        let name = name.expect("generated function name should parse");
                        (name.index, name.name.to_owned())
                    }));
                }
                _ => {}
            }
        }
        return Some((
            module_name.expect("debug name section should identify its module"),
            functions,
        ));
    }
    None
}

fn debug_local_names(wasm: &[u8]) -> std::collections::BTreeMap<u32, Vec<(u32, String)>> {
    let mut output = std::collections::BTreeMap::new();
    for payload in Parser::new(0).parse_all(wasm) {
        let Payload::CustomSection(section) = payload.expect("generated module should parse")
        else {
            continue;
        };
        let wasmparser::KnownCustom::Name(reader) = section.as_known() else {
            continue;
        };
        for subsection in reader {
            let wasmparser::Name::Local(functions) =
                subsection.expect("generated name subsection should parse")
            else {
                continue;
            };
            for function in functions {
                let function = function.expect("generated local names should parse");
                let names = function
                    .names
                    .into_iter()
                    .map(|name| {
                        let name = name.expect("generated local name should parse");
                        (name.index, name.name.to_owned())
                    })
                    .collect();
                output.insert(function.index, names);
            }
        }
    }
    output
}

fn debug_global_names(wasm: &[u8]) -> Vec<(u32, String)> {
    for payload in Parser::new(0).parse_all(wasm) {
        let Payload::CustomSection(section) = payload.expect("generated module should parse")
        else {
            continue;
        };
        let wasmparser::KnownCustom::Name(reader) = section.as_known() else {
            continue;
        };
        for subsection in reader {
            let wasmparser::Name::Global(names) =
                subsection.expect("generated name subsection should parse")
            else {
                continue;
            };
            return names
                .into_iter()
                .map(|name| {
                    let name = name.expect("generated global name should parse");
                    (name.index, name.name.to_owned())
                })
                .collect();
        }
    }
    Vec::new()
}

#[test]
fn debug_statements_are_checked_but_erased_from_release_lowering() {
    use splitscript::{BuildProfile, CompilerOptions};

    let source = include_str!("../debug_profile.split");
    let checked = splitscript::check(splitscript::parse(source).unwrap())
        .expect("supported debug statements should typecheck");
    assert!(
        checked
            .typed_hir()
            .action_bodies()
            .flat_map(|body| &body.body.statements)
            .filter(|statement| statement.debug_only)
            .count()
            >= 5
    );

    let debug_functions = checked
        .syntax()
        .functions
        .iter()
        .filter(|function| function.debug_only)
        .collect::<Vec<_>>();
    assert_eq!(debug_functions.len(), 2);
    let debug_globals = checked
        .syntax()
        .globals
        .iter()
        .filter(|global| global.debug_only)
        .collect::<Vec<_>>();
    assert_eq!(debug_globals.len(), 1);
    let debug_lowering = splitscript::lower_wasm_with_options(
        &checked,
        CompilerOptions {
            profile: BuildProfile::Debug,
            ..CompilerOptions::default()
        },
    );
    let release_lowering = splitscript::lower_wasm_with_options(
        &checked,
        CompilerOptions {
            profile: BuildProfile::Release,
            ..CompilerOptions::default()
        },
    );
    for function in debug_functions {
        assert!(
            debug_lowering
                .body(splitscript::compiler::wasm_ir::BodyOwner::Function(
                    splitscript::compiler::semantic::FunctionInstance::monomorphic(function.id),
                ))
                .is_some()
        );
        assert!(
            release_lowering
                .body(splitscript::compiler::wasm_ir::BodyOwner::Function(
                    splitscript::compiler::semantic::FunctionInstance::monomorphic(function.id),
                ))
                .is_none()
        );
    }
    assert!(debug_lowering.contains_global(debug_globals[0].id));
    assert!(!release_lowering.contains_global(debug_globals[0].id));

    let debug = splitscript::compile_with_options(
        source,
        CompilerOptions {
            profile: BuildProfile::Debug,
            ..CompilerOptions::default()
        },
    )
    .unwrap();
    let release = splitscript::compile_with_options(
        source,
        CompilerOptions {
            profile: BuildProfile::Release,
            ..CompilerOptions::default()
        },
    )
    .unwrap();
    for wasm in [&debug, &release] {
        Validator::new_with_features(WasmFeatures::all())
            .validate_all(wasm)
            .expect("profile-erased programs should remain valid WebAssembly GC");
    }
    for debug_only in [
        b"debug conditional".as_slice(),
        b"debug statement".as_slice(),
        b"debug loop".as_slice(),
        b"debug function".as_slice(),
        b"debug method".as_slice(),
        b"debug binding".as_slice(),
        b"debug local".as_slice(),
        b"runtime_print_message".as_slice(),
    ] {
        assert!(contains_string_literal(&debug, debug_only));
        assert!(!contains_string_literal(&release, debug_only));
    }
    let count_globals = |wasm: &[u8]| {
        Parser::new(0)
            .parse_all(wasm)
            .find_map(|payload| match payload.unwrap() {
                Payload::GlobalSection(section) => Some(section.count()),
                _ => None,
            })
            .unwrap()
    };
    assert_eq!(count_globals(&debug), count_globals(&release) + 1);
    assert!(release.len() < debug.len());
}

#[test]
fn debug_bindings_support_suspension_and_are_erased_from_release() {
    use splitscript::{BuildProfile, CompilerOptions};

    for binding in [
        "debug let module = await process.module(\"debug-only.dll\")\n\
         debug print(module.address as String)",
        "debug let marker = retry process.read<i32>(0)\n\
         debug print(marker as String)",
    ] {
        let source = format!(r#"state "game.exe" {{}} onAttach {{ {binding} }}"#);
        let debug = splitscript::compile_with_options(
            &source,
            CompilerOptions {
                profile: BuildProfile::Debug,
                ..CompilerOptions::default()
            },
        )
        .expect("debug suspension bindings should compile");
        let release = splitscript::compile_with_options(
            &source,
            CompilerOptions {
                profile: BuildProfile::Release,
                ..CompilerOptions::default()
            },
        )
        .expect("release should type-check and erase debug suspension bindings");
        Validator::new_with_features(WasmFeatures::all())
            .validate_all(&debug)
            .unwrap();
        Validator::new_with_features(WasmFeatures::all())
            .validate_all(&release)
            .unwrap();
        assert!(release.len() < debug.len());
        assert!(!contains_string_literal(&release, b"debug-only"));
    }
}

#[test]
fn debug_bindings_are_visible_only_from_debug_code() {
    for source in [
        r#"
            state "game.exe" {}
            debug let hidden = 1
            whileAttached { print(hidden as String) }
        "#,
        r#"
            state "game.exe" {}
            whileAttached {
                debug let hidden = 1
                print(hidden as String)
            }
        "#,
        r#"
            state "game.exe" {}
            debug let hidden = 1
            whileAttached { hidden = 2 }
        "#,
    ] {
        let errors = splitscript::compile(source)
            .expect_err("retained code must not reference an erased binding");
        assert!(errors.iter().any(|error| {
            error
                .message
                .contains("debug-only binding `hidden` can only be used from debug code")
        }));
    }

    splitscript::compile(
        r#"
            state "game.exe" {}
            debug let hidden = 1
            whileAttached {
                debug let local = hidden + 1
                debug print(local as String)
                debug hidden = local
            }
        "#,
    )
    .expect("debug statements may share debug globals and local bindings");
}

#[test]
fn debug_modifier_rejects_terminators_and_duplicates() {
    for statement in ["debug return", "debug throw \"failure\""] {
        let source = format!(r#"state "game.exe" {{}} onAttach {{ {statement} }}"#);
        let errors = splitscript::compile(&source).expect_err("unsupported debug form must fail");
        assert!(
            errors
                .iter()
                .any(|error| error.message.contains("`debug` currently supports"))
        );
    }

    let errors = splitscript::compile(
        r#"state "game.exe" {} whileAttached { debug debug print("nested") }"#,
    )
    .expect_err("duplicate debug modifiers must fail");
    assert!(
        errors
            .iter()
            .any(|error| error.message.contains("more than one `debug` modifier"))
    );

    let errors = splitscript::compile(
        r#"
            state "game.exe" {}
            debug fn trace() { print("trace") }
            whileAttached { trace() }
        "#,
    )
    .expect_err("release-visible code must not call a debug-only function");
    assert!(errors.iter().any(|error| {
        error
            .message
            .contains("debug-only function `trace` can only be called from debug code")
    }));
}

#[test]
fn compiles_a_complete_autosplitter_to_valid_wasm_gc() {
    let wasm = splitscript::compile(EXAMPLE).expect("example should compile");
    Validator::new_with_features(WasmFeatures::all())
        .validate_all(&wasm)
        .expect("generated WebAssembly GC should validate");
    let metadata = Parser::new(0)
        .parse_all(&wasm)
        .find_map(
            |payload| match payload.expect("generated module should parse") {
                Payload::CustomSection(section) if section.name() == "splitscript" => {
                    Some(serde_json::from_slice::<serde_json::Value>(section.data()).unwrap())
                }
                _ => None,
            },
        )
        .expect("generated modules should identify their compiler");
    assert_eq!(
        metadata["compiler"]["version"],
        splitscript::COMPILER_VERSION
    );
    assert_eq!(metadata["target"], "wasm-gc");
    assert_eq!(metadata["hostAbi"], "livesplit-auto-splitting");
    match splitscript::COMPILER_GIT_REVISION {
        Some(revision) => assert_eq!(metadata["compiler"]["gitRevision"], revision),
        None => assert!(metadata["compiler"]["gitRevision"].is_null()),
    }
}

#[test]
fn linear_memory_grows_only_for_large_strings_used_by_the_linear_abi() {
    for (body, minimum_pages) in [
        // Settings reserve ABI scratch before their linear string data.
        (
            format!("settings {{ \"{}\" => enabled: true }}", "x".repeat(70_000)),
            3,
        ),
        (
            format!("whileAttached {{ print(\"{}\") }}", "x".repeat(70_000)),
            1,
        ),
    ] {
        let source = format!("state \"game.exe\" {{}}\n{body}");
        let (wasm, report) = release_emission(&source);
        assert_eq!(report.minimum_memory_pages, minimum_pages);
        Validator::new_with_features(WasmFeatures::all())
            .validate_all(&wasm)
            .expect("large strings must validate in either representation");
    }
}

#[test]
fn linear_memory_moves_static_data_after_large_read_scratch() {
    let chunk_fields = (0..32)
        .map(|index| format!("field{index}: u64,"))
        .collect::<Vec<_>>()
        .join("\n");
    let large_fields = (0..260)
        .map(|index| format!("chunk{index}: Chunk,"))
        .collect::<Vec<_>>()
        .join("\n");
    let source = format!(
        r#"
            struct Chunk {{
                {chunk_fields}
            }}
            struct Large {{
                {large_fields}
            }}
            state "game.exe" {{}}
            whileAttached {{
                let value: Large! = process.read(0x100)
            }}
        "#
    );
    let wasm =
        splitscript::compile(&source).expect("large readable structs should size scratch storage");
    let minimum_pages = Parser::new(0)
        .parse_all(&wasm)
        .find_map(
            |payload| match payload.expect("generated module should parse") {
                Payload::MemorySection(memories) => Some(
                    memories
                        .into_iter()
                        .next()
                        .expect("generated module should declare memory")
                        .expect("generated memory should parse")
                        .initial,
                ),
                _ => None,
            },
        )
        .expect("generated module should contain a memory section");

    assert!(minimum_pages >= 2);
    Validator::new_with_features(WasmFeatures::all())
        .validate_all(&wasm)
        .expect("large-struct WebAssembly GC should validate");
}

#[test]
fn generated_module_requires_gc() {
    let wasm = splitscript::compile(EXAMPLE).expect("example should compile");
    let features = WasmFeatures::all() - WasmFeatures::GC;
    assert!(
        Validator::new_with_features(features)
            .validate_all(&wasm)
            .is_err()
    );
}

#[test]
fn compiles_attach_await_and_print_hello_world() {
    let wasm = splitscript::compile(HELLO).expect("hello world should compile");
    Validator::new_with_features(WasmFeatures::all())
        .validate_all(&wasm)
        .expect("hello world WebAssembly GC should validate");
    for expected in [
        b"Lunistice-Demo.exe".as_slice(),
        b"GameAssembly.dll".as_slice(),
        b"Hello, world from SplitScript!".as_slice(),
    ] {
        assert!(contains_string_literal(&wasm, expected));
    }
}

#[test]
fn compiles_the_complete_settings_showcase() {
    let checked = splitscript::check(splitscript::parse(SETTINGS_EXAMPLE).unwrap())
        .expect("settings example should type-check");
    let choice = checked
        .syntax()
        .settings
        .iter()
        .find(|setting| {
            matches!(
                setting.kind,
                splitscript::compiler::ast::SettingKind::Choice { .. }
            )
        })
        .expect("settings example has a choice");
    let splitscript::compiler::ast::SettingKind::Choice {
        enumeration,
        default_variant,
        options,
        ..
    } = &choice.kind
    else {
        unreachable!();
    };
    let name = &enumeration.name;
    let declaration = checked
        .syntax()
        .enums
        .iter()
        .find(|item| item.name == *name)
        .unwrap();
    let expected_default = declaration
        .variants
        .iter()
        .find(|variant| variant.name == *default_variant)
        .unwrap()
        .id;
    assert_eq!(
        checked.semantics().setting_choice_default(choice.id),
        Some(expected_default)
    );
    assert_eq!(
        checked.typed_hir().setting_choice_default(choice.id),
        Some(expected_default)
    );
    for option in options {
        let expected = declaration
            .variants
            .iter()
            .find(|variant| variant.name == option.variant)
            .unwrap()
            .id;
        assert_eq!(
            checked.semantics().setting_choice_option(option.id),
            Some(expected)
        );
        assert_eq!(
            checked.typed_hir().setting_choice_option(option.id),
            Some(expected)
        );
    }

    let wasm = splitscript::codegen(&checked);
    Validator::new_with_features(WasmFeatures::all())
        .validate_all(&wasm)
        .expect("settings example WebAssembly GC should validate");
    for expected in [
        b"Enable Auto Splitting".as_slice(),
        b"Profile Name".as_slice(),
        b"Player".as_slice(),
        b"Capture Source".as_slice(),
        b"Layout File".as_slice(),
        b"image/*".as_slice(),
    ] {
        assert!(contains_string_literal(&wasm, expected));
    }
}

#[test]
fn owned_managed_class_equality_is_recursive_and_demand_driven() {
    let source = r#"
        image "Assembly-CSharp" {
            class Node { String label; [Node?] children; }
        }
        state "game.exe" {}
        fn same(left: Node, right: Node) -> bool { return left == right }
        setup { let callback = same }
    "#;
    let (wasm, report) = release_emission(source);
    Validator::new_with_features(WasmFeatures::all())
        .validate_all(&wasm)
        .unwrap();
    assert!(
        report
            .functions
            .iter()
            .any(|(_, name)| name == "__splitscript::equals::Node")
    );
    assert!(
        !report
            .functions
            .iter()
            .any(|(_, name)| name.contains("::snapshot") || name.contains("::managed::"))
    );
    let unused = source.replace("return left == right", "return true");
    let (_, report) = release_emission(&unused);
    assert!(
        !report
            .functions
            .iter()
            .any(|(_, name)| name.contains("::equals::"))
    );
}

#[test]
fn recursive_snapshot_equality_checks_every_field() {
    let source = r#"
        image "Assembly-CSharp" {
            class Node { Node? next; Node.Ref live; }
        }
        state "game.exe" {}
        fn same(left: Node, right: Node) -> bool { return left == right }
        setup { let callback = same }
    "#;
    let error = splitscript::compile(source)
        .map(|wasm| wasm.len())
        .unwrap_err();
    assert!(format!("{error:?}").contains("equality"));
}

#[test]
fn snapshot_equality_composes_through_recursive_maps_and_sets() {
    let source = r#"
        image "Assembly-CSharp" {
            class Node { Map<String, Set<Node?>> neighbors; }
        }
        state "game.exe" {}
        fn same(left: Node, right: Node) -> bool { return left == right }
        setup { let callback = same }
    "#;
    let (wasm, report) = release_emission(source);
    Validator::new_with_features(WasmFeatures::all())
        .validate_all(&wasm)
        .unwrap();
    for kind in ["Node", "map#", "set#"] {
        assert!(
            report
                .functions
                .iter()
                .any(|(_, name)| name.starts_with(&format!("__splitscript::equals::{kind}")))
        );
    }
    assert!(
        !report
            .functions
            .iter()
            .any(|(_, name)| name.contains("::managed::"))
    );
}

#[test]
fn storing_nested_collections_does_not_emit_collection_equality() {
    for collection in [
        "Map.new<String, Set<i32>>()",
        "Set.new<Map<String, Set<i32>>>()",
    ] {
        let source = format!(
            r#"
            state "game.exe" {{}}
            let values = {collection}
            whileAttached {{ print(values.length()) }}
        "#
        );
        let (wasm, report) = release_emission(&source);
        Validator::new_with_features(WasmFeatures::all())
            .validate_all(&wasm)
            .unwrap();
        assert!(
            !report
                .functions
                .iter()
                .any(|(_, name)| name.starts_with("__splitscript::equals::map#")
                    || name.starts_with("__splitscript::equals::set#")),
            "{collection}"
        );
    }
}

#[test]
fn unused_managed_list_key_schemas_add_no_wasm() {
    let native = r#"state "game.exe" {}"#;
    let (expected, _) = release_emission(native);
    for field in [
        "Set<List<String>>",
        "Map<List<List<String?>>, List<String?>>",
    ] {
        let source = format!(
            r#"
            image "Assembly-CSharp" {{ class Root {{ static {field} values; }} }}
            {native}
        "#
        );
        let (wasm, _) = release_emission(&source);
        assert_eq!(wasm, expected, "{field}");
    }
}

#[test]
fn managed_comparison_budget_helpers_follow_read_demand() {
    let schema = r#"
        enum Code: u32 { First, Second, Third }
        image "Assembly-CSharp" {
            class Key { String label; Code code; }
            class Root { static Set<Key> values; }
        }
    "#;
    let read = format!("{schema} state Unity [\"game.exe\"] {{ values = Root.values?; }}");
    let (wasm, report) = release_emission(&read);
    Validator::new_with_features(WasmFeatures::all())
        .validate_all(&wasm)
        .unwrap();
    assert!(
        report
            .functions
            .iter()
            .any(|(_, name)| name.contains("::managed_equals::"))
    );
    assert!(
        !report
            .functions
            .iter()
            .any(|(_, name)| name == "__splitscript::equals::Key")
    );

    let (_, report) = release_emission(&format!(
        "{read} whileAttached {{ setVariable(\"same\", current.values == old.values) }}"
    ));
    assert!(
        report
            .functions
            .iter()
            .any(|(_, name)| name.contains("::managed_equals::"))
    );
    assert!(
        report
            .functions
            .iter()
            .any(|(_, name)| name == "__splitscript::equals::Key")
    );

    let local = format!(
        "{schema} state \"game.exe\" {{}} fn same(a: Key, b: Key) -> bool {{ return a == b }} setup {{ let callback = same }}"
    );
    let (_, report) = release_emission(&local);
    assert!(
        !report
            .functions
            .iter()
            .any(|(_, name)| name.contains("::managed_equals::"))
    );
    assert!(
        !report
            .runtime_helpers
            .iter()
            .any(|name| name.contains("ManagedWork"))
    );
}
