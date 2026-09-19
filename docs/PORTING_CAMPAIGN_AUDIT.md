# ASL porting campaign audits

This audit classifies the candidates described by the earlier external report,
now archived as `old/PORTING_FEEDBACK.md` in the porting workspace. They were
produced with compiler revision
`69f2bd9d3eb4`. The audit was performed against revision `b82638c`, the source
ASL corpus, the current compiler, and maintained in-tree ports and host
fixtures. The campaign was not rerun: its reports are evidence about the
workflow and diagnostics that the porter actually experienced.

The status terms are deliberately strict:

- **compile-only** means the candidate compiled, but no runtime fixture or live
  game established behavior;
- **behavior-limited** means review found a known omission or semantic change;
- **runtime-verified** means a maintained in-tree port has deterministic host
  coverage for the behavior named here, not that the external candidate or a
  live game was tested;
- **intentionally failing** is reserved for a focused diagnostic probe. None of
  the six campaign candidates is in that category.

## Candidate classifications

| Candidate | Campaign status | Current in-tree evidence | Review result |
| --- | --- | --- | --- |
| Arietta of Spirits | **compile-only** | **runtime-verified** core port | The candidate's extensionless Windows process identity does not satisfy the current exact-name host contract. The ASL version label has no runtime role with one physical layout, and its empty lifecycle blocks are correctly absent. |
| Aquanox | **behavior-limited** | **runtime-verified** core port | Start, split, reset, loading, optional string reads, and detach cleanup are covered. The startup dialog and timing-method read/write remain unavailable host capabilities. |
| Operation Matriarchy | **compile-only** | **runtime-verified** maintained port | The candidate has no runtime evidence and retains mutable values across attachment. The maintained port resets them in `onAttach`, validates the split filename shape, and uses the exact Windows process candidate expected by its fixture. |
| A Plague Tale: Innocence | **behavior-limited** | **runtime-verified** maintained port for Steam, Epic, Xbox, and unsupported layouts | The candidate approximates an exact timer-start event with one game-memory transition and does not make the `APT` parent setting gate its children. The maintained port observes the timer-state transition and explicitly gates every child, but exact host timer events still require runtime support. The startup dialog and timing-method mutation remain omitted. |
| Axiom Verge | **behavior-limited** | **runtime-verified** maintained Steam/vanilla scenario; other build branches are compile-covered | The candidate is a reduced demonstration, not a faithful port. It omits `RandomAV`, three of four distribution/process offsets, most settings, and exact timer events. It scans only the main module, omits the scan result's four-byte adjustment, passes a byte count where `readUtf16Le` expects code units, truncates fractional game ticks, and fails to advance checkpoint/item counters past known disabled settings. The maintained port corrects the memory, settings, and build-selection issues, approximates start resets through a timer-state transition, and registers all 119 boolean settings. Dialog/timing-method behavior and exact host timer events remain unavailable. |
| Neon White | **compile-only** | **runtime-verified** maintained port | The candidate's shadow-global transcription remains compile-only. The independently reconstructed maintained port uses direct current-snapshot replacement and covers filtering, timing, lifecycle decisions, failed reads, and reattachment with a deterministic host fixture. Its reviewed offsets still represent only one build and have not been live-game validated. |

All six candidates retain the ASL spelling of their native process name. The
current Windows host reports executable filenames including `.exe`, so those
extensionless candidates do not attach there. Portable cross-platform process
identity remains R3 host work, but documentation and audits must describe the
exact-name contract that exists today.

## Docs-only campaign at revision `87d3650`

The later docs-only campaign produced 75 compiling files but no live-runtime
evidence. Its report often classified a clean compile as `PORTED`, and several
entries declared existing facilities unavailable. This first tranche checks
five high-risk candidates against their ASL sources and the current compiler.

The findings below use these classifications. None dismisses the campaign
report: difficulty finding an existing facility is actionable product evidence
just as much as a missing facility is.

- **discoverability failure** means the language supports the source behavior,
  but the documentation, search, compiler guidance, editor, or API shape did
  not lead the porter to it;
- **port bug** means the translation changed behavior despite sufficient
  existing facilities;
- **host gap** means preserving the behavior needs a runtime contract;
- **language or library question** means a public design decision remains and
  this audit does not choose it;
- **policy** means the difference is intentional and documented.

The existing facilities were compiled together in one probe: alternate exact
process names, build enums initialized through `process.name()`, `tickRate`,
`timer.state()`, and schema-declared bounded managed strings all compose in one source
file.

