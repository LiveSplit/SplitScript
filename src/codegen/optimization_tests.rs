//! Regression coverage and opt-in size/cost measurements for the Release pipeline.
const CORPUS: &[(&str, &str)] = &[
    ("minish_cap", "examples/minish_cap.split"),
    ("unity_explicit", "examples/lunistice.split"),
    ("native", "examples/neon_white.split"),
    ("large_native", "examples/a_hat_in_time.split"),
    ("unity_automatic", "tests/managed_live_identity.split"),
    ("collections", "tests/managed_map_inline.split"),
    ("nested_collections", "tests/managed_nested_map.split"),
    ("async", "examples/cancellation.split"),
    ("async_loop", "tests/async_loop.split"),
];

/// Optional local scripts stay outside the repository and normal test suite.
/// The manifest is a JSON array of [artifact name, source path] pairs.
fn measurement_corpus() -> Vec<(String, String)> {
    let mut corpus: Vec<_> = CORPUS
        .iter()
        .map(|&(name, path)| (name.to_owned(), path.to_owned()))
        .collect();
    if let Some(path) = std::env::var_os("SPLITSCRIPT_SIZE_EXTRA_CORPUS") {
        let extra: Vec<(String, String)> =
            serde_json::from_slice(&std::fs::read(path).unwrap()).unwrap();
        for (name, path) in extra {
            assert!(
                !name.is_empty() && name.bytes().all(|b| b.is_ascii_alphanumeric() || b == b'_')
            );
            assert!(!corpus.iter().any(|(existing, _)| existing == &name));
            corpus.push((name, path));
        }
    }
    corpus
}

fn validate(wasm: &[u8]) {
    wasmparser::Validator::new_with_features(wasmparser::WasmFeatures::all())
        .validate_all(wasm)
        .unwrap();
}

fn stable_debug_bytes(wasm: &[u8]) -> Vec<u8> {
    // Existing DWARF variable entries iterate randomized local maps. Exclude
    // only that unstable section; executable code, names, source maps and
    // line tables must remain byte-identical when optimization is requested.
    let mut bytes = wasm[..8].to_vec();
    for payload in wasmparser::Parser::new(0).parse_all(wasm) {
        let payload = payload.unwrap();
        if matches!(&payload, wasmparser::Payload::CustomSection(s) if s.name() == ".debug_info") {
            continue;
        }
        if let Some((id, range)) = payload.as_section() {
            bytes.push(id);
            bytes.extend_from_slice(&(range.len() as u64).to_le_bytes());
            bytes.extend_from_slice(&wasm[range]);
        }
    }
    bytes
}

fn compile(source: &str, profile: crate::BuildProfile, optimize: bool) -> Vec<u8> {
    let checked = crate::check(crate::lower(crate::parse(source).unwrap())).unwrap();
    let options = crate::CompilerOptions {
        profile,
        ..Default::default()
    };
    super::compile_internal(
        crate::lower_wasm_with_options(&checked, options),
        None,
        if optimize {
            super::OptimizationMode::Size
        } else {
            super::OptimizationMode::None
        },
    )
}

#[test]
fn debug_is_byte_identical_and_release_shrinks() {
    let source =
        r#"state "game.exe" {} fn answer() -> i32 { return 42 } start { return answer() == 42 }"#;
    let debug = compile(source, crate::BuildProfile::Debug, true);
    assert_eq!(debug, compile(source, crate::BuildProfile::Debug, false));
    assert_eq!(debug, crate::compile(source).unwrap());
    let release = compile(source, crate::BuildProfile::Release, true);
    assert!(release.len() < compile(source, crate::BuildProfile::Release, false).len());
    validate(&debug);
    validate(&release);
}

