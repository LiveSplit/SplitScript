# SplitScript roadmap

## 2026-09-26: compact GC null encoding

- Emit GC null constants with the one-byte `none` heap type instead of a
  concrete struct/array type index, including global initializers and frame
  resets. Source callable values are GC wrapper structs; this does not change
  the separate Wasm function-reference or extern-reference hierarchies.
- This direct codegen improvement applies to Debug and Release, removes
  unnecessary type lookups and introduces no optimization pass.
- On top of the earlier non-returning-expression fix, Release files shrink
  by 41 bytes for Lunistice, 631 for automatic Unity live identity, 668 for
  managed maps and 671 for nested managed maps. The other four size-corpus
  fixtures are unchanged. Regression coverage checks compact null encoding
  and validates large managed schemas and async failures in both profiles.

## 2026-09-11: every runtime value has lazy Debug formatting

- Made `Debug` a total capability for concrete runtime values and preserved
  `Display` as its user-facing fallback. Source structs, enums, managed class
  snapshots, and opted-in containers keep recursive structural output, while
  live managed references, closures, futures, and representation-only
  standard-library types receive stable opaque output. Conditional managed
  snapshot fields are shown only while their attachment shape is active.
- Centralized the universal fallback and structural-versus-opaque decision in
  capability analysis. Removed repeated iterator `Debug` declarations, kept
  private cursor and captured closure state hidden, and retained custom
  standard-library formatting such as `FileVersion`.
- Kept both formatter kinds demand-driven through the existing reachability
  plan. A formatter and its structural helper graph are emitted only when a
  concrete value is displayed; a standalone opaque formatter does not pull in
  multiline structural-formatting helpers.
- Added specialization, nested-container, valid-Wasm, and debug-name
  reachability coverage for closures while preserving the existing exact
  runtime-output coverage for all standard iterator kinds.

## 2026-09-11: readable address spaces share one capability

- Added source-defined `MemoryReader<T>` with an associated `Address` type and
  one generic `read<U: MemoryReadable>` contract. Native `Process` readers use
  `address`; GBA, PS1, PS2, Master System, Genesis, GameCube, and Wii providers
  use their original `u32` guest-address domains.
- Moved bounded UTF-8 and UTF-16LE methods onto the shared capability while
  keeping module discovery, mapped ranges, and scanning process-only. Emulator
  reads reuse their existing address translation and byte-order backends, then
  feed common strict UTF-8 and replacement-decoding UTF-16 helpers.
- Extended the generated standard-library catalog so nominal types can define
  associated types, and carried generic capability method type arguments
  cleanly through inference, specialization, scratch planning, dependency
  pruning, and Wasm lowering. Only the concrete reader backend and requested
  decoder are retained.
- Added editor, generic-call, native-runtime, emulator-provider, catalog, and
  valid-Wasm coverage without introducing a new host-runtime primitive.

## 2026-09-02: one script can select among typed state providers

- Added named `provider Name: Provider { ... }` alternatives inside `state`,
  with one generated read-only `provider: StateProvider` discriminant. Common
  fields remain available through the ordinary snapshots, while a direct match
  refines provider-only fields and roots through the existing layout-aware type
  and scoped-global architecture.
- Attachment now deduplicates the alternatives' process-name union, polls every
  applicable provider discovery cooperatively, and selects the first completed
  alternative in source order. Native identity selection and suspending
  emulator discovery share the same generated lifecycle without attaching to a
  candidate process more than once.
- Generalized provider resolution, direct reads, validation, reachability,
  runtime storage, process-name metadata, formatting, navigation, hover,
  completion, semantic highlighting, and documentation around provider sets.
  Added valid-Wasm and mocked-runtime coverage while retaining the concise
  single-provider form unchanged.

## 2026-09-01: bounded signature scans can report exhaustion

- Added `process.scanOnce(address, size, signature) -> async address?` as the
  single finite scanning primitive: it returns the first match or `None` after
  one complete cooperative pass instead of silently restarting.
- Reused the existing bounded range-scanning state machine and process-close
  cancellation. Waiting process, module, fallback, and process-wide scans keep
  their repeat-until-found behavior, without acquiring parallel `Once` APIs.
- Documented why range exhaustion differs from elapsed-time cancellation and
  covered matches beyond the first cooperative window, completed absence, and
  the prohibition on rescanning after completion in a real Wasm runtime test.

## 2026-09-01: impossible literal provider reads stop during checking

- Added provider-owned readable guest-memory ranges to the generated standard
  library catalog, with validation for malformed and overlapping declarations.
- Literal PS2 state paths and direct reads now produce a focused source
  diagnostic when the complete value cannot fit in the supported guest-memory
  domain; computed addresses retain their existing runtime failure behavior.
- Runtime address translation and generated provider/reference documentation
  consume the same catalog metadata, preventing their boundaries from drifting.

## 2026-09-01: ASL primitive state widths are explicit

- Added a compiler-owned ASL primitive-state migration concept and a
  self-contained porting table for every scalar type, including LiveSplit's
  deliberate one-byte `bool` read.
- Distinguished exact scalar mappings from `stringN`, `byteN`, pointers,
  enums, manual reads, and values created in C# action code, where the source
  or runtime representation must be inspected rather than guessed from a
  familiar name.
- Added catalogued, case-insensitive documentation query aliases such as
  `ASL bool`, `ASL int`, and `ASL bool byte int`. Query aliases remain separate
  from source spellings, so search vocabulary cannot accidentally become a
  parser diagnostic or rewrite candidate.

## 2026-09-01: fixed-array memory bounds are visible before design

- Documented the 4,096-element and 65,536-byte process-read bounds on both the
  exact `[T; N]` language reference and static `at` memory paths.
- Expanded the ASL contiguous-memory recipe with a compiler-checked
  expression-valued state field that transactionally constructs a growable
  array from sparse focused reads. This gives authors a concrete alternative to
  an oversized fixed array when the unused bytes need not be copied.
- Added catalog regressions that keep the limits and alternative visible on the
  canonical pages.

## 2026-09-01: language examples validate their complete source

- Fixed the rendered `choice` setting example by adding the required commas to
  both enum-backed options.
- Split catalog example validation into explicitly complete programs and
  intentionally contextualized fragments. The language-catalog regression now
  parses and formats every complete validation fixture before type-checking the
  formatted result, while complete visible examples must validate their own
  source rather than a substitute program.

## 2026-09-01: ASL numeric roots retain their module base

- Added a self-contained ASL porting recipe showing that a bare numeric native
  state or `DeepPointer` root is normally relative to the selected executable's
  main module, while SplitScript's integer-rooted `at` form is an absolute
  virtual address. The side-by-side source makes the required module string
  explicit and explains why both compile despite reading different locations.
- Expanded the compiler-owned `DeepPointer` migration concept and generated
  capability index with the same semantic warning and a direct recipe link.
  Exact `DeepPointer` and native-state numeric-root searches are regression
  tested against that canonical migration page.

## 2026-09-01: optional state expressions contextualize absence

- Expression-valued optional state fields now resolve a bare `None`, including
  a transparent value block ending in `None`, against the declared field type
  before lowering the field's implicit transactional result boundary.
- Generalized contextual absence lowering through successful result layers, so
  these fields produce `Ok(None)` without a state- or `String`-specific backend
  path. Non-optional fields still diagnose the declared value mismatch and
  point back to the field annotation.
- Added debug/release Wasm validation for `String?` and `u32?`, retained the
  representative propagated-read regression, and compiled the original
  porting probe without its false `Some("")` substitution.

## 2026-09-01: managed schema layout failures stop before code generation

- Added one post-inference remote-memory validation boundary shared by native
  state paths and terminal managed fields. Managed references and bounded
  managed strings retain their dedicated decoders; every other managed value
  must satisfy the ordinary `MemoryReadable` contract before lowering.
- Replaced the `[String]` managed-field code-generation assertion with a source
  diagnostic anchored on the field type. It names the managed field, explains
  the missing fixed layout, and distinguishes a growable SplitScript array from
  a managed array/list representation without prematurely designing the
  ASR-dependent collection API.
- Added debug and release regressions for the original Here Comes Niko-shaped
  source, plus the complete state-layout suite and all-target validation.

## 2026-08-31: completion reuses one revision context

- Completion now borrows one recovered source document, syntax tree, cursor,
  replacement span, and indexed lossless token stream. Grammar-specific paths
  no longer re-lex the complete source or deep-clone the recovered program.
- Receiver inference consults the current revision's semantic snapshot before
  constructing a repaired probe and returns compact type/constraint facts
  rather than another cloned program. Token-boundary member lookup now remains
  correct across comments, whitespace, and end of file.
- On the fixed 30-sample runner this cuts warm-sequence p95 from 166 to 82 ms
  (small), 259 to 177 ms (Lunistice), and 385 to 236 ms (generated large),
  without increasing the sequence's retained cache.

## 2026-08-31: interactive query latency and heap baselines

- Expanded the release tooling runner from one generated hover probe to small,
  maintained Lunistice, and generated-large fixtures covering diagnostics,
  root and member completion, hover, semantic tokens, repeated full-sync edits,
  a warm multi-query sequence, and in-process language-service recovery.
- Added allocator-backed retained and peak heap measurements without another
  benchmark dependency. The runner separates repeated-query growth from the
  complete live cache retained by each query shape.
- Recorded p50/p95 and heap baselines with explicit interaction targets. Member
  completion is the first measured bottleneck: it reaches 166 ms on the focused
  source, 262 ms on Lunistice, and 413 ms on the 500-function fixture, while
  diagnostics remain within the initial target.

## 2026-08-30: deduplicated runtime verification artifacts

- Separated `cargo xtask check` artifact definitions from runtime scenarios.
  The verifier now compiles and validates each unique output once, then runs
  every scenario against that artifact; the current 93-scenario matrix needs
  65 builds instead of 93.
- Added deterministic fixture-plan validation. Reusing an output for a
  different source or profile and repeating an exact runtime scenario now fail
  before compilation begins.
- Moved the proposed SNES provider to the ASR-dependent deferred roadmap after
  confirming the current ASR tree has no SNES implementation to align with.

## 2026-08-30: struct literals gain identity-safe field shorthand

- Added struct field initializer shorthand: `Point { x }` lowers through the
  same value-path semantics as `Point { x: x }`. Repeated explicit
  initializers produce configurable warning `SS1010` with a machine-applicable
  fix.
- Formatter, highlighting, definitions, references, and identity-checked
  rename understand the shorthand. Renaming only the field or only the local
  expands it to preserve both meanings.

## 2026-08-30: profile-aware unused guidance shares declaration reachability

- Added configurable warning `SS1009` for normal locals, globals, and
  functions consumed exclusively by debug-only code. Ordinary unused warnings
  remain profile-stable, while authors now see work unnecessarily retained in
  release builds.
- Labelled the existing declaration graph with debug/release reachability and
  propagated those profiles through transitive helpers, capability bodies, and
  named function values. This avoids a separate debug-only name scan and also
  corrects function-value reachability for ordinary unused analysis.
- Added machine-applicable `debug` modifier insertions where erasing the whole
  declaration is safe. Release-visible assignments keep the warning but
  suppress an unsafe edit. Compiler, release-Wasm, warning-policy, and LSP
  quick-fix coverage exercise the shared behavior.

## 2026-08-30: unused state values become visible without changing execution

- Added `SS1004` warnings for state fields whose produced snapshot values never
  reach `current`, `old`, or another observed state field. Candidate-field
  dependencies close transitively, while shared named-layout declarations
  produce one logical diagnostic with every physical declaration labelled.
- Kept polling execution separate from value observation. An unused field's
  process reads, effects, and called helpers still execute and remain reachable;
  the warning never silently changes runtime behavior.
- Added the ordinary `_` suppression convention with a validated editor rename
  that updates shared layout declarations together. Displaying a value through
  `print` or `setVariable` is an ordinary read and does not warn.
- Corrected the documentation boundary: state snapshots are internal runtime
  storage, not an implicitly host-visible interface. The settings UI remains
  host-visible through its explicit ABI.

## 2026-08-30: self-contained first build journey

- Added a compiler-owned Getting Started guide that takes a new author through
  installing a supplied VSIX or invoking the native CLI, declaring exact native
  attachment, one typed setting, one pointer-backed state field, transactional
  `old` / `current` comparison, debug-watch and release outputs, diagnostics,
  documentation discovery, and the current host-loading boundary.
- Kept the guide independent of the repository's full autosplitter files. Its
  focused visible snippets carry hidden compiler-checking context, and the
  complete documentation graph validates their intra-doc links and routes.
- Corrected the remaining first-use roadmap so disposable examples are neither
  bundled nor promoted as required author documentation.

## 2026-08-30: extension package documentation for authors

- Rewrote the VS Code package README around outcomes and the first successful
  author workflow, with exact commands, neighboring Wasm output, bundled
  tooling, requirements, host boundaries, and recovery steps.
- Moved npm setup, VSIX construction, worker architecture, extension-host
  launches, and browser-host testing into a contributor-only development guide.

## 2026-08-30: repository entry point by audience

- Replaced the implementation-inventory README with an honest entry point for
  VS Code authors, native CLI users, ASL porters, and compiler contributors.
- Led with current product and host status, a minimal native state/split
  fragment, neighboring Wasm output, and the early distribution boundary.
- Routed exact language, standard-library, ABI, architecture, conformance, and
  measurement facts to their durable documents instead of duplicating stale
  feature and backend inventories or promoting full ports as tutorials.
- Kept the durable porting-campaign audit discoverable for contributors while
  leaving full scripts outside the user authoring path.

## 2026-08-30: stale public capability claims removed

- Corrected hand-written language and architecture prose that still described
  generalized user functions, value-producing block branches, LSP formatting,
  browsable and terminal documentation, wrapper editor types, closures,
  iterators, arrays, strings, structs, and other GC values as future work.
- Kept genuinely pending host events, machine-readable/static exporters, race
  combinators, resource ownership, and unsupported-build behavior explicitly
  pending rather than flattening all forward-looking prose indiscriminately.

## 2026-08-30: one canonical standard-library reference

- Turned `STANDARD_LIBRARY.md` into a task-oriented chooser for native,
  emulator, Unity, value, state, text, and timer workflows. Exact signatures,
  bounds, effects, availability, and examples now point to compiler-generated
  symbol pages rather than a hand-maintained member inventory.
- Replaced the duplicated string-operation table with unit- and task-based
  guidance linked to exact generated `String` members.
- Moved the complete catalog, semantic-ID, inference, HIR, backend dispatch,
  diagnostic, and documentation architecture section intact into
  `COMPILER.md`, keeping contributor detail without presenting it as public API.
- Resolved every newly referenced catalog identity through the CLI documentation
  query to catch misspelled or nonexistent links.

## 2026-08-30: user language guide separated from compiler internals

- Added task-based navigation from the self-contained Getting Started workflow
  into state, ordinary logic, value modeling, timer control, failure/async, and
  native text/memory concepts without depending on full example files.
- Removed HIR, Wasm representation, GC-layout, backend-lowering,
  continuation-frame, and linear-memory exposition from user semantics while
  preserving observable evaluation, lifetime, suspension, and module-size
  behavior.
- Moved debug name/DWARF/source-identity details and the exact decimal parsing
  implementation into `COMPILER.md`; existing async and ABI architecture pages
  already own the remaining physical details.

## 2026-08-29: managed strings become schema values

- Added ordinary `String` and `String?` fields to managed class declarations,
  with shared readers for static fields, live references, and transactional
  snapshots.
- Made nullability and failure explicit: required null strings fail, optional
  null strings become `None`, memory failures remain errors, and invalid UTF-16 uses replacement
  decoding.
- Removed the raw public `Process.readManagedString` route, moved Lunistice's
  DLC scene into its `GameManager` snapshot, and updated completion, hover,
  semantic highlighting, formatting, diagnostics, and the
  reference journey around the schema-first spelling.
- Recorded struct-initializer shorthand and adjacent shared-prefix state reads
  as separate follow-up work so neither is smuggled into this storage-policy
  milestone without its own rename and memory-semantics design.

## 2026-08-29: complete schema-first Unity documentation

- Made the `Unity` provider page the canonical managed-memory entry point. It
  now explains automatic and explicit backend selection, schema binding,
  attachment-scoped scene access, live references, transactional snapshots,
  allocation behavior, and demand-driven generated support together.
- Expanded the `image`, `class`, `static`, and `from` language pages with the
  runtime semantics that matter when designing a schema: reachable binding,
  unambiguous aliases, live singleton replacement, fallible remote hops,
  layout refinement, and the absence of partially populated snapshots.
- Tightened the standard-library and ASL-porting guides around one public
  schema workflow. Mono and IL2CPP metadata traversal stays private, while the
  remaining bounded managed-string limitation is described as a schema-value
  gap rather than a reason to reproduce backend internals.
- Added reference regressions that require the Unity journey to retain its
  identity, failure, layout, cost, and private-boundary explanations.

## 2026-08-29: stable managed-snapshot projection reuse

- Added a conservative Wasm-IR local plan for managed snapshots projected from
  `current` and `old` repeatedly in one synchronous body. Generated code reuses
  both profitable snapshot roots and repeated direct fields without treating
  live process-memory operations as pure or carrying cached values across a
  suspension.
- Made source state assignment part of the compiler's shared mutation census.
  Any assignable `current` snapshot root is excluded from reuse, while `old`
  remains immutable. Focused lowering tests cover both the profitable reuse and
  mutation exclusion paths.
- Reduced the current release Lunistice artifact from 34,428 to 34,413 bytes.
  The small but measured saving closes this specific duplicated-GC-projection
  case without introducing a general optimizer or weakening read ordering.

## 2026-08-29: closed pure module-global initialization

- Generalized initialized top-level bindings from literal aggregates and one
  special set constructor to any closed, synchronous, pure expression. They may
  allocate, use value blocks, and call source-defined or standard-library
  helpers, and execute exactly once before `setup`.
- Extended operational summaries with transitive global reads and writes.
  Initializers now reject global dependencies and mutation, settings, timer,
  process and contextual state, and suspension with diagnostics that identify
  the responsible helper and referenced declaration.
- Lowered runtime initializers into ordinary Wasm IR blocks with shared local,
  specialization, reachability, and expression emission machinery. Literal
  scalars continue to use Wasm global constant expressions, avoiding runtime
  work where the binary format already represents the value directly.

## 2026-08-28: schema-only public Unity metadata

- Made top-level managed `image`, `namespace`, `class`, `static`, and `from`
  declarations the sole public route for Mono and IL2CPP metadata. The runtime,
  image, class, field, static-table, and offset objects remain available to the
  trusted provider implementation but no longer appear in user lookup,
  completion, hover, navigation, or generated reference pages.
- Ported the maintained ARTIFICIAL, Himno, and attachment smoke examples to
  generated schema bindings. Their deterministic runtime fixtures continue to
  cover static scalar fields, replaceable singleton references, and bounded
  cooperative IL2CPP discovery.
- Rebuilt the ASL migration journey around the schema model and added focused
  diagnostics and documentation search coverage for `UnityASL`,
  `mono.Make<T>`, `mono.MakeString`, `Unity.mono`, hidden traversal types, and
  managed-field concepts without offering an unsafe mechanical rewrite.

## 2026-08-26: demand-driven generated code

- Kept source-defined async functions fully lazy: construction only captures
  their inputs, while the body advances exclusively when its state machine is
  polled. First and subsequent polls now share one canonical dispatcher body
  instead of duplicating every suspension and retry attempt.
- Stopped structural formatting dependency discovery at source-defined
  `Display` and `Debug` implementations, preventing unused derived float
  formatters and decimal tables from leaking into an artifact.
- Added privileged managed-backend metadata to configured state-provider
  selectors. An explicit `Unity.il2cpp(...)` or `Unity.mono(...)` build now
  emits only its reachable schema binder; automatic Unity detection continues
  to retain both backends.
- Reduced the release Lunistice artifact from 90,413 bytes to 30,638 bytes,
  while preserving both base-game and DLC runtime behavior. The result is
  smaller than the 34,437-byte Rust ASR comparison build.

## 2026-08-23: Structural user implementations for catalog capabilities

- Added catalog-authored structural method requirements and used them for
  `Display.toString`.
- User structs and enums now satisfy `Display` by defining the exact ordinary
  source method, without an `impl` block or annotation.
- Routed casts, interpolation, `print`, and custom-variable values through the
  same source method in validation, reachability, effect analysis, and Wasm
  code generation.

## 2026-08-23: source-defined intrinsic signatures and context

- Made asynchronous completion part of privileged standard-library signatures
  through `-> async T`. `await` therefore depends on the expression's type just
  as `retry` depends on `T!`; neither is declared as an operation category.
- Added narrowly scoped intrinsic context attributes for attached-process and
  state-snapshot requirements, process-close cancellation, and `onAttach`
  availability. The closed Rust intrinsic registry remains an independent
  trust boundary for lowering, helpers, host imports, and exact validation.
- Kept high-level standard-library composition as ordinary source code. Its
  transitive effects and suspension behavior are inferred from checked bodies,
  while its public async result types remain explicit and visible to tooling.

## 2026-08-23: private source-defined standard-library helpers

- Added `private fn` declarations to trusted standard-library source. Private
  callables retain stable compiler identities and participate in ordinary
  parsing, inference, effect analysis, specialization, and code generation,
  while public name/member lookup, completion, hover, and generated
  documentation omit them.
- Consolidated active and loaded Unity scene decoding behind one private
  `UnitySceneManager.snapshot` method without exposing an implementation helper
  to scripts.
- Centralized repeated PE optional-header validation for module version,
  export, and pointer-width queries, repeated GBA 64-bit memory-pointer
  decoding, and the shared Dolphin/libretro provider preflight behind private
  source helpers.
- Hid emulator attachment discovery behind the corresponding `state` providers
  and Mono discovery behind `Unity.mono()`. Raw-address class, field, and static
  table traversal now remains an implementation detail of the typed
  `MonoImage` and `MonoClass` APIs, while pointer-width-aware reads stay public
  for advanced metadata traversal.
- Added private nominal types to trusted standard-library source. Their stable
  identities, fields, layouts, and source bodies remain available to checking,
  specialization, and code generation, while user type lookup, completion,
  navigation, and generated documentation see only public types.
- Made `MonoLayout` and `MonoLayout.forVersion` implementation details, and
  added loader validation that rejects a private type escaping through a public
  field, callable signature, or state-provider process type.

## 2026-08-23: source-defined Unity scene snapshots

- Added cooperative `Unity.sceneManager()` discovery for ASR's Windows,
  Linux, and macOS UnityPlayer scene-manager layouts without a new compiler
  intrinsic.
- Added immutable active and loaded `UnityScene` snapshots carrying native
  address, signed build index, asset path, and derived scene name, with
  transactional failure instead of partial state updates.
- Made growable array backing storage support non-null standard-library GC
  elements while retaining non-null source reads.
- Documented the direct `LoadSceneManager`, `Scenes.Active`, and
  `Scenes.Loaded` migration path.

## 2026-08-23: typed guidance for literal setting lookups

- Added warning `SS1007` for literal dynamic lookups whose meaning is already
  known from the settings declaration.
- Made `settings.enabled("key")` and `oldSettings.enabled("key")` offer
  machine-applicable typed-member rewrites when a source-visible boolean member
  exists, while preserving computed and generated-family lookup patterns.
- Diagnosed `contains("key")` for a known value declaration as always true,
  with a semantics-preserving replacement and separate intent guidance for
  reading the typed setting value.

## 2026-08-22: normalized provider reads and Sega Genesis support

- Replaced the native-address-only emulator boundary with a normalized
  guest-byte read contract shared by explicit methods and state pointer paths.
  Existing providers use a generic translation adapter while retaining their
  original address and failure semantics.
- Added `state Genesis` and `genesis: GenesisEmulator` with source-defined
  discovery for Fusion, Gens, BlastEm, Sega Classics, and the four Genesis
  libretro cores supported by ASR.
- Normalized word-swapped emulator storage, including unaligned reads crossing
  16-bit boundaries, before recursively decoding big-endian primitives,
  structs, fixed arrays, floating-point values, and guest pointer paths.

## 2026-08-22: source-defined Wii emulator provider

- Added `state Wii` and `wii: WiiEmulator` with source-defined Dolphin and
  `dolphin_libretro.dll` discovery.
- Added exact MEM1 and MEM2 guest bounds and native mapping translation.
- Reused provider-selected big-endian decoding for explicit reads, structs,
  fixed arrays, and guest pointer paths.

## 2026-08-22: provider-owned byte order and GameCube support

- Separated fixed `MemoryReadable` shape from the byte order supplied by an
  emulator provider.
- Added recursive big-endian decoding for primitives, structs, fixed arrays,
  floating-point values, signed integers, and provider pointer paths.
- Added `state GCN` and `gcn: GCNEmulator` with source-defined Dolphin and
  `dolphin_libretro.dll` discovery and MEM1 address translation.

## 2026-08-22: source-defined Master System emulator provider

- Added `state SMS` and `sms: SMSEmulator` with source-defined discovery for
  Fusion, BlastEm, Mednafen, and five supported RetroArch cores from ASR.
- Refreshed Fusion's moving pointer at the read boundary, retained the
  core-specific libretro discovery paths, and validated core lifetime before
  using stable mappings.
- Routed explicit reads and guest pointer paths through the shared provider-read
  contract with exact 8 KiB work-RAM bounds.

## 2026-08-22: source-defined PlayStation emulator provider

- Added `state PS1` and a typed `ps1: PS1Emulator` root with source-defined
  discovery for the seven backend families supported by ASR: ePSXe, pSX,
  DuckStation, Mednafen, PCSX-Redux, XEBRA, and multiple RetroArch cores.
- Kept moving DuckStation and PCSX-Redux mappings current at the read boundary,
  validated libretro core lifetime, and routed explicit reads plus guest pointer
  paths through the shared provider-read contract.
- Preserved original PlayStation address bounds and little-endian typed memory
  reads without adding PS1-specific parser or type-checker behavior.

## 2026-08-22: source-defined PlayStation 2 emulator provider

- Added `state PS2` with a typed `ps2: PS2Emulator` root and source-defined
  PCSX2 and RetroArch discovery derived from ASR, including current PCSX2
  exports, legacy 32/64-bit signatures, and the 64-bit libretro core.
- Replaced the GBA-only code-generation branch with one intrinsic-owned
  provider-read contract consumed by explicit reads and state polling alike.
  Renamed the existing public provider type to `GBAEmulator` without an alias.
- Extended emulator-backed `at` fields to follow 32-bit guest pointer paths,
  translating and validating every dereference through the selected provider.

## 2026-08-22: attachment-scoped inferred globals

- Added bare top-level declarations such as `let gameAssembly` for values that
  are discovered by `onAttach`, with bidirectional type inference across
  assignments, state expressions, helper calls, and attached actions.
- Proved definite initialization per successful attachment path and named state
  layout. Direct `match layout` refinement exposes layout-specific values and
  helper requirements propagate transitively like attached-process effects.
- Rejected reads and writes before initialization or outside their attachment
  lifetime, cleared numeric and GC-backed storage on detach, and preserved
  debug-only erasure in both build profiles.
- Added formatter, hover, inlay-hint, documentation, diagnostics, and validated
  WebAssembly GC coverage, including non-null source values backed by nullable
  lifetime storage.

## 2026-08-20: value-producing loop expressions

- Added Rust-style [`loop`] expressions: an unbroken loop has type [`Never`],
  `break value` determines its bidirectionally inferred result, and a bare
  break produces [`None`].
- Kept value-carrying breaks exclusive to [`loop`], so nested [`while`] and
  runtime [`for`] loops always capture their own bare break.
- Preserved loop results and live values across async tick suspension, with
  formatter, diagnostics, editor catalog, language guide, and C#/JavaScript/
  Rust/ASL porting coverage.

## 2026-08-20: numeric tick rates and frame durations

- Generalized dynamic `setTickRate` calls over `Numeric` while retaining the
  runtime's stable `f64` ABI boundary.
- Generalized `Duration.fromFrames` over every integer representation, keeping
  whole-frame division type-directed and avoiding overflow in fractional-frame
  scaling.

## 2026-08-20: exact numeric Duration constructors

- Unified the public unit constructors under `T: Numeric`, accepting every
  built-in integer and floating-point type without retaining parallel
  whole-number names.
- Added compiler-owned, capability-directed standard-library implementation
  cases: one public operation selects an `Integer` or `Float` source body only
  after concrete generic specialization, without introducing general overload
  resolution or a public sealed-capability concept.
- Preserved negative durations and exact full-width integer milliseconds and
  nanoseconds, added representable-range saturation for scaled units, and
  covered direct and generic calls through executable Wasm runtime tests.

## 2026-08-19: composable state-field failure propagation

- Defined one implicit `T!` boundary for every expression-backed state field,
  allowing internal postfix `?` and a fallible final call to propagate into the
  same transactional field update without a helper or nested result.