### First-tranche classifications

| Candidate | Campaign result | Audit result |
| --- | --- | --- |
| Arietta of Spirits | `PORTED` | The extensionless Windows provider compiles but does not attach under the current host contract. This is an exact-name **discoverability failure**. Omitting the single ASL version label remains a reasonable **policy** until another physical layout is supported. |
| TUNIC | `PORTED-LIMITED` | The campaign could not find how one file accepts both `TUNIC` and `Secret Legend`; one state candidate array supports both exact executable names. The port also manually decodes a Mono string instead of declaring a bounded managed `String` field. Both are **discoverability failures**. Missing Unity scene behavior is a **language or library question**, and exact `onStart` cleanup remains a **host gap**. |
| A Proof of Concept | `PORTED-LIMITED` as two files | One state candidate array, an ordinary build enum initialized from `process.name()`, and conditional fields can represent both executables in one file. Producing separate files is a **discoverability failure** around the combined attachment/shape workflow. Timer run-offset mutation and exact `onStart` restoration remain **host gaps**. |
| Aim Climb | `PORTED-LIMITED` | Dropping ASL's 60 Hz `refreshRate` is a **discoverability failure** because `tickRate { attached: 60 }` owns this lifecycle behavior. The module-qualified `.exe` reads do not repair the extensionless provider. Dynamic lookup of a statically declared setting is valid but misses its typed member. |
| 25 To Life | `PORTED-LIMITED` | Missing `timer.state() == TimerState.NotRunning` is a **discoverability failure** because it can preserve the source's accumulator reset. Accumulating on every positive IGT rollback instead of only a positive-to-zero boundary is a **port bug**. The provider is also extensionless. |

### Behavior ledger

#### Arietta of Spirits

- Source: one process identity and one labelled physical layout.
- Candidate: the same extensionless process identity, with the label omitted.
- Correction: use and live-verify the exact Windows executable filename,
  including `.exe`. No shape discriminator is needed until another build exists.

This is a silent failure category: type checking says nothing about whether the
host will ever discover the declared process.

#### TUNIC

- Source: `TUNIC` and `Secret Legend` are alternate attachment identities.
- Candidate: only extensionless `TUNIC`, with a report claiming the second name
  requires another generated file.
- Correction: declare both exact executable names in one state candidate array.
- Source: UnityASL reads `LastEvent` as a managed string.
- Candidate: hard-coded Mono object offsets followed by `readUtf16Le`.
- Correction: declare the field as `String lastEvent;` in the Unity schema and use its typed fallible field path.
- Remaining question: scene transitions arm starting and drive area splits, but
  the candidate removes scene state and starts on any timer-running edge. A
  Unity scene facility needs a separate design discussion.
- Remaining host gap: source `onStart` clears run-scoped event history even for
  starts outside the script's `start` block. The candidate clears it only on a
  game-state reset.

The current candidate must not be described as preserving start or area-split
behavior.

#### A Proof of Concept

- Source: one script supports two executable identities, two `LevelID`
  addresses, two route tables, and identity-specific start/reset rules.
- Candidate: two independent SplitScript files because the porter believed one
  provider could not represent both.
- Correction: use an alternate-name provider, one build enum variant and
  conditional field branch per executable, then initialize the enum from
  `process.name()` in `onAttach`. Keep the distinct route mapping behind that
  selection.
- Remaining host gap: saving, replacing, and restoring the timer run offset
  requires timer configuration and exact `onStart` support. It must not be
  approximated through game time.

#### Aim Climb

- Source: `refreshRate = 60` and one statically declared `Split` setting.
- Candidate: polling is omitted as allegedly host-controlled, and the setting
  is queried dynamically by its host key.
- Correction: use `tickRate { attached: 60 }` and prefer the typed setting
  member. Use the exact `.exe` filename in the provider itself; having `.exe` in
  module-qualified field paths does not affect attachment discovery.

No new host facility is needed for this source behavior.

#### 25 To Life

- Source: clear cumulative game time whenever the timer is not running, and add
  old IGT only when the new IGT is exactly zero.
- Candidate: clear only on the game-level reset predicate and accumulate on any
  decrease from a positive value.
- Correction: use `timer.state()` in `whileAttached` for the timer reset, and
  preserve the narrower positive-to-zero accumulation condition unless live
  testing establishes that the ASL source itself is wrong. Verify and use the
  exact `.exe` provider identity.

This case demonstrates why adding an API based only on campaign feedback would
be harmful: the required timer-phase API already exists.