#[test]
fn source_effects_gc_results_failures_and_suspension_match_without_optimization() {
    let source = r#"
        state "game.exe" {}
        let calls = 0
        struct Boxed { value: i32, }
        fn first() -> i32 { calls = calls * 10 + 1; return 11 }
        fn second() -> i32 { calls = calls * 10 + 2; return 3 }
        fn subtract(a: i32, b: i32) -> i32 { return a - b }
        fn early() -> i32 { if calls == 12 { return 7 } return 9 }
        fn fail() -> i32! { throw "failure" }
        fn boxed() -> Boxed { return Boxed { value: 42 } }
        fn delayed() -> async i32 { await nextTick(); return 5 }
        onAttach {
            let difference = subtract(first(), second())
            let fallback = fail() else 4
            let branch = early()
            let object = boxed()
            let later = await delayed()
            setVariable("result", `{difference},{calls},{fallback},{branch},{object.value},{later}`)
        }
    "#;
    let mut config = wasmtime::Config::new();
    config.wasm_gc(true).wasm_function_references(true);
    let engine = wasmtime::Engine::new(&config).unwrap();
    let baseline = compile(source, crate::BuildProfile::Release, false);
    let optimized = compile(source, crate::BuildProfile::Release, true);
    assert!(optimized.len() < baseline.len());
    let sized = crate::compile_with_options(
        source,
        crate::CompilerOptions {
            profile: crate::BuildProfile::Release,
            ..Default::default()
        },
    )
    .unwrap();
    assert_eq!(optimized, sized);
    for wasm in [baseline, optimized] {
        assert_eq!(run_script(&engine, &wasm), ["8,12,4,7,42,5"]);
    }
}

fn run_script(engine: &wasmtime::Engine, wasm: &[u8]) -> Vec<String> {
    validate(wasm);
    let module = wasmtime::Module::new(engine, wasm).unwrap();
    let mut linker = wasmtime::Linker::<Vec<String>>::new(engine);
    linker
        .func_wrap("env", "process_attach", |_: i32, _: i32| 1_i64)
        .unwrap();
    linker
        .func_wrap("env", "process_detach", |_: i64| {})
        .unwrap();
    linker
        .func_wrap("env", "process_is_open", |_: i64| 1_i32)
        .unwrap();
    linker
        .func_wrap("env", "timer_get_state", || 0_i32)
        .unwrap();
    linker
        .func_wrap("env", "runtime_set_tick_rate", |_: f64| {})
        .unwrap();
    linker
        .func_wrap(
            "env",
            "timer_set_variable",
            |mut caller: wasmtime::Caller<'_, Vec<String>>,
             _: i32,
             _: i32,
             pointer: i32,
             length: i32| {
                let memory = caller.get_export("memory").unwrap().into_memory().unwrap();
                let value = String::from_utf8(
                    memory.data(&caller)[pointer as usize..(pointer + length) as usize].to_vec(),
                )
                .unwrap();
                caller.data_mut().push(value);
            },
        )
        .unwrap();
    let mut store = wasmtime::Store::new(engine, Vec::new());
    let instance = linker.instantiate(&mut store, &module).unwrap();
    instance
        .get_typed_func::<(), ()>(&mut store, "_start")
        .unwrap()
        .call(&mut store, ())
        .unwrap();
    let update = instance
        .get_typed_func::<(), ()>(&mut store, "update")
        .unwrap();
    for _ in 0..4 {
        update.call(&mut store, ()).unwrap();
    }
    store.into_data()
}

