# Release Wasm size optimization

Release compilation runs a bounded size optimizer after emission. Debug and hot
reload bypass all optimizer scans. The objective is a smaller complete Wasm
file; execution-speed optimization remains the engine's job. Binaryen is not a
compiler dependency.

Direct emission also uses `struct.new_default` for payload-free first enum
variants in both profiles. This skips explicit zero/null operands without
adding an analysis pass. Release string emission chooses passive GC initializer
data by encoded cost instead of using a fixed minimum string length.

## Promoted pipeline

Before each of the two bounded cleanup sweeps, a module-wide analysis removes
unobservable globals and substitutes short, proven constant initial values.
The cleanup sweeps simplify integer instructions, fold adjacent integer
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

## Control cleanup and Celeste follow-up — 2026-10-02

A local-copy propagation trial was rejected: running it early slightly grew
all four original real-script outputs, and moving it after control cleanup
saved only 0 / 4 / 0 / 12 bytes. Collection-fixture gains did not justify that
additional analysis. Binaryen's `--vacuum` instead exposed inexpensive missing
rules in the existing cleanup: empty else arms, discarded global reads, and
general constant-condition branches.

The promoted rules remove an empty `else`, replace a completely empty untyped
`if` with a drop of its condition, and discard unused global reads. They retain
condition calls and traps, global writes and typed block parameters. A literal
condition selects one complete arm; the replacement initially retains the
original typed block label, so branch payloads and depths remain valid. Existing
label cleanup then removes unused labels. This extends the existing bounded
scans, without another pass or any Debug work.

| Real script | Previous master | New Release | Additional saving |
| --- | ---: | ---: | ---: |
| Minish Cap | 32,086 | 31,965 | 121 |
| Lunistice | 28,933 | 28,684 | 249 |
| Celeste external port | 32,364 | 32,275 | 89 |
| Neon White | 4,692 | 4,675 | 17 |
| A Hat in Time | 48,325 | 48,173 | 152 |

The Celeste source is `live_split_celeste_port.split` from the sibling porting
workspace, SHA-256
`B8C09AD7A525B2200F58FE1902D365D11B07A6A2FE4A0467891BD59DA0D9378A`.
Its unoptimized Release is 35,462 bytes: the complete pipeline saves 3,187 bytes
(9.0%). The other primary scripts now save 9.0% / 8.7% relative to pre-optimizer
master. None of the ten measured scripts grows. Celeste's baseline and optimized
modules validate in both wasmparser and Node, and its Debug equivalence check
passes. There is no maintained Celeste behavioral harness in this repository.

Five new runtime tests cover typed parameters and multi-value branch payloads,
truthy non-Boolean constants, outer branch targets, selected and unselected
traps, condition effects, implicit typed else values, non-defaultable local
initialization, and retained global writes. The earlier assignment-factoring
test now uses an opaque local condition so it continues to test factoring,
rather than having the new constant-arm rule erase the conditional first.
All 512 library, 696 compiler, 20 binary and five example tests pass, along with
Clippy, formatting and the browser-target check.
All 176 maintained modules validate, all 211 runtime scenarios and the
Debug/Release profile check pass, and all 36 baseline-versus-optimized corpus
runtime invocations pass. The runner additionally validates both Celeste modules
and reports its missing behavioral harness explicitly.
The unchanged Unity size gate passes, including per-function/type budgets and
Lunistice base/DLC behavior; no baseline refresh was needed.

Optimized-host seven-sample medians, with no concurrent build or runtime suite:

| Primary script | Passes disabled | Complete Release backend |
| --- | ---: | ---: |
| Minish Cap | 3.050 ms | 6.681 ms |
| Lunistice | 16.682 ms | 19.842 ms |
| Celeste | 2.577 ms | 5.293 ms |

These include lowering/emission and exclude parsing/type checking. The totals
remain in the previous pipeline's range; cross-run variation does not isolate
the incremental cost of individual rules. Debug bypasses all these scans.