### Next audit tranche

#### Age of Empires II: Definitive Edition

Campaign status: `BLOCKED`

Audit result: mostly a false aggregate blocker with two narrower host gaps.

- Source: four version-labelled layouts selected from the main executable's PE
  file version.
- Campaign claim: executable file-version metadata and alternate layout
  selection are unavailable.
- Existing translation: declare a four-variant build enum, read
  `process.mainModule().fileVersion()` in `onAttach`, compare `FileVersion`
  values with version literals, assign the matching build variant, and guard
  version-specific fields with it.
  The exact Windows process candidate must be `AoE2DE_s.exe` under the current
  host contract.
- Current compiler result: `Process.closed()` now completes as `async Never`,
  so `module.fileVersion() else await process.closed()` has type
  `FileVersion` and provides a clean unsupported-build path without a fake
  version value. A braced fallback block remains separate syntax work; bare
  `else return` is still correctly rejected because it would leave the build
  global uninitialized.
- Source: map and lost-time logic reads `timer.CurrentPhase` and
  `timer.CurrentSplitIndex`.
- Campaign claim: timer phase and split index are unavailable.
- Existing translation: use `timer.state()` and the optional
  `timer.currentSplitIndex()`. The absent split index needs explicit control
  flow and must not be cast from the host's signed sentinel.
- Residual host gap: the final lost-time split compares the current index with
  `timer.Run.Count - 1`. SplitScript does not expose the configured segment
  count, so the script cannot know that the current segment is the last one.
- Residual host gap: `onReset` clears five run-scoped values at the exact timer
  reset event. Polling `TimerState.NotRunning` can reconstruct the stable state
  in ordinary cases, but it is not an ordered lossless reset notification.
- Policy requiring review: the ASL source falls back to its newest layout for
  an unknown executable version. A maintained port must explicitly choose
  whether to preserve that risky fallback or reject the unsupported build with
  `await process.closed()`.

This source is suitable for a behavior-limited port today. The ordinary map
timer, win split, cumulative game time, version comparison, timer-state logic,
and unsupported-build suspension do not need new APIs. Last-segment detection
and exact reset-event semantics remain unavailable host behavior.

### Unsigned memory values: Alkali

Campaign status: `PORTED-LIMITED`

Audit result: the candidate silently narrows every source `ulong` watcher even
though the language supports the exact unsigned representation.

- Source: all seven memory watchers are C# `ulong` values, including the file
  timer. Their faithful SplitScript type is `u64`; the parser's existing
  machine-applicable migration fix rewrites `ulong` to `u64`, including in a
  state field, and a focused compiler probe preserves the full unsigned range.
- Candidate: every watcher is declared as `i64` because the porter did not find
  the compiler's unsigned 64-bit type. Clean compilation therefore hides a
  semantic narrowing for values above `i64::MAX`.
- `Duration.fromMilliseconds` now accepts the `u64` watcher directly and uses
  its exact integer implementation. The candidate's intermediate `f64` cast is
  unnecessary and should be removed; this no longer blocks faithful timing.
- Attachment is independently incorrect: the provider candidate is
  extensionless even though module-qualified fields name `Alkali.exe`. Under
  the current exact host contract, the state candidate itself must be
  `Alkali.exe`.

### Fixed memory arrays: 1001 Spikes Ukampa

Campaign status: `PORTED-LIMITED`

Audit result: the fixed array is supported directly, but the candidate silently
disables one of its two modes.

- Source: `byte35 levelFlags` is one contiguous 35-byte memory watcher. The
  candidate's `[u8; 35] at 0x12BEC0` is the exact typed state representation;
  indexing with `u32` and the bounded level counter preserve its memory shape.
- Source: the timer category selects Any% Ukampa or All Skulls Ukampa. Timer run
  metadata is a real host gap, so replacing that selection with an explicit
  setting is a defensible policy only when the setting actually controls the
  branch.
- Port bug: the candidate declares `allSkullsMode key "allSkulls"` but tests a
  separate `allSkulls` global that is initialized to `false` and never assigned.
  The All Skulls branch can therefore never execute despite a clean compile.
  This is direct evidence for the planned unused-setting and behavior-influence
  lints; it is not evidence for another collection type.
- Policy requiring review: the source's death count is host-visible auxiliary
  output. The candidate drops the `lives` watcher and both display values. That
  does not alter split timing, but a fidelity ledger must record the omitted
  custom-variable behavior rather than silently calling the port complete.
