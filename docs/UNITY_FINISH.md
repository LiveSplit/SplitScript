# Unity finish scope and size audit

2026-09-21. This is the current execution order for finishing the Unity work.
It supersedes the open-ended sequencing and stale checklist in
[the original research plan](UNITY_ASR_PARITY_PLAN.md), which remains the source
of the ASR comparison and implementation evidence.

## Finish line

Ship the researched ASR profiles and runtime improvements, plus the requested
recursive String/array/List/Map/Set reads and owned class snapshots. Unused
features must not add their discovery/readers to Wasm. Explicit-profile
Lunistice Release must be below 30,000 bytes with the existing source, output
pipeline, and base/DLC behavior preserved. Breaking internal changes are fine.

Do not add further features or expand malformed-metadata coverage without a
concrete defect that prevents these requirements from working. Raw UTF-16,
migration support, and unrelated compiler optimization are outside this work.

## What explains the size

These are stored measurements, inspected without rebuilding:

| Revision | Explicit Lunistice bytes | Change |
| --- | ---: | --- |
| `001c38a` | 23,141 | Before IL2CPP profile migration |
| `52f5b65` | 76,166 | Profile migration and source-defined metadata discovery |
| `d5cc416` | 74,926 | Nested-name/generic metadata changes |
| `8590627` | 56,641 | Shared field traversal |
| `8a759ae` | 60,345 | Latest committed implementation |
| Before class-scan change | 57,399 | Pending discovery consolidation |
| `57ed738` | 49,437 | Bounded synchronous class scan |
| `7545e6f` | 43,627 | Bounded image scan and fixed-width discovery |
| `c4b0e39` | 41,504 | Debug-only descriptor sanity checks |
| `c5089e5` | 40,194 | Direct field binding |
| `2c0dc00` | 38,995 | Smaller lookup continuations |
| Current tested working tree | 36,922 | Remove unused linear-memory string copies |

Historical evidence: `tests/baselines/unity.json` at each revision. Current
evidence: `target/unity-baseline/report.json` (ephemeral build artifact).

The major regression happened during the profile migration, before the nested
collection work. At `52f5b65`, field lookup alone occupied 21,166 bytes and class
lookup 14,611 bytes. The migration replaced compact compiler-generated readers
with source-defined async discovery. The generated implementation, rather than
just the number of profile entries, made this expensive.

At the start of this audit, explicit output contained 50,604 bytes of code, 2,838 bytes of data, and
3,957 bytes of other sections/framing. Its largest function bodies included:

| Function | Bytes |
| --- | ---: |
| Class discovery poll | 9,750 |
| Selected provider preparation poll | 4,453 |
| Image discovery poll | 3,173 |
| IL2CPP global discovery poll | 2,292 |
| Shared field traversal step | 2,284 |
| Profile validation | 1,941 |
| Shared field traversal poll | 1,622 |

These are absolute costs, not individually proven removable savings. They do
not yet attribute every byte of growth. The output also retains a 1,020-byte
32-bit table-discovery poll despite selecting a fixed x64 profile; this is a
concrete specialization candidate. Runtime validation of a built-in constant
profile is another candidate. Neither alone can close the 27,400-byte gap.

The old IL2CPP year/version API is already removed. Mono's V1/V1Cattrs/V2/V3
families remain in current ASR, and no Mono functions appear in the explicit
Lunistice report. That report contains the selected profile factory and no
automatic profile-selection catalog. Removing legacy version tables is therefore
not a pending fix for this artifact.

## Remaining work, in order

1. **Restore the size requirement first.** Work from the measured discovery
   costs above. Simplify generated async traversal and specialize fixed profile
   facts so unnecessary width/layout branches and built-in validation disappear.
   Keep retry, cancellation, required lookup behavior, and the requested profiles.
   Measure each isolated change; do not claim prospective savings as achieved.
   If library changes are insufficient, identify a specific code-generation
   defect in these routines before broadening into compiler work.
2. **Close a finite parity checklist.** Map the 14 researched ASR PRs and the
   user's nesting/snapshot requirements to existing code and tests. Profiles,
   recursive containers, and snapshots are already implemented. Repair only
   demonstrated missing requirements. Additional metadata proof frameworks or
   traversal redesigns are not independent completion requirements.
3. **Run final relevant verification and commit.** Verify imported profiles,
   nested reads/snapshot failure behavior, unused-feature exclusion, Lunistice
   editions, and the size gate. Reconcile docs with actual support and state
   native Linux/macOS validation limits explicitly; synthetic fixture evidence
   is not native game validation. Complete logical commits and the goal once
   the required behavior and size gates pass.

Builds/tests remain serialized through `scripts/run_limited.py`. No game should
be left running. Each implementation update should name the requirement being
addressed, the measured result, and the next remaining gate. Do not continue
accepting size growth merely because the new code is reachable.

