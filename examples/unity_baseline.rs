//! Unity migration gate. Run through `cargo xtask unity-baseline`.
//! Reports describe raw Release Wasm; no optimizer or debug module is involved.

use std::{collections::BTreeMap, env, fs, path::Path, process::Command, time::Instant};

use serde::{Deserialize, Serialize};
use splitscript::{BuildProfile, CompilerOptions, compiler::CodegenReport};
use wasmparser::{Parser, Payload, Validator, WasmFeatures};

const BASELINE: &str = "tests/baselines/unity.json";
const OUTPUT: &str = "target/unity-baseline";

#[derive(Debug, Serialize, Deserialize)]
struct Report {
    format_version: u32,
    compiler: String,
    rustc: String,
    node: String,
    compiler_build: String,
    artifact_profile: String,
    fingerprint_algorithm: String,
    fixtures: BTreeMap<String, FixtureReport>,
}

#[derive(Debug, Serialize, Deserialize)]
struct FixtureReport {
    source_fingerprint: String,
    compile_micros: u128,
    module_bytes: usize,
    /// Payload and framing together; the 8-byte Wasm header is separate.
    sections: BTreeMap<String, usize>,
    defined_functions: u32,
    types: usize,
    function_body_bytes: BTreeMap<String, usize>,
    emission: CodegenReport,
}

fn schema(provider: &str, field: &str) -> String {
    format!(
        "image \"Assembly-CSharp\" {{ class Probe {{ {field} }} }}\n\
         state {provider} [\"game.exe\"] {{ value = Probe.value?; }}\n"
    )
}

fn fixtures() -> BTreeMap<String, String> {
    let lunistice = include_str!("lunistice.split").replace("\r\n", "\n");
    let selector = "Unity.il2cpp(Il2CppProfile.unity2022_3_0f1X64())";
    assert_eq!(
        lunistice.matches(selector).count(),
        1,
        "update the auto-profile fixture when the explicit API changes"
    );
    BTreeMap::from([
        ("native".into(), "state \"game.exe\" {}".into()),
        (
            "unity-nested-metadata".into(),
            include_str!("../tests/unity_nested_metadata.split").into(),
        ),
        (
            "il2cpp-auto-profile".into(),
            include_str!("../tests/il2cpp_profiles.split").into(),
        ),
        (
            "il2cpp-x86-profile".into(),
            include_str!("../tests/il2cpp_profile_explicit_x86.split").into(),
        ),
        (
            "pe-debug-id".into(),
            include_str!("../tests/pe_debug_id.split").into(),
        ),
        (
            "elf-build-id".into(),
            include_str!("../tests/elf_build_id.split").into(),
        ),
        (
            "mach-uuid".into(),
            include_str!("../tests/mach_uuid.split").into(),
        ),
        (
            "local-map".into(),
            include_str!("../tests/map_runtime.split").into(),
        ),
        (
            "local-set".into(),
            include_str!("../tests/set_runtime.split").into(),
        ),
        (
            "il2cpp-scalar".into(),
            schema(selector, "static i32 value;"),
        ),
        (
            "il2cpp-string".into(),
            schema(selector, "static String value;"),
        ),
        (
            "il2cpp-unused-string".into(),
            schema(selector, "static i32 value; static String unused;"),
        ),
        (
            "mono-scalar".into(),
            schema("Unity.mono(MonoVersion.V2)", "static i32 value;"),
        ),
        (
            "mono-profiles-auto".into(),
            include_str!("../tests/mono_profiles.split").into(),
        ),
        (
            "mono-old".into(),
            include_str!("../tests/mono_profiles_V1.split").into(),
        ),
        (
            "mono-string".into(),
            schema("Unity.mono(MonoVersion.V2)", "static String value;"),
        ),
        (
            "mono-unused-string".into(),
            schema(
                "Unity.mono(MonoVersion.V2)",
                "static i32 value; static String unused;",
            ),
        ),
        ("lunistice".into(), lunistice.clone()),
        (
            "lunistice-auto".into(),
            lunistice.replace(selector, "Unity"),
        ),
        (
            "mono-inherited-static".into(),
            include_str!("../tests/managed_inherited_static_runtime.split").into(),
        ),
        (
            "il2cpp-instances".into(),
            include_str!("../tests/managed_instances_runtime.split").into(),
        ),
        (
            "mono-instances".into(),
            include_str!("../tests/managed_instances_mono_runtime.split").into(),
        ),
    ])
}

// A reproducible source fingerprint, not a security or binary-identity hash.
fn fingerprint(source: &str) -> String {
    let hash = source.bytes().fold(0xcbf2_9ce4_8422_2325u64, |hash, byte| {
        (hash ^ u64::from(byte)).wrapping_mul(0x100_0000_01b3)
    });
    format!("{hash:016x}")
}

