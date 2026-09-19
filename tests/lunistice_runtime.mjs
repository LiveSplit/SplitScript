import fs from "node:fs";

const wasmPath = process.argv[2];
if (!wasmPath) throw new Error("usage: node tests/lunistice_runtime.mjs <lunistice.wasm>");

const bytes = fs.readFileSync(wasmPath);
const dlc = process.argv.includes("--dlc");
const ambiguousClass = process.argv.includes("--ambiguous-class");
const missingClass = process.argv.includes("--missing-class");
const ambiguousField = process.argv.includes("--ambiguous-field");
const missingField = process.argv.includes("--missing-field");
const transientBindingRead = process.argv.includes("--transient-binding-read");
const mixedLayout = process.argv.includes("--mixed-layout");
const inheritedField = process.argv.includes("--inherited-field");
const decoder = new TextDecoder();
const base = 0x1000n;
const memoryImage = new Uint8Array(0x2800000);
const view = new DataView(memoryImage.buffer);
const messages = [];
const variables = new Map();
const tickRates = [];
const gameTimes = [];
let starts = 0;
let splits = 0;
let resets = 0;
let pauses = 0;
let timerState = 0;
let variableWrites = 0;
let failReads = false;
let processOpen = true;
let detaches = 0;
let scanReads = 0;
let bulkScanReads = 0;
let attachedRateScanReads = undefined;
const levelOrSceneReadWidths = new Set();
const levelTimeVectorReads = new Set();
let countSnapshotRoots = false;
let gameManagerRootReads = 0;
let timerRootReads = 0;
let instance;

const absolute = (relative) => 0x1000n + BigInt(relative);
const pointer = (relative, value) => view.setBigUint64(relative, BigInt(value), true);
const string = (relative, value) => memoryImage.set(new TextEncoder().encode(`${value}\0`), relative);
const field = (table, index, nameAddress, offset) => {
    pointer(table + index * 0x20, nameAddress);
    view.setUint32(table + index * 0x20 + 0x18, offset, true);
};

// Put each full-module IL2CPP discovery target deep into a realistically
// sized image. This makes the fixture exercise many cooperative scan polls
// instead of accidentally completing every scan in one update.
view.setUint16(0, 0x5a4d, true);
view.setUint32(0x3c, 0x80, true);
view.setUint32(0x80, 0x4550, true);
view.setUint16(0x98, 0x20b, true);
const assembliesInstruction = 0x100500;
const metadataAddress = 0x200600;
const metadataReference = 0x2400700;
const shiftInstruction = 0x2400720;
const storeInstruction = 0x2400740;
memoryImage.set([0x75, 0x11, 0x48, 0x8b, 0x1d], assembliesInstruction);
view.setInt32(assembliesInstruction + 5, 0x800 - (assembliesInstruction + 9), true);
memoryImage.set([0x48, 0x3b, 0x1d], assembliesInstruction + 9);
view.setInt32(assembliesInstruction + 12, 0x808 - (assembliesInstruction + 16), true);
string(metadataAddress, "global-metadata.dat");
// Real GameAssembly images contain thousands of unrelated LEA instructions.
// Checking only one candidate per update made source-defined Unity discovery
// take about a minute despite every individual byte-range scan being bounded.
for (let decoy = 0x400000; decoy < 0x2400000; decoy += 0x1000) {
    memoryImage.set([0x48, 0x8d, 0x0d], decoy);
    view.setInt32(decoy + 3, metadataAddress + 1 - (decoy + 7), true);
}
memoryImage.set([0x48, 0x8d, 0x0d], metadataReference);
view.setInt32(metadataReference + 3, metadataAddress - (metadataReference + 7), true);
memoryImage.set([0x48, 0xc1, 0xe9], shiftInstruction);
memoryImage.set([0x48, 0x89, 0x05], storeInstruction);
view.setInt32(storeInstruction + 3, 0x900 - (storeInstruction + 7), true);
pointer(0x800, 0x1810);
pointer(0x808, 0x1818);
pointer(0x810, 0x1a00);
pointer(0xa00, 0x1b00);
pointer(0xa18, 0x1c00);
pointer(0xb08, 0x1c00);
string(0xc00, "Assembly-CSharp");
view.setUint32(0xb18, 2, true);
pointer(0xb28, 0x1d00);
view.setUint32(0xd00, 0, true);
pointer(0x900, 0x1d10);
pointer(0xd10, 0x3000);
pointer(0xd18, 0x3800);
if (ambiguousClass) {
    view.setUint32(0xb18, 3, true);
    pointer(0xd20, 0x4000);
    pointer(0x3010, 0xb000);
    pointer(0x3018, 0xb040);
}