- Preserved per-field retention for both intermediate discovery failures and
  final read failures while successful sibling fields continue to advance.
- Added compiler-checked reference guidance and an executable Wasm regression
  covering inference, code generation, initialization, and retained values.

## 2026-08-18: semantic links throughout generated documentation

- Made language keywords in compiler-checked examples navigate through the
  same definition identities already used by types, methods, and fields.
- Added ambiguity-safe rustdoc-style intra-doc links for explanatory prose,
  then applied them throughout the standard library, language catalog,
  migration catalog, and bundled author guides.
- Kept compact hover and completion text readable by presenting intra-doc
  references as ordinary code spans outside the full documentation viewer,
  and made graph validation reject future known-symbol prose that is not linked.

## 2026-08-18: staged embedded compilation

- Split the portable compiler service into owned analysis, Wasm-lowering, and
  encoding products without exposing or serializing compiler-internal IR.
- Made Node and browser compiler workers yield between stages, discard a
  superseded debug-watch revision, and return a typed cancellation outcome.
- Kept source diagnostics revisioned, prevented discarded stages from
  publishing artifacts, and covered cancellation through shared connection
  tests and both generated worker runtimes.

## 2026-08-18: cooperative compilation boundary

- Added a thread-safe compilation cancellation token and typed cancellation
  outcome shared by native and embedded compiler products.
- Established safe checkpoints around analysis, Wasm lowering, encoding, and
  publication so cancellation cannot become a source diagnostic or publish a
  superseded artifact.
- Extended the compiler service with a distinct cancellation result and
  recorded the remaining staged-worker protocol needed to observe new requests
  while core Wasm compilation is running.

## 2026-08-18: portable toolchain architecture decision

- Recorded the accepted direct core-Wasm architecture with separate compiler
  and language workers for desktop and web extension hosts.
- Defined the conformance boundary across shared Rust semantics, the Wasm
  adapter, worker transports, packaged extension hosts, and native shells.
- Retired speculative WASI-versus-component packaging alternatives while
  preserving the Component Model as a separate possible LiveSplit host-ABI
  concern.

## 2026-08-18: language-background guides

- Added concise bundled guides for C#, JavaScript, and Rust authors that
  explain SplitScript's lifecycle, fixed-width types, inference, options and
  results, async cancellation, settings, and process-memory boundaries.
- Made every SplitScript snippet independently compiler-checked with hidden
  context, matching the canonical ASL cookbook's validation standard.
- Kept the generated ASL migration map exclusive to the ASL guide while all
  four guides remain navigable from the generated reference index.

## 2026-08-18: capability-driven migration diagnostics

- Completed focused migration diagnostics for legacy lifecycle blocks, timer
  member chains, type-aware member spelling, fallible-value recovery, and
  string concatenation without introducing compatibility aliases.
- Kept machine-applicable rewrites limited to substitutions proven equivalent
  by parser or type resolution; semantic choices remain explanatory fixes.
- Preserved the support categories and canonical documentation links from the
  compiler-owned migration catalog.

## 2026-08-17: complete compiler-checked ASL cookbook

- Completed the remaining helper recipe with explicit caller-selected
  `StateSnapshot` parameters alongside concise contextual `old` and `current`
  access.
- Documented inference, read-only snapshot values, forwarding through helper
  call graphs, and the narrower call-site availability boundary.
- Every focused SplitScript recipe remains independently compiled, while the
  migration catalog and bundled quick map provide the canonical index.

## 2026-08-17: load removal and computed game time migration

- Added compiler-checked recipes for `isLoading` and `gameTime`, including
  their deliberate `None` and fallthrough behavior when a tick has no new
  trustworthy observation.
- Documented typed `Duration` reporting and frame conversion separately from
  the still-unavailable ability to read LiveSplit's current host timer values.
- Added both concepts to the compiler-owned migration catalog, generated
  capability index, and navigable recipe map.

## 2026-08-17: static settings migration recipe

- Added a self-contained compiler-checked recipe for migrating ASL runtime
  registration to typed boolean, choice, and file settings with explicit
  labels, stable host keys, defaults, hierarchy, and documentation tooltips.
- Documented current and previous per-tick settings views, explicit parent
  gating, file filters, and the boundary between typed members, dynamic
  boolean lookup, and declaration membership.
- Repointed the existing registration and dynamic-key migration concepts to
  the focused recipe, so the generated quick map separates ordinary static
  settings from finite compile-time families.

## 2026-08-17: contiguous memory aggregate migration

- Added a compiler-checked porting recipe for replacing physically contiguous
  ASL watchers with one naturally aligned `MemoryReadable` struct or exact
  `[T; N]` state field, based on maintained struct and byte-array ports.
- Documented the one-read behavior, little-endian and natural-alignment rules,
  growable-versus-fixed array distinction, and the cases that must wait for
  explicit packing, offsets, or endianness controls.
- Added the pattern to the compiler-owned migration catalog, generated index,
  and navigable quick map with direct links to `struct`, `[T; N]`, `state`, and
  `Process.read`.

## 2026-08-17: race-free standard-library initialization

- Replaced the check-then-initialize sequence for compiler-derived
  standard-library operation metadata with `OnceLock`'s atomic initialization
  boundary, so parallel compiler and documentation requests cannot derive the
  same source bodies concurrently.
- Added a multithreaded regression test that proves one initialization closure
  wins and every caller observes the initialized catalog.

## 2026-08-17: navigable migration map

- Added a concise quick map to the bundled ASL porting guide, grouping the
  compiler-owned migration catalog by focused cookbook recipe rather than
  duplicating its exhaustive concept index.
- Linked canonical targets directly into the generated language and
  standard-library reference, while keeping target-less planned host features
  visible through their detailed migration pages.
- Added a front-loaded checklist for high-frequency ASL concepts such as exact
  attachment names, named layouts, timer state and split index, polling rates,
  numeric widths, module versions, settings families, arrays, and managed
  strings so authors do not need to read the cookbook linearly.
- Validated the generated recipe anchors and symbol links as part of the
  complete documentation graph.

## 2026-08-17: compiler-checked porting cookbook

- Made every SplitScript fence in the bundled ASL porting guide compile as an
  independent focused program, with rustdoc-style hidden context authored next
  to the visible recipe and removed from rendered documentation.
- The first validation pass caught and corrected stale Unity IL2CPP spelling,
  singleton-call, state-layout delimiter, optional-module, and module-version
  examples instead of allowing the bundled guide to drift from the compiler.
- Reconciled the lifecycle roadmap item with the existing guide and maintained
  setup, state-ready, attached-control, detach, action-default, timer-state,
  and tick-rate runtime fixtures.

## 2026-08-17: Neon White runtime conformance

- Reconstructed the Neon White port independently from its source ASL and used
  direct current-snapshot replacement for transient level IDs, rush-time
  suppression, and double-count prevention.
- Added deterministic host coverage for first-snapshot seeding, both split
  paths, start/reset, game time, failed reads with advancing siblings,
  detachment, and reattachment.
- Promoted only the maintained in-tree port to runtime-verified evidence and
  recorded its single-build offsets, exact Windows process identity, and lack
  of live-game validation separately from the compile-only campaign candidate.

## 2026-08-17: exact semantic standard-library examples

- Removed the shared compiler fixtures that had allowed documentation examples
  to validate unrelated code. Every standard-library example now compiles its
  exact visible snippet through a focused lifecycle wrapper or explicitly
  authored rustdoc-style hidden context.
- Kept multiline visible snippets byte-for-byte contiguous inside validation
  programs, allowing compiler semantic spans and definition links to map back
  to every displayed line instead of silently falling back to lexical color.
- Made catalog validation reject fixtures that omit their visible snippet,
  rewrote examples to be focused and self-contained, and report every broken
  example together in repository verification.

## 2026-08-17: direct guidance for attachment and snapshot mistakes

- Added a compiler-owned attachment-state migration concept and a
  self-contained recipe for native process candidates, typed emulator roots,
  lifecycle availability, and transactional snapshots.
- Linked the missing-state diagnostic directly to that recipe, linked attempts
  to mutate `old` to the existing mutable-current guidance, and linked invalid
  pre-snapshot access to the snapshot-dependent-helper guidance.
- Reconciled the campaign audit with implemented result, string, capability,
  literal setting-key, module-discovery, and documentation routing work, then
  promoted the remaining compile-only Neon White rewrite to the next concrete
  runtime-evidence goal.

## 2026-08-17: validated documentation graph across compiler products

- Added one compiler-owned validation pass over all 413 reference
  pages. It composes language, migration, and standard-library catalog
  validation with canonical page identity, complete metadata, relative link,
  semantic-code HTML link, and heading-fragment checks.
- Added a deterministic reference-index snapshot so an accidental symbol,
  hierarchy, title, signature, or summary change fails repository verification
  and requires an intentional expectation update.
- Compared every native LSP page with the direct documentation model and added
  exact-page checks through both packaged desktop and browser language-server
  workers. The VS Code web-host acceptance test now verifies that catalog hover
  opens the requested symbol page beside the script.

## 2026-08-17: documentation links in ordinary editor workflows

- Added exact compiler-owned documentation identities to catalog-backed hover
  and completion results. Their **Open full documentation** link opens the
  corresponding Markdown page beside the source editor rather than returning
  authors to the reference index.
- Made go-to-definition on language and standard-library symbols navigate to
  the same exact `splitscript-docs:` page while preserving ordinary source
  definitions for user declarations.
- Restricted trusted language-server Markdown to the single documentation
  command; arbitrary command links remain inert in both desktop and browser
  extension clients.

## 2026-08-17: intent-specific ASL module migration diagnostics

- Added one compiler-owned `asl.process.modules` concept that distinguishes the
  main executable, a known optional module, a required waiting module, typed
  executable identity, and genuine unknown-name enumeration.
- Recognizable `modules.First()`, module-list query, and plain enumeration
  shapes now produce focused diagnostics linked to the exact migration page.
  They deliberately avoid automatic rewrites because suspension, optionality,
  and predicate intent must be selected by the author.
- Expanded the self-contained porting guide with focused main-module size and
  typed product-version snippets. Full module enumeration remains an explicit
  host-runtime gap and is not approximated with mapped memory ranges.

## 2026-08-17: self-contained documentation in every compiler product

- Added native `splitc docs`, which renders the same compiler-owned Markdown as
  the editor by exact catalog title, stable migration identity, or virtual page
  path. An omitted topic renders the reference index, and unknown topics offer
  bounded exact-substring suggestions rather than choosing fuzzily.
- Bundled the canonical ASL migration guide into native, language-server, and
  extension products. Reworked it to be self-contained with focused conceptual
  snippets: it neither embeds nor links to complete autosplitter files, so those
  examples can be changed or removed independently.
- Added catalog/page identity checks, self-containment validation, CLI parsing
  coverage, and end-to-end topic rendering for both migration diagnostics and
  standard-library symbols.

## 2026-08-16: navigable language and migration reference

- Extended the compiler-owned documentation graph beyond the standard library
  with searchable language items, lifecycle blocks, contextual roots, every
  structured migration concept, and the canonical ASL porting guide. Pages use
  stable hierarchical identities, breadcrumbs, compiler-checked examples, and
  direct links from migration guidance to canonical language and library APIs.
- Attached stable migration topics to structured parser, resolver, and type
  checker diagnostics. The LSP publishes exact `splitscript-docs:` code links,
  the embedded service preserves the topic, and the native CLI prints it.
- Replaced the obsolete state-less-file error with the current single-file
  autosplitter contract and canonical `state "game.exe" { ... }` / `state GBA
  { ... }` syntax. Added graph-wide page/identity validation and focused
  lifecycle, LSP, CLI, and parser coverage.

## 2026-08-15: explicit current-state overrides

- Made `current.field = value` and compound forms first-class typed
  assignments. They use ordinary field inference and operator resolution,
  remain available through snapshot-dependent helper functions, affect later
  actions in the same tick, and become `old` on the following successful poll.
- Kept `old` directly read-only and added focused diagnostics for attempts to
  rewrite history. State-field filters remain the transactional alternative
  when an invalid candidate must be rejected before initial publication.
- Carried the statement through recovered syntax, semantic navigation,
  formatting, effects, typed HIR, suspension-safe Wasm IR, Wasm GC lowering,
  documentation, and an executing runtime test.

## 2026-08-15: type-directed completion in every type position

- Replaced separate generic-call and shallow state-field completion paths with
  one lexical recognizer for function parameters and results, global and local
  annotations, casts, state and layout fields, struct fields, enum payloads,
  nested arrays, generic constructors, and explicit generic call arguments.
- Restricted those positions to type candidates and structural type snippets,
  eliminating unrelated functions, values, namespaces, and control-flow
  keywords while retaining completion during syntax recovery.
- Centralized primitive, standard-library, named-constructor, source struct,
  source enum, array, option, result, and async candidates with catalog-backed
  documentation and snippets.

## 2026-08-15: declaration-aware type mismatch diagnostics

- Carried the source of an expected type through parameters, explicit return
  types, globals, locals, later assignments, state fields and their filters,
  struct fields, enum payloads, and nested option/result/collection checking.
- Reworded concrete mismatches around the supplied value, including
  unsuffixed integer and floating-point literals, while preserving focused
  capability, integer-range, optional-value, and fallible-value diagnostics.
- Added secondary labels at the declaration that imposed the expectation, so
  the native CLI and LSP show both the incompatible use and its type contract.

## 2026-08-15: statically safe settings-key lookups

- Indexed every declared host key and its setting kind in the declaration
  environment, including explicit keys and expanded finite-family keys.
- Validated literal keys passed to `SettingsView.enabled` and `contains`, with
  declaration labels, closest-key rewrites, kind-specific guidance, and direct
  member suggestions where one exists. Computed strings retain runtime lookup
  semantics.
- Completed compatible keys inside quoted arguments for both current and old
  settings views, replacing only the string contents and carrying setting
  documentation into the editor.

## 2026-08-15: source-annotated native CLI diagnostics

- Replaced the native compiler and formatter CLI's flat diagnostic strings
  with `codespan-reporting` source annotations while retaining the shared
  compiler-owned diagnostic model used by the LSP and embedded service.
- Preserved stable `SS` codes, warning severities, notes, fix guidance, Unicode
  byte spans, and terminal-aware color across one-shot builds and watch mode.
- Rendered all primary and secondary labels as source locations. A real
  duplicate-state diagnostic verifies that both declarations remain visible,
  alongside focused coverage for warnings, fixes, and zero-width EOF spans.

## 2026-08-15: self-guiding fallible and capability diagnostics

- Replaced dead-end `T!` use errors with structured guidance for `else` and
  `match`, adding postfix `?` only when the enclosing function actually has a
  fallible boundary.
- Recognized `return fallibleValue else None` in a function returning `T?` and
  provided a machine-applicable `else return None` rewrite, validated by
  compiling the edited source.
- Preserved capability requirements through failed inference so diagnostics
  name the exact missing capability. Declared-only capability sets also list
  every accepted concrete type, while structural capabilities avoid claiming
  that structs or fixed arrays form a closed set.

## 2026-08-15: explicit string-concatenation guidance

- Replaced the generic numeric-capability error for string `+` and `+=` with a
  focused diagnostic that recommends template interpolation for `Display`
  values and `String.concat` for an existing `[String]`.
- Covered string/string and mixed string/numeric expressions, local compound
  assignment, and indexed compound assignment without adding string `+` or an
  evaluation-changing automatic rewrite.
- Kept numeric and user-defined catalog operators on the ordinary resolution
  path; the diagnostic triggers only when an operand resolves to the standard
  `String` type.

## 2026-08-15: six-script porting campaign audit

- Audited every report from the `69f2bd9` campaign against its source ASL, the
  current compiler, maintained ports, and deterministic host fixtures without
  rerunning the campaign or treating compilation as behavioral parity.
- Classified Arietta of Spirits, Operation Matriarchy, and Neon White as
  compile-only candidates; Aquanox, A Plague Tale: Innocence, and Axiom Verge
  are behavior-limited. No candidate was an intentionally failing probe.
- Recorded the exact omissions and separated a fixed formatter defect,
  discoverability failures for existing facilities, genuine compiler work,
  host-runtime requirements, and unsupported claims. Five separate maintained
  ports have bounded runtime evidence; Neon White still needs promotion and a
  deterministic fixture.

## 2026-08-12: one exact process-detach lifecycle action

- Replaced the overlapping `onDetached` and `onProcessExit` actions with
  `onDetach`, which runs exactly once after a real attached process closes and
  never during initial detached startup.
- Made `onDetach` parameterless and unavailable to process providers or state
  snapshots, because closure can occur before attachment discovery or the
  first snapshot completes.
- Removed the detached-entry runtime flag and moved initial detached policy to
  the existing `setup` boundary; scripts that change persistent tick rates now
  establish them in `setup` and restore them in `onDetach`.
- Updated ASL `exit` migration guidance to target `onDetach`; exact ASL
  `shutdown` remains a separate host-runtime requirement.

## 2026-08-12: exact process-exit lifecycle

- Added synchronous `onProcessExit`, which runs exactly once after a previously
  attached process closes, never during initial detached startup, and before
  `onDetached` restores detached-state policy.
- Cleared the unusable handle, provider state, selected layout, and pending
  continuations before invoking source cleanup, while withholding snapshots
  because closure can precede their initialization.
- Replaced the migration cookbook's `attachedOnce` guard after finding 409
  legacy ASL `exit` blocks, including widespread game-time pause cleanup.

## 2026-08-12: attached-update timer-decision control

- Made `whileAttached` fall through as true and accept an explicit boolean
  result, with false returning from the generated update before every timer
  decision while retaining the refreshed state snapshot.
- Matched legacy ASL `update { return false; }` directly instead of adding a
  separate `shouldEvaluate` concept or a rollback-like state-field mechanism.
- Confirmed the pattern in 220 of 968 extracted ASL update blocks and added
  runtime coverage for both not-running and running timer phases.

## 2026-08-12: post-snapshot attachment initialization

- Added synchronous `onStateReady`, which runs once per attachment after the
  first complete state poll commits with equal `old` and `current` snapshots.
- Kept suspending discovery and layout selection in `onAttach`, and delayed
  `whileAttached` plus timer decisions until the following update.
- Matched the useful post-refresh half of legacy ASL `init` without exposing
  default-initialized state or adding a host ABI, allocation, or hidden guard.

## 2026-08-12: contextual state snapshots in helper functions

- Allowed ordinary helpers to read `old` and `current` directly and propagated
  the resulting snapshot requirement through recursive source call graphs.
- Rejected direct and transitive use from `setup`, `onAttach`, `onDetached`,
  state reads, and state filters, while filtering invalid completions and
  exposing the requirement in function hovers.
- Reused the already double-buffered GC snapshot globals in code generation,
  preserving first-snapshot seeding without hidden parameters or per-tick
  allocations, and documented the canonical ASL migration pattern.

## 2026-08-10: deterministic integer radix formatting

- Added catalog-declared `Integer.toString(radix) -> String!` for bases 2
  through 36 across every fixed-width integer and `address`.
- Defined lowercase digits, sign-magnitude output for negative values and
  signed minima, explicit invalid-radix errors, and uppercase composition
  through `String.toAsciiUpperCase`.
- Generalized the existing allocation helper without changing decimal Display,
  and added runtime, must-use, completion, hover, and C# migration coverage.

## 2026-08-10: exactly-once compound indexed assignment

- Added arithmetic, remainder, bitwise, and shift compound assignments for
  array elements through the same catalog-defined operators as ordinary
  assignments.
- Evaluated the collection, index, and right operand once in source order and
  retained compiler temporaries across suspensions without exposing synthetic
  bindings to source tooling.
- Preserved aliases, narrow-integer masking, bounds traps, and non-structural
  iteration behavior, with synchronous and multi-suspension runtime coverage.

## 2026-08-10: timer metadata migration boundaries

- Audited recurring ASL timer paths against the actual LiveSplit Wasm host and
  confirmed that current game time, run/segment metadata, run offset, and
  timing-method access have no present imports.
- Added distinct compiler diagnostics for script-owned versus host-owned game
  time, read-only route metadata, and controlled user-visible timer mutation,
  avoiding generic unknown-name errors and unsafe `Instant` substitutions.
- Added migration-catalog entries and cookbook guidance tied to the typed,
  coherent-snapshot runtime requirements already recorded in R5.

## 2026-08-10: source-defined first-match array removal

- Added equality-constrained `[T].remove(value) -> bool` as standard-library
  source composed from `indexOf` and `removeAt`, matching ordered C# list
  semantics without another backend intrinsic.
- Removed only the first duplicate, reported absence without mutation, kept
  aliases and capacity stable, released reference slots, and invalidated
  iteration only when a match was removed.
- Added reverse empty-array inference, fixed-array diagnostics and completion
  filtering, runtime coverage, and `List<T>.Remove` migration guidance.

## 2026-08-10: source-defined optional array pop

- Added growable `[T].pop() -> T?` as a standard-library body composed from
  indexed access and `removeAt`, without adding a backend intrinsic.
- Kept an empty pop non-structural, preserved aliases and capacity, released
  removed reference elements, and invalidated active iteration only when an
  element was removed.
- Made `None`/value conditional inference independent of branch order and
  covered fixed-array diagnostics, completion, runtime aliases, reuse, and
  iteration behavior.

## 2026-08-10: in-place indexed array removal

- Added growable `[T].removeAt(index)` with logical-length bounds checks,
  overlap-safe in-place shifting, stable aliases, and retained capacity.
- Released the vacated trailing reference slot for WebAssembly garbage
  collection and treated successful removal as a structural mutation that
  invalidates active iteration.
- Kept exact arrays free of length-changing methods and added completion,
  inference, diagnostics, primitive/reference runtime, bounds, and migration
  documentation coverage. This primitive can support a future source-defined
  optional `pop` without another backend implementation.

## 2026-08-10: source-defined bulk array extension

- Added `[T].extend(values)` as ordinary standard-library source composed from
  indexed reads and `push`, without introducing another backend intrinsic.
- Captured the source length before mutation so aliases remain stable and
  self-extension duplicates the original sequence exactly once.
- Added fixed-array filtering and diagnostics, bidirectional empty-array
  inference, C# `AddRange` migration guidance, and runtime coverage for growth,
  reference elements, aliases, and fail-fast iteration.

## 2026-08-10: capacity-preserving growable-array clear

- Added source-declared `[T].clear()` while keeping `[T; N]` exact and free of
  length-changing methods.
- Preserved wrapper identity and backing capacity across clearing, released
  live reference slots for garbage collection, and invalidated active
  iteration as a structural mutation.
- Added inference, completion, fixed-array diagnostics, aliasing, GC-reference,
  and fail-fast runtime coverage without emitting helpers for unused array
  layouts.

## 2026-08-10: capability-backed indexed array assignment

- Added `array[index] = value` for both growable and exact-length arrays by
  lowering it through the source-declared `Array.set` standard-library
  operation rather than adding a second mutation backend.
- Preserved stable aliases, Wasm bounds checks, and non-structural iteration
  semantics while evaluating the array and index exactly once.
- Added a focused boundary for compound indexed assignment until its lowering
  can guarantee the same single evaluation for effectful receivers and indices.
- Replaced the maintained runtime fixture's direct `set` call with the new
  syntax and added formatter, type, aliasing, and diagnostic coverage.

## 2026-08-10: usage-driven empty-collection inference

- Allowed `[]` and `Set.new()` to retain an unresolved element variable across
  local bindings instead of rejecting them at their construction site.
- Let later push/insert operations, indexing, assignments, returns, and
  function arguments constrain that same variable through ordinary
  bidirectional inference.
- Added source-local ambiguity diagnostics only for collections that remain
  genuinely unconstrained after function generic generalization and numeric
  defaulting, plus inferred inlay-hint coverage.

## 2026-08-10: fail-fast structural collection iteration

- Added a structural version to stable array and set wrappers and captured it
  when a `for` loop begins, including across suspending loop bodies.
- Made array append and successful set insertion/removal/clearing invalidate
  active traversal through every alias, trapping on the next loop advance
  instead of silently skipping or revisiting values.
- Kept element replacement and no-op set mutations valid during traversal and
  avoided per-loop collection snapshots or hot-path allocations.
- Added host runtime coverage for alias mutation, fail-fast traps, and the
  permitted non-structural/no-op cases.

## 2026-08-10: stable source-array values

- Replaced the public array value's raw Wasm GC array representation with a
  stable wrapper containing replaceable backing storage and a logical length.
- Preserved `[T; N]` as the exact memory-readable source type while sharing the
  same wrapper ABI with `[T]`; exact raw storage remains available underneath.
- Adapted literals, memory reads, indexing, iteration, length, replacement,
  async signature scanning, and runtime helpers to distinguish the wrapper
  from its backing and to observe logical length rather than future capacity.
- Canonicalized wrapper and raw-storage layouts by physical backend element
  type so independently specialized generic arrays share valid Wasm GC types.

## 2026-08-10: compiler-internal array storage identity

- Separated raw capacity-backed Wasm GC arrays from source-level `[T]` in the
  backend type and reachability graphs.
- Migrated `Set<T>` allocation, growth, copying, removal, clearing, and
  iteration to the internal storage identity without changing source behavior.
- Established the representation boundary needed to make source arrays stable
  wrappers while retaining raw fixed storage for memory layouts.

## 2026-08-10: one array family for C# list migration

- Rejected a separate future `List<T>` compatibility type: C# ordered list
  semantics belong on variable-length `[T]`, while `[T; N]` remains the exact
  fixed-length form.
- Corrected migration diagnostics and documentation so `Set<T>` is recommended
  only for genuinely unique unordered data, never as a substitute for a list.
- Planned stable identity plus capacity-backed storage for growable arrays so
  Wasm GC's fixed physical array length does not break aliases or force one
  allocation per append. This supersedes earlier archived array-or-set wording.

## 2026-08-10: typed ASL timer-phase migration

- Mapped the recurring `timer.CurrentPhase` and `TimerPhase` variant patterns
  to the existing `timer.state()` and exhaustive `TimerState` API.
- Added machine-applicable path rewrites while keeping a source-defined enum
  named `TimerPhase` legal and untouched.
- Documented `TimerState.Unknown` and rejected migration by numeric enum order;
  timer time, split metadata, timing method, and run offset remain distinct
  host-contract work.

## 2026-08-10: source-specific C# `Convert` migration

- Classified the observed integer, floating-point, boolean, and string
  `Convert.To*` families separately instead of suggesting one unsafe cast.
- Kept migration rewrites deliberate where C# checked overflow,
  midpoint-to-even conversion, current-culture parsing, null handling, or
  formatting semantics differ from SplitScript.
- Recorded integer radix formatting as a remaining standard-library gap and
  removed the completed `Convert.To*` audit from the active campaign queue.

## 2026-08-10: type-aware C# collection count migration

- Added a machine-applicable `.Count` to `.length()` rewrite after the receiver
  has resolved to an array or `Set<T>`.
- Kept collection selection separate: arrays count ordered elements, while sets
  count unique values, so the existing `List<T>` migration diagnostic still
  requires the author to choose from source behavior first.
- Refined the active campaign-audit queue so completed length/count and numeric
  guidance are not repeatedly planned as missing features.

## 2026-08-10: type-aware C# length migration

- Added separate diagnostics for C# array and string `.Length` instead of
  treating the shared spelling as one operation.
- Made array length a machine-applicable `.length()` rewrite while preserving
  its `u32` result in the guidance.
- Kept string length deliberate: emptiness becomes `isEmpty()`, while
  `byteLength()` is reserved for proven ASCII or UTF-8 byte-oriented logic
  because C# counts UTF-16 code units.

## 2026-08-10: source-defined numeric squaring

- Added `Numeric.squared()` as a canonical source-defined method implemented
  by one multiplication, preserving receiver width, integer wrapping, and IEEE
  floating-point behavior without another compiler intrinsic.
- Added focused `Math.Pow`/`MathF.Pow` guidance for the corpus-proven cases:
  `value.squared()` for exponent two and an explicit width-checked shift for
  power-of-two masks.
- Deliberately left general floating powers out of the public API until a
  maintained port needs them and can justify carrying a vetted libm algorithm.

## 2026-08-10: source-defined signed absolute value

- Moved `abs()` from the floating-point-only capability to `Signed` and
  implemented it in canonical SplitScript source from `max` and unary
  negation, making it available to signed integers and floats without a
  dedicated compiler intrinsic.
- Documented and runtime-tested floating negative zero and NaN together with
  the language's wrapping signed-minimum behavior.
- Added focused `Math.Abs`/`MathF.Abs` migration guidance that contrasts C#'s
  signed-minimum exception and calls out unsigned conversions and decimal
  overloads instead of applying an unsafe rewrite.

## 2026-08-10: typed minimum and maximum migration guidance

- Added focused `Math.Min`/`MathF.Min` and `Math.Max`/`MathF.Max`
  diagnostics that route calls to the existing type-preserving `min` and `max`
  methods, including fully qualified `System` paths.
