//! Canonical catalog for language syntax and domain-specific constructs.
//!
//! These items are deliberately separate from [`crate::stdlib`]: `retry`,
//! lifecycle actions, and the settings DSL are syntax, not fake functions.
//! Documentation generators and editor tooling can still consume the same
//! metadata model through [`crate::catalog`].

use std::collections::HashSet;

use crate::{
    ast::ActionKind,
    catalog::{Documentation, Example},
    types::BuiltinType,
};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum LanguageItemKind {
    Keyword,
    Declaration,
    Syntax,
    BuiltinType(BuiltinType),
    SnapshotRoot,
    Action(ActionKind),
}

/// Where a catalog item is meaningful as an unqualified source completion.
/// Documentation entries are intentionally broader than insertable syntax;
/// concepts such as "closure" and "range" have pages but no token with that
/// spelling.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum LanguageCompletionSite {
    Expression,
    Statement,
    Loop,
    Return,
    Generator,
    Method,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct LanguageCompletion {
    pub site: LanguageCompletionSite,
    pub insert_text: &'static str,
    pub is_snippet: bool,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct LanguageItem {
    pub id: LanguageItemId,
    pub name: &'static str,
    pub kind: LanguageItemKind,
    /// Compact source shape suitable for completion details and reference docs.
    pub form: &'static str,
    pub documentation: Documentation<LanguageItemId>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ActionReferenceFacts {
    pub timing: &'static str,
    pub available_context: &'static str,
    pub suspension: &'static str,
    pub result: &'static str,
    pub fallthrough: &'static str,
}

const DECLARATIONS_SOURCE: &str = r#"state "game.exe" {}

struct Position {
    x: f32,
    y: f32,
}

enum Mode {
    Menu,
    Playing,
}

fn modeName(mode: Mode) {
    return match mode {
        Mode.Menu => "Menu",
        Mode.Playing => "Playing"
    }
}

fn executableKind(name: String) {
    return match name {
        "game.exe" => "full game",
        "game-demo.exe" => "demo",
        _ => "unsupported",
    }
}

fn Position.isOrigin() {
    return self.x == 0.0 && self.y == 0.0
}

whileAttached {
    let origin = Position { x: 0.0, y: 0.0 }
    if origin.isOrigin() {
        print(modeName(Mode.Playing))
    }
}"#;

const STATE_SOURCE: &str = r#"state "game.exe" {
    score = process.read<i32>(0x1000);
}

split {
    return current.score > old.score
}"#;

const FALLIBLE_STATE_SOURCE: &str = r#"state "game.exe" {
    score: i32 = {
        let address = process.follow(0x1000, [0x20])?
        process.read(address)
    };
}"#;

const ALTERNATE_PROCESS_STATE_SOURCE: &str = r#"state ["game.exe", "game-demo.exe"] {
    score: i32 at 0x1000;
}

split {
    return current.score > old.score
}"#;

const MULTI_PROVIDER_STATE_SOURCE: &str = r#"state {
    provider Windows: Native ["game.exe"] {
        level: u32 at 0x1000;
        checkpoint: u8 at 0x1100;
    },
    provider Advance: GBA {
        level: u32 at 0x03000010;
        room: u8 at 0x03000020;
    },
}

split {
    return match provider {
        StateProvider.Windows => current.checkpoint == 1,
        StateProvider.Advance => current.room == 5,
    }
}"#;

const MULTI_LAYOUT_STATE_SOURCE: &str = r#"enum Build {
    Steam,
    GOG,
}

let build: Build

state "game.exe" {
    if build == Build.Steam {
        level: u32 at 0x1000;
        checkpoint: u8 at 0x1100;
    } else {
        level: u32 at 0x2000;
        checkpoint: u16 at 0x2100;
    }
}

onAttach {
    let module = await process.mainModule()
    if module.size == 10_000 {
        build = Build.Steam
        return
    }
    if module.size == 20_000 {
        build = Build.GOG
        return
    }
    await process.closed()
}

whileAttached {
    setVariable("Level", current.level)
    setVariable("Checkpoint", match build {
        Build.Steam => current.checkpoint as u16,
        Build.GOG => current.checkpoint,
    })
}"#;

const MANAGED_IMAGE_SOURCE: &str = r#"enum Edition {
    BaseGame,
    Demo,
}

let edition: Edition

image "Assembly-CSharp" {
    class Player {
        u32 score;
    }

    class GameManager from ["Manager", "GameManager"] {
        static GameManager instance;
        Player player;

        if edition == Edition.BaseGame {
            u32 level;
        } else {
            u32 scene;
        }
    }
}

state Unity ["game.exe"] {
    manager: GameManager = GameManager.instance?.snapshot()?
}

onAttach {
    let managers = await GameManager.instances()
    print(managers.length())
}

whileAttached {
    if edition == Edition.BaseGame {
        print(current.manager.level)
    }
    else {
        print(current.manager.scene)
    }
}"#;

const MANAGED_NAMESPACE_SOURCE: &str = r#"image "Assembly-CSharp" {
    namespace Game {
        class Player {
            u32 score;
        }
    }
}

state Unity ["game.exe"] {}"#;

const MANAGED_FROM_SOURCE: &str = r#"image "Assembly-CSharp" {
    class Player from ["Game.Player", "Player"] {
        u32 score from ["_score", "<Score>k__BackingField"];
    }
}

state Unity ["game.exe"] {}"#;

const GBA_STATE_SOURCE: &str = r#"state GBA {
    room: u8 at 0x03000010;
}"#;

const PS1_STATE_SOURCE: &str = r#"state PS1 {
    health: u16 at 0x80012346;
}"#;

const PS2_STATE_SOURCE: &str = r#"state PS2 {
    health: u16 at 0x00123456;
}"#;

const SMS_STATE_SOURCE: &str = r#"state SMS {
    lives: u8 at 0xc010;
}"#;

const GENESIS_STATE_SOURCE: &str = r#"state Genesis {
    score: u32 at 0x1200;
}"#;

const GCN_STATE_SOURCE: &str = r#"state GCN {
    room: u16 at 0x80001000;
}"#;

const WII_STATE_SOURCE: &str = r#"state Wii {
    room: u16 at 0x80001000;
}"#;

const TICK_RATE_SOURCE: &str = r#"state "game.exe" {}

tickRate {
    attached: 60,
    detached: 2,
}
"#;

const POINTER_STATE_SOURCE: &str = r#"state "game.exe" {
    score: i32 at 0x1000;
}

split {
    return current.score > old.score
}"#;

const NATIVE_STRING_STATE_SOURCE: &str = r#"state "game.exe" {
    mapName at "game.dll", 0x1234, 0x20 as utf8(64);
}

whileAttached {
    print(current.mapName)
}"#;

const NATIVE_UTF16LE_STATE_SOURCE: &str = r#"state "game.exe" {
    chapterName at "game.dll", 0x2345, 0x18 as utf16le(64);
}

whileAttached {
    print(current.chapterName)
}"#;

const SETTINGS_SOURCE: &str = include_str!("../examples/lso_desktop_settings.split");

const SETTINGS_FAMILY_SOURCE: &str = r#"state "game.exe" {}

settings {
    "Levels" {
        /// Controls the corresponding level split.
        for level in 2..=36 {
            `Level {level}` key `{level}`: true,
        },
    },
}

split {
    return settings.enabled(currentLevel as String)
}

let currentLevel = 2
"#;

const RUNTIME_RANGE_SOURCE: &str = r#"state "game.exe" {}

fn visit(checkpoints: u8..=u8) {
    for checkpoint in checkpoints {
        print(checkpoint)
    }
}

whileAttached {
    for index in 0u32..<3 {
        print(index)
    }
    visit(1u8..=3)
}
"#;

const DOCUMENTATION_COMMENT_SOURCE: &str = r#"state "game.exe" {}

/// Formats a level number for display.
fn levelLabel(level) {
    return `Level {level}`
}

settings {
    /// Splits when the boss is defeated.
    "Split boss" => splitBoss: true
}
"#;

const CONTROL_FLOW_SOURCE: &str = r#"state "game.exe" {}

enum Mode {
    Menu,
    Playing,
}

fn modeName(mode: Mode) {
    return match mode {
        Mode.Menu => "Menu",
        Mode.Playing => "Playing"
    }
}

whileAttached {
    let mode = if timer.state() == TimerState.Running {
        Mode.Playing
    } else {
        Mode.Menu
    }
    if mode == Mode.Playing {
        print(modeName(mode))
    }
    debug print("development trace")

    let repetitions = 0
    while repetitions < 2 {
        repetitions += 1
        if repetitions == 1 {
            continue
        }
        break
    }

    for label in ["one", "two"] {
        print(label)
    }
}"#;

const LOOP_SOURCE: &str = r#"state "game.exe" {}

fn chooseModule(useVulkan: bool) -> String {
    return loop {
        if useVulkan {
            break "EngineWin64sv.dll"
        }
        break "EngineWin64s.dll"
    }
}

fn waitForever() -> async Never {
    loop {
        await nextTick()
    }
}

setup {
    print(chooseModule(false))
}"#;

const FOR_SOURCE: &str = r#"state "game.exe" {}

settings {
/// Controls each generated level split.
for level in 2..=36 {
    `Level {level}` key `{level}`: true,
}
}

whileAttached {
for label in ["one", "two"] {
    print(label)
}
}"#;

const IF_SOURCE: &str = r#"state "game.exe" {}

fn levelKind(isBoss: bool) -> String {
    let label = if isBoss {
        let kind = "Boss"
        `{kind} level`
    } else {
        "Level"
    }
    return label
}

whileAttached {
if timer.state() == TimerState.Running {
    print("Timer is running")
}
}"#;

const VALUE_BLOCK_SOURCE: &str = r#"state "game.exe" {}

fn levelKind(isBoss: bool) -> String {
    let label = if isBoss {
        let kind = "Boss"
        `{kind} level`
    } else {
        "Level"
    }
    return label
}

setup {
    print({
        let level = 7
        `Level {level}`
    })
}"#;

const EQUALITY_SOURCE: &str = r#"state "game.exe" {}

fn same(left: i32, right: i32) -> bool {
    return left == right
}

fn different(left: i32, right: i32) -> bool {
    return left != right
}
"#;

const FAILURE_SOURCE: &str = r#"state "game.exe" {}

fn readOrZero() {
    return process.read<i32>(0x1000) else 0
}

fn forwarded() -> i32! {
    return process.read<i32>(0x1000)?
}

fn unavailable() -> i32! {
    throw "not available"
}

fn explicitError() -> i32! {
    return Err("not available")
}

fn engineModule() {
    return retry {
        let module = process.loadedModule("EngineWin64s.dll")
            else process.loadedModule("EngineWin64sv.dll")
            else throw "engine module is not loaded yet"
        module
    }
}

whileAttached {
    print(readOrZero() as String)
}"#;

const ASYNC_SOURCE: &str = r#"state "game.exe" {}

fn loadGameAssembly() -> async Module {
    let module = await process.module("GameAssembly.dll")
    return module
}

fn readMarker() {
    return process.read<i32>(0x3000)
}

onAttach {
    let offsets: [i64; 2] = [0x100, 0x20]
    let module = await process.module("GameAssembly.dll")
    let marker = retry readMarker()
    let health = retry {
        let player = process.follow(module.address, offsets)?
        process.read<i32>(player)?
    }
    print(`ready {module.address}:{marker}:{health}`)
}"#;

// Catalog examples are compiled all the way through Wasm. The declaration is
// deliberately unreachable until typed source-function frame emission lands;
// parsing, checking, hover, and inlay inference still validate its async
// signature and body here.
const ASYNC_RESULT_SOURCE: &str = r#"state "game.exe" {}

fn loadGameAssembly() -> async Module {
    let module = await process.module("GameAssembly.dll")
    return module
}

onAttach {
    let module = await process.module("GameAssembly.dll")
    print(module.address)
}"#;

const CLOSURE_SOURCE: &str = r#"state "game.exe" {}

fn apply(value: u32, transform: (u32) -> u32) -> u32 {
    return transform(value)
}

whileAttached {
    let offset = 2u32
    let addOffset = value => value + offset
    let widen = (value: u16) -> u32 => value as u32
    let counter = 0u32
    let increment = () => {
        counter += 1
        return counter
    }
    print(addOffset(3))
    print(widen(3))
    print(apply(4, value => value * 2))
    print(increment())
}

onAttach {
    let afterTick = (value: u32) => {
        await nextTick()
        return value + 1
    }
    print(await afterTick(4))
}"#;

const TYPES_AND_LITERALS_SOURCE: &str = r#"state "game.exe" {}

fn maybe(value) -> i32? {
    if value > 0 {
        return value
    }
    return None
}

fn result() -> i32! {
    return 42
}

fn ignoreUnsupportedBuild() -> async Never {
    await process.closed()
}

onAttach {
    let module = await process.module("GameAssembly.dll")
    let marker = await module.scan(sig"48 8B ?? B?")
    let supportedVersion = v"1.2.3.4"
    let optionalMarker = Some(marker)
    let successfulValue = Ok(result() else 0)
    let text = (successfulValue else 0) as String
    print(`marker {optionalMarker else 0 as address}, value {text}`)
}"#;

const ITERATOR_SOURCE: &str = r#"state "game.exe" {}

whileAttached {
    let iterator = [10, 20].iterator()
    let label = match iterator.next() {
        Item(value) => `item {value}`,
        End => "finished",
    }
    print(label)
}"#;

const GENERATOR_SOURCE: &str = r#"state "game.exe" {}

fn values(end: u32) -> iterator u32 {
    let value = 0u32
    while value < end {
        yield value
        value += 1
    }
}

whileAttached {
    for value in values(3) {
        print(value)
    }
}"#;

const LIFECYCLE_SOURCE: &str = r#"state "game.exe" {}

setup {}
selectProcess {}
onDetach {}
onAttach {}
onStart {}
onReset {}
whileAttached {}
start {}
split {}
reset {}
isLoading {}
gameTime {}
"#;