- Attachment remains compile-only. The extensionless Windows process candidate
  must be replaced with and live-verified against the exact `.exe` identity.

### Managed growable collections: Alba

Campaign status: `BLOCKED`

Audit result: the report identifies a real Unity library gap but overstates the
required language feature.

- Source: the settings hierarchy and every task identifier are statically
  authored in `startup`; no runtime settings declaration is required. The
  existing settings DSL can declare them, and `settings.enabled(name)` can
  query a discovered task name when exact static keys are preserved.
- Existing language support: growable arrays and loops can retain discovered
  task addresses, names, required values, and previous readings. Ordinary
  process reads in `whileAttached` can update those arrays and reproduce the
  `MemoryWatcherList` changed-value test. Runtime-created state fields are not
  necessary.
- Existing Mono support provides class discovery, named field offsets, static
  field paths, typed process reads, and managed-string decoding. The unresolved
  boundary is typed traversal of managed `List<T>` and its backing managed
  array without scripts guessing object-header and element-data offsets.
- Design remains open: this audit does not choose a Unity API, target layout,
  or runtime intrinsic. The residual need belongs to the existing managed-list
  roadmap item and must keep Mono versions and target families explicit.

### Genuine host boundary: Amnesia AMFP and TDD

Campaign status: `BLOCKED`

Audit result: the loading-state blocker is genuine and must not be disguised as
an ordinary SplitScript memory helper.

- Source: both scripts locate executable instructions, allocate executable
  process memory, copy and replace instructions, install jumps, suspend and
  resume the game around patching, and restore every modification during
  shutdown. The injected code publishes a synthetic loading flag that the
  scripts combine with readable game state.
- Existing support covers version layouts, alternate exact process identities,
  signatures, pointer resolution, bounded strings, and all ordinary reads. It
  cannot provide the synthetic flag because the host ABI intentionally exposes
  no allocation, arbitrary writes, process suspension, or code-patch lifetime.
- Required host semantics are safe observation of the transient load events,
  cancellation and cleanup when attachment ends or traps, and restoration that
  cannot leave the game patched. This evidence does not justify exposing raw
  executable-memory mutation to scripts; a native loading signal or a tightly
  constrained host-owned instrumentation contract must be evaluated separately.
- AMFP additionally mutates run offset and timing method. Those are independent
  host gaps and should not be bundled into the loading-state design.

### Growable pointer-path composition: Ato

Campaign status: `PORTED-LIMITED`

Audit result: the candidate's path-length branching exposes a discoverability
failure around growable arrays and pointer-path composition.

- Source: `Array.Copy` copies a selected version-specific offset path and adds
  one boss, arena, scroll, or rune offset before constructing a `DeepPointer`.
- Candidate: `readDynamic` branches separately for path lengths 4, 5, 6, and 9
  and reconstructs a fixed literal in every branch. Its feedback claims array
  construction is static and asks for slice/append support.
- Existing translation: create `let fullPath: [i64] = []`, call
  `fullPath.extend(path)`, call `fullPath.push(lastOffset)`, and pass the result
  to `process.follow(base, fullPath)`. A focused current-compiler probe validates
  that exact generic function and state-field call.
- The branch explosion therefore records a **discoverability failure**. The
  porting guide now puts growable-array composition next to dynamic
  `DeepPointer` migration rather than expecting authors to connect separate
  pointer and collection chapters.
- Ato's timer event and mutation omissions remain independent host gaps; the
  array correction does not improve their fidelity.

### Version-selection discoverability failure: Borderlands

Campaign status: `BLOCKED`

Audit result: the complete active loading behavior is supported by the current
language and standard library.

- Source: two labelled layouts contain the same two boolean fields at different
  addresses. `modules.First().FileVersionInfo.FileVersion` selects patch 1.0 for
  `1.0.0.0` and patch 1.4.1 for `1.5.0.0`; loading is the OR of both fields.
- Existing translation: attach to exact `Borderlands.exe`, declare a build enum
  and two conditional field branches, read `process.mainModule().fileVersion()`
  in `onAttach`, assign the matching variant, and use `await process.closed()` for read
  failure or an unsupported version. A focused current-compiler probe validates
  this complete shape.
- The report could not find executable file-version dispatch. This is a
  **discoverability failure**, and no module-size approximation or new host
  selector is needed.
- `FileVersion` literals are first-class match patterns, so version selection
  maps directly to `v"1.0.0.0" => Build.Patch100` arms. Because executable
  versions form an open value space, the match must include `_` for unsupported
  builds; no string parsing or chained comparison workaround is required.
