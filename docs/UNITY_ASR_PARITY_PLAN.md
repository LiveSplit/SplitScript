# Unity support: ASR research and SplitScript implementation plan

Status: researched on 2026-09-18; the measurement gate, reachable binding/scratch allocation, and Step 3 binary identity readers are implemented. Runtime profile integration and recursive managed readers remain planned. See [implementation progress](#implementation-progress).

## Objective and baseline

Bring the improvements from ero-qt's ASR Unity series into SplitScript's schema-based Unity provider, then go beyond ASR with **recursively composable managed reads integrated with class snapshots**. A dictionary containing arrays of strings, or a class snapshot containing such a dictionary, must work through the same decoding system. Flat collection readers alone do not satisfy this plan. **Unused managed features must contribute no feature-specific discovery, decoding, tables, or scratch storage to generated Wasm, and size must be checked at every implementation step using Lunistice as the running baseline.** Replace obsolete implementations where necessary; SplitScript is not stable, so source compatibility, deprecation periods, and compatibility shims are not requirements. Update examples, tests, documentation, and editor support with each API change.

The comparison is pinned to:

- ASR start, inclusive: [`566e9a12a62a677bed0abbd1998d533cfa01760c`](https://github.com/LiveSplit/asr/commit/566e9a12a62a677bed0abbd1998d533cfa01760c), “add mock host for tests,” in PR #142. Its parent is `12375fc19255b19b0a2333bbd435a44e5a607256`.
- ASR end: [`cf732d3aeac7509c8ab5f29a4d0d28f16487d245`](https://github.com/LiveSplit/asr/commit/cf732d3aeac7509c8ab5f29a4d0d28f16487d245), upstream `master` when checked, merging PR #160.
- SplitScript: `81cd3e83950c6398619265936ca219d9a58a1060`.

`C:\Projekte\asr` was on `read-into-uninit-slice-unsound` at `07d7992`, with unrelated uncommitted changes. Its `origin/master` and `upstream/master` refs also did not identify the current upstream tip. Research therefore used a separate checkout under the ignored `target/asr-unity-review` directory, fetched directly from LiveSplit/asr. The original checkout was not changed. This is a comparison of committed upstream source and SplitScript source, not of the dirty local ASR branch.

The inclusive upstream range contains **14 merged PRs**. The reviewed evidence includes their commit history and diffs, final implementation, measured layout tables, and synthetic tests. Live games and the test suites were not run for this planning task; fixture coverage below describes source inspected, not new test results.

## Implementation progress

The first implementation slice establishes measurement before changing Unity behavior:

- Added `compiler::codegen_with_report`, which reports the actual retained function indices/names, runtime-helper plan, scratch reservation, static data bounds, and initial memory pages alongside the ordinary module. It adds no report data or debug names to Release Wasm.
- Added `cargo xtask unity-baseline` and `tests/baselines/unity.json`. The gate validates 14 Release artifacts, accounts for every section and defined function, checks growth and newly retained helpers/functions, and executes Lunistice base/DLC host scenarios. It is part of both `conformance` and `check`. See [baseline instructions](BASELINES.md#unity-migration-gate).
- Recorded initial Lunistice size: **27,677 bytes** with explicit IL2CPP selection; **51,354 bytes** with automatic Unity selection. The initial reported artifact was byte-identical to the module produced by the pre-change conformance run. Automatic selection currently has size coverage only in this new gate.
- Added Release tests for unused string-decoder exclusion, explicit backend pruning, and local collections excluding managed runtime functions. The report path preserves Release bytes; Debug runtime/name/metadata sections are also unchanged. Existing Debug DWARF global ordering is nondeterministic and is excluded from the byte comparison.
- Baseline verification revealed two existing demand gaps: unconditional **22,528-byte scratch reservations** and generated schema preparation binding unread declarations even though their decoders are pruned. The next two slices address these gaps. Do not mistake decoder exclusion for complete metadata-discovery exclusion.

The second implementation slice makes scratch reservations follow reachable operations:

- Runtime-helper descriptors declare their scratch roles. Settings and process-selection buffers follow their actual host imports. Undemanded buffers have zero capacity, and trying to emit an address for an undemanded buffer fails during compilation.
- ABI read capacity follows reachable typed reads, including generic specializations, managed fields/snapshots, and provider-specific pointer widths. Unused large structs and unreachable read functions no longer enlarge scratch. Genesis word-swapped reads retain the required normalization headroom, including in native string input buffers.
- Empty native scripts and local map/set fixtures now reserve **zero scratch bytes** and start with one memory page instead of two. Lunistice reserves **14,336 bytes**, down from 22,528, while its Wasm remains **27,677 bytes** and still starts with two memory pages. No fixture grows: native/map/set modules shrink by four bytes each; the Mono string fixture shrinks by eleven bytes because its relocated buffer addresses use shorter Wasm immediates.
- `tests/baselines/unity-initial.json` preserves the original measurements; `unity.json` is the rolling reviewed baseline. This change does not fix eager schema metadata binding or add profile/collection APIs.
- Validation: full `cargo xtask conformance` passes, including all 97 runtime scenarios and the Lunistice base/DLC size gate; the comparison against the preserved initial report also passes. Clippy with warnings denied and formatting checks pass. Added regressions cover absent buffers, alias-bank bounds, unused large declarations/dead reads, reader pointer width, and normalized string-read headroom. No live-game validation was performed.

The third implementation slice makes generated metadata binding follow reachable operations:

- Analyze resolved, profile-filtered operations before making generated provider preparation reachable. Keep fields used by live reads, every instance field read by a reachable snapshot, class headers needed by instances/components, and automatic shape-selection evidence. Prune unused class/image lookup awaits, field probes, binding fields, and constructor values before rebuilding async control flow. User-authored effects and checked tooling products retain their original declarations.
- Conditional presence validation follows the retained binding fields. Automatically selected shapes still probe their evidence even when it is never read by the script; explicitly selected shapes do not require unread fields to exist.
- Static-read transaction caches now follow reachability too. A dead string read previously caused cache planning to request a Result GC type that was never emitted; regression coverage now checks both the missing metadata and the unused decoder/cache paths.
- Compiler regressions cover byte-identical Release output for scalar versus unused-string schemas on both backends, dead and Debug-only reads, unused classes/images/instance scans, complete snapshot reads, and automatic versus explicit shape evidence. A Mono runtime fixture reuses inherited-static memory with missing unread metadata to verify that attachment still completes.
- Size comparison: unused-string schemas shrink by **699 bytes (IL2CPP)** and **703 bytes (Mono)** and become byte-identical to scalar schemas. Mono instances shrink by **3,959 bytes** by omitting unread `health` metadata discovery. All other fixtures and scratch reservations are unchanged; Lunistice remains **27,677 bytes** explicit / **51,354 bytes** automatic, and both edition scenarios pass. The strict gate flags only a synthetic future rename (`expr6432` → `expr6424`, same 437-byte body); no module, section, body, or memory grows. This rename is reviewed in the rolling baseline; the original baseline is preserved.
- Validation: full `cargo xtask conformance` passes with **652 compiler integration tests**, **98 runtime scenarios**, and the 14-fixture size gate plus Lunistice base/DLC. Clippy with warnings denied, formatting, and diff checks pass. No live-game validation was performed.

The fourth slice adds the binary identities needed by measured runtime profiles (Step 3):

- Added `Module.peDebugId() -> PeDebugId?!` for PE32/PE32+ RSDS records. `PeDebugId` retains all 16 GUID bytes in CodeView storage order and the PDB age, with structural equality and `fromParts` for expected identities. Missing debug metadata and older CodeView formats return `None`; malformed or unreadable records return an error.
- Added `Module.elfBuildId() -> [u8]?!` for little-endian ELF32/ELF64. It derives relocation from the header's `PT_LOAD`, locates `PT_NOTE` through virtual addresses, respects note padding, and returns exactly 1–32 bytes. Oversized IDs fail without truncation.
- Added `Module.machUuid() -> [u8; 16]?!` for an active little-endian 64-bit Mach-O image at the host-reported module base. It validates command bounds and sizes before reading `LC_UUID`; raw universal files, Mach-O32, and big-endian images are rejected. This API consumes the mapped image and does not choose an architecture from an on-disk wrapper.
- All three are source-defined standard-library readers using the existing host ABI. A shared private span check guards mapped bounds and address overflow. Traversals fail explicitly beyond 4096 PE entries, 1024 ELF program headers / 4096 ELF notes, or 4096 Mach-O commands. They do not silently inspect only an initial table prefix.
- Added reusable mapped-image builders and **196 runtime cases** covering both PE/ELF widths and Mach-O x86-64/arm64, absence versus failure, late entries, byte order, age comparison, relocation, different file offsets/RVAs, malformed lengths, failed reads, work limits, and host-read bounds. Identity readers have separate size fixtures and format-specific reachability coverage.
- The first size check exposed unconditional emission of standard-library GC types and their constructed field types. Fixed type planning to retain only reachable declarations and their transitive fields, including dependencies of runtime-helper signatures, intrinsic scratch state, result error strings, settings refresh, and derived formatting. Regression coverage distinguishes PE identity and ELF segment types from their shared GUID array layout, including unused function parameters.
- Final size comparison: all 14 existing fixtures shrink, with no new runtime helpers or functions and no scratch/page growth. Empty native Wasm is **615 bytes** (previously 1,005); Lunistice is **27,204 bytes** explicit / **50,575 bytes** automatic (previously 27,677 / 51,354). The new complete identity fixtures are **7,135 bytes PE**, **7,718 bytes ELF**, and **4,308 bytes Mach-O**. Comparisons against the previous and initial reports require review only for these new fixtures and generated numeric-name shifts; the rolling gate now covers 17 artifacts.
- Validation on 2026-09-19: **442 library tests**, **653 compiler integration tests**, and **101 runtime scenarios** pass, including all 196 identity cases and Lunistice base/DLC. Formatting, Clippy with warnings denied, documentation validation, and the reviewed size gate pass. No live-game validation was performed.
- Exact Mono/IL2CPP profile selection is not wired to these readers yet. Linux/macOS Unity discovery and live-game validation also remain separate work.

Step 1 is **in progress**, not complete: executable contracts for new profile/read APIs and recursive acceptance examples still need implementation. No new managed collection or recursive snapshot support is claimed by these slices. Live game validation has not been performed.

## What changed upstream

| PR and integration commit | Changes and implications |
| --- | --- |
| [#142](https://github.com/LiveSplit/asr/pull/142), `6bc54a1` | Adds the test mock host and mapped-binary identities: PE CodeView/PDB GUID **and age**, ELF GNU build ID, and Mach-O UUID. Follow-up commits validate declared header/directory/command sizes, module bounds, ELF load bias and program headers. ELF identities support 1–32 bytes and little-endian ELF32/ELF64; Mach-O UUID reading supports little-endian 64-bit images, not Mach-O32. These readers identify runtime builds; they do not supply layouts themselves. |
| [#143](https://github.com/LiveSplit/asr/pull/143), `ab15838` | Adds exact Windows Mono build profiles keyed by PDB GUID **and age**, checked against target pointer width. Supports assembly names through the image as an alternative to the assembly structure; removes an unused class-image offset. Auto attachment prefers a known build and otherwise retains version-based detection. Profiles cover old and modern Mono and x86/x64; provenance comments distinguish PDB measurements from layouts reconstructed by hand. |
| [#144](https://github.com/LiveSplit/asr/pull/144), `b49e074` | Adds measured Windows IL2CPP builds, image-based assembly names, full four-part player-version matching, x86 discovery signatures, pointer-width-correct assembly reads, and bounded/validated global discovery. Fixes the Unity 2023 year classification and a metadata-handle spelling error. Initially selects measured layouts using metadata version + player version + width and waits for metadata initialization before fallback on measured players. **#160 supersedes this selection mechanism.** |
| [#145](https://github.com/LiveSplit/asr/pull/145), `691da03` | Extracts a shared managed metadata walker, assembly/class cursors, and resumable pointer-path resolution. Backend-specific operations remain explicit: assembly/class enumeration, field count, static table, and object-to-class lookup. Adds parity tests before replacing duplicate Mono/IL2CPP walks. This is an architectural change rather than a new language feature. |
| [#146](https://github.com/LiveSplit/asr/pull/146), `b45a873` | Adds .NET nested names (`Namespace.Outer+Inner`), validates the full declaring-type chain, and obtains the namespace from the outermost type. Mono inflated generic instances obtain their field **count** from their generic definition while reading fields from the instance. IL2CPP's `u16::MAX` generic-definition field count is treated as no usable fields. Field lookup returns its declaring class, so inherited statics use the owner's static storage in class and pointer-path reads. |
| [#147](https://github.com/LiveSplit/asr/pull/147), `59a8fac` | Adds shared length-prefixed managed UTF-16 strings and arrays of fixed-layout values, at both pointer widths. Reads reject null containers, invalid lengths, over-capacity results, and failed payload reads. Strings preserve embedded NUL and raw unpaired UTF-16 units. Array results are bounded `ArrayVec`s after review, not padded fixed arrays. |
| [#148](https://github.com/LiveSplit/asr/pull/148), `18dafaf` | Adds managed lists. Resolves `_items` and `_size` from the actual runtime class hierarchy and caches the layout for repeated reads. Review hardening requires a `System.Collections.Generic.List\`1` ancestor, including derived lists, rather than accepting any object with similarly named fields. Reads use live size, validate it against backing capacity, and do not require capacity to fit the output bound. |
| [#150](https://github.com/LiveSplit/asr/pull/150), `d47fefd` | Adds dictionary layout discovery and live-pair reads. Supports underscored and older field names, follows backing-field type metadata to the entry class, subtracts the boxed header from entry size/member offsets, and validates member room/stride. Skips freed entries, balances observed live entries against `count - freeCount`, bounds scanning, and accepts canonical empty dictionaries with unallocated backing arrays. Includes the later hardening commits, not just the initial reader. |
| [#151](https://github.com/LiveSplit/asr/pull/151), `4a60e56` | Adds hash sets using discovered slot layouts and live count versus high-water mark. Supports `_slots` and `m_slots` naming families, validates a HashSet ancestor, skips freed slots, checks exact live tally, and accepts canonical empty unallocated sets. |
| [#152](https://github.com/LiveSplit/asr/pull/152), `1578dde` | Adds the older corlib parallel-array dictionary/set shapes. Resolves links, keys/values or slots, touched count, and live count. Link entries must be exactly two integers; `HashCode` and `Next` may occur in either order. Liveness is the hash high bit (`0x80000000`), not the entry-array freed-slot convention. |
| [#153](https://github.com/LiveSplit/asr/pull/153), `4d98f0d` | Adds reference arrays/lists with element stride owned by target pointer width, preserving null elements in place. Adds string reads directly from an object address, avoiding an extra reference dereference. Returns object addresses; it does not implement recursive object snapshots, managed equality, or automatic conversion of dictionary reference keys/values. |
| [#155](https://github.com/LiveSplit/asr/pull/155), `5021c43` | Adds measured Linux Mono profiles and additional measured members in fallback layouts. Reads the Mono library's own ELF build ID first. Only when the library has no readable ID does `UnityPlayer.so` provide the identity; an unknown but present library ID must not be overridden by a known player ID. The measured table has 13 x64 entries, from Unity 5.6 through 6000.7, across V1/V1Cattrs/V2/V3. |
| [#156](https://github.com/LiveSplit/asr/pull/156), `6a0bbdc` | Adds exact Mac Mono UUID profiles: two architecture slices for Unity 6000.5.10f1, x86_64 and arm64. Adds the measured members needed for nested/generic/collection handling and strengthens Mach-O V3 fallback layout information. This is not a comprehensive measured database of every macOS Unity release. |
| [#160](https://github.com/LiveSplit/asr/pull/160), `cf732d3` | Replaces IL2CPP version buckets and metadata-version detection with complete measured `Profile` values. Auto attachment selects an exact full Unity version + width when available, otherwise a nearest profile by major/minor. Exposes named built-in profiles and exhaustive custom profile construction; explicit attachment rejects pointer-width mismatch and can omit the automatic profile table. Deletes the old IL2CPP `Version` API. |

PR numbering gaps do not represent missing migrations: #149, #154, and #157–159 are not additional merged changes in this pinned series.

### Final profile selection, rather than intermediate implementations

The final [IL2CPP build lookup](https://github.com/LiveSplit/asr/blob/cf732d3aeac7509c8ab5f29a4d0d28f16487d245/src/game_engine/unity/il2cpp/builds.rs) does this:

1. Filter measured builds to the target width.
2. Prefer an exact match of all four `UnityPlayer.dll` file-version components.
3. Otherwise choose the last/newest table entry whose **major/minor** is at or below the target major/minor.
4. If none qualifies, choose the oldest entry at that width.

“Nearest” is not absolute numerical distance and is not a floor over the complete version. For example, an unmeasured 2023.1.10 player takes 2023.1.22; exact 2023.1.0 keeps its own profile. The fourth component is the numeric PE private part, not simply the digit after `f`. The source table has 22 profiles: x86 and x64 for 11 players, from 2018.4.36f1 through 6000.7.0a3.

This fallback is a compatibility heuristic, not proof that an unmeasured build has the same layout. Port its selection tests and diagnostic showing target and selected versions. Validate discovered metadata before accepting a binding. Do not silently describe a nearest match as an exact measurement.

The final IL2CPP attach path still scans the `global-metadata.dat` **name** to locate code references and globals. It no longer scans loaded metadata headers to select a version/profile. Do not implement the intermediate #144 metadata-header selection, its waiting/fallback machinery, or the deleted year table alongside #160.

Mono remains different: exact PE/ELF/Mach-O identity selection followed by format/width/version fallback. Do not apply IL2CPP nearest-version selection to Mono.

### Boundaries of upstream support

- Final IL2CPP attachment is Windows PE with `GameAssembly.dll`/`UnityPlayer.dll`, x86 and x64. Linux/macOS IL2CPP and ARM IL2CPP are not delivered by this series.
- Mono already had platform-specific discovery in ASR before these changes. Porting its new Linux/Mac profiles to SplitScript also requires the underlying discovery and symbol-reading capabilities that SplitScript has not implemented.
- ASR collection support is for the specific metadata shapes implemented upstream. It does not provide recursively composed high-level decoders, multidimensional/jagged-array decoding, every new HashSet layout, all generic type forms, or arbitrary CLR object graphs. **SplitScript must add recursive composition, including jagged arrays, beyond that baseline.** Multidimensional CLR arrays remain a separate layout feature; arbitrary object graphs are not followed implicitly beyond declared schema fields.
- ASR's shared walker retains some permissive/unbounded historical behavior, such as a signed Mono bucket count cast to `u64` and parent walks without a global cycle budget. Copy the supported layouts and behavior, but retain/add SplitScript's bounded, cancellable execution model.

## What SplitScript already has, and what is missing

| Area | Current evidence | Required action |
| --- | --- | --- |
| Schema-based Unity workflow | `image`/`namespace`/`class`, `from` alternatives, conditional shapes, `UnityRuntime`/`UnityMetadataClass` adapters in `stdlib/standard.split`; binding plan in `src/managed.rs` | Keep this as the public workflow. ASR's low-level Rust objects are implementation references, not an API to reproduce verbatim. |
| Managed strings | `ManagedFieldRead::ManagedString`; required `maxLength` of 1–2048 UTF-16 units; nullable strings, failed-read propagation, replacement decoding, embedded NUL, static/live/snapshot readers | Extend rather than recreate. The object decoder hardcodes length at `0x10` and characters at `0x14`; make these `2 * pointerBytes` and `2 * pointerBytes + 4`. Its field-pointer reader already branches on width, which does **not** make the payload decoder 32-bit-capable. |
| Inherited static ownership | `UnityField.owner`; Mono `fieldAddress`/static paths and IL2CPP helpers; SplitScript commit `a7a9c58`; `tests/managed_inherited_static_runtime.*` | Preserve and extend regression coverage. This is already a real implementation, not missing #146 work. The existing fixture's generic-looking class name does not test an inflated generic definition/count route. |
| Class/field names | Both backends handle namespace-qualified names, aliases, backing fields, and ambiguity diagnostics | Add nested declaring chains and generic count/type information. Preserve ambiguity detection; ASR's first matching name is not a replacement for SplitScript's explicit binding rules. |
| IL2CPP layouts | `src/codegen/unity_layout.rs`: four year buckets, a single `OBJECT_LAYOUT`, `POINTER_SIZE = 8`; helpers in `src/codegen/runtime_helpers/unity.rs` | Replace with complete profiles. Field-count/static-table offsets are only part of the needed data. |
| Older IL2CPP image layout | `compile_unity_get_class` always reads count at `0x18` and dereferences the handle at `0x28` | Fix as part of profile conversion: old base/2019 families use count at `0x1c` and an **inline** type start at `0x18`. Existing documentation advertises more layout variants than this code implements faithfully. |
| Unity 2023 detection | `Unity.detectIl2cppVersion` already tests `major > 2022` | The specific #144 comparison fix is already present. The entire detector is nevertheless replaced by final #160 selection. |
| Mono layouts | Source-defined `MonoLayout.forVersion`; PE64 V2/V3 only; discovery hardcodes `mono-2.0-bdwgc.dll` and the x64 Windows export/signature route | Add full profiles, old Mono, PE32, ELF/Mach-O discovery, identity readers, and exact-build selection. |
| Binary metadata | `Module.fileVersion`, pointer-size detection, `Module.peExport`, module/range/read/scan primitives exist | Reuse these foundations. Add PE debug identity, ELF build identity, Mach-O UUID, and the missing platform symbol/discovery paths. No direct ASR crate dependency exists in `Cargo.toml`; upgrading a dependency will not supply these changes. |
| Collections/capabilities | Local arrays/maps/sets and recursive capability analysis in `src/capabilities.rs` exist; `MemoryReadable` derives fixed layouts; `validate_remote_memory_layouts` explicitly rejects growable managed `[T]` fields | Add recursive `ManagedReadable` analysis and decoder plans. A local container's Wasm layout is not a managed heap layout; accepting `[T]` as a raw memory read would be incorrect. |
| References and snapshots | `T.Ref`, transactional `snapshot()`, `T.instances()`, state caching, process-scoped lifetime rules. `managed_field_value_type` maps a declared class to a live reference, and current snapshot generation reuses field readers | Separate live-access types from recursively materialized snapshot field types. Existing top-level transactional construction is not a complete deep snapshot decoder for nested references/containers. |
| Native Unity scenes/components | `unity.scenes`, hierarchy walking, typed components already exist | Keep regression coverage. The ASR range does not change `scene_manager`; do not couple this migration to a scene subsystem rewrite. |
| Testing | `SplitScriptHost`, Mono fixture builder, Lunistice/Mono/instance/static fixtures, compiler tests, `cargo xtask conformance` | Extend these instead of transplanting ASR's Rust host ABI. Import scenarios and measured facts, then exercise SplitScript-generated Wasm. |

### Important semantic differences to decide explicitly

**Strings:** ASR's `ManagedString` is a raw UTF-16-unit container; SplitScript produces a normal UTF-8 `String`, replacing invalid surrogate sequences. Keep ordinary text reads as normal `String` values. To achieve the raw-unit capability too, add a bounded managed UTF-16 storage decoder whose output is `[u16]`; this avoids changing every language string operation or exposing Rust's `ManagedString` representation. Test both paths with unpaired surrogates and embedded NUL.

**Collections:** ASR exposes bounded vectors of values, pairs, or addresses. SplitScript should decode recursively into its normal local `[T]`, `Map<K,V>`, and `Set<T>` values. `Map<String, [String]>` is a required ordinary case, not a special convenience API. Local map/set equality remains SplitScript equality; a custom .NET comparer is not reproduced. Detect duplicate decoded keys/elements that would silently lose remote entries and fail the materialization with a diagnostic. Offer an entry/value sequence projection for callers who need every remote entry despite incompatible equality, or whose keys cannot satisfy local `Equatable`; this must not replace first-class nested map/set support.

**References:** ASR returns `Address`, including zero, for reference array/list elements. SplitScript must distinguish live access (`T.Ref`) from managed materialization (`T`, the declared class snapshot). A root `.snapshot()` recursively materializes declared class fields and container contents into values; callers should not need a manual `.snapshot()`/loop at every nested level. An explicitly requested live-reference view can still return `[T.Ref?]`, subject to lifetime restrictions. Neither a copied outer array nor `ManagedReadable` makes live references retainable in `current`/`old`. Nullability is expressed at each level and preserves element positions.

**Atomicity:** successful decoding means a complete locally constructed result, not an atomic snapshot of another process. Bounds/count checks detect many races but cannot prove the game did not mutate between reads. Never publish partial results; preserve the current state failure/retention policy, and state the remaining race limitation in documentation.

## Target architecture and API direction

Use one attachment-scoped runtime description with backend, binary format/architecture, pointer width, selected layout, selection provenance, and discovered globals. Keep optional measured members explicitly absent when unknown. Do not substitute zero offsets for unavailable declaring-type, instance-size, or generic/type information.

Separate three concerns:

1. **Discovery and profile selection:** source-defined standard-library code orchestrates bounded module/symbol/signature/identity reads and returns a validated descriptor.
2. **Metadata operations:** common matching, inheritance, nested names, and collection shape resolution consume a backend adapter. The adapter owns assembly/class enumeration, field count, static storage, and object/type-to-class decoding.
3. **Value decoding:** one recursive `ManagedReadable` plan composes bounded string, array, list, dictionary, set, and class-snapshot decoders. Each container supplies correctly described remote element slots to its child plan. Compiler lowering supplies typed local values, nullability, shared budgets, and failure behavior.

Keep high-level policy in the standard library and use compiler/runtime intrinsics only where generic typed decoding, resumable traversal, or efficient memory copying warrants them. Do not add a second hand-maintained offset table in emitted instruction code. Extend `src/codegen/unity_layout.rs` or replace its role with a canonical descriptor catalog consumed by the standard library and low-level helpers. Choose the storage location once in Step 2; generate any secondary representation.

For public API changes, the recommended direction is:

- Keep `state Unity` automatic detection.
- Replace numeric `Unity.il2cpp(0/2019/2020/2022)` with explicit complete IL2CPP profile selection. Expose named measured constants and exhaustive custom profiles. A spelling such as `Unity.il2cpp(UnityIl2CppProfiles.UNITY_2022_3_0F1_X86_64)` is a **proposed API**, not existing syntax. Auto selection and explicit selection should share attachment implementation but have distinct reachability roots.
- Retain explicit Mono family selection where useful, add V1/V1Cattrs, and make automatic Mono selection prefer exact binary identity. Offer explicit complete Mono layouts through the same validated descriptor mechanism if needed by unmeasured games.
- Add `ManagedReadable` to the existing capability system. Fixed-layout `MemoryReadable` types are its base cases; high-level `String`, arrays, maps, sets, nullable reference values, and declared class snapshots compose it recursively.
- Prefer ordinary schema value types such as `Map<String, [String]>`, `[[Player?]]`, and `Set<String>` over mandatory `ManagedArray`/`ManagedDictionary` wrappers at every level. Keep remote storage shape separate: a local `[T]` may be backed by a managed vector or a `List<T>`, resolved from field/runtime metadata. Allow explicit storage hints where metadata cannot determine the supported shape; never guess the representation from local value size. Raw UTF-16 remains an explicit opt-in storage projection.
- Extend `ManagedFieldBinding` to carry declared schema type, live-access type, snapshot type, remote storage description, and recursive decoder-plan identity. Array/list materialization produces sequences, dictionary materialization produces maps, set materialization produces sets, and class materialization produces class snapshots. The live-reference projection is distinct and opt-in where required.
- Finalize concrete grammar and bound spelling in Step 1, then update parser, type checking, formatting, diagnostics, highlighting, completion, documentation, and fixtures together. No deprecated parallel spellings are needed.

### Recursive `ManagedReadable` contract

The user's requested nesting is a first-class requirement. The capability means “this type has a managed materialization plan given runtime layout, storage context, and read limits,” not “this type can be copied from one contiguous byte range.” `process.read<T>` continues to require `MemoryReadable`; the managed binder and snapshot reader use `ManagedReadable`.

| Value type | Capability rule | Managed decoding behavior |
| --- | --- | --- |
| Any `T: MemoryReadable` | Also satisfies `ManagedReadable` | Reuse checked fixed-layout decoding for a compatible inline slot. Target layout compatibility is still required; capability membership alone does not prove arbitrary CLR struct packing. |
| `String` | Built-in managed decoder | Read a managed object reference and decode its bounded UTF-16 payload into a local string. |
| `[T]` | Requires `T: ManagedReadable` | Discover/validate vector or list shape and invoke the child decoder for every element. `[String]` uses pointer slots, not local string sizes. `[[T]]` follows references to inner managed arrays/lists recursively. |
| `Map<K,V>` | Requires managed-readable keys/values and local `K: Equatable` | Discover dictionary storage, scan live entries, recursively decode keys and values, then construct the local map without silent key loss. |
| `Set<T>` | Requires `T: ManagedReadable + Equatable` | Discover set storage, recursively decode live elements, and validate that local equality does not collapse distinct remote entries. |
| `T?` | Requires a supported nullable managed representation and a readable child | Null reference produces `None`; non-null invokes the child. `String?`, `[String?]?`, and optional class references are supported. CLR `Nullable<T>` value types need their own measured inline representation; they are not automatically pointer-nullable. |
| Declared managed class `C` | Derived recursively from active instance-field plans | Read declared fields and recursively construct the local immutable `C` snapshot. Static fields are roots, not recursively included snapshot members. |
| Explicit live `C.Ref` | Separate live-access operation | Retains process identity/address semantics; it is not a deep snapshot value and must not become transitively retainable. |

The implication is **`MemoryReadable` implies `ManagedReadable`**, not the reverse. The equivalent super-capability relationship is `MemoryReadable: ManagedReadable`; defining `ManagedReadable: MemoryReadable` would wrongly require strings/maps to have a fixed byte layout. Implement the implication in both generic constraint inference and concrete capability analysis. Avoid infinite recursion between “managed fallback to memory layout” and “memory requires managed”: prove the fixed-layout base case directly and memoize recursive obligations.

Represent the decoder as a graph of interned plan IDs, with nodes such as fixed value, string, nullable reference, vector, list, dictionary, set, and class snapshot. Each node has a local result type and a distinct remote representation. A remote slot describes inline storage versus a reference slot versus an already-resolved object address, target width, extent/stride, and metadata type when available. This prevents double dereferences and mistakes such as using `sizeof(local Map)` as dictionary entry width. Primitive/value-struct children are inline; strings, containers, and managed classes normally occupy reference slots. Explicit boxed-value support, if needed, must remove the object header exactly once.

Use one `ManagedReadContext` per root read/snapshot: runtime/attachment identity, limits, remaining bytes, decoded elements/objects, scanned slots, maximum depth, and an active-address path for cycle detection. All recursive children consume the **same** counters. Limits reset only at a new root operation, never per dictionary value or per nested array. Per-field limits can tighten the root policy; they cannot replenish it. This bounds a small outer map containing many large inner arrays/strings. Resolve layout caches by attachment + actual runtime class + decoder shape, and revalidate shape when the object class changes.

Recursive **types** must not cause infinite compiler expansion: intern placeholders before descending, then finish the plan graph and generate mutually recursive helpers/GC types as needed. Recursive **remote object graphs** are a separate issue: permit finite acyclic traversals and repeated shared children, reject an actual cycle on the active decode path or exhausted depth with a structured failure. Do not treat any repeated address as a cycle if its previous traversal already completed. A later alias-preserving graph snapshot API can be separate; the initial value snapshot contract does not expose partly initialized cyclic objects.

The same graph powers all entry points: static field materialization, instance field materialization, class `.snapshot()`, and nested container elements. The snapshot projection must recursively replace remote class references with class values. Audit `SemanticModel::managed_field_value_type`, `managed_snapshot_field_type`, structural field traversal, snapshot member type checking, `src/codegen/gc_types.rs`, display/equality, and reachability; sharing the current live field reader unchanged would leave addresses inside nested snapshots.

Snapshot-owned arrays/maps/sets must remain immutable through the complete reachable snapshot value, even though ordinary local containers support mutation. Either use the existing retention/copy boundary if it already guarantees that recursively, or add deep freezing/copy-on-write at publication. An immutable outer class with an aliased mutable inner map is insufficient. Incomplete child allocations are private, never inserted into `current` or a published parent; failure at any depth rejects the complete root read. An async root keeps its context across yields and aborts on process close/replacement.

Illustrative target usage (new nested managed types and read-limit configuration are implementation work; this is not a claim that it compiles today):

```splitscript
image "Assembly-CSharp" {
    class Catalog {
        static Catalog instance;
        Map<String, [String]> labelsByCategory;
        [[Item?]] groups;
    }
    class Item {
        String name maxLength 64;
        Map<String, [String]> attributes;
    }
}

state Unity ["game.exe"] {
    catalog: Catalog = Catalog.instance?.snapshot()?;
}
```

The root snapshot has one finite read policy covering all map entries, inner arrays, strings, and class objects (the exact configuration spelling is chosen in Step 1). `catalog.labelsByCategory` is an ordinary local map; `catalog.groups` contains local optional `Item` snapshots. Reading either later performs no process reads. Changing the remote strings or replacing an inner container cannot change an accepted `old.catalog`. Inactive conditional fields are not traversed or charged. Null at a non-optional level, unreadable nested data, type/shape mismatch, a cycle, or budget exhaustion rejects the root snapshot rather than returning a partially filled graph.

### Demand-driven emission and size gate for every step

Implementation priority: deliver the new Unity functionality. Use measurements
as a regression guard while doing that work; do not schedule independent
optimization passes over existing behavior. Address unnecessary growth caused
by a new feature or a concrete implementation need, and otherwise move on.

The compiler may know every profile and decoder. The generated autosplitter must contain only what its reachable operations need. Treat this as an architectural invariant from the first refactor, not a final optimization pass.

1. **Root demand in operations, not declarations or capabilities.** Implementing `ManagedReadable`, declaring a class with a dictionary field, or using a local `Map` must not by itself retain a managed dictionary reader. A reachable read of that field or a reachable `.snapshot()` of a class containing that active field does create demand. A snapshot intentionally reads all its active declared instance fields, even when later code only examines one; document this distinction.
2. **Traverse only demanded decoder-plan nodes.** `Map<String, [String]>` demands dictionary shape resolution, string decoding, vector/list decoding as actually required, local map construction/equality, and their shared primitives. It does not demand set scanning or unrelated class snapshots. Deduplicate shared nodes/helpers across all roots and monomorphizations.
3. **Track discovery separately from payload decoding.** A dictionary root demands dictionary ancestor/field/type resolution; a scalar or string root does not. Keep list/dictionary/set shape resolvers as separately reachable functions. Do not emit a universal runtime `match` covering all managed shapes that makes all branches reachable. The compiler-side plan graph may use enums; generated calls should be specialized to reachable nodes.
4. **Prune the whole dependency chain.** Apply demand to function bodies, async continuation/frame fields, GC types, globals, type/field-name strings, signatures, profile members, failure strings, data segments, and scratch reservations. Reuse `reachability.rs`, `dependencies.rs`, `function_plan.rs`, `data_plan.rs`, and `runtime_helper_registry.rs`; a parallel feature registry with independent retention rules would invite drift.
5. **Make metadata capabilities conditional.** Scalar/static reads must not demand dictionary-entry `instance_size` or type-to-class machinery. Complete profiles are validated in the compiler; emitted representations can project only members needed by the reachable operations. Optional nested/generic capabilities should not drag all collection walkers into every attachment.
6. **Separate automatic and explicit profile roots.** Explicit IL2CPP profile selection excludes the auto-selection table and unnecessary version/identity readers. Explicit backend selection excludes the other backend. Auto selection may legitimately retain the candidate profiles/backends it can choose; account for this data and code separately from collection growth. When one output type allows multiple remote shapes, retain only those genuinely unresolved by the chosen storage policy, and offer an explicit hint to specialize further.
7. **Classify conditional use correctly.** An unused class or never-read field adds no decoder demand. A compile-time unreachable branch adds none. A runtime-selected conditional schema field that can participate in a reachable snapshot needs its decoder; calling that code unused merely because one test selects the other variant would be wrong.

Before implementation, create a reproducible size report at the pinned SplitScript baseline. Rebuild the compiler with fixed tools and compile fixtures in SplitScript Release mode; keep compiler build profile, compiler/tool versions, source hashes, and custom-section policy identical. Track the raw emitted artifact as the primary result. Binaryen, if used as a secondary diagnostic, must use a pinned version and must not hide unused code that SplitScript should have excluded itself. Existing `scripts/binaryen-review.mjs` section parsing and `docs/BASELINES.md` are useful starting points, but their historical numbers are not current baselines.

After **every numbered step**, compare with both the previous step and the initial baseline. Record total bytes, code/data/type/custom-section bytes, function/type counts, retained managed helpers/resolvers, scratch reservations, and compile/runtime costs where affected. For a positive byte delta, identify the actual added functions/data and the script feature that requires them. An explanation such as “the new framework is larger” is insufficient. Intended runtime correctness work or newly selected profile data can justify growth; unused dictionary/list/set machinery cannot. Reject unexplained growth and fix its dependency root before closing the step. Do not compensate for a leaked feature with an unrelated byte saving elsewhere.

Use these fixtures throughout, not only at the end:

| Fixture | Required absence/purpose |
| --- | --- |
| Minimal native script without Unity | No Unity attachment, profiles, metadata names, or managed decoders. |
| Local map/set operations without managed reads | Local collection operations only; no managed map/set offset discovery. |
| Explicit-backend/profile scalar-only Unity | No strings/arrays/lists/maps/sets or their shape resolvers; no unrelated backend/auto-profile database. |
| **Lunistice base/DLC** | Running real-script size and behavior baseline; retain its actual scalar/string/schema needs, with no managed dictionaries, sets, or lists added incidentally. |
| Equivalent Lunistice auto-profile and explicit-profile variants | Attribute profile/selection overhead separately. After replacing year selectors, compare the updated script using an evidence-backed profile; record the source/API change so the size difference is interpretable. |
| String-only, value-array-only, and list-only scripts | Each adds only its own decoder/shape dependencies and shared primitives. |
| Nested map of string arrays; class snapshot containing it | Required transitive nested decoders are present, no unrelated set reader. |
| Unused nested collection declarations and unreachable reads | Same feature dependency set as the corresponding script without those declarations. |

Add automated positive/negative reachability assertions in `tests/compiler/profiles_codegen.rs` and section/helper reports to the existing verification tooling. Inspect the semantic dependency plan and actual Wasm sections/call bodies/data; symbol-name absence alone is not proof after stripping. Pin size budgets only after measuring current output; do not invent a numeric baseline in this plan. Run the Lunistice base/DLC host fixtures after every step and for any size increase, plus relevant focused regressions. Use controlled live Lunistice probes when the attachment/profile/reader behavior changes and the game is available; record live coverage separately from synthetic coverage.

## Ordered implementation plan

Each step is a reviewable change with its own acceptance gate **and the mandatory demand/size/Lunistice gate above**. Steps 1–7 establish correct runtime/metadata foundations; Steps 8–12 add recursive managed materialization; Step 13 completes platform coverage; Step 14 validates and documents the result. Platform work can be split into smaller commits without declaring parity complete early.

### 1. Lock down the new contract and baseline fixtures

1. Record the pinned upstream revision and a per-PR checklist in the implementation issue/PR. Preserve provenance and MIT/Apache attribution for copied code, layouts, and fixtures.
2. Specify the profile selector, `ManagedReadable` rules, remote storage hints, live/snapshot type projections, nested nullability, root budgets, per-field limits, and process-lifetime constraints. Include raw UTF-16 as a separate opt-in decoder and nested map/array/class examples as acceptance targets.
3. Specify finite metadata/collection work limits and typed error versus pending behavior. Unloaded modules/uninitialized metadata may retry; unavailable profile capabilities or malformed layout descriptions need a useful diagnostic, not indefinite retries.
4. Extend `tests/support/splitscript_host.mjs` only where needed for read accounting, module identities, mapped ranges, and failures. Generalize `tests/support/mono_v2_fixture.mjs` and factor reusable IL2CPP fixtures from `tests/lunistice_runtime.mjs`.
5. Capture existing Lunistice base/DLC, Mono, inherited-static, strings, instances, and scene behavior before refactoring. Build fixtures from independently specified memory layouts, not directly from the descriptor under test. Capture the reproducible Wasm size/dependency baseline and wire up the per-step reporting gate before adding features.

**Gate:** design examples and negative examples are agreed in code/tests; baseline conformance and Lunistice size/dependency measurements are recorded; every expected migration has an owner step below. No requirement to keep old syntax compiling after it is replaced.

### 2. Replace partial layout facts with complete validated descriptors

1. Define common assembly/class/field offsets plus backend-specific Mono and IL2CPP descriptors. Include format/architecture/width, assembly-name route, field stride, declaring type, instance size, type data/kind, generic routes, static storage mode, and field-count representation.
2. Model IL2CPP image start as an explicit `Inline(offset)` or `Handle(offset)` variant; never derive it indirectly from a year number. Include class-field count and all class/image/field offsets that now live in `OBJECT_LAYOUT`.
3. Model Mono V1/V1Cattrs static storage separately from the newer `vtable + methodCount * pointerBytes` route. Include generic-definition count routing and type metadata capabilities.
4. Validate descriptors: supported target formats/architectures, pointer width, member sizes/strides, overflow, required name routes, and impossible combinations. A custom profile is exhaustive; future fields require an explicit choice.
5. Thread selected descriptors through `UnityModule`, `MonoModule`, provider bindings, async state storage, runtime-helper signatures, static-table reads, and instance-header/component logic.
6. Remove the four-entry `VERSION_LAYOUTS`, fixed IL2CPP `POINTER_SIZE`, and generic hardcoded `OBJECT_LAYOUT` once all consumers move. Keep each measured fact in one authoritative location. Ensure the emitted descriptor representation can omit unreachable profile tables and unused feature-specific members.

**Files:** `src/codegen/unity_layout.rs`, `stdlib/standard.split`, `src/codegen/runtime_helpers/unity.rs`, `src/codegen/async_state.rs`, `src/codegen/data_plan.rs`, standard-library schema/catalog/intrinsic definitions, helper registry and generated binding fields.

**Gate:** descriptor validation tests and existing PE64 fixtures pass; emitters do not redeclare offsets; an explicit profile retains only its own layout and needed helpers.

### 3. Add bounded binary identity readers

1. Extend module metadata functionality with PE debug identity, ELF build ID, and Mach-O UUID readers using the existing process-read/module-range ABI.
2. PE: honor optional-header size and directory count, validate the complete debug directory and CodeView record inside the mapped image, accept RSDS, preserve GUID bytes and age, and handle canonical GUID byte order correctly.
3. ELF: handle little-endian ELF32/ELF64; validate program-header entry sizes and complete table ranges; derive load bias from the mapped `PT_LOAD`; walk bounded `PT_NOTE` contents using virtual addresses/alignment; retain exactly 1–32 ID bytes and reject larger IDs without truncation.
4. Mach-O: locate the mapped little-endian 64-bit header, validate `ncmds`/`sizeofcmds`, every command size and range, then read `LC_UUID` from the active architecture slice. Do not accidentally parse an on-disk universal wrapper as a mapped header.
5. Validate all address arithmetic against overflow and module bounds before reading. Bound work per update without arbitrary “first 16 entries” correctness shortcuts.

**Gate:** translate #142 tests, including absent identity, truncated records, old PE debug format, late debug/note/load-command entries, mapped ELF at nonzero virtual address, oversized IDs, wrong endianness, and Mach-O32 rejection. Prove failed parsing does not issue out-of-range reads.

### 4. Implement exact Mono profiles and complete Windows Mono attachment

1. Import the final Windows Mono table and provenance, including later generic/collection members added after #143. Key by GUID + age; verify pointer width before accepting the profile.
2. Expand discovery to `mono.dll` and `mono-2.0-bdwgc.dll`, PE32 and PE64. Reuse `peExport` and add both x86 absolute-address signature forms as well as the existing x64 RIP-relative route.
3. Automatic attachment tries exact identity first, then the supported family/format/width fallback. Preserve the distinction between no identity, unknown identity, and failed discovery; report selected route once per attachment rather than logging every retry.
4. Implement V1/V1Cattrs detection for old runtime modules and their static-data path. Avoid requiring `UnityPlayer.dll` when the old runtime can be identified without it.
5. Update explicit Mono selectors and affected examples/tests in the same change. Test identity mismatch and missing capabilities rather than silently borrowing PE64 offsets.

**Gate:** Windows V1/V1Cattrs/V2/V3, x86/x64 fixtures bind images/classes/statics; matching GUID with wrong age is not an exact match; replacing/restarting a process clears identities, globals, and bindings.

### 5. Implement final IL2CPP profiles and robust x86/x64 attachment

1. Import all 22 final measured profiles and their 11 full player versions. Add named profile constants and exhaustive custom profiles. Remove the numeric-year provider API and update all scripts/tests/docs that use it.
2. Reuse `Module.fileVersion()` for all four components. Implement exact-then-major/minor selection precisely as #160, including same-major/minor later-patch choice and oldest-profile fallback. Emit a concise target/selected-profile diagnostic.
3. Explicit profile attachment validates target architecture/width and bypasses auto-version lookup. Ensure it does not retain unused profile tables or require auto-selection-only metadata.
4. x64 discovery: validate both begin/end displacement targets of the assembly-vector signature (`end == begin + 8`); locate the metadata-name reference, shift, and table store within module-bounded windows. Continue after false signature candidates instead of committing to the first prefix match.
5. x86 discovery: validate vector globals at four-byte spacing, absolute-address metadata references, and all three table-store forms, including the post-6000.3 form. Select the earliest valid store as upstream does and validate resulting globals inside the runtime module.
6. Keep scans cooperative. Attachment can find globals before the runtime populates them; binding must wait for usable metadata rather than freezing an empty image/type table.
7. Implement both inline and handle image type-start routes, image-based assembly names, and every target-width array/stride/read. Fix the existing old-image-layout mismatch as part of this work.

**Gate:** full-profile golden cases; old inline and newer handle images; Unity 2023 exact and nearest cases; x86/x64 false-positive signatures; module-edge scan windows; delayed globals; explicit width mismatch; custom profiles; explicit versus automatic Wasm size/reachability comparison.

### 6. Consolidate metadata traversal behind backend adapters

1. Introduce common image/class/field operations and cursors with explicit backend-specific enumeration and static/object-class operations, following #145's boundary.
2. Share assembly-name routing, name/namespace matching, aliases, backing fields, owner propagation, and collection ancestor validation. Keep Mono hash buckets and IL2CPP type-table slices backend-specific.
3. Preserve SplitScript's single-result and ambiguity semantics for `from` alternatives, conditional shape probes, and same-named inherited fields. Do not replace them with ASR's first-match return behavior.
4. Decide and test the engine-base-class stopping policy consistently. Current IL2CPP traversal explicitly stops at `Object`/`UnityEngine`; current Mono probing walks to a null parent. Treat this as a deliberate language policy decision, not an unnoticed side effect of consolidation.
5. Make cursors resumable with finite byte/node budgets, count validation, cycle/depth detection, and process-close cancellation. Separate “not initialized,” “complete search with no match,” and malformed/unreadable traversal as far as the available evidence permits.
6. Keep metadata work in attachment or lazy shape resolution, not repeated for every ordinary per-tick field read. Cache only attachment-valid facts; do not freeze singleton object addresses.

**Gate:** parity fixtures for both backends plus missing/ambiguous names, backing fields, aliases, corrupt counts, cycles, read failures, cancellation, and reattachment. A huge but valid image must progress over multiple polls without monopolizing one update.

### 7. Add nested classes and generic metadata routing

1. Support metadata names like `Game.Outer+Inner+Leaf` through existing `from` names. A new nested schema grammar is unnecessary for this capability.
2. Match the leaf and validate every enclosing name, the outermost namespace, and a null declaring parent beyond the outermost type. If the descriptor lacks a declaring offset, reject the nested lookup explicitly.
3. Mono: detect generic-instance class kind and follow generic-class/container-class metadata for field count, but read the inflated class's field array and field offsets.
4. IL2CPP: reject `u16::MAX` as a real count. Implement the supported type-to-class route through SZARRAY element types and cached generic classes; do not treat every type-data word as a class pointer. Bound recursion and reject unsupported kinds. Extend beyond ASR where nested materialization needs additional array-element, generic-argument, declared-class, or runtime-class metadata; do not assume ASR's collection-entry-only route covers arbitrary nested shapes. Distinguish type validation from the shape resolution that can be done lazily on a live object.
5. Preserve declaring-owner static reads across ordinary, inherited, nested, and inflated generic cases. Extend both class binding and any pointer/path lowering, not just the main state expression path.

**Gate:** port #146 nested/generic tests and add distinct derived/base static tables on **both** backends. Cover duplicate leaf names in different enclosing classes/namespaces, missing declaring metadata, invalid generic pointers, and late materialization of generic classes.

### 8. Implement recursive `ManagedReadable` and finish string parity

1. Add the managed capability behavior and implication to `stdlib/standard.split`, its loader/catalog, `src/capabilities.rs`, inference constraints, and remote-memory validation. All existing `MemoryReadable` types qualify through the fixed-layout base case; high-level container/class types qualify recursively without gaining `MemoryReadable`.
2. Implement the interned recursive decoder graph, remote-slot description, root read context, shared budget counters, cycle/depth behavior, and structured nested error path. Centralize live-access versus snapshot type projections instead of letting each collection invent them.
3. Integrate the plan with existing class snapshot generation now: snapshot field types and decoders materialize child class values, honor active conditional fields, and do not leave hidden live refs. Support finite recursive schemas without infinitely expanding compiler plans or helper emission.
4. Add pointer width to the managed string object decoder; compute length/characters from the two-word object header. Update helper signatures, registry entries, scratch planning, dependency retention, and all field/snapshot callers.
5. Reuse one object-address decoder from both field-reference reads and nested elements. Avoid double-dereferencing a string object's address.
6. Keep bounded normal `String` and `String?` decoding, with current invalid-UTF-16 replacement semantics. Add the bounded raw UTF-16 storage projection returning `[u16]` for unit-preserving access.
7. Retain declared bounds and scratch-size validation under the root policy. A count above the bound is an error; a failed payload read is not an empty or null string. Add explicit demand edges for each generated decoder and the discovery operations it needs; capability checking alone emits nothing.

**Files:** `src/capabilities.rs`, `src/semantic.rs`, `src/structural.rs`, `src/managed.rs`, capability inference/catalog definitions, `src/codegen/managed_snapshots.rs`, `src/codegen/gc_types.rs`, `src/codegen.rs` snapshot field projection, `src/codegen/runtime_helpers/process.rs`, `src/codegen/runtime_helper_registry.rs`, `src/codegen/runtime_helpers.rs`, `src/codegen/expression.rs`, scratch/dependency planning, syntax/type validation for read policies and storage hints.

**Gate:** capability implication and generic constraints; rejection of unsupported managed representations; recursive class plans and acyclic/cyclic object fixtures; strings on both backends and widths; empty, embedded NUL, surrogate pairs, lone high/low surrogate, negative length, exact bound, over-bound, null versus unreadable pointer; static/live/snapshot/object-address paths; unused string/class readers remain unretained.

### 9. Add managed value arrays and schema decoder integration

1. Extend syntax/AST, type checking, binding plans, diagnostics, formatting, editor support, and documentation for ordinary managed-readable value types, agreed storage hints, and nested read policies. Keep unsupported storage forms rejected until their decoder exists.
2. Add array plan nodes to Step 8's decoder graph, with a child plan rather than a `MemoryReadable`-only element restriction. Reuse them from static fields, live materialization, conditional fields, class snapshots, and other containers.
3. Arrays: dereference the field once, skip the two-word object header and bounds word, read length at target pointer width, and read data after four pointer words. Check length before narrowing, multiplication, allocation, or payload reads. Implement vectors and recursively composed jagged arrays; rectangular/multidimensional CLR array storage is a separate feature.
4. Decode inline fixed-layout primitives/enums/structs with existing checked memory-layout machinery; reference elements call their child managed decoder. Managed `char` is `u16`, managed bool occupies one byte, and target references are pointer-width slots; do not infer these from host/Wasm type sizes. Distinguish unboxed managed value layout from arbitrary local struct layout and require an explicit layout where it cannot be proven.
5. Charge outer and inner arrays, strings, class objects, and scanned slots to the same root budget. Publish a freshly allocated result only on complete success. Preserve previous accepted state on failure using existing state semantics; ensure snapshot-owned nested containers cannot be mutated through an alias.

**Gate:** `[String]`, `[[String?]?]`, and arrays of class snapshots; zero/exact/over-bound arrays, full-width high length bits, arithmetic overflow, invalid element representations, nontrivial inline/pointer stride, failed nested payload read, null at each level, shared-budget exhaustion, and `old` stability after remote mutation or attempted local alias mutation. Replace the existing blanket `[String] coinFlags` rejection fixture with positive bounded managed decoding and focused unsupported-shape/budget diagnostics; `process.read<[String]>` must still fail its `MemoryReadable` constraint.

### 10. Add lists and complete live/snapshot container projections

1. Resolve a genuine `System.Collections.Generic.List\`1` ancestor from the live object's runtime class; support derived lists and reject unrelated lookalikes.
2. Cache `_items`/`_size` layout by attachment and actual concrete runtime class. Resolve lazily when a non-null object first appears; null/late-created collections must not force the entire schema to wait forever before unrelated fields can bind. Revalidate cache selection if a replacement object's runtime class changes.
3. Read live size and backing array capacity separately. Capacity may exceed the declared output bound; live size may not. Reject negative size, missing backing storage where required, and size beyond backing length.
4. Add list nodes that invoke the same recursive child decoder as arrays. Preserve null slots where the element type is optional, with no compaction. Strings, nested arrays/lists/maps, and class values compose without per-combination code paths; unsupported collection nodes remain unavailable until their steps land.
5. Materializing `List<C>` as `[C]` produces class snapshots; an explicit live view can produce `[C.Ref]`. Root class `.snapshot()` recursively uses value projections at every container/class level, without manual per-element conversions. Propagate failures to the root, preserve null positions, and prevent live refs escaping attachment scope. Follow only source-declared fields, with Step 8's cycle/depth rules.
6. Apply both per-string and shared root byte/element/object/work limits to nested collections. Cache discovery by runtime shape, but never cache previously read payload as a new snapshot.

**Gate:** derived/lookalike lists, inflated generic lists, oversized backing with small live count, torn resize, null elements, x86 stride, replacement objects/classes, nested list/array/string/class snapshots, and compiler rejection of retained live-reference collections. Confirm metadata reads stop after successful shape caching and inspecting a materialized snapshot performs no process reads.

### 11. Add entry-array dictionaries and slot-array hash sets

1. Resolve actual `Dictionary\`2`/`HashSet\`1` ancestors and both naming generations described in the PR matrix. Require shape-defining members such as buckets even when the reader does not use the buckets directly.
2. Follow the backing field's type to the inflated entry/slot class using Step 7. Compute stride from instance size minus boxed header, and normalize member offsets the same way.
3. Validate positive stride, integer fields, member room, non-overflowing offsets, and supported remote element encodings before storing a shape. Upstream uses a 1024-byte entry scratch bound; choose/document SplitScript's bound and test boundary values. Inline key/value widths must fit their members; reference-valued keys/values occupy target pointer width regardless of their eventual local size. Child plans handle materialization after slot validation.
4. Dictionaries scan allocated entries and expect `count - freeCount` live pairs. Sets scan through the high-water index and expect `count` live values. Validate signed counts, ordering, backing reach, and output limits before scanning.
5. Skip entries with hash `u32::MAX` or `next < -1`, covering the supported freed-entry conventions. Require exact observed/live tally; never silently return a short or truncated result. Accept canonical empty unallocated entry/slot storage.
6. Cache by concrete collection type and element decoder, not just field offset or collection kind. Distinct generic instantiations may have different strides/layouts.
7. Construct ordinary local `Map<K,V>`/`Set<T>` values by recursively invoking key/value/element plans. Support `Map<String, [String]>`, `[Map<String, [String]>]`, nested maps, and maps containing declared class snapshots without special readers for those combinations. Validate duplicate decoded keys/elements under local equality and fail rather than lose entries; provide the lossless pair/value projection separately. ASR's generic raw reader does not supply managed comparer semantics.
8. Bound work by **allocated/scanned slots**, not just live results. ASR permits up to `1 << 20` slots; a direct synchronous port could violate SplitScript's update budget. Use a documented finite synchronous scan limit for state reads, and a cancellable async snapshot API for larger reads if needed. Measure and set the actual limits before declaring the reader ready.

**Gate:** both naming families/backends/widths; derived and impostor types; wrong field types, incomplete classes, scrambled/oversized layouts, holes and each freed marker, all-deleted collections, empty unallocated storage, live-size versus backing-size limits, exact count mismatch, nested reference/value decoding, comparer collisions, cache invalidation, and measured per-update work. A declared class snapshot containing a map of string arrays is mandatory. Retained snapshots must not share mutable construction buffers or retain nested live references.

### 12. Add old corlib parallel-array collections

1. Dictionaries: resolve `table`, `linkSlots`, `keySlots`, `valueSlots`, `touchedSlots`, and `count`.
2. Sets: resolve `table`, `links`, `slots`, `touched`, and `count`.
3. Resolve the Link value layout through metadata, remove the boxed header, and require an eight-byte pair of `HashCode`/`Next` in either `(0,4)` or `(4,0)` order.
4. Validate `0 <= count <= touched`, all required backing lengths, element strides, and work/output bounds. Walk touched links and use the high-bit live flag; read matching key/value/slot indices only.
5. Reuse recursive typed decoding, root transaction semantics, caches, and shared scan budgets from Step 11. A nested value behaves identically whether its dictionary uses entry structs or parallel arrays. Explicitly fixture the supported empty representation for this shape rather than assuming the unallocated-entry-array special case applies identically.

**Gate:** both link-member orders, holes, high-bit liveness, unequal backing lengths, oversized touched count, live tally mismatch, reference-width correctness, and old Mono integration. Support is advertised per discovered shape, not per guessed Unity year.

### 13. Complete Linux and macOS Mono discovery/profile integration

1. Linux: discover `libmono.so`/`libmonobdwgc-2.0.so`, resolve `mono_assembly_foreach` from mapped ELF symbols, and implement the x64 `48 8B 3D` assembly-list route with proper module/load-bias handling.
2. Import the final Linux measured table and additional measured generic/type members in applicable fallback layouts. Prefer the runtime's own ID; use `UnityPlayer.so` only when no runtime ID is readable. Unknown runtime ID + known player ID is a required negative case.
3. macOS: discover `libmono.0.dylib`/`libmonobdwgc-2.0.dylib`, resolve `_mono_assembly_foreach`, and implement the mapped Mach-O x86_64 and arm64 discovery paths. For ARM64, validate instruction masks and correctly decode signed PC-relative page displacement and scaled load offset; do not reuse x64 arithmetic merely because both are 64-bit.
4. Import both measured 6000.5.10f1 UUID entries and their complete layout. Select the active architecture slice and keep architecture distinct from pointer width.
5. Port the relevant existing ASR family-detection and fallback capabilities needed for unmeasured Mono libraries, with explicit format/architecture support limits. Do not claim 32-bit ELF/Mach-O runtime attachment just because the identity parser can read a format.
6. Audit process/module/range reporting in the native host and generated runtime on these platforms. Reuse host imports where possible; update ABI and host implementations only if an actual missing primitive is found. Require real platform smoke validation of module bases, sizes, symbol addresses, and readable ranges.

**Gate:** synthetic ELF/Mach-O identity + symbol + attachment + metadata tests run in CI; native Linux x64 and Mac x86_64/arm64 smoke results are recorded separately. On platforms not available to the implementer, track the missing live validation explicitly rather than presenting synthetic success as real-game confirmation.

### 14. Validate complete behavior, size, tooling, and documentation

1. Register new `.split`/Node fixtures in `src/bin/xtask.rs` and test relevant Debug and Release output. Run `cargo xtask conformance`; use `cargo xtask check` for the full repository gate, including formatting, clippy, docs, and extension validation.
2. Check `ManagedReadable` implication and generic constraints, recursive storage/result projections, nested bound diagnostics, recursive schema planning, process-reference escape rules, conditional fields, metadata ambiguity, and transitive helper reachability in `tests/compiler/`.
3. Check generated Wasm validation plus observable host behavior: metadata-read counts, scan/read budgets, cancellation, process replacement, all-or-error collection publication, state failure retention, stable `old`, and no new timer actions caused by initialization failures.
4. Keep existing Lunistice base/DLC, inherited-static, Mono instances, IL2CPP instances, scene, and typed-component fixtures passing after API updates. Update `examples/lunistice.split`, other explicit Unity scripts, and their documentation to measured profile selectors or automatic detection based on verified targets, not a blind year-to-profile mapping.
5. Review the accumulated **per-step** Lunistice and feature-isolation size reports, not just a final total. Verify every positive delta is attributed to reachable work and that unused resolvers/readers/data never appeared along the way. An explicit single profile must not pull all profiles/readers into the final Wasm; a string-only script must not contain dictionary offset discovery; ordinary field reads must not trigger repeated metadata discovery.
6. Run controlled real-game or recorded-memory probes for representative old/new Mono, old inline/new handle IL2CPP, x86/x64, and each collection family. Record exact binary identity/player version, selected layout, expected values, provenance, and environment. Reuse available Lunistice evidence only after verifying what it actually covers.
7. Update `docs/STANDARD_LIBRARY.md`, `docs/COMPILER.md`, `docs/LANGUAGE.md`, `docs/MIGRATION_CAPABILITIES.md`, `docs/ASL_PORTING.md`, relevant port pages, and generated language/editor documentation. Publish a format/architecture/profile/collection support matrix and explain `MemoryReadable` versus recursive `ManagedReadable`, storage hints, deep snapshots, selection heuristics, root budgets, nested nullability, raw UTF-16, references, comparer differences, cycles, and snapshot race limits.
8. Remove stale numeric-year tables, old selectors, unreachable helper variants, and claims of unsupported capabilities. No compatibility layer is required.

**Gate:** all supported cells in the matrix below have recorded fixture coverage; required repository checks pass; unavailable live validations are explicitly identified; each PR's behavior is implemented or an intentional language-level difference is documented.

## Verification matrix and completion criteria

Avoid a huge blind Cartesian product: test every profile's data/selection invariants, every distinct runtime layout/discovery family end to end, and every reader against each materially different width/backend/shape.

| Dimension | Required coverage |
| --- | --- |
| Mono selection | Known/unknown/missing GUID+age; width mismatch; V1/V1Cattrs/V2/V3; ELF runtime identity versus player fallback; active Mach-O slice UUID |
| IL2CPP selection | Every exact profile; same-major/minor later patch; between families; older/newer than corpus; 16-bit rejection; explicit/custom profiles; no unnecessary auto-table retention |
| Discovery | PE x86/x64 signatures and exports; ELF x64 symbols/load bias; Mach-O x86_64/arm64 symbols/instructions; false matches; module edges; delayed metadata; cancellation |
| Metadata | Assembly-name routes; inline/handle type start; sparse class tables; nested declaring chains; generic counts/types; inherited owner; ambiguity; corrupt/cyclic metadata |
| Readers | Both widths; recursive value/reference arrays and lists; jagged arrays; both entry/slot naming generations; parallel arrays; strings/raw UTF-16; null/empty/capacity/length/tally errors; failed nested reads; local comparer collisions |
| Recursive composition | Map of string arrays; array of maps; map of maps; list of optional class snapshots with nested maps; scalar/string/map mixtures; null at outer/inner/element levels; shared child versus actual cycle; shared total-budget exhaustion |
| Runtime integration | Static/live/conditional/deep-snapshot paths; collection shape caching; replacement class; current/old stability after remote and attempted local mutation; failure at a deep leaf rejects the root; process restart; instances/components |
| Cost | Bounded attachment polls and shared root scan/decode work; no per-tick metadata walks after cache resolution; per-step Lunistice size deltas; unused feature discovery/readers/data/scratch absent; exact transitive demand for nested decoders |

The implementation is complete when all 14 PRs have an outcome in this checklist:

- [ ] #142 identities and equivalent generated-Wasm test infrastructure.
- [ ] #143 Windows Mono exact profiles and image-name routing.
- [ ] #144 measured IL2CPP layouts, x86 discovery, and corrected reads; superseded selection intentionally omitted.
- [ ] #145 shared metadata operations with SplitScript-specific scheduling/ambiguity semantics.
- [ ] #146 nested/generic handling and owner-aware static regression coverage.
- [ ] #147 width-correct strings, raw UTF-16 access, and bounded value arrays.
- [ ] #148 runtime-validated lists with cached layouts.
- [ ] #150 validated dictionary layouts and complete live-pair reads.
- [ ] #151 hash sets with correct high-water/count semantics.
- [ ] #152 old parallel-array shapes and distinct liveness rules.
- [ ] #153 reference array/list elements and direct string-object decoding.
- [ ] #155 Linux exact identities/profiles and required discovery support.
- [ ] #156 macOS UUID profiles and required discovery support.
- [ ] #160 complete explicit/custom IL2CPP profiles, final auto selection, and removal of year buckets.
- [ ] Beyond ASR: recursive `ManagedReadable`, with `MemoryReadable` base cases and generic constraint support.
- [ ] Beyond ASR: natural nested arrays/lists/maps/sets/strings and declared class snapshots through one decoder graph.
- [ ] Beyond ASR: transitive immutable snapshot ownership, per-root budgets, nested nullability/failures, cycle handling, and retained-state safety.
- [ ] Every implementation step has a Lunistice size/behavior report; all positive size deltas have a reachable-feature explanation.
- [ ] Unused feature resolvers, readers, metadata names, profile data, GC types, and scratch storage are absent from generated Wasm.

## Source map for implementation

Upstream links below are pinned to the reviewed tip; the PR table provides the historical changes.

- [Binary identities: PE](https://github.com/LiveSplit/asr/blob/cf732d3aeac7509c8ab5f29a4d0d28f16487d245/src/file_format/pe.rs), [ELF](https://github.com/LiveSplit/asr/blob/cf732d3aeac7509c8ab5f29a4d0d28f16487d245/src/file_format/elf.rs), [Mach-O](https://github.com/LiveSplit/asr/blob/cf732d3aeac7509c8ab5f29a4d0d28f16487d245/src/file_format/macho.rs).
- [Mono attachment and identity precedence](https://github.com/LiveSplit/asr/blob/cf732d3aeac7509c8ab5f29a4d0d28f16487d245/src/game_engine/unity/mono/mod.rs), [Windows builds](https://github.com/LiveSplit/asr/blob/cf732d3aeac7509c8ab5f29a4d0d28f16487d245/src/game_engine/unity/mono/builds.rs), [Linux builds](https://github.com/LiveSplit/asr/blob/cf732d3aeac7509c8ab5f29a4d0d28f16487d245/src/game_engine/unity/mono/linux_builds.rs), [Mac builds](https://github.com/LiveSplit/asr/blob/cf732d3aeac7509c8ab5f29a4d0d28f16487d245/src/game_engine/unity/mono/mac_builds.rs).
- [IL2CPP attachment](https://github.com/LiveSplit/asr/blob/cf732d3aeac7509c8ab5f29a4d0d28f16487d245/src/game_engine/unity/il2cpp/mod.rs), [complete profile schema](https://github.com/LiveSplit/asr/blob/cf732d3aeac7509c8ab5f29a4d0d28f16487d245/src/game_engine/unity/il2cpp/offsets.rs), [named profiles](https://github.com/LiveSplit/asr/blob/cf732d3aeac7509c8ab5f29a4d0d28f16487d245/src/game_engine/unity/il2cpp/profiles.rs).
- [Shared walk and collection shape resolution](https://github.com/LiveSplit/asr/blob/cf732d3aeac7509c8ab5f29a4d0d28f16487d245/src/game_engine/unity/managed/walk.rs), [backend operations](https://github.com/LiveSplit/asr/blob/cf732d3aeac7509c8ab5f29a4d0d28f16487d245/src/game_engine/unity/managed/runtime.rs), [cursors](https://github.com/LiveSplit/asr/blob/cf732d3aeac7509c8ab5f29a4d0d28f16487d245/src/game_engine/unity/managed/cursor.rs), [pointer paths](https://github.com/LiveSplit/asr/blob/cf732d3aeac7509c8ab5f29a4d0d28f16487d245/src/game_engine/unity/managed/pointer.rs).
- [Shared value readers](https://github.com/LiveSplit/asr/blob/cf732d3aeac7509c8ab5f29a4d0d28f16487d245/src/game_engine/unity/managed/readers.rs), [raw managed strings](https://github.com/LiveSplit/asr/blob/cf732d3aeac7509c8ab5f29a4d0d28f16487d245/src/game_engine/unity/managed/string.rs). The Mono/IL2CPP directories contain `walk_tests.rs`, `readers_tests.rs`, and `collections_tests.rs`; Mono also has `identity_tests.rs`.
- SplitScript entry points: [standard library](../stdlib/standard.split), [managed binding plan](../src/managed.rs), [remote-memory validation](../src/validation.rs), [IL2CPP layout facts](../src/codegen/unity_layout.rs), [IL2CPP helpers](../src/codegen/runtime_helpers/unity.rs), [managed string decoding](../src/codegen/runtime_helpers/process.rs), [field expression lowering](../src/codegen/expression.rs), [snapshot generation](../src/codegen/managed_snapshots.rs), [static read caching](../src/codegen/managed_state_reads.rs), [async lowering](../src/codegen/async_state.rs), [fixture registration](../src/bin/xtask.rs).

The intended result is one coherent Unity implementation with complete selected layouts, recursive managed-readable values, deep class snapshots, bounded shared read contexts, and generated Wasm that pays only for reachable features. The work is a source/compiler migration extending ASR's capabilities, not a dependency update or a literal transplant of ASR's Rust public API.