macro_rules! focused_example {
    ($name:ident, $title:literal, $source:literal, $validation:expr) => {
        const $name: &[Example] = &[Example::checked($title, $source, $validation)];
    };
}

const ATTACHMENT_GLOBAL_SOURCE: &str = r#"let module

state "game.exe" {
    level: u32 = process.read(module.address)?;
}

onAttach {
    module = await process.mainModule()
}"#;

const MODULE_GLOBAL_SOURCE: &str = r#"fn defaultDelay() -> Duration {
    return Duration.fromSeconds(1.5)
}

let retryDelay = defaultDelay()

state "game.exe" {}

gameTime {
    return retryDelay
}"#;

const LET_EXAMPLE: &[Example] = &[
    Example::checked(
        "Infer a local type",
        "let retryDelay = 30",
        DECLARATIONS_SOURCE,
    ),
    Example::checked(
        "Keep an attachment-discovered value",
        ATTACHMENT_GLOBAL_SOURCE,
        ATTACHMENT_GLOBAL_SOURCE,
    ),
    Example::checked(
        "Compute module state once",
        "let retryDelay = defaultDelay()",
        MODULE_GLOBAL_SOURCE,
    ),
    Example::checked(
        "Combine patterns while sharing one binding",
        "return match side {\n    Side.Left(value) | Side.Right(value) => value,\n    Side.Idle => 0,\n}",
        "enum Side { Left(u32), Right(u32), Idle }\nstate \"game.exe\" {}\nfn unwrap(side: Side) -> u32 {\n    return match side {\n        Side.Left(value) | Side.Right(value) => value,\n        Side.Idle => 0,\n    }\n}",
    ),
];
const BINDING_PATTERN_SOURCE: &str = r#"struct Point {
    x: u32,
    y: u32,
}

struct Impossible {
    value: Never,
}

enum PointOrNever {
    Point(Point),
    Impossible(Impossible),
}

let { x: originX, y: originY } = Point { x: 0, y: 0 }

state "game.exe" {}

fn sum(PointOrNever.Point({ x, y }): PointOrNever) -> u32 {
    return x + y
}

whileAttached {
    let { x, y }: Point = Point { x: originX, y: originY }
    let add = ({ x: left, y: right }: Point) => left + right
    for { x: item, y: _ } in [Point { x, y }] {
        print(sum(PointOrNever.Point(Point { x: add(Point { x: item, y: 1 }), y: 0 })))
    }
}"#;

const BINDING_PATTERN_EXAMPLES: &[Example] = &[
    Example::checked(
        "Destructure a struct once",
        "let { x, y }: Point = point",
        BINDING_PATTERN_SOURCE,
    ),
    Example::checked(
        "Destructure parameters and iterator items",
        "fn sum({ x, y }: Point) -> u32 {\n    return x + y\n}\n\nfor { x, y: _ } in points {\n    print(x)\n}",
        BINDING_PATTERN_SOURCE,
    ),
];
focused_example!(
    FUNCTION_EXAMPLE,
    "Declare a helper",
    "fn isBoss(level) {\n    return level == 7\n}",
    DECLARATIONS_SOURCE
);
const CLOSURE_EXAMPLES: &[Example] = &[
    Example::checked(
        "Write an explicit result type",
        "let widen = (value: u16) -> u32 => value as u32",
        CLOSURE_SOURCE,
    ),
    Example::checked(
        "Pass behavior to a function",
        "let doubled = apply(4, value => value * 2)",
        CLOSURE_SOURCE,
    ),
    Example::checked(
        "Capture and update a local",
        "let counter = 0u32\nlet increment = () => {\n    counter += 1\n    return counter\n}",
        CLOSURE_SOURCE,
    ),
    Example::checked(
        "Suspend inside a closure",
        "let afterTick = (value: u32) => {\n    await nextTick()\n    return value + 1\n}\nprint(await afterTick(4))",
        CLOSURE_SOURCE,
    ),
];
const CALLABLE_TYPE_EXAMPLE: &[Example] = &[
    Example::checked(
        "Accept a callable value",
        "fn apply(value: u32, transform: (u32) -> u32) -> u32 {\n    return transform(value)\n}",
        CLOSURE_SOURCE,
    ),
    Example::checked(
        "Store a named function",
        "fn increment(value: u32) -> u32 {\n    return value + 1\n}\n\nlet later = increment\nprint(later(4))",
        CLOSURE_SOURCE,
    ),
];
focused_example!(
    RECORD_EXAMPLE,
    "Group related values",
    "struct Position {\n    x: f32,\n    y: f32,\n}",
    DECLARATIONS_SOURCE
);
const PAYLOAD_ENUM_SOURCE: &str = r#"state "game.exe" {}

enum LoadState {
    Loading,
    Ready(u32),
    Failed(String),
}
"#;

const PROCESS_READABLE_ENUM_SOURCE: &str = r#"enum GameState: i32 {
    Mission = 0,
    TitleScreen = 1,
    Menu = 2,
}

state "game.exe" {
    gameState: GameState at 0x1000
}
"#;

const ENUM_EXAMPLES: &[Example] = &[
    Example::checked(
        "Describe distinct states",
        "enum Mode {\n    Menu,\n    Playing,\n}",
        DECLARATIONS_SOURCE,
    ),
    Example::checked(
        "Attach data to variants",
        "enum LoadState {\n    Loading,\n    Ready(u32),\n    Failed(String),\n}",
        PAYLOAD_ENUM_SOURCE,
    ),
    Example::checked(
        "Read an integer-backed game state",
        "enum GameState: i32 {\n    Mission = 0,\n    TitleScreen = 1,\n    Menu = 2,\n}\n\nstate \"game.exe\" {\n    gameState: GameState at 0x1000\n}",
        PROCESS_READABLE_ENUM_SOURCE,
    ),
];
const STATE_DECL_EXAMPLES: &[Example] = &[
    Example::checked(
        "Read state from a native process",
        "state \"game.exe\" {\n    score = process.read<i32>(0x1000);\n}",
        STATE_SOURCE,
    ),
    Example::checked(
        "Compose fallible address discovery with a read",
        FALLIBLE_STATE_SOURCE,
        FALLIBLE_STATE_SOURCE,
    ),
    Example::checked(
        "Try alternate executable names",
        "state [\"game.exe\", \"game-demo.exe\"] {\n    score: i32 at 0x1000;\n}",
        ALTERNATE_PROCESS_STATE_SOURCE,
    ),
    Example::checked(
        "Share one autosplitter across different runtimes",
        MULTI_PROVIDER_STATE_SOURCE,
        MULTI_PROVIDER_STATE_SOURCE,
    ),
    Example::checked(
        "Support multiple game builds",
        MULTI_LAYOUT_STATE_SOURCE,
        MULTI_LAYOUT_STATE_SOURCE,
    ),
    Example::checked(
        "Read state from a GBA emulator",
        "state GBA {\n    room: u8 at 0x03000010;\n}",
        GBA_STATE_SOURCE,
    ),
    Example::checked(
        "Read state from a PlayStation emulator",
        "state PS1 {\n    health: u16 at 0x80012346;\n}",
        PS1_STATE_SOURCE,
    ),
    Example::checked(
        "Read state from a PlayStation 2 emulator",
        "state PS2 {\n    health: u16 at 0x00123456;\n}",
        PS2_STATE_SOURCE,
    ),
    Example::checked(
        "Read state from a Master System emulator",
        "state SMS {\n    lives: u8 at 0xc010;\n}",
        SMS_STATE_SOURCE,
    ),
    Example::checked(
        "Read state from a Sega Genesis emulator",
        "state Genesis {\n    score: u32 at 0x1200;\n}",
        GENESIS_STATE_SOURCE,
    ),
    Example::checked(
        "Read state from a GameCube emulator",
        "state GCN {\n    room: u16 at 0x80001000;\n}",
        GCN_STATE_SOURCE,
    ),
    Example::checked(
        "Read state from a Wii emulator",
        "state Wii {\n    room: u16 at 0x80001000;\n}",
        WII_STATE_SOURCE,
    ),
];
focused_example!(
    STATE_POINTER_EXAMPLE,
    "Read a pointer-backed field",
    "score: i32 at 0x1000",
    POINTER_STATE_SOURCE
);
focused_example!(
    NATIVE_STRING_DECODER_EXAMPLE,
    "Decode a native string field",
    "mapName at 0x1234 as utf8(64)",
    NATIVE_STRING_STATE_SOURCE
);
focused_example!(
    NATIVE_UTF16LE_DECODER_EXAMPLE,
    "Decode a native UTF-16LE field",
    "chapterName at 0x2345 as utf16le(64)",
    NATIVE_UTF16LE_STATE_SOURCE
);
focused_example!(
    SETTINGS_DECL_EXAMPLE,
    "Declare a toggle",
    "settings {\n    \"Split bosses\" => splitBosses: true,\n}",
    SETTINGS_SOURCE
);
focused_example!(
    SETTING_KEY_EXAMPLE,
    "Keep a stable host key",
    "\"Enable Auto Splitting\" => enableAutoSplitting key \"auto-splitting\": true",
    SETTINGS_SOURCE
);
focused_example!(
    TEXT_INPUT_SETTING_EXAMPLE,
    "Accept free-form text",
    "\"Profile Name\" => profileName: \"Player\"",
    SETTINGS_SOURCE
);
const IF_EXAMPLES: &[Example] = &[
    Example::checked(
        "Run a conditional statement",
        "if timer.state() == TimerState.Running {\n    print(\"Timer is running\")\n}",
        IF_SOURCE,
    ),
    Example::checked(
        "Choose a value",
        "let label = if isBoss { \"Boss\" } else { \"Level\" }",
        IF_SOURCE,
    ),
    Example::checked(
        "Prepare a branch value",
        "let label = if isBoss {\n    let kind = \"Boss\"\n    `{kind} level`\n} else {\n    \"Level\"\n}",
        IF_SOURCE,
    ),
];
const VALUE_BLOCK_EXAMPLES: &[Example] = &[
    Example::checked(
        "Compute a value with local steps",
        "let label = {\n    let level = 7\n    `Level {level}`\n}",
        VALUE_BLOCK_SOURCE,
    ),
    Example::checked(
        "Prepare an argument locally",
        "print({\n    let level = 7\n    `Level {level}`\n})",
        VALUE_BLOCK_SOURCE,
    ),
];
const ELSE_EXAMPLES: &[Example] = &[
    Example::checked(
        "Provide a read fallback",
        "let health = process.read<i32>(healthAddress) else 0",
        FAILURE_SOURCE,
    ),
    Example::checked(
        "Retry either of two module names",
        "let engine = retry {\n    let module = process.loadedModule(\"EngineWin64s.dll\")\n        else process.loadedModule(\"EngineWin64sv.dll\")\n        else throw \"engine module is not loaded yet\"\n    module\n}",
        FAILURE_SOURCE,
    ),
];
focused_example!(
    WHILE_EXAMPLE,
    "Repeat while attached",
    "while index < values.length() {\n    index += 1\n}",
    CONTROL_FLOW_SOURCE
);
const LOOP_EXAMPLES: &[Example] = &[
    Example::checked(
        "Repeat without falling through",
        "loop {\n    await nextTick()\n}",
        LOOP_SOURCE,
    ),
    Example::checked(
        "Break with a value",
        "let moduleName = loop {\n    if useVulkan {\n        break \"EngineWin64sv.dll\"\n    }\n    break \"EngineWin64s.dll\"\n}",
        LOOP_SOURCE,
    ),
];
const FOR_EXAMPLES: &[Example] = &[
    Example::checked(
        "Iterate over an array",
        "for label in [\"one\", \"two\"] {\n    print(label)\n}",
        FOR_SOURCE,
    ),
    Example::checked(
        "Declare a family of settings",
        "/// Controls each generated level split.\nfor level in 2..=36 {\n    `Level {level}` key `{level}`: true,\n}",
        FOR_SOURCE,
    ),
];
const RANGE_EXAMPLES: &[Example] = &[
    Example::checked(
        "Exclude the upper endpoint",
        "for index in 0u32..<3 {\n    print(index)\n}",
        RUNTIME_RANGE_SOURCE,
    ),
    Example::checked(
        "Pass an inclusive range as a value",
        "fn visit(checkpoints: u8..=u8) {\n    for checkpoint in checkpoints {\n        print(checkpoint)\n    }\n}\n\nvisit(1u8..=3)",
        RUNTIME_RANGE_SOURCE,
    ),
    Example::checked(
        "Match an integer interval",
        "return match level {\n    0..<10 => \"early\",\n    10..=20 => \"late\",\n    _ => \"outside\",\n}",
        "state \"game.exe\" {}\nfn classify(level: i32) -> String {\n    return match level {\n        0..<10 => \"early\",\n        10..=20 => \"late\",\n        _ => \"outside\",\n    }\n}",
    ),
];
focused_example!(
    ARRAY_REST_PATTERN_EXAMPLE,
    "Match an array prefix and suffix",
    "return match bytes {\n    [0x53, .., 0] => true,\n    _ => false,\n}",
    "state \"game.exe\" {}\nfn framed(bytes: [u8]) -> bool {\n    return match bytes {\n        [0x53, .., 0] => true,\n        _ => false,\n    }\n}"
);
const BREAK_EXAMPLES: &[Example] = &[
    Example::checked(
        "Exit a conditional loop",
        "while !found {\n    if cannotContinue { break }\n}",
        CONTROL_FLOW_SOURCE,
    ),
    Example::checked(
        "Produce a loop value",
        "let moduleName = loop {\n    if useVulkan { break \"EngineWin64sv.dll\" }\n    break \"EngineWin64s.dll\"\n}",
        LOOP_SOURCE,
    ),
];
focused_example!(
    CONTINUE_EXAMPLE,
    "Skip an iteration",
    "while index < count {\n    index += 1\n    if index == ignored { continue }\n    inspect(index)\n}",
    CONTROL_FLOW_SOURCE
);
focused_example!(
    DEBUG_EXAMPLE,
    "Add development-only logging",
    "debug print(`level: {current.level}`)",
    CONTROL_FLOW_SOURCE
);
focused_example!(
    INSPECT_EXAMPLE,
    "Publish and keep using a value",
    "let level = inspect(current.level)",
    "state \"game.exe\" { level: u32 at 0x1000; }\nwhileAttached {\n    let level = inspect(current.level)\n    print(level)\n}"
);
const MATCH_EXAMPLES: &[Example] = &[
    Example::checked(
        "Handle every enum variant",
        "let label = match mode {\n    Mode.Menu => \"Menu\",\n    Mode.Playing => \"Playing\"\n}",
        CONTROL_FLOW_SOURCE,
    ),
    Example::checked(
        "Dispatch on exact string contents",
        "return match name {\n    \"game.exe\" => \"full game\",\n    \"game-demo.exe\" => \"demo\",\n    _ => \"unsupported\",\n}",
        CONTROL_FLOW_SOURCE,
    ),
    Example::checked(
        "Destructure an exact array shape",
        "return match bytes {\n    [0x53, value, 0] => value,\n    _ => 0,\n}",
        "state \"game.exe\" {}\nfn decode(bytes: [u8]) -> u8 {\n    return match bytes {\n        [0x53, value, 0] => value,\n        _ => 0,\n    }\n}",
    ),
    Example::checked(
        "Match selected struct fields",
        "return match point {\n    Point { label: \"start\", x } => x,\n    _ => 0,\n}",
        "struct Point {\n    x: i32,\n    label: String,\n}\nstate \"game.exe\" {}\nfn horizontal(point: Point) -> i32 {\n    return match point {\n        Point { label: \"start\", x } => x,\n        _ => 0,\n    }\n}",
    ),
];
const IS_EXAMPLES: &[Example] = &[
    Example::checked(
        "Bind a value on the matching path",
        "if value is Some(number) && number > 0 {\n    print(number)\n}",
        "state \"game.exe\" {}\nfn inspectValue(value: u32?) {\n    if value is Some(number) && number > 0 {\n        print(number)\n    }\n}",
    ),
    Example::checked(
        "Use the binding on a negated condition's else path",
        "if !(value is Some(number)) {\n    return 0\n} else {\n    return number\n}",
        "state \"game.exe\" {}\nfn unwrapOrZero(value: u32?) -> u32 {\n    if !(value is Some(number)) {\n        return 0\n    } else {\n        return number\n    }\n}",
    ),
];
focused_example!(
    RETURN_EXAMPLE,
    "Return a value",
    "fn double(value) {\n    return value * 2\n}",
    FAILURE_SOURCE
);
focused_example!(
    THROW_EXAMPLE,
    "Return an error",
    "fn requireAddress(value) -> address! {\n    if value == 0 { throw \"address is null\" }\n    return value\n}",
    FAILURE_SOURCE
);
focused_example!(
    ASYNC_RESULT_EXAMPLE,
    "Declare an asynchronous helper",
    "fn loadGameAssembly() -> async Module {\n    let module = await process.module(\"GameAssembly.dll\")\n    return module\n}",
    ASYNC_RESULT_SOURCE
);
focused_example!(
    ITERATOR_TYPE_EXAMPLE,
    "Declare a synchronous generator",
    "fn values(end: u32) -> iterator u32 {\n    let value = 0u32\n    while value < end {\n        yield value\n        value += 1\n    }\n}",
    GENERATOR_SOURCE
);
focused_example!(
    YIELD_EXAMPLE,
    "Produce the next item",
    "yield value",
    GENERATOR_SOURCE
);
focused_example!(
    AWAIT_EXAMPLE,
    "Wait during attachment",
    "let module = await process.module(\"GameAssembly.dll\")",
    ASYNC_SOURCE
);
const RETRY_EXAMPLES: &[Example] = &[
    Example::checked(
        "Retry one fallible expression",
        "let marker = retry readMarker()",
        ASYNC_SOURCE,
    ),
    Example::checked(
        "Retry a complete fallible block",
        "let health = retry {\n    let player = process.follow(module.address, offsets)?\n    process.read<i32>(player)?\n}",
        ASYNC_SOURCE,
    ),
];
const PROPAGATE_EXAMPLES: &[Example] = &[
    Example::checked(
        "Forward an error from a function",
        "let health = process.read<i32>(healthAddress)?",
        FAILURE_SOURCE,
    ),
    Example::checked(
        "Reject one state-field update",
        "score: i32 = {\n    let address = process.follow(0x1000, [0x20])?\n    process.read(address)\n};",
        FALLIBLE_STATE_SOURCE,
    ),
];
focused_example!(
    CAST_EXAMPLE,
    "Convert an integer",
    "let label = level as String",
    TYPES_AND_LITERALS_SOURCE
);
focused_example!(
    SOME_EXAMPLE,
    "Construct an optional value",
    "let selected: i32? = Some(7)",
    TYPES_AND_LITERALS_SOURCE
);
focused_example!(
    ITERATOR_ITEM_EXAMPLE,
    "Construct an iterator item step",
    "let step: IteratorStep<i32> = Item(7)",
    ITERATOR_SOURCE
);
focused_example!(
    ITERATOR_END_EXAMPLE,
    "Construct an exhausted iterator step",
    "let step: IteratorStep<i32> = End",
    ITERATOR_SOURCE
);
focused_example!(
    OK_EXAMPLE,
    "Construct a successful result",
    "let health: i32! = Ok(100)",
    TYPES_AND_LITERALS_SOURCE
);
focused_example!(
    ERR_EXAMPLE,
    "Construct an error",
    "let health: i32! = Err(\"health is unavailable\")",
    FAILURE_SOURCE
);
focused_example!(
    SELF_EXAMPLE,
    "Use the method receiver",
    "fn Position.isOrigin() {\n    return self.x == 0.0 && self.y == 0.0\n}",
    DECLARATIONS_SOURCE
);
focused_example!(
    SIGNATURE_EXAMPLE,
    "Match machine code",
    "let marker = await module.scan(sig\"48 8B ?? B?\")",
    TYPES_AND_LITERALS_SOURCE
);
focused_example!(
    VERSION_EXAMPLE,
    "Match a Windows file version",
    "if version == v\"1.2.3.4\" { print(\"supported build\") }",
    TYPES_AND_LITERALS_SOURCE
);
focused_example!(
    TEMPLATE_EXAMPLE,
    "Interpolate a value",
    "let label = `Level {current.level}`",
    TYPES_AND_LITERALS_SOURCE
);
focused_example!(
    ARRAY_TYPE_EXAMPLE,
    "Annotate an exact-length array",
    "let offsets: [i64; 2] = [0x100, 0x20]",
    TYPES_AND_LITERALS_SOURCE
);
focused_example!(
    ARRAY_INDEX_EXAMPLE,
    "Read an array element",
    "let opcode = bytes[1]",
    "state \"game.exe\" {}\nwhileAttached {\n    let bytes: [u8; 3] = [0x48, 0x8b, 0x01]\n    let opcode = bytes[1]\n    print(opcode)\n}"
);
focused_example!(
    OPTION_TYPE_EXAMPLE,
    "Annotate an optional value",
    "let selectedLevel: i32? = None",
    TYPES_AND_LITERALS_SOURCE
);
focused_example!(
    RESULT_TYPE_EXAMPLE,
    "Return a fallible value",
    "fn readHealth() -> i32! {\n    return process.read<i32>(healthAddress)?\n}",
    TYPES_AND_LITERALS_SOURCE
);
const DOCUMENTATION_COMMENT_EXAMPLES: &[Example] = &[
    Example::checked(
        "Document a source symbol",
        "/// Formats a level number for display.\nfn levelLabel(level) {\n    return `Level {level}`\n}",
        DOCUMENTATION_COMMENT_SOURCE,
    ),
    Example::checked(
        "Add a setting tooltip",
        "/// Splits when the boss is defeated.\n\"Split boss\" => splitBoss: true",
        DOCUMENTATION_COMMENT_SOURCE,
    ),
];
focused_example!(
    CHOICE_EXAMPLE,
    "Choose an enum value",
    "\"Character\" => character: choice {\n    \"Hana\" => Character.Hana default,\n    \"Toree\" => Character.Toree,\n}",
    SETTINGS_SOURCE
);
focused_example!(
    FILE_EXAMPLE,
    "Choose a file",
    "\"Layout\" => layout: file {\n    \"Layout files\" => \"*.lsl\"\n}",
    SETTINGS_SOURCE
);
focused_example!(
    SETTINGS_FAMILY_EXAMPLE,
    "Declare numbered level settings",
    "for level in 2..=36 {\n    `Level {level}` key `{level}`: true,\n}",
    SETTINGS_FAMILY_SOURCE
);
focused_example!(
    EQUALITY_EXAMPLE,
    "Compare equal values",
    "left == right",
    EQUALITY_SOURCE
);
focused_example!(
    INEQUALITY_EXAMPLE,
    "Compare different values",
    "left != right",
    EQUALITY_SOURCE
);