On the new output, Binaryen's instruction pass saves another 194 / 186 / 412
bytes on Minish Cap / Lunistice / Celeste, after subtracting its no-pass
roundtrip. Local simplification/coalescing saves 160 / 116 / 43, code folding
92 / 114 / 92, and global simplification 60 / 3 / 252. Full `-Oz` reaches
29,653 / 26,354 / 28,882 bytes, leaving gaps of 2,312 / 2,330 / 3,393 bytes.
Instruction simplification, with Celeste included, is the next largest common
opportunity among these measured individual passes; results do not add linearly.
Celeste's instruction diff includes repeated all-zero struct constructors
replaced by `struct.new_default` (eleven six-field and six three-field examples),
108 removed null assertions, and eleven load/widen pairs combined into
`i64.load32_u`. Default struct construction is a concrete candidate for a direct
emitter improvement that could benefit both profiles without adding a pass.

## Default enum emission follow-up — 2026-10-02

For a payload-free first enum variant, the tag is zero and every payload slot
already receives its Wasm default. Emitting `struct.new_default` replaces those
explicit operands and `struct.new`, retaining the same type and fresh allocation.
This is a constant-time choice at the constructor site, skips the old field
emission loop, and applies to Debug as well as Release. Constructors with payload
expressions retain their existing emission, including effects and negative zero.
Contextual optional/result conversions still run outside this emission routine.

| Real script | Previous Release | New Release | Release saving | Debug code-section saving |
| --- | ---: | ---: | ---: | ---: |
| Minish Cap | 31,965 | 31,933 | 32 | 32 |
| Lunistice | 28,684 | 28,660 | 24 | 25 |
| Celeste external port | 32,275 | 32,117 | 158 | 223 |
| Neon White | 4,675 | 4,675 | 0 | 0 |
| A Hat in Time | 48,173 | 47,785 | 388 | 388 |

Debug savings above use executable code rather than varying DWARF metadata.
No measured module grows. Automatic Unity and the two collection fixtures also
save 1,610 / 1,746 / 1,746 Release bytes; the real-script results justify the
change independently. Existing optimization passes can amplify or absorb direct
emission savings, so Debug and Release deltas need not match.

Relative to the original pre-optimizer outputs, cumulative savings are now
3,178 bytes (9.1%) for Minish Cap, 2,753 (8.8%) for Lunistice, and 3,345 (9.4%)
for Celeste. These include the emitter improvement; the current pass-disabled
outputs themselves are smaller than the original baseline.

The added source-level regression runs with both profiles and with optimization
enabled/disabled. It checks mixed and packed payload fields, nonzero tags,
effectful zero payloads, the sign of negative zero, and optional/result wrapping.
All 513 library, 696 compiler, 20 binary and five example tests pass, alongside
Clippy and the browser-target check. The corpus runner passes all 36 maintained
behavioral invocations and validates Celeste, which still lacks a maintained
gameplay harness here. Its external source remains unchanged.

All 176 maintained modules validate, all 211 runtime scenarios and the
Debug/Release profile check pass. The Unity gate required a reviewed baseline
refresh: four automatic-profile fixtures gain four shared IL2CPP helpers and
lose one shared Mono helper, adding three function/type entries (20 type-section
bytes and six function-section bytes). The two Mono Linux build bodies change
from 28-byte wrappers plus a 98-byte shared helper to 95/105-byte direct bodies,
a local 46-byte cost. The changed constructor shapes enable different sharing
groups; no new runtime dependency, scratch memory or source fixture is retained.
Against the immediately previous compiler, the three automatic-profile/metadata
fixtures shrink by 1,592 bytes each and automatic Lunistice by 1,628 bytes.
The baseline is refreshed to the measured output, tightening the complete-module
budgets as well. Its recheck and Lunistice base/DLC behavioral checks pass.

Binaryen `-Oz` still produces 29,653 / 26,354 / 28,882 bytes for Minish Cap /
Lunistice / Celeste, leaving gaps of 2,280 / 2,306 / 3,235 bytes. Instruction
simplification now saves 162 / 162 / 254 bytes relative to Binaryen's no-pass
roundtrip. Widened loads remain a possible direct-emission follow-up. No new
compiler timing claim is made for this shortcut; it introduces no optimizer scan.

## Global cleanup and pass attribution — 2026-10-02