// Class names and common empty namespace.
string(0xa000, missingClass ? "UnrelatedManager" : "GameManager");
string(0xa020, "Timer");
string(0xa040, "");

// GameManager class, fields, static table, and singleton.
pointer(0x2010, 0xb000);
pointer(0x2018, 0xb040);
// GameManager inherits from a UnityEngine class. Its metadata must form the
// traversal boundary rather than being interpreted with the game class's
// field layout forever.
pointer(0x2058, 0xe000);
pointer(0x2080, 0x5000);
pointer(0x20b8, 0x7000);
pointer(0xd010, 0xf000);
pointer(0xd018, 0xf040);
string(0xe000, inheritedField ? "BaseManager" : "MonoBehaviour");
string(0xe040, inheritedField ? "" : "UnityEngine");
view.setUint16(0xd124, 1, true);
// Real IL2CPP classes may expose unreadable field slots after their usable
// metadata entries. Field discovery must skip those slots instead of retrying
// the entire alias forever before later aliases are considered.
view.setUint16(0x2124, 119, true);
const gameFieldNames = [
    "<Instance>k__BackingField",
    dlc ? "<GameState>k__BackingField" : "gameState",
    "_deaths",
    ...(inheritedField ? [] : [missingField ? "unrelatedPoints" : "_points"]),
    ...(mixedLayout ? ["currentLevel", "_currentScene"] : [dlc ? "_currentScene" : "currentLevel"]),
];
const gameFieldOffsets = gameFieldNames.map(name => ({
    "<Instance>k__BackingField": 0x20,
    "<GameState>k__BackingField": 0x30,
    gameState: 0x30,
    unrelatedPoints: 0x34,
    _points: 0x34,
    _deaths: 0x38,
    currentLevel: 0x3c,
    _currentScene: mixedLayout ? 0x48 : 0x3c,
}[name]));
for (let index = 0; index < gameFieldNames.length; index += 1) {
    const relative = 0xa100 + index * 0x30;
    string(relative, gameFieldNames[index]);
    field(0x4000, index, absolute(relative), gameFieldOffsets[index]);
}
if (inheritedField) {
    // Keep the base class's field table outside the derived class's deliberately
    // oversized 119-slot table. Overlapping them fabricates a second derived
    // declaration of `_points`, which is observably distinct now that field
    // identity includes the declaring runtime class for static storage.
    pointer(0xd080, 0x15000);
    string(0xa600, "_points");
    field(0x14000, 0, absolute(0xa600), 0x34);
}
if (ambiguousField) {
    string(0xa1f0, "_points");
    field(0x4000, 5, absolute(0xa1f0), 0x48);
}
pointer(0x6020, 0xa000);

// Timer class, fields, static table, and singleton.
pointer(0x2810, 0xb020);
pointer(0x2818, 0xb040);
pointer(0x2858, 0);
pointer(0x2880, 0x5200);
pointer(0x28b8, 0x7200);
view.setUint16(0x2924, 5, true);
const timerFieldNames = ["<Instance>k__BackingField", "currentLevelTime", "currentLevelTimeVector", "timerStopped", "character"];
const timerFieldOffsets = [0x20, 0x30, 0x34, 0x40, 0x44];
for (let index = 0; index < timerFieldNames.length; index += 1) {
    const relative = 0xa300 + index * 0x30;
    string(relative, timerFieldNames[index]);
    field(0x4200, index, absolute(relative), timerFieldOffsets[index]);
}
pointer(0x6220, 0xa100);