macro_rules! language_item {
    ($id:ident, $name:expr, $kind:expr, $form:expr, $summary:expr, $details:expr, $examples:expr) => {
        LanguageItem {
            id: LanguageItemId::$id,
            name: $name,
            kind: $kind,
            form: $form,
            documentation: Documentation {
                summary: $summary,
                details: $details,
                examples: $examples,
                related: &[],
            },
        }
    };
}

macro_rules! action_item {
    ($id:ident, $action:ident, $name:literal, $summary:literal, $details:literal,
     $example:literal, related: $related:expr) => {
        LanguageItem {
            id: LanguageItemId::$id,
            name: $name,
            kind: LanguageItemKind::Action(ActionKind::$action),
            form: concat!($name, " { ... }"),
            documentation: Documentation {
                summary: $summary,
                details: $details,
                examples: &[Example::checked(
                    concat!("Use ", $name),
                    $example,
                    LIFECYCLE_SOURCE,
                )],
                related: $related,
            },
        }
    };
    ($id:ident, $action:ident, $name:literal, $summary:literal, $details:literal,
     $example:literal) => {
        language_item!(
            $id,
            $name,
            LanguageItemKind::Action(ActionKind::$action),
            concat!($name, " { ... }"),
            $summary,
            $details,
            &[Example::checked(
                concat!("Use ", $name),
                $example,
                LIFECYCLE_SOURCE,
            )]
        )
    };
}

macro_rules! builtin_type_item {
    ($ty:ident, $name:literal, $summary:literal, $details:literal, $example:literal) => {
        LanguageItem {
            id: LanguageItemId::BuiltinType(BuiltinType::$ty),
            name: $name,
            kind: LanguageItemKind::BuiltinType(BuiltinType::$ty),
            form: $name,
            documentation: Documentation {
                summary: $summary,
                details: $details,
                examples: &[Example::checked(
                    concat!("Use ", $name),
                    $example,
                    TYPES_AND_LITERALS_SOURCE,
                )],
                related: &[],
            },
        }
    };
}

macro_rules! compiler_symbol_item {
    ($id:expr, $name:literal, $kind:expr, $form:literal, $summary:literal, $details:literal,
     $example:literal, $validation:expr) => {
        LanguageItem {
            id: $id,
            name: $name,
            kind: $kind,
            form: $form,
            documentation: Documentation {
                summary: $summary,
                details: $details,
                examples: &[Example::checked(
                    concat!("Use ", $name),
                    $example,
                    $validation,
                )],
                related: &[],
            },
        }
    };
}

macro_rules! define_language_catalog {
    (
        ordinary_before { $(language_item!($before_id:ident, $($before:tt)*)),* $(,)? }
        builtins { $(builtin_type_item!($builtin_id:ident, $($builtin:tt)*)),* $(,)? }
        compiler_symbols { $(compiler_symbol_item!(LanguageItemId::$compiler_id:ident, $($compiler:tt)*)),* $(,)? }
        ordinary_after { $(language_item!($after_id:ident, $($after:tt)*)),* $(,)? }
        actions { $(action_item!($action_id:ident, $($action:tt)*)),* $(,)? }
    ) => {
        #[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord)]
        pub enum LanguageItemId {
            $($before_id,)*
            BuiltinType(BuiltinType),
            $($compiler_id,)*
            $($after_id,)*
            $($action_id),*
        }

        const ITEMS: &[LanguageItem] = &[
            $(language_item!($before_id, $($before)*),)*
            $(builtin_type_item!($builtin_id, $($builtin)*),)*
            $(compiler_symbol_item!(LanguageItemId::$compiler_id, $($compiler)*),)*
            $(language_item!($after_id, $($after)*),)*
            $(action_item!($action_id, $($action)*),)*
        ];
    };
}