fn measure(source: &str) -> Result<(Vec<u8>, FixtureReport), String> {
    let options = CompilerOptions {
        profile: BuildProfile::Release,
        ..Default::default()
    };
    let started = Instant::now();
    let checked = splitscript::check(splitscript::lower(
        splitscript::parse(source).map_err(|errors| format!("{errors:?}"))?,
    ))
    .map_err(|errors| format!("{errors:?}"))?;
    let (wasm, emission) = splitscript::compiler::codegen_with_report(&checked, options);
    let compile_micros = started.elapsed().as_micros();
    Validator::new_with_features(WasmFeatures::all())
        .validate_all(&wasm)
        .map_err(|error| error.to_string())?;
    let mut report = FixtureReport {
        source_fingerprint: fingerprint(source),
        compile_micros,
        module_bytes: wasm.len(),
        sections: BTreeMap::new(),
        defined_functions: 0,
        types: 0,
        function_body_bytes: BTreeMap::new(),
        emission,
    };
    let mut previous_end = 8;
    let mut body_index = 0;
    for payload in Parser::new(0).parse_all(&wasm) {
        let payload = payload.map_err(|error| error.to_string())?;
        if let Some((id, range)) = payload.as_section() {
            let name = match &payload {
                Payload::CustomSection(section) => format!("custom:{}", section.name()),
                _ => format!("{id}:{}", section_name(id)),
            };
            *report.sections.entry(name).or_default() += range.end - previous_end;
            previous_end = range.end;
        }
        match payload {
            Payload::TypeSection(section) => {
                for group in section {
                    report.types += group.map_err(|error| error.to_string())?.types().len();
                }
            }
            Payload::FunctionSection(section) => report.defined_functions = section.count(),
            Payload::CodeSectionEntry(body) => {
                let (index, name) = report
                    .emission
                    .functions
                    .get(body_index)
                    .ok_or("missing emitted function name")?;
                // Include the index: multiple monomorphizations may share a display name.
                report
                    .function_body_bytes
                    .insert(format!("{index}:{name}"), body.range().len());
                body_index += 1;
            }
            _ => {}
        }
    }
    if report.sections.values().sum::<usize>() + 8 != wasm.len()
        || body_index != report.defined_functions as usize
        || body_index != report.emission.functions.len()
    {
        return Err("section/function report does not account for the emitted module".into());
    }
    Ok((wasm, report))
}

fn section_name(id: u8) -> &'static str {
    match id {
        1 => "type",
        2 => "import",
        3 => "function",
        4 => "table",
        5 => "memory",
        6 => "global",
        7 => "export",
        8 => "start",
        9 => "element",
        10 => "code",
        11 => "data",
        12 => "data-count",
        13 => "tag",
        _ => "unknown",
    }
}

fn version(command: &str) -> Result<String, String> {
    let output = Command::new(command)
        .arg("--version")
        .output()
        .map_err(|error| error.to_string())?;
    if !output.status.success() {
        return Err(format!("{command} --version failed"));
    }
    Ok(String::from_utf8_lossy(&output.stdout).trim().to_owned())
}

fn bodies_by_name(report: &FixtureReport) -> BTreeMap<&str, Vec<usize>> {
    let mut bodies = BTreeMap::<&str, Vec<usize>>::new();
    for (key, bytes) in &report.function_body_bytes {
        let (_, name) = key
            .split_once(':')
            .expect("function keys include their index");
        bodies.entry(name).or_default().push(*bytes);
    }
    // Names can be shared by monomorphizations; compare their size multiset
    // independently of index shifts caused by other emitted functions.
    for sizes in bodies.values_mut() {
        sizes.sort_unstable();
    }
    bodies
}

