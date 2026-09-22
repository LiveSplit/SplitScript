# Compiler baselines

SplitScript keeps a small, dependency-free baseline runner for end-to-end
compiler latency and generated WebAssembly size:

```console
cargo run --release --example compiler_baseline -- 200
```

The optional positional argument is the number of measured samples per
fixture. Each fixture receives 20 unmeasured warmup compilations first. Every
sample calls the public one-shot `splitscript::compile_with_options` API
in-process with the SplitScript release profile; the numbers exclude building
the Rust compiler executable, filesystem I/O, `wasm-tools` validation, and
host-runtime execution.

Append `--frontend` to isolate parsing, standard-library augmentation, and
declaration resolution through the public `parse`/`lower` APIs:

```console
cargo run --release --example compiler_baseline -- 200 --frontend
```

This mode includes disposal of the resulting lowered program but does not run
type checking or generate Wasm; its Wasm-size column is `-`. Warmups initialize
the standard-library graph and its caches, so these are repeated in-process
measurements, not process-startup measurements.

Append `--stages` to measure the public analysis, Wasm-lowering, and encoding
APIs separately:

```console
cargo run --profile max-opt --example compiler_baseline -- 200 --stages
```

Analysis includes parsing, inference, typed-HIR construction, and validation.
Each phase consumes the preceding phase's product; encoding therefore also
includes disposing of the checked program and Wasm IR. Disposal of the final
artifact is outside the encoding timer. These are fresh compilations after
standard-library initialization, with the same warmup and sample counts as
the end-to-end runner.

Timing values are diagnostic baselines, not test thresholds. OS scheduling,
CPU power state, Rust updates, and allocator changes can move them without a
compiler regression. Generated Wasm byte counts are deterministic, but should
also be reviewed rather than frozen into brittle assertions because valid
backend changes can alter them intentionally.

## 2026-09-22 library dependency selection

Ordinary compilations now inject a conservative dependency closure of the
standard-library source bodies. A graph cached with the library's rendered
tokens indexes every callable name and follows references transitively.
Receiver types are not known yet, so all candidates with the same name are
retained, including every case of an overload. Constants, implicit capability
implementations, formatting, provider attachment/validation, and generated
provider/managed helpers supply additional roots. Every user declaration is
still checked, including unused functions. The complete library bootstrap
remains the validation authority; signature-only documentation contexts still
fail strict compilation. Debug and Release use the same selection path.

This is the general implementation motivated by the experiment below. Minish
Cap now lowers **55 function bodies instead of 412**, including its four user
functions. The current explicit-profile Lunistice fixture lowers 256 bodies.
The maintained `compiler_baseline` example now includes both actual
autosplitters, alongside the existing smaller fixtures.

Measurements compare the compiler through `746243e` with dependency selection
on top of `fba3680` (the intervening commit changes documentation only). Both
use ordinary Rust release builds, identical frozen source files, CPU 0, 20
warmups and 50 measured samples, with both run orders. This paired baseline
was somewhat slower than the earlier probe's baseline; percentages below use
the paired measurements, not timings from another run.

| Actual autosplitter / measurement | Before median, first / reverse | After median, first / reverse |
| --- | ---: | ---: |
| Minish Cap compilation | 66.30 / 67.79 ms | **9.46 / 9.01 ms** |
| Lunistice compilation | 122.48 / 125.01 ms | **82.80 / 83.19 ms** |
| Minish Cap LSP edit → diagnostics | 48.58 / 50.69 ms | **4.99 / 4.78 ms** |

Minish Cap compilation improves 86–87%, Lunistice 32–33%, and Minish Cap
diagnostics about 90%. Compilation p95 falls from 91.51/75.42 to
11.58/10.74 ms for Minish Cap and from 137.32/134.91 to 87.17/92.97 ms for
Lunistice. Minish Cap diagnostics p95 falls from 52.30/54.61 to 6.54/6.12 ms.
The LSP measurement still alternates a trailing newline and measures complete
revision rebuilding, not incremental semantic-edit performance.

To distinguish warm compiler throughput from command-line startup, a separate
measurement starts a fresh `splitc` process for every sample, compiles Minish
Cap, and writes its release Wasm. With three initial process runs excluded
and 15 measured launches per pair, medians fall from 132.00/124.30 to
70.03/69.16 ms. This includes process startup, full library initialization,
source loading, and output writing; filesystem caches are warm. The remaining
initialization cost is not hidden by the warm 9 ms result.

| Release Wasm | Before | After |
| --- | ---: | ---: |
| Minish Cap | 35,334 B | 35,326 B |
| Lunistice, explicit IL2CPP profile | 32,121 B | 32,121 B |
| Lunistice, automatic Unity discovery | 142,234 B | 142,220 B |

The other six release fixtures retain their sizes. Removing unused library
types can change GC type numbering and remove redundant array subtype entries;
the Minish Cap type table is smaller, so this change does not claim byte-
identical output. All nine fixtures in both Debug and Release pass
`wasm-tools validate --features all`, with no size increases in either profile.

Validation: 694 compiler integration tests pass, including runtime tests for
providers, managed schemas, collections, closures, and generic effects. The
final library run passes 459 tests, with one manual benchmark ignored. New
tests compare selected and complete library compilation for Minish Cap,
Lunistice, cancellation, sets, and maps: diagnostics and user-function effects
agree, backend helper/scratch plans agree, and both profiles emit valid Wasm.
Additional checks preserve unused-user-function errors and token spans, and
assert that Minish Cap no longer retains most of the library. The existing
strict documentation-context test caught a distinction during development;
the final implementation preserves it without changing that test.

Logs and frozen runners are under ignored
`target/performance-review/2026-09-22`, principally
`dependencies-measurements.log`, `dependencies-cold-minish.log`,
`dependencies-counts.log`, `dependencies-compiler-tests.log`,
`dependencies-lib-tests-fixed.log`, and the two `dependencies-*-sizes.log`
files. Next latency work should target remaining Lunistice work and library
startup costs using these real-autosplitter measurements.

## 2026-09-22 Minish Cap regression diagnosis

Minish Cap is the primary latency target for the next architectural change.
Its 15,128-byte source is identical at `40d72a0` and `746243e` (SHA-256
`4fad141c14e762910b1cbb61142d7ee5ee206cfa6476c140acd2d5415adef328`). The earlier
paired measurements put warm ordinary-release compilation at about 23 ms
before the regression, 98 ms at `4720d69`, and 60 ms after the performance
fixes through `746243e`.

A fresh comparison of the saved binaries separates the remaining regression:

| Minish Cap phase | `40d72a0` median | `746243e` median |
| --- | ---: | ---: |
| Analysis | 15.82 ms | 42.54 ms |
| Wasm lowering | 2.85 ms | 10.22 ms |
| Encoding, including intermediate disposal | 3.78 ms | 9.78 ms |

The empty fixture still costs 41.36 ms for analysis and 10.00 ms for lowering
in the current compiler. Minish Cap's own code is therefore not the main
source of the increase. The compiler processes 412 function bodies, only
four of which belong to the autosplitter. The bundled catalog grew from
308,872 to 600,330 source bytes; its current 408 source-defined function
bodies contain 299,874 bytes of body text. Unity/Mono/IL2CPP families alone
account for 293 bodies and 224,382 bytes (75% of body text).

### Controlled unused-library experiment

An **unshipped diagnostic probe**, based on `746243e`, retains full library
bootstrap validation and catalog metadata, then omits Unity/Mono/IL2CPP
implementation bodies from subsequent compilations. Constants remain
injected. It permits those intentionally absent templates at the HIR and
signature-validation boundaries. The filtered source/tokens are cached just
like the complete library. This is a counterfactual for Minish Cap, not a
general-purpose dependency selector or a production optimization.

The same executable ran with the filter disabled/enabled, then in reverse
order, pinned to CPU 0, with 20 warmups and 50 measured samples per run.
Only the real Minish Cap fixture was timed. Rust used the ordinary release
profile; emitted Wasm used the SplitScript release profile.

| Measurement | All library bodies | Unused families omitted |
| --- | ---: | ---: |
| Compilation median, first pair | 59.75 ms | 19.58 ms |
| Compilation p95, first pair | 63.74 ms | 21.55 ms |
| Compilation median, reverse pair | 59.68 ms | 19.57 ms |
| Compilation p95, reverse pair | 64.67 ms | 20.72 ms |
| LSP edit → diagnostics median, first pair | 44.80 ms | 12.92 ms |
| LSP edit → diagnostics p95, first pair | 47.89 ms | 13.27 ms |
| LSP edit → diagnostics median, reverse pair | 42.68 ms | 12.92 ms |
| LSP edit → diagnostics p95, reverse pair | 48.19 ms | 14.08 ms |

The actual stdio LSP harness alternates a trailing newline, forcing a new
document revision and analysis; this measures full revision rebuilding, not
incremental semantic-edit performance. All responses were free of errors.
Warmups exclude the complete, once-per-graph library bootstrap, so the
experiment does not establish a cold-start improvement.

| Minish Cap probe phase | All library bodies | Unused families omitted |
| --- | ---: | ---: |
| Analysis median | 40.07 ms | 11.88 ms |
| Wasm lowering median | 9.53 ms | 2.84 ms |
| Encoding, including intermediate disposal, median | 9.12 ms | 4.71 ms |

Both probe outputs pass `wasm-tools validate --features all` and have the
same size, 35,300 bytes. The probe uses a shortened compiler revision stamp,
so this is not a size reduction from the production 35,334-byte module.
The modules are not byte-identical because removing library declarations
changes constructed-type numbering. Comparing the entire printed modules
after a consistent bijective type-index renaming and reordering the type
declarations produces an exact match; instructions, imports, exports, data,
and function ordering are otherwise unchanged. This is a structural output
check for the experiment, not a substitute for runtime coverage of a future
production implementation.

This accounts for roughly **40 ms, or 67%, of current Minish Cap compilation**
and recovers the remembered 20 ms range. Backend reachability already keeps
these unused implementations out of emitted Wasm, but runs after parsing,
type checking, HIR/effect analysis, and Wasm lowering have paid for them.
The next fix must avoid or reuse those earlier library stages. Preserve full
library validation, diagnostics for every user function, precise higher-order
effects, and implicit/provider dependencies. A hardcoded Unity exclusion is
not an acceptable implementation, and this investigation does not change
production compiler behavior.

Local probe sources and logs remain under ignored
`target/performance-review/2026-09-22`: `0758-profile` contains the disposable
snapshot updated to current sources, `minish-probe.log` and
`minish-lsp-probe.log` contain the paired results, `library-groups.log` contains
the catalog breakdown, and `minish-probe-type-renaming.txt` records the output
comparison. The stage comparisons are
`coverage-release-stages-minish-current.txt` and
`old-release-stages-minish-now.txt`.

## 2026-09-22 exhaustiveness checks for catch-all patterns

`missing_patterns` now returns immediately when a checked, unguarded binding
or wildcard already covers the entire input. Previously it built a pattern
matrix, checked inhabitedness, and expanded type constructors before the
recursive witness search recognized that nothing could be missing. For enums,
constructor expansion repeatedly copied the complete variant list. This path
also ran for ordinary function parameters and variable declarations.

Pattern checking, duplicate/unreachable-arm diagnostics, guarded-arm handling,
and analysis of refutable patterns remain in place. Debug, Release, and the
LSP share the shortcut. It also correctly covers uninhabited input types.

Compared with `8b842ea`, using ordinary Cargo release, frozen inputs, CPU
affinity, 20 warmups, and 50 measured samples, with serialized builds/runs:

| Compilation fixture | Before → shortcut | Reverse-order before → shortcut |
| --- | ---: | ---: |
| minimal | 54.72 → 55.03 ms | 56.69 → 57.01 ms |
| Minish Cap | 60.21 → 59.96 ms | 61.60 → 61.30 ms |
| cancellation | 54.20 → 52.27 ms | 54.98 → 55.07 ms |
| settings | 56.53 → 53.44 ms | 55.71 → 55.18 ms |

These small-fixture compile results are mixed and do not establish a general
compilation speedup. Actual stdio LSP diagnostics show a consistent 6–8%
improvement for the 500-helper fixture, with mixed smaller-fixture results:

| LSP fixture | Before → shortcut | Reverse-order before → shortcut |
| --- | ---: | ---: |
| small | 47.06 → 42.01 ms | 41.50 → 42.68 ms |
| Minish Cap | 47.60 → 41.89 ms | 41.36 → 40.89 ms |
| 500 helpers | 66.70 → 61.51 ms | 63.99 → 60.17 ms |

A separate scaling fixture contains one 128-variant enum and 64 ordinary
functions taking that enum as a parameter (4,717 source bytes). Full CLI
compilation improves from **187.20 to 152.16 ms median (19%)**, with p95 moving
from 211.47 to 184.94 ms. This comparison alternates binary order, uses four
warmups and 20 samples per binary, and includes process startup and I/O; it is
not directly comparable with the in-process table above. Both binaries emit
the same 597-byte Release module after normalizing the compiler revision.

All nine regular Release and nine Debug fixtures validate at unchanged sizes.
Release bytes match after revision normalization; Debug differences remain
limited to the revision and previously observed DWARF record ordering.
All 52 focused pattern, exhaustiveness, and guard tests pass, including async
execution, refutable binding diagnostics, and recursively uninhabited types.
The larger-enum output also validates, and formatting checks pass.
Artifacts are under ignored `target/performance-review/2026-09-22`, including
`coverage-release-*`, `coverage-wide-enum.log`, and `common/wide_enum.split`.
The packaged `max-opt` profile and extension packaging were not rebuilt.

The investigation also narrowed the remaining fixed type-checking cost:
minimal input spends about 12 ms checking function bodies, 1.1 ms generalizing
functions, and 2 ms publishing inference results. More intrusive diagnostic
timers found only four associated-type projections and about 0.17 ms in type
construction. Those measurements include instrumentation overhead and are
used to choose work, not to claim end-to-end gains. The suspected repeated
generic-call scan was therefore left unchanged.

## 2026-09-22 compact-key HIR sorting

Profiling `0758c7c` separates minimal-input HIR construction into visiting
(7.04 ms), collecting hash-map entries (1.11 ms), sorting (3.44 ms), and body
construction (0.38 ms). Each typed expression occupies 248 bytes in this native
build. Sorting compact cached keys avoids repeatedly moving these large
records during comparisons, while retaining the same stable ID ordering.
This changes one shared HIR path used by Debug, Release, and the LSP.

An isolated probe sorts the same 17,058 complete expression records with a
fixed shuffle. With five warmups and 25 samples, ordinary sorting takes
3.22 ms median; cached-key sorting takes 0.62 ms. This is a local diagnostic,
not the claimed end-to-end speedup. No profiling instrumentation is retained.

The saved `0758c7c` compiler and this change use ordinary Cargo release,
identical frozen sources, CPU affinity, 20 warmups, and 50 measured samples.
Builds and measurements run serially. End-to-end compilation medians:

| Fixture | Before → cached keys | Reverse-order before → cached keys |
| --- | ---: | ---: |
| minimal | 60.95 → 60.16 ms | 56.69 → 57.82 ms |
| Minish Cap | 66.74 → 67.76 ms | 65.32 → 61.13 ms |
| cancellation | 60.81 → 57.75 ms | 59.93 → 54.20 ms |
| settings | 61.55 → 58.87 ms | 61.76 → 55.23 ms |

Cancellation and settings improve in both orders; minimal and Minish Cap
remain mixed. Actual stdio LSP diagnostics improve in both orders, by about
3–9% across the three fixtures:

| Fixture | Before → cached keys | Reverse-order before → cached keys |
| --- | ---: | ---: |
| small | 48.92 → 45.35 ms | 45.73 → 44.25 ms |
| Minish Cap | 48.30 → 45.83 ms | 46.20 → 42.38 ms |
| 500 helpers | 70.63 → 64.10 ms | 67.98 → 63.10 ms |

All nine Release and nine Debug fixtures validate with unchanged sizes.
Release bytes match after normalizing the embedded compiler revision; Debug
differences are limited to that revision and the previously observed DWARF
record ordering. This is a compiler-latency improvement, not a Wasm-size pass.
Focused checks passed: both expression-index tests, both effect-summary
equivalence tests, the typed-HIR snapshot, and nested value-block compilation.
Measurements and the isolated probe are under ignored
`target/performance-review/2026-09-22` (`sort-release-*`, `hir-profile-*`).
The packaged `max-opt` profile and extension packaging were not rebuilt for
this small shared-path change.

## 2026-09-22 indexed HIR expression lookup

The follow-up replaces repeated binary searches in `TypedProgram::expression`
with a direct position table when expression IDs are nearly dense. Minimal
input has 17,058 expressions in an ID range of 17,090. Holes retain missing-ID
semantics. The table is bounded to twice the expression count; sparse generated
IDs use the original sorted lookup. Debug, Release, and LSP share this path.

Compared with the saved effect-summary-reuse compiler, using the same ordinary
Cargo-release profile, frozen fixtures, CPU affinity, 20 warmups, and 50
samples. No builds overlap measurements. Compilation medians:

| Fixture | Before → indexed | Reverse-order before → indexed |
| --- | ---: | ---: |
| minimal | 66.70 → 61.02 ms | 68.28 → 65.64 ms |
| Minish Cap | 73.00 → 65.88 ms | 72.55 → 66.83 ms |
| cancellation | 65.97 → 61.38 ms | 67.03 → 61.15 ms |
| settings | 65.75 → 62.22 ms | 66.15 → 61.75 ms |

