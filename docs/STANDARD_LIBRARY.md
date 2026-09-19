# SplitScript standard library

The standard library provides reusable process, emulator, Unity, value, and
timer facilities. Game-specific names, addresses, signatures, and decisions
belong in the autosplitter; portable memory models, decoding, collections,
timing, and cancellation belong here.

Use this page to choose a workflow. Use the compiler-owned documentation
reference for exact signatures, constraints, effects, availability, related
symbols, and examples: open **SplitScript: Open Documentation** in VS Code or
run `splitc docs` in a terminal.

- For an ordinary desktop game, start with [native process support](#native-process-support).
- For a console game, choose the matching [typed emulator provider](#gba-emulator-support)
  and write original guest addresses in state fields.
- For managed Unity memory, use the [schema-first Unity workflow](#unity-managed-schemas).
- For transactional reads and failure handling, see
  [watchers, strings, and timing](#watchers-strings-and-timing).
- For compiler internals behind catalog items and lowering, use
  [`COMPILER.md`](COMPILER.md); those details are not public API.

## Native process support

An ordinary `state "game.exe" { ... }` selects the catalog's native state
provider. The executable names still come from the source declaration, but
attachment, the implicit value, direct state reads, documentation, and editor
behavior use the same provider model as emulator-backed states.

The provider exposes a read-only `process: Process` value. [`Process`] is a
nominal scalar handle, not a namespace: it can be passed to functions, returned,
or stored in inferred locals and globals. Open its generated type page for the
complete method set. Method lowering consumes the written receiver, so this is
valid ordinary typed code:

```splitscript
state "game.exe" {}

fn readScore(attached: Process, address: address) -> u32! {
    return attached.read(address)
}

whileAttached {
    let attached = process
    let score = readScore(attached, 0x1234) else 0
}
```

The native provider uses an identity attachment: the host's attached-process
handle already is the `Process` representation. Emulator-backed providers
instead name an ordinary standard-library function that asynchronously
constructs a different nominal value. The available providers are [`GBA`],
[`PS1`], [`PS2`], [`SMS`], [`Genesis`], [`GCN`], and [`Wii`]. This distinction
is catalog metadata, not a provider-name switch in the parser, checker, or
runtime lifecycle.

## GBA emulator support

`state GBA { ... }` selects the standard-library GBA state provider. The
provider attaches to a supported emulator, discovers its EWRAM and IWRAM
mapping, and exposes a read-only `gba: GBAEmulator` value. Its generic
[`GBAEmulator.read`] method accepts original GBA hardware addresses and infers
the memory representation from its expected result type. Reads outside
`0x02000000..0x02040000` and `0x03000000..0x03008000`, including reads that
cross either region boundary, return an error.

State fields with a fixed address should use the concise [`at`](syntax@at) syntax. The GBA
provider maps it through the same address translation and typed-read operation
as `gba.read`; the explicit method remains available for addresses computed by
the script.

```splitscript
state GBA {
    inventory: [u8; 6] at 0x02002B32
    scene: u8 at 0x03000BF4
}
```

`[T; N]` carries its exact element count in the type. When `T` is
`MemoryReadable`, the provider reads the complete fixed array in one host call
and constructs the GC array only after that call succeeds. It otherwise uses
ordinary array indexing, so `current.inventory[0]` reads the first byte
from the already captured snapshot.

Discovery covers VisualBoyAdvance/VBA-M, mGBA's contiguous mapping, NO$GBA,
standalone Mednafen, the supported RetroArch cores, and mGBA-based BizHawk.
Pointer-backed layouts refresh the current RAM base during reads so starting
or reloading a ROM does not leave the script with a stale mapping.

The emulator policy, signatures, and mapping selection live together in
`GBAEmulator.discover`, an ordinary source-defined standard-library function.
Memory-range lookup and module signature selection are bounded suspension
primitives: each poll inspects at most one range or one signature window before
returning control to the host. Only hardware-address translation and the final
host memory read remain compiler-provided representation primitives.

Each emulator provider also owns a private synchronous mapping probe. If that
probe fails before a state poll, the mapping—not merely the host process—is the
logical attachment boundary: attachment-scoped state is cleared, `onDetach`
runs for a completed attachment, and cooperative discovery resumes against the
still-open emulator. A replacement mapping starts a fresh `onAttach` lifecycle
and state baseline.

The provider owns the emulator executable list and attachment lifecycle.
Autosplitters do not call an attachment function or retain an optional handle.
Only `gba` is available as the process-access root in a GBA script; ordinary
native-process scripts use `process` instead. This keeps the two memory models
distinct and lets completion and diagnostics present only the applicable API.

## PlayStation 2 emulator support

`state PS2 { ... }` selects the PlayStation 2 provider and introduces a
read-only `ps2: PS2Emulator` value. It supports PCSX2 and 64-bit RetroArch with
the `pcsx2_libretro.dll` core. PCSX2 discovery prefers the exported `EEmem`
symbol and retains signature fallbacks for older 32-bit and 64-bit builds;
RetroArch resolves `retro_get_memory_data` from the loaded core.

Both [`PS2Emulator.read`] and state-field [`at`](syntax@at) declarations use original PS2
addresses. Reads must lie entirely within `0x00100000..=0x01ffffff`.
Provider-relative pointer paths dereference 32-bit guest pointers through the
same translation backend before applying each signed offset:

```splitscript
state PS2 {
    health: u16 at 0x00123456;
    inventory: u32 at 0x00110000, 0x20, 0x8;
}
```

Emulator translation is selected by the direct-read intrinsic's central
provider contract. Ordinary method calls and generated state polling consume
that same contract, so adding an emulator no longer requires a provider-name or
provider-specific branch in either code-generation path. Only the target address
translation remains a Rust runtime helper; attachment and discovery policy stay
in `stdlib/standard.split` as ordinary checked SplitScript source.

## PlayStation emulator support

`state PS1 { ... }` introduces a read-only `ps1: PS1Emulator` value and accepts
original PlayStation addresses in `0x80000000..=0x817fffff`. It supports ePSXe,
pSX, DuckStation, Mednafen, PCSX-Redux, XEBRA, and RetroArch cores based on
Beetle PSX, SwanStation, and PCSX ReARMed.

Discovery and signatures are source-defined. Stable emulators retain their RAM
base directly; DuckStation refreshes its exported or signature-discovered RAM
pointer on each read, and PCSX-Redux follows its discovered native pointer path
on each read. RetroArch validates that the selected core remains mapped before
using its discovered base.

```splitscript
state PS1 {
    health: u16 at 0x80012346;
    inventory: u32 at 0x80020000, 0x20, 0x8;
}
```

Explicit [`PS1Emulator.read`] calls and state-field pointer paths use the same
provider contract and little-endian `MemoryReadable` layouts.

## Sega Master System and Game Gear emulator support

`state SMS { ... }` introduces `sms: SMSEmulator` and maps original work-RAM
addresses in `0xc000..=0xdfff`. The source-defined provider covers Fusion,
BlastEm, Mednafen, and RetroArch cores for Genesis Plus GX, PicoDrive, SMS Plus,
and Gearsystem.

Fusion's moving native pointer is resolved on every read. Stable standalone and
libretro mappings retain their discovered base, while libretro reads first
validate that the selected core remains mapped.

```splitscript
state SMS {
    lives: u8 at 0xc010;
    inventory: u16 at 0xc100, 0x20, 0x4;
}
```

Both [`SMSEmulator.read`] and state-field pointer paths use the shared
provider-read contract and little-endian `MemoryReadable` layouts.

## Sega Genesis emulator support

`state Genesis { ... }` introduces `genesis: GenesisEmulator` and maps the
console's 64 KiB work RAM at offsets `0x0000..=0xffff`. The source-defined
provider covers Fusion, Gens, BlastEm, Sega Game Room, Sega Genesis Classics,
and RetroArch cores for BlastEm, Genesis Plus GX, Genesis Plus GX Wide, and
PicoDrive.

```splitscript
struct PlayerState {
    score: u32,
    velocity: i16,
}

state Genesis {
    player: PlayerState at 0x1201;
    inventory: u16 at 0x2000, 0x20, 0x4;
}
```

Fusion and Sega Classics refresh their moving native pointers at the read
boundary. Libretro reads validate that the selected core remains mapped. The
provider normalizes the reversed bytes within each native 16-bit word used by
Gens, BlastEm, Sega Classics, and the supported libretro cores, including
unaligned reads that cross word boundaries. Explicit [`GenesisEmulator.read`]
calls and state pointer paths then share the ordinary recursive big-endian
decoder for primitives, structs, fixed arrays, and guest pointers.

## Nintendo GameCube emulator support

`state GCN { ... }` introduces `gcn: GCNEmulator` and maps original MEM1
addresses in `0x80000000..=0x817fffff`. The source-defined provider supports
Dolphin and 64-bit RetroArch with `dolphin_libretro.dll`.

```splitscript
struct PlayerState {
    health: u16,
    position: [f32; 3],
}

state GCN {
    player: PlayerState at 0x80001000;
}
```

GameCube reads use provider-owned big-endian decoding. The same recursive
decoder handles primitive values, every field of a readable struct, fixed
arrays, and intermediate `u32` pointers in state-field pointer paths.

## Nintendo Wii emulator support

`state Wii { ... }` introduces `wii: WiiEmulator` and maps original MEM1
addresses in `0x80000000..=0x817fffff` and MEM2 addresses in
`0x90000000..=0x93ffffff`. Dolphin and 64-bit RetroArch with
`dolphin_libretro.dll` are supported.

```splitscript
state Wii {
    player: PlayerState at 0x80001000;
    worldState: u32 at 0x90002000;
}
```

Explicit reads and state pointer paths share the same bounds checks, address
translation, and recursive big-endian decoder as the GameCube provider.

## Exact API reference

This overview explains when to use each standard-library domain. Exact public
members are generated from the compiler catalog rather than copied here.
Open the documentation page for a type, capability, provider, method, field,
variant, or operator to see its current signature, constraints, effects,
runtime availability, related symbols, and compiler-checked examples.

The catalog, semantic-identity, inference, lowering, and documentation
architecture is contributor material documented in [`COMPILER.md`](COMPILER.md).

## Value layer

`String` is a first-class, statically typed value backed by a WebAssembly GC
array of immutable UTF-8 bytes. String literals can be inferred into locals,
stored in continuation frames across `await`, compared by content with `==`
and `!=`, and passed to APIs such as `print`.

```text
let message = "Assembly-CSharp is ready"

if (message.byteLength() != 0) {
    print(message)
}
```

The runtime ABI still accepts strings through linear memory. A single standard
library boundary adapter grows the exported memory when necessary, copies the
GC string to scratch space, and invokes the host. Language code never receives
or manages that scratch pointer.

Reusable functions, type-directed methods, named immutable GC structs, payload
enums with exhaustive matching, mutable arrays, sets, ranges, iterators,
closures, optional and fallible wrappers, UTF-16 decoding, numeric formatting,
and string construction are part of the value layer. They can be nested and
retained across suspension. Open the generated page for a value type or
capability to see every available member and the bounds under which it exists.

## ASR layer

The reusable ASR surface is grouped by responsibility:

- Live user settings with nested headings and tooltips, booleans, free-form
  text inputs with defaults, enum-backed choices, file selectors with
  glob/MIME filters, and typed current/previous tick snapshots.
- Process attachment to an ordered list of executable names, GC `Module`
  values containing base and size, and managed attachment-lifetime cancellation.
- Compile-time parsed `sig"..."` literals and overlapping page-based module
  scanning, including full-byte and nibble wildcards.
- Typed synchronous and retrying reads for fixed-width primitives and naturally
  laid-out readable structs, non-null address discovery, 64-bit pointer
  traversal, RIP-relative decoding, and arbitrary-range scans.
- [`nextTick`] as a host-independent one-update suspension integrated with the
  implicit async action state machine and attachment-lifetime cancellation. The
  language-level `retry expression` form polls arbitrary `T!` expressions;
  race combinators remain planned on the same foundation.
- Unity IL2CPP module/image/class/field/static-instance discovery, versioned
  layouts, generated typed bindings, and managed-string decoding.
- Unity Mono assembly/image/class/inherited-field/static-table discovery for
  modern 64-bit Windows V2 and V3 metadata layouts.
- Watchers with current/old pairs and change predicates.
- Timer state and controls, custom runtime variables, tick-rate control, and
  saturating duration conversion.

These APIs are implemented as ordinary typed standard-library facilities where
possible. Compiler lowering is reserved for representation primitives, host
imports, static signature data, and suspension points that cannot yet be
expressed as normal source code.

## Unity managed schemas

Managed Mono and IL2CPP memory has one canonical public workflow: declare a
top-level schema and use the [`Unity`] state provider. Scripts do not discover
runtime images, classes, static tables, field offsets, or metadata pointers.
They declare the managed names and types they consume; `state Unity` discovers
the backend, binds the reachable part of that schema once per attachment, and
exposes typed fallible references:

```splitscript
image "Assembly-CSharp" {
    class PlayerStats {
        static PlayerStats current from ["Instance", "_instance"];
        i32 district from "currDistrict";
        bool inRun;
    }
}

state Unity ["game.exe"] {
    district: i32 = PlayerStats.current?.district?;
    inRun: bool = PlayerStats.current?.inRun?;
}
```

`state Unity` automatically distinguishes supported Mono and IL2CPP backends.
Prefer this form unless the exact target layout is already known and automatic
detection is inappropriate. Such a target may select
`Unity.mono(MonoVersion.V1)`, `Unity.mono(MonoVersion.V1Cattrs)`,
`Unity.mono(MonoVersion.V2)`, `Unity.mono(MonoVersion.V3)`, or an IL2CPP layout
such as `Unity.il2cpp(Il2CppProfile.unity2021_3_11f1X64())` in the state header. These selectors configure the
provider; they are not callable discovery functions.

IL2CPP profiles contain complete measured layouts, with x86 and x64 factories
for 11 Unity releases. Automatic selection reads all four numeric file-version
components from `UnityPlayer.dll`, prefers an exact match at the target width,
then selects the newest measured major/minor family not newer than the target
(or the oldest profile when the target predates the catalog). The runtime log
names both the player version and selected profile. Explicit selection checks
the GameAssembly pointer width and omits player-version lookup and the catalog.

For a separately measured player, construct an `Il2CppProfile` struct literal
with every field supplied. Optional unknown facts must be `None`; offsets are
never filled in from another profile. A provider argument may call a constructor
function whose body only returns a value composed from constant inputs. Custom
profiles are validated before metadata scanning; malformed layouts and pointer
width mismatches reject the attachment until that process closes.

On Windows, automatic Mono selection first matches the runtime DLL's PDB GUID
and age against 26 measured profiles and verifies its pointer width. Missing,
unknown, or unreadable identities fall back to family detection: class metadata
distinguishes the two old `mono.dll` families, and the UnityPlayer file version
distinguishes modern V2/V3. Selection reports its route in the runtime log.
Explicit Mono selectors skip PDB discovery and the measured-build table. All four
families support 32-bit and 64-bit metadata and static fields; managed string
payloads still require the forthcoming width-aware reader migration.

The same provider exposes `unity: UnityContext` as a read-only,
attachment-scoped value. Its [`UnityContext.scenes`](field@UnityContext.scenes)
facility is prepared once only when referenced and snapshots Unity's native
active, loaded, and persistent `DontDestroyOnLoad` scenes:

```splitscript
state Unity ["game.exe"] {
    activeScene = unity.scenes.active();
    loadedScenes = unity.scenes.loaded();
    persistentScene = unity.scenes.persistent();
}
```

Scene operations are fallible and participate in the ordinary state-field
retention boundary. The resulting [`UnityScene`] values are immutable local
snapshots, so `old` remains stable if Unity later unloads or reuses the native
scene address. Native scene discovery does not also trigger managed-runtime
metadata discovery unless a managed schema is reachable.

`from "name"` supplies an exact metadata name and `from ["first", "second"]`
supplies alternative names. Instance fields without `from` also recognize
the conventional C# automatic-property backing-field spelling. Nested classes
use .NET names such as `class Leaf from "Game.Outer+Middle+Leaf"`. Both backends
check every enclosing class, require the outermost class to have no further
declaring parent, and read the namespace from that outermost class. Unqualified
nested names require an empty outer namespace; unqualified flat names still
match any namespace. Profiles without a measured declaring-class offset reject
nested lookup explicitly. Unreadable metadata retries and cancels on process exit.
Schema bindings with only flat names omit the nested-name matcher entirely;
dynamic class-name APIs retain the general matcher.

Mono inflated generic classes read their field count from the generic definition
while retaining the inflated class's field array, offsets, and static owner.
Null or unreadable generic metadata remains pending instead of falling back to
an unrelated count slot. Mono class-cache traversal yields every 64 entries,
detects linked-list cycles, and limits a search to 1,048,576 classes; field walks
limit each class to 65,535 fields and inheritance to 128 classes.

Both backends use the same field cursor. It performs at most 64 metadata steps
per update and 1,048,576 steps per lookup, precomputes backing-field aliases,
and scans the full supported inheritance chain before accepting a unique field.
Distinct owners or offsets are ambiguous even when the names are alternatives;
aliases for the same owner and offset denote one field. The cursor stops before
`UnityEngine.MonoBehaviour` and `System.Object`; same-named game classes and other
name/namespace combinations remain searchable. Null-name slots are holes;
unreadable names or offsets retry without accepting a partial search. Negative
field offsets reject unsupported thread-static storage. Static reads always use
the matched field's declaring class, and `UnityField.index` is its metadata index
within that class on both backends.

Class-typed
static and instance fields are live references: every state poll rereads the
current singleton and following object pointers instead of caching a transient
object address during attachment. Scalar leaves use their declared fixed-width
type and do not allocate a new GC object. Strings, arrays, explicit class
snapshots, and completed instance searches materialize owned values.

Field-address addition and complete scalar, reference, and string read spans
are checked against the target's pointer width before a host read. Overflow is
a failed read even if the wrapped address would be readable. Static binding
retries invalid table-plus-offset addresses and rereads the table after yielding,
so corrected storage can become available without restarting attachment.

The declared class name `T` is an immutable local snapshot, while `T.Ref` is a
live remote object reference. Each instance-field hop from `T.Ref` is fallible.
Use postfix `?` to propagate a failed hop to the surrounding state field,
function, or `retry` boundary. Calling `reference.snapshot()` reads every
active instance field before constructing `T`. Declared child classes become
owned child snapshots recursively: a field declared `Child` is `Child.Ref` on
the live parent and `Child` on its snapshot; `Child?` similarly becomes
`Child.Ref?` or `Child?`. A null optional child becomes `None`, while unreadable
memory still fails. Explicit `Child.Ref` fields cannot be materialized in an
owned snapshot.

If any nested field fails, the whole operation returns an error and no partial
snapshot escapes. Previous snapshots keep their values when remote objects
change or a later read fails. Recursive schemas are allowed, but an object
cycle along the active read path fails; a shared child reached through separate
completed paths is copied independently. Each nested snapshot operation is
limited to 64 active objects, 1,024 total object visits, and 16,384 active field
reads. Conditional fields follow the active attachment shape and inactive
fields do not consume this work budget. Snapshot readers and the shared read
context are generated only when needed:

```splitscript
state Unity ["game.exe"] {
    manager: GameManager = GameManager.instance?.snapshot()?;
}
```

`await T.instances()` returns a completed `[T.Ref]` snapshot of live objects.
The compiler binds the class's IL2CPP class pointer or active Mono vtable once
per attachment, then cooperatively scans readable, writable, non-executable
memory at the target's natural pointer alignment. Both byte work and matches
are bounded per poll, and closing the process cancels the unfinished scan:

```splitscript
onAttach {
    let enemies = await Enemy.instances()
    print(enemies.length())
}
```

The runtime traversal types and raw metadata offsets remain private to the
trusted standard library and generated schema binder. Generated backend
discovery, metadata bindings, field readers, snapshot readers, and instance
scanners are retained only when the checked script can reach them.
Declaring an unread field, class, or image does not require that metadata to
exist in the game. Reads in unreachable functions and Debug-only reads in a
Release build do not bind metadata. A reachable `snapshot()` still requires
every active instance field, and automatic attachment-shape selection still
probes its metadata evidence even when the script never reads those fields.

The IL2CPP implementation supports the existing 64-bit base, 2019, 2020, and
2022 layouts. The Mono implementation supports modern 64-bit Windows V2 and V3
metadata layouts, where V3 corresponds to Unity 2021.2 and newer. Older Mono V1,
32-bit, ELF, and Mach-O layouts require explicit backend support rather than
guessed offsets. Both binders yield between bounded discovery work so attachment
does not monopolize a timer update.

Loaded Windows modules expose [`Module.peExport`] as ordinary
source-defined standard-library composition. It validates the mapped PE export
directory, name/ordinal/function tables, and rejects forwarded exports rather
than pretending their forwarder string is executable code. This is useful for
runtime metadata discovery such as Mono's `mono_assembly_foreach`; it is not a
cross-platform symbol API, and malformed or absent exports remain ordinary
`T!` errors.

Build identities are available through [`Module.peDebugId`],
[`Module.elfBuildId`], and [`Module.machUuid`]. The PE reader returns a
[`PeDebugId`] containing the stored GUID bytes and PDB age; both components
participate in equality. ELF reads preserve 1–32 GNU build-ID bytes and account
for relocation. Mach-O reads return the active mapped 64-bit image's 16 UUID
bytes. These methods return an optional identity inside a fallible result:
`None` means a supported image has no supported identity record; an error
means malformed, unsupported, unreadable, or excessive metadata. They validate
mapped bounds before reading and use finite traversal limits. They do not
read an on-disk universal binary or select a Unity runtime profile themselves.

An [`address`] supports [`address.offset`] for integer displacements and
[`address.add`] for unsigned full-width deltas. Signed arguments retain their sign;
smaller integer widths are extended before addition. Both wrap modulo the 64-bit address space while
keeping target pointers nominally distinct from numeric sizes. Static [`at`](syntax@at)
paths keep their absolute root unsigned, but module-relative and
post-dereference offsets are signed. `process.follow` accepts `[i64]`, and
`MemoryPath` stores signed dereference and final offsets, so negative native
pointer paths do not require lossy casts.

## Watchers, strings, and timing

Expression-backed state fields form persistent watchers. Initialization waits
for every required field to succeed in one poll and seeds `old == current`.
Later, each successful `T!` advances that field and each error retains its last
accepted value; actions see the resulting `current` and prior `old` objects.
Fields may refer to siblings from the same state snapshot independent of source
order. The compiler evaluates the resulting dependency graph in topological
order and rejects cycles. A sibling can also be the dynamic base of an
[`at`](syntax@at) pointer path. When a dependency fails, its
dependents are skipped for that poll and retain their own accepted values,
preventing a stale candidate address from being dereferenced.
[`Process.read`] infers its
`MemoryReadable` type from the field, annotation, or later usage. This includes
fixed-width primitives and both source- and catalog-declared structs containing
only readable fields. Struct fields use declaration order and natural
alignment; one host read obtains the complete layout before the compiler
recursively constructs its GC value.
[`Result.discardError`] is ordinary source-defined library composition over
wrapper matching. It turns success into a present `T?` and error into `None`,
which lets one intentionally unavailable state field commit as absent while
all remaining required field failures continue to reject the transaction. It
discards the error string and therefore is not a general replacement for
handling or propagating failures.
Immediate process operations return `T!`: fixed-layout reads, pointer following,
relative-address decoding, and string decoding. They can be handled
synchronously with `else` or `?`; `retry expression` polls any of them across
attached updates and yields `T`. Native NUL-terminated UTF-8 uses
[`Process.readUtf8`]. The bound is part of the operation, so
all successful values have the ordinary `String` type rather than generated
`string32`-style types. Pointer state fields can write
`name at address as utf8(maxBytes)`; this is sugar for the same strict,
bounded, fallible decode after the pointer path is resolved. Native
NUL-terminated UTF-16LE similarly uses
[`Process.readUtf16Le`] or
`name at address as utf16le(maxUtf16Units)`. Its bound is measured in code
units, malformed surrogate sequences become the Unicode replacement character,
and successful values are still ordinary `String` values. Unity managed
strings are declared directly in an [`image`] / [`class`] schema as
`String field;`. The generated reader follows the
managed object layout and stored length, rejects inaccessible payloads or
exhausted materialization budgets, and replaces
malformed surrogate sequences with the Unicode replacement character.

Numeric conversions and integer formatting use `value as Type`. The [`Display`]
capability is the single contract for `as String`, JavaScript-style template
strings such as `` `{stage}-{act}` ``, `print`, and `setVariable`.
Standard-library nominal types may fulfill that contract with an `@display`
source method; primitives retain their compact compiler implementation.
[`String.concat`] remains available as the underlying collection helper.
[`Timer.state`], [`Timer.pauseGameTime`], [`Timer.resumeGameTime`], and [`setTickRate`]
wrap their ASR host calls. [`isLoading`] remains the normal declarative load-
removal API; explicit pause/resume is for lifecycle transitions such as process
exit cleanup. The language-level [`tickRate`] policy defaults to 120 Hz attached
and 1 Hz detached and reapplies those rates at lifecycle transitions.
[`setTickRate`] is the dynamic escape hatch: it is measured in updates per
second, affects the next host wait after the current update, and persists until
another call or attachment-state transition. It accepts every integer and
floating-point type and converts the value only at the host's `f64` ABI boundary.
[`Timer.state`] returns the exhaustive [`TimerState`] enum with
`NotRunning`, `Running`, `Paused`, `Ended`, and `Unknown`; raw host integers are
normalized only at the ABI boundary. [`Duration.fromSeconds`] converts Unity's
floating-point level clock into timer game time.