- Documented and runtime-tested the floating-point contract: NaN propagation,
  negative-zero selection for `min`, and positive-zero selection for `max`.
- Kept implicit C# numeric conversions and decimal overloads explicit rather
  than guessing an automatic rewrite or result type.

## 2026-08-10: directed-rounding migration guidance

- Added focused `Math.Floor`/`MathF.Floor` and
  `Math.Ceiling`/`MathF.Ceiling` diagnostics that route binary floating-point
  values to the existing `floor()` and `ceil()` methods.
- Made f32/f64 width, directed-rounding behavior, IEEE special values, and the
  unsupported C# decimal overloads explicit rather than applying an unsafe
  rewrite.
- Recognized corpus-proven fully qualified `System.Math` and `System.MathF`
  paths without broadening migration matching to arbitrary namespaces.

## 2026-08-10: overload-aware C# rounding guidance

- Added focused migration diagnostics for `Math.Round` and `MathF.Round`,
  directing default midpoint-to-even calls to `round()` and decimal-place
  calls to `roundTo(digits)`.
- Made the `Math`/`MathF` result-width distinction explicit and deliberately
  withheld an automatic rewrite for decimal inputs and explicit
  `MidpointRounding` modes such as `AwayFromZero`.
- Added the compiler-owned rounding concept to the generated migration
  capability index and covered canonical f32 and f64 forms.

## 2026-08-10: source-declared floating-point truncation

- Added `Float.truncate()` to the canonical standard-library source with an
  exact trusted contract and direct `f32.trunc`/`f64.trunc` WebAssembly
  lowering.
- Documented and tested rounding toward zero, signed zero, infinity, NaN, and
  preservation of the receiver's f32 or f64 width.
- Added focused `Math.Truncate`/`MathF.Truncate` migration guidance that makes
  the C# result-width distinction explicit rather than applying an unsafe
  rewrite.

## 2026-08-10: source-declared floating-point square root

- Added `Float.sqrt()` to the canonical standard-library source with an exact
  trusted contract and direct `f32.sqrt`/`f64.sqrt` WebAssembly lowering.
- Documented and tested positive values, negative inputs, signed zero,
  infinity, NaN, and preservation of the receiver's f32 or f64 width.
- Added focused `Math.Sqrt`/`MathF.Sqrt` migration guidance that makes the C#
  result-width distinction explicit rather than applying an unsafe rewrite.

## 2026-08-10: type-directed integer complement and narrow arithmetic

- Added `Integer.bitNot()` and overloaded unary `!` by type: booleans use
  logical negation, while integers use width-preserving bitwise complement.
  The foreign `~` spelling receives a machine-applicable migration fix.
- Centralized post-operation normalization for `i8`, `u8`, `i16`, and `u16`
  across arithmetic, bitwise and shift operations, compound assignments, and
  unary operators so physical WebAssembly `i32` storage cannot leak past the
  source type's width.
- Covered catalog identities, migration recovery, global constants, direct
  methods, ordinary and compound arithmetic, signed extension, and real
  Wasmtime execution.

## 2026-08-10: catalog-owned unary operators

- Added `bool.not()` and `Signed.negate()` to the canonical source-defined
  standard library and bound `!` and unary `-` to those declarations.
- Extended the backend-neutral operator schema, catalog validation, intrinsic
  contracts, inference, and Wasm IR lowering so unary syntax and direct method
  calls publish the same semantic call identities.
- Preserved zero-overhead global constants and primitive Wasm lowering,
  including boolean inversion, signed integer wrapping, floating-point sign
  changes, and direct Wasmtime execution coverage.

## 2026-08-10: catalog-owned structural equality

- Added `equals` and `notEquals` to the source-defined `Equatable` capability
  and bound `==` and `!=` to those declarations, completing catalog ownership
  for non-logical binary operators.
- Preserved specialized semantics for strings, standard enums, structs, payload
  enums, options, results, scalar values, IEEE 754 floats, and zero-sized
  `None`, including demand-driven structural equality functions.
- Taught reachability and code generation to plan equality from intrinsic call
  identities, and covered both operator syntax and direct method calls through
  semantic resolution and valid Wasm GC output.

## 2026-08-10: source-declared integer bit operations

- Declared bitwise OR, XOR, AND, left shift, and right shift on the `Integer`
  capability, with method forms and focused documentation for signed shifts,
  discarded bits, and WebAssembly shift-count masking.
- Extended the operator catalog and trusted intrinsic boundary so `|`, `^`,
  `&`, `<<`, `>>`, and their compound assignments resolve through standard-
  library declarations instead of checker-only capability branches.
- Reused the common primitive binary lowering path and covered operator syntax,
  direct method calls, semantic identities, compound assignment, and generated
  documentation fixtures.

## 2026-08-10: source-declared multiplicative operators

- Declared multiplication and division on the `Numeric` capability and
  remainder on `Integer`, including focused documentation of integer traps,
  truncation, remainder signs, and floating-point division.
- Extended the backend-neutral operator schema and trusted intrinsic contracts
  so `*`, `/`, `%`, and their compound assignments resolve through those source
  declarations rather than a parallel checker-only rule.
- Unified ordinary-call and compound-assignment lowering through one mapping
  from numeric intrinsic identities to WebAssembly arithmetic operations, with
  catalog-resolution and code-generation coverage for every operator.

## 2026-08-10: heterogeneous operators and monotonic deadlines

- Generalized catalog-defined binary operators so the right operand may differ
  from the receiver while arithmetic results still preserve the receiver type.
  Ordinary numeric and Duration operators retain their same-type declarations.
- Added source-defined `Instant + Duration` and matching compound assignment for
  concise monotonic deadlines. The exact nanosecond calculation clamps below
  the clock origin and above the largest representable instant rather than
  wrapping.
- Covered inferred and explicit result types, source-defined lowering, compound
  assignment, negative underflow, positive overflow, and the real monotonic
  clock import in compiler and deterministic Wasm runtime tests.

## 2026-08-10: Unicode-scalar string padding

- Added immutable `String.padStart(width, fill)` and
  `String.padEnd(width, fill)` for corpus-backed zero-padding and diagnostic
  table alignment without adopting C# method aliases or optional parameters.
- Defined width as a count of Unicode scalar values. The runtime counts UTF-8
  leading bytes, reuses an already-wide string, and otherwise encodes the fill
  `char` directly into one exact-sized output allocation.
- Added focused `PadLeft`/`PadRight` migration guidance covering direction,
  explicit space filling, and the difference between .NET UTF-16 code units,
  Unicode scalar values, and terminal display columns.
- Extended catalog, documentation, diagnostics, and deterministic Wasm runtime
  coverage across ASCII and multibyte receivers and fill characters.

## 2026-08-09: allocation-free reverse string search

- Added `String.lastIndexOf(substring) -> u32?` for the recurring path,
  filename, and object-name searches found in seven corpus scripts.
- Kept the language's explicit string model: positions are UTF-8 byte offsets,
  absence is `None`, the empty substring matches the final byte boundary, and
  the reverse scan performs no intermediate allocation.
- Added focused C# `LastIndexOf` migration guidance for the UTF-16 index and
  `-1` sentinel differences rather than silently rewriting surrounding index
  arithmetic or overloads.
- Covered missing, empty, overlapping, ASCII, and multibyte matches in the
  deterministic WebAssembly string runtime fixture.

## 2026-08-09: single-allocation string joining

- Added `String.join(values, separator)` for typed `[String]` collections,
  preserving empty elements and adding separators only between adjacent values.
- Generalized interpolation, `String.concat`, and `String.join` onto one Wasm
  helper. Concatenation uses an internal null separator, so it does not allocate
  a synthetic empty string; the final output remains one exact-sized allocation.
- Added focused C# `String.Join` guidance without swapping arguments
  automatically because its object, enumerable, variadic, and range overloads
  require explicit conversion to `[String]`.

## 2026-08-09: typed string emptiness

- Added source-defined `String.isEmpty()` as ordinary composition over
  `byteLength`, with generated documentation, completion, hover, must-use, and
  runtime coverage but no backend intrinsic.
- Added focused guidance for the recurring C# `String.IsNullOrEmpty` pattern.
  Required strings use `.isEmpty()`, while `String?` must explicitly match
  `None` and `Some`; failed process reads retain their separate Result policy.
- Deliberately avoided recreating nullable strings or guessing an automatic
  rewrite from a static C# call whose migrated value type is not yet known.

## 2026-08-09: explicit ASCII whitespace trimming

- Added `String.trimAsciiWhitespace()` for recurring corpus parsing of game
  identifiers, log lines, and configuration text.
- The operation removes the six ASCII whitespace bytes at the boundaries,
  preserves non-ASCII and interior bytes, and reuses an unchanged immutable
  string. Its single scan feeds the existing UTF-8 slice helper without
  allocating intermediate Results.
- Added focused no-fix guidance for C# `Trim()`: its Unicode whitespace model,
  character-array overloads, `TrimStart`, and `TrimEnd` need explicit review.

## 2026-08-09: allocation-conscious ASCII uppercasing

- Added `String.toAsciiUpperCase()` for corpus uses involving mission names,
  map identifiers, and hexadecimal text without claiming locale-sensitive or
  full Unicode behavior.
- Generalized lower- and uppercasing onto one Wasm runtime helper and one Rust
  emitter. Both directions reuse an already-normalized immutable string and
  allocate only when an ASCII letter changes.
- Added the C# `ToUpper()` migration spelling, generated catalog metadata,
  completion/hover coverage, and runtime tests that preserve non-ASCII bytes.

## 2026-08-09: Result-aware C# string replacement migration

- Connected corpus uses of C# `String.Replace` to the existing immutable
  `String.replaceAll` operation with a focused diagnostic and cookbook entry.
- Kept the migration free of automatic edits: SplitScript requires explicit
  Result handling, while C# also permits a null replacement to mean deletion.
- Reused the source-defined standard-library catalog rather than adding a
  compatibility alias or another string implementation.

## 2026-08-09: allocation-free UTF-8 substring positions

- Added `String.indexOf(substring) -> u32?` using the existing allocation-free
  runtime search helper. It returns the first UTF-8 byte offset, zero for an
  empty needle, and `None` when absent.
- Added runtime coverage including a non-ASCII prefix, proving that the result
  is a byte offset rather than a Unicode-scalar or UTF-16 index.
- Added a focused C# `IndexOf` diagnostic with no automatic edit because C#
  uses UTF-16 code units and a `-1` sentinel while SplitScript uses byte offsets
  and `Option`.

## 2026-08-09: composable C# duration-constructor migration

- Verified that the common C# forms `TimeSpan.FromSeconds(...)` and
  `TimeSpan.FromMilliseconds(...)` produce staged machine-applicable fixes for
  both the type and callable spelling.
- Applying all offered edits now has regression coverage proving that the
  result is canonical, compilable `Duration.fromSeconds(...)` and
  `Duration.fromMilliseconds(...)` source.
- Added the corpus-proven C# static-property rewrite from `TimeSpan.Zero` to
  the canonical constructor call `Duration.zero()`; no compatibility property
  was introduced.
- Separated ordinary value-path migrations from rewrites that require an
  attached native process, so process availability no longer leaks into
  unrelated fixes.
- Classified corpus uses of `TimeSpan.Parse` instead of adding a misleading
  compatibility parser. The compiler now distinguishes fixed duration data
  from needless timer-value string round trips and explains why each requires
  an explicit rewrite.
- Added source-defined `Duration.fromMinutes`, `fromHours`, and `fromDays`
  conversions from corpus evidence. They reuse `fromSeconds`, carry ordinary
  generated documentation and editor metadata, and add no backend intrinsic.
- Deliberately kept C# ticks out of the public API; exact subsecond sources use
  the language-level nanosecond constructor instead.
- Added a dedicated `TimeSpan.FromTicks` diagnostic that states the
  100-nanosecond conversion and its signed-range limitation rather than
  offering an unsafe whole-expression edit.
- Kept JavaScript `${...}` out of typo recovery because it is valid
  SplitScript source with observably different literal-dollar semantics.

## 2026-08-09: Rust binding-modifier recovery and literal dollar semantics

- `let mut` now recovers as an ordinary mutable SplitScript `let` declaration
  with a machine-applicable removal of the redundant Rust modifier, at both
  global and local declaration sites.
- Deliberately did not diagnose JavaScript-style `${value}`. In SplitScript,
  `$` is ordinary template text, so that spelling validly emits a dollar sign
  followed by an interpolation; deleting it would change observable output.
- Documented this template behavior and removed both decisions from active
  foreign-spelling work.

## 2026-08-09: JavaScript strict-equality recovery

- Kept JavaScript `===` and `!==` invalid while lexing each as one recoverable
  source token. The parser now reports a single structured migration diagnostic
  and offers a machine-applicable `==` or `!=` replacement.
- Added `==` and `!=` as documented language-catalog syntax, giving migration
  metadata, generated documentation, hover queries, and validation one
  canonical target rather than a parser-only spelling table.
- Removed the completed operator fixes from the active foreign-spelling work.

## 2026-08-09: C# static numeric-parser discovery

- Connected the existing strict `String.parse<T>()` implementation to familiar
  C# `Parse` and `TryParse` calls across fixed-width integer and floating-point
  type names.
- The focused diagnostic moves the target type to the receiving boundary and
  explains Result-based fallbacks and output-parameter migration without an
  unsafe mechanical rewrite.

## 2026-08-09: boundary-aware C# substring migration

- Confirmed that `Substring` is a recurring ASL corpus pattern and added a
  type-aware diagnostic instead of treating it as an ordinary method typo.
- The diagnostic and checked porting guide distinguish C#'s `(start, length)`
  and suffix overloads from SplitScript's fallible `(start, exclusiveEnd)`
  slice, and explicitly require an encoding review because C# positions are
  UTF-16 code units while SplitScript positions are UTF-8 bytes.
- Removed the already-completed native-string decoder diagnostic slice from
  the active migration roadmap.

## 2026-08-09: type-aware C# string-equality migration

- Added a focused diagnostic for C# `String.Equals` on the resolved
  standard-library string type. It directs exact comparisons to `==` and
  explicitly case-insensitive ASCII identifiers to `equalsIgnoreAsciiCase`
  without mistaking user-defined methods or mechanically rewriting C#
  comparison-mode overloads.
- Extended the checked migration catalog and ASL porting guide with the same
  equality distinction.

## 2026-08-09: ASL runtime settings-registration guidance

- Bounded startup-generated boolean settings were already expressible as
  compile-time settings families. The migration catalog now separately records
  legacy runtime registration, and `settings.Add(...)` receives focused
  guidance for individual declarations, finite families, stable keys, and
  data-driven lookup without offering an unsafe mechanical rewrite.

## 2026-08-09: safe named-layout selection discovery

- Made the missing-`onAttach` diagnostic generate a placeholder selector for
  every declared layout, always ending in `await process.closed()` so unknown
  builds remain attached but inert instead of polling a guessed layout.
- Added a machine-applicable process-close fallback when an existing selector
  can complete without returning a layout, together with focused rationale in
  the diagnostic.
- Specialized top-level `onAttach` completion for named layouts with the same
  exhaustive, unsupported-build-safe structure while retaining ordinary action
  completion elsewhere.
- Treated the generated layout value's type as a GC storage root, allowing a
  deliberately inert process-close-only selector to compile without requiring
  any reachable layout constructor.

## 2026-08-09: conventional compiler CLI discovery

- Replaced the hand-written native argument parser with one derived Clap model,
  while keeping Clap out of the embedded compiler's Wasm dependency graph.
- Added successful `splitc --help`/`-h` and `splitc --version`/`-V` queries,
  plus `help`, `watch --help`, and `fmt --help` command-specific discovery.
- Documented compilation outputs, profiles, warning policy, watch behavior, and
  formatting checks in the CLI itself rather than treating help flags as input
  paths.
- Kept malformed or incomplete invocations distinct with exit status 2 and a
  concise standard-error usage hint; integration tests exercise the real
  binary's streams, status codes, and stable version form.

## 2026-08-09: encoding-aware native-string migration

- Re-audited ASL `stringN` against LiveSplit's implementation: the suffix is a
  byte count, while auto-detection chooses UTF-16LE only when the second byte is
  zero. SplitScript continues to require an explicit encoding.
- Added distinct maybe-incorrect migration actions for UTF-8 byte bounds and,
  when the ASL bound is even, the equivalent half-sized UTF-16LE code-unit
  bound. Odd bounds deliberately receive no inexact UTF-16 rewrite.
- Corrected the maintained Battlefront II port and fixture from an erroneous
  32-byte UTF-16 read to the source's 16-byte ASCII/UTF-8 read. The dedicated
  process-operation fixture continues to cover UTF-16 surrogate pairs,
  replacement decoding, NUL termination, failures, and bounds.

## 2026-08-09: semantic migration for legacy lists

- Triaged the campaign's array-search and growing visited-map reports against
  the source-defined array methods, `Set<T>`, and maintained OpenJK-Speed and A
  Plague Tale ports.
- Documented the semantic split between fixed ordered arrays, growable unique
  sets, compact closed-domain bit sets, and the still-planned growable ordered
  collection that preserves duplicates.
- Added a focused `List<T>` diagnostic that explains both supported choices,
  including `indexOf`'s optional `u32` result, without offering a rewrite that
  could silently change ordering, duplication, or lifetime behavior.

## 2026-08-09: discoverable cooperative scan migration

- Confirmed that module, explicit-range, and process-wide signature scans
  already yield after bounded work, preserve their cursor across ticks, and
  cancel with the attached process. The maintained A Hat in Time port and
  deterministic process-scan fixtures exercise that behavior.
- Added a migration concept and cookbook recipe explaining that legacy C# scan
  threads should be removed rather than translated, and how to choose among
  `scan`, `scanAny`, `scanMemory`, and `scanMemoryAny`.
- Kept timeout or race behavior explicit and planned instead of changing a
  retrying awaited scan into an implicit one-shot result.

## 2026-08-09: discoverable immutable-snapshot migration

- Triaged legacy assignments to `current` against the ASL corpus and the
  existing state-field rejection semantics. Retaining the prior watcher value
  is already supported; arbitrary derived or run-owned mutations require a
  different destination rather than mutable snapshots.
- Corrected the migration catalog to describe the supported trailing field
  `if` and `Err(message)` pattern, including its first-snapshot and independent
  sibling-field behavior.
- Added a focused diagnostic for legacy `current.field = ...` statements. It
  explains filtering, derivation, and script-owned state without offering an
  unsafe structural rewrite.

## 2026-08-09: monotonic-delay migration boundary

- Triaged campaign reports that treated every real-time delay as unavailable.
  Confirmed that event-anchored debouncing, cooldowns, and delayed actions map
  to the existing monotonic `Instant` and exact `Duration` APIs.
- Added a migration concept, cookbook recipe, and focused `DateTime.Now`
  diagnostics for that supported pattern.
- Kept LiveSplit's `timer.CurrentTime.RealTime` as a distinct planned host
  capability and added a separate diagnostic, preventing run-relative timer
  metadata from being silently replaced with process-independent elapsed time.

## 2026-08-09: explicit timer split-index migration

- Triaged the campaign's reported split-index blocker against the legacy ASL
  corpus and the current `timer.currentSplitIndex()` implementation.
- Added a compiler-owned migration concept and cookbook recipe that preserve
  the optional `u64` model, including the negative no-attempt sentinel, skipped
  segments, and the post-final-split index.
- Added a focused diagnostic for `timer.CurrentSplitIndex` without an automatic
  rewrite: correct migration must choose how `None` affects the surrounding
  control flow rather than merely changing member casing.

## 2026-08-09: discoverable attached-process identity migration

- Triaged the campaign's reported process-name blocker against the current
  compiler and confirmed that `process.name()` already provides the required
  attached identity.
- Added a compiler-owned `asl.process.identity` migration concept and cookbook
  recipe explaining exact matched candidates versus executable paths, module
  metadata, versions, and signatures.
- Added a contextual diagnostic for ASL `game.ProcessName`. It offers a
  machine-applicable `process.name()` rewrite only where a native attached
  process is truly available, explains explicit parameter passing in ordinary
  functions, and preserves user-defined members with the same spelling.

## 2026-08-09: reusable port-conformance host

- Extracted a shared Node host for the generated LiveSplit ABI, including
  exact process attachment, modules, 64-bit memory ranges and custom reads,
  settings registration and snapshots, timer state and actions, host metadata,
  monotonic time, variables, messages, and tick-rate observations.
- Added bounded `updateUntil` polling with diagnostic state, explicit process
  close/restart control, settings mutation, and handle-leak observability so
  maintained ports can cover failures and lifecycle transitions without
  copying ABI boilerplate.
- Migrated action defaults, tick-rate lifecycle, host metadata, the complete
  settings showcase, and signed pointer traversal to exercise distinct parts
  of the shared harness.
- Added a port-conformance guide explaining fixture construction, CI
  registration, evidence boundaries, and why compilation alone is not runtime
  fidelity.

## 2026-08-09: executable tick-rate lifecycle contract

- Added a deterministic host fixture covering an initial 60 Hz detached baseline,
  process-specific 100 and 120 Hz attachment rates, and restoration to 60 Hz
  after both detachments.
- Documented that `setTickRate` is measured in updates per second, controls the
  host wait after the current update returns, and persists until another call.
  Process closure does not reset it automatically.
- Established the approachable lifecycle pattern: `onDetached` establishes the
  baseline initially and restores it after closure, while `onAttach` selects
  the attached cadence. No duplicate `setup` call is needed.
- Recorded finite-value and scheduler-bound validation as a host-runtime
  correctness requirement rather than hiding it in the compiler.

## 2026-08-09: exact floating-point representations and result obligations

- Added `f32.fromBits`/`f64.fromBits` and `.toBits()` as ordinary documented
  standard-library operations backed by Wasm reinterpret instructions. Signed
  zero, subnormals, infinities, and NaN payloads round-trip without numeric
  conversion or allocation.
- Decimal-literal hover now reports the inferred `f32` or `f64` width and its
  exact rounded IEEE-754 bit pattern, complementing the existing target-width
  underflow and overflow diagnostics.
- Made non-mutating value-producing standard-library operations must-use by
  default, while preserving specific authored explanations and leaving
  mutating status-returning operations intentionally discardable.
- Covered exact runtime bit patterns for both widths, catalog signatures,
  hover output, and the generic must-use policy.

## 2026-08-09: cascade-free failed-declaration recovery

- Failed global, local, and suspending `let` initializers now retain their
  declared source identity with an internal poison type. Later references no
  longer become misleading unknown-variable errors merely because the
  initializer already produced a focused diagnostic.
- The poison type absorbs contextual unification and capability requirements,
  and propagates through member access, indexing, calls, and operators without
  inventing secondary type errors. Strict compilation still fails and the
  poison type can never reach code generation.
- Preserved editor navigation through invalid code: go to definition and rename
  continue to resolve a failed binding while inlay hints hide its deliberately
  unknown type. Regression coverage includes globals, ordinary locals, and
  awaited declarations plus chained members, indexes, calls, comparisons, and
  generic `Display` consumers.

## 2026-08-09: Unicode characters and explicit UTF-8 string inspection

- Added the nominal `char` type and single-quoted literals for exactly one
  Unicode scalar value. Characters support equality, matching, `Display`, and
  lossless conversion to `u32`, while remaining separate from integer and
  process-memory types.
- Added fallible `String.byteAt(byteIndex)` for raw UTF-8 bytes and
  `String.charAt(byteIndex)` for the character beginning at a byte boundary.
  Both operations follow the existing byte-index policy instead of inheriting
  JavaScript or C# UTF-16 indexes.
- Kept inspection allocation-free below the language-level error boundary. The
  shared Wasm helper checks bounds and defensively validates complete UTF-8,
  including overlong encodings, surrogate code points, and U+10FFFF.
- Extended the deterministic string fixture across one- through four-byte
  values, continuation-byte rejection, raw continuation-byte access, and
  end/out-of-range failures.
- Updated the maintained Tiberian Sun port to compare its proven ASCII splash
  positions with character literals without allocating temporary strings.

## 2026-08-09: compile-time boolean settings families

- Added a finite inclusive `for value in start..=end` form to the settings DSL.
  Label and stable-key templates interpolate the compile-time u32 binding, one
  default applies to every generated boolean, and family documentation becomes
  every generated tooltip.
- Preserved the ordinary settings pipeline by lowering families into concrete
  declarations before validation and code generation. The retained source
  declaration independently powers formatting, highlighting, hover, and
  binding navigation; generated implementation names never appear as typed
  settings members.
- Promoted Drug Dealer Simulator as the motivating maintained port. Its 35
  level settings, exact keys and labels, defaults, tooltips, live changes,
  pointer path, split behavior, process lifecycle, and `.exe` host identity are
  covered by a deterministic runtime fixture.
- Kept truly discovered or unbounded mutable settings deferred to the host
  evolution work rather than making a compile-time convenience imply runtime
  mutation.

## 2026-08-09: explicit ASCII string normalization

- Added immutable `String.toAsciiLowerCase()` with an intentionally narrow
  ASCII contract. It changes only `A` through `Z`, preserves all other UTF-8
  bytes, and reuses the receiver without allocation when no byte changes.
- Kept the API in the source-defined standard-library catalog with its docs,
  example, must-use obligation, signature, effects, completion, and hover.
  Rust owns only the validated Wasm GC byte-transform helper.
- Extended the string runtime fixture across mixed ASCII, non-ASCII UTF-8, and
  already-normalized input, and documented C# `ToLower()` migration without
  pretending to provide culture-sensitive Unicode casing.
- Promoted Tiberian Sun as the motivating maintained port, covering its
  localized completion-text patterns, explicit native decoding, fallible byte
  slices, timer behavior, and process lifecycle in a deterministic host.

## 2026-08-09: faithful Nioh multi-layout port

- Replaced the campaign's unsafe newest-version fallback with a maintained
  port of all three Nioh layouts selected by main-module size. Unsupported
  builds report their size and remain inert until the process closes.
- Translated ASL's extensionless `Nioh` process name to the current Windows
  host's required `Nioh.exe`, while retaining the 29 Hz cadence, direct and
  indirect fields, and loading predicate. Detach restores 1 Hz.
- Added host fixtures for versions 1.21.04, 1.21.05, and 1.21.06 plus an
  unsupported build. They cover 64-bit pointer traversal, independent field
  progress after a failed read, timer-action order, attach/detach, layout
  selection, messages, and tick-rate transitions.
- This completes the first repository-owned campaign pressure set alongside
  OpenJK, Battlefront II, and Dark SASI.

## 2026-08-09: faithful Dark SASI timer-metadata port

- Replaced the campaign's level-only partial translation with a maintained
  port of DarkSASI's split-index dispatch. `timer.currentSplitIndex()` keeps
  LiveSplit authoritative and exposes the signed host sentinel as `None`.
- Recreated the source stopwatch through monotonic `Instant` values. Index 2
  restarts the timestamp on every poll until its level-8 split; the final split
  occurs at the exact 52-second threshold and clears the pending value.
- Added host fixtures for full and skipped routes, absent split indices,
  manual timer restart, loading, exact nanosecond threshold behavior, and
  detach. This validates existing timer/clock APIs instead of adding callbacks,
  a timer-phase mirror, or game-specific state.

## 2026-08-09: faithful Battlefront II bounded native-string port

- Promoted `swbf2_loadremover_v2.asl` into a maintained host-executed port. Its
  16-byte Galactic Conquest sentinel now uses the explicit bounded `utf8(16)`
  decoder rather than ASL's heuristic `string16` representation.
- Preserved both settings, stable keys, defaults, tooltips, precedence,
  victory and Galactic Conquest split transitions, and mode-specific loading
  rules without adding a game-specific compiler abstraction.
- Added fixtures for both settings configurations, NUL and full-bound native
  input, ignored data after a terminator, failed-read retention, timer action
  behavior, settings-handle cleanup, and detach. Recorded the extensionless
  process name as evidence without claiming the deferred cross-platform policy
  is solved.

## 2026-08-09: faithful OpenJK run-scoped-set port

- Promoted the reviewed `OpenJK-Speed.asl` behavior into a maintained
  SplitScript port instead of treating the generated campaign candidate as
  authoritative. The port preserves start/reset/loading transitions, ignored
  academy and empty maps, and one split per other visited map.
- Replaced the ASL `List<string>` with one global `Set<String>`. The set is
  cleared at the per-process `onAttach` boundary and by the original reset
  action on a stable opening-map tick, retaining state across ordinary ticks
  without allocating on every update.
- Added a deterministic host fixture covering the exact process name, bounded
  string read, timer-action order, duplicate suppression, run reset, loading,
  detach, and reattach. Documented the explicit UTF-8 choice and preserved the
  reset-block cleanup at its original lifecycle boundary.

## 2026-08-06: run-scoped sets and source generic type applications

