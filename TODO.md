# SplitScript active roadmap

This file contains only work that is still active, deliberately deferred, or
ongoing. Completed milestones belong in
[`docs/ROADMAP_ARCHIVE.md`](docs/ROADMAP_ARCHIVE.md), not as checked boxes here.

The roadmap is ordered by user and porting impact:

- **Now** — the next concrete goal;
- **P0** — work that blocks faithful autosplitters or protects a foundational
  compiler boundary;
- **P1** — important product, tooling, and language expansion after the current
  porting blockers;
- **P2 / deferred** — useful work that should wait for evidence or a cheaper
  dependency;
- **Ongoing** — evidence and maintenance expected alongside every priority.

General rules:

- Drive language and standard-library growth from reviewed, faithful ports.
- Prefer one reusable typed abstraction over compatibility aliases or
  game-specific compiler branches.
- Bring language syntax, semantics, and public standard-library API choices to
the user before implementation. Porting evidence should frame the decision
  and possible designs, not silently choose one.
- Keep ordinary library behavior in `stdlib/standard.split`; reserve Rust for
  representations, validated intrinsics, runtime helpers, and ABI boundaries.
- Add compiler, runtime, formatter, and editor coverage in the same change when
  a feature crosses those surfaces.
- Treat every reported workaround, misunderstanding, and omission as product
  evidence, including when the intended facility already exists. Classify it as
  a documentation, compiler-guidance, tooling, language/library, or host-runtime
  issue instead of dismissing it as port-author error. Lead authors to canonical
  typed patterns without hiding genuine ergonomic gaps behind migration advice.
- Record host-runtime gaps found during ports in
  [`docs/RUNTIME_EVOLUTION.md`](docs/RUNTIME_EVOLUTION.md), with evidence and
  semantic requirements before proposing import spellings. Keep compiler-only
  work in this roadmap and implemented contracts in `docs/ABI.md`.
- Remove completed work from this file during the next roadmap update and
  summarize the milestone in the archive.

## Now — eliminate confirmed porting-campaign correctness failures

