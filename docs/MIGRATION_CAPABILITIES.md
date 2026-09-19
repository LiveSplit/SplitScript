<!-- Generated from the compiler-owned migration catalog. -->
# Migration by source

Find the source concept or task first, then follow its canonical SplitScript direction. Exact APIs remain documented by the compiler-owned language and standard-library reference.

## ASL

Start with the [complete ASL porting guide](ASL_PORTING.md) for lifecycle and semantic context.

### Attachment and state

- **Attachment state declaration** (*Use a typed pattern*): Declare one native process name, an array of alternate process names, or a typed emulator provider per autosplitter file. An ASL declaration listing multiple processes becomes alternate names for one attachment, not concurrent attachments. Names are exact host identities, so Windows executable candidates currently include `.exe`. The declaration owns attachment and defines the fields polled into `old` and `current`. Canonical: `state`. [Porting recipe](ASL_PORTING.md#attachment-state-declarations).

- **Bounded native stringN state** (*Use a typed pattern*): Choose the native encoding explicitly: `utf8` bounds bytes and `utf16le` bounds two-byte code units. Canonical: `state`. [Porting recipe](ASL_PORTING.md#bounded-native-stringn-state).

- **Version-labelled state blocks** (*Use a typed pattern*): Use one state block, initialize an ordinary build enum in `onAttach`, and guard version-specific fields with it. Canonical: `state`. [Porting recipe](ASL_PORTING.md#version-labelled-asl-states).

- **Contiguous memory aggregates** (*Use a typed pattern*): Read physically contiguous values as one naturally aligned struct or fixed-length `[T; N]` array when that type exactly matches the target-memory layout. Canonical: `struct`, `[T; N]`, `state`, `Process.read`. [Porting recipe](ASL_PORTING.md#contiguous-structs-and-fixed-arrays).

- **State snapshots in helper functions** (*Supported directly*): Helpers may read `old` and `current` directly or accept caller-selected snapshots as inferred parameters. The compiler propagates direct snapshot requirements and rejects calls before committed snapshots exist. Canonical: `fn`, `old`, `current`. [Porting recipe](ASL_PORTING.md#snapshot-dependent-helper-functions).

- **MemoryWatcher** (*Use a typed pattern*): Declare polled memory in `state`; use a trailing field `if` with `value` and return `Err(message)` when a transient candidate should retain its last accepted value. Canonical: `state`. [Porting recipe](ASL_PORTING.md#retaining-the-last-accepted-field-value).

- **MemoryWatcherList** (*Use a typed pattern*): Use `state` for a fixed set of named transactional reads or retain runtime-discovered homogeneous addresses in an array; managed collection enumeration remains a distinct provider requirement. Canonical: `state`.

- **Attempt-scoped and run-scoped variables** (*Supported directly*): Declare a bare top-level `let` and assign it on every completing `onStart` path. The inferred attempt-scoped value remains available across process detach and is cleared after `onReset`, replacing manually reset run-owned state in polling code. Canonical: `let`, `onStart`, `onReset`. [Porting recipe](ASL_PORTING.md#legacy-asl-lifecycle-blocks).

- **Assignments to current** (*Supported directly*): Assign directly to `current.field` for an explicit post-read override; `old` remains read-only. Use a trailing state-field `if` when rejection must happen at the transactional acceptance boundary, including the first snapshot. Canonical: `current`. [Porting recipe](ASL_PORTING.md#retaining-the-last-accepted-field-value).

### Process and memory

- **Blocking sleeps** (*Use a typed pattern*): `Thread.Sleep` cannot block SplitScript's cooperative update loop. Model delayed work as state across ticks with `Instant` and `Duration`, or apply `future.timeout` when the source is bounding an existing asynchronous operation. Canonical: `Instant`, `Duration`, `future.timeout`. [Porting recipe](ASL_PORTING.md#monotonic-delays-and-debouncing).

- **ASL primitive state types** (*Use a typed pattern*): Preserve the bytes read by the ASL state declaration: `bool` is one byte; signed and unsigned integers and floating-point values map to the corresponding explicit-width SplitScript type. Treat `stringN`, `byteN`, pointers, enums, and values created in C# action code according to their actual representation rather than their nearest-looking name. Canonical: `bool`, `i8`, `u8`, `i16`, `u16`, `i32`, `u32`, `i64`, `u64`, `f32`, `f64`. [Porting recipe](ASL_PORTING.md#asl-primitive-state-types).

- **DeepPointer and native state roots** (*Supported directly*): A bare numeric root in an ASL native state field or `DeepPointer` is normally main-module-relative. Preserve it as `at "game.exe", offset`; SplitScript's `at offset` form is an absolute virtual address. Use typed state paths for polled fields or `process.follow` for dynamically discovered paths. Canonical: `state`, `Process.follow`. [Porting recipe](ASL_PORTING.md#asl-numeric-roots-are-module-relative).

- **Background signature scans** (*Use a typed pattern*): Remove legacy worker threads and await a module, explicit-range, or process-wide scan. Waiting scans repeat cooperative passes until a match appears; `process.scanOnce` completes with `None` after one bounded-range pass when absence is meaningful. Process closure cancels either form. Canonical: `Module.scan`, `Module.scanAny`, `Process.scan`, `Process.scanOnce`, `Process.scanMemory`, `Process.scanMemoryAny`. [Porting recipe](ASL_PORTING.md#background-signature-scans).

- **Task.Run** (*Use a typed pattern*): Replace worker threads with cooperative `await` discovery or a bounded `retry` transaction so timer updates keep yielding predictably. Canonical: `await`, `retry`. [Porting recipe](ASL_PORTING.md#background-signature-scans).

- **Attached process identity** (*Supported directly*): Use `process.name()` to read the exact process candidate that matched during attachment; use module metadata when the executable name alone does not identify a build. Canonical: `Process.name`. [Porting recipe](ASL_PORTING.md#attached-process-identity).

- **Loaded module discovery** (*Use a typed pattern*): Replace the enumerable ASL module bag with the narrow typed probe that matches the source intent: main executable discovery, a known optional module, a required module, or typed build identity. Preserve genuine unknown-name enumeration as an explicit host-runtime gap. Canonical: `Process.mainModule`, `Process.loadedModule`, `Process.module`, `Module.fileVersion`, `Module.productVersion`, `Module.versionInfo`, `Module.md5`. [Porting recipe](ASL_PORTING.md#attached-process-identity).

### Lifecycle and timer

- **Monotonic delays and debouncing** (*Use a typed pattern*): Replace elapsed-time uses of `DateTime.Now` or `Stopwatch` with an `Instant` captured at the source event and an exact `Duration` comparison. Canonical: `Instant`, `Duration`. [Porting recipe](ASL_PORTING.md#monotonic-delays-and-debouncing).

- **startup lifecycle block** (*Use a typed pattern*): Use settings and global declarations for data, then `setup` for remaining process-independent startup statements. Canonical: `setup`. [Porting recipe](ASL_PORTING.md#legacy-asl-lifecycle-blocks).

- **init lifecycle block** (*Use a typed pattern*): Use `onAttach` for pre-poll process discovery and `onStateReady` for post-refresh snapshot initialization. Canonical: `onAttach`, `onStateReady`. [Porting recipe](ASL_PORTING.md#legacy-asl-lifecycle-blocks).

- **update lifecycle block** (*Use a typed pattern*): Use `whileAttached`; returning false skips the remaining timer decisions for that update. Canonical: `whileAttached`. [Porting recipe](ASL_PORTING.md#legacy-asl-lifecycle-blocks).

- **exit lifecycle block** (*Use a typed pattern*): Use `onDetach` for cleanup that runs exactly once after an attached process closes. Canonical: `onDetach`. [Porting recipe](ASL_PORTING.md#legacy-asl-lifecycle-blocks).

- **Exit-time game-time cleanup** (*Supported directly*): Use `onDetach` for process-exit cleanup and explicitly pause or resume game time only when the original exit block changes that host state. Canonical: `onDetach`, `timer.pauseGameTime`, `timer.resumeGameTime`. [Porting recipe](ASL_PORTING.md#process-exit-game-time-cleanup).

- **start and reset timer event handlers** (*Supported directly*): Keep `onStart` and `onReset`; SplitScript samples timer transitions before process attachment so both actions also run while detached. Canonical: `onStart`, `onReset`. [Porting recipe](ASL_PORTING.md#legacy-asl-lifecycle-blocks).

- **Current timer state** (*Supported directly*): Replace `timer.CurrentPhase` with `timer.state()` and compare the exhaustive `TimerState` enum instead of relying on legacy numeric phase values. Canonical: `timer.state`, `TimerState`. [Porting recipe](ASL_PORTING.md#timer-state).

- **Current timer split index** (*Supported directly*): Call `timer.currentSplitIndex()` and handle its optional `u64` result so the host's negative no-attempt sentinel cannot become a route index. Canonical: `timer.currentSplitIndex`. [Porting recipe](ASL_PORTING.md#timer-split-index).

- **Load removal** (*Supported directly*): Return the game's known loading state from `isLoading`; fall through or return `None` when the script has no new loading-state observation. Canonical: `isLoading`. [Porting recipe](ASL_PORTING.md#load-removal-and-computed-game-time).

- **Script-computed game time** (*Supported directly*): Return a typed `Duration` from `gameTime` when the game exposes its own elapsed clock; fall through or return `None` when no new value is available. Canonical: `gameTime`, `Duration`, `Duration.fromFrames`. [Porting recipe](ASL_PORTING.md#load-removal-and-computed-game-time).

- **refreshRate** (*Supported directly*): Use the declarative `tickRate` policy for stable attached and detached polling rates; reserve `setTickRate` for temporary dynamic changes. Canonical: `tickRate`.

### Settings

- **Dynamic settings lookup** (*Supported directly*): Replace `settings[key]` with `settings.enabled(key)` and `settings.ContainsKey(key)` with `settings.contains(key)`. Declare exact host strings with `key "..."`; text, choice, and file settings remain statically typed. Canonical: `settings`, `oldSettings`. [Porting recipe](ASL_PORTING.md#static-settings-declarations).

- **Runtime settings registration** (*Use a typed pattern*): Move `settings.Add` calls into the static `settings` declaration, preserving the display label, stable host key, default, hierarchy, and tooltip explicitly. A bounded `settings.Add` loop becomes a compile-time `settings family` instead of hand-expanded declarations. Canonical: `settings`, `oldSettings`, `stable setting key`, `settings family`. [Porting recipe](ASL_PORTING.md#static-settings-declarations).

- **Finite startup-generated settings** (*Supported directly*): Use a compile-time settings family for bounded integer-keyed booleans; it lowers to ordinary declarations and remains available through `settings.enabled(key)`. Canonical: `settings family`. [Porting recipe](ASL_PORTING.md#finite-settings-families).

### Collections and text

- **Bounded integer ranges** (*Supported directly*): Use `..<` for an exclusive upper endpoint or `..=` for an inclusive one; SplitScript rejects bare `..` so the endpoint policy is explicit. Canonical: `range`. [Porting recipe](ASL_PORTING.md#bounded-integer-iteration).

- **Tuple-shaped collection entries** (*Use a typed pattern*): SplitScript does not use anonymous tuple element types. Give the entry shape a small named `struct`, store it in `[Entry]`, and destructure its fields in loops and patterns. Canonical: `struct`. [Porting recipe](ASL_PORTING.md#collection-search-and-run-scoped-sets).

- **List<T> collections** (*Use a typed pattern*): Use `[T]` for C# ordered list semantics; size-changing operations belong on variable-length arrays, while `[T; N]` remains fixed. In Unity schemas, `List<T>` describes remote list storage and reads produce owned arrays. Canonical: `[T].length`, `[T].contains`, `[T].indexOf`, `[T].set`, `[T].push`, `[T].extend`, `[T].remove`, `[T].removeAt`, `[T].pop`, `[T].clear`. [Porting recipe](ASL_PORTING.md#collection-search-and-run-scoped-sets).

### Unity and emulators

- **GBA emulator attachment and memory mapping** (*Supported directly*): Use `GBA` for VisualBoyAdvance or VBA-M, mGBA, NO$GBA, Mednafen, supported RetroArch cores, and mGBA-based BizHawk. The provider owns emulator discovery and the `gba` root reads original EWRAM and IWRAM addresses without manual `DeepPointer` mappings. Canonical: `GBA`.

- **PlayStation emulator attachment and memory mapping** (*Supported directly*): Use `PS1` for ePSXe, pSX, DuckStation, Mednafen, PCSX-Redux, XEBRA, and supported RetroArch cores. The provider owns emulator discovery and the `ps1` root reads original PlayStation addresses. Canonical: `PS1`.

- **PlayStation 2 emulator attachment and memory mapping** (*Supported directly*): Use `PS2` for PCSX2 and the supported RetroArch PCSX2 core. The provider owns emulator discovery and the `ps2` root reads original PlayStation 2 addresses without manual host-memory mappings. Canonical: `PS2`.

- **Master System and Game Gear emulator attachment** (*Supported directly*): Use `SMS` for Fusion, BlastEm, Mednafen, and supported RetroArch Master System or Game Gear cores. The provider owns emulator discovery and the `sms` root reads original work-RAM addresses. Canonical: `SMS`.

- **Genesis emulator attachment and memory mapping** (*Supported directly*): Use `Genesis` for Fusion, Gens, BlastEm, Sega Game Room or Genesis Classics, and supported RetroArch cores. The provider owns discovery, normalizes emulator storage and byte order, and reads original work-RAM offsets through `genesis`. Canonical: `Genesis`.

- **GameCube emulator attachment and memory mapping** (*Supported directly*): Use `GCN` for Dolphin and the supported RetroArch Dolphin core. The provider owns emulator discovery, address translation, and big-endian decoding, so `gcn` reads original GameCube addresses without manual byte swapping. Canonical: `GCN`.

- **Wii emulator attachment and memory mapping** (*Supported directly*): Use `Wii` for Dolphin and the supported RetroArch Dolphin core. The provider owns emulator discovery, MEM1 and MEM2 translation, and big-endian decoding, so `wii` reads original Wii addresses without manual byte swapping. Canonical: `Wii`.

- **UnityASL, mono.Make, and managed metadata** (*Supported directly*): Use the `Unity` state provider with top-level `image`, `namespace`, and `class` schemas instead of manually discovering Mono or IL2CPP metadata with `UnityASL`, `mono.Make<T>`, or `mono.MakeString`. Canonical: `Unity`, `image`, `namespace`, `class`, `static`, `from`. [Porting recipe](ASL_PORTING.md#unityasl-and-managed-metadata).

### Unsupported host behavior

- **File modification metadata** (*Planned*): `File.GetLastWriteTime` and `File.GetLastWriteTimeUtc` need a read-only metadata API and a host-defined timestamp contract. The current handle-free `File` surface reads complete bytes or UTF-8 text but does not expose metadata.

- **JSON data** (*Planned*): JSON parsing and serialization need a typed SplitScript data model; do not substitute ad-hoc string operations for `JsonConvert`, `JObject`, or `System.Text.Json` calls.

- **shutdown lifecycle block** (*Planned*): Exact script teardown needs the planned host shutdown notification; `onDetach` is not equivalent. [Porting recipe](ASL_PORTING.md#legacy-asl-lifecycle-blocks).

- **split timer event handler** (*Planned*): Exact `onSplit` delivery still needs an ordered host event contract that can distinguish splits, skips, and undos between updates. [Porting recipe](ASL_PORTING.md#legacy-asl-lifecycle-blocks).

- **LiveSplit current real time** (*Planned*): Use `Instant` only for independent elapsed-time checks; exact `timer.CurrentTime.RealTime` metadata requires additional host support. [Porting recipe](ASL_PORTING.md#monotonic-delays-and-debouncing).

- **LiveSplit current game time** (*Planned*): Keep script-computed game time as a typed `Duration`; reading the host's coherent optional game-time snapshot requires additional runtime support. [Porting recipe](ASL_PORTING.md#livesplit-timer-metadata-and-control).

- **LiveSplit run and segment metadata** (*Planned*): Current segment identity, route length, category/game names, and splits-file metadata require a typed read-only host snapshot. [Porting recipe](ASL_PORTING.md#livesplit-timer-metadata-and-control).

- **LiveSplit timer configuration** (*Planned*): Run-offset and timing-method access needs an ordered least-privilege host contract; ports must not silently omit these user-visible mutations. [Porting recipe](ASL_PORTING.md#livesplit-timer-metadata-and-control).

## C#

- **Variable declarations** (*Supported directly*): Use one inferred `let` declaration; SplitScript has no const/let split. Canonical: `let`.

- **Function declarations** (*Supported directly*): Functions and methods use the `fn` declaration keyword. Canonical: `fn`.

- **String type** (*Supported directly*): The immutable UTF-8 string type is named `String`. Canonical: `String`.

- **ASCII string lowercasing** (*Supported directly*): Use `toAsciiLowerCase` when game identifiers require ASCII-only normalization; this is not culture-sensitive Unicode lowercasing. Canonical: `String.toAsciiLowerCase`. [Porting recipe](ASL_PORTING.md#c-string-operations).

- **ASCII string uppercasing** (*Supported directly*): Use `toAsciiUpperCase` when game identifiers require ASCII-only normalization; this is not culture-sensitive Unicode uppercasing. Canonical: `String.toAsciiUpperCase`. [Porting recipe](ASL_PORTING.md#c-string-operations).

- **String equality** (*Supported directly*): Use `==` or `!=` for exact string content equality; use `equalsIgnoreAsciiCase` only when ASCII-insensitive matching is intended. Canonical: `String`, `String.equalsIgnoreAsciiCase`. [Porting recipe](ASL_PORTING.md#c-string-operations).

- **Bitwise complement** (*Supported directly*): Use type-directed `!`: it is logical negation for booleans and width-preserving bitwise complement for integers. Canonical: `Integer.bitNot`.

- **Substring extraction** (*Use a typed pattern*): Use fallible `slice(start, exclusiveEnd)` only after translating C#'s length argument and verifying that UTF-16 source positions are valid UTF-8 byte offsets. Canonical: `String.slice`. [Porting recipe](ASL_PORTING.md#c-string-operations).

- **Substring position** (*Use a typed pattern*): Use `indexOf` for an optional UTF-8 byte offset; review C# UTF-16 index arithmetic and replace the `-1` sentinel with `T?` handling. Canonical: `String.indexOf`. [Porting recipe](ASL_PORTING.md#c-string-operations).

- **Last substring position** (*Use a typed pattern*): Use `lastIndexOf` for an optional final UTF-8 byte offset; review C# UTF-16 index arithmetic and replace the `-1` sentinel with `T?` handling. Canonical: `String.lastIndexOf`. [Porting recipe](ASL_PORTING.md#c-string-operations).

- **Exact string replacement** (*Use a typed pattern*): Use fallible `replaceAll` for immutable exact replacement; explicitly handle failure and translate a null C# replacement to an empty string only when deletion was intended. Canonical: `String.replaceAll`. [Porting recipe](ASL_PORTING.md#c-string-operations).

- **ASCII whitespace trimming** (*Use a typed pattern*): Use `trimAsciiWhitespace` for text known to use ASCII boundary whitespace; review Unicode and character-set trimming explicitly. Canonical: `String.trimAsciiWhitespace`. [Porting recipe](ASL_PORTING.md#c-string-operations).

- **String padding** (*Use a typed pattern*): Use `padStart(width, fill)` or `padEnd(width, fill)` with an explicit character; review C# UTF-16 widths against SplitScript's Unicode-scalar widths. Canonical: `String.padStart`, `String.padEnd`. [Porting recipe](ASL_PORTING.md#c-string-operations).

- **Nullable string emptiness** (*Use a typed pattern*): Use `String.isEmpty` for required strings and match `String?` explicitly when absence should also count as empty. Canonical: `String.isEmpty`. [Porting recipe](ASL_PORTING.md#c-string-operations).

- **Nullable blank strings** (*Use a typed pattern*): Use `String.isBlank` for required strings and match `String?` explicitly when absence should also count as blank. Canonical: `String.isBlank`. [Porting recipe](ASL_PORTING.md#c-string-operations).

- **String length** (*Use a typed pattern*): Use `isEmpty()` for emptiness and `byteLength()` only for UTF-8 byte-oriented or proven ASCII logic; C# `Length` counts UTF-16 code units. Canonical: `String.isEmpty`, `String.byteLength`. [Porting recipe](ASL_PORTING.md#c-string-operations).

- **Array length** (*Supported directly*): Call `values.length()` for the `u32` element count of dynamic and fixed arrays. Canonical: `[T].length`. [Porting recipe](ASL_PORTING.md#collection-search-and-run-scoped-sets).

- **Bulk array extension** (*Supported directly*): Call `values.extend(moreValues)` to append a typed array in order; extending an array with itself duplicates its original contents once. Canonical: `[T].extend`. [Porting recipe](ASL_PORTING.md#collection-search-and-run-scoped-sets).

- **Unique-value collections** (*Supported directly*): Use `Set<T>` for growable unique values. Replace `HashSet<T>` with `Set<T>`, then review source operations whose names or return values differ. Canonical: `Set.new`. [Porting recipe](ASL_PORTING.md#collection-search-and-run-scoped-sets).

- **Key-value collections** (*Use a typed pattern*): Use `Map<K, V>` for growable key-value collections. Indexing returns a `MapEntry<K, V>` so an absent key remains distinguishable even when `V` is optional; use `Map.containsKey` when only membership matters. Canonical: `Map.new`. [Porting recipe](ASL_PORTING.md#collection-search-and-run-scoped-sets).

- **Collection count** (*Supported directly*): After choosing an array or set from the source's ordering and uniqueness requirements, call `values.length()` for its `u32` count. Canonical: `[T].length`, `Set.length`. [Porting recipe](ASL_PORTING.md#collection-search-and-run-scoped-sets).

- **String collection joining** (*Use a typed pattern*): Use `String.join(values, separator)` for a typed string array; convert C# object, variadic, enumerable, and range overloads explicitly. Canonical: `String.join`. [Porting recipe](ASL_PORTING.md#c-string-operations).

- **Floating-point square root** (*Use a typed pattern*): Use `value.sqrt()` with an explicit f32 or f64 boundary when preserving C# Math versus MathF semantics. Canonical: `Float.sqrt`.

- **Floating-point truncation** (*Use a typed pattern*): Use `value.truncate()` with an explicit f32 or f64 boundary when preserving C# Math versus MathF semantics. Canonical: `Float.truncate`.

- **Floating-point rounding** (*Use a typed pattern*): Use `value.round()` or `value.roundTo(digits)` for midpoint-to-even floating-point rounding; review result width, decimal inputs, and explicit midpoint modes. Canonical: `Float.round`, `Float.roundTo`.

- **Floating-point floor** (*Use a typed pattern*): Use `value.floor()` with an explicit f32 or f64 boundary; review C# decimal inputs separately. Canonical: `Float.floor`.

- **Floating-point ceiling** (*Use a typed pattern*): Use `value.ceil()` with an explicit f32 or f64 boundary; review C# decimal inputs separately. Canonical: `Float.ceil`.

- **Numeric minimum** (*Use a typed pattern*): Use `left.min(right)` after establishing one intended numeric type; review C# implicit conversions and decimal overloads. Canonical: `Numeric.min`.

- **Numeric maximum** (*Use a typed pattern*): Use `left.max(right)` after establishing one intended numeric type; review C# implicit conversions and decimal overloads. Canonical: `Numeric.max`.

- **Signed absolute value** (*Use a typed pattern*): Use `value.abs()` after establishing a signed numeric type; review C# signed-minimum overflow, unsigned conversions, and decimal inputs. Canonical: `Signed.abs`.

- **Numeric powers** (*Use a typed pattern*): Use `value.squared()` for the corpus-proven exponent two and an explicit typed shift for power-of-two masks; general floating powers remain planned. Canonical: `Numeric.squared`.

- **Numeric string parsing** (*Supported directly*): Replace static Parse/TryParse calls and output parameters with fallible `text.parse()` and ordinary `T!` handling. Canonical: `String.parse`. [Porting recipe](ASL_PORTING.md#c-string-operations).

- **Integer conversion** (*Use a typed pattern*): Choose fixed-width `as`, midpoint-to-even rounding, strict string parsing, or an explicit boolean mapping from the source type; C# checked overflow is not SplitScript cast behavior. Canonical: `as`, `if`, `String.parse`, `Float.round`. [Porting recipe](ASL_PORTING.md#c-convert-operations).

- **Floating-point conversion** (*Use a typed pattern*): Use an explicit f32/f64 `as` cast for numbers, `String.parse` for text, or an `if` expression for booleans. Canonical: `as`, `if`, `String.parse`. [Porting recipe](ASL_PORTING.md#c-convert-operations).

- **Boolean conversion** (*Use a typed pattern*): Use `value != 0` for numbers, the value itself for bool, and explicit trimmed ASCII-insensitive true/false handling for strings. Canonical: `if`, `String.trimAsciiWhitespace`, `String.equalsIgnoreAsciiCase`. [Porting recipe](ASL_PORTING.md#c-convert-operations).

- **Display conversion** (*Use a typed pattern*): Use `value as String` for ordinary Display conversion and `integer.toString(radix)` for bases 2 through 36; culture, null, and object overloads require separate policies. Canonical: `as`, `Integer.toString`. [Porting recipe](ASL_PORTING.md#c-convert-operations).

- **Timer durations** (*Supported directly*): Use `Duration` instead of C#'s `TimeSpan`. Canonical: `Duration`.

- **Text duration parsing** (*Use a typed pattern*): Replace `TimeSpan.Parse` according to whether the input is fixed data or an already-typed timer value; do not preserve culture-sensitive parsing by default. Canonical: `Duration.fromSeconds`, `Duration.fromMilliseconds`, `Duration.fromParts`.

- **C# duration ticks** (*Use a typed pattern*): Convert 100-nanosecond C# ticks to a source unit explicitly, with range review; SplitScript does not expose C# ticks as a native duration unit. Canonical: `Duration.fromNanoseconds`.

- **Fixed-width numeric types** (*Supported directly*): Memory-facing numbers use explicit signedness and bit widths. Canonical: `i8`, `u8`, `i16`, `u16`, `i32`, `u32`, `i64`, `u64`, `f32`, `f64`.

## JavaScript

- **Absent optional values** (*Supported directly*): `None` is SplitScript's zero-sized unit value and the absent side of an option. Canonical: `None`.

- **Strict equality operators** (*Supported directly*): Use typed `==` and `!=`; SplitScript has no coercing equality operators, so JavaScript's extra `=` is unnecessary. Canonical: `==`, `!=`.

## Rust

- **Numeric byte order** (*Supported directly*): Use `Numeric.swapBytes` to reverse a numeric value's raw bytes after reading data stored in the opposite byte order. It preserves the exact integer or floating-point type; eight-bit values are unchanged. Canonical: `Numeric.swapBytes`.
