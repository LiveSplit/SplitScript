# Compiler and generated WebAssembly performance plan

Reviewed: 2026-09-04, against `70998d4` plus the existing uncommitted
standard-library caching/indexing changes. At review time `HEAD` matched the
local `origin/master` reference; the performance work was in the working tree.

Start by eliminating repeated work and unnecessary output. The largest known
compile-time opportunity is reusing standard-library compiler products. The
lowest-risk output improvements are compact local declarations, shared ordinary
function signatures, and operation-level reachability for sets. General
optimization passes should follow those changes, rather than block them.

Priority clarification (2026-09-07): compiler/editor latency and generated
release Wasm size are the main goals. Compiler RAM is secondary; defer further
ownership changes, including shared source documents, unless measurements show
a worthwhile latency benefit. Prefer shared code generation across debug and
release when the improvement is a cheap encoding choice. Reserve profile-specific
optimization for work with a meaningful compilation cost, and preserve debug
metadata without introducing unnecessary differences in runtime behavior.

The original review and measurements below are retained as the starting point.
Implementation progress is tracked separately here; investigation artifacts
are under ignored `target/performance-review`.

## Current priorities after the September 22 reassessment

Rebuilt `40d72a0` and `4720d69` with identical compatible fixtures. Ordinary
Rust-release compilation is now roughly **4–5 times slower**: minimal
19 → 92–94 ms, Minish Cap 23 → 98 ms. Actual stdio LSP diagnostics show the
same fixed-cost increase: small 15 → 78–80 ms, 500 helpers 39 → 102–104 ms.
Reverse-order repeats confirm the difference. These are fresh measurements;
the older `max-opt` timings below are historical, not today's baseline.

