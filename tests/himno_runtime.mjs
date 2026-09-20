import { SplitScriptHost } from "./support/splitscript_host.mjs";
import { createMonoV2Fixture } from "./support/mono_v2_fixture.mjs";

const wasmPath = process.argv[2];
if (!wasmPath) {
    throw new Error("usage: node tests/himno_runtime.mjs <himno.wasm>");
}

const mono = createMonoV2Fixture({
    className: "PlayerStats",
    fields: [
        { name: "script", offset: 0x10 },
        { name: "currDistrict", offset: 0x20 },
        { name: "inRun", offset: 0x24 },
    ],
});
const scriptPointer = mono.staticTable + mono.fieldOffsets.get("script");
const firstStats = 0xb000n;
const runningStats = 0xb100n;
const resetStats = 0xb200n;

const writeStats = (address, district, inRun) => {
    mono.writeObjectHeader(address);
    mono.writeI32(address + mono.fieldOffsets.get("currDistrict"), district);
    mono.writeBytes(address + mono.fieldOffsets.get("inRun"), [inRun ? 1 : 0]);
};
writeStats(firstStats, 1, false);
writeStats(runningStats, 1, true);
writeStats(resetStats, 1, false);
mono.writeU64(scriptPointer, firstStats);

const host = await SplitScriptHost.instantiate(wasmPath);
host.addProcess("Himno.exe", mono.process);
host.start();
host.update(12);

// Replacing the singleton, rather than mutating the original object, proves
// that the source MemoryPath follows the static pointer on every state poll.
mono.writeU64(scriptPointer, runningStats);
host.updateUntil(
    () => host.timerCalls.starts === 1,
    "Himno did not start after replacing the PlayerStats singleton",
);

mono.writeI32(runningStats + mono.fieldOffsets.get("currDistrict"), 11);
host.update();
mono.writeI32(runningStats + mono.fieldOffsets.get("currDistrict"), 12);
host.update();
if (host.timerCalls.splits !== 1) {
    throw new Error(`Himno district transition did not split: ${host.json(host.summary())}`);
}

mono.writeU64(scriptPointer, resetStats);
host.updateUntil(
    () => host.timerCalls.resets === 1,
    "Himno did not reset after replacing the singleton with menu state",
);

console.log(host.json({
    starts: host.timerCalls.starts,
    splits: host.timerCalls.splits,
    resets: host.timerCalls.resets,
}));
