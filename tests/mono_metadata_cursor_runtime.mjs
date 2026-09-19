import assert from "node:assert/strict";
import { readFile } from "node:fs/promises";
import { SplitScriptHost } from "./support/splitscript_host.mjs";
import { createMonoPeFixture } from "./support/mono_pe_fixture.mjs";

const [wasmPath] = process.argv.slice(2);
const profiles = JSON.parse(await readFile(new URL("./fixtures/mono-pe-profiles.json", import.meta.url)));
let cases = 0;
for (const width of [32, 64]) for (const mode of ["large", "cancel", "cycle", "negative count", "excessive count"]) {
    const wide = width === 64, bytes = width / 8;
    const profile = profiles.builds.find(p => p.width === width && p.version === "V2");
    const fixture = createMonoPeFixture(profile);
    const { memory } = fixture;
    const write = (at, size, value) => {
        const buffer = new Uint8Array(size), view = new DataView(buffer.buffer);
        if (size === 8) view.setBigUint64(0, BigInt(value), true);
        else view.setUint32(0, Number(value) >>> 0, true);
        buffer.forEach((byte, i) => memory.set(at + BigInt(i), byte));
    };
    const ptr = (at, value) => write(at, bytes, value);
    const next = BigInt(wide ? 0x108 : 0xa8), name = BigInt(wide ? 0x48 : 0x2c);
    if (mode === "large" || mode === "cancel") {
        const text = new Uint8Array(256); text.set(new TextEncoder().encode("Unrelated"));
        text.forEach((byte, i) => memory.set(0x70000n + BigInt(i), byte));
        ptr(0x13000n, 0x40000n);
        for (let i = 0; i < 192; i++) {
            const klass = 0x40000n + BigInt(i * 0x200);
            ptr(klass + name, 0x70000n);
            ptr(klass + next, i === 191 ? 0x14000n : klass + 0x200n);
        }
    }
    if (mode === "cycle") ptr(0x14000n + next, 0x14000n);
    if (mode.includes("count")) write(0x12000n + BigInt(wide ? 0x4d8 : 0x360), 4, mode === "negative count" ? -1 : 1048577);
    const host = await SplitScriptHost.instantiate(wasmPath);
    host.addProcess("game.exe", fixture.process); host.start();
    let ticks = 0, maxReads = 0;
    while (ticks < 50 && !host.messages.includes("42") && !host.messages.some(m => m.includes("Mono class cache"))) {
        const before = fixture.reads; host.update(); maxReads = Math.max(maxReads, fixture.reads - before); ticks++;
        if (mode === "cancel" && ticks === 2) break;
    }
    if (mode === "large") {
        assert(host.messages.includes("42"));
        assert(ticks >= 4, "large class cache monopolized one update");
        assert(maxReads < 600, `class-cache poll made ${maxReads} reads`);
    } else {
        assert(!host.messages.includes("42"));
        if (mode !== "cancel") assert(host.messages.some(m => m.includes("Mono class cache")), host.messages.join("\n"));
        host.setProcessOpen("game.exe", false); host.update();
        host.addProcess("game.exe", createMonoPeFixture(profile).process);
        host.updateUntil(() => host.messages.includes("42"), "restart after cache traversal");
    }
    cases++;
}
console.log(JSON.stringify({ metadataCursorCases: cases }));