fn compare(previous: &Report, current: &Report) -> Vec<String> {
    let mut changes = Vec::new();
    if previous.format_version != current.format_version
        || previous.artifact_profile != current.artifact_profile
        || previous.fingerprint_algorithm != current.fingerprint_algorithm
    {
        changes.push("baseline format/profile differs; compare with a compatible report".into());
    }
    if previous.rustc != current.rustc
        || previous.node != current.node
        || previous.compiler_build != current.compiler_build
    {
        changes.push(
            "toolchain/compiler build differs; review and remeasure under identical tools".into(),
        );
    }
    for (name, now) in &current.fixtures {
        let Some(before) = previous.fixtures.get(name) else {
            changes.push(format!("{name}: new fixture requires a reviewed baseline"));
            continue;
        };
        println!(
            "{name:24} {:7} -> {:7} ({:+})",
            before.module_bytes,
            now.module_bytes,
            now.module_bytes as i64 - before.module_bytes as i64
        );
        if before.source_fingerprint != now.source_fingerprint {
            changes.push(format!(
                "{name}: source changed; review the API/fixture migration"
            ));
        }
        let mut growth = |label: &str, old: u64, new: u64| {
            if new > old {
                changes.push(format!(
                    "{name}: {label} grew {old} -> {new} (+{})",
                    new - old
                ));
            }
        };
        growth(
            "module bytes",
            before.module_bytes as u64,
            now.module_bytes as u64,
        );
        growth(
            "scratch bytes",
            before.emission.scratch_bytes,
            now.emission.scratch_bytes,
        );
        growth(
            "ABI scratch",
            before.emission.abi_read_capacity.into(),
            now.emission.abi_read_capacity.into(),
        );
        growth(
            "memory pages",
            before.emission.minimum_memory_pages,
            now.emission.minimum_memory_pages,
        );
        growth(
            "functions",
            before.defined_functions.into(),
            now.defined_functions.into(),
        );
        growth("types", before.types as u64, now.types as u64);
        for (section, bytes) in &now.sections {
            growth(
                section,
                *before.sections.get(section).unwrap_or(&0) as u64,
                *bytes as u64,
            );
        }
        for helper in &now.emission.runtime_helpers {
            if !before.emission.runtime_helpers.contains(helper) {
                changes.push(format!("{name}: newly retained runtime helper {helper}"));
            }
        }
        // Names as a multiset catch newly retained discovery functions even
        // when an unrelated optimization makes the complete module smaller.
        let names = |report: &FixtureReport| {
            let mut names = BTreeMap::<String, usize>::new();
            for (_, name) in &report.emission.functions {
                *names.entry(name.clone()).or_default() += 1;
            }
            names
        };
        let old_names = names(before);
        for (function, count) in names(now) {
            if count > *old_names.get(&function).unwrap_or(&0) {
                changes.push(format!("{name}: newly retained function {function}"));
            }
        }
        let before_bodies = bodies_by_name(before);
        for (function, sizes) in bodies_by_name(now) {
            let Some(old_sizes) = before_bodies.get(function) else {
                continue;
            };
            for (instance, (old, new)) in old_sizes.iter().zip(&sizes).enumerate() {
                if new > old {
                    changes.push(format!(
                        "{name}: {function} body {instance} grew {old} -> {new} (+{})",
                        new - old
                    ));
                }
            }
        }
    }
    for name in previous.fixtures.keys() {
        if !current.fixtures.contains_key(name) {
            changes.push(format!("{name}: baseline fixture was removed"));
        }
    }
    changes
}

fn run() -> Result<(), String> {
    let arguments = env::args().skip(1).collect::<Vec<_>>();
    let (record, baseline) = match arguments.as_slice() {
        [] => (false, BASELINE),
        [mode] if mode == "--record" => (true, BASELINE),
        [mode, path] if mode == "--compare" => (false, path.as_str()),
        _ => return Err("usage: unity_baseline [--record | --compare REPORT.json]".into()),
    };
    env::set_current_dir(env!("CARGO_MANIFEST_DIR")).map_err(|error| error.to_string())?;
    fs::create_dir_all(OUTPUT).map_err(|error| error.to_string())?;
    let mut report = Report {
        format_version: 1,
        compiler: splitscript::COMPILER_VERSION_TEXT.into(),
        rustc: version("rustc")?,
        node: version("node")?,
        compiler_build: if cfg!(debug_assertions) {
            "debug"
        } else {
            "optimized"
        }
        .into(),
        artifact_profile: "release-unoptimized-wasm".into(),
        fingerprint_algorithm: "fnv1a64-lf-source".into(),
        fixtures: BTreeMap::new(),
    };
    for (name, source) in fixtures() {
        let source = source.replace("\r\n", "\n");
        let (wasm, measured) = measure(&source).map_err(|error| format!("{name}: {error}"))?;
        fs::write(format!("{OUTPUT}/{name}.wasm"), wasm).map_err(|error| error.to_string())?;
        report.fixtures.insert(name, measured);
    }
    let json = serde_json::to_string_pretty(&report).map_err(|error| error.to_string())? + "\n";
    fs::write(format!("{OUTPUT}/report.json"), &json).map_err(|error| error.to_string())?;
    // Always run both edition scenarios before accepting or recording a report.
    // Auto selection is size-only until that fixture models player identity.
    for edition in [None, Some("--dlc")] {
        let mut command = Command::new("node");
        command.args([
            "tests/lunistice_runtime.mjs",
            &format!("{OUTPUT}/lunistice.wasm"),
        ]);
        if let Some(edition) = edition {
            command.arg(edition);
        }
        let output = command.output().map_err(|error| error.to_string())?;
        if !output.status.success() {
            return Err(format!(
                "Lunistice {edition:?} failed:\n{}\n{}",
                String::from_utf8_lossy(&output.stdout),
                String::from_utf8_lossy(&output.stderr)
            ));
        }
        fs::write(
            format!(
                "{OUTPUT}/lunistice-{}.json",
                if edition.is_some() { "dlc" } else { "base" }
            ),
            output.stdout,
        )
        .map_err(|error| error.to_string())?;
    }
    if record {
        if let Some(parent) = Path::new(baseline).parent() {
            fs::create_dir_all(parent).map_err(|error| error.to_string())?;
        }
        fs::write(baseline, json).map_err(|error| error.to_string())?;
        println!("Recorded {baseline}; review its diff and explain each growth before committing.");
    } else {
        let previous: Report = serde_json::from_str(
            &fs::read_to_string(baseline).map_err(|error| format!("{baseline}: {error}"))?,
        )
        .map_err(|error| error.to_string())?;
        let changes = compare(&previous, &report);
        if !changes.is_empty() {
            return Err(format!(
                "Unity baseline review required:\n{}\nFull function/section report: {OUTPUT}/report.json",
                changes.join("\n")
            ));
        }
    }
    println!("Unity size gate and Lunistice base/DLC behavior passed.");
    Ok(())
}