Looking only at isolated instruction passes underestimated the remaining
opportunities. Binaryen 132's `src/passes/pass.cpp`, `SimplifyGlobals.cpp`, and
`Inlining.cpp` show how its optimizing global/inlining passes rerun function
cleanup after exposing new opportunities. A fresh experiment replayed every
prefix of the open-world `-Oz` pipeline in one process, tested each standalone
pass with the same optimization/shrink settings, and disabled major pass families
within the full pipeline. Every resulting module validated in Node; each complete
replayed pipeline matched `-Oz` in size.

On master `40dd52a`, disabling these families increased `-Oz` output by:

| Family disabled | Minish Cap | Lunistice | Celeste | A Hat in Time |
| --- | ---: | ---: | ---: | ---: |
| Local simplification, reuse and common expressions | 2,646 | 3,356 | 1,061 | 1,856 |
| Inlining with cleanup | 504 | 898 | 571 | 533 |
| Global simplification and ordering | 401 | 38 | 1,462 | 3,711 |
| Branch/control simplification and folding | 533 | 848 | 826 | 933 |
| GC/reference optimizations | 183 | -49 | 259 | 21 |

These are interacting pipeline dependencies, not additive savings estimates for
our compiler. In particular, disabling local cleanup also affects cleanup after
inlining. Standalone optimizing inlining saved 1,396 / 1,820 / 1,466 bytes on
Minish Cap / Lunistice / Celeste; much of that includes general function cleanup.
The earlier narrow inlining prototype's small gain is not an upper bound.

Global cleanup was selected for its large measured Celeste/A Hat in Time benefit
and comparatively small implementation. The new Release analysis counts all
reads and checks every write against the global's literal initializer. Private,
unread globals can be removed; globals that only ever hold their initial value
can also be removed when replacing reads does not increase instruction size.
Deleted stores become `drop`, preserving evaluation, calls and traps. Existing
instruction/control cleanup then removes redundant constants and dead branches.
Another bounded sweep can discover globals made unread by that cleanup.

A concrete source is settings storage: the emitter reserves both current and
previous values even when no code reads the previous value. Global cleanup
removes unused storage and maintenance writes without changing host settings
calls. Global counts fall from 197 to 105 in Celeste, 461 to 245 in A Hat in
Time, 84 to 52 in Minish Cap, and 20 to 17 in Lunistice.

Imports, exports, shared globals and references in module initializers/offsets
remain pinned. Unknown global-reference instructions are pinned conservatively.
Nonliteral or allocating initializers stay intact. Floating constants are compared
by bits; there is no floating-point arithmetic folding. Surviving indices are
remapped through the Wasm reencoder, including module-level references. Function
indices and report metadata remain unchanged. Each rewrite must shrink the whole
module; Debug does not execute any of this analysis.

| Real script | Previous master | New Release | Additional saving | Binaryen `-Oz` | Remaining gap |
| --- | ---: | ---: | ---: | ---: | ---: |
| Minish Cap | 31,933 | 31,532 | 401 | 29,653 | 1,879 |
| Lunistice | 28,660 | 28,632 | 28 | 26,354 | 2,278 |
| Celeste external port | 32,117 | 30,648 | 1,469 | 28,882 | 1,766 |
| Neon White | 4,675 | 4,664 | 11 | — | — |
| A Hat in Time | 47,785 | 44,120 | 3,665 | 42,014 | 2,106 |

The ten-fixture corpus has no growth, and Debug executable/stable metadata remains
identical with optimization enabled or disabled. The external Celeste source is
unchanged; it receives compilation and validation, not a gameplay test. All 36
maintained baseline/optimized runtime invocations pass. Focused tests cover calls
and traps from removed stores, imported/exported globals, mutations between calls,
negative zero/NaN bits, large literals, and global initializer/data-offset remapping.
The Debug-profile compiler regression now allows additional internal globals to
be eliminated; it still explicitly checks that the debug-only binding was erased
from Release lowering.

All 517 library, 696 compiler, 20 binary and five example tests pass, alongside
Clippy, formatting and the browser-target check. All 176 maintained modules
validate and all 211 runtime scenarios plus the Debug/Release profile check pass.
The unchanged Unity size gate and Lunistice base/DLC behavior pass; no baseline
refresh is needed.

Optimized-host seven-sample medians, with no concurrent build or runtime suite:

| Primary script | Passes disabled | Complete Release backend |
| --- | ---: | ---: |
| Minish Cap | 2.971 ms | 7.616 ms |
| Lunistice | 16.579 ms | 20.688 ms |
| Celeste | 2.614 ms | 6.269 ms |

A Hat in Time measures 4.450 -> 9.336 ms. These include lowering/emission and
exclude parsing/type checking. The primary-script totals are approximately
0.85–0.98 ms above the earlier control-cleanup measurements; cross-run differences
do not isolate this pass's cost. Large collection timing was noisy and is not
used to infer overhead. The byte gains justify the added Release work.

Rerunning Binaryen attribution on this output leaves only 11 / 11 / 0 / 53 bytes
of benefit from its global family on Minish Cap / Lunistice / Celeste / A Hat in
Time. This addresses the measured opportunity. The next substantial direction is
local/control simplification that also enables profitable inlining, especially for
Lunistice; widened loads are a much smaller priority.

Reproduce the attribution with a generated size corpus (optionally including the
external Celeste manifest):

```powershell
node scripts/binaryen-pass-attribution.mjs C:/Projekte/binaryen/bin/wasm-opt.exe
```

This writes per-prefix, standalone and disabled-family modules plus `report.json`
under `target/binaryen-attribution`. Binaryen is an offline reference tool only.

## Conditional expressions and fallthrough — 2026-10-02