Generated Release size moved in the other direction. Minish Cap is now 35,334
bytes (previously 45,636), and all seven compatible size fixtures shrank.
Current explicit/automatic Lunistice remains at the accepted 32,121/142,234
bytes. Binaryen 132, used only as an offline reference, reaches 26,604 and
104,123 bytes respectively with `-Oz --closed-world`. This is evidence of
remaining opportunity, not a proposal to ship Binaryen or a promised result
from one native pass. See [the complete measurements](docs/BASELINES.md#2026-09-22-performance-reassessment).

The next implementation order is:

1. **Reduce repeated effect-analysis work (implemented September 22).** Temporary profiling of minimal
   compilation attributes about 36 ms to `effects::polymorphic::infer`: nine
   rounds reevaluate all 408 function bodies. First prototype dependency-driven
   reevaluation or per-body summary reuse within one analysis. Track every
   summary read, including higher-order calls and nested closures, and preserve
   the existing fixed-point semantics, ordering, and iteration bound. Validate
   against full reevaluation on recursive calls, returned closures, iterator
   effects, source-defined capabilities, and the bundled library. Do not skip
   validation of uncalled bodies or cache compilation-owned IDs across programs.
   The implementation records summary reads, including nested closures and
   implicit display calls, and reevaluates a body only when one of those inputs
   changed. Updates remain simultaneous, with the same iteration limit. Every
   body is still analyzed initially, and reuse ends with the current analysis.
   Ordinary-release compilation improves by 28–33% in paired measurements;
   minimal compilation drops from 93–96 to 65–66 ms. The remaining latency is
   still well above September 12, so typed-HIR work is the next priority.
2. **Investigate typed-HIR construction before adding a large cache.** It costs
   about 13 ms even for minimal input. Separate syntax visiting, expression
   materialization, and function-body construction; look for repeated tree
   walks and index lookups. Keep improvements shared by compiler and LSP and by
   Debug and Release. Type checking itself is another 16 ms, mainly body work.
   Concrete candidates in `TypedProgram::build`: expressions, assignments, and
   patterns first enter hash maps and are then collected and sorted; binding
   patterns are also lowered while constructing function/global bodies after
   the syntax visitor has populated the pattern table. Profile these costs and
   verify ID/ownership requirements before replacing either path.
   A measured follow-up now indexes nearly dense expression IDs, avoiding
   binary searches throughout later analysis and lowering (another 4–10%
   median compilation improvement). Sparse IDs retain the existing lookup.
   Removing value-block prefix copies did not produce a repeatable gain and
   was not retained. See [the lookup measurements](docs/BASELINES.md#2026-09-22-indexed-hir-expression-lookup).
3. **Revisit library-product reuse with the new floor.** The standard-library
   source grew from 308,872 to 600,330 bytes. Minimal compilation analyzes
   17,058 expressions, and frontend-only time rose from 2.27 to 8.62 ms.
   The design constraints in order 8 still apply, but the previous 2–3 ms
   argument for indefinite deferral is no longer current. Measure the benefit
   after the smaller effect/HIR changes before choosing parsed versus typed
   template reuse. Wasm lowering also rose from 2.67 to 11.06 ms; revisit demand
   driven lowering only through the existing dependency contracts.
4. **Then resume native Release size passes.** Follow the constant-folding,
   size-driven inlining, and reachability sequence already in `TODO.md`.
   Measure minimal/native, async, explicit Unity, automatic Unity, and managed
   collection cases. Preserve the Unity runtime checks accepted at closeout.
   Cheap encoding improvements should continue to share Debug/Release code;
   only costly optimization passes need a Release-only path.

For every compiler slice, repeat the same-input latency comparison in both
orders, check actual LSP diagnostics with `scripts/lsp_baseline.mjs`, validate
output and runtime behavior, and then measure the packaged `max-opt` binaries.
Keep normal `release` unchanged. The initial reassessment added a reusable LSP
benchmark and updated the evidence and priorities. The subsequent summary-reuse
implementation and its verification are recorded in
[the follow-up measurements](docs/BASELINES.md#2026-09-22-effect-summary-reuse).

## First implementation batch

Implemented on 2026-09-04:

- Cached immutable backend contract validation and indexed function declarations
  and Wasm bodies, including the generic-template fallback (part of order 2).
- Grouped adjacent script/async/start-function locals and interned ordinary
  function signatures across imports and definitions (order 3). Existing
  hand-written runtime local groups are unchanged.
- Added an eight-entry LRU cache of owned completion receiver facts, including
  failed probes, invalidated with the source revision (part of order 5). It
  retains no temporary database. This removes repeated probes at unchanged
  completion sites; the first request after an edit can still require a probe.
- Grouped specialization expressions by owner with stable expression ordering
  (order 6), and borrowed semantic snapshots when building highlights (the small
  first step of order 7).

Release sizes are now 34,526 bytes for Lunistice, 48,773 for Minish Cap, and
3,152 for the set runtime fixture. See [baselines](docs/BASELINES.md) for the
before/after results. These reductions do not include set-operation pruning.

Validation: the full `cargo xtask check` passed for this batch, including the new
cache/type/local regression tests, editor/browser workers, Wasm validation, and
host-runtime fixtures. Repeated Lunistice and Minish Cap builds are byte-identical.

Still open: detailed stage instrumentation, shared stage ownership, parsed
standard-library reuse, and the larger backend/optimizer work. The suggested
delivery order remains a guide to those remaining slices, not a list of fully
completed milestones.

## Set-operation implementation batch

Implemented order 4 on 2026-09-04. Reachability now records each demanded set
operation per concrete element type, including generic specializations. Function
declarations and bodies use that demand, retaining the internal `insert ->
contains` dependency and comparison equality helpers. Set layouts remain
independently reachable for storage, iteration, and signatures.

The new/length-only release probe shrank from 1,943 to 1,534 bytes (409 bytes,
21.0%) relative to `dcfb1ec`. The expanded runtime fixture, which also exercises
insertion without an explicit contains call on a second set type, shrank from
3,808 to 3,597 bytes on identical input. Lunistice and Minish Cap are unchanged.
See [baselines](docs/BASELINES.md) for the fixture and comparison details.

Regression coverage checks every operation for numeric, string, and array
elements; iteration; generic constructors; unused functions; and removal of
debug-only operation demand in release. The expanded host-runtime fixture
checks duplicate insertion in both profiles.

Validation: the full `cargo xtask check` passed, including 605 compiler
integration tests, library tests, documentation, VS Code/browser checks, Wasm
validation, and host-runtime fixtures.

## Direct completion receiver implementation batch

Implemented another part of order 5 on 2026-09-04. Member completion now uses
the current semantic snapshot for a known receiver expression or the selected
prefix of a resolved field/call path. It distinguishes a method's receiver from
its result and handles source fields, snapshot/settings roots, indexed values,
result propagation, and generic constraints.

Successful strict analysis avoids a repaired database at these sites even on
the first request after an edit. Recovered or missing facts still use the
existing bounded probe cache. Constructor-field substitutions without direct
facts continue to use repair. Root-effect completion is addressed separately
below.

Regression tests count probes and check candidate types across path prefixes,
call results, source edits, recovered members, and failed lookups. Updated
timings and validation are recorded in [baselines](docs/BASELINES.md).

Paired edit-to-member-completion medians improved from 169.25 to 83.63 ms
(small), 253.78 to 93.74 ms (Lunistice), and 381.03 to 146.93 ms (500 helpers).
These are valid-source field-completion sites, not an estimate for every
incomplete edit. Standard-library parsing/checking remains a major fixed cost.

Validation: all 49 targeted completion tests and the full `cargo xtask check`
passed, including 406 library tests, 605 compiler integration tests, documentation,
editor/browser checks, and the Wasm/runtime fixtures.

## Root completion availability implementation batch

Implemented the root-effect portion of order 5 on 2026-09-04. Completion now
keeps compact sets of source-function IDs requiring an attached process or state
snapshots, instead of copying the complete operation analysis on every request.
Direct facts are shared for the revision. Repaired results, including failures,
use a four-entry LRU keyed by the identifier's replacement span; source edits
invalidate the cache. Repaired compiler databases are not retained.

Top-level completion, fully available lifecycle contexts, and files without
standalone user functions do not request this availability analysis at all.
Context filtering still applies at each request, including transitive process
and state dependencies.

Added a focused `tooling_baseline 500 30 --root-effects` mode for valid,
partially typed, and unsuccessfully repaired root-completion sites. Regression
tests check context-specific filtering, reuse across caret positions and
contexts, source edits, failed results, and bounded eviction. Timings and
validation are recorded in [baselines](docs/BASELINES.md).

With 500 helpers, repeated partial-root completion improved from 138.13 to
5.40 ms; repeated failed repair improved from 217.87 to 4.77 ms. First requests
after edits remain approximately unchanged. Retained cache growth is 96–216
bytes for the measured workloads.

Validation: all 53 targeted completion tests and the full `cargo xtask check`
passed, including 410 library tests, 605 compiler integration tests, documentation,
editor/browser checks, and Wasm/runtime fixtures.

## Lazy release debug-name implementation batch

Implemented the remaining debug-name item from order 2 on 2026-09-04. Function
declaration accepts lazy name builders; release planning does not format or
store names for helpers, specialized source functions, async pollers, closures,
or function-value adapters. Names shared by an async initializer/poller are
still rendered once in debug mode.

Function indices and body counts now derive from the function section itself.
They no longer depend on a debug-name vector existing. A 129-declaration
regression checks identical encoded declarations in both profiles, zero release
name-builder calls, and matching body counts. Existing profile tests cover
debug names and DWARF locations.

Release outputs for seven representative sources are byte-identical, including
Lunistice, Minish Cap, sets, async loops, postfix calls, debug-only code, and a
closure/function-value fixture. Some debug `.debug_info` sections vary between
processes; repeat runs of the unchanged before compiler reproduce that behavior.
The other sections, including executable code and names, are identical. This
existing debug reproducibility issue is separate follow-up work.

See [baselines](docs/BASELINES.md) for timing results and validation. This removes
compiler bookkeeping work; it does not reduce emitted release-module sizes.
The paired compiler timings do not establish an overall compile-time improvement.

Validation: the focused declaration test, all 24 profile codegen tests, and the
full `cargo xtask check` passed, including 411 library tests, 605 compiler
integration tests, documentation, editor/browser checks, and Wasm/runtime fixtures.

## Standard-library token reuse implementation batch

Implemented a precursor to order 8 on 2026-09-04. Each standard-library graph
now retains the tokens for its rendered source. Augmentation clones those
tokens and rebases their spans, lexing only the user prefix and the generated
provider suffix. Dynamic tokens move out of the lossless lexer result instead
of cloning their owned text. Lexical failures fall back to whole-source lexing
to preserve behavior at fragment boundaries.

The combined source still passes through one parser, preserving syntax ID
allocation, constructed-type interning, user-visible counts, and diagnostics.
This does not reuse parsed ASTs or checked semantics, and it still reparses the
user prefix. Parsed-template reuse needs a separate remapping design because
constructed types are interned across all declarations in a compilation.

Token-equivalence tests cover distinct graphs, empty libraries, generated managed
provider preparation, Unicode, comments, incomplete syntax, and lexical errors.
The existing parallel initialization and augmentation tests also pass. A new
`compiler_baseline -- 200 --frontend` mode measures parse/augmentation/resolution
separately from checking and code generation. See [baselines](docs/BASELINES.md)
for the measured cost of token copying versus repeated lexing.

The alternating token benchmark improved from 1.305 to 0.198 ms (84.8%). Full
compilation medians improved by 0.1–1.6 ms in the paired run, while p95 values
overlap. Nine representative release modules are byte-identical, including
both managed-runtime backends. Parsed-template and semantic reuse remain open.

Validation: all nine targeted library-injection tests and the full
`cargo xtask check` passed, including 413 library tests (one manual benchmark
ignored), 605 compiler integration tests, documentation, editor/browser checks,
and Wasm/runtime fixtures. The manual release benchmark also passed separately.

## Shared strict/recovery lowering implementation batch

Implemented another slice of order 7 on 2026-09-06. The editor database now
shares one `Arc<LoweredProgram>` between strict and recovery queries whenever
strict lowering succeeds. Type errors and later validation errors no longer
cause recovery to repeat standard-library augmentation, parsing, and declaration
resolution or retain a second copy of the lowered program. Sharing works with
either query order and remains scoped to the source revision.

Syntax errors and failed generated-source augmentation still use the recovered
user tree. This fallback never enters the strict cache and does not retry the
same failed augmentation. Warning-policy changes retain semantic products;
source edits invalidate both lowering queries.

All 47 targeted compiler-query tests passed, including new checks for query
order, error classes, diagnostic offsets, warning policy, and valid/invalid
revision transitions. The `tooling_baseline -- 500 30 --recovery` mode measures
edit-to-diagnostics-plus-hover latency and retained/peak heap growth on valid,
type-error, validation-error, and syntax-error inputs. See
[baselines](docs/BASELINES.md) for the paired results.

Measured error-path medians improved from 83.75 to 55.85 ms (small type error),
131.61 to 94.72 ms (500 helpers), and 80.65 to 55.12 ms (validation error).
Valid-source and syntax-error controls are approximately unchanged. Peak heap
growth during the edit query fell from 5.45 to 3.56 MiB for the small error
fixtures and from 16.36 to 11.36 MiB for the large one; the runner reported
unchanged retained-state values. Seven release fixtures are byte-identical.

Validation: the full `cargo xtask check` passed, including 414 library tests
(one manual benchmark ignored), 608 compiler integration tests, documentation,
editor/browser checks, and Wasm/runtime fixtures.

This shares lowering only. Strict and recovering type inference can still run
separately after a failed check; retaining partial inference results remains the
next part of order 7. Cross-compilation parsed-library reuse remains separate.

## Failed-check inference reuse implementation batch

Implemented the diagnostics-first inference-reuse slice of order 7 on
2026-09-06. The strict checker now has an internal tooling path that retains
partial semantics when inference fails, rather than discarding the output and
running inference again for recovery. The editor database publishes those facts
in its revision's recovery cache while keeping strict checking unsuccessful.
The public one-shot strict API still returns only a checked program or errors.

Failures in post-type validation also retain the already-computed effect facts
when strict and recovery validation prerequisites agree. Syntax/resolution
failures before inference keep the existing recovery path. Existing recovery
snapshots are not replaced if recovery was explicitly requested first.

Tests count inference runs and compare retained results with independently
executed strict/recovery APIs: diagnostics and ordering, visible expression IDs,
inferred values, resolved calls, and effects. Additional cases cover warning
policy, source edits, successful repairs, and failures before inference.
See [baselines](docs/BASELINES.md) for the paired recovery-query measurements.

Measured semantic-error medians improved another 24.6–26.5%: 53.49 to 39.94 ms
for a small type error, 92.93 to 70.08 ms with 500 helpers, and 53.92 to 39.64 ms
for unavailable detached-state access. Valid and syntax-error controls and heap
readings are approximately unchanged. Seven release fixtures are byte-identical.

Validation: four focused inference-reuse tests, all 47 compiler-query tests, and
the full `cargo xtask check` passed, including 418 library tests (one manual
benchmark ignored), 608 compiler integration tests, documentation, editor/browser
checks, and Wasm/runtime fixtures.

This avoids duplicate inference after the normal strict-first failed check.
Recovery-first followed by strict checking can still run inference separately;
successful strict/recovery products also remain separately owned. Parsed-library
template reuse and broader immutable-stage sharing remain open.

## Shared checked products and query-order implementation batch

Implemented the next slice of order 7 on 2026-09-07. Successful strict and
recovering queries now share a checked product, including syntax, declaration
HIR, semantics, and effects. Recovery-first queries use the strict checker when
syntax and resolution prerequisites agree, so subsequent strict queries reuse
the same success or failure. Cloning a recovered result shares immutable data.
Syntax/resolution failures retain the existing recovery path.

The new query-order benchmark exposed invalid state-field separators in the
older small/large tooling fixtures. Their commas are now semicolons; previous
"valid" hover measurements reached checked semantics through parser repair.
Both sides of this batch's comparison use the corrected fixtures. Historical
timings remain useful for their original workloads but are not directly
comparable with these corrected inputs.

Valid strict/recovery pairs improved 32.1–35.7% in median latency. Retained heap
for both products fell from 5.26 to 4.75 MiB (small) and 17.09 to 14.82 MiB
(500 helpers). Recovery-first error checks also avoid a second inference pass.
Recovery alone has a memory tradeoff: successful results now retain the complete
checked program, increasing retained heap from 2.42 to 4.75 MiB and from 11.05
to 14.82 MiB respectively. The ordinary successful diagnostics/hover path
already uses the strict checked product; these query-pair gains are not a claim
of another equivalent end-to-end LSP improvement.

Seven focused tests cover one inference pass in either order, shared identity,
agreement with independent recovery, warning policy, and old snapshots after
edits. All 47 compiler-query integration tests passed. Seven release fixtures
remain byte-identical. The full `cargo xtask check` passed, including 421 library
tests (one manual benchmark ignored), 608 compiler integration tests,
documentation, editor/browser checks, and Wasm/runtime fixtures. Timing details
are recorded in [baselines](docs/BASELINES.md).

This resolves the preceding batch's outstanding query-order duplication for
compatible inputs. Parsed-library templates and remaining parse/lower stage
copies are still open; no incremental inference or release optimizer was added.

## Parser token ownership and intervening-commit review

Reviewed `57f2564..ac45db0` on 2026-09-07, including ordinary conditional
state, insertion-ordered maps, collection completion, catalog patterns, and
iterator display specialization. Reconstructed `57f2564` in an ignored source
snapshot and compared unchanged compiler fixtures with `ac45db0`. Minimal,
cancellation, and settings compile medians increased 5.5–8.6%; frontend medians
increased 9.7–11.1%. This is evidence of aggregate overhead, not attribution to
one commit. New map methods extend the bundled source that every compilation
parses and checks, reinforcing order 8's priority. Lunistice's source changed
with the state migration and is excluded from that fixed-input comparison.

Seven unchanged release fixtures have identical non-custom Wasm sections across
those commits. The archived build lacks Git identity metadata, so its raw files
are 39 bytes smaller; that is a measurement artifact, not generated-code growth.
New map behavior and the migrated Lunistice fixture have separate coverage.

Implemented a smaller parser improvement first: `TokenCursor` now borrows the
preceding token from its existing vector instead of cloning every consumed
token, including owned identifier and literal text. Only contextual operator
splitting retains a synthesized previous token. Failed contextual probes also
borrow the current token. This benefits ordinary and privileged catalog parsing
without changing identities, spans, lexical rules, or recovery behavior.

Focused tests cover borrowed token identity, independent cloned cursor history,
failed probes, split/reassembled operators, subsequent advancement, and EOF.
Nine current release fixtures, including map runtime and Lunistice, remain
byte-identical. Measurements and full validation are recorded in
[baselines](docs/BASELINES.md).

Compiler medians improved 4.2–6.9%, with 9.6–12.9% frontend improvements.
The new `tooling_baseline --stages` mode measures edit-to-parse/lower/check
cumulatively; paired checks improved 6.0–8.5% in that harness. Whole editor
diagnostics/hover timings changed direction between the original and extended
harness builds, so this is not a stable end-to-end LSP speedup claim. Native
server and embedded-worker measurements remain open. Retained heap is unchanged.

Validation: all 98 syntax tests and the full `cargo xtask check` passed,
including 418 compiler-library tests (one manual benchmark ignored), 617
compiler integration tests, documentation, editor/browser workers, and
Wasm/runtime checks for the current map and conditional-state behavior.

The newer catalog materialization paths also contain linear type-table searches
for existing arrays/applications and application-layout arguments. Their scaling
on large generic/map workloads remains an investigation target; no isolated
regression or optimization claim is established for those searches yet.

## Shared augmented syntax implementation batch

Implemented another slice of order 7 after `88378b2`. Lowered and checked
programs now share the immutable compilation syntax, including injected
standard-library bodies. Cloning a lowered program for a database check no
longer duplicates that tree. This is sharing within one source revision, not
a parsed-library cache across unrelated compilations: every revision still
parses, resolves, and validates its own library bodies and identities.

Lowering also waits until augmentation is complete before cloning user syntax
for the no-augmentation fallback. The usual augmented path no longer creates
and immediately discards that copy. Recovery without strict syntax still
retains its existing separate user tree and never promotes it to strict codegen.

Tests cover shared identity across strict/recovery query orders, cloned lowered
programs with and without injected library bodies, source-revision isolation,
and unchanged code generation from old snapshots after an edit. Measurements
and full validation results are recorded in [baselines](docs/BASELINES.md).

Retained heap for successful strict/recovery query pairs drops by 1.09 MiB
(21.9%) on a small document and 1.77 MiB (11.7%) with 500 helpers. Failed checks
mainly benefit in peak heap. Initial timing runs had substantial drift; a
closer repeated comparison is approximately at parity. No large latency
speedup is claimed for this ownership change.

Validation: nine focused checking tests, all 47 compiler-query integration
tests, and the full `cargo xtask check` passed. The full run includes 420
compiler-library tests (one manual benchmark ignored), 617 compiler integration
tests, documentation, editor/browser workers, and Wasm/runtime checks. Nine
release fixture modules remain byte-identical.

User syntax, source documents, declaration HIR, and resolution tables still
have additional stage copies. Parsed-library reuse remains the larger fixed-cost
opportunity; this batch does not introduce incremental inference or skip checks.

## Async dispatcher implementation batch

Replaced repeated frame-PC comparisons with structured `br_table` dispatch for
async bodies with 2–128 states, in both debug and release. This is direct
emission without a new optimization pass. Single-state bodies retain the smaller
linear form; unusually large machines retain shallow dispatch to bound nesting.
The shared body emitter derives branch depths from the selected layout, including
retry, suspension, and loop break/continue targets.

Release Lunistice shrinks from 34,500 to 33,439 bytes (1,061 bytes, 3.1%). The
Mono managed-instance fixture saves 1,154 bytes (4.3%), the other managed-instance
fixture saves 502 bytes (2.9%), and cancellation saves 68 bytes (2.4%). Five
other release controls remain byte-identical. These savings are in the code
section. Compiler latency and validation are recorded in
[baselines](docs/BASELINES.md); no compiler speedup is assumed from smaller output.

Validation: `cargo xtask check` passed, including all 619 compiler integration
tests, editor/browser tests, Wasm validation, and runtime fixtures. New coverage
checks table boundaries and executes retry, nested futures, break/continue, and
loop exhaustion in both profiles.

## Binaryen reference study and revised next steps

After `72a6a15`, updated the local Binaryen installation to 132 and compared
eight release modules with rewrite-only, instruction peephole, O4, Oz, and a
combined closed-world/convergence pipeline. The full investigation and commands
are in [BINARYEN_OPTIMIZATION_REVIEW.md](BINARYEN_OPTIMIZATION_REVIEW.md).
Binaryen remains an offline reference, with no compiler or build dependency.

The next output-size work should start with two shared emission fixes:

1. Avoid completion tails after unconditional branches/returns, using explicit
   fallthrough information in the existing emitter.
2. Avoid redundant `ref.as_non_null` assertions at known GC consumers, preserving
   required non-null types and trap ordering across effects.

Binaryen's reader/writer plus its instruction peephole pass reduces Lunistice
from 33,439 to 29,418 bytes without changing its defined-function count. The
remaining targets are tighter GC type planning, then bounded release constant
propagation, branch cleanup/factoring, and size-budgeted inlining. Plain inlining
in isolation enlarged all three measured large fixtures; it is not the first
implementation target. Keep compiler/editor latency work, particularly the
standard-library frontend floor, alongside these output improvements.

Validation: 48 modules validated and 78 runtime scenarios matched their original
traces. A separate 75-output single-pass sweep was validated for size attribution.
The study includes caveats about the combined closed-world result, optimization
cost, type identity, and the Windows Node runtime configuration used for testing.

## Async emission cleanup implementation batch

Implemented the first output-size slice from the Binaryen study after `6a658a7`.
Async emission now reports whether a block may fall through, using the control
flow it already emits. State completion tails and loop-back branches are omitted
after unconditional transfers, including nested if/match/fallback branches. Real
fallthrough paths still complete normally. This adds no separate optimization
pass and uses the same implementation in debug and release.

Also removed repeated null assertions from named-function, closure, and leaf
future poll frame loads: their signatures already declare non-null parameters.
The `NonNullLocal` frame source makes that invariant explicit. Nullable global
frames retain their checks. This is the first, type-proven subset of the null
assertion work; consumer-based removal with trap/effect ordering remains open.

Release sizes fall from 33,439 to 30,784 bytes for Lunistice, 48,773 to 45,434
for Minish Cap, 25,454 to 22,870 for Mono managed instances, 16,867 to 15,536
for managed instances, and 2,727 to 2,706 for cancellation. Four controls remain
byte-identical. All savings are in the code section. Detailed attribution and
validation are recorded in [baselines](docs/BASELINES.md).

Remaining work: safe null-check removal at unary GC consumers, ordinary-body
terminal fallbacks, and the later type-planning/release passes from the study.
Do not interpret the smaller output as a measured compiler latency improvement.

Validation: the full `cargo xtask check` passed, including all 621 compiler
integration tests, editor/browser workers, Wasm validation, and runtime fixtures.
Thirteen additional before/after release scenarios matched their original traces.
New regressions cover nested return/fallthrough behavior in both profiles, dead
dispatcher tails, and non-null poll-frame signatures and loads.

## Unary GC read cleanup implementation batch

After `d332a14`, removed 157 direct-emission assertions immediately before
`struct.get`, packed `struct.get_u`, and `array.len` across expression, async,
collection, provider, and runtime-helper emission. These unary instructions
already accept nullable receivers and trap on null; no operand evaluation can
intervene between the removed assertion and the read. The codegen module now
documents that contract. Both profiles share the change, with no extra pass.

Checks needed by non-null argument/result types and checks before later write
or index operands remain. Assertions hidden inside reusable reference-load
helpers also remain; changing those requires reviewing all consumer contracts.

Release output shrinks by 219 bytes for Lunistice (30,784 → 30,565), 321 bytes
for Minish Cap (45,434 → 45,113), and 2–123 bytes for each of the other seven
measured fixtures. All savings are in the code section. Comparing complete
release instruction listings confirms that only adjacent unary null assertions
were removed: receivers, subsequent operands, calls, and branches are unchanged.
All nine modules validate and all thirteen release runtime traces match.
See [baselines](docs/BASELINES.md) for sizes, timing limits, and full validation.
The full `cargo xtask check` passed, including all 621 compiler integration tests,
editor/browser workers, embedded Wasm compiler, and host-runtime fixtures.

Next opportunities remain consumer contracts in shared load helpers, ordinary
function terminal fallbacks, and measured GC type planning. Compiler latency
work should continue to target repeated checking/augmentation rather than RAM.

## Distribution profile and deferred field-read experiment

Added the user-requested Rust `max-opt` profile, inheriting release with full
LTO, one code generation unit, aborting panics, and symbol stripping. The main
release profile retains Cargo defaults. Production VSIX builds and the CI
verification matrix use `max-opt`; development extension builds use release.
The matrix builds both native executables and validates the native and embedded
distribution compilers. Benchmark commands can select either Rust profile,
independently of the generated script's debug/release profile.

The retained profile reduces the native CLI from 10.82 to 6.30 MB and the
embedded compiler from 8.88 to 6.35 MB. Native benchmark medians are roughly
equal; the embedded sample is modestly faster. All nine script artifacts are
byte-identical between the Rust profiles. Full `cargo xtask check` passed.

The next typed-field cleanup was measured but **not retained**. Removing 68
adjacent unary assertions saved 146 bytes in Lunistice, 252 in Minish Cap, and
186 in Mono, with matching validation and runtime traces. However, the ordinary
release compiler ran roughly 7–9% slower across repeated runs and a freshly
rebuilt baseline. The frontend-only path also slowed down, so the underlying
native-code cause is not isolated. A same-profile `max-opt` comparison instead
improved, but that does not resolve the ordinary release regression. Keep the
existing code until this tradeoff is understood; compiler latency is higher
priority than these small additional script-size savings.

All experimental measurements, including the single-codegen-unit probe, are
recorded in [baselines](docs/BASELINES.md). Continue with the larger parsing and
checking reuse opportunities in order 8; frame-load contracts and typed unary
cleanup remain candidates rather than completed work.

## Parser prefix-scan removal

The next latency investigation found that every block entry and recovery start
rescanned all preceding tokens to reconstruct brace depth. This made parsing
quadratic for files with many blocks, including the augmented library source.
The shared token cursor now tracks depth while consuming tokens; recovery reads
the same counter instead of maintaining separate copies. Contextual operator
splitting, cloned cursors, documentation, stray closers, and EOF preserve the
existing behavior. No syntax template cache or profile-specific parser is needed.

An isolated 4,000-function parse falls from 78.8 to 7.9 ms in Rust release and
77.6 to 7.4 ms in `max-opt`, with roughly linear scaling after the change.
Real one-shot compilation medians improve in both Rust profiles, including a
reverse-order repeat; see [baselines](docs/BASELINES.md) for the full tables.
The allocator-instrumented editor harness has mixed results, so these compiler
numbers alone should not be presented as an across-the-board LSP improvement.
Follow-up measurements of the actual `max-opt` language server over stdio improve
edit-to-diagnostics medians on all three fixtures in both run orders: the first
run changes small/Lunistice/large from 44.5/50.9/75.9 to 41.6/47.0/69.3 ms.
The embedded compiler also improves: Lunistice is 65.9 → 50.8 ms, confirmed at
65.9 → 52.0 ms in reverse order. These measure warm compilation, not startup.

All nine generated release fixtures remain byte-identical. Debug executable
sections and line tables also match. Comparison exposed existing nondeterministic
DWARF variable ordering: the old compiler reproduces the same alternate debug
artifacts. Track that independently of this latency change.
Full `cargo xtask check` passed, including editor/browser workers, Wasm
validation, and all 95 runtime scenarios.

Continue profiling the remaining frontend floor before implementing parsed
library templates. Prefix-scan removal reduces avoidable parsing work but does
not avoid reparsing augmented source, and does not complete order 8.

## Expression delimiter-scan removal

The follow-up to `21da5b6` found a second prefix scan in ordinary parenthesized
expressions, calls, and array argument lists. Expression lists repeatedly
reconstructed all three delimiter counts, even on valid input. The cursor now
maintains parentheses, brackets, and braces together; expressions retain a
depth snapshot for root recovery. The parser no longer has either prefix-scan
helper or separate advancing recovery counters. Forward lookahead still uses
a copied depth without consuming tokens. Both script profiles share this code.

The call-heavy 4,000-function parser benchmark falls from 1.38 seconds to
10.2 ms in Rust release and 1.75 seconds to 9.9 ms in `max-opt`. Full compilation
improves by roughly 44–53% across the four fixtures and two run orders.
The frontend baseline (parse, augmentation, resolution, and disposal) falls
from 22–32 ms to 2.2–2.7 ms. All nine release outputs remain byte-identical;
40 malformed-source diagnostic comparisons also match exactly. Measurements
are recorded in [baselines](docs/BASELINES.md).

The actual `max-opt` LSP improves edit-to-diagnostics medians from
41.2/46.7/71.5 to 21.6/22.3/45.4 ms (small/Lunistice/generated large).
The embedded compiler improves from 43.2/51.7 to 20.3/24.1 ms
(minimal/Lunistice). Reverse-order runs confirm both results. These are warm
workloads, with startup excluded. Full `cargo xtask check` passed, including
editor/browser workers, Wasm validation, and all 95 runtime scenarios.

Reprioritize order 8 accordingly: a complex parsed-template cache can now save
only a fraction of a roughly 2–3 ms frontend budget in these fixtures. Profile
checking, specialization, and emission before introducing syntax-ID remapping
or larger caches. Reuse of checked library facts remains a candidate, subject
to the existing semantic requirements; the old parsing-floor measurements no
longer justify implementing parsed-library templates first.

## Capability inheritance indexing

The next analysis probe found roughly 12 ms in function-body inference, 3 ms
in typed-HIR construction, and 5 ms in validation on a warm minimal compile.
Dependency ordering and generic generalization were comparatively small.
These internal timings were temporary; the reusable `compiler_baseline
--stages` mode now separates the public analysis, Wasm-lowering, and encoding
phases without adding production instrumentation.

Capability implication previously allocated traversal containers and walked
the same immutable catalog hierarchy on each constraint query. The shared
standard-library graph now indexes transitive implication once. Both script
profiles and all compiler consumers use the same query. Existing hierarchy
tests also cover longer transitive paths, unrelated capabilities, direction,
reflexivity, and constraint reduction order.

Controlled native runs put the analysis saving at about 1.2 ms (5–6%); full
compilation improves in both Rust profiles and both run orders. Nine release
outputs remain byte-identical. Detailed measurements and limitations are in
[baselines](docs/BASELINES.md). This is a compiler latency change; it does not
reduce generated script Wasm size.

Full `cargo xtask check` passed, including the editor/browser workers and all
95 runtime scenarios. The packaged native and embedded compilers grow by about
6 KiB each; generated scripts are unchanged.

Actual `max-opt` LSP edit-to-diagnostics medians improve by roughly 1–2 ms
across the three fixtures, with the same direction in reverse-order runs.
The embedded compiler also improves modestly in median, while its p95 remains
noisy. Both service comparisons read identical sources.

Continue profiling inference and validation. In particular, the associated-type
solver still revisits all earlier projections after each ordinary unification,
including a nested scan to discover equal receivers. Measure that work before
introducing a worklist or dependency index; preserve delayed errors and generic
signature instantiation. Keep the remaining output-size work separate from
these inference changes.

## Source-path suggestion indexing

Revisited on 2026-09-12 against `d741ef4`, after the process-reader, pointer
width, recovery, and Debug changes. The earlier optimizations remain present.
The associated-type solver probe measured only about 0.35 ms and four retained
projections on the minimal fixture, so a new solver is not the next priority.
Expression self-timings instead put about 8.6 ms in calls, excluding the time
spent checking their argument expressions.

Ordinary path-based method calls ask for function-name suggestions before
resolving their receiver. That query scanned every public catalog item and
allocated each item's complete source path, even when the receiver's name had
no matching library scope. The immutable library graph now indexes public
items by their exact parent path. Suggestions borrow candidate names from
that scope; root-level user functions and wrapper constructors are still added
at query time. Visibility, catalog order, ambiguity handling, and lookup
precedence are preserved. Source-path construction is shared with the existing
public `item_path` API, and both script profiles use the same implementation.

Full compilation medians fall by 27–40% across four fixtures, both Rust
profiles, and both run orders. Analysis saves roughly 8–11 ms while lowering
and encoding remain nearly unchanged. All nine release script outputs are
byte-identical to the current baseline. Full `cargo xtask check` passed,
including browser-host checks and all 96 runtime scenarios.
Actual `max-opt` LSP edit-to-diagnostics medians improve by roughly 7–10 ms
across small, Lunistice, and 500-function fixtures in both run orders.
See [baselines](docs/BASELINES.md) for measurements and validation.
Temporary profiling code has been removed. Further latency work should profile
the remaining analysis and validation cost rather than assume associated-type
solving dominates. Generated script size remains a separate work item.

## Evidence and scope

There are three different performance concerns:

- Time spent executing SplitScript compilation and editor queries.
- Size and runtime cost of the **generated script module**, especially with
  `--profile release`.
- Size/startup of the compiler's own Wasm binary embedded in VS Code. This is a
  separate artifact and a secondary concern for this plan.

`cargo --release` optimizes the Rust compiler executable; SplitScript's
`--profile release` selects generated-code behavior. Measurements must identify
both. Rust Cargo profile settings do not optimize the script module's emitted
instructions.

### Existing latency measurements

The working-tree [baselines](docs/BASELINES.md) record release compilation
medians of 45.68 ms for minimal, 54.44 ms for Lunistice, 46.68 ms for cancellation,
and 49.58 ms for settings. Their earlier stage probe attributes about 24.9 ms to
parsing augmented standard-library source and only 0.067 ms to exact resolution.
These are prior measurements, not newly timed results from this review.

The same document records member-completion medians of 98.2 ms for the small
fixture, 153.0 ms for Lunistice, and 199.2 ms for the generated large fixture.
Even the unchanged-revision multi-query sequence takes 49.1/157.3/234.8 ms.
Those editor measurements are labelled `70998d4`, before the working-tree cache
change. Repeat them before assigning savings to another change. Separate valid,
syntax-invalid, and type-invalid fixtures: fast diagnostics that stop early
are not evidence of a fast successful semantic check.

### Fresh release-output inspection

Rebuilt `splitc` using `cargo build --release --bin splitc`, then compiled the
following with `--profile release`. Sizes below are section **payload** bytes;
the total also includes the module header and section envelopes.

| Fixture | Total bytes | Code | Types | Data |
| --- | ---: | ---: | ---: | ---: |
| `examples/lunistice.split` | 34,830 | 31,165 | 1,848 | 990 |
| `examples/minish_cap.split` | 49,307 | 43,200 | 1,691 | 2,991 |
| `tests/set_runtime.split` | 3,187 | 2,120 | 569 | 58 |

All three validate with `wasm-tools validate --features all`. None contains a
name or DWARF section. Each has a 160-byte `splitscript` custom section including
its envelope; removing that identity metadata would have little impact and is
not recommended. Debug companions were used for function-name attribution,
without adding names to release output.

Lunistice's two largest function bodies are the generated provider preparation
poller (11,595 bytes) and `UnityIl2Cpp` poller (6,806 bytes), together about 59%
of its code payload. Their body bytes match the named debug companions exactly.
Minish Cap's largest release body is 27,826 bytes; the debug companion identifies
a GBA discovery poller of the same size and corresponding signature. Its indices
differ between profiles, so this is supporting attribution rather than an exact
body-byte match. Large provider pollers deserve investigation, but their size
alone does not prove that their work is redundant.

The July baseline uses a different compiler and generated-code profile. Do not
treat “under 30 KB” as an established regression boundary or promise that the
small fixes below will restore that size. No history bisect is needed.

## Suggested delivery order

Each row should be a separately reviewable change. Effort estimates are relative
scope, not time commitments. Re-measure after each group before proceeding.

| Order | Work | Benefit | Scope / confidence |
| --- | --- | --- | --- |
| 1 | Add focused timing counters and output attribution | Makes improvements measurable | Small; measurement foundation |
| 2 | Cache fixed validation; index function/body lookup; avoid release debug-name work | Compiler throughput | Small; repeated work confirmed, savings unmeasured |
| 3 | Group local declarations and intern ordinary function signatures | Smaller release modules | Small to medium; duplication measured |
| 4 | Make set operations individually reachable | Smaller modules and less emission | Small to medium; unused bodies demonstrated |
| 5 | Reuse receiver facts and bounded completion probes | Warm editor latency | Medium; repeated semantic work confirmed |
| 6 | Group specialization expressions by owner | Compiler scaling | Small to medium; full-map scan confirmed |
| 7 | Share immutable stage products and failed-check results | Editor latency and allocations | Medium; repeated copying/checking confirmed |
| 8 | Cache parsed standard-library templates, then reusable semantics | Largest fixed-cost opportunity | Medium to large; ID/span design required |
| 9 | Investigate large async/provider code and lazy body lowering | Compile time, module size, runtime | Medium to large; savings require experiments |
| Later | Local release optimization passes, selective inlining, fine-grained incremental checking | Further improvements | Defer until the above is measured |

## 1. Measure the work that will actually change

Extend [compiler_baseline](examples/compiler_baseline.rs) with optional stage
timings or internal counters for user parse, augmentation/render/lex/parse,
resolution, checking, typed HIR/validation, Wasm lowering, specialization,
reachability/planning, and encoding. The public stages alone combine several of
these costs. Count user/library expressions, checked functions, lowered bodies,
concrete function instances, and synthesized completion databases.

Extend [tooling_baseline](examples/tooling_baseline.rs) with isolated repeated
member completion at the same revision/offset, trailing-dot and complete-field
requests, and type-error recovery. Measure actual body edits as well as its
current appended-comment edit. Record native and embedded-worker latency
separately, including process/worker cold initialization. The process-wide
standard-library bootstrap is intentionally excluded by the current warm setup.

Add a reproducible size report using the existing `wasmparser` development
dependency: section bytes, function body sizes with compiler-owned labels,
local declaration groups, exact signature duplicates, emitted helper kinds,
and static data. Keep profiling labels in the report, not in release modules.

Acceptance: sequential before/after runs with identical source, toolchain,
profiles, and warning policy; 200 compiler samples and at least 30 editor
samples after warmup. Do not benchmark while the full test suite is running.
Use counters and semantic assertions in tests, not wall-clock thresholds.

## 2. Remove small, repeated compiler work

These items are implemented in the batches above. The following is the original
review rationale and acceptance criteria.

- [codegen::compile](src/codegen.rs) calls `validate_intrinsic_effects()` and
  `unity_layout::validate()` for every module. The former walks the complete
  intrinsic/helper dependency graph. Cache these immutable descriptor checks
  once, while preserving validation failures and dedicated validation tests.
  Do not cache source-dependent semantic validation this way.
- [check_function_bodies](src/typeck/body_pass.rs) finds each SCC member by
  scanning `program.functions`. Build an ID-to-position index once. Recovery
  leaves holes in IDs, so an unchecked `functions[id.index()]` is incorrect.
  Reuse the same approach for function lookups in backend planning.
- [wasm_ir::Program::body](src/wasm_ir.rs) scans bodies and can scan again for a
  generic template fallback. Index exact owners and template declarations,
  preserving that fallback. Keep deterministic iteration independent of maps.
- [function_plan::declare](src/codegen/function_plan.rs) formats and stores
  debug names even for release, although [module assembly](src/codegen/module_assembly.rs)
  only emits them when debug artifacts exist. Make name construction lazy and
  debug-only. This saves compiler work, not release file bytes.

Acceptance: equivalent diagnostics, recovery behavior, and emitted bytes for
lookup/validation changes, accounting for intentional compiler identity metadata
differences between builds. Record stage savings before pursuing smaller scans.

## 3. Compact the Wasm encoding

### Adjacent local declarations

[script_functions](src/codegen/script_functions.rs) and
[async_state](src/codegen/async_state.rs) repeatedly use
`Function::new(local_types.into_iter().map(|ty| (1, ty)))`. This encodes each
local as a separate declaration group. Combine adjacent locals with exactly
equal Wasm value types; preserve local ordering and indices.

Inspection of the fresh binaries found **213 bytes** of local-declaration
payload savings in Lunistice and **369 bytes** in Minish Cap from this change
alone. These calculations regroup existing declarations, including reference
types; they exclude possible extra savings in enclosing length prefixes. They
are not measured optimized output sizes. A shared helper should also cover
hand-written runtime functions where adjacent groups can be combined.

### Ordinary function signatures

[imports::encode](src/codegen/imports.rs) and
[function_plan::declare](src/codegen/function_plan.rs) append a new type for
each function. Outside the GC recursive group, Lunistice has 78 function-type
entries but only 62 distinct exact signatures; Minish Cap has 86 versus 54.

Share an interner between imports and defined functions, keyed by exact Wasm
parameter/result types, including reference nullability and concrete heap-type
indices. Allocate in deterministic first-use order. Start with ordinary function
types after GC layout assignment; leave recursive groups, subtype contracts,
and nominal GC layout identity untouched. Do not merge two source layouts just
because their printed fields look alike.

Acceptance: validate both profiles; execute closure/function-reference, generic,
async, and GC runtime tests. Local grouping must preserve instructions and
indices; signature sharing may change type indices and must update every
consumer. Report actual whole-module savings without adding overlapping size
estimates together.

## 4. Emit only demanded set operations

Implemented in the set-operation batch above. The following is the original
review evidence and acceptance criteria.

This is a confirmed **Set-specific gap**, not evidence that every runtime helper
is emitted unconditionally. [function_plan](src/codegen/function_plan.rs) declares
all six operations for each reachable set type, and
[set_functions::compile](src/codegen/set_functions.rs) emits all six. Arrays
already distinguish demand for `push`, `removeAt`, and `clear` in
[reachability](src/codegen/reachability.rs); most runtime helpers also follow
explicit dependency roots.

A fresh probe using only `Set.new<u32>()` and `visited.length()` produces a
1,960-byte release module. Its unused `contains`, `insert`, `remove`, and `clear`
bodies occupy 66, 137, 144, and 40 bytes respectively: **387 body bytes** before
function/type entries and length prefixes. Body hashes match the named debug
companion. These are removable candidates, not an implemented saving.

Reproduction source:

```splitscript
state "game.exe" {}
let visited = Set.new<u32>()
whileAttached {
    setVariable("count", visited.length())
}
```

Track demand by concrete set type **and operation**. Expand internal edges such
as `insert -> contains`, plus needed equality/storage helpers, to a fixed point.
Use that plan consistently for declarations and emission. Avoid removing
equality helpers still required by a demanded operation.

Acceptance: new/length-only code contains no mutation/search bodies; insert-only
code retains its contains dependency. Run string, structural-equality, iterator
mutation/version, and existing set runtime coverage. Audit other helper families
with the same report before claiming this problem is widespread.

## 5. Stop rechecking a whole file for warm member completion

The receiver cache, direct receiver selection, and root-effect probe reuse are
implemented in the batches above. The following is the original review;
receiver recovery and constructor-field substitution offer further work.

In [completion::analyze_receiver_database](src/completion.rs), path receivers
prefer a resolved method-call receiver. With a field such as `point.x`, there
may be a perfectly good expression type but no resolved call receiver; `?`
then discards the direct result. `infer_receiver` removes the member suffix and
constructs a fresh `CompilerDatabase`. That probe is not retained across
requests. Recovered receiver facts also force a probe. Root effect completion
has another fresh-database fallback in `completion_operation_analysis`.

First select the receiver expression explicitly and use its known type when
valid, preserving the distinction between `receiver.method` and
`receiver.method().`. Do not simply substitute an enclosing call's result type.
Keep recovery when facts are missing or unreliable. Next memoize a bounded
number of probes/results per source revision, keyed by completion kind, receiver
position, replacement range, and repair. Include failures. Cache owned semantic
facts or retain their owner; never reuse a probe's `TypeId` against another
database's type store.

Acceptance: repeated same-revision field completion performs no new semantic
check when direct facts exist; a necessary probe runs at most once per key.
Test partial calls, chained/optional/result receivers, generics, user methods,
syntax/type errors, edits, and multiple offsets. Preserve results and recovery
quality while reducing the warm-sequence latency in the existing benchmark.

## 6. Make specialization proportional to the functions it visits

[specialization::materialize](src/codegen/specialization.rs) collects an
`ExprId -> owner` map, then scans **the entire map for each reachable function
instance**, rejecting expressions from other owners. The scan component is
approximately `instances * all expressions`, including library expressions.

Build `owner -> ordered expression IDs` and a separate root-expression list in
one traversal. Each specialization then visits only its template's expressions.
Use stable ordering: the current hash-map traversal is not a suitable foundation
for deterministic constructed-type allocation. Preserve nested closure
ownership and capability/overload resolution.

Acceptance: counters show visits proportional to roots plus expressions in
visited instances. Test 50/500/2,000 unrelated helpers and multiple generic
instances, recursive functions, closures, and source-defined providers. Compare
behavior and deterministic output rather than assuming every changed type index
is a semantic regression.

## 7. Share immutable compiler products before adding incremental inference

[database/queries](src/database/queries.rs) clones recovered syntax into strict
parse, clones parsed/lowered programs to consume stage APIs, and clones the
semantic model when building highlights. `semantic_snapshot()` can run strict
checking, lose the semantic output on error, and perform recovering lowering
and checking again. A repaired-source path can add another database.

Start with the cheap borrow: hold the snapshot `Arc` while building highlights
instead of cloning its semantics. Then introduce shared immutable document,
syntax, and lowered products behind the stage APIs. Share ordinary strict and
recovering lowering where their inputs and invariants actually agree.

Unify the checking implementation around an internal result containing partial
semantics plus diagnostics; strict compilation accepts it only when all required
invariants pass. Reuse partial facts for tooling instead of doing the same
inference twice. Keep recovery placeholders out of typed code generation and
keep warning policy separate from semantic-cache invalidation.

Acceptance: unchanged editor results on valid and invalid source; failed strict
checks do not automatically cause duplicate inference; retained-cache and peak
allocation measurements show the ownership change is worthwhile. Use the
existing query/recovery tests and explicitly cover warning-policy changes.

## 8. Reuse standard-library templates across compilations

**Reprioritized September 22:** the former 2.2–2.7 ms frontend baseline has
grown to 6.8–9.4 ms, while checking/validation remain the larger costs. First
address the measured effect-analysis and typed-HIR work described above, then
reassess this prototype. The design and correctness constraints below still
apply; growth in the bundled library makes reuse more relevant again.

The initial working-tree change cached rendered source and name indexes. The
token-reuse batch above also caches lexed library tokens, but still does **not**
cache parsing or checking. In the original
[augmentation path](src/stdlib/library_bodies.rs), each source was concatenated
with every library body, lexed and parsed again, including the
already-parsed user prefix. Checking then processes all injected functions;
[validation](src/validation.rs) verifies their signatures/effects against the
bootstrapped metadata again. [Wasm lowering](src/wasm_ir.rs) also lowers all
non-erased function templates before backend reachability chooses emitted ones.

Stage this larger change:

1. Cache a parsed, immutable library template per validated `StandardLibrary`
   graph. Keep user-dependent provider preparation separate. Prototype assembly
   without reparsing the user prefix or lexing static library text.
2. Establish an explicit remapping boundary for every syntax identity and span:
   functions, expressions, bindings, constructed types, nominal declarations,
   and generated provider declarations. Existing visible-count assumptions and
   user-source diagnostics must remain correct. An internal library source/ID
   domain is a possible design, but does not require public modules.
3. Reuse validated generic signatures, typed templates, and effect facts only
   after separating library-owned identities from each compilation's mutable
   type/inference stores. Materialize user-specific instantiations on demand.

Do not reuse a checked synthetic program's raw IDs in another compilation. Do
not replace repeated lexing with repeated deep copies of owned token text and
assume it is faster; the existing baseline notes explicitly warn about that.
Measure clone/remapping cost before choosing the template representation.

Maintain an exhaustive library-validation path in build/tests and validation
for each injected graph. Source-specific constraints, user capability
implementations, overload selection, and provider/schema preparation must still
be checked. Merely omitting unused source bodies before checking can change
diagnostics and is not an acceptable shortcut.

Acceptance: materially reduce the measured augmentation/checking floor, with
matching diagnostics, editor identities, deterministic output, and runtime
behavior. Exercise multiple graphs/contexts, source lengths, Unicode, malformed
source, generic calls, custom capability methods, and managed providers. Land
parsed-template reuse independently if semantic reuse needs a larger redesign.

## 9. Investigate the large bodies and remaining backend work

The size evidence directs attention to generated provider preparation and
asynchronous discovery. [compile_async_body](src/codegen/async_state.rs) emits a
shared table dispatch for 2–128 states, with the measured savings recorded above.
Measure repeated generated binding logic and polling runtime separately from
the useful provider work.

Factor repeated binding/discovery sequences into shared ordinary helpers where
the encoded call/signature/frame cost is lower. Helper extraction remains an
experiment; it is not yet proven to shrink the observed modules.
Preserve retry ordering, suspension, break/continue, cancellation, attachment
lifetime, and debug source locations. Use Lunistice, Minish Cap, and the async
runtime fixtures as acceptance workloads.

Separately, consider lazy Wasm lowering of templates once the complete checked
program exists. It can avoid building IR for unused library bodies without
skipping source validation. Derive demand from the existing semantic/backend
contracts, including provider attachment/preparation, named function values,
closures, custom Display/Debug, constants, and capability/overload calls. Avoid
creating a second subtly different reachability definition merely to save a
pass. This primarily reduces compiler work; emitted functions already follow
reachability today.

## Deferred work and constraints

- **Release optimization passes:** begin with typed constant folding and dead
  branch cleanup, then tiny wrapper inlining under an explicit size budget.
  Run them after profile erasure and before final dependency/data/type planning,
  or recompute those products. [constant.rs](src/constant.rs) classifies some
  syntax-level constants; it is not a general evaluator. Preserve integer width,
  overflow/traps, floating-point signed zero/NaNs, evaluation order, mutation,
  fallible operations, and async behavior. Effect metadata alone does not prove
  an expression cannot trap. General inlining can enlarge the very pollers that
  dominate current output.
- **Fine-grained incremental checking:** first remove duplicate checks and the
  static library cost. Then measure remaining edit latency before introducing
  function/SCC invalidation. Full-sync text transport alone is not the principal
  measured bottleneck. If queued edits remain a problem, design version-aware
  cancellation/coalescing around the synchronous LSP/worker request path while
  preserving ordering and diagnostics for the newest version.
- **Compiler-in-Wasm packaging:** benchmark the embedded module separately.
  Explore a dedicated Cargo profile, LTO/size settings, and stripping for that
  artifact, measuring startup and compilation latency as well as download size.
  Do not add a script optimizer dependency to solve the compiler binary's size,
  or assume a smaller embedded compiler executes faster.
- **Avoid semantic shortcuts:** settings descriptions, state polling, failure
  values, and cancellation can remain observable even if a user expression does
  not read their result. Existing demand-driven helper, float-table, string, GC,
  and debug-section handling should be preserved. Remove additional work only
  with an explicit dependency/observability argument.

## Completion criteria

For each implemented slice, record before/after stage or query timings and
release size attribution in [docs/BASELINES.md](docs/BASELINES.md). Require
semantic/runtime equivalence, stable editor recovery, bounded caches, and
deterministic output. Run focused tests during development and `cargo xtask check`
before committing compiler/tooling changes. Debug metadata quality remains a
requirement even though only release module size is a primary optimization goal.

The first milestone is orders 1–6: measurable reductions in repeated work and
unneeded encoding, with no general optimizer or incremental inference framework.
Use those results to scope immutable-product sharing and standard-library reuse.

Initial review validation: `cargo xtask check` passed on the accompanying working-tree
caching/indexing changes, including Rust, editor/browser-worker, Wasm validation,
and runtime checks. The separate fresh release inspection above also validated
its three fixture modules. Implementation began after that review; see the
progress section above.
