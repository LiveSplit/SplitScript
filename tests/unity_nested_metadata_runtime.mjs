import assert from "node:assert/strict";
import { readFile } from "node:fs/promises";
import { SplitScriptHost } from "./support/splitscript_host.mjs";
import { createIl2cppPeFixture } from "./support/il2cpp_pe_fixture.mjs";
import { createMonoPeFixture } from "./support/mono_pe_fixture.mjs";

const [wasmPath] = process.argv.slice(2);
const profiles = JSON.parse(await readFile(new URL("./fixtures/mono-pe-profiles.json", import.meta.url)));
let cases = 0;
for (const backend of ["mono", "il2cpp"]) for (const width of [32, 64]) {
    const wide = width === 64, bytes = width / 8;
    function fixture(mode = "valid") {
        const profile = profiles.builds.find(p => p.width === width && p.version === "V2");
        const result = backend === "mono" ? createMonoPeFixture(profile)
            : createIl2cppPeFixture({ width, version: [2022, 3, 0, 37029] });
        const memory = result.memory;
        const write = (at, value) => { for (let i = 0; i < value.length; i++) memory.set(at + BigInt(i), value[i]); };
        const ptr = (at, value) => {
            const buffer = new Uint8Array(bytes), view = new DataView(buffer.buffer);
            if (wide) view.setBigUint64(0, value, true); else view.setUint32(0, Number(value), true);
            write(at, buffer);
        };
        const text = (at, value) => {
            const buffer = new Uint8Array(256); buffer.set(new TextEncoder().encode(value)); write(at, buffer);
        };
        // Independent metadata members for modern Mono and IL2CPP.
        const name = BigInt(backend === "mono" ? (wide ? 0x48 : 0x2c) : (wide ? 0x10 : 8));
        const namespace = BigInt(backend === "mono" ? (wide ? 0x50 : 0x30) : (wide ? 0x18 : 0xc));
        const declaring = BigInt(backend === "mono" ? (wide ? 0x38 : 0x24) : (wide ? 0x50 : 0x28));
        const leaf = 0x14000n, middle = 0x30000n, outer = 0x31000n, decoy = 0x32000n;
        text(0x22000n, "Leaf"); text(0x23000n, "LeafNamespaceMustBeIgnored");
        for (const [klass, label, space, parent, stringBase] of [
            [middle, "Middle", "MiddleNamespaceMustBeIgnored", outer, 0x40000n],
            [outer, "Outer", "Game", 0n, 0x40200n],
            [decoy, "Leaf", "Game", 0n, 0x40400n],
        ]) {
            ptr(klass + name, stringBase); text(stringBase, label);
            ptr(klass + namespace, stringBase + 0x100n); text(stringBase + 0x100n, space);
            ptr(klass + declaring, parent);
        }
        ptr(leaf + declaring, middle);
        if (backend === "mono") {
            const next = BigInt(wide ? 0x108 : 0xa8);
            ptr(leaf + next, decoy); ptr(decoy + next, 0n);
        } else ptr(0x13000n + BigInt(9 * bytes), decoy);
        if (mode === "wrong outer") text(0x40200n, "Other");
        if (mode === "wrong namespace") text(0x40300n, "Other");
        if (mode === "missing enclosure") ptr(leaf + declaring, 0n);
        if (mode === "extra enclosure") ptr(outer + declaring, middle);
        if (mode === "cycle") ptr(middle + declaring, middle);
        if (mode === "unreadable enclosure") memory.delete(middle + name);
        if (mode === "ambiguous") {
            ptr(decoy + declaring, middle);
        }
        if (mode === "unreadable enclosure") result.repair = () => ptr(middle + name, 0x40000n);
        return result;
    }
    for (const mode of ["valid", "wrong outer", "wrong namespace", "missing enclosure", "extra enclosure", "cycle", "unreadable enclosure", "ambiguous"]) {
        const remote = fixture(mode);
        const host = await SplitScriptHost.instantiate(wasmPath);
        host.addProcess("game.exe", remote.process); host.start();
        if (mode === "valid") {
            host.updateUntil(() => host.messages.includes("42"), `${backend}/${width}: nested class`);
        } else {
            host.update(200);
            assert(!host.messages.includes("42"), `${backend}/${width}/${mode}: invalid class selected`);
            if (mode === "ambiguous") assert(host.messages.some(m => m.includes("multiple Unity classes")), host.messages.join("\n"));
            if (mode === "unreadable enclosure") {
                remote.repair();
                host.updateUntil(() => host.messages.includes("42"), "retry after metadata becomes readable");
            } else {
                host.setProcessOpen("game.exe", false); host.update();
                host.addProcess("game.exe", fixture().process);
                host.updateUntil(() => host.messages.includes("42"), "reattach after rejected nested class");
            }
        }
        assert(remote.reads < 5000, `${backend}/${width}/${mode}: unbounded metadata work`);
        cases++;
    }
}
for (const width of [32, 64]) {
    const host = await SplitScriptHost.instantiate(wasmPath);
    const fixture = createMonoPeFixture({ version: "V1Cattrs", width, exact: false });
    host.addProcess("game.exe", fixture.process); host.start();
    host.updateUntil(() => host.messages.includes("Unity profile lacks nested class metadata"), "reject unsupported nested metadata");
    assert(!host.messages.includes("42"));
    cases++;
}
console.log(JSON.stringify({ nestedMetadataCases: cases }));