## First size correction

IL2CPP class lookup now performs at most 64 slots in a synchronous step, with a
small async wrapper retaining only the scan cursor. Failed batches retry without
committing partial progress. Null slots, delayed initialization, ambiguity,
negative starts, count limits, nested matching, yielding, and cancellation retain
their existing behavior. This follows the existing field cursor approach.

The isolated change reduces explicit Lunistice from 57,399 to **49,437 bytes**
(-7,962), and automatic selection from 179,728 to **171,009 bytes** (-8,719).
The class step is 1,140 bytes; the former class poll was 9,750 bytes. Combined
with the previously pending checked-name/image consolidation, explicit output
shrinks 10,908 bytes relative to the committed baseline. It is still 19,438 bytes
above the maximum permitted by the below-30,000-byte requirement.

All 33 metadata artifacts validate and all 38 associated runtime scenarios pass,
including 98 bounds/retry cases in each build profile, profile selection, nested
names, inherited fields, and cancellation. Both Lunistice editions pass. All 38
baseline artifacts shrink or remain the same size; native, local Map/Set, and
standalone identity/export fixtures retain identical sizes. Runtime helper sets,
scratch requirements, and initial memory pages are unchanged. The small increases
in function/type counts represent the cursor, its result, and synchronous helpers
replacing the much larger async routine; they do not reverse the byte savings.

The 48 compiler profile/code-generation tests and documentation validation also
pass under the resource guard.

The next size work remains discovery/preparation and specialization of known
profile facts. This change does not establish final parity or final size acceptance.

## Image scanning and fixed profile widths

IL2CPP image discovery now scans at most 64 assembly slots synchronously and
keeps a small async continuation. A failed batch retries without losing earlier
completed batches. The fixtures cover large vectors and repair in a later batch
at both pointer widths, in addition to the existing initialization/error cases.

Generated state preparation selects the matching discovery root when a literal
profile or zero-argument constructor chain exposes its pointer width. Built-in
profiles and ordinary custom descriptors use this path. More complex constant
expressions conservatively retain generic discovery. Profile and target-width
validation still run. Automatic selection and ordinary dynamic calls continue to
support both architectures. This is a bounded specialization in existing Unity
provider preparation, not a general compiler optimization pass.

Explicit Lunistice shrinks **49,437 -> 43,627 bytes** (-5,810), with both editions
passing. Its x86 discovery functions are absent. The x86 fixture likewise omits
the x64 discovery routine and relative-target scan helper. Automatic Lunistice
shrinks **171,009 -> 170,339 bytes** (-670). Automatic output retains separate
architecture routines and their types; the image-scan savings exceed that cost.
Every one of the 38 baseline sources is unchanged, no fixture grows, and no new
runtime helper, scratch capacity, or initial memory page is retained.

All 49 profile/code-generation tests pass, including built-in x86/x64 profiles,
constructor aliases, custom descriptors, and opposite-architecture exclusion.
All 33 metadata artifacts validate and all 38 runtime scenarios pass. The bounds
fixture now has 102 cases per build profile with a maximum of 259 reads in one
update, including the new image-batch retry cases.

The 56 catalog/type-checking tests, Clippy with warnings denied, and documentation
validation also pass under the resource guard.

The explicit artifact remains 13,628 bytes above the largest size permitted by
the completion requirement. The original parity audit and final size acceptance
remain open; this checkpoint does not add another feature requirement.

## Debug-only descriptor sanity checks

Following the user's request, IL2CPP descriptor alignment, offset, and stride
sanity checks now use the existing `debug` statement. Debug still rejects the
malformed custom-profile fixture before discovery. Release omits the validator
for both measured and custom profiles. Target pointer-width validation remains
in both build profiles, as do checked reads, bounded traversal, and snapshot
failure behavior. No separate trusted-profile API or compiler routing was added.

Explicit Lunistice shrinks **43,627 -> 41,504 bytes** (-2,123), with both editions
passing. Automatic selection remains 170,339 bytes. All 38 size fixtures shrink
or stay unchanged; runtime helper sets, scratch capacity, and memory pages are
unchanged. The 49 profile/code-generation tests pass, including Debug retention
and Release exclusion of descriptor validation. All five relevant runtime
artifacts validate and pass 56 cases, covering measured profiles, custom profiles,
wrong target widths, reattachment, and malformed descriptors in Debug.
Documentation validation passes. Builds and tests ran through the resource guard.

Final size acceptance remains open: 11,505 bytes must still be removed to reach
an artifact strictly below 30,000 bytes. The parity audit also remains open.

## Finite parity audit: implementation and coverage