Further Binaryen source/output inspection separated the effects of inlining
from the cleanup it triggers. On `1091435`, standalone plain inlining grew
Minish Cap / Lunistice / Celeste to 33,034 / 31,225 / 31,290 bytes, whereas
optimizing inlining reached 30,101 / 26,718 / 29,139. Restricting the latter to
single-caller candidates (plus Binaryen's trivial-wrapper rule) retained almost
all its savings. The missing ingredient is still cleanup around expanded code.

Two local experiments were not promoted:

- Retrying the earlier inliner against all defined functions, including generated
  helpers, with the current local/control cleanup saved only 167 / 95 / 115 bytes
  on the primary scripts. Trying all call sites improved Celeste by only another
  six bytes. That gain does not yet justify importing the inlining machinery.
- Inferring integer arguments identical at every direct call saved only
  7 / 8 / 7 bytes on the primary scripts. Larger automatic-Unity and collection
  savings did not justify another whole-module analysis.

The selected change extends structured control cleanup. Short, closed integer
expressions and reference reads can replace an `if`/`else` with a `select` when
both arms are nontrapping and effect-free. Arm and condition scans are bounded;
condition writes to an arm's locals prevent reordering. Calls, stores, loads,
division, casts that can trap, and allocations are not speculated. Reference
results use typed `select`. A separate three-byte gain from recognizing calls
inside conditions was discarded rather than adding call metadata for it.

Fallthrough cleanup removes a branch/return only when the immediately following
block/function ends reach the same destination. It uses wasmparser's instruction
arities and tracks structured operand-stack heights. Every crossed frame must
have the same stack base and exact result types; equal arity alone is insufficient
for GC reference subtypes. Conditional branches become a drop of the evaluated
condition. Loop backedges, branches that discard extra operands, and unsupported
exception/continuation control remain intact. This adds a Release-only body
analysis inside the existing bounded cleanup, with the existing body/module size
gates. It does not change function indices, signatures or Debug emission.

| Real script | Previous master | New Release | Additional saving | Binaryen `-Oz` | Remaining gap |
| --- | ---: | ---: | ---: | ---: | ---: |
| Minish Cap | 31,532 | 31,477 | 55 | 29,651 | 1,826 |
| Lunistice | 28,632 | 28,530 | 102 | 26,351 | 2,179 |
| Celeste external port | 30,648 | 30,502 | 146 | 28,882 | 1,620 |
| Neon White | 4,664 | 4,646 | 18 | 3,936 | 710 |
| A Hat in Time | 44,120 | 43,943 | 177 | 42,014 | 1,929 |

No measured fixture grows. Conditional-expression selection alone accounts for
26 / 50 / 108 / 116 bytes on Minish Cap / Lunistice / Celeste / A Hat in Time;
fallthrough cleanup provides the remaining gains. This is a smaller incremental
step than globals, and the measurements do not claim it closes the inlining gap.
The wider inliner and constant-argument trial are preserved only as local
experiments under `target`, not enabled compiler passes.

Six new runtime tests cover conditional arithmetic, mutations in the condition,
unselected calls and traps, nullable reference selections, branch conditions and
effects, discarded stack operands, loop backedges, typed block parameters,
multiple results, and incompatible intermediate reference-result types.
All 523 library, 696 compiler, 20 binary and five example tests pass. The corpus
passes its Debug equivalence checks and all 36 maintained runtime invocations;
the external Celeste port validates and remains unchanged, without a maintained
gameplay harness here.

All 176 maintained modules validate, all 211 runtime scenarios and the
Debug/Release profile check pass, as do Clippy, the browser compiler check and
the Unity size gate including Lunistice base/DLC behavior. No Unity baseline
refresh was needed. The optimized host reproduces identical sizes for all ten
corpus fixtures.

Seven warmed, alternating optimized-host samples measured these backend medians:

| Real script | Passes disabled | Complete Release backend |
| --- | ---: | ---: |
| Minish Cap | 2.986 ms | 8.335 ms |
| Lunistice | 16.096 ms | 21.148 ms |
| Celeste | 4.851 ms | 11.682 ms |
| Neon White | 0.517 ms | 1.423 ms |
| A Hat in Time | 4.629 ms | 10.575 ms |

These include lowering/emission and exclude parsing/type checking. They measure
the entire pipeline, not the isolated cost of this change. Cross-run timing
variation, particularly in Celeste's passes-disabled baseline, prevents treating
differences from the previous measurement as this pass's cost. Debug still
bypasses all optimization passes.

A separate diagnostic Binaryen run with `--closed-world -Oz --converge` reached
28,834 / 24,196 / 28,297 / 40,104 bytes for Minish Cap / Lunistice / Celeste /
A Hat in Time. These reference modules validate in Node but were not run through
the behavioral harness. They are not the ordinary `-Oz` comparison in the table
above. The larger reduction, especially in Lunistice, motivates further study
of interacting type/call simplification and repeated cleanup; it does not imply
that a single missing pass will achieve it.

## Cost-based GC string literals — 2026-10-02

Function-level Binaryen inspection exposed another direct-emission opportunity:
GC strings shorter than 32 bytes still used one `i32.const` per UTF-8 byte,
followed by `array.new_fixed`. Many metadata and display strings are shorter
than that cutoff but substantially cheaper as passive data. For example, each
ASCII byte at or above 64 needs a two-byte signed LEB operand in addition to the
constant opcode, whereas passive data stores the byte once.

Release emission now compares the two encodings as each literal is emitted.
The comparison includes constant operands, segment indices, newly stored bytes,
segment headers, the DataCount section, and a conservative allowance for growth
of the data-section size prefix. Existing pooled bytes are reused. Only a
strictly smaller estimate is accepted; empty and tiny literals stay inline.
This extends the existing literal pool without adding a module pass or a
Binaryen dependency. Each use still allocates a fresh GC array, and passive
initializer data remains available across calls and suspension. It does not
occupy linear memory. Debug retains its existing emission and hot-reload behavior.

Measured against master `3b849bb`, with the same real-script sources:

| Real script | Previous Release | New Release | Additional saving | New Binaryen `-Oz` | Remaining gap |
| --- | ---: | ---: | ---: | ---: | ---: |
| Minish Cap | 31,477 | 30,516 | 961 | 28,687 | 1,829 |
| Lunistice | 28,530 | 27,949 | 581 | 25,781 | 2,168 |
| Celeste external port | 30,502 | 30,352 | 150 | 28,730 | 1,622 |
| Neon White | 4,646 | 4,488 | 158 | 3,775 | 713 |
| A Hat in Time | 43,943 | 43,943 | 0 | 42,014 | 1,929 |

No measured fixture grows. Automatic Unity and the collection fixtures also
shrink, but the real-script savings justify the change independently. These
savings also improve the input to Binaryen: its new outputs are smaller too,
so the remaining optimization gap is largely unchanged. A refreshed pass-family
comparison still identifies local/control cleanup and its interaction with
inlining as the larger remaining opportunities.

Validation covers complete encoded module sizes around signed/unsigned LEB
boundaries, section overhead, duplicate literals, ASCII and multibyte UTF-8.
A source-level runtime test checks short literals, empty strings, byte lengths,
repeated allocation and suspension in both profiles. The existing static-data
test now explicitly checks active segments: passive GC data is allowed, while
GC-only literals must still stay out of linear memory.

All 525 library, 697 compiler, 20 binary and five example tests pass, as do
Clippy and the browser compiler check. All 176 maintained modules validate,
all 211 runtime scenarios and the Debug/Release profile check pass, and the
size corpus passes its 36 behavioral invocations. Celeste remains unchanged
and validates, without a maintained gameplay harness here.

The Unity baseline requires a reviewed refresh because bytes move from code
into passive data. All 38 complete modules shrink relative to the stored
baseline; function/type counts, helper sets, source fingerprints, linear static
data, scratch/read capacities and memory-page counts are unchanged. Data-section
growth is expected, and local Map/Set fixtures gain a three-byte DataCount
section. Several discovery/scanning helpers grow by one byte because an existing
pooled string's offset crosses the signed-LEB 64-byte boundary. For example,
Lunistice's `UnityDiscoverIl2Cpp64::poll` differs from `3b849bb` only by changing
that offset from 56 to 87. Both Lunistice editions pass before refreshing the
baseline. The refresh also records the earlier control/global savings already
on master; the incremental real-script table above isolates this change.

## Nested selections and early exits — 2026-10-02

Binaryen's remaining control-flow differences include a value-producing `if`
whose first arm immediately branches out, and chains of pure selections. The
existing Release cleanup now converts the first pattern into `br_if` followed
by the other arm. It retains a typed block around that arm until ordinary label
cleanup proves the label unused. This preserves internal branch targets, values
below the condition, and loop backedges. The rule excludes parameterized ifs,
whose bare branch can carry a payload, and branches to the if's own label.

Expression analysis now recognizes both ordinary and typed `select` operands.
It rewrites completed inner selections before inspecting their parents, so a
chain can simplify in one traversal. This replaces the old deferred list of
nonoverlapping edits; no new pass or cleanup sweep is added. The existing
32-instruction arm and 256-instruction condition limits remain, as do the
nontrapping/effect-free arm requirement and rejection of conflicting condition
writes. Debug emission is unchanged.

Measured against master `77bea0a`:

| Real script | Previous Release | New Release | Additional saving | Binaryen `-Oz` | Remaining gap |
| --- | ---: | ---: | ---: | ---: | ---: |
| Minish Cap | 30,516 | 30,492 | 24 | 28,662 | 1,830 |
| Lunistice | 27,949 | 27,920 | 29 | 25,781 | 2,139 |
| Celeste external port | 30,352 | 30,210 | 142 | 28,721 | 1,489 |
| Neon White | 4,488 | 4,482 | 6 | 3,775 | 707 |
| A Hat in Time | 43,943 | 43,834 | 109 | 41,999 | 1,835 |

No measured fixture grows. Early-exit rewriting alone contributes 24 / 4 / 49 /
16 bytes on Minish Cap / Lunistice / Celeste / A Hat in Time; nested selections
provide the remainder. The change is small in implementation scope and has its
clearest real-script benefit in Celeste and A Hat in Time. Full Binaryen
pass-family comparisons still show larger interacting local/inlining savings;
this is incremental control cleanup, not a replacement for that work.

Three new runtime tests cover early-exit effects, discarded stack operands,
internal typed labels, loop backedges, parameterized-if rejection, a 17-way
selection chain, and writes inside selection conditions. The GC reference test
also exercises nested typed selections in both the condition and an arm.
All 528 library, 697 compiler, 20 binary and five example tests pass, as do
Clippy, the browser compiler check, Debug-equivalence checks, and the size
corpus's 36 behavioral invocations. Celeste's external source remains unchanged
and validates; it still has no maintained gameplay harness here.
All 176 maintained modules validate, all 211 runtime scenarios and the
Debug/Release profile check pass, and the optimized Unity gate passes both
Lunistice editions without a baseline refresh.

## Default structs and left-hand identities — 2026-10-02

A local-lifetime experiment reused slots for nonoverlapping lexical intervals,
conservatively widening them across loops and protecting reads of implicit
defaults. Alone it saved 68 / 28 / 28 bytes on Lunistice / Celeste / A Hat in
Time but grew Minish Cap by 83 bytes by interfering with function sharing.
Giving the earlier inliner full module type information, two cleanup sweeps,
local reuse, and temporary sinking improved the all-call-site trial to
292 / 300 / 351 / 237 bytes saved on Minish Cap / Lunistice / Celeste /
A Hat in Time. Those experimental modules validate, but the larger machinery
and extra analyses are still not promoted. Sources and logs remain under
`target/lifetimes-trial` and `target/lifetimes-inline-sinking.log` locally.

Binaryen's instruction diff instead exposed a smaller implementation opportunity:
some ordinary struct constructors still push every zero/null field explicitly.
The existing Release instruction cleanup now uses `struct.new_default` when
every operand is a literal Wasm default. It retains the exact struct type and
a fresh allocation. Packed integer fields, positive floating-point zero and
nullable references are supported; negative zero, NaNs, nonzero fields and
effectful computations are not replaced. This complements the earlier direct
default-enum emitter shortcut; the general operand check remains Release-only.

The same cleanup now removes left-hand integer identities such as `0 + index`,
`1 * value` and `-1 & value` when the other operand is a nontrapping, zero-input
push. In particular, `local.tee` is not such a push. These rules extend the
existing instruction traversal and add no module pass. Debug is unchanged.

Measured against master `8bc799d`:

| Real script | Previous Release | New Release | Additional saving | Binaryen `-Oz` | Remaining gap |
| --- | ---: | ---: | ---: | ---: | ---: |
| Minish Cap | 30,492 | 30,421 | 71 | 28,662 | 1,759 |
| Lunistice | 27,920 | 27,836 | 84 | 25,781 | 2,055 |
| Celeste external port | 30,210 | 30,102 | 108 | 28,721 | 1,381 |
| Neon White | 4,482 | 4,451 | 31 | 3,775 | 676 |
| A Hat in Time | 43,834 | 43,667 | 167 | 41,999 | 1,668 |

No measured fixture grows. Struct-default rewriting alone contributes
68 / 66 / 90 / 164 bytes on Minish Cap / Lunistice / Celeste / A Hat in Time.
Binaryen's final outputs on those four scripts are unchanged, so these savings
reduce the measured gap rather than also moving the reference result.

Three new runtime tests check packed/float/reference fields, fresh allocation
identity, retained effects, negative-zero and NaN payload bits, both integer
widths and boundaries, and the distinction between local reads and tees.

Validation passed: 531 library, 697 compiler, 20 binary and five example tests;
Clippy with warnings denied; the browser compiler wasm32 check; all 176
maintained modules and 211 runtime scenarios; the Debug/Release profile check;
and 36 corpus runtime invocations. The Unity size gate and Lunistice base/DLC
behavior passed without changing the baseline. All ten corpus fixtures validate
and retain Debug equivalence. Celeste received compilation, validation and
Debug-equivalence checks; no maintained gameplay harness is available here.

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
fixture also checks the entire Debug binary byte for byte. The original pass
promotion made no Debug emitter changes; the later default-enum shortcut above
intentionally benefits both profiles.

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

External real scripts can join these opt-in measurements without becoming
repository fixtures. Set `SPLITSCRIPT_SIZE_EXTRA_CORPUS` to a JSON manifest of
`[artifact_name, source_path]` pairs before either measurement command. Names
must be unique ASCII letters/digits/underscores; paths may be absolute or
relative to the repository. For the sibling Celeste porting workspace:

```powershell
'[["celeste", "../vibe-asl-porting/ports/live_split_celeste_port.split"]]' |
    Set-Content target/extra-size-corpus.json
$env:SPLITSCRIPT_SIZE_EXTRA_CORPUS = 'target/extra-size-corpus.json'
cargo test --lib write_size_corpus --offline -- --ignored --nocapture
node scripts/wasm-size-runtime.mjs
cargo test --profile max-opt --lib measure_optimization_overhead --offline -- --ignored --nocapture
```

The external modules receive the same size, Wasm validation and Debug-equivalence
checks. The runtime runner validates external modules with Node and explicitly
reports when there is no maintained behavioral harness; this is not an in-game
test. The source files remain untouched. Normal tests and default measurements
do not require the external workspace.
