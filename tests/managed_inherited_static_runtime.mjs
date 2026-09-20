import { SplitScriptHost } from "./support/splitscript_host.mjs";
import { createMonoV2Fixture } from "./support/mono_v2_fixture.mjs";

const wasmPath = process.argv[2];
if (!wasmPath) {
    throw new Error("usage: node tests/managed_inherited_static_runtime.mjs <autosplitter.wasm>");
}

const mono = createMonoV2Fixture({
    className: "LevelFlowService",
    fields: [{ name: "_state", offset: 0x20 }],
    parent: {
        className: "Service`1",
        namespace: "Unity.Common",
        fields: [{ name: "_instance", offset: 0x10 }],
    },
});
const instance = 0xc000n;
mono.writeObjectHeader(instance);
mono.writeU64(mono.parentStaticTable + mono.parentFieldOffsets.get("_instance"), instance);
mono.writeI32(instance + mono.fieldOffsets.get("_state"), 42);

// A value at the same offset in the derived class's distinct static table
// catches implementations that discard the field's declaring class.
mono.writeU64(mono.staticTable + mono.parentFieldOffsets.get("_instance"), 0xd000n);
mono.writeObjectHeader(0xd000n);
mono.writeI32(0xd000n + mono.fieldOffsets.get("_state"), 99);

const host = await SplitScriptHost.instantiate(wasmPath);
host.addProcess("game.exe", mono.process);
host.start();
host.updateUntil(
    () => host.messages.length !== 0,
    "the inherited static singleton did not produce a state snapshot",
);

if (host.messages[0] !== "42") {
    throw new Error(`inherited static storage used the wrong declaring class: ${host.json(host.summary())}`);
}

console.log(host.json({ messages: host.messages }));
