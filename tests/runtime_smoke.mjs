import fs from "node:fs";

const wasmPath = process.argv[2];

if (!wasmPath) {
    throw new Error("usage: node tests/runtime_smoke.mjs <autosplitter.wasm>");
}

const bytes = fs.readFileSync(wasmPath);
const decoder = new TextDecoder();
const messages = [];
const variables = [];
const tickRates = [];
let modulePolls = 0;
let gameAssemblyPolls = 0;
let moduleSizePolls = 0;
const moduleAddressNames = [];
const moduleSizeNames = [];
let scanReads = 0;
let scanBytes = 0;
let scalarReads = 0;
const scanBytesPerTick = [];
const attachNames = [];
let instance;
const gameAssembly = new Uint8Array(0x2000);
gameAssembly.set([0x4d, 0x5a], 0x00);
gameAssembly.set([0x48, 0x8b, 0x7c, 0x89], 0x0ffe);
const gameAssemblyView = new DataView(gameAssembly.buffer);
gameAssemblyView.setUint32(0x3c, 0x80, true);
gameAssemblyView.setUint32(0x80, 0x00004550, true);
gameAssemblyView.setUint16(0x98, 0x020b, true);
gameAssemblyView.setBigUint64(0x100, 0x1200n, true);
gameAssemblyView.setBigUint64(0x220, 0x1300n, true);
gameAssemblyView.setInt32(0x310, 42, true);
gameAssemblyView.setInt32(0x400, 0xfc, true);
gameAssembly.set([0x75, 0x11, 0x48, 0x8b, 0x1d], 0x500);
gameAssemblyView.setInt32(0x505, 0x2f7, true);
gameAssembly.set([0x48, 0x3b, 0x1d], 0x509);
gameAssemblyView.setInt32(0x50c, 0x808 - 0x510, true);
gameAssembly.set(new TextEncoder().encode("global-metadata.dat\0"), 0x600);
gameAssembly.set([0x48, 0x8d, 0x0d], 0x700);
gameAssemblyView.setInt32(0x703, -0x107, true);
gameAssembly.set([0x48, 0xc1, 0xe9], 0x720);
gameAssembly.set([0x48, 0x89, 0x05], 0x740);
gameAssemblyView.setInt32(0x743, 0x1b9, true);
gameAssemblyView.setBigUint64(0x800, 0x1810n, true);
gameAssemblyView.setBigUint64(0x808, 0x1818n, true);
gameAssemblyView.setBigUint64(0x810, 0x1a00n, true);
gameAssemblyView.setBigUint64(0xa00, 0x1b00n, true);
gameAssemblyView.setBigUint64(0xa18, 0x1c00n, true);
gameAssemblyView.setBigUint64(0xb08, 0x1c00n, true);
gameAssembly.set(new TextEncoder().encode("Assembly-CSharp\0"), 0xc00);
gameAssemblyView.setUint32(0xb18, 1, true);
gameAssemblyView.setBigUint64(0xb28, 0x1d00n, true);
gameAssemblyView.setUint32(0xd00, 0, true);
gameAssemblyView.setBigUint64(0x900, 0x1d10n, true);
gameAssemblyView.setBigUint64(0xd10, 0x1e00n, true);
gameAssemblyView.setBigUint64(0xe10, 0x1f00n, true);
gameAssemblyView.setBigUint64(0xe18, 0x1fc0n, true);
gameAssemblyView.setBigUint64(0xe58, 0n, true);
gameAssemblyView.setBigUint64(0xe80, 0x2400n, true);
gameAssemblyView.setBigUint64(0xeb8, 0x2600n, true);
gameAssembly.set(new TextEncoder().encode("GameManager\0"), 0xf00);
gameAssemblyView.setUint16(0xf24, 1, true);
gameAssembly.set(new TextEncoder().encode("\0"), 0xfc0);
gameAssemblyView.setBigUint64(0x1400, 0x2500n, true);
gameAssemblyView.setUint32(0x1418, 0x20, true);
gameAssembly.set(new TextEncoder().encode("<Instance>k__BackingField\0"), 0x1500);
gameAssemblyView.setBigUint64(0x1620, 0x2700n, true);
const managedScene = "Shrine01 🦊";
gameAssemblyView.setUint32(0x1810, managedScene.length, true);
for (let index = 0; index < managedScene.length; index += 1) {
    gameAssemblyView.setUint16(0x1814 + index * 2, managedScene.charCodeAt(index), true);
}

