import fs from "node:fs";

const wasmPath = process.argv[2];
if (!wasmPath) {
    throw new Error("usage: node tests/managed_instances_runtime.mjs <autosplitter.wasm>");
}

const base = 0x1000n;
const memoryImage = new Uint8Array(0xa0000);
const view = new DataView(memoryImage.buffer);
const decoder = new TextDecoder();
const messages = [];
let scanReads = 0;
let instance;

const absolute = relative => base + BigInt(relative);
const pointer = (relative, value) => view.setBigUint64(relative, BigInt(value), true);
const string = (relative, value) => memoryImage.set(new TextEncoder().encode(`${value}\0`), relative);

// Minimal IL2CPP measured 2021.3 metadata graph for Assembly-CSharp.Enemy. The explicit
// provider selector avoids version probing but intentionally retains the same
// cooperative image and class discovery used by real schemas.
view.setUint16(0, 0x5a4d, true);
view.setUint32(0x3c, 0x80, true);
view.setUint32(0x80, 0x4550, true);
view.setUint16(0x98, 0x20b, true);
const assembliesInstruction = 0x500;
const metadataAddress = 0x600;
const metadataReference = 0x700;
const shiftInstruction = 0x720;
const storeInstruction = 0x740;
memoryImage.set([0x75, 0x11, 0x48, 0x8b, 0x1d], assembliesInstruction);
view.setInt32(assembliesInstruction + 5, 0x800 - (assembliesInstruction + 9), true);
memoryImage.set([0x48, 0x3b, 0x1d], assembliesInstruction + 9);
view.setInt32(assembliesInstruction + 12, 0x808 - (assembliesInstruction + 16), true);
string(metadataAddress, "global-metadata.dat");
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
view.setUint32(0xb18, 1, true);
pointer(0xb28, 0x1d00);
view.setUint32(0xd00, 0, true);
pointer(0x900, 0x1d10);
pointer(0xd10, 0x3000);
pointer(0x2010, 0xb000);
pointer(0x2018, 0xb040);
string(0xa000, "Enemy");
string(0xa040, "");

// Two naturally aligned objects carry the IL2CPP class pointer in their first
// word. They lie on opposite sides of the per-poll byte budget, proving that a
// completed array is assembled across cooperative polls rather than by one
// unbounded update.
pointer(0x11000, 0x3000);
pointer(0x91000, 0x3000);

const text = (pointer, length) => decoder.decode(
    new Uint8Array(instance.exports.memory.buffer, pointer, length),
);
const env = {
    runtime_set_tick_rate() {},
    process_attach: (pointer, length) => text(pointer, length) === "game.exe" ? 1n : 0n,
    process_detach() {},
    process_is_open: () => 1,
    process_read(_process, address, pointer, length) {
        if (length > 8) scanReads += 1;
        const offset = Number(address - base);
        if (offset < 0 || offset + length > memoryImage.length) return 0;
        new Uint8Array(instance.exports.memory.buffer, pointer, length)
            .set(memoryImage.subarray(offset, offset + length));
        return 1;
    },
    process_get_module_address: () => base,
    process_get_module_size: () => BigInt(memoryImage.length),
    process_get_memory_range_count: () => 1n,
    process_get_memory_range_address: () => absolute(0x10000),
    process_get_memory_range_size: () => 0x90000n,
    process_get_memory_range_flags: () => 0x6n,
    runtime_print_message(pointer, length) {
        messages.push(text(pointer, length));
    },
};

({ instance } = await WebAssembly.instantiate(fs.readFileSync(wasmPath), { env }));
instance.exports._start();
let completedAt;
for (let tick = 0; tick < 80 && messages.length === 0; tick += 1) {
    instance.exports.update();
    if (messages.length !== 0) completedAt = tick + 1;
}

if (JSON.stringify(messages) !== JSON.stringify(["2"])) {
    throw new Error(`unexpected managed instances: ${JSON.stringify({ messages, scanReads })}`);
}
if (completedAt < 3 || scanReads < 2) {
    throw new Error(`instance discovery did not span cooperative polls: ${JSON.stringify({ completedAt, scanReads })}`);
}

console.log(JSON.stringify({ messages, completedAt, scanReads }));