fn main() -> std::process::ExitCode {
    match run() {
        Ok(()) => std::process::ExitCode::SUCCESS,
        Err(error) => {
            eprintln!("{error}");
            std::process::ExitCode::FAILURE
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn report() -> Report {
        let (_, fixture) = measure("state \"game.exe\" {}").unwrap();
        Report {
            format_version: 1,
            compiler: "test".into(),
            rustc: "test".into(),
            node: "test".into(),
            compiler_build: "test".into(),
            artifact_profile: "release-unoptimized-wasm".into(),
            fingerprint_algorithm: "fnv1a64-lf-source".into(),
            fixtures: BTreeMap::from([("native".into(), fixture)]),
        }
    }

    #[test]
    fn growth_cannot_hide_behind_an_unrelated_saving() {
        let before = report();
        let mut after: Report =
            serde_json::from_value(serde_json::to_value(&before).unwrap()).unwrap();
        let fixture = after.fixtures.get_mut("native").unwrap();
        fixture.module_bytes -= 1;
        fixture
            .emission
            .runtime_helpers
            .push("UnusedManagedReader".into());
        fixture
            .emission
            .functions
            .push((999, "UnusedOffsetDiscovery".into()));
        fixture.emission.scratch_bytes += 1;
        *fixture.function_body_bytes.values_mut().next().unwrap() += 1;
        *fixture.sections.entry("11:data".into()).or_default() += 1;
        let changes = compare(&before, &after);
        for expected in [
            "UnusedManagedReader",
            "UnusedOffsetDiscovery",
            "scratch bytes",
            "11:data",
            "body 0 grew",
        ] {
            assert!(
                changes.iter().any(|change| change.contains(expected)),
                "{changes:?}"
            );
        }
    }

    #[test]
    fn source_changes_and_removed_fixtures_require_review() {
        let before = report();
        let mut after: Report =
            serde_json::from_value(serde_json::to_value(&before).unwrap()).unwrap();
        after.fixtures.get_mut("native").unwrap().source_fingerprint = "changed".into();
        assert!(
            compare(&before, &after)
                .iter()
                .any(|change| change.contains("source changed"))
        );
        after.fixtures.clear();
        assert!(
            compare(&before, &after)
                .iter()
                .any(|change| change.contains("fixture was removed"))
        );
    }

    #[test]
    fn timing_noise_does_not_change_the_size_gate() {
        let before = report();
        let mut after: Report =
            serde_json::from_value(serde_json::to_value(&before).unwrap()).unwrap();
        after.fixtures.get_mut("native").unwrap().compile_micros += 1_000_000;
        assert!(compare(&before, &after).is_empty());
    }

    #[test]
    fn native_and_local_collections_retain_no_managed_runtime() {
        let fixtures = fixtures();
        for name in ["native", "local-map", "local-set"] {
            let (_, report) = measure(&fixtures[name]).unwrap();
            for (_, function) in &report.emission.functions {
                assert!(
                    !["Unity", "Il2Cpp", "Mono", "Managed"]
                        .iter()
                        .any(|backend| function.contains(backend)),
                    "{name} retained {function}"
                );
            }
        }
    }
}