const env = {
    timer_get_state: () => 0,
    timer_start() {},
    timer_split() {},
    timer_reset() {},
    timer_set_game_time() {},
    timer_pause_game_time() {},
    timer_resume_game_time() {},
    timer_set_variable(keyPointer, keyLength, valuePointer, valueLength) {
        const memory = instance.exports.memory.buffer;
        variables.push([
            decoder.decode(new Uint8Array(memory, keyPointer, keyLength)),
            decoder.decode(new Uint8Array(memory, valuePointer, valueLength)),
        ]);
    },
    runtime_set_tick_rate(rate) {
        tickRates.push(rate);
    },
    process_attach(pointer, length) {
        const name = decoder.decode(
            new Uint8Array(instance.exports.memory.buffer, pointer, length),
        );
        attachNames.push(name);
        return name === "Lunistice-Demo.exe" ? 1n : 0n;
    },
    process_detach() {},
    process_is_open: () => 1,
    process_read(_process, address, pointer, length) {
        const offset = Number(address - 0x1000n);
        if (offset < 0 || offset + length > gameAssembly.length) {
            return 0;
        }
        new Uint8Array(instance.exports.memory.buffer, pointer, length).set(
            gameAssembly.subarray(offset, offset + length),
        );
        if (length > 8) {
            scanReads += 1;
            scanBytes += length;
        } else {
            scalarReads += 1;
        }
        return 1;
    },
    process_get_module_address(_process, pointer, length) {
        const name = decoder.decode(
            new Uint8Array(instance.exports.memory.buffer, pointer, length),
        );
        moduleAddressNames.push(name);
        modulePolls += 1;
        if (name === "GameAssembly.dll" && ++gameAssemblyPolls < 3) return 0n;
        return 0x1000n;
    },
    process_get_module_size(_process, pointer, length) {
        moduleSizeNames.push(decoder.decode(
            new Uint8Array(instance.exports.memory.buffer, pointer, length),
        ));
        moduleSizePolls += 1;
        return 0x2000n;
    },
    runtime_print_message(pointer, length) {
        const memory = new Uint8Array(instance.exports.memory.buffer, pointer, length);
        messages.push(decoder.decode(memory));
    },
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

for (let tick = 0; tick < 40 && !messages.includes("Hello, world from SplitScript!"); tick += 1) {
    const before = scanBytes;
    instance.exports.update();
    scanBytesPerTick.push(scanBytes - before);
}

const expected = "Hello, world from SplitScript!";

if (modulePolls !== 5) {
    throw new Error(`expected 5 module polls, got ${modulePolls}`);
}

if (moduleSizePolls !== 2) {
    throw new Error(`expected two module-size polls, got ${moduleSizePolls}`);
}

if (moduleAddressNames.join(",") !== [
    "Lunistice-Demo.exe",
    "GameAssembly.dll",
    "GameAssembly.dll",
    "GameAssembly.dll",
    "Lunistice-Demo.exe",
].join(",")) {
    throw new Error(`unexpected module-address names: ${JSON.stringify(moduleAddressNames)}`);
}

if (moduleSizeNames.join(",") !== "GameAssembly.dll,Lunistice-Demo.exe") {
    throw new Error(`unexpected module-size names: ${JSON.stringify(moduleSizeNames)}`);
}

if (scanReads > 18) {
    throw new Error(`exceeded the eighteen bulk/scan read budget: ${scanReads}`);
}

if (scalarReads > 64) {
    throw new Error(`exceeded the scalar read budget: ${scalarReads}`);
}

const activeScanTicks = scanBytesPerTick.filter((bytes) => bytes > 0);
if (activeScanTicks.length < 6) {
    throw new Error(
        `expected Unity discovery to yield across at least six scan ticks, got ${JSON.stringify(scanBytesPerTick)}`,
    );
}

if (Math.max(...scanBytesPerTick) > 8192) {
    throw new Error(
        `expected bounded Unity discovery work per tick, got ${JSON.stringify(scanBytesPerTick)}`,
    );
}

if (attachNames.join(",") !== "Lunistice.exe,Lunistice-Demo.exe") {
    throw new Error(`unexpected attachment order: ${JSON.stringify(attachNames)}`);
}

if (messages.length !== 1 || messages[0] !== expected) {
    throw new Error(`expected one hello-world message, got ${JSON.stringify(messages)}`);
}

if (JSON.stringify(variables) !== JSON.stringify([
    ["Probe", "-42:7"],
    ["Process", "Lunistice-Demo.exe"],
])) {
    throw new Error(`unexpected runtime variables: ${JSON.stringify(variables)}`);
}

if (JSON.stringify(tickRates) !== JSON.stringify([1, 120])) {
    throw new Error(`unexpected tick rates: ${JSON.stringify(tickRates)}`);
}

console.log(
    JSON.stringify({
        attachNames,
        moduleAddressNames,
        moduleSizeNames,
        modulePolls,
        moduleSizePolls,
        scanReads,
        scanBytesPerTick,
        scalarReads,
        messages,
        variables,
        tickRates,
    }),
);