- Added `Set<T>` as the first growable run-scoped collection, with
  `Set.new<T>()`, `length`, source-defined `isEmpty`, linear `contains`,
  `insert`, `remove`, `clear`, and direct `for` traversal. Global construction
  runs once per script instance, so visited checkpoints persist across ticks
  without allocating a new set on every update.
- Kept the public declaration in `stdlib/standard.split`. The generated type-
  constructor catalog now retains parameter constraints, so the type checker
  enforces `T: Equatable` from that declaration rather than a duplicate Set-
  specific rule. Completion, hover documentation, semantic highlighting, and
  static `Set.new` completion consume the same catalog identity.
- Lowered each reachable concrete set to one mutable Wasm GC object holding a
  growable GC-array backing store and logical length. The runtime fixture covers
  growth, duplicate insertion, persistence, removal, iteration, clearing, and
  string equality; the representation allocates only for construction, growth,
  and clearing.
- Added first-class source syntax identities for named generic applications,
  while retaining every written occurrence for lossless tooling. The formatter
  consequently understands generic angle delimiters, compact and multiline
  trailing commas, and repeated `Set<T>` annotations.

## 2026-08-06: source-defined array search

- Added `contains` and `indexOf` to both `[T]` and `[T; N]` as ordinary
  standard-library source bodies. Their per-method `Equatable` constraint,
  signatures, documentation, completion, hover data, and focused examples all
  originate in `stdlib/standard.split`; no backend intrinsic was added.
- Canonicalized provisional array layouts after inference so independently
  inferred occurrences of the same `[T]` type share one WebAssembly GC nominal
  identity. Compile and runtime coverage exercises integer and string search,
  present and absent indices, and equality-constraint failures.

## 2026-08-05: explicit script-instance setup and lifecycle migration guidance

- Added `setup`, a synchronous process-independent lifecycle block that runs
  once per loaded Wasm instance after globals and settings are initialized.
  It has a true zero-parameter Wasm signature and is invoked by `_start`; the
  host defers that initializer to the beginning of the first interruptible
  update rather than executing arbitrary user code during instantiation.
- Kept lifecycle boundaries explicit: `setup` cannot access a process provider,
  state snapshots, `await`, or `retry`; `onAttach` remains the per-process
  suspending discovery phase. Debug-watch replacement naturally runs setup for
  the newly loaded instance.
- Added targeted, non-machine-applicable diagnostics for legacy ASL `startup`,
  `init`, `update`, `exit`, `shutdown`, and timer-event-shaped blocks. The
  migration catalog and cookbook now explain exact timing differences and
  identify cases that still require a host contract instead of suggesting
  unsafe aliases.
- Added parser, type/effect, catalog-hover, Wasm-validation, and host-runtime
  coverage, including proof that setup runs once during `_start` and never on
  later updates.

## 2026-08-04: faithful A Hat in Time production port

- Ported the production-scale autosplitter with its full settings hierarchy,
  IL mode, detailed/rift/position split tables, game-time correction,
  start/reset behavior, and split lock.
- Replaced the legacy background thread with process-lifetime-cancelled,
  suspending discovery. Process-wide scans traverse readable mapped ranges in
  deterministic bounded windows and yield between windows so one update cannot
  monopolize the autosplitting thread.
- Represented discovered timer, save-data, actor, and coordinate roots through
  ordinary source globals, structs, arrays, functions, and declarative state
  expressions rather than game-specific compiler paths.
- Used the official current-split-index observation for the polling-based
  debounce lock and documented its skip/undo limitation instead of inventing a
  host callback. Runtime fixtures cover signature fallback, discovered pointer
  chains, settings behavior, scan budgets, and unsupported builds.

## 2026-08-04: source-defined cooperative engine discovery

- Moved Unity IL2CPP and GBA emulator discovery out of compiler-owned helpers
  and into ordinary privileged SplitScript bodies in `stdlib/standard.split`.
  Provider metadata can now name any compatible source-defined or intrinsic
  catalog callable; the generic attachment lifecycle polls its typed async
  frame and cancels it when the process closes.
- Added bounded memory-range selection and multi-signature module scanning.
  Each poll inspects at most one host range or one signature window, so neither
  engine discovery path can hide an unbounded scan behind an async-looking API.
- Removed the hard-coded GBA process/signature tables and attachment helper.
  Emulator policy, signatures, layout selection, and supported process names
  are now co-located with `GBAEmulator.discover`; only target-address
  translation and host reads remain representation intrinsics.
- Extended the Minish Cap runtime fixture to cover both stable mGBA and
  pointer-backed VBA mappings and to assert the per-update memory-range query
  bound.

## 2026-08-04: optional pointer fields and faithful Aquanox port

- Added explicit optional pointer fields: `field: T? at ...` reads the
  contained `MemoryReadable` representation but accepts module, traversal,
  final-read, or decoder failure as `None`. Required `T at ...` fields retain
  their existing initialization and last-accepted-value semantics.
- Lowering constructs `Result<Option<T>>` directly for native and provider
  reads. An absent optional field can initialize a snapshot and advance
  independently between `None` and `Some(T)` without fabricating a default or
  weakening transactional required fields.
- Added a maintained Aquanox port. Its optional secondary menu string becomes
  `None` when loading invalidates the pointer, preserving the original manual
  split condition alongside automatic/final splits, start/reset behavior,
  load removal, and detach cleanup.
- Expanded compiler, Wasm-validation, host-runtime, language-reference,
  catalog-hover, and ASL-porting coverage. The transactional runtime fixture
  proves optional scalar and UTF-8 fields can initialize absent, become
  present, become absent again, and recover.

## 2026-08-04: contextual syntax has precise editor identities

- Audited every identifier spelling interpreted contextually by the canonical
  grammar. State pointer-path `at`, stable settings-map `key`, settings
  `choice` / `default` / `file` / `mime`, and loop `in` now retain exact spans
  and resolve to their documented language-catalog concepts.
- Semantic highlighting consumes those same syntax spans instead of treating
  the spellings as global keywords. An ordinary parameter, local, field, or
  function named `at`, `key`, `choice`, `default`, `file`, `mime`, or `in`
  therefore keeps its source-symbol identity.
- Added focused state-pointer and stable-key catalog documentation plus
  compiler-query and highlighting regressions. Existing `utf8`, provider, and
  globally reserved keyword paths remain shared with their established
  standard-library or language-catalog identities.

## 2026-08-04: precise editor cursor boundaries

- Centralized editor symbol selection in the lossless source document while
  distinguishing character-based hover from zero-width caret operations.
  Hover gives precedence to an exact token beginning at the pointer, so the
  `(` in `foo(bar)` no longer selects `foo` as well.
- Definition, references, and rename conventionally keep selecting a word at
  its half-open end, including before adjacent punctuation. Meaningful postfix
  `?`/`!` tokens retain their own language identities, and whitespace is never
  skipped.
- Added lossless, compiler-query, and UTF-16 LSP regressions for adjacent
  punctuation, a one-space gap, line/file endings, hover, navigation,
  references, rename, and postfix punctuation.

## 2026-08-04: attachment-aware root completion

- Root completion now shares the exhaustive lifecycle attachment predicate
  used by semantic validation. `process` and `gba` are omitted from
  `onDetached`, while attaching and attached lifecycle actions retain their
  selected typed provider.
- Detached completion filters global catalog operations and source functions
  that require an attached process. User-function requirements come from the
  same fixed-point `OperationAnalysis` as diagnostics, including transitive
  helper calls rather than a second completion-only effect model.
- Incomplete root identifiers are replaced by a temporary inert `None` probe
  when necessary, so transitive filtering remains available during ordinary
  typing. Added native/GBA and direct/transitive/safe function regressions.

## 2026-08-04: provenance-safe numeric inference defaults

- Numeric defaulting now follows literal provenance through unification and
  generic instantiation. Components containing an unsuffixed integer or
  floating-point literal default to `i32` or `f64`; an integer-looking literal
  in a required floating-point context defaults to `f64`. A specific `Integer`
  or `Float` constraint likewise selects the corresponding language default.
- Broad capability-only constraints no longer manufacture a concrete
  representation. `Numeric`, `Signed`, `MemoryReadable`, and `Display` values
  remain ambiguous; affected process reads, state fields, locals, globals,
  parameters, arrays, wrappers, and generic bodies require an annotation, with
  a focused diagnostic for pointer-backed state fields whose memory type cannot
  be inferred.
- `MemoryReadable` is a hard defaulting boundary: a process read or `at` field
  never obtains its representation from a numeric literal or an `Integer` or
  `Float` default. Memory width and interpretation must be explicit or inferred
  from another exact type.
- Recovering editor analysis now uses an explicit semantic error type instead
  of publishing `i32`. It has no source spelling, layout, capabilities, or
  code-generation path, and inlay hints suppress it while other editor queries
  remain available.
- Added inference and editor regression coverage and made the Lunistice points
  and deaths memory widths explicit after the stricter rule exposed their
  former `Display`-only inference.

## 2026-08-03: faithful Arietta of Spirits port

- Added a maintained port of the Arietta of Spirits 1.2.9.0 autosplitter. It
  preserves stage-based start, split, and reset transitions together with
  pause-menu load removal using two bounded native UTF-8 state fields.
- Added release-Wasm host coverage for initial snapshot seeding, all timer
  actions, process closure, the complete pointer paths, and independent
  persistent-field advancement when one final string read fails.
- Expanded the ASL porting guide with the compact example and the observed
  bounded-string failure semantics. The fixture now runs in `cargo xtask check`.

## 2026-08-03: source-body signature conformance

- Added a semantic construction-time boundary for source-defined
  standard-library bodies. Every public receiver, parameter, and completion
  shape must be a consistent instance of the ordinary SplitScript body's
  inferred function scheme before operation analysis begins.
- The validator handles nested Array, Option, Result, and fixed-array shapes.
  It permits safely more-general inferred templates while rejecting concrete
  narrowing, inconsistent generic relationships, and undeclared inferred
  capability requirements. Capability inheritance is respected.
- Isolated this compiler-semantic responsibility in
  `validation/stdlib_bodies.rs` and added focused fixtures so malformed
  privileged source cannot reach monomorphization or Wasm code generation.

## 2026-08-03: initialized persistent state fields

- Audited the original LiveSplit ASL refresh path and ASR watchers. Legacy ASL
  seeds `old` and `current` from one real read but substitutes zero or null for
  failed fields; SplitScript now preserves the safe initialization behavior
  without inheriting those failed-read defaults.
- State initialization requires every required field to succeed in one poll,
  seeds `old == current`, and skips lifecycle actions for that poll. Detaching
  clears readiness so a new process can never inherit a previous process's
  values.
- Made the state-field assignment the persistent watcher boundary. Later
  successes advance independently and errors retain that field's accepted
  value, while `toOption()` deliberately accepts `None`. Struct- and
  array-valued fields remain the explicit unit for values that must advance
  atomically.
- Removed the fabricated field-local `old` binding. Pointer filters receive
  only the raw `value` and return `Err(message)` to reject a transient candidate;
  lifecycle `old` remains the full preceding snapshot.

## 2026-08-03: immutable per-field state filtering

- Audited AAWCB, Aragami, and the wider ASL corpus and separated three
  previously conflated behaviors. Aragami is ordinary persistent derived state
  already modeled by `whileAttached`; AAWCB requires retaining one field's
  last accepted value while other fields advance; boolean ASL `update` results
  are a later lifecycle-evaluation gate and do not roll state back.
- Added an ordinary trailing `if` expression to state fields. Its read-only
  `value` and `old` bindings have the field's inferred type; the first
  successful poll passes the raw candidate as both, and later polls use the
  last committed field value. `current` and `old` remain immutable and the
  complete filtered snapshot still commits atomically.
- Added the faithful And All Would Cry Beware port. Its transient scenes 7 and
  8 are filtered without discarding entity-count changes from the same tick,
  eliminating the original mutation of `current.Scene`.
- Added runtime coverage for independent field advancement, snapshot rotation,
  and detach/re-attach initialization, plus formatter, catalog documentation,
  semantic highlighting, typed hover, and completion coverage for the new DSL
  context.

## 2026-08-03: attach-time-discovered and optional state sources

- Added the source-defined generic `Result<T>.toOption()` operation. It maps a
  successful read to a present `T?` and a failed read to `None`, so one
  deliberately optional field can be absent without weakening the required
  fields in the same transactional snapshot.
- Extended inference so `Some`/`None` and `Ok`/`Err` patterns can determine an
  otherwise unconstrained wrapper input, and allowed nested wrapper lifts such
  as `T?` into the outer `T?!` produced by a state poll. The standard-library
  implementation uses the ordinary source checker and Wasm-GC lowering.
- Expanded the host-executed process-result fixture: required failures still
  skip the complete tick, while an optional read failure commits `None` and a
  later successful read becomes present. Catalog tests keep `toOption()`
  source-defined and its generic signature visible to editor tooling.
- Documented the full attach-discovery contract using maintained ABZÛ and
  Borderlands ports: polling begins only after `onAttach` completes,
  unsupported builds wait for process closure, and PE32 traversal selects
  `PointerSize.Bit32` on `MemoryPath` rather than introducing `at32` syntax.

## 2026-08-03: string-keyed settings and faithful Akiba runtime coverage

- Added optional `key "host-key"` metadata to settings declarations. The key is
  always the exact nonempty, globally unique string used for host registration,
  persistence, tooltips, and live refresh; source code continues to use the
  readable declaration identifier.
- Declared the shared `SettingsView.enabled(key: String) -> bool` surface in
  `stdlib/standard.split`. Its trusted backend implementation searches only
  declared boolean settings in the already-refreshed current or previous
  snapshot, returns false for unknown or heterogeneous keys, performs no host
  query, and allocates no polling-time strings.
- Replaced Akiba's Trip's temporary 50-way settings match with string keys in
  its immutable mission structs and `settings.enabled(point.settingKey)`.
  Added release-Wasm runtime coverage for module-relative reads, exact host-key
  registration, live enable/disable changes, chapter gating, coordinate
  matching, and the prologue transition.
- Added parser and semantic diagnostics, formatter/highlighting support,
  standard-library completion and hover documentation, migration guidance,
  and current/previous settings runtime checks. Repository-wide verification
  passes with Akiba promoted from compile-only to a runtime fixture.

## 2026-08-03: next production pressure case selected

- Re-audited the manual ASL porting notes against the compiler-owned migration
  capability index instead of treating every old deferral as a missing feature.
- Selected Akiba's Trip: Undead & Undressed as the next maintained port:
  structs, fixed arrays, `for`, decimal rounding, and nested settings already
  cover its data model, while typed data-driven settings lookup remains a
  genuine, high-frequency corpus gap.
- Kept growable collections out of the initial goal because this splitter's
  coordinate tables are immutable; fixed arrays express their actual lifetime
  and make settings identity the isolated language boundary.

## 2026-08-03: one-time aggregate globals

- Added global struct, fixed-array, variable-array, and string constants whose
  GC values are materialized exactly once in the WebAssembly module start
  function instead of being reconstructed on every polling tick.
- Split module-start emission out of settings code so enum globals, aggregate
  globals, initial snapshots, async storage, and setting registration have one
  explicit initialization boundary.
- Added a backend regression that validates the generated Wasm GC and proves
  the struct, array, and string constructors run in `_start` before the globals
  are observed.
- Added the maintained Akiba's Trip: Undead & Undressed port with seven typed
  mission-coordinate tables, source-faithful f32-to-f64 rounding, nested route
  settings, and a temporary explicit setting-key match. Repository verification
  now compiles and validates the port as a release artifact.

This roadmap is ordered by dependency and impact, not merely implementation
size. Language semantics should settle before editor tooling treats them as a
stable public contract.

Priority meanings:

- **P0** — correctness or foundational design needed by real autosplitters.
- **P1** — important language and API ergonomics once the foundations exist.
- **P2** — tooling, migration, build profiles, and ecosystem scaling.
- **Ongoing** — work that should continuously validate every other priority.

## Active-roadmap consolidation (2026-08-03)

The active roadmap was rebuilt after commit `8e2edc9` so it contains only
unfinished, deliberately deferred, and ongoing work. The large checked
checklists removed at that point represented these completed milestones:

- one canonical source-defined standard library in `stdlib/standard.split`,
  with validated privileged declarations, a small intrinsic trust boundary,
  source-defined synchronous and suspending bodies, inferred generic function
  schemes, demand-driven specialization, and catalog-driven documentation and
  tooling;
- first-class storable `async T` futures with expression-level `await`, typed
  Wasm-GC frames, nested suspension, process-close cancellation, and shared
  source/intrinsic polling semantics;
- typed state providers for ordinary processes and GBA, including the faithful
  Minish Cap port, fixed-length memory-readable arrays, generic process reads,
  expression-backed state fields, and transactional snapshots;
- named uniform state layouts selected by `onAttach`, safe unsupported-build
  waiting, typed module/process identity, PE file-version discovery, version
  literals, source-defined `MemoryPath`, and maintained ABZU, Martha Is Dead,
  Borderlands, and Alan Wake pressure cases;
- source-defined capability inheritance and `Display`, array iteration, UTF-8
  string operations, numeric helpers, `Duration`, monotonic `Instant`, operator
  roles, warnings and unused-code analysis, migration diagnostics, formatter
  improvements, and identity-aware editor features;
- a self-contained VS Code desktop/web extension using separate bundled core-
  Wasm compiler and language-service workers, with release builds, revision-
  safe debug watch, virtual-workspace support, and a real
  `@vscode/test-web` acceptance test; and
- `None` as the sole unit type and value, with `void` removed and plain unit
  erased from function ABI, locals, globals, discarded expressions, and async
  completion storage. Aggregate physical-layout specialization remains a
  deliberately deferred optimization.

Detailed earlier architectural history remains below. New completed milestones
should be summarized here rather than accumulating checked boxes in
`TODO.md`.

### Faithful A Plague Tale: Innocence port (2026-08-03)

The first goal after the roadmap consolidation re-audited the original ASL and
ported its complete maintained behavior rather than the scalar subset produced
by the old batch converter:

- Steam, Epic, Xbox, and unsupported executable layouts are selected in
  `onAttach` and exercised by host-run fixtures;
- its `string50` map field uses bounded UTF-8 decoding after the complete
  pointer path, settings preserve the original parent gating, a typed chapter
  enum plus `u32` bit set prevents duplicate checkpoint splits, and timer-start
  detection resets run progress without a C# event handler;
- explicit `timer.pauseGameTime()` / `resumeGameTime()` intrinsics expose the
  existing least-privilege host operations, and guarded `onDetached` cleanup
  preserves the ASL `exit` behavior without running at initial detached entry;
- the original runtime behavior of `stringN` is recorded in
  `docs/ASL_PORTING.md`. A Plague Tale's ASCII identifiers fit the existing
  strict UTF-8 decoder; native UTF-16/autodetect replacement behavior remains
  evidence-driven rather than becoming another pseudo-type family;
- the port uncovered and fixed a typed-HIR panic when an enum value was
  implicitly lifted into `T?`, and corrected decoded state fields that had
  consumed the base address before resolving their remaining pointer offsets;
  and
- `cargo xtask check` compiles and validates the example, runs all four new
  fixtures, and retains the complete native/compiler/extension/browser/runtime
  matrix.

### Structured migration catalog and capability index (2026-08-03)

Migration knowledge now has a dependency-light compiler-owned catalog with
stable string IDs, ASL/C#/JavaScript/Rust provenance, direct/pattern/planned/
non-goal status, canonical language and standard-library targets, cookbook
anchors, and applicable foreign spellings. The compiler validates every target
against the active catalogs and every recipe anchor against the maintained
porting guide; `docs/MIGRATION_CAPABILITIES.md` is generated from the same data
and protected by a drift test. Existing `const`/`var`, `null`, `func`/
`function`, `string`, `TimeSpan`, and C# numeric-type diagnostics now consume
these entries without changing their machine-applicable edits.

The first shape-aware rules are catalog-owned as well. A type-first ASL
`stringN` state field recovers into an explicit bounded decoder and offers a
non-preferred, maybe-incorrect UTF-8 rewrite with the ASL UTF-16/autodetection
caveat. Duplicate state declarations explain named layouts and attach-time
selection but deliberately offer no automatic merge. Regression tests preserve
ordinary fields named like `string50`, later declarations, and focused
single-diagnostic recovery.

## P0 — Maintainability audit and architectural convergence (2026-07-30)

This audit covers the current compiler, standard library, backend, language
server, extension, tests, and roadmap after the first hierarchical
standard-library migration. The migration was worthwhile, but it exposed
several boundaries that are still nominal rather than enforced. Address these
before another large language or standard-library expansion.

Measured hotspots at the time of the audit:

- `parser.rs`: 3,207 lines / 109 functions;
- `typeck.rs`: 3,055 lines / 72 functions;
- `codegen/runtime_helpers.rs`: 2,621 lines;
- `database.rs`: 1,935 lines / 86 functions;
- `codegen/expression.rs`: 1,725 lines;
- `lsp.rs`: 1,645 lines;
- `wasm_ir.rs`: 1,516 lines;
- `stdlib.rs`, `language.rs`, `formatter.rs`, `hir.rs`, `stdlib/catalog.rs`,
  and `codegen.rs`: each over 1,000 lines;
- `tests/compiler.rs`: 7,414 lines / 129 tests;
- nine codegen submodules import their parent with `use super::*`;
- compiler stages directly construct the global `StandardLibrary` 89 times;
- `IntrinsicId` is matched 48 times in expression emission, 33 times in
  dependency planning, 21 times in Wasm IR, and 15 times in async lowering.

File size is a symptom, not the primary refactoring criterion. Split a file
only after its responsibilities communicate through a named input/output
boundary; do not turn one shared mutable context into many mutually coupled
files.

The intended dependency direction is:

```text
catalog schema -> validated catalog graph -> compiler context
source text -> syntax -> declarations/resolution -> typed HIR
typed HIR -> complete backend IR -> backend plans -> Wasm encoding
editor protocol -> compiler database/query snapshots -> compiler stages
```

No arrow should point backwards, and catalog producers must be replaceable
without changing their consumers.

### 1. Catalog graph, compiler context, and future stdlib loader

- [x] Replace the zero-sized, globally reconstructed `StandardLibrary` with an
  immutable validated graph owned by `CompilerContext` and passed through
  parsing, resolution, checking, tooling, documentation, and backend lowering.
  `StandardLibrary` now owns an `Arc` graph, products are cloneable rather than
  copyable, algorithms borrow the graph, and an independently constructed graph
  is injected through the complete pipeline in tests. Source-loaded declaration
  storage remains active future work rather than a hidden global-lifetime
  assumption.
- [x] Thread the first compiler-context seam through the active pipeline:
  parsed, lowered, checked, and recovered products retain one
  `CompilerContext`; inference, typed HIR, semantic validation, database-backed
  tooling, documentation queries, Wasm IR, backend planning, and emission all
  consume its standard-library handle. The 89 audit-time catalog
  reconstructions are now limited to compatibility entry points,
  context-free AST formatting, and tests. The parent item remains open until
  alternate/source-loaded graphs can be constructed and exercised.
- [x] Make the physical Wasm GC layout a catalog-derived plan. `GcLayout` now
  owns standard and inferred type indices, standard field slots, enum variant
  indices, value representations, and the async-frame position. Emitters no
  longer reconstruct the default catalog, so semantic and physical phases
  cannot silently disagree about the selected standard library.
- [x] Break the former dependency cycles between authored catalog data,
  declarations, public queries, and compiler semantic types. The layers are
  now one-way: one raw hierarchy in `stdlib/source.rs`; independently generated
  opaque IDs in `ids.rs`; dependency-light declaration and callable schemas;
  normalized data in `catalog.rs`; validation and indexed graph consumers;
  then the compiler-specific `stdlib_semantic` adapter.
- [x] Break the compiler-semantic half of that cycle. The standard-library
  schema, authored catalog, declarations, and graph no longer depend on
  `BuiltinType` or semantic `TypeKind`; `stdlib_semantic` is now the one-way
  adapter that owns typed call candidates and receiver applicability for the
  checker, completion, and hover. An architecture test rejects future
  semantic-type imports in backend-neutral catalog modules.
- [x] Split the declaration producer without creating a parallel registry.
  `source.rs` contains the hierarchy once and invokes independent ID and
  normalized-data consumers. `declarations.rs` no longer imports authored
  catalog tables, callable schema lives in `schema.rs`, and catalog-wide
  checks live above both in `validation.rs`. Architecture tests enforce these
  directions.
- [x] Make the authored hierarchy survive as a real normalized graph with
  owner-to-children and name/path indices. The current macro is hierarchical
  only at its input and immediately emits flat slices; most queries repeatedly
  linearly scan those slices. Flat deterministic iteration views may remain,
  but ownership and lookup must be indexed graph facts.
- [x] Decide and implement the ID model required by a source-loaded standard
  library. Rust enum variants are convenient well-known handles but cannot be
  the identity of declarations loaded from SplitScript. Use data-backed stable
  IDs/newtypes plus an explicit table of well-known compiler contracts, or a
  build-time source compilation scheme; do not make every consumer depend on
  generated Rust variants. Catalog symbol identities are now opaque `u32`
  newtypes with generated well-known constants and an internal loader
  constructor; their debug names preserve existing diagnostics. Only
  `IntrinsicId` remains a closed enum because compiler implementations must be
  exhaustively trusted.
- [x] Replace the positional `function_item!` / `method_item!` invocation
  grammar with one named callable declaration nested under its owner. Kind,
  generic parameters, value parameters, result, effects, availability,
  documentation, intrinsic binding, and the focused public example now live
  together in `stdlib/source.rs`; `catalog.rs` contains only the normalizing
  consumer and shared full-program example-validation fixtures. An
  architecture test rejects the retired callable factories and verifies that
  every bundled intrinsic has the complete named metadata shape.
- [x] Generalize the catalog type-expression model. `stdlib::TypeRef` now has
  atoms for core/nominal/parameter identities plus one recursive
  `Application { constructor, arguments }` form keyed by an open
  `StdlibTypeConstructorId`; Array, Option, and Result are declared unary
  constructors rather than closed expression variants. Rendering, inference,
  receiver applicability, hover substitution, and expected-Result inference
  consume that shape, while catalog validation checks constructor identity,
  arity, recursive arguments, and parameter scope. Future generic catalog
  constructors no longer require another `TypeRef` variant, although their
  semantic/runtime implementation still needs an explicit trusted contract.
- [ ] Use `StdlibCapabilityId` directly in generic bounds and replace the
  closed `TypeConstraint::{Numeric, MemoryReadable}` adapter. Replace
  inference's fixed `Requirements` mapping and `capabilities.rs` hard-coded
  capability switch with one extensible capability solver/registry that can
  describe marker, structural, and custom privileged capabilities.
  - [x] Generic bounds now contain catalog capability IDs directly; catalog
    validation rejects missing IDs and the retired `TypeConstraint` enum is
    gone.
  - [x] Inference requirements are a deduplicated capability-ID set rather
    than a fixed eight-bit mirror, so source-loaded IDs cannot collide or be
    silently ignored.
  - [x] Each capability declaration now selects `Declared`, structural
    equality, or structural memory-layout behavior. Inference admissibility,
    method discovery, and final semantic validation dispatch through that
    descriptor instead of matching the Equatable/MemoryReadable IDs in each
    phase.
  - [ ] Add the trusted custom-capability handler registry when the first
    capability cannot be expressed by declared membership, structural
    equality, or structural memory layout. Validate handler bindings exactly
    like intrinsic call bindings rather than adding another ID switch.
- [x] Normalize effects into a validated effect set rather than an arbitrary
  slice. Reject contradictions such as `Pure` plus writes/allocation, derive
  suspension/cancellation facts once, and cross-check intrinsic/helper/host
  effects so a public declaration cannot understate its implementation.
  - [x] Public callable effects now use a canonical deduplicated `EffectSet`
    with deterministic iteration. Catalog validation rejects empty sets,
    `Pure` combined with any observable effect, retry/suspend conflicts,
    impossible cancellation, process reads without attachment, and invalid
    onAttach availability; `OperationSemantics` derives suspension,
    attachment, and cancellation from that normalized set.
  - [x] Cross-check each intrinsic's declared effects against its trusted
    lowering/helper/host descriptor once the intrinsic registry below owns
    those implementation facts.
    The trusted intrinsic contract owns and exactly validates public effects
    and availability. The runtime-helper registry now recursively derives the
    observable timer, process, and runtime effects of every direct helper and
    ABI root, rejects unsupported ABI effect categories, and requires exact
    agreement with the trusted contract before Wasm emission.
- [ ] Give all standard-library symbols one documentation/link identity and
  validate examples for namespaces, types, fields, variants, capabilities,
  constructors, and callables. Callable docs currently use `StdlibItemId`
  links while other symbols use `StdlibSymbolId`, and only callables require
  examples.
  - [x] Use `Documentation<StdlibSymbolId>` for callables as well as every
    other library symbol, and validate related links against the complete
    symbol graph. Cross-kind links no longer need a callable-only adapter.
  - [ ] Author and compile focused examples for non-callable symbols, then
    require examples uniformly without padding documentation with artificial
    shared fixtures.