Start with the unchecked tasks in
[Fix the confirmed 2026-08-30 campaign defects](#fix-the-confirmed-2026-08-30-campaign-defects),
in their written order. The managed collection schema panic is now stopped at
the shared remote-memory validation boundary, and expression-valued optional
state fields now apply the same contextual `None` conversion as other typed
positions, and the ASL numeric-root migration now distinguishes main-module
offsets from SplitScript absolute addresses. The compiler-owned `choice`
example is corrected and language-catalog validation now parses, formats, and
type-checks every complete fixture while classifying focused fragments
explicitly. The fixed-array reference now exposes its process-read bounds and
the transactional sparse-read alternative. Unknown state providers now list
the catalog-owned choices, repair only clear typos, and link a dedicated
provider index. Statement-shaped match arms now point at the missing braces and
offer a complete rewrite instead of blaming an existing comma. Bare-global
lifetime errors now spell out the one direct `onAttach`/`onStart` assignment
rule and the helper-call boundary. ASL primitive state types now have a
physical-width mapping and qualified exact search aliases that do not
masquerade as compiler rewrite spellings. Literal provider-memory reads are
now checked against the same catalog-owned ranges used by runtime translation
and reference documentation. The confirmed campaign-defect sequence is
complete, the approved finite `process.scanOnce` primitive now reports
bounded-range exhaustion without changing waiting scans, and suspending
`whileAttached` now supports cancellable post-attachment rediscovery without
running timer decisions against stale snapshots. Continue with the remaining
P0 porting gaps below. The active Unity migration and managed collection work
follows the finite completion scope in [UNITY_FINISH.md](docs/UNITY_FINISH.md).
Bring new language, standard-library, provider, or host-surface decisions below
back to the user. Large foreign-input recovery is now bounded and prompt, and
the current corpus-proven exact
migration searches lead to canonical facilities or honest pending-design
pages. Runtime-dependent timer, writable-file, file-metadata, JSON, and module
enumeration work remains explicitly deferred until its host contracts are
ready. Continue with the highest-impact compiler-only gaps below in the
meantime.

## Unity schema foundation and deferred follow-ups

Use the Rust Lunistice autosplitter and `examples/lunistice.split` as the first
acceptance pair. The current SplitScript Unity API is not a compatibility
boundary: replace its manual image/class/offset plumbing where the schema model
provides a clearer typed facility. Keep the generated implementation
demand-driven, allocation-conscious, backend-independent, and integrated with
the ordinary compiler symbol graph rather than adding Unity-shaped exceptions
to inference or code generation.

### P0.1 — represent managed metadata in ordinary compiler architecture

- [x] Add first-class `image`, `namespace`, `class`, field, static-field,
  metadata-name (`from`), and conditional-field declarations to syntax,
  AST/HIR, recovery, formatting, and source traversal. Declarations use
  mandatory semicolons and never depend on line breaks. Preserve documentation
  on every declared image, namespace, class, and field.
- [x] Resolve image schemas through the normal semantic symbol tables. Give
  every generated class, reference, field, and static member a stable
  identity used by type inference, diagnostics, rename, go-to-definition,
  completion, hover, semantic highlighting, selection ranges, documentation,
  and unused analysis. Do not teach those consumers unrelated lists of Unity
  names.
- [x] Define the backend-independent class model: `T.Ref` is a live remote
  managed reference; `T` is an immutable local snapshot. Class-typed managed
  fields produce references, terminal value fields produce fallible values,
  and `.snapshot()` reads the declared value shape transactionally. Generate
  snapshot readers only when reachable.
  - [x] Derive one backend-independent binding plan from checked schema and
    semantic identities. It preserves image and namespace ownership, class and
    binding alternatives, static versus instance fields, declared versus read
    value types, exact aliases, and conventional C# automatic-property backing
    candidates. Snapshot code generation consumes this projection instead of
    maintaining a parallel Unity class registry.
  - [x] Resolve and lower common static managed roots and fallible
    live-reference field reads through that plan. Every remote hop produces an
    ordinary `T!`, so paths such as `GameManager.instance?.player?.score?`
    expose each possible failure through the language's existing propagation
    model. Cached metadata is shared while object pointers are re-read live.
  - [x] Generate reachable transactional `.snapshot()` readers through the
    same binding plan. Each reader evaluates every active instance field into
    temporary fallible results before constructing the immutable GC value,
    propagates the first failure without exposing a partial object, preserves
    stable slots for conditional fields, and is emitted only when a
    checked call can reach it.
- [x] Specify deterministic metadata-name resolution. A missing `from` uses the
  source member name, `from "name"` names one exact metadata member, and
  `from ["first", "second"]` names explicit alternatives. C# automatic-property
  backing fields participate in the same canonical lookup. Ambiguity is an
  error rather than declaration-order selection.
  - [x] Centralize canonical binding-name expansion on managed field
    declarations. Type checking and backend planning now share exact `from`
    alternatives and implicit source-name/backing-field candidates, and reject
    collisions between either form before runtime binding.
  - [x] Feed namespace-qualified class aliases into both Mono and IL2CPP
    attachment binders. The generated binder performs alias discovery once per
    attachment and retains the uniquely selected class metadata for later
    field binding.
  - [x] Replace first-match alias selection with complete Mono and IL2CPP
    binding probes. A completed traversal now distinguishes a unique match, a
    stable miss, an ambiguity, and a transient structural process-memory failure;
    misses and ambiguities report the responsible source aliases once and keep
    the attachment inert until the process closes instead of rescanning
    metadata forever.
- [x] Remove `layout` as a user-facing concept and express every observable
  shape decision with ordinary typed state. Attachment-static decisions such
  as edition, storefront, renderer, or build are bare enum globals initialized
  by `onAttach` or, when uniquely proven, by Unity metadata. Decisions that may
  change while a process remains attached are ordinary enum-valued state
  fields. The compiler retains one finite internal predicate model for field
  availability and schema planning; users should not need a parallel `Layout`
  struct, implicit `layout` value, or generated `StateLayout` enum.
  - [x] Generalize the predicate representation across attachment globals and
    dynamic state fields. Use it for path-sensitive
    member availability, declaration checking, and code generation rather than
    introducing syntax-specific aliases.
  - [x] Allow attachment enum globals to guard state and managed-class fields.
    Freeze a global once it participates in attachment shape, and let uniquely
    distinguishing Unity metadata initialize an otherwise bare global before
    `onAttach`. The Lunistice example now uses `edition: Edition` directly.
  - [x] Allow enum-valued state fields to guard other state fields. Poll the
    discriminator first, initialize an entering branch transactionally, leave
    inactive branches unread, and seed newly active fields into `old` so a
    shape transition cannot manufacture a split from storage defaults.
  - [x] Replace every maintained example, test, guide, hover, completion item,
    and generated-reference entry that still teaches `layout`, `Layout`, or
    `StateLayout` with ordinary globals/state fields. Remove the old grammar,
    AST nodes, generated symbols, backend storage, and layout-return contract
    from `onAttach`. The removed experimental SplitScript syntax receives no
    compatibility path or bespoke diagnostic; ASL migration diagnostics still
    guide source-language layout labels to the ordinary shape model.
  - [x] Document compiler-initialized attachment globals in hover/reference
    output and diagnose partial mixed ownership when one shape dimension is
    assigned by `onAttach` while another is expected from metadata.
  - [x] Allow state and managed-class fields to be conditioned by ordinary,
    statically decidable predicates over finite enum dimensions. Type-check
    those predicates once and use the same predicate representation for member
    availability, control-flow refinement, binding, diagnostics, and codegen.
  - [x] Support `else if` and `else` chains for conditional state and managed
    fields. Represent every branch as the exact bounded set of shape
    assignments left after preceding branches, so complements across several
    dimensions remain correct in tooling, availability checks, binding, and
    generated polling code.
  - [x] Make managed schema probes contribute constraints to the global shape
    dimensions. Probe results preserve both offsets and presence. When the
    complete set of conditional fields gives every bounded shape combination
    a distinct exact presence pattern, attachment initializes the responsible
    bare globals automatically before user `onAttach` code runs. Otherwise
    require direct `onAttach` assignments and explain why automatic selection
    is not decisive. The compiler bounds the product rather than enumerating
    an unbounded cartesian product, and a failed automatic match rejects that
    process for the remainder of its lifetime.
  - [x] Remove generated `<Class>.Layout` enums, `<Class>.layout` values, and
    per-class public refinement. If a genuinely observable class-only schema
    distinction is needed, declare it as another global dimension; if the
    distinction preserves the public API, keep it as a private binding
    alternative.
  - [x] Give both backends one internal three-state field probe that separates
    transient read failure, completed absence, and a found offset. Generated
    attachment binding probes complete alternatives without adding a public
    lookup API or duplicating the metadata scanner. Reuse this mechanism as
    the low-level evidence source for global shape constraints.
  - [x] Replace the temporary inert zero-match behavior with a focused runtime
    attachment report naming every observed conditional managed field and the
    expected presence pattern of each responsible source shape. Distinct
    exact patterns make multiple runtime matches impossible; indistinguishable
    patterns remain a compile-time error.

### P0.2 — bind schemas once per attachment

- [x] Introduce a single internal Unity metadata adapter shared by Mono and
  IL2CPP. Keep native metadata traversal and process scanning intrinsic, but
  generate high-level bindings and reads from the source schema. Replace the
  internal split between `UnityModule`/`UnityClass` and
  `MonoModule`/`MonoClass` where it no longer expresses a real backend
  difference.
  - [x] Route automatically detected runtimes through one private,
    source-defined runtime/image/class adapter before generated binding begins.
    The adapter owns backend dispatch while generated schemas use one common
    image, class, field, static-table, and pointer-width path. Keep explicit
    backend selectors specialized for now so release reachability can still
    prune the unused traversal implementation.
  - [x] Make manual runtime/image/class/offset traversal and all of its backend
    struct types private to the trusted standard library. Remove them from
    public lookup, completion, hover, navigation, and generated documentation;
    retain only the pieces reached by provider preparation and schema binding.
    Do not preserve a public dynamic escape hatch by default: introduce one
    only after a representative schema limitation proves its need and its
    source design has been approved.
- [x] Make Unity an attachment/state provider with an automatic form and
  explicit backend/version forms, including `state Unity [...]`,
  `state Unity.il2cpp(Il2CppProfile.unity2021_3_11f1X64()) [...]`, and `state Unity.mono(...) [...]`.
  Provider setup is cooperative and cancelled with the process. Ordinary state
  fields can read generated managed members without manually retaining classes,
  static tables, instance addresses, or offsets.
  - [x] Generalize state syntax so a named provider can own source-declared
    process candidates (`state Provider ["game.exe", ...]`), with catalog-mode
    validation for providers whose process list is instead fixed. Keep provider
    names in their contextual namespace so the future `state Unity` provider
    can coexist with the existing `Unity.*` API namespace.
  - [x] Model qualified provider configurations in the privileged catalog and
    ordinary syntax. `state Provider.selector(...) [...]` uses normal typed,
    compile-time-constant expressions; completion, hover, semantic
    highlighting, formatting, selection ranges, and provider documentation are
    derived from the same selector declarations. Distinguish source-process
    providers from the single catalog-declared default so Native remains the
    meaning of bare `state "..."` while Unity can also accept source candidates.
  - [x] Publish the Unity provider with a compiler-owned preparation phase that
    completes before user `onAttach`. Automatic mode detects Mono versus IL2CPP
    from loaded runtime modules and Unity metadata; explicit selectors retain
    typed version arguments.
- [x] Cache common image, class, static-table, and field-offset metadata for one
  attachment. Re-follow dynamic object pointers on each read so replaceable
  singletons remain correct. Scalar live paths allocate no fresh GC object per
  tick.
- [x] Share each fallible managed static-root read across one candidate-state
  transaction, including reads made by transitively called helpers. Clear the
  cache before every poll so singleton replacement is observed on the next
  tick, and keep lifecycle calls outside the transaction. The Lunistice
  harness requires exactly one `GameManager` and one `Timer` singleton read per
  snapshot instead of one root read per field. Emit one generated reader per
  static field rather than inlining its cache and host-read branches at every
  use; the optimized Lunistice module is 28,529 bytes.
- [x] Apply common-subexpression planning to ordinary native state pointer
  paths. Resolve a shared module lookup or raw pointer dereference once into an
  `update` local and reuse it across sibling fields, including the case where
  fields add different offsets after the same dereference. Resolution remains
  lazy inside the active shape branch, failures retain the existing per-field
  boundary, and locals reset naturally for the next candidate snapshot. The
  release Neon White fixture shrank from 6,707 to 6,515 bytes while removing
  the duplicate host calls.
- [x] Extend the attachment cache with managed field offsets and presence
  evidence used to validate the attachment-wide shape. Strings, arrays, and
  explicit snapshots may still allocate their returned values.
- [x] Preserve the existing transactional state-field failure boundary for
  generated member reads. A failed pointer hop or memory read retains the last
  accepted field value; Unity declarations must not introduce a second failure
  model.

### P0.3 — prove the design by simplifying Lunistice

- [x] Declare the Lunistice `GameManager` and `Timer` schemas, including shared
  fields, base-game and DLC-demo conditional shapes, alternative singleton
  names, `LevelTimeParts`, and the bounded DLC scene string.
- [x] Port `examples/lunistice.split` from manual class/instance/offset globals
  and raw `process.read` calls to generated transactional `GameManager` and
  `Timer` snapshots. The DLC scene is now an ordinary schema-declared
  `String scene` field read as part of the shared snapshot. Preserve
  all existing autosplitter behavior and keep the user's current local example
  edits out of intermediate mechanical rewrites.
- [x] Add synthetic runtime coverage for both base and DLC metadata shapes,
  singleton replacement, backing-field lookup, inherited fields, failed reads,
  and ambiguous shapes. Compare the resulting script structure and behavior
  with `C:\Projekte\lunistice-auto-splitter`.
- [x] Treat the milestone as complete only when all generated Unity symbols are
  navigable and documented in the editor and reference viewer, and the port no
  longer contains manual Unity metadata offsets or attachment bookkeeping.

### P1 — complete the Unity object model on the same foundation

- [x] Add cooperative `T.instances()` scanning by managed class/vtable. It
  returns a completed `[T.Ref]` snapshot, scans only readable and writable
  non-executable ranges at natural pointer alignment, bounds bytes and matches
  per poll, preserves process cancellation, and binds the backend-specific
  IL2CPP class pointer or Mono vtable once per attachment. Its compiler-provided
  leaf future, runtime scanner, completion, hover, navigation, highlighting,
  and Mono/IL2CPP runtime fixtures share the ordinary typed architecture rather
  than exposing raw vtable machinery.
- [x] Unify native scene-manager discovery and immutable active, loaded, and
  don't-destroy-on-load snapshots under the typed attachment context
  `unity.scenes`. State-provider contexts are catalog-declared, resolved through
  the ordinary symbol graph, prepared once per attachment only when referenced,
  and available wherever the provider's process value is available. The old
  public `Unity.sceneManager()` workflow is private implementation detail.
- [x] Add exact, bounded scene hierarchy lookup through `UnityScene.find`,
  `UnityGameObject.find`, and `UnityGameObject.child`, plus schema-derived
  `UnityGameObject.component<T>() -> T.Ref!`. Native traversal lives in
  privileged SplitScript standard-library source, while the compiler supplies
  only the declared class's backend-neutral runtime header and nominal result
  type. A tiny class schema is the typed escape for otherwise-unknown
  components; defer any raw dynamic API until a real port proves it necessary.
- [ ] After ASR has a tested implementation, align Unity global managers such
  as `unity.time.frameCount` and `unity.time.timeScale` with it through
  reachable source-defined declarations rather than independently inventing
  native discovery or bespoke compiler names. Until then, keep this deferred;
  the current ASR Unity support covers managed metadata and scenes but not the
  native time manager.
- [x] Add managed-string fields as `String field` and `String? field`, with
  shared decoding for static, live, and snapshot reads. Object headers supply
  the length; nullability is typed and malformed UTF-16 uses replacement decoding.
- [x] Complete the ASR Unity migration using
  [the finite finish scope](docs/UNITY_FINISH.md). Profiles, nested managed
  String/array/List/Map/Set reads, and owned class snapshots are implemented and
  verified. Final Lunistice Release: 32,121 bytes explicit / 142,234 automatic.
  The user accepted stopping without a concrete further saving of roughly
  500 bytes or more, replacing the earlier below-30,000-byte completion gate.
  General Release compiler optimizations remain the separate P1 project below.
- [ ] Exercise the managed collection surface when revisiting Alba and A Short
  Hike. Arrays and remote `List<T>` materialize as `[T]`; remote maps and sets
  retain `Map` and `Set` semantics. Keep dynamic typed tag values as a separate
  language-design question driven by those ports.
- [ ] Decide explicit Unity field-versus-property binding after the completed
  Unity migration. Prefer explicit intent over automatically trying a C#
  property's backing-field name for every field alias. Consider whether a
  property marker is needed, but do not choose or introduce syntax yet. Keep
  existing automatic matching and Lunistice declarations unchanged until that
  design is settled; then align schema binding, diagnostics, docs, and tests.

## P0 — make docs-first ASL porting semantically reliable

The first clean-folder exercise had only the compiler and legacy ASL inputs. It
produced 52 compiler-valid ports or partial ports across 53 reviewed scripts and
learned the language exclusively through `splitc docs`. Exact Windows
executable names and build-specific memory shapes were found, demonstrating
that the earlier documentation work helped. Compilation still hid substantial
semantic drift:
every Unity port combined `state Unity` with manual runtime and metadata
discovery, and emulator ports manually rediscovered mappings and byte order
despite matching typed providers.

The 2026-08-30 follow-up independently produced 16 warning-free ports or partial
ports and two deliberately unported architecture cases. It confirmed that the
schema-first Unity and typed emulator journeys now work when available, while
finding a managed-schema compiler panic, inconsistent optional state typing,
several diagnostics and compiler-owned examples that mislead authors, and
faithful-port gaps around finite scanning, PS2 discovery, persistent files,
timer metadata, and runtime-varying memory shapes. Treat silent canonical-API
misses and compile-clean semantic substitutions as higher-risk evidence than an
explicit compile error. The external exercise directory is disposable evidence,
not a durable roadmap dependency; retain actionable conclusions and minimized
in-tree regression cases instead of paths or assumptions tied to one generated
corpus.

The 2026-09-04 WIP pass produced 33 compiler-clean ports or partial ports, but
was less disciplined about behavioral equivalence and repeated one class of
literal-precondition observation roughly forty times. Treat its compile-only
ports as hypotheses, not validation. Its deep attempts nevertheless add strong
independent evidence for runtime-dependent state polling and an associative
map, and reinforce the existing timer, writable-file, dynamic-settings, module
enumeration, managed-collection, and process-write host gaps. Rechecking the
current compiler also confirms that a 166 KiB unmodified legacy ASL still takes
more than 60 seconds without producing diagnostics. Conversely, `Module.md5`,
conditional state shapes, finite scans, growable arrays, sets and maps,
`Instant`, and static settings families already cover several reported
omissions; those findings are documentation or migration-search work, not
reasons to add duplicate APIs.

### Turn every reported blocker into an actionable product outcome

- [x] Make managed schemas the one canonical public Unity workflow. Move
  `Unity.mono`, `Unity.il2cpp`, `MonoModule` / `MonoImage` / `MonoClass`,
  `UnityModule` / `UnityImage` / `UnityClass`, and raw managed field/static-table
  traversal behind the trusted standard library where generated providers and
  schema binders still need them; delete unused surface rather than keeping
  compatibility aliases. Before retaining any public low-level escape hatch,
  bring a concrete schema limitation and proposed source API back for approval.
  A script using `state Unity` must not be led toward discovering the same
  runtime again in `onAttach`.
- [x] Rebuild the current Unity documentation journey around schema-first
  ports: `state Unity`, `image`, namespace/class declarations, static and
  instance roots, bounded managed strings, attachment shape globals, and
  fallible live paths. Exact searches for `UnityASL`, `mono.Make<T>`,
  `mono.MakeString`,
  `Unity.mono`, and conceptual queries such as “managed field” reach this
  workflow rather than a low-level class API. Contextual migration diagnostics
  cover the old helper spellings without claiming a mechanically safe rewrite.
- [x] Extend that journey with transactional snapshots. The managed-class
  reference documents `T.Ref`, fallible live hops, `T`, transactional
  `.snapshot()`, shape refinement, and demand-driven reader generation; editor
  hover and navigation lead back to the source class and fields.
- [x] Add the approved higher-level bounded managed-string declaration and keep
  examples, diagnostics, tooling, runtime semantics, and migration docs on the
  schema workflow rather than raw object traversal.
- [x] Make emulator providers equally difficult to miss. General state and
  porting documentation now links all seven typed providers instead of using
  GBA as the whole model. Provider pages own their emulator, core, address,
  byte-order, and legacy-`DeepPointer` vocabulary; the ASL/Rust migration index
  covers every provider; and reference-search tests require `Dolphin`, `Fusion`,
  `RetroArch`, `PCSX2`, `DuckStation`, `mGBA`, and `DeepPointer` to surface the
  corresponding provider declarations rather than only backing struct types.
- [x] Make `setTickRate` document the complete polling policy in its own page:
  the default 120 Hz attached and 1 Hz detached rates, when those lifecycle
  values are applied, how a top-level `tickRate` declaration overrides either
  default, and how a dynamic call persists only until another call or the next
  attachment transition. Link both directions and replace the misleading
  `setTickRate(120)` example with one that demonstrates a genuinely temporary
  dynamic adjustment. Keep `refreshRate` migration guidance pointed at the
  declarative block.
- [x] Fix the confirmed unused bare-global inference failure. Ambiguous global
  types are now recovered at the shared inference-finalization boundary with a
  source-facing diagnostic anchored at the declaration and secondary labels on
  non-concrete assignments and uses; internal inference-variable names no
  longer leak at an unrelated declaration. Attachment- and attempt-scoped
  inference remains shared, and a concrete `MemoryPath` initialization/use
  regression verifies that valid attachment state still infers normally.
- [x] Give reserved keywords used as identifiers a focused parser diagnostic.
  Source-defined names now share one recovering parser path: a reserved word
  is consumed in place, diagnosed at its declaration, and gets a
  machine-applicable trailing-underscore rename instead of producing a later
  missing-brace cascade. The path covers globals, locals, loop/closure/pattern
  bindings, functions and parameters, structs, enums, managed declarations,
  layouts, state fields, and settings. The reserved set is intentionally
  limited to words that take over expression or statement grammar; contextual
  DSL words such as `at`, `from`, `key`, and `static` remain legal ordinary
  names away from their grammar positions.
- [x] Make sibling state-field references order-independent within each active
  physical layout. Build and validate an explicit dependency graph, diagnose
  cycles at every participating declaration, and evaluate dependencies before
  their consumers. Expression-backed fields and dynamic `at` bases share the
  same resolution path. A failed dependency skips its dependents for that poll
  so no stale candidate address is dereferenced; every affected field retains
  its own last accepted value.
- [x] Add `String.isBlank()` with Unicode `White_Space` semantics; several
  ports silently replaced C# `IsNullOrWhiteSpace` with an empty-string test.
  Keep absence explicit for `String?`, provide focused C# migration guidance,
  and pin the internal property table and its boundary tests to a documented
  Unicode version so future updates are deliberate.
- [x] Recheck clean-compiling omissions against facilities added before or
  during the exercise. `timer.currentSplitIndex()`, generic numeric `Duration`
  constructors, and finite settings families already surfaced through focused
  reference searches and canonical pages. Add the missing attempt/run-scoped
  state migration topic, teach the `let` page the alternate vocabulary, and
  make `settings.Add` loop searches lead to compile-time settings families
  instead of hand-expanded declarations. Search regressions now preserve these
  routes before proposing duplicate features.

### Prevent clean-compiling semantic drift

- [x] Add a review checklist and compiler-query fixture for ports that compile
  while bypassing a canonical provider. Check process identity, selected build,
  provider choice, byte order, lifecycle ownership, settings reachability,
  integer width, failure behavior, and deliberately omitted source branches.
  The fixture should assert documentation journeys and diagnostics, not bundle
  disposable full-script outputs.

### Fix the confirmed 2026-08-30 campaign defects

- [x] Stop unsupported managed collection fields during schema checking rather
  than letting `[String]` reach the code-generation assertion that every
  managed value field is [`MemoryReadable`]. Emit a normal source diagnostic on
  the field type in every profile and retain a minimized no-panic regression.
  This is independent of the deferred product design for managed arrays/lists:
  unsupported source must never panic while SplitScript waits for ASR's tested
  representation.
- [x] Make contextual [`None`] conversion consistent for expression-valued
  state fields. `label: String? = None` must type-check by the same expected-type
  conversion used for locals, returns, and other state expressions, without
  special-casing `String` or weakening transactional field typing. Add positive
  coverage for multiple optional value types and preserve a useful mismatch
  diagnostic when no optional target is expected.
- [x] Explain the legacy ASL address-base semantic explicitly. The ASL porting
  journey and exact `DeepPointer` / native-state migration searches must show
  that a bare numeric ASL root is normally main-module-relative and therefore
  becomes `at "game.exe", offset`; copying it as an integer SplitScript root is
  an absolute address and can compile while reading the wrong memory. Cover the
  distinction with a focused compiler-query regression rather than attempting
  an unreliable cross-platform source warning.
- [x] Fix the compiler-owned `choice` setting example by emitting required
  commas, and make every complete source snippet owned by the language catalog
  parse, format, and type-check in an automated documentation test. Keep
  intentionally partial syntax fragments explicitly classified so catalog
  prose cannot silently publish uncompilable examples again.
- [x] Document the 4,096-element fixed-array limit on the exact [`[T; N]`]
  reference and memory-state paths before authors design large native blobs
  around an impossible schema. Link to growable state value blocks as the
  current selective-read alternative while keeping the existing precise
  declaration diagnostic.
- [x] Improve unknown-provider diagnostics. List the currently valid state
  providers, suggest the nearest spelling when applicable, and link the state
  provider index. Do not imply that a correctly spelled unsupported provider
  such as `SNES` exists; the separate ASR-dependent SNES task remains deferred.
- [x] Diagnose statement-shaped `match` arms at the statement/expression
  boundary. When assignment or a side-effecting `if` appears directly after
  `=>`, explain that the arm needs braces and offer a machine-applicable braced
  rewrite instead of claiming that the already-present comma is missing.
- [x] Make the missing bare-global lifecycle-initializer diagnostic say that
  initialization must be a direct assignment in exactly one `onAttach` or
  `onStart` boundary and that assignments hidden in called helpers do not
  establish lifetime. Preserve the existing excellent dual-boundary labels and
  avoid promising interprocedural definite-initialization analysis.
- [x] Add a concise ASL primitive-type migration topic and exact search aliases
  for queries such as `ASL bool byte int`. Show physical-width mappings and the
  cases that require inspecting source/runtime representation rather than
  blindly choosing a similarly named SplitScript type.
- [x] Reject statically impossible literal guest-memory reads at provider
  checking time. A constant PS2 address outside the provider's declared readable
  domain must produce a focused diagnostic before code generation; dynamic
  addresses retain their runtime failure. Keep this diagnostic synchronized
  with the provider's actual supported domain as that domain evolves.

## P0 — fix editor correctness and the first-use path

The 2026-08-29 usability and codebase reviews are preserved as evidence in
[`USABILITY.md`](USABILITY.md) and [`CODEBASE_REVIEW.md`](CODEBASE_REVIEW.md).
The roadmap below keeps their actionable findings, deduplicated against the
existing compiler, porting, and packaging work. Exact API facts must continue
to come from compiler-owned catalogs; hand-written documents teach tasks and
concepts rather than maintaining a parallel inventory.

### Give a new author one short successful workflow

- [x] Add a compiler-checked, self-contained Getting Started guide with focused
  inline snippets rather than a bundled or linked full autosplitter. Cover
  obtaining the extension or CLI, creating a `.split` file,
  debug and release builds, neighboring `.wasm` output, the currently supported
  host-loading workflow and limitations, opening/searching docs, and reading
  the first diagnostic. Introduce attachment, one typed setting, one `at` state
  field, `old` / `current`, and one timer decision before scans, Unity,
  conditional shapes, or async discovery.
- [x] Rewrite the packaged VS Code README as an extension-user/Marketplace
  artifact: outcome, first workflow, commands and outputs, bundled compiler,
  documentation, requirements, limitations, and troubleshooting. Move npm,
  VSIX construction, web-host tests, workers, and Extension Development Host
  instructions to `editors/vscode/DEVELOPMENT.md`, linked once for contributors.
- [x] Restructure the repository README as an honest audience router. Lead with
  what an autosplitter author can accomplish, current project/host status, a
  minimal warning-free example, and paths for extension users, CLI users, ASL
  porters, and compiler contributors. Route production-port evidence to durable
  contributor documents and keep backend, LSP, and debug-metadata inventories
  in developer architecture documents; do not make full examples part of the
  user path.
- [x] Keep maintained port and conformance evidence discoverable for compiler
  contributors without making the `examples` directory part of the authoring
  path. User guides must remain self-contained and must not link to disposable
  full autosplitter scripts.
- [x] Add type-directed completion for enum-backed choice settings. Inside
  every choice option and its `default` value, offer enum types in expression
  position and, after a qualified enum name, only that enum's variants. Reuse
  ordinary expression/type completion and the setting checker's inferred enum
  identity rather than maintaining a settings-only symbol list; cover empty,
  partially written, and already-constrained choice declarations in the shared
  compiler completion tests used by every editor frontend. The implementation
  reuses the source-enum candidate builders, narrows later options from the
  preceding choice entries even while the declaration is incomplete, and
  excludes payload variants that the choice-setting contract rejects.

### Make every documentation surface agree

- [x] Correct current contradictions before expanding prose: generic user
  functions and source async helpers are implemented; strings, structs,
  arrays, closures, iterators, UTF-16 decoding, numeric formatting, string
  construction, browsable docs, and generated reference pages must not still
  be described as future work.
- [x] Keep compiler catalogs canonical for exact signatures, effects,
  availability, members, and support status; hand-written pages should link to
  those facts instead of copying tables.
- [x] Split the user-facing language path from compiler architecture. Route new
  authors through the self-contained Getting Started workflow, then teach
  ordinary language, state/memory, settings/timer, failure/async, and advanced
  providers without depending on full example files. Move HIR, Wasm
  representation, GC layout, continuation frames, lowering, and DWARF details
  to `COMPILER.md`, `ABI.md`, or a focused developer guide. Give the remaining
  long concept pages task-named navigation and a table of contents.
- [x] Turn the standard-library Markdown into a task-oriented overview linked
  to generated exact symbol pages. Move its catalog IDs, inference internals,
  HIR/Wasm IR, compiler context, and backend dispatch material to compiler
  architecture, and remove manually maintained public-member inventories.
- [x] Replace the flat migration capability table and repeated alphabetical
  catalog renderings with source-first, task-grouped navigation. Within ASL,
  group attachment/state, process/memory, lifecycle/timer, settings,
  collections/text, Unity/emulators, and unsupported host behavior. Detailed
  pages use one consistent shape: source pattern, canonical example, semantic
  difference, supported hosts, and related reference links; keep catalog IDs
  in metadata/URLs rather than reader-facing labels. Keep the C#, JavaScript,
  and Rust guides compact, add a clear next step, and use a few explicit
  source-spelling to SplitScript-spelling pairs rather than another large table.
- [x] Add compact decision guides for lifecycle availability, choosing a state
  field form, failure/async syntax, and string units. The lifecycle matrix must
  state timing, available roots/globals, suspension policy, return type, and
  fallthrough for every action. The other guides should lead from a task to
  `at` versus discovery, required versus optional fields, build-specific shapes
  versus independent discriminators, `T?` / `T!` / `else` / `?` / `retry` /
  `await`, and UTF-8 byte,
  Unicode scalar, UTF-16LE, and managed-string units.
- [x] Publish a renderer-produced static HTML reference for web readers so
  compiler-owned intra-doc links, semantic code highlighting, hierarchy, and
  search work outside the editor. Generate it in CI rather than committing the
  roughly half-megabyte reference tree, validate pages, links, anchors, visible
  examples, and hidden-line removal, and deploy it to GitHub Pages.
- [x] Improve generated reference presentation where catalog facts are
  mechanically correct but user-hostile. Attachment requirements now render as
  one actionable availability rule instead of overlapping facts; structural
  type forms use concise source spellings in indexes, headings, breadcrumbs,
  and member names while retaining capability constraints in their signatures;
  and `onAttach` links its core state, shape selection, suspension, and snapshot
  concepts. Every static example continues through compiler-produced semantic
  rendering.

## P0 — unblock the next representative native ports

### Lifecycle semantics exposed by legacy ASL

- [x] Add timer-global `onStart` and `onReset` actions by sampling
  `timer_get_state` near the beginning of each update, before process
  attachment and state polling. The first update establishes a baseline;
  later `NotRunning -> active` and `active -> NotRunning` transitions fire
  once, including while detached. Process providers, attachment globals,
  `current`, and `old` remain unavailable. The compiler emits the
  monitor global, import, and transition code only when either action exists.
  Script-requested starts and resets are observed naturally on the following
  update rather than being invoked directly and then rediscovered.
- [x] Infer attempt-scoped bare globals from definite `onStart` assignments,
  using the same global syntax and viral requirement analysis as attachment
  state instead of adding a separate attempt block. Generated readiness keeps
  backend defaults unobservable, detach preserves the values, `onReset` can
  inspect them before they are cleared, and a mid-attempt first timer sample
  deliberately remains only a baseline. Lunistice now uses this state without
  dummy initializers or reset bookkeeping in `whileAttached`.
- [ ] Keep ASL `shutdown` and exact `onSplit` delivery as host requirements.
  Shutdown requires the host to invoke a teardown export before disabling,
  reloading, or dropping a module; lossless split/skip/undo ordering requires
  the event contract in R2. Track teardown in R6 of
  [`docs/RUNTIME_EVOLUTION.md`](docs/RUNTIME_EVOLUTION.md).

### State shapes, discovery, and process identity

- [x] Make `process.findMemoryRange(size, access)` wait until a matching range
  exists and return `async MemoryRange`, not `async MemoryRange?`. The current
  optional result is a leftover from the old single-tick synchronous search:
  after the operation became cooperative, resolving to `None` after one
  bounded pass forces every caller to write an outer asynchronous retry loop
  that the API should own. A poll should scan a bounded amount of metadata,
  yield, refresh the range snapshot after a complete miss, and continue until
  it finds a match or attachment cancellation ends the future. Make this a
  direct breaking change without an optional compatibility form. Audit other
  asynchronous discovery APIs for the same accidental “not found yet” option;
  retain optionality only when absence is a meaningful completed result rather
  than temporary discovery state. The only remaining async options are private
  Unity metadata probes, where completed class or field absence is shape
  evidence rather than a request to keep waiting.
- [ ] Add pointer-path shape sharing or overrides only if a maintained port proves that
  repeated pointer paths across many versions are materially unmaintainable.
  Keep the selected physical layout auditable.
- [ ] Add safe full-module enumeration for ports that cannot know every module
  name in advance. Waiting `process.module(name)` and synchronous optional
  `process.loadedModule(name)` cover known-name discovery, but the SEGA Master
  Splitter must select or report whichever libretro core is loaded. Define a
  bounded immutable module snapshot with typed metadata rather than exposing
  host handles or manual `free` calls, and coordinate missing host support
  through the runtime-evolution contract.
- [x] Add a deterministic executable fingerprint for build selection.
  `Module.md5() -> async String!` hashes the module's exact complete on-disk
  bytes and returns canonical uppercase hexadecimal, preserving the evidence
  used by the Bloodstained, Ender Lilies, COTM, and COTM2 sources. The compiler
  reads at most 512 KiB per poll through the portable read-only WASI path,
  closes its descriptor before yielding, restarts when size or modification
  time changes between polls, and is cancelled with the attachment lifetime. This
  deliberately does not weaken an exact source hash to PE version metadata or
  mapped module size.
- [x] Add compiler-owned same-name process selection without exposing PIDs or
  handles. `selectProcess` now evaluates each candidate through the ordinary
  native `process` value before provider setup and `onAttach`; `true` promotes
  the handle, while `false`, fallthrough, postfix `?`, or `throw` rejects and
  detaches only that candidate. The generated module uses the official
  two-pass `process_list_by_name` plus `process_attach_by_pid` ABI only when the
  block exists, rejects truncated process-set snapshots, and preserves the
  direct `process_attach(name)` path otherwise. Candidate order remains
  intentionally unspecified.
- [ ] Finish the remaining official host ABI as typed language facilities,
  preserving semantics without exposing owned numeric handles or manual
  `free` calls. Timer segment history, skip/undo, executable path, host OS, and
  host architecture and same-name process selection are available now. Mapped
  ranges are exposed as a synchronous GC-owned `[MemoryRange]` snapshot with typed
  readable, writable, and executable flags. Dynamic declaration membership is
  available through `settings.contains(key)` without exposing host values;
  represent recursive settings maps/lists/values as GC-owned collections
  and a typed value enum. Preserve atomic `storeIfUnchanged` behavior when
  mutable settings data is eventually exposed. The settings declaration DSL
  remains the normal registration API, and `start`/`split`/`reset` blocks remain
  preferable to duplicate direct timer commands. Coordinate the host-owned
  portions through R1 and R3 in
  [`docs/RUNTIME_EVOLUTION.md`](docs/RUNTIME_EVOLUTION.md).
- [ ] Extend signatures only for corpus-proven gaps: reusable scan targets,
  fallback signatures, range/page selection, capture transforms, relative
  address decoding, and concise pointer-follow composition. Existing `sig`,
  scan, follow, and `readRelative32` APIs should be documented before new APIs
  are introduced. The OpenGOAL and Abe's Oddysee evidence now has one approved
  finite primitive: `process.scanOnce(address, size, signature)` cooperatively
  completes with `Some(address)` or `None` after one full bounded-range pass.
  Waiting scans retain their repeat-until-found behavior, and no unrelated
  discovery API gains a `Once` variant. Keep the remaining extensions gated on
  representative ports rather than broadening signatures speculatively.
- [ ] Design typed byte-order reads for every [`MemoryReadable`] value once a
  representative port needs more than scalar conversion. The design must
  recursively decode structs and fixed arrays, compose with ordinary reads and
  state-field `at` declarations, and make mixed-endian fields auditable without
  per-tick temporary allocations. `Numeric.swapBytes()` now covers the Sonic 3
  A.I.R. scalar case; do not choose the aggregate API before a concrete target
  establishes its shape.
- [ ] Add exact struct layout controls only when a target requires them:
  offsets, padding/alignment, packing, and per-field byte order. Keep
  field-order native-endian layout as the default and diagnose overlaps and
  unsupported combinations.
- [ ] Evaluate adjacent state reads after the existing exact pointer-prefix
  sharing pass. When several fields resolve the same base and cover neighboring
  bytes, compare two user-facing outcomes before choosing one: a contextual
  suggestion to model the memory shape as one [`MemoryReadable`] struct, or a
  lowering pass that safely coalesces the reads without changing per-field
  failure retention. Any automatic form must prove compatible layout guards,
  byte order, alignment, optional fields, overlapping ranges, process-memory
  volatility, and the rule that successful sibling fields may advance when one
  field fails. Do not emit a noisy warning or widen reads across inaccessible
  pages until those semantics and a representative port justify it.

### Polling, mutable watcher patterns, and settings

- [x] Support runtime-dependent state-field activation through ordinary
  conditional field groups selected by enum-valued state discriminators. The
  existing state dependency graph polls each discriminator before its selected
  group, inactive groups perform no reads, and a failed newly active group
  skips that tick without partially replacing the previous accepted snapshot.
  A successful shape transition seeds newly visible `old` / `current` fields
  equally. The MCC acceptance regression maps unknown `gameIndicator` bytes to
  an explicit source enum fallback, proves that Halo 1 and Halo 2 pointers are
  mutually exclusive, and proves that an unknown future game ID reads neither
  group. Attachment-static enum globals cover build-dependent Outer Wilds
  shapes; Rain World and Bloodstained's pointers that become stale while the
  process stays open remain part of the separate provider-refresh lifecycle
  design, not a reason to add a mutable `MemoryWatcherList` object.
- [x] Design and implement one typed associative `Map<K, V>` now that the MGS
  signature/checker dispatch, MGS2 room callbacks, Halo objective guards,
  Bully route tables, and Uncharted Waters route data demonstrate genuine
  runtime key-to-value lookup. The proposal must compare an equality-backed
  insertion-order map with a hashed representation before committing to a
  user-facing `Hashable` capability, and define stable GC identity, replacement
  versus insertion, `get`/absence and indexing semantics, removal, iteration
  order, mutation-during-iteration behavior, inference, structural
  Debug/Display, and reachability-driven code generation. Use `Map`, not a C#
  `Dictionary` alias. The approved implementation is an equality-backed,
  insertion-ordered, stable-identity GC container with `new`, `length`,
  `isEmpty`, `containsKey`, `insert`, indexed read/write (including compound
  assignment), `remove`, `clear`, and entry iteration. Indexing traps for a
  missing key. There is deliberately no `get`: callers that care about absence
  use `containsKey`, which keeps optional values unambiguous. Iteration yields
  structurally destructurable `MapEntry<K, V>` values and supports
  `for { key, value } in map`; replacement preserves order while structural
  mutation invalidates active iteration. Type inference, lazy structural Debug,
  editor completion, generic standard-library pattern usefulness, runtime
  behavior, and reachability-driven code generation are covered. Keep focused
  `Dictionary` / `IDictionary` / `HashMap` migration guidance as a separate
  follow-up if port evidence shows that generic unknown-name diagnostics are
  insufficient. Keep heterogeneous `ExpandoObject` data and runtime settings
  registration as separate designs.
- [ ] Add conditional settings visibility or enablement only with explicit host
  semantics for persisted hidden values and parent changes. Until then,
  document that headings are visual and parent boolean settings must gate child
  behavior in source; do not let a compiling hierarchy imply behavior the host
  does not provide.
- [ ] Distinguish compile-time settings generation from genuinely runtime-named
  settings in port reviews. Bounded level and boss tables should use settings
  families, and statically known entries should use typed `settings.name`
  access. Consider runtime registration only for data discovered from the game
  itself, such as A Short Hike's dynamic tag dictionary, and specify
  persistence, ordering, duplicate keys, live UI updates, and typed value access
  before proposing a host API.
- [ ] Continue `[T]` as the growable ordered sequence instead of adding a
  separate `List<T>` type. Stable wrapper identity, replaceable capacity-backed
  storage, logical length, amortized `push`, and capacity-preserving `clear`
  now preserve aliases across growth and reset; clearing releases live GC
  references without reallocating. `[T; N]` remains fixed-length and does not
  advertise or accept size-changing methods. Source-defined `extend` appends a
  typed array and safely handles self-extension. Source-defined optional
  `pop` now composes indexed access with `removeAt`, returns `None` without a
  structural mutation when empty, and retains capacity. Equality-constrained
  `remove(value)` now removes the first match and reports absence without a
  structural mutation. Indexed `removeAt` shifts in place while preserving
  aliases and capacity, with explicit bounds behavior. Corpus review found no
  current indexed-insertion use, so defer that API until maintained-port
  evidence establishes its semantics. Both array forms retain indexing,
  iteration, search, length, and `values[index] = value` replacement. Plain
  indexed assignment, including compound operators, is non-structural and
  preserves aliases while evaluating the collection, index, and right operand
  exactly once. Structural mutation invalidates active iteration without
  allocating snapshots; preserve that rule for every future collection
  mutator.

### Standard-library and type-system boundaries

- [x] Separate user-facing `Display` from structural `Debug` while retaining a
  boilerplate-free default. An exact `fn Type.toString() -> String` controls
  direct display; otherwise `Display` falls back to `Debug`. Structs, enums,
  arrays, fixed arrays, sets, options, results, ranges, and iterator steps
  derive a stable multiline `Debug` representation conditionally on their
  contained values. Nested strings and characters are quoted and escaped, and
  `fn Type.debugString() -> String` overrides nested formatting without `impl`
  ceremony. Capability checking and hover insight describe custom versus
  derived implementations eagerly, while code generation materializes only
  reachable concrete formatters and recursively reachable custom methods.
  Generated traversal is depth-bounded so mutable recursive container graphs
  render `<cycle>` instead of hanging an autosplitter. Equality, Display, and
  Debug share canonical aggregate metadata and deterministic backend planning.
- [x] Let catalog-defined structural method contracts be satisfied by ordinary
  user methods without `impl` ceremony. `fn Type.toString() -> String` and
  `fn Type.debugString() -> String` satisfy `Display` and `Debug`
  respectively, and implicit conversion calls retain the source method's
  reachability and effects. Standard-library implementations remain explicit
  and privileged.
- [x] Establish the initial user-facing capability policy without adding `impl`
  syntax. Behavioral capabilities are satisfied structurally by exact ordinary
  methods, and associated types are inferred from a complete matching method
  contract. `Debug` remains universally compiler-derived, `Display` uses an
  exact `toString` override and otherwise falls back to `Debug`, and equality
  remains structurally derived. Representation-sensitive `MemoryReadable` and
  the numeric capability hierarchy remain sealed. User-declared capabilities
  stay deferred until scripts demonstrate a concrete need for them.
- [x] Keep capability declarations, requirements, associated types,
  documentation, method lookup, and inheritance in the source-defined
  standard-library model rather than introducing a parallel checker table.
  The checker stores only inferred source implementations of those catalog
  contracts in the semantic model.
- [ ] Add a custom capability handler registry only when the first capability
  cannot be expressed by declared membership, structural equality, structural
  memory layout, or a source-defined implementation.

- [x] Introduce source-defined `Iterable` and `Iterator` capabilities and make
  `for` project its item type through those contracts. Built-in arrays, sets,
  and integer ranges now create first-class cursor values. `next` returns
  `IteratorStep<T>`, whose `Item(T)` and `End` cases keep an iterator over `T?`
  from confusing `Item(None)` with exhaustion. Array and set cursors detect
  structural mutation; direct `for` loops retain their allocation-free
  specialized lowering. Untyped helper parameters infer the minimal
  `T: Iterable` constraint, and the projected `T.Iterator.Item` participates in
  ordinary bidirectional parameter, result, and capability inference.
- [x] Lower `for` over an existing `Iterator` through its ordinary `next`
  protocol, consuming that cursor, while `for` over `Iterable` retains an
  independent traversal. Lazy source-defined `map` and `filter` adapters store
  ordinary closures, compose without intermediate collections, participate in
  generic capability dispatch, and remain live across suspending loop bodies.
  Iterator cursors themselves implement `Iterable`; calling `iterator()` is an
  identity operation that preserves the current cursor position. Generic
  `T: Iterable` loops lower through `T.iterator()` and `Iterator.next`, while
  monomorphic collection and range loops retain their direct fast paths.
  Constructed callable layouts, generic standard-library struct fields,
  reachability, scratch planning, and intrinsic dependencies all use the same
  demand-driven specialization path rather than adapter-specific intrinsics.
- [x] Add synchronous generator functions as the ergonomic source-defined
  iterator facility. `fn values() -> iterator T { yield value }` constructs a
  lazy resumable cursor: the call executes no body code, each `next()` resumes
  until one `yield`, and fallthrough or bare `return` permanently yields
  [`End`]. Generator values implement both [`Iterator`] and identity
  [`Iterable`], alias one cursor when copied, and compose with `for`, `map`,
  `filter`, generic capability dispatch, and closures. Reuse async continuation
  graphs, suspension liveness, typed GC frames, captures, specialization, and
  dynamic frame dispatch behind one resumable-frame architecture; add `Yield`
  as a distinct IR terminator rather than disguising a yielded item as future
  completion. Keep generator construction lazy and generated implementations
  reachable on demand. Initially reject `await`, `retry`, value-returning
  `return`, and uncaught failure inside generators: fallible and asynchronous
  iteration need separately approved protocols and must not be flattened into
  synchronous `IteratorStep`. Cover syntax recovery, formatting, inference,
  diagnostics, Debug, hover, navigation, highlighting, completion, selection,
  documentation, nested control flow, aliasing, permanent exhaustion, generic
  calls, closures, runtime behavior, and Wasm validation.
- [x] Make `iterator T` the single public erased iterator type. Arrays, sets,
  maps, integer ranges, `map`, `filter`, and generators now expose the same
  type; ordinary functions can forward any existing iterator without becoming
  generators. Concrete cursor layouts and their raw intrinsics remain private
  standard-library implementation details and are absent from name resolution,
  completion, hover, and generated documentation.
- [x] Implement one hidden exact-runtime-representation analysis for erased
  values. Public types remain `iterator T`, `async T`, and ordinary callable
  types; one backend fact domain now retains a proven generator frame, future
  frame, closure, or named-function target through locals, forwarding calls,
  and unambiguous control flow. Exact iterator advancement, future polling, and
  callable invocation use direct calls. Heterogeneous aggregates, conflicting
  joins, escaping values, and unknown calls monotonically fall back to the
  existing dynamic dispatch, and a shared function body is never cloned merely
  because its callers have different producers. This stays separate from type
  inference and capability semantics, so it cannot change which programs
  type-check. The release Lunistice fixture shrank from 45,921 to 42,780 bytes
  (3,141 bytes / 6.8%), while the complete iterator, closure, and async runtime
  suites retained their behavior.

### Engine and emulator providers

- [x] Make physical `MemoryReadable` layout depend on the concrete reader
  context. Native `address` fields use the attached process's detected pointer
  width and alignment; nested structs, fixed arrays, field offsets, aggregate
  sizes, and array strides derive recursively from that context. Emulator
  readers retain their catalog-owned guest width and byte order. Cache layouts
  by type and context, reserve bounded scratch for the largest supported
  layout, and show both offsets in hover when a native struct differs between
  32-bit and 64-bit targets. Ordinary native pointer paths should infer the
  process width. Explicit mixed-width reads remain deliberately unimplemented
  until a real target proves the need and their public spelling is approved.
- [x] Decide and implement the source-defined provider mapping lifecycle before claiming
  parity for emulator cores that unload without their host process exiting.
  Every emulator provider now owns a private synchronous mapping-validation
  operation in the catalog. A failed validation ends the logical attachment
  while retaining the still-open emulator process: attachment-scoped state and
  continuations are cleared, `onDetach` runs for a completed attachment, and
  ordinary cooperative discovery starts again. A replacement mapping runs
  `onAttach` and seeds a fresh `old` / `current` baseline. Native and Unity
  providers opt out; the contract is general without adding public syntax.
- [x] Finish the schema-first Unity value surface rather than adding another
  public Mono path API. Bounded managed strings, scalar values, singleton and
  static roots, and nested references should all be expressible in `image` /
  `class` declarations and consumable by ordinary state snapshots. Use the
  campaign's `mono.Make<T>` and `mono.MakeString` cases to test the schema, its
  diagnostics, and its documentation; keep `staticFieldPath`, raw field offsets,
  object-header arithmetic, and backend-specific target-family traversal
  private implementation tools. Bring any schema shape that still cannot
  represent a real port back as a design question before adding public syntax.
  The final Assemble with Care gap required no new syntax: field binding now
  retains the declaring runtime class found along the inheritance chain and
  resolves each static field through that class's storage. A concrete schema
  can therefore consume a singleton declared by a closed generic base with an
  ordinary `static Concrete instance from "_instance";` declaration. A focused
  Mono fixture keeps distinct base and derived static tables and rejects the
  previously plausible wrong-table value.
- [ ] Verify and extend PS2 guest-memory coverage against ASR before changing
  the provider boundary. Resident Evil Code: Veronica X reads its disc product
  code at `0x00015B90`, below SplitScript's documented
  `0x00100000..=0x01ffffff` domain, so the canonical provider cannot auto-select
  a region even though the legacy script can. Determine whether ASR deliberately
  excludes the low region, whether PCSX2 mappings expose it consistently, and
  how RetroArch cores differ. Then propose the correct provider domain and
  runtime validation; do not paper over the gap with a game-specific exception.
  A 2026-09-02 audit of the current local ASR source confirms that its public
  PS2 address conversion still documents and enforces
  `0x00100000..=0x01FFFFFF`. Wait for ASR to establish and test the broader
  domain before changing SplitScript.
- [ ] Assess an Unreal provider only after representative `GWorld`, object, and
  name traversal ports establish the required surface.
## P1 — expand migration guidance and automated fixes

- [x] Expand the structured foreign-spelling entries beyond the existing
  declarations, option value, strings, durations, and numeric types. Add new
  entries only for corpus-proven, unambiguous spellings that are not already
  handled by the type-aware callable suggestion machinery. Exact `HashSet`,
  `Dictionary`, `IDictionary`, and `HashMap` type spellings now recover to the
  canonical `Set` and `Map` names with machine-applicable fixes. Tuple
  collections lead to named structs, `Environment.TickCount` and `Stopwatch`
  lead to event-anchored `Instant` use, and duplicate native state searches lead
  to one state with shape globals and conditional fields. Exact searches for
  `File.GetLastWriteTime`, JSON, and `Thread.Sleep` now identify their pending
  host/data design or cooperative alternative rather than returning a
  misleading nearby symbol. Canonical syntax remains unique; no compatibility
  aliases were added.

## P1 — reduce generated Wasm size with Release-only compiler optimizations

The [promoted size pipeline](docs/WASM_SIZE_OPTIMIZATION.md) performs bounded
Release-only instruction, integer-constant, dead-code, local and control-flow
cleanup, shared returns, and sharing of bodies differing in integer constants.
With instruction, constant-local, global and control cleanup plus direct default-enum
emission, conditional/fallthrough cleanup and cost-based GC literals, it saves 15.5% / 15.0%
on Minish Cap / Lunistice relative to pre-optimizer master. The external Celeste
port is also measured and saves 17.1%. Debug bypasses all passes and also benefits
from the default-enum emission shortcut, which adds no analysis.
[Single-reference inlining with control-flow liveness and type pruning](docs/WASM_SIZE_OPTIMIZATION.md#control-flow-liveness-and-inlining--2026-10-02)
adds 523 / 865 / 533 bytes of savings on Minish Cap / Lunistice / Celeste. The
complete module must shrink after cleanup, and all added analysis is Release-only.
[Local layout and conditional exits](docs/WASM_SIZE_OPTIMIZATION.md#local-layout-and-conditional-exits--2026-10-02)
add another 150 / 174 / 121 bytes on those scripts (845 on automatic Lunistice).
[Private argument specialization](docs/WASM_SIZE_OPTIMIZATION.md#private-argument-specialization--2026-10-02)
adds 88 / 96 / 50 more (491 on automatic Lunistice), with the original full
pipeline retained as a size fallback.
[Temporary structs and shared branch tails](docs/WASM_SIZE_OPTIMIZATION.md#temporary-structs-and-shared-branch-tails--2026-10-02)
save another 227 / 118 / 197 bytes on Minish Cap / Lunistice / Celeste and
779 on automatic Lunistice. Only private allocations with proven field-only
uses become scalar locals; constructor effects, traps and per-iteration defaults
remain observable. All added analysis stays in Release.
[Repeated-expression reuse](docs/WASM_SIZE_OPTIMIZATION.md#repeated-expression-reuse--2026-10-02)
saves another 216 / 366 / 151 bytes on Minish Cap / Lunistice / Celeste, 307 on
A Hat in Time and 1,597 on automatic Lunistice. It runs after the existing
pipeline and requires each changed function and the complete module to shrink.
[Cleanup after expression reuse](docs/WASM_SIZE_OPTIMIZATION.md#cleanup-after-expression-reuse--2026-10-03)
saves another 53 / 34 / 36 bytes on Minish Cap / Lunistice / Celeste. It removes
discarded nontrapping calculations and revisits local/control cleanup, while
preserving operand effects, traps and non-defaultable reference initialization.
Larger branch-fact, local-allocation and multi-use-wrapper trials did not justify
promotion; Binaryen still saves 671 / 565 / 441 bytes on those three scripts.
The earlier broader inlining/temporary-sinking and propagation prototypes remain
on the experimental branch; their remaining real-script benefit does not yet justify promotion. Prefer
measured savings on Minish Cap, Lunistice and Celeste over large synthetic fixtures. Direct emitter
fixes that add no analysis may apply to both profiles.

Implement in this order:

1. [x] Establish a representative Release size corpus, including small native
   scripts, explicit-profile and automatic-profile Unity scripts, nested managed
   collections, and async code. Record final module bytes and section sizes
   under one fixed emission pipeline, with optimization enabled and disabled.
   Review actual size changes for every pass rather than assuming an IR
   simplification makes the executable smaller.
2. [ ] Extend the current integer folding and uniformly constant-local propagation
   to broader constant propagation over typed IR, followed by
   unreachable-branch and unused-value elimination. Preserve integer widths,
   overflow, floating-point behavior, traps, fallible results, evaluation order,
   and observable effects. Reuse existing constant evaluation where its
   semantics match runtime evaluation; never evaluate process reads or other
   host effects at compile time.
3. [ ] Establish useful additional savings from size-driven inlining before
   promoting the existing experiment. Account for removal of the original
   function, call overhead, locals, retained dependencies, and cleanup already
   performed by the promoted pipeline. Treat recursion and suspension
   boundaries conservatively. Do not use hotness or runtime-speed heuristics.
4. [ ] Recompute reachability after simplification and inlining so newly unused
   functions, runtime helpers, imports, types, static data, and scratch regions
   can disappear. Integrate this with existing demand-driven emission instead
   of maintaining a second dependency model.
5. [x] Verify equivalent observable behavior with focused runtime tests and the
   maintained corpus, confirm Debug emission is unaffected, and record measured
   byte savings and compiler resource costs. Enable the proven passes only for
   Release. Add further passes only when size measurements justify them.

## P1 — measure and improve interactive compiler queries

- [x] Build one request-scoped completion context from the existing recovered
  `SourceDocument`, token stream, syntax, cursor, and lazy semantic facts.
  Remove repeated `lex_lossless` calls, use the current database before a
  repair probe, return compact receiver facts rather than cloned programs, and
  use binary search/`partition_point` for ordered token and definition-reference
  cursor lookups with boundary/trivia regressions.
- [x] Replace deep per-stage ownership copies with shared immutable compiler
  products. Share source documents, syntax, and stable resolution inputs through
  `Arc` or borrowing; let each later stage own only transformed facts. Measure
  parse, diagnostics, semantic tokens, hover, and completion on one database
  before and after, rather than hiding the issue behind additional `Clone`
  implementations. Parse, lower, check, and recovery now share one immutable
  source product; lowered declarations and resolution inputs are shared through
  strict and recovered checking. The exact-parent 30-sample runner improved
  every cold generated-source editor query, including diagnostics, both
  completion shapes, hover, and semantic tokens, while reducing the retained
  complete-query cache from 13.8 MiB to 9.6 MiB. See `docs/BASELINES.md` for the
  stage and query data.
- [x] Bound recovery and diagnostic construction on large foreign inputs. The
  shared recovering lexer now resumes from the failed token boundary instead of
  re-lexing the complete prefix after every error, with an explicit no-progress
  fallback for diagnostics whose span excludes the triggering token. Published
  compiler errors have a deterministic 100-diagnostic budget plus an omitted
  count while semantic recovery still sees the complete source. Generated
  regressions cover thousands of independent lexical failures and token-local
  no-progress recovery without depending on the disposable corpus. The same
  165,809-byte legacy ASL that exceeded 60 seconds at revision `57f2564` now
  returns the bounded, useful diagnostic set in about 0.11 seconds through the
  `max-opt` CLI on the same machine.

## P1 — source-level debugging after the debugger boundary is chosen

Debug builds should support breakpoints and source stepping in `.split` files;
release builds must remain stripped. Embedded DWARF is the source-level format.
Do not add JavaScript source maps. Further implementation is deliberately
paused until the JavaScript-debugger experiment is compared with the current
native Wasmtime path and a typed-IR interpreter; do not let partially working
DWARF displace current porting and language-correctness work.

### Prove the Wasmtime/debugger boundary

- [ ] Before extending native DWARF further, evaluate the JavaScript debugger's
  WebAssembly support against the real Wasmtime host, especially GC objects.
  If it still cannot provide a coherent SplitScript-level experience, compare
  a language-native debugger that interprets typed IR with continuing to adapt
  native debuggers. Do not commit to a custom DAP until this experiment shows
  which boundary actually preserves source values and control flow.
- [ ] Build a minimal fixture against the exact Wasmtime configuration used by
  LiveSplit. With `Config::debug_info(true)`, verify source breakpoints,
  stepping, stacks, scalar locals/globals, and GC references across supported
  debugger/platform combinations. Source stepping works in the Windows host.
  Wasmtime's native-DWARF transform now preserves source subprogram names and
  direct scalar local/parameter location lists: direct values use its required
  trailing `DW_OP_stack_value`. A Windows CodeLLDB run against the real host now
  resolves `setup`, `add`, and `whileAttached`, binds source breakpoints, and
  displays a live scalar local. Emit the supported `DW_LANG_C11` compatibility
  language until SplitScript has an LLDB plugin: `DW_LANG_lo_user` leaves names
  and locals hidden. Deliberately omit compilation-unit PC ranges so LLDB derives
  ownership from complete child subprogram ranges; Wasmtime 45's generic unit
  range transform drops native regions for non-monotonic control flow. Continue
  with parameter-liveness cases, globals, GC-backed values, and other debugger /
  platform combinations. Wasmtime 45 explicitly discards
  `DW_OP_WASM_location` for globals and operand-stack values, so compiler tests
  that merely decode those expressions are insufficient.
- [ ] Design a debugger-visible representation for source globals. Prefer a
  debug-only, runtime-independent shadow location in linear memory over
  hard-coding Wasmtime's private VMContext layout; update the shadow on every
  source-global write and prove scalar inspection before attempting GC values.
- [ ] Record Wasm GC inspection as an experimental result. The DWARF-for-Wasm
  convention locates Wasm values but does not by itself define traversal of
  `structref`/`arrayref`. If standard consumers cannot inspect aggregates,
  expose honest opaque references first; do not describe Wasmtime's moving GC
  heap as C-style memory.

### Preserve provenance and emit metadata

- [x] Carry one single-file source identity through parsing, lowering,
  checking, and backend planning. CLI builds use the absolute input path,
  extension builds use VS Code's native file path (or the URI for non-file
  documents), and intentionally path-less APIs use deterministic `input.split`.
  Do not introduce general `FileId` infrastructure before a real multi-source
  feature.
- [x] Retain source origins for all typed-HIR constructs. Expression and
  statement/control-flow origins now survive Wasm IR lowering and movement into
  async poll bodies, and explicit suspend/resume boundaries retain the original
  `await`/`retry` span while generated runtime scaffolding has no source
  location. Executable enum and aggregate global initializers map `_start` rows
  to their declarations; primitive constant expressions correctly have no
  executable breakpoint address.
- [ ] Extend the profile-aware `DebugArtifactPlan` beyond its completed final
  function-index and function-body maps. Expression instruction boundaries are
  recorded during encoding, rebased to Code-section-relative DWARF addresses,
  and verified against `wasmparser`; async suspend/resume boundaries use
  distinct line discriminators. Add GC-layout and async-frame-location plans.
- [x] Emit a deterministic WebAssembly `name` section for every imported and
  defined function in debug builds, including runtime helpers, generic source
  specializations, async init/poll functions, lifecycle/state readers, and the
  exported entry points. Release modules contain no `name` or `.debug_*`
  sections.
- [ ] Extend the same `name` section beyond the completed source-owned globals,
  parameters, and direct-function locals. Add GC types, fields, settings
  storage, and honest names for values moved into async frames after their final
  index plans expose stable source identities.
- [x] Emit deterministic DWARF v5 compilation-unit, source-backed subprogram,
  and expression line-table sections with `gimli::write`. Tests decode the
  result and require every row to land on a real Wasm instruction boundary;
  release output remains stripped.
- [ ] Extend the completed primitive scalar types, source-global/direct-function
  parameter locations, declaration-to-scope local ranges, and statement/control
  flow rows with enums and honest async-frame variable locations. Suspension
  and resumption already have distinct rows at the source `await`/`retry` span.
  Add GC aggregates only to the level proven usable by the compatibility fixture.

### Integrate the host and editor only after metadata works

- [ ] Add a debug-profile marker to the SplitScript ABI metadata and enable
  Wasmtime debug transforms only for debug sessions. Measure startup, tick, and
  memory overhead.
- [ ] Prove a manual native-debugger workflow in the real host before adding VS
  Code UI: attach, state polling, lifecycle actions, retry/await resumption, and
  module reload.
- [ ] Prefer a thin VS Code launch/attach integration with a supported native
  adapter. Build a SplitScript DAP only if the compatibility experiment proves
  a concrete source-language or GC inspection gap.
- [ ] Add deterministic artifact tests, backtrace/source-line coverage, and a
  documented host/Wasmtime/debugger/platform capability matrix.

References:

- [WebAssembly name section](https://webassembly.github.io/spec/core/appendix/custom.html#name-section)
- [DWARF for WebAssembly](https://yurydelendik.github.io/webassembly-dwarf/)
- [Wasmtime native debugging](https://docs.wasmtime.dev/examples-debugging-native-debugger.html)

## P1 — harden and publish the portable toolchain

The extension already bundles the same Rust compiler and language service as
optimized core Wasm in separate browser-compatible workers. Desktop and web
extension hosts support language features, release builds, and revision-safe
debug watch without external executables. Native `splitc` and `splitls` remain
separate first-class products. The architecture experiment is complete; the
remaining work is product hardening and distribution.

- [x] Publish a platform-neutral VSIX from every verified `master` push. CI
  moves the stable `latest` tag and replaces `splitscript-latest.vsix` on one
  durable GitHub release; pull requests and other branches remain read-only.

- [ ] Publish extension version 0.1.0 through the Visual Studio Marketplace's
  pre-release channel. Packaging already applies the VSCE pre-release marker,
  the manifest identity is `LiveSplit.splitscript`, and Marketplace-facing
  documentation is ready. Smoke-test the complete CI-assembled VSIX rather than
  a single-platform local package, then upload that immutable artifact through
  the `LiveSplit` publisher. Keep later Marketplace uploads explicitly
  versioned; the rolling GitHub `latest` release may continue replacing its
  separately distributed asset.

- [x] Add cooperative cancellation points to expensive compiler stages so a
  superseded editor build can stop work rather than merely have its completed
  response discarded. The shared compiler and service distinguish typed
  cancellation from diagnostics; the embedded worker retains opaque analysis
  and Wasm-IR stages, yields between them, and discards superseded debug-watch
  revisions before publication. Add finer-grained checks inside a pass only if
  measurement shows one stage still blocks for too long.
- [ ] Measure repeated full-size builds, warm language queries, memory recovery,
  and worker restarts in desktop, web, and virtual workspaces. Keep the language
  worker responsive while the separate compiler worker builds.
- [x] Recover the measured 3–5% one-shot release-compilation regression since
  the 2026-08-31 optimization checkpoint. Sequential 200-sample runs established
  the overall change; alternating parent/child runs isolated the larger step to
  `c0bbf93` (whole-file APIs), despite byte-identical output. Cache the
  compiler-owned rendered standard-library body source once per validated graph
  and index hidden source functions by name for type checking and lowering.
  Current medians are within -1.2% to +1.8% of the checkpoint while preserving
  the 5.1% smaller Lunistice release Wasm.
- [ ] Reduce the remaining roughly 46 ms fixture-independent one-shot compiler
  cost. A release-stage probe attributes about 24.9 ms to parsing augmented
  standard-library source and only 0.067 ms to exact resolution. Design a parsed
  semantic cache or reachability-based body selection that preserves validation,
  diagnostics, deterministic source ranges, and generated bytes; do not clone a
  token stream whose owned token text costs as much as lexing it.
- [x] Complete packaging audits in `cargo xtask check`: generated Wasm/binding
  freshness, optimized artifact size, VSIX contents, no unintended native
  binaries or network bootstrap, worker failure recovery, stale-result
  suppression, and local/virtual output writes. The debugger's five declared
  `splitscript_process_native.node` platform bridges are intentional package
  contents; reject native files outside that closed manifest rather than
  applying the older no-native-binaries assumption.
- [ ] Test one platform-neutral VSIX on Windows, Linux, macOS, desktop, remote,
  and web hosts.
- [x] Publish a native `splitc` / `splitls` ZIP for Windows x64 and tarballs for
  Linux x64 and ARM64 and macOS Intel and Apple Silicon from the same rolling
  `latest` release as the VSIX. Each native runner builds and starts both tools,
  packages the installation guide, and contributes to one release checksum
  manifest.
- [x] Run the complete compiler and runtime conformance corpus on every native
  release platform rather than only building and smoke-testing its tools. The
  shared `cargo xtask conformance` boundary runs all compiler tests, builds the
  exact revision-stamped max-opt CLI/LSP binaries, compiles and validates every
  fixture, and executes every runtime harness on Windows x64, Linux x64/ARM64,
  and macOS x64/ARM64 before those same binaries are packaged.
- [x] Document batteries-included extension installation separately from native
  CLI/LSP installation, including supported hosts, package size, memory use,
  debug-watch output, and failure recovery.

### Hosted browser IDE

- [ ] Build a deployment-sized Code OSS web proof of concept with the packaged
  SplitScript extension preinstalled. Record upstream/license obligations,
  branding, extension-gallery policy, static hosting requirements, payload,
  startup, memory, and maintenance cost. `@vscode/test-web` remains test
  infrastructure, not a redistributable product.
- [ ] Define persistent browser workspaces and Wasm artifact download/export,
  then add curated examples, templates, documentation links, and a settings
  preview without forking the compiler or language service.
- [ ] Package the workbench and extension with content hashes and a strict CSP;
  test Chromium, Firefox, and Safari. Keep a custom Monaco shell only as a
  fallback if Code OSS proves unsuitable.

## P1 — remaining language and runtime breadth

- [x] Make array comparison and matching coherent with their value semantics.
  Both `[T]` and `[T; N]` derive structural `Equatable` when `T: Equatable`, and
  exact recursive array patterns support nested tests and bindings. Fixed-array
  lengths are checked statically; growable lengths are tested safely at runtime.
- [x] Add recursive pattern alternation with `patternA | patternB`. Alternatives
  are represented directly rather than as duplicated arms, contribute their
  union to exhaustiveness, try left to right, and retain one guard/body scope.
  Every alternative binds the same names with compatible types; repeated
  occurrences share hover, navigation, rename, highlighting, and debug identity.
- [x] Make wrapper and enum payloads recursive patterns rather than
  binding-only slots. `Some("Inf") | Some("inf")`, `Ok([first, second])`, and
  nested enum payload patterns now share the same inference, exhaustiveness,
  tooling, and code-generation path as array patterns and bindings.
- [x] Add partial field-based struct patterns. `Point { x, y: 0 }` binds `x`,
  recursively tests `y`, and ignores omitted fields without a `..` marker.
  Shorthand fields retain both the destination-field and arm-binding identities
  for hover, navigation, highlighting, references, and safe rename expansion;
  constrained patterns require a fallback while binding-only partial patterns
  are irrefutable.
- [x] Make exhaustiveness and reachability recursive across finite nested
  constructors. Multiple arms can jointly cover wrapper and enum payloads,
  struct fields, and fixed-array elements while preserving correlations;
  missing nested cases produce a concrete witness when one is useful.
- [x] Add type-directed completion inside patterns. Offer only constructors
  legal for the matched type: enum variants, `Some` / `None`, `Ok` / `Err`,
  `Item` / `End`, booleans, remaining struct fields, fixed-array shapes, and
  recursive subpattern snippets. Suppress expression-only keywords and generic
  declarations in pattern position. Keep the candidate model compiler-owned
  so terminal/editor clients do not duplicate pattern grammar or type logic.
  Completion now derives the matched and nested payload types from the
  recovering semantic snapshot, recognizes half-written arms and delimiters,
  narrows qualified enums, omits already-used struct fields, and hands one
  compiler-owned candidate list to every editor frontend.
- [x] Add closed integer range patterns using the language's unambiguous `..<`
  and `..=` bounds. Literal bounds, including negative signed literals, bind
  more tightly than alternation. Use interval-based usefulness analysis rather
  than enumerating values, so finite integer domains can be proven exhaustive
  and fully covered ranges become unreachable while partial overlaps remain
  useful. Reject empty, reversed, bare-`..`, non-integer, and chained forms;
  cover code generation, completion, hover documentation, and language guides.
- [ ] Revisit range-pattern breadth when a concrete use case needs it:
  open-ended or character intervals and general `value @ pattern` binding.
  Keep these orthogonal to the closed integer interval semantics rather than
  overloading `..` or adding a range-specific binding special case.
- [x] Add array rest patterns for fixed and growable arrays. One `..` inside an
  array pattern matches a variable-length middle, so `[first, ..]`,
  `[.., last]`, `[first, .., last]`, and `[..]` share one prefix/rest/suffix
  representation through parsing, inference, usefulness, code generation,
  completion, hover, and documentation. Fixed arrays validate their minimum
  explicit length; growable arrays use symbolic length representatives for
  exhaustiveness and reachability. The rest binds and copies nothing. A future
  general `value @ pattern` feature may add a typed rest binding once the
  language has an appropriate slice or view type, without making array rest a
  special binding form.
- [x] Add `expression is pattern` as an ordinary boolean expression with
  path-sensitive pattern bindings. Reuse the complete recursive match-pattern
  grammar and evaluate the scrutinee exactly once; a mismatch produces `false`
  rather than requiring exhaustiveness. Give `is` comparison precedence above
  `&&` and `||`, and require `!(value is pattern)` when negating the complete
  test.
  - [x] Introduce one condition-flow analysis with explicit true and false
    outcomes. Carry pattern bindings through parentheses, `!`, short-circuit
    `&&`, and short-circuit `||` alongside the compiler's existing shape
    refinements. The right operand of `&&` receives facts from the left true
    edge; the right operand of `||` receives facts from the left false edge;
    joined paths retain only bindings initialized with the same identity on
    every incoming edge.
  - [x] Make the resulting bindings available on the proven edge of executable
    condition consumers: `if` then/else branches, `while` bodies, later
    short-circuit operands, and match-arm bodies after a successful guard.
    Never leak a binding merely because the boolean is stored or passed, never
    carry it through eager boolean/bitwise operators or calls, and keep lexical
    value blocks and callable boundaries as scope barriers. Reject a second
    live conditional declaration with the same name rather than silently
    shadowing it.
  - [x] Lower `is` through the existing typed pattern representation and
    pattern-test emitter. Allocate binding locals only on the successful path,
    preserve them through ordinary short-circuit control flow and async
    continuation frames, and keep failed partial matches unobservable.
  - [x] Generalize compiler-owned pattern completion, hover, navigation,
    references, rename, semantic highlighting, selection ranges, and
    documentation from match arms to `is` patterns. Diagnose uses on an edge
    where the declaration is not definitely initialized, allow irrefutable
    binding patterns with an always-matches warning, and allow binding-free
    `is` tests in static shape predicates while rejecting declarations that
    have no executable lexical scope.
- [x] Generalize patterns into irrefutable binding declarations everywhere
  executable code introduces values. Local and initialized global `let`
  declarations, named-function parameters, closure parameters, and `for`
  bindings should accept the same recursive pattern grammar as `match` and
  `is`; schema fields and uninitialized attachment-scoped globals remain named
  declarations. A declaration consumes one value, evaluates it once, and
  projects every leaf binding without flattening function or closure ABI
  parameters.
  - [x] Replace declaration-specific name slots with one shared syntax and
    checked-pattern representation. Keep a compiler-owned root value for the
    incoming aggregate, preserve one `ValueId` per source binding, and reuse
    the existing resolved field/variant/wrapper projections rather than adding
    destructuring-only resolution tables.
  - [x] Require declaration patterns to be semantically irrefutable. Use the
    recursive usefulness engine to prove that no inhabited value is missing,
    and centralize cycle-safe inhabitedness so `Never` removes impossible enum
    variants, wrapper cases, struct values, and nonempty fixed arrays. Produce
    concrete missing-pattern diagnostics for genuinely refutable declarations.
  - [x] Lower locals, globals, parameters, closures, synchronous and async
    `for` loops through one binding-plan emitter. Evaluate each initializer,
    argument, or iterator item once; preserve source mutability, captures,
    suspension liveness, unused analysis, debug identities, and global
    initialization ordering for every projected leaf.
  - [x] Generalize formatting, completion, hover, navigation, references,
    rename, semantic highlighting, selection ranges, symbols, and inlay hints
    to declaration patterns. Annotate the whole incoming parameter/value while
    exposing the projected type and declaration identity on each leaf binding.
  - [x] Document declaration destructuring, its differences from refutable
    `is` / `match`, the whole-value meaning of type annotations, exact supported
    binding sites, and `Never`-aware irrefutability. Add parser, diagnostic,
    inference, runtime, async, global, tooling, and documentation coverage.
  - [x] Permit contextual anonymous struct patterns. `{ x, y }` resolves to the
    one concrete nominal struct supplied by an initializer, scrutinee, iterator
    item, wrapper payload, or explicit whole-value annotation; it never infers
    a structural type from field names. Preserve explicit `Point { x, y }`,
    reject unconstrained uses with one focused diagnostic, and share field
    completion, hover, navigation, highlighting, and safe shorthand rename.
- [x] Add shorthand struct field initializers: `Point { x }` means
  `Point { x: x }`. When an explicit initializer repeats the exact field name,
  emit a warning with a machine-applicable rewrite to the shorthand. Rename
  must preserve meaning in both directions: renaming either the field or the
  referenced local independently expands shorthand back to an explicit
  `newField: oldLocal` or `oldField: newLocal` initializer as appropriate;
  renaming both together may retain shorthand. Cover parsing, formatting,
  inference, hover, navigation, references, highlighting, and extraction.

- [ ] Design exact host-driven `onSplit` delivery. It must fire even when no
  game process or emulator is attached and distinguish an ordinary split from
  skips, undos, and multiple timer operations between updates. Specify the
  segment identity, whether it is observed before or after advancement,
  ordering relative to polling and detach, reentrancy, and suspension. Keep
  attachment-dependent roots unavailable rather than fabricating stale
  snapshots. The current runtime only calls `update`; R2 in
  [`docs/RUNTIME_EVOLUTION.md`](docs/RUNTIME_EVOLUTION.md) is the canonical
  runtime-side requirement.
- [ ] Design the remaining typed least-privilege timer/run API without
  redeclaring facilities that already exist. `timer.state()`, optional
  `currentSplitIndex()`, segment history, skip/undo, and explicit game-time
  pause/resume are available now and first need better porting discovery. The
  residual host surface is timing method, category/game/attempt metadata,
  current segment name and run count, timer real/game-time observations, and run
  offset. The SEGA Master Splitter needs game/category identity, while Abe's
  Oddysee needs exact current real/game time for correctness and persistent
  feedback rather than display alone. Separate read-only observations from
  mutations, distinguish the monotonic `Instant` clock from timer real time,
  and add ABI support only where the host can expose stable semantics. Use the
  repeated `timer.CurrentTime`, `timer.CurrentSplit.Name`, `timer.Run.Offset`,
  category, and timing-method ports as the evidence ledger; coordinate the host
  side through R5 in
  [`docs/RUNTIME_EVOLUTION.md`](docs/RUNTIME_EVOLUTION.md). The current runtime
  does not expose those observations at all, so defer the SplitScript source
  shape until a host contract exists. In particular, do not introduce an
  update-wide snapshot abstraction speculatively. LiveSplit Core's internal
  point-in-time snapshot only freezes clock calculations performed through one
  observation; whether any analogous value belongs in the autosplitting ABI or
  source language must follow the runtime design.
- [ ] Decide the remaining imperative timer-control boundary separately from
  read-only timer observations. Ato pauses the full timer phase and Spider-Man
  resets when its emulator closes; SplitScript currently exposes declarative
  `start` / `split` / `reset` decisions plus game-time pause/resume, which do not
  express either behavior. Determine which operations the runtime can support
  safely, whether lifecycle-triggered control is desirable, and how feedback
  loops are prevented. Bring the language/stdlib contract back for approval and
  document intentional non-goals rather than silently omitting source behavior.
- [ ] Design the writable/persistent half of the file API from the Abe's
  Oddysee acceptance case. `File.readAllBytes` and `File.readAllText` now cover
  whole-file input without handles, but a faithful split database also needs
  directory creation, atomic replacement or whole-file write, and deliberate
  deletion; Bloodstained and the SEGA Master Splitter add diagnostic-log use
  cases. Specify autosplitter-relative versus absolute paths, WASI preopen and
  host-consent policy, encoding, overwrite/atomicity, interruption, size limits,
  and release behavior before exposing mutations. Prefer handle-free whole-file
  operations unless a real port proves streaming is necessary, and bring this
  standard-library and sandbox decision back for approval.
- [ ] Design read-only file change metadata for Outer Wilds-style save polling.
  Decide whether the smallest stable API is an opaque change token or explicit
  size and modification time, and specify missing-file behavior, timestamp
  precision, fallibility, suspension, and the guarantees available through the
  host's WASI view. Do not introduce a general `DateTime` model merely to copy
  `File.GetLastWriteTime`; bring the standard-library shape back for approval
  and test it against real replacement, truncation, and same-size updates.
- [ ] Design JSON support after the typed `Map` representation is settled, using
  the Outer Wilds save file as the acceptance case. Cover UTF-8 objects, arrays,
  strings, numbers, booleans, and null; typed inspection, useful parse errors,
  numeric fidelity, duplicate object keys, ordering, depth and size limits, and
  release-size cost. Keep JSON parsing separate from file polling and runtime
  settings, and bring the value/API shape back for approval before implementation.
- [ ] Complete structured future composition with explicit cancellation
  semantics. `future.race([async T])` and
  `future.timeout(async T, Duration) -> async T!` are implemented with lazy
  construction and one producer-agnostic polling path for source functions,
  closures, and intrinsic futures. Timeout starts its monotonic deadline on
  first poll, gives a ready operation deadline priority, and reuses an
  operand's existing fallible channel instead of creating `T!!`. Do not expose
  threads or unconstrained background tasks. Keep bounded concurrent scanning
  as a separate scheduling decision rather than silently making an unbounded
  race array consume arbitrary work per update.
- [ ] Broaden suspending control flow incrementally from real ports and add a
  host-executed conformance fixture for each new shape.
- [ ] Finish first-class function values and lexical closures for iterator
  adapters, separately from legacy delegate migration. The maintained Axiom
  Verge port already established that its three callback-shaped C# delegates
  are clearer as ordinary typed functions, so callbacks alone are not evidence
  for a delegate/event model. Lazy `map` and `filter` do provide a distinct
  reason to store callable behavior. Callable type syntax, arrow closures,
  bidirectional parameter/result inference, invocation, independent closure
  bodies, typed Wasm GC function references, immutable environments, and
  shared GC cells for mutable captures are implemented. Shared cells work for
  returned and nested closures and survive async continuation frames. Untyped
  higher-order helpers infer callable parameters from invocation and specialize
  those signatures independently at each concrete call site. Async closure
  bodies now use the same typed continuation-frame and runtime-tag dispatch as
  source async functions, including captured values and mutable parameters
  that survive suspension. A closure declared in a generic helper is emitted
  independently for every concrete helper instance, including its callable
  signature, immutable environment, mutable capture cells, and async frame.
  Explicit `(parameters) -> Result => body` signatures
  constrain closure bodies, and omitted parameter and result types are shown as
  one complete virtual signature through inlay hints. Latent effects now stay
  symbolic through callable parameters, returned closures, captures, and lazy
  `map` / `filter` adapter fields; invoking or iterating a concrete value
  instantiates those effects per call site, so an unrelated effectful callback
  cannot poison a pure specialization. Function hover exposes which parameters
  are invoked or iterated. Ordinary lexical control flow is enforced: `return`
  belongs to the closure, while `break` and `continue` cannot target loops
  outside it. Named functions and closures now share the same callable type,
  typed GC representation, invocation path, and latent-effect analysis. A
  captureless adapter bridges a named function's ordinary ABI to the callable
  environment ABI without synthesizing source closures.
- [ ] Complete remaining ordinary library gaps when a port needs them:
  immutable String operations beyond the corpus-proven P0 slice, additional
  numeric operations, and typed time operations proven useful by maintained
  ports.
- [ ] Add general floating-point power only when a maintained port needs a
  negative or non-integral exponent. Port and attribute a vetted implementation
  such as Rust compiler-builtins' MIT-licensed libm `pow`/`powf`, including its
  scaling helpers, rather than introducing an ad-hoc approximation or a host
  import. Keep `squared()` as the simple exact-intent API for exponent two.
- [ ] Add structural anonymous structs only after named structs prove materially
  noisy. Decide explicitly whether anonymous structs are memory-readable.

## P2 — documentation and editor evolution

- [x] Render generic type parameters consistently in generated documentation
  indexes and navigation. All spellings already derive from canonical type-
  constructor metadata; the inconsistency came from serializing a unary form
  such as `Set<T>` as raw Markdown, where `<T>` was consumed as an HTML tag
  while `Map<K, V>` happened to survive. Compiler-generated symbol labels now
  share one Markdown-boundary escaping function, while catalog identities stay
  raw. Catalog-wide terminal and HTML tests round-trip every current and future
  type constructor through root indexes, declaration headings, breadcrumbs,
  and search results, preventing the same class of drift from recurring.
- [x] Make exact documentation queries for concrete generic members resolve to
  their canonical operation instead of only returning a ranked type list. For
  example, `splitc docs MapEntry.key` shows the specialized field. Concise
  generic owner/member aliases are derived from catalog identities and resolve
  directly only when they identify one canonical public page; private iterator
  cursor implementations deliberately have no documentation topics.
- [ ] Improve navigation for very long terminal guide results without changing
  their content model. Put stable exact subsection identities near the top of
  monolithic guides and make focused `splitc docs` queries obvious before a
  terminal or calling tool truncates later sections. Treat this as mitigated and
  lower priority because exact-topic searches already work; do not split the
  canonical guide into duplicated prose.
- [x] Add document highlights for every source-owned occurrence of the symbol
  under the cursor, with read/write classification, and type-definition
  navigation for inferred source and catalog types.
- [ ] Extend document highlights to compiler-owned standard-library and
  language symbols without resolving every token independently. Add folding
  ranges for declarations, blocks, multiline expressions, comments, and
  settings trees once their recovered-syntax boundaries are stable.
- [ ] Add call hierarchy after the compiler exposes one reusable call graph,
  multi-range formatting when it materially improves editor workflows, and
  debugger inline values together with the eventual debugging strategy. Do
  not prioritize implementation hierarchy, linked editing, document colors,
  or inline completion without a concrete SplitScript use case.
- [ ] Add completion snippets for lifecycle blocks, struct construction, and
  common standard-library calls. Module scope plus state, settings, and
  tick-rate declarations are grammar-aware already; contextual match-pattern
  completion is tracked at P1 with the rest of pattern matching. Keep
  candidates compiler-owned and the VS Code client thin.
- [ ] Continue adding focused labels, notes, and machine-applicable fixes for
  real confusing cases rather than growing a speculative diagnostic catalog.
- [ ] Introduce file identities, modules, and imports only together with a real
  multi-source use case. Most autosplitters should remain pleasant as one file.

## P2 — architecture, verification, and release scaling

- [ ] Add one catalog-owned constant-precondition analysis rather than a family
  of one-off lints. When compile-time evaluation proves that a literal call or
  operation must fail, trap, or never complete, diagnose it using the same
  constraint metadata that documents the API; do not repeat the porting
  report's roughly forty separate variants. Cover empty or undersized scan
  patterns, decoder and collection bounds, invalid rates and radices, empty
  delimiters, literal parsing, division by zero, provider address domains,
  empty reduction identities, and literal file paths as those contracts are
  represented. Decide warning versus error based on whether the failure can be
  intentionally handled through `T!`, provide focused fixes, and ignore values
  that are not statically known. Keep cross-platform executable spelling and
  host path normalization out of this value analysis.
- [ ] Decompose broad backend and type-checking coordinators by semantic
  ownership under existing behavior tests. Extract structural expression,
  call dispatch, numeric, string/collection, process/memory, and suspension
  emitters with narrow inputs; separate call candidate collection, generic
  solving/selection, and diagnostic construction. Move one family at a time
  and do not replace one all-purpose context with nested bags of everything.
- [x] Compile each unique runtime `(source, output, profile)` artifact once in
  `cargo xtask check`, validate it once, and run all argument/scenario variants
  against that artifact. Compilation is modeled separately from runtime
  scenarios; fixture planning rejects conflicting output definitions and exact
  duplicate scenarios without reducing maintained host coverage. The current
  matrix compiles and validates 65 artifacts for 93 runtime scenarios instead
  of rebuilding 28 duplicate artifacts.
- [ ] Add a small shared `instantiateRuntimeFixture` host for common Wasm
  loading, text/memory helpers, default imports, `_start`, and `update`, while
  keeping unusual host behavior and assertions local. Migrate harnesses only
  when touched so focused tests do not acquire an oversized universal host.
- [ ] Consolidate desktop/browser compiler-worker and language-client lifecycle
  policy behind host-neutral controllers parameterized by worker creation,
  transport, message, subscription, and error adapters. Keep Node and browser
  entry points thin and run the same start, restart, cancellation, failure, and
  stop contract tests through both adapters.
- [ ] Extend the VS Code compiler-task harness beyond saved-document identity
  to exercise release/watch stale-result suppression, atomic output
  replacement, and temporary-file cleanup through host-neutral controller
  tests. The production paths already guard these cases; this is regression
  hardening rather than a known editor correctness defect.
- [ ] Replace the opaque aggregate documentation fingerprint with reviewable
  checked-in metadata or per-page fingerprints that report changed URIs and
  fields. Provide an explicit regeneration command; retain full graph, link,
  anchor, and example validation as the semantic guard.
- [ ] Tighten release reproducibility and licensing: pin Rust through
  `rust-toolchain.toml`, pin third-party Actions to reviewed commit SHAs, add
  the declared MIT and Apache-2.0 license texts to source and packages, and
  inherit shared Rust version/edition/license fields from `[workspace.package]`.
  Keep extension versioning independent only if release policy says so.

## P2 / deliberately deferred

- [ ] Replace string-only errors with a structured identity plus human-readable
  message after concrete error inspection needs justify the language surface.
  Code must eventually distinguish compiler/runtime error kinds such as a
  future timeout from an operand's own failure without comparing display text,
  while `T!`, `?`, `retry`, `throw`, and existing fallback syntax retain one
  ergonomic propagation channel. Decide user construction, matching, querying,
  and custom errors together rather than reserving ad hoc strings or special
  cases for `future.timeout`. This is separate from the implemented backend
  optimization that omits unobserved payload construction.

- [ ] Reconsider diagnostic debouncing or cancellation only if interactive
  measurements or a reproduced editor stall identify diagnostics as the cause.
  Current synchronous publication is simple and no observed problem justifies
  adding clocks, queues, version coordination, and separate native/WebAssembly
  scheduling paths. Keep full-sync text transport until measurement likewise
  demonstrates that incremental synchronization is worth its complexity.

- [ ] After ASR has a tested SNES provider, align SplitScript with its higan,
  bsnes, Snes9x, BizHawk, RetroArch, and lsnes-bsnes discovery and memory
  semantics rather than independently inventing them from the Super Metroid
  port. The eventual source design should normally match the existing
  `state GCN` / `state Genesis` provider model, but must still be brought to the
  user before implementation. A manual RetroArch memory root remains an interim
  port, not the canonical endpoint.
- [ ] After ASR has a tested Sega CD / Mega-CD provider, add one typed provider
  aligned with its emulator discovery, RAM mapping, address-domain, and byte-
  order semantics. Do not infer those contracts independently from the SEGA
  Master Splitter or expose a raw process-memory workaround as the canonical
  API; bring the provider surface back for approval once the runtime exists.
- [ ] Revisit native-state suggestions for typed emulator providers only if
  future porting evidence shows this remains a recurring source of incorrect
  scripts after the current provider documentation and search work. This is
  speculative guidance for otherwise valid source, not a present usability
  blocker; any eventual design must account for ambiguous emulator process
  names and cross-platform executable identity without producing a noisy
  warning.
- [ ] Design a contextual `default` literal backed by a source-defined
  `Default` capability. Like `None`, it may be assigned directly where the
  expected type or later constraints determine a unique target, but it must not
  silently become the fallback for failed inference. Define capability
  membership for primitives and standard-library types; make structs defaultable
  only when every field is defaultable; and require an explicit decision for
  enums and collections rather than assuming a first variant or allocating
  implicitly. Keep `default` distinct from `None`: `None` is the unit value and
  absent option case, while `default` constructs the target type's declared
  default value. Add focused ambiguity/unsupported-type diagnostics, hover,
  completion, formatting, and source-defined capability documentation when the
  feature is implemented.
- [ ] After the immediate exact-name documentation work, settle cross-platform
  process identity with the host runtime before warning about extensionless
  native `state` names. ASL commonly omits Windows' `.exe`, but extensionless
  names are valid on Linux and macOS, so a compiler warning would create false
  positives without target knowledge. Decide whether the language should state
  a target/platform policy, the runtime should normalize executable identity,
  or declarations should provide target-specific candidates. Only then add a
  warning or migration rewrite, with attachment fixtures on all three hosts.
- [ ] Specialize physical aggregate layouts around zero-sized `None` only when
  measured size or allocation pressure justifies it. Structs may omit unit
  fields; `None?` still needs distinct empty/present states; `None!` retains
  tag/error; `[None]` retains its logical length. Keep
  all specialization behind one physical-layout abstraction so construction,
  matching, equality, field indices, DWARF, and codegen cannot disagree.
- [ ] Design explicit `throw`/`catch` boundaries later. Postfix `?` remains the
  ergonomic propagation operation and uncaught errors return through `T!`.
- [ ] Coalesce non-overlapping async-frame slots only if real autosplitters make
  frame size material and cleanup remains cancellation-safe.
- [ ] Extend `debug` to more declaration kinds only when a concrete use case
  defines checking, reachability, and release erasure.
- [ ] Write an explicit sandbox policy before adding process writes/injection,
  file mutation or access outside the runtime's approved WASI preopens,
  network/process launching, modal UI, audio, or broad host control. The
  handle-free read-only `File` API remains inside those preopens; its proposed
  writable/persistent counterpart must settle overwrite, deletion, and consent
  here before implementation. Use stats-file game time, install-file discovery,
  injected load removers, and timing-method prompts from the ASL corpus as
  concrete policy cases. Prefer file settings and typed host metadata where
  they suffice. Dangerous capabilities require visible consent and cleanup
  semantics; some may remain intentional non-goals.

## Ongoing — evidence, correctness, and maintainability

- [ ] Treat hand-reviewed ports as authoritative migration evidence. Generated
  candidates estimate frequency but do not prove semantic parity.
- [ ] Treat compiler-clean generated ports as hypotheses rather than successful
  ports. Audit attachment identity, selected builds, lifecycle/timer behavior,
  settings reachability, integer signedness/width, failure behavior, and omitted
  source branches even when no diagnostic fired. Record both discoverability
  failures (existing facilities that porters could not find) and silent false
  successes (compiling scripts that cannot attach or disable behavior).
- [ ] Keep a structured record per port: source, target build, preserved
  behavior, omissions, blockers, workaround quality, compiler revision, and
  runtime status. Distinguish complete, variant-limited, and behavior-limited
  ports.
- [ ] Re-audit old notes after major compiler milestones. Missing or
  hard-to-find documentation, weak diagnostics, and unrecorded compiler
  provenance must not become duplicate feature work.
- [ ] Preserve diagnostics that porters explicitly found useful. Keep focused
  regression tests for optional/value mismatches, unhandled `T!` values, and
  machine-applicable unused-variable fixes while improving adjacent guidance;
  a new help path must not replace a clearer primary error or reintroduce
  cascades.
- [ ] Maintain a representative warning-free corpus spanning native games,
  Unity Mono/IL2CPP, emulators, pointer paths, signatures, settings trees,
  loading, game time, cancellation, and unusual numeric layouts.
- [ ] Use maintained ports as formatter fixtures, LSP integration projects,
  documentation examples, runtime tests, and performance inputs.
- [ ] Keep `cargo xtask check` as the single local and CI verification matrix.
  Extend it whenever a product surface is added; generated Wasm/Wat belongs
  under ignored `target` directories.
- [ ] Replace the stale hard-coded list of modules above roughly 1,000 lines in
  `COMPILER.md` with a generated ownership signal or a policy that records the
  named boundaries already reviewed. Review oversized coordinators when related
  work changes them; split only at a product, context, visitor, or semantic
  domain boundary, since line count alone is not a reason to scatter shared
  mutable state.
- [ ] Add a generated large-catalog performance dimension when alternate
  catalog construction exists, covering validation, indexing, completion,
  hover, and documentation queries.

## Recommended execution order

1. Coordinate the read-only timer metadata/time surface, imperative timer
   control, writable files, read-only file metadata, JSON, and safe module
   enumeration with their host-runtime contracts. Abe's Oddysee, Outer Wilds,
   Ato, Spider-Man, and the SEGA Master Splitter are the acceptance evidence;
   deterministic executable identity already exists through `Module.md5()`.
2. After completing the current Unity goal, schedule the P1 Release-only Wasm
   size project separately, following its measurement, constant-folding,
   size-driven inlining, and reachability sequence. Resume measured
   compiler/editor performance, release hardening, hosted IDE, and debugging
   work after the correctness and product-design sequence above.
3. Keep only the portions of shared readable-memory helpers that need a new
   host primitive, the PS2 low-memory domain, `unity.time`, Sega CD, and SNES
   gated on tested ASR evidence. Keep writes/injection,
   physical `None` specialization, and other broad host powers deferred until
   their explicit dependencies and policies are ready.