The additional median improvement is about 4–10%. Tail latency was noisy in
this session, with some p95s above 100 ms; these are diagnostic comparisons,
not performance thresholds. Actual stdio LSP medians:

| Fixture | Before → indexed | Reverse-order before → indexed |
| --- | ---: | ---: |
| small | 55.13 → 50.57 ms | 54.35 → 50.08 ms |
| Minish Cap | 55.46 → 48.43 ms | 51.97 → 49.62 ms |
| 500 helpers | 76.52 → 75.60 ms | 77.77 → 71.68 ms |

All nine Release and nine Debug size fixtures validate and retain their sizes.
Release output is identical after normalizing the compiler revision. Debug
differences are restricted to that revision and DWARF record ordering; repeated
builds with the pre-change compiler also vary in `.debug_info` ordering.

A separate prototype removed deep copies when lowering value-block prefixes.
Its paired timings did not show a repeatable gain, so it was not retained.

Focused validation passed: library Clippy, lookup parity across IDs and holes,
sparse/empty lookup bounds, effect-summary equivalence, the typed-HIR snapshot,
and nested value-block compilation. The extension packaging/browser gate was
not rerun for this slice.

## 2026-09-22 effect-summary reuse

Follow-up to the reassessment below: effect inference now records the function
summaries read by each body and reevaluates that body only when an input changed.
The initial round still visits every body. Nested closures, higher-order calls,
and implicit display calls use the same tracked lookup. Updates are published
together after each round, preserving the previous simultaneous fixed-point
semantics and 64-round limit. Reuse is local to one analysis; no compilation IDs
or validation results are cached across programs. Debug, Release, and LSP share
the implementation.

Paired ordinary Cargo-release measurements compare the frozen `4720d69`
compiler with this change on `f241574` (whose compiler is identical to
`4720d69`). Same frozen fixtures, CPU affinity, 20 warmups, 50 samples, and
measurement procedure as the reassessment. Builds never overlap timing runs.
Documentation-only verification fixes were applied after these binaries
were saved; neither changes compilation semantics.

| Fixture | Before → after median | Reverse-order before → after median | Before → after p95, first pair |
| --- | ---: | ---: | ---: |
| minimal | 95.69 → 65.73 ms | 92.53 → 65.06 ms | 102.91 → 69.61 ms |
| Minish Cap | 105.37 → 70.50 ms | 97.78 → 69.89 ms | 113.31 → 74.27 ms |
| cancellation | 91.03 → 64.30 ms | 90.93 → 63.32 ms | 95.11 → 66.67 ms |
| settings | 92.70 → 64.87 ms | 91.93 → 63.91 ms | 95.04 → 68.13 ms |

This removes 28–33% of total compilation time in these runs. It does not restore
the September 12 baseline: minimal compilation remains about three times as
slow. Typed-HIR construction, type checking, and repeated library work remain
the next targets. A separate post-change stage run measures minimal analysis
at 49.05 ms, Wasm lowering at 10.57 ms, and encoding at 5.94 ms (medians).
This localizes the gain to analysis; stage runs are diagnostic and need not
sum to an end-to-end median.

Actual stdio LSP edit-to-diagnostics measurements, with identical source hashes
on both sides:

| Fixture | Before → after median | Reverse-order before → after median | Before → after p95, first pair |
| --- | ---: | ---: | ---: |
| small | 80.79 → 52.64 ms | 78.96 → 53.61 ms | 84.47 → 54.84 ms |
| Minish Cap | 80.79 → 52.10 ms | 80.05 → 54.95 ms | 87.22 → 55.88 ms |
| 500 helpers | 102.89 → 73.70 ms | 107.53 → 77.86 ms | 109.90 → 81.72 ms |

The regression tests compare full summaries (including returned symbolic
values) against the original full-reevaluation algorithm for the bundled
library, Minish Cap, Lunistice, recursive/higher-order calls, nested returned
closures, iterators, global mutation, and implicit display calls. The minimal
fixture requires fewer than half as many function-body evaluations; this is a
work-count assertion, not a timing threshold.

All nine Release size fixtures are unchanged: minimal 597 bytes, Minish Cap
35,334, cancellation 2,241, settings 7,987, debug-profile fixture 1,170, set
3,106, map 4,779, explicit Lunistice 32,121, automatic Lunistice 142,234.
Every output validates with `wasm-tools --features all`. Before/after bytes
match after replacing the one compiler Git-revision field in module metadata;
code, data, types, and all other content are identical.

The same nine fixtures also validate in Debug and retain identical sizes and
executable sections. Four have different ordering of type/variable records in
the DWARF `.debug_info` section; the other five match completely after the
revision substitution. Debug results are therefore not claimed to be wholly
byte-identical.

Native Rust verification builds now use the user-approved 3,072 MiB ceiling in
`scripts/run_limited.py`; its default remains 768 MiB, which is still used for
script compilation, measurements, and runtime verification. Cargo's ordinary
`release` profile is unchanged. After `cargo clean`, verification builds use
`CARGO_PROFILE_DEV_DEBUG=0` and `CARGO_PROFILE_TEST_DEBUG=0` to reduce disk use.
These omit Rust symbols without disabling assertions or SplitScript's Debug
profile; the packaged `max-opt` settings are unchanged.

The packaged `max-opt` follow-up measures compilation medians of 59.05 ms
(minimal), 62.29 ms (Minish Cap), 56.78 ms (cancellation), and 57.34 ms
(settings). LSP medians are 46.65, 46.79, and 66.19 ms for small, Minish Cap,
and 500 helpers respectively. These are post-change absolute measurements,
not a paired `max-opt` improvement claim.

Validation: Clippy passed; all 454 active library tests and 137 syntax/loader
tests passed, as did the 20 CLI/LSP unit tests. The integration run passed
693 tests; its sole failure was an outdated `inspect` wording assertion,
which was corrected and passed on a targeted rerun. A stale Unity documentation
assertion and a missing `debug` documentation link were also repaired. The
full extension packaging/browser stage was not rerun for this change.

## 2026-09-22 performance reassessment

Compared freshly built `40d72a0` (September 12) with `4720d69` (the committed
inspect workflow). These measurements supersede the older timings for current
prioritization. Compiler and editor latency have regressed substantially;
generated Release Wasm has become smaller on every compatible fixture tested.

Windows x86-64, Rust 1.98.1, Node 24.14.0. Both compiler revisions use Cargo's
ordinary `release` profile, unchanged; these are **not new `max-opt` or embedded
compiler measurements**. Each revision was extracted with `git archive` and
built in a separate target directory. Reusing a target directory initially
retained an incompatible old local dependency; that failed build was discarded.
`SPLITSCRIPT_GIT_REVISION` was explicitly set for each archive build. No timings
come from pre-existing executables or the instrumented profiling build.

All timing runs are serial, at ordinary priority, pinned to logical CPU 0
(affinity mask 1), with 20 warmups and 50 measured samples per fixture. Rust
builds ran under `scripts/run_limited.py` with a 1,536 MiB process-tree limit;
measurements and script compilation used 768 MiB. Builds did not overlap
measurements. Executable compilation and startup are outside the latency timers.

The same September 12 fixture bytes are used on both sides. The common
`compiler_baseline.rs` runner substitutes `minish_cap.split` for Lunistice and
is linked using `rustc -O` against each freshly built library. The other three
rows are unchanged. Lunistice's old Unity API is incompatible with the new
compiler, so it is measured separately for current output size, not silently
ported and presented as identical input.

End-to-end one-shot compilation, in milliseconds:

| Fixture | Source bytes | Old → current median | Reverse-order old → current | Old → current p95, first pair |
| --- | ---: | ---: | ---: | ---: |
| minimal | 19 | 19.13 → 91.64 | 19.02 → 94.28 | 20.85 → 97.28 |
| Minish Cap | 15,128 | 23.20 → 98.45 | 22.84 → 98.35 | 24.37 → 104.49 |
| cancellation | 507 | 17.48 → 90.93 | 17.40 → 91.34 | 18.83 → 93.14 |
| settings | 4,269 | 18.20 → 91.60 | 17.87 → 93.90 | 19.62 → 95.97 |

The public stage runner localizes most of the increase to analysis. Stage
medians come from separate runs and need not sum to the end-to-end median:

| Fixture | Analysis old → current | Wasm lowering old → current | Encoding old → current |
| --- | ---: | ---: | ---: |
| minimal | 14.34 → 79.77 ms | 2.67 → 11.06 ms | 1.72 → 6.66 ms |
| Minish Cap | 16.26 → 78.62 ms | 2.97 → 11.09 ms | 3.93 → 9.35 ms |
| cancellation | 13.57 → 75.24 ms | 2.59 → 10.55 ms | 1.60 → 5.55 ms |
| settings | 14.07 → 75.26 ms | 2.65 → 10.56 ms | 1.68 → 5.70 ms |

Frontend-only medians (parse, augmentation, resolution, and disposal) rose
from 2.27 to 8.62 ms for minimal, 3.08 to 9.42 ms for Minish Cap, 2.29 to
8.30 ms for cancellation, and 1.93 to 6.76 ms for settings.

A separate temporary instrumented build attributes the current analysis cost.
It uses five warmups and ten measured samples per fixture on the same pinned
CPU. These diagnostic timings are not mixed into the uninstrumented comparisons:

| Current analysis component | Minimal | Minish Cap | Current Lunistice |
| --- | ---: | ---: | ---: |
| type checking, total | 16.18 ms | 16.76 ms | 17.70 ms |
| type-checking bodies, included above | 13.24 ms | 14.03 ms | 14.75 ms |
| typed-HIR construction | 12.94 ms | 13.70 ms | 14.42 ms |
| validation, total | 40.41 ms | 42.03 ms | 44.73 ms |
| effect inference, included above | 36.38 ms | 37.75 ms | 39.94 ms |

Effect inference performs nine full-program rounds on all three fixtures.
Minimal input alone has 408 function bodies and 17,058 typed expressions;
Minish Cap has 412/17,634 and Lunistice 413/19,002. The bundled catalog source
grew from 308,872 to 600,330 bytes between the two commits. This makes repeated
library work a concrete target. The measurements do not isolate a single
culprit commit or establish that any runtime validation should be removed.
All tracing was confined to the ignored profiling snapshot.

The new `scripts/lsp_baseline.mjs` measures the actual stdio language server
with its normal allocator. It waits for versioned diagnostics after alternating
full-text edits and fails on error diagnostics, protocol errors, premature exit,
or timeout. It records SHA-256 hashes of its input sources. It does not measure
completion, hover, startup, invalid-source recovery, or browser-worker latency.

| Fixture | Old → current median | Reverse-order old → current | Old → current p95, first pair |
| --- | ---: | ---: | ---: |
| small, 171 bytes | 15.46 → 79.50 ms | 15.33 → 78.26 ms | 16.43 → 83.83 ms |
| Minish Cap, 15,128 bytes | 17.11 → 80.53 ms | 16.85 → 80.91 ms | 19.03 → 90.60 ms |
| 500 helpers, 29,990 bytes | 38.68 → 102.09 ms | 38.67 → 104.02 ms | 42.91 → 107.37 ms |

Run this benchmark against an already-built executable and an explicit frozen
fixture. For a comparison, use the same file, CPU affinity, and sample count,
then reverse the executable order. On Windows, wrap the run in the resource
guard; the following command alone does not set CPU affinity:

```console
python scripts/run_limited.py --memory-mib 768 --seconds 240 --log target/lsp-baseline.log -- node scripts/lsp_baseline.mjs target/max-opt/splitls.exe examples/minish_cap.split 50
```

Raw generated Release modules, without Binaryen:

| Identical fixture | Old bytes | Current bytes | Change |
| --- | ---: | ---: | ---: |
| minimal | 1,007 | 597 | −410 |
| Minish Cap | 45,636 | 35,334 | −10,302 |
| cancellation | 2,701 | 2,241 | −460 |
| settings | 8,772 | 7,987 | −785 |
| debug-profile fixture, compiled as Release | 1,590 | 1,170 | −420 |
| set runtime | 3,569 | 3,106 | −463 |
| map runtime | 4,976 | 4,779 | −197 |

Section sizes include their framing. Minish Cap's code section falls from
39,715 to 29,843 bytes, accounting for 9,872 bytes of its total reduction.
Cancellation's code section stays at 1,584 bytes; its savings come from types
(543 → 150) and data (104 → 37). Map's code section actually grows from 3,796
to 4,015 bytes while its smaller types/data still reduce the complete module.
Do not infer code simplification from total size alone.

Today's Lunistice remains **32,121 bytes** with an explicit IL2CPP profile and
**142,234 bytes** with automatic Unity selection, matching the accepted Unity
closeout sizes. All 16 freshly emitted modules passed `wasm-tools validate
--features all`. This audit does not remove recent runtime validation features
to recover historical size figures.

Binaryen 132 reference results on current artifacts:

| Fixture | Raw bytes | `-O4` bytes | `-Oz` bytes |
| --- | ---: | ---: | ---: |
| minimal | 597 | 460 | 459 |
| Minish Cap | 35,334 | 28,850 | 29,288 |
| Lunistice, explicit profile | 32,121 | 27,210 | 26,604 |
| Lunistice, automatic Unity | 142,234 | 106,739 | 104,123 |

Both optimizer modes used `--closed-world` and explicitly enabled GC,
reference types, multivalue, bulk memory, sign extension, nontrapping float
conversion, and mutable globals. A preliminary `--all-features` run emitted
an encoding the installed `wasm-tools 1.201.0` could not validate, so those
results were discarded; the table uses the narrower feature set. All eight
listed optimized modules validate. Lunistice's raw, O4, and Oz modules each
pass the maintained base-game and DLC runtime scenarios (six runs). Recorded
host traces are identical across raw/O4/Oz for each scenario. This does not
claim runtime coverage of automatic Unity attachment or general equivalence
of every Binaryen transformation. Binaryen remains an offline comparison tool.

Raw logs, source snapshots, fixture hashes, section breakdowns, and saved
uninstrumented executables are under ignored
`target/performance-review/2026-09-22`. The LSP harness is retained in the
repository so the protocol measurement can be repeated after future changes.
The harness passed `node --check`, four complete server runs (both comparison
orders), and a missing-executable failure check. No Rust implementation changed,
so this reassessment did not rerun the full compiler conformance suite.

## Unity migration gate

Run the gate after each Unity implementation step:

```console
cargo xtask unity-baseline
```

It builds the runner with Cargo's `max-opt` profile, compiles 17 fixtures with
SplitScript's Release profile, validates the modules, and runs the Lunistice
base and DLC synthetic host scenarios. `cargo xtask conformance` and
`cargo xtask check` include this gate. The automatic Unity Lunistice variant
is measured but not executed here: the current host fixture does not provide
the mapped player identity needed for automatic attachment. Live game checks
remain separate from these synthetic scenarios.

`tests/baselines/unity.json` holds the rolling reviewed measurements;
`tests/baselines/unity-initial.json` preserves the initial migration baseline.
Each invocation
writes the current report, Wasm modules, and Lunistice runtime JSON to
`target/unity-baseline/`. Reports record compiler/Rust/Node versions, compiler
build mode, LF-normalized source fingerprints, total and per-section bytes,
defined-function/type counts, individual function body sizes, retained
runtime helpers, scratch bytes, static data bounds, and initial memory pages.
Section sizes include framing and sum to the module size minus its eight-byte
header. Function body sizes exclude their LEB length prefixes. The standard
`splitscript` custom section is counted; no name or DWARF sections or external
Wasm optimization are added for measurement. The source fingerprint is
FNV-1a 64-bit for reproducibility, not a cryptographic identity.

The compiler sidecar comes from the same plan that emits the Release artifact.
The runner verifies its function count against the binary, and compiler tests
verify that requesting it leaves Release bytes unchanged. Names alone do not
prove pruning: the gate also examines actual section sizes and memory demand.
An increase in any tracked section, function body, memory demand, function/type count, or a
new helper/function requires review even if an unrelated saving reduces the
total size. Source changes, added/removed fixtures, and changed toolchain/build
mode also require review. A single compilation duration is recorded for
diagnosis, including first-use initialization and reporting; it is not a
stable performance threshold. Use the warmed runner above for latency work.

For an intentional change, preserve the previous and initial JSON before
updating the reviewed baseline. Compare against both snapshots:

```console
cargo xtask unity-baseline --compare PATH_TO_PREVIOUS_REPORT.json
cargo xtask unity-baseline --compare tests/baselines/unity-initial.json
cargo xtask unity-baseline --record
```

`--record` runs validation and both Lunistice scenarios before writing the
baseline, but is an explicit overwrite, not evidence that growth is justified.
Review the report diff and record which used feature requires every increase
in the implementation log/PR before accepting it. Keep the initial report
unchanged throughout the migration; `--compare` does not replace the checked-in
baseline.

The initial measurements use SplitScript `81cd3e83950c`, Rust 1.98.1, and Node
24.14.0 on Windows x86-64. Explicit IL2CPP Lunistice is **27,677 bytes**;
automatic Unity selection is **51,354 bytes**. The paired unused-string
fixtures expose existing metadata-binding overhead without retaining a string
decoder. All initial fixtures reserve **22,528 scratch bytes**, including the
1,009-byte empty native artifact. These are measured shortcomings to remove,
not acceptable reasons to retain unused managed collection support later.

### Demand-driven scratch allocation

Scratch now follows retained helper roles and reachable read types, including
provider pointer widths and Genesis normalization padding. Unused declarations
and unreachable reads do not enlarge it. The initial and rolling reports show:

| Fixture | Wasm bytes, initial → current | Scratch bytes, initial → current | Initial memory pages |
| --- | ---: | ---: | ---: |
| Empty native | 1,009 → 1,005 | 22,528 → 0 | 2 → 1 |
| Local map | 5,250 → 5,246 | 22,528 → 0 | 2 → 1 |
| Local set | 3,571 → 3,567 | 22,528 → 0 | 2 → 1 |
| IL2CPP scalar | 14,881 → 14,881 | 22,528 → 8,192 | 2 → 2 |
| IL2CPP string | 15,599 → 15,599 | 22,528 → 14,336 | 2 → 2 |
| Mono scalar | 20,277 → 20,277 | 22,528 → 4,104 | 2 → 2 |
| Mono string | 20,996 → 20,985 | 22,528 → 10,248 | 2 → 2 |
| Lunistice | 27,677 → 27,677 | 22,528 → 14,336 | 2 → 2 |
| Lunistice auto | 51,354 → 51,354 | 22,528 → 14,336 | 2 → 2 |

No measured fixture grows. Native/map/set savings come from moving static data
to address zero when no scratch is needed; Mono string savings come from
shorter buffer-address immediates. Scratch byte savings do not reduce allocated
memory unless a page boundary is crossed. At this stage, unread schema fields
still incurred metadata-binding cost; the following slice addresses that.

### Reachable metadata binding

Generated provider preparation now resolves only demanded fields, classes, and
images. Snapshot fields and automatic shape evidence remain demand roots.
Unused lookup awaits and binding storage disappear before async lowering;
unreachable static reads no longer reserve transaction-cache globals.

| Fixture | Previous Wasm bytes | Current Wasm bytes | Change |
| --- | ---: | ---: | ---: |
| IL2CPP with unused string | 15,580 | 14,881 | -699 |
| Mono with unused string | 20,980 | 20,277 | -703 |
| Mono instances | 21,465 | 17,506 | -3,959 |
| Lunistice explicit | 27,677 | 27,677 | 0 |
| Lunistice automatic | 51,354 | 51,354 | 0 |

All other fixtures and scratch reservations are unchanged. The unused-string
artifacts are now byte-identical to their scalar counterparts. The Mono
instances fixture still resolves its class header, but no longer resolves the
unread `health` field; three field-discovery functions and their async poll
functions disappear. Both Lunistice edition scenarios pass.

The comparison against the previous and initial reports requires review only
for a synthetic future name changing from `expr6432::poll` to `expr6424::poll`
as lowered expression IDs shift. Its body remains 437 bytes; there is no new
runtime helper or module/section/body/memory growth. This rename was reviewed
when updating the rolling baseline; the strict name-retention gate is unchanged.

### Binary identities and unused standard-library types (2026-09-19)

The PE debug ID, ELF build ID, and Mach-O UUID readers add three fixtures.
Their first measurement exposed eager emission of every standard-library GC
type, including unused declarations and their constructed field types. Type
planning now follows reachable values, transitive fields, and the storage and
signatures required by emitted runtime code. Unused identity readers and types
are excluded; shared array layouts remain when other reachable reads need them.

| Fixture | Previous Wasm bytes | Current Wasm bytes | Scratch bytes |
| --- | ---: | ---: | ---: |
| Empty native | 1,005 | 615 | 0 |
| IL2CPP scalar | 14,881 | 14,576 | 8,192 |
| Mono scalar | 20,277 | 19,909 | 4,104 |
| Lunistice explicit | 27,677 | 27,204 | 14,336 |
| Lunistice automatic | 51,354 | 50,575 | 14,336 |
| PE debug identity | New | 7,135 | 32 |
| ELF build identity | New | 7,718 | 56 |
| Mach-O UUID | New | 4,308 | 32 |

All 14 existing fixtures shrink, with unchanged function counts, runtime-helper
sets, scratch reservations, and initial memory pages. No existing section or
function body grows. The three new measurements cover complete fixtures,
including result handling and printing, rather than isolated parser costs.

Comparisons against both the previous and initial reports flag the new fixtures
and changed numeric IDs in generated array/set/debug/future names. These are
reviewed identifier shifts, not new functions in existing scripts. The rolling
baseline records all 17 fixtures; the initial baseline remains unchanged.
All 101 runtime scenarios pass, including 196 identity cases and Lunistice
base/DLC. Automatic Lunistice remains size-only in this gate.

### Windows Mono profiles and discovery (2026-09-19)

Imported 26 measured PE profiles and eight family/width fallbacks. Exact build
selection is reachable only through automatic Unity detection. Explicit selectors
now return their concrete backend directly; the former shared runtime wrapper
retained opposite-backend GC types and would have added 39–45 unused bytes to
explicit IL2CPP scripts when Mono's descriptor expanded.

| Fixture | Previous bytes | Current bytes | Delta |
| --- | ---: | ---: | ---: |
| Native | 615 | 615 | 0 |
| IL2CPP scalar | 14,576 | 13,682 | -894 |
| IL2CPP string | 15,294 | 14,400 | -894 |
| Mono scalar | 19,909 | 23,966 | +4,057 |
| Mono string | 20,615 | 24,672 | +4,057 |
| Mono inherited static | 20,797 | 24,629 | +3,832 |
| Mono instances | 17,149 | 21,008 | +3,859 |
| Lunistice explicit | 27,204 | 23,141 | -4,063 |
| Lunistice automatic | 50,575 | 72,974 | +22,399 |
| Mono old explicit fixture | New | 24,241 | New |
| Mono automatic profile fixture | New | 60,488 | New |

Local map/set and the three identity-only fixtures are unchanged. Scratch and
initial memory pages are unchanged for every existing fixture. Scalar and
unused-string schemas remain byte-identical on both backends.

Explicit Mono growth comes from the eight fallback constructors, bounded x86/x64
discovery, old runtime selection, image-relative assembly names, and old static
storage. Automatic selection additionally retains the PE identity reader, GUID
comparisons, all 26 measured factories, and old-family detection. Small changes
in existing automatic function bodies follow shifted indices. Explicit Mono
contains no PDB reader or measured-build table, and explicit IL2CPP contains no
Mono module type. No managed collection reader or resolver is added by this step.

The reviewed rolling baseline covers 19 fixtures. All 106 runtime scenarios pass,
including 70 Mono profile cases and Lunistice base/DLC. Automatic Lunistice has
size coverage; synthetic Mono attachment has runtime coverage. No live-game
validation is claimed.

## 2026-09-12 source-path suggestion indexing

Source baseline: `d741ef4`. Intervening work changed process readers, pointer
width handling, Debug behavior, recovery, and examples. Both sides of this
comparison were rebuilt from that baseline with identical current fixtures;
older September 8 timings and byte counts are not used as the before case.

Qualified method calls performed a full public-catalog scan for possible
function-name typos, allocating every item's source path even for valid local
receivers. The shared graph now indexes public candidates by their exact
parent path, preserving catalog order and visibility. Candidate names are
borrowed during lookup; source functions and `Some`/`Ok`/`Err` remain dynamic
root-scope candidates. There is no script-profile branch or optimization pass.

Windows x86-64, Rust 1.98.1, Node 24.14.0. Native measurements use 20 warmups,
50 samples, ordinary priority, and affinity mask 1 (logical CPU 0), with
sequential runs. The unchanged `examples/compiler_baseline.rs` runner was
linked directly using `rustc -O` against each Cargo-built compiler library,
avoiding an extra build of unused benchmark dev dependencies. The `max-opt`
runner also uses fat LTO, one codegen unit, and panic abort. Compilation of
the Rust executables is outside every timer.

Ordinary-release medians (the second pair reverses the run order):

| Fixture | Before → after | Reverse-order before → after |
| --- | ---: | ---: |
| minimal | 26.65 → 19.26 ms | 26.95 → 18.93 ms |
| Lunistice | 29.27 → 21.43 ms | 29.67 → 20.87 ms |
| cancellation | 26.47 → 18.05 ms | 25.61 → 17.82 ms |
| settings | 29.03 → 18.25 ms | 29.13 → 17.53 ms |

`max-opt` medians:

| Fixture | Before → after | Reverse-order before → after |
| --- | ---: | ---: |
| minimal | 26.05 → 18.52 ms | 26.70 → 18.24 ms |
| Lunistice | 27.93 → 20.12 ms | 28.55 → 20.26 ms |
| cancellation | 25.20 → 16.86 ms | 25.03 → 16.63 ms |
| settings | 26.58 → 16.93 ms | 24.18 → 17.48 ms |

The ordinary-release stage runner attributes the gain to analysis:

| Fixture | Analysis before → after | Wasm lowering before → after | Encoding before → after |
| --- | ---: | ---: | ---: |
| minimal | 22.13 → 14.31 ms | 2.76 → 2.76 ms | 2.01 → 2.06 ms |
| Lunistice | 22.76 → 15.03 ms | 2.86 → 2.86 ms | 3.27 → 3.20 ms |
| cancellation | 21.29 → 12.96 ms | 2.58 → 2.64 ms | 1.67 → 1.58 ms |
| settings | 24.29 → 13.27 ms | 2.67 → 2.64 ms | 1.82 → 1.68 ms |

The actual packaged `max-opt` language server was measured over stdio with
the system allocator, waiting for versioned diagnostics after each full-text
edit. It uses the same sample counts and CPU affinity, inherited from the
Node harness, and excludes process startup. Both versions read identical
current sources:

| Fixture | Median before → after | Reverse-order before → after | First-run p95 before → after |
| --- | ---: | ---: | ---: |
| small | 22.46 → 14.65 ms | 23.36 → 15.10 ms | 25.03 → 17.17 ms |
| Lunistice | 22.66 → 16.08 ms | 23.74 → 15.63 ms | 24.81 → 18.28 ms |
| 500 functions | 45.87 → 38.73 ms | 48.29 → 38.49 ms | 69.85 → 43.49 ms |

The large-fixture p95 was noisy, but medians improve in both run orders.
This batch does not include a fresh embedded-compiler latency comparison;
the embedded compiler and browser-host behavior were covered by verification.

Nine release script outputs are byte-identical before and after: Lunistice
31,816 bytes (including the current local example edits), Minish Cap 45,636,
settings 8,772, cancellation 2,701, managed instances 15,414, Mono managed
instances 22,824, debug-profile fixture 1,590, set runtime 3,569, and map
runtime 4,976. No generated-script size reduction is claimed. Saved binaries,
temporary profiling evidence, and raw logs are in ignored
`target/performance-review`, under `suggestions`, `expression-probe`, and
`projections` names. All production timing probes were removed.

The packaged native CLI grows from 6,394,368 to 6,397,952 bytes, and the
language server from 4,416,512 to 4,420,096 bytes (3.5 KiB each). These are
compiler executable sizes, separate from the unchanged generated scripts.

Full `cargo xtask check` passed: formatting, strict Clippy, 513 generated
documentation pages, 104 syntax tests, 28 loader tests, 432 library tests
(one ignored), 629 compiler integration tests, 18 CLI tests, the language-server
unit test, 32 editor tests, browser-host and worker tests, Wasm validation,
and 96 runtime scenarios across 68 unique verification modules.

## 2026-09-08 capability inheritance indexing

Source baseline: `27549bf`. Capability implication queries now consult the
immutable standard-library graph's transitive index instead of allocating a
worklist and visited set on every query. Production code has no timing probes.
The new `--stages` benchmark uses the public cancellable compilation phases.

Windows x86-64, Rust 1.98.1; 20 warmups and 50 measured samples per fixture.
Unrestricted runs varied substantially with scheduling, so the controlled
comparison pinned every process to logical CPU 0 (affinity mask 1), with
ordinary priority and sequential runs. These numbers should not be compared
directly with earlier unpinned baselines. First-run medians, followed by a
second comparison in reverse order:

| Rust profile | Fixture | Before → after | Reverse-order before → after |
| --- | --- | ---: | ---: |
| release | minimal | 28.37 → 27.20 ms | 26.42 → 25.26 ms |
| release | Lunistice | 31.43 → 30.42 ms | 29.39 → 28.64 ms |
| release | cancellation | 26.95 → 25.90 ms | 24.54 → 24.23 ms |
| release | settings | 27.69 → 26.64 ms | 25.52 → 24.75 ms |
| max-opt | minimal | 26.64 → 24.97 ms | 24.84 → 24.40 ms |
| max-opt | cancellation | 24.81 → 22.92 ms | 24.29 → 23.06 ms |
| max-opt | settings | 25.44 → 23.93 ms | 24.29 → 23.08 ms |

The Lunistice fixture was independently edited between the two `max-opt`
builds (4,567 to 4,674 source bytes), so that pair is excluded. The ordinary
release comparison embeds identical fixture bytes. The decreasing absolute
times across run orders also show why the small percentages should not be
treated as precise guarantees.

The ordinary release stage comparison with the same affinity measured:

| Fixture | Analysis before → after | Wasm lowering before → after | Encoding before → after |
| --- | ---: | ---: | ---: |
| minimal | 22.00 → 20.79 ms | 2.67 → 2.63 ms | 1.97 → 1.79 ms |
| Lunistice | 24.03 → 22.83 ms | 2.77 → 2.79 ms | 2.87 → 3.01 ms |
| cancellation | 21.47 → 20.10 ms | 2.58 → 2.59 ms | 1.57 → 1.52 ms |
| settings | 21.76 → 20.52 ms | 2.64 → 2.60 ms | 1.61 → 1.53 ms |

All nine release output comparisons are byte-identical: Lunistice 30,565,
Minish Cap 45,113, managed instances 15,413, Mono managed instances 22,770,
cancellation 2,700, settings 8,771, debug-profile fixture 1,589, set runtime
3,568, and map runtime 4,975 bytes. Both compilers read the same current
fixture files for these comparisons. No script-size saving is claimed.
Raw logs and saved executables are under ignored `target/performance-review`
with `capability` in their names.

Packaged `max-opt` compiler artifacts before → after: `splitc`
6,296,576 → 6,302,720 bytes, `splitls` 4,318,720 → 4,325,376 bytes,
and embedded compiler Wasm 6,338,996 → 6,345,311 bytes. The small artifact
increase is separate from the unchanged generated script sizes.

The actual packaged services were measured with Node 24.14.0, the same
affinity mask, 20 warmups, 50 samples, and sequential runs in both orders.
The LSP harness measures `didChange` through versioned diagnostics over stdio
with the ordinary system allocator. Node and its child server share the
affinity. Embedded measurements call the public compile ABI and exclude
instantiation (about 14 ms), using `--single-threaded
--no-wasm-async-compilation`. Both versions read the same current sources.

| Service | Fixture | Median before → after | Reverse-order before → after |
| --- | --- | ---: | ---: |
| LSP | small | 21.54 → 20.71 ms | 21.83 → 20.75 ms |
| LSP | Lunistice | 22.31 → 21.37 ms | 22.77 → 21.72 ms |
| LSP | 500 functions | 46.14 → 44.07 ms | 46.14 → 44.68 ms |
| embedded compiler | minimal | 21.49 → 21.22 ms | 22.03 → 20.66 ms |
| embedded compiler | Lunistice | 25.22 → 24.89 ms | 25.44 → 24.53 ms |

Embedded p95 remained noisy and did not improve consistently; the table
supports a modest median benefit, not a guaranteed tail-latency reduction.

Full `cargo xtask check` passed: formatting, strict Clippy, generated
documentation, 102 syntax tests, 28 loader tests, 420 library tests (one
ignored), 621 compiler integration tests, 17 CLI tests, the language-server
unit test, 19 editor tests, browser/embedded workers, Wasm validation, and
95 runtime scenarios across 67 unique verification modules.

## 2026-09-04 baseline

- Measured source: `70998d4` plus the standard-library caching/indexing change
  described below
- Rust: `rustc 1.98.1 (48a229cea 2026-09-01)`, LLVM 22.1.8
- Cargo: 1.98.1
- Platform: Windows x86-64, 32 logical CPUs
- Rust harness profile: `release`
- SplitScript profile: `release`
- Warmups: 20 per fixture
- Samples: 200 per fixture

| Fixture | Source bytes | Wasm bytes | Median | p95 |
| --- | ---: | ---: | ---: | ---: |
| minimal | 19 | 1,009 | 45.68 ms | 48.87 ms |
| Lunistice | 4,554 | 34,830 | 54.44 ms | 56.55 ms |
| cancellation | 507 | 2,817 | 46.68 ms | 49.16 ms |
| settings | 3,956 | 8,119 | 49.58 ms | 52.12 ms |

The nearest recorded optimization-week comparison was reconstructed from
commit `c3a5b33` (2026-08-31) with the same toolchain, machine, runner settings,
and source byte counts. Its medians were 46.21 ms, 54.72 ms, 46.30 ms, and
48.69 ms respectively. Lunistice generated 36,694 bytes. The current medians
range from 1.2% faster to 1.8% slower, which is effectively parity at this
measurement scale, while the current Lunistice output is 1,864 bytes (5.1%)
smaller.