- [x] Generate the ABI and language-catalog IDs and tables from their
  declarations as well. `AbiImportId` currently has a manual enum, manual
  `COUNT`, and order-sensitive table; `LanguageItemId` likewise precedes a
  separate item table. Reuse common catalog validation/index/documentation
  infrastructure without pretending ABI imports or syntax keywords are
  standard-library functions.
  - [x] Make the ABI declaration list generate `AbiImportId`, `ALL`, `COUNT`,
    indices, and the normalized import table together. The public ID API and
    deterministic import order are unchanged, but adding an import can no
    longer omit its ID or require a manually synchronized count.
  - [x] Normalize the heterogeneous language-item, built-in-type, compiler
    symbol, and lifecycle-action declarations behind one authored source, then
    generate their identities and item views without erasing the useful
    `BuiltinType`/`ActionKind` relationships. One grouped declaration macro now
    accepts ordinary syntax, built-in types, compiler-provided snapshot roots,
    and lifecycle actions, then generates `LanguageItemId` and the ordered item
    table together. Payload identities remain typed rather than being flattened
    into strings or pretending syntax is standard-library API.

### 2. Intrinsic and generated-runtime architecture

- [x] Introduce a small trusted intrinsic registry independent of the public
  stdlib declaration source. Each intrinsic contract should own its expected
  signature shape, effects, availability, lowering class (ordinary, retry,
  suspension, host boundary, or representation primitive), scratch needs,
  helper roots, and host-import roots. Validate privileged stdlib bindings
  against it before checking user code.
  - [x] Generate an exhaustive `IntrinsicId::ALL` view and require one closed
    Rust `IntrinsicContract` per ID. Contracts now own callable shape,
    generic/value arity, exact `EffectSet`, availability, and lowering class;
    standard-library validation rejects mismatched or orphaned public
    bindings.
  - [x] Move direct generated-helper and host-import roots into the contracts.
    Backend dependency planning now interprets those roots and no longer
    matches `IntrinsicId`; adding a call implementation cannot silently omit
    its direct runtime dependencies.
  - [x] Move synchronous and suspension scratch policy into the contracts.
    Wasm-IR local planning now interprets typed scratch policies (core,
    expression, or Result payload plus slot count) instead of maintaining two
    more `IntrinsicId` switches.
  - [x] Move complete parameter/result shape into the contracts. Trusted
    signatures now describe generic parameters by ordinal, capability bounds,
    typed-function selection, method receivers, recursive constructor
    applications, every parameter type and literal rule, and result type.
    Catalog validation ignores cosmetic generic names but rejects any
    implementation-relevant mismatch. Transitive helper/ABI effects are also
    checked against the exact public effect set.
- [x] Lower a resolved stdlib call to an explicit backend call operation once.
  Wasm IR now owns `CallTarget` and converts semantic `ResolvedCall` at its
  input boundary; a standard-library target stores both stable item ID and
  trusted intrinsic ID plus concrete type/receiver facts. Codegen no longer
  imports or rematches front-end `ResolvedCall`, dependency analysis consumes
  contract roots, and local planning consumes contract scratch policies. The
  final expression/async emitter dispatch remains deliberately exhaustive
  until emitter strategies move into the contract registry.
- [x] Replace the parallel generated-helper enum/order, transitive dependency
  matches, signature match, and body-emission match with one helper descriptor
  registry and a body plan. Function signature order and code-body order must
  be represented by the same plan rather than reproduced by separate loops.
  - [x] Give every helper one descriptor containing its stable identity,
    deterministic order, symbolic Wasm signature, direct helper dependencies,
    direct ABI imports, and body builder. Intrinsic contracts should refer to
    that same identity rather than maintaining an `IntrinsicHelper` mirror.
  - [x] Build one dependency-closed, ordered `RuntimeHelperPlan` per backend
    compilation. Allocate function indices from that plan and emit bodies by
    iterating the exact same entries, including settings helpers; a missing or
    extra body must become structurally impossible rather than an ordering
    convention between `function_plan.rs`, `runtime_helpers.rs`, and
    `codegen.rs`.
  - [x] Validate the descriptor graph for duplicate identities, dependency
    cycles, forward references that violate body availability, and ABI roots.
    Keep focused tests for the Unity dependency closure and emitted
    signature/body parity. `runtime_helper_registry.rs` is now the only
    canonical helper iteration order and owns symbolic signatures, direct
    helper/ABI roots, and body-builder callbacks. Intrinsic contracts use the
    same `RuntimeHelperId`; dependency closure interprets descriptors; function
    planning resolves their signatures into one ordered `RuntimeHelperPlan`;
    and body emission iterates those exact entries, including settings
    adapters. Registry tests reject missing/duplicate identities, duplicate
    roots, and dependencies ordered after callers, while architecture checks
    reject the former per-phase helper matches.
- [x] Add an explicit linear-memory layout plan. `LinearMemoryLayout` now packs
  typed scratch roles into validated primary/companion alias classes, sizes
  ABI output from the largest readable layout and scan overlap from the actual
  signature set, page-aligns immutable data after scratch, and derives initial
  pages plus growable host-string staging from the complete layout. String and
  signature pools store relative offsets until this plan relocates them, so
  scratch growth cannot invalidate embedded pointers. Stress tests cover
  multi-page static strings, scratch growth from a readable struct larger than
  one page, long planner-level signatures, bounds, and alias placement.
- [x] Complete the collision-safety slice of that plan: reserve the first Wasm
  page for centralized runtime scratch regions, place immutable data after it,
  derive minimum pages from the collected payload, validate wasm32 bounds, and
  stress-test a string large enough to require three pages. Typed scratch
  handles and workload-derived packing are completed below.
- [x] Replace fixed runtime global indices and scratch-address constants with
  typed plans (`RuntimeGlobals`, `LinearMemoryLayout`, and planned scratch
  handles). Emitters should not know that `current` happens to be global 1 or
  that string scratch happens to begin at 32 KiB.
  - [x] Have `global_plan` allocate and return a typed `RuntimeGlobals` value
    for process, current/old snapshots, attach readiness, async frame, and the
    detached-entry latch. Expression, async, settings, and update emitters now
    receive those named roles; the six parent-module numeric constants are
    gone and changing global declaration order cannot silently retarget them.
  - [x] Replace remaining numeric scratch addresses with typed regions from
    `LinearMemoryLayout`, including alignment, size, ownership/lifetime, and
    checked accessors for each helper/settings operation.
    - [x] Centralize the formerly named constants as `ScratchRegion` values for
      settings length/string decoding, signature scanning, C strings, and
      managed UTF-16/UTF-8. Their bounds and required non-overlaps are checked
      when the layout is planned and consumers receive regions explicitly.
    - [x] Place unbounded host String staging at the first page after immutable
      data and grow memory before writes. Long `print`/`setVariable` values can
      no longer overwrite static strings or signatures beginning at page 1.
    - [x] Replace raw address-zero ABI read destinations across process, Unity,
      state, and async emitters with one aligned `AbiReadScratch` role. Every
      read proves its complete output size fits, all decoding uses named bases,
      and the aliasing contract requires callers to materialize values before
      a nested synchronous read. An architecture test scans every process-read
      emitter and rejects anonymous destinations or address-zero loads.
    - [x] Replace the remaining fixed first-page coordinates with a packed
      region planner and explicit primary/companion alias classes. Capacity is
      derived from readable layouts and collected signature lengths; fixed API
      limits such as the existing 255-byte signature diagnostic remain
      source-facing. Settings decoding, scanning, C-string checks, ABI reads,
      and managed UTF input share the primary class only when their synchronous
      phases are mutually exclusive, while settings length/managed UTF output
      occupy the disjoint companion class.
- [x] Centralize Unity/IL2CPP version layouts and discovery signatures in a
  validated domain descriptor. `codegen/unity_layout.rs` now owns the accepted
  version rows, pointer width, versioned field-count/static-table offsets, the
  invariant assembly/image/class/field memory schema, module name, discovery
  scan windows and displacements, and every built-in signature. Static-data
  collection, attachment, synchronous helpers, and async static-table polling
  consume those identities/facts. Descriptor validation rejects duplicate
  versions/signatures, malformed signatures, bad alignment/scalar sizes, and
  inconsistent object strides; an architecture test prevents the former
  repeated literals from escaping back into Wasm emitters. Adding a supported
  64-bit layout is now one validated version-table row rather than a numeric
  hunt through instruction builders.
- [x] Split `codegen/runtime_helpers.rs` by genuine runtime domain after the
  registries/plans exist: strings/UTF conversion, equality, process memory and
  signatures, settings adapters, and Unity metadata. Give each module explicit
  inputs and returned bodies; no `use super::*`. The descriptor callback layer
  is now a 145-line orchestrator. String/formatting, structural equality,
  process-memory/signature/managed-string, Unity metadata, and Unity attachment
  implementations live in explicit-import modules, all below 1,000 lines;
  settings remains in its existing dedicated adapter module. This split was
  performed only after `RuntimeHelperPlan` made function identity, dependencies,
  signatures, and body order an enforced interface.
- [x] Remove all codegen `use super::*` imports. Move shared backend types into
  deliberately named modules and make each encoder depend only on its plan and
  narrow emission context. The current physical file split does not enforce
  the architectural boundaries described in `docs/COMPILER.md`.
  - [x] Remove every production wildcard parent import and make each codegen
    module state its crate, sibling, and parent dependencies explicitly. This
    also removed the accidental `codegen.rs` prelude: symbols no longer need to
    be imported by the root merely so children inherit them.
  - [x] Give shared backend concepts deliberate owners. `backend_type.rs` owns
    physical Wasm value categories; `gc_layout.rs` owns deterministic GC index
    assignment; `equality_plan.rs` owns equality function indices;
    `runtime_helper_registry.rs` owns `RuntimeHelperPlan`; `async_frame.rs`
    owns suspension-frame storage; expression lowering owns match-local
    storage; `global_plan.rs` owns setting globals; and `context.rs` owns the
    immutable post-plan emission and attachment contexts. `codegen.rs` is now
    an orchestrator rather than a miscellaneous backend type warehouse.
  - [x] Replace uses of the complete `EmissionContext` where an encoder only
    needs a smaller plan-specific view. Settings and per-tick update now expose
    `SettingsContext` and `UpdateContext`; runtime-helper construction receives
    only semantic facts, settings inputs, its helper plan, data pools, GC plan,
    and linear-memory plan. Script-body, expression, and async emission retain
    the complete context because they genuinely span locals, globals, calls,
    equality, process memory, state snapshots, and suspension storage.

### 3. Front-end and semantic stage boundaries

- [x] Make parsing purely syntactic. `parser.rs` currently pre-scans top-level
  declarations, allocates semantic declaration/layout IDs, knows the standard
  library, resolves struct/enum constructors, and emits some redeclaration
  diagnostics (sometimes with a default span). Move declaration collection,
  nominal lookup, reserved-name checks, and constructor resolution into a real
  resolution/lowering stage; syntax type paths should retain source names.
  - [x] Establish an enforceable diagnostic boundary: parser recovery now
    contains grammar diagnostics only, while post-syntax declaration
    validation owns core/standard/duplicate nominal conflicts, precise name
    spans, and secondary labels. Parsed, lowered, recovered, and database
    products retain those resolution diagnostics independently; formatting
    remains available for syntactically valid but semantically conflicting
    source, and strict checking reports the conflict. Struct/enum declaration
    identities are assigned while parsing rather than borrowed from the token
    pre-scan. Source type annotations now retain arbitrary nominal names and
    their source spans; unknown-name diagnostics belong to resolution, and
    recovering checking uses a total error placeholder instead of relying on a
    parser-enforced `unreachable!`. Struct literals now retain their nominal
    spelling/span in syntax and publish the resolved `StructId` through the
    semantic model; parsing recognizes their `field:` grammar shape without a
    declaration lookup.
  - [x] Remove the parser's declaration/catalog pre-scan and enum
    classification completely. Parsed enum match/settings references retain an
    `EnumReference::Named` spelling/span, and ordinary two-segment paths/calls
    remain ordinary syntax. `resolution::resolve_program`, invoked by `lower`,
    resolves source and catalog enums, rewrites enum constructors, validates
    choice-setting restrictions/constructor arity, and publishes resolved
    references. Typed HIR owns a separate `TypedPattern` with concrete
    `EnumTypeId`s, so unresolved syntax cannot leak into Wasm lowering. Source
    struct, enum, and constructed-layout IDs now start in their own typed
    identity spaces without a token-count offset scan. The final substep below
    records why parser-owned constructed IDs remain syntax rather than layout.
  - [x] Classify parser-assigned declaration, expression, binding, and
    constructed-type IDs explicitly as stable syntax identities. Array,
    Option, and Result tables intern source type-expression structure only;
    they do not allocate inferred semantic types, memory layouts, or Wasm GC
    types. Resolution may replace nominal references in the lowered copy, and
    checking/backend stages retain independent identities. This keeps visitors,
    diagnostics, and editor queries stable without moving syntax-node identity
    into a later semantic pass.
- [x] Replace the minimal declaration-only HIR with a clear resolution product
  or remove that stage. Today `lower` mostly indexes declaration names while
  `typeck` repeats declaration-environment construction and performs most name
  resolution, making the advertised `syntax -> resolved HIR -> typed HIR`
  pipeline misleading.
  `LoweredProgram` is now the explicit resolution product: it owns syntax with
  nominal type names, enum constructors/patterns/settings, and struct literal
  identities resolved to stable source/catalog IDs, plus resolution
  diagnostics. The former ambiguously named `hir::Program` is now the
  deliberately narrow `hir::DeclarationIndex`; it supports pre-check tooling
  but no longer pretends to be a resolved body IR. Type checking consumes the
  resolved syntax, does not rebuild the nominal type-name environment, and
  publishes a distinct typed body HIR with resolved calls, members, patterns,
  conversions, and types. Type-directed call/member/binding inference remains
  in the checker by design rather than being mislabeled as syntactic lowering.
- [x] Split `typeck.rs` around explicit products: declaration/signature
  collection, body checking, expression constraints, call/member resolution,
  exhaustiveness/control-flow, and finalization. Replace the large `Checker`
  state machine's interacting booleans (`in_function`, `checking_suspension`,
  `checking_state_source`, `allowing_null`, and others) with scoped context
  enums/guards so invalid mode combinations are unrepresentable.
  - [x] Extract the first declaration product from the flat checker state.
    `typeck/declarations.rs` now owns source nominal declarations, named-type
    bindings, state/settings/global bindings, user function/method signatures,
    and debug-callable identity as one `DeclarationEnvironment`; lexical scopes
    and transient body modes remain separate.
  - [x] Replace the mutually stale `in_function`, `current_action`, and
    `current_callable` fields with one `CallableContext`, and replace the
    independently combinable `checking_suspension` / `checking_state_source`
    flags with one `ExpressionMode`. Replace the independent optional failure
    boundary and `used_propagation` flag with `FailureContext`, so propagation
    use cannot exist without the result boundary it targets. `NonePolicy` now
    names the distinction between ordinary optional construction and the two
    domain-nullable action results instead of carrying an unexplained
    `allowing_null` flag.
  - [x] Make all transient checker modes scoped. Debug-only code, loop depth,
    suspension/state-expression mode, nullable-return policy, failure
    propagation, and callable/return contexts now enter through restoring
    helpers; checking one field or callable cannot leak state into the next.
    `DebugContext` and `LoopContext` replace the remaining boolean/counter
    conventions, while `FailureContext` returns propagation evidence from its
    exact boundary.
  - [x] Extract real passes rather than merely distributing one state machine.
    A 95-line driver initializes the checker and sequences
    `declaration_pass`, `body_pass`, and `finalization`; declaration and
    signature collection, global/state/function/action bodies, statements and
    lexical scopes, expression constraints, call/member resolution,
    syntax-level control-flow facts, and semantic publication each have a
    named owner. The audit-time 3,055-line root is now 576 lines and no
    type-checking module exceeds 1,000 lines. The complete 219-test Rust suite
    and warnings-denied Clippy pass across this boundary.
- [x] Move post-type-check semantic validation out of `lib.rs::check` into a
  named validation stage that owns effect, detached-call, equality, memory,
  and generic-capability diagnostics. Strict and recovering checks should
  share the same stage boundaries and publish the same available facts.
  `validation.rs` now returns one `ValidationOutput` containing derived
  capabilities, operation effects, and diagnostics. Strict checking consumes
  it before publishing `CheckedProgram`; recovering checking runs the same
  stage whenever typed HIR is available, retains effects even when validation
  rejects the program, and reports the same detached/capability diagnostics.
- [x] Reconcile the repeated type universes deliberately. Core primitives are
  manually repeated in `CoreTypeId`, syntax `TypeRef`, `BuiltinType`, and the
  backend physical `Type`; constructed types are converted through several
  parallel matches. Generate mechanical primitive mappings from one core
  declaration and document the genuinely necessary syntax/inference/semantic/
  physical distinctions. `with_core_types!` now authors each magical core
  primitive once and generates `CoreTypeId`, ordered metadata (canonical name,
  capabilities, and memory layout), and the backend's deliberately physical
  variants/conversion. Syntax stores `TypeRef::Core(CoreTypeId)` rather than
  repeating thirteen variants; semantic `BuiltinType` is a descriptive alias
  of that same ID rather than another enum. Constructed syntax, semantic, and
  physical types remain distinct because they carry stage-specific layout and
  representation facts.
- [x] Reassess expression duplication across syntax `ExprKind`, typed
  `TypedExpressionKind`, and Wasm `ExpressionKind`. Typed HIR should retain
  semantic source shape, but the backend IR should contain completed backend
  operations rather than a third mostly source-shaped copy. Add a Wasm-IR
  visitor/folder so reachability, dependencies, data collection, and local
  planning do not each grow another recursive switch for every expression.
  The lowered IR remains a stable-ID expression DAG because it owns backend
  call targets, conversions, failure boundaries, and match layouts that syntax
  cannot represent. `wasm_ir::Visitor` now owns recursive program/block/
  statement/terminator/expression traversal, while
  `visit_expression_children` is the one exhaustive direct-edge definition
  for worklist analyses. Reachability consumes that edge query; local and
  intrinsic-scratch planning and suspension-frame liveness consume the lowered
  visitor rather than reopening typed HIR. Declaration stores are explicit IR
  facts, so storage planning no longer guesses from an assignment-shaped node.
  Dependency and static-data planning deliberately scan the flat reachable
  expression table and match only payloads they consume; they do not duplicate
  recursive child traversal.
- [x] Make `lower_wasm` produce the complete backend input. It now returns a
  `codegen::BackendProgram` that owns the profile-specific Wasm IR and borrows
  the matching syntax, semantic model, constructed-type layouts,
  memory layouts, equality analysis, and standard-library identity. The
  product dereferences to its Wasm IR for staged inspection, while binary
  encoding accepts only the complete product, preventing callers from mixing
  unrelated earlier-stage results. Once global constants, local/scratch
  planning, and frame liveness migrated to Wasm IR, the backend product also
  stopped retaining typed HIR merely as an escape hatch.
- [x] Replace the code generator's eight positional entry arguments with a
  named product boundary. The temporary `codegen::Inputs` migration seam was
  superseded by `BackendProgram`; new inputs cannot be silently reordered or
  independently threaded into encoding.
- [x] Reduce cloning and duplicated recovery paths. `CompilerDatabase` rebuilds
  products by cloning syntax and repeats “strict check or recovering check” in
  many queries. Publish a shared semantic snapshot/view that exposes whichever
  facts survived, while keeping strict compilation incapable of consuming
  recovery placeholders.
  `SemanticSnapshot` now owns the strict/recovered choice once and provides
  shared syntax, source-document, semantic-model, enum, context, effect, and
  optional typed-HIR views. Position analysis, navigation, highlighting,
  hover, signature help, and definition indexing consume that product. Hover
  retains the shared snapshot rather than cloning whole syntax and semantic
  models; strict compilation and the typed reference index still require a
  checked program deliberately.

### 4. Tooling, tests, and repository-scale maintenance

- [x] Split `database.rs` into the revision/query cache, semantic snapshot
  access, definition/reference indexing, and rename validation. Completion,
  insight, highlighting, and symbols should consume stable query interfaces
  rather than reach through the database into stage internals.
  `database/queries.rs` owns source revisions and stage/query orchestration,
  `cache.rs` owns one invalidated-per-revision cache product, `snapshot.rs`
  owns the editor-safe semantic view, `position.rs` owns cursor analysis,
  `references.rs` owns typed value references, and `rename.rs` owns
  identity-preserving edits. The remaining 932-line database root owns the
  stable source-definition index and resolution mapping; no database module
  exceeds the soft 1,000-line threshold, and production modules use explicit
  imports rather than parent glob coupling.
- [x] Replace the monolithic raw-`serde_json::Value` LSP handler with typed
  protocol DTOs (prefer the maintained `lsp-types`/`lsp-server` ecosystem if
  its cost is acceptable), a request router, document store, and conversion
  modules. Malformed parameters should produce consistent protocol errors
  rather than ad hoc `None` paths. Keep transport framing separate.
  The transport remains isolated in `bin/splitls.rs`; `lsp/protocol.rs`
  deserializes the JSON-RPC envelope and every supported incoming parameter
  shape, the root routes named methods and lifecycle state,
  `lsp/documents.rs` owns open buffers and their compiler databases, and
  `lsp/conversion.rs` owns byte/UTF-16 and compiler-product serialization.
  Request decoding consistently returns `-32600` for malformed envelopes and
  `-32602` for malformed method parameters. The production root fell from
  1,645 audit-time lines (including tests) to 574 lines; its 750-line protocol
  suite now has a dedicated module. A direct `serde` DTO layer was sufficient
  for the current small method surface, so adopting a larger protocol crate is
  deferred until it removes more code than it adds.
- [x] Split the VS Code extension into language-client discovery/lifecycle and
  compiler build/watch task management. Model build/watch state with one task
  controller instead of module-level booleans and process handles, and add
  extension tests for completion of builds, watcher exit races, and disposal.
  `languageClient.ts` now owns server discovery plus start/stop/restart,
  `compilerTasks.ts` owns release/watch UX and processes, and `extension.ts` is
  activation wiring only. One discriminated compiler-task controller replaces
  the independent release boolean, watcher handle, and status globals.
  `ExclusiveTaskState` uses task identity so a delayed close/completion event
  cannot clear a newer owner; Node tests cover exclusion, stale watcher events,
  and idempotent completion, while TypeScript strict checking covers controller
  disposal and command wiring.
- [x] Split `tests/compiler.rs` by stable subsystem (parsing/recovery,
  inference/checking, catalogs/tooling, lowering/codegen) and extract shared
  source/Wasm assertions. Keep cross-stage tests, but make failures point to
  one architectural area.
  The 7,805-line integration-test root is now a 47-line shared-fixture/module
  index. Ten named modules own compiler queries/navigation, parser recovery,
  catalogs/types, migration diagnostics, failure semantics, profiles/codegen,
  expressions/control flow, inference/language, async/runtime behavior, and
  snapshot rendering. Shared fixtures remain defined once, the complete 138
  tests retain one integration binary, and failure names now include their
  subsystem module.
- [x] Add one repository verification command (`cargo xtask check`, `just
  check`, or equivalent) that runs formatting, Clippy with warnings denied,
  all Rust tests, VS Code TypeScript checks, both release examples, Wasm
  validation, and every Node runtime harness. The Node harnesses are currently
  documented/manual and there is no CI configuration, so ordinary `cargo test`
  does not protect runtime behavior.
  `cargo xtask check` now owns that exact matrix. Its runner uses an isolated
  target directory so Windows can execute nested Cargo commands without trying
  to replace the running `xtask.exe`; generated modules live only under the
  ignored `target/verify`. Lunistice is compiled once as a publishable release
  artifact and once in debug for the harness that deliberately asserts debug
  attachment messages. The complete command passes locally, including base and
  DLC Lunistice host simulations and every Option/Result/async/settings/profile
  runtime fixture.
- [x] Add CI using that exact verification command and cache only disposable
  build outputs. Never commit generated `.wasm`/`.wat` files.
  `.github/workflows/check.yml` runs the same `cargo xtask check` entry point on
  Windows, caches only npm's disposable download data, and rejects tracked
  `.wasm`/`.wat` artifacts before verification. Toolchain setup is explicit;
  the workflow does not maintain a second hand-written test matrix.
- [x] Keep compile-time, warm-query, generated-Wasm-size, and LSP latency
  baselines. The one-shot runner records compiler/Wasm size, while the tooling
  runner generates a 500-function source and measures cached database and
  in-process JSON-RPC queries. Large-catalog scaling remains paired with the
  alternate-graph ownership task in the active roadmap.
- [x] Narrow the crate's public surface after internal interfaces stabilize.
  `compiler` and `tooling` facades now classify the public products; root
  implementation modules are private and integration tests consume the same
  facades. Keep one crate until two real consumers need independently versioned
  APIs.
- [x] Split this roadmap into a short active `TODO.md` and this archived design/
  completion history. The former 1,800-line roadmap is preserved here while
  the root file contains only active and deliberately deferred work.
- [x] Establish a soft 1,000-line module review threshold and responsibility
  checks, not a hard mechanical limit. The audit-time priorities now have
  named boundaries: the parser root is 410 lines with declarations,
  statements, expressions, types, and recovery modules; the checker root is
  576 lines with explicit passes; runtime helpers are split by host domain;
  the database root is 932 lines with cache/snapshot/navigation/query modules;
  the LSP root is 574 lines with protocol/document/conversion modules; and the
  compiler integration-test root is a 47-line subsystem index. A file crossing
  the threshold prompts an ownership review, but cohesive tables/visitors and
  test suites are not split solely to satisfy a line counter. Remaining large
  modules are listed in `docs/COMPILER.md` as candidates for the next
  interface-led change rather than treated as an emergency mechanical pass.

### Audit baseline and immediate repair

- [x] Inventory module/file sizes, catalog construction, dependency cycles,
  standard-library and intrinsic fan-out, stage inputs, test entry points, and
  the current runtime/compiler verification surface.
- [x] Restore a warning-free `cargo clippy --all-targets -- -D warnings`; the
  dependency/toolchain update currently reports an unnecessary lazy boolean
  closure in `database.rs`.
- [x] Begin with the validated catalog graph/compiler-context seam, while
  fixing the linear-memory overlap as an independent correctness slice. These
  two changes unblock the future stdlib loader and make backend refactoring
  safe; large-file decomposition follows their interfaces.

## P0 — Unify the standard-library declaration and type model

Do this before any further substantial standard-library expansion. This work
turns `StandardLibrary` from the former callable-only catalog into the source
of truth for namespaces, nominal types, fields, enum variants, capabilities,
and runtime representations. The remaining unchecked items finish unifying
source type resolution, inference, and derived capabilities around that graph.

Preserve the current language and generated behavior while establishing this
invariant:

> Adding an ordinary standard-library nominal type requires one declaration.
> Intrinsic behavior may additionally require one deliberately scoped
> implementation, but must not require parser, inference, checker,
> documentation, LSP, or physical-layout declarations.

### Hierarchical standard-library authoring model

The compiler and tooling now consume one normalized symbol graph, but the Rust
source that creates that graph is still fragmented: types, namespaces, fields,
variants, callable items, owner links, qualified names, and intrinsic IDs live
in parallel blocks. The architectural goal is not complete until the authoring
model mirrors the API hierarchy as cleanly as the consumer model does.

- [x] As the immediate migration, replace the parallel `declare_standard_types!`,
  `declare_standard_namespaces!`, `declare_standard_fields!`,
  `declare_standard_variants!`, and `declare_standard_items!` inputs with one
  hierarchical declarative Rust macro. This is an authoring adapter for the
  normalized symbol graph, not the intended permanent source format.
- [x] Make each nominal type declaration contain its documentation,
  capabilities, value-usage policy, runtime representation, public and
  runtime-private fields, enum variants, associated functions, and instance
  methods. Opening `Module`, `Duration`, or `UnityClass` must reveal its whole
  public and physical API in one place.
- [x] Give root functions, namespaces, nested namespaces, core-type extension
  methods, capabilities, and type constructors equally explicit owner blocks.
  `process.read`, numeric methods, address methods, and array methods must not
  remain a flat exceptional list.
- [x] Derive owners and qualified names from declaration nesting. A member
  declaration must not repeat `StdlibOwner::Type(...)`, `Module.scan`, and
  `Module` independently.
- [x] Generate `StdlibNamespaceId`, `StdlibTypeId`, `StdlibFieldId`,
  `StdlibVariantId`, `StdlibItemId`, `IntrinsicId`, and the flattened
  `NAMESPACES`/`TYPES`/`FIELDS`/`VARIANTS`/`ITEMS` tables from the hierarchical
  source. Generated flat tables remain an internal compatibility layer for
  generic consumers, not an authoring surface.
