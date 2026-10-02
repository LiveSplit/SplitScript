# Release Wasm size optimization

Release compilation runs a bounded size optimizer after emission. Debug and hot
reload bypass all optimizer scans. The objective is a smaller complete Wasm
file; execution-speed optimization remains the engine's job. Binaryen is not a
compiler dependency.

## Promoted pipeline

Two cleanup sweeps simplify integer instructions, fold adjacent integer
constants and propagate uniformly constant integer locals, remove unreachable
code and unused locals, simplify structured
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
and maintenance surface yet. The original straight-line propagation experiment stays off
master; a separately measured constant-local analysis is described below.
Redundant null rewriting is omitted because direct emitter fixes already
provide its savings in both profiles.

Integer constant folding stays because it saves 113 bytes on Minish Cap and
fits into the existing instruction scan. A second cleanup sweep saves another
282 / 41 bytes on the primary scripts when measured with temporary sinking
enabled; it exposes useful simplifications without introducing another analysis.
Pass savings interact and must not be added together as independent totals.

## Instruction cleanup follow-up — 2026-10-02

A fresh Binaryen 132 pass comparison on the promoted Release output identified
instruction simplification as the largest remaining individual opportunity on
the primary scripts. After subtracting Binaryen's no-pass re-encoding, it saved
371 / 315 bytes on Minish Cap / Lunistice; constant propagation saved 224 / 14,
local simplification/coalescing 179 / 157, and code folding 92 / 125.

The existing Release cleanup now also inverts integer comparisons followed by
`eqz`, removes redundant Boolean normalization where the value or its consumer
permits it, and simplifies lossless extend/wrap pairs. Floating-point comparison
inversion is excluded because NaNs invalidate the usual inequality identities.

Identical `if` arms share one body while retaining the condition's evaluation
and the original typed label as a block. Negated conditions with an `else` can
swap complete arms. Branch depths, block parameters and result values remain
intact. Arm comparisons examine at most 256 instructions, and overlapping inner
matches wait for the existing second cleanup sweep. A more elaborate nested
rewrite added no real-script savings and was discarded.

Redundant null assertions are removed from statically non-null constructor/cast
results and before GC reads/writes when intervening operands are only individual
nontrapping constants or reads. Checks stay before calls, writes and potentially
trapping operands, preserving the first trap and preceding observable effects.
The pipeline still has two bounded cleanup sweeps and the same strict body and
module size gates. Debug bypasses these changes.

| Real script | Before | After | Additional saving |
| --- | ---: | ---: | ---: |
| Minish Cap | 32,648 | 32,488 | 160 |
| Lunistice | 29,163 | 29,025 | 138 |
| Neon White | 4,768 | 4,692 | 76 |
| A Hat in Time | 48,536 | 48,449 | 87 |

Seven new runtime tests cover signed/unsigned boundaries, non-Boolean conditions,
truncation, unordered NaNs, typed branch parameters/results, nested selected
calls, preserved condition traps, GC constructor results, packed array reads,
and null checks before calls or division traps. All 501 library, 696 compiler,
20 binary and five example tests pass, as do Clippy, the browser-target check,
and 36 baseline-versus-optimized corpus runtime invocations.
All 176 maintained modules validate; all 211 runtime scenarios and the
Debug/Release profile check pass.
The unchanged Unity size gate also passes, including individual function/type
budgets and Lunistice base/DLC behavior; no baseline refresh was needed.

Final optimized-host seven-sample medians, with no concurrent build or runtime
suite, are:

| Real script | Passes disabled | Complete Release backend | Added backend time |
| --- | ---: | ---: | ---: |
| Minish Cap | 3.101 ms | 6.717 ms | 3.616 ms |
| Lunistice | 16.138 ms | 19.389 ms | 3.251 ms |
| Neon White | 0.501 ms | 1.033 ms | 0.532 ms |
| A Hat in Time | 4.387 ms | 7.360 ms | 2.973 ms |

These include lowering/emission but exclude parsing/type checking. The complete
pipeline remains close to the original promotion's 6.448 / 19.161 ms totals on
the primary scripts. Cross-run differences include measurement variation and
do not isolate the new rules' cost. Samples remain in `target/size-check/timing.json`.

Binaryen's remaining instruction-only savings fall to 194 / 195 bytes on the
primary scripts. Full `-Oz` on the new output reaches 29,638 / 26,344 bytes;
these are diagnostic comparisons, not an assertion that the remaining passes
compose additively. The next measured candidates are constant propagation on
Minish Cap and further instruction/local simplification on both primary scripts.

