import assert from "node:assert/strict";
import { readFile } from "node:fs/promises";
import { SplitScriptHost } from "./support/splitscript_host.mjs";
import { createMonoPeFixture } from "./support/mono_pe_fixture.mjs";

const [wasmPath] = process.argv.slice(2);
const profiles = JSON.parse(await readFile(new URL("./fixtures/mono-pe-profiles.json", import.meta.url)));
let cases = 0;
for (const version of ["V2", "V3"]) for (const width of [32, 64]) {
    for (const mode of ["direct", "inherited", "null descriptor", "null definition", "unreadable kind", "negative count", "excessive count", "cycle"]) {
        const profile = profiles.builds.find(p => p.width === width && p.version === version);
        const fixture = createMonoPeFixture(profile);
        const { memory } = fixture;
        const wide = width === 64;
        const write = (at, size, value) => {
            const bytes = new Uint8Array(size), view = new DataView(bytes.buffer);
            if (size === 8) view.setBigUint64(0, BigInt(value), true);
            else if (size === 1) view.setUint8(0, value);
            else view.setUint32(0, Number(value) >>> 0, true);
            bytes.forEach((byte, i) => memory.set(at + BigInt(i), byte));
        };
        const ptr = (at, value) => write(at, width / 8, value);
        const count = BigInt(wide ? 0x100 : version === "V2" ? 0xa4 : 0x9c);
        const kind = BigInt(version === "V2" ? (wide ? 0x2a : 0x1e) : (wide ? 0x1b : 0xf));
        const generic = BigInt(wide ? 0xf0 : version === "V2" ? 0x94 : 0x8c);
        const parent = BigInt(wide ? 0x30 : 0x20);
        const klass = 0x14000n, descriptor = 0x31000n, definition = 0x32000n;
        let owner = klass;
        if (mode === "inherited") {
            owner = 0x30000n;
            for (let i = 0n; i < 0x200n; i++) {
                const byte = memory.get(klass + i);
                if (byte !== undefined) memory.set(owner + i, byte);
            }
            ptr(klass + parent, owner);
            write(klass + count, 4, 0);
            // Separate derived static storage contains the wrong answer.
            const infoOffset = BigInt(wide ? 0xd0 : version === "V2" ? 0x84 : 0x7c);
            ptr(klass + infoOffset, 0x34000n); ptr(0x34000n + BigInt(width / 8), 0x35000n);
            const slots = version === "V2" ? (wide ? 0x40 : 0x28) : (wide ? 0x48 : 0x2c);
            ptr(0x35000n + BigInt(slots + 3 * width / 8), 0x36000n); write(0x36010n, 4, 99);
        }
        write(owner + kind, 1, 0xfb); // Only the low three bits identify the class kind.
        write(owner + count, 4, 0);
        ptr(owner + generic, descriptor); ptr(descriptor, definition);
        write(definition + count, 4, 1);
        // No field array or static storage exists on the generic definition.
        // Reading either there would fail or produce the wrong answer.
        if (mode === "null descriptor") ptr(owner + generic, 0n);
        if (mode === "null definition") ptr(descriptor, 0n);
        if (mode === "unreadable kind") memory.delete(owner + kind);
        if (mode === "negative count") write(definition + count, 4, -1);
        if (mode === "excessive count") write(definition + count, 4, 65536);
        if (mode === "cycle") ptr(owner + parent, owner);
        const host = await SplitScriptHost.instantiate(wasmPath);
        host.addProcess("game.exe", fixture.process); host.start();
        if (mode === "direct" || mode === "inherited") {
            host.updateUntil(() => host.messages.includes("42"), `${version}/${width}/${mode}`);
        } else {
            host.update(200);
            assert(!host.messages.includes("42"), `${version}/${width}/${mode}: invalid generic metadata accepted`);
            if (mode.includes("count") || mode === "cycle") {
                assert(host.messages.some(m => m.includes("metadata work limit")), host.messages.join("\n"));
            } else {
                ptr(owner + generic, descriptor); ptr(descriptor, definition); write(owner + kind, 1, 0xfb);
                host.updateUntil(() => host.messages.includes("42"), "late generic metadata");
            }
        }
        assert(fixture.reads < 5000, `${version}/${width}/${mode}: unbounded metadata work`);
        cases++;
    }
}
console.log(JSON.stringify({ genericMetadataCases: cases }));
