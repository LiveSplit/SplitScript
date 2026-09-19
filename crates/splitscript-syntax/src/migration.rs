//! Structured knowledge for migrating autosplitters from familiar languages.
//!
//! This catalog deliberately does not redeclare SplitScript's language or
//! standard-library APIs. Instead, it maps foreign concepts and spellings to
//! canonical catalog names that the compiler validates against the active
//! language and standard-library graphs.

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct MigrationConceptId(&'static str);

impl MigrationConceptId {
    pub const fn new(value: &'static str) -> Self {
        Self(value)
    }

    pub const fn as_str(self) -> &'static str {
        self.0
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct MigrationDiagnosticId(&'static str);

impl MigrationDiagnosticId {
    pub const fn new(value: &'static str) -> Self {
        Self(value)
    }

    pub const fn as_str(self) -> &'static str {
        self.0
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum SourceLanguage {
    Asl,
    CSharp,
    JavaScript,
    Rust,
}

impl SourceLanguage {
    pub const fn name(self) -> &'static str {
        match self {
            Self::Asl => "ASL",
            Self::CSharp => "C#",
            Self::JavaScript => "JavaScript",
            Self::Rust => "Rust",
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum MigrationSupport {
    Direct,
    TypedPattern,
    Planned,
    SandboxNonGoal,
}

impl MigrationSupport {
    pub const fn label(self) -> &'static str {
        match self {
            Self::Direct => "Supported directly",
            Self::TypedPattern => "Use a typed pattern",
            Self::Planned => "Planned",
            Self::SandboxNonGoal => "Intentional sandbox non-goal",
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum MigrationTarget {
    Language(&'static str),
    StandardLibraryType(&'static str),
    StandardLibraryItem(&'static str),
    StateProvider(&'static str),
}

impl MigrationTarget {
    pub const fn display(self) -> &'static str {
        match self {
            Self::Language(name)
            | Self::StandardLibraryType(name)
            | Self::StandardLibraryItem(name)
            | Self::StateProvider(name) => name,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum ForeignSpellingContext {
    VariableDeclaration,
    VariableModifier,
    OptionalValue,
    FunctionDeclaration,
    StructDeclaration,
    Type,
    StaticTypeReceiver,
    Method,
    ValuePath,
    AttachedProcessValuePath,
    Operator,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ForeignSpellingReplacement {
    Text(&'static str),
    Remove,
}

impl ForeignSpellingReplacement {
    pub const fn text(self) -> &'static str {
        match self {
            Self::Text(value) => value,
            Self::Remove => "",
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ForeignSpelling {
    pub source: SourceLanguage,
    pub context: ForeignSpellingContext,
    pub spelling: &'static str,
    pub replacement: ForeignSpellingReplacement,
    pub message: &'static str,
    pub primary_label: &'static str,
    pub fix_title: &'static str,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct MigrationConcept {
    pub id: MigrationConceptId,
    pub name: &'static str,
    pub sources: &'static [SourceLanguage],
    pub support: MigrationSupport,
    pub summary: &'static str,
    pub targets: &'static [MigrationTarget],
    pub cookbook_anchor: Option<&'static str>,
    pub spellings: &'static [ForeignSpelling],
}

/// A documentation-search phrase that names a migration concept without being
/// source syntax itself.
///
/// Keep these separate from [`ForeignSpelling`]: query aliases may include a
/// source-language qualifier or several representative tokens, and must never
/// accidentally become compiler rewrite candidates.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct MigrationQueryAliases {
    pub concept: MigrationConceptId,
    pub queries: &'static [&'static str],
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct MigrationDiagnostic {
    pub id: MigrationDiagnosticId,
    pub concept: MigrationConceptId,
    pub message: &'static str,
    pub primary_label: &'static str,
    pub notes: &'static [&'static str],
}

pub const ASL_STRING_N_FIELD_DIAGNOSTIC: MigrationDiagnosticId =
    MigrationDiagnosticId::new("asl.state.string-n-field");
pub const DUPLICATE_STATE_DIAGNOSTIC: MigrationDiagnosticId =
    MigrationDiagnosticId::new("asl.state.duplicate-version-state");
pub const ASL_STARTUP_DIAGNOSTIC: MigrationDiagnosticId =
    MigrationDiagnosticId::new("asl.lifecycle.startup-block");
pub const ASL_INIT_DIAGNOSTIC: MigrationDiagnosticId =
    MigrationDiagnosticId::new("asl.lifecycle.init-block");
pub const ASL_UPDATE_DIAGNOSTIC: MigrationDiagnosticId =
    MigrationDiagnosticId::new("asl.lifecycle.update-block");
pub const ASL_EXIT_DIAGNOSTIC: MigrationDiagnosticId =
    MigrationDiagnosticId::new("asl.lifecycle.exit-block");
pub const ASL_SHUTDOWN_DIAGNOSTIC: MigrationDiagnosticId =
    MigrationDiagnosticId::new("asl.lifecycle.shutdown-block");
pub const ASL_TIMER_EVENT_DIAGNOSTIC: MigrationDiagnosticId =
    MigrationDiagnosticId::new("asl.lifecycle.timer-event-block");
pub const ASL_TIMER_PHASE_DIAGNOSTIC: MigrationDiagnosticId =
    MigrationDiagnosticId::new("asl.timer.phase-type");
pub const ASL_CURRENT_SPLIT_INDEX_DIAGNOSTIC: MigrationDiagnosticId =
    MigrationDiagnosticId::new("asl.timer.current-split-index-path");
pub const ASL_MONOTONIC_TIME_DIAGNOSTIC: MigrationDiagnosticId =
    MigrationDiagnosticId::new("asl.time.monotonic-path");
pub const ASL_TIMER_REAL_TIME_DIAGNOSTIC: MigrationDiagnosticId =
    MigrationDiagnosticId::new("asl.timer.current-real-time-path");
pub const ASL_TIMER_GAME_TIME_DIAGNOSTIC: MigrationDiagnosticId =
    MigrationDiagnosticId::new("asl.timer.current-game-time-path");
pub const ASL_TIMER_RUN_METADATA_DIAGNOSTIC: MigrationDiagnosticId =
    MigrationDiagnosticId::new("asl.timer.run-metadata-path");
pub const ASL_TIMER_CONTROL_DIAGNOSTIC: MigrationDiagnosticId =
    MigrationDiagnosticId::new("asl.timer.control-path");

pub const ASL_MEMORY_WATCHER_LIST_DIAGNOSTIC: MigrationDiagnosticId =
    MigrationDiagnosticId::new("asl.state.memory-watcher-list-type");
pub const ASL_TASK_RUN_DIAGNOSTIC: MigrationDiagnosticId =
    MigrationDiagnosticId::new("asl.async.task-run-call");
pub const ASL_SETTINGS_LOOKUP_DIAGNOSTIC: MigrationDiagnosticId =
    MigrationDiagnosticId::new("asl.settings.dynamic-lookup");
pub const ASL_SETTINGS_ADD_DIAGNOSTIC: MigrationDiagnosticId =
    MigrationDiagnosticId::new("asl.settings.add-call");
pub const ASL_MODULES_MAIN_DIAGNOSTIC: MigrationDiagnosticId =
    MigrationDiagnosticId::new("asl.process.modules-main-call");
pub const ASL_MODULES_QUERY_DIAGNOSTIC: MigrationDiagnosticId =
    MigrationDiagnosticId::new("asl.process.modules-query-call");
pub const ASL_MODULES_ENUMERATION_DIAGNOSTIC: MigrationDiagnosticId =
    MigrationDiagnosticId::new("asl.process.modules-enumeration");
pub const CSHARP_STRING_EQUALS_DIAGNOSTIC: MigrationDiagnosticId =
    MigrationDiagnosticId::new("csharp.string.equals-call");
pub const CSHARP_STRING_SUBSTRING_DIAGNOSTIC: MigrationDiagnosticId =
    MigrationDiagnosticId::new("csharp.string.substring-call");
pub const CSHARP_STRING_INDEX_OF_DIAGNOSTIC: MigrationDiagnosticId =
    MigrationDiagnosticId::new("csharp.string.index-of-call");
pub const CSHARP_STRING_LAST_INDEX_OF_DIAGNOSTIC: MigrationDiagnosticId =
    MigrationDiagnosticId::new("csharp.string.last-index-of-call");
pub const CSHARP_STRING_REPLACE_DIAGNOSTIC: MigrationDiagnosticId =
    MigrationDiagnosticId::new("csharp.string.replace-call");
pub const CSHARP_STRING_TRIM_DIAGNOSTIC: MigrationDiagnosticId =
    MigrationDiagnosticId::new("csharp.string.trim-call");
pub const CSHARP_STRING_PADDING_DIAGNOSTIC: MigrationDiagnosticId =
    MigrationDiagnosticId::new("csharp.string.padding-call");
pub const CSHARP_STRING_IS_NULL_OR_EMPTY_DIAGNOSTIC: MigrationDiagnosticId =
    MigrationDiagnosticId::new("csharp.string.is-null-or-empty-call");
pub const CSHARP_STRING_IS_NULL_OR_WHITE_SPACE_DIAGNOSTIC: MigrationDiagnosticId =
    MigrationDiagnosticId::new("csharp.string.is-null-or-white-space-call");
pub const CSHARP_STRING_LENGTH_DIAGNOSTIC: MigrationDiagnosticId =
    MigrationDiagnosticId::new("csharp.string.length-property");
pub const CSHARP_ARRAY_LENGTH_DIAGNOSTIC: MigrationDiagnosticId =
    MigrationDiagnosticId::new("csharp.array.length-property");
pub const CSHARP_COLLECTION_COUNT_DIAGNOSTIC: MigrationDiagnosticId =
    MigrationDiagnosticId::new("csharp.collection.count-property");
pub const CSHARP_STRING_JOIN_DIAGNOSTIC: MigrationDiagnosticId =
    MigrationDiagnosticId::new("csharp.string.join-call");
pub const CSHARP_NUMERIC_PARSE_DIAGNOSTIC: MigrationDiagnosticId =
    MigrationDiagnosticId::new("csharp.numeric.static-parse-call");
pub const CSHARP_CONVERT_INTEGER_DIAGNOSTIC: MigrationDiagnosticId =
    MigrationDiagnosticId::new("csharp.convert.integer-call");
pub const CSHARP_CONVERT_FLOAT_DIAGNOSTIC: MigrationDiagnosticId =
    MigrationDiagnosticId::new("csharp.convert.float-call");
pub const CSHARP_CONVERT_BOOLEAN_DIAGNOSTIC: MigrationDiagnosticId =
    MigrationDiagnosticId::new("csharp.convert.boolean-call");
pub const CSHARP_CONVERT_STRING_DIAGNOSTIC: MigrationDiagnosticId =
    MigrationDiagnosticId::new("csharp.convert.string-call");
pub const CSHARP_SQUARE_ROOT_DIAGNOSTIC: MigrationDiagnosticId =
    MigrationDiagnosticId::new("csharp.math.square-root-call");
pub const CSHARP_TRUNCATE_DIAGNOSTIC: MigrationDiagnosticId =
    MigrationDiagnosticId::new("csharp.math.truncate-call");
pub const CSHARP_ROUND_DIAGNOSTIC: MigrationDiagnosticId =
    MigrationDiagnosticId::new("csharp.math.round-call");
pub const CSHARP_FLOOR_DIAGNOSTIC: MigrationDiagnosticId =
    MigrationDiagnosticId::new("csharp.math.floor-call");
pub const CSHARP_CEILING_DIAGNOSTIC: MigrationDiagnosticId =
    MigrationDiagnosticId::new("csharp.math.ceiling-call");
pub const CSHARP_MIN_DIAGNOSTIC: MigrationDiagnosticId =
    MigrationDiagnosticId::new("csharp.math.min-call");
pub const CSHARP_MAX_DIAGNOSTIC: MigrationDiagnosticId =
    MigrationDiagnosticId::new("csharp.math.max-call");
pub const CSHARP_ABS_DIAGNOSTIC: MigrationDiagnosticId =
    MigrationDiagnosticId::new("csharp.math.abs-call");
pub const CSHARP_POWER_DIAGNOSTIC: MigrationDiagnosticId =
    MigrationDiagnosticId::new("csharp.math.power-call");
pub const CSHARP_TIMESPAN_PARSE_DIAGNOSTIC: MigrationDiagnosticId =
    MigrationDiagnosticId::new("csharp.timespan.parse-call");
pub const CSHARP_TIMESPAN_TICKS_DIAGNOSTIC: MigrationDiagnosticId =
    MigrationDiagnosticId::new("csharp.timespan.from-ticks-call");
pub const ASL_UNITY_SCHEMA_DIAGNOSTIC: MigrationDiagnosticId =
    MigrationDiagnosticId::new("asl.unity.managed-schema");

pub const DIAGNOSTICS: &[MigrationDiagnostic] = &[
    MigrationDiagnostic {
        id: ASL_STRING_N_FIELD_DIAGNOSTIC,
        concept: MigrationConceptId::new("asl.state.string-n"),
        message: "ASL `stringN` fields need an explicit SplitScript memory decoder",
        primary_label: "this ASL pseudo-type combines a byte bound with automatic string decoding",
        notes: &[
            "ASL `stringN` reads N bytes, then auto-detects UTF-16LE from the second byte; SplitScript requires the encoding to be chosen from evidence",
            "`utf8` bounds are bytes, while `utf16le` bounds are two-byte code units; both suggested rewrites are intentionally maybe-incorrect",
        ],
    },
    MigrationDiagnostic {
        id: DUPLICATE_STATE_DIAGNOSTIC,
        concept: MigrationConceptId::new("asl.state.version-label"),
        message: "SplitScript uses one `state` declaration with conditional fields for game versions",
        primary_label: "merge this declaration and guard version-specific fields with an enum global",
        notes: &[
            "initialize the enum global from reliable version evidence in `onAttach` before polling begins",
            "compatible conditional fields form a common interface; test or match the enum global before accessing missing or conflicting fields",
            "merging is not automatic because versioned fields and pointer paths may differ semantically",
        ],
    },
    MigrationDiagnostic {
        id: ASL_STARTUP_DIAGNOSTIC,
        concept: MigrationConceptId::new("asl.lifecycle.startup"),
        message: "ASL `startup` is not a SplitScript lifecycle block",
        primary_label: "split startup work according to what it initializes",
        notes: &[
            "declare controls in `settings` and constants with global `let` declarations",
            "put remaining process-independent startup statements in `setup`; it runs once after settings are initialized",
            "do not move process discovery into `setup`; use `onAttach` for each selected process",
            "legacy timer event subscriptions are a separate migration concern and do not become `start`, `split`, or `reset` decision blocks",
        ],
    },
    MigrationDiagnostic {
        id: ASL_INIT_DIAGNOSTIC,
        concept: MigrationConceptId::new("asl.lifecycle.init"),
        message: "ASL `init` has no blind one-to-one lifecycle rename",
        primary_label: "choose the destination from the state this block needs",
        notes: &[
            "use `onAttach` for suspending process discovery and attachment-shape initialization before SplitScript starts polling",
            "use synchronous `onStateReady` for initialization that consumes the first complete snapshot; `old` and `current` are equal there",
        ],
    },
    MigrationDiagnostic {
        id: ASL_UPDATE_DIAGNOSTIC,
        concept: MigrationConceptId::new("asl.lifecycle.update"),
        message: "ASL `update` is named `whileAttached` for ordinary per-tick work",
        primary_label: "review the block's boolean control result before moving it",
        notes: &[
            "`whileAttached` runs after a successful state refresh and before timer-decision blocks",
            "an explicit `return false` skips all remaining timer decisions for that update; fallthrough, bare return, and true continue normally",
        ],
    },
    MigrationDiagnostic {
        id: ASL_EXIT_DIAGNOSTIC,
        concept: MigrationConceptId::new("asl.lifecycle.exit"),
        message: "ASL `exit` is named `onDetach`",
        primary_label: "use the process-closure lifecycle boundary",
        notes: &[
            "`onDetach` runs exactly once after a previously attached process closes and never at initial detached startup",
            "the closed process and state snapshots are unavailable; use `setup` for initial process-independent policy",
        ],
    },
    MigrationDiagnostic {
        id: ASL_SHUTDOWN_DIAGNOSTIC,
        concept: MigrationConceptId::new("asl.lifecycle.shutdown"),
        message: "ASL `shutdown` has no SplitScript equivalent yet",
        primary_label: "script teardown requires a host lifecycle callback",
        notes: &[
            "do not use `onDetach`: it runs for process transitions rather than when the script is disabled, reloaded, or dropped",
            "the runtime evolution plan records the required teardown export and ordering contract",
        ],
    },
    MigrationDiagnostic {
        id: ASL_TIMER_EVENT_DIAGNOSTIC,
        concept: MigrationConceptId::new("asl.timer.events"),
        message: "ASL `onSplit` has no exact SplitScript equivalent yet",
        primary_label: "split observation needs ordered host event data",
        notes: &[
            "SplitScript supports sampled `onStart` and `onReset` actions directly, including while detached",
            "an exact `onSplit` must distinguish splits, skips, and undos that can occur between two updates",
            "the runtime evolution plan records the required ordered host event contract",
        ],
    },
    MigrationDiagnostic {
        id: ASL_TIMER_PHASE_DIAGNOSTIC,
        concept: MigrationConceptId::new("asl.timer.state"),
        message: "ASL `TimerPhase` is `TimerState` in SplitScript",
        primary_label: "use the typed timer state and its named variants",
        notes: &[
            "read the current value with `timer.state()` and compare it with `TimerState.NotRunning`, `Running`, `Paused`, or `Ended`",
            "SplitScript also exposes `TimerState.Unknown` so an unrecognized future host value does not masquerade as a known state",
            "do not preserve numeric comparisons or ordering on the legacy enum; match the states whose behavior the script actually needs",
        ],
    },
    MigrationDiagnostic {
        id: ASL_CURRENT_SPLIT_INDEX_DIAGNOSTIC,
        concept: MigrationConceptId::new("asl.timer.current-split-index"),
        message: "ASL `timer.CurrentSplitIndex` is optional in SplitScript",
        primary_label: "call `timer.currentSplitIndex()` and handle the no-attempt case",
        notes: &[
            "`timer.currentSplitIndex()` returns `u64?`; `None` represents every negative host value when no attempt is in progress",
            "a decision block commonly uses `let index = timer.currentSplitIndex() else return false`, while other contexts can use `match`",
            "the index also advances for skipped segments and equals the segment count after the final split",
        ],
    },
    MigrationDiagnostic {
        id: ASL_MONOTONIC_TIME_DIAGNOSTIC,
        concept: MigrationConceptId::new("asl.time.monotonic-delay"),
        message: "use SplitScript's monotonic clock for elapsed-time checks",
        primary_label: "capture `Instant.now()` at the event and compare an exact `Duration`",
        notes: &[
            "`Instant` is appropriate for debouncing, cooldowns, and delayed actions because it never moves backwards during one runtime instance",
            "it has no calendar value; logging timestamps and other wall-clock uses are intentionally outside this API",
            "use `startedAt.hasElapsed(Duration.fromMilliseconds(...))` for the common polling pattern",
        ],
    },
    MigrationDiagnostic {
        id: ASL_TIMER_REAL_TIME_DIAGNOSTIC,
        concept: MigrationConceptId::new("asl.timer.current-real-time"),
        message: "LiveSplit run real time is not the same as SplitScript's monotonic clock",
        primary_label: "determine whether this code needs an independent delay or actual timer metadata",
        notes: &[
            "for a delay anchored to a game event, store `Instant.now()` at that event and compare a `Duration`",
            "the exact LiveSplit `CurrentTime.RealTime` phase is not exposed by the current host contract",
            "do not silently replace run-relative time used for offsets or game-time calculations with elapsed process-independent time",
        ],
    },
    MigrationDiagnostic {
        id: ASL_TIMER_GAME_TIME_DIAGNOSTIC,
        concept: MigrationConceptId::new("asl.timer.current-game-time"),
        message: "LiveSplit's current game time is not readable through the current host contract",
        primary_label: "determine whether this is the value produced by this script or external timer state",
        notes: &[
            "when the script computes this value, keep the typed `Duration` in script-owned state and return it from `gameTime` instead of reading it back from LiveSplit",
            "game time may be absent and may also be changed by the host or another component, so a future read API must return a coherent optional timer snapshot",
            "do not substitute `Instant`: monotonic elapsed time does not pause for loads or represent LiveSplit's game-time clock",
        ],
    },
    MigrationDiagnostic {
        id: ASL_TIMER_RUN_METADATA_DIAGNOSTIC,
        concept: MigrationConceptId::new("asl.timer.run-metadata"),
        message: "LiveSplit run and segment metadata is not exposed by the current host contract",
        primary_label: "this value needs a typed read-only timer snapshot from the host",
        notes: &[
            "current segment names, route length, game/category names, and the splits-file path are host-owned metadata rather than process memory",
            "do not duplicate a route name table in source unless the maintained autosplitter intentionally owns that fixed route",
            "the runtime evolution plan requires optional current-segment data and one coherent snapshot per update before these paths can be supported faithfully",
        ],
    },
    MigrationDiagnostic {
        id: ASL_TIMER_CONTROL_DIAGNOSTIC,
        concept: MigrationConceptId::new("asl.timer.controlled-mutation"),
        message: "LiveSplit run offset and timing-method control needs an explicit host contract",
        primary_label: "this is user-visible timer configuration, not ordinary script state",
        notes: &[
            "the current host does not expose run-offset or timing-method reads or writes to WebAssembly autosplitters",
            "a future API must define ordering with timer decisions, reset and undo behavior, persistence, precision, and how concurrent UI changes are resolved",
            "do not silently drop this operation or replace it with `setVariable`; record the port as behavior-limited until the host contract exists",
        ],
    },
    MigrationDiagnostic {
        id: ASL_MEMORY_WATCHER_LIST_DIAGNOSTIC,
        concept: MigrationConceptId::new("asl.state.memory-watcher-list"),
        message: "ASL `MemoryWatcherList` has no single collection-shaped replacement",
        primary_label: "choose the representation from how this watcher list is populated",
        notes: &[
            "declare a fixed set of named reads in `state`; SplitScript updates the snapshot transactionally and exposes `old` and `current`",
            "when runtime discovery produces a homogeneous set of addresses, retain those addresses in `[address]` and perform typed reads while iterating",
            "managed list or dictionary enumeration is a Unity provider gap rather than an array spelling; keep that requirement explicit instead of guessing object offsets",
            "there is no automatic rewrite because `MemoryWatcherList` is used for several materially different ownership and discovery patterns",
        ],
    },
    MigrationDiagnostic {
        id: ASL_TASK_RUN_DIAGNOSTIC,
        concept: MigrationConceptId::new("asl.async.task-run"),
        message: "`Task.Run` threads do not belong in SplitScript's cooperative execution model",
        primary_label: "express the operation's suspension or retry boundary directly",
        notes: &[
            "use `await` for intrinsically asynchronous discovery such as attaching, module loading, and incremental scans",
            "use `retry expression` or `retry { ... }` for bounded synchronous fallible work that should be attempted again on the next tick",
            "autosplitter code must yield predictably so one background operation cannot make timer updates appear hung",
            "there is no automatic rewrite because a thread body may mix discovery, polling, mutation, and host operations with different cooperative equivalents",
        ],
    },
    MigrationDiagnostic {
        id: ASL_SETTINGS_LOOKUP_DIAGNOSTIC,
        concept: MigrationConceptId::new("asl.settings.dynamic-lookup"),
        message: "ASL settings-map lookup uses typed `SettingsView` operations in SplitScript",
        primary_label: "this value is a settings view, not a general indexable map",
        notes: &[
            "read a statically named declaration with `settings.name` so its type remains visible to inference and editor tooling",
            "use `settings.enabled(key)` when a computed string selects a declared boolean setting",
            "use `settings.contains(key)` when declaration membership must be distinguished from a disabled boolean value",
        ],
    },
    MigrationDiagnostic {
        id: ASL_SETTINGS_ADD_DIAGNOSTIC,
        concept: MigrationConceptId::new("asl.settings.registration"),
        message: "ASL `settings.Add` calls become declarations in SplitScript",
        primary_label: "settings are declared statically rather than registered at runtime",
        notes: &[
            "declare one boolean setting as `\"Label\" => name key \"host-key\": true,` inside `settings`",
            "for a bounded numbered family, use `for value in start..=end { `Label {value}` key `{value}`: true, }` instead of expanding every member by hand",
            "read a statically named setting through `settings.name`; use `settings.enabled(key)` only when a data-driven string selects among declared boolean settings",
            "there is no automatic rewrite because `settings.Add` overloads encode labels, parents, defaults, and runtime control flow differently",
        ],
    },
    MigrationDiagnostic {
        id: ASL_MODULES_MAIN_DIAGNOSTIC,
        concept: MigrationConceptId::new("asl.process.modules"),
        message: "ASL `modules.First()` usually means the attached executable module",
        primary_label: "discover the main module explicitly in `onAttach`",
        notes: &[
            "use `let executable = await process.mainModule()` and then read `executable.address` or `executable.size`",
            "use `executable.fileVersion()`, `productVersion()`, or `versionInfo()` for typed Windows executable identity",
            "if the source intentionally selects a named non-main module, use `process.loadedModule(name)` for an optional probe or await `process.module(name)` when attachment must wait for it",
        ],
    },
    MigrationDiagnostic {
        id: ASL_MODULES_QUERY_DIAGNOSTIC,
        concept: MigrationConceptId::new("asl.process.modules"),
        message: "ASL module-list queries need an intent-specific SplitScript probe",
        primary_label: "replace the collection query with explicit module discovery",
        notes: &[
            "for a known optional name, use `process.loadedModule(name) != None`; this synchronous probe does not wait for an absent platform or mod-loader module",
            "for a known required name, use `await process.module(name)` in `onAttach`; the operation suspends until that module is loaded",
            "use `await process.mainModule()` for the executable itself, including size, path, signature, and typed version checks",
            "a query whose predicate cannot be reduced to a known name genuinely requires full module enumeration, which is tracked as a host-runtime requirement and is not currently exposed",
        ],
    },
    MigrationDiagnostic {
        id: ASL_MODULES_ENUMERATION_DIAGNOSTIC,
        concept: MigrationConceptId::new("asl.process.modules"),
        message: "ASL's enumerable `modules` collection has no direct SplitScript value",
        primary_label: "choose a typed module probe from the source code's actual intent",
        notes: &[
            "use `await process.mainModule()` for the executable, `process.loadedModule(name)` for a known optional module, or `await process.module(name)` for a known required module",
            "use typed module size, path, version, or signature evidence when the source is identifying a build rather than enumerating modules for its own sake",
            "full unknown-name module enumeration remains a host-runtime requirement; do not substitute mapped memory ranges because mappings and loaded modules have different semantics",
        ],
    },
    MigrationDiagnostic {
        id: CSHARP_STRING_EQUALS_DIAGNOSTIC,
        concept: MigrationConceptId::new("string.equality"),
        message: "C# `String.Equals` becomes an equality expression in SplitScript",
        primary_label: "compare string values with `==` or `!=`",
        notes: &[
            "`left == right` compares immutable strings by their exact UTF-8 text, not by object identity",
            "use `left.equalsIgnoreAsciiCase(right)` only when the game identifier intentionally ignores ASCII letter case",
            "there is no automatic rewrite because C# also has static and comparison-mode overloads whose semantics must be reviewed",
        ],
    },
    MigrationDiagnostic {
        id: CSHARP_STRING_SUBSTRING_DIAGNOSTIC,
        concept: MigrationConceptId::new("string.substring"),
        message: "C# `String.Substring` needs an explicit UTF-8 boundary review",
        primary_label: "SplitScript `slice` uses UTF-8 byte offsets and an exclusive end",
        notes: &[
            "for proven ASCII text, `value.Substring(start, length)` becomes `value.slice(start, start + length)` with ordinary Result handling",
            "for proven ASCII text, `value.Substring(start)` becomes `value.slice(start, value.byteLength())`",
            "C# indexes UTF-16 code units; non-ASCII offsets cannot be copied into `slice` because SplitScript indexes UTF-8 bytes and rejects positions inside a character",
            "there is no automatic rewrite because the compiler cannot prove the source text is ASCII or recover C# overload semantics from the method name alone",
        ],
    },
    MigrationDiagnostic {
        id: CSHARP_STRING_INDEX_OF_DIAGNOSTIC,
        concept: MigrationConceptId::new("string.index-of"),
        message: "C# `String.IndexOf` needs an explicit index-model review",
        primary_label: "SplitScript returns an optional UTF-8 byte offset",
        notes: &[
            "rewrite an ordinal ASCII search as `text.indexOf(substring)` and handle `None` instead of comparing the result with C#'s `-1` sentinel",
            "SplitScript offsets count UTF-8 bytes; C# string offsets count UTF-16 code units, so copied arithmetic is only equivalent for proven ASCII text",
            "comparison-mode and start-index overloads need separate review; the canonical operation is exact and case-sensitive from the beginning of the string",
            "there is no automatic rewrite because changing both the index unit and absence representation can require surrounding control-flow changes",
        ],
    },
    MigrationDiagnostic {
        id: CSHARP_STRING_LAST_INDEX_OF_DIAGNOSTIC,
        concept: MigrationConceptId::new("string.last-index-of"),
        message: "C# `String.LastIndexOf` needs an explicit index-model review",
        primary_label: "SplitScript returns an optional UTF-8 byte offset",
        notes: &[
            "rewrite an ordinal ASCII search as `text.lastIndexOf(substring)` and handle `None` instead of comparing the result with C#'s `-1` sentinel",
            "SplitScript offsets count UTF-8 bytes; C# string offsets count UTF-16 code units, so copied arithmetic is only equivalent for proven ASCII text",
            "comparison-mode, start-index, and count overloads need separate review; the canonical operation is exact and case-sensitive over the complete string",
            "there is no automatic rewrite because changing both the index unit and absence representation can require surrounding control-flow changes",
        ],
    },
    MigrationDiagnostic {
        id: CSHARP_STRING_REPLACE_DIAGNOSTIC,
        concept: MigrationConceptId::new("string.replacement"),
        message: "C# `String.Replace` becomes fallible `replaceAll` in SplitScript",
        primary_label: "use the immutable replacement result and handle failure",
        notes: &[
            "for a non-empty exact search and a non-null replacement, rewrite `text.Replace(search, replacement)` as `text.replaceAll(search, replacement)`",
            "`replaceAll` returns the Result type `String!`; use `?`, `else`, or `match` according to the surrounding failure policy because an empty search or unrepresentable result length is an error",
            "C# permits a null replacement to mean deletion; SplitScript has no null string, so pass the empty string explicitly when deletion is intended",
            "there is no automatic rewrite because the compiler cannot choose the surrounding Result handling or prove that a nullable C# replacement is non-null",
        ],
    },
    MigrationDiagnostic {
        id: CSHARP_STRING_TRIM_DIAGNOSTIC,
        concept: MigrationConceptId::new("string.ascii-trim"),
        message: "C# `String.Trim` needs an explicit whitespace-model review",
        primary_label: "SplitScript trims a fixed ASCII whitespace set",
        notes: &[
            "for game identifiers, configuration lines, and log text known to use ASCII whitespace, rewrite `text.Trim()` as `text.trimAsciiWhitespace()`",
            "SplitScript removes only space, tab, line feed, vertical tab, form feed, and carriage return; C# `Trim()` recognizes a broader Unicode whitespace set",
            "character-array overloads and the related `TrimStart` and `TrimEnd` operations require separate boundary logic and do not map to this method",
            "there is no automatic rewrite because the compiler cannot prove that the input's surrounding whitespace is ASCII",
        ],
    },
    MigrationDiagnostic {
        id: CSHARP_STRING_PADDING_DIAGNOSTIC,
        concept: MigrationConceptId::new("string.padding"),
        message: "C# string padding needs an explicit width-model review",
        primary_label: "use `padStart(width, fill)` or `padEnd(width, fill)`",
        notes: &[
            "rewrite `text.PadLeft(width, fill)` as `text.padStart(width, fill)` and `text.PadRight(width, fill)` as `text.padEnd(width, fill)`",
            "SplitScript always requires one `char` fill; pass `' '` explicitly for C# overloads that omit the padding character",
            "SplitScript width counts Unicode scalar values, while C# width counts UTF-16 code units; copied widths are equivalent for proven ASCII text",
            "padding returns the original immutable string when it is already wide enough and otherwise performs one exact-sized allocation",
            "there is no automatic rewrite because direction, omitted fill, and the surrounding width assumptions need to remain visible",
        ],
    },
    MigrationDiagnostic {
        id: CSHARP_STRING_IS_NULL_OR_EMPTY_DIAGNOSTIC,
        concept: MigrationConceptId::new("string.null-or-empty"),
        message: "C# `String.IsNullOrEmpty` crosses SplitScript's Option boundary",
        primary_label: "choose emptiness or optional absence from the value's type",
        notes: &[
            "for a required `String`, rewrite `String.IsNullOrEmpty(value)` as `value.isEmpty()` because the value cannot be null",
            "for `String?`, use `match value { None => true, Some(text) => text.isEmpty() }` so absence remains explicit",
            "process and state read failures are not automatically null strings; preserve their declared Result or Option policy before checking emptiness",
            "there is no automatic rewrite because the static call does not reveal whether the migrated value should be required, optional, or fallible",
        ],
    },
    MigrationDiagnostic {
        id: CSHARP_STRING_IS_NULL_OR_WHITE_SPACE_DIAGNOSTIC,
        concept: MigrationConceptId::new("string.null-or-white-space"),
        message: "C# `String.IsNullOrWhiteSpace` crosses SplitScript's Option boundary",
        primary_label: "choose Unicode blankness or optional absence from the value's type",
        notes: &[
            "for a required `String`, rewrite `String.IsNullOrWhiteSpace(value)` as `value.isBlank()` because the value cannot be null",
            "for `String?`, use `match value { None => true, Some(text) => text.isBlank() }` so absence remains explicit",
            "`isBlank()` uses the Unicode `White_Space` property and therefore includes empty strings, ASCII whitespace, and non-ASCII whitespace such as non-breaking space",
            "process and state read failures are not automatically null strings; preserve their declared Result or Option policy before checking blankness",
            "there is no automatic rewrite because the static call does not reveal whether the migrated value should be required, optional, or fallible",
        ],
    },
    MigrationDiagnostic {
        id: CSHARP_STRING_LENGTH_DIAGNOSTIC,
        concept: MigrationConceptId::new("string.length"),
        message: "C# string `Length` has no encoding-neutral SplitScript rename",
        primary_label: "choose the length unit required by the surrounding logic",
        notes: &[
            "use `value.isEmpty()` for zero-length checks so encoded length units do not matter",
            "use `value.byteLength()` for proven ASCII text or logic that intentionally works with SplitScript UTF-8 byte offsets",
            "C# `String.Length` counts UTF-16 code units, while SplitScript `byteLength()` counts UTF-8 bytes; the values can differ for non-ASCII text",
            "there is no automatic rewrite because indexing, slicing, display width, and emptiness require different canonical operations",
        ],
    },
    MigrationDiagnostic {
        id: CSHARP_ARRAY_LENGTH_DIAGNOSTIC,
        concept: MigrationConceptId::new("array.length"),
        message: "C# array `Length` is `length()` in SplitScript",
        primary_label: "array length is exposed as a method",
        notes: &[
            "`values.length()` returns the element count as `u32` for both `[T]` and fixed `[T; N]` arrays",
            "review arithmetic copied from C# when it relied on the signed `i32` result of `Array.Length`",
        ],
    },
    MigrationDiagnostic {
        id: CSHARP_COLLECTION_COUNT_DIAGNOSTIC,
        concept: MigrationConceptId::new("collection.count"),
        message: "C# collection `Count` is `length()` in SplitScript",
        primary_label: "collection count is exposed as a method",
        notes: &[
            "`values.length()` returns a `u32` element count for arrays and a `u32` unique-value count for `Set<T>`",
            "choose the SplitScript collection shape before applying this rewrite: arrays preserve fixed order, while sets preserve growable unique membership",
            "review arithmetic copied from C# when it relied on the signed `i32` result of common collection `Count` properties",
        ],
    },
    MigrationDiagnostic {
        id: CSHARP_STRING_JOIN_DIAGNOSTIC,
        concept: MigrationConceptId::new("string.join"),
        message: "C# `String.Join` needs an explicit collection conversion",
        primary_label: "SplitScript joins one typed string array",
        notes: &[
            "rewrite `String.Join(separator, values)` as `String.join(values, separator)` when `values` is a `[String]`",
            "SplitScript allocates the final UTF-8 string once and inserts the separator only between adjacent values; empty and single-element arrays add no separators",
            "C# overloads accepting objects, variadic arguments, generic enumerables, or a start/count range require the values to be converted into a string array explicitly",
            "there is no automatic rewrite because the argument order changes and the C# overload does not prove a `[String]` input",
        ],
    },
    MigrationDiagnostic {
        id: CSHARP_SQUARE_ROOT_DIAGNOSTIC,
        concept: MigrationConceptId::new("math.square-root"),
        message: "C# square root is a type-preserving `sqrt` method in SplitScript",
        primary_label: "move this operation onto the floating-point value",
        notes: &[
            "rewrite `Math.Sqrt(value)` as `(value as f64).sqrt()` when C#'s binary64 result is required",
            "rewrite `MathF.Sqrt(value)` as `(value as f32).sqrt()` when the source intentionally uses binary32",
            "SplitScript `sqrt` preserves its receiver type; negative values produce NaN, and signed zero, infinity, and NaN follow IEEE 754",
            "there is no automatic rewrite because selecting f32 or f64 is part of the migrated program's memory and comparison semantics",
        ],
    },
    MigrationDiagnostic {
        id: CSHARP_TRUNCATE_DIAGNOSTIC,
        concept: MigrationConceptId::new("math.truncate"),
        message: "C# truncation is a type-preserving `truncate` method in SplitScript",
        primary_label: "move this operation onto the floating-point value",
        notes: &[
            "rewrite `Math.Truncate(value)` as `(value as f64).truncate()` when C#'s binary64 result is required",
            "rewrite `MathF.Truncate(value)` as `(value as f32).truncate()` when the source intentionally uses binary32",
            "SplitScript `truncate` preserves its receiver type and rounds toward zero; signed zero, infinity, and NaN follow IEEE 754",
            "there is no automatic rewrite because selecting f32 or f64 is part of the migrated program's memory and comparison semantics",
        ],
    },
    MigrationDiagnostic {
        id: CSHARP_ROUND_DIAGNOSTIC,
        concept: MigrationConceptId::new("math.round"),
        message: "C# rounding needs a receiver method and overload review in SplitScript",
        primary_label: "move this operation onto the floating-point value",
        notes: &[
            "rewrite the default midpoint-to-even form as `(value as f64).round()` for `Math.Round`, or `(value as f32).round()` for `MathF.Round`",
            "rewrite the integer-digits overload as `(value as f64).roundTo(digits)` for `Math.Round`; preserve f32 instead when migrating `MathF.Round`",
            "SplitScript `round` and `roundTo` preserve their receiver type and round halfway values to even",
            "decimal inputs and explicit `MidpointRounding` modes such as `AwayFromZero` are not equivalent to the available floating-point methods and need a semantic rewrite",
            "there is no automatic rewrite because the selected C# overload determines the result width, decimal behavior, and midpoint rule",
        ],
    },
    MigrationDiagnostic {
        id: CSHARP_FLOOR_DIAGNOSTIC,
        concept: MigrationConceptId::new("math.floor"),
        message: "C# floor is a type-preserving `floor` method in SplitScript",
        primary_label: "move this operation onto the floating-point value",
        notes: &[
            "rewrite `Math.Floor(value)` as `(value as f64).floor()` when C#'s binary64 result is required",
            "rewrite `MathF.Floor(value)` as `(value as f32).floor()` when the source intentionally uses binary32",
            "SplitScript `floor` preserves its receiver type and rounds toward negative infinity; signed zero, infinity, and NaN follow IEEE 754",
            "C# decimal inputs are not equivalent to binary floating-point and need a semantic rewrite",
            "there is no automatic rewrite because selecting f32 or f64 is part of the migrated program's memory and comparison semantics",
        ],
    },
    MigrationDiagnostic {
        id: CSHARP_CEILING_DIAGNOSTIC,
        concept: MigrationConceptId::new("math.ceiling"),
        message: "C# ceiling is a type-preserving `ceil` method in SplitScript",
        primary_label: "move this operation onto the floating-point value",
        notes: &[
            "rewrite `Math.Ceiling(value)` as `(value as f64).ceil()` when C#'s binary64 result is required",
            "rewrite `MathF.Ceiling(value)` as `(value as f32).ceil()` when the source intentionally uses binary32",
            "SplitScript `ceil` preserves its receiver type and rounds toward positive infinity; signed zero, infinity, and NaN follow IEEE 754",
            "C# decimal inputs are not equivalent to binary floating-point and need a semantic rewrite",
            "there is no automatic rewrite because selecting f32 or f64 is part of the migrated program's memory and comparison semantics",
        ],
    },
    MigrationDiagnostic {
        id: CSHARP_MIN_DIAGNOSTIC,
        concept: MigrationConceptId::new("math.minimum"),
        message: "C# minimum is a receiver-based `min` method in SplitScript",
        primary_label: "move this operation onto the first numeric value",
        notes: &[
            "rewrite `Math.Min(left, right)` or `MathF.Min(left, right)` as `left.min(right)` after making both operands the same intended numeric type",
            "SplitScript `min` preserves that type: integers retain their signedness and width, while floating-point values propagate NaN and choose negative zero over positive zero",
            "C# implicit numeric conversions and decimal overloads need an explicit semantic rewrite",
            "there is no automatic rewrite because the static call alone does not prove the selected C# overload or the intended result type",
        ],
    },
    MigrationDiagnostic {
        id: CSHARP_MAX_DIAGNOSTIC,
        concept: MigrationConceptId::new("math.maximum"),
        message: "C# maximum is a receiver-based `max` method in SplitScript",
        primary_label: "move this operation onto the first numeric value",
        notes: &[
            "rewrite `Math.Max(left, right)` or `MathF.Max(left, right)` as `left.max(right)` after making both operands the same intended numeric type",
            "SplitScript `max` preserves that type: integers retain their signedness and width, while floating-point values propagate NaN and choose positive zero over negative zero",
            "C# implicit numeric conversions and decimal overloads need an explicit semantic rewrite",
            "there is no automatic rewrite because the static call alone does not prove the selected C# overload or the intended result type",
        ],
    },
    MigrationDiagnostic {
        id: CSHARP_ABS_DIAGNOSTIC,
        concept: MigrationConceptId::new("math.absolute-value"),
        message: "C# absolute value is a receiver-based `abs` method in SplitScript",
        primary_label: "move this operation onto a signed numeric value",
        notes: &[
            "rewrite `Math.Abs(value)` or `MathF.Abs(value)` as `value.abs()` after establishing the intended signed integer or floating-point type",
            "SplitScript floating-point `abs` preserves the receiver width, changes negative zero to positive zero, and leaves NaN as NaN",
            "SplitScript signed-integer arithmetic wraps, so `abs` leaves the minimum value of each integer type unchanged; C# `Math.Abs` throws for a signed minimum value",
            "unsigned inputs, C# implicit numeric conversions, and decimal overloads need an explicit semantic rewrite",
            "there is no automatic rewrite because the static call alone does not prove the selected C# overload or the intended overflow policy",
        ],
    },
    MigrationDiagnostic {
        id: CSHARP_POWER_DIAGNOSTIC,
        concept: MigrationConceptId::new("math.power"),
        message: "C# power needs an exponent-specific rewrite in SplitScript",
        primary_label: "choose the operation that expresses this exponent's intent",
        notes: &[
            "rewrite `Math.Pow(value, 2)` or `MathF.Pow(value, 2)` as `value.squared()` after establishing the intended f64 or f32 receiver width",
            "rewrite a mask-shaped `Math.Pow(2, exponent)` as an explicit shift such as `1u64 << exponent`, choosing a width that can represent the highest required bit and validating the shift range",
            "SplitScript `squared` preserves the receiver type; integer overflow wraps to that type while floating-point multiplication follows IEEE 754",
            "general negative, fractional, infinite, and NaN exponents do not yet have a canonical SplitScript power operation",
            "there is no automatic rewrite because the call syntax alone does not prove the exponent value, result width, or whether the operation constructs a mask",
        ],
    },
    MigrationDiagnostic {
        id: CSHARP_NUMERIC_PARSE_DIAGNOSTIC,
        concept: MigrationConceptId::new("string.numeric-parse"),
        message: "C# static numeric parsing becomes `String.parse<T>()` in SplitScript",
        primary_label: "move the target type to the receiving boundary",
        notes: &[
            "rewrite `Int32.Parse(text)` as `let value: i32 = text.parse()?`, or use `else` when malformed input needs a fallback",
            "rewrite `TryParse` output parameters with ordinary Result control flow such as `match` or `else`; SplitScript does not mutate an out argument",
            "SplitScript parsing consumes strict ASCII decimal text in full; whitespace, separators, trailing text, and integer overflow are errors",
            "there is no automatic rewrite because the call alone does not identify the receiving declaration, fallback behavior, or `TryParse` control flow",
        ],
    },
    MigrationDiagnostic {
        id: CSHARP_CONVERT_INTEGER_DIAGNOSTIC,
        concept: MigrationConceptId::new("conversion.integer"),
        message: "C# integer conversion needs a source-specific SplitScript rewrite",
        primary_label: "choose checked parsing, numeric casting, rounding, or boolean mapping",
        notes: &[
            "for an integer source, use `value as i32` (or the matching fixed-width target) only after reviewing narrowing: SplitScript retains low bits, while `Convert` throws on out-of-range narrowing",
            "for an f32 or f64 source, `value.round() as i32` preserves midpoint-to-even rounding only for finite in-range values; SplitScript's final cast saturates and maps NaN to zero instead of throwing",
            "for a boolean source, write `if value { 1 } else { 0 }`; for a string source, infer the fixed-width target from `text.parse()` and handle its Result",
            "C# string conversion accepts current-culture formatting and surrounding whitespace, while SplitScript numeric parsing is strict locale-independent ASCII decimal text",
            "there is no automatic rewrite because the static call does not identify the source type, overflow policy, or failure boundary",
        ],
    },
    MigrationDiagnostic {
        id: CSHARP_CONVERT_FLOAT_DIAGNOSTIC,
        concept: MigrationConceptId::new("conversion.float"),
        message: "C# floating conversion needs a source-specific SplitScript rewrite",
        primary_label: "choose a numeric cast, strict string parse, or boolean mapping",
        notes: &[
            "for a numeric source, use `value as f32` or `value as f64` to make the intended floating width explicit",
            "for a string source, write `let value: f64 = text.parse()?` (or handle it with `else`); SplitScript parses strict locale-independent ASCII rather than current-culture text",
            "for a boolean source, write `if value { 1.0 } else { 0.0 }` with the receiving f32 or f64 type made explicit",
            "there is no automatic rewrite because the static call does not identify the source type or parse-failure policy",
        ],
    },
    MigrationDiagnostic {
        id: CSHARP_CONVERT_BOOLEAN_DIAGNOSTIC,
        concept: MigrationConceptId::new("conversion.boolean"),
        message: "C# boolean conversion needs a source-specific SplitScript expression",
        primary_label: "preserve numeric or textual truth semantics explicitly",
        notes: &[
            "for a numeric source, use `value != 0`; this preserves `Convert.ToBoolean`'s nonzero rule, including negative values",
            "for an existing boolean, use the value directly rather than converting it",
            "for a string source, trim and compare `true` or `false` explicitly with `equalsIgnoreAsciiCase`; decide how malformed text becomes a Result or fallback",
            "there is no automatic rewrite because numeric, string, optional, and already-boolean inputs require different expressions",
        ],
    },
    MigrationDiagnostic {
        id: CSHARP_CONVERT_STRING_DIAGNOSTIC,
        concept: MigrationConceptId::new("conversion.string"),
        message: "C# `Convert.ToString` needs a Display or integer-radix rewrite",
        primary_label: "separate display conversion from radix or culture-sensitive formatting",
        notes: &[
            "for one non-null value implementing Display, use `value as String`; interpolation, `print`, and `setVariable` accept Display values directly without a cast",
            "C# null and object overloads have different behavior and need an explicit Option or concrete-type policy",
            "rewrite `Convert.ToString(integer, radix)` as `integer.toString(radix)?` or handle its Result with `else`; SplitScript supports bases 2 through 36 and emits lowercase digits",
            "SplitScript radix formatting uses a leading minus sign for negative values; it does not reproduce C#'s two's-complement output for negative hexadecimal values",
            "culture and format-provider overloads are intentionally not rewritten because SplitScript display is deterministic and locale-independent",
        ],
    },
    MigrationDiagnostic {
        id: CSHARP_TIMESPAN_PARSE_DIAGNOSTIC,
        concept: MigrationConceptId::new("duration.parse"),
        message: "C# `TimeSpan.Parse` needs an explicit duration migration",
        primary_label: "review whether this text is data or a serialized timer value",
        notes: &[
            "for a fixed literal, construct the exact value with `Duration.fromSeconds`, `Duration.fromMilliseconds`, or `Duration.fromParts` rather than preserving a runtime parser",
            "when the input came from `timer.CurrentTime` or another duration converted to text, keep the value typed instead of serializing and reparsing it",
            "C# parsing is culture-sensitive and accepts a broad grammar; SplitScript does not provide a compatibility parser whose meaning could vary by host locale",
            "there is no automatic rewrite because the call does not reveal the input format or whether the surrounding timer API itself needs redesign",
        ],
    },
    MigrationDiagnostic {
        id: CSHARP_TIMESPAN_TICKS_DIAGNOSTIC,
        concept: MigrationConceptId::new("duration.csharp-ticks"),
        message: "C# ticks must be converted to a language-level duration unit",
        primary_label: "one C# tick is exactly 100 nanoseconds",
        notes: &[
            "for a proven in-range tick count, use `Duration.fromNanoseconds(ticks * 100)` with an explicit `i64` value",
            "prefer the source's native seconds, milliseconds, frames, or nanoseconds when available instead of preserving a C# representation detail",
            "multiplying an arbitrary signed 64-bit tick count by 100 can overflow even though C# `TimeSpan` accepts that tick count",
            "there is no automatic rewrite because SplitScript's exact nanosecond constructor cannot represent the full C# tick range through a single `i64` nanosecond count",
        ],
    },
    MigrationDiagnostic {
        id: ASL_UNITY_SCHEMA_DIAGNOSTIC,
        concept: MigrationConceptId::new("asl.unity.managed-schema"),
        message: "declare Unity managed metadata with an `image` schema",
        primary_label: "manual Unity runtime and metadata traversal is not a public SplitScript API",
        notes: &[
            "use `state Unity` (or an explicit `Unity.mono(...)` / `Unity.il2cpp(...)` provider selector), then declare managed images, classes, static roots, and fields at the top level",
            "read generated managed references in state expressions; each field hop is fallible and can use postfix `?`",
            "there is no mechanical rewrite because legacy helper calls do not contain the complete class and field schema",
        ],
    },
];

pub fn legacy_managed_method_diagnostic(name: &str) -> Option<MigrationDiagnosticId> {
    matches!(name, "Make" | "MakeString").then_some(ASL_UNITY_SCHEMA_DIAGNOSTIC)
}

pub fn legacy_string_method_diagnostic(name: &str) -> Option<MigrationDiagnosticId> {
    match name {
        "Equals" => Some(CSHARP_STRING_EQUALS_DIAGNOSTIC),
        "Substring" => Some(CSHARP_STRING_SUBSTRING_DIAGNOSTIC),
        "IndexOf" => Some(CSHARP_STRING_INDEX_OF_DIAGNOSTIC),
        "LastIndexOf" => Some(CSHARP_STRING_LAST_INDEX_OF_DIAGNOSTIC),
        "Replace" => Some(CSHARP_STRING_REPLACE_DIAGNOSTIC),
        "Trim" => Some(CSHARP_STRING_TRIM_DIAGNOSTIC),
        "PadLeft" | "PadRight" => Some(CSHARP_STRING_PADDING_DIAGNOSTIC),
        _ => None,
    }
}

pub fn legacy_string_field_diagnostic(name: &str) -> Option<MigrationDiagnosticId> {
    (name == "Length").then_some(CSHARP_STRING_LENGTH_DIAGNOSTIC)
}

pub fn legacy_array_field_diagnostic(name: &str) -> Option<MigrationDiagnosticId> {
    match name {
        "Length" => Some(CSHARP_ARRAY_LENGTH_DIAGNOSTIC),
        "Count" => Some(CSHARP_COLLECTION_COUNT_DIAGNOSTIC),
        _ => None,
    }
}

pub fn legacy_set_field_diagnostic(name: &str) -> Option<MigrationDiagnosticId> {
    (name == "Count").then_some(CSHARP_COLLECTION_COUNT_DIAGNOSTIC)
}

pub fn legacy_static_call_diagnostic(
    path: &[String],
    argument_count: usize,
) -> Option<MigrationDiagnosticId> {
    if matches!(path, [task, method] if task == "Task" && method == "Run") {
        return Some(ASL_TASK_RUN_DIAGNOSTIC);
    }
    if matches!(
        path,
        [owner, method]
            if (owner == "Unity" && matches!(method.as_str(), "mono" | "il2cpp"))
                || (owner == "mono" && matches!(method.as_str(), "Make" | "MakeString"))
    ) {
        return Some(ASL_UNITY_SCHEMA_DIAGNOSTIC);
    }
    if matches!(path, [modules, method] if modules == "modules" && method == "First") {
        return Some(if argument_count == 0 {
            ASL_MODULES_MAIN_DIAGNOSTIC
        } else {
            ASL_MODULES_QUERY_DIAGNOSTIC
        });
    }
    if matches!(
        path,
        [modules, method]
            if modules == "modules"
                && matches!(
                    method.as_str(),
                    "Any" | "FirstOrDefault" | "Single" | "SingleOrDefault" | "Where"
                )
    ) {
        return Some(ASL_MODULES_QUERY_DIAGNOSTIC);
    }
    let (owner, method) = match path {
        [owner, method] => (owner, method),
        [system, owner, method]
            if system == "System" && matches!(owner.as_str(), "Math" | "MathF" | "Convert") =>
        {
            (owner, method)
        }
        _ => return None,
    };
    if owner == "String" && method == "IsNullOrEmpty" {
        return Some(CSHARP_STRING_IS_NULL_OR_EMPTY_DIAGNOSTIC);
    }
    if owner == "String" && method == "IsNullOrWhiteSpace" {
        return Some(CSHARP_STRING_IS_NULL_OR_WHITE_SPACE_DIAGNOSTIC);
    }
    if owner == "String" && method == "Join" {
        return Some(CSHARP_STRING_JOIN_DIAGNOSTIC);
    }
    if owner == "Duration" && method == "Parse" {
        return Some(CSHARP_TIMESPAN_PARSE_DIAGNOSTIC);
    }
    if owner == "Duration" && method == "FromTicks" {
        return Some(CSHARP_TIMESPAN_TICKS_DIAGNOSTIC);
    }
    if owner == "Convert" {
        return match method.as_str() {
            "ToSByte" | "ToByte" | "ToInt16" | "ToUInt16" | "ToInt32" | "ToUInt32" | "ToInt64"
            | "ToUInt64" => Some(CSHARP_CONVERT_INTEGER_DIAGNOSTIC),
            "ToSingle" | "ToDouble" => Some(CSHARP_CONVERT_FLOAT_DIAGNOSTIC),
            "ToBoolean" => Some(CSHARP_CONVERT_BOOLEAN_DIAGNOSTIC),
            "ToString" => Some(CSHARP_CONVERT_STRING_DIAGNOSTIC),
            _ => None,
        };
    }
    if matches!(owner.as_str(), "Math" | "MathF") && method == "Sqrt" {
        return Some(CSHARP_SQUARE_ROOT_DIAGNOSTIC);
    }
    if matches!(owner.as_str(), "Math" | "MathF") && method == "Truncate" {
        return Some(CSHARP_TRUNCATE_DIAGNOSTIC);
    }
    if matches!(owner.as_str(), "Math" | "MathF") && method == "Round" {
        return Some(CSHARP_ROUND_DIAGNOSTIC);
    }
    if matches!(owner.as_str(), "Math" | "MathF") && method == "Floor" {
        return Some(CSHARP_FLOOR_DIAGNOSTIC);
    }
    if matches!(owner.as_str(), "Math" | "MathF") && method == "Ceiling" {
        return Some(CSHARP_CEILING_DIAGNOSTIC);
    }
    if matches!(owner.as_str(), "Math" | "MathF") && method == "Min" {
        return Some(CSHARP_MIN_DIAGNOSTIC);
    }
    if matches!(owner.as_str(), "Math" | "MathF") && method == "Max" {
        return Some(CSHARP_MAX_DIAGNOSTIC);
    }
    if matches!(owner.as_str(), "Math" | "MathF") && method == "Abs" {
        return Some(CSHARP_ABS_DIAGNOSTIC);
    }
    if matches!(owner.as_str(), "Math" | "MathF") && method == "Pow" {
        return Some(CSHARP_POWER_DIAGNOSTIC);
    }
    if method != "Parse" && method != "TryParse" {
        return None;
    }
    matches!(
        owner.as_str(),
        "SByte"
            | "Byte"
            | "Int16"
            | "UInt16"
            | "Int32"
            | "UInt32"
            | "Int64"
            | "UInt64"
            | "Single"
            | "Double"
            | "sbyte"
            | "byte"
            | "short"
            | "ushort"
            | "int"
            | "uint"
            | "long"
            | "ulong"
            | "float"
            | "double"
    )
    .then_some(CSHARP_NUMERIC_PARSE_DIAGNOSTIC)
}

pub fn legacy_lifecycle_diagnostic(name: &str) -> Option<MigrationDiagnosticId> {
    Some(match name {
        "startup" => ASL_STARTUP_DIAGNOSTIC,
        "init" => ASL_INIT_DIAGNOSTIC,
        "update" => ASL_UPDATE_DIAGNOSTIC,
        "exit" => ASL_EXIT_DIAGNOSTIC,
        "shutdown" => ASL_SHUTDOWN_DIAGNOSTIC,
        "onSplit" => ASL_TIMER_EVENT_DIAGNOSTIC,
        _ => return None,
    })
}

pub fn legacy_value_path_diagnostic(path: &str) -> Option<MigrationDiagnosticId> {
    if path == "modules" {
        return Some(ASL_MODULES_ENUMERATION_DIAGNOSTIC);
    }
    if path == "timer.CurrentSplitIndex" {
        return Some(ASL_CURRENT_SPLIT_INDEX_DIAGNOSTIC);
    }
    if path == "DateTime.Now"
        || path.starts_with("DateTime.Now.")
        || path == "System.DateTime.Now"
        || path.starts_with("System.DateTime.Now.")
        || path == "Environment.TickCount"
        || path.starts_with("Environment.TickCount.")
        || path == "Environment.TickCount64"
        || path.starts_with("Environment.TickCount64.")
        || path == "System.Environment.TickCount"
        || path.starts_with("System.Environment.TickCount.")
        || path == "System.Environment.TickCount64"
        || path.starts_with("System.Environment.TickCount64.")
    {
        return Some(ASL_MONOTONIC_TIME_DIAGNOSTIC);
    }
    if path == "timer.CurrentTime.RealTime" || path.starts_with("timer.CurrentTime.RealTime.") {
        return Some(ASL_TIMER_REAL_TIME_DIAGNOSTIC);
    }
    if path == "timer.CurrentTime.GameTime" || path.starts_with("timer.CurrentTime.GameTime.") {
        return Some(ASL_TIMER_GAME_TIME_DIAGNOSTIC);
    }
    if path == "timer.Run.Offset"
        || path.starts_with("timer.Run.Offset.")
        || path == "timer.CurrentTimingMethod"
        || path.starts_with("timer.CurrentTimingMethod.")
    {
        return Some(ASL_TIMER_CONTROL_DIAGNOSTIC);
    }
    if path == "timer.Run"
        || path.starts_with("timer.Run.")
        || path == "timer.CurrentSplit"
        || path.starts_with("timer.CurrentSplit.")
    {
        return Some(ASL_TIMER_RUN_METADATA_DIAGNOSTIC);
    }
    None
}

pub fn legacy_type_diagnostic(name: &str) -> Option<MigrationDiagnosticId> {
    match name {
        "MemoryWatcherList" => Some(ASL_MEMORY_WATCHER_LIST_DIAGNOSTIC),
        "TimerPhase" => Some(ASL_TIMER_PHASE_DIAGNOSTIC),
        "Stopwatch" => Some(ASL_MONOTONIC_TIME_DIAGNOSTIC),
        "UnityModule" | "UnityImage" | "UnityClass" | "UnityField" | "MonoModule" | "MonoImage"
        | "MonoClass" => Some(ASL_UNITY_SCHEMA_DIAGNOSTIC),
        _ => None,
    }
}

const ASL: &[SourceLanguage] = &[SourceLanguage::Asl];
const CSHARP: &[SourceLanguage] = &[SourceLanguage::CSharp];
const JAVASCRIPT: &[SourceLanguage] = &[SourceLanguage::JavaScript];
const CSHARP_JAVASCRIPT: &[SourceLanguage] = &[SourceLanguage::CSharp, SourceLanguage::JavaScript];
const ASL_CSHARP: &[SourceLanguage] = &[SourceLanguage::Asl, SourceLanguage::CSharp];
const ASL_RUST: &[SourceLanguage] = &[SourceLanguage::Asl, SourceLanguage::Rust];

const LET_SPELLINGS: &[ForeignSpelling] = &[
    ForeignSpelling {
        source: SourceLanguage::JavaScript,
        context: ForeignSpellingContext::VariableDeclaration,
        spelling: "const",
        replacement: ForeignSpellingReplacement::Text("let"),
        message: "SplitScript uses `let` instead of `const` for variable declarations",
        primary_label: "replace this familiar declaration keyword",
        fix_title: "replace `const` with `let`",
    },
    ForeignSpelling {
        source: SourceLanguage::CSharp,
        context: ForeignSpellingContext::VariableDeclaration,
        spelling: "var",
        replacement: ForeignSpellingReplacement::Text("let"),
        message: "SplitScript uses `let` instead of `var` for variable declarations",
        primary_label: "replace this familiar declaration keyword",
        fix_title: "replace `var` with `let`",
    },
    ForeignSpelling {
        source: SourceLanguage::Rust,
        context: ForeignSpellingContext::VariableModifier,
        spelling: "mut",
        replacement: ForeignSpellingReplacement::Remove,
        message: "SplitScript `let` bindings are already mutable",
        primary_label: "remove this Rust-only binding modifier",
        fix_title: "remove `mut`",
    },
];

const NONE_SPELLINGS: &[ForeignSpelling] = &[ForeignSpelling {
    source: SourceLanguage::JavaScript,
    context: ForeignSpellingContext::OptionalValue,
    spelling: "null",
    replacement: ForeignSpellingReplacement::Text("None"),
    message: "SplitScript uses `None` instead of `null` for absent optional values",
    primary_label: "replace this JavaScript-style value",
    fix_title: "replace `null` with `None`",
}];

const FN_SPELLINGS: &[ForeignSpelling] = &[
    ForeignSpelling {
        source: SourceLanguage::Rust,
        context: ForeignSpellingContext::FunctionDeclaration,
        spelling: "func",
        replacement: ForeignSpellingReplacement::Text("fn"),
        message: "SplitScript uses `fn` instead of `func` for functions",
        primary_label: "replace this familiar function keyword",
        fix_title: "replace `func` with `fn`",
    },
    ForeignSpelling {
        source: SourceLanguage::JavaScript,
        context: ForeignSpellingContext::FunctionDeclaration,
        spelling: "function",
        replacement: ForeignSpellingReplacement::Text("fn"),
        message: "SplitScript uses `fn` instead of `function` for functions",
        primary_label: "replace this familiar function keyword",
        fix_title: "replace `function` with `fn`",
    },
];

const STRUCT_DECLARATION_SPELLINGS: &[ForeignSpelling] = &[ForeignSpelling {
    source: SourceLanguage::CSharp,
    context: ForeignSpellingContext::StructDeclaration,
    spelling: "record",
    replacement: ForeignSpellingReplacement::Text("struct"),
    message: "SplitScript uses `struct` instead of C#'s `record` declaration keyword",
    primary_label: "replace this C# declaration keyword",
    fix_title: "replace `record` with `struct`",
}];

macro_rules! type_spelling {
    ($source:expr, $context:expr, $foreign:literal, $canonical:literal, $message:literal, $label:literal) => {
        ForeignSpelling {
            source: $source,
            context: $context,
            spelling: $foreign,
            replacement: ForeignSpellingReplacement::Text($canonical),
            message: $message,
            primary_label: $label,
            fix_title: concat!("replace `", $foreign, "` with `", $canonical, "`"),
        }
    };
}

const STRING_SPELLINGS: &[ForeignSpelling] = &[
    type_spelling!(
        SourceLanguage::CSharp,
        ForeignSpellingContext::Type,
        "string",
        "String",
        "SplitScript uses `String` instead of `string` for the string type",
        "type names are case-sensitive"
    ),
    type_spelling!(
        SourceLanguage::CSharp,
        ForeignSpellingContext::StaticTypeReceiver,
        "string",
        "String",
        "SplitScript uses `String` instead of `string` for the string type",
        "type names are case-sensitive"
    ),
];

const STRING_ASCII_LOWER_SPELLINGS: &[ForeignSpelling] = &[type_spelling!(
    SourceLanguage::CSharp,
    ForeignSpellingContext::Method,
    "ToLower",
    "toAsciiLowerCase",
    "SplitScript makes this conversion's ASCII-only semantics explicit with `toAsciiLowerCase`",
    "replace this culture-sensitive C# method name"
)];

const STRING_ASCII_UPPER_SPELLINGS: &[ForeignSpelling] = &[type_spelling!(
    SourceLanguage::CSharp,
    ForeignSpellingContext::Method,
    "ToUpper",
    "toAsciiUpperCase",
    "SplitScript makes this conversion's ASCII-only semantics explicit with `toAsciiUpperCase`",
    "replace this culture-sensitive C# method name"
)];

const ARRAY_EXTEND_SPELLINGS: &[ForeignSpelling] = &[type_spelling!(
    SourceLanguage::CSharp,
    ForeignSpellingContext::Method,
    "AddRange",
    "extend",
    "SplitScript uses `extend` to append one typed array to another",
    "replace this C# collection method name"
)];

const SET_SPELLINGS: &[ForeignSpelling] = &[
    type_spelling!(
        SourceLanguage::CSharp,
        ForeignSpellingContext::Type,
        "HashSet",
        "Set",
        "SplitScript uses `Set<T>` instead of `HashSet<T>` for unique collections",
        "replace this C# collection type name"
    ),
    type_spelling!(
        SourceLanguage::CSharp,
        ForeignSpellingContext::StaticTypeReceiver,
        "HashSet",
        "Set",
        "SplitScript uses `Set` instead of `HashSet` for unique collections",
        "replace this C# collection type name"
    ),
];

const MAP_SPELLINGS: &[ForeignSpelling] = &[
    type_spelling!(
        SourceLanguage::CSharp,
        ForeignSpellingContext::Type,
        "Dictionary",
        "Map",
        "SplitScript uses `Map<K, V>` instead of `Dictionary<K, V>` for key-value collections",
        "replace this C# collection type name"
    ),
    type_spelling!(
        SourceLanguage::CSharp,
        ForeignSpellingContext::StaticTypeReceiver,
        "Dictionary",
        "Map",
        "SplitScript uses `Map` instead of `Dictionary` for key-value collections",
        "replace this C# collection type name"
    ),
    type_spelling!(
        SourceLanguage::CSharp,
        ForeignSpellingContext::Type,
        "IDictionary",
        "Map",
        "SplitScript uses `Map<K, V>` instead of `IDictionary<K, V>` for key-value collections",
        "replace this C# collection type name"
    ),
    type_spelling!(
        SourceLanguage::Rust,
        ForeignSpellingContext::Type,
        "HashMap",
        "Map",
        "SplitScript uses `Map<K, V>` instead of Rust's `HashMap<K, V>` for key-value collections",
        "replace this Rust collection type name"
    ),
];

const INTEGER_SWAP_BYTES_SPELLINGS: &[ForeignSpelling] = &[
    type_spelling!(
        SourceLanguage::Rust,
        ForeignSpellingContext::Method,
        "swap_bytes",
        "swapBytes",
        "SplitScript spells this numeric byte-order operation `swapBytes`",
        "replace this Rust method name"
    ),
    type_spelling!(
        SourceLanguage::Rust,
        ForeignSpellingContext::Method,
        "from_be",
        "swapBytes",
        "process memory is decoded little-endian, so convert a big-endian integer with `swapBytes`",
        "replace this Rust byte-order conversion"
    ),
];

const STRICT_EQUALITY_SPELLINGS: &[ForeignSpelling] = &[
    type_spelling!(
        SourceLanguage::JavaScript,
        ForeignSpellingContext::Operator,
        "===",
        "==",
        "SplitScript uses typed `==` instead of JavaScript's `===`",
        "replace this JavaScript equality operator"
    ),
    type_spelling!(
        SourceLanguage::JavaScript,
        ForeignSpellingContext::Operator,
        "!==",
        "!=",
        "SplitScript uses typed `!=` instead of JavaScript's `!==`",
        "replace this JavaScript inequality operator"
    ),
];

const BITWISE_COMPLEMENT_SPELLINGS: &[ForeignSpelling] = &[type_spelling!(
    SourceLanguage::CSharp,
    ForeignSpellingContext::Operator,
    "~",
    "!",
    "SplitScript overloads `!` for integer bitwise complement instead of using `~`",
    "replace this familiar bitwise-complement operator"
)];

const DURATION_SPELLINGS: &[ForeignSpelling] = &[
    type_spelling!(
        SourceLanguage::CSharp,
        ForeignSpellingContext::Type,
        "TimeSpan",
        "Duration",
        "SplitScript uses `Duration` instead of `TimeSpan` for timer durations",
        "replace this C# type name"
    ),
    type_spelling!(
        SourceLanguage::CSharp,
        ForeignSpellingContext::StaticTypeReceiver,
        "TimeSpan",
        "Duration",
        "SplitScript uses `Duration` instead of `TimeSpan` for timer durations",
        "replace this C# type name"
    ),
    type_spelling!(
        SourceLanguage::CSharp,
        ForeignSpellingContext::ValuePath,
        "Duration.Zero",
        "Duration.zero()",
        "SplitScript constructs a zero duration with `Duration.zero()`",
        "replace this C# static property"
    ),
];

macro_rules! numeric_spelling {
    ($foreign:literal, $canonical:literal) => {
        ForeignSpelling {
            source: SourceLanguage::CSharp,
            context: ForeignSpellingContext::Type,
            spelling: $foreign,
            replacement: ForeignSpellingReplacement::Text($canonical),
            message: concat!(
                "SplitScript uses `",
                $canonical,
                "` instead of `",
                $foreign,
                "` for this numeric type"
            ),
            primary_label: "replace this C# numeric type name",
            fix_title: concat!("replace `", $foreign, "` with `", $canonical, "`"),
        }
    };
}

const NUMERIC_SPELLINGS: &[ForeignSpelling] = &[
    numeric_spelling!("sbyte", "i8"),
    numeric_spelling!("byte", "u8"),
    numeric_spelling!("short", "i16"),
    numeric_spelling!("ushort", "u16"),
    numeric_spelling!("int", "i32"),
    numeric_spelling!("uint", "u32"),
    numeric_spelling!("long", "i64"),
    numeric_spelling!("ulong", "u64"),
    numeric_spelling!("float", "f32"),
    numeric_spelling!("double", "f64"),
];

const ASL_PROCESS_IDENTITY_SPELLINGS: &[ForeignSpelling] = &[ForeignSpelling {
    source: SourceLanguage::Asl,
    context: ForeignSpellingContext::AttachedProcessValuePath,
    spelling: "game.ProcessName",
    replacement: ForeignSpellingReplacement::Text("process.name()"),
    message: "ASL `game.ProcessName` is `process.name()` in SplitScript",
    primary_label: "read the exact process candidate that matched during attachment",
    fix_title: "replace `game.ProcessName` with `process.name()`",
}];

const ASL_TIMER_STATE_SPELLINGS: &[ForeignSpelling] = &[
    type_spelling!(
        SourceLanguage::Asl,
        ForeignSpellingContext::ValuePath,
        "timer.CurrentPhase",
        "timer.state()",
        "ASL `timer.CurrentPhase` is `timer.state()` in SplitScript",
        "read the typed current timer state"
    ),
    type_spelling!(
        SourceLanguage::Asl,
        ForeignSpellingContext::ValuePath,
        "TimerPhase.NotRunning",
        "TimerState.NotRunning",
        "ASL `TimerPhase.NotRunning` is `TimerState.NotRunning` in SplitScript",
        "use the SplitScript timer-state variant"
    ),
    type_spelling!(
        SourceLanguage::Asl,
        ForeignSpellingContext::ValuePath,
        "TimerPhase.Running",
        "TimerState.Running",
        "ASL `TimerPhase.Running` is `TimerState.Running` in SplitScript",
        "use the SplitScript timer-state variant"
    ),
    type_spelling!(
        SourceLanguage::Asl,
        ForeignSpellingContext::ValuePath,
        "TimerPhase.Paused",
        "TimerState.Paused",
        "ASL `TimerPhase.Paused` is `TimerState.Paused` in SplitScript",
        "use the SplitScript timer-state variant"
    ),
    type_spelling!(
        SourceLanguage::Asl,
        ForeignSpellingContext::ValuePath,
        "TimerPhase.Ended",
        "TimerState.Ended",
        "ASL `TimerPhase.Ended` is `TimerState.Ended` in SplitScript",
        "use the SplitScript timer-state variant"
    ),
];

pub const CONCEPTS: &[MigrationConcept] = &[
    MigrationConcept {
        id: MigrationConceptId::new("numeric.byte-order"),
        name: "Numeric byte order",
        sources: &[SourceLanguage::Rust, SourceLanguage::CSharp],
        support: MigrationSupport::Direct,
        summary: "Use [`Numeric.swapBytes`] to reverse a numeric value's raw bytes after reading data stored in the opposite byte order. It preserves the exact integer or floating-point type; eight-bit values are unchanged.",
        targets: &[MigrationTarget::StandardLibraryItem("Numeric.swapBytes")],
        cookbook_anchor: None,
        spellings: INTEGER_SWAP_BYTES_SPELLINGS,
    },
    MigrationConcept {
        id: MigrationConceptId::new("declaration.let"),
        name: "Variable declarations",
        sources: &[
            SourceLanguage::CSharp,
            SourceLanguage::JavaScript,
            SourceLanguage::Rust,
        ],
        support: MigrationSupport::Direct,
        summary: "Use one inferred [`let`] declaration; SplitScript has no const/let split.",
        targets: &[MigrationTarget::Language("let")],
        cookbook_anchor: None,
        spellings: LET_SPELLINGS,
    },
    MigrationConcept {
        id: MigrationConceptId::new("value.none"),
        name: "Absent optional values",
        sources: JAVASCRIPT,
        support: MigrationSupport::Direct,
        summary: "[`None`] is SplitScript's zero-sized unit value and the absent side of an option.",
        targets: &[MigrationTarget::Language("None")],
        cookbook_anchor: None,
        spellings: NONE_SPELLINGS,
    },
    MigrationConcept {
        id: MigrationConceptId::new("declaration.function"),
        name: "Function declarations",
        sources: CSHARP_JAVASCRIPT,
        support: MigrationSupport::Direct,
        summary: "Functions and methods use the [`fn`] declaration keyword.",
        targets: &[MigrationTarget::Language("fn")],
        cookbook_anchor: None,
        spellings: FN_SPELLINGS,
    },
    MigrationConcept {
        id: MigrationConceptId::new("type.string"),
        name: "String type",
        sources: CSHARP,
        support: MigrationSupport::Direct,
        summary: "The immutable UTF-8 string type is named [`String`].",
        targets: &[MigrationTarget::StandardLibraryType("String")],
        cookbook_anchor: None,
        spellings: STRING_SPELLINGS,
    },
    MigrationConcept {
        id: MigrationConceptId::new("string.ascii-lowercase"),
        name: "ASCII string lowercasing",
        sources: CSHARP,
        support: MigrationSupport::Direct,
        summary: "Use [`toAsciiLowerCase`] when game identifiers require ASCII-only normalization; this is not culture-sensitive Unicode lowercasing.",
        targets: &[MigrationTarget::StandardLibraryItem(
            "String.toAsciiLowerCase",
        )],
        cookbook_anchor: Some("c-string-operations"),
        spellings: STRING_ASCII_LOWER_SPELLINGS,
    },
    MigrationConcept {
        id: MigrationConceptId::new("string.ascii-uppercase"),
        name: "ASCII string uppercasing",
        sources: CSHARP,
        support: MigrationSupport::Direct,
        summary: "Use [`toAsciiUpperCase`] when game identifiers require ASCII-only normalization; this is not culture-sensitive Unicode uppercasing.",
        targets: &[MigrationTarget::StandardLibraryItem(
            "String.toAsciiUpperCase",
        )],
        cookbook_anchor: Some("c-string-operations"),
        spellings: STRING_ASCII_UPPER_SPELLINGS,
    },
    MigrationConcept {
        id: MigrationConceptId::new("string.equality"),
        name: "String equality",
        sources: CSHARP,
        support: MigrationSupport::Direct,
        summary: "Use [`==`] or [`!=`] for exact string content equality; use [`equalsIgnoreAsciiCase`] only when ASCII-insensitive matching is intended.",
        targets: &[
            MigrationTarget::StandardLibraryType("String"),
            MigrationTarget::StandardLibraryItem("String.equalsIgnoreAsciiCase"),
        ],
        cookbook_anchor: Some("c-string-operations"),
        spellings: &[],
    },
    MigrationConcept {
        id: MigrationConceptId::new("operator.strict-equality"),
        name: "Strict equality operators",
        sources: JAVASCRIPT,
        support: MigrationSupport::Direct,
        summary: "Use typed [`==`] and [`!=`]; SplitScript has no coercing equality operators, so JavaScript's extra `=` is unnecessary.",
        targets: &[
            MigrationTarget::Language("=="),
            MigrationTarget::Language("!="),
        ],
        cookbook_anchor: None,
        spellings: STRICT_EQUALITY_SPELLINGS,
    },
    MigrationConcept {
        id: MigrationConceptId::new("operator.bitwise-complement"),
        name: "Bitwise complement",
        sources: CSHARP_JAVASCRIPT,
        support: MigrationSupport::Direct,
        summary: "Use type-directed [`!`]: it is logical negation for booleans and width-preserving bitwise complement for integers.",
        targets: &[MigrationTarget::StandardLibraryItem("Integer.bitNot")],
        cookbook_anchor: None,
        spellings: BITWISE_COMPLEMENT_SPELLINGS,
    },
    MigrationConcept {
        id: MigrationConceptId::new("iteration.range"),
        name: "Bounded integer ranges",
        sources: &[
            SourceLanguage::Asl,
            SourceLanguage::CSharp,
            SourceLanguage::Rust,
        ],
        support: MigrationSupport::Direct,
        summary: "Use [`..<`] for an exclusive upper endpoint or [`..=`] for an inclusive one; SplitScript rejects bare `..` so the endpoint policy is explicit.",
        targets: &[MigrationTarget::Language("range")],
        cookbook_anchor: Some("bounded-integer-iteration"),
        spellings: &[],
    },
    MigrationConcept {
        id: MigrationConceptId::new("string.substring"),
        name: "Substring extraction",
        sources: CSHARP,
        support: MigrationSupport::TypedPattern,
        summary: "Use fallible `slice(start, exclusiveEnd)` only after translating C#'s length argument and verifying that UTF-16 source positions are valid UTF-8 byte offsets.",
        targets: &[MigrationTarget::StandardLibraryItem("String.slice")],
        cookbook_anchor: Some("c-string-operations"),
        spellings: &[],
    },
    MigrationConcept {
        id: MigrationConceptId::new("string.index-of"),
        name: "Substring position",
        sources: CSHARP,
        support: MigrationSupport::TypedPattern,
        summary: "Use `indexOf` for an optional UTF-8 byte offset; review C# UTF-16 index arithmetic and replace the `-1` sentinel with [`T?`] handling.",
        targets: &[MigrationTarget::StandardLibraryItem("String.indexOf")],
        cookbook_anchor: Some("c-string-operations"),
        spellings: &[],
    },
    MigrationConcept {
        id: MigrationConceptId::new("string.last-index-of"),
        name: "Last substring position",
        sources: CSHARP,
        support: MigrationSupport::TypedPattern,
        summary: "Use [`lastIndexOf`] for an optional final UTF-8 byte offset; review C# UTF-16 index arithmetic and replace the `-1` sentinel with [`T?`] handling.",
        targets: &[MigrationTarget::StandardLibraryItem("String.lastIndexOf")],
        cookbook_anchor: Some("c-string-operations"),
        spellings: &[],
    },
    MigrationConcept {
        id: MigrationConceptId::new("string.replacement"),
        name: "Exact string replacement",
        sources: CSHARP,
        support: MigrationSupport::TypedPattern,
        summary: "Use fallible [`replaceAll`] for immutable exact replacement; explicitly handle failure and translate a null C# replacement to an empty string only when deletion was intended.",
        targets: &[MigrationTarget::StandardLibraryItem("String.replaceAll")],
        cookbook_anchor: Some("c-string-operations"),
        spellings: &[],
    },
    MigrationConcept {
        id: MigrationConceptId::new("string.ascii-trim"),
        name: "ASCII whitespace trimming",
        sources: CSHARP,
        support: MigrationSupport::TypedPattern,
        summary: "Use [`trimAsciiWhitespace`] for text known to use ASCII boundary whitespace; review Unicode and character-set trimming explicitly.",
        targets: &[MigrationTarget::StandardLibraryItem(
            "String.trimAsciiWhitespace",
        )],
        cookbook_anchor: Some("c-string-operations"),
        spellings: &[],
    },
    MigrationConcept {
        id: MigrationConceptId::new("string.padding"),
        name: "String padding",
        sources: CSHARP,
        support: MigrationSupport::TypedPattern,
        summary: "Use `padStart(width, fill)` or `padEnd(width, fill)` with an explicit character; review C# UTF-16 widths against SplitScript's Unicode-scalar widths.",
        targets: &[
            MigrationTarget::StandardLibraryItem("String.padStart"),
            MigrationTarget::StandardLibraryItem("String.padEnd"),
        ],
        cookbook_anchor: Some("c-string-operations"),
        spellings: &[],
    },
    MigrationConcept {
        id: MigrationConceptId::new("string.null-or-empty"),
        name: "Nullable string emptiness",
        sources: CSHARP,
        support: MigrationSupport::TypedPattern,
        summary: "Use [`String.isEmpty`] for required strings and match `String?` explicitly when absence should also count as empty.",
        targets: &[MigrationTarget::StandardLibraryItem("String.isEmpty")],
        cookbook_anchor: Some("c-string-operations"),
        spellings: &[],
    },
    MigrationConcept {
        id: MigrationConceptId::new("string.null-or-white-space"),
        name: "Nullable blank strings",
        sources: CSHARP,
        support: MigrationSupport::TypedPattern,
        summary: "Use [`String.isBlank`] for required strings and match `String?` explicitly when absence should also count as blank.",
        targets: &[MigrationTarget::StandardLibraryItem("String.isBlank")],
        cookbook_anchor: Some("c-string-operations"),
        spellings: &[],
    },
    MigrationConcept {
        id: MigrationConceptId::new("string.length"),
        name: "String length",
        sources: CSHARP,
        support: MigrationSupport::TypedPattern,
        summary: "Use `isEmpty()` for emptiness and [`byteLength()`] only for UTF-8 byte-oriented or proven ASCII logic; C# `Length` counts UTF-16 code units.",
        targets: &[
            MigrationTarget::StandardLibraryItem("String.isEmpty"),
            MigrationTarget::StandardLibraryItem("String.byteLength"),
        ],
        cookbook_anchor: Some("c-string-operations"),
        spellings: &[],
    },
    MigrationConcept {
        id: MigrationConceptId::new("array.length"),
        name: "Array length",
        sources: CSHARP,
        support: MigrationSupport::Direct,
        summary: "Call `values.length()` for the [`u32`] element count of dynamic and fixed arrays.",
        targets: &[MigrationTarget::StandardLibraryItem("Array.length")],
        cookbook_anchor: Some("collection-search-and-run-scoped-sets"),
        spellings: &[],
    },
    MigrationConcept {
        id: MigrationConceptId::new("array.extend"),
        name: "Bulk array extension",
        sources: CSHARP,
        support: MigrationSupport::Direct,
        summary: "Call `values.extend(moreValues)` to append a typed array in order; extending an array with itself duplicates its original contents once.",
        targets: &[MigrationTarget::StandardLibraryItem("Array.extend")],
        cookbook_anchor: Some("collection-search-and-run-scoped-sets"),
        spellings: ARRAY_EXTEND_SPELLINGS,
    },
    MigrationConcept {
        id: MigrationConceptId::new("collection.set"),
        name: "Unique-value collections",
        sources: CSHARP,
        support: MigrationSupport::Direct,
        summary: "Use [`Set<T>`] for growable unique values. Replace `HashSet<T>` with [`Set<T>`], then review source operations whose names or return values differ.",
        targets: &[MigrationTarget::StandardLibraryItem("Set.new")],
        cookbook_anchor: Some("collection-search-and-run-scoped-sets"),
        spellings: SET_SPELLINGS,
    },
    MigrationConcept {
        id: MigrationConceptId::new("collection.map"),
        name: "Key-value collections",
        sources: &[SourceLanguage::CSharp, SourceLanguage::Rust],
        support: MigrationSupport::TypedPattern,
        summary: "Use [`Map<K, V>`] for growable key-value collections. Indexing returns a [`MapEntry<K, V>`] so an absent key remains distinguishable even when `V` is optional; use [`Map.containsKey`] when only membership matters.",
        targets: &[MigrationTarget::StandardLibraryItem("Map.new")],
        cookbook_anchor: Some("collection-search-and-run-scoped-sets"),
        spellings: MAP_SPELLINGS,
    },
    MigrationConcept {
        id: MigrationConceptId::new("asl.collection.tuple-entry"),
        name: "Tuple-shaped collection entries",
        sources: ASL_CSHARP,
        support: MigrationSupport::TypedPattern,
        summary: "SplitScript does not use anonymous tuple element types. Give the entry shape a small named [`struct`], store it in `[Entry]`, and destructure its fields in loops and patterns.",
        targets: &[MigrationTarget::Language("struct")],
        cookbook_anchor: Some("collection-search-and-run-scoped-sets"),
        spellings: &[],
    },
    MigrationConcept {
        id: MigrationConceptId::new("collection.count"),
        name: "Collection count",
        sources: CSHARP,
        support: MigrationSupport::Direct,
        summary: "After choosing an array or set from the source's ordering and uniqueness requirements, call `values.length()` for its [`u32`] count.",
        targets: &[
            MigrationTarget::StandardLibraryItem("Array.length"),
            MigrationTarget::StandardLibraryItem("Set.length"),
        ],
        cookbook_anchor: Some("collection-search-and-run-scoped-sets"),
        spellings: &[],
    },
    MigrationConcept {
        id: MigrationConceptId::new("string.join"),
        name: "String collection joining",
        sources: CSHARP,
        support: MigrationSupport::TypedPattern,
        summary: "Use `String.join(values, separator)` for a typed string array; convert C# object, variadic, enumerable, and range overloads explicitly.",
        targets: &[MigrationTarget::StandardLibraryItem("String.join")],
        cookbook_anchor: Some("c-string-operations"),
        spellings: &[],
    },
    MigrationConcept {
        id: MigrationConceptId::new("math.square-root"),
        name: "Floating-point square root",
        sources: CSHARP,
        support: MigrationSupport::TypedPattern,
        summary: "Use `value.sqrt()` with an explicit f32 or f64 boundary when preserving C# Math versus MathF semantics.",
        targets: &[MigrationTarget::StandardLibraryItem("Float.sqrt")],
        cookbook_anchor: None,
        spellings: &[],
    },
    MigrationConcept {
        id: MigrationConceptId::new("math.truncate"),
        name: "Floating-point truncation",
        sources: CSHARP,
        support: MigrationSupport::TypedPattern,
        summary: "Use `value.truncate()` with an explicit f32 or f64 boundary when preserving C# Math versus MathF semantics.",
        targets: &[MigrationTarget::StandardLibraryItem("Float.truncate")],
        cookbook_anchor: None,
        spellings: &[],
    },
    MigrationConcept {
        id: MigrationConceptId::new("math.round"),
        name: "Floating-point rounding",
        sources: CSHARP,
        support: MigrationSupport::TypedPattern,
        summary: "Use `value.round()` or `value.roundTo(digits)` for midpoint-to-even floating-point rounding; review result width, decimal inputs, and explicit midpoint modes.",
        targets: &[
            MigrationTarget::StandardLibraryItem("Float.round"),
            MigrationTarget::StandardLibraryItem("Float.roundTo"),
        ],
        cookbook_anchor: None,
        spellings: &[],
    },
    MigrationConcept {
        id: MigrationConceptId::new("math.floor"),
        name: "Floating-point floor",
        sources: CSHARP,
        support: MigrationSupport::TypedPattern,
        summary: "Use `value.floor()` with an explicit f32 or f64 boundary; review C# decimal inputs separately.",
        targets: &[MigrationTarget::StandardLibraryItem("Float.floor")],
        cookbook_anchor: None,
        spellings: &[],
    },
    MigrationConcept {
        id: MigrationConceptId::new("math.ceiling"),
        name: "Floating-point ceiling",
        sources: CSHARP,
        support: MigrationSupport::TypedPattern,
        summary: "Use `value.ceil()` with an explicit f32 or f64 boundary; review C# decimal inputs separately.",
        targets: &[MigrationTarget::StandardLibraryItem("Float.ceil")],
        cookbook_anchor: None,
        spellings: &[],
    },
    MigrationConcept {
        id: MigrationConceptId::new("math.minimum"),
        name: "Numeric minimum",
        sources: CSHARP,
        support: MigrationSupport::TypedPattern,
        summary: "Use `left.min(right)` after establishing one intended numeric type; review C# implicit conversions and decimal overloads.",
        targets: &[MigrationTarget::StandardLibraryItem("Numeric.min")],
        cookbook_anchor: None,
        spellings: &[],
    },
    MigrationConcept {
        id: MigrationConceptId::new("math.maximum"),
        name: "Numeric maximum",
        sources: CSHARP,
        support: MigrationSupport::TypedPattern,
        summary: "Use `left.max(right)` after establishing one intended numeric type; review C# implicit conversions and decimal overloads.",
        targets: &[MigrationTarget::StandardLibraryItem("Numeric.max")],
        cookbook_anchor: None,
        spellings: &[],
    },
    MigrationConcept {
        id: MigrationConceptId::new("math.absolute-value"),
        name: "Signed absolute value",
        sources: CSHARP,
        support: MigrationSupport::TypedPattern,
        summary: "Use `value.abs()` after establishing a signed numeric type; review C# signed-minimum overflow, unsigned conversions, and decimal inputs.",
        targets: &[MigrationTarget::StandardLibraryItem("Signed.abs")],
        cookbook_anchor: None,
        spellings: &[],
    },
    MigrationConcept {
        id: MigrationConceptId::new("math.power"),
        name: "Numeric powers",
        sources: CSHARP,
        support: MigrationSupport::TypedPattern,
        summary: "Use `value.squared()` for the corpus-proven exponent two and an explicit typed shift for power-of-two masks; general floating powers remain planned.",
        targets: &[MigrationTarget::StandardLibraryItem("Numeric.squared")],
        cookbook_anchor: None,
        spellings: &[],
    },
    MigrationConcept {
        id: MigrationConceptId::new("string.numeric-parse"),
        name: "Numeric string parsing",
        sources: CSHARP,
        support: MigrationSupport::Direct,
        summary: "Replace static Parse/TryParse calls and output parameters with fallible `text.parse()` and ordinary [`T!`] handling.",
        targets: &[MigrationTarget::StandardLibraryItem("String.parse")],
        cookbook_anchor: Some("c-string-operations"),
        spellings: &[],
    },
    MigrationConcept {
        id: MigrationConceptId::new("conversion.integer"),
        name: "Integer conversion",
        sources: CSHARP,
        support: MigrationSupport::TypedPattern,
        summary: "Choose fixed-width [`as`], midpoint-to-even rounding, strict string parsing, or an explicit boolean mapping from the source type; C# checked overflow is not SplitScript cast behavior.",
        targets: &[
            MigrationTarget::Language("as"),
            MigrationTarget::Language("if"),
            MigrationTarget::StandardLibraryItem("String.parse"),
            MigrationTarget::StandardLibraryItem("Float.round"),
        ],
        cookbook_anchor: Some("c-convert-operations"),
        spellings: &[],
    },
    MigrationConcept {
        id: MigrationConceptId::new("conversion.float"),
        name: "Floating-point conversion",
        sources: CSHARP,
        support: MigrationSupport::TypedPattern,
        summary: "Use an explicit f32/f64 [`as`] cast for numbers, [`String.parse`] for text, or an [`if`] expression for booleans.",
        targets: &[
            MigrationTarget::Language("as"),
            MigrationTarget::Language("if"),
            MigrationTarget::StandardLibraryItem("String.parse"),
        ],
        cookbook_anchor: Some("c-convert-operations"),
        spellings: &[],
    },
    MigrationConcept {
        id: MigrationConceptId::new("conversion.boolean"),
        name: "Boolean conversion",
        sources: CSHARP,
        support: MigrationSupport::TypedPattern,
        summary: "Use `value != 0` for numbers, the value itself for bool, and explicit trimmed ASCII-insensitive true/false handling for strings.",
        targets: &[
            MigrationTarget::Language("if"),
            MigrationTarget::StandardLibraryItem("String.trimAsciiWhitespace"),
            MigrationTarget::StandardLibraryItem("String.equalsIgnoreAsciiCase"),
        ],
        cookbook_anchor: Some("c-convert-operations"),
        spellings: &[],
    },
    MigrationConcept {
        id: MigrationConceptId::new("conversion.string"),
        name: "Display conversion",
        sources: CSHARP,
        support: MigrationSupport::TypedPattern,
        summary: "Use `value as String` for ordinary Display conversion and `integer.toString(radix)` for bases 2 through 36; culture, null, and object overloads require separate policies.",
        targets: &[
            MigrationTarget::Language("as"),
            MigrationTarget::StandardLibraryItem("Integer.toString"),
        ],
        cookbook_anchor: Some("c-convert-operations"),
        spellings: &[],
    },
    MigrationConcept {
        id: MigrationConceptId::new("type.duration"),
        name: "Timer durations",
        sources: CSHARP,
        support: MigrationSupport::Direct,
        summary: "Use [`Duration`] instead of C#'s `TimeSpan`.",
        targets: &[MigrationTarget::StandardLibraryType("Duration")],
        cookbook_anchor: None,
        spellings: DURATION_SPELLINGS,
    },
    MigrationConcept {
        id: MigrationConceptId::new("duration.parse"),
        name: "Text duration parsing",
        sources: CSHARP,
        support: MigrationSupport::TypedPattern,
        summary: "Replace `TimeSpan.Parse` according to whether the input is fixed data or an already-typed timer value; do not preserve culture-sensitive parsing by default.",
        targets: &[
            MigrationTarget::StandardLibraryItem("Duration.fromSeconds"),
            MigrationTarget::StandardLibraryItem("Duration.fromMilliseconds"),
            MigrationTarget::StandardLibraryItem("Duration.fromParts"),
        ],
        cookbook_anchor: None,
        spellings: &[],
    },
    MigrationConcept {
        id: MigrationConceptId::new("duration.csharp-ticks"),
        name: "C# duration ticks",
        sources: CSHARP,
        support: MigrationSupport::TypedPattern,
        summary: "Convert 100-nanosecond C# ticks to a source unit explicitly, with range review; SplitScript does not expose C# ticks as a native duration unit.",
        targets: &[MigrationTarget::StandardLibraryItem(
            "Duration.fromNanoseconds",
        )],
        cookbook_anchor: None,
        spellings: &[],
    },
    MigrationConcept {
        id: MigrationConceptId::new("asl.time.monotonic-delay"),
        name: "Monotonic delays and debouncing",
        sources: ASL_CSHARP,
        support: MigrationSupport::TypedPattern,
        summary: "Replace elapsed-time uses of `DateTime.Now` or `Stopwatch` with an [`Instant`] captured at the source event and an exact [`Duration`] comparison.",
        targets: &[
            MigrationTarget::StandardLibraryType("Instant"),
            MigrationTarget::StandardLibraryType("Duration"),
        ],
        cookbook_anchor: Some("monotonic-delays-and-debouncing"),
        spellings: &[],
    },
    MigrationConcept {
        id: MigrationConceptId::new("asl.async.blocking-sleep"),
        name: "Blocking sleeps",
        sources: ASL_CSHARP,
        support: MigrationSupport::TypedPattern,
        summary: "`Thread.Sleep` cannot block SplitScript's cooperative update loop. Model delayed work as state across ticks with [`Instant`] and [`Duration`], or apply [`future.timeout`] when the source is bounding an existing asynchronous operation.",
        targets: &[
            MigrationTarget::StandardLibraryType("Instant"),
            MigrationTarget::StandardLibraryType("Duration"),
            MigrationTarget::StandardLibraryItem("future.timeout"),
        ],
        cookbook_anchor: Some("monotonic-delays-and-debouncing"),
        spellings: &[],
    },
    MigrationConcept {
        id: MigrationConceptId::new("asl.file.last-write-time"),
        name: "File modification metadata",
        sources: ASL_CSHARP,
        support: MigrationSupport::Planned,
        summary: "`File.GetLastWriteTime` and `File.GetLastWriteTimeUtc` need a read-only metadata API and a host-defined timestamp contract. The current handle-free [`File`] surface reads complete bytes or UTF-8 text but does not expose metadata.",
        targets: &[],
        cookbook_anchor: None,
        spellings: &[],
    },
    MigrationConcept {
        id: MigrationConceptId::new("asl.data.json"),
        name: "JSON data",
        sources: ASL_CSHARP,
        support: MigrationSupport::Planned,
        summary: "JSON parsing and serialization need a typed SplitScript data model; do not substitute ad-hoc string operations for `JsonConvert`, `JObject`, or `System.Text.Json` calls.",
        targets: &[],
        cookbook_anchor: None,
        spellings: &[],
    },
    MigrationConcept {
        id: MigrationConceptId::new("type.fixed-width-number"),
        name: "Fixed-width numeric types",
        sources: CSHARP,
        support: MigrationSupport::Direct,
        summary: "Memory-facing numbers use explicit signedness and bit widths.",
        targets: &[
            MigrationTarget::Language("i8"),
            MigrationTarget::Language("u8"),
            MigrationTarget::Language("i16"),
            MigrationTarget::Language("u16"),
            MigrationTarget::Language("i32"),
            MigrationTarget::Language("u32"),
            MigrationTarget::Language("i64"),
            MigrationTarget::Language("u64"),
            MigrationTarget::Language("f32"),
            MigrationTarget::Language("f64"),
        ],
        cookbook_anchor: None,
        spellings: NUMERIC_SPELLINGS,
    },
    MigrationConcept {
        id: MigrationConceptId::new("asl.memory.primitive-types"),
        name: "ASL primitive state types",
        sources: ASL,
        support: MigrationSupport::TypedPattern,
        summary: "Preserve the bytes read by the ASL state declaration: [`bool`] is one byte; signed and unsigned integers and floating-point values map to the corresponding explicit-width SplitScript type. Treat `stringN`, `byteN`, pointers, enums, and values created in C# action code according to their actual representation rather than their nearest-looking name.",
        targets: &[
            MigrationTarget::Language("bool"),
            MigrationTarget::Language("i8"),
            MigrationTarget::Language("u8"),
            MigrationTarget::Language("i16"),
            MigrationTarget::Language("u16"),
            MigrationTarget::Language("i32"),
            MigrationTarget::Language("u32"),
            MigrationTarget::Language("i64"),
            MigrationTarget::Language("u64"),
            MigrationTarget::Language("f32"),
            MigrationTarget::Language("f64"),
        ],
        cookbook_anchor: Some("asl-primitive-state-types"),
        spellings: &[],
    },
    MigrationConcept {
        id: MigrationConceptId::new("asl.state.attachment"),
        name: "Attachment state declaration",
        sources: ASL,
        support: MigrationSupport::TypedPattern,
        summary: "Declare one native process name, an array of alternate process names, or a typed emulator provider per autosplitter file. An ASL declaration listing multiple processes becomes alternate names for one attachment, not concurrent attachments. Names are exact host identities, so Windows executable candidates currently include `.exe`. The declaration owns attachment and defines the fields polled into [`old`] and [`current`].",
        targets: &[MigrationTarget::Language("state")],
        cookbook_anchor: Some("attachment-state-declarations"),
        spellings: &[],
    },
    MigrationConcept {
        id: MigrationConceptId::new("asl.state.string-n"),
        name: "Bounded native stringN state",
        sources: ASL,
        support: MigrationSupport::TypedPattern,
        summary: "Choose the native encoding explicitly: [`utf8`] bounds bytes and [`utf16le`] bounds two-byte code units.",
        targets: &[MigrationTarget::Language("state")],
        cookbook_anchor: Some("bounded-native-stringn-state"),
        spellings: &[],
    },
    MigrationConcept {
        id: MigrationConceptId::new("asl.state.version-label"),
        name: "Version-labelled state blocks",
        sources: ASL,
        support: MigrationSupport::TypedPattern,
        summary: "Use one state block, initialize an ordinary build enum in [`onAttach`], and guard version-specific fields with it.",
        targets: &[MigrationTarget::Language("state")],
        cookbook_anchor: Some("version-labelled-asl-states"),
        spellings: &[],
    },
    MigrationConcept {
        id: MigrationConceptId::new("asl.state.contiguous-aggregate"),
        name: "Contiguous memory aggregates",
        sources: ASL_CSHARP,
        support: MigrationSupport::TypedPattern,
        summary: "Read physically contiguous values as one naturally aligned struct or fixed-length [`[T; N]`] array when that type exactly matches the target-memory layout.",
        targets: &[
            MigrationTarget::Language("struct"),
            MigrationTarget::Language("[T; N]"),
            MigrationTarget::Language("state"),
            MigrationTarget::StandardLibraryItem("Process.read"),
        ],
        cookbook_anchor: Some("contiguous-structs-and-fixed-arrays"),
        spellings: STRUCT_DECLARATION_SPELLINGS,
    },
    MigrationConcept {
        id: MigrationConceptId::new("asl.state.helper-snapshots"),
        name: "State snapshots in helper functions",
        sources: ASL,
        support: MigrationSupport::Direct,
        summary: "Helpers may read [`old`] and [`current`] directly or accept caller-selected snapshots as inferred parameters. The compiler propagates direct snapshot requirements and rejects calls before committed snapshots exist.",
        targets: &[
            MigrationTarget::Language("fn"),
            MigrationTarget::Language("old"),
            MigrationTarget::Language("current"),
        ],
        cookbook_anchor: Some("snapshot-dependent-helper-functions"),
        spellings: &[],
    },
    MigrationConcept {
        id: MigrationConceptId::new("asl.memory.deep-pointer"),
        name: "DeepPointer and native state roots",
        sources: ASL,
        support: MigrationSupport::Direct,
        summary: "A bare numeric root in an ASL native state field or `DeepPointer` is normally main-module-relative. Preserve it as `at \"game.exe\", offset`; SplitScript's `at offset` form is an absolute virtual address. Use typed state paths for polled fields or `process.follow` for dynamically discovered paths.",
        targets: &[
            MigrationTarget::Language("state"),
            MigrationTarget::StandardLibraryItem("Process.follow"),
        ],
        cookbook_anchor: Some("asl-numeric-roots-are-module-relative"),
        spellings: &[],
    },
    MigrationConcept {
        id: MigrationConceptId::new("asl.memory.background-scan"),
        name: "Background signature scans",
        sources: ASL,
        support: MigrationSupport::TypedPattern,
        summary: "Remove legacy worker threads and await a module, explicit-range, or process-wide scan. Waiting scans repeat cooperative passes until a match appears; `process.scanOnce` completes with [`None`] after one bounded-range pass when absence is meaningful. Process closure cancels either form.",
        targets: &[
            MigrationTarget::StandardLibraryItem("Module.scan"),
            MigrationTarget::StandardLibraryItem("Module.scanAny"),
            MigrationTarget::StandardLibraryItem("Process.scan"),
            MigrationTarget::StandardLibraryItem("Process.scanOnce"),
            MigrationTarget::StandardLibraryItem("Process.scanMemory"),
            MigrationTarget::StandardLibraryItem("Process.scanMemoryAny"),
        ],
        cookbook_anchor: Some("background-signature-scans"),
        spellings: &[],
    },
    MigrationConcept {
        id: MigrationConceptId::new("asl.async.task-run"),
        name: "Task.Run",
        sources: ASL_CSHARP,
        support: MigrationSupport::TypedPattern,
        summary: "Replace worker threads with cooperative [`await`] discovery or a bounded [`retry`] transaction so timer updates keep yielding predictably.",
        targets: &[
            MigrationTarget::Language("await"),
            MigrationTarget::Language("retry"),
        ],
        cookbook_anchor: Some("background-signature-scans"),
        spellings: &[],
    },
    MigrationConcept {
        id: MigrationConceptId::new("asl.process.identity"),
        name: "Attached process identity",
        sources: ASL,
        support: MigrationSupport::Direct,
        summary: "Use `process.name()` to read the exact process candidate that matched during attachment; use module metadata when the executable name alone does not identify a build.",
        targets: &[MigrationTarget::StandardLibraryItem("Process.name")],
        cookbook_anchor: Some("attached-process-identity"),
        spellings: ASL_PROCESS_IDENTITY_SPELLINGS,
    },
    MigrationConcept {
        id: MigrationConceptId::new("asl.process.modules"),
        name: "Loaded module discovery",
        sources: ASL,
        support: MigrationSupport::TypedPattern,
        summary: "Replace the enumerable ASL module bag with the narrow typed probe that matches the source intent: main executable discovery, a known optional module, a required module, or typed build identity. Preserve genuine unknown-name enumeration as an explicit host-runtime gap.",
        targets: &[
            MigrationTarget::StandardLibraryItem("Process.mainModule"),
            MigrationTarget::StandardLibraryItem("Process.loadedModule"),
            MigrationTarget::StandardLibraryItem("Process.module"),
            MigrationTarget::StandardLibraryItem("Module.fileVersion"),
            MigrationTarget::StandardLibraryItem("Module.productVersion"),
            MigrationTarget::StandardLibraryItem("Module.versionInfo"),
            MigrationTarget::StandardLibraryItem("Module.md5"),
        ],
        cookbook_anchor: Some("attached-process-identity"),
        spellings: &[],
    },
    MigrationConcept {
        id: MigrationConceptId::new("asl.state.memory-watcher"),
        name: "MemoryWatcher",
        sources: ASL,
        support: MigrationSupport::TypedPattern,
        summary: "Declare polled memory in [`state`]; use a trailing field [`if`] with `value` and return `Err(message)` when a transient candidate should retain its last accepted value.",
        targets: &[MigrationTarget::Language("state")],
        cookbook_anchor: Some("retaining-the-last-accepted-field-value"),
        spellings: &[],
    },
    MigrationConcept {
        id: MigrationConceptId::new("asl.state.memory-watcher-list"),
        name: "MemoryWatcherList",
        sources: ASL_CSHARP,
        support: MigrationSupport::TypedPattern,
        summary: "Use [`state`] for a fixed set of named transactional reads or retain runtime-discovered homogeneous addresses in an array; managed collection enumeration remains a distinct provider requirement.",
        targets: &[MigrationTarget::Language("state")],
        cookbook_anchor: None,
        spellings: &[],
    },
    MigrationConcept {
        id: MigrationConceptId::new("asl.lifecycle.startup"),
        name: "startup lifecycle block",
        sources: ASL,
        support: MigrationSupport::TypedPattern,
        summary: "Use settings and global declarations for data, then [`setup`] for remaining process-independent startup statements.",
        targets: &[MigrationTarget::Language("setup")],
        cookbook_anchor: Some("legacy-asl-lifecycle-blocks"),
        spellings: &[],
    },
    MigrationConcept {
        id: MigrationConceptId::new("asl.lifecycle.init"),
        name: "init lifecycle block",
        sources: ASL,
        support: MigrationSupport::TypedPattern,
        summary: "Use [`onAttach`] for pre-poll process discovery and [`onStateReady`] for post-refresh snapshot initialization.",
        targets: &[
            MigrationTarget::Language("onAttach"),
            MigrationTarget::Language("onStateReady"),
        ],
        cookbook_anchor: Some("legacy-asl-lifecycle-blocks"),
        spellings: &[],
    },
    MigrationConcept {
        id: MigrationConceptId::new("asl.lifecycle.update"),
        name: "update lifecycle block",
        sources: ASL,
        support: MigrationSupport::TypedPattern,
        summary: "Use [`whileAttached`]; returning false skips the remaining timer decisions for that update.",
        targets: &[MigrationTarget::Language("whileAttached")],
        cookbook_anchor: Some("legacy-asl-lifecycle-blocks"),
        spellings: &[],
    },
    MigrationConcept {
        id: MigrationConceptId::new("asl.lifecycle.exit"),
        name: "exit lifecycle block",
        sources: ASL,
        support: MigrationSupport::TypedPattern,
        summary: "Use [`onDetach`] for cleanup that runs exactly once after an attached process closes.",
        targets: &[MigrationTarget::Language("onDetach")],
        cookbook_anchor: Some("legacy-asl-lifecycle-blocks"),
        spellings: &[],
    },
    MigrationConcept {
        id: MigrationConceptId::new("asl.lifecycle.exit-game-time-cleanup"),
        name: "Exit-time game-time cleanup",
        sources: ASL,
        support: MigrationSupport::Direct,
        summary: "Use [`onDetach`] for process-exit cleanup and explicitly pause or resume game time only when the original exit block changes that host state.",
        targets: &[
            MigrationTarget::Language("onDetach"),
            MigrationTarget::StandardLibraryItem("timer.pauseGameTime"),
            MigrationTarget::StandardLibraryItem("timer.resumeGameTime"),
        ],
        cookbook_anchor: Some("process-exit-game-time-cleanup"),
        spellings: &[],
    },
    MigrationConcept {
        id: MigrationConceptId::new("asl.lifecycle.shutdown"),
        name: "shutdown lifecycle block",
        sources: ASL,
        support: MigrationSupport::Planned,
        summary: "Exact script teardown needs the planned host shutdown notification; [`onDetach`] is not equivalent.",
        targets: &[],
        cookbook_anchor: Some("legacy-asl-lifecycle-blocks"),
        spellings: &[],
    },
    MigrationConcept {
        id: MigrationConceptId::new("asl.timer.start-reset-events"),
        name: "start and reset timer event handlers",
        sources: ASL,
        support: MigrationSupport::Direct,
        summary: "Keep [`onStart`] and [`onReset`]; SplitScript samples timer transitions before process attachment so both actions also run while detached.",
        targets: &[
            MigrationTarget::Language("onStart"),
            MigrationTarget::Language("onReset"),
        ],
        cookbook_anchor: Some("legacy-asl-lifecycle-blocks"),
        spellings: &[],
    },
    MigrationConcept {
        id: MigrationConceptId::new("asl.state.attempt-scoped"),
        name: "Attempt-scoped and run-scoped variables",
        sources: ASL,
        support: MigrationSupport::Direct,
        summary: "Declare a bare top-level [`let`] and assign it on every completing [`onStart`] path. The inferred attempt-scoped value remains available across process detach and is cleared after [`onReset`], replacing manually reset run-owned state in polling code.",
        targets: &[
            MigrationTarget::Language("let"),
            MigrationTarget::Language("onStart"),
            MigrationTarget::Language("onReset"),
        ],
        cookbook_anchor: Some("legacy-asl-lifecycle-blocks"),
        spellings: &[],
    },
    MigrationConcept {
        id: MigrationConceptId::new("asl.timer.events"),
        name: "split timer event handler",
        sources: ASL,
        support: MigrationSupport::Planned,
        summary: "Exact `onSplit` delivery still needs an ordered host event contract that can distinguish splits, skips, and undos between updates.",
        targets: &[],
        cookbook_anchor: Some("legacy-asl-lifecycle-blocks"),
        spellings: &[],
    },
    MigrationConcept {
        id: MigrationConceptId::new("asl.timer.state"),
        name: "Current timer state",
        sources: ASL,
        support: MigrationSupport::Direct,
        summary: "Replace `timer.CurrentPhase` with [`timer.state()`] and compare the exhaustive [`TimerState`] enum instead of relying on legacy numeric phase values.",
        targets: &[
            MigrationTarget::StandardLibraryItem("timer.state"),
            MigrationTarget::StandardLibraryType("TimerState"),
        ],
        cookbook_anchor: Some("timer-state"),
        spellings: ASL_TIMER_STATE_SPELLINGS,
    },
    MigrationConcept {
        id: MigrationConceptId::new("asl.timer.current-split-index"),
        name: "Current timer split index",
        sources: ASL,
        support: MigrationSupport::Direct,
        summary: "Call [`timer.currentSplitIndex()`] and handle its optional [`u64`] result so the host's negative no-attempt sentinel cannot become a route index.",
        targets: &[MigrationTarget::StandardLibraryItem(
            "timer.currentSplitIndex",
        )],
        cookbook_anchor: Some("timer-split-index"),
        spellings: &[],
    },
    MigrationConcept {
        id: MigrationConceptId::new("asl.timer.load-removal"),
        name: "Load removal",
        sources: ASL,
        support: MigrationSupport::Direct,
        summary: "Return the game's known loading state from [`isLoading`]; fall through or return [`None`] when the script has no new loading-state observation.",
        targets: &[MigrationTarget::Language("isLoading")],
        cookbook_anchor: Some("load-removal-and-computed-game-time"),
        spellings: &[],
    },
    MigrationConcept {
        id: MigrationConceptId::new("asl.timer.computed-game-time"),
        name: "Script-computed game time",
        sources: ASL,
        support: MigrationSupport::Direct,
        summary: "Return a typed [`Duration`] from [`gameTime`] when the game exposes its own elapsed clock; fall through or return [`None`] when no new value is available.",
        targets: &[
            MigrationTarget::Language("gameTime"),
            MigrationTarget::StandardLibraryType("Duration"),
            MigrationTarget::StandardLibraryItem("Duration.fromFrames"),
        ],
        cookbook_anchor: Some("load-removal-and-computed-game-time"),
        spellings: &[],
    },
    MigrationConcept {
        id: MigrationConceptId::new("asl.timer.current-real-time"),
        name: "LiveSplit current real time",
        sources: ASL,
        support: MigrationSupport::Planned,
        summary: "Use [`Instant`] only for independent elapsed-time checks; exact `timer.CurrentTime.RealTime` metadata requires additional host support.",
        targets: &[],
        cookbook_anchor: Some("monotonic-delays-and-debouncing"),
        spellings: &[],
    },
    MigrationConcept {
        id: MigrationConceptId::new("asl.timer.current-game-time"),
        name: "LiveSplit current game time",
        sources: ASL,
        support: MigrationSupport::Planned,
        summary: "Keep script-computed game time as a typed [`Duration`]; reading the host's coherent optional game-time snapshot requires additional runtime support.",
        targets: &[],
        cookbook_anchor: Some("livesplit-timer-metadata-and-control"),
        spellings: &[],
    },
    MigrationConcept {
        id: MigrationConceptId::new("asl.timer.run-metadata"),
        name: "LiveSplit run and segment metadata",
        sources: ASL,
        support: MigrationSupport::Planned,
        summary: "Current segment identity, route length, category/game names, and splits-file metadata require a typed read-only host snapshot.",
        targets: &[],
        cookbook_anchor: Some("livesplit-timer-metadata-and-control"),
        spellings: &[],
    },
    MigrationConcept {
        id: MigrationConceptId::new("asl.timer.controlled-mutation"),
        name: "LiveSplit timer configuration",
        sources: ASL,
        support: MigrationSupport::Planned,
        summary: "Run-offset and timing-method access needs an ordered least-privilege host contract; ports must not silently omit these user-visible mutations.",
        targets: &[],
        cookbook_anchor: Some("livesplit-timer-metadata-and-control"),
        spellings: &[],
    },
    MigrationConcept {
        id: MigrationConceptId::new("asl.settings.dynamic-lookup"),
        name: "Dynamic settings lookup",
        sources: ASL,
        support: MigrationSupport::Direct,
        summary: "Replace `settings[key]` with `settings.enabled(key)` and `settings.ContainsKey(key)` with `settings.contains(key)`. Declare exact host strings with `key \"...\"`; text, choice, and file settings remain statically typed.",
        targets: &[
            MigrationTarget::Language("settings"),
            MigrationTarget::Language("oldSettings"),
        ],
        cookbook_anchor: Some("static-settings-declarations"),
        spellings: &[],
    },
    MigrationConcept {
        id: MigrationConceptId::new("asl.settings.registration"),
        name: "Runtime settings registration",
        sources: ASL,
        support: MigrationSupport::TypedPattern,
        summary: "Move `settings.Add` calls into the static [`settings`] declaration, preserving the display label, stable host key, default, hierarchy, and tooltip explicitly. A bounded `settings.Add` loop becomes a compile-time [`settings family`] instead of hand-expanded declarations.",
        targets: &[
            MigrationTarget::Language("settings"),
            MigrationTarget::Language("oldSettings"),
            MigrationTarget::Language("stable setting key"),
            MigrationTarget::Language("settings family"),
        ],
        cookbook_anchor: Some("static-settings-declarations"),
        spellings: &[],
    },
    MigrationConcept {
        id: MigrationConceptId::new("asl.settings.finite-family"),
        name: "Finite startup-generated settings",
        sources: ASL,
        support: MigrationSupport::Direct,
        summary: "Use a compile-time settings family for bounded integer-keyed booleans; it lowers to ordinary declarations and remains available through `settings.enabled(key)`.",
        targets: &[MigrationTarget::Language("settings family")],
        cookbook_anchor: Some("finite-settings-families"),
        spellings: &[],
    },
    MigrationConcept {
        id: MigrationConceptId::new("asl.collection.list"),
        name: "List<T> collections",
        sources: ASL_CSHARP,
        support: MigrationSupport::TypedPattern,
        summary: "Use [`[T]`] for C# ordered list semantics; size-changing operations belong on variable-length arrays, while [`[T; N]`] remains fixed. In Unity schemas, [`List<T>`] describes remote list storage and reads produce owned arrays.",
        targets: &[
            MigrationTarget::StandardLibraryItem("Array.length"),
            MigrationTarget::StandardLibraryItem("Array.contains"),
            MigrationTarget::StandardLibraryItem("Array.indexOf"),
            MigrationTarget::StandardLibraryItem("Array.set"),
            MigrationTarget::StandardLibraryItem("Array.push"),
            MigrationTarget::StandardLibraryItem("Array.extend"),
            MigrationTarget::StandardLibraryItem("Array.remove"),
            MigrationTarget::StandardLibraryItem("Array.removeAt"),
            MigrationTarget::StandardLibraryItem("Array.pop"),
            MigrationTarget::StandardLibraryItem("Array.clear"),
        ],
        cookbook_anchor: Some("collection-search-and-run-scoped-sets"),
        spellings: &[],
    },
    MigrationConcept {
        id: MigrationConceptId::new("asl.state.mutable-current"),
        name: "Assignments to current",
        sources: ASL,
        support: MigrationSupport::Direct,
        summary: "Assign directly to `current.field` for an explicit post-read override; [`old`] remains read-only. Use a trailing state-field [`if`] when rejection must happen at the transactional acceptance boundary, including the first snapshot.",
        targets: &[MigrationTarget::Language("current")],
        cookbook_anchor: Some("retaining-the-last-accepted-field-value"),
        spellings: &[],
    },
    MigrationConcept {
        id: MigrationConceptId::new("asl.runtime.refresh-rate"),
        name: "refreshRate",
        sources: ASL,
        support: MigrationSupport::Direct,
        summary: "Use the declarative [`tickRate`] policy for stable attached and detached polling rates; reserve [`setTickRate`] for temporary dynamic changes.",
        targets: &[MigrationTarget::Language("tickRate")],
        cookbook_anchor: None,
        spellings: &[],
    },
    MigrationConcept {
        id: MigrationConceptId::new("asr.emulator.gba"),
        name: "GBA emulator attachment and memory mapping",
        sources: ASL_RUST,
        support: MigrationSupport::Direct,
        summary: "Use [`GBA`] for VisualBoyAdvance or VBA-M, mGBA, NO$GBA, Mednafen, supported RetroArch cores, and mGBA-based BizHawk. The provider owns emulator discovery and the `gba` root reads original EWRAM and IWRAM addresses without manual `DeepPointer` mappings.",
        targets: &[MigrationTarget::StateProvider("GBA")],
        cookbook_anchor: None,
        spellings: &[],
    },
    MigrationConcept {
        id: MigrationConceptId::new("asr.emulator.ps1"),
        name: "PlayStation emulator attachment and memory mapping",
        sources: ASL_RUST,
        support: MigrationSupport::Direct,
        summary: "Use [`PS1`] for ePSXe, pSX, DuckStation, Mednafen, PCSX-Redux, XEBRA, and supported RetroArch cores. The provider owns emulator discovery and the `ps1` root reads original PlayStation addresses.",
        targets: &[MigrationTarget::StateProvider("PS1")],
        cookbook_anchor: None,
        spellings: &[],
    },
    MigrationConcept {
        id: MigrationConceptId::new("asr.emulator.ps2"),
        name: "PlayStation 2 emulator attachment and memory mapping",
        sources: ASL_RUST,
        support: MigrationSupport::Direct,
        summary: "Use [`PS2`] for PCSX2 and the supported RetroArch PCSX2 core. The provider owns emulator discovery and the `ps2` root reads original PlayStation 2 addresses without manual host-memory mappings.",
        targets: &[MigrationTarget::StateProvider("PS2")],
        cookbook_anchor: None,
        spellings: &[],
    },
    MigrationConcept {
        id: MigrationConceptId::new("asr.emulator.sms"),
        name: "Master System and Game Gear emulator attachment",
        sources: ASL_RUST,
        support: MigrationSupport::Direct,
        summary: "Use [`SMS`] for Fusion, BlastEm, Mednafen, and supported RetroArch Master System or Game Gear cores. The provider owns emulator discovery and the `sms` root reads original work-RAM addresses.",
        targets: &[MigrationTarget::StateProvider("SMS")],
        cookbook_anchor: None,
        spellings: &[],
    },
    MigrationConcept {
        id: MigrationConceptId::new("asr.emulator.genesis"),
        name: "Genesis emulator attachment and memory mapping",
        sources: ASL_RUST,
        support: MigrationSupport::Direct,
        summary: "Use [`Genesis`] for Fusion, Gens, BlastEm, Sega Game Room or Genesis Classics, and supported RetroArch cores. The provider owns discovery, normalizes emulator storage and byte order, and reads original work-RAM offsets through `genesis`.",
        targets: &[MigrationTarget::StateProvider("Genesis")],
        cookbook_anchor: None,
        spellings: &[],
    },
    MigrationConcept {
        id: MigrationConceptId::new("asr.emulator.gcn"),
        name: "GameCube emulator attachment and memory mapping",
        sources: ASL_RUST,
        support: MigrationSupport::Direct,
        summary: "Use [`GCN`] for Dolphin and the supported RetroArch Dolphin core. The provider owns emulator discovery, address translation, and big-endian decoding, so `gcn` reads original GameCube addresses without manual byte swapping.",
        targets: &[MigrationTarget::StateProvider("GCN")],
        cookbook_anchor: None,
        spellings: &[],
    },
    MigrationConcept {
        id: MigrationConceptId::new("asr.emulator.wii"),
        name: "Wii emulator attachment and memory mapping",
        sources: ASL_RUST,
        support: MigrationSupport::Direct,
        summary: "Use [`Wii`] for Dolphin and the supported RetroArch Dolphin core. The provider owns emulator discovery, MEM1 and MEM2 translation, and big-endian decoding, so `wii` reads original Wii addresses without manual byte swapping.",
        targets: &[MigrationTarget::StateProvider("Wii")],
        cookbook_anchor: None,
        spellings: &[],
    },
    MigrationConcept {
        id: MigrationConceptId::new("asl.unity.managed-schema"),
        name: "UnityASL, mono.Make, and managed metadata",
        sources: ASL_CSHARP,
        support: MigrationSupport::Direct,
        summary: "Use the [`Unity`] state provider with top-level [`image`], [`namespace`], and [`class`] schemas instead of manually discovering Mono or IL2CPP metadata with `UnityASL`, `mono.Make<T>`, or `mono.MakeString`.",
        targets: &[
            MigrationTarget::StateProvider("Unity"),
            MigrationTarget::Language("image"),
            MigrationTarget::Language("namespace"),
            MigrationTarget::Language("class"),
            MigrationTarget::Language("static"),
            MigrationTarget::Language("from"),
        ],
        cookbook_anchor: Some("unityasl-and-managed-metadata"),
        spellings: &[],
    },
];

/// Exact documentation queries that are useful discovery phrases but are not
/// literal foreign syntax. These aliases are shared by CLI and editor search.
pub const QUERY_ALIASES: &[MigrationQueryAliases] = &[
    MigrationQueryAliases {
        concept: MigrationConceptId::new("asl.memory.primitive-types"),
        queries: &[
            "ASL primitive types",
            "ASL state types",
            "ASL numeric types",
            "ASL bool byte int",
            "ASL bool",
            "ASL byte",
            "ASL sbyte",
            "ASL short",
            "ASL ushort",
            "ASL int",
            "ASL uint",
            "ASL long",
            "ASL ulong",
            "ASL float",
            "ASL double",
            "ASL byteN",
        ],
    },
    MigrationQueryAliases {
        concept: MigrationConceptId::new("asl.collection.tuple-entry"),
        queries: &[
            "tuple collection",
            "tuple array",
            "List<Tuple>",
            "List<(key, value)>",
        ],
    },
    MigrationQueryAliases {
        concept: MigrationConceptId::new("asl.time.monotonic-delay"),
        queries: &[
            "Environment.TickCount",
            "Environment.TickCount64",
            "System.Environment.TickCount",
            "System.Environment.TickCount64",
            "Stopwatch",
            "DateTime.Now",
        ],
    },
    MigrationQueryAliases {
        concept: MigrationConceptId::new("asl.state.version-label"),
        queries: &[
            "duplicate ASL state blocks",
            "version labelled states",
            "version labeled states",
        ],
    },
    MigrationQueryAliases {
        concept: MigrationConceptId::new("asl.async.blocking-sleep"),
        queries: &["Thread.Sleep", "System.Threading.Thread.Sleep"],
    },
    MigrationQueryAliases {
        concept: MigrationConceptId::new("asl.file.last-write-time"),
        queries: &["File.GetLastWriteTime", "File.GetLastWriteTimeUtc"],
    },
    MigrationQueryAliases {
        concept: MigrationConceptId::new("asl.data.json"),
        queries: &[
            "JSON",
            "JsonConvert.DeserializeObject",
            "JObject",
            "System.Text.Json",
        ],
    },
];

pub fn concept(id: MigrationConceptId) -> Option<&'static MigrationConcept> {
    CONCEPTS.iter().find(|concept| concept.id == id)
}

pub fn diagnostic(id: MigrationDiagnosticId) -> Option<&'static MigrationDiagnostic> {
    DIAGNOSTICS.iter().find(|diagnostic| diagnostic.id == id)
}

pub fn foreign_spelling(
    spelling: &str,
    context: ForeignSpellingContext,
) -> Option<&'static ForeignSpelling> {
    CONCEPTS
        .iter()
        .flat_map(|concept| concept.spellings)
        .find(|candidate| candidate.spelling == spelling && candidate.context == context)
}

#[cfg(test)]
mod tests {
    use std::collections::HashSet;

    use super::*;

    #[test]
    fn concept_and_spelling_identities_are_unique() {
        let mut ids = HashSet::new();
        let mut diagnostic_ids = HashSet::new();
        let mut spellings = HashSet::new();
        let mut query_aliases = HashSet::new();
        for concept in CONCEPTS {
            assert!(
                ids.insert(concept.id),
                "duplicate ID {}",
                concept.id.as_str()
            );
            assert!(!concept.name.trim().is_empty());
            assert!(!concept.sources.is_empty());
            assert!(!concept.summary.trim().is_empty());
            for spelling in concept.spellings {
                assert!(
                    spellings.insert((spelling.context, spelling.spelling)),
                    "duplicate spelling {}",
                    spelling.spelling
                );
                if let ForeignSpellingReplacement::Text(replacement) = spelling.replacement {
                    assert!(!replacement.is_empty());
                }
            }
        }
        for aliases in QUERY_ALIASES {
            assert!(
                concept(aliases.concept).is_some(),
                "query aliases reference an unknown concept {}",
                aliases.concept.as_str()
            );
            assert!(!aliases.queries.is_empty());
            for query in aliases.queries {
                assert!(!query.trim().is_empty());
                assert!(
                    query_aliases.insert(query.to_ascii_lowercase()),
                    "duplicate query alias {query}"
                );
            }
        }
        for diagnostic in DIAGNOSTICS {
            assert!(
                diagnostic_ids.insert(diagnostic.id),
                "duplicate diagnostic ID {}",
                diagnostic.id.as_str()
            );
            assert!(
                concept(diagnostic.concept).is_some(),
                "diagnostic {} references an unknown concept",
                diagnostic.id.as_str()
            );
            assert!(!diagnostic.message.trim().is_empty());
            assert!(!diagnostic.primary_label.trim().is_empty());
        }
    }
}