- [x] Keep intrinsic implementation bodies deliberately separate, but bind
  their generated intrinsic key alongside the owning function or method.
  Validation must prove that every declared intrinsic has exactly one backend
  implementation and that no implementation is orphaned.
- [x] Migrate every existing declaration, including the test-only ordinary
  catalog struct, then delete the retired macros, duplicated owner/name data,
  and manual intrinsic-ID list.
- [x] Add architecture tests showing that representative types with fields,
  variants, associated functions, and methods are declared in owner blocks yet
  remain resolvable, documentable, completable, and code-generatable through
  the normalized graph.
- [ ] Long term, define the standard library in SplitScript source and make its
  loader produce the same normalized symbol graph as the interim Rust macro.
  Add the prerequisite language features deliberately: modules/namespaces,
  generic declarations and capability bounds, declaration-only intrinsic and
  host functions, effect metadata, private runtime fields, attached
  documentation, and ordinary reusable library bodies.
  Organize those bundled sources as domain modules (`core`, process memory,
  timer/runtime, Unity, and future engines) rather than recreating the interim
  single-file token stream, which is already close to 1,000 declarative lines.
- [ ] Compile bundled standard-library sources in an explicit privileged mode,
  never as ordinary project code. Only that mode may declare intrinsic or host
  bindings, representation hooks, runtime-private fields, trusted effects, and
  other low-level implementation details. User files must be unable to enable
  the mode, import its private surface, shadow the bundled library, or call raw
  intrinsic entry points.
- [ ] Keep a small Rust intrinsic/host registry as the trust boundary. Loading
  the SplitScript standard library must resolve every privileged declaration
  against that registry and verify its signature, effects, availability,
  suspension/cancellation behavior, and representation contract. Reject
  unknown, duplicate, orphaned, or understated bindings before compiling user
  code.
- [ ] Once the SplitScript source loader covers the complete library, delete
  the interim Rust declaration macro. Rust should retain only core primitive
  definitions, backend-neutral representation primitives, the host ABI
  catalog, and deliberately scoped intrinsic lowering implementations.

### Library declaration graph

- [x] Extend `StandardLibrary` from a callable catalog into a complete,
  backend-independent symbol graph with stable IDs for namespaces, nominal
  types, fields, runtime-private slots, enum variants, callables, capabilities,
  and intrinsic implementations.
- [x] Give every callable an explicit owner—root, namespace, nominal type, or
  capability—instead of deriving namespaces from string path prefixes.
  Declare `process`, `timer`, and `Unity` as namespaces; keep `Duration` and
  the Unity value types as nominal types with associated or instance members.
- [x] Replace catalog `Builtin(BuiltinType)` and `Named("...")` references with
  stable core-type, standard-type, type-parameter, and constructed-type
  identities. Names are lookup/display data, never semantic identity.
- [x] Describe public fields and runtime-private storage in the owning type
  declaration. Runtime metadata must be backend-neutral: scalar, GC struct,
  GC array, enum, compile-time-only, and derived struct representations rather
  than `wasm_encoder` types or numeric heap/field indices.
- [x] Generate namespace, standard type, field, variant, and callable IDs
  together with their declaration rows, so adding an ordinary symbol cannot
  leave a parallel ID enum or inverse owner list out of sync.
- [x] Add catalog validation for unique IDs and names, resolvable owners and
  type references, valid representation dependencies, field/variant identity,
  complete public documentation, capability consistency, and intrinsic
  signature/implementation agreement.

### One semantic type universe

- [x] Restrict compiler-core types to genuine language primitives and
  constructors: `None`, `bool`, fixed-width numbers, `address`, inference
  variables, arrays, `T?`, and `T!`. Model `String`, `Duration`, `Module`,
  `TimerState`, and the Unity family as declared nominal library types;
  represent `Signature` as a single declared compile-time intrinsic type.
- [x] Preserve unresolved nominal type paths in syntax and resolve them against
  one environment containing core, standard-library, and source declarations.
  The parser must not enumerate future standard-library type names.
  - [x] Keep source-written standard-library type names nominal in syntax and
    resolve them to catalog identities when entering inference/semantics.
  - [x] Apply the same name-resolution boundary to source struct/enum names:
    source annotations now carry an interned nominal-name identity and resolve
    alongside catalog names only when entering semantics.
  - [x] Consolidate constructor, enum-pattern, choice-setting, and nominal-type
    lookup into one declaration environment rather than parallel parser maps.
- [x] Simplify inference to known semantic `TypeId` values plus inference
  variables and the minimum temporary constructor terms needed while a
  constructed type's element/value remains unresolved. Remove the parallel
  nominal variants and conversion tables in `ast::TypeRef`, `inference::Type`,
  and `BuiltinType` as their migrations complete.
  - [x] Build one semantic `TypeStore` before inference and represent every
    standard-library nominal inference case as its canonical `Type::Known(TypeId)`.
    Checked publication preserves that identity directly; no library type has
    a dedicated inference or semantic variant or a post-inference conversion
    table.
  - [x] Represent source struct and enum inference types with the same known
    semantic `TypeId` values used by checked programs; nominal inference no
    longer has separate standard/source variants.
  - [x] Represent core primitives with their canonical semantic `TypeId` as
    well; inference no longer has parallel primitive or nominal variants.
  - [x] Intern resolved array, Option, and Result terms during inference while
    retaining only the minimum temporary constructor terms needed while their
    element/value types are still unresolved.
- [x] Introduce well-known type/variant handles for genuine language and ABI
  contracts such as string literals, interpolation, `gameTime`, signature
  literals, and timer-state conversion. A well-known handle references the
  catalog declaration; it does not redeclare its name, fields, variants,
  capabilities, nullability, or representation.
- [x] Generalize equality, process-memory layout, interpolation/string
  conversion, and future traits into capability queries over semantic
  `TypeId`. Derive struct/enum capabilities from their members where
  appropriate rather than matching concrete library types.
  - [x] Declare core and standard-type capabilities once in the catalog and
    make inference constraints, callable applicability, casts,
    interpolation, memory-read eligibility, and equality query them instead
    of maintaining concrete-type lists.
  - [x] Treat inference checks for source structs, enums, and wrappers as
    conservative admissibility only, then prove them through one recursive
    semantic-`TypeId` capability query. Preserve precise equality failures and
    process-memory layouts as capability evidence for diagnostics and backend
    planning; validate catalog type-parameter constraints generically.

### Generic members, layouts, and tooling

- [x] Resolve standard and source fields through one declaration query and one
  stable member identity. Remove `BuiltinFieldId` and the type checker’s
  `Module`/Unity field-name tables.
- [x] Make completion, hover, signature help, go-to-definition, semantic
  highlighting, rename reservation, and generated documentation traverse the
  same symbol graph. Remove standard types and fields from `LanguageCatalog`;
  that catalog should contain only keywords, syntax, lifecycle/actions, and
  other genuinely language-defined concepts.
- [x] Plan reachable Wasm GC layouts from semantic type declarations. Backend
  code must query type and field layout IDs rather than use fixed constants
  such as `UNITY_IMAGE_TYPE` or numeric field indices.
- [x] Make intrinsic lowering request its actual temporary values and query
  declared representations. Remove unconditional Unity scratch locals and
  helper signatures that reconstruct library types independently.
- [x] Keep exact-type branching only inside behavior that is intrinsically
  type-specific. Such code may reference stable standard type/field/variant
  IDs, but it must not restate their source names, semantic shapes, storage
  layout, or documentation.

### Vertical migration and removal order

- [x] Establish catalog declarations and adapters without changing source
  syntax or runtime behavior; add characterization tests for compiler queries,
  docs/LSP results, GC layouts, and generated Wasm before deleting old paths.
- [x] Migrate explicit `process`, `timer`, and `Unity` namespaces and switch all
  editor/documentation discovery away from inferred callable path prefixes and
  hard-coded namespace lists.
- [x] Migrate `TimerState` first as the enum/variant proving case. Remove its
  synthetic AST enum, checker injection, name-based backend lookup, and
  duplicate `LanguageCatalog` entries.
- [x] Migrate `Module` as the field and nominal-GC-struct proving case. Its
  `address` and `size` members and physical slots must come from one
  declaration.
- [x] Migrate `UnityModule`, `UnityImage`, `UnityClass`, and `UnityField`
  together, including public fields, runtime-private ownership references,
  methods, async temporaries, generated helpers, and GC layout planning.
- [x] Migrate `Duration`, then `String`, then `Signature`, accounting for their
  lifecycle/ABI contracts, non-nullability, literal/interpolation behavior,
  array representation, and compile-time-only behavior without duplicating
  their declarations.
- [x] Delete the retired standard-type variants, conversion matches, fixed GC
  indices, numeric field indices, special member tables, and duplicate
  language/tooling documentation after each vertical slice. Do not retain
  compatibility aliases; the language is not yet in production.
- [x] Finish with full compiler, formatter, generated-documentation, LSP,
  extension, example-autosplitter, Wasm validation, and runtime regression
  coverage. Add a test proving that a new ordinary catalog struct with fields
  becomes resolvable, documentable, completable, and code-generatable without
  adding a concrete-type match elsewhere.
  - [x] A test-only `CatalogStructProbe` declaration exercises nominal
    resolution, semantic `TypeId` identity, public fields, go-to-definition,
    hover documentation, completion, derived process-memory layout,
    structural equality helpers, and valid Wasm GC generation. Its ID is not
    referenced by production compiler or tooling code.
  - [x] Re-run the full Rust suite, formatter check, VS Code TypeScript check,
    release compilation of both example autosplitters, and Wasm validation.
    The existing Node 24 Lunistice harness still reaches its previously
    characterized null-dereference in the unchanged attachment runtime; the
    generated module validates and this refactor does not alter that runtime
    path.

## Completed foundation — Conditional state expressions

### Expression-valued `if`

- [x] Make `if` an expression with type inference across its branches.
- [x] Evaluate only the selected branch. This is essential for memory reads:
  an inactive branch must not touch the target process.
- [x] Use ordinary expression syntax in `state` rather than inventing a
  state-only conditional around individual fields.
- [x] Allow a state field to combine edition-specific representations into one
  enum value. The Lunistice state should become conceptually:

  ```text
  levelOrScene = if isDlcDemo {
      LevelOrScene.Scene(process.readManagedString(
          process.read(gameManagerInstance.offset(levelOrSceneOffset)),
          128
      ))
  } else {
      LevelOrScene.Level(process.read(
          gameManagerInstance.offset(levelOrSceneOffset)
      ))
  }
  ```

  This fixes the current bug where both `level` and `currentScene` are read from
  the same union-like location even though only one representation is valid.
  Implemented in [`examples/lunistice.split`](examples/lunistice.split); its
  runtime test verifies a 4-byte base-game read versus an 8-byte DLC pointer
  read at the shared location, never both.

## P0 — Compiler and standard-library architecture checkpoint

Do this checkpoint before adding `Option`, `Result`, generic reads, traits, or
another substantial batch of standard-library APIs. The original prototype
was useful for discovering the language, but did not scale: its parsed AST was
mutated in place with inferred types, calls remained string paths, type
checking and code generation resolved built-ins independently, and
[`src/codegen.rs`](src/codegen.rs) owns ABI declarations, runtime generation,
async lowering, layout, helper selection, and expression emission in one file.
`min`, `max`, and `clamp` make the problem especially visible: their names,
typing rules, temporary-local requirements, and lowering are manually
recognized in several unrelated functions.

This is an architectural refactor, not a language redesign. Preserve the
current source syntax and generated behavior with regression tests while
introducing the following boundaries.

### Staged compiler pipeline and semantic model

- [x] Replace the single mutable AST pipeline with explicit products:
  `syntax AST -> resolved HIR -> typed HIR -> lowered Wasm IR -> WebAssembly`.
  Syntax nodes should describe what the user wrote; inferred types, resolved
  symbols, coercions, and selected calls belong in semantic IR.

  - [x] Add an inspectable declaration-level HIR at `lower`, keep syntax
    immutable during type checking, and return inferred backend layouts in the
    checked product instead of appending them to the parsed AST.
  - [x] Materialize checked statement and expression resolution into typed body
    HIR nodes. Paths, calls, members, constructors, assignments, patterns, and
    choice settings carry their stable semantic targets directly; resolutions
    that depend on inference are intentionally attached after checking.
  - [x] Publish typed HIR with `TypeId` results and explicit coercions, then
    make the backend consume it instead of syntax plus semantic side tables.

    - [x] Give every checked expression a stable-ID-ordered typed HIR node with
      its `TypeId`, span, and optional type-directed resolution. Migrate backend
      path, assignment, constructor, and match-pattern identity lookup to it.
    - [x] Move expression/statement shape and child ownership into typed HIR,
      including match arms and async statements. Add typed-HIR traversal and
      migrate backend string/signature discovery to it as the first structural
      consumers.
    - [x] Represent interpolation-to-string conversion explicitly on the typed
      operand edge, migrate ordinary statement/expression emission and async
      lowering together, and remove backend syntax walks and expression
      resolution side-table lookups from user bodies.
- [x] Give declarations, types, standard-library items, and call targets stable
  IDs. A typed call must identify a user function/method or a standard-library
  item directly; the backend must never resolve `Vec<String>` paths again.

  - [x] Assign stable per-program `ExprId` values during parsing and key
    resolved standard-library calls by expression identity rather than source
    spans.
  - [x] Assign stable `FunctionId` values to user functions and methods, resolve
    every user call to its callable ID, and make the backend dispatch through a
    single ID-indexed Wasm function table rather than function/method names.
  - [x] Assign stable `ValueId` values to globals, parameters, locals, await
    bindings, state fields, and settings. Expose ordinary and snapshot-aware
    path roots for go-to-definition and use ID-keyed local, async-frame, state,
    settings, and global storage for backend reads.
  - [x] Assign stable `AssignmentId` values, publish assignment targets as
    `ValueId`, and make local, async-frame, and global writes consume semantic
    targets rather than resolving names again in the backend.
  - [x] Assign `ValueId` values to match payload bindings, include the resolved
    receiver root and semantic receiver type in method-call facts, and lower
    receivers directly through ID-keyed storage without backend name maps.
  - [x] Assign typed IDs to structs, enums, struct fields, enum variants, and
    built-in fields. Publish semantic member chains for paths and method
    receivers, and make struct literals and enum constructors expose their
    resolved field/variant IDs to the backend.
  - [x] Assign `PatternId` and `SettingChoiceOptionId` values, publish resolved
    enum variants for match arms and choice defaults/options, and remove their
    backend variant-name lookup.
- [x] Separate syntactic type references, private inference types, semantic
  `TypeId` values, and backend physical value categories. Constructed semantic
  types retain the stable declaration/layout identities needed by tooling and
  Wasm GC lowering.

  - [x] Introduce the inference-free `TypeId` / `TypeKind` interner used by
    checked semantic call resolutions and editor-facing queries. Move catalog
    built-ins and typed-path arguments to the independent `BuiltinType` model.
  - [x] Store every resolved expression type as a semantic `TypeId`, expose it
    by `ExprId`, and make the Wasm backend consume the semantic facts.
  - [x] Keep omitted global/local/await/state/parameter/function-result
    annotations absent in checked syntax. Publish every inferred declaration
    type by `ValueId` (and each function result by `FunctionId`) in the semantic
    model, and make the Wasm backend consume those facts.
  - [x] Publish struct-field, enum-payload, and array-element layouts as
    ID-keyed semantic `TypeId` facts, give arrays a dedicated `ArrayTypeId`, and
    make the Wasm backend consume those facts instead of AST layout types.
  - [x] Introduce inference-free `ast::TypeRef` values for every source
    annotation, cast target, and integer suffix. Keep parser-owned array type
    references separate from checker-owned inferred array layouts.
  - [x] Remove the temporary `Expr::ty` slot and synthetic-expression escape
    hatch. Record pending expression types directly by `ExprId` during checking
    and resolve them into the semantic model without mutating or revisiting the
    syntax tree.
  - [x] Move temporary types and inference variables completely out of `ast`.
    A dedicated inference context now owns union/find unification, requirement
    composition, literal bounds, defaulting, and inferred array layouts.
  - [x] Remove `TypeStore`'s parallel legacy representation. Wasm storage/value
    selection now lowers semantic `TypeId` / `TypeKind` values into
    backend-local physical categories without importing inference types or
    reading source annotations.
- [x] Keep bidirectional inference and expected-type propagation, but express
  them through a reusable constraint/unification layer. Standard-library
  overloads and methods must submit the same constraints as ordinary language
  constructs instead of having one-off branches in `Checker::call`.

  - [x] Extract the reusable inference context and make ordinary expressions,
    declarations, and standard-library generic constraints share it.
  - [x] Replace the remaining procedural overload/method selection branches
    with declarative candidates that submit constraints to the same context.
  - [x] Defer struct-member resolution when a receiver is still an inference
    variable, then solve it from later call sites or a unique combination of
    accessed fields. `fn levelTimeText(parts) { parts.minutes }` now infers
    `LevelTimeParts` from `levelTimeText(current.levelTimeParts)` without an
    annotation; genuinely ambiguous shared field names receive a focused
    diagnostic instead of an arbitrary nominal-type guess.
- [x] Add shared AST visitor and folder utilities. String/signature collection,
  await discovery, local collection, and later tooling passes should not each
  need another exhaustive recursive walker whenever a new expression kind is
  added. Give resolved/typed HIR sibling traversal utilities when those IRs are
  introduced rather than pretending the syntax visitor can traverse them.
- [x] Expose the stages through a compiler facade (`parse`, `lower`, `check`,
  and `codegen`) while retaining `compile` as the convenient one-shot API.
  Stage results must be inspectable without invoking the WebAssembly backend.

### One standard-library catalog

- [x] Replace name-based `Builtin::resolve` and method-name special cases with
  a declarative, backend-independent `StandardLibrary` catalog. Each item needs
  one canonical struct containing:

  - a stable symbol ID and qualified name;
  - item kind, receiver (for methods), parameters, return type scheme, generic
    variables, and capability constraints;
  - availability and effects such as pure, process-reading, suspending,
    lifecycle-restricted, or debug-only;
  - summary, full documentation, parameter documentation, examples,
    deprecation/migration information, and links to related items;
  - an implementation key: ordinary SplitScript body, compiler intrinsic, or
    host ABI operation. The catalog may name an intrinsic, but backend code is
    the only layer that knows how that intrinsic becomes Wasm.

- [x] Provide read-only catalog queries for exact lookup, method lookup by
  receiver/capability, symbol enumeration, signature rendering, and
  documentation retrieval. Type checking, diagnostics, generated docs, LSP
  completion, hover, and signature help must all call these APIs rather than
  maintain parallel lists.
- [x] Model language-only concepts—keywords, action/lifecycle blocks, the
  settings DSL, and syntax forms—in a sibling `LanguageCatalog` using the same
  documentation/example model. They are not fake functions, but the docs and
  editor still need a uniform way to discover them. First-class control flow
  such as `retry expression` belongs here rather than in `StandardLibrary`.
- [x] Validate the catalogs at test time: IDs and canonical names are unique,
  references resolve, every intrinsic has a backend implementation, every
  public item has documentation, and every example parses and type-checks.
- [x] Migrate `min`, `max`, and `clamp` first as the proving case. They should
  be numeric methods with a shared constrained type variable and stable
  intrinsic IDs (or ordinary generic library bodies once those are
  expressible). Remove all raw checks for those names from type checking,
  temporary-local collection, and expression emission.
- [x] Then migrate every existing built-in and type-directed method. Adding a
  normal library API after this point should require one catalog declaration
  plus either a source body or one deliberately scoped backend intrinsic—not
  edits across parser, checker, code generator, documentation, and LSP code.
- [ ] Once modules and generic library functions exist, allow ordinary
  standard-library declarations and bodies to be authored in SplitScript and
  compiled into the same validated symbol graph. Keep most future library
  functionality there; reserve compiler intrinsics for representation
  primitives, host boundaries, and suspension/control-flow operations that
  cannot be ordinary source code. The interim hierarchical Rust declaration
  macro must feed the same graph so this later source migration replaces only
  the producer, not every compiler and tooling consumer.

### ABI, lowering, and code-generation boundaries

- [x] Describe host imports in a declarative ABI catalog containing their Wasm
  signature, ownership/lifetime contract, effects, and documentation. Generate
  import declarations and host-backed standard-library bindings from it, and
  make [`docs/ABI.md`](docs/ABI.md) a generated/verified view rather than a
  second hand-maintained source of truth.
- [x] Introduce a small Wasm-oriented lowering IR with explicit locals,
  blocks/terminators, coercions, and suspension points. This
  should replace direct typed-HIR-to-encoder emission and the remaining ad hoc
  scratch-layout planning, and prepare for
  `else return`, transactional `Result` handling, profile erasure, and a real
  async state-machine pass; a general optimizer or target-independent SSA IR is
  not required yet.

  - [x] Expose an inspectable `lower_wasm` stage with structured statements,
    `Fallthrough`/`Return`/`Suspend` terminators, and explicit suspension
    continuations. Make ordinary action/function emission and `onAttach`
    state-machine construction consume it.
  - [x] Plan value locals, async-frame fields, match inputs/payload bindings,
    and numeric-intrinsic scratch locals in the lowering product using semantic
    `TypeId`s. Assign concrete Wasm indices only in the encoder.
  - [x] Lower expression operations and typed coercion edges into the Wasm IR.
    Do not add a backend-only failure channel while doing this; result-aware
    control-flow edges belong after the language's real `T!` semantics.

    - [x] Add a stable-ID expression plan and migrate None/bool/integer/float
      literals, resolved value/member paths, unary operations, binary
      operations, explicit casts, semantic result types, and implicit
      Option/Result lift edges. Ordinary encoding consumes these nodes without
      re-querying their typed-HIR operation or path resolution.
    - [x] Migrate String literals/interpolation with explicit String-conversion
      edges, signature literals and their data collection, arrays, resolved
      struct-field constructors, and resolved enum-variant constructors.
    - [x] Migrate call arguments and resolved call targets, including user
      function IDs, method receivers/member chains, standard-library item IDs,
      and inferred generic type arguments. Ordinary and suspending call
      emission no longer queries typed HIR for semantic call resolution.
    - [x] Migrate expression-valued `if`, including nested `else if` branches
      and value-producing GC/reference branches.
    - [x] Migrate `else` fallback branches and postfix `?` propagation,
      preserving value, `return value`, bare `return`, and the exact inferred
      Result boundary in Wasm IR.
    - [x] Migrate `match` with resolved enum variants, payload binding IDs,
      literal/wildcard patterns, guards, and result arms. Remove the temporary
      `ExpressionKind::TypedHir` boundary and the expression encoder's
      typed-HIR context.
- [x] Split code generation by responsibility only after the typed-HIR and
  lowering interfaces exist: ABI/imports, GC type/layout planning, ordinary
  expression lowering, async/state-machine lowering, generated runtime,
  standard-library intrinsic lowering, and final Wasm encoding. Moving the
  existing functions into many files before establishing those data flows
  would only redistribute the coupling.

  - [x] Extract host-import type emission and stable ABI function-index
    assignment into `codegen/imports.rs`, driven solely by `AbiCatalog`.
  - [x] Extract deterministic GC type/layout construction into
    `codegen/gc_types.rs`. Return the completed recursive type section and next
    free type index as an explicit backend plan covering state, built-ins,
    async frames, structs, enums, arrays, Options, and Results.
  - [x] Extract final section assembly after generated functions, globals,
    exports, and data dependencies are represented as explicit backend plans.
  - [x] Extract ordinary block/assignment/expression emission and
    standard-library intrinsic dispatch into `codegen/expression.rs`, around
    the completed Wasm-IR expression plan and a narrow set of entry points used
    by actions, state reads, and async polling.
  - [x] Extract `onAttach` async/state-machine emission into
    `codegen/async_state.rs`, around Wasm-IR suspension states, continuation
    blocks, `retry`, and process-lifetime cancellation regions. Keep polling
    and state traversal private behind one orchestrator entry point.
  - [x] Extract settings registration, host-map refresh, value decoding,
    current/old rotation, tooltips/filters, and start-time initialization into
    `codegen/settings.rs`, exposing only its three generated function bodies.
  - [x] Extract the per-tick update runtime into `codegen/update.rs`: process
    attach/detach, cancellation cleanup, settings refresh, transactional state
    reads, snapshot rotation, lifecycle action ordering, and timer dispatch.
  - [x] Extract generated String, formatting, equality, scanning, address,
    managed-memory, and Unity/IL2CPP helpers into the
    `codegen/runtime_helpers/` domain modules. Generate runtime bodies from one
    descriptor-backed `RuntimeHelperPlan` and structural equality bodies from
    its separate type-directed plan.
  - [x] Extract generated-function signature and index planning into
    `codegen/function_plan.rs`. Allocate helper, settings, equality, user,
    state-read, action, start, and update functions in one deterministic pass;
    body emitters only consume its named indices.
  - [x] Extract state-read, user-function, and ordinary action body generation
    plus Wasm-local assignment into `codegen/script_functions.rs`.
  - [x] Extract global/data planning and final section assembly so
    `codegen::compile` becomes only the deterministic module orchestrator.
- [x] Track dependencies between selected library items, generated helpers,
  strings/signatures, and host imports so the backend can eventually emit only
  what a program uses. Deterministic output remains required.

  - [x] Introduce a backend dependency analysis over resolved standard-library
    calls and generated-helper edges. Use it to omit Unity module-name and
    IL2CPP signature data from scripts that never call `Unity.il2cpp`.
  - [x] Make generated function planning and body emission consume the helper
    set so unused helper signatures and placeholder bodies are removed. Helper
    calls use checked dependency-index lookups, and settings-free scripts omit
    both settings adapter functions and their update/start calls.
  - [x] Derive the required host-import set transitively from emitted helpers,
    state sources, settings, lifecycle behavior, and direct intrinsics. Emit
    the filtered subset in ABI-catalog order and use checked import lookups;
    setting registration/value imports are filtered by individual kind.
  - [x] Compute source-function and expression reachability from actions,
    state expressions, and global initializers, following user-function and
    user-method calls transitively. Filter user function signatures/bodies,
    source strings/signature literals, helpers, and host imports by that set.
  - [x] Filter generated structural-equality signatures/bodies from reachable
    `==` / `!=` operand types, recursively retaining nested struct/enum and
    String equality dependencies. A minimal script now emits no equality body.
  - [x] Introduce an explicit `GcLayout` plan returned by GC type construction;
    use it for recursive field storage, globals, generated function signatures,
    and expression/action local types while preserving the current layout set.
  - [x] Replace emitter-side semantic-ID index arithmetic with `GcLayout`
    lookups for aggregate construction/access, defaults, memory reads, settings,
    generated helpers, and failure propagation. Dynamic GC types now fail fast
    if they reach the fixed built-in type conversion path.
  - [x] Filter inferred GC layouts by reachable storage, signatures, and
    expressions, with transitive closure through struct fields, enum payloads,
    arrays, Options, and Results. Compiler-generated helper layouts are
    explicit roots, and `GcLayout` owns both compact ordering and encoding.

### Refactor safety and scope

- [x] Capture compile-pass and runtime behavior for every currently supported
  feature before changing the pipeline, including both Lunistice layouts,
  settings changes, await cancellation, transactional failed reads, numeric
  methods, matches, strings, and GC values across suspension.
- [x] Add focused snapshots for resolved/typed HIR and diagnostics. Backend
  tests should assert observable Wasm behavior instead of depending on
  incidental function/type indices.
- [ ] Keep this as internal modules in the existing crate initially. Split into
  syntax, HIR/type-system, standard-library, compiler, Wasm backend, CLI, docs,
  and LSP crates only when the new interfaces are stable and at least two
  consumers need them. Crate boundaries should enforce proven architecture,
  not be used to discover it.
- [x] Record compile-time and generated-Wasm-size baselines. Correctness and a
  clean semantic API come first, but the catalog/query design must not require
  repeatedly scanning all symbols or regenerating all helpers for each editor
  request.

## P0 — `Option`, `Result`, and explicit failure

The semantic type representation, catalog type schemes, typed HIR, and basic
lowering IR from the architecture checkpoint are prerequisites for this work.

### `Option` and `Result`

- [x] Add `T?` as the spelling of an optional constructed type. Preserve its
  value type and monomorphized Wasm GC layout in semantic queries.
- [x] Use `None` to construct the empty option and `Some(value)` as an optional
  explicit present-value constructor. A plain `T` automatically lifts into
  `T?` when the expected type is optional. Require an annotation or other
  expected-type context when a bare `None` has no constraint on `T`.
- [x] Add `T!` as a result constructed type with a standard language error
  payload. Preserve its value type and monomorphized Wasm GC layout in semantic
  queries; the error type is deliberately not generic initially.
