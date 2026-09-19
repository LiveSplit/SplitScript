# Porting ASL to SplitScript

This self-contained guide records mappings proven by maintained, host-executed
ports. It is not a token-substitution table: ASL's dynamic state and C# runtime
sometimes need a typed SplitScript design rather than a literal translation.
Every required semantic distinction and canonical pattern is explained here;
it has no source-file or repository-document prerequisites. Examples are small,
focused snippets that explain one concept rather than complete autosplitters.
Every SplitScript snippet is compiled as an independent focused program during
repository verification. The rendered guide omits its rustdoc-style hidden
setup, so visible code stays limited to the concept while types, effects,
lifecycle availability, and current API spellings remain checked.

## Review a compiler-clean port

A successful build proves that the port is valid SplitScript. It does not prove
that the script attaches to the intended game or preserves the source behavior.
Before runtime testing, review these semantic choices:

- **Process identity:** Check every exact process candidate against the host.
  Windows executable identities include `.exe`. An array represents alternate
  names for one attachment, not several simultaneous attachments.
- **Selected build:** Preserve every source version distinction that changes an
  address, type, or behavior. Initialize one or more ordinary enum globals from
  reliable evidence, use them to guard conditional fields, and handle an
  unknown build explicitly instead of silently choosing a default.
- **Provider choice:** Use the canonical typed provider. Unity ports declare
  [`image`] schemas under [`Unity`]. Emulator ports use the provider for the
  original console rather than manually rediscovering emulator memory.
- **Byte order:** Let an emulator provider decode its console's memory. For
  native data, retain the source format's byte order deliberately and use
  [`Numeric.swapBytes`] only when the stored order differs from the host order.
- **Lifecycle ownership:** Place attachment discovery in [`onAttach`], polled
  observation in state fields or [`whileAttached`], timer decisions in their
  action blocks, and attempt state in [`onStart`] when appropriate. Confirm the
  relative timing against the source instead of moving code merely because it
  compiles in another block.
- **Settings reachability:** Confirm that every behavior gate reads its typed
  setting and that every exposed setting can affect reachable behavior. The
  unused-setting warning is review evidence, not just cleanup advice.
- **Integer width:** Preserve signedness and physical width for every memory
  field, constant, sentinel, mask, and comparison. Do not widen a field merely
  because its current values happen to fit.
- **Failure behavior:** Decide whether absence should retry, wait, skip the
  current tick, use an honest default, detach, or stop an unsupported
  attachment. An [`else`] that makes code compile can still change behavior.
- **Omitted branches:** Record every source branch, setting, platform, version,
  and timer behavior left out of the port. An intentional omission is valid;
  an invisible omission is a semantic regression.

Use `splitc docs search` with the source vocabulary while reviewing. Searches
such as `.exe`, `version labelled states`, `settings.Add`, `UnityASL`,
`mono.Make`, `Dolphin`, `DeepPointer`, and `TryParse` lead to the canonical
language or provider pages. Keep the review record beside the maintained port;
generated full-script candidates are evidence to inspect, not fixtures to
preserve.

When one ASL expression establishes several related locals, an irrefutable
[`binding pattern`] can unpack a struct, fixed array, or uniquely possible enum
variant directly in an initialized [`let`], function parameter, closure
parameter, or runtime [`for`] binding. The compiler rejects a pattern whenever
another runtime shape is possible and points to [`is`] or [`match`] instead;
this keeps destructuring from silently skipping an update.

## Attachment state declarations

Every SplitScript file is one executable autosplitter and declares exactly one
attachment provider. A native Windows game names the exact process candidate
that the host must match:

```splitscript
state "game.exe" {
    health: i32 at 0x1000;
}
```

Use an array when editions have different executable names but share the same
state shape:

```splitscript
state ["game.exe", "game-demo.exe"] {
    health: i32 at 0x1000;
}
```

These strings are exact host process identities, not portable paths. The
current Windows host reports the executable filename including `.exe`, so a
Windows candidate must include that extension; `state "game"` will not attach
to `game.exe`. Other host platforms use their exact runtime identity. The array
contains alternate names for one attachment, not several processes to attach
to concurrently. Build-specific addresses belong in conditional field branches
selected by enum globals initialized from [`onAttach`], rather than in multiple
ASL-style state blocks.

Typed emulator support replaces the native process root. Choose the provider
for the emulated console: [`GBA`], [`PS1`], [`PS2`], [`SMS`], [`Genesis`],
[`GCN`], or [`Wii`]. Each provider owns the supported emulator process names,
memory discovery, guest-address translation, byte order, and attachment
lifecycle. State fields therefore use original console addresses instead of
ASL `DeepPointer` mappings or host-memory byte swapping. The provider also
introduces a matching read root (`gba`, `ps1`, `ps2`, `sms`, `genesis`, `gcn`,
or `wii`) for dynamically computed addresses.

For example, a GBA autosplitter reads original GBA hardware addresses directly:

```splitscript
state GBA {
    room: u8 at 0x03000010;
}
```

Search the reference for the console or emulator name, such as `Dolphin`,
`Fusion`, `mGBA`, `PCSX2`, or `RetroArch`, to find the corresponding provider
and its exact supported hosts.

The state declaration also defines the transactional snapshots. After the
first complete poll, [`current`] contains the latest accepted values and [`old`]
contains the preceding accepted values. `process` or the provider-specific
root is available only during attachment-owned lifecycle phases; [`old`] and
[`current`] are unavailable before the first complete snapshot and are not
guaranteed during [`onDetach`].

## ASL primitive state types

For a type written directly in an ASL [`state`] declaration, preserve the exact
bytes that LiveSplit reads. The scalar mappings are:

| ASL state type | SplitScript type | Bytes read |
| --- | --- | ---: |
| <code>bool</code> | [`bool`] | 1 |
| `byte` | [`u8`] | 1 |
| `sbyte` | [`i8`] | 1 |
| `short` | [`i16`] | 2 |
| `ushort` | [`u16`] | 2 |
| `int` | [`i32`] | 4 |
| `uint` | [`u32`] | 4 |
| `long` | [`i64`] | 8 |
| `ulong` | [`u64`] | 8 |
| `float` | [`f32`] | 4 |
| `double` | [`f64`] | 8 |

In particular, ASL deliberately reads <code>bool</code> as one byte; do not
substitute a four-byte C or platform ABI boolean. For example:

```asl
state("game") {
    bool loading: 0x1234;
    int room: 0x1238;
}
```

becomes:

```splitscript
state "game.exe" {
    loading: bool at "game.exe", 0x1234;
    room: i32 at "game.exe", 0x1238;
}
```

This table applies to ASL state fields, not every similarly named value in the
C# action bodies. Inspect the source and target representation in these cases:

- ASL `byteN` reads exactly `N` raw bytes. Preserve that as a fixed
  [`[T; N]`] of [`u8`] only when the port still needs the same raw
  buffer; otherwise decode the actual format deliberately.