#[test]
fn default_enum_constructors_preserve_payloads_effects_and_wrapper_conversions() {
    let source = r#"
        state "game.exe" {}
        enum Mixed { Empty, Byte(u8), Signed(i8), Wide(i64), Small(f32), Float(f64), Text(String) }
        enum First { Value(i64), Empty }
        enum Negative { Value(f64), Empty }
        let calls = 0
        fn zero() -> i64 { calls = calls + 1; return 0 }
        fn optional() -> Mixed? { return Mixed.Empty }
        fn fallible() -> Mixed! { return Mixed.Empty }
        onAttach {
            let empty = Mixed.Empty
            let same = empty == Mixed.Empty
            let byte = match Mixed.Byte(255) { Mixed.Byte(value) => value as i32, _ => -1 }
            let first = match First.Value(zero()) { First.Value(value) => value, _ => -1 }
            let negative = match Negative.Value(-0.0) {
                Negative.Value(value) => 1.0 / value < 0.0,
                _ => false,
            }
            let option = optional() else Mixed.Text("wrong")
            let result = fallible() else Mixed.Text("wrong")
            setVariable("result", `{same},{byte},{first},{negative},{calls},{option == empty},{result == empty}`)
        }
    "#;
    let mut config = wasmtime::Config::new();
    config.wasm_gc(true).wasm_function_references(true);
    let engine = wasmtime::Engine::new(&config).unwrap();
    for profile in [crate::BuildProfile::Debug, crate::BuildProfile::Release] {
        for optimize in [false, true] {
            let wasm = compile(source, profile, optimize);
            let defaults = wasmparser::Parser::new(0)
                .parse_all(&wasm)
                .filter_map(|payload| match payload.unwrap() {
                    wasmparser::Payload::CodeSectionEntry(body) => Some(
                        body.get_operators_reader()
                            .unwrap()
                            .into_iter()
                            .filter(|op| {
                                matches!(op, Ok(wasmparser::Operator::StructNewDefault { .. }))
                            })
                            .count(),
                    ),
                    _ => None,
                })
                .sum::<usize>();
            assert!(defaults > 0, "default constructors missing in {profile:?}");
            assert_eq!(run_script(&engine, &wasm), ["true,255,0,true,1,true,true"]);
        }
    }
}

#[test]
fn real_scripts_shrink_and_reports_track_exported_function_indices() {
    for &(_, path) in &CORPUS[..4] {
        let source = std::fs::read_to_string(path).unwrap();
        let checked = crate::check(crate::lower(crate::parse(&source).unwrap())).unwrap();
        let options = crate::CompilerOptions {
            profile: crate::BuildProfile::Release,
            ..Default::default()
        };
        let mut original_report = super::CodegenReport::default();
        let baseline = super::compile_internal(
            crate::lower_wasm_with_options(&checked, options),
            Some(&mut original_report),
            super::OptimizationMode::None,
        );
        let (optimized, mut report) =
            super::compile_with_report(crate::lower_wasm_with_options(&checked, options));
        validate(&baseline);
        validate(&optimized);
        assert!(optimized.len() < baseline.len(), "{path}");
        assert_eq!(
            optimized,
            super::compile(crate::lower_wasm_with_options(&checked, options))
        );
        let exported_names = |wasm: &[u8], report: &super::CodegenReport| {
            let mut names = Vec::new();
            for p in wasmparser::Parser::new(0).parse_all(wasm) {
                if let wasmparser::Payload::ExportSection(exports) = p.unwrap() {
                    for export in exports {
                        let export = export.unwrap();
                        if export.kind == wasmparser::ExternalKind::Func {
                            let (_, name) = report
                                .functions
                                .iter()
                                .find(|(i, _)| *i == export.index)
                                .unwrap();
                            names.push((export.name.to_owned(), name.clone()));
                        }
                    }
                }
            }
            names
        };
        assert_eq!(
            exported_names(&baseline, &original_report),
            exported_names(&optimized, &report)
        );
        let mut indices = report.functions.iter().map(|(i, _)| *i).collect::<Vec<_>>();
        indices.sort_unstable();
        indices.dedup();
        assert_eq!(indices.len(), report.functions.len());
        report.functions = original_report.functions.clone();
        report.inlined_functions.clear();
        assert_eq!(report, original_report);
    }
}