- The source's `doStart` value is initialized but unused. There are no active
  start, split, reset, settings, or timer-event behaviors hidden behind the
  version blocker. Runtime verification is still required for both address
  layouts and exact PE version values.

### Timer-metadata discoverability failure: Castle of Illusion HD

Campaign status: `BLOCKED`

Audit result: every active timer decision is expressible with the current
language and standard library.

- Source: three ordinary memory watchers drive start, loading, boss history,
  cutscene history, and 23 route-position split predicates. Under the current
  exact-name contract the attachment candidate is `COI.exe`, not the source's
  extensionless ASL process identity `COI`.
- Campaign claim: the port requires both current split metadata and arbitrary
  mutation of fields on the current sample. The source does assign
  `timer.CurrentSplit.Name`, but never reads that local; no segment-name API is
  part of its active behavior. Its route dispatch only needs the existing
  optional `timer.currentSplitIndex()`.
- The source-created `previousBossHP` and `cutsceneCount` sample properties are
  run-owned history rather than process memory. Typed globals can hold them,
  while locals captured before each update preserve the source's `old` view
  before the globals are mutated. Resetting them for route index zero and at
  the same post-split boundaries preserves the source decisions. The assigned
  `previousCutsceneStatus` property is never read and can be omitted.
- A focused current-compiler probe covers the exact process and pointer paths,
  optional split-index fallback, all 23 expected-level arms, boss and cutscene
  history, final split, start, and loading behavior. It compiles without a new
  timer, snapshot-mutation, or dynamic-field facility.
- The empty ASL `gameTime` block has no behavior to preserve. Live-game
  validation is still required for the bounded level-name encoding, pointer
  paths, and route transitions; successful compilation is not runtime proof.

### Formerly fingerprint-limited layouts: COTM and COTM2

Campaign status: exact build selection is now expressible; complete ports remain
to be implemented and validated.

Audit result: the timer and layout claims were false aggregate blockers, and
the one exact-build-selection requirement now has a direct language API.

- Both sources declare two named physical layouts for one Windows executable.
  SplitScript can represent those shapes with one ordinary build enum and
  conditional fields, expose their common state fields, and initialize the
  selected build in `onAttach`. Their exact
  current attachment candidates are `COTM.exe` and `game.exe`.
- COTM's reset logic uses `timer.CurrentPhase`, and its stage and boss-rush
  routes use `timer.CurrentSplitIndex`. These map to the existing exhaustive
  `timer.state()` and optional `timer.currentSplitIndex()` APIs. COTM2 does not
  read either timer value at all, despite the campaign report naming both as
  blockers.
- COTM2's save-slot-dependent `DeepPointer` values do not require dynamic state
  declarations. A selected version-specific base, ordinary address arithmetic,
  `process.follow`, typed globals for the one-tick-old derived values, and
  process reads in `whileAttached` preserve that update shape. Its settings are
  statically known and fit the existing settings DSL.
- The formerly residual gap was exact layout evidence. Both ASL sources hash
  the complete executable and compare known MD5 values; COTM explicitly notes
  that module size is identical between its builds. The corpus provides no
  equivalent PE file-version or stable signature evidence, so silently
  choosing by size, always selecting the newest layout, or inventing an
  address-validity probe would change the source's build policy.
- `Module.md5()` now preserves that evidence directly: it hashes the exact
  complete on-disk file through bounded cooperative reads, restarts on an
  observed file change, and returns the uppercase spelling used by both
  sources. This removes the fingerprint blocker without unrestricted
  filesystem access or a whole-module synchronous tick. The remaining work is
  to implement and live-validate both ports and their pointer layouts.

### Existing managed-instance paths: Circuit Superstars

Campaign status: `BLOCKED`

Audit result: the managed path fits the canonical Unity schema; exact timer
start notification is the only active behavior that remains host-limited.

- Source: UnityASL finds `RaceManager.Instance`, reads two fields on that
  replaceable singleton, and follows its `localStage` reference to
  `RaceStage.State`. This is not unstructured runtime reflection after
  attachment; every class and field name is statically known.
- Existing translation: declare `RaceManager.Instance`, its scalar fields, the
  `localStage` class reference, and `RaceStage.State` in one top-level managed
  schema. Generated live references reread the singleton and nested object on
  every poll, so replacement does not leave a stale cached address.