The review uses ASR commit `cf732d3aeac7509c8ab5f29a4d0d28f16487d245` in a
separate local checkout; the user's ASR branch and changes remain untouched.
Both profile importers pass `--check` against that commit: 26 PE Mono profiles
and eight fallbacks, 13 ELF profiles and four fallbacks, two Mach-O profiles and
four fallbacks, and 22 complete IL2CPP profiles. These checks compare the source
library and checked-in fixture catalogs, not just profile counts.

| Requirement | Current implementation | Maintained behavior evidence |
| --- | --- | --- |
| #142 binary identities and testing | `Module.peDebugId`, `elfBuildId`, `machUuid`; synthetic host and mapped-image fixtures | `pe_debug_id_runtime.mjs`, `unix_identity_runtime.mjs` |
| #143 Windows Mono profiles | Imported GUID+age/width descriptors, old and modern module discovery, assembly/image name routes | `mono_profiles_runtime.mjs` |
| #144 IL2CPP layouts and discovery | Full descriptors, x86/x64 scans, inline/handle starts, bounded module windows | `il2cpp_profiles_runtime.mjs`, `unity_metadata_bounds_runtime.mjs`; intermediate version selection superseded by #160 |
| #145 shared traversal | `UnityClassNames`, `UnityFields`, backend cursors, declaring-owner propagation | `unity_field_cursor_runtime.mjs`, `mono_metadata_cursor_runtime.mjs`, `unity_metadata_bounds_runtime.mjs` |
| #146 nested/generic metadata and statics | Declaring-chain names, Mono generic-definition counts, IL2CPP sentinel count, owner-relative statics | `unity_nested_metadata_runtime.mjs`, `mono_generic_metadata_runtime.mjs`, `managed_inherited_static_runtime.mjs`, `mono_static_path_runtime.mjs` |
| #147 managed strings and value arrays | Width-aware object headers, stored lengths, shared managed decoder graph | `managed_string_width_runtime.mjs`, `managed_arrays_runtime.mjs`; raw UTF-16 intentionally excluded |
| #148 lists | Runtime ancestor/layout discovery, actual size versus backing capacity, attachment-scoped cache | `managed_list_layout_runtime.mjs`, `managed_lists_runtime.mjs` |
| #150 dictionaries | Validated entry metadata and live-slot scan, recursive local `Map` construction | `managed_keyed_layout_runtime.mjs`, `managed_keyed_slots_runtime.mjs`, `managed_maps_runtime.mjs` |
| #151 hash sets | Slot layout/high-water scan and recursive local `Set` construction | The shared keyed fixtures plus `managed_sets_runtime.mjs` |
| #152 old corlib parallel arrays | Separate links/keys/values or slots, either link-member order, hash-high-bit liveness | Parallel modes in the keyed, map, and set fixtures |
| #153 reference arrays/lists and direct strings | Pointer-width element slots, nullable children, recursive string/object decoders | Array/list/string fixtures and `managed_deep_snapshot_runtime.mjs` |
| #155 Linux Mono | Runtime-ID precedence, measured/fallback layouts, ELF symbol and x64 discovery | `mono_linux_runtime.mjs`, `mono_explicit_unix_runtime.mjs`, `elf_export_runtime.mjs` |
| #156 macOS Mono | Active-slice UUID/architecture, x86_64 and ARM64 discovery, measured/fallback layouts | `mono_mac_runtime.mjs`, `mono_explicit_unix_runtime.mjs`, `mach_export_runtime.mjs` |
| #160 final IL2CPP selection | Complete explicit/custom profiles; exact four-part version then major/minor fallback; removed year selectors | Profile importer, `il2cpp_profiles_runtime.mjs`, compiler `profiles_codegen` tests |

The internal list/keyed layout and IL2CPP storage fixtures run through
`src/managed_collection_layout_tests.rs`; public schema fixtures run through
the `xtask` runtime catalog. Both must be included in final verification.

Beyond ASR, `src/managed_read.rs` models fixed memory values, strings, arrays,
lists, maps, sets, classes, and nullable children as composable decoder nodes.
Compiler capability tests cover the `MemoryReadable` implication and generic
constraints. Public fixtures exercise maps of string arrays, arrays of maps,
nested keyed collections, lists of class snapshots, recursive class trees,
nullability, duplicate decoded keys, and transitive freezing. They also check
that a failed nested read preserves `current` and `old`. The compiler's unused
schema tests compare actual module/section sizes, helpers, data bounds, and
scratch requirements, rather than relying only on retained symbol names.

The runtime and Lunistice port transparently recognize property backing fields,
as ASR does. Conflicting exact-name documentation and an AST comment have been
corrected to describe that behavior. The user prefers explicit field/property
intent in the future; the syntax decision is deferred in `TODO.md`. The
experimental exact-name change and its Lunistice source edits were reverted.

A stale standard-library paragraph also claimed IL2CPP generic storage was
unimplemented; it now describes the existing implementation.