define_language_catalog! {
    ordinary_before {
    language_item!(
        Let,
        "let",
        LanguageItemKind::Keyword,
        "let pattern = expression or let name",
        "Declares an inferred variable.",
        "Bindings are mutable and their types are inferred bidirectionally from initializers, assignments, and uses. An initialized local or top-level declaration may use an irrefutable [`binding pattern`] to project several names from its value; the initializer is evaluated once and an annotation after the pattern describes that complete incoming value. An initialized top-level binding is module state: its initializer runs exactly once, before [`setup`]. It may allocate and call synchronous pure helpers, but it must be closed: it cannot read or write another global, observe settings, timer, process, or state context, or suspend. A bare top-level [`let`] remains a single name and instead gets its lifetime from its direct lifecycle initializer: [`onAttach`] creates attachment-scoped state that is cleared on detach, while [`onStart`] creates attempt-scoped (or run-scoped) state that is cleared after [`onReset`]. The lifecycle initializer must assign it on every completing path.",
        LET_EXAMPLE
    ),
    language_item!(
        Function,
        "fn",
        LanguageItemKind::Declaration,
        "fn name(parameters) { ... }",
        "Declares a function or method.",
        "Each parameter may be an irrefutable [`binding pattern`]. It still consumes exactly one argument; an annotation applies to that complete argument while the projected names receive their field or payload types. Parameter and result annotations are optional when constraints from the body and call sites determine them. A function declared `->` [`iterator`] `T` may either forward an existing iterator with ordinary [`return`] or become a lazy synchronous generator by using [`yield`].",
        FUNCTION_EXAMPLE
    ),
    language_item!(
        Closure,
        "closure",
        LanguageItemKind::Syntax,
        "value => expression | (left: T, right: U) -> Result => { ... }",
        "Creates a callable value with lexical captures.",
        "Each parenthesized parameter may be an irrefutable [`binding pattern`] and still consumes one argument. Parameter and result types are inferred bidirectionally from the body, invocation sites, and any expected [`callable type`]. A single inferred name may omit parentheses; zero, multiple, annotated, or destructured parameters use parentheses. An explicit result uses `(parameters) -> Result => body`; write [`async`] `T` for an explicitly asynchronous closure or [`iterator`] `T` for a closure that forwards or generates an iterator. The body is any expression, including a [`value block`]. An async body may use [`await`] or [`retry`]; an iterator body becomes a lazy generator only when it uses [`yield`]. Calling an async function or generator creates its continuation without executing the body. Captured immutable values are retained in the closure environment. A mutable local is captured by reference through one shared cell, so assignments in the closure and its declaring scope observe each other even after the closure is returned or stored across [`await`]. [`return`] exits the closure itself; [`break`] and [`continue`] cannot escape into an outer loop.",
        CLOSURE_EXAMPLES
    ),
    language_item!(
        BindingPattern,
        "binding pattern",
        LanguageItemKind::Syntax,
        "let Pattern = value | fn name(Pattern: Type) | for Pattern in values",
        "Destructures one value into local names where a declaration cannot fail.",
        "Initialized [`let`] declarations, named [`fn`] parameters, parenthesized [`closure`] parameters, and runtime [`for`] bindings accept the same recursive struct, enum, wrapper, array, wildcard, and alternative pattern grammar as [`match`] and [`is`]. The incoming initializer, argument, or iterator item is evaluated exactly once and remains one ABI value; each binding leaf receives its projected type. A type annotation after the pattern constrains the complete incoming value, not an individual leaf. When that context already identifies one concrete struct, `{ field, other: pattern }` may omit its nominal type name; `Name { ... }` remains available explicitly and field names alone never infer a structural type. Such a pattern must be irrefutable: it must cover every value its type can actually contain. [`Never`] is uninhabited recursively, so an enum variant carrying [`Never`], a struct containing it, or a nonempty fixed array of it cannot occur and need not be covered. For an ordinary optional or multi-variant value, use [`is`] to test one case or an exhaustive [`match`] instead. Uninitialized attachment- and attempt-scoped globals deliberately remain single names because lifecycle code initializes their storage later.",
        BINDING_PATTERN_EXAMPLES
    ),
    language_item!(
        CallableType,
        "callable type",
        LanguageItemKind::Syntax,
        "(Parameter, ...) -> Result",
        "Describes a first-class callable value.",
        "The parameter list may be empty and the result may be any ordinary type, including [`async`] `T` and [`iterator`] `T`. A value of this type is invoked with ordinary call syntax. Both named [`fn`] values and [`closure`] expressions infer this type from either direction and share one runtime representation. Calling an async or generator callable creates a lazy continuation; merely storing the callable never executes its effects. Callable values are intentionally not [`Equatable`].",
        CALLABLE_TYPE_EXAMPLE
    ),
    language_item!(
        Struct,
        "struct",
        LanguageItemKind::Declaration,
        "struct Name { field: Type }",
        "Declares an immutable nominal structure.",
        "Structs provide named fields, structural equality when their fields support it, and fixed process-memory layouts when every field is readable. In a literal, `Name { field }` is shorthand for `Name { field: field }`; an explicit repeated initializer receives a safe shorthand fix, and renaming either identity expands the shorthand when needed. In [`match`] and [`binding pattern`] positions, `Name { field, other: pattern }` binds `field`, recursively tests `other`, and ignores omitted fields without requiring a `..` marker. The name may be omitted as `{ field, other: pattern }` when the surrounding value already has one concrete struct type; this remains nominal matching rather than structural inference.",
        RECORD_EXAMPLE
    ),
    language_item!(
        Enum,
        "enum",
        LanguageItemKind::Declaration,
        "enum Name[: Integer] { Variant[(Type)] [= value] }",
        "Declares a nominal sum type.",
        "Enums support optional variant payloads, exhaustive match expressions, and structural equality when their payloads support it. Writing a fixed-width integer representation after the name, such as `enum GameState: i32`, instead declares the enum's process-memory encoding and derives [`MemoryReadable`]. Process-readable variants cannot carry payloads. Their discriminants start at zero, increase by one when omitted, and may be written explicitly with `= value`; duplicates and values outside the representation are compile errors. Reading an unknown raw discriminant fails the enclosing memory read instead of constructing an invalid enum value. The same validation applies recursively inside readable [`struct`]s and fixed arrays and respects the state provider's byte order.",
        ENUM_EXAMPLES
    ),
    language_item!(
        ManagedImage,
        "image",
        LanguageItemKind::Declaration,
        "image \"Assembly-CSharp\" { class Name { ... } }",
        "Declares the managed types exposed by one runtime image.",
        "An [`image`] schema names the managed assembly image that owns its classes. The [`Unity`] state provider resolves the reachable image and class metadata once per attachment before state polling begins. Unused schema declarations do not retain generated binding or reading code. Schema declarations describe metadata; they do not read live game memory until a [`static`] or instance field path is evaluated. Runtime image traversal is a private implementation detail of the provider rather than an alternative public workflow.",
        &[Example::checked(
            "Describe a managed assembly image",
            "image \"Assembly-CSharp\" {\n    class Player {\n        u32 score;\n    }\n\n    class GameManager from [\"Manager\", \"GameManager\"] {\n        static GameManager instance;\n        Player player;\n\n        if edition == Edition.BaseGame {\n            u32 level;\n        } else {\n            u32 scene;\n        }\n    }\n}",
            MANAGED_IMAGE_SOURCE,
        )]
    ),
    language_item!(
        ManagedNamespace,
        "namespace",
        LanguageItemKind::Declaration,
        "namespace Name { class Type { ... } }",
        "Declares a managed metadata namespace inside an image schema.",
        "A [`namespace`] preserves the metadata qualification shared by the managed [`class`] declarations nested inside it. Namespaces may be nested when the runtime metadata uses several qualification segments. Source code still refers to a declared class by its SplitScript class name; the namespace controls managed metadata lookup rather than creating a value namespace.",
        &[Example::checked(
            "Declare a class in a managed namespace",
            "namespace Game {\n    class Player {\n        u32 score;\n    }\n}",
            MANAGED_NAMESPACE_SOURCE,
        )]
    ),
    language_item!(
        ManagedClass,
        "class",
        LanguageItemKind::Declaration,
        "class Name from [\"Alias\", ...] { Type field; static Type field; if shape == Variant { ... } }",
        "Declares a typed managed class binding.",
        "The class name `T` denotes an immutable local snapshot, while `T.Ref` denotes a live remote object reference. Fields without [`static`] are read fallibly from a `T.Ref`; static fields are read through the class name. Field lookup follows the runtime inheritance chain. For an inherited static field, the provider binds storage through the runtime class that actually declares the field, including a closed generic base, while the source still uses the ergonomic `Derived.field` spelling. Each live field hop yields [`T!`], so postfix [`?`] can propagate an unsuccessful lookup to the surrounding state field, function, or [`retry`] boundary. Calling `reference.snapshot()` reads every active instance field first and exposes one [`T!`] only when the complete snapshot succeeds; no partially populated object escapes when a read fails. Live scalar paths reread remote memory without allocating a GC object, while snapshots and arrays materialize owned values. `await T.instances()` cooperatively scans readable, writable, non-executable process memory and returns a completed `[T.Ref]` snapshot. A [`UnityGameObject`](type@UnityGameObject) obtained through `unity.scenes` can use `component<T>()` to find the same typed `T.Ref` by runtime class. Both traversal paths are bounded, and generated binders, readers, snapshots, and scans are retained only when used. The optional [`from`] list supplies runtime metadata names. Fields declared directly in the class are always available. Put build-specific fields in an [`if`] / [`else if`](keyword@if) / [`else`](keyword@if) chain over an enum global initialized during [`onAttach`]. When the managed metadata uniquely identifies the enum variant, the [`Unity`] provider initializes that global automatically. Each later branch describes exactly the variants left unmatched by earlier branches, and the same predicate refines those fields in ordinary code. Mono and IL2CPP metadata traversal remains private to the [`Unity`] provider.",
        &[
            Example::checked(
                "Follow a typed managed field path",
                "score: u32 = GameManager.instance?.player?.score?",
                MANAGED_IMAGE_SOURCE,
            ),
            Example::checked(
                "Capture one transactional class snapshot",
                "manager: GameManager = GameManager.instance?.snapshot()?",
                MANAGED_IMAGE_SOURCE,
            ),
            Example::checked(
                "Discover live instances cooperatively",
                "let managers = await GameManager.instances()",
                MANAGED_IMAGE_SOURCE,
            ),
            Example::checked(
                "Read a component from a scene hierarchy",
                "let scene = unity.scenes.active() else return\nlet object = scene.find(\"Managers/GameManager\") else return\nlet manager = object.component<GameManager>() else return",
                MANAGED_IMAGE_SOURCE,
            ),
        ]
    ),
    language_item!(
        ManagedStaticField,
        "static",
        LanguageItemKind::Syntax,
        "static Type field;",
        "Declares a managed field read through its class rather than an instance.",
        "Access the field as `Class.field`. Resolving the field offset and reading its live value are fallible, just like an instance-field hop. Lookup follows the runtime inheritance chain and uses the declaring runtime class's static storage, so a field inherited from a generic base needs no raw path or alternate source spelling. A static field may reference its own class, which is useful for singleton instances such as `GameManager.instance`. Metadata offsets and static storage are bound once per attachment, but the field value itself is read live so a replaced singleton is observed on the next state poll.",
        &[Example::checked(
            "Read a managed singleton",
            "let manager = GameManager.instance else return",
            MANAGED_IMAGE_SOURCE,
        )]
    ),
    language_item!(
        ManagedMetadataNames,
        "from",
        LanguageItemKind::Syntax,
        "class Name from \"MetadataName\" { ... } | Type field from [\"name\", \"fallback\"];",
        "Supplies one or more runtime metadata names for a managed declaration.",
        "Without [`from`], a managed [`class`] or field uses its SplitScript declaration name for metadata lookup. A single quoted name replaces that default. An array describes equivalent runtime names, which supports renamed classes and fields across builds without changing the stable source-facing name. The binder requires one unambiguous match rather than selecting whichever candidate happens to be discovered first. For an instance field without an explicit [`from`], lookup also accepts the conventional C# automatic-property backing-field spelling. Metadata aliases do not create a source-visible shape discriminator; use an ordinary enum global when the source-visible shape or behavior actually differs. Editor rename preserves the effective metadata candidates by inserting an explicit [`from`] clause when changing a declaration whose source name was still implicit.",
        &[Example::checked(
            "Try alternate managed field names",
            "u32 score from [\"_score\", \"<Score>k__BackingField\"];",
            MANAGED_FROM_SOURCE,
        )]
    ),
    language_item!(
        State,
        "state",
        LanguageItemKind::Declaration,
        "state \"game.exe\" { ... } | state Provider { ... } | state { provider Name: Provider { ... }, ... }",
        "Declares process attachment and persistent watched state.",
        "A native string is an exact host process identity. The current Windows host reports executable filenames including `.exe`, so a Windows candidate must include that extension. An array tries alternate executable names in order; it does not attach to several processes at once. A named standard-library provider selects a typed memory model. [`Unity`] binds managed [`image`] schemas while [`GBA`], [`PS1`], [`PS2`], [`SMS`], [`Genesis`], [`GCN`], and [`Wii`] expose emulator-specific read roots and accept original console addresses in state fields. When one autosplitter supports genuinely different runtimes, a `state { provider Name: Provider { ... }, ... }` declaration cooperatively tries the alternatives that accept the attached process and selects the first one that completes. The read-only `provider: StateProvider` value identifies that choice. Fields with compatible declarations in every alternative remain directly available through [`current`] and [`old`]; a direct [`match`] on [`provider`] exposes alternative-only fields and provider roots. Process names shared by alternatives are attached only once. A provider alternative must read directly from an attached process; providers with a prepared schema or attachment context, such as [`Unity`], use the concise single-provider form. Build-specific fields may use an [`if`] / [`else if`](keyword@if) / [`else`](keyword@if) chain over an enum global initialized during [`onAttach`], or over an enum state field when the shape can change while attached. Later branches cover the exact variants left unmatched by earlier branches. Compatible fields declared in every possible branch remain directly available; other fields require the corresponding predicate or [`match`] refinement. Every state expression has one implicit fallible boundary ([`T!`]): internal postfix [`?`] and a fallible final call propagate into that same boundary. Use an ordinary [`value block`] when address discovery or decoding needs several local steps; its final expression supplies the field value without requiring a helper function. A field may use another field from the same active branch by name, including as the base of an [`at`](syntax@at) path. Declaration order is irrelevant: the compiler evaluates dependencies first and rejects cycles. Initialization requires all required fields to succeed in one poll and seeds [`old`] and [`current`] equally without running lifecycle actions. Later, failed fields retain their accepted values while successful independent fields advance; a dependent field is not evaluated when one of its dependencies fails. Deliberately optional reads can discard their error into [`T?`] with [`discardError`](method@Result.discardError).",
        STATE_DECL_EXAMPLES
    ),
    language_item!(
        StateProviderAlternative,
        "provider",
        LanguageItemKind::Syntax,
        "provider Name: Provider [\"process.exe\"] { ... }",
        "Declares one named runtime alternative inside a multi-provider state.",
        "Each [`provider`](syntax@provider) alternative gives a local `StateProvider` variant name, a standard-library state provider, that provider's process configuration, and its physical state fields. Alternatives are tried cooperatively in source order only when they accept the attached process. The first one whose provider discovery completes becomes the attachment's read-only [`provider`] value. Compatible fields declared by every alternative form the common snapshot interface. Match directly on [`provider`] to use alternative-only fields or roots such as [`process`](provider@Native) and [`gba`](provider@GBA). The process-name union is deduplicated before attachment. Alternatives must use providers that read directly from the attached process; a provider with a prepared attachment context, such as [`Unity`], uses the single-provider [`state`] form.",
        &[Example::checked(
            "Refine runtime-specific state",
            "provider Advance: GBA {
    level: u32 at 0x03000010;
    room: u8 at 0x03000020;
}",
            MULTI_PROVIDER_STATE_SOURCE,
        )]
    ),
    language_item!(
        TickRate,
        "tickRate",
        LanguageItemKind::Declaration,
        "tickRate { attached: 60, detached: 2 }",
        "Overrides the lifecycle-owned polling rates.",
        "SplitScript defaults to 1 Hz with no host process and 120 Hz while acquiring or polling a state provider. The active rate is applied before cooperative provider discovery begins, because range and signature scans make bounded progress per update, and remains in effect through [`onAttach`] and attached polling. Whenever the logical attachment ends, the detached rate is restored before [`onDetach`]. For native games this normally follows the process lifetime. Emulator mapping loss ends the logical attachment independently while the host remains open; rediscovery raises the rate again before it starts. Consequently an emulator awaiting a guest currently remains at the active rate when discovery itself is an indefinitely pending scan. Either field may be omitted to retain its default; [`setTickRate`] remains available for temporary dynamic changes until the next lifecycle transition.",
        &[Example::checked(
            "Override lifecycle polling rates",
            "tickRate {\n    attached: 60,\n    detached: 2,\n}",
            TICK_RATE_SOURCE,
        )]
    ),
    language_item!(
        ConditionalStateFields,
        "conditional fields",
        LanguageItemKind::Declaration,
        "let shape: Shape; state Provider { if shape == Shape.Variant { ... } else { ... } }",
        "Declares fields whose availability follows an ordinary enum value.",
        "A bare enum global assigned on every successful [`onAttach`] path is an attachment-scoped shape discriminator. It is frozen after attachment and can guard conditional fields in both [`state`] and managed [`class`] declarations. The [`Unity`] provider can initialize such a global automatically when the active managed metadata uniquely identifies one variant; otherwise [`onAttach`] assigns it explicitly. An enum-valued state field can instead describe a shape that changes while attached. The compiler tracks either kind through [`if`] / [`else if`](keyword@if) / [`else`](keyword@if), direct [`match`], and [`is`] conditions. Compatible fields present with the same type in every possible branch form the common snapshot interface. Other fields are available only where their predicate has been proven. Dynamic branch transitions are transactional: newly active fields must all read successfully, and their first accepted values seed [`old`] and [`current`] equally.",
        &[
            Example::checked(
                "Select and refine a supported build shape",
                "if build == Build.Steam {\n    level: u32 at 0x1000;\n    checkpoint: u8 at 0x1100;\n} else {\n    level: u32 at 0x2000;\n    checkpoint: u16 at 0x2100;\n}",
                MULTI_LAYOUT_STATE_SOURCE,
            ),
            Example::checked(
                "Refine managed fields with the shared discriminator",
                "if edition == Edition.BaseGame {\n    print(manager.level else 0)\n} else {\n    print(manager.scene else 0)\n}",
                MANAGED_IMAGE_SOURCE,
            ),
        ]
    ),
    language_item!(
        StatePointerField,
        "at",
        LanguageItemKind::Syntax,
        "field: T at module-or-field, offset, ... | field: T? at module-or-field, offset, ...",
        "Reads a persistent state field through a pointer path.",
        "A string selects a module-relative pointer base, an integer selects an absolute base, and a sibling field name uses that field's candidate value as a dynamic base. Each following integer is an address offset. Sibling references are independent of declaration order, must stay within the same active conditional branch, and may also appear in expression-backed fields. The compiler evaluates their dependency graph in order and rejects cycles. If a dependency fails, the dependent read is skipped and retains its previous accepted value. A required `T` field is a [`T!`] boundary: initialization waits for it, while a later failed read retains its last accepted value. An explicitly optional [`T?`] field instead accepts its own read failure as [`None`] and a successful read as [`Some`]`(T)`, so absence is observable in [`current`] and [`old`]. The exact memory representation must be explicit or inferred from an exact use; optional read semantics require the [`T?`] annotation. A memory-readable [`[T; N]`] field reads the complete contiguous array in one operation and is limited to 4,096 elements and 65,536 bytes. For a larger region when only selected values are needed, use an expression-valued state field that constructs a growable [`[T]`] from focused reads instead of declaring one oversized fixed array.",
        STATE_POINTER_EXAMPLE
    ),
    language_item!(
        NativeStringDecoder,
        "utf8",
        LanguageItemKind::Syntax,
        "field at address as utf8(maxBytes)",
        "Decodes a bounded native UTF-8 string state field.",
        "This is state-field sugar for a bounded read-and-decode operation, not a string-size type. It follows the complete pointer path, reads at most 4096 bytes once, and stops at the first NUL byte. A required field rejects its candidate when memory cannot be read or the bytes are not valid UTF-8; an explicitly annotated optional [`String`] ([`T?`]) field observes that failure as [`None`]. Without the optional annotation, the field type is inferred as [`String`].",
        NATIVE_STRING_DECODER_EXAMPLE
    ),
    language_item!(
        NativeUtf16LeDecoder,
        "utf16le",
        LanguageItemKind::Syntax,
        "field at address as utf16le(maxUtf16Units)",
        "Decodes a bounded native UTF-16LE string state field.",
        "This state-field sugar follows the complete pointer path, reads at most 2048 little-endian UTF-16 code units once, and stops at the first NUL code unit. Unpaired surrogate code units become the Unicode replacement character. A required field rejects its candidate when memory cannot be read; an explicitly annotated optional [`String`] ([`T?`]) field observes that failure as [`None`]. Without the optional annotation, the field type is inferred as [`String`].",
        NATIVE_UTF16LE_DECODER_EXAMPLE
    ),
    language_item!(
        Settings,
        "settings",
        LanguageItemKind::Declaration,
        "settings { \"Group\" { \"Label\" => name key \"host-key\": value } }",
        "Declares live user settings.",
        "Settings support nested headings, [`///`] tooltips, booleans, [`text input setting`] strings, [`choice setting`] values, and [`file setting`] selectors. An optional [`stable setting key`] is the exact string stored in the host settings map; otherwise the source identifier is used. [`settings`] and [`oldSettings`] refresh every update.",
        SETTINGS_DECL_EXAMPLE
    ),
    language_item!(
        SettingFamily,
        "settings family",
        LanguageItemKind::Syntax,
        "for value in start..=end { `label {value}` key `{value}`: default }",
        "Declares a finite family of boolean settings at compile time.",
        "The inclusive [`u32`] range is expanded during compilation into ordinary host settings. Label and key templates may interpolate only the loop binding. Generated entries deliberately have no statically named settings member; query them with [`SettingsView.enabled`]. A [`///`] comment on the family becomes every generated setting's tooltip, and nesting it in a quoted group preserves the normal heading structure.",
        SETTINGS_FAMILY_EXAMPLE
    ),
    language_item!(
        StableSettingKey,
        "stable setting key",
        LanguageItemKind::Syntax,
        "\"Label\" => name key \"host-key\": value",
        "Assigns an explicit stable key in the host settings map.",
        "The quoted key is used for persistent host storage and dynamic [`SettingsView.enabled`] lookups. The source identifier remains the statically typed member exposed through [`settings`] and [`oldSettings`]. Without [`key`](syntax@stable setting key), the source identifier is also the host key. Editor rename inserts that previous identifier as an explicit [`key`](syntax@stable setting key), preserving saved settings while allowing the local member name to change.",
        SETTING_KEY_EXAMPLE
    ),
    language_item!(
        If,
        "if",
        LanguageItemKind::Keyword,
        "if condition { ... } else { ... }",
        "Branches as a statement or expression.",
        "Expression-valued [`if`] requires an [`else`] branch and infers both branch values against one result type. A branch may be a [`value block`] when it needs local steps before producing its value.",
        IF_EXAMPLES
    ),
    language_item!(
        ValueBlock,
        "value block",
        LanguageItemKind::Syntax,
        "{ statements; finalExpression }",
        "Runs scoped statements and yields its final expression.",
        "A value block may appear anywhere an expression is accepted, including an [`if`] branch, [`match`] arm, fallback [`else`], function argument, or state-field initializer. Its bindings are local to the block. The final expression supplies the block's value; a block without one yields [`None`], unless control always leaves through [`return`], [`break`], [`continue`], [`throw`], or another [`Never`] expression. A semicolon after the final expression is accepted for familiarity but warned about and removed by the formatter because the expression is still the value. Function and lifecycle bodies are statement blocks, not value blocks: use [`return`] explicitly when they return a value.",
        VALUE_BLOCK_EXAMPLES
    ),
    language_item!(
        Equality,
        "==",
        LanguageItemKind::Syntax,
        "left == right",
        "Compares two values for equality.",
        "Equality is typed and never coerces between unrelated representations. [`String`] values compare by text content, while structs and enums compare structurally when their contents satisfy [`Equatable`].",
        EQUALITY_EXAMPLE
    ),
    language_item!(
        Inequality,
        "!=",
        LanguageItemKind::Syntax,
        "left != right",
        "Compares two values for inequality.",
        "Inequality is the typed negation of [`==`] and supports the same exact types and structural comparisons through [`Equatable`].",
        INEQUALITY_EXAMPLE
    ),
    language_item!(
        Else,
        "else",
        LanguageItemKind::Keyword,
        "value else fallback",
        "Provides a branch or unwrap fallback.",
        "After [`if`], [`else`] selects the alternate branch. After a [`T?`] or [`T!`] expression, it unwraps success or evaluates its right operand on absence or error. That operand is any ordinary expression; [`return`], [`break`], [`continue`], and [`throw`] work naturally because each has type [`Never`]. Chained fallbacks associate to the right.",
        ELSE_EXAMPLES
    ),
    language_item!(
        While,
        "while",
        LanguageItemKind::Keyword,
        "while condition { ... }",
        "Repeats a block while its condition is true.",
        "The condition must be [`bool`] and is evaluated before every iteration. The loop body has its own lexical scope. In [`onAttach`], [`await`] and [`retry`] resume through explicit loop-header and exit states without replaying completed iterations.",
        WHILE_EXAMPLE
    ),
    language_item!(
        Loop,
        "loop",
        LanguageItemKind::Keyword,
        "loop { ... }",
        "Repeats a block until it breaks.",
        "Without a reachable [`break`], [`loop`] has type [`Never`] and cannot fall through. `break value` exits the nearest [`loop`] expression and supplies its result; all such values are inferred bidirectionally as one type. A bare [`break`] supplies [`None`]. Value-carrying breaks are deliberately unavailable in [`while`] and runtime [`for`] loops. [`continue`] starts the next iteration. In an async context, [`await`] and [`retry`] preserve the loop and its live values across ticks.",
        LOOP_EXAMPLES
    ),
    language_item!(
        For,
        "for",
        LanguageItemKind::Keyword,
        "for value in iterable { ... }",
        "Iterates over an array, set, or integer range.",
        "The iterable expression is evaluated exactly once. The read-only element may be an irrefutable [`binding pattern`] lexically scoped to the body and inferred from [`[T]`], [`[T; N]`], [`Set`], or an integer [`range`]. [`break`] and [`continue`] target the nearest loop. In [`onAttach`], a body containing [`await`] or [`retry`] preserves the iterable and all projected bindings across suspension. Inside [`settings`], an inclusive range expands a [`settings family`] at compile time instead of creating a runtime loop.",
        FOR_EXAMPLES
    ),
    language_item!(
        Range,
        "range",
        LanguageItemKind::Syntax,
        "start..<end | start..=end | T..<T | T..=T",
        "Describes an explicitly exclusive or inclusive integer interval.",
        "[`..<`](syntax@range) excludes the upper endpoint and [`..=`](syntax@range) includes the upper endpoint. Bare `..` is rejected as a range so endpoint inclusion never depends on remembered language convention; inside an array pattern, it is the separate [`array rest pattern`] syntax. In an expression, the bounds create a first-class range value; the corresponding type repeats its bound type around the same operator. In a [`match`] pattern, integer literal bounds test that interval directly and may be combined with `|`. Both bounds and the matched value have one exact [`Integer`] type. Empty or reversed range patterns are errors, while reversed range values are empty. Direct [`for`] iteration does not allocate a range object; storing or passing the range preserves it as an immutable value.",
        RANGE_EXAMPLES
    ),
    language_item!(
        ArrayRestPattern,
        "array rest pattern",
        LanguageItemKind::Syntax,
        "[prefix, .., suffix]",
        "Matches a variable-length middle of an array.",
        "Inside an array [`match`] pattern, `..` matches zero or more elements between the explicit prefix and suffix. There may be at most one rest marker. It does not bind or copy the skipped elements. `[first, ..]` matches every nonempty array, `[.., last]` matches from the end, and `[..]` matches every length. Exact patterns without `..` still require exactly their written length. For [`[T; N]`], the compiler checks that the explicit prefix and suffix fit in `N`; for growable [`[T]`], the runtime checks the minimum length before reading either side.",
        ARRAY_REST_PATTERN_EXAMPLE
    ),
    language_item!(
        Break,
        "break",
        LanguageItemKind::Keyword,
        "break | break value",
        "Exits the nearest enclosing loop.",
        "[`break`] is an ordinary [`Never`]-typed expression which exits the innermost [`while`], runtime [`for`], or [`loop`]. Inside a [`loop`] expression, `break value` also supplies the expression's result; a bare break produces [`None`]. Value-carrying breaks are rejected in [`while`] and [`for`] so they can never accidentally skip an inner statement loop to target an outer value loop. Because it never yields locally, [`break`] can appear in any expression position whose evaluation may exit that loop, including a fallback [`else`].",
        BREAK_EXAMPLES
    ),
    language_item!(
        Continue,
        "continue",
        LanguageItemKind::Keyword,
        "continue",
        "Starts the next iteration of the nearest enclosing loop.",
        "[`continue`] is an ordinary [`Never`]-typed expression. It may appear anywhere an expression is accepted inside a loop, including a fallback [`else`]; evaluating it starts the next iteration instead of yielding a local value. The loop condition is then evaluated again.",
        CONTINUE_EXAMPLE
    ),
    language_item!(
        Debug,
        "debug",
        LanguageItemKind::Keyword,
        "debug statement",
        "Keeps a development-only statement in debug builds.",
        "Debug statements, bindings, globals, and `debug fn` declarations are fully parsed and type-checked in every profile, then erased from release lowering before dependency and reachability discovery. Debug-only names may only be used from debug code. Terminating statements remain rejected. A standalone [`inspect`] statement is implicitly debug-only; when its value is consumed, release builds retain the operand and erase only publication. Use [`print`](fn@print) for a chronological diagnostic log and [`setVariable`](fn@setVariable) for meaningful end-user-facing information displayed on the speedrun timer in both profiles.",
        DEBUG_EXAMPLE
    ),
    language_item!(
        Inspect,
        "inspect",
        LanguageItemKind::Keyword,
        "inspect(expression)",
        "Publishes a live debug value and returns it unchanged.",
        "[`inspect`] evaluates its operand once, temporarily reuses the variables panel to display the value under the operand's trimmed source spelling in debug builds, and produces that same typed value. When a surrounding expression consumes the result, release lowering keeps the operand but removes inspection formatting and its runtime dependency. A standalone `inspect(expression)` statement is implicitly debug-only: release builds omit the complete statement and do not evaluate `expression` or any of its side effects. Declarations reached only by standalone inspections receive [`debug`]-modifier guidance. The operand must implement [`Display`]. Use [`setVariable`](fn@setVariable) for intentional end-user-facing information displayed on the speedrun timer in both profiles and [`print`](fn@print) for a chronological diagnostic log.",
        INSPECT_EXAMPLE
    ),
    language_item!(
        Match,
        "match",
        LanguageItemKind::Keyword,
        "match value { pattern => expression }",
        "Exhaustively matches a value.",
        "[`match`] supports enum payloads, partial field-based [`struct`] patterns, optional [`None`]/[`Some`]`(value)` patterns, iterator [`End`]/[`Item`]`(value)` patterns, fallible [`Err`]`(error)`/[`Ok`]`(value)` patterns, recursive exact and [`array rest pattern`]s, string, character, integer, boolean, and file-version literals, closed integer [`range`] patterns, guards, a wildcard, and recursive `left | right` alternatives. A struct pattern ignores omitted fields; `Name { field }` binds that field, while `Name { field: pattern }` recursively tests it. Because a match scrutinee supplies a concrete type, the same patterns may usually be shortened to `{ field }` and `{ field: pattern }`; they are still nominal and never select a struct merely by its field names. Alternatives are tried left to right and contribute their union to exhaustiveness. Every alternative in one arm must bind exactly the same names with compatible types; those occurrences form one logical binding for the guard and body. Array elements can bind values or contain any other pattern. A [`[T; N]`] exact pattern must have exactly `N` elements; `..` instead permits an omitted middle, including in growable [`[T]`] patterns. String patterns compare contents, not WebAssembly GC identities. Enum, wrapper, and array matches are checked recursively for exhaustiveness; guarded arms do not establish coverage.",
        MATCH_EXAMPLES
    ),
    language_item!(
        Is,
        "is",
        LanguageItemKind::Keyword,
        "expression is pattern",
        "Tests a value against a pattern.",
        "[`is`] evaluates its left operand exactly once and returns [`bool`]. It accepts the same recursive patterns as [`match`], but a mismatch simply produces `false` and therefore needs no exhaustive fallback. Bindings exist only on control-flow paths where the match is proven: the true edge of a direct test, the false edge after [`!`], the following operand of `&&` or `||` when short-circuiting proves it, and the corresponding [`if`] branch, [`while`] body, or guarded [`match`] arm. Storing or passing the boolean discards that proof. Write `!(value is pattern)` to negate the complete test.",
        IS_EXAMPLES
    ),
    language_item!(
        Return,
        "return",
        LanguageItemKind::Keyword,
        "return expression",
        "Returns from the current function or action.",
        "[`return`] is an ordinary [`Never`]-typed expression, so it may appear anywhere an expression is accepted and satisfies any surrounding expected type by leaving the current function or action instead of yielding locally. Functions infer their result from explicit [`return`] expressions and call-site constraints. Unlike a nested [`value block`], a function or lifecycle body never returns its final expression implicitly. Lifecycle actions apply their domain default when control falls through.",
        RETURN_EXAMPLE
    ),
    language_item!(
        Throw,
        "throw",
        LanguageItemKind::Keyword,
        "throw error",
        "Transfers an error to the nearest failure boundary.",
        "[`throw`] is an ordinary [`Never`]-typed expression. Inside [`retry`], it ends the current attempt and retries the complete operand on the next attached update. Otherwise it returns an error from the enclosing [`T!`] function or rejects the current state-field value. The error expression must be a [`String`]. Because it never yields locally, it can be used directly as a fallback [`else`] operand or in any other expression position.",
        THROW_EXAMPLE
    ),
    language_item!(
        Async,
        "async",
        LanguageItemKind::Keyword,
        "fn name() -> async T { ... }",
        "Marks an explicitly typed function result as asynchronous.",
        "A function containing [`await`] or [`retry`] has an async result. Write [`async`] `T` when its result type is explicit; when the result type is omitted, both [`async`] and `T` are inferred. Calling a source-defined async function creates an attachment-lifetime future value without polling it. That [`async`] `T` value can be stored in locals and aggregates, passed to functions, and awaited later. Its typed continuation frame retains parameters, live locals, nested futures, and the completed `T`. Futures cannot escape into globals because state-provider detachment owns their cancellation.",
        ASYNC_RESULT_EXAMPLE
    ),
    language_item!(
        IteratorType,
        "iterator",
        LanguageItemKind::Keyword,
        "fn name(...) -> iterator T { ... } | (...) -> iterator T => { ... }",
        "Names the type-erased synchronous iterator over `T`.",
        "Arrays, sets, ranges, maps, adapters, and generator bodies expose the same [`iterator`] `T` type; their representation-specific cursor types are private standard-library details. A function or closure may return any existing iterator with ordinary [`return`]. If its body contains [`yield`], the body instead becomes a lazy generator: calling it allocates a cursor without running the body, and each [`Iterator.next`] resumes until one yield. Fallthrough or a bare return permanently returns [`End`]. Copies alias the same cursor position, and the value implements both [`Iterator`] and identity [`Iterable`], so it composes with [`for`], [`map`], and [`filter`]. Generators are synchronous: [`await`], [`retry`], value-returning return, and uncaught fallible control are rejected rather than silently changing the iterator protocol.",
        ITERATOR_TYPE_EXAMPLE
    ),
    language_item!(
        Yield,
        "yield",
        LanguageItemKind::Keyword,
        "yield expression",
        "Suspends a generator after producing one item.",
        "[`yield`] is available only inside a function or closure explicitly returning [`iterator`] `T`. Its expression must have type `T`. The generator preserves its parameters, captures, and live locals, and the next [`Iterator.next`] continues immediately after the yield. It is distinct from [`return`]: a bare return exhausts the cursor, while returning a value is an error.",
        YIELD_EXAMPLE
    ),
    language_item!(
        Await,
        "await",
        LanguageItemKind::Keyword,
        "let value = await operation",
        "Waits for an asynchronous value and yields its result.",
        "[`await`] is an ordinary prefix expression available in [`onAttach`] and source-defined [`async`] helpers. It accepts any [`async`] `T` expression, yields `T`, and can be nested in calls, operators, member access, conditionals, matches, fallbacks, and loop conditions. Source future values may be stored and awaited repeatedly; an already completed future yields its retained result without rerunning its body. The attachment-lifetime continuation tree is cancelled when the state provider detaches, including when an emulator mapping disappears without its host process closing.",
        AWAIT_EXAMPLE
    ),
    language_item!(
        Retry,
        "retry",
        LanguageItemKind::Keyword,
        "let value = retry fallibleExpression | let value = retry { ... }",
        "Retries synchronous fallible work until it succeeds.",
        "[`retry`] creates a local error boundary around any ordinary expression. The complete operand is evaluated again once per attached update. A [`T!`] error, postfix [`?`], or [`throw`] ends only the current attempt; a successful final value yields `T`. A value block is not a special retry form: braces are ordinary expressions, so `retry { ... }` naturally retries every statement and its final expression. [`return`], [`break`], and [`continue`] keep their normal lexical targets. Like [`await`], [`retry`] binds more tightly than fallback [`else`]. Adjacent unparenthesized forms warn because `(retry value) else fallback` and `retry (value else fallback)` establish different boundaries. The attempt itself must remain synchronous and bounded: evaluating [`await`] or another [`retry`] inside it is rejected, while merely calling an [`async`] function to construct a future is allowed. A containing function infers an [`async`] result unless its result type is explicit, in which case write `-> async T`. Process closure cancels the retry boundary.",
        RETRY_EXAMPLES
    ),
    language_item!(
        Propagate,
        "?",
        LanguageItemKind::Syntax,
        "resultExpression?",
        "Propagates a [`T!`] error.",
        "Postfix [`?`] unwraps success or transfers the original error to the nearest failure boundary: a [`retry`] operand restarts on the next attached update, a state-field assignment rejects that field update, and a [`T!`] function returns the error. A state expression may use a [`value block`] to perform several local steps; every [`?`] and a fallible final expression still target that one field boundary, so no helper function is required.",
        PROPAGATE_EXAMPLES
    ),
    language_item!(
        AsCast,
        "as",
        LanguageItemKind::Keyword,
        "expression as Type",
        "Explicitly converts a value.",
        "Casts are checked statically. String interpolation uses the same [`Display`] conversion as an explicit [`as`] [`String`] cast.",
        CAST_EXAMPLE
    ),
    language_item!(
        SomeConstructor,
        "Some",
        LanguageItemKind::Syntax,
        "Some(value)",
        "Explicitly constructs a present optional value.",
        "[`Some`] infers `T` from its value and constructs [`T?`]. Plain `T` values still lift automatically whenever [`T?`] is expected.",
        SOME_EXAMPLE
    ),
    language_item!(
        IteratorItem,
        "Item",
        LanguageItemKind::Syntax,
        "Item(value)",
        "Constructs a yielded iterator step.",
        "[`Item`] infers `T` from its value and constructs [`IteratorStep`]`<T>`. Unlike [`Some`], an item may itself contain [`None`] without ending iteration. Match it together with [`End`].",
        ITERATOR_ITEM_EXAMPLE
    ),
    language_item!(
        IteratorEnd,
        "End",
        LanguageItemKind::Syntax,
        "End",
        "Constructs an exhausted iterator step.",
        "[`End`] obtains `T` from surrounding [`IteratorStep`]`<T>` context and marks that an [`Iterator`] has no more items. Match it together with [`Item`].",
        ITERATOR_END_EXAMPLE
    ),
    language_item!(
        SuccessConstructor,
        "Ok",
        LanguageItemKind::Syntax,
        "Ok(value)",
        "Explicitly constructs a successful result value.",
        "[`Ok`] infers `T` from its value and constructs [`T!`]. Plain `T` values still lift automatically whenever [`T!`] is expected.",
        OK_EXAMPLE
    ),
    language_item!(
        ErrorConstructor,
        "Err",
        LanguageItemKind::Syntax,
        "Err(message)",
        "Constructs a [`T!`] error.",
        "[`Err`] takes a [`String`] and obtains its successful `T` type from surrounding [`T!`] context.",
        ERR_EXAMPLE
    ),
    language_item!(
        SelfValue,
        "self",
        LanguageItemKind::Keyword,
        "self",
        "Refers to the current method receiver.",
        "A function declared as [`fn`] `Type.name` receives an implicit, precisely typed [`self`] value.",
        SELF_EXAMPLE
    ),
    language_item!(
        SignatureLiteral,
        "sig",
        LanguageItemKind::Syntax,
        "sig\"48 8B ?? B?\"",
        "Constructs a checked signature literal.",
        "Signatures are parsed at compile time and support full-byte and nibble wildcards.",
        SIGNATURE_EXAMPLE
    ),
    language_item!(
        VersionLiteral,
        "v",
        LanguageItemKind::Syntax,
        "v\"major.minor.build.private\"",
        "Constructs a checked Windows file-version literal.",
        "A version literal contains exactly four decimal [`u16`] components and has type [`FileVersion`]. It may also be used directly as a [`match`] pattern; an open-ended version match requires a wildcard arm. The quoted boundary keeps malformed versions from being parsed as unrelated numeric or member expressions.",
        VERSION_EXAMPLE
    ),
    language_item!(
        TemplateString,
        "template string",
        LanguageItemKind::Syntax,
        "`text {expression}`",
        "Interpolates values into a String.",
        "Backtick strings use braces without JavaScript's dollar marker. A dollar sign is ordinary text, so `${value}` emits `$` followed by the interpolated value rather than being treated as a typo. Non-[`String`] values use the same [`Display`] conversion as an [`as`] [`String`] cast.",
        TEMPLATE_EXAMPLE
    ),
    language_item!(
        ArrayType,
        "[T; N]",
        LanguageItemKind::Syntax,
        "[Element] or [Element; Length]",
        "Names a garbage-collected array type.",
        "[`[T]`] accepts any length. [`[T; N]`] carries an exact compile-time length, can be used wherever [`[T]`] is expected, and has a fixed process-memory layout when `T` satisfies [`MemoryReadable`]. Both forms compare structurally when `T` satisfies [`Equatable`] and support exact recursive [`match`] patterns. Process-memory reads of a fixed array are limited to 4,096 elements and 65,536 bytes so generated code and host-memory traffic remain bounded. When a larger native region only needs sparse values, construct a growable [`[T]`] in an expression-valued [`state`] field from focused reads instead of declaring an oversized [`[T; N]`].",
        ARRAY_TYPE_EXAMPLE
    ),
    language_item!(
        ArrayIndex,
        "array indexing",
        LanguageItemKind::Syntax,
        "array[index]",
        "Reads an array element.",
        "The receiver may be [`[T]`] or [`[T; N]`], the index is inferred as [`u32`], and the result has type `T`. WebAssembly performs the bounds check.",
        ARRAY_INDEX_EXAMPLE
    ),
    language_item!(
        OptionType,
        "T?",
        LanguageItemKind::Syntax,
        "Type?",
        "Names an optional type.",
        "A [`T?`] contains either [`Some`]`(T)` or [`None`]. Plain values lift to [`Some`], and [`match`] uses [`Some`]`(value)` plus [`None`].",
        OPTION_TYPE_EXAMPLE
    ),
    }
    builtins {
    builtin_type_item!(
        Never,
        "Never",
        "Describes an expression that cannot produce a value.",
        "The [`Never`] type is the bottom of SplitScript's type hierarchy and can flow into any expected value type. It is inferred for genuinely divergent control flow and is erased from WebAssembly. [`Process.closed`] returns [`async`] [`Never`] because attachment-lifetime cancellation prevents its await from resuming.",
        "fn ignoreUnsupportedBuild() -> async Never {\n    await process.closed()\n}"
    ),
    builtin_type_item!(
        None,
        "None",
        "Stores the single unit value [`None`].",
        "None is the ordinary return type for procedures and functions without a returned data value. Plain unit parameters, results, locals, globals, and async completions are erased from the physical Wasm representation. The same value converts to the empty side of T?, while Some(None) remains a present unit value.",
        "let unit: None = None"
    ),
    builtin_type_item!(
        Bool,
        "bool",
        "Stores a boolean value.",
        "Boolean values are true or false and are required by conditions and lifecycle decision blocks.",
        "let isBoss = current.level == 7"
    ),
    builtin_type_item!(
        Char,
        "char",
        "Stores one Unicode scalar value.",
        "Character literals use single quotes and contain exactly one Unicode scalar value. A char is distinct from u32, implements Display and equality, converts losslessly to u32 with [`as`], and has no implicit process-memory encoding.",
        "let marker: char = 'ß'"
    ),
    builtin_type_item!(
        I8,
        "i8",
        "Stores an 8-bit signed integer.",
        "Fixed-width integers make process-memory layouts and numeric bounds explicit.",
        "let direction: i8 = -1"
    ),
    builtin_type_item!(
        U8,
        "u8",
        "Stores an 8-bit unsigned integer.",
        "Fixed-width integers make process-memory layouts and numeric bounds explicit.",
        "let flags: u8 = 0xff"
    ),
    builtin_type_item!(
        I16,
        "i16",
        "Stores a 16-bit signed integer.",
        "Fixed-width integers make process-memory layouts and numeric bounds explicit.",
        "let temperature: i16 = -20"
    ),
    builtin_type_item!(
        U16,
        "u16",
        "Stores a 16-bit unsigned integer.",
        "Fixed-width integers make process-memory layouts and numeric bounds explicit.",
        "let room: u16 = 12"
    ),
    builtin_type_item!(
        I32,
        "i32",
        "Stores a 32-bit signed integer.",
        "Unconstrained integer literals and values specifically constrained as Integer default to i32. Memory reads never use this default and require an explicit or otherwise exact representation.",
        "let health: i32 = process.read(healthAddress) else 0"
    ),
    builtin_type_item!(
        U32,
        "u32",
        "Stores a 32-bit unsigned integer.",
        "Fixed-width integers make process-memory layouts and numeric bounds explicit.",
        "let character: u32 = process.read(characterAddress) else 0"
    ),
    builtin_type_item!(
        I64,
        "i64",
        "Stores a 64-bit signed integer.",
        "Fixed-width integers make process-memory layouts and numeric bounds explicit.",
        "let frameCount: i64 = 7200"
    ),
    builtin_type_item!(
        U64,
        "u64",
        "Stores a 64-bit unsigned integer.",
        "Fixed-width integers make process-memory layouts and numeric bounds explicit.",
        "let sectionOffset: u64 = 0x1_0000"
    ),
    builtin_type_item!(
        Address,
        "address",
        "Stores a process address.",
        "Addresses use the target process pointer width and support checked address-oriented APIs without becoming ordinary untyped integers.",
        "let player: address = retry process.follow(base, offsets)"
    ),
    builtin_type_item!(
        F32,
        "f32",
        "Stores a 32-bit floating-point number.",
        "Floating-point values are useful for game coordinates, timers, and duration conversion. Decimal exponents and finite subnormal values are supported; target-width underflow and overflow are diagnosed.",
        "let smallestPositive: f32 = 1e-45"
    ),
    builtin_type_item!(
        F64,
        "f64",
        "Stores a 64-bit floating-point number.",
        "Floating-point values are useful for game coordinates, timers, and duration conversion. Decimal exponents and finite subnormal values are supported. Unconstrained floating-point literals and values specifically constrained as Float default to f64. Memory reads never use this default and require an explicit or otherwise exact representation.",
        "let smallestPositive: f64 = 5e-324"
    ),
    }
    compiler_symbols {
    compiler_symbol_item!(
        LanguageItemId::CurrentSnapshot,
        "current",
        LanguageItemKind::SnapshotRoot,
        "current.stateField",
        "Accesses the current committed state snapshot.",
        "State fields refresh before [`whileAttached`] and timer-decision actions run. Direct assignment replaces a field for the remainder of this tick and the resulting snapshot becomes [`old`] on the next successful poll. A failed field retains its last accepted value.",
        "current.level = 1",
        STATE_SOURCE
    ),
    compiler_symbol_item!(
        LanguageItemId::OldSnapshot,
        "old",
        LanguageItemKind::SnapshotRoot,
        "old.stateField",
        "Accesses the previous committed state snapshot.",
        "[`old`] contains the preceding emitted snapshot. A rejected field remains unchanged while successful sibling fields can advance into [`current`].",
        "return current.level != old.level",
        STATE_SOURCE
    ),
    compiler_symbol_item!(
        LanguageItemId::OldSettingsView,
        "oldSettings",
        LanguageItemKind::SnapshotRoot,
        "oldSettings.settingName",
        "Accesses the previous settings view.",
        "[`settings`] refreshes on every update; [`oldSettings`] retains the preceding values for change detection.",
        "let changed = settings.enabled != oldSettings.enabled",
        SETTINGS_SOURCE
    ),
    }
    ordinary_after {
    language_item!(
        ResultType,
        "T!",
        LanguageItemKind::Syntax,
        "Type!",
        "Names a fallible result type.",
        "A [`T!`] contains either [`Ok`]`(T)` or a [`String`] error. Plain values lift to [`Ok`], and [`match`] uses [`Ok`]`(value)` plus [`Err`]`(error)`.",
        RESULT_TYPE_EXAMPLE
    ),
    language_item!(
        DocumentationComment,
        "///",
        LanguageItemKind::Syntax,
        "/// documentation text",
        "Documents a source declaration, state field, setting, or heading.",
        "On functions and methods, global variables, state fields, structs and their fields, and enums and their variants, the documentation appears in editor hovers. On settings and headings, it becomes a tooltip in the settings UI. Consecutive documentation-comment lines form paragraphs; use an empty [`///`] line to start a new paragraph.",
        DOCUMENTATION_COMMENT_EXAMPLES
    ),
    language_item!(
        TextInputSetting,
        "text input setting",
        LanguageItemKind::Syntax,
        "\"Label\" => name: \"default value\"",
        "Declares a free-form text-input setting.",
        "The quoted value is both the initial default and the signal that this declaration uses a text-input widget. The live [`String`] is available through [`settings`] and its previous-tick value through [`oldSettings`]. An existing host value takes precedence over the default.",
        TEXT_INPUT_SETTING_EXAMPLE
    ),
    language_item!(
        ChoiceSetting,
        "choice setting",
        LanguageItemKind::Syntax,
        "\"Label\" => name: choice { \"Option\" => Enum.Variant default }",
        "Declares an enum-backed setting choice.",
        "Exactly one option may carry `default`; every option maps to a variant of one inferred [`enum`] type.",
        CHOICE_EXAMPLE
    ),
    language_item!(
        FileSetting,
        "file setting",
        LanguageItemKind::Syntax,
        "\"Label\" => name: file { \"Files\" => \"*.ext\" mime => \"type/*\" }",
        "Declares a file-selection setting.",
        "File settings support named glob filters, a wildcard fallback, and one or more MIME filters. A selected file is stored as an absolute path in the runtime's portable filesystem namespace and can be passed to [`File.readAllBytes`] or [`File.readAllText`]. The host filesystem is currently mounted read-only below `/mnt`: Windows `C:\\foo\\bar.txt` becomes `/mnt/c/foo/bar.txt`, while Linux or macOS `/foo/bar.txt` becomes `/mnt/foo/bar.txt`.",
        FILE_EXAMPLE
    ),
    }
    actions {
    action_item!(
        Setup,
        Setup,
        "setup",
        "Initializes one loaded script instance.",
        "Runs once from the module start entry point, after globals and [`settings`] are initialized and refreshed. The autosplitting runtime defers that entry point until the beginning of the first interruptible update. Settings and process-independent operations are available, but process providers, [`current`], [`old`], and suspension are not.",
        "setup {\n    print(\"Autosplitter loaded\")\n}"
    ),
    action_item!(
        SelectProcess,
        SelectProcess,
        "selectProcess",
        "Chooses among same-name process candidates.",
        "Runs synchronously for each candidate before provider setup and [`onAttach`]. The candidate is exposed as [`process`](provider@Native). Return `true` to accept it or `false` to try another candidate. This block is an implicit error boundary: postfix [`?`] and [`throw`] reject only the current candidate. Falling through also rejects the candidate. State snapshots, provider roots, and attachment-scoped globals are not initialized yet.",
        "selectProcess {\n    let path = process.path()?\n    return path.endsWith(\"/wanted/game.exe\")\n}",
        related: &[LanguageItemId::ResultType, LanguageItemId::Propagate, LanguageItemId::Throw, LanguageItemId::OnAttach]
    ),
    action_item!(
        OnDetach,
        OnDetach,
        "onDetach",
        "Handles the end of a successfully initialized state-provider attachment.",
        "Runs synchronously once when a logical attachment whose [`onAttach`] completed ends, after its provider state, attachment-scoped globals, and pending continuations are cleared. Usually this happens when the host process closes; an emulator provider also detaches when its game mapping disappears, even if the emulator remains open. It does not run when attachment initialization was still pending or rejected through postfix [`?`] or [`throw`], and it never runs for the initial detached state; use [`setup`] for one-time script initialization. Process providers and state snapshots are unavailable.",
        "onDetach {\n    timer.pauseGameTime()\n}"
    ),
    action_item!(
        OnAttach,
        OnAttach,
        "onAttach",
        "Initializes one state-provider attachment.",
        "This action is implicitly suspending and owns attachment-lifetime cancellation for [`await`] and [`retry`] continuations. It is also an implicit error boundary: postfix [`?`] or [`throw`] rejects this attachment, keeps its host process inert until it closes, and never runs [`onDetach`] for the incomplete attachment. A bare global assigned on every successful path becomes attachment-scoped and may select conditional [`state`] and managed [`class`] fields. Attachment-scoped globals are frozen after this action completes. Emulator providers run this action again when a newly discovered game mapping replaces one that disappeared without closing the emulator.",
        "onAttach {\n    let module = await process.module(\"GameAssembly.dll\")\n}",
        related: &[LanguageItemId::State, LanguageItemId::ConditionalStateFields, LanguageItemId::Await, LanguageItemId::Retry, LanguageItemId::Propagate, LanguageItemId::Throw, LanguageItemId::OnStateReady]
    ),
    action_item!(
        OnStateReady,
        OnStateReady,
        "onStateReady",
        "Initializes one committed state snapshot.",
        "Runs synchronously once per attachment, immediately after the first complete state poll. Both [`old`] and [`current`] are available and equal. The attached process is available, but suspension is not. [`whileAttached`] and timer-decision actions begin on the following update.",
        "onStateReady {\n    print(`Initial level: {current.level}`)\n}"
    ),
    action_item!(
        OnStart,
        OnStart,
        "onStart",
        "Reacts after the timer starts.",
        "Runs once when consecutive updates observe the timer leave [`TimerState.NotRunning`]. A bare global assigned on every completing path becomes attempt-scoped: it remains live across process detach and is cleared after [`onReset`]. The first update establishes a baseline without firing, so loading during an active attempt does not synthesize initialization. Observation happens after settings refresh but before process attachment and state polling, so this action can run while detached. Process providers, attachment-scoped globals, [`current`], and [`old`] are unavailable. A start requested by this script is observed on the following update rather than invoking this action directly from the [`start`] decision.",
        "let elapsed\n\nonStart {\n    elapsed = 0.0\n}"
    ),
    action_item!(
        OnReset,
        OnReset,
        "onReset",
        "Reacts after the timer resets.",
        "Runs once when consecutive updates observe the timer enter [`TimerState.NotRunning`]. Attempt-scoped globals remain available during this action and are cleared after it completes. The first update establishes a baseline without firing. Observation happens after settings refresh but before process attachment and state polling, so the action remains available while detached. Process providers, attachment-scoped globals, [`current`], and [`old`] are unavailable. A reset requested by this script is observed on the following update rather than invoking this action directly from the [`reset`] decision.",
        "onReset {\n    print(\"Attempt reset\")\n}"
    ),
    action_item!(
        WhileAttached,
        WhileAttached,
        "whileAttached",
        "Runs on every initialized attached update.",
        "State and [`settings`] data has already refreshed when this action runs. The initialization poll is deliberately skipped. This action may use [`await`] or [`retry`]; at most one invocation is in flight for an attachment, it is polled once per attached update, and process closure cancels it. While it is pending, every timer-decision action is skipped. Completion does not start another invocation until the next update. Falling through or returning `true` continues to the timer-decision actions; returning `false` skips all of them for the completion update.",
        "let marker\n\nonAttach {\n    marker = await process.scanMemory(sig\"10 08 ?? ??\")\n}\n\nwhileAttached {\n    if (process.read<u16>(marker) else 0) != 0x0810 {\n        marker = await process.scanMemory(sig\"10 08 ?? ??\")\n        return false\n    }\n}",
        related: &[LanguageItemId::OnAttach, LanguageItemId::Await, LanguageItemId::Retry]
    ),
    action_item!(
        Start,
        Start,
        "start",
        "Decides whether to start the timer.",
        "Falling through returns `false`.",
        "start {\n    return current.inGame && !old.inGame\n}"
    ),
    action_item!(
        Split,
        Split,
        "split",
        "Decides whether to advance the current split.",
        "Falling through returns `false`.",
        "split {\n    return current.level != old.level\n}"
    ),
    action_item!(
        Reset,
        Reset,
        "reset",
        "Decides whether to reset the timer.",
        "Falling through returns `false`.",
        "reset {\n    return current.newGame && !old.newGame\n}"
    ),
    action_item!(
        IsLoading,
        IsLoading,
        "isLoading",
        "Reports loading state when known.",
        "Falling through returns [`None`] so the runtime leaves the current loading state unchanged.",
        "isLoading {\n    return current.scene == \"Loading\"\n}"
    ),
    action_item!(
        GameTime,
        GameTime,
        "gameTime",
        "Reports the current game time when known.",
        "Falling through returns [`None`] so the runtime leaves the current game time unchanged.",
        "gameTime {\n    return Duration.fromSeconds(current.gameTime)\n}"
    ),
    }
}