- A focused current-compiler probe covers both singleton fields, the nested
  `localStage -> State` reference, fallible reads at the state boundary,
  start/split decisions, accumulated game time, and the always-paused loading
  clock. It compiles without a new instance-binding API or dynamic state field.
- The source's `onStart` callback clears accumulated time and rearms splitting.
  A SplitScript `start` block can do the same for starts initiated by that
  script, but it cannot observe an external timer start exactly. R2 remains the
  faithful gap for that event; it is independent of Unity traversal.
- Attachment must use the exact current Windows identity
  `circuit-superstars.exe`. The correct Mono layout family and managed field
  behavior still require live-game validation. Attachment-owned paths are
  replaced on every successful `onAttach`, so the helper's explicit reset does
  not require a guest-visible managed-object lifetime API.

### Background pointer discovery without dynamic watchers: AER

Campaign status: `BLOCKED`

Audit result: the report mistakes raw replaceable `DeepPointer` chains for a
managed-object and dynamic-watcher requirement. The active loading behavior is
already expressible.

- Source: both the `Teleporter` and later-created `LoadingScreen` values are
  found through fixed 32-bit pointer paths rooted in `mono.dll`. The source does
  not query Mono class or field metadata. Its background task only retries the
  latter path until it resolves, then replaces a placeholder boolean watcher.
- Existing translation: retain three `MemoryPath?` globals, populate them from
  `await process.module("mono.dll")` in `onAttach`, and resolve each path from a
  boolean expression-backed state field. A focused current-compiler probe
  covers both pointer shapes and combines all three flags in `isLoading`.
- Read semantics: the source configures every watcher with
  `ReadFailAction.SetZeroOrNull`. A helper that uses `path.resolve() else return
  false` and `process.read<bool>(address) else false` reproduces that behavior;
  ordinary state failure retention would be the wrong choice here.
- Polling and cancellation are existing facilities. The source's integer
  `refreshRate` calculations select 2 Hz while detached and 58 Hz while
  attached, which map to one `tickRate` declaration. Process closure cancels
  attachment-owned discovery, and the paths are replaced on the next successful
  `onAttach`, so no cancellation-token or shutdown API is required for the
  loading provider.
- The embedded sound, modal message, version label, and optional debug logging
  are auxiliary UI behavior rather than inputs to the timer decisions. Omitting
  them must be recorded in a fidelity ledger, but they do not block the loading
  remover.
- Attachment under the current exact-name contract is `AER.exe`. The pointer
  width, offsets, and flag timing still require live-game verification; the
  compiler probe proves representability, not runtime correctness.

### Finite settings and static singleton paths: Bzzzt

Campaign status: `BLOCKED`

Audit result: both reported blockers are existing facilities. The active timer
behavior is representable with finite settings declarations and the canonical
managed schema.

- Source: `startup` registers the fixed keys `"1"` through `"51"`; only levels
  13, 26, and 39 default to enabled. This is bounded declaration data despite
  being written as a C# loop. Compile-time settings families cover each uniform
  range, while three ordinary declarations preserve the exceptional defaults.
  `settings.enabled(current.level as String)` performs the same dynamic lookup,
  gated by the source's `levels` parent setting.
- Source: UnityASL follows the static `Main.instance` singleton and reads four
  fields declared on `Main`. A top-level `Main` schema declares that singleton
  and its fields directly; generated live references observe a replaced
  singleton rather than caching one object address at attachment time.
- A focused current-compiler probe covers all four fields, the partitioned level
  settings, computed string-key lookup, and the original start, split, and reset
  predicates. The source's `Log` watcher and scene-helper opt-in do not feed any
  active timer decision.
- Attachment under the current exact-name contract is `Bzzzt.exe`. The correct
  Mono layout family, class and field spellings, and runtime behavior still
  require live-game validation. The uniform-default limitation makes this
  particular family verbose, but it is an ergonomics question rather than a
  runtime-registration blocker.

### Cross-class paths and scene snapshots: Assemble with Care

Campaign status: `BLOCKED`

Audit result: the source exposes two distinct requirements. Loading-scene
snapshots remain timer-critical, while its unusual cross-class static path is
also a managed-schema gap now that raw metadata traversal is intentionally
private.

- Source: the generic `Service` base class declares `_instance`, the concrete
  `LevelFlowService` class supplies the static storage and `_state` instance
  field, and `GameStart.forceReloadGame` is an ordinary static boolean. The
  metadata comes from multiple classes, but the resulting reads are still one
  static slot followed by one managed pointer dereference.