const gameManager = 0x9000;
const timer = 0x9100;
const sceneObject = 0xc000;
const sceneName = "Shrine01";
view.setUint32(sceneObject + 0x10, sceneName.length, true);
for (let index = 0; index < sceneName.length; index += 1) {
    view.setUint16(sceneObject + 0x14 + index * 2, sceneName.charCodeAt(index), true);
}
const setGame = ({ state = 0, points = 12, deaths = 3, level = 0 } = {}) => {
    view.setInt32(gameManager + 0x30, state, true);
    view.setInt32(gameManager + 0x34, points, true);
    view.setInt32(gameManager + 0x38, deaths, true);
    if (dlc) pointer(gameManager + 0x3c, absolute(sceneObject));
    else view.setInt32(gameManager + 0x3c, level, true);
};
const setTimer = ({ levelTime = 0, minutes = 0, seconds = 0, hundredths = 0, stopped = true, character = 0 } = {}) => {
    view.setFloat32(timer + 0x30, levelTime, true);
    view.setFloat32(timer + 0x34, minutes, true);
    view.setFloat32(timer + 0x38, seconds, true);
    view.setFloat32(timer + 0x3c, hundredths, true);
    view.setUint8(timer + 0x40, stopped ? 1 : 0);
    view.setUint32(timer + 0x44, character, true);
};
setGame();
setTimer();

const text = (pointer, length) => decoder.decode(new Uint8Array(instance.exports.memory.buffer, pointer, length));
const env = {
    timer_get_state: () => timerState,
    timer_start() { starts += 1; timerState = 1; },
    timer_split() { splits += 1; },
    timer_reset() { resets += 1; timerState = 0; },
    timer_set_game_time(seconds, nanos) { gameTimes.push([Number(seconds), nanos]); },
    timer_pause_game_time() { pauses += 1; },
    timer_resume_game_time() {},
    timer_set_variable(keyPointer, keyLength, valuePointer, valueLength) {
        variableWrites += 1;
        variables.set(text(keyPointer, keyLength), text(valuePointer, valueLength));
    },
    runtime_set_tick_rate(rate) {
        tickRates.push(rate);
        if (rate === 120 && attachedRateScanReads === undefined) {
            attachedRateScanReads = scanReads;
        }
    },
    process_attach(pointer, length) {
        const expected = dlc ? "Lunistice-Demo.exe" : "Lunistice.exe";
        return processOpen && text(pointer, length) === expected ? 1n : 0n;
    },
    process_detach() { detaches += 1; },
    process_is_open: () => processOpen ? 1 : 0,
    process_read(_process, address, pointer, length) {
        if (length > 8) scanReads += 1;
        if (length > 0x100) bulkScanReads += 1;
        if (address === absolute(gameManager + 0x3c)) levelOrSceneReadWidths.add(length);
        if (countSnapshotRoots && address === absolute(0x6020)) gameManagerRootReads += 1;
        if (countSnapshotRoots && address === absolute(0x6220)) timerRootReads += 1;
        if (address >= absolute(timer + 0x34) && address < absolute(timer + 0x40)) {
            levelTimeVectorReads.add(`${address - absolute(timer + 0x34)}:${length}`);
        }
        if (failReads) return 0;
        const offset = Number(address - base);
        if (offset < 0 || offset + length > memoryImage.length) return 0;
        new Uint8Array(instance.exports.memory.buffer, pointer, length).set(memoryImage.subarray(offset, offset + length));
        return 1;
    },
    process_get_module_address: () => base,
    process_get_module_size: () => BigInt(memoryImage.length),
    runtime_print_message(pointer, length) { messages.push(text(pointer, length)); },
    user_settings_add_bool: () => 1,
    user_settings_add_title() {},
    user_settings_add_choice() {},
    user_settings_add_choice_option: () => 0,
    user_settings_add_file_select() {},
    user_settings_add_file_select_name_filter() {},
    user_settings_add_file_select_mime_filter() {},
    user_settings_set_tooltip() {},
    settings_map_load: () => 1n,
    settings_map_free() {},
    settings_map_get: () => 0n,
    setting_value_free() {},
    setting_value_get_bool: () => 0,
    setting_value_get_string: () => 0,
};

({ instance } = await WebAssembly.instantiate(bytes, { env }));
instance.exports._start();
if (transientBindingRead) {
    failReads = true;
    for (let tick = 0; tick < 3; tick += 1) instance.exports.update();
    failReads = false;
}
for (let tick = 0; tick < 180 && variableWrites < 5; tick += 1) instance.exports.update();
const expectedBindingFailure = ambiguousClass
    ? "multiple Unity classes match metadata names"
    : missingClass
        ? "no Unity class matches metadata names"
        : ambiguousField
            ? "multiple Unity fields match metadata names"
            : missingField
                ? "no Unity field matches metadata names"
                : mixedLayout
                    ? "Could not select the attachment shape"
                : undefined;