#[derive(Debug, Clone, Copy, Default)]
pub struct LanguageCatalog;

impl LanguageCatalog {
    pub const fn new() -> Self {
        Self
    }

    pub fn items(self) -> impl ExactSizeIterator<Item = &'static LanguageItem> {
        ITEMS.iter()
    }

    pub fn item(self, id: LanguageItemId) -> &'static LanguageItem {
        ITEMS
            .iter()
            .find(|item| item.id == id)
            .expect("every language item ID must have a catalog entry")
    }

    pub fn item_by_name(self, name: &str) -> Option<&'static LanguageItem> {
        ITEMS.iter().find(|item| item.name == name)
    }

    /// Returns source insertion metadata only for items that have a meaningful
    /// unqualified spelling. Context-independent documentation concepts and
    /// infix/postfix syntax deliberately return `None`.
    pub const fn completion(self, id: LanguageItemId) -> Option<LanguageCompletion> {
        let (site, insert_text, is_snippet) = match id {
            LanguageItemId::Let => (
                LanguageCompletionSite::Statement,
                "let ${1:name} = ${2:value}",
                true,
            ),
            LanguageItemId::If => (
                LanguageCompletionSite::Expression,
                "if ${1:condition} {\n    $0\n}",
                true,
            ),
            LanguageItemId::While => (
                LanguageCompletionSite::Statement,
                "while ${1:condition} {\n    $0\n}",
                true,
            ),
            LanguageItemId::Loop => (
                LanguageCompletionSite::Expression,
                "loop {\n    $0\n}",
                true,
            ),
            LanguageItemId::For => (
                LanguageCompletionSite::Statement,
                "for ${1:value} in ${2:values} {\n    $0\n}",
                true,
            ),
            LanguageItemId::Break => (LanguageCompletionSite::Loop, "break${1: value}", true),
            LanguageItemId::Continue => (LanguageCompletionSite::Loop, "continue", false),
            LanguageItemId::Debug => (
                LanguageCompletionSite::Statement,
                "debug ${1:statement}",
                true,
            ),
            LanguageItemId::Inspect => (
                LanguageCompletionSite::Expression,
                "inspect(${1:expression})",
                true,
            ),
            LanguageItemId::Match => (
                LanguageCompletionSite::Expression,
                "match ${1:value} {\n    ${2:pattern} => $0\n}",
                true,
            ),
            LanguageItemId::Return => (LanguageCompletionSite::Return, "return${1: value}", true),
            LanguageItemId::Yield => (LanguageCompletionSite::Generator, "yield ${1:value}", true),
            LanguageItemId::Throw => (LanguageCompletionSite::Expression, "throw ${1:error}", true),
            LanguageItemId::Await => (
                LanguageCompletionSite::Expression,
                "await ${1:future}",
                true,
            ),
            LanguageItemId::Retry => (
                LanguageCompletionSite::Expression,
                "retry ${1:fallibleExpression}",
                true,
            ),
            LanguageItemId::SelfValue => (LanguageCompletionSite::Method, "self", false),
            LanguageItemId::SomeConstructor => {
                (LanguageCompletionSite::Expression, "Some(${1:value})", true)
            }
            LanguageItemId::IteratorItem => {
                (LanguageCompletionSite::Expression, "Item(${1:value})", true)
            }
            LanguageItemId::IteratorEnd => (LanguageCompletionSite::Expression, "End", false),
            LanguageItemId::SuccessConstructor => {
                (LanguageCompletionSite::Expression, "Ok(${1:value})", true)
            }
            LanguageItemId::ErrorConstructor => {
                (LanguageCompletionSite::Expression, "Err(${1:error})", true)
            }
            LanguageItemId::SignatureLiteral => (
                LanguageCompletionSite::Expression,
                "sig\"${1:pattern}\"",
                true,
            ),
            LanguageItemId::VersionLiteral => (
                LanguageCompletionSite::Expression,
                "v\"${1:major.minor.build.private}\"",
                true,
            ),
            _ => return None,
        };
        Some(LanguageCompletion {
            site,
            insert_text,
            is_snippet,
        })
    }

    /// Resolves an exact source token or a short syntax spelling to its
    /// canonical documentation item.
    pub fn item_for_source_token(self, token: &str) -> Option<&'static LanguageItem> {
        self.item_by_name(token)
            .filter(|item| item.id != LanguageItemId::StatePointerField)
            .or_else(|| {
                let id = match token {
                    "Address" => LanguageItemId::BuiltinType(BuiltinType::Address),
                    "[" => LanguageItemId::ArrayType,
                    "..<" | "..=" => LanguageItemId::Range,
                    "?" => LanguageItemId::OptionType,
                    "!" => LanguageItemId::ResultType,
                    "///" => LanguageItemId::DocumentationComment,
                    "`" => LanguageItemId::TemplateString,
                    _ => return None,
                };
                Some(self.item(id))
            })
    }

    pub fn builtin_type(self, ty: BuiltinType) -> Option<&'static LanguageItem> {
        ITEMS
            .iter()
            .find(|item| item.id == LanguageItemId::BuiltinType(ty))
    }

    pub fn action(self, action: ActionKind) -> &'static LanguageItem {
        ITEMS
            .iter()
            .find(|item| item.kind == LanguageItemKind::Action(action))
            .expect("every action kind must have a language catalog entry")
    }

    pub const fn action_reference_facts(self, action: ActionKind) -> ActionReferenceFacts {
        match action {
            ActionKind::Setup => ActionReferenceFacts {
                timing: "Once, when the loaded module first updates",
                available_context: "settings and initialized module globals",
                suspension: "not allowed",
                result: "None",
                fallthrough: "complete setup",
            },
            ActionKind::SelectProcess => ActionReferenceFacts {
                timing: "For each same-name candidate before provider setup and onAttach",
                available_context: "the temporary process candidate, settings, and initialized module globals",
                suspension: "not allowed; postfix ? and throw reject the candidate",
                result: "bool with an implicit error boundary",
                fallthrough: "false; reject this candidate",
            },
            ActionKind::OnStart => ActionReferenceFacts {
                timing: "After an observed timer transition out of NotRunning",
                available_context: "settings, module globals, and globals initialized here",
                suspension: "not allowed",
                result: "None",
                fallthrough: "complete the event",
            },
            ActionKind::OnReset => ActionReferenceFacts {
                timing: "After an observed timer transition into NotRunning",
                available_context: "settings, module globals, and attempt globals",
                suspension: "not allowed",
                result: "None",
                fallthrough: "complete, then clear attempt globals",
            },
            ActionKind::OnAttach => ActionReferenceFacts {
                timing: "Once after acquiring and preparing a process",
                available_context: "process, prepared provider roots, settings, and globals",
                suspension: "await and retry allowed; ? and throw reject; cancelled on process close",
                result: "None on success; implicit error boundary",
                fallthrough: "finish attachment",
            },
            ActionKind::OnStateReady => ActionReferenceFacts {
                timing: "Once after the first complete state snapshot",
                available_context: "process, provider roots, globals, old, and current",
                suspension: "not allowed",
                result: "None",
                fallthrough: "complete initialization",
            },
            ActionKind::WhileAttached => ActionReferenceFacts {
                timing: "Every initialized attached update after state refresh",
                available_context: "process, provider roots, globals, old, and current",
                suspension: "await and retry allowed; one invocation is polled per update and cancelled on process close",
                result: "bool",
                fallthrough: "true; continue to timer decisions after completion; pending skips them",
            },
            ActionKind::Start => ActionReferenceFacts {
                timing: "After whileAttached when the sampled timer is NotRunning",
                available_context: "process, provider roots, globals, old, and current",
                suspension: "not allowed",
                result: "bool",
                fallthrough: "false; do not start",
            },
            ActionKind::IsLoading => ActionReferenceFacts {
                timing: "After start handling when the sampled timer is Running or Paused",
                available_context: "process, provider roots, globals, old, and current",
                suspension: "not allowed",
                result: "bool?",
                fallthrough: "None; retain the current loading state",
            },
            ActionKind::GameTime => ActionReferenceFacts {
                timing: "After isLoading when the sampled timer is Running or Paused",
                available_context: "process, provider roots, globals, old, and current",
                suspension: "not allowed",
                result: "Duration?",
                fallthrough: "None; retain the current game time",
            },
            ActionKind::Reset => ActionReferenceFacts {
                timing: "After loading and game-time updates when the sampled timer is Running or Paused",
                available_context: "process, provider roots, globals, old, and current",
                suspension: "not allowed",
                result: "bool",
                fallthrough: "false; continue to split",
            },
            ActionKind::Split => ActionReferenceFacts {
                timing: "After reset declines when the sampled timer is Running or Paused",
                available_context: "process, provider roots, globals, old, and current",
                suspension: "not allowed",
                result: "bool",
                fallthrough: "false; do not split",
            },
            ActionKind::OnDetach => ActionReferenceFacts {
                timing: "Once after a successfully initialized state-provider attachment ends and its context is cleared",
                available_context: "settings, module globals, and live attempt globals",
                suspension: "not allowed",
                result: "None",
                fallthrough: "complete cleanup",
            },
        }
    }

    pub fn validate(self) -> Vec<String> {
        let mut errors = Vec::new();
        let mut ids = HashSet::new();
        let mut names = HashSet::new();
        let mut example_sources = HashSet::new();
        for item in ITEMS {
            if !ids.insert(item.id) {
                errors.push(format!("duplicate language item ID `{:?}`", item.id));
            }
            if !names.insert(item.name) {
                errors.push(format!("duplicate language item name `{}`", item.name));
            }
            if item.form.trim().is_empty() {
                errors.push(format!("`{}` has no syntax form", item.name));
            }
            if item.documentation.summary.trim().is_empty() {
                errors.push(format!("`{}` has no documentation summary", item.name));
            }
            if item.documentation.details.trim().is_empty() {
                errors.push(format!("`{}` has no documentation details", item.name));
            }
            if item.documentation.examples.is_empty() {
                errors.push(format!("`{}` has no examples", item.name));
            }
            for example in item.documentation.examples {
                if example.title.trim().is_empty()
                    || example.source.trim().is_empty()
                    || !example.has_validation_source()
                {
                    errors.push(format!("`{}` has an incomplete example", item.name));
                }
                if !example_sources.insert(example.source) {
                    errors.push(format!(
                        "`{}` reuses another symbol's visible example",
                        item.name
                    ));
                }
            }
            for related in item.documentation.related {
                if !ITEMS.iter().any(|candidate| candidate.id == *related) {
                    errors.push(format!(
                        "`{}` links to missing language item `{:?}`",
                        item.name, related
                    ));
                }
            }
            if let LanguageItemKind::Action(action) = item.kind
                && item.name != action.name()
            {
                errors.push(format!(
                    "action catalog name `{}` does not match `{}`",
                    item.name,
                    action.name()
                ));
            }
        }
        for action in [
            ActionKind::Setup,
            ActionKind::SelectProcess,
            ActionKind::OnDetach,
            ActionKind::OnAttach,
            ActionKind::OnStateReady,
            ActionKind::OnStart,
            ActionKind::OnReset,
            ActionKind::WhileAttached,
            ActionKind::Start,
            ActionKind::Split,
            ActionKind::Reset,
            ActionKind::IsLoading,
            ActionKind::GameTime,
        ] {
            if !ITEMS
                .iter()
                .any(|item| item.kind == LanguageItemKind::Action(action))
            {
                errors.push(format!("missing action catalog entry `{}`", action.name()));
            }
        }
        for builtin in [
            BuiltinType::Never,
            BuiltinType::None,
            BuiltinType::Bool,
            BuiltinType::I8,
            BuiltinType::U8,
            BuiltinType::I16,
            BuiltinType::U16,
            BuiltinType::I32,
            BuiltinType::U32,
            BuiltinType::I64,
            BuiltinType::U64,
            BuiltinType::Address,
            BuiltinType::F32,
            BuiltinType::F64,
        ] {
            if self.builtin_type(builtin).is_none() {
                errors.push(format!("missing built-in type catalog entry `{builtin}`"));
            }
        }
        errors
    }
}