## Constant-local follow-up — 2026-10-02

Minish Cap retains packed constants in locals and repeatedly extracts their
halves with shifts and truncation. Binaryen's `--precompute-propagate` exposed
this opportunity. The new Release cleanup collects integer locals whose
explicit assignments all store the same literal, then replaces reads dominated
by a store. A conflicting or nonconstant write disqualifies the entire local.
Facts become available only after a store, including for parameters and
default-initialized locals. Scope exits and `else` discard facts established
inside that scope, so a branch that skips initialization cannot inherit them.
Outer facts survive calls and loops because all writes agree. Unsupported
exception/continuation control is excluded. This is a bounded linear scan,
without iterative control-flow analysis or heap/global assumptions.

Adjacent integer wrap/extend and zero tests now fold as well. Existing arithmetic,
local removal and branch cleanup consume the exposed constants. Large literals
may temporarily expand individual reads; the existing strict final body and
module size gates reject a result that does not shrink. Debug still bypasses
all optimization scans.

| Real script | Previous master | New Release | Additional saving |
| --- | ---: | ---: | ---: |
| Minish Cap | 32,488 | 32,086 | 402 |
| Lunistice | 29,025 | 28,933 | 92 |
| Neon White | 4,692 | 4,692 | 0 |
| A Hat in Time | 48,449 | 48,325 | 124 |

Constant-local propagation without the new unary folding saved 285 / 64 / 0 /
84 bytes respectively. Together they save 8.6% / 7.9% on the primary scripts
relative to pre-optimizer master. No fixture in the nine-script corpus grows;
automatic Unity and the two collection fixtures save 298 / 307 / 307 additional
bytes, and both small async fixtures are unchanged.

Six new runtime tests cover packed constants, both conditional arms, skipped
initialization through branches/tables, loop backedges, conflicting writes,
parameter/default values, repeated equal writes, signed/unsigned conversions,
division traps and rejection of larger expanded literals. The existing
Never-emission test now recognizes its marker as either an i32 or i64 constant,
because folding an extension legitimately changes the instruction width.

Validation passes: 507 library tests, all 696 compiler tests (including the
updated marker check), 20 binary tests and five example tests; Clippy; and the
browser compiler's wasm32 target check. All 176 maintained modules validate,
all 211 runtime scenarios and the Debug/Release profile check pass, and all 36
baseline-versus-optimized corpus runtime invocations pass.
The unchanged Unity size gate passes, including per-function/type budgets and
Lunistice base/DLC behavior; no baseline refresh was needed.

Optimized-host seven-sample medians, measured without a concurrent build or
runtime suite, are 2.929 -> 6.300 ms for Minish Cap and 17.086 -> 20.343 ms for
Lunistice (passes disabled -> complete Release backend). Neon White measures
0.523 -> 1.112 ms and A Hat in Time 4.360 -> 7.682 ms. These include
lowering/emission but exclude parsing/type checking; cross-run variation means
they do not isolate the new analysis's cost. The complete primary-script
backend remains in the same range as the preceding pipeline. The large
synthetic timings were noisy, so they are not used to infer pass overhead.

After this change Binaryen's standalone constant-propagation pass offers no net
saving relative to its no-pass roundtrip on the primary scripts. Instruction
simplification still saves 194 / 193 bytes, local simplification/coalescing
161 / 155, and code folding 92 / 125. Full `-Oz` reaches 29,653 / 26,350 bytes,
leaving a 2,433 / 2,583-byte gap. These pass results are diagnostic and do not
compose additively; Binaryen's own final output can change with input shape.

## Original promotion compiler cost

Optimized-host seven-sample medians for the promoted pipeline:

| Real script | Passes disabled | Promoted Release backend | Added backend time |
| --- | ---: | ---: | ---: |
| Minish Cap | 2.978 ms | 6.448 ms | 3.470 ms |
| Lunistice | 16.505 ms | 19.161 ms | 2.656 ms |
| Neon White | 0.505 ms | 1.005 ms | 0.500 ms |
| A Hat in Time | 4.556 ms | 7.469 ms | 2.913 ms |

These measurements include lowering and emission but exclude parsing/type
checking. Previous full-experiment totals were 7.591 / 20.856 ms on the primary
scripts. The selected pipeline is faster in these measurements while retaining
almost all size savings; cross-run differences are not isolated per-pass costs.
All nine fixtures and individual samples are in `target/size-check/timing.json`.
The optimized-host corpus check also validates identical sizes to the native
development build. Debug does not pay these optimizer costs.

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