- The ordinary `GameStart.forceReloadGame` static field fits the current schema.
  The `_instance` slot does not: it is declared by the generic base while its
  static storage belongs to the concrete class. Preserve this as evidence for
  an approved cross-class schema design rather than restoring public
  `staticTable`, raw field offsets, or `MemoryPath` composition.
- Residual gap: every split condition branches on
  `vars.Unity.Scenes.Loading[0].Index`. No process-memory substitute in the
  source identifies that value, and SplitScript does not yet expose a typed
  Unity active/loading scene snapshot. A faithful port remains blocked on the
  existing scene-provider roadmap item. The port therefore has two focused
  schema/provider blockers rather than a need for a generic public metadata API.
- Attachment under the current exact-name contract is `AWC.exe`. Mono layout,
  class names, offsets, scene semantics, and lifecycle timing still require
  live-game validation once the scene provider exists.

### Alternate executable layouts: Crazy Machines

Campaign status: `PORTED-LIMITED`

Audit result: the omitted alternate executables exposed weak discoverability
for existing state features and one genuine language papercut. The complete
source behavior is now directly representable.

- Source: `CrazyMachines`, `cm_family`, and `cmnftl` declare the same three byte
  fields at executable-specific addresses. The start latch, win edge, and menu
  reset consume only that common field interface.
- Candidate: only the first layout is retained because the report says that
  alternate state selection has not been demonstrated. It also drops the
  source's 120 Hz refresh rate.
- Existing facilities: use one exact-name candidate array containing
  `CrazyMachines.exe`, `cm_family.exe`, and `cmnftl.exe`; declare one build enum
  variant and conditional field branch per executable; assign the build from
  `process.name()` in `onAttach`; and
  declare `tickRate { attached: 120 }`.
- The natural selector is a `match process.name()` with one string-literal arm
  per executable. The audit initially had to replace that expression with an
  `if` chain because string literals were not accepted as patterns. String
  patterns now compare decoded contents, require a wildcard for exhaustiveness,
  diagnose duplicate values, and work across suspending match arms.
- A focused current-compiler probe validates the complete composition and
  exposes `win` through the common snapshot interface. The unmatched-name arm
  waits for process closure, so shape initialization remains total without a silent
  fallback.
- Live-game verification is still required for the executable filenames,
  addresses, pointer chains, and transition timing. Compilation proves that all
  three source layouts are representable, not that those legacy layouts remain
  correct.

### Cooperative module discovery and runtime pointer bounds: Hades

Campaign status: `BLOCKED`

Audit result: the reported module-enumeration and runtime-range blockers mix a
real discoverability gap, an unnecessarily fixed translation, and one narrower
host boundary.

- Source: the engine module is selected with `StartsWith("EngineWin64s")`.
  The two documented concrete images are `EngineWin64s.dll` and
  `EngineWin64sv.dll`. The candidate kept only the first exact name and therefore
  silently dropped Vulkan support.
- Existing translation: known exact alternatives compose with synchronous
  `process.loadedModule(name)` checks inside one [`retry`] value block. A
  focused compiler probe validates both names, whole-attempt tick retry, and
  valid WebAssembly GC. Arbitrary prefix
  enumeration remains a narrower host gap only if future unlisted suffixes are
  part of the required compatibility contract.
- The source screen vector already exposes begin and end pointers. Walking a
  mutable address cursor with [`while cursor < end`](language@while) and
  `cursor = cursor.offset(8)` preserves that runtime bound. The candidate's
  fixed 32-index array is neither required nor faithful, so this case does not
  establish a runtime-range requirement.
- A value-producing [`loop`] remains a valid low-level composition, but
  whole-expression [`retry`] now captures this discovery shape directly: the
  complete fallible transaction restarts on the next tick, and postfix [`?`]
  transfers failure to that local boundary without a manual infinite loop.
- Exact `shutdown` and timer-event callback behavior remain the existing host
  lifecycle gaps. Live-game validation is still required for module identities,
  vector layout, vtable probing, and split timing.

Further campaign work should begin with a concrete friction report and reduce it
to a focused source comparison or compiler probe. The generated ports are
supporting evidence, not an exhaustive conformance corpus. Every resulting
change must still separate static compilation, deterministic host-fixture
coverage, and live-game validation.

## Cross-cutting findings

### Compiler defects resolved during the audit