- ASL `stringN` also reads `N` bytes, then guesses UTF-16LE from the second byte
  and otherwise decodes as UTF-8. SplitScript requires the real encoding and
  bound; follow [Bounded native `stringN` state](#bounded-native-stringn-state).
- An enum, flags value, sentinel, or pointer stored in a numeric field still
  has the width and signedness declared by the ASL watcher. Give it a richer
  local type only after the memory read preserves those bytes.
- C# locals, `vars` entries, casts, `IntPtr` values, custom structs, and manual
  `ReadValue<T>` calls are not ASL primitive state declarations. Inspect the
  type or runtime read that creates the value instead of choosing a
  SplitScript type from its variable name.

The compiler documentation recognizes qualified searches such as
`ASL bool`, `ASL int`, and `ASL bool byte int` and opens this mapping directly.

## Translating statement-heavy expressions

ASL and C# helpers often need several statements to choose one value. In an
expression position, SplitScript braces form a value block: local statements
run first and the final expression supplies the value. This works in [`if`]
branches, [`match`] arms, fallback [`else`] expressions, arguments, and state
initializers:

```splitscript
# state "game.exe" {}
fn category(isBoss: bool) -> String {
    let label = if isBoss {
        let kind = "Boss"
        `{kind} level`
    } else {
        "Level"
    }
    return label
}
# setup { print(category(true)) }
```

The final expression is local to the nested block and has no [`return`] keyword.
Functions and lifecycle actions remain statement bodies and require explicit
[`return`]. A value block with no final expression yields [`None`]; a block that
always returns, throws, breaks, or continues has type [`Never`]. A trailing
semicolon after the final expression is accepted, but the compiler warns and
the formatter removes it because it is still the block's value.

## Infinite loops and value-carrying breaks

Use [`loop`] for unconditional repetition. A loop without a reachable
[`break`] has type [`Never`]. Within a [`loop`] expression, `break value`
supplies its result and all break values are inferred together. A bare break
supplies [`None`].

```splitscript
# state "game.exe" {}
fn chooseImage(vulkan: bool) -> String {
    return loop {
        if vulkan {
            break "EngineWin64sv.dll"
        }
        break "EngineWin64s.dll"
    }
}
# setup { print(chooseImage(false)) }
```

Legacy ASL and C# `while (true)` and JavaScript `while (true)` normally become
[`loop`] when they are intentionally unconditional. Keep [`while`] when the
condition carries real policy. Unlike Rust, SplitScript functions do not
implicitly return a final loop expression: write `return loop { ... }`.
Value-carrying [`break`] is limited to [`loop`]; a nested [`while`] or runtime
[`for`] always captures its own bare break.

## ASL numeric roots are module-relative

A numeric root in a legacy ASL native state field or `DeepPointer`
is normally an offset from the selected process's main module. Preserve that
base explicitly in SplitScript. For example, this ASL watcher:

```asl
state("game") {
    int health: 0x1234, 0x10;
}
```

becomes:

```splitscript
state "game.exe" {
    health: i32 at "game.exe", 0x1234, 0x10;
}
```

The string root tells [`at`](syntax@at) to resolve `0x1234` relative to that
module. Writing `at 0x1234, 0x10` instead gives SplitScript the absolute virtual
address `0x1234`. Both forms compile because both meanings are useful, but they
do not read the same location. Do not copy an ASL numeric root into the absolute
form merely because the literals look identical. If the ASL explicitly builds
an absolute address at runtime, preserve that absolute meaning instead.

## Signed pointer offsets

ASL `DeepPointer` paths commonly contain negative offsets. Preserve them
directly; do not cast them through [`u64`]:

```splitscript
state "game.exe" {
    health: i32 at "game.dll", 0x120, -0x18;
}
```

The absolute root in `at 0xffff_ffff_ffff_fff0` remains an unsigned 64-bit
address. Module-relative roots and every offset after a dereference are signed
[`i64`], and arithmetic wraps modulo the address space. The equivalent dynamic
APIs are `process.follow(base, offsets: [i64])`,
`address.offset(displacement)`, which accepts any integer width, and signed
[`MemoryPath`] offsets.

## Composing dynamic pointer paths

When the dynamic part is itself watched state, use the sibling field directly
as the [`at`](syntax@at) base. Field order does not matter:

```splitscript
state "game.exe" {
    loading: bool at loadingAddress;
    loadingAddress: address = process.read(0x1000)?;
}
```

The compiler evaluates `loadingAddress` first. If that read fails, `loading` is
not evaluated with a stale address; both fields retain their last accepted
values. Cyclic sibling dependencies are rejected with labels on every field in
the cycle.

ASL scripts often copy a selected `DeepPointer` offset array and append one
runtime-specific final offset. SplitScript's growable [`[T]`] arrays support
that operation directly; do not branch over every possible path length. Create
a fresh array, [`extend`] it with the selected path, [`push`] the final offset,
then pass the complete path to [`Process.follow`]:

```splitscript
fn readDynamic(base: address, path: [i64], finalOffset: i64) -> f64! {
    let fullPath: [i64] = []
    fullPath.extend(path)
    fullPath.push(finalOffset)
    let target = process.follow(base, fullPath)?
    return process.read<f64>(target)
}

state "game.exe" {
    value: f64 = readDynamic(0x1000, [0x10, 0x20], 0x30);
}
```

[`extend`] copies the elements into the new array, so the selected source path
is not mutated. [`push`] adds one element and keeps the resulting value an
ordinary `[i64]`; [`Process.follow`] therefore handles every source length with
the same function.

An ASL background task that repeatedly retries the same `DeepPointer` usually
does not need a task or dynamically replaced watcher in SplitScript. Retain a
[`MemoryPath`] after module discovery and resolve it from an expression-backed
state field on every poll:

```splitscript
# let loadingPath: MemoryPath? = None
fn readLoading(path: MemoryPath?) -> bool {
    let selectedPath = path else return false
    let address = selectedPath.resolve() else return false
    return process.read<bool>(address) else false
}

# state "game.exe" {
#     loading: bool = readLoading(loadingPath);
# }
# onAttach {
#     let module = await process.module("game.dll")
#     loadingPath = module.address.memoryPath(
#         [0x20, 0x18],
#         0x4,
#     )
# }
```

[`MemoryPath.resolve`] follows the retained chain afresh, so an object that is
created or replaced later becomes visible without guest-managed cancellation.
The explicit `else false` matches ASL `ReadFailAction.SetZeroOrNull`. Letting a
fallible read escape the field instead keeps its last accepted value, matching
the default persistent-watcher behavior. Native pointer reads use the attached
executable's detected PE, ELF, or Mach-O pointer width rather than the host's
width.

## Bounded native `stringN` state

The original ASL runtime implements `stringN` by:

1. resolving the complete `DeepPointer` path;
2. reading exactly `N` bytes;
3. choosing UTF-16LE when the second byte is zero, otherwise UTF-8;
4. decoding with .NET replacement behavior; and
5. truncating at the first decoded NUL character.

SplitScript does not preserve that heuristic. Choose the encoding from the
game's memory layout. For identifiers known to contain valid ASCII or UTF-8,
use a bounded UTF-8 decoder on the state field:

```splitscript
state "game.exe" {
    map at "game.exe", 0x123456, 0x20, 0x18 as utf8(50);
}
```

The number is a maximum byte count, not part of the field's type. The result is
an ordinary immutable [`String`]. SplitScript resolves every pointer offset first,
performs one bounded final read, stops at the first NUL byte, and rejects that
field's candidate when UTF-8 is invalid. It deliberately does not expose
`string50` as a type.

For a known native UTF-16LE buffer, use a code-unit bound instead. An ASL
`string32` field reads 32 bytes, so its equivalent bound is 16 UTF-16 code
units:

```splitscript
state "game.exe" {
    chapter at 0x123456 as utf16le(16);
}
```

The dynamic equivalent is `process.readUtf16Le(address, maxUtf16Units)`. Both
forms stop at the first NUL code unit and replace malformed surrogate
sequences. An odd ASL byte bound has no exact UTF-16LE rewrite because its final
byte is an incomplete code unit.

When the parser encounters a type-first field such as
`string50 map : 0x100`, it explains this distinction and offers separate
**maybe-incorrect** UTF-8 and UTF-16LE rewrites. Neither edit is preferred or
machine-applicable because only the autosplitter author can verify the target
encoding. The UTF-16LE action is available only for an even ASL byte bound.

The stricter UTF-8 malformed-input policy is equivalent for A Plague Tale's
ASCII map identifiers. A Unity managed string is not a native buffer: declare
it in the managed [`class`] schema with [`maxLength`](syntax@maxLength) instead
of applying a native decoder to its object address.

The maintained Arietta of Spirits port uses independent `utf8(128)` and
`utf8(8)` fields for its stage and pause-menu identifiers. Its host fixture
also proves the persistent-watcher rule: a failed stage-string read retains
that field while a successful pause flag from the same poll still advances.

## Contiguous structs and fixed arrays

Several separate ASL watchers may describe one physically contiguous native
value. When the target layout is known, a SplitScript struct reads the complete
value once and gives each component a name. Fields use declaration order with
natural alignment:

```splitscript
struct LevelTimeParts {
    minutes: f32,
    seconds: f32,
    hundredths: f32,
}

state "game.exe" {
    levelTime: LevelTimeParts at "game.exe", 0x1200;
}
```

This struct is 12 bytes because all three fields have four-byte size and
alignment. In a mixed struct, each field starts at the next multiple of its
own alignment and the final size is rounded to the largest field alignment.
SplitScript currently reads these values as little-endian. Do not use a struct
for a packed, explicitly padded, or differently endian target layout; exact
layout controls remain intentionally deferred until maintained-port evidence
requires them.

For a contiguous homogeneous region, [`[T; N]`] carries the physical element
count in the type and also performs one host read:

```splitscript
state "game.exe" {
    inventory: [u8; 6] at "game.exe", 0x2b32;
}

split {
    return old.inventory[0] != current.inventory[0]
}
```

Use [`[T; N]`], not growable [`[T]`], for process-memory layout. The fixed array
still supports indexing and iteration, while its exact length prevents a port
from silently reading a different number of bytes. A struct or fixed array is
[`MemoryReadable`] only when every contained field or element has a fixed
readable layout. One fixed-array process read is limited to 4,096 elements and
65,536 bytes so generated code and host-memory traffic remain bounded.

Do not model a much larger native region as one fixed array when the script only
needs a few selected values. An expression-valued state field can construct a
growable [`[T]`] transactionally from focused reads:

```splitscript
state "game.exe" {
    sampledFlags: [u8] = {
        let flags: [u8] = []
        flags.push(process.read<u8>(0x1000)?)
        flags.push(process.read<u8>(0x5000)?)
        flags.push(process.read<u8>(0x9000)?)
        flags
    };
}

whileAttached {
    print(current.sampledFlags)
}
```

Every focused read must succeed before the new array is published, preserving
the state transaction without reading the unused bytes between samples.

## C# string operations

SplitScript methods use lower camel case, so C# `StartsWith` becomes
[`startsWith`]. For ASCII game identifiers, C# `ToLower()` becomes the more
explicit [`toAsciiLowerCase()`], while `ToUpper()` becomes
[`toAsciiUpperCase()`]:

```splitscript
# state "game.exe" {
#     map at 0x1000 as utf8(64);
#     mission at 0x1100 as utf8(64);
# }
# whileAttached {
let normalizedMap = current.map.toAsciiLowerCase()
let normalizedMission = current.mission.toAsciiUpperCase()
# print(normalizedMap)
# print(normalizedMission)
# }
```

These conversions change only `A` through `Z` or `a` through `z` and preserve
all other UTF-8 bytes. They are not culture-sensitive or full Unicode case
conversion. [`slice`] uses
UTF-8 byte offsets rather than .NET UTF-16 indices and fails when an offset is
out of range or inside a multibyte code point, so do not mechanically translate
`Substring` without checking the target data.

For text proven to be ASCII, translate the overload shapes explicitly. C#
`value.Substring(start, length)` becomes
`value.slice(start, start + length)`, while `value.Substring(start)` becomes
`value.slice(start, value.byteLength())`. Both SplitScript calls are fallible.
For non-ASCII text, first derive UTF-8 byte boundaries rather than copying the
original UTF-16 positions.

C# `value.Trim()` recognizes Unicode whitespace. For game identifiers, log
lines, and configuration text known to use ASCII whitespace, use the deliberately
explicit operation:

```splitscript
# state "game.exe" {
#     logLine at 0x1000 as utf8(64);
# }
# whileAttached {
let eventName = current.logLine.trimAsciiWhitespace()
# print(eventName)
# }
```

It removes space, tab, line feed, vertical tab, form feed, and carriage return
from both ends, preserves interior and non-ASCII bytes, and reuses the original
immutable string when nothing changes. The compiler does not rewrite `Trim()`
automatically because Unicode whitespace, character-array overloads,
`TrimStart`, and `TrimEnd` have different semantics.

C# `PadLeft` and `PadRight` map to directionally named immutable operations:

```splitscript
# state "game.exe" {}
# onAttach {
# let chapterNumber: u32 = 7
let chapter = chapterNumber as String
let chapterKey = chapter.padStart(2, '0')
let column = chapterKey.padEnd(8, ' ')
# print(column)
# }
```

SplitScript always requires the fill [`char`]; pass `' '` explicitly for C#
overloads that omit it. Width counts Unicode scalar values rather than .NET
UTF-16 code units or terminal display columns, so copied widths are directly
equivalent only for text proven to be ASCII. An already-wide string is reused;
otherwise the result is allocated once at its exact UTF-8 byte length.

C# combines nullability and emptiness in `String.IsNullOrEmpty(value)`.
SplitScript keeps those concerns in the type. A required [`String`] cannot be
null, so use its source-defined method directly:

```splitscript
# state "game.exe" {
#     checkpoint at 0x1000 as utf8(64);
# }
# whileAttached {
let missingCheckpoint = current.checkpoint.isEmpty()
# print(missingCheckpoint)
# }
```

When the migrated value is deliberately optional, handle both variants:

```splitscript
# state "game.exe" {
#     checkpoint: String? at 0x1000 as utf8(64);
# }
# whileAttached {
let missingCheckpoint = match current.checkpoint {
    None => true,
    Some(text) => text.isEmpty(),
}
# print(missingCheckpoint)
# }
```

A failed process read is not automatically an empty or null string. Decide
first whether that boundary should remain fallible ([`T!`]), become a `String?`, or
retain the last accepted state value. The compiler therefore gives
`String.IsNullOrEmpty` focused guidance without guessing an automatic rewrite.

C# `String.IsNullOrWhiteSpace(value)` has the same optionality boundary. For a
required [`String`], use `value.isBlank()`. It returns true for the empty string
and for text made entirely from Unicode `White_Space` characters, including
non-breaking space. For `String?`, keep absence explicit:

```splitscript
# state "game.exe" {}
# fn missingLabel(value: String?) -> bool {
return match value {
    None => true,
    Some(text) => text.isBlank(),
}
# }
```

Unlike [`trimAsciiWhitespace`], [`isBlank`] is deliberately Unicode-aware and
does not allocate a normalized string.

C# `String.Length` counts UTF-16 code units, so it has no encoding-neutral
rename for SplitScript's immutable UTF-8 strings. Prefer the operation that
expresses the surrounding intent. An emptiness check does not need a numeric
unit:

```splitscript
# state "game.exe" {
#     map at 0x1000 as utf8(64);
# }
# whileAttached {
if current.map.isEmpty() {
    return false
}
# }
```

Use [`byteLength()`] only for text proven to be ASCII or code intentionally
working with the UTF-8 byte offsets returned by `indexOf`, [`lastIndexOf`], and
accepted by [`slice`], [`byteAt`], and [`charAt`]. Non-ASCII text can have different
UTF-16 code-unit and UTF-8 byte counts, so the compiler diagnoses `.Length`
without applying a speculative fix.

C# `String.Join` puts the separator first and has many object, enumerable,
variadic, and range overloads. SplitScript accepts one typed string array and
puts the values first:

```splitscript
# state "game.exe" {}
# onAttach {
# let routeParts = ["chapter", "level"]
let routeName = String.join(routeParts, ".")
# print(routeName)
# }
```

The implementation measures the complete UTF-8 result and allocates it once.
Empty and single-element arrays add no separator; empty elements are preserved.
Convert non-string values explicitly before joining. The compiler does not
swap arguments automatically because a C# call does not prove that its chosen
overload already contains a `[String]`.

C# `value.IndexOf(substring)` returns a UTF-16 code-unit index or `-1`.
SplitScript `value.indexOf(substring)` instead returns a UTF-8 byte offset as
`u32?`; handle [`None`] directly. The numeric offsets are equivalent only for
text proven to be ASCII:

```splitscript
# state "game.exe" {
#     map at 0x1000 as utf8(64);
# }
# split {
let separator = current.map.indexOf("_") else return false
# return separator > 0
# }
```

Comparison-mode and start-index overloads need an explicit rewrite rather than
a method-name substitution.

C# `value.LastIndexOf(substring)` has the same index-unit and absence hazards.
For exact searches over proven ASCII text, use `value.lastIndexOf(substring)`
and handle its `u32?` result directly:

```splitscript
# state "game.exe" {
#     path at 0x1000 as utf8(64);
# }
# split {
let separator = current.path.lastIndexOf("/") else return false
# return separator > 0
# }
```

The operation searches the complete string and returns a UTF-8 byte offset.
An empty substring is found at the final byte boundary. C# comparison-mode,
start-index, and count overloads need a deliberate rewrite.

C# `value.Replace(search, replacement)` maps to immutable
`value.replaceAll(search, replacement)` when `search` is non-empty and
`replacement` is not null. The SplitScript operation is fallible, so retain
that policy explicitly rather than discarding the result:

```splitscript
# state "game.exe" {
#     map at 0x1000 as utf8(64);
# }
# whileAttached {
let displayName = current.map.replaceAll("_", " ") else current.map
# print(displayName)
# }
```

C# permits a null replacement to mean deletion; pass `""` explicitly in
SplitScript only when that was the source's intent. An empty search is an error,
as is a result whose byte length cannot be represented. The compiler therefore
explains `.Replace(...)` but does not offer a blind rename that would leave
[`T!`] handling unresolved.

C# `left.Equals(right)` normally becomes `left == right`; SplitScript compares
strings by exact UTF-8 text rather than GC reference identity. If the source
intentionally ignores ASCII letter case, write
`left.equalsIgnoreAsciiCase(right)` instead. The compiler does not rewrite
`Equals` automatically because C#'s static and comparison-mode overloads need
semantic review.

Use `charAt(byteIndex)` for textual character checks. It returns a [`char`] and
still takes a UTF-8 byte offset; an offset into the middle of a multibyte scalar
is an error. Use `byteAt(byteIndex)` only when the ASL is genuinely inspecting
encoded bytes. Neither operation adopts C#'s or JavaScript's UTF-16 indexing:

```splitscript
# state "game.exe" {
#     map at 0x1000 as utf8(64);
# }
# split {
let slash = current.map.charAt(7) else return false
return slash == '/'
# }
```

C# `Split` maps to fallible, lower-camel-case [`String.split`]. SplitScript matches one
exact non-empty delimiter from left to right and preserves leading, adjacent,
and trailing empty segments:

```splitscript
# state "game.exe" {
#     levelPicture at 0x1000 as utf8(64);
# }
# whileAttached {
let fileParts = current.levelPicture.split(".") else []
let levelParts = fileParts[0].split("_") else []
# print(levelParts.length())
# }
```

The empty delimiter is an error rather than a request to split UTF-8 bytes.
For a name such as `01_02_1.dds`, the two calls above produce `01`, `02`, and
`1` without discarding meaningful empty fields in other inputs.

C# `Int32.Parse`, `Double.Parse`, and their fixed-width relatives map to
`text.parse()`. Put the SplitScript numeric type at the receiving boundary and
let inference flow backward:

```splitscript
# state "game.exe" {
#     percentageText at 0x1000 as utf8(16);
# }
# whileAttached {
let percentage: f64 = current.percentageText.parse() else 0.0
# }
```

The compiler recognizes the C# static `Parse` and `TryParse` families and
points to this pattern. It intentionally does not rewrite them: `TryParse`
output parameters become ordinary [`T!`] control flow, and the receiving
declaration or fallback determines the target type.

Unlike an exception-catching `Parse` call, failure is an ordinary [`T!`] value.
Use [`else`] for a fallback, [`?`] to propagate the error from a function or state
field, or [`match`] when failure needs its own behavior. C# `TryParse` therefore
does not need an output parameter. Parsing consumes the complete ASCII decimal
string and rejects whitespace, separators, and trailing text. Float targets
accept case-insensitive `NaN`, `inf`, and `Infinity`; decimal overflow produces
infinity and underflow produces zero, while integer overflow remains an error.
Float conversion is correctly rounded directly to [`f32`] or [`f64`] and does not
inherit C# culture settings.

## C# `Convert` operations

C# `Convert.To*` calls do not all map to one SplitScript cast. Choose the
operation from the source value and the behavior the script needs:

```splitscript
# state "game.exe" {}
# onAttach {
# let byteValue: u8 = 7
# let floatValue: f64 = 3.5
# let numericFlag: i32 = 1
# let text = "42.5"
let widened: i32 = byteValue as i32
let rounded: i32 = floatValue.round() as i32
let enabled = numericFlag != 0
let parsed: f64 = text.parse() else 0.0
# }
```

Fixed-width integer [`as`] casts use SplitScript's numeric cast rules. In
particular, narrowing integers retain their low bits, while C# `Convert` throws
when a narrowing conversion is out of range. A floating-point [`as`] cast
truncates toward zero, saturates at an integer boundary, and maps NaN to zero.
For a finite, in-range value, `value.round() as i32` preserves the
midpoint-to-even rounding of `Convert.ToInt32`; it still does not reproduce C#
overflow and NaN exceptions.

For strings, infer the intended fixed-width number from the receiving boundary
and use fallible [`parse()`]. SplitScript parsing is strict, locale-independent
ASCII decimal parsing, unlike C# conversions that may accept surrounding
whitespace and current-culture formatting. A numeric `Convert.ToBoolean(value)`
becomes `value != 0`. For text, trim and compare `true` and `false` explicitly
with [`equalsIgnoreAsciiCase`], choosing a [`T!`] value or fallback for malformed text.

The ordinary one-value `Convert.ToString(value)` maps to [`Display`]:

```splitscript
# state "game.exe" {}
# onAttach {
# let value: i32 = 7
let text = value as String
print(value)
setVariable("Value", value)
# print(text)
# }
```

Interpolation, [`print`], and [`setVariable`] already accept [`Display`] values, so
they do not need an intermediate string cast.

A port-defined struct or enum is a [`Display`] value automatically and receives
a stable multiline structural representation. Define
`fn Type.toString() -> String` only when the timer-facing text should use a
custom format; the result may be inferred. No `impl` block or annotation is
necessary.

```splitscript
# state "game.exe" {}
struct Position {
    x: i32,
    y: i32,
}
fn Position.toString() { return `({self.x}, {self.y})` }
# onAttach {
setVariable("Position", Position { x: 3, y: 5 })
# }
```

The integer-radix overload maps to the fallible [`Integer.toString`] method:

```splitscript
# state "game.exe" {}
# onAttach {
# let cellId: u32 = 0x2a
let hexadecimal = cellId.toString(16) else ""
let uppercase = hexadecimal.toAsciiUpperCase()
# print(uppercase)
# }
```

Radices from 2 through 36 use `0` through `9` and lowercase `a` through `z`.
Negative values retain a leading minus sign, including signed minima. This
differs from C#'s two's-complement rendering of negative values in base 2, 8,
or 16, so review any negative-source call rather than translating it blindly.
An out-of-range radix returns an error. Culture/provider, null, and object
overloads remain separate policies and are not ordinary [`Display`] conversions.

## Version-labelled ASL states

The second argument in `state("game.exe", "Steam")` is a build label, not
another executable candidate. Represent that persistent fact as an ordinary
enum global initialized during [`onAttach`], then use the same value to select
conditional state fields and refine their use:

```splitscript
enum Build {
    Steam,
    Epic,
}

let build: Build

state "game.exe" {
    if build == Build.Steam {
        loading: bool at "engine.dll", 0x1000;
        checkpoint: u8 at "engine.dll", 0x1100;
    } else {
        loading: bool at "engine.dll", 0x2000;
        checkpoint: u16 at "engine.dll", 0x2100;
    }
}

onAttach {
    let executable = await process.mainModule()
    if executable.size == 10_000 {
        build = Build.Steam
    } else if executable.size == 20_000 {
        build = Build.Epic
    } else {
        await process.closed()
    }
}

split {
    return match build {
        Build.Steam => old.checkpoint != current.checkpoint,
        Build.Epic => old.checkpoint != current.checkpoint,
    }
}
```

Detection is ordinary typed code and may use module size, [`FileVersion`],
signatures, process identity, or discovered memory. `await process.closed()`
keeps an unsupported attachment inert without detaching and immediately
reattaching to the same process.

Compatible fields declared by every conditional branch expose one common
interface. When a field is missing or the same name has a conflicting type,
test or match the enum global and access the field only where that condition is
proven. The compiler gives each incompatible declaration its real physical
type rather than turning it into an option or inventing a default. An honest
typed default is still appropriate when consumers already define that value as
unavailable; the A Plague Tale Xbox build uses `cutsceneState: i32 = 0` for
exactly that reason.

When the original script has several independent build facts, avoid turning
their cartesian product into many version-labelled states. Declare enum-valued
globals independently. Assign each one during [`onAttach`]. Any of them can
guard native state fields and managed class fields with the same predicate:

```splitscript
enum Edition {
    Full,
    Demo,
}

enum Storefront {
    Steam,
    GOG,
}

let edition: Edition
let storefront: Storefront

state Unity ["game.exe"] {
    if edition == Edition.Full {
        level: u32 at 0x1000;
    }
}

onAttach {
    edition = Edition.Full
    storefront = Storefront.Steam
}
```

One enum can describe a complete build when those facts always move together.
Use separate globals when edition, storefront, renderer, or another fact can
vary independently or is shared with managed metadata. These are ordinary
values rather than a separate layout subsystem, so the same control-flow and
exhaustiveness rules apply everywhere.

## Attached process identity

ASL exposes the selected process through `game.ProcessName`. In a native
SplitScript state, use `process.name()`:

```splitscript
enum Build {
    FullGame,
    Demo,
}

let build: Build

state ["game.exe", "game-demo.exe"] {}

onAttach {
    build = match process.name() {
        "game.exe" => Build.FullGame,
        "game-demo.exe" => Build.Demo,
        _ => await process.closed(),
    }
}
```

The returned string is the exact candidate from the [`state`] declaration that
matched during attachment. It is not the executable path and does not perform
another host lookup. String literals are first-class [`match`] patterns and
compare text contents, so this is the direct selector when executable names map
to build shapes. Keep the wildcard arm because strings are an open-ended
domain. When several builds share a name, discriminate with reliable evidence
such as `process.mainModule().size`, `process.path()`,
[`Module.fileVersion()`], [`Module.productVersion()`], or a signature instead.
When the source distinguishes byte-exact builds by hashing the executable, use
[`Module.md5()`] rather than substituting one of those weaker identities.

Legacy `modules.Any(...)` checks often test for one optional module rather than
requiring full enumeration. Use the synchronous optional probe in that case:

```splitscript
# state "game.exe" {}
# onAttach {
let steam = match process.loadedModule("steam_api.dll") {
    Some(_) => true,
    None => false,
}
# print(steam)
# }
```

Use `await process.module("GameAssembly.dll")` when attachment must wait until
a required module loads. Do not use that waiting form for optional platform or
mod-loader detection: an absent module would keep [`onAttach`] pending forever.

The common ASL expression `modules.First()` normally refers to the executable
itself. Discover that value once in [`onAttach`], then use its typed fields:

```splitscript
# state "game.exe" {}
onAttach {
    let executable = await process.mainModule()
    print(`executable size: {executable.size}`)
}
```

This replaces `ModuleMemorySize` with `size` and `BaseAddress` with [`address`].
It also makes the suspension visible: module discovery happens before polling,
not implicitly whenever a property is read.

The two version methods return typed four-part [`FileVersion`] values rather than
the punctuation-dependent strings exposed by C# `FileVersionInfo`. Use
[`Module.versionInfo()`] when both identities are needed, so the PE resource is
only traversed once.

```splitscript
# state "game.exe" {}
onAttach {
    let executable = await process.mainModule()
    let product = executable.productVersion() else return
    let label = match product {
        v"1.2.3.4" => "recognized build",
        _ => "unsupported build",
    }
    print(label)
}
```

[`FileVersion`] literals are also first-class [`match`] patterns. Always add a
wildcard arm because executable versions are an open set.

Preserve an ASL `MD5.Create().ComputeHash(File.OpenRead(module.FileName))`
probe with the module's cooperative fingerprint method. It hashes the complete
on-disk file, not the mapped image, and produces the same uppercase 32-character
spelling commonly created with `BitConverter.ToString(...).Replace("-", "")`:

```splitscript
enum Build {
    Original,
    Updated,
}

let build: Build

state "game.exe" {
    if build == Build.Original {
        value: u32 at 0x1000;
    } else {
        value: u32 at 0x2000;
    }
}

onAttach {
    let executable = await process.mainModule()
    let fingerprint = (await executable.md5())?
    build = match fingerprint {
        "951389C953020FC7B5DEF32E7BED129A" => Build.Original,
        "E1E439BD3FE89DE97BE08B15505837E2" => Build.Updated,
        _ => await process.closed(),
    }
}
```

Hashing yields between bounded file windows and restarts if the file changes,
so it belongs in [`onAttach`] rather than a per-tick decision block. Filesystem
or metadata failures flow through `String!`; postfix [`?`] is appropriate when a
failed identity probe should reject the current attachment path.

Only preserve full enumeration when the source genuinely needs to inspect
unknown module names. SplitScript does not currently expose that host operation.
Do not replace it with `process.memoryRanges()`: mapped memory ranges and loaded
modules have different identities and lifetime semantics. Record such a port as
host-limited instead of silently changing its behavior.

The compiler recognizes the exact legacy path `game.ProcessName` and offers a
machine-applicable `process.name()` rewrite where the native `process` value is
in scope. Ordinary functions do not implicitly capture an attachment; pass the
name as a parameter when helper logic needs it.

## Timer state

ASL's `timer.CurrentPhase` maps directly to [`timer.state()`]. Compare the
resulting exhaustive enum by name:

```splitscript
# state "game.exe" {
#     inMenu: bool at 0x1000;
# }
reset {
    return timer.state() == TimerState.NotRunning && current.inMenu
}
```

The familiar variants retain their meanings as [`TimerState.NotRunning`],
`Running`, `Paused`, and `Ended`. SplitScript additionally exposes
[`TimerState.Unknown`]; the runtime maps an unrecognized future host value there
instead of fabricating a known state. Use an explicit wildcard or `Unknown`
arm when matching according to the behavior the script needs.

Do not preserve integer comparisons such as `timer.CurrentPhase > 0`.
[`TimerState`] is an enum, not an ordered number. Compare or match the named
states that made the original condition true. The compiler offers
machine-applicable rewrites for `timer.CurrentPhase` and the four legacy
`TimerPhase` variants. A legacy enum name in a type or match pattern receives
focused guidance without reserving `TimerPhase` as a source identifier.

## Timer split index

ASL exposes `timer.CurrentSplitIndex` as a signed integer. SplitScript uses
[`timer.currentSplitIndex()`] and makes the no-attempt state explicit as [`None`]:

```splitscript
# state "game.exe" {
#     level: u32 at 0x1000;
# }
split {
    let index = timer.currentSplitIndex() else return false
    return match index {
        0 => current.level == 2,
        1 => current.level == 7,
        _ => false,
    }
}
```

The result type is `u64?`. Every negative value from the host ABI maps to
[`None`]; nonnegative values map to the corresponding [`u64`]. Do not cast the
signed sentinel into an unsigned index. A skipped segment advances the index,
and after the final split the index equals the route's segment count. The index
is therefore authoritative route progress, not a count of splits requested by
this autosplitter.

The compiler recognizes `timer.CurrentSplitIndex`, but deliberately does not
rewrite it automatically because the correct [`None`] behavior depends on the
surrounding control flow. Use [`else`] for an early fallback or [`match`] when the
absent state needs distinct behavior.

## Load removal and computed game time

Use [`isLoading`] when the game exposes whether its own clock should be paused.
Return `true` while loading and `false` while gameplay is advancing. When a
sentinel means the script has no trustworthy observation for this tick, fall
through or return [`None`] so the timer keeps its current pause state:

```splitscript
state "game.exe" {
    loadingState: i32 at 0x1000;
}

isLoading {
    if current.loadingState < 0 {
        return None
    }
    return current.loadingState != 0
}
```

This third state is intentionally different from `false`: `false` actively
resumes game time, while [`None`] leaves the previous host state unchanged. A
bare [`return`] and ordinary fallthrough also produce [`None`]. Prefer this action
for regular load removal rather than repeatedly calling
[`timer.pauseGameTime()`] and [`timer.resumeGameTime()`].

When the game exposes its own elapsed time, return a typed [`Duration`] from
[`gameTime`]. Direct values automatically become the present side of the
optional action result; no `Some(...)` constructor is needed:

```splitscript
state "game.exe" {
    elapsedFrames: i64 at 0x1008;
}

gameTime {
    if current.elapsedFrames < 0 {
        return None
    }
    return Duration.fromFrames(current.elapsedFrames, 60)
}
```

Use the constructor that matches the game's representation, such as
[`Duration.fromSeconds`], [`fromMilliseconds`], [`fromFrames`], or `fromParts`.
Falling through from [`gameTime`] leaves the host's last value unchanged; it
does not set zero. [`isLoading`] runs before [`gameTime`] on each eligible update,
so the two actions may be combined when the game provides both an independent
loading flag and an authoritative elapsed clock.

These actions report script-owned observations to the timer host. They do not read
back `timer.CurrentTime.GameTime`, which may have been changed by the host or
another component and remains a separate host-contract requirement below.

## LiveSplit timer metadata and control

Several legacy paths inspect host-owned timer data rather than the attached
game. They do not currently have faithful SplitScript replacements:

- `timer.CurrentTime.GameTime` reads LiveSplit's optional game-time clock. If
  this autosplitter computes the value, keep that [`Duration`] in script-owned
  state and also return it from [`gameTime`]; the host's possibly externally
  changed value cannot yet be read back.
- `timer.CurrentSplit.Name`, indexed `timer.Run[...]` segments,
  `timer.Run.Count`, `CategoryName`, `GameName`, and `FilePath` require a typed
  read-only run snapshot. Do not silently copy route metadata into the script
  unless that fixed route is intentionally owned by the autosplitter.
- `timer.Run.Offset` and timing-method access are user-visible configuration.
  Reads and writes need defined ordering, persistence, reset/undo behavior,
  precision, and conflict handling with LiveSplit's UI.

The current Wasm host exposes none of those observations or mutations. The
compiler emits focused diagnostics for each category instead of a generic
unknown-name error or a misleading automatic rewrite. Mark a port as
behavior-limited when it genuinely depends on one of them. The planned host
contract will expose optional values from one coherent snapshot per update and
will keep read-only metadata separate from controlled mutation authority.

## Monotonic delays and debouncing

ASL scripts often use `DateTime.Now`, `DateTime.Now.TimeOfDay`, or a
`Stopwatch` only to measure time since a game event. Use [`Instant`] for that
process-independent elapsed time:

```splitscript
# state "game.exe" {
#     inMenu: bool at 0x1000;
# }
let enteredMenuAt: Instant? = None

whileAttached {
    if current.inMenu && !old.inMenu {
        enteredMenuAt = Instant.now()
    } else if !current.inMenu {
        enteredMenuAt = None
    }
}

reset {
    let detectedAt = enteredMenuAt else return false
    return detectedAt.hasElapsed(Duration.fromMilliseconds(500))
}
```

[`Instant.now()`] reads a monotonic host clock. It never moves backwards during
one autosplitter instance and has no meaningful absolute or calendar value.
[`elapsed()`], `durationSince(...)`, and `hasElapsed(...)` produce or compare
exact [`Duration`] values, making them appropriate for cooldowns, debouncing,
delayed splits, and retry deadlines. They continue advancing independently of
LiveSplit's loading and pause state.

Do not mechanically replace `timer.CurrentTime.RealTime`. That ASL expression
may mean the current LiveSplit attempt's run-relative time rather than an
independent delay. If the original logic starts its measurement at a game event,
capture an [`Instant`] there. If it needs the actual timer phase for an offset,
run-age check, or custom game-time calculation, the current host contract does
not expose equivalent metadata. The compiler diagnoses these paths separately
so this distinction is not hidden by a convenient but incorrect rewrite.

## Retrying an attach-time read transaction

ASL ports sometimes use a hand-written `while (true)` loop to repeat several
dependent reads until a complete pointer chain or metadata group becomes
available. In SplitScript, use one [`retry`] expression instead. Braces are an
ordinary value-producing expression, so the block naturally creates one local
failure boundary for every [`?`] inside it:

```splitscript
# state "game.exe" {}
onAttach {
    let healthAddress = retry {
        let manager = process.read<address>(0x1000)?
        let player = process.read<address>(manager.add(0x20))?
        player.add(0x18)
    }
    print(`health address {healthAddress}`)
}
```

A failed read starts the complete block again on the next attached update; no
locals from the failed attempt survive. A final [`T!`] error or [`throw`] has
the same retry behavior. [`return`] still exits the surrounding function, and
[`break`] or [`continue`] still target their lexical loop. Keep one attempt
synchronous and bounded: [`await`] and nested [`retry`] are rejected inside the
operand. Await scans, module discovery, and other intrinsically asynchronous
operations before entering the retry block.

## Attach-time-discovered addresses

Keep discovery in [`onAttach`] and polling in the state declaration. Declare an
attachment-discovered value with a bare top-level [`let`], without a fake zero
or [`None`] initializer. Its type is inferred from assignments and uses. The
compiler proves that every successful attachment path initializes it before
polling begins, and the runtime clears its storage when that process detaches.
An unsupported build should await process closure instead of completing:

```splitscript
let loadingAddress

state "game.exe" {
    loading: i32 = process.read(loadingAddress);
}

onAttach {
    let executable = await process.mainModule()
    if executable.size == 47_570_944 {
        loadingAddress = executable.address + 0x029020f4
    } else {
        print(`unsupported module size {executable.size}`)
        await process.closed()
    }
}
```

A bare global may belong to only the attachment shapes whose [`onAttach`]
paths initialize it. Access it under the same direct condition or [`match`] on
the shape enum global that refines conditional state fields. Helpers inherit
these requirements, so a helper reading a Steam-only value is callable from
the `Build.Steam` arm but not from unrefined polling code. Values assigned on
every successful attachment path remain available everywhere while attached.

Expression-backed fields are persistent watcher values. The initial snapshot
waits for every required field to succeed together. Afterwards, a failed [`T!`]
retains that field's last accepted value while successful siblings advance. If
several values must advance atomically, read them as one struct- or array-valued
state field. For a field that is semantically absent on some ticks, declare
[`T?`] and convert just that read:

```splitscript
state "game.exe" {
    requiredLevel: u32 at "game.exe", 0x1000;
    optionalBonus: u32? at "game.exe", 0x2000;
}
```

For a static pointer path, the explicit [`T?`] annotation maps a failed module
lookup, pointer traversal, final read, or decoder to a successfully accepted
[`None`]; success produces `Some(T)`. This differs from a required `T` field,
whose failed result retains the last accepted value after initialization. The
maintained Aquanox port uses `String? at ... as utf8(32)` because the original
ASL watcher becoming `null` is itself the manual level-end signal.

The same policy remains available to a discovered-address expression with
`process.read<T>(address).discardError()`. Prefer direct `T? at` syntax when the
path is static so the declaration shows both the memory layout and its absence
semantics in one place.

Pointer width is a property of the attached executable. Static [`at`](syntax@at)
fields and reusable `base.memoryPath(offsets, finalOffset)` values both detect
and use it automatically, including when a 64-bit host reads a PE32 or another
32-bit target. See the maintained ABZÛ and Borderlands examples for full
discovery and PE32 forms.

## Background signature scans

Legacy ASL often starts a C# `Thread` or task for signature discovery so a
large scan does not block the autosplitting runtime's update loop. Do not translate that worker
or its cancellation token. SplitScript scans are already asynchronous: they
inspect only a bounded memory window per tick, yield to the host between
windows, preserve their cursor, and are discarded automatically if the
attached process closes.

Choose the narrowest range justified by the source:

```splitscript
# state "game.exe" {}
onAttach {
    let executable = await process.mainModule()
    let code = await executable.scan(sig"48 8B 05 ?? ?? ?? ??")
    let table = retry process.readRelative32(code.offset(3))

    let heapMarker = await process.scanMemory(
        sig"54 49 4D 52 ?? ?? ?? ??",
    )
    print(`table {table}, marker {heapMarker}`)
}
```

Use [`Module.scan`] when a pattern belongs to one known image and `process.scan`
for another explicit address range. Use `process.scanMemory` only when the
legacy source genuinely enumerates readable mappings or the target may live
outside known modules. [`scanAny`] and [`scanMemoryAny`] accept an array of
signatures and return both the address and selected index, which keeps fallback
build selection in one cooperative pass.

Those operations deliberately keep starting new passes until a signature
appears. When a legacy script uses exhaustion as build or version evidence,
scan the exact bounded range once instead:

```splitscript
# state "game.exe" {}
# onAttach {
let executable = await process.mainModule()
let marker = await process.scanOnce(
    executable.address,
    executable.size,
    sig"55 4E 53 55 50 50 4F 52 54 45 44",
) else {
    print("unsupported executable")
    throw "required marker is absent"
}
# print(marker)
# }
```

[`Process.scanOnce`] still spreads one complete pass across bounded per-tick
windows, but returns [`None`] instead of silently beginning another pass.
`future.timeout(process.scan(...), duration)` answers a different question: it
bounds elapsed time and cannot prove that the range was exhausted.

When Windows-native discovery starts from an exported runtime entry point,
resolve that exact symbol through the module instead of scanning the entire
process:

```splitscript
# state "game.exe" {}
# onAttach {
let mono = await process.module("mono-2.0-bdwgc.dll")
let assemblyForeach = mono.peExport("mono_assembly_foreach") else return
# print(assemblyForeach)
# }
```

[`peExport`] validates PE table bounds and rejects forwarded exports. It is
deliberately PE-specific; ELF and Mach-O symbols need their own proven parser or
a future portable host contract rather than being mislabeled as PE exports.

When an ASL helper enables `LoadSceneManager` and reads `Scenes.Active` or
`Scenes.Loaded`, select the [`Unity`] state provider and read immutable scene
snapshots through its attachment-scoped `unity` context:

```splitscript
state Unity ["game.exe"] {
    activeScene = unity.scenes.active();
    loadedScenes = unity.scenes.loaded();
    persistentScene = unity.scenes.persistent();
}

isLoading {
    if current.loadedScenes.isEmpty() {
        return None
    }
    let firstLoaded = current.loadedScenes[0]
    return current.activeScene.name != firstLoaded.name
}
```

The provider discovers and caches [`UnityScenes`] once per attachment, but only
when the script references `unity`; a managed-schema-only port does not pay for
native scene discovery. [`UnityScenes.active`](method@UnityScenes.active),
[`UnityScenes.loaded`](method@UnityScenes.loaded), and
[`UnityScenes.persistent`](method@UnityScenes.persistent) support the
UnityPlayer layouts covered by ASR: 32-bit and 64-bit Windows players and
64-bit Linux and macOS players. Each operation copies the native address,
signed build index, asset path, and name. This is intentionally a snapshot
rather than a live scene handle: [`old`] remains stable even after Unity unloads
or reuses the original native object. Failed or incomplete reads reject that
state field update and retain its preceding value; they do not publish a
partially populated loaded-scene array. The signed index preserves Unity's
initialization value of `-1`.

## UnityASL and managed metadata

Do not copy `UnityASL`, `mono.Make<T>`, image lookup, class lookup, static-table
lookup, or metadata-offset bookkeeping into a SplitScript port. There is no
public low-level metadata traversal workflow to choose instead. Declare the
managed shape once with [`image`], [`namespace`], [`class`], [`static`], and
[`from`]. The [`Unity`] state provider discovers the selected Mono or IL2CPP
backend, binds the reachable schema once per attachment, and exposes ordinary
typed, fallible references:

```splitscript
image "Assembly-CSharp" {
    class PlayerStats {
        static PlayerStats current from ["Instance", "_instance"];
        i32 district from "currDistrict";
        bool inRun;
        String sceneName maxLength 64;
    }
}

state Unity ["game.exe"] {
    district: i32 = PlayerStats.current?.district?;
    inRun: bool = PlayerStats.current?.inRun?;
    sceneName: String = PlayerStats.current?.sceneName?;
}
```

The automatic [`Unity`] selector chooses a supported backend from the loaded
modules. Select `Unity.mono(MonoVersion.V2)` or `Unity.il2cpp(Il2CppProfile.unity2021_3_11f1X64())` after the
[`state`] keyword only when the target's exact metadata layout is known and
automatic detection is inappropriate. [`MonoVersion.V3`] selects the Unity
2021.2-and-newer PE64 Mono layout; `V2` selects the preceding modern layout.

A class-typed field is a live managed reference. Every poll rereads the current
static singleton and each following object pointer, so replacing a managed
object does not leave an attachment-time address cached in the script. Scalar
fields use their declared fixed-width type without allocating a new GC object.
The class name `T` is an immutable local snapshot type and `T.Ref` is a live
remote reference. Each live hop returns [`T!`]. Postfix [`?`] propagates a
failure into the surrounding state field, function, or [`retry`] boundary.
`reference.snapshot()` recursively copies all active fields before constructing
`T`, so a failed nested member never publishes a partially populated object.
A child declared `Child` becomes an owned `Child` inside the snapshot; the same
field on a live parent yields `Child.Ref`. Nullable children preserve [`None`].
Remote changes do not mutate an existing snapshot. Recursive class schemas are
supported, but remote object cycles fail. Nested reads share limits of 64 active
objects, 1,024 object visits, and 16,384 active field reads per operation.
Snapshots, arrays, strings, and completed instance searches materialize owned
values; generated support is retained only when reachable.

A managed string leaf is declared as an ordinary [`String`] or optional [`T?`]
field with an explicit [`maxLength`](syntax@maxLength) read policy. The bound is
the maximum UTF-16 code-unit length accepted from the target process; exceeding
it rejects the read instead of truncating text. A required string rejects a
null managed reference, while an optional string maps that reference to
[`None`]. Both still propagate memory failures normally. This keeps object
headers and backend-specific decoding inside the generated Unity reader.

Use [`from`] for exact metadata names or ordered name alternatives. Without it,
the source declaration name is used and instance fields also recognize the
conventional C# automatic-property backing-field spelling. Build-specific
managed shapes use the same attachment-wide enum globals and
[`if`] / [`else if`](keyword@if) / [`else`](keyword@if) chains as state fields.
The compiler rejects a native [`state`] declaration that tries to consume managed
schema references, so managed reads cannot silently bypass the Unity provider.

Field lookup follows the target's runtime inheritance chain. Declare an
inherited singleton on the concrete schema class exactly as it is consumed:

```splitscript
image "Assembly-CSharp" {
    class LevelFlowService {
        static LevelFlowService instance from "_instance";
        i32 state from "_state";
    }
}
# state Unity ["game.exe"] {}
```

If `_instance` is declared by a closed generic base such as
`Service<LevelFlowService>`, the generated binder retains that declaring
runtime class and uses its static storage automatically. Do not declare the
generic metadata definition, expose its static table, or reproduce the raw
field path. Older V1, 32-bit, ELF, and Mach-O Mono targets still need explicit
backend support.

When a port needs the mapping metadata itself, take a typed snapshot rather
than reproducing the host's numeric count/index ABI:

```splitscript
# state "game.exe" {}
# onAttach {
let ranges = process.memoryRanges()
for range in ranges {
    if range.readable && range.executable {
        debug print(`executable mapping at {range.address}`)
    }
}
# }
```

[`memoryRanges`] is synchronous because it only copies cheap host metadata.
Searching the contents of those ranges should still use the suspending scan
APIs so large reads yield between bounded windows.

When attachment should wait for a mapping with an exact size and permission
set, use [`Process.findMemoryRange`]:

```splitscript
# state "game.exe" {}
onAttach {
    let mapping = await process.findMemoryRange(
        0x48000,
        MemoryRangeAccess.ReadWrite,
    )
    print(`mapping starts at {mapping.address}`)
}
```

The future inspects at most one mapped range per update and refreshes the
mapping snapshot after a complete miss. It does not return a temporary
[`None`]; it remains pending until a match appears or the process closes. Use
[`Process.memoryRanges`] instead when a completed absence must immediately
select a different discovery strategy.

An awaited scan remains pending when no signature is present; it does not
produce a temporary zero address. This matches attach-time discovery that
should wait for a module or runtime allocation to become ready. If an absent
pattern must instead select an unsupported-build path after a deadline, keep
that as an explicit timeout/race requirement—the language does not silently
turn a retrying scan into a one-shot result.

## Retaining the last accepted field value

Some ASL `update` blocks overwrite one newly read watcher with its old value to
filter a transient sentinel. The direct port is valid in [`whileAttached`]:

```splitscript
# state "game.exe" {
#     scene: i32 at 0x1000;
# }
whileAttached {
    if current.scene == 7 || current.scene == 8 {
        current.scene = old.scene
    }
}
```

[`current`] is the one mutable snapshot root; [`old`] remains read-only history.
An assignment is visible to later code and later lifecycle actions in the same
tick, and the mutated snapshot naturally becomes [`old`] on the next successful
poll. Compound forms such as `current.count += 1` use the field's ordinary
typed operator.

Use a state-field filter instead when the candidate itself is invalid. This is
the stronger transactional form: it can reject the first candidate before any
snapshot is published, and it keeps the acceptance rule beside the read.

Add an ordinary trailing [`if`] to that pointer-path field:

```splitscript
state "game.exe" {
    scene: i32 at "engine.dll", 0x1000 if value == 7 || value == 8 {
        Err("transient loading scene")
    } else {
        value
    };
    entities: i32 at "engine.dll", 0x2000;
}
```

`value` is the successfully read candidate and has the field's inferred type.
A plain value accepts the candidate; `Err(message)` rejects it. Before the
first snapshot, rejection leaves state uninitialized, so no fabricated old
value or stale value from another process is observable. Afterwards, the field
retains its accepted value and successful sibling fields continue to advance.

For example, the filter can retain a scene during loading scenes 7 and 8 while
an independently read entity count continues to advance. By contrast, an ASL
`update` block that returns `false` does not roll back state at all; it skips
lifecycle decisions after the refresh. SplitScript does not add a separate
lifecycle concept for that behavior until a maintained port demonstrates that
ordinary field expressions and [`whileAttached`] cannot represent the required
result clearly.

## Collection search and run-scoped sets

C# array `.Length` maps directly to the SplitScript method `.length()`. It
returns the [`u32`] element count for both [`[T]`] and fixed [`[T; N]`] arrays. The
compiler can apply this rename automatically, after which signed C# index
arithmetic may still need an explicit width cast.

After choosing a SplitScript collection shape, C# `.Count` also becomes
`.length()`. For an array this is the element count; for [`Set<T>`] it is the
number of unique stored values.

C# `List<T>` maps to SplitScript's [`[T]`] array type. SplitScript will not add a
separate compatibility-shaped `List<T>`. [`[T]`] is the variable-length ordered
sequence, while [`[T; N]`] carries an exact fixed length for layouts and other
code where the size is part of the type. Use [`Set<T>`] only when the original
data is genuinely an unordered collection of unique values, not merely because
its current API happens to be mutable.

Arrays provide `contains` and `indexOf` when their elements support equality:

```splitscript
# state "game.exe" {
#     level: i32 at 0x1000;
# }
let levelRoute = [12, 5, 6, 7, 9, 10, 11, 14]

split {
    let oldIndex = levelRoute.indexOf(old.level) else return false
    let currentIndex = levelRoute.indexOf(current.level) else return false
    return currentIndex == oldIndex + 1
}
```

Unlike C# `List<T>.IndexOf`, `indexOf` returns `u32?`; absence is [`None`], never
a signed `-1` sentinel. Replace an existing element with ordinary indexed
assignment; the index is [`u32`], aliases observe the change, and an out-of-range
index traps just like an indexed read:

```splitscript
# state "game.exe" {}
# onAttach {
# let route = [1, 2, 3]
# let currentIndex: u32 = 1
# let nextLevel = 7
route[currentIndex] = nextLevel
route[currentIndex] += 1
# print(route[currentIndex])
# }
```

Plain indexed assignment evaluates the collection and index once. Compound
forms such as `route[nextIndex()] += 1` additionally evaluate the right operand
once and use the same typed operator as an ordinary `+=`; `nextIndex()` is not
called twice. Growable [`[T]`] supports [`push`], [`extend`], indexed [`removeAt`],
optional [`pop`], first-match `remove(value)`, and capacity-preserving `clear`. C#
`list.AddRange(values)` becomes
`list.extend(values)` once both collections are represented as typed arrays;
self-extension duplicates the original elements once. Successful structural
operations invalidate active iteration. C# `RemoveAt(index)` maps directly to
`removeAt(index)`; an out-of-range [`u32`] index traps just like array indexing.
Use `let last = values.pop() else ...` where C# removes a final list or stack
element: SplitScript returns [`None`] for an empty array instead of throwing.
`List<T>.Remove(value)` maps to `remove(value)`, removes only the first equal
element, and returns whether a match existed. Ignoring that boolean is valid
when the source does not distinguish absence.
[`[T; N]`] remains fixed-length and supports none of these operations.

Use [`Set<T>`] when values are discovered while the run progresses and only
membership matters:

```splitscript
# state "game.exe" {
#     map at 0x1000 as utf8(64);
# }
let visitedMaps = Set.new<String>()

onAttach {
    visitedMaps.clear()
}

split {
    if current.map == old.map {
        return false
    }
    return visitedMaps.insert(current.map)
}
```

[`Set.insert`] returns true only for a new value. The set object and its contents
persist across ticks and detachments until explicitly cleared or the script is
unloaded. Clear it at the lifecycle boundary that matches the original source:
[`onAttach`] for per-process state, or a detected timer-start transition for
per-attempt state. The maintained OpenJK-Speed port exercises the former.

## Bounded integer iteration

For bounded integer iteration, use an explicit SplitScript range instead of
constructing an array of indices:

```splitscript
# state "game.exe" {}
# let count: u32 = 4
# whileAttached {
for index in 0u32..<count {
    print(index)
}
# }
```

[`..<`](syntax@range) excludes the upper endpoint and
[`..=`](syntax@range) includes it. This differs from
Rust, where bare `..` is the exclusive spelling, and from C# range syntax,
where `..` describes slicing bounds rather than iteration. SplitScript requires
the `<` or `=` marker; writing bare `..` produces a diagnostic with both
machine-applicable choices. The type itself uses the same explicit shape,
[`T..<T`](syntax@range) or [`T..=T`](syntax@range), when a range is stored or
passed to a helper. A direct loop
does not allocate a collection or range object.

When membership comes from a small closed enum, a typed bit set remains more
compact and makes the finite domain explicit:

```splitscript
# state "game.exe" {}
enum Chapter {
    Village,
    Farm,
}

let completedChapters: u32 = 0

fn chapterMask(chapter: Chapter) -> u32 {
    return match chapter {
        Chapter.Village => 1u32 << 0,
        Chapter.Farm => 1u32 << 1,
    }
}
```

Detect the timer transition in [`whileAttached`], clear the bit set, and mark the
starting map before [`split`] is evaluated. This reproduces an ASL `timer.OnStart`
handler without a separate event API. The generated update loop runs
[`whileAttached`] before timer-decision actions.

Growable ordered storage, insertion order, and repeated equal values all belong
to [`[T]`]. They do not justify another collection type. Record any still-missing
specific operation rather than describing `List<T>` itself as missing. Indexed
insertion remains deferred until a maintained port demonstrates that it is
needed.

## Static settings declarations

Move ASL `settings.Add(...)` registration into the declarative [`settings`]
block. The quoted text before `=>` is the user-facing label, the identifier is
the statically typed source member, and an optional `key "..."` preserves the
exact stable string stored in the host settings map:

```splitscript
enum Route {
    AnyPercent,
    AllBosses,
}

settings {
    /// Splits when a configured game event occurs.
    "Enable Auto Splitting" => autoSplit key "auto-split": true,
    /// Selects the route-specific split rules.
    "Route" => route: choice {
        "Any%" => Route.AnyPercent default,
        "All Bosses" => Route.AllBosses,
    },
}

state "game.exe" {
    bossDefeated: bool at 0x1000;
}

split {
    return settings.autoSplit
        && settings.route == Route.AllBosses
        && !old.bossDefeated
        && current.bossDefeated
}
```

Consecutive [`///`] documentation comments become the setting tooltip. The
legacy comma-separated tooltip string is deliberately not accepted. Boolean
settings are [`bool`]. A quoted string default creates a free-form text input
whose current and previous values are available as [`String`]. A `choice` is
the source enum named by its variants. One choice entry must carry `default`.
Quoted groups add visual hierarchy only:
they do not disable their children, so preserve an ASL parent checkbox by
testing that boolean explicitly in the split condition.

File selectors produce a [`String`] path and can declare extension globs, a
catch-all `_`, and MIME filters. They remain typed settings rather than direct
filesystem access:

```splitscript
state "game.exe" {}

settings {
    "Paths" {
        /// Selects the layout consumed by the host integration.
        "Layout File" => layoutFile: file {
            "Layout files" => "*.json *.yaml",
            _ => "*.*",
            mime => "application/json",
        },
    },
    /// Reloads the selected layout after the user changes this option.
    "Live Reload" => liveReload: false,
}

whileAttached {
    if settings.liveReload != oldSettings.liveReload {
        print(`Live reload: {settings.liveReload}`)
    }
    setVariable("Layout", settings.layoutFile)
}
```

[`settings`] and [`oldSettings`] are complete current and previous views refreshed
once per update, so a comparison detects user changes without caching a second
copy. Statically known values should use their typed members. When a data table
selects among boolean keys, use `settings.enabled(key)`; use
`settings.contains(key)` when declaration membership and a disabled value must
remain distinct. Literal keys are checked and completed against this file's
declarations, including explicit `key` strings.

When the source registers a bounded numbered series in a `settings.Add` loop,
use a compile-time [`settings family`] rather than copying every generated
declaration by hand. The next section defines its label, key, default, tooltip,
and lookup behavior.

## Finite settings families

Prefer direct `settings.name` access when the setting is known statically. For
data tables whose entries select among declared boolean settings, give each
declaration its exact host-map string with `key "..."` and use
`settings.enabled(key)`. This remains boolean-only and is not a dynamically
typed replacement for text, choice, or file settings. Literal keys are validated and
completed against the declarations; computed unknown keys return false. If the
original settings have a boolean parent, gate the child result explicitly; a
quoted SplitScript heading is visual only. The complete A Plague Tale example
preserves its **All Chapters** parent semantics this way.

When cursor advancement depends on declaration membership rather than whether
the split is enabled, keep the two questions separate:

```splitscript
# state "game.exe" {}
# settings {
#     "Checkpoint" => checkpoint: true,
# }
# let checkpointIndex = 0
# split {
# let checkpointKey = "checkpoint"
if settings.contains(checkpointKey) {
    checkpointIndex += 1
    return settings.enabled(checkpointKey)
}
# return false
# }
```

`contains` recognizes declared boolean, text, choice, and file keys, including
explicit `key "..."` spellings. It returns false for visual headings and unknown
keys. This matches legacy `Settings.ContainsKey` without exposing a dynamically
typed host map.

When legacy `startup` creates a bounded numbered family, declare it at compile
time rather than expanding dozens of source members or mutating the settings
map:

```splitscript
# state "game.exe" {}
settings {
    "Levels" {
        for level in 2..=36 {
            `{level}` key `{level}`: true,
        },
    },
}
```

This registers the exact stable keys `"2"` through `"36"`. The generated
entries are intentionally available only through `settings.enabled(key)`; they
do not create artificial members such as `settings.level17`. Label and key
templates may interpolate only the range binding, and a documentation comment
on the family becomes every generated tooltip. See the maintained Drug Dealer
Simulator port for registration and runtime evidence.

A source loop with a small number of exceptional defaults is still finite
declaration data, not runtime settings registration. Partition the uniform
ranges and declare each exception directly:

```splitscript
# state "game.exe" {}
settings {
    "Levels" {
        for level in 1..=2 {
            `Level {level}` key `{level}`: false,
        },
        "Level 3" => level3 key "3": true,
        for level in 4..=5 {
            `Level {level}` key `{level}`: false,
        },
    },
}
```

All five host keys remain stable and available to
[`SettingsView.enabled`](method@SettingsView.enabled). Runtime registration is
needed only when the set of keys itself is discovered after compilation, not
merely because the ASL source used a [`for`] loop to declare a bounded table.

## Snapshot-dependent helper functions

Ordinary helper functions may refer to [`old`] and [`current`] directly, just as a
process-dependent helper may refer to `process`:

```splitscript
state "game.exe" {
    level: u32 at 0x1000
}

fn enteredLevel(level) {
    return old.level != level && current.level == level
}

split {
    return enteredLevel(7u32)
}
```

The compiler derives a state-snapshot requirement from the helper body and
propagates it through every calling helper. Such functions are offered only in
contexts where a complete pair of snapshots exists: [`whileAttached`] and the
timer-decision actions. Calling one from [`setup`], [`onAttach`], [`onDetach`], a
state source, or a state filter produces a focused diagnostic. This keeps the
concise ASL helper shape without exposing default-initialized or stale state.

Pass snapshots explicitly when the helper should operate on caller-selected
history or when its argument order is part of the helper's meaning:

```splitscript
state "game.exe" {
    level: u32 at 0x1000
}

fn levelChanged(before, after) {
    return before.level != after.level
}

fn enteredLevel(before, after, level) {
    return levelChanged(before, after) && after.level == level
}

split {
    return enteredLevel(old, current, 7u32)
}
```

The field accesses and calls infer `before` and `after` as the generated
`StateSnapshot` type. They are ordinary read-only values, so the caller may
choose which available snapshots represent each role and may forward them
through more helpers. Passing snapshots removes the helper's implicit snapshot
dependency; only evaluating [`old`] and [`current`] at the call site requires a
committed pair. Direct access remains the concise choice when a helper always
means this tick's transition.

## Legacy ASL lifecycle blocks

The similarly shaped block names are not interchangeable. The original
LiveSplit component invokes them at different boundaries:

| ASL construct | Exact legacy timing | SplitScript direction |
| --- | --- | --- |
| `startup` | Once when the script is loaded, before process attachment | Put settings in [`settings`], constant data in global initializers, and remaining process-independent statements in [`setup`]. |
| `init` | Once for each found process, after one legacy state refresh; a failure retries attachment initialization | Put suspending discovery and attachment-shape initialization in [`onAttach`]. Put synchronous work that consumes the first complete snapshot in [`onStateReady`]. |
| `update` | After each refresh and before all timer decisions; `false` skips the remaining decisions for that tick | Put per-tick work in [`whileAttached`]. It may suspend for recoverable discovery; an explicit `return false` preserves the legacy control result exactly. |
| `exit` | When the attached process exits | Use [`onDetach`]. It runs exactly once for a real process closure and never at initial detached startup. |
| `shutdown` | When the script is disabled, reloaded, dropped, or LiveSplit exits | No exact host callback exists yet; do not approximate it with [`onDetach`]. |
| `timer.OnStart`, `timer.OnReset` | Timer events that may be raised independently of this script's decision blocks and while no game is attached | Keep the logic in [`onStart`] and [`onReset`]. SplitScript samples timer state before process attachment on every update, so these actions remain available while detached. |
| `timer.OnSplit` | An ordered segment event that must distinguish ordinary splits, skips, and undos | No exact host event exists yet. Do not approximate bookkeeping-sensitive behavior from only the current split index. |

For example, process-independent ASL startup statements belong in [`setup`], not
[`onAttach`]:

```splitscript
# state "game.exe" {}
setup {
    print("Autosplitter loaded")
}
```

[`setup`] runs at the beginning of the module's first interruptible host update,
after settings are available, but cannot use `process`, `gba`, [`current`],
[`old`], [`await`], or [`retry`]. A debug-watch replacement loads a new module and
therefore runs it again on that module's first update.

[`onStart`] and [`onReset`] are timer-global rather than process lifecycle
actions. They run before attachment and state polling when two consecutive
updates observe the timer leave or enter [`TimerState.NotRunning`]. The first
update only establishes a baseline. They may use settings and ordinary globals,
but not `process`, an emulator provider, attachment-scoped globals,
[`current`], or [`old`]. A start or reset requested by this script is observed
on the following update instead of being invoked directly from [`start`] or
[`reset`]. This sampled contract delivers an ordinary persistent transition
once, but cannot recover a start and reset that both occur between updates.

Attempt state uses the same global syntax as attachment state; there is no
separate attempt declaration or lifecycle block. Declare a bare global and
assign it directly on every completing path through [`onStart`]:

```splitscript
# state "game.exe" {}
let collectedItems

onStart {
    collectedItems = 0
}

onReset {
    print(`Collected {collectedItems} items`)
}
```

The compiler infers `collectedItems` as attempt-scoped (sometimes called
run-scoped in existing autosplitters). It remains available
across a game-process detach and is cleared after [`onReset`] completes. It may
also be used from [`split`], [`reset`], [`isLoading`], and [`gameTime`], with the
requirement propagating through helper functions. Loading an autosplitter while
the timer is already running does not synthesize a start event or expose backend
default storage; ordinary use should load the autosplitter before beginning the
attempt.

Legacy `init` combines two boundaries that SplitScript keeps explicit. Use
[`onAttach`] for discovery that may suspend and for attachment-shape initialization. Use
[`onStateReady`] for synchronous initialization that needs polled state:

```splitscript
image "Assembly-CSharp" {
    class GameManager {
        static GameManager instance from "Instance";
        u32 level;
    }
}

state Unity ["game.exe"] {
    level: u32 = GameManager.instance?.level?;
}

onStateReady {
    print(`Initial level: {current.level}`)
}
```

[`onStateReady`] runs once per attachment only after every field in the first
snapshot was read and accepted. [`old`] and [`current`] are both that snapshot, so
initialization cannot look like a transition from default values. It cannot
suspend. [`whileAttached`] and timer-decision actions begin on the next update.

Legacy `update { return false; }` maps directly to [`whileAttached`]. The state
snapshot has already refreshed, but the remaining timer decisions are skipped
for that update:

```splitscript
# state "game.exe" {}
# let helperLoaded = true
whileAttached {
    if !helperLoaded {
        return false
    }

    // Per-update bookkeeping.
}
```

Falling through, a bare [`return`], or `return true` continues to [`start`],
[`isLoading`], [`gameTime`], [`reset`], and [`split`] as applicable. This control result
does not reject or roll back the refreshed snapshot.

[`whileAttached`] may also [`await`] or [`retry`] when data must be rediscovered
without waiting for the process to restart. The runtime owns at most one
invocation per attachment and polls it once after each state refresh. While it
is pending, all timer-decision blocks are skipped. When it completes, `false`
still skips that completion update; falling through or `true` permits the
decisions, and a fresh invocation starts only on the following update. Process
closure cancels the pending invocation.

Keep ordinary one-time discovery in [`onAttach`]. Use suspending
[`whileAttached`] only when the discovered allocation itself can move while the
same process remains open:

```splitscript
# state "game.exe" {}
let framebuffer

onAttach {
    framebuffer = await process.scanMemory(sig"10 08 ?? ??")
}

whileAttached {
    let header = process.read<u16>(framebuffer) else 0
    if header != 0x0810 {
        framebuffer = await process.scanMemory(sig"10 08 ?? ??")
        return false
    }
}
```

State fields have already refreshed before each poll of this action. A newly
assigned address therefore affects declarative state reads on the following
update; returning `false` on rediscovery prevents timer decisions from using
the completion update's older snapshot.

ASL `refreshRate` is a frequency. Migrate a stable attached cadence to the
declarative lifecycle policy:

```splitscript
# state "game.exe" {}
tickRate {
    attached: 60,
}
```

SplitScript defaults to 120 Hz while attached and 1 Hz while detached. It
applies the attached rate before [`onAttach`] begins, which is important when
module or signature discovery suspends across updates, and restores the
detached rate before [`onDetach`]. Add `detached: value` only when the 1 Hz
default is unsuitable. Use [`setTickRate`] only when the rate must change
dynamically within one attachment; the next lifecycle transition reapplies the
declaration.

## Process-exit game-time cleanup

ASL commonly pauses game time in `exit`. Map that cleanup directly to
[`onDetach`]:

```splitscript
# state "game.exe" {}
onDetach {
    timer.pauseGameTime()
}
```

The compiler invokes this block once after clearing the closed handle, provider
state, attachment-scoped globals, and pending process-lifetime continuations. Neither
`process` nor state snapshots are available in [`onDetach`]: a process may close
before attachment initialization or the
first state poll completes.

Use [`isLoading`] for ordinary load removal. [`timer.pauseGameTime()`] and
[`timer.resumeGameTime()`] are explicit lifecycle tools, not a replacement for
that declarative action.
