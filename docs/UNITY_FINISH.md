# Unity finish scope and size audit

2026-09-20. This is the current execution order for finishing the Unity work.
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
| Current tested working tree | 43,627 | Bounded image scan and fixed-width discovery |

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
