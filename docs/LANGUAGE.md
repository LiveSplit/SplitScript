# SplitScript language design

## Design center

SplitScript combines familiar expression syntax with ASL's domain concepts
rather than trying to be source-compatible with JavaScript, C#, or old ASL.

| General scripting familiarity | Autosplitter-specific syntax |
| --- | --- |
| `let`, braces, property access | Direct `start`, `split`, `reset`, `isLoading`, `gameTime` blocks |
| `==`, `!=`, `&&`, `||`, `if` | `state`, `settings`, `current`, `old` |
| Inference by default, annotations when useful | Declarative pointer paths and automatic polling |
| Familiar `await` expressions | Process-lifetime cancellation and tick-based retry |

It is statically typed. There is no JavaScript-style numeric supertype and no
implicit widening between integer widths. This is important when values come
from process memory.

New authors should begin with [`GETTING_STARTED.md`](GETTING_STARTED.md). Use
this longer page as a concept reference:

- Attach and read game state: [state and pointer paths](#state-and-pointer-paths)
  and [discovered state and watchers](#discovered-state-and-watchers).
- Express ordinary logic: [variables and inference](#variables-and-inference),
  [value blocks](#value-blocks), and [functions](#functions).
- Model values and collections: [structs](#structs),
  [pattern matching](#pattern-matching), [arrays](#arrays), [sets](#sets), and
  [integer ranges](#integer-ranges).
- Configure and control the timer: [settings](#settings) and [actions](#actions).
- Handle absence, failure, and waiting: [optional and fallible values](#optional-and-fallible-values)
  and [structured async initialization](#structured-async-initialization).
- Decode native values and text: [typed process memory](#typed-process-memory).

Inference flows through global uses and assignments as well as initializers.
An unannotated mutable global initialized to `None` becomes `T?` when a later
ordinary assignment supplies a `T`; a standalone `None` global remains the
zero-sized `None` type. An explicit annotation remains available when it makes
the intended stored type clearer.

A top-level declaration without an initializer gets its lifetime from the one
lifecycle action that initializes it. `onAttach` creates attachment-scoped
state, while `onStart` creates attempt-scoped state. The initializer must
assign the value on every completing path, and its type is inferred
bidirectionally from those assignments and later uses. Assigning the same bare
global from both boundaries is an error rather than an implicit lifetime
choice.

A top-level declaration with an initializer is module-scoped state. Its
initializer runs exactly once, before `setup`, and may allocate or call
synchronous pure helpers. The initializer must be closed: it cannot read or
write another global, observe settings, timer, process, or state context, or
suspend. Runtime-dependent values belong in the lifecycle boundary that owns
their lifetime.

Attachment-scoped storage is cleared when the process detaches and is not
available in detached actions. A value may be initialized for only some
attachment shapes; a direct test or `match` on the ordinary enum globals that
describe those shapes refines where it and helpers that use it are available.

Attempt-scoped storage remains alive across process detach and is cleared after
`onReset`. It is available in `onStart`, `onReset`, `split`, `reset`,
`isLoading`, and `gameTime`; helpers inherit this requirement. Actions that
may otherwise run before a start, such as `setup`, `onAttach`, `onDetach`,
`whileAttached`, and `start`, cannot use it. An initialized top-level
declaration remains module-scoped.

Closed comma-separated forms use punctuation rather than line breaks to
separate items. This includes arguments, array and struct literals, match arms,
struct fields, enum variants, settings, choice options, and file
filters. A trailing comma is always optional. The formatter adds one when it
lays a list out across multiple lines and omits it for a compact one-line list.
State fields are instead separated by semicolons because their unclosed pointer
paths already use commas between offsets; the final semicolon is optional and
the formatter adds it in multiline state blocks. Ordinary statements remain
newline-terminated, with semicolons available when multiple statements share a
line.

## Polling-rate policy

SplitScript owns the ordinary state-provider polling policy. With no
declaration, the generated module selects 1 Hz with no host process and 120 Hz
while acquiring or polling a state provider. The active rate is applied before
cooperative provider discovery begins because range enumeration and signature
scans make bounded progress per update. It remains in effect through `onAttach`
and attached polling. Ending the logical attachment restores 1 Hz before
`onDetach` runs. For native games that boundary normally follows process
closure. Emulator mapping loss ends the logical attachment independently while
the host remains open; rediscovery raises the active rate again before it
starts. An emulator waiting for a guest therefore currently stays at the active
rate when its discovery operation is an indefinitely pending scan.

Override either lifecycle rate declaratively when a game needs another cadence:

```splitscript
tickRate {
    attached: 60,
    detached: 2,
}
```

An omitted field keeps its 120 Hz or 1 Hz default. `setTickRate(hz)` remains
available for a temporary dynamic adjustment; the next attachment or detach
transition reapplies this declaration. Rates must be finite and greater than
zero.

## State and pointer paths

```splitscript
state "game.exe" {
    level: u16 at "game.exe", 0x1234, -0x20;
    score: u32 at 0x7ff612341000;
}
```

To attach to the first available edition of a game, provide an ordered process
list. Each name is attempted once per tick until one attaches.

```text
state ["game.exe", "game-demo.exe"] {}
```

A typed state provider may introduce additional read-only attachment context
beside its process value. For example, [`state Unity`](provider@Unity) exposes
`unity: UnityContext` wherever `process` is available. Context preparation is
demand-driven and runs once before [`onAttach`], so merely selecting a provider
does not discover facilities the script never references:

```splitscript
state Unity ["game.exe"] {
    scene = unity.scenes.active();
}
```

With a module name, the first signed `i64` offset is added to the module base.
Every remaining signed offset follows a 64-bit pointer, adds the offset, and
continues. Without a module name, the first value is a full-width unsigned
absolute address and only subsequent offsets are signed. Address addition wraps
modulo the 64-bit address space. The final read uses the declared width and
signedness. A failed path rejects that field's candidate value. An offset whose
magnitude does not fit `i64` is rejected instead of silently changing its sign.

State field annotations are optional and participate in whole-program
inference. Expression-backed fields normally obtain their type directly from
the right-hand side. A pointer-path field has no typed right-hand side, so its
type must come from a `current`/`old` use or an explicit annotation. The
compiler reports an ambiguity if neither provides enough information.

A state field can refer to another field from the same active shape by its
source name. This works in an expression-backed field and as a dynamic pointer
base after [`at`](syntax@at):

```splitscript
state "game.exe" {
    health: u32 at playerAddress, 0x20;
    playerAddress: address = process.read(0x1000)?;
    displayedHealth: u32 = health;
}
```

Declarations do not need to be topologically ordered. The compiler builds the
dependency graph, evaluates `playerAddress` before `health` and `health` before
`displayedHealth`, and reports every participating declaration when it finds a
cycle. In a conditional field group, a reference resolves to a sibling that is
available under the same shape predicate.

After attachment, initialization waits for one poll in which every required
field succeeds. That snapshot initializes both `old` and `current`, and
lifecycle actions begin on the following poll. Consequently action code never
observes synthetic zero-filled state. Later, each successful field advances;
a failed field retains its last accepted value while successful sibling fields
still advance. A dependent field is not evaluated in a poll where any direct
dependency failed, so it cannot follow an address from an older candidate; it
retains its own last accepted value too. The resulting snapshots are ordinary
typed values in action code.

Some watchers use failed memory access as meaningful absence rather than a
transient error. Write that choice explicitly with an optional pointer field:

```text
state "game.exe" {
    requiredMenu: String at 0x1000 as utf8(32);
    optionalMenu: String? at 0x2000 as utf8(32);
}
```

The required field keeps the initialization/retention behavior above. The
optional field accepts a failed module lookup, pointer traversal, final memory
read, or decoder as `None`; a successful read is `Some(T)`. It therefore never
blocks initialization and can visibly transition between a value and `None`
in `old` and `current`. The `T?` annotation is mandatory because it selects
failure semantics in addition to constraining the inferred value type.

A pointer-path field can use an ordinary trailing `if` expression to accept the
raw value or produce an error. Expression-backed fields
already have an ordinary right-hand side and should put the `if` there instead:

```text
state "game.exe" {
    scene: i32 at 0x1000 if value == 7 || value == 8 {
        Err("transient loading scene")
    } else {
        value
    };
    entities: i32 at 0x2000;
}
```

Inside this field-local expression, the read-only `value` binding has the
field's inferred type. A plain value is accepted and `Err(message)` rejects the
candidate. Before initialization, any rejected required field leaves state
uninitialized. Afterwards, rejection retains that field's last value without
discarding a new `entities` value from the same poll. Snapshot `current` and
`old` values stay read-only and are available only after initialization.

Games with multiple supported memory shapes use ordinary enum globals for the
facts that select those shapes. Assign every such global in [`onAttach`], then
use normal `if`, `else if`, and `else` groups in state and managed declarations:

```splitscript
enum Edition {
    BaseGame,
    Demo,
}

enum Storefront {
    Steam,
    GOG,
}

let edition: Edition
let storefront: Storefront

state Unity ["game.exe"] {
    if edition == Edition.BaseGame {
        level: u32 at 0x1000;
    } else {
        scene: String = unity.scenes.active();
    }
}

onAttach {
    edition = Edition.BaseGame
    storefront = Storefront.Steam
}
```

Shape globals are attachment-scoped and become read-only after `onAttach`.
The same predicate refines every declaration guarded by it, including managed
fields. Each later branch covers exactly the shape combinations not selected
by an earlier branch, including conditions over several independent facts:

```splitscript
image "Assembly-CSharp" {
    class GameManager {
        static GameManager instance;

        if edition == Edition.BaseGame {
            u32 level;
        } else {
            String scene;
        }
    }
}

whileAttached {
    if edition == Edition.BaseGame {
        let manager = GameManager.instance else return
        print(manager.level else 0)
    } else {
        let manager = GameManager.instance else return
        print(manager.scene else "Unknown")
    }
}
```

The same class declaration also provides cooperative live-instance discovery.
`await T.instances()` produces a completed `[T.Ref]` snapshot without blocking
one update on an unbounded process scan:

```splitscript
onAttach {
    let managers = await GameManager.instances()
    print(managers.length())
}
```

If conditional managed fields give each possible dimension combination a
unique presence pattern, attachment initializes the shape globals automatically
before user [`onAttach`] code runs. User attachment code can already read them.

Facts are independent, so an edition and storefront do not require a cartesian
product of public variants. Managed classes do not create separate selectors.
A class-only distinction that affects its public fields is another ordinary
attachment-wide enum global; metadata spellings that preserve the public shape
stay private binding alternatives. If managed metadata cannot uniquely identify
every combination, [`onAttach`] must assign every shape global explicitly after
checking the remaining build facts.

Fields declared in every exhaustive branch with a compatible type form the
common snapshot interface. A missing field or a same-named field with a
conflicting type remains branch-specific and requires refinement:

```text
enum Build { V8, V9 }
let build: Build

state "Ronin.exe" {
    if build == Build.V8 {
        loading: i32 at 0x100;
        bike: i16 at 0x104;
    } else {
        loading: i32 at 0x200;
        bike: u16 at 0x204;
    }
}

onAttach { build = Build.V8 }

isLoading {
    return current.loading == 1
}

split {
    return match build {
        Build.V8 => old.bike != 21_368 && current.bike == 21_368,
        Build.V9 => old.bike != 52_688 && current.bike == 52_688,
    }
}
```

The incompatible `bike` declarations remain distinct typed fields. They are not
optional and the compiler does not synthesize a default to hide the difference.
Accessing `current.bike` without a matching shape refinement is an error.

## Variables and inference

```text
fn consumeU64(value: u64) {}

let tickCount = 0

whileAttached {
    consumeU64(tickCount) // the call infers tickCount as u64
}

split {
    let changed = current.level != old.level; // inferred bool
    return changed;
}
```

There is one declaration form: `let`. The compiler assigns fresh type variables
to unannotated values and unifies constraints from assignments, operators,
arrays, function bodies, return values, and call sites. Information therefore
flows in either direction: both `value == 0` and `0 == value` infer the literal
from `value`. This applies equally to top-level persistent variables: their
types may come from their initializer or from any later use. An annotation is
only needed to constrain an otherwise ambiguous value or document an important
boundary.

Struct member access participates in the same whole-program inference. The
compiler can defer resolving a field path until a later call site supplies the
parameter's nominal struct type, so helpers do not need redundant annotations:

```text
fn levelTimeText(parts) {
    return `{parts.minutes as u32}:{parts.seconds as u32}`
}

levelTimeText(current.levelTimeParts)
```

Several accessed fields can jointly identify a unique struct even without a
call-site constraint. If the remaining field set matches multiple nominal
structs, the compiler reports those candidates instead of guessing.

Annotations and integer suffixes are constraints, not a routine requirement.
Integer literals may be decimal (`42`), hexadecimal (`0xff`), or binary
(`0b1111_0000`); `_` separators are ignored. Suffixes such as `1u8`, `10i64`,
`0xffu32`, and `0b1000u16` remain available when a literal is genuinely
unconstrained or when an exact type should be documented. A leading minus is
part of an integer literal, so signed minima such as `-128i8` and
`-9223372036854775808i64` are representable directly and may also appear in
patterns. Parenthesized and other computed operands still use ordinary unary
negation, as in `-(value + 1)`. An
unresolved inference component defaults when it contains an unsuffixed literal
or has a specific numeric-kind constraint: integer literals and `Integer`
values default to `i32`, while floating-point literals and `Float` values
default to `f64`. An integer-looking literal required to satisfy `Float` also
defaults to `f64`. Broader capabilities such as `Numeric`, `Signed`,
`MemoryReadable`, or `Display` do not choose a representation on their own; an
otherwise ambiguous value needs an annotation. Memory reads are stricter: a
component constrained by `MemoryReadable` never uses a numeric default, even if
it also contains a literal or an `Integer`/`Float` constraint. The concrete
memory representation must come from an annotation, explicit generic argument,
or another exact type. Mutable `let` bindings are monomorphic. Unannotated user
function parameters and results are generalized at the declaration boundary,
then instantiated independently for each call site.

Decimal floating-point literals may use an exponent, such as `1e-45` or
`6.022e+23`. They are rounded once to their inferred `f32` or `f64` target and
must remain finite and nonzero when the written significand is nonzero.
Representable subnormal values are valid: `let value: f32 = 1e-45` produces the
smallest positive `f32` (bit pattern `0x00000001`), while
`let value: f64 = 5e-324` produces the smallest positive `f64`. A literal that
underflows its target to zero or overflows it to infinity is a type error rather
than silently changing the comparison value.

Hovering a decimal floating-point literal shows both its inferred width and
the exact rounded IEEE-754 bits. Use `f32.fromBits(bits)` or
`f64.fromBits(bits)` when the bit pattern itself is the source data, and
`.toBits()` for the inverse reinterpretation. These operations preserve signed
zero and NaN payloads and do not perform a numeric conversion.

```splitscript
let negativeZero = f32.fromBits(0x8000_0000u32)
let representation = negativeZero.toBits()
```

Assignments support `=`, the arithmetic compound forms `+=`, `-=`, `*=`, `/=`,
and `%=`, plus `|=`, `&=`, `^=`, `<<=`, and `>>=` for integers. A compound
assignment uses exactly the same operand typing and runtime operation as its
ordinary binary operator while resolving the destination only once. These
forms also work on indexed array elements.

Every integer type, including `address`, can be formatted in bases 2 through 36
with `value.toString(radix)`. The operation returns `String!` because a dynamic
radix may be outside that range. Alphabetic digits are lowercase; compose
`toAsciiUpperCase()` when uppercase output is desired. Signed values use a
leading minus sign and their mathematical magnitude, including signed minima.

```splitscript
let hexadecimal = addressValue.toString(16) else ""
```

Unread parameters, local variables, loop elements, match payloads, and
`await`/`retry` bindings produce non-fatal warnings. The analysis follows
resolved value identities, so shadowed names and method receivers are handled
correctly. A plain assignment is only a write and does not make a binding used;
a compound assignment also reads the previous value. Prefix an intentionally
unused name with `_`. The warning's quick fix chooses a non-conflicting
underscore-prefixed name and updates writes to that same binding.

The compiler separately warns when a state field's produced snapshot value is
never read by reachable code. Reads through `current`, `old`, and sibling state
fields count and propagate through sibling dependencies. A compatible field in
several conditional branches produces one warning with every physical
declaration labelled. Displaying a field through `print` or `setVariable` is an
ordinary read and does not warn. Polling still executes when this warning is
present, so the compiler does not silently remove process reads or other
effects. Prefix an intentionally observation-only field with `_`; the editor's
validated rename fix updates every declaration of the same logical field.

The compiler also warns about private globals, functions, structs, and enums
that cannot be reached from lifecycle behavior, state polling expressions, or
the host-visible settings interface. Reachability is transitive and follows
resolved identities: a helper called only by another dead helper is still
unused, while a helper called by an unused state field still executes and
remains reachable. Types in a reachable function signature and types nested
inside runtime state storage or a reachable struct or enum remain live. Debug
statements participate in this analysis in both build profiles so editor
warnings do not change when publishing a release build. Prefix an intentionally
reserved declaration with `_` to suppress its warning.

That stable ordinary-unused result is paired with profile-aware guidance. A
normal local, global, or function that is reachable, but only through `debug`
statements, produces `SS1009`: release builds retain work that no release code
can consume. The analysis propagates profile reachability through helper calls
and named function values, so an entire diagnostics-only helper chain is
identified rather than only its first function. A quick fix inserts `debug`
when erasing the whole declaration is safe. If release-visible code still
assigns a global or local, the warning remains but no unsafe edit is offered.
Declarations already nested in debug-only code do not warn.

Reachable structs and enums receive member-level checks without cascading from
an entirely dead type. Accessing a struct member reads that field; merely
constructing or deserializing the struct does not. Constructing or matching an
enum variant observes that variant, and variants exposed by a choice setting
are host-visible. Structural `==` and `!=` observe every field or variant in the
recursively compared shape. Unobserved fields and variants produce non-fatal
warnings with their exact declaration-name spans and support the same `_`
suppression convention.

Statically named value settings are checked by the same reachable-code
analysis. Reading `settings.name` or `oldSettings.name` observes that setting,
as does a literal-key `settings.enabled("runtime-key")` or
`settings.contains("runtime-key")` lookup. A computed key may name any setting,
so its presence conservatively suppresses unused-setting warnings. Headings and
the generated members of settings families are not diagnosed individually. An
unused setting's quick fix prefixes its source name with `_` while retaining an
explicit host key, which keeps existing saved settings compatible.

Warning codes are stable tooling identifiers: `SS1001` denotes a discarded
must-use value, `SS1002` an unread local binding, `SS1003` an unreachable
declaration, `SS1004` an unused state field, setting, struct field, or enum
variant, `SS1009` a declaration consumed only by debug code, and `SS1010` an
explicit struct initializer that can use field shorthand. The wording may
improve without requiring editor integrations to classify messages by text.

Compiler hosts can configure every warning code as `allow`, `warn`, or `deny`.
Allowing suppresses that diagnostic, while denying makes the configured build
fail but keeps the original `SS100x` code and source information. This policy
does not change whether the source parses or type-checks, so editor semantic
features remain available for denied warnings. With `splitc`, repeat
`--allow`, `--warn`, or `--deny` followed by a code; use `warnings` to select
all warning codes. Later arguments override earlier selectors.

The language server offers preferred quick fixes that apply the `_`
suppression convention. For ordinary declarations, state fields, and nominal
members, the action is a complete validated rename: references in dead helper
code, shared conditional field declarations, and struct-literal labels are updated as
well, name collisions gain additional underscores, and the edited program must
still preserve every resolved declaration identity. An unused setting instead
retains or introduces its original host key while changing only its statically
accessible source name.

Supported value types are:

- `bool`
- `char`, exactly one Unicode scalar value
- `i8`, `u8`, `i16`, `u16`, `i32`, `u32`, `i64`, `u64`
- `address`, a nominal 64-bit target-process address
- `f32`, `f64`
- `String`, immutable UTF-8 text
- `[T]`, a mutable array whose length is not encoded in its type
- `[T; N]`, a mutable array with exactly `N` elements
- `T?`, an optional value containing either `Some(T)` or `None`
- `T!`, a result containing either `T` or a standard string error
- the built-in value types `Duration`, `FileVersion`, and `Module`
- `Signature`, the compile-time-only type produced by `sig"..."`

Character literals use single quotes and contain exactly one Unicode scalar
value. They support the ordinary escaped characters plus `\u{...}` scalar
escapes. A `char` is distinct from an integer: it supports equality and
`Display`, and `value as u32` exposes its scalar value, but arbitrary integers
cannot be cast into characters. It is also not directly memory-readable,
because an address alone does not specify a character encoding.

```splitscript
let separator = '/'
let smile = '\u{1f642}'
print(`Character: {smile}`)
```

The usual arithmetic, comparison, logical, bitwise, and shift operators are
supported. Because values are statically typed, `==` and `!=` are unambiguous;
there are no coercing versus strict comparison variants.

Unary `!` is type-directed: it performs logical negation on `bool` and a
width-preserving bitwise complement on every integer type. The familiar `~`
spelling is diagnosed with a machine-applicable replacement. Integer
arithmetic, unary negation, bitwise operations, and shifts normalize their
results to the declared width, so `255u8 + 1u8` wraps to `0u8`.

Operators use Rust's relative precedence. From tightest to loosest, the
currently supported operators are unary operators, `as`, `*`/`/`/`%`, `+`/`-`,
`<<`/`>>`, `&`, `^`, `|`, comparisons, `&&`, and `||`. In particular, bitwise
operators bind more tightly than comparisons, so `level & 1 == 0` means
`(level & 1) == 0`. Comparisons share one precedence level and cannot be
chained without parentheses.

Every integer and floating-point type provides type-directed `min`, `max`, and
`clamp` methods. Arguments are inferred as the receiver's exact type and each
receiver and argument is evaluated once. `clamp(lower, upper)` is equivalent to
`value.max(lower).min(upper)` for correctly ordered bounds.

```text
let cappedStage = stage.min(7)
let nonNegative = score.max(0)
let normalized = amount.clamp(0.0, 1.0)
```

`if` conditions are expressions and do not require delimiter parentheses.
Parentheses remain available for ordinary expression grouping. `else if`
chains do not need a nested brace block.

```text
if level == 14 {
    act = "X"
} else if level & 1 == 0 {
    act = "1"
} else {
    act = "2"
}
```

`if` can also produce a value. An expression-valued `if` requires an `else`,
and both branches are inferred bidirectionally from each other and from the
surrounding expected type. Only the selected branch is evaluated.

```text
let label: String = if isDemo {
    "Demo"
} else if isDlc {
    "DLC"
} else {
    "Base Game"
}
```

`while` repeats a statement block as long as its `bool` condition remains true.
The condition is evaluated before every iteration, and declarations in the body
are scoped to that body.

```text
let index = 0
let total = 0
while index < 5 {
    index += 1
    total += index
}
```

`loop` repeats without a condition. It is an expression: each `break value`
supplies a possible result, and inference unifies those values with each other
and with the surrounding expected type. A bare `break` supplies `None`.

```text
let moduleName: String = loop {
    let module = process.loadedModule("EngineWin64s.dll")
        else process.loadedModule("EngineWin64sv.dll")
        else {
            await nextTick()
            continue
        }
    break module.name
}
```

A `loop` with no reachable `break` has type `Never`, so it can satisfy any
expected expression type and makes following control flow unreachable. This is
the direct form for an intentional infinite loop:

```text
fn waitForever() -> async Never {
    loop {
        await nextTick()
    }
}
```

Unlike Rust, a function body still does not return its final expression
implicitly. Use `return loop { ... }` when a value-producing loop is the
function result. C#, JavaScript, and ASL authors should prefer `loop` over
`while true` only when unconditional repetition or a value-carrying break is
the actual intent.

`for name in array` visits each element of a general `[T]` or exact-length
`[T; N]` array. The array expression is evaluated once, the element type is
inferred in both directions, and the read-only element binding is scoped to the
loop body.

```text
for item in inventory {
    if item == ignoredItem {
        continue
    }
    inspect(item)
}
```

`for` supports the same `break`, `continue`, and fallback control flow as
`while`. A `for` body in `onAttach` may also use `await` or `retry`; the array,
next index, current element, and collection identity are retained across
suspension without evaluating the array expression again.

Structurally changing the iterated collection invalidates that traversal. If
an alias appends to the array, or successfully inserts into, removes from, or
clears the set, the loop stops with a runtime error when it next advances. This
fail-fast rule catches aliasing too and does not allocate a snapshot on every
tick. Replacing an existing array element with `set` is not a structural
change. A duplicate set insertion or removal of a missing value is also a
no-op and does not invalidate the loop.

`break` exits the nearest enclosing loop. In a `loop` expression,
`break expression` also supplies the loop's value. Value-carrying breaks are
not accepted by `while` or runtime `for`; this prevents a break inside a nested
statement loop from accidentally targeting an outer value loop. `continue`
skips the rest of the current iteration and evaluates that loop's condition
again, or immediately begins the next unconditional `loop` iteration. Nested
`if` blocks do not change which loop either keyword targets.

```text
while index < values.length {
    index += 1
    if index == ignoredIndex {
        continue
    }
    if total >= limit {
        break
    }
}
```

[`break`] and [`continue`] are ordinary expressions with type [`Never`]. They
can be used wherever an expression is accepted inside their target loop, not
only in a special fallback form. This includes a fallback for a `T?` or `T!`
value nested inside an expression-valued [`if`], [`match`] arm, or
short-circuit expression:

```text
let entry = process.read(address) else continue
let selected = if useFallback {
    optionalValue else break
} else {
    defaultValue
}
```

Inside `onAttach`, `await` and `retry` may suspend within a loop. Resuming does
not replay work from before the pending operation: the loop can `break`,
`continue`, or complete the current iteration normally. Nested loops target
their nearest loop and preserve values needed after suspension.

## Debug-only statements

Prefix a statement with `debug` to retain it in debug builds and erase it from
release builds:

```text
debug print(`level {current.level}`)
debug if current.level == 7 {
    print("testing the final split")
}
debug attempts += 1
debug let inspectedLevel = current.level
```

Globals and functions can also be debug-only:

```text
debug let traceLimit = 10

debug fn traceLevel(level: i32) {
    debug let capped = level.min(traceLimit)
    print(`level {capped}`)
}

whileAttached {
    debug traceLevel(current.level)
}
```

A debug function may call ordinary or other debug functions. Debug statements
can declare locals with `debug let`, including values produced by `await` or
`retry` in `onAttach`; later debug statements in the same scope can use them.
An ordinary statement or ordinary function may not use a debug-only function,
global, or local because that reference would remain after its declaration is
erased. The compiler reports this at the reference site; wrapping the use in
`debug` establishes the required context. A release module contains neither the
function body nor storage and initialization for a debug-only global.

Debug-only code is still parsed, resolved, and type-checked in release builds,
so a stale diagnostic path cannot silently rot. A release module does not
retain debug-only messages or logging facilities used only by erased code.

The `debug` modifier accepts bindings, expression statements, assignments,
`if`, `while`, `for`, and `await` or `retry` statements. It rejects `return`,
`throw`, `break`, and `continue` because erasing a terminator would change the
surrounding release control flow.

## Inspecting live values

`inspect(expression)` evaluates its operand exactly once, publishes its current
display value in the variables panel under the expression's source spelling,
and returns the same typed value:

```text
let doubled = inspect(current.level) * 2
inspect(current.player)
```

In a debug build this publishes the same value as
`setVariable("current.level", current.level)` while still producing
`current.level` for the surrounding expression. When that value is consumed,
a release build erases only the publication: `inspect(expression)` becomes
`expression`, so the operand still runs exactly once with all of its ordinary
side effects. The operand must implement [`Display`].

A standalone `inspect(expression)` statement is different: because its result
is discarded, the compiler treats the complete statement as debug-only and
does not evaluate `expression` at all in a release build. This is intentionally
convenient for temporary diagnostic calculations, but it also means side
effects inside a standalone inspection disappear. Bind the result or use it in
a surrounding expression when evaluation must remain part of the release
autosplitter. Locals, globals, and helper functions reached only through
standalone inspections receive the same `debug`-modifier guidance as code used
only by an explicit `debug` statement.

Use `inspect(expression)` for temporary, source-labelled developer debugging.
Although it reuses the variables panel, it is not autosplitter output and is
therefore compiled out of release builds. Use [`setVariable`] for meaningful
end-user-facing information displayed on the speedrun timer in both profiles,
and [`print`] for a chronological diagnostic log. Use [`debug`]
around a complete statement when its operand and all of its side effects
should also disappear from release builds.

Blocks are expressions wherever an expression is expected. They may contain
statements and yield their final expression, including inside an `if` branch.
A state-field assignment is a failure boundary, so `?` can propagate a read
error directly out of the selected branch:

```text
levelOrScene = if isDlcDemo {
    LevelOrScene.Scene(GameManager.instance?.scene?)
} else {
    LevelOrScene.Level(
        process.read(gameManager.offset(levelOrSceneOffset))?
    )
}
```

## Optional and fallible values

`T?` represents an optional value and `T!` represents an operation that can
fail with the language's standard string error. `None` constructs an empty
`T?` value and `Err(message)` constructs a failed `T!` value. A plain `T` is
lifted automatically when `T?` or `T!` is expected. `Some(value)` and `Ok(value)` are
available when the wrapper state should be explicit, but are never required.
Because their payload supplies `T`, `Some(value)` and `Ok(value)` can infer a
wrapper type without an annotation. `None` and `Err(message)` still need
expected-type context because they contain no value from which to infer the
missing `T`.

Both forms support structural `==` and `!=` when `T` itself supports
equality. Two absent `T?` values are equal; an absent and present value are not;
two present values compare their payloads. Fallible values first compare
whether they are successes or errors. Successes compare their values, errors compare their
error strings by content, and a success never equals an error. This composes
through structs and enums that contain wrapper fields or payloads.

Standard-library operations that return a value without mutating their receiver
are must-use by default. Writing such a call as a bare expression statement
produces a warning because discarding its only useful outcome is normally a
mistake. Mutating operations such as `Set.insert` remain intentionally
discardable even when they return status information. Individual declarations
can provide a more specific explanation; immutable string transforms do so to
make clear that they return a new string.

`T?` and `T!` additionally carry a must-use obligation on the value
type itself, because absence or failure must not be silently ignored. Using a
must-use value in an assignment, argument, return, match, `else`, or `?`
consumes it. The warning is non-fatal: debug watch and release builds still emit
their Wasm artifact.

Both wrappers can be matched exhaustively with explicit state patterns:

```text
fn describeOptional(value: i32?) -> String {
    return match value {
        None => "none",
        Some(present) if present > 10 => "large",
        Some(present) => present as String
    }
}

fn describeResult(value: i32!) -> String {
    return match value {
        Err(error) => `error: {error}`,
        Ok(success) => success as String
    }
}
```

The binding inside `Some` has type `T`; the binding inside `Ok` has type `T`;
and the binding inside `Err(error)` has type `String`. `_` matches either state without
binding a value. Exhaustiveness checks require both states unless `_` is
present. A guard narrows when an arm applies but does not count as covering its
state, so the guarded `Some` arm above still needs the following unguarded
arm. Payload extraction and guard evaluation happen only after the pattern's
wrapper state matches.

Postfix `?` unwraps a `T!` or propagates its error to the nearest failure
boundary. State-field assignments are implicit boundaries. A function returning
`T!` is also a boundary, so differently typed failures can pass through without
manually rebuilding `Err`:

```text
fn readMode(address: address) -> i32! {
    let object = process.read(address)?
    return process.read(object)?
}
```

Actions such as `whileAttached` are not result boundaries, so an unhandled `?`
there is a compile-time error. Propagation never turns a failed read into a
zero or default value.

An explicit `throw error` expression transfers a `String` error through the
same boundary mechanism. It has type [`Never`], so it can appear wherever an
expression is accepted. It is available in functions returning `T!`, state
fields, and a [`retry`] operand:

```text
fn requirePositive(value: i32) -> i32! {
    if value < 0 {
        throw "expected a positive value"
    }
    return value
}
```

`throw` and the error arm of `?` lower to the same failure-transfer operation.
Explicit nested `catch` boundaries are planned; until then, an action without a
result boundary rejects `throw` just as it rejects `?`.

```text
let selected: String? = None
let discovered: address! = module.address
let failed: address! = Err("module was not found")
```

Use the low-precedence `else` operation to unwrap either type. A value fallback
has the same type as the wrapped value:

```text
let displayName = selected else "Unknown"
let address = discovered else 0 as address
```

The right operand of fallback [`else`] is any ordinary expression; the parser
does not recognize a special list of fallback forms. Since [`return`],
[`break`], [`continue`], and [`throw`] all have type [`Never`], each can satisfy
the wrapped value's expected type by transferring control rather than yielding
locally. Returning is an explicit alternative to `?` and retains an ordinary
`T!` return value rather than using a hidden failure channel:

```text
fn requireAddress(value: address!) -> address! {
    let address = value else return Err("required address is unavailable")
    return address
}
```

Fallbacks can be chained with a final thrown error inside [`retry`]:

```splitscript
let engine = retry {
    let module = process.loadedModule("EngineWin64s.dll")
        else process.loadedModule("EngineWin64sv.dll")
        else throw "engine module is not loaded yet"
    module
}
```

`else` is looser than `||` and all other expression operators and associates
to the right. Consequently, `optional else result else fallback` means
`optional else (result else fallback)`. A bare `None` or `Err(...)` needs an
annotation, argument type, return type, or other expected-type context to
determine its contained `T`.

Like other prefix operators, [`await`] and [`retry`] bind more tightly than
fallback `else`: `retry value else fallback` means `(retry value) else
fallback`. Because the alternate interpretation establishes a materially
different retry boundary, writing these constructs next to each other without
parentheses produces a warning with fixes for both `(retry value) else
fallback` and `retry (value else fallback)`.

## Value blocks

A brace-delimited block in an expression position may perform scoped work and
then yield its final expression. This is useful in [`if`] branches, [`match`]
arms, fallback [`else`] expressions, arguments, and state-field initializers:

```text
let label = if isBoss {
    let kind = "Boss"
    `{kind} level`
} else {
    "Level"
}

print({
    let level = 7
    `Level {level}`
})
```

The final expression has no special keyword. Its value becomes the value of
the entire block, and local bindings disappear when the block ends. A block
without a final expression yields [`None`]. A block which always transfers
control through [`return`], [`break`], [`continue`], [`throw`], or another
[`Never`] expression instead has type [`Never`] and can satisfy any surrounding
result type.

A trailing semicolon after the final expression is accepted, but it does not
discard the value. The compiler warns and offers to remove it, and the
formatter removes it automatically. This keeps a stray semicolon from silently
changing a block's type.

Function, method, lifecycle, and loop bodies are statement blocks rather than
value blocks. They do not implicitly return their final expression. Write
[`return`] explicitly from a function or action; the compiler diagnoses a
Rust-style omitted `return` and offers the insertion when it is unambiguous.

## Casts

Conversions use the Rust- and TypeScript-style `as` operator:

```text
let frame = rawFrame as u32
let seconds = frame as f64 / 60.0
let label = frame as String
```

`as` binds more tightly than arithmetic and comparisons and can be chained,
as in `levelTime as u32 as String`. Numeric casts support every integer width,
`address`, `f32`, and `f64`. Narrowing integer casts retain the low bits;
float-to-integer casts saturate at the destination bounds and convert NaN to
zero, matching Rust's cast behavior. Casting an integer or address to `String`
formats its decimal value. Other reference and domain types are not castable.

## Functions

`///` documentation comments can precede functions and methods, global
variables, state fields, structs and their fields, and enums and their
variants. The language server includes this documentation in hover information
alongside inferred types and function effects. Consecutive non-empty lines form
one paragraph; an empty `///` line starts a new Markdown paragraph.

```text
/// Returns whether the player reached the final stage.
fn isFinalLevel(level) {
    return stage(level) == 7
}

struct Position {
    /// Horizontal position in world units.
    x: f32,
    /// Vertical position in world units.
    y: f32,
}
```

Documentation comments on settings remain their runtime GUI tooltips as
described in the settings section. Ordinary `//` comments are never published
as documentation.

Function parameter and return annotations are optional. Their constraints are
solved together with every function body and call site, including forward
calls. A function with no value-returning `return` is inferred as returning
[`None`]. Explicit annotations can still be added at API boundaries. Function
bodies always use explicit [`return`]; only a nested [value block](#value-blocks)
yields its final expression.

```text
fn isFinalLevel(level) {
    return stage(level) == 7
}

fn stage(level) {
    return (level / 2) + 1
}
```

### Binding patterns

An initialized `let`, function or parenthesized closure parameter, and `for`
binding may destructure one incoming value. The initializer, argument, or
iterator item is evaluated once, and an annotation after the pattern describes
that complete value:

```text
struct Point {
    x: i32,
    y: i32,
}

let { x, y } = Point { x: 1, y: 2 }

fn sum({ x, y }: Point) -> i32 {
    return x + y
}

for { x, y: _ } in points {
    print(x)
}
```

The anonymous `{ x, y }` form is available only when the surrounding context
already supplies one concrete struct type. It is nominal shorthand, not a
structural pattern: `fn usePoint({ x, y }) {}` is ambiguous and needs either
`{ x, y }: Point` or the explicit `Point { x, y }` spelling. The same rule
applies recursively inside wrapper and enum payloads, `match`, and `is`.

A declaration pattern must be irrefutable: every possible value of its type
must match. Bindings and wildcards are irrefutable, and variants whose payload
contains `Never` do not make a possible counterexample. Use `is` or `match`
when another inhabited shape can occur. Uninitialized attachment- and
attempt-scoped globals remain single-name declarations because lifecycle code
assigns their storage later.

Functions are independent of a particular action snapshot. Values from
`current` or `old` are passed explicitly, keeping helpers reusable and making
their dependencies visible. Suspending functions return [`async`] values and
may use [`await`] where their attachment-lifetime effect permits it.

### Callable values and closures

A callable type spells out its parameters and result:

```text
fn apply(value: u32, transform: (u32) -> u32) -> u32 {
    return transform(value)
}
```

A closure creates such a value with `=>`. One parameter may omit parentheses;
zero or multiple parameters use parentheses. The body is any expression, so a
value block provides local statements when needed:

```text
let offset = 2u32
let addOffset = value => value + offset

let counter = 0u32
let increment = () => {
    counter += 1
    return counter
}

print(apply(4, value => value * 2))
print(addOffset(increment()))
```

An explicit result type goes between the parameter list and `=>`. Explicit
results require the parenthesized parameter form:

```text
let widen = (value: u16) -> u32 => value as u32
```

A closure body may suspend. Its callable result is inferred as `async T`, and
calling it creates a typed future just like calling a named async function:

```text
let afterTick = (value: u32) => {
    await nextTick()
    return value + 1
}
print(await afterTick(4))
```

Parameter and result types are inferred from the body, invocation sites, and
an expected callable type in either direction. Write `-> async T` when an
asynchronous closure result is explicit. Creating a closure retains its
lexical captures but does not execute the body. Immutable captures are stored
as values. Mutable locals use one shared cell, so the declaring scope,
returned closures, nested closures, and continuations across [`await`] all see
the same assignments. [`return`] exits the closure itself; [`break`] and
[`continue`] cannot target a loop outside it. Callable values do not implement
[`Equatable`].

### Iterators and synchronous generators

`iterator T` is the common, type-erased cursor type returned by arrays, sets,
ranges, maps, adapters, and generators. Their concrete cursor representations
are standard-library implementation details. This means one function can
forward any iterator with the same item type:

```text
fn values(useRange: bool) -> iterator u32 {
    if useRange {
        return (1..<4).iterator()
    }
    return [1, 2, 3].iterator()
}
```

A function or closure whose `iterator T` body contains `yield` is a lazy
synchronous generator. Calling it creates a cursor without executing the body.
Each `next()` resumes the body until one `yield`, returning `Item(value)`;
reaching the end or a bare `return` permanently returns `End`.

```text
fn values(end: u32) -> iterator u32 {
    let value = 0u32
    while value < end {
        yield value
        value += 1
    }
}

for value in values(3) {
    print(value)
}
```

Generator cursors implement both [`Iterator`] and [`Iterable`], so they compose
with `for`, `map`, `filter`, and generic helpers accepting an iterable. Copying
a cursor creates an alias to the same position; call the generator again for
an independent traversal. A closure can use the same protocol by explicitly
returning `iterator T`:

```text
let offset = 10u32
let values: (u32) -> iterator u32 = (end: u32) -> iterator u32 => {
    let value = 0u32
    while value < end {
        yield value + offset
        value += 1
    }
}
```

This initial protocol is deliberately synchronous. A generator cannot use
[`await`] or [`retry`], return a value, or let an error escape; those operations
need a separately typed asynchronous or fallible iteration protocol rather
than changing what [`IteratorStep`] means.

## Structs

Structs are immutable, named value shapes. Declarations can refer to structs
declared later in the file. Struct
literals are checked for unknown, duplicate, missing, and incorrectly typed
fields; their source order does not matter.

```text
struct Digits {
    minutes: f32,
    seconds: f32,
    hundredths: f32,
}

fn isFresh(value: Digits) -> bool {
    return value.minutes == 0.0 && value.seconds == 0.0
}

let digits = Digits {
    seconds: 0.0,
    hundredths: 0.0,
    minutes: 0.0,
}
```

When a local has the same name as a field, the field name alone is shorthand
for the repeated initializer: `Position { x, y }` means
`Position { x: x, y: y }`. Writing the repeated form produces `SS1010` with a
safe quick fix. The two names still have independent identities: renaming the
struct field expands the shorthand to `horizontal: x`, while renaming the
local expands it to `x: horizontal`.

The same field syntax destructures structs in [`match`](#pattern-matching).
`Position { x, y: 0 }` binds `x`, requires `y` to equal zero, and ignores every
field not named by the pattern. An explicit field accepts any recursive
pattern. A shorthand field represents both the declared struct field and the
new arm binding; hover shows both identities, navigation prefers the struct
field, and rename expands the shorthand when the two names diverge.

Structs may contain other structs and strings, pass through functions, and
remain live across `await`. Immutability keeps
shared metadata bindings predictable; a new value is constructed when a
snapshot needs to change.

A struct whose fields are all fixed-width primitive memory values or other
readable structs also implements the compiler-known `MemoryReadable`
capability. Its process-memory layout follows declaration order with natural
alignment: every field starts at the next multiple of its own alignment, and
the final size is rounded up to the largest field alignment. For example,
`{ tag: u8, count: u32, flags: u16 }` has offsets 0, 4, and 8, alignment 4,
and size 12. Reads are currently little-endian. Explicit offsets, packing, and
endianness controls are intentionally deferred until a real target needs them.

## Pattern matching

Enums model values that can have one of several shapes. Each variant may carry
one typed payload; a struct can be used when a variant needs multiple values.
`match` destructures payloads and is checked for duplicate, foreign, unknown,
and missing variants. The same patterns can be tested inside a boolean
condition with `is`:

```text
fn positive(value: i32?) -> bool {
    if value is Some(number) && number > 0 {
        return true
    }
    return false
}
```

The left operand of `is` is evaluated once. A mismatch produces `false`, so an
`is` expression does not need to be exhaustive. A pattern binding exists only
where short-circuit control flow proves that the pattern matched. This includes
the right operand of `&&`, the `then` branch of `if`, and the body of `while`.
The false edge of `||` and the `else` branch of `!(value is pattern)` likewise
carry a proof that makes the binding available. Storing the boolean in a local
or passing it to a function deliberately does not preserve that proof.

Negate the complete test with parentheses:

```text
if !(value is Some(number)) {
    return 0
} else {
    return number
}
```

`is` has comparison precedence: it binds more tightly than `&&` and `||`, and
comparison chaining is rejected. The pattern syntax and binding rules below
are shared by both `match` and `is`.

```text
enum LevelOrScene {
    Level(i32),
    Scene(String),
}

fn isFirst(value: LevelOrScene) -> bool {
    return match value {
        LevelOrScene.Level(level) => level == 0,
        LevelOrScene.Scene(scene) => scene == "Shrine01",
    }
}
```

Strings, characters, integers, booleans, file versions, and arrays can be
matched with literals. String patterns compare decoded text contents. An exact
array pattern such as `[first, second]` matches precisely two elements and binds
their values. A single `..` rest marker permits any number of elements between
an explicit prefix and suffix: `[first, ..]` matches every nonempty array,
`[.., last]` reads from the end, `[first, .., last]` requires at least two
elements, and `[..]` matches every length. The rest marker itself binds and
copies nothing. Element patterns are recursive, so literals, `_`, wrapper or
enum patterns, nested arrays, bindings, and `left | right` alternatives can be
mixed.
Alternatives are attempted from left to right. They remain one arm, so an `if`
guard applies after any alternative succeeds. Every alternative must bind the
same names with compatible types; repeated occurrences are one logical binding
for hovering, navigation, renaming, the guard, and the arm body. `_` is the
catch-all pattern. Patterns participate in inference, so the parameter types
below are inferred as an integer and `bool` from their uses.

Closed integer intervals use the same explicit endpoint operators as range
values: `start..<end` excludes `end`, while `start..=end` includes it. Both
bounds are integer literals (including negative signed literals) of the matched
integer type. A bare `..` is rejected as an integer interval, so a reader never
needs to remember an implicit inclusion convention; inside an array pattern,
it is the rest marker described above. Range patterns bind more tightly than
alternation, making `0..<10 | 20..=30` the union of two intervals. Empty or
reversed intervals are errors.

```text
fn classify(value: i8) -> String {
    return match value {
        -128..<-10 => "low",
        -10..=10 => "middle",
        11..=127 => "high",
    }
}
```

Range patterns currently test an interval rather than introducing a binding.
When an arm body needs the matched value, match a named local and refer to that
local from the body. Exhaustiveness and unreachable-arm analysis operate on
intervals without expanding them into individual integers. Consequently,
complete finite partitions such as the `i8` example above need no wildcard,
partially overlapping ranges remain reachable for their uncovered portion, and
a range already covered by earlier arms is diagnosed as unreachable.

```text
fn characterName(character, dlcDemo) {
    return match character {
        3 if dlcDemo => "Accel",
        3 => "Erika",
        6 if dlcDemo => "Cres",
        _ => "Unknown",
    }
}
```

Alternatives avoid repeating an arm body and may destructure the same logical
value from different shapes:

```text
enum Side {
    Left(u32),
    Right(u32),
    Idle,
}

fn unwrap(side: Side) -> u32 {
    return match side {
        Side.Left(value) | Side.Right(value) => value,
        Side.Idle => 0,
    }
}
```

Writing `Side.Left(value) | Side.Right(other)` is an error because `value` and
`other` would not both be initialized for the shared arm. The same rule applies
recursively, such as `[Some(value), None] | [None, Some(value)]`.

Wrapper and enum payloads are full patterns rather than binding-only slots.
They may test literals, arrays, nested wrappers, enum variants, or alternatives
while still retaining the familiar binding shorthand:

```text
fn classify(value: String?) -> String {
    return match value {
        Some("Inf") | Some("inf") => "infinity",
        Some("ok") | Some("error") => "status",
        Some(other) => other,
        None => "missing",
    }
}
```

Because `Some("Inf")` matches only that payload, it does not make the `Some`
case exhaustive by itself. Use `Some(_)`, a payload binding, or a complete
finite partition such as `Some(true)` and `Some(false)` to cover every present
value.

Struct patterns name only the fields that matter. Omitted fields are ignored;
there is no separate `..` marker. A bare field is a binding shorthand, while an
explicit `field: pattern` can contain a literal, wildcard, nested struct,
wrapper, array, enum, or alternative. Because `match` already knows the
scrutinee's concrete type, its struct name may be omitted:

```text
struct Point {
    x: i32,
    y: i32,
    label: String,
}

fn horizontal(point: Point) -> i32 {
    return match point {
        { label: "start", x } => x,
        { x: 0 | 1 } => 0,
        _ => -1,
    }
}
```

A struct pattern containing only bindings and wildcards is irrefutable even if
it omits fields, so `{ x } => x` can be the sole arm for a `Point`. The
anonymous spelling remains nominal: the concrete contextual type determines
the struct, never the field names. `Point { x }` remains available when an
explicit type name improves clarity or supplies otherwise-missing context. A literal or other
condition normally needs later coverage, but several arms may jointly cover a
finite field: `Point { flag: true }` and `Point { flag: false }` cover every
`Point`. Unknown and duplicate fields are errors, and every nested pattern is
checked against the declared field type.

String matches are useful for selecting exact host identities while keeping
the dispatch exhaustive. Assign an ordinary attachment-scoped enum global when
the identity selects a persistent memory shape:

```text
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

An unguarded arm counts toward exhaustiveness; a guarded arm may still reject
its pattern. Alternatives contribute the union of their patterns. Enum matches
must cover every variant, boolean matches must cover
`true` and `false`. String, character, and file-version domains are open-ended
and require an unguarded `_` arm. Integer matches require `_` unless their
literal and range patterns cover the complete finite domain. A wildcard can
also make an enum or boolean match exhaustive. Exhaustiveness and
unreachable-arm analysis are recursive: separate arms can jointly partition
wrapper or enum payloads, struct fields, and array elements, while
correlations between several fields or elements remain intact. For example,
the four combinations of two booleans exhaust `[bool; 2]`; only the two equal
pairs do not. A different exact fixed-array element count is a compile-time
error; a rest pattern must instead fit its explicit prefix and suffix within
the fixed length. A growable `[T]` may have any runtime length, so exact array
arms need a fallback, while `[..]` is exhaustive and `[]` together with
`[_, ..]` covers every length.

```text
fn decodeHeader(bytes: [u8]) -> u8? {
    return match bytes {
        [0x53, .., value, 0] => value,
        _ => None,
    }
}

fn addPair(values: [u32?; 2]) -> u32 {
    return match values {
        [Some(first), Some(second)] => first + second,
        _ => 0,
    }
}
```

Enums are immutable values and can be nested in structs, passed through
functions, and retained across `await`. This directly models the original
Lunistice autosplitter's base-game level number versus DLC-demo scene name.

Enums support `==` and `!=`. Equality is structural: variants must match, and
payload variants then compare their active payloads. Structs are structural as
well, comparing fields in declaration order. The capability is derived
recursively, so an enum carrying numbers, strings, arrays, other equatable
enums, or equatable structs needs no declaration or implementation boilerplate.
A comparison is rejected at compile time with the field or variant path when a
contained type does not support equality.

## Methods

A function can belong to a type by qualifying its name. The receiver is
available as the implicit, statically typed `self` parameter.

```text
fn LevelOrScene.isFirst() -> bool {
    return match self {
        LevelOrScene.Level(level) => level == 0,
        LevelOrScene.Scene(scene) => scene == "Shrine01"
    }
}

if game.location.isFirst() {
    print("First level")
}
```

Methods can have additional typed parameters and may be invoked through nested
struct paths. Method syntax provides type-directed organization and an implicit
receiver; calls otherwise behave like global helpers.

## Arrays

Arrays are mutable values. `[T]` omits an exact length and newly constructed
values of that type are growable. `[T; N]` records an exact compile-time
element count. An omitted shape in a local or function annotation is inferred:
it does not erase an exact shape already known for the value. Consequently a
read-only helper taking `[T]` accepts both forms, while a helper that calls
`push`, `pop`, `extend`, `remove`, `removeAt`, or `clear` is inferred to require
a growable array. Passing `[T; N]` to that helper is rejected at the call rather
than silently invalidating the exact-length type.

The same shape relationship is preserved through aliases, forwarded calls,
closures, and return values. For example, an identity helper returning its
`[T]` parameter returns `[T; N]` when called with `[T; N]`; the result cannot
then be structurally mutated. A general growable `[T]` cannot be narrowed to a
particular length without proof.

```text
fn first(values: [u8]) -> u8 {
    return values[0] // accepts `[u8]` and `[u8; N]`
}

fn append(values: [u8], value: u8) {
    values.push(value) // `values` must be growable
}
```

Non-empty literals infer their element type. An expected `[T; N]` also checks
the literal's exact element count. An empty literal keeps an unresolved element
type that later uses can constrain: `push`, indexing, assignment, return types,
and function arguments all participate. Add an annotation only when the empty
array remains genuinely unconstrained.

```text
let bytes: [u8] = [0x48, 0x00, 0x01]
let header: [u8; 3] = [0x48, 0x00, 0x01]
let inferred = [1, 2, 3] // [i32]
let empty: [u16] = []
let discovered = []
discovered.push(0x8bu8) // discovered is [u8]
discovered.clear()

bytes[1] = 0x8b
bytes[2] += 1
let opcode = bytes[1]
let count = bytes.length()
let hasTerminator = bytes.contains(0)
let marker = bytes.indexOf(0x8b) // u32?
```

`length()` returns `u32`, `array[index]` returns `T`, and
`array[index] = value` replaces the selected element. The assignment is
resolved through the standard library's array-mutation capability: the array
and index are each evaluated once, aliases observe the replacement, both `[T]`
and `[T; N]` support it, and out-of-range access traps. Replacement does not
change collection structure, so it does not invalidate active iteration.
Compound indexed assignments use the same catalog-defined operators as local
and global assignments. The collection, index, and right operand are evaluated
once in that order; compiler temporaries preserve them across `await` and
`retry`. `contains(value)` tests every element from the beginning, while
`indexOf(value)` returns the first matching `u32` index or `None`. These search
methods are available when the element type supports `Equatable`; they are
ordinary source-defined library loops rather than dedicated compiler
operations. Arrays can contain structs, enums, strings, and other arrays, and
can themselves be stored in structs or retained across suspension.

Both `[T]` and `[T; N]` support `==` and `!=` when `T` implements
[`Equatable`]. Equality is structural: lengths must match and elements are
compared in order with their own equality implementation. Arrays may also be
destructured by exact [`match`] patterns, as described in
[Pattern matching](#pattern-matching).

Only growable `[T]` arrays provide `push(value)`, `extend(values)`,
`removeAt(index)`, `remove(value)`, `pop()`, and `clear()`. Extension appends a
typed array in order and captures its source length first, so
`values.extend(values)` duplicates the original elements once. `removeAt`
shifts later elements left and traps when the index is outside the logical
length, consistently with indexed access. Equality-constrained `remove`
removes only the first equal element and reports whether one was found; an
absent value leaves the array unchanged. `pop` returns the final element as
`T?`; an empty array returns `None` without mutating, while a present element
is removed in place. Clearing keeps the array object and its backing capacity,
so aliases still observe the same array and later growth can reuse its storage.
Removed, popped, and cleared reference elements are released rather than
retained in unused capacity. Successful length changes are structural
mutations and invalidate an active traversal; exact `[T; N]` arrays provide
none of these operations because their length is part of their type.

Arrays can be traversed directly without manually managing an index:

```text
for byte in header {
    print(byte)
}
```

A non-empty `[T; N]` is `MemoryReadable` when `T` has a fixed readable layout.
`process.read` and `state ... at` then fetch the complete `N * stride(T)` byte
range once and only publish the newly constructed array if that read succeeds.
This makes indexed flags and inventories transactional rather than a collection
of independently failing state fields. Reads are currently limited to 4096
elements and 65536 bytes to bound generated code and host-memory traffic.

Nested arrays can combine both forms, for example `[[u8; 4]; 2]`. Wrapper
postfixes apply to the complete array type, so `[T; N]?` is an optional sized
array and `[T!; N]` is a sized array of fallible values.

## Sets

`Set<T>` is a growable collection of unique values. `T` must implement
`Equatable`; this constraint is declared by the standard library and applies
both to inferred construction and explicit `Set<T>` annotations. `Set.new()`
keeps its element type unresolved until a later insertion, lookup, assignment,
return, or argument constrains it. An explicit type remains useful when there
is no such use:

```text
let visited = Set.new()

whileAttached {
    if visited.insert(current.roomName) {
        print(`First visit to {current.roomName}`)
    }
}

let pending: Set<String> = Set.new()
```

A global set is constructed once for the loaded script instance and retains its
contents across ticks and attachments until it is explicitly changed or the
instance is unloaded. Constructing one inside a polling block instead creates a
fresh empty set each time that expression runs.

`insert(value)` returns whether the value was new. `remove(value)` returns
whether a value was present, `contains(value)` performs a linear equality
search, `length()` returns `u32`, `isEmpty()` reports whether the length is zero,
and `clear()` removes all values. The set object remains stable across mutation;
its backing storage allocates at construction, when insertion grows capacity,
and when clearing releases stored references.

Sets can be traversed directly:

```text
for room in visited {
    print(room)
}
```

Set iteration order is not a language guarantee. Structurally mutating the set
while traversing it invalidates the loop using the same fail-fast rule as
growable arrays. Compute pending additions or removals separately and apply
them after the loop.

## Integer ranges

Integer ranges are first-class immutable values. The operator always states
whether the upper endpoint participates: [`start..<end`](syntax@range) is
exclusive and [`start..=end`](syntax@range) is inclusive. Bare `start..end` is deliberately rejected, with
fixes for both spellings, so a reader never has to remember another language's
range convention.

```splitscript
for index in 0u32..<count {
    inspect(index)
}

let checkpoints: u8..=u8 = 1..=3
visit(checkpoints)
```

Both endpoints must have one exact integer type. The corresponding type syntax
repeats that type around the same operator: [`T..<T`](syntax@range) or
[`T..=T`](syntax@range). Inference flows
through either endpoint, annotations, arguments, returns, and loop bindings.
The immutable [`start`](field@T..<T.start) and
[`end`](field@T..<T.end) fields expose those endpoints without
changing their integer type. [`contains`](method@ExclusiveRange.contains)
tests membership using the range's endpoint policy, while
[`isEmpty`](method@ExclusiveRange.isEmpty) recognizes equal exclusive
bounds and reversed ranges. Inclusive ranges provide the corresponding
[`start`](field@T..=T.start),
[`end`](field@T..=T.end),
[`contains`](method@InclusiveRange.contains), and
[`isEmpty`](method@InclusiveRange.isEmpty) members.

```splitscript
let checkpoints = 2u16..<5
print(`from {checkpoints.start} to {checkpoints.end}`)
if checkpoints.contains(4) {
    print("checkpoint 4 is active")
}
```

An ascending range whose end precedes its start is empty; an exclusive range
also excludes equal endpoints. Direct `for` iteration does not allocate a range
object, while a stored or passed range is an immutable value. Inclusive iteration records
completion separately from its current integer, so ending at an integer type's
maximum value terminates without wrapping.

`for` evaluates its iterable exactly once. A suspending body preserves the
range, current bound, and endpoint policy across ticks, and `continue` cannot
repeat the current element. Descending ranges, custom step sizes, and
open-ended ranges are intentionally deferred until concrete autosplitter ports
establish their semantics.

## Settings

```text
enum CaptureMode {
    WindowTitle,
    ExecutableName,
    FullPath,
}

settings {
    /// Options used during normal operation.
    "General" {
        /// Can be changed while the splitter is running.
        "Enable Auto Splitting"
            => enableAutoSplitting key "auto-splitting": true,

        /// Free-form name used by the autosplitter.
        "Profile Name" => profileName: "Player",

        /// Chooses how the target application is identified.
        "Capture Source"
            => captureMode: choice {
                "Window Title" => CaptureMode.WindowTitle,
                "Executable Name" => CaptureMode.ExecutableName default,
                "Full Path" => CaptureMode.FullPath,
            },

        /// Files used by the autosplitter.
        "Files" {
            /// Accepts image and JSON layout files.
            "Layout File" => layoutFile: file {
                "Images" => "*.png *.jpg",
                _ => "*.json",
                mime => "image/*",
            },
        },
    },
}
```

Quoted blocks create settings titles. Nesting determines their heading level.
Consecutive `///` documentation comments immediately before a title or setting
become its tooltip. Lines in the same paragraph are joined with spaces; empty
`///` lines preserve paragraph breaks. Ordinary `//` comments remain regular
comments and do not become GUI text.

A boolean setting infers its type from `true` or `false`. A quoted string
default declares a free-form text-input setting and exposes its live value as a
`String`; an existing host value takes precedence over the declared default. A
`choice` is backed by a payloadless enum, so matching it is exhaustive and type
checked. A `file` setting is also a `String` and can declare named glob filters,
an unnamed fallback filter, and MIME filters. A selected file is stored as an
absolute path in the runtime's portable filesystem namespace and can be passed
directly to `File.readAllBytes` or `File.readAllText`. The host filesystem is
currently mounted read-only below `/mnt`: Windows `C:\foo\bar.txt` becomes
`/mnt/c/foo/bar.txt`, while Linux or macOS `/foo/bar.txt` becomes
`/mnt/foo/bar.txt`.

The optional `key "host-key"` clause gives a setting the exact string key used
in the host settings map. Without it, the source identifier is also the host
key. Script code always uses the readable declaration name, such as
`settings.enableAutoSplitting`; the external key is metadata for persistence
and data-driven lookup. Keys must be nonempty and unique across the complete
settings block.

Large finite boolean families use a compile-time `for` declaration instead of
hand-written members or mutable runtime registration:

```text
settings {
    "Levels" {
        /// Splits when the player reaches this level.
        for level in 2..=36 {
            `Level {level}` key `{level}`: true,
        },
    },
}
```

The bounds are inclusive non-negative `u32` constants, and one family may
produce at most 4096 settings. Label and key templates may interpolate only
the family binding. The compiler expands the range into ordinary boolean
settings, so generated entries use the same registration, stable-key,
snapshot, default, tooltip, and validation paths as explicit declarations.
They intentionally do not invent statically named members; use
`settings.enabled(key)` or `oldSettings.enabled(key)` for lookup. A `///`
comment before `for` becomes every generated setting's tooltip. Quoted groups
remain visual headings and do not implicitly gate child values.

`settings.enabled(key)` performs allocation-free data-driven lookup over the
declared boolean settings, using those same host-map strings. `oldSettings`
provides the corresponding method for the preceding snapshot. A literal key is
validated against the declarations and editor completion offers only compatible
boolean keys. For a computed string, an unknown key—or one belonging to a text,
choice, or file setting—returns `false`; the API therefore does not erase
heterogeneous setting values into a dynamic type. Use `settings.contains(key)`
when data-driven code must distinguish an unknown key from a declared but
disabled setting. Its literal keys are checked and completed against boolean,
text, choice, and file declarations; visual headings are not values.

Controls are registered during `_start`. At the beginning of every exported
tick, including detached ticks, the compiler loads the current host settings
map. User code sees the freshly decoded values through `settings`; the values
from the preceding tick are available through `oldSettings` for change
detection. Missing host entries use their declared defaults (or an empty string
for a file setting). Misspelled settings and invalid choice variants are
compile-time errors. The quoted text before `=>` is the setting's visible
label; `///` documentation comments are the only way to define its tooltip.

## Actions

`setup` runs once for each loaded script instance at the beginning of its
first host update, after global values and the settings UI have been initialized
and the current settings have been loaded. It is intended for
process-independent startup work such as printing a startup message:

```text
setup {
    print("Autosplitter loaded")
}
```

`setup` cannot access `process`, `gba`, `current`, `old`, or suspend. The body
is emitted in `_start`; the autosplitting runtime deliberately defers that export
until the first controlled update so arbitrary startup code can be interrupted.
A debug watch reload creates a new module instance, so its `setup` block runs
again on that instance's first update.
This is deliberately distinct from `onAttach`, which runs once for every
selected process and may suspend while performing discovery.

When multiple running processes can have the same configured name,
`selectProcess` examines each candidate before provider discovery and
`onAttach`:

```text
selectProcess {
    let path = process.path()?
    return path.endsWith("/wanted/game.exe")
}
```

The candidate is available through the ordinary native `process` value even
when the state provider is Unity or an emulator. Return `true` to promote that
candidate to the script's one attachment lifetime, or `false` to detach it and
try another. Falling through is `false`. This block is an implicit error
boundary, so postfix `?` and `throw` also reject only the current candidate;
they do not abort the update or prevent another PID from being tried. `None`
remains an ordinary optional value and is not a selection result. The block is
synchronous and runs before provider roots, attachment-scoped
globals, `current`, or `old` exist. Candidate ordering is unspecified, so the
predicate should identify the desired process from stable process evidence.
Scripts without this block retain the host's direct name-attachment path and
pay no PID-enumeration cost.

`onDetach` runs exactly once when a successfully initialized state-provider
attachment ends. This normally happens when its process closes, but an emulator
provider also detaches when its game mapping disappears while the emulator
remains open. It does not run during initial detached startup, repeat on
detached ticks, or run for an attachment rejected before `onAttach` completed:

```text
onDetach {
    timer.pauseGameTime()
}
```

The ended attachment's provider state and pending continuations are cleared
before the block runs; a closed process handle is cleared as well, while an
emulator host handle may be retained privately for rediscovery. `process`,
`gba`, `current`, and `old` are unavailable across this lifecycle boundary.

`onStart` and `onReset` react to timer transitions independently of process
attachment:

```text
let elapsed
let leftFirstLevel

onStart {
    elapsed = 0.0
    leftFirstLevel = false
}

onReset {
    print(`Finished with {elapsed} seconds`)
}
```

A bare global assigned by `onStart` is attempt-scoped. It needs no dummy
initializer, remains available if the game process detaches during the attempt,
and is released after `onReset` completes. This keeps attempt state as an
ordinary global rather than introducing a second attempt declaration syntax.
Attempt-scoped values are also available from `split`, `reset`, `isLoading`,
and `gameTime`.

After refreshing settings near the beginning of every update, the generated
monitor samples the timer state. The first sample establishes a baseline
without firing. A later transition from `TimerState.NotRunning` to any active
state runs `onStart`; a transition back to `TimerState.NotRunning` runs
`onReset`. `onReset` can inspect attempt-scoped values before they are cleared.
This happens before attachment and state polling, so both blocks run
while detached as well. They may use settings and ordinary globals, but cannot
use `process`, an emulator provider, attachment-scoped globals,
`current`, or `old`. They do not suspend and do not return a value.

The decision actions do not invoke these blocks directly. If `start` or `reset`
requests a timer mutation, the corresponding transition is sampled once on the
following update. A transition that persists across an update is therefore
delivered exactly once; multiple opposing timer changes entirely between two
updates cannot be reconstructed without a future host event queue.

The first observed timer state is only a baseline. Loading a script while the
timer is already active therefore does not invent an `onStart` event, and
attempt-dependent decision actions remain inactive until a later observed
start. Autosplitters are normally loaded before an attempt begins; scripts that
must reconstruct an already-running attempt should use explicitly initialized
module state instead.

`setTickRate(hz)` uses updates per second. The runtime reads the
resulting interval after the current `update` returns, so a call affects the
wait before the following update rather than the invocation already in
progress. It is the dynamic escape hatch from the top-level `tickRate` policy:
the selected value persists until another call or until the next attachment
state transition reapplies the declared/default lifecycle rate.

`onStateReady` runs synchronously once per attachment, immediately after the
first complete state snapshot is committed:

```text
onStateReady {
    print(`Initial level: {current.level}`)
}
```

Both `old` and `current` are available and equal in this block. This prevents
the initial read from looking like a transition from zero/default storage.
The attached process is also available, but `await` and `retry` are not.
`whileAttached` and the timer-decision actions begin on the following update.

```text
start {
    return current.level == 1;
}

split {
    return current.level != old.level;
}

reset {
    return current.level == 0;
}

isLoading {
    return current.loading;
}

gameTime {
    return Duration.fromFrames(current.frames, 60);
}
```

Every action may fall through or use a bare `return`; the runtime supplies its
domain default:

| Action | Fallthrough result | Runtime meaning |
| --- | --- | --- |
| `selectProcess` | `false` | Reject this same-name process candidate |
| `start`, `split`, `reset` | `false` | Do not perform the timer action |
| `isLoading` | `None` | Leave the current game-time pause state unchanged |
| `gameTime` | `None` | Do not set a new game time |

`None` is an explicit return value only for `isLoading` and `gameTime`, where it
represents a real third state. It is deliberately rejected in `start`, `split`,
and `reset`; those blocks are simply boolean and default to `false`.
`gameTime` otherwise returns a `Duration`. Source-defined constructors include
`Duration.zero()`, exact integer unit and nanosecond constructors,
`Duration.fromFrames<T: Integer>(T, T)`, and `Duration.fromParts(i64, i32)`.
`Duration.fromMilliseconds`, `fromSeconds`, `fromMinutes`, `fromHours`,
`fromDays`, and `fromNanoseconds` accept every integer and floating-point type.
Integer inputs stay exact without passing through floating point. A day is
always exactly 86,400 seconds; these are elapsed durations, not calendar values. `setup`,
`onDetach`, `onStart`, `onReset`, and `onStateReady` return nothing.
`whileAttached` runs before timer actions on every initialized attached tick.
It may explicitly return a boolean control result: `false` skips all timer
decisions for the current update, while `true`, a bare `return`, and fallthrough
continue normally. State has already refreshed and is not rolled back.

`current.field = value` explicitly replaces one field after that refresh;
compound forms such as `current.count += 1` use the field's normal typed
operator. The replacement is visible to later statements and later actions in
the same tick, then becomes part of `old` on the next successful poll. `old` is
always read-only. Prefer a trailing state-field `if` when an invalid candidate
must be rejected before the first complete snapshot can be published.

## Discovered state and watchers

An [`at`](syntax@at) field retains the compact static pointer-path syntax. A state field may
instead use a typed expression, allowing `onAttach` to discover Unity roots and
field offsets once and the state DSL to read them every tick:

```text
let manager = 0
let pointsOffset = 0

state "game.exe" {
    points: i32 = process.read(manager.offset(pointsOffset));
}
```

Every state expression has one implicit `T!` failure boundary: a plain value is
lifted automatically, while a process read already returns a result. Postfix `?`
inside the expression propagates into that same boundary, including when the
final operation is itself fallible:

```text
state "game.exe" {
    score: i32 = process.read(process.follow(base, offsets)?);
}
```

There is no nested result and no helper-function boundary in this form. A
failure from either `follow` or `read` rejects this field's candidate value. The
first snapshot requires all required fields to succeed together and initializes
`old` and `current` to the same value. On later polls, an error keeps that
field's accepted value while successful fields advance. Put values that must
advance atomically into one struct- or array-valued state field; the whole
aggregate is then one acceptance unit.

When one field is genuinely optional, make its value type optional and convert
that read's error with the source-defined `discardError()` method:

```text
state "game.exe" {
    level: u32 = process.read(levelAddress);
    bonus: u32? = process.read<u32>(bonusAddress).discardError();
}
```

A failed `level` read retains its last accepted value after initialization. A
failed `bonus` read is instead accepted as `current.bonus == None`; a later
successful read becomes a present `u32` without an explicit `Some` constructor.
The error text is deliberately discarded, so `discardError()` should not be used
merely to silence an unexpected failure.

Declare an attach-time discovery value as an attachment-scoped global by
omitting its initializer. State polling is gated until `onAttach` completes,
and the compiler proves that the value is assigned on every successful path
where it can be read. This avoids exposing a fake zero or `None` sentinel:

```text
let levelAddress

state "game.exe" {
    level: u32 = process.read(levelAddress)?;
}

onAttach {
    levelAddress = (await process.mainModule()).address
}
```

Unsupported builds should remain suspended with `await process.closed()`
rather than complete without initializing a required attachment value.

Pointer width belongs to the attached executable, not to the state-field
declaration or the host running SplitScript. Native reads detect PE, ELF, and
Mach-O pointer width from the executable image, including 32-bit targets on a
64-bit host:

```text
state "game.exe" {
    loading: bool = process.read<bool>(
        executableBase.memoryPath([0x00480af0], 0).resolve()?
    )?;
}
```

This is why there is no separate `at32` spelling: static [`at`](syntax@at)
paths, `process.follow`, and `MemoryPath.resolve` all use the attached
executable's detected native width. The maintained Borderlands PE32 layout has
a host-executed fixture for this form.

## Structured async initialization

`onAttach` is inherently suspending; it does not need an `async` modifier.

```text
onAttach {
    let expectedVersion: u32 = 1
    let gameAssembly = await process.module("GameAssembly.dll")
    let marker = await gameAssembly.scan(sig"48 8B ?? B? 00")
    if expectedVersion == 1 && marker != 0 {
        print("Initialization finished")
    }
}
```

Suspending operations are polled once per runtime tick until they complete.
State reads and timer actions remain gated while initialization is pending. If
the state provider detaches at any suspension point, the generated
attachment-lifetime scope cancels the initializer, resets it, and starts fresh
with the next attachment. This includes an emulator game mapping disappearing
without its host process closing. This is the language-level counterpart to ASR's
`until_closes`, without requiring scripts to manually write the outer
attach/cancellation loops.

`onAttach` is also an implicit ordinary error boundary. Postfix [`?`] and
[`throw`] reject the acquired process instead of aborting the update. The
runtime retains that rejected process handle without polling state or invoking
`onAttach` again until the process closes; this prevents the discovery loop
from selecting the same unsupported live instance every tick. Because the
attachment never completed, its closure does not invoke [`onDetach`]. A later
process instance starts with a fresh initializer and cleared attachment state.

[`retry`] is the dual of [`await`]. [`await`] polls one already-asynchronous
value without evaluating its source expression again. `retry expression`
instead places a local failure boundary around synchronous work, evaluates the
complete operand once per attached update, and starts that operand from the
beginning after a failure. A direct [`T!`] error, postfix [`?`], or [`throw`]
reaching the boundary keeps the retry pending. Success yields `T`.

The operand can be any ordinary expression. In particular, `retry { ... }`
does not use a special retry-block grammar: braces are the same value-producing
block expression accepted in arguments, branches, casts, and other expression
positions. This lets one attempt contain several dependent reads while keeping
one local failure boundary:

```text
fn readMarker() {
    return process.read<i32>(0x3000)
}

onAttach {
    let marker = retry readMarker()
    let playerHealth = retry {
        let player = process.follow(marker as address, [0x10, 0x20])?
        process.read<i32>(player)?
    }
    print(`marker {marker}, health {playerHealth}`)
}
```

Every statement in the value block is evaluated again on the next attempt;
locals from a failed attempt are not retained. The final successful expression
is lifted into the retry result, so it can be a plain `T` after earlier `?`
operations. [`return`] still exits the enclosing function, while [`break`] and
[`continue`] still target the nearest lexical loop. [`throw`] is different only
because the retry boundary deliberately catches errors.

One attempt must be synchronous and bounded so an attached update cannot hang.
Evaluating [`await`] or another [`retry`] anywhere inside the operand is an
error. Calling an async function is allowed because that synchronously creates
an `async T` value; polling it with [`await`] is what is prohibited. Put
intrinsically asynchronous discovery outside the retry and await it normally.
Process closure cancels the whole retry boundary.

`await nextTick()` is the basic scheduling primitive. It always suspends once
and resumes on the following attached-process update. It is useful when a game
needs one frame to publish metadata after another awaited discovery:

```text
let gameAssembly = await process.module("GameAssembly.dll")
await nextTick()
let marker = await gameAssembly.scan(sig"48 8B ?? ??")
```

Like every `onAttach` suspension, a pending next-tick continuation is discarded
if the process closes.

Source-defined helpers use `async T` as a real future type. The annotation is
required only when the return type is written explicitly; otherwise both the
future and its completion type are inferred:

```text
fn afterTick(value) {
    await nextTick()
    return value
}

onAttach {
    let pending = afterTick(42)
    print("future created")
    let value = await pending
    print(value)
}
```

Calling `afterTick` evaluates and captures its arguments once; it does not begin
polling the body.
Intrinsically asynchronous operations behave the same way, so
`let pending = process.module("game.dll")` creates a future that may be passed
or stored before `await pending`. An `async T` value can be held in a local,
struct, enum, option, result, or array and passed as a parameter. Once complete,
the future retains `T`, so another await returns the same value without rerunning the
operation. Merely creating a future is synchronous. Futures are owned by the
state-provider attachment lifetime and therefore cannot be stored in globals.

`await` is an ordinary prefix expression rather than a declaration form. It
can appear inside member access, arguments, arithmetic, interpolation,
conditional and match arms, guards, fallbacks, and loop conditions. Earlier
operands are evaluated once, and only the selected branch is polled, preserving
source evaluation order across ticks.

`onAttach` supports the same variables, assignments, expressions, calls, and
conditional control flow as other action blocks, including suspensions nested
in `if`, `else if`, and `else` branches. Local values needed later survive the
suspension. Conditions and side effects before a pending operation are not
replayed on the next tick; success continues through the selected branch and
then rejoins statements after it.

`process.module(name)` produces a `Module` with `address: address` and
`size: u64`. A signature literal is distinct from
`String`: its nibbles are checked and converted to needle/mask bytes at compile
time.

```text
let code = await process.module("GameAssembly.dll")
let matchAddress = await code.scan(sig"48 8B ?? ?? B? 00")
```

`?` may replace either nibble, so `??` matches any byte and `B?` matches any
byte whose high nibble is `B`. Module scans read overlapping 4 KiB chunks and
therefore find patterns crossing page boundaries. A missing pattern suspends
and retries on the next tick; process closure cancels the whole initializer.

Windows executable versions have their own checked `FileVersion` value and
`v"major.minor.build.private"` literal. The literal requires exactly four
decimal components, each within the `u16` range. It is therefore safe to use in
typed build selection without parsing host-formatted version strings. Version
literals may also be used directly as [`match`] patterns. A wildcard is
required because new executable versions may exist beyond the versions listed
by the script.

```text
enum Build {
    V1000,
    V1500,
}

let build: Build

onAttach {
    let executable = await process.mainModule()
    let version = executable.fileVersion() else v"0.0.0.0"
    if version == v"1.5.0.0" {
        build = Build.V1500
        return
    }

    await process.closed()
}
```

```text
build = match version {
    v"1.0.0.0" => Build.V1000,
    v"1.5.0.0" => Build.V1500,
    _ => await process.closed(),
}
```

The quotes deliberately bound the complete structured literal, as they do for
`sig"..."`, so an omitted or malformed component receives one focused parser
diagnostic rather than being interpreted as unrelated numeric/member syntax.

## Typed process memory

`process.read(address)` infers its exact memory representation from an
annotation, state-field usage, argument, fallback, or other surrounding type
constraint. A synchronous read returns `T!`, so failure must be handled with
`else`, propagated, or passed directly to a state field. Retrying the same call
re-evaluates it on later ticks and yields `T` directly. `retry` responds to the
`T!` state only; it does not invent a separate sentinel value for failure.

```text
let mode: i32 = retry process.read(object + 0x10)
let elapsed: f32 = process.read(object + 0x18) else 0.0
let next: address = retry process.read(object + 0x20)
```

Named `MemoryReadable` structs use the same call. The runtime performs one host
read for the complete naturally aligned struct, then constructs its immutable
value. Nested readable structs are decoded recursively,
so a state snapshot cannot observe a torn mixture of individually read fields.

```text
struct LevelTimeParts {
    minutes: f32,
    seconds: f32,
    hundredths: f32,
}

state "game.exe" {
    levelTimeParts: LevelTimeParts = process.read(timer.offset(levelTimeOffset));
}
```

Native strings are read as bounded decoding operations rather than synthetic
types such as `string32`. `process.readUtf8(address, maxBytes)` reads at most
4096 bytes in one host call, stops at the first NUL byte (or at the bound), and
returns `String!`. An inaccessible range, a zero or excessive bound, or invalid
UTF-8 is an ordinary error. `process.readUtf16Le(address, maxUtf16Units)` reads
at most 2048 little-endian UTF-16 code units, also stops at NUL or the bound,
and replaces malformed surrogate sequences with the Unicode replacement
character. Unity managed strings instead use schema fields such as
`String scene;`; their generated readers understand the managed
object layout and expose an ordinary `String!` field hop.

Pointer-backed state fields have compact sugar for the same operation. The
decoder applies after the complete module-relative pointer path has been
resolved, and it infers the field as `String`:

```text
state "game.exe" {
    mapName at "game.dll", 0x1234, 0x20 as utf8(64);
    chapterName at "game.dll", 0x2345, 0x18 as utf16le(64);
}
```

The bound describes a read operation, not the resulting value's type. All
decoded values are ordinary `String` values; there are no `string64`-style
pseudo-types. `utf8` is strict because invalid bytes cannot form a language
string. `utf16le` deliberately uses replacement decoding, matching the native
ASL UTF-16 behavior while naming the byte order explicitly.

When no context determines the representation, add an annotation or use an
explicit type argument such as `process.read<u8>(address)`. Any
`MemoryReadable` type can be written there, including named structs and
fixed-length arrays. An ambiguous generic read produces a diagnostic showing
both fixes. `address` is nominal
rather than an alias for `u64`, preventing a module size or counter from being
passed where a target pointer is required.

`Module` values retain the identity used to discover them. In addition to
`address` and `size`, `module.path()` returns the runtime's absolute portable
filesystem path as `String!`. Windows paths map below `/mnt/<drive>`; Linux and
macOS paths gain the `/mnt` prefix. The operation is fallible because the host
may not expose a path. A returned path can be passed directly to
`File.readAllBytes` or `File.readAllText`.

```text
let executable = await process.mainModule()
let executablePath = executable.path() else "Unavailable"
```

For Windows PE modules, `module.fileVersion()` and `module.productVersion()`
parse the bounded numeric `VS_FIXEDFILEINFO` resource directly from process
memory. They return equatable `FileVersion` structs with `major`, `minor`,
`build`, and `privatePart` fields. This keeps version selection typed instead
of relying on legacy version strings with inconsistent separators. When both
identities are needed, `module.versionInfo()` returns them from one resource
lookup as the `file` and `product` fields of `ModuleVersionInfo`.

```text
let version = executable.fileVersion() else return
if version.major == 1 && version.minor == 2 {
    print("recognized executable version")
}

let product = executable.productVersion() else return
print(`product {product}`)
```

Generic calls put type arguments directly after the callable name:

```text
let header = process.read<Header>(address) else return
let bytes = process.read<[u8; 16]>(address) else return
```

There is no Rust-style `::` turbofish. The opening `<` must touch the callable
name. This keeps `value < limit` an ordinary comparison while making
`read<u32>(address)` unambiguous; the formatter removes any remaining spaces
around the generic call delimiters.

`process.follow(base, offsets)` accepts `[i64]` and reads a non-null 64-bit
pointer at every successive `current + offset` location. This uses the same
wrapping signed-displacement arithmetic as static [`at`](syntax@at) paths and
`MemoryPath`. `process.readRelative32(location)`
decodes the common x86-64 RIP-relative form as `location + 4 + i32(location)`.
Both return `address!`. Use `else` or `?` for a one-shot attempt, or `retry` in
`onAttach` to poll until they succeed.

```text
let object = retry process.follow(module.address, [0x10, -0x28])
let target = retry process.readRelative32(instruction + 0x3)
let found = await process.scan(target, 0x200, sig"48 8B ?? ??")
```

Use `address.offset(displacement)` when the displacement is signed and
`address.add(delta)` when an unsigned full-width `u64` delta is already
available. Both preserve the nominal `address` type and wrap modulo 2^64.
`address.memoryPath(dereferences, finalOffset)` stores signed `i64` dereference
and final offsets and resolves them with `offset`. Pointer reads automatically
use the attached executable's native pointer width.

`print` is a regular typed builtin available in every action block and writes
through the runtime debug-message API. Its argument is any `String` expression,
not only a literal. Strings use content equality with `==` and `!=`, and
`value.byteLength()` returns a string's UTF-8 byte length. A message after the final await in
`onAttach` therefore prints once per successful process attachment, while a
message in `whileAttached` prints every attached tick.

The immutable `String` API uses explicit UTF-8 byte semantics where indexing
is involved:

Use the generated [`String`] page for the exact member set, signatures, effects,
and examples. Choose operations by unit and task:

- For UTF-8 storage size and raw offsets, use [`String.byteLength`],
  [`String.byteAt`], [`String.indexOf`], and [`String.slice`].
- For Unicode scalar access and scalar-count padding, use [`String.charAt`],
  [`String.padStart`], and [`String.padEnd`]. These are not terminal-column
  measurement APIs.
- For exact search and transformation, use [`String.contains`],
  [`String.startsWith`], [`String.endsWith`], [`String.replaceAll`], and
  [`String.split`]. ASCII-only case and trimming operations say so in their
  symbol names and documentation.
- For construction and conversion, use [`String.concat`], [`String.join`],
  interpolation, [`Display`], or [`String.parse`] according to the source value.

Case conversion reuses an already-normalized immutable string and allocates
only when at least one ASCII letter changes. The operations intentionally have
ASCII-specific names: full Unicode case conversion can change byte length and
requires a separate, explicitly specified API.

All string offsets are UTF-8 byte offsets. `byteAt` is appropriate for binary
inspection and accepts every in-range byte. `charAt` decodes the complete
Unicode scalar beginning at an offset and returns it as a `char`; it fails when
the offset points into a multibyte sequence. It deliberately does not use
JavaScript's or C#'s UTF-16 code-unit indexing:

```splitscript
let separator = "map_01".charAt(3) else return
let sharpS = "Straße".charAt(4) else return
return separator == '_' && sharpS == 'ß'
```

`text.parse<T>() -> T!` parses the complete string as a numeric value. The
target type is normally inferred from the assignment, return value, or
fallback, and can be written explicitly when needed:

```splitscript
let percentage: f64 = current.percentageText.parse() else 0.0
let lives = current.livesText.parse<u8>()?
```

Parsing accepts an optional ASCII sign and decimal digits. Floating-point
targets additionally accept a decimal point and `e`/`E` exponent, plus
case-insensitive `NaN`, `inf`, and `Infinity`. Floating-point decimals are
correctly rounded directly to the target width with ties to even: finite
overflow produces signed infinity and underflow produces signed zero. Integer
overflow remains an error. Whitespace, digit separators, and trailing text are
rejected, so malformed game-memory input uses ordinary `T!` handling
rather than silently producing a partial value.

A managed class may declare `String field;` or `String? field;`. The reader
uses the object's stored UTF-16 length and decodes the complete payload in
chunks. Required null references and failed memory access are ordinary errors;
optional null references become `None`. Malformed surrogate sequences become
the Unicode replacement character. Nested reads share the materialization
budget, and a failed child rejects its containing snapshot.

JavaScript-inspired template strings use backticks and `{expression}`, without
JavaScript's `$` marker. Existing strings are inserted directly. Every other
interpolated value is converted by the same rules as `value as String`; integer
widths, `address`, and standard-library types with source-defined formatting
such as `FileVersion` are supported, while values without the [`Display`]
capability produce compile-time errors. Template strings may contain multiple interpolations,
nested expressions, and newlines. Literal braces are written as `\{` and `\}`.
`$` is ordinary template text, but JavaScript-style `${value}` warns because it
is usually an accidental spelling. Remove the `$` for `{value}`, or write
`\${value}` explicitly to emit a literal dollar sign followed by the
interpolation of `value`.

```text
let level = `{stage}-{act}`
let levelTime = `{minutes as u32}:{twoDigits(seconds as u32)}`
let executableLabel = `version {version}`
```

Transformations such as `toAsciiLowerCase`, `toAsciiUpperCase`,
`trimAsciiWhitespace`, `replaceAll`, and `split` do not mutate their receiver.
They return new values and are marked must-use, so a discarded result receives
a focused warning explaining the immutable behavior.
The [`print`] and [`setVariable`] functions accept any [`Display`] value
and apply these same conversions at the runtime boundary, so numeric values and
addresses do not need an explicit `as String` cast. Standard-library types can
provide one checked implementation for all four conversion entry points
without special syntax. [`FileVersion`] uses this mechanism to render
`major.minor.build.private`. [`Timer.state`] and [`setTickRate`] expose the
corresponding runtime facilities.

User structs and enums derive [`Display`] automatically. The default is a
stable multiline structural representation containing the type, variant,
field, and payload names. That derived implementation is materialized only when
a reachable conversion needs the concrete type. Defining the exact method
`fn TypeName.toString() -> String` overrides the derived representation, and
the `String` result may be inferred from the body. User code does not write an
`impl` block, capability declaration, or annotation. The compiler checks an
override's receiver, parameters, result, and inferred effects as an ordinary
source method, and implicit uses preserve those effects through their call
graphs.

```splitscript
struct Position {
    x: i32,
    y: i32,
}

fn Position.toString() -> String {
    return `({self.x}, {self.y})`
}

state "game.exe" {}
whileAttached {
    let position = Position { x: 3, y: 5 }
    print(position)
}
```

Standard-library implementations remain explicit and privileged so built-in
and representation-sensitive behavior stays auditable without imposing that
ceremony on short autosplitter scripts.
`timer.state()` returns `TimerState`, a
compiler-provided enum with `NotRunning`, `Running`, `Paused`, `Ended`, and
`Unknown`. Match it like any other enum; the raw host integer is not visible to
source code.
`timer.pauseGameTime()` and `timer.resumeGameTime()` provide explicit timer
mutation for lifecycle cleanup. Prefer `isLoading` for ordinary load removal.

The generated loop follows this order:

1. Refresh settings, sample timer state when `onStart` or `onReset` exists, and
   dispatch one observed lifecycle transition.
2. Attach to the configured process, first running `selectProcess` for each
   same-name candidate when declared; return and retry next tick if none is
   accepted.
3. Detect a closed process or an invalidated provider mapping, end the logical
   attachment, run `onDetach` only if `onAttach` completed successfully, and
   return. Mapping invalidation retains the still-open host process so provider
   discovery can run again.
4. Apply the active tick rate before any cooperative provider discovery, then
   discover and prepare the state provider and run `onAttach`.
5. Commit the first complete state as equal `old` and `current` snapshots, run
   `onStateReady`, and return; or rotate and refresh an initialized snapshot.
6. On initialized updates after that first snapshot, run `whileAttached`.
7. If the timer has not started, evaluate `start`.
8. If it is running or paused, apply `isLoading`, then `gameTime`, then `reset`;
   evaluate `split` only when reset did not trigger.