- [x] Use `Err("message")` to construct failure and `Ok(value)` as an optional
  explicit success constructor. A plain `T` automatically lifts into the
  successful result when `T!` is expected. Require an annotation or other
  expected-type context when `Err` has no constraint on its success type.
- [x] Reject identical adjacent postfix constructors (`T??` and `T!!`) with a
  focused diagnostic. Mixed postfixes compose normally: `T!?` is an optional
  result and `T?!` is a fallible option. Expression propagation after a cast
  remains explicit through parentheses, such as `(value as T!)?`.
- [x] Record optional/successful lifts explicitly on typed-HIR expression edges
  with source and target `TypeId`s, and lower empty, successful, and failed
  values to their WebAssembly GC representations.
- [x] Define structural equality for both wrappers when `T` supports equality:
  Options compare empty/present state and present values; Results compare their
  success/error state, successful values, or standard error strings.
- [x] Define exhaustive matching for both wrapper types. Options use `None` and
  `Some(value)`; Results use `Err(error)` and `Ok(value)`. `_` remains a full
  wildcard, and guarded arms do not satisfy exhaustiveness.
- [x] Extend bidirectional inference so payload-bearing `Some(value)` and
  `Ok(value)` constructors work without expected-type context. Canonicalize
  provisional wrapper layouts after inference so later annotated uses share one
  nominal Wasm GC type. Payload-free `None` and success-type-free `Err(error)`
  still require context because their missing type cannot be inferred.
- [x] Decide which standard-library operations return `T?` versus `T!`.
  Immediate process operations whose attempt can fail return `T!`: fixed-layout
  reads, pointer following, relative-address decoding, and managed-string
  decoding. Suspended module/signature/Unity discovery yields `T` because
  temporary absence is its pending state and process closure is cancellation.
  Future one-shot lookup APIs where absence is an expected completed outcome
  return `T?`; none of the current catalog operations have that contract.
- [x] Preserve transactional state polling: an unhandled failed state read must
  skip the entire snapshot rather than commit partially updated fields.
- [x] After `T!` is represented in typed HIR, make result propagation explicit
  in the lowering IR. Replace the hidden `READ_FAILED_GLOBAL` writes emitted by
  individual process-read call cases with ordinary `Result` construction and
  handling. The state transaction boundary may initially preserve the same
  all-or-nothing commit behavior, but it must consume the language-level result
  path rather than a second backend-only error channel.

### `else` unwrapping and control flow

- [x] Support a low-precedence `else` operation for `T?` and `T!`, serving the
  common roles of `unwrap_or` and Rust's `let ... else` without method noise.
- [x] Permit a value fallback:

  ```text
  let name = optionalName else "Unknown"
  ```

- [x] Permit `return` as diverging control flow in the fallback:

  ```text
  let module = findModule() else return Err("module not found")
  ```

- [x] Add `while condition { ... }` statement loops as the foundation for loop
  control flow. Conditions are checked before every iteration and loop bodies
  are lexically scoped.
- [x] Add statement-form `break` and `continue` with nearest-loop behavior,
  including from nested conditional blocks. They are currently rejected
  outside loops.
- [x] Add direct `else break` and `else continue` fallback branches. Expression
  lowering carries structured branch targets through nested expression `if`,
  match arms, and short-circuit expressions. `else` is otherwise
  the lowest-precedence, right-associative expression operation; expression
  `if` owns its braced `else`, while `else return` is a diverging fallback.
- [x] Produce a useful diagnostic when `else` is applied to a non-optional,
  non-result value.
- [x] Add postfix `?` for `T!`. It unwraps success and propagates the original
  error to the nearest typed failure boundary. A state-field assignment and a
  function returning `T!` are currently boundaries.
- [x] Add `throw error` as the underlying error control-flow primitive for
  `T!` functions. Thrown errors are typed independently from `Err(error)`, which
  constructs an ordinary result value. Explicit `throw` and the failure arm of
  `value?` share one failure-transfer lowering operation rather than separate
  implicit-return implementations.
## P0 — Typed process deserialization

Start this only after standard-library calls resolve through the catalog and
typed HIR. Struct layout/deserialization should be a reusable semantic service,
not another collection of `process.read` branches inside code generation.

### One generic `process.read`

- [x] Replace primitive-specific calls such as `process.read<i32>(address)` with
  `process.read(address)` and infer the result from its expected type.
- [x] Add generic calls such as `process.read<i32>(address)` as the explicit type
  escape hatch for contexts where inference has no constraint.
- [x] Make synchronous and retried reads share the same type-directed API and
  clear failure semantics.
- [x] When inference is ambiguous, explain which annotation would resolve it
  and show an example in the diagnostic.

### Structs as readable memory layouts

- [x] Introduce a compiler-known `MemoryReadable` / `Deserializable` capability.
  Every primitive memory type implements it.
- [x] Automatically make a struct readable when all its fields are readable.
  The initial layout is field order with explicit, documented size and padding
  rules.
- [x] Read a struct with one host `process_read` call, then deserialize its
  fields locally. Besides being faster, this gives a coherent snapshot.
- [ ] Once real target layouts require it, add declarative layout controls for
  exact field offsets, explicit padding/packing, per-struct or per-field
  little-/big-endian decoding, and eventually custom decoding, without changing
  `process.read` call sites. Keep natural layout as the zero-annotation default.
- [x] Add a struct for Lunistice's adjacent clock fields and replace three
  reads with one:

  ```text
  struct LevelTimeParts {
      minutes: f32,
      seconds: f32,
      hundredths: f32,
  }

  levelTimeParts: LevelTimeParts = process.read(
      timerInstance.offset(levelTimeVectorOffset)
  );
  ```

### Managed strings

- [x] Remove the naming inconsistency of the standalone
  `process.readManagedString` function.
- [x] Initially place specialized readers in the same namespace, for example
  `process.readManagedString(address, maxUtf16Units)`, because a managed string
  is a pointer-based runtime object rather than a fixed inline memory layout.
- [ ] Explore representing Unity managed strings as a `Deserializable` wrapper
  once custom deserialization exists. Do not pretend they are ordinary inline
  `String` values merely to force them through the generic reader.

### Traits / interfaces

- [x] Establish one compiler-known structural equality capability shared by
  semantic diagnostics and Wasm helper generation. Structs and enums derive it
  recursively from their fields and payloads; this service can later feed LSP
  availability and hover information without reconstructing backend rules.
- [ ] Design a trait or type-class system compatible with bidirectional
  inference. It should support standard-library constraints such as
  `Deserializable` without forcing routine scripts to spell generic bounds.
- [ ] Start with compiler-known traits needed by memory reading, formatting,
  equality, and string interpolation.
- [ ] Decide later whether users can define and implement their own traits.
  Avoid committing to a full Rust-like trait system before real splitter ports
  demonstrate the necessary surface.
- [ ] Put trait/capability declarations and implementations in the same
  semantic catalog used by standard-library signatures. Completion and hover
  need to explain why a method is available for an inferred type, including
  which bound supplied it.

### Anonymous structs

- [ ] Add structural anonymous struct values and types after named-struct
  deserialization is stable.
- [ ] Infer their field types bidirectionally and allow ordinary field access,
  nesting, matching, and GC storage.
- [ ] Decide whether anonymous structs can implement `Deserializable` from a
  type annotation or remain value-level conveniences only. Named structs should
  remain the recommended form for documented target-process layouts.

## P1 — Core language and lifecycle polish

### Compound assignment

- [x] Support `+=`, `-=`, `*=`, `/=`, `%=` and the applicable bitwise/shift
  assignment operators.
- [x] Reuse the normal operator typing and cast rules. The left side must be an
  assignable location and must be evaluated exactly once.
- [x] Replace verbose code such as
  `runTimeSeconds = runTimeSeconds + old.levelTime` with
  `runTimeSeconds += old.levelTime` in the Lunistice port.

### Timer state as an enum

- [x] Replace the integer returned by `timer.state()` with an exhaustive
  `TimerState` enum.
- [x] Name variants after the actual ASR states and document host integer
  conversion only at the ABI boundary.
- [x] Support structural `==` / `!=` for enums, including active payload
  comparison, and update the Lunistice port to compare named `TimerState`
  variants directly instead of matching solely to emulate equality.

### Small global runtime functions

- [x] Make frequently used, unambiguous operations global:
  `setVariable(key, value)` and `setTickRate(hz)`.
- [x] Keep namespaced APIs where the namespace provides real disambiguation or
  discoverability. Do not flatten the entire standard library as a blanket
  rule.
- [x] Remove the prototype spellings `timer.setVariable` and
  `runtime.setTickRate`; the language has no compatibility burden yet, so one
  canonical spelling is preferable to aliases or migration diagnostics.
- [x] Rename the implementation-shaped `Duration.saturatingSecondsF32` API to
  the scripting-oriented `Duration.fromSeconds`. Keep range handling as a
  documented safety property, not part of the function name, and retain no
  compatibility alias while the language is still unpublished.

### Lifecycle vocabulary

- [x] Rename `update` to `whileAttached` so its execution condition is visible
  in the source.
- [x] Audit all lifecycle names together. The former detached block runs once
  on entry, so `onDetached` is more accurate than `whileDetached`.
- [x] Reserve `whileDetached` for a future block that genuinely runs on every
  detached polling tick, if a real use case needs it.
- [x] Use the coherent set `onAttach`, `whileAttached`, and
  `onDetached`, with the async and cancellation behavior documented beside the
  names.

### General async and cancellation lowering

- [x] After the lowering IR and `Result` semantics are stable, replace the
  current `onAttach` statement-index special case with a dedicated
  state-machine transformation over lowered control flow. Every await has a
  stable poll state and continuation state; nested conditional branches lower
  through the same dispatcher without replaying preceding statements while a
  poll remains pending.
- [x] Compute which locals actually live across each suspension point rather
  than storing every attach local in one continuation frame. Backward liveness
  is recorded on each lowered `Suspend`; the physical GC frame contains the
  deterministic union, while locals killed before use remain ordinary Wasm
  locals.
- [ ] Once real-world frame sizes justify it, coalesce non-overlapping
  suspension-live ranges into shared physical frame slots without changing the
  per-suspension liveness exposed by the lowering IR.
- [x] Model process lifetime as a structured cancellation region so library
  futures can offer the equivalent of ASR's `until_process_closes(...)`
  without every operation hard-coding `onAttach` checks. The lowered body owns
  the region, cancellable `Suspend` terminators reference it, and process exit
  resets readiness plus the complete continuation frame in one runtime action.
- [x] Let the standard-library catalog describe whether an operation suspends,
  can be cancelled, or requires an attached process. The checker, async
  lowering, docs, and LSP signature/hover output should expose the same facts.

  - [x] Add `RequiresAttachedProcess` and `CancelsOnProcessClose` alongside the
    existing suspension/retry effects, and make async lowering derive its
    cancellation edge from the resolved catalog item.
  - [x] Normalize raw effects into a public `OperationSemantics` query shared
    by the checker and lowering. Validate contradictory catalog declarations,
    render the same facts for hover/documentation consumers, and reject direct
    process operations in `onDetached`.
  - [x] Infer operational requirements through user-function call graphs so a
    helper that reads process state cannot be called transitively from
    `onDetached`. Publish the fixed-point result through `CheckedProgram` for
    ordinary functions, methods, and recursive call graphs without manual
    function annotations.
  - [ ] Surface those effects through the future machine-readable docs and LSP
    hover/signature protocol rather than inventing editor-specific flags.
- [ ] Broaden suspending control flow and the future library incrementally:

  - [x] Allow awaits in nested `if` / `else if` / `else` control flow inside
    `onAttach`, preserving the selected branch and its continuation.
  - [x] Add `await nextTick()` as the first reusable suspension primitive. It
    resumes on the following attached-process update without replaying prior
    statements and is cancelled with its process-lifetime region.
  - [x] Add first-class `retry expression` control flow for arbitrary `T!`
    expressions. It re-evaluates the expression once per update, yields `T` on
    success, and uses the assignment as its suspension/Result boundary. This
    works through ordinary user functions rather than a hard-coded builtin.
  - [x] Lower `while` loops containing `await` or `retry` through explicit async
    header and exit states. Resumed bodies preserve nearest-loop `break` and
    `continue` targets, including fallback forms and nested suspending loops.
  - [ ] Add suspending user functions and reusable race combinators after their
    inference, cancellation, and frame-ownership rules are specified.

## P2 — Later control-flow extensions

Explicit catches are intentionally deferred. Ordinary autosplitters can already
handle failures with `T!`, `else`, postfix `?`, `throw`, function boundaries,
and transactional state-field boundaries; catch syntax does not currently
unblock a representative port.

- [ ] Design explicit `catch` boundaries and their expression syntax. An
  uncaught throw leaves a `T!` function as its error result; state-field
  assignments catch into their poll result. Ensure nested catches compose
  without losing the original error or forcing equal success types.
- [ ] Once explicit catches exist, allow `throw` anywhere their boundary is in
  scope, including actions and expression-oriented state DSL contexts where a
  statement block becomes available.

## P2 — Debug and release profiles

Implement this on typed HIR/lowering IR and catalog effects. Profile erasure
must be a semantic lowering pass, not conditionals scattered through AST walks
and Wasm emission.

- [x] Add explicit debug and release compiler profiles to the CLI and compiler
  library API. `--profile debug|release` is shared by one-shot and watch builds,
  debug is the default, and the selected profile is retained by Wasm lowering
  for the upcoming semantic erasure pass.
- [x] Add the first `debug` modifier for expression statements and calls such
  as `debug print(...)`, assignments, `if`, `while`, and unbound suspension
  statements.
- [x] Extend `debug` to function and method declarations. Debug functions are
  checked normally but omitted from release Wasm IR.
- [x] Add debug-only local bindings, suspended bindings, and globals. Retained
  code cannot use their names, and release lowering removes global storage and
  initialization as well as local statements.
- [ ] Extend `debug` to remaining declarations only when a real use case
  establishes their erasure and dependency rules.
- [x] Remove supported debug-only statements from release WebAssembly before
  reachability, and verify their strings and imports are eliminated too.
- [x] Enforce the release name-resolution rule for functions and bindings:
  ordinary retained code cannot use a debug-only name, while debug statements
  and debug functions can. Apply the same rule when more declaration kinds
  become debug-capable.
- [x] Initially restrict `debug` to statements whose removal has a
  clear type (`unit`). A value-producing debug expression needs either a
  release fallback or another explicit rule; silently inventing a default value
  would be unsafe. Debug-only bindings now have explicit lexical visibility
  and erasure rules; terminating statements remain rejected.
- [x] Type-check debug-only code in release builds so debug paths do not
  silently rot, while removing it before release code generation.
- [x] Add profile-aware compiler and runtime tests and document current
  diagnostic and debug logging behavior.

## P2 — Formatter, LSP, and editor support

### Watch builds

- [x] Add `splitc watch <input.split> [-o <output.wasm>]` with an immediate
  initial build and content-based change detection that survives editor file
  replacement and coarse filesystem timestamps.
- [x] Publish each successful module through a same-directory temporary file
  and rename, so debugger reloaders do not observe partial Wasm. Preserve the
  last successful output when reading or compilation fails.

### Tooling-ready syntax and compiler database

Do this immediately before formatter/LSP implementation. It builds on the
compiler facade and typed HIR, but it does not block the intervening language
and standard-library work.

- [x] Add a lossless source document and token/trivia layer. The compiler uses
  the same lexer pass for parsing and for an ordered lexeme stream that retains
  whitespace, ordinary comments, documentation comments, exact token spelling,
  and byte spans across parsed, lowered, and checked products. Formatting must
  consume this layer rather than pretty-printing the semantic AST.
- [x] Add an editor-facing recovering parse API with a partial AST, multiple
  diagnostics, and explicit missing/error recovery nodes at top-level
  declaration boundaries. Batch parsing uses the same pass but remains strict.
- [x] Recover independently inside function, action, and nested statement
  blocks. Synchronize at semicolons, closing braces, and plausible statements
  on later lines without consuming a valid boundary token.
- [x] Recover invalid struct fields and enum variants independently while
  retaining later valid members and their stable IDs.
- [x] Recover invalid state fields independently in both supported state
  syntaxes, retaining other pointer paths and state expressions.
- [x] Recover neighboring settings independently in the simple settings block,
  nested documentation DSL, and older constructor-shaped syntax.
- [x] Recover invalid `choice` options and file filters while retaining their
  containing setting and later valid entries.
- [x] Recover invalid match arms independently while retaining the match
  expression, later arms, and enclosing function or action.
- [x] Recover invalid function parameters independently and retain function
  bodies when the parameter list is missing its closing parenthesis.
- [x] Recover malformed array elements and function-call arguments
  independently, retaining later expressions and their enclosing statement.
- [x] Recover malformed struct-literal fields and template interpolations
  independently, retaining neighboring fields, later interpolations, and the
  enclosing expression.
- [x] Add a syntax-only error expression and use it for missing unary/binary
  operands and malformed parenthesized expressions. Preserve the following
  statement without allowing recovery placeholders into typed HIR.
- [x] Recover missing conditions, empty or malformed branches, and a missing
  `else` inside expression-valued `if`, retaining the complete conditional and
  following statements.
- [x] Recover malformed declaration and statement root expressions without
  discarding globals, state fields, locals, assignments, control-flow
  statements, suspensions, throws, or standalone expression statements.
  Missing `match` scrutinees likewise retain the enclosing match expression.
- [ ] When modules or another multi-source feature are actually introduced,
  add `FileId`, a source map, and file-aware spans as part of that feature.
  Single-file scripts keep file-local byte spans for now; line/column
  conversion remains at the presentation boundary.
- [x] Move diagnostics into a dedicated model with stable compiler-stage codes
  and severity. Lexical, syntax, type, and post-type semantic errors use
  `SS0001` through `SS0004`, and the CLI renderer exposes the same values that
  editor tooling can query.
- [x] Add primary and secondary labels, notes, and applicability-classified,
  multi-edit fixes to the shared diagnostic value. CLI rendering consumes this
  model, and the repeated wrapper-postfix diagnostic proves a real
  machine-applicable source edit. Eventual LSP conversion must use these same
  values rather than introducing a parallel diagnostic shape.
- [ ] Enrich individual diagnostics with focused labels, notes, and fixes as
  language features and editor code actions are implemented; do not block the
  compiler database on exhaustively annotating every existing error first.
- [x] Back the compiler facade with reusable single-source queries for syntax,
  lowering, name resolution, inference, references, and diagnostics. Begin
  with explicit caching/invalidation; adopt a framework such as Salsa only if
  measurement shows it is worthwhile.

  - [x] Add a revisioned `CompilerDatabase` that caches recovering/strict
    parsing, declaration lowering, checking, and diagnostics as shared query
    results. Identical source updates are no-ops; changed text invalidates all
    dependent stages without introducing a `FileId`.
  - [x] Expose declaration lookup, inferred expression/value/function-result
    types, semantic type shapes, resolved calls and paths, assignment targets,
    and a cached read/write reference index without forcing clients to know
    which compiler product owns each fact.
- [x] Preserve partial syntax/HIR results after errors and expose symbol/type
  lookup at a source position. Completion, hover, semantic tokens, navigation,
  and code actions must not invoke or depend on the Wasm backend.

  - [x] Lower declarations retained by the recovering parser into cached HIR
    even when strict parsing fails.
  - [x] Query the smallest checked expression at a byte position with its
    inferred `TypeId`, semantic `TypeKind`, and resolved path/call/constructor
    information.
  - [x] Expose exact lossless token lookup at a byte position and align the
    identifier components of typed paths and call targets with their precise
    source spans, excluding arguments and nested child expressions.
  - [x] Resolve identifier segments in checked expressions to exact
    source-definition spans for values, functions, struct fields, enums, and
    enum variants. Represent standard-library calls and compiler-provided
    fields as catalog targets rather than inventing source spans.
  - [x] Add a syntax-reference index for source-defined types in annotations,
    method receivers, return types, enum payloads, and casts, plus struct
    literal type/field labels and enum-pattern type/variant labels. Declaration
    and pattern-binding identifiers navigate to themselves.
  - [x] Navigate source-spellable built-in types, array/option/result syntax,
    wrapper constructors and patterns, keywords, lifecycle names, setting
    documentation, and choice/file DSL tokens to stable `LanguageCatalog`
    items. Choice option enum/variant labels navigate to their source.
  - [x] Catalog and navigate snapshot roots, standard-library value fields,
    and the `TimerState` type and variants. Their semantic IDs resolve to the
    `StandardLibrary` symbol graph rather than editor-specific special cases.
  - [x] Preserve useful semantic facts when other expressions fail type
    checking so navigation and hover remain available in unaffected regions.
    The recovering checker publishes its diagnostics alongside a partial
    semantic model; database type, resolution, position-analysis, and
    definition queries fall back to that model without constructing typed HIR
    or invoking the Wasm backend.

### Formatter

- [x] Build a canonical formatter first, sharing the compiler lexer/parser.
- [x] Preserve ordinary comments and `///` setting documentation comments.
- [x] Cover settings DSL indentation, match arms, interpolated strings, state
  expressions, and multiline process reads.
- [x] Expose formatting through `splitc fmt` and a cached
  `CompilerDatabase::format` query for the future LSP.

### Language server

- [x] Create the `splitls` LSP server module and stdio binary inside the
  existing crate, backed by one reusable `CompilerDatabase` per open document
  rather than reparsing independently for each feature. Keep it internal until
  the crate-splitting criteria above are met.
- [x] Implement diagnostics and formatting first, including full document
  synchronization, UTF-16 positions, document versions, structured diagnostic
  metadata, and cached whole-document formatting edits.
- [x] Add semantic highlighting, including settings titles, state fields,
  action/lifecycle blocks, types, enum variants, signatures, and debug-only
  code. A cached compiler-owned highlight index combines lossless lexical
  tokens, syntax declarations, and recovered semantic resolutions; the LSP
  only converts its byte spans into delta-encoded UTF-16 semantic tokens.
- [x] Add completion for keywords, action blocks, standard-library symbols,
  settings, state snapshots, struct fields, enum variants, and inferred methods.
  The compiler owns candidate kinds, snippets, documentation, and replacement
  spans. An incomplete `receiver.` is probed without its unfinished suffix so
  receiver types and struct/user/standard-library members stay inferable. Root
  completion follows lexical scope and includes parameters, preceding ordinary
  and suspension bindings, nested-block locals, and match bindings while
  excluding declarations after the cursor.
- [x] Add standard-library hover and signature help directly from
  `StandardLibrary` catalog queries. Completion already consumes catalog
  signatures and documentation; hover and signature help additionally show
  inferred substitutions, effects/availability, parameter docs, and the
  compiler-validated catalog examples without importing the Wasm backend.
  Signature help counts nested delimiters correctly and probes the inferred
  receiver when a method call is still syntactically incomplete. Source hover
  renders inferred types for globals, locals, parameters, state and setting
  fields, struct fields, functions and methods, structs, enums, and variants.
  Function and method hover also renders transitive operational effects,
  attachment constraints, synchronous behavior, and debug-only availability.
- [x] Drive keyword, settings DSL, and lifecycle hover documentation from the
  sibling `LanguageCatalog`; completion already consumes the catalog and the
  extension must not duplicate prose or syntax lists.
- [x] Add an integration test that asks the LSP for a catalog symbol such as
  numeric `.clamp`, then verifies that completion and hover expose the same
  signature and documentation as generated standard-library docs. The
  renderer-independent `StandardLibraryDocumentation` payload is now shared by
  completion, hover, and signature help; the test compares their JSON-RPC
  responses against both its generic and inferred `T = i32` forms.
- [x] Add go-to-definition and find-references for functions, types, globals,
  state fields, settings, struct fields, and enum variants. Both features use
  stable source declaration IDs and exact identifier-token references from the
  compiler database; the LSP only maps those spans into locations for the
  current single-file URI. References distinguish same-spelling declarations
  and honor the protocol's `includeDeclaration` flag.
- [x] Add identity-safe rename after the core navigation features are stable.
  `prepareRename` selects the exact occurrence under the cursor, and the
  compiler query validates identifier syntax, reserved catalog names, and a
  rebuilt candidate document. It additionally verifies that every existing
  source reference retains the same stable declaration ID, preventing captures
  that could still type-check. Rename currently requires a semantically valid
  document so newly introduced conflicts can be distinguished reliably.
- [x] Add document symbols and code actions. A cached, editor-neutral compiler
  query restores source order and models state/settings as domain containers,
  nested setting titles as outline groups, struct/enum children, methods, and
  lifecycle events. LSP quick fixes are derived from the compiler's structured
  diagnostic fixes, honor requested ranges and `context.only`, and preserve
  applicability metadata rather than duplicating repair logic in the server.

### VS Code extension

- [x] Package the LSP client, language configuration, file association, basic
  fallback grammar, formatter integration, and build/debug tasks. The
  TypeScript client uses `vscode-languageclient` 10, discovers configured,
  bundled, repository-development, or `PATH` copies of `splitls`, and restarts
  when server settings change. The repository launch/task configuration builds
  both halves and also exposes compile/watch tasks for the current `.split`
  file. The extension itself exposes two explicit editor-title, context-menu,
  and Command Palette workflows: a status-bar-managed debug watcher that
  rebuilds after saves, and a one-shot release build that cannot race with the
  watcher. Both share compiler discovery, automatic initial saving,
  notifications, and an output channel; the extension package passes TypeScript
  checking and a dry-run pack.
- [x] Use semantic tokens from the LSP as the authoritative highlighting layer;
  keep TextMate highlighting only as a fast startup fallback. The extension
  enables semantic highlighting for SplitScript, contributes every custom
  domain token type and modifier with standard supertypes/theme scopes, and has
  a Rust integration test that prevents its manifest from drifting from the
  server legend.
- [ ] Add snippets for state, settings, lifecycle blocks, match, structs, and
  common process/Unity attachment patterns.

## P2 — Documentation and migration

### Generated language and standard-library documentation

- [ ] Build the browsable rustdoc-like renderer as a consumer of
  `StandardLibrary` and `LanguageCatalog`; the structured source of truth and
  compiled-example validation are established in the P0 architecture
  checkpoint, not recreated here. Reuse the renderer-independent
  `StandardLibraryDocumentation` entry model already consumed by editor tools.
- [ ] Generate canonical signatures from semantic type schemes and link types,
  traits, methods, related items, source definitions, and host capabilities.
- [ ] Publish machine-readable catalog data for editor tooling that cannot link
  the compiler library directly, with a schema/compiler version handshake.
- [ ] Test that rendered pages, machine-readable output, and LSP hover identify
  the same catalog item and use the same documentation payload.

### Guides for existing communities

- [ ] Write “Coming from old ASL / C#”, “Coming from TypeScript / JavaScript”,
  and “Coming from Rust” guides.
- [ ] Include syntax maps, lifecycle differences, numeric and address types,
  nullability/results, process reads, async attachment, settings, and complete
  small ports.
- [ ] Explain semantic differences, not just token substitutions—especially
  transactional state, inference, cancellation, and WebAssembly sandboxing.

### Familiarity-oriented diagnostics instead of broad aliases

- [x] Add recovery diagnostics with preferred machine-applicable fixes for the
  first unambiguous foreign spellings:
  - C#/JavaScript declarations: `const` and `var` → `let`; `func` and
    `function` → `fn`;
  - JavaScript absence and C# library names: `null` → `None`, `string` →
    `String`, and `TimeSpan` → `Duration`;
  - C# numeric keywords: `sbyte`/`byte`, `short`/`ushort`, `int`/`uint`,
    `long`/`ulong`, and `float`/`double` → the corresponding
    `i8`/`u8` through `i64`/`u64` and `f32`/`f64` types.
  Recovery must produce canonical syntax immediately so one familiar spelling
  does not cause a cascade of unrelated parser or type errors.
- [x] Add context-sensitive “did you mean” diagnostics for unresolved catalog
  and user-defined function/method calls. Compare names across camelCase,
  PascalCase, and snake_case before applying ordinary edit-distance matching,
  so `Duration.FromSeconds`, `Duration.from_seconds`, and small typos all point
  to `Duration.fromSeconds`. Filter methods by the inferred receiver type,
  replace only the exact name segment, and suppress ambiguous or unrelated
  guesses. LSP code actions expose unique suggestions as machine-applicable.
- [ ] Move the growing foreign-spelling table and its source-language,
  replacement, explanation, and applicability metadata behind a migration
  catalog consumed by diagnostics and LSP code actions. Keep context-sensitive
  recognition in the parser/checker: catalog data must not make ordinary
  bindings such as `let double = ...; double.clamp(...)` look like type names.
- [ ] Add the remaining unambiguous token and delimiter fixes first:
  - JavaScript `===`/`!==` → `==`/`!=` (there is no coercing equality), and
    `${value}` → `{value}` inside SplitScript backtick interpolation;
  - TypeScript `boolean` and CLR `Boolean` → `bool`, plus CLR primitive names
    such as `Int32`, `UInt32`, `Single`, `Double`, and `System.Int32` → their
    canonical SplitScript types;
  - C#/Rust `IntPtr`, `UIntPtr`, `nint`, and `nuint` → `address` only in
    target-process-address contexts where that nominal conversion is correct;
  - Rust `let mut name` → `let name`, because SplitScript `let` bindings are
    already mutable.