if (expectedBindingFailure !== undefined) {
    if (variableWrites !== 0) {
        throw new Error(`an invalid binding reached state polling: ${variableWrites}`);
    }
    if (!messages.some(message => message.includes(expectedBindingFailure))) {
        throw new Error(`missing binding diagnostic ${JSON.stringify(expectedBindingFailure)}: ${JSON.stringify(messages)}`);
    }
    if (mixedLayout) {
        for (const expected of [
            "Assembly-CSharp::GameManager.level",
            "Assembly-CSharp::GameManager.scene",
            "Expected attachment shape `edition = Edition.BaseGame`",
            "Expected attachment shape `edition = Edition.DlcDemo`",
        ]) {
            if (!messages.some(message => message.includes(expected))) {
                throw new Error(`shape report omitted ${JSON.stringify(expected)}: ${JSON.stringify(messages)}`);
            }
        }
    }
    console.log(JSON.stringify({ messages, variableWrites, tickRates }));
} else {
if (variableWrites < 5) {
    throw new Error(
        `attachment did not finish: messages=${JSON.stringify(messages)}, scanReads=${scanReads}, variableWrites=${variableWrites}, tickRates=${JSON.stringify(tickRates)}`,
    );
}
const bulkScanReadsAfterAttachment = bulkScanReads;
if (attachedRateScanReads !== 0) {
    throw new Error(`attached tick rate was applied after scanning began: ${attachedRateScanReads}`);
}

// Managed class snapshots and the remaining derived scene read share static
// roots across the surrounding state transaction. Reading each singleton once
// avoids multiplying host calls while still observing replacement singleton
// objects on the next tick.
countSnapshotRoots = true;
const replacementGameManager = 0x9200;
const replacementTimer = 0x9300;
memoryImage.copyWithin(replacementGameManager, gameManager, gameManager + 0x100);
memoryImage.copyWithin(replacementTimer, timer, timer + 0x100);
view.setUint32(replacementGameManager + 0x34, 99, true);
pointer(0x6020, absolute(replacementGameManager));
pointer(0x6220, absolute(replacementTimer));
instance.exports.update();
countSnapshotRoots = false;
if (gameManagerRootReads !== 1 || timerRootReads !== 1) {
    throw new Error(
        `managed snapshot roots were not shared: GameManager=${gameManagerRootReads}, Timer=${timerRootReads}`,
    );
}
if (variables.get("Points") !== "99") {
    throw new Error(`replacement singleton was not observed: Points=${variables.get("Points")}`);
}
pointer(0x6020, absolute(gameManager));
pointer(0x6220, absolute(timer));
instance.exports.update();

// Prime snapshots, transition timerStopped to false, enter results, then roll
// the level clock over. The accumulated game time must become 10 + 1 seconds.
instance.exports.update();
setTimer({ stopped: false });
instance.exports.update();
setTimer({ stopped: false, levelTime: 10, seconds: 10, character: dlc ? 6 : 1 });
instance.exports.update();
setGame({ state: 6, level: 0 });
instance.exports.update();
setGame({ state: 0, level: 1 });
setTimer({ stopped: false, levelTime: 1, seconds: 1, character: dlc ? 6 : 1 });
instance.exports.update();

// Failed fields retain their accepted values. The watcher tick still runs
// against that stable snapshot, so ordinary actions and variable writes are
// not suppressed by a transient read failure.
const writesBeforeFailure = variableWrites;
const timesBeforeFailure = gameTimes.length;
setGame({ state: 0, points: 99, level: 1 });
failReads = true;
instance.exports.update();
failReads = false;
if (variableWrites !== writesBeforeFailure + 5 || gameTimes.length !== timesBeforeFailure + 1) {
    throw new Error("a failed state read did not retain values for an ordinary watcher tick");
}
if (variables.get("Points") !== "12") throw new Error("failed snapshot leaked partial state");

// Simulate a runner manually starting the timer after accumulated time exists.
// The NotRunning -> Running transition must clear accumulation, and a level
// clock rollback before the first completed level must reset the timer.
setGame({ state: 0, points: 12, level: 0 });
timerState = 0;
instance.exports.update();
timerState = 1;
setTimer({ stopped: false, levelTime: 2, seconds: 2, character: dlc ? 6 : 1 });
instance.exports.update();
if (gameTimes.at(-1)?.[0] !== 2) {
    throw new Error(`runner-started timing did not clear accumulation: ${JSON.stringify(gameTimes)}`);
}
setTimer({ stopped: false, levelTime: 5, seconds: 5, character: dlc ? 6 : 1 });
instance.exports.update();
setTimer({ stopped: false, levelTime: 1, seconds: 1, character: dlc ? 6 : 1 });
instance.exports.update();

// Base-game final-level -> credits is the second split condition. The DLC
// scene-based layout intentionally does not use this transition.
instance.exports.update();
timerState = 1;
setGame({ state: 0, points: 12, level: 13 });
instance.exports.update();
setGame({ state: 0, points: 12, level: 2 });
instance.exports.update();

if (starts !== 1) {
    throw new Error(`expected one start, got ${starts}; variables=${JSON.stringify(Object.fromEntries(variables))}; messages=${JSON.stringify(messages)}`);
}
const expectedSplits = dlc ? 1 : 2;
if (splits !== expectedSplits) throw new Error(`expected ${expectedSplits} splits, got ${splits}`);
if (resets !== 1) throw new Error(`expected one early-level reset, got ${resets}`);
if (pauses < 3) throw new Error(`expected game time to remain paused, got ${pauses} pauses`);
if (!gameTimes.some(([seconds, nanos]) => seconds === 11 && nanos === 0)) {
    throw new Error(`expected accumulated game time 11.0, got ${JSON.stringify(gameTimes)}`);
}
const expectedVariables = new Map([
    ["Points", "12"],
    ["Resets", "3"],
    ["Level Time", "0:01.00"],
    [dlc ? "Scene" : "Level", dlc ? "Shrine01" : "2-1"],
    ["Character", dlc ? "Cres" : "Toree"],
]);
for (const [key, value] of expectedVariables) {
    if (variables.get(key) !== value) throw new Error(`expected ${key}=${value}, got ${variables.get(key)}`);
}

// Values outside the four known host states are normalized to
// TimerState.Unknown rather than leaking an integer into language code.
timerState = 99;
instance.exports.update();

// Closing the process returns the runtime to the detached polling rate
// immediately, without waiting for another attachment attempt.
processOpen = false;
instance.exports.update();
if (detaches !== 1) throw new Error(`expected one process detach, got ${detaches}`);
instance.exports.update();
if (JSON.stringify(tickRates) !== JSON.stringify([1, 120, 1])) {
    throw new Error(`unexpected tick rates: ${JSON.stringify(tickRates)}`);
}

const expectedLevelOrSceneReadWidths = [dlc ? 8 : 4];
if (JSON.stringify([...levelOrSceneReadWidths]) !== JSON.stringify(expectedLevelOrSceneReadWidths)) {
    throw new Error(
        `inactive level/scene representation was read: ${JSON.stringify([...levelOrSceneReadWidths])}`,
    );
}
if (JSON.stringify([...levelTimeVectorReads]) !== JSON.stringify(["0:12"])) {
    throw new Error(`level time vector was not read atomically: ${JSON.stringify([...levelTimeVectorReads])}`);
}
if (bulkScanReads !== bulkScanReadsAfterAttachment) {
    throw new Error(
        `bulk process scanning continued after attachment: ${bulkScanReadsAfterAttachment} -> ${bulkScanReads}`,
    );
}

// A fresh process must repeat provider preparation and automatic shape
// selection. In particular, the old shape-ready state must not survive after
// its attachment-scoped `edition` discriminator has been cleared.
const writesBeforeReattach = variableWrites;
processOpen = true;
for (let tick = 0; tick < 180 && variableWrites === writesBeforeReattach; tick += 1) {
    instance.exports.update();
}
if (variableWrites === writesBeforeReattach) {
    throw new Error(`reattachment did not reach state polling: tickRates=${JSON.stringify(tickRates)}`);
}
processOpen = false;
instance.exports.update();
instance.exports.update();
if (detaches !== 2) throw new Error(`expected two process detaches, got ${detaches}`);
if (JSON.stringify(tickRates.slice(-2)) !== JSON.stringify([120, 1])) {
    throw new Error(`reattachment did not restore both tick rates: ${JSON.stringify(tickRates)}`);
}

console.log(JSON.stringify({ dlc, starts, splits, resets, pauses, gameTimes, variables: Object.fromEntries(variables), tickRates, messages, bulkScanReads, levelOrSceneReadWidths: [...levelOrSceneReadWidths], levelTimeVectorReads: [...levelTimeVectorReads] }));

}