#[test]
#[ignore = "manual size comparison; writes validated Wasm and corpus.json"]
fn write_size_corpus() {
    let output = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("target/size-check");
    std::fs::create_dir_all(&output).unwrap();
    let mut rows = Vec::new();
    for (name, path) in measurement_corpus() {
        let source = std::fs::read_to_string(&path).unwrap();
        let checked = crate::check(crate::lower(crate::parse(&source).unwrap())).unwrap();
        let compile_checked = |profile, mode| {
            let options = crate::CompilerOptions {
                profile,
                ..Default::default()
            };
            super::compile_internal(
                crate::lower_wasm_with_options(&checked, options),
                None,
                mode,
            )
        };
        let mut row = serde_json::json!({"name": name, "source": path});
        row["external"] = (!CORPUS.iter().any(|&(builtin, _)| builtin == name)).into();
        for (label, enabled) in [("baseline", false), ("release", true)] {
            let wasm = compile_checked(
                crate::BuildProfile::Release,
                if enabled {
                    super::OptimizationMode::Size
                } else {
                    super::OptimizationMode::None
                },
            );
            validate(&wasm);
            row[label] = wasm.len().into();
            let sections = wasmparser::Parser::new(0)
                .parse_all(&wasm)
                .filter_map(|p| {
                    p.unwrap()
                        .as_section()
                        .map(|(id, range)| (id.to_string(), range.len().into()))
                })
                .collect::<serde_json::Map<String, serde_json::Value>>();
            row[format!("{label}_sections")] = sections.into();
            std::fs::write(output.join(format!("{name}.{label}.wasm")), wasm).unwrap();
        }
        let debug = compile_checked(crate::BuildProfile::Debug, super::OptimizationMode::Size);
        let unoptimized_debug =
            compile_checked(crate::BuildProfile::Debug, super::OptimizationMode::None);
        if debug != unoptimized_debug {
            std::fs::write(output.join(format!("{name}.debug.wasm")), &debug).unwrap();
            std::fs::write(
                output.join(format!("{name}.debug_baseline.wasm")),
                &unoptimized_debug,
            )
            .unwrap();
            let repeated_baseline =
                compile_checked(crate::BuildProfile::Debug, super::OptimizationMode::None);
            std::fs::write(
                output.join(format!("{name}.debug_baseline_repeat.wasm")),
                &repeated_baseline,
            )
            .unwrap();
            assert!(
                stable_debug_bytes(&unoptimized_debug) == stable_debug_bytes(&repeated_baseline),
                "{name}: unstable Debug executable"
            );
        }
        assert!(
            stable_debug_bytes(&debug) == stable_debug_bytes(&unoptimized_debug),
            "{name}: Debug executable or stable metadata differ"
        );
        validate(&debug);
        assert!(row["release"].as_u64().unwrap() <= row["baseline"].as_u64().unwrap());
        eprintln!(
            "{name}: {} -> {} bytes; Debug code and stable metadata identical",
            row["baseline"], row["release"]
        );
        rows.push(row);
    }
    std::fs::write(
        output.join("corpus.json"),
        serde_json::to_vec_pretty(&rows).unwrap(),
    )
    .unwrap();
}

#[test]
#[ignore = "manual optimized-host backend timing; writes timing.json"]
fn measure_optimization_overhead() {
    if cfg!(debug_assertions) {
        panic!("use cargo test --profile max-opt for meaningful backend timings");
    }
    let root = std::path::Path::new(env!("CARGO_MANIFEST_DIR"));
    let output = root.join("target/size-check");
    std::fs::create_dir_all(&output).unwrap();
    let mut rows = Vec::new();
    for (name, path) in measurement_corpus() {
        let source = std::fs::read_to_string(root.join(path)).unwrap();
        let checked = crate::check(crate::lower(crate::parse(&source).unwrap())).unwrap();
        let options = crate::CompilerOptions {
            profile: crate::BuildProfile::Release,
            ..Default::default()
        };
        let mut samples = [Vec::new(), Vec::new()];
        // Warm both paths; alternate order to reduce systematic cache/order
        // bias. Parsing/checking are excluded, lowering/emission are included.
        for repeat in 0..8 {
            for position in 0..2 {
                let index = (repeat + position) % 2;
                let mode = if index == 0 {
                    super::OptimizationMode::None
                } else {
                    super::OptimizationMode::Size
                };
                let start = std::time::Instant::now();
                std::hint::black_box(super::compile_internal(
                    crate::lower_wasm_with_options(&checked, options),
                    None,
                    mode,
                ));
                if repeat != 0 {
                    samples[index].push(start.elapsed().as_micros());
                }
            }
        }
        for values in &mut samples {
            values.sort_unstable();
        }
        let before = samples[0][3];
        let after = samples[1][3];
        eprintln!("{name}: median backend {before} -> {after} us");
        rows.push(serde_json::json!({ "name": name, "baseline_us": before, "release_us": after, "samples_us": samples }));
    }
    std::fs::write(
        output.join("timing.json"),
        serde_json::to_vec_pretty(&rows).unwrap(),
    )
    .unwrap();
}