The largest isolated step first appears in commit `c0bbf93` ("Add read-only
whole-file APIs"). Two alternating, sequential 20-sample runs of its parent
`44beaae` and `c0bbf93` placed the parent around 47.5 ms for minimal and 56.9 ms
for Lunistice, and the child around 50.5 ms and 59.3 ms respectively. The
generated bytes were identical at that boundary, so the regression was compiler
execution overhead from the file-API implementation rather than emitted unused
helpers. Caching the compiler-owned rendered standard-library body source and
indexing hidden source functions by name recovered the remaining regression
without changing emitted bytes.

The roughly 46 ms fixture-independent cost remains an optimization opportunity.
A release-stage probe measured approximately 24.9 ms in parsing the augmented
standard-library source and only 0.067 ms in exact name resolution. Avoiding the
remaining parse and type-check work requires a semantic cache or reachability
design that preserves standard-library body validation. This fixed cost predates
the 2026-08-31 checkpoint; do not compare it directly to the July result below as
evidence of a recent regression because the compiler and bundled standard
library changed substantially in between.

## 2026-09-04 first performance-plan implementation batch

Compared `fd4db26` with the first implementation batch in
[`PERFORMANCE_PLAN.md`](../PERFORMANCE_PLAN.md), using the same toolchain and
machine as the preceding September baseline. The changes cache immutable
backend validation, index declarations and Wasm bodies, group specialization
work by function owner, reuse completion receiver facts, borrow semantic
snapshots for highlights, group adjacent generated locals, and share ordinary
Wasm function signatures.

Fresh release-module inspection:

| Fixture | Before bytes | After bytes | Reduction |
| --- | ---: | ---: | ---: |
| Lunistice | 34,830 | 34,526 | 304 (0.87%) |
| Minish Cap | 49,307 | 48,773 | 534 (1.08%) |
| Set runtime | 3,187 | 3,152 | 35 (1.10%) |

Lunistice now has 62 ordinary function-type entries for 62 distinct signatures,
down from 78 entries. Minish Cap has 54 for 54, down from 86. GC recursive groups
and nominal layout identities are not merged. Adjacent script, async, and start
function locals are grouped without reordering local indices; a few manually
declared runtime-local groups remain uncombined. These are encoding savings,
not set-operation pruning or general optimization passes. Release modules still
contain their compiler identity metadata and omit debug sections.

All three new modules pass `wasm-tools validate --features all`. Separate-process
recompilations of Lunistice and Minish Cap produce byte-identical output.

The receiver cache holds up to eight owned fact sets (including failed probes)
per document revision, rather than retaining temporary compiler databases.
Repeated requests for a cached receiver avoid semantic probing; the first
request after an edit still uses the existing inference/recovery path. Fixed
backend validation is cached per process, so fresh CLI processes still validate
the contracts once. Neither change removes standard-library parsing/checking.

Sequential before/after runs used saved release harness executables, with no
repository build or test running during measurement. Each compiler row has 20
warmups and 200 measured samples; each editor row has 20 warmups where the
harness specifies them and 30 measured samples.

| Compiler fixture | Before median | After median | Before p95 | After p95 |
| --- | ---: | ---: | ---: | ---: |
| minimal | 48.42 ms | 48.35 ms | 50.98 ms | 50.51 ms |
| Lunistice | 57.29 ms | 57.81 ms | 59.37 ms | 60.15 ms |
| cancellation | 49.33 ms | 49.37 ms | 50.56 ms | 51.00 ms |
| settings | 51.37 ms | 50.76 ms | 53.68 ms | 52.18 ms |

These one-shot medians move by less than 1.2% in either direction, so the batch
does not establish an overall compiler latency improvement. The dominant
standard-library parse/check floor remains. The minimal/cancellation/settings
release modules shrink from 1,009/2,817/8,119 to 1,006/2,795/8,044 bytes.

| Editor fixture / query | Before median | After median | Before p95 | After p95 |
| --- | ---: | ---: | ---: | ---: |
| small / warm multi-query sequence | 51.251 ms | 0.055 ms | 56.794 ms | 0.057 ms |
| Lunistice / warm multi-query sequence | 97.012 ms | 0.092 ms | 100.196 ms | 0.095 ms |
| generated 500 helpers / warm multi-query sequence | 129.132 ms | 11.716 ms | 133.433 ms | 12.575 ms |
| small / edit to member completion | 98.420 ms | 97.131 ms | 101.448 ms | 101.137 ms |
| Lunistice / edit to member completion | 153.894 ms | 151.415 ms | 159.880 ms | 156.459 ms |
| generated 500 helpers / edit to member completion | 199.070 ms | 197.856 ms | 203.224 ms | 203.676 ms |

The warm sequence is diagnostics, root completion, member completion, hover,
and semantic tokens at unchanged source/positions. Its improvement is mostly
eliminating repeated receiver-probe databases. It does not represent a speedup
for every keystroke: edit-to-member-completion still pays the first semantic
analysis/probe and remains approximately unchanged. The generated large warm
sequence still has other source/query work to address. Each fixture retains
352 additional bytes after the warm sequence, while its recorded retained-state
peak is unchanged. New regression tests count successful and failed probes and
verify reuse across caret positions and invalidation after a source edit.

The full `cargo xtask check` passed for this batch: 403 library tests, 602 compiler
integration tests, syntax/stdlib-loader and CLI tests, editor and browser-worker
checks, generated-module validation, and the maintained host-runtime fixtures.

## 2026-09-04 set-operation pruning

Compared a saved release compiler built at `dcfb1ec` with the set-operation
implementation, using the same toolchain and machine. Every row compiles
identical source before and after with the SplitScript release profile.

| Fixture | Before bytes | After bytes | Reduction |
| --- | ---: | ---: | ---: |
| Set new/length-only probe | 1,943 | 1,534 | 409 (21.0%) |
| Expanded set runtime, including numeric insert-only use | 3,808 | 3,597 | 211 (5.54%) |
| Original set runtime from `dcfb1ec`, all operations used | 3,152 | 3,152 | 0 |
| Lunistice | 34,526 | 34,526 | 0 |
| Minish Cap | 48,773 | 48,773 | 0 |

The focused probe is the source in section 4 of
[`PERFORMANCE_PLAN.md`](../PERFORMANCE_PLAN.md). Its four unused operation bodies
account for 387 removed bytes; body length prefixes save another six bytes,
function declarations four, and ordinary function signatures twelve. The GC
layouts and 160-byte compiler metadata section remain unchanged.

The maintained `tests/set_runtime.split` now additionally inserts a numeric
value twice without explicitly calling contains. Its harness verifies the
results are `true,false`, exercising the internal insertion dependency. That
fixture expansion is why its absolute size differs from the original runtime
fixture; the 211-byte saving compares the expanded source against itself.

The original runtime fixture uses all operations, so it has nothing to prune.
Lunistice and Minish Cap do not use sets. This change removes unnecessary
declarations/body generation for sparse set use; compiler or LSP latency was
not remeasured for this batch, and no overall latency improvement is claimed.

Both profiles pass the expanded Node set-runtime harness. All measured release
modules pass Wasm validation. Regression tests cover operation selection for
numeric, string, and structural array elements, iteration, generic instances,
unreachable functions, and debug-only operations erased in release.

The full `cargo xtask check` passed, including 403 library tests, 605 compiler
integration tests, documentation validation, editor/browser workers and web host,
generated-module validation, and the maintained host-runtime fixtures. The
verification log is under ignored `target/performance-review/set-pruning-check.log`.

## 2026-09-04 direct member receiver selection

Compared a saved release tooling harness from `ae3e404` with direct receiver
selection, using the same fixtures and runner (`tooling_baseline 500 30`):
20 warmups where specified by the harness and 30 measured samples per row.
Measurements ran sequentially without concurrent repository builds or tests.
An intermediate run identified the remaining snapshot-prefix case; the after
column below is the final implementation including that case.

| Edit to member completion | Before median | After median | Before p95 | After p95 |
| --- | ---: | ---: | ---: | ---: |
| small | 169.25 ms | 83.63 ms | 184.47 ms | 88.51 ms |
| Lunistice | 253.78 ms | 93.74 ms | 263.73 ms | 107.62 ms |
| generated 500 helpers | 381.03 ms | 146.93 ms | 404.47 ms | 167.98 ms |

Median reductions are 50.6%, 63.1%, and 61.4%. Absolute timings are slower than
the earlier September runs even for unchanged code, so compare this paired
measurement rather than attributing differences from earlier sessions to this
change. The measured sources and compiler/toolchain profiles are unchanged.

These fixtures request completion inside valid field paths (`point.x` and
`current.<field>`). The compiler now selects the receiver's typed path prefix
from the existing semantic snapshot, avoiding a repaired database and another
semantic pass. Completed calls and indexed expressions use their own result
types; completion inside a method name uses its receiver type. Regression
tests check both positive and negative candidates and count probes.

Per-request peak allocation deltas for edit-to-member completion fall from
6,991,226 to 3,996,238 bytes (small), 7,467,861 to 1,901,710 (Lunistice), and
22,047,501 to 10,691,337 (generated large). This removes transient probe work;
the existing bounded receiver cache is still used.

This does not eliminate semantic analysis after each edit. Recovered or
unavailable receiver facts still use the repair path, with successful and
failed probes cached per revision. Constructor-field prefixes without known
substitutions and root-effect completion remain separate opportunities. Warm
cached queries were already fast; this batch targets the first request on a
new revision rather than claiming a further general warm-query improvement.

Logs are under ignored `target/performance-review/direct-before-tooling.txt`
and `direct-final-tooling.txt`.

All 49 targeted completion tests and the full `cargo xtask check` passed:
406 library tests, 605 compiler integration tests, documentation validation,
editor/browser workers and web host, Wasm validation, and runtime fixtures.
The full-check log is `target/performance-review/direct-completion-check.log`.

## 2026-09-04 root completion availability caching

The focused root-completion workload is reproducible with:

```console
cargo run --release --example tooling_baseline -- 500 30 --root-effects
```

It measures 500 pure helper functions plus state-dependent functions, completing
`hel` in `onDetach`. The valid case completes inside `helper0(0)`, the partial
case contains an unknown `hel` expression that repair can fix, and the failed
case adds another unknown expression that repair cannot fix. Each case measures
both appended-comment edits and repeated requests at the unchanged identifier.

Compared `e5c1a8a` plus this benchmark mode against the implementation using
saved/rebuilt release harnesses, the same toolchain and source bytes, 20 warmups,
and 30 measured samples. Runs were sequential without repository builds or
tests in parallel.

| Repeated root completion | Before median | After median | Before p95 | After p95 |
| --- | ---: | ---: | ---: | ---: |
| Valid source | 5.37 ms | 5.35 ms | 5.88 ms | 5.67 ms |
| Partial identifier, successful repair | 138.13 ms | 5.40 ms | 147.03 ms | 5.84 ms |
| Failed repair | 217.87 ms | 4.77 ms | 232.52 ms | 5.07 ms |

Successful and failed repeated repairs improve by 96.1% and 97.8% respectively.
The valid case already reused semantic analysis and remains approximately
unchanged: constructing/filtering hundreds of candidates still costs time.

| First root completion after edit | Before median | After median | Before p95 | After p95 |
| --- | ---: | ---: | ---: | ---: |
| Valid source | 78.77 ms | 81.14 ms | 147.52 ms | 238.63 ms |
| Partial identifier | 352.61 ms | 355.47 ms | 374.09 ms | 372.83 ms |
| Failed repair | 436.46 ms | 436.74 ms | 457.07 ms | 481.82 ms |

No first-request improvement is established. Edit medians are close, while
tail latency, especially in the valid case, varies substantially. The cache
invalidates on every edit; repeated standard-library work and strict/recovered
analysis are still the major cold-query targets.

The cache stores source-function availability IDs, not complete effect analyses
or repaired compiler databases. Retained heap after one request rises by 120
bytes (valid), 216 (successful repair), and 96 (failed repair). Repeated-query
peak allocation deltas drop from 361,675 to 293,491 bytes, 16,652,453 to 293,491,
and 19,002,751 to 131,072 respectively. Initial-query peak usage is unchanged.

Separately, counter tests verify that top-level completion, fully available
lifecycle contexts, and sources without standalone functions skip availability
analysis. These cases are not timed by the focused table above. Other tests
verify transitive restrictions, per-context filtering, caret reuse, source-edit
invalidation, distinct repair keys, and eviction from the four-entry cache.

Logs are under ignored `target/performance-review/root-effects-before.txt` and
`root-effects-after.txt`.

All 53 targeted completion tests and the full `cargo xtask check` passed:
410 library tests, 605 compiler integration tests, documentation validation,
editor/browser workers and web host, Wasm validation, and runtime fixtures.
The full-check log is `target/performance-review/root-effects-check.log`.

## 2026-09-04 lazy release debug names

Before: `9ec7b63`. After: the lazy debug-name construction change. Both
compiler runners used the Rust release profile, 20 warmups, and 200 measured
iterations on the same fixtures. The runs were sequential, without concurrent
builds or tests.

| Fixture | Before median | After median | Before p95 | After p95 | Release Wasm bytes (both) |
| --- | ---: | ---: | ---: | ---: | ---: |
| minimal | 86.26 ms | 84.83 ms | 93.38 ms | 96.59 ms | 1,006 |
| Lunistice | 100.37 ms | 100.76 ms | 110.11 ms | 111.46 ms | 34,526 |
| cancellation | 86.25 ms | 87.61 ms | 95.26 ms | 97.74 ms | 2,795 |
| settings | 89.91 ms | 90.10 ms | 103.04 ms | 100.24 ms | 8,044 |

These timings do not establish an overall compile-time improvement. The change
removes debug-name formatting and allocation in release builds; a focused test
verifies that release never invokes the name builders while preserving function
indices and counts across 129 declarations. Standard-library parsing/checking
remains a much larger fixed cost.

Before/after release modules were byte-identical for Lunistice, Minish Cap,
`set_runtime`, `async_loop`, `postfix_calls`, `debug_profile`, and a callable
probe containing both a captured closure and a generic function-value adapter.
All seven outputs in both profiles passed Wasm validation. Debug modules were
also byte-identical except for `.debug_info` in `set_runtime`, `postfix_calls`,
and `debug_profile`. Repeating those builds with the unchanged before compiler
reproduced the `.debug_info` variation. All other sections, including code and
names, matched. This existing debug reproducibility issue is separate from the
release optimization.

The focused declaration test, all 24 profile codegen integration tests, and the
full `cargo xtask check` passed, including 411 compiler library tests, 605 compiler
integration tests, documentation, editor/browser checks, and Wasm/runtime fixtures.

## 2026-09-04 standard-library token reuse

Before: `13e805b`. After: cached library tokens and consuming extraction of
dynamic tokens during augmentation. The cache belongs to the standard-library
graph; it contains 13,626 tokens for 90,560 bytes of rendered source. Each
compilation still copies the cached tokens, reparses the combined program, and
checks its library functions. This is lexing reuse, not parsed-template reuse.

A focused release benchmark alternates the original whole-source lexer and
cached-token assembly within each pair, with 20 warmup pairs and 200 measured
pairs. It uses the minimal source, excludes graph initialization and disposal
of the resulting token vector, and includes the original lexer's owned-token
copy. Run it with:

```console
cargo test --release --lib benchmark_library_token_assembly -- --ignored --nocapture
```

| Token assembly | Median | p95 |
| --- | ---: | ---: |
| original whole-source lexing | 1,305.1 µs | 1,491.6 µs |
| cached library tokens | 197.8 µs | 227.2 µs |

This isolates a 1.107 ms (84.8%) median reduction in token assembly. The cache
retains tokens and their owned text once per graph in addition to the rendered
source; compiler contexts sharing that graph share the cache. User source and
provider-specific tokens are not retained there.

Full release compilation used the same four fixtures, 20 warmups, and 100
samples per row. Before/after executables ran sequentially without concurrent
builds or tests:

| Fixture | Before median | After median | Before p95 | After p95 | Release Wasm bytes (both) |
| --- | ---: | ---: | ---: | ---: | ---: |
| minimal | 84.59 ms | 83.22 ms | 92.84 ms | 92.59 ms | 1,006 |
| Lunistice | 99.19 ms | 97.56 ms | 107.64 ms | 108.85 ms | 34,526 |
| cancellation | 86.01 ms | 85.87 ms | 96.01 ms | 96.98 ms | 2,795 |
| settings | 89.91 ms | 88.94 ms | 98.45 ms | 99.77 ms | 8,044 |

These modest median changes are consistent with the isolated saving; overlapping
p95 values do not establish a tail-latency improvement. The new `--frontend`
runner also measured parse/augmentation/resolution, including disposal of the
lowered result. A repeat with 20 warmups and 100 samples gave:

| Fixture | Before median | After median | Before p95 | After p95 |
| --- | ---: | ---: | ---: | ---: |
| minimal | 25.10 ms | 24.58 ms | 26.45 ms | 25.57 ms |
| Lunistice | 51.79 ms | 51.79 ms | 56.78 ms | 60.90 ms |
| cancellation | 43.55 ms | 42.05 ms | 48.94 ms | 47.56 ms |
| settings | 45.56 ms | 44.34 ms | 54.82 ms | 51.91 ms |

An initial 200-sample frontend run measured 41.46 → 26.38 ms for minimal, but
the repeat did not reproduce that large difference. Do not attribute it to the
cache. Parsing and declaration resolution remain much larger costs than lexing.

All nine compared release modules were byte-identical: Lunistice, Minish Cap,
both managed-instance runtime fixtures, set runtime, async loop, postfix calls,
debug profile, and the captured-closure/generic-function-value probe. All 18
outputs passed Wasm validation. Debug differences were limited to `.debug_info`
in Lunistice, set runtime, and postfix calls; executable code, names, and all
other sections matched. The preceding batch documents the existing debug-info
reproducibility issue; repeating Lunistice with the unchanged before compiler
also reproduced the variation during this batch.

All nine targeted library-injection tests passed. The manual token benchmark
is ignored during ordinary correctness checks and has no timing assertion.
The full `cargo xtask check` passed, including 413 compiler library tests (one
manual benchmark ignored), 605 compiler integration tests, documentation,
editor/browser checks, and Wasm/runtime fixtures.

## 2026-09-06 shared strict/recovery lowering

Before: `0a0c6c8`, including the intervening text-setting and settings-expression
changes. After: sharing successful strict lowering with recovery in the editor
database. Rust `1.98.1`, Windows x86-64, 32 logical CPUs, release harness, 20
warmups, and 30 samples per row. Before and after runs were sequential without
concurrent builds or tests.

```console
cargo run --release --example tooling_baseline -- 500 30 --recovery
```

Each sample edits the source, requests diagnostics, then requests semantic
facts and hover. This includes strict checking and any recovery needed after
an error. The fixtures cover valid source, a missing-name type error, the same
error with 500 helpers, unavailable state access in `onDetach`, and incomplete
function syntax. All reuse the runner's normal offset-preserving edit pattern.
The standard-library graph is initialized before measurement.

| Fixture | Before median | After median | Before p95 | After p95 |
| --- | ---: | ---: | ---: | ---: |
| valid control | 50.20 ms | 49.51 ms | 55.79 ms | 56.90 ms |
| type error | 83.75 ms | 55.85 ms | 111.98 ms | 56.99 ms |
| type error, 500 helpers | 131.61 ms | 94.72 ms | 136.58 ms | 97.45 ms |
| validation error | 80.65 ms | 55.12 ms | 86.85 ms | 61.96 ms |
| syntax-error control | 49.0 µs | 49.9 µs | 52.7 µs | 51.4 µs |

Error-path medians improved by 25.5–36.9 ms (28–33%). The valid and syntax-error
controls are approximately unchanged. Successful strict lowering is now reused
even when a later stage rejects the program; recovery no longer repeats library
augmentation, parsing, and declaration resolution. Both query orders share the
same `Arc<LoweredProgram>`. Type inference itself can still run twice.

| Fixture | Before peak heap growth during edit query | After peak heap growth |
| --- | ---: | ---: |
| valid control | 3.81 MiB | 3.81 MiB |
| type error | 5.45 MiB | 3.56 MiB |
| type error, 500 helpers | 16.36 MiB | 11.36 MiB |
| validation error | 5.45 MiB | 3.56 MiB |
| syntax-error control | 15.3 KiB | 15.3 KiB |

The runner's retained-state rows reported unchanged values: 3,029,995 bytes
for valid, 566,374 for type error, 8,884,158 for large type error, 565,699 for
validation error, and 52,288 for syntax error. The measurements support lower
transient heap growth, not a measured steady-state retention reduction.

Seven release modules were byte-identical before/after: Lunistice (34,526
bytes), Minish Cap (48,773), desktop settings (8,790), managed-instance runtime
(17,369), managed-instance Mono runtime (26,608), debug profile (1,591), and set
runtime (3,597). The settings fixture is larger than in the 2026-09-04 baseline
because this comparison starts after the intervening settings commits.

All 47 targeted compiler-query tests passed, covering sharing in either query
order, independent recovered facts, diagnostic identity, warning-policy changes,
and source invalidation. Syntax recovery remains separate from strict lowering.
The full `cargo xtask check` passed, including 414 library tests (one manual
benchmark ignored), 608 compiler integration tests, documentation, editor/browser
checks, and Wasm/runtime fixtures.

## 2026-09-06 failed-check inference reuse

Before: `d3367af` (shared lowering). After: retaining partial inference output
from a failed strict check in the database's recovery cache. Rust `1.98.1`,
Windows x86-64, 32 logical CPUs, release harness, 20 warmups, and 30 samples
per row. Runs were sequential without concurrent builds or tests, using the
unchanged `tooling_baseline -- 500 30 --recovery` fixtures.

| Edit → diagnostics and hover | Before median | After median | Before p95 | After p95 |
| --- | ---: | ---: | ---: | ---: |
| valid control | 48.35 ms | 48.89 ms | 49.16 ms | 51.14 ms |
| type error | 53.49 ms | 39.94 ms | 56.15 ms | 41.14 ms |
| type error, 500 helpers | 92.93 ms | 70.08 ms | 95.73 ms | 73.32 ms |
| detached-state error | 53.92 ms | 39.64 ms | 56.28 ms | 40.83 ms |
| syntax-error control | 50.3 µs | 49.4 µs | 83.9 µs | 51.9 µs |

The three semantic-error fixtures improved by 24.6–26.5% in median latency.
Valid and syntax-error controls remain approximately unchanged. Heap readings
are essentially unchanged: the largest transient allocations still occur before
the avoided second inference pass. Peak growth in the error rows remains about
3.56 MiB (small) and 11.36 MiB (500 helpers); retained deltas change by at most
eight bytes. This batch establishes a latency improvement, not a memory saving.

The runner calls its detached-state row `recovery_validation_error`, but its
direct `current` reference is rejected during type inference. The name describes
the source-level failure, not an isolated post-type-validation stage. Separate
regression tests use a helper with latent process effects to exercise a genuine
post-type-validation failure and verify that its cached effect facts match a
fresh recovery check.

Tests count exactly one inference run for a failed strict check followed by
recovery. They compare diagnostics (including syntax warnings and ordering),
visible expression IDs, inferred values, resolved calls, and effects against
independently executed public APIs. Warning-policy changes retain the cached
facts; edits invalidate them. Failures before inference keep the existing
recovery path, and previously published recovery snapshots remain stable.

Recovery-first followed by strict checking can still run inference twice.
This change targets the normal diagnostics-first editor path. Successful
strict compilation does not create an additional recovery product.
A database used only for a failing strict check now retains partial facts until
its revision is invalidated; the usual diagnostics/recovery sequence already
retained those facts. The public one-shot strict API does not retain them.

The same seven release fixtures as the preceding batch remained byte-identical,
including both managed-runtime backends and desktop settings. All four targeted
inference-reuse tests and all 47 compiler-query integration tests passed.
The full `cargo xtask check` passed, including 418 library tests (one manual
benchmark ignored), 608 compiler integration tests, documentation, editor/browser
checks, and Wasm/runtime fixtures.

## 2026-09-07 shared checked products and query order

The new benchmark caught invalid state-field separators in the shared small
and large tooling fixtures: commas have been corrected to semicolons. Earlier
"valid" diagnostics/hover rows reached checked semantics through the editor's
length-preserving parser repair. Historical measurements remain records of
those workloads; do not compare their absolute timings directly with runs of
the corrected fixtures. Both executables below use the corrected sources and
the same new harness, whose strict checks assert the expected success/failure.

Before: `8c2f6d4`. After: shared successful checked/recovery products and reuse
in either query order. Rust `1.98.1`, Windows x86-64, 32 logical CPUs, release
harness, 20 warmups and 30 measured edits per row. Runs were sequential without
concurrent builds or tests:

```console
cargo run --release --example tooling_baseline -- 500 30 --check-order
```

Each row alternates source revisions and requests the indicated database
queries. These timings include cache invalidation and frontend work; they are
not cached-query hit timings. The post-type-validation fixture calls a helper
with latent process effects from `onDetach`.

| Fixture / query order | Before median | After median | Before p95 | After p95 |
| --- | ---: | ---: | ---: | ---: |
| valid / strict → recovery | 70.87 ms | 45.65 ms | 73.93 ms | 47.78 ms |
| valid / recovery → strict | 67.17 ms | 44.37 ms | 69.06 ms | 46.66 ms |
| valid / recovery only | 46.15 ms | 43.13 ms | 49.69 ms | 46.75 ms |
| valid, 500 helpers / strict → recovery | 104.51 ms | 67.25 ms | 122.84 ms | 72.42 ms |
| valid, 500 helpers / recovery → strict | 100.10 ms | 67.94 ms | 109.94 ms | 71.20 ms |
| valid, 500 helpers / recovery only | 68.90 ms | 68.66 ms | 80.20 ms | 71.67 ms |
| type error / strict → recovery | 37.97 ms | 36.13 ms | 38.93 ms | 40.54 ms |
| type error / recovery → strict | 51.75 ms | 36.39 ms | 52.71 ms | 37.57 ms |
| type error / recovery only | 37.52 ms | 36.68 ms | 38.55 ms | 38.16 ms |
| post-type-validation error / strict → recovery | 45.17 ms | 44.20 ms | 46.18 ms | 47.17 ms |
| post-type-validation error / recovery → strict | 67.18 ms | 43.95 ms | 69.88 ms | 45.36 ms |
| post-type-validation error / recovery only | 45.26 ms | 43.96 ms | 46.50 ms | 45.47 ms |
| syntax error / strict → recovery | 42.7 µs | 43.9 µs | 44.1 µs | 53.8 µs |
| syntax error / recovery → strict | 42.8 µs | 42.6 µs | 43.8 µs | 95.1 µs |
| syntax error / recovery only | 42.9 µs | 41.7 µs | 43.5 µs | 43.3 µs |

Valid query pairs improve 32.1–35.7% in median latency. Recovery-first type
and post-type-validation failures improve 29.7% and 34.6%, respectively.
Strict-first failures already reused inference before this batch. Recovery-only
and syntax-error controls show no comparable structural speedup; small timing
differences should not be overinterpreted. Ordinary successful diagnostics/hover
already uses strict checking, so the paired-query gains do not describe another
equivalent improvement to that LSP path.

Fresh-database retained heap, excluding initialized process-wide caches:

| Successful fixture / requests | Before | After |
| --- | ---: | ---: |
| small / both queries, either order | 5.26 MiB | 4.75 MiB |
| 500 helpers / both queries, either order | 17.09 MiB | 14.82 MiB |
| small / recovery only | 2.42 MiB | 4.75 MiB |
| 500 helpers / recovery only | 11.05 MiB | 14.82 MiB |

Sharing avoids a second recovery semantic model, but recovery alone now retains
the complete checked product, including compilation syntax and typed HIR. This
is an explicit retention tradeoff for subsequent strict-query reuse, not a
universal memory reduction. Fresh-database peak growth for small strict-first
pairs falls from 9.50 to 6.66 MiB; large strict-first pairs fall from 22.30 to
16.17 MiB. Error/syntax retained readings are essentially unchanged.
Recovery-first error queries also reduce peak growth by avoiding another check;
strict-first and recovery-only error peaks remain essentially unchanged.

All seven focused inference/sharing tests and 47 compiler-query integration
tests passed. Tests verify pointer sharing and agreement with standalone
recovery, exactly one inference in either order for compatible inputs,
warning-policy changes, and snapshot stability after source edits. The same
seven release fixtures as the previous batch remain byte-identical: Lunistice
(34,526 bytes), Minish Cap (48,773), desktop settings (8,790), managed instances
(17,369), Mono managed instances (26,608), debug-profile fixture in release
(1,591), and set runtime (3,597).

The full `cargo xtask check` passed: formatting, Clippy, documentation, 421
library tests (one manual benchmark ignored), 608 compiler integration tests,
editor/browser workers, the embedded Wasm compiler, Wasm validation, and
host-runtime fixtures. Smoke runs of the existing default and `--recovery`
tooling modes also passed with the corrected fixtures (five generated helpers,
20 warmups, one measured sample; used for assertions, not timing conclusions).

## 2026-09-07 intervening commits and parser token ownership

Reviewed `57f2564..ac45db0` before making the parser change. These commits add
maps, migrate conditional state to ordinary globals, extend collection/pattern
tooling, and fix iterator display specialization. The older source was rebuilt
from an ignored archive. The current baseline was rebuilt from the workspace;
an initial run using a colliding stale Cargo output was discarded after binary
hash verification. Only the verified binaries contribute measurements below.

Rust `1.98.1`, Windows x86-64, 32 logical CPUs; release harness and SplitScript
release output. Each compiler row has 20 warmups and 200 measured samples,
using `compiler_baseline -- 200` and `compiler_baseline -- 200 --frontend`.
All measurements run sequentially without concurrent builds or tests.

Unchanged-source comparison across the intervening commits:

| Fixture | Source bytes | `57f2564` compile median | `ac45db0` compile median | Old frontend median | New frontend median |
| --- | ---: | ---: | ---: | ---: | ---: |
| minimal | 19 | 46.96 ms | 50.82 ms | 24.42 ms | 26.78 ms |
| cancellation | 507 | 49.67 ms | 52.39 ms | 24.32 ms | 26.81 ms |
| settings | 4,269 | 50.59 ms | 54.93 ms | 25.84 ms | 28.71 ms |

Compile medians increased 5.5–8.6%, with 9.7–11.1% frontend growth. This
comparison establishes an aggregate slowdown, not which individual commit
caused it. Map methods add source bodies to the library parsed and checked for
every compilation, consistent with the measured fixed-cost growth. Parsed and
checked library reuse remain larger opportunities. Lunistice changed from
4,554 to 4,567 source bytes and migrated its state model, so its before/after
times are excluded from this unchanged-source comparison.

The seven unchanged release fixtures below have byte-identical non-custom
sections across these commits. Raw archived-build files are each 39 bytes
smaller because the archive lacks the compiler's Git identity, rather than
because of different executable content:

| Fixture | Non-custom bytes, both commits | Code section bytes, both commits |
| --- | ---: | ---: |
| Minish Cap | 48,613 | 42,862 |
| desktop settings | 8,630 | 5,862 |
| managed instances | 17,209 | 15,300 |
| Mono managed instances | 26,448 | 23,324 |
| debug-profile fixture in release | 1,431 | 701 |
| set runtime | 3,437 | 2,508 |
| cancellation | 2,635 | 1,675 |

Non-custom bytes include the module header and each non-custom section's
encoding. This sample shows no generated-code size regression from the
intervening commits; it does not cover every new map/iterator specialization.

The implementation in this batch removes the token cursor's unconditional
clone on every advance. Ordinary `bump()` and `previous()` borrow token storage;
contextual operator splitting retains its separate synthesized token. Generic
closer/fallible-suffix probes also stop cloning the current token. Tests verify
that token storage is reused and preserve splitting, operator reassembly,
cloned-cursor independence, failed probes, and EOF behavior.

Paired compiler measurements for the cursor change, using unchanged current
fixtures on both sides:

| Fixture / stage | Before median | After median | Before p95 | After p95 |
| --- | ---: | ---: | ---: | ---: |
| minimal / frontend | 26.78 ms | 23.93 ms | 27.93 ms | 24.95 ms |
| Lunistice / frontend | 33.53 ms | 29.20 ms | 46.63 ms | 30.39 ms |
| cancellation / frontend | 26.81 ms | 24.24 ms | 28.95 ms | 25.61 ms |
| settings / frontend | 28.71 ms | 25.79 ms | 29.98 ms | 27.36 ms |
| minimal / compile | 50.82 ms | 48.68 ms | 55.07 ms | 50.93 ms |
| Lunistice / compile | 61.31 ms | 57.28 ms | 64.23 ms | 59.36 ms |
| cancellation / compile | 52.39 ms | 48.76 ms | 56.24 ms | 50.63 ms |
| settings / compile | 54.93 ms | 51.16 ms | 57.63 ms | 57.93 ms |

Frontend medians improve 9.6–12.9%; total compile medians improve 4.2–6.9%.
This recovers much of the measured aggregate slowdown on unchanged fixtures,
without bypassing library validation or changing language behavior.

The tooling runner now supports cumulative stage measurements:

```console
cargo run --release --example tooling_baseline -- 500 30 --stages
```

Each stage row edits the source and requests `parse`, `lower`, or `check`.
Later rows include prerequisite stages and disposal of the preceding revision;
they are not isolated pass timers. Both sides below use the same extended
harness, 20 warmups, and 30 measured edits:

| Fixture / database query | Before median | After median | Before p95 | After p95 |
| --- | ---: | ---: | ---: | ---: |
| small / parse | 9.5 µs | 9.1 µs | 9.7 µs | 12.0 µs |
| small / lower | 27.67 ms | 24.68 ms | 30.17 ms | 25.68 ms |
| small / check | 51.48 ms | 48.38 ms | 53.17 ms | 52.26 ms |
| 500 helpers / parse | 2.48 ms | 2.17 ms | 2.66 ms | 2.42 ms |
| 500 helpers / lower | 42.60 ms | 37.09 ms | 44.56 ms | 39.86 ms |
| 500 helpers / check | 75.81 ms | 69.33 ms | 80.17 ms | 71.62 ms |

The lowering improvement is consistent with avoiding token-text clones during
augmented-library parsing. Retained and peak byte measurements are essentially
unchanged: the avoided short-lived copies do not determine the heap peak.

The same extended harness with `--recovery` measures edit → diagnostics → hover:

| Fixture | Before median | After median | Before p95 | After p95 |
| --- | ---: | ---: | ---: | ---: |
| valid | 51.62 ms | 48.19 ms | 52.84 ms | 49.60 ms |
| type error | 41.98 ms | 38.62 ms | 43.94 ms | 40.07 ms |
| type error, 500 helpers | 67.25 ms | 64.02 ms | 71.02 ms | 81.56 ms |
| detached-state error | 42.23 ms | 38.58 ms | 46.16 ms | 40.68 ms |
| syntax error | 49.7 µs | 49.5 µs | 51.0 µs | 89.8 µs |

Editor timing is sensitive to the harness build. Before adding the stage mode,
two paired runs of the original harness showed the opposite direction: valid
medians were 49.49/49.59 ms before versus 53.66/54.24 ms after; small type errors
were 40.85/40.21 versus 44.44/44.09 ms. Stage instrumentation and its matched
recovery pair show improvements, but this disagreement remains unexplained.
Do not treat this batch as proof of a stable end-to-end LSP latency improvement.
The repeatable structural change is eliminated token copying; the separate
compiler/frontend measurements and stage measurements establish its benefit
for those measured builds. Native `splitls` and embedded-worker measurements
remain necessary for a stronger editor claim.

The intervening-commit editor comparison is likewise only a diagnostic signal:
the original harness measured `57f2564` at 48.96 ms for valid edits, 40.16 ms for
type errors, and 62.74 ms with 500 helpers, versus `ac45db0` at roughly 49.5,
40–41, and 63.6–63.8 ms. Successful retained heap increased from 4,985,297 to
5,231,862 bytes across those commits. This parser change leaves those retained
bytes unchanged.

Compared with `ac45db0`, nine release fixtures remain byte-identical after this
change, with matching compiler identity: the seven above, current Lunistice
(34,500 bytes), and map runtime (5,016 bytes).

Validation: `cargo test -p splitscript-syntax` passed all 98 syntax tests.
The full `cargo xtask check` passed, including formatting, Clippy,
documentation, 418 compiler-library tests (one manual benchmark ignored),
617 compiler integration tests, editor/browser workers, the embedded Wasm
compiler, Wasm validation, and host-runtime fixtures including maps and
the migrated conditional-state examples. Both benchmark modes completed with
their fixture assertions enabled.

## 2026-09-07 shared augmented syntax

Before: `88378b2`. After: reference-counted immutable compilation syntax shared
by lowered/checked products, plus lazy cloning of user syntax when augmentation
is absent. Ordinary database checks no longer deep-copy the augmented program.
Library parsing, resolution, inference, and validation still run for each
revision; no syntax identities are reused across independently lowered sources.
The public one-shot compilation path also uses the database for analysis, so
it reaches this ownership boundary.

Rust `1.98.1`, Windows x86-64, 32 logical CPUs, release harness/profile, with
20 warmups per row. Tooling uses 500 generated helpers and 30 measured edits;
the initial compiler runs use 200 samples. Runs are sequential without
concurrent builds/tests. Saved before executables are retained under ignored
`target/performance-review`, along with all raw logs.

Fresh-database heap measurements exclude initialized process-wide caches:

| Fixture / requests | Before retained bytes | After retained bytes | Before peak growth | After peak growth |
| --- | ---: | ---: | ---: | ---: |
| small valid / strict + recovery, either order | 5,229,648 | 4,086,275 | 7,106,727 | 5,964,306 |
| 500 helpers, valid / strict + recovery, either order | 15,826,141 | 13,974,189 | 17,112,874 | 15,261,874 |
| small valid / diagnostics + hover | 5,231,862 | 4,088,489 | 7,106,727 | 5,964,306 |
| small type error / diagnostics + hover | 2,608,467 | 2,608,491 | 4,368,982 | 3,225,672 |
| 500 helpers, type error / diagnostics + hover | 11,585,771 | 11,585,795 | 14,329,869 | 12,477,980 |

Successful retained heap drops by 1,143,373 bytes (1.09 MiB, 21.9%) for the
small fixture and 1,851,952 bytes (1.77 MiB, 11.7%) with 500 helpers. Recovery-only
success has the same retained bytes as the successful query pairs because that
query already retains the complete checked product. Failed checks already
discarded the temporary syntax copy, so sharing reduces their peak rather than
retained heap. Their 24-byte retained increase comes from the ownership-layout
change. Syntax-error recovery likewise has a 24-byte retained increase, with
approximately 2.2 KiB less peak growth.

The nine release fixture modules are byte-identical before/after, including
compiler metadata: Minish Cap (48,773 bytes), desktop settings (8,790), managed
instances (17,369), Mono managed instances (26,608), the debug-profile fixture
in release (1,591), set runtime (3,597), cancellation (2,795), Lunistice (34,500),
and map runtime (5,016).

Timing was unstable during the initial runs. For example, the unchanged before
executable measured 83.44 ms for the small database check and 136.95 ms with
500 helpers, then 48.86 and 70.57 ms on a later run. Initial 200-sample compiler
medians were 87.10/103.90/90.06/97.80 ms before and 49.19/58.45/48.90/51.24 ms
after (minimal/Lunistice/cancellation/settings). The apparent large improvement
cannot be attributed to this change: the saved before executable recovered on
repetition too. Do not compare these initial timings with earlier sessions or
claim their ratios as speedups.

A closer sequential repeat uses the same 30-edit stage harness, 20 warmups:

| Fixture / cumulative database query | Before median | After median | Before p95 | After p95 |
| --- | ---: | ---: | ---: | ---: |
| small / parse | 8.9 µs | 9.1 µs | 9.4 µs | 9.4 µs |
| small / lower | 24.72 ms | 26.07 ms | 26.49 ms | 27.21 ms |
| small / check | 48.86 ms | 49.45 ms | 51.54 ms | 50.55 ms |
| 500 helpers / parse | 2.22 ms | 2.29 ms | 3.48 ms | 3.29 ms |
| 500 helpers / lower | 39.66 ms | 37.92 ms | 55.42 ms | 40.34 ms |
| 500 helpers / check | 70.57 ms | 69.42 ms | 74.48 ms | 72.98 ms |

The additional compiler repeat uses 50 samples after 20 warmups, as a drift
check rather than a replacement for the initial 200-sample run:

| Fixture | Before median | After median | Before p95 | After p95 |
| --- | ---: | ---: | ---: | ---: |
| minimal | 51.48 ms | 48.80 ms | 89.21 ms | 50.70 ms |
| Lunistice | 57.36 ms | 57.10 ms | 59.27 ms | 60.90 ms |
| cancellation | 48.54 ms | 48.19 ms | 53.30 ms | 49.96 ms |
| settings | 50.60 ms | 51.05 ms | 52.56 ms | 54.74 ms |

These repeats show approximate latency parity with mixed small changes. The
established improvement is retained/peak heap and elimination of deep copies,
not a reliable percentage reduction in total compiler or LSP latency.

Nine focused checking tests and all 47 compiler-query integration tests passed.
New assertions verify shared augmented syntax in both query orders, independent
revision ownership, and unchanged release code generation from an old checked
snapshot after an edit. Signature-only documentation contexts still fail strict
body validation when required library bodies are absent.

The full `cargo xtask check` passed: formatting, Clippy, documentation, 420
compiler-library tests (one manual benchmark ignored), 617 compiler integration
tests, editor/browser workers, the embedded Wasm compiler, Wasm validation,
and host-runtime fixtures.

## 2026-09-07 shared async table dispatch

Comparison against `3543444`, using saved release-built compiler executables
and unchanged fixtures. Both SplitScript profiles now emit `br_table` for async
bodies with 2–128 states. This changes direct instruction emission, without an
optimization pass or a separate debug implementation. Single-state bodies keep
the smaller linear encoding; machines above 128 states keep shallow dispatch
to bound structured nesting. Compiler RAM was not a target for this batch.

### Generated release modules

| Fixture | Before bytes | After bytes | Saved bytes |
| --- | ---: | ---: | ---: |
| Lunistice | 34,500 | 33,439 | 1,061 (3.1%) |
| managed instances | 17,369 | 16,867 | 502 (2.9%) |
| managed instances Mono | 26,608 | 25,454 | 1,154 (4.3%) |
| cancellation | 2,795 | 2,727 | 68 (2.4%) |
| Minish Cap | 48,773 | 48,773 | 0 |
| settings | 8,790 | 8,790 | 0 |
| debug-profile fixture, release output | 1,591 | 1,591 | 0 |
| set runtime | 3,597 | 3,597 | 0 |
| map runtime | 5,016 | 5,016 | 0 |

All size reductions are in the code section, including its encoded length.
The five unchanged modules are byte-identical. Lunistice's code section drops
from 30,931 to 29,870 bytes; the Mono fixture's drops from 23,324 to 22,170.
This does not establish a runtime speedup for the generated script.

### Compiler latency

Rust 1.98.1, Windows x86-64, 32 logical CPUs; release-built Rust harness and
SplitScript release profile. The normal in-process baseline uses 200 samples
after 20 warmups per fixture, with builds and tests stopped:

| Fixture | Before median | After median | Before p95 | After p95 |
| --- | ---: | ---: | ---: | ---: |
| minimal | 48.21 ms | 49.38 ms | 54.48 ms | 52.91 ms |
| Lunistice | 57.99 ms | 58.70 ms | 67.82 ms | 61.71 ms |
| cancellation | 49.94 ms | 49.57 ms | 62.80 ms | 51.10 ms |
| settings | 50.70 ms | 52.20 ms | 57.47 ms | 62.85 ms |

The affected fixtures move by -0.7% to +1.2% at the median, while unchanged
controls move by +2.4% to +3.0%. This shows approximate compilation-time parity,
not a demonstrated compiler speedup or a material dispatcher-related slowdown.

A separate debug-profile CLI check uses 50 samples after 10 warmups per fixture
and executable. It includes process startup, compiler initialization, and file
I/O, so it must not be compared directly with the in-process numbers above:

| Fixture | Before median | After median | Before p95 | After p95 |
| --- | ---: | ---: | ---: | ---: |
| Lunistice | 138.61 ms | 124.02 ms | 145.34 ms | 128.30 ms |
| cancellation | 123.83 ms | 112.49 ms | 132.98 ms | 118.20 ms |
| settings | 126.25 ms | 114.76 ms | 133.59 ms | 117.99 ms |

The unchanged settings control improves similarly, so these CLI numbers do not
attribute the improvement to dispatch. They show no added debug compilation
cost in this run. The deterministic output-size reduction is the reason to
keep this change; compile/editor latency remains the next measurement target.

New regression coverage validates table labels/default branches at small and
large state counts in both profiles, and executes retry, nested async calls,
loop continue/break, exhaustion, and completion in both profiles. Existing async
runtime coverage now exercises table dispatch in the default debug profile too.

The full `cargo xtask check` passed: formatting, Clippy, documentation, 98 syntax
tests, 420 compiler-library tests (one manual benchmark ignored), 619 compiler
integration tests, editor/browser workers, the embedded Wasm compiler, generated
Wasm validation, and host-runtime fixtures.

## 2026-09-07 Binaryen reference study

The [Binaryen review](../BINARYEN_OPTIMIZATION_REVIEW.md) records a separate
experiment against `72a6a15`, using Binaryen 132 solely as an offline reference.
It contains the full eight-fixture size matrix, section attribution, isolated
pass results, runtime checks, and reproduction commands.

Lunistice is 33,439 bytes from SplitScript, 29,418 after Binaryen's instruction
peephole pass and reader/writer normalization, 27,974 with O4, 26,960 with Oz,
and 22,678 with the combined O4/size/closed-world/convergence experiment. Minish
Cap is respectively 48,773, 43,821, 40,010, 39,535, and 28,075 bytes. Custom
sections remain 160 bytes in all outputs. These are opportunities identified
for future native compiler work, not size reductions already implemented in
SplitScript or a new external optimization stage in its build pipeline.

## 2026-09-07 async completion and frame-load emission

Comparison against `6a658a7`, implementing two direct-emission improvements from
the Binaryen study. Async block emission returns conservative fallthrough
information; completion tails and loop-back branches are omitted after
unconditional transfers, including nested branches. Poll functions also load
their non-null frame parameters without reasserting non-null. Nullable global
frames keep their checks. Both changes use the same code in debug and release.

### Release output attribution

| Fixture | Before | After fallthrough cleanup | After both changes | Total saved |
| --- | ---: | ---: | ---: | ---: |
| Lunistice | 33,439 | 31,951 | 30,784 | 2,655 (7.9%) |
| Minish Cap | 48,773 | 47,225 | 45,434 | 3,339 (6.8%) |
| managed instances | 16,867 | 16,141 | 15,536 | 1,331 (7.9%) |
| managed instances Mono | 25,454 | 23,720 | 22,870 | 2,584 (10.2%) |
| cancellation | 2,727 | 2,706 | 2,706 | 21 (0.8%) |
| settings | 8,790 | 8,790 | 8,790 | 0 |
| debug-profile fixture, release output | 1,591 | 1,591 | 1,591 | 0 |
| set runtime | 3,597 | 3,597 | 3,597 | 0 |
| map runtime | 5,016 | 5,016 | 5,016 | 0 |

All savings are in the code section, including section framing. The four
unchanged controls are byte-identical. Code sections for Lunistice, Minish Cap,
and Mono shrink from 29,870/42,862/22,170 to 27,215/39,523/19,586 bytes.
This captures part of the Binaryen rewrite/peephole opportunity; it does not
remove every redundant null assertion or all dead code in ordinary functions.

### Compiler latency

Rust 1.98.1 on Windows x86-64, 32 logical CPUs, release-built Rust harness,
SplitScript release profile, 200 measured samples after 20 warmups per fixture.
Saved before/after executables run sequentially without builds or tests.

| Fixture | Before median | After median | Before p95 | After p95 |
| --- | ---: | ---: | ---: | ---: |
| minimal | 48.76 ms | 53.25 ms | 52.27 ms | 56.64 ms |
| Lunistice | 59.84 ms | 64.65 ms | 78.24 ms | 113.31 ms |
| cancellation | 49.67 ms | 53.49 ms | 51.35 ms | 57.23 ms |
| settings | 51.81 ms | 55.99 ms | 54.24 ms | 58.85 ms |

Medians increase by about 8–9% even on the unchanged minimal/settings controls.
An earlier before-only run also drifted from 49 ms on minimal to about 93–103
ms on the remaining fixtures, making a single sequential run insufficient to
attribute small latency differences to the patch.

A reverse-order repeat uses 50 samples after 20 warmups, running the new
executable first and then the saved old one:

| Fixture | Before median | After median | Before p95 | After p95 |
| --- | ---: | ---: | ---: | ---: |
| minimal | 52.56 ms | 54.55 ms | 94.14 ms | 95.40 ms |
| Lunistice | 105.63 ms | 105.72 ms | 117.04 ms | 117.88 ms |
| cancellation | 87.49 ms | 94.88 ms | 96.51 ms | 116.10 ms |
| settings | 93.23 ms | 94.20 ms | 99.83 ms | 104.41 ms |

This repeat again changes the absolute timing regime substantially, including
for the old executable. Lunistice is approximately equal in that pair; the other
differences remain mixed. No compiler speedup or precise patch-attributable
slowdown is established. The deterministic reduction in emitted code is the
reason to retain these changes.

The initial focused run passed all 120 async tests. Added runtime cases exercise
nested if/match returns and optional fallbacks in both profiles. The dispatcher
boundary regression also checks that unconditional transfers are not followed
by dead completion instructions. A separate signature/emission regression covers
non-null frame parameters for named async functions, closures, and stored futures.

During validation a Windows PDB link failure coincided with low disk space.
The user-requested `cargo clean` removed 13.8 GiB; benchmark executables and
investigation artifacts were preserved outside the target directory during the
clean and then restored. The final validation rebuild uses the fresh target.

All nine updated release fixture modules validate. Thirteen before/after release
runtime scenarios also produce identical traces: Lunistice base/DLC, transient
metadata reads, mixed/inherited layouts, both Minish Cap backends, settings,
cancellation, managed runtimes, sets, and maps. As in the Binaryen study, these
trace checks use Node's single-threaded, synchronous-Wasm-compilation settings
to avoid the Windows host shutdown assertion; they do not measure Wasm runtime
performance. The focused poll-frame signature regression passes as well.

Final `cargo xtask check` passed: formatting, Clippy, documentation, 98 syntax
tests, 420 compiler-library tests (one manual benchmark ignored), 621 compiler
integration tests, editor/browser workers, the embedded Wasm compiler, generated
Wasm validation, and host-runtime fixtures. Repeated Lunistice and Minish Cap
release builds are byte-identical.

## 2026-09-07 unary GC read emission

Comparison against `d332a14`, removing explicit null assertions immediately
before unary GC reads at 157 emission sites. The read itself traps on null,
so removing this adjacent instruction preserves trap ordering and receiver
evaluation. Checks before other operands of indexed reads/writes, and checks
required by non-null type contracts, remain. This is shared debug/release
emission with fewer encoder calls, not an added optimization pass.

### Release output attribution

| Fixture | Before | After | Saved |
| --- | ---: | ---: | ---: |
| Lunistice | 30,784 | 30,565 | 219 |
| Minish Cap | 45,434 | 45,113 | 321 |
| managed instances | 15,536 | 15,413 | 123 |
| managed instances Mono | 22,870 | 22,770 | 100 |
| cancellation | 2,706 | 2,700 | 6 |
| settings | 8,790 | 8,771 | 19 |
| debug-profile fixture, release output | 1,591 | 1,589 | 2 |
| set runtime | 3,597 | 3,568 | 29 |
| map runtime | 5,016 | 4,975 | 41 |

All savings are in the code section; section framing does not change in these
fixtures. Lunistice, Minish Cap, and Mono code sections are now 26,996, 39,202,
and 19,486 bytes. The minimal fixture remains 1,006 bytes.

### Compiler latency

Same Windows/Rust environment as the preceding batch, release-built Rust
harness and SplitScript release profile. Saved before executable followed by
the new executable, 100 measured samples after 20 warmups per fixture, with no
concurrent build or test run.

| Fixture | Before median | After median | Before p95 | After p95 |
| --- | ---: | ---: | ---: | ---: |
| minimal | 51.94 ms | 51.18 ms | 54.09 ms | 55.88 ms |
| Lunistice | 62.63 ms | 57.94 ms | 71.22 ms | 64.82 ms |
| cancellation | 53.64 ms | 48.62 ms | 59.48 ms | 55.30 ms |
| settings | 57.39 ms | 54.37 ms | 61.44 ms | 59.46 ms |

This sample shows no latency regression. Recent sequential measurements on
this machine have substantial drift, and the change removes very little work
relative to total compilation. These numbers do not establish a repeatable
compiler speedup; the deterministic size reduction is the established benefit.

### Validation

All nine new release modules validate with `wasm-tools --features all`.
Complete before/after WAT listings match after deleting only the old
`ref.as_non_null` lines immediately followed by `struct.get*` or `array.len`.
The removal counts exactly match the size savings above. This checks that
receiver evaluation, calls, stores, indexed reads, and all other instructions
remain unchanged, including checks which guard later operand effects.

Thirteen release runtime traces match: five Lunistice scenarios, both Minish
Cap backends, settings, cancellation, both managed runtimes, sets, and maps.
Artifacts and timing logs are in `target/performance-review/unary-gc-*`.

Full `cargo xtask check` passed: formatting, Clippy, documentation, 98 syntax
tests, 420 compiler-library tests (one manual benchmark ignored), 621 compiler
integration tests, editor/browser workers, the embedded Wasm compiler, generated
Wasm validation, and host-runtime fixtures.

## 2026-09-08 distribution profile and field-read experiment

### Retained change: Rust `max-opt`

The user requested a separate distribution profile rather than modifying
ordinary release. `max-opt` inherits release and enables full LTO, one code
generation unit, `panic = "abort"`, and symbol stripping. Production VSIX builds
and the repository verification matrix select it for the embedded compiler;
the matrix also builds `splitc` and `splitls` with it and uses that `splitc` for
generated-module runtime verification. Development extension builds continue
to use release. The existing CI jobs inherit these choices through xtask and
the extension's production build script.

Benchmark the compiler with:

```console
cargo run --profile max-opt --example compiler_baseline -- 200
```

This is the Rust compiler build profile. The harness continues to request
SplitScript release output and now prints its actual debug-assertion setting
rather than incorrectly identifying every build as Rust release.

The saved native CLI built from `5f50ac1` decreases from 10,820,096 bytes with
release to 6,303,232 bytes with `max-opt` (41.7%). Both commands also built the
benchmark example. All nine release script fixtures are **byte-identical**
between those compiler executables, including metadata. Script sizes therefore
remain those of the preceding unary-GC batch, including 30,565-byte Lunistice
and 45,113-byte Minish Cap; this profile optimizes the compiler's own executable.

Native harness measurements use Rust 1.98.1 on Windows x86-64, 32 logical CPUs,
50 measured samples after 20 warmups, with no simultaneous build or test. These
two runs use the same unchanged compiler source and different Rust profiles:

| Fixture | Release median | `max-opt` median | Release p95 | `max-opt` p95 |
| --- | ---: | ---: | ---: | ---: |
| minimal | 49.07 ms | 48.60 ms | 50.24 ms | 50.23 ms |
| Lunistice | 56.44 ms | 55.96 ms | 58.17 ms | 59.11 ms |
| cancellation | 47.37 ms | 47.49 ms | 49.92 ms | 50.00 ms |
| settings | 50.27 ms | 49.90 ms | 52.08 ms | 51.26 ms |

These native timings are approximately equal, given measurement noise; no
native compiler speedup is established for the retained change. The cached-
dependency rebuild of `splitc` plus the benchmark took 26.11 seconds with
release and 1 minute 59 seconds with `max-opt`. This is an intentional build-
time/distribution-size tradeoff. Ordinary release settings remain unchanged.

The final embedded compiler falls from 8,882,426 to **6,345,753 bytes**, saving
2,536,673 bytes (28.6%). A separate Node 24 run uses the service ABI, 20 warmups
and 50 measured compilations per fixture, with synchronous Wasm compilation
and a single thread. It includes request/response handling:

| Fixture | Release median | `max-opt` median | Release p95 | `max-opt` p95 |
| --- | ---: | ---: | ---: | ---: |
| minimal | 57.09 ms | 54.82 ms | 60.00 ms | 57.03 ms |
| Lunistice | 67.52 ms | 65.77 ms | 70.34 ms | 68.40 ms |

This sample is modestly faster; the size reduction is the stronger result.
Instantiation was about 14 ms for both artifacts in this run, excluding the
first source compilation. This is not a browser-startup benchmark. The final
native CI build produced a 6,303,232-byte `splitc` and 4,324,864-byte `splitls`.
The first final-profile embedded build took 1 minute 46 seconds, including its
dependency rebuild. Native and embedded artifacts use identical compiler source.

Full `cargo xtask check` passed, including formatting, Clippy, documentation,
98 syntax tests, 420 library tests (one manual benchmark ignored), all 621
compiler integration tests, editor/browser workers, Wasm validation, and the
host-runtime matrix executed through the `max-opt` compiler.

Production `npm run package:vsix` also passed. The 1.93 MB VSIX contains the
6,345,753-byte `max-opt` compiler, verified by matching its archived SHA-256
against the Cargo artifact. Production Node compiler/LSP workers and browser
bundles passed their runtime checks. The compiler-worker test used a separately
bundled test client because production ships that client inside the extension
bundle rather than as the loose file expected by the development harness.

### Deferred experiment: typed unary field reads

Removing 68 assertions immediately before typed field-read helpers saved 146
bytes in Lunistice (145 instructions plus one body-length byte), 252 in Minish
Cap, 186 in Mono, 18 in managed instances, 5 in cancellation, 3 in settings,
and 37 in maps. Set and debug-profile controls kept identical non-custom
sections. All nine outputs validated, complete WAT differences contained only
the intended unary null assertions, and thirteen release runtime traces matched.
A focused debug/release test of packed fields, receiver effects, captures,
destructuring, and equality also passed during the experiment.

However, three native release comparisons showed a repeatable slowdown:

| Fixture | Saved before → experimental after (100 samples) | Reverse-order repeat (50 samples) | Freshly rebuilt before → after (50 samples) |
| --- | ---: | ---: | ---: |
| minimal | 48.13 → 52.24 ms | 49.00 → 52.44 ms | 49.07 → 52.52 ms |
| Lunistice | 57.05 → 62.06 ms | 56.45 → 60.95 ms | 56.44 → 61.39 ms |
| cancellation | 47.90 → 51.59 ms | 47.37 → 51.08 ms | 47.37 → 51.91 ms |
| settings | 50.54 → 54.23 ms | 49.82 → 53.73 ms | 50.27 → 53.92 ms |

All runs use 20 warmups. A frontend-only 50-sample run also regressed: minimal
24.79 → 28.61 ms, Lunistice 30.10 → 35.51 ms, cancellation 25.35 → 29.59 ms,
settings 26.70 → 30.38 ms. This path never emits Wasm, so extra encoder work
does not explain the difference. Native optimization/code-layout effects are
a hypothesis, not an isolated cause.

A same-profile `max-opt` comparison instead improved from
48.60/55.96/47.49/49.90 ms to 46.41/52.98/44.02/46.41 ms. That does not resolve
the ordinary release regression. The typed-read cleanup and its temporary
test were reverted; this batch does **not** claim their output-size savings or
their `max-opt` speedup. The latency priority outweighs these small size wins.

Before the separate-profile request, a command-line-only single-codegen-unit
probe on the experimental source produced an 8,942,080-byte native CLI and a
6,487,307-byte embedded compiler, versus 8,882,426 bytes for the prior embedded
release compiler. Its native medians were 48.03/56.11/47.14/50.13 ms. These
exploratory artifacts are not the final profile and did not change Cargo's
ordinary release configuration.

Evidence is retained under ignored `target/performance-review/typed-gc-*` and
`max-opt-*` files, including a patch for the deferred experiment. Future work
should reproduce latency in both Rust profiles before reviving that cleanup.

## 2026-09-08 parser brace-depth tracking

Source baseline: `25c23a8`. The parser previously scanned the complete consumed
token prefix whenever it entered a block or started recovery. A cursor-owned
depth now advances with tokens, and recovery uses that same depth. This removes
quadratic prefix scanning without caching/remapping syntax or adding a separate
debug/release path. Stray closing braces still saturate the depth at zero.

The new dependency-free syntax benchmark isolates parsing and disposal from
lexing and token cloning:

```console
cargo run -p splitscript-syntax --release --example parser_scaling -- 50
cargo run -p splitscript-syntax --profile max-opt --example parser_scaling -- 50
```

Each function has one nested conditional block. Runs use 10 warmups and 50
samples on Windows x86-64 with Rust 1.98.1. Medians:

| Functions | Source bytes | Rust release before → after | Rust max-opt before → after |
| --- | ---: | ---: | ---: |
| 100 | 4,810 | 0.288 → 0.221 ms | 0.257 → 0.208 ms |
| 500 | 24,410 | 1.809 → 0.854 ms | 1.710 → 0.851 ms |
| 1,000 | 48,910 | 5.307 → 1.810 ms | 5.111 → 1.777 ms |
| 2,000 | 98,910 | 17.614 → 3.878 ms | 17.096 → 3.626 ms |
| 4,000 | 198,910 | 78.826 → 7.940 ms | 77.647 → 7.444 ms |

Freshly built `compiler_baseline` binaries use 20 warmups and 50 measured
samples per fixture. Compilation and frontend runs were kept separate from
builds/tests. The reverse-order repeat confirms the full-compilation gains:

| Fixture | Rust release before → after | Release reverse-order repeat | Rust max-opt before → after | Max-opt reverse-order repeat |
| --- | ---: | ---: | ---: | ---: |
| minimal | 52.95 → 49.79 ms | 53.19 → 49.69 ms | 47.94 → 44.99 ms | 48.77 → 45.37 ms |
| Lunistice | 61.32 → 58.58 ms | 61.46 → 58.27 ms | 55.91 → 51.49 ms | 56.55 → 52.14 ms |
| cancellation | 52.88 → 49.21 ms | 51.43 → 49.59 ms | 46.39 → 43.17 ms | 47.66 → 44.08 ms |
| settings | 53.97 → 52.23 ms | 54.37 → 50.66 ms | 49.82 → 45.73 ms | 48.89 → 46.55 ms |

Frontend-only medians (`compiler_baseline -- 50 --frontend`), including result
disposal and excluding type checking/emission:

| Fixture | Rust release before → after | Rust max-opt before → after |
| --- | ---: | ---: |
| minimal | 28.79 → 26.01 ms | 25.52 → 21.79 ms |
| Lunistice | 35.16 → 32.21 ms | 30.54 → 26.87 ms |
| cancellation | 29.44 → 26.49 ms | 25.78 → 22.48 ms |
| settings | 31.38 → 27.59 ms | 27.11 → 23.44 ms |

The instrumented `tooling_baseline` result is mixed. Two `max-opt` runs (30
samples, 20 warmups, reverse order on the repeat) improve the generated
500-function file but make several small/Lunistice queries a few percent slower.
For example, first-run edit-to-diagnostics medians are 43.59 → 46.00 ms (small),
50.77 → 51.30 ms (Lunistice), and 64.05 → 62.19 ms (large). The repeat is
44.29 → 46.97, 51.30 → 51.94, and 63.99 → 62.45 ms. This allocator-instrumented
executable does not establish an across-the-board editor speedup. Its saved
output still says `profile=release`; the files were built with `--profile max-opt`.
The harness header now reports debug assertions, as `compiler_baseline` does,
rather than hardcoding a Cargo profile.

The actual distribution `splitls` executable was then measured over its stdio
LSP transport with its ordinary allocator, alternating a trailing-newline edit
and awaiting versioned diagnostics. These runs use the same three sources,
20 warmups and 50 samples; startup/initial document opening are excluded.
The pre-change distribution artifact has the same compiler logic as `25c23a8`.
Its build metadata identifies `5f50ac1`; the intervening profile/wiring commit
does not alter compiler or LSP logic. Both run orders improve, so the
instrumented harness's small-file regression does not reproduce
in this production workload. The cause of that discrepancy is not isolated.

| Fixture | Edit → diagnostics median before → after | p95 before → after | Reverse-order median before → after |
| --- | ---: | ---: | ---: |
| small | 44.53 → 41.62 ms | 46.18 → 43.45 ms | 45.00 → 41.19 ms |
| Lunistice | 50.86 → 46.98 ms | 52.47 → 48.36 ms | 50.09 → 46.97 ms |
| generated large | 75.95 → 69.28 ms | 79.53 → 74.06 ms | 77.67 → 68.63 ms |

The embedded `max-opt` compiler's service ABI was measured in Node 24 with
`--single-threaded --no-wasm-async-compilation`, 20 warmups and 50 samples.
Module instantiation is separate (roughly 14 ms in each run); this is repeated
compilation, not a browser startup measurement.

| Fixture | Median before → after | p95 before → after | Reverse-order median before → after |
| --- | ---: | ---: | ---: |
| minimal | 55.67 → 42.95 ms | 58.50 → 44.63 ms | 54.61 → 43.48 ms |
| Lunistice | 65.88 → 50.84 ms | 67.83 → 52.94 ms | 65.86 → 52.04 ms |

The embedded compiler shrinks slightly, from 6,345,753 to 6,342,419 bytes.
The native `max-opt` CLI is 6,299,136 bytes and language server 4,320,768 bytes,
each 4,096 bytes smaller than before. These are compiler artifacts, separate
from generated script size.

All nine release script fixtures are byte-identical, including metadata.
Lunistice remains 30,565 bytes and Minish Cap 45,113 bytes. All nine debug
fixtures have identical executable sections and line tables. Four differ only
in `.debug_info`; repeated old-compiler builds of `debug_profile.split` reproduce
both complete before/after hashes, identifying existing DWARF variable-order
nondeterminism rather than a parser regression.

Validation: full `cargo xtask check` passed, including 100 syntax tests,
420 library tests, 621 compiler integrations, editor/browser workers, Wasm
validation, and 95 runtime scenarios built from 67 unique artifacts. The new
syntax benchmark also passes a targeted Clippy check. Cursor tests exercise
unmatched braces, strings/comments, lookahead, EOF, contextual token splitting,
and clones; existing nested parser-recovery tests remain green.

Evidence is retained under ignored `target/performance-review/brace-*`,
`compiler-brace-*`, `parser-brace-*`, and `tooling-brace-*` files. Parsed-library
reuse and remaining frontend work still need separate profiling; this change
does not eliminate augmented parsing or claim script-size savings.

## 2026-09-08 expression delimiter-depth tracking

Source baseline: `21da5b6`, with brace-depth tracking already retained. Calls,
array lists, parenthesized expressions, and expression recovery still scanned
the entire consumed token prefix for parentheses/bracket/brace counts. The
cursor now maintains all three counts, and root recovery saves the starting
depth before parsing an expression. No delimiter-prefix scan remains, and
recovery shares the cursor's counters. Crossed delimiters retain independent
counts with saturating closers, preserving malformed-source behavior.

The syntax benchmark now accepts `--calls` for functions containing
`consume([1, 2], pair(3, 4))`:

```console
cargo run -p splitscript-syntax --release --example parser_scaling -- 10 --calls
cargo run -p splitscript-syntax --profile max-opt --example parser_scaling -- 10 --calls
```

Windows x86-64, Rust 1.98.1. These runs have 10 warmups and 10 measured samples;
lexing/token cloning are outside the timer, parsing and disposal are inside.
Medians:

| Functions | Source bytes | Rust release before → after | Rust max-opt before → after |
| --- | ---: | ---: | ---: |
| 100 | 5,310 | 1.136 → 0.276 ms | 1.387 → 0.264 ms |
| 500 | 26,910 | 22.721 → 1.310 ms | 28.933 → 1.305 ms |
| 1,000 | 53,910 | 85.821 → 2.497 ms | 113.743 → 2.357 ms |
| 2,000 | 108,910 | 344.398 → 5.070 ms | 443.385 → 5.116 ms |
| 4,000 | 218,910 | 1,377.603 → 10.197 ms | 1,745.079 → 9.888 ms |

Fresh native compiler benchmarks use 20 warmups and 50 measured samples per
fixture, with no concurrent builds/tests from this task. All scripts use the
SplitScript release profile; columns distinguish the Rust executable profiles.

| Fixture | Rust release before → after | Release reverse-order repeat | Rust max-opt before → after | Max-opt reverse-order repeat |
| --- | ---: | ---: | ---: | ---: |
| minimal | 49.46 → 26.42 ms | 49.33 → 26.11 ms | 44.69 → 24.78 ms | 44.63 → 24.96 ms |
| Lunistice | 57.24 → 29.54 ms | 57.53 → 28.58 ms | 51.67 → 27.39 ms | 50.79 → 27.75 ms |
| cancellation | 48.52 → 24.81 ms | 48.98 → 24.30 ms | 43.35 → 22.81 ms | 43.78 → 22.99 ms |
| settings | 51.16 → 25.66 ms | 51.77 → 24.28 ms | 46.76 → 23.37 ms | 45.66 → 23.54 ms |

Frontend-only medians from `compiler_baseline -- 50 --frontend`, including
disposal and excluding checking/emission:

| Fixture | Rust release before → after | Rust max-opt before → after |
| --- | ---: | ---: |
| minimal | 25.77 → 2.21 ms | 21.97 → 2.23 ms |
| Lunistice | 31.86 → 2.68 ms | 26.96 → 2.60 ms |
| cancellation | 26.56 → 2.26 ms | 22.35 → 2.19 ms |
| settings | 27.92 → 2.22 ms | 23.56 → 2.19 ms |

The actual `max-opt` language server was measured over stdio, alternating
trailing-newline edits and awaiting versioned diagnostics. These use 20 warmups
and 50 samples, the ordinary allocator, and the same sources as the previous
distribution benchmark. Initial process/document startup is excluded. Saved
pre-change distribution binaries have the compiler logic of `21da5b6`.

| Fixture | Edit → diagnostics median before → after | p95 before → after | Reverse-order median before → after |
| --- | ---: | ---: | ---: |
| small | 41.17 → 21.65 ms | 42.84 → 22.72 ms | 41.31 → 21.86 ms |
| Lunistice | 46.71 → 22.29 ms | 49.19 → 23.82 ms | 47.37 → 22.77 ms |
| generated large | 71.54 → 45.44 ms | 76.83 → 50.21 ms | 70.94 → 45.57 ms |

The embedded compiler's service ABI uses Node 24.14.0 with
`--single-threaded --no-wasm-async-compilation`, 20 warmups and 50 samples:

| Fixture | Median before → after | p95 before → after | Reverse-order median before → after |
| --- | ---: | ---: | ---: |
| minimal | 43.19 → 20.26 ms | 45.25 → 21.38 ms | 43.15 → 20.24 ms |
| Lunistice | 51.65 → 24.10 ms | 54.84 → 24.74 ms | 51.24 → 23.72 ms |

Module instantiation remains roughly 14 ms and is outside these compilation
timings; this does not measure browser startup. The embedded compiler changes
from 6,342,419 to 6,338,996 bytes. Native `max-opt` CLI size changes from
6,299,136 to 6,296,576 bytes, and LSP from 4,320,768 to 4,318,720 bytes.
These small compiler-artifact reductions are separate from script Wasm size.

This changes the priority of parsed-library caching: the whole frontend now
costs about 2–3 ms in these fixtures. Profile the remaining checking/emission
work before adding template cloning and syntax-ID remapping to save a fraction
of that budget. This change retains full augmented parsing and validation.

All nine release script fixtures are byte-identical, including metadata.
Lunistice remains 30,565 bytes and Minish Cap 45,113 bytes. Debug executable
sections and line tables match on all nine fixtures; only the already-recorded
DWARF variable-order variation appears, in two `.debug_info` sections this run.
Forty before/after malformed-source comparisons preserve complete diagnostics
and source locations across globals, state fields, functions, and actions.

Validation: full `cargo xtask check` passed, including 102 syntax tests,
420 library tests, 621 compiler integrations, editor/browser workers, Wasm
validation, and all 95 runtime scenarios from 67 unique artifacts. The updated
syntax benchmark also passes targeted Clippy. New tests cover independent
delimiter counts, crossed/stray closers, string contents, contextual generic
token merging, snapshots, and cloned cursors; existing recovery tests pass.

Raw logs and saved binaries are under ignored `target/performance-review`,
using `delimiter-*`, `compiler-delimiter-*`, `parser-delimiter-*`, and
`splitc-delimiter-*`, `splitls-delimiter-*`, and `embedded-delimiter-*` names.

## 2026-07-28 historical baseline

- Rust: `rustc 1.97.0 (2d8144b78 2026-07-07)`, LLVM 22.1.6
- Cargo: 1.97.0
- Platform: Windows x86-64, 32 logical CPUs
- Rust harness profile: `release`
- SplitScript profile: `debug` (the one-shot API's default at the time)
- Warmups: 20 per fixture
- Samples: 200 per fixture

| Fixture | Source bytes | Wasm bytes | Median | p95 |
| --- | ---: | ---: | ---: | ---: |
| minimal | 19 | 433 | 8.0 µs | 8.5 µs |
| Lunistice | 6,823 | 11,763 | 716.3 µs | 873.7 µs |
| cancellation | 506 | 2,155 | 50.0 µs | 65.3 µs |
| settings | 2,735 | 4,400 | 92.2 µs | 121.2 µs |

The Lunistice fixture is currently the broadest real autosplitter in the
repository and is the primary trend signal. The smaller fixtures help identify
fixed compiler overhead and regressions isolated to async lowering or settings.

## Tooling and large-source baseline

Reusable editor-query and in-process LSP measurements have a separate runner:

```console
cargo run --release --example tooling_baseline -- 500 100
```

The optional arguments select the number of generated helper functions and
measured samples. The runner uses a focused small script, the maintained
Lunistice source as a real medium script, and a deterministic generated large
source. It measures cold and post-edit diagnostics, root and member completion,
hover, semantic tokens, a warm multi-query sequence, full-sync `didChange` to
published diagnostics, and in-process language-server restart to hover. Warm
operations receive 20 unmeasured calls first.

A counting system allocator remains active for the whole run. Every latency row
therefore also reports net retained growth after its warmup and the largest
transient heap increase during its measured samples. A second table constructs
one fresh database per query shape and reports the complete retained cache and
peak heap attributable to that live database. The allocation atomics add a
small constant measurement cost, so compare latency only with runs from this
same runner. The restart row covers rebuilding the Rust language service; the
separate portable-toolchain work still owns browser/desktop Worker startup and
transport measurements.

Record results with the toolchain, platform, source size, function count, and
sample count. Timing remains a trend signal rather than a brittle test
threshold. Add a generated large-catalog dimension when `CompilerContext` can
own and inject an alternate validated graph; measuring repeated lookups in the
fixed bundled catalog would not exercise catalog scaling.

### 2026-07-31 tooling baseline

- Rust: `rustc 1.97.0 (2d8144b78 2026-07-07)`, LLVM 22.1.6
- Platform: Windows x86-64, 32 logical CPUs
- Profile: `release`
- Fixture: 500 functions, 29,879 source bytes
- Warmups: 20 per warm query
- Samples: 200 (one sample for the intentionally cold check)

| Query | Median | p95 |
| --- | ---: | ---: |
| database cold check | 3,966.7 µs | 3,966.7 µs |
| database warm check | 0.0 µs | 0.1 µs |
| database warm hover | 5.6 µs | 6.1 µs |
| database warm highlights | 0.0 µs | 0.1 µs |
| database warm definitions | 0.0 µs | 0.1 µs |
| in-process LSP warm hover | 31.7 µs | 45.3 µs |

### 2026-08-31 interactive-query baseline

- Rust: `rustc 1.98.0`, release profile
- Platform: Windows x86-64, 32 logical CPUs
- Fixtures: 171-byte focused source, 4,554-byte Lunistice source, and a
  29,990-byte generated source with 500 helpers
- Warmups: 20 per warm query
- Samples: 30 per latency row

| Fixture | Query | Median | p95 | Peak heap delta |
| --- | --- | ---: | ---: | ---: |
| small | cold diagnostics | 47.5 µs | 67.9 µs | 55.6 KiB |
| small | edit → diagnostics | 44.6 µs | 61.2 µs | 13.8 KiB |
| small | edit → root completion | 49.8 ms | 51.0 ms | 3.1 MiB |
| small | edit → member completion | 161.1 ms | 166.4 ms | 5.5 MiB |
| small | edit → hover | 85.7 ms | 90.3 ms | 3.1 MiB |
| small | edit → semantic tokens | 84.9 ms | 92.4 ms | 3.1 MiB |
| small | warm multi-query sequence | 159.5 ms | 166.1 ms | 5.5 MiB |
| small | LSP `didChange` → diagnostics | 102.0 µs | 105.3 µs | 21.9 KiB |
| small | LSP restart → hover | 79.7 ms | 85.4 ms | 5.5 MiB |
| Lunistice | cold diagnostics | 92.0 ms | 98.5 ms | 6.2 MiB |
| Lunistice | edit → diagnostics | 92.1 ms | 96.4 ms | 1.5 MiB |
| Lunistice | edit → root completion | 98.4 ms | 104.1 ms | 1.5 MiB |
| Lunistice | edit → member completion | 251.5 ms | 262.4 ms | 6.3 MiB |
| Lunistice | edit → hover | 92.5 ms | 99.0 ms | 1.4 MiB |
| Lunistice | edit → semantic tokens | 92.1 ms | 97.3 ms | 1.4 MiB |
| Lunistice | warm multi-query sequence | 249.7 ms | 259.2 ms | 6.3 MiB |
| Lunistice | LSP `didChange` → diagnostics | 92.7 ms | 95.8 ms | 1.5 MiB |
| Lunistice | LSP restart → hover | 91.5 ms | 96.7 ms | 6.2 MiB |
| generated large | cold diagnostics | 13.5 ms | 17.2 ms | 6.3 MiB |
| generated large | edit → diagnostics | 12.1 ms | 13.8 ms | 1.0 MiB |
| generated large | edit → root completion | 147.0 ms | 155.4 ms | 8.6 MiB |
| generated large | edit → member completion | 382.0 ms | 412.9 ms | 18.0 MiB |
| generated large | edit → hover | 142.3 ms | 154.4 ms | 7.9 MiB |
| generated large | edit → semantic tokens | 151.4 ms | 163.2 ms | 8.0 MiB |
| generated large | warm multi-query sequence | 378.2 ms | 385.3 ms | 18.0 MiB |
| generated large | LSP `didChange` → diagnostics | 12.1 ms | 12.7 ms | 1.0 MiB |
| generated large | LSP restart → hover | 153.1 ms | 167.8 ms | 18.6 MiB |

The steady live-database cache after the complete warm query sequence is
2.5 MiB for the small fixture, 4.8 MiB for Lunistice, and 10.7 MiB for the
generated large fixture. The corresponding one-shot peak deltas are 8.0 MiB,
11.0 MiB, and 28.4 MiB.

Initial optimization targets for this fixed runner are:

- p95 below 100 ms for each individual post-edit query on every fixture;
- p95 below 150 ms for the complete warm multi-query sequence;
- at most 8 MiB retained by that complete sequence; and
- at most 20 MiB transient heap growth during one sequence.

### 2026-08-31 completion-context follow-up

The first optimization pass made completion borrow the revision's recovered
source document, syntax, and lossless token stream; consult the current
database before constructing a repair probe; and carry only the inferred
receiver type and constraints out of a probe. It also replaced the completion
cursor's ordered-token searches with `partition_point`. The same 30-sample run
then measured:

| Fixture | Query | Baseline p95 | Follow-up p95 |
| --- | --- | ---: | ---: |
| small | edit → member completion | 166.4 ms | 179.2 ms |
| small | warm multi-query sequence | 166.1 ms | 82.4 ms |
| Lunistice | edit → member completion | 262.4 ms | 258.9 ms |
| Lunistice | warm multi-query sequence | 259.2 ms | 177.1 ms |
| generated large | edit → member completion | 412.9 ms | 373.2 ms |
| generated large | warm multi-query sequence | 385.3 ms | 235.6 ms |

The standalone member row includes the semantic pass needed by a newly edited,
otherwise cold database, so the small-fixture variation is within the observed
machine noise rather than evidence of a regression. The realistic warm
sequence benefits directly: it no longer repeats semantic analysis in a second
database for member completion. Its retained cache remains 2.5 MiB, 4.8 MiB,
and 10.7 MiB respectively; sequence peak deltas remain 8.0 MiB, 11.0 MiB, and
27.9 MiB. The remaining medium and large sequence misses therefore point to
semantic-stage ownership/copying and candidate construction, not another
completion lexer pass.

Diagnostics already satisfy the interaction target. Completion—especially
member completion—and the semantic products it requests are the first measured
bottleneck. Optimize those paths before changing full-sync transport or adding
incremental invalidation machinery.

### 2026-09-04 interactive-query baseline

- Commit: `70998d4`
- Rust: `rustc 1.98.1 (48a229cea 2026-09-01)`, LLVM 22.1.8
- Platform: Windows x86-64, 32 logical CPUs
- Rust harness profile: `release`
- Fixtures: 171-byte focused source, 4,554-byte Lunistice source, and a
  29,990-byte generated source with 500 helpers
- Warmups: 20 per warm query
- Samples: 30 per latency row

| Fixture | Query | Median | p95 |
| --- | --- | ---: | ---: |
| small | cold diagnostics | 52.0 µs | 80.0 µs |
| small | edit → diagnostics | 49.8 µs | 88.1 µs |
| small | edit → root completion | 51.1 ms | 52.5 ms |
| small | edit → member completion | 98.2 ms | 111.9 ms |
| small | edit → hover | 48.3 ms | 49.9 ms |
| small | edit → semantic tokens | 49.5 ms | 50.1 ms |
| small | warm multi-query sequence | 49.1 ms | 50.3 ms |
| small | LSP `didChange` → diagnostics | 55.9 µs | 62.4 µs |
| small | LSP restart → hover | 50.0 ms | 51.0 ms |
| Lunistice | cold diagnostics | 56.9 ms | 58.1 ms |
| Lunistice | edit → diagnostics | 56.3 ms | 57.9 ms |
| Lunistice | edit → root completion | 56.5 ms | 57.9 ms |
| Lunistice | edit → member completion | 153.0 ms | 161.2 ms |
| Lunistice | edit → hover | 55.5 ms | 57.4 ms |
| Lunistice | edit → semantic tokens | 56.3 ms | 57.8 ms |
| Lunistice | warm multi-query sequence | 157.3 ms | 171.2 ms |
| Lunistice | LSP `didChange` → diagnostics | 56.8 ms | 59.3 ms |
| Lunistice | LSP restart → hover | 56.7 ms | 58.3 ms |
| generated large | cold diagnostics | 7.3 ms | 8.5 ms |
| generated large | edit → diagnostics | 7.1 ms | 7.8 ms |
| generated large | edit → root completion | 80.1 ms | 82.4 ms |
| generated large | edit → member completion | 199.2 ms | 204.1 ms |
| generated large | edit → hover | 78.4 ms | 87.3 ms |
| generated large | edit → semantic tokens | 153.3 ms | 161.4 ms |
| generated large | warm multi-query sequence | 234.8 ms | 246.8 ms |
| generated large | LSP `didChange` → diagnostics | 12.7 ms | 14.8 ms |
| generated large | LSP restart → hover | 154.0 ms | 162.6 ms |

Most latency rows improved materially from the 2026-08-31 baseline. In
particular, Lunistice cold diagnostics improved from 92.0 ms to 56.9 ms and
member-completion p95 improved from 258.9 ms to 161.2 ms. The generated warm
sequence p95 increased modestly from 235.6 ms to 246.8 ms, while its member
completion p95 improved from 373.2 ms to 204.1 ms.

Memory is recorded for completeness but is lower priority than compiler latency
and generated WebAssembly size. The retained live-database cache after the warm
sequence is 2.89 MiB for small, 5.67 MiB for Lunistice, and 12.62 MiB for
generated large, versus 2.5 MiB, 4.8 MiB, and 10.7 MiB on 2026-08-31. The
corresponding peak deltas are 9.56 MiB, 12.74 MiB, and 33.35 MiB, versus 8.0
MiB, 11.0 MiB, and 27.9 MiB.

### 2026-09-12 shared compiler-product follow-up

The compiler database now shares one immutable source document and syntax tree
across recovering parse, strict parse, lowering, checking, and recovery.
Lowered declaration and resolution products are also shared with checking;
typed HIR owns only its transformed body facts while retaining the same
declaration index. This removes the former whole-program clones at database
stage boundaries.

Measurements used Rust 1.98.1, the release harness, 20 warmups, and 30 measured
samples. Runs were sequential. The stage comparison used the same working-tree
source before and after the change:

| Fixture | Stage | Before median | After median |
| --- | --- | ---: | ---: |
| small | parse | 10.1 µs | 7.7 µs |
| small | lower | 2,489.4 µs | 2,424.8 µs |
| small | check | 14,985.1 µs | 14,556.3 µs |
| generated large | parse | 1,759.5 µs | 1,249.9 µs |
| generated large | lower | 5,507.5 µs | 4,485.4 µs |
| generated large | check | 25,939.2 µs | 22,657.2 µs |

For an exact immediate-parent comparison, commit `6add20a` and the changed
compiler were built in turn into the same target directory. The maintained
Lunistice file had unrelated working-tree changes and is therefore deliberately
omitted from this table; the small and generated fixtures were byte-identical.

| Fixture | Query | Parent p95 | Shared-products p95 |
| --- | --- | ---: | ---: |
| small | cold diagnostics | 15.73 ms | 15.58 ms |
| small | edit → diagnostics | 15.10 ms | 15.38 ms |
| small | edit → root completion | 14.67 ms | 14.39 ms |
| small | edit → member completion | 14.59 ms | 14.43 ms |
| small | edit → hover | 24.88 ms | 23.89 ms |
| small | edit → semantic tokens | 26.36 ms | 24.34 ms |
| generated large | cold diagnostics | 49.48 ms | 43.13 ms |
| generated large | edit → diagnostics | 47.76 ms | 43.57 ms |
| generated large | edit → root completion | 53.15 ms | 49.42 ms |
| generated large | edit → member completion | 54.34 ms | 49.57 ms |
| generated large | edit → hover | 55.71 ms | 53.37 ms |
| generated large | edit → semantic tokens | 58.96 ms | 55.30 ms |

The generated fixture's retained complete-query cache fell from 13.76 MiB to
9.64 MiB. Its already-cached multi-query p95 varied from 11.49 ms to 12.91 ms;
this change therefore claims improvement for revision-building editor queries
and retained ownership, not for query work that executes after every requested
product is already cached.
