# Release Wasm size optimization

Release compilation runs a bounded size optimizer after emission. Debug and hot
reload bypass all optimizer scans. The objective is a smaller complete Wasm
file; execution-speed optimization remains the engine's job. Binaryen is not a
compiler dependency.

## Promoted pipeline

Two cleanup sweeps simplify integer instructions, fold adjacent integer
constants, remove unreachable code and unused locals, simplify structured
control flow, and share identical return sequences. The second sweep handles
patterns exposed by the first. Compilation does not iterate to a fixed point,
and it stops early when a sweep does not shrink the module.

Function sharing then merges bodies that differ only in integer constants.
Each original function remains a small wrapper passing its constants to a
shared helper. Existing indices, signatures, exports and references remain
intact. Reports retain the original functions and add the shared helpers.

Every cleanup body and complete module must shrink. Function-sharing costs
include wrappers, helper bodies, new signatures/declarations, and section
encoding overhead. The implementation preserves integer overflow and traps,
does not fold floating-point operations, and retains effects of discarded
values. Control-flow changes remap branch targets. Shared return sequences
exclude bodies with non-defaultable locals; function sharing excludes tail
calls and exception/continuation constructs. The tests cover those boundaries,
GC/reference values, multi-value returns, large indices, and observable effects.

## Selection from the experimental branch

The promotion was measured against `master` at `1b65a40` and the full pipeline
at `3ecf397` on `experiment/wasm-size-optimizations`. The experiment and its
Binaryen comparisons remain on that branch. Commit `3f3c9be` records a
reproducible comparison with individual passes disabled.

| Real script | Previous master | Promoted Release | Saved | Reduction |
| --- | ---: | ---: | ---: | ---: |
| Minish Cap | 35,111 | 32,648 | 2,463 | 7.0% |
| Lunistice | 31,413 | 29,163 | 2,250 | 7.2% |
| Neon White | 5,073 | 4,768 | 305 | 6.0% |
| A Hat in Time | 50,205 | 48,536 | 1,669 | 3.3% |

The selected passes retain 98.2% / 98.4% of the full experimental savings on
Minish Cap / Lunistice. Inlining and temporary sinking stay experimental:
together, they save only another 46 / 36 bytes compared with this pipeline.
Temporary sinking alone contributes 36 / 17 bytes in the two-sweep configuration.
The remaining size benefit does not justify promoting that additional analysis
and maintenance surface yet. The disabled propagation experiment also stays off
master. Redundant null rewriting is omitted because direct emitter fixes already
provide its savings in both profiles.

Integer constant folding stays because it saves 113 bytes on Minish Cap and
fits into the existing instruction scan. A second cleanup sweep saves another
282 / 41 bytes on the primary scripts when measured with temporary sinking
enabled; it exposes useful simplifications without introducing another analysis.
Pass savings interact and must not be added together as independent totals.

## Validation and reproduction

The promoted pipeline has focused instruction/control/function-sharing tests
and a source-level behavioral test covering evaluation order, GC values,
fallible results and suspension. The real-script regression checks module
validation, size reduction, unchanged original function indices and report
metadata, and identical output with or without requesting a report.

The opt-in nine-fixture corpus records complete module bytes and section payload
sizes, validates baseline and optimized modules, and checks Debug equivalence.
DWARF `.debug_info` variable entries already have nondeterministic ordering in
unoptimized builds, so that comparison excludes only this section; executable
code, names, source maps and line tables must match. A smaller deterministic
fixture also checks the entire Debug binary byte for byte. No Debug emitter
changes are part of this promotion.

Validation passes: 494 library, 696 compiler, 20 binary and four example tests;
Clippy; formatting/diff checks; and the browser compiler's wasm32 target check.
All 176 maintained modules validate, all 211 runtime scenarios and the
Debug/Release profile check pass. The size-corpus runner additionally passes
36 behavioral invocations comparing unoptimized and promoted output.

```powershell
$env:CARGO_PROFILE_DEV_DEBUG = '0'
$env:CARGO_PROFILE_TEST_DEBUG = '0'
$env:CARGO_INCREMENTAL = '0'
cargo test --lib --test compiler --bin splitc --bin splitls --examples --offline
cargo clippy --all-targets --offline -- -D warnings
cargo check -p splitscript-vscode-wasm --target wasm32-unknown-unknown --offline
cargo test --lib write_size_corpus --offline -- --ignored --nocapture
node scripts/wasm-size-runtime.mjs
cargo test --profile max-opt --lib measure_optimization_overhead --offline -- --ignored --nocapture
```

Artifacts stay under `target/size-check`. Timing uses seven warmed, alternating
samples per mode, excludes parsing/type checking, and includes lowering and
emission. The maintained runtime runner compares baseline and optimized output
through the existing behavioral scenarios rather than checking bytes alone.
