import fs from "node:fs";

const wasmPath = process.argv[2];
if (!wasmPath) {
    throw new Error("usage: node tests/inspect_runtime.mjs <inspect.wasm>");
}

const decoder = new TextDecoder();
const variables = new Map();
const messages = [];
let instance;
const stateBytes = new Uint8Array(5);
new DataView(stateBytes.buffer).setUint32(0, 42, true);
stateBytes[4] = 1;

const text = (pointer, length) => decoder.decode(
    new Uint8Array(instance.exports.memory.buffer, pointer, length),
);

const env = {
    timer_get_state: () => 0,
    runtime_set_tick_rate() {},
    process_attach: () => 1n,
    process_detach() {},
    process_is_open: () => 1,
    process_read(_process, address, destination, size) {
        const offset = Number(address - 0x1000n);
        if (offset < 0 || offset + Number(size) > stateBytes.length) {
            return 0;
        }
        new Uint8Array(instance.exports.memory.buffer, destination, Number(size))
            .set(stateBytes.subarray(offset, offset + Number(size)));
        return 1;
    },
    runtime_print_message(pointer, length) {
        messages.push(text(pointer, length));
    },
    timer_set_variable(keyPointer, keyLength, valuePointer, valueLength) {
        const key = text(keyPointer, keyLength);
        if (!variables.has(key)) {
            variables.set(key, text(valuePointer, valueLength));
        }
    },
};

({ instance } = await WebAssembly.instantiate(fs.readFileSync(wasmPath), { env }));
instance.exports._start();
for (let tick = 0; tick < 4 && !variables.has("Returned"); tick += 1) {
    instance.exports.update();
}

const observed = Object.fromEntries(variables);
const expected = {
    current: "StateSnapshot {\n    value: 42,\n    active: true,\n}",
    "measured() + 1": "42",
    Returned: "42",
    evaluations: "1",
    Optional: "Some(\n    1,\n)",
    "print(\"side effect\")": "None",
};
if (JSON.stringify(observed) !== JSON.stringify(expected)) {
    throw new Error(`unexpected inspected values: ${JSON.stringify(observed)}`);
}
if (JSON.stringify(messages) !== JSON.stringify(["side effect"])) {
    throw new Error(`inspect evaluated its operand more than once: ${JSON.stringify(messages)}`);
}

console.log(JSON.stringify({ observed, messages }));