The formatter could merge a generic close with a following assignment or
comparison. Revision `b82638c` fixed the parser/formatter boundary without
making whitespace semantic and added nested generic, cast, comparison, shift,
result/option postfix, and strict-equality recovery coverage.

String `+` and `+=` now produce focused guidance for template interpolation
and `String.concat` instead of ending at a numeric-capability error. No rewrite
is offered because text, separators, and evaluation grouping require author
intent.

Fallible-value diagnostics now name value fallback and exhaustive `match`
handling, and mention postfix `?` only inside an actual failure boundary. A
direct optional return using `else None` gets a narrowly-scoped
machine-applicable `else return None` rewrite. Generic failures retain their
capability requirements through inference, name the missing capability, and
list accepted concrete types when the catalog proves that set is finite.

### Existing facilities whose user journeys failed

Finding an implementation after the campaign does not invalidate the report.
Each item below is a product failure in documentation, search, completion,
diagnostics, or API ergonomics even when it does not require a second language
feature:

- growable `[T]` is the ordered `List<T>` replacement and supports indexing,
  iteration, search, insertion at the end, removal, and clearing;
- finite ASL dictionaries used only to declare settings should become the
  settings DSL, while `Set<T>` covers unordered run-scoped membership;
- captured helpers that do not need first-class identity become named `fn`
  declarations with explicit parameters or shared globals;
- state candidate rejection retains a previous field value without making
  `current` mutable; derived run-owned values belong in globals;
- `process.loadedModule(name)`, `process.mainModule()`, executable versions,
  typed shape selection, and whole-expression [`retry`] cover reviewed
  known-name probes, while arbitrary prefix module
  discovery still requires host enumeration;
- `scan`, `readRelative32`, `MemoryPath`, `onAttach`, and `retry` cover the
  reviewed scanner callback and background retry shapes without exposing
  threads.

The roadmap therefore asks the compiler and editor to lead authors to these
facilities rather than adding compatibility aliases or duplicate abstractions.

### Documentation journey probe and first follow-through

A focused `splitc docs` probe reproduced the campaign's general discovery
failure without auditing every generated port. `MemoryWatcherList` and
`string128` returned no documentation; `Task.Run`, `vars.Helper.Scenes`,
`mono.Make`, `mono.MakeString`, `settings.ContainsKey`, and settings indexing
either led only to a broad guide or ranked an unrelated topic first. This is
actionable evidence regardless of whether lower-level primitives can be made to
compose.

The first follow-through gives `MemoryWatcherList` and `Task.Run` focused
compiler-owned pages and contextual source diagnostics. Arbitrary `stringN`
widths normalize to the bounded native-string topic. Settings indexing and
`ContainsKey` additionally receive safe automatic rewrites to `enabled` and
`contains`. The outcomes deliberately differ: settings lookup is already a
direct typed operation; `Task.Run` must be mapped from its intent to cooperative
control flow; and `MemoryWatcherList` has distinct fixed-state,
discovered-address, and managed-collection shapes. Unity scenes and ergonomic
typed Mono scalar/string paths remain provider design work. They deliberately
do not get placeholder migration pages whose answer is only "not available";
the roadmap prioritizes implementing a real API first. In particular, the
existence of `MemoryPath` does not make manual
object-header arithmetic an acceptable final `mono.MakeString` migration.

### Real compiler and documentation work

The campaign provides remaining concrete cases for:

- literal settings-key checking, completion, and nearest-key suggestions;
- lifecycle, state-mutation, module-discovery, and missing-state diagnostics
  that link directly to the canonical recipe;
- packaging and link checks so compiler-owned migration material is reachable
  in native, bundled, and editor workflows.

### Actual host or language boundaries

`onStart` and `onReset` are available through timer-state sampling, including
while detached. Exact lossless delivery across multiple transitions between
updates, `shutdown`, and `onSplit` still need ordered host events. Dialogs,
timing-method mutation, arbitrary filesystem or process control, and similar UI
capabilities need a sandbox policy. Full
unknown-module enumeration and portable process identity remain runtime design
work. Dynamic bags, closures, and user-created background threads are not
justified by these candidates because their reviewed uses already have typed
translations; a future maintained port must prove a remaining semantic need.

## Evidence rule

The maintained host fixtures are the authoritative automated evidence for the
six in-tree ports. Candidate compilation is never sufficient for behavioral
parity, and the campaign's Neon White candidate remains distinct from the
independently reconstructed maintained port. The exact evidence and remaining
build limitations are recorded in
[`NEON_WHITE_PORT.md`](NEON_WHITE_PORT.md).