No additional requested feature is identified by this review. Completion still
requires the size gate and final repository verification. Native Linux/macOS
game validation is unavailable here and remains explicitly unverified; the
platform evidence above is synthetic. Lunistice's recorded live evidence covers
title-screen attachment, while maintained fixtures cover base/DLC behavior.
The separate P1 compiler optimization project in `TODO.md` is not a dependency.

## Direct field binding

Removed redundant IL2CPP forwarding futures and the obsolete `!0` ambiguity
sentinel check: the shared field cursor already rejects ambiguous metadata.
Checks, rejection messages, backing-field matching, retry, cancellation, and
declaring-owner static storage remain in place. Lunistice source is unchanged.

Explicit Release shrinks **41,504 -> 40,194 bytes** (-1,310); automatic selection
shrinks **170,339 -> 168,934 bytes** (-1,405). All 38 baseline artifacts shrink or
stay unchanged, with the same source fingerprints, runtime helpers, scratch
capacities, and initial page counts. The 33 metadata artifacts and 38 associated
runtime scenarios passed for this isolated change, as did both Release
Lunistice editions. Evidence is in `target/unity-direct-fields-*.log`.

The retained wrapper-only patch was rebuilt and the 33-artifact / 38-scenario
metadata matrix passed again. All 38 baseline sources are unchanged, both
Release Lunistice editions pass, and the reviewed baseline passes its strict
gate. Final evidence is in `target/unity-direct-fields-final-*.log`.
The broader exact-alias experimental test run is not used as verification of
this retained patch. General compiler optimizations remain deferred.

**Remaining:** remove another 10,195 bytes to get strictly below 30,000, then run
the final repository checks. The finite functional audit found no further
missing requested features. No game was launched.


## Smaller lookup continuations

Moved field-name expansion into a synchronous helper, removed the image lookup
forwarding future, and merged required-class validation into the class scan
continuation. Static-slot lookup retries its table read and checked offset
together. Matching, ambiguity, missing-class behavior, and cancellation are
preserved. These are library changes to the discovery routines introduced by
the Unity migration; general compiler optimizations remain deferred.

Explicit Lunistice shrinks **40,194 -> 38,995 bytes** (-1,199); automatic selection
shrinks **168,934 -> 167,631 bytes** (-1,303). All 38 baseline artifacts shrink or
stay unchanged, with identical runtime helper sets, scratch capacities, read
capacities, and initial page counts. The static-slot retry adds a 36-byte error
literal while removing substantially more continuation code and types. Nine
unaffected artifacts retain identical normalized function bodies and sections.
The Lunistice source fingerprint changes only because of a line wrap; compiling
the prior source with the current compiler produces byte-identical Release Wasm.

All 33 metadata artifacts and 38 runtime scenarios pass, both Release Lunistice
editions pass, and documentation validation passes. Logs are
`target/unity-lookup-continuations-*.log`.

**Remaining:** remove another 8,996 bytes to get strictly below 30,000, then
complete final repository verification. No game was launched.


## Remove unused linear-memory string copies

The static-data planner interned every reachable string expression, although
ordinary strings are constructed as GC arrays. This also retained text from
Unity metadata errors whose payload construction was already omitted. Static
strings now follow their actual ABI consumers: process and module names,
settings, pointer paths, and attachment-shape reports. GC string construction,
error behavior, signature data, and the output pipeline are unchanged. This
fixes demand-driven emission; it does not introduce the deferred general
optimization passes or the earlier string-pooling experiment.

Explicit Lunistice shrinks **38,995 -> 36,922 bytes** (-2,073); automatic selection
shrinks **167,631 -> 159,829 bytes** (-7,802). All 38 artifacts shrink or remain
unchanged. Source fingerprints, function-body byte counts, type/function counts,
helper sets, scratch capacity, and initial pages are identical. Both Release
Lunistice editions pass.

Reachability tests now inspect GC byte-array instructions as well as literal
bytes. New checks cover retained module-query data, omitted GC/error copies, and
large strings that do or do not require linear memory. The compiler run passed
684 tests; its sole failure was an outdated page-count expectation in the new
settings case. After correcting that expectation, all 50 profile tests and the
targeted signature-data test pass.

The full public runtime catalog compiled and validated 170 artifacts and passed
207 of 208 scenarios. The remaining smoke fixture supplied a GameManager object
without its IL2CPP class header, so the existing nominal-class check rejected it.
Adding the missing header makes that scenario pass as well. No runtime behavior
was changed to accommodate the fixture. Logs: `target/unity-static-data-*.log`.

The strict regression baseline, documentation validation, Rustfmt, and Clippy
with warnings denied pass under the resource guard.

**Remaining:** remove another 6,923 bytes to get strictly below 30,000, then
complete final repository verification. No game was launched.