- [ ] Add type-aware fixes only after name and type resolution, so they are not
  offered for shadowed user symbols:
  - JavaScript/TypeScript `value ?? fallback` → `value else fallback` when the
    left side is an Option or Result and the fallback has the unwrapped type;
  - `.toString()`/`.ToString()` → `as String`, and `.Length` → `.length`, when
    the receiver and result make the rewrite equivalent;
  - `console.log(value)` and Rust `println!(...)` → `print(...)` only for call
    shapes whose formatting behavior is preserved;
  - C# `Math.Min`/`Math.Max`/`Math.Clamp` and Rust `min`/`max` forms → the
    type-directed `.min(...)`, `.max(...)`, and `.clamp(...)` methods.
- [ ] Provide focused explanatory diagnostics without an automatic replacement
  when the foreign type has no unique equivalent: TypeScript `number` and
  `bigint`, C# `decimal`, and generic error-bearing `Result<T, E>` all require a
  width, representation, or error-model decision from the author.
- [ ] Recognize common structural syntax and show a canonical example, but leave
  multi-token rewriting to `splitc migrate` unless equivalence is proven:
  - Rust `Option<T>`/`Result<T, E>` versus `T?`/`T!`, postfix `.await`,
    `unwrap_or`, `vec![]`, `loop`, and `&str`;
  - C# casts `(T)value`, `new` expressions, `$"..."` interpolation, and
    `switch`; JavaScript ternaries, optional chaining, arrow functions, object
    literals, and `switch`;
  - explicit `async onAttach` should explain that `onAttach` is inherently
    suspending; `async fn` must not be presented as equivalent until reusable
    suspending functions exist.
- [ ] Add old-ASL-specific lifecycle migration diagnostics:
  - `update` → `whileAttached` is a direct block-name migration;
  - `init` should point to `onAttach` and explain suspension and automatic
    process-close cancellation;
  - `exit` should point to `onDetached` while warning that `onDetached` also
    runs once for the initial detached state;
  - `startup`, `shutdown`, and `onStart` need guidance rather than blind renames
    because their lifetime boundaries do not all have one-to-one replacements.
- [ ] Teach `splitc migrate` the old-ASL shapes that require coordinated AST
  rewrites: `state("process")` declarations, `vars`, `settings.Add`/
  `AddDropdown`/`SetToolTip`, refresh-rate assignment, timer APIs, C# memory-read
  helpers, and action/lifecycle blocks. Preserve comments and report every
  construct that needs manual review.
- [ ] Test every migration rule with a positive case, a shadowing/ambiguity
  negative case, the advertised applicability, and—where a fix is marked
  machine-applicable—a test that applying all edits yields compiling canonical
  source. Build the eventual migration-command fixtures from real splitter
  ports rather than synthetic syntax alone.
- [ ] Prefer diagnostics and automated code actions over accepting foreign
  spellings as permanent aliases. Multiple equivalent syntaxes fragment style,
  complicate the formatter and documentation, and make search/completion less
  predictable. Only add a compatibility alias when porting data shows that it
  removes substantial friction without changing semantics; the formatter must
  still emit one canonical spelling.

## Ongoing — Port-driven language development

- [ ] Maintain a representative corpus of real autosplitters covering native
  games, Unity Mono, Unity IL2CPP, Unreal, emulators, pointer-heavy games,
  settings-heavy splitters, load removal, game-time calculation, and process
  restarts.
- [ ] Port additional splitters incrementally and record every missing feature,
  awkward pattern, generated-Wasm issue, and diagnostic failure.
- [ ] Promote repeated game-independent patterns into the standard library;
  keep game-specific signatures, offsets, and policies in scripts.
- [ ] Add a runtime conformance test for every promoted feature, including
  cancellation, failed reads, settings changes, process closure, and memory
  handle cleanup.
- [ ] Use the port corpus as formatter fixtures, LSP integration projects,
  documentation examples, compile-time benchmarks, and release regressions.
- [ ] Do not declare the language generally usable based only on Lunistice. The
  corpus—not speculative feature count—is the readiness criterion.

## Recommended execution order

1. Keep the completed expression-valued `if` and Lunistice union-state behavior
   locked down with compile and runtime regression tests.
2. Establish the compiler facade, semantic `TypeId`/symbol model, constraint
   layer, resolved typed HIR, and shared visitors without changing language
   behavior.
3. Build the standard-library/language/ABI catalogs and their validation API.
   Migrate `min`/`max`/`clamp` first, then all existing built-ins; make a small
   catalog documentation query testable before adding more library APIs.
4. Add the Wasm-oriented lowering IR and backend boundaries, initially without
   inventing a separate failure protocol. Split the code generator internally
   only as those interfaces make the split natural.
5. Specify and implement `T?`, `T!`, and `else` fallback/control flow on those
   foundations, including their typed-HIR and Wasm GC representations.
6. Lower process-read failures through ordinary `T!` values and result-aware
   control flow, then implement generic typed reads, named-struct
   deserialization, the minimal
   capability/trait machinery, and Lunistice's single `LevelTimeParts` read.
7. Land compound assignment, `TimerState`, global convenience functions, and
   lifecycle renaming as one language-consistency pass through the catalog.
8. Generalize async lowering and structured process cancellation before adding
   a larger future/combinator library.
9. Add lossless syntax, structured diagnostics, source-aware compiler queries,
   and the formatter; then build LSP and VS Code support. Standard-library
   completion/hover/signature help must already have catalog data to consume.
10. Build the browsable documentation renderer and machine-readable catalog
    export from the same API used by the LSP.
11. Add debug/release profiles as typed-HIR/lowering passes and expand them
    using real debugging workflows.
12. Continue porting diverse autosplitters and measuring compile time/Wasm size
   throughout every step; use the corpus to revise priorities rather than
   waiting until the architecture is “finished.”

## P0 — Non-uniform named state layouts and Ronin port (2026-08-03)

A corpus audit of 1,593 versioned ASL scripts found 95 with genuinely missing
fields or conflicting same-named field types. Ronin was selected as the
smallest maintained pressure case with two real layouts: both expose
`loading: i32`, while `bike` changes from `i16` in version 8 to `u16` in
version 9.

Named layouts now project only fields present everywhere with a compatible
type into the common `StateSnapshot` interface. A direct exhaustive
`match layout` refines both `old` and `current` to the selected layout, exposing
missing or conflicting fields with their concrete types. The backend stores
incompatible declarations in distinct Wasm GC struct fields and maps compatible
declarations to one common slot; it does not introduce options or semantic
defaults. Completion, hover/navigation identities, semantic highlighting, and
rename preserve the same distinction.

`examples/ronin.split` ports the original splitter through this model. Runtime
fixtures cover both executable versions and an unsupported build, including
their different pointer paths, signedness, start/game-time behavior, and bike
split. The language reference, ASL porting guide, migration diagnostic, and
language catalog document the refinement rule.

# 2026-08-06: bounded native UTF-16LE strings

- Added `process.readUtf16Le(address, maxUtf16Units) -> String!` and matching
  `as utf16le(maxUtf16Units)` pointer-state sugar without introducing sized
  string pseudo-types or reusing Unity managed-string object decoding.
- Native reads are bounded to 2048 little-endian code units, stop at the first
  NUL, preserve supplementary characters, and replace malformed surrogate
  sequences with U+FFFD. Zero/excessive bounds and failed memory are ordinary
  Result failures; optional state fields retain the existing observable-None
  semantics.
- Added completion, hover, semantic highlighting, type inference, diagnostics,
  code generation, and a host-executed regression covering direct and state
  reads, NUL termination, surrogate pairs, replacement decoding, failure, and
  bounds.

# 2026-08-06: signed native pointer offsets

- Separated full-width unsigned absolute roots from signed module-relative and
  post-dereference offsets in the syntax AST. Static `at` paths now accept
  negative `i64` offsets while preserving the entire unsigned address domain.
- Made `process.follow`, source-defined `MemoryPath`, and generic
  `address.offset<T: Integer>` use the same signed wrapping-displacement model;
  `address.add(u64)` remains the explicit unsigned-delta operation.
- Added range diagnostics and a host-executed fixture covering a negative
  module root, negative intermediate/final offsets, dynamic pointer following,
  reusable paths, and a high unsigned absolute address.

# 2026-08-09: decimal exponents and finite subnormal floats

- Added decimal exponent syntax while preserving its exact source spelling in
  the formatter and editor token stream.
- Accepted representable `f32` and `f64` subnormal constants, while diagnosing
  target-specific underflow to zero and overflow to infinity.
- Added a host-executed fixture that reads IEEE-754 bit pattern one from process
  memory at both widths and compares it with decimal literals and global
  constants.

# 2026-08-09: exact string splitting and Operation Matriarchy

- Added `String.split(delimiter) -> [String]!` with exact UTF-8 matching,
  preserved leading/adjacent/trailing empty segments, and an error for an empty
  delimiter. Its trusted helper allocates the exact WebAssembly GC result array
  and exposes the same catalog documentation to compiler and editor tooling.
- Added a maintained Operation Matriarchy port and deterministic host fixture.
  The port reads the original bounded `Game.dll` level-picture field, parses
  identifiers such as `01_02_1.dds`, and proves start, reset, split, loading,
  attachment, and executable/module-name behavior.

# 2026-08-09: strict fallible numeric parsing

- Added `String.parse<T: Numeric>() -> T!`, with bidirectional target-type
  inference and ordinary `Result` fallback, propagation, and matching.
- Integer parsing is exact across every signed and unsigned width (including
  `address`); floating-point parsing uses allocation-free exact decimal
  conversion with ties-to-even rounding directly to `f32` or `f64`, including
  subnormals, signed overflow/underflow, `NaN`, `inf`, and `Infinity`.
- Added compiler/editor catalog coverage, ASL/C# migration guidance, and a
  host-executed differential regression covering thousands of boundaries,
  long and randomized decimals, malformed input, inference, non-finite values,
  overflow, underflow, and hard halfway cases.

# 2026-08-10: stable growable arrays and amortized push

- Represented every source array as a stable WebAssembly GC wrapper over
  compiler-internal raw storage, keeping logical length separate from physical
  capacity so aliases remain valid when storage is replaced.
- Added catalog-defined `[T].push(value)`, geometric capacity growth, and a
  focused `[T; N]` diagnostic and completion filter. Fixed arrays remain exact,
  memory-readable values and cannot change length.
- Added compiler, editor, validation, and host-runtime coverage that crosses
  multiple growth boundaries and observes mutations through an alias.

# 2026-08-12: typed mapped-memory snapshots

- Added synchronous `process.memoryRanges() -> [MemoryRange]`, copying the
  existing host count/index metadata into a stable WebAssembly GC-owned array.
- Preserved host order and decoded ABI permission bits into typed readable,
  writable, and executable fields without exposing handles or manual frees.
- Kept content scanning async and cooperative while avoiding needless
  update-by-update latency for cheap mapping metadata enumeration.

# 2026-08-13: optional loaded-module probes

- Added synchronous `process.loadedModule(name) -> Module?` for known optional
  platform and mod-loader modules, while retaining `process.module(name)` as
  the waiting attach-time operation for required modules.
- Kept module-name transport and ABI handles inside a trusted runtime helper;
  scripts receive the existing typed GC-owned `Module` representation.
- Added a host-executed regression proving present and absent lookup semantics
  without adding full module enumeration prematurely.

# 2026-08-13: dynamic setting membership

- Added allocation-free `SettingsView.contains(key)` over the compiler-known
  value-setting declarations, distinguishing a disabled known boolean from an
  unknown key without exposing a dynamically typed settings map.
- Included boolean, choice, and file keys while deliberately excluding visual
  headings, and shared the UTF-8 key comparison lowering with `enabled`.
- Added host-runtime coverage for current and previous settings views, every
  value-setting kind, headings, unknown keys, and disabled known values.

# 2026-08-13: maintained Axiom Verge migration evidence

- Ported the legacy Axiom Verge autosplitter with its full 119-key settings
  catalog, four platform offsets, cooperative signature discovery, 32-bit
  pointer paths, dynamic UTF-16 event identifiers, game time, starts, splits,
  and reset-on-death behavior.
- Replaced callback-shaped C# delegates with ordinary typed functions and
  preserved recursive parent-setting gates explicitly; disabled but declared
  identifiers still advance their event cursors without splitting.
- Added a deterministic vanilla-Steam runtime fixture and a structured
  conformance record that separates verified behavior from variant coverage,
  conditional settings UI, exact timer events, and scan-deadline limitations.

# 2026-08-13: source-defined PE export lookup

- Added `Module.peExport(name) -> address!` as a source-defined standard-library
  parser over validated DOS, PE, optional-header, export-directory,
  name/ordinal, and function tables.
- Kept forwarded exports and out-of-image metadata as ordinary errors instead
  of treating forwarder strings or malformed RVAs as executable addresses.
- Added deterministic runtime coverage for direct, absent, and forwarded
  exports as the reusable native prerequisite for Unity Mono discovery.

# 2026-08-13: source-defined Unity Mono and ARTIFICIAL

- Added typed `MonoModule`, `MonoImage`, and `MonoClass` traversal for explicit
  modern 64-bit Windows V2/V3 layouts, entirely in standard-library
  SplitScript on top of existing process reads, scanning, and PE export lookup.
- Established trusted cross-type access for source-defined standard-library
  bodies while keeping runtime-private fields invisible to user code and
  editor tooling.
- Ported ARTIFICIAL and added a deterministic host fixture covering PE export
  resolution, RIP-relative assembly-list discovery, managed assembly/class/
  field traversal, static values, and start/split/reset/game-time behavior.

# 2026-08-13: composable Mono singleton paths and Himno

- Added source-defined immutable `MemoryPath.dereference` composition and
  `MonoClass.staticFieldPath`, allowing a state poll to reread a replaceable
  static singleton before applying an instance-field offset.
- Extracted the PE64 Mono V2 fixture builder so maintained ports share one
  metadata-memory contract instead of cloning a large synthetic graph.
- Ported Himno and verified start, split, and reset behavior while replacing
  the `PlayerStats.script` singleton between every relevant snapshot.

# 2026-08-13: lifecycle-owned polling rates

- Bisected the real-game Lunістice attachment regression to the source-defined
  cooperative Unity migration: its 64 KiB scan polls had inherited the
  script's detached 1 Hz rate until discovery completed.
- Made 120 Hz attached and 1 Hz detached language defaults, applied before
  `onAttach` polling and before `onDetach` respectively, with an optional
  top-level `tickRate` declaration for overrides.
- Retained `setTickRate` for temporary dynamic changes and made lifecycle
  transitions reassert the declarative policy.
- Expanded the Lunістice fixture to spread IL2CPP discovery across a 4 MiB
  module and verify that the attached cadence is selected before scanning.

# 2026-08-15: whitespace-independent type boundaries

- Kept maximal-munch `>=`, `>>`, `>>=`, and `!=` tokens for ordinary
  expressions while allowing the type grammar to fission them contextually
  into source-accurate generic closers and, at declaration initializer
  boundaries, result postfixes and residual assignments. Expression casts keep
  `T!=value` as inequality, while `T! == value` explicitly casts to a fallible
  type before comparing.
- Retained every written constructed-type boundary and explicit generic-call
  range in the syntax model, then gave the formatter a logical token stream so
  adjacent and multiline nested generics format and indent as distinct
  delimiters without changing the lossless lexer.
- Added compact, spaced, and tab-separated boundary matrices; nested generic
  declarations and calls; option/result postfixes; privileged standard-library
  source; ordinary comparison/shift operators; multiline trailing commas; and
  parse/format/parse idempotence coverage alongside the maintained examples.

# 2026-08-19: first-class `Never` type

- Added the source-level `Never` bottom type and directional bottom coercion,
  allowing divergent branches to join with ordinary values in fallback,
  conditional, and match expressions without manufacturing a unit value.
- Declared `Process.closed()` as `async Never`, replaced the intrinsic-specific
  return-path exception with semantic terminal-flow analysis, and exposed the
  completion type consistently through generated documentation, hover, and
  inlay hints.
- Erased standalone bottom values from Wasm parameters, results, locals,
  globals, and async frames while retaining legal uninhabited payload storage
  for constructed forms such as `Never?`; added synchronous and async
  validation coverage for named-layout attachment and bottom/value joins.

# 2026-08-19: file-version match patterns

- Made checked `v"major.minor.build.private"` literals first-class [`match`]
  patterns, with ordinary `FileVersion` typing, duplicate-arm detection, guards,
  and direct four-component Wasm comparisons.
- Kept exhaustiveness honest for the open executable-version space by requiring
  a wildcard arm, and replaced the Borderlands port's chained-comparison
  workaround with direct typed version dispatch in the language and porting
  documentation.

# 2026-08-19: scoped value blocks

- Made brace-delimited blocks first-class expressions in conditional branches,
  match arms, fallbacks, arguments, and state initializers, with lexical locals
  and the final expression supplying the value.
- Preserved explicit `return` for function, method, and lifecycle bodies, with
  a focused diagnostic and machine-applicable insertion for Rust-style omitted
  returns.
- Accepted a trailing tail semicolon without changing semantics, while warning
  about it, offering removal, and making the formatter emit the canonical
  semicolon-free value form.
- Unified synchronous and suspending block lowering so `await`, `retry`, and
  divergent control flow retain their enclosing function, loop, or state-field
  boundary inside nested expressions.

# 2026-08-20: string match patterns

- Made decoded string literals first-class [`match`] patterns with ordinary
  `String` inference, duplicate detection, guards, and wildcard-based
  exhaustiveness for the open string domain.
- Lowered matching to the shared content-equality helper rather than GC
  reference identity, including statement-level matches produced when an arm
  suspends.
- Replaced the Crazy Machines alternate-layout audit workaround with direct
  `match process.name()` dispatch and documented exact process-identity
  selection for ASL ports.

# 2026-08-23: semantic document and type navigation

- Added compiler-owned document highlights for source declarations and every
  resolved occurrence, preserving read and write classification from the
  existing value-reference analysis.
- Added inferred type-definition navigation to source structs, enums, state and
  settings declarations, and canonical documentation pages for built-in and
  standard-library types.
- Kept the LSP as a conversion layer over the shared database products and
  advertised both native protocol capabilities to editor clients.

# 2026-08-24: protocol-aware usage and method receivers

- Unified implicit display recognition for interpolation, string casts,
  `print`, and custom variables at the typed-HIR boundary, then made unused
  declaration reachability follow both custom capability methods and fields
  read by compiler-derived aggregate formatting.
- Made `self` a documented keyword with semantic highlighting and concrete
  receiver-type hover, while ensuring compiler-provided values without source
  declarations cannot leave dangling source-definition references that block
  language documentation.

# 2026-08-24: shortest floating-point Display and Debug

- Made `f32` and `f64` implement `Debug` and therefore `Display`, including
  recursive formatting inside structs, arrays, options, results, and the other
  conditionally debug-printable containers.
- Adapted zmij's correctness-first Schubfach conversion to lazily emitted
  Wasm-GC runtime helpers. The power-of-ten data and width-specific formatter
  are included only when reachable float formatting needs them, with no new
  host ABI and no approximate intermediate decimal conversion.
- Covered signed zero, subnormals, finite boundaries, infinities, NaN, and
  deterministic sampled bit patterns against `zmij::Buffer` at runtime.

# 2026-08-29: order-independent state dependencies

- Made sibling state-field references resolve within the active physical
  layout in both ordinary field expressions and dynamic `at` bases.
- Added a compiler-owned dependency graph with stable topological evaluation
  and multi-label cycle diagnostics, without making source declaration order
  semantic.
- Propagated failed dependencies at the candidate-snapshot boundary: dependent
  fields are skipped and retain their accepted values instead of dereferencing
  stale candidate addresses, while independent siblings still advance.

# 2026-08-29: canonical identifier syntax

- Centralized the ASCII letter/underscore identifier-start and
  alphanumeric/underscore continuation rules in the shared syntax crate for
  lexing, completion, and rename validation.
- Removed the accidental acceptance of `$` in source identifiers. It remains
  ordinary content in strings and template strings, including process names
  such as `NO$GBA.EXE`.

# 2026-08-29: explicit dollar intent in template interpolation

- Added warning `SS1008` for JavaScript-style `${value}` in backtick strings,
  explaining that SplitScript interpolation starts with `{` and that the
  unescaped dollar would otherwise be rendered literally.
- Offered machine-applicable fixes for both possible intentions: remove `$` for
  `{value}`, or escape it as `\${value}` when the rendered text needs the dollar
  before the interpolated value. Ordinary dollar text remains warning-free.

# 2026-08-30: identity-safe VS Code saves

- Made release builds and debug watch retain the SplitScript document selected
  when the command starts instead of rereading the editor that happens to be
  focused after saving completes.
- Followed the exact URI returned by VS Code's `workspace.save` and
  `workspace.saveAs` APIs. Untitled Save As can replace the original document;
  the controller opens and revalidates that returned resource, including its
  language, saved state, and URI, before taking a compiler snapshot.
- Added host-neutral tests for focus changes during save, untitled document
  replacement, cancelled saves, and a Save As target with the wrong language.

# 2026-08-30: bounded documentation page caching

- Derived one immutable finite route set from the compiler-owned documentation
  index plus its root page, and reject every noncanonical virtual-document URI
  before allocating a page-cache entry.
- Kept lazy independent rendering for valid pages while proving that one
  thousand distinct invalid requests retain no cache state and that every
  indexed route still renders through the validated documentation graph.

# 2026-08-30: compiler-owned same-name process selection

- Added the synchronous `selectProcess` lifecycle block before provider setup
  and `onAttach`. Each candidate is exposed through the ordinary native
  `process` value; `true` promotes it, while `false` and fallthrough detach it.
- Reused the ordinary `Result<bool>` compiler representation as the block's
  implicit failure boundary. Postfix `?` and `throw` therefore reject only the
  current candidate without a selector-specific propagation mechanism;
  `None` remains an ordinary optional value and is not accepted as a decision.
- Lowered candidate discovery through the official two-pass
  `process_list_by_name` and `process_attach_by_pid` ABI. Dynamic staging grows
  to the complete list, process-set growth rejects the partial snapshot, and
  candidate order is explicitly unspecified. Scripts without a selector keep
  the original direct name-attachment code and imports.
- Kept PIDs, raw handles, enumeration storage, and detach ownership out of the
  language. Provider roots, layout values, snapshots, and attachment globals
  remain unavailable until a candidate has been promoted.
- Added native and Unity-backed compiler coverage plus a runtime fixture where
  `false`, a propagated read error, and `true` candidates are tried in order;
  only the accepted process proceeds into state polling and timer decisions.

# 2026-08-31: fallible attachment initialization

- Made `onAttach` an implicit ordinary error boundary without changing its
  successful `None`, `Layout`, or `StateLayout` result. Postfix `?` and
  `throw` now reject an acquired process through the same typed failure
  transfer used by state fields, functions, retries, and `selectProcess`.
- Retain a rejected process handle inert until that exact instance closes,
  preventing immediate rediscovery and repeated initialization every tick.
  Successful layout and attachment-global paths keep their existing definite
  initialization analysis; rejection paths are terminal and need not invent
  values.
- Made the attachment readiness flag authoritative for lifecycle delivery.
  `onDetach` now runs only after complete provider preparation and successful
  `onAttach`; rejection, pending initialization, and process-lifetime
  cancellation clear their state without synthesizing a detach event.
- Documented the error boundary, retained-process behavior, and lifecycle
  ordering in the compiler-owned language reference and guide, with runtime
  coverage for read propagation, explicit throws, pending cancellation,
  explicit layouts, attachment globals, implicit initialization, and ordinary
  successful detach.

# 2026-08-31: demand-driven error payload construction

- Added a whole-program failure-payload demand pass after reachability. Result
  status and value fields keep their stable representation, while an
  unobserved error field is populated with null instead of allocating and
  embedding a message.
- Joined demand across existing result identities and propagation edges rather
  than cloning functions per call site. If any reachable caller observes a
  shared function's error, that function keeps the payload for every caller.
- Counted `Err` bindings, result equality, derived formatting, `?` forwarding,
  and managed-result conversion as observers. Source error expressions with
  possible effects still execute even when their produced string is discarded.
- Deferred a user-visible structured error identity to the lower-priority
  roadmap; payload erasure does not require or prejudge that language design.

# 2026-09-01: catalog-guided state-provider diagnostics

- Made unknown state providers list every currently supported provider from
  the compiler-owned standard-library catalog rather than maintaining a second
  diagnostic list.
- Added machine-applicable spelling repairs for unambiguous single edits, case
  mistakes, and adjacent transpositions. Distinct unsupported platform names
  such as `SNES` remain unsupported names rather than being misleadingly
  rewritten to a nearby provider.
- Added a generated state-provider index and a generic compact diagnostic
  documentation target shared by the LSP, CLI, and embedded compiler service.
  Migration topics and direct reference pages remain distinct while occupying
  one optional pointer in parser diagnostics.

# 2026-09-01: statement-shaped match-arm guidance

- Recognized assignments and side-effecting `if` chains written directly after
  a match arm's `=>` at the parser's existing expression/statement boundary.
  Valid value expressions, including `if` expressions with a final `else`,
  retain their ordinary grammar.
- Replaced the misleading missing-comma or missing-`else` cascade with one
  diagnostic explaining that statement bodies need braces.
- Added a machine-applicable whole-body rewrite to `pattern => { ... }` that
  preserves nested delimiters and multiline source, plus recovery coverage
  proving neighboring arms survive and the fixed program compiles.

# 2026-09-01: explicit bare-global lifetime boundaries

- Changed the missing lifecycle-initializer diagnostic to require a direct
  assignment in exactly one `onAttach` or `onStart` block, matching the
  compiler's existing lifetime classification.
- Explained that assignments performed inside a called helper do not establish
  a bare global's lifetime. Helpers can still use already classified scoped
  globals; this guidance does not promise interprocedural definite
  initialization.
- Kept the existing attachment-versus-attempt conflict labels and subsequent
  per-path initialization analysis unchanged, with a regression for the
  formerly confusing helper-assignment case.

# 2026-09-02: cancellable post-attachment discovery

- Allowed `whileAttached` to use `await` and `retry` through the same typed
  continuation lowering as `onAttach`, with disjoint action slots in one
  process-lifetime frame rather than a scan-specific task abstraction.
- Retained exactly one invocation per attachment. It is polled once after each
  state refresh; pending skips every timer-decision action, completion applies
  the existing Boolean gate, and the next invocation starts on the following
  update.
- Cancelled and discarded a pending invocation on process closure. A later
  attachment always starts from the action entry with fresh locals and nested
  future state.
- Kept signature scan budgets unchanged and cooperative. Runtime coverage uses
  the Metal Slug 3 shape: validate a stored address, rescan after the allocation
  moves without process restart, assign the replacement, and return `false` so
  decisions cannot observe the completion update's older state snapshot.
- Documented the lifecycle, state-replacement timing, and canonical division:
  ordinary one-time discovery belongs in `onAttach`; suspending
  `whileAttached` is for resources that can move while the process stays open.

# 2026-09-11: runtime-varying watched values

- Evaluated the FNaF Security Breach interactible watchers, which replace two
  dynamically typed ASL watcher slots with Boolean, integer, floating-point,
  and multi-value readers selected by the current interaction.
- Kept this out of the language surface: one ordinary source enum with
  semantically named payload variants is the canonical statically typed state
  value. Struct payloads cover cases that read several related values.
- A selected match arm performs only its typed reads; failure retains the last
  accepted complete enum value, success atomically advances it, and variant
  changes remain visible through ordinary `old` / `current` matching. Separate
  optional payload fields would admit invalid or independently stale states,
  while a dynamic watcher type would add no required expressiveness.
- Added no redundant feature-specific test because payload enums, lazy match
  evaluation, persistent expression-backed fields, and snapshot transitions
  are already independently covered by their owning compiler/runtime tests.

# 2026-09-13: capability-driven indexing

- Added catalog-defined `Index` and `IndexAssign` capabilities with inherited
  `Key` and `Value` associated types. Arrays and maps implement the same
  protocol, while source structs participate through exact ordinary `at` and
  `set` methods without `impl` syntax.
- Kept the receiver implicit in every capability declaration as `Self`, with
  super-capabilities written after `:`. Associated types inherited through the
  capability graph now have one validated owner and one projection path.
- Generalized bracket reads, writes, and compound assignments while retaining
  direct allocation-free array lowering. Untyped helpers infer the minimal
  `T: Index` contract together with `T.Key` and `T.Value`.
- Centralized method-receiver traversal in Wasm IR so capability calls,
  ordinary methods, intrinsics, library overloads, and managed operations use
  the same child order during async normalization and code generation.
