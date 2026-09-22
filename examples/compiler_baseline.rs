//! Repeatable local compile-time and generated-Wasm-size baseline.
//!
//! Run with `cargo run --release --example compiler_baseline -- 200`.
//! Use `--profile max-opt` instead of `--release` to benchmark the packaged compiler.
//! Append `--frontend` to measure parsing, library augmentation, and declaration
//! resolution without type checking or Wasm generation (including result drop).
//! Append `--stages` to time the public analysis, Wasm-lowering, and encoding phases.

use std::{hint::black_box, time::Instant};

const DEFAULT_ITERATIONS: usize = 200;
const WARMUP_ITERATIONS: usize = 20;

const FIXTURES: [(&str, &str); 5] = [
    ("minimal", "state \"game.exe\" {}"),
    ("minish_cap", include_str!("minish_cap.split")),
    ("lunistice", include_str!("lunistice.split")),
    ("cancellation", include_str!("cancellation.split")),
    ("settings", include_str!("lso_desktop_settings.split")),
];

fn main() {
    let mut arguments = std::env::args().skip(1);
    let iterations = arguments
        .next()
        .map(|value| {
            value
                .parse::<usize>()
                .expect("iteration count must be a positive integer")
        })
        .unwrap_or(DEFAULT_ITERATIONS);
    assert!(iterations > 0, "iteration count must be positive");
    let frontend = match arguments.next().as_deref() {
        None => false,
        Some("--frontend") => true,
        Some("--stages") => {
            assert!(arguments.next().is_none(), "too many baseline arguments");
            run_stages(iterations);
            return;
        }
        Some(other) => panic!("unknown baseline option: {other}"),
    };
    assert!(arguments.next().is_none(), "too many baseline arguments");

    println!("rust_debug_assertions={}", cfg!(debug_assertions));
    println!(
        "splitscript_profile={}",
        if frontend { "n/a" } else { "release" }
    );
    println!("pipeline={}", if frontend { "frontend" } else { "compile" });
    println!(
        "platform={}-{} logical_cpus={}",
        std::env::consts::OS,
        std::env::consts::ARCH,
        std::thread::available_parallelism().map_or(1, usize::from)
    );
    println!("warmup_iterations={WARMUP_ITERATIONS} measured_iterations={iterations}");
    println!("fixture\tsource_bytes\twasm_bytes\tmedian_us\tp95_us");

    for (name, source) in FIXTURES {
        for _ in 0..WARMUP_ITERATIONS {
            black_box(run(source, frontend));
        }

        let mut samples = Vec::with_capacity(iterations);
        let mut wasm_bytes = 0;
        for _ in 0..iterations {
            let start = Instant::now();
            let wasm = run(black_box(source), frontend);
            samples.push(start.elapsed().as_nanos());
            wasm_bytes = wasm.len();
            black_box(wasm);
        }
        samples.sort_unstable();
        let median = samples[samples.len() / 2];
        let p95 = samples[(samples.len() * 95).div_ceil(100) - 1];
        println!(
            "{name}\t{}\t{}\t{}\t{}",
            source.len(),
            if frontend {
                "-".to_owned()
            } else {
                wasm_bytes.to_string()
            },
            nanos_to_micros(median),
            nanos_to_micros(p95)
        );
    }
}

fn run(source: &str, frontend: bool) -> Vec<u8> {
    if frontend {
        black_box(splitscript::lower(
            splitscript::parse(source).expect("baseline fixture should parse"),
        ));
        return Vec::new();
    }
    splitscript::compile_with_options(
        source,
        splitscript::CompilerOptions {
            profile: splitscript::BuildProfile::Release,
            ..splitscript::CompilerOptions::default()
        },
    )
    .unwrap_or_else(|diagnostics| {
        panic!(
            "baseline fixture failed to compile: {}",
            diagnostics
                .iter()
                .map(ToString::to_string)
                .collect::<Vec<_>>()
                .join("; ")
        )
    })
}

fn nanos_to_micros(nanos: u128) -> String {
    format!("{:.1}", nanos as f64 / 1_000.0)
}

fn run_stages(iterations: usize) {
    use splitscript::{
        CompilationCancellation, CompilerContext,
        analyze_named_with_context_and_options_cancellable, encode_lowered_compilation_cancellable,
        lower_analyzed_compilation_cancellable,
    };

    println!("rust_debug_assertions={}", cfg!(debug_assertions));
    println!("splitscript_profile=release pipeline=stages");
    println!("warmup_iterations={WARMUP_ITERATIONS} measured_iterations={iterations}");
    println!("fixture\tphase\tmedian_us\tp95_us");
    for (name, source) in FIXTURES {
        let mut samples = [Vec::new(), Vec::new(), Vec::new()];
        let cancellation = CompilationCancellation::new();
        for iteration in 0..WARMUP_ITERATIONS + iterations {
            let start = Instant::now();
            let analyzed = analyze_named_with_context_and_options_cancellable(
                CompilerContext::default(),
                "<baseline>",
                black_box(source),
                splitscript::CompilerOptions {
                    profile: splitscript::BuildProfile::Release,
                    ..Default::default()
                },
                &cancellation,
            )
            .expect("baseline analysis succeeds");
            let analysis_time = start.elapsed().as_nanos();
            let start = Instant::now();
            let lowered = lower_analyzed_compilation_cancellable(analyzed, &cancellation)
                .expect("baseline lowering succeeds");
            let lowering_time = start.elapsed().as_nanos();
            let start = Instant::now();
            let artifact = encode_lowered_compilation_cancellable(lowered, &cancellation)
                .expect("baseline encoding succeeds");
            let encoding_time = start.elapsed().as_nanos();
            black_box(artifact);
            if iteration >= WARMUP_ITERATIONS {
                for (samples, duration) in
                    samples
                        .iter_mut()
                        .zip([analysis_time, lowering_time, encoding_time])
                {
                    samples.push(duration);
                }
            }
        }
        for (phase, samples) in ["analysis", "wasm_lowering", "encoding"]
            .into_iter()
            .zip(&mut samples)
        {
            samples.sort_unstable();
            println!(
                "{name}\t{phase}\t{}\t{}",
                nanos_to_micros(samples[samples.len() / 2]),
                nanos_to_micros(samples[(samples.len() * 95).div_ceil(100) - 1]),
            );
        }
    }
}
