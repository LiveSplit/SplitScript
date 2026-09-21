import assert from "node:assert/strict";
import { readFile } from "node:fs/promises";
import { SplitScriptHost } from "./support/splitscript_host.mjs";
import { createMonoPeFixture } from "./support/mono_pe_fixture.mjs";
import { createIl2cppPeFixture } from "./support/il2cpp_pe_fixture.mjs";

const [wasmPath] = process.argv.slice(2);
const grouped = process.argv.includes("--grouped");
const profiles = JSON.parse(await readFile(new URL("./fixtures/mono-pe-profiles.json", import.meta.url)));
let cases = 0;
for (const backend of ["mono", "il2cpp"]) for (const width of [32, 64]) {
    const wide = width === 64, bytes = width / 8;
    const profile = profiles.builds.find(p => p.width === width && p.version === "V2");
    const modes = grouped
        ? ["direct", "inherited", "shadowed", "ambiguous aliases", "unreadable offset", "large", "late grouped offset"]
        : ["direct", "backing", "hole", "same slot", "ambiguous aliases", "inherited", "shadowed",
        "unreadable name", "unreadable offset", "null field array", "negative offset", "large",
        "ordinary MonoBehaviour", "System MonoBehaviour", "ordinary Object", "UnityEngine Object",
        "engine boundary", "object boundary"];
    function fixture(mode) {
        const fixture = backend === "mono" ? createMonoPeFixture(profile)
            : createIl2cppPeFixture({ width, version: [2022, 3, 0, 37029] });
        const memory = fixture.memory;
        const write = (at, size, value) => {
            const buffer = new Uint8Array(size), view = new DataView(buffer.buffer);
            if (size === 8) view.setBigUint64(0, BigInt(value), true);
            else if (size === 2) view.setUint16(0, Number(value), true);
            else view.setUint32(0, Number(value) >>> 0, true);
            buffer.forEach((byte, i) => memory.set(at + BigInt(i), byte));
        };
        const ptr = (at, value) => write(at, bytes, value);
        const text = (at, value) => {
            const buffer = new Uint8Array(256); buffer.set(new TextEncoder().encode(value));
            buffer.forEach((byte, i) => memory.set(at + BigInt(i), byte));
        };
        const mono = backend === "mono";
        const parent = BigInt(mono ? (wide ? 0x30 : 0x20) : (wide ? 0x58 : 0x2c));
        const name = BigInt(mono ? (wide ? 0x48 : 0x2c) : (wide ? 0x10 : 8));
        const namespace = BigInt(mono ? (wide ? 0x50 : 0x30) : (wide ? 0x18 : 0xc));
        const fields = BigInt(mono ? (wide ? 0x98 : 0x60) : (wide ? 0x80 : 0x40));
        const count = BigInt(mono ? (wide ? 0x100 : 0xa4) : (wide ? 0x124 : 0xac));
        const countBytes = mono ? 4 : 2;
        const fieldName = BigInt(mono ? bytes : 0), fieldValue = BigInt(wide ? 0x18 : 0xc);
        const stride = BigInt(wide ? 0x20 : mono ? 0x10 : 0x14);
        const klass = 0x14000n, base = 0x30000n, table = 0x50000n;
        ptr(klass + fields, table);
        ptr(table + fieldName, 0x60000n); text(0x60000n, "first"); write(table + fieldValue, 4, 0x10);
        let repair;
        const inherited = ["inherited", "shadowed", "ordinary MonoBehaviour", "System MonoBehaviour", "ordinary Object", "UnityEngine Object", "engine boundary", "object boundary"].includes(mode);
        if (inherited) {
            for (let i = 0n; i < 0x200n; i++) {
                const byte = memory.get(klass + i);
                if (byte !== undefined) memory.set(base + i, byte);
            }
            ptr(klass + parent, base);
            if (mode !== "shadowed") write(klass + count, countBytes, 0);
            ptr(base + name, 0x61000n); text(0x61000n, "Base");
            ptr(base + namespace, 0x61100n); text(0x61100n, "Game");
            // The base retains the original static table. A mistaken read
            // against the derived class must return 99, not the expected 42.
            if (mono) {
                ptr(klass + BigInt(wide ? 0xd0 : 0x84), 0x34000n);
                ptr(0x34000n + BigInt(bytes), 0x35000n);
                ptr(0x35000n + BigInt((wide ? 0x40 : 0x28) + 3 * bytes), 0x36000n);
            } else ptr(klass + BigInt(wide ? 0xb8 : 0x5c), 0x36000n);
            write(0x36010n, 4, 99);
            if (mode.includes("MonoBehaviour") || mode === "engine boundary") text(0x61000n, "MonoBehaviour");
            if (mode.includes("Object") || mode === "object boundary") text(0x61000n, "Object");
            if (mode === "engine boundary" || mode === "UnityEngine Object") text(0x61100n, "UnityEngine");
            if (mode === "object boundary" || mode === "System MonoBehaviour") text(0x61100n, "System");
        }
        if (mode === "backing") text(0x60000n, "<second>k__BackingField");
        if (["hole", "same slot", "ambiguous aliases", "unreadable name"].includes(mode)) {
            write(klass + count, countBytes, 2);
            ptr(table + stride + fieldName, 0x60100n); text(0x60100n, "second");
            write(table + stride + fieldValue, 4, mode === "ambiguous aliases" ? 0x20 : 0x10);
            if (mode === "hole") ptr(table + fieldName, 0n);
            if (mode === "unreadable name") {
                memory.delete(table + fieldName);
                repair = () => ptr(table + fieldName, 0x60000n);
            }
        }
        if (mode === "unreadable offset") {
            memory.delete(table + fieldValue);
            repair = () => write(table + fieldValue, 4, 0x10);
        }
        if (mode === "null field array") {
            ptr(klass + fields, 0n); repair = () => ptr(klass + fields, table);
        }
        if (mode === "negative offset") write(table + fieldValue, 4, -1);
        if (mode === "large") {
            text(0x60200n, "unrelated");
            write(klass + count, countBytes, 192);
            for (let i = 0; i < 192; i++) {
                ptr(table + BigInt(i) * stride + fieldName, i === 191 ? 0x60000n : 0x60200n);
                write(table + BigInt(i) * stride + fieldValue, 4, 0x10);
            }
        }
        if (grouped) {
            const owner = inherited ? base : klass;
            let previousCount = 0;
            for (let byte = 0; byte < countBytes; byte++) {
                previousCount += memory.get(owner + count + BigInt(byte)) * 2 ** (byte * 8);
            }
            for (const [index, field] of ["extraOne", "extraTwo", "extraThree"].entries()) {
                const entry = table + BigInt(previousCount + index) * stride;
                const fieldText = 0x62000n + BigInt(index) * 0x100n;
                const offset = 0x14 + index * 4;
                ptr(entry + fieldName, fieldText); text(fieldText, field);
                write(entry + fieldValue, 4, offset);
                write((mono ? 0x18000n : 0x16000n) + BigInt(offset), 4, 43 + index);
                if (inherited) write(0x36000n + BigInt(offset), 4, 99);
            }
            write(owner + count, countBytes, previousCount + 3);
            if (mode === "late grouped offset") {
                const entry = table + BigInt(previousCount + 1) * stride;
                memory.delete(entry + fieldValue);
                repair = () => write(entry + fieldValue, 4, 0x18);
            }
        }
        return { fixture, repair };
    }
    for (const mode of modes) {
        const { fixture: remote, repair } = fixture(mode);
        const host = await SplitScriptHost.instantiate(wasmPath);
        host.addProcess("game.exe", remote.process); host.start();
        const rejected = ["ambiguous aliases", "shadowed", "negative offset", "engine boundary", "object boundary"].includes(mode);
        if (rejected || repair) {
            host.update(150);
            assert(!host.messages.includes("42"), `${backend}/${width}/${mode}: invalid field published state`);
            if (repair) {
                assert(!host.messages.some(message => message.startsWith("no Unity field")), "unreadable metadata was treated as absence");
                repair(); host.updateUntil(() => host.messages.includes("42"), "late field metadata");
            } else {
                const expected = mode.includes("boundary") ? "no Unity field" : mode === "negative offset" ? "thread-static" : "multiple Unity fields";
                assert(host.messages.some(message => message.includes(expected)
                    && message.includes("first") && message.includes("second")),
                    `${mode}: diagnostic must retain the reason and both field aliases: ${host.messages}`);
                host.setProcessOpen("game.exe", false); host.update();
                host.addProcess("game.exe", fixture("direct").fixture.process);
                host.updateUntil(() => host.messages.includes("42"), "reattach after rejected fields");
            }
        } else {
            let ticks = 0, maxReads = 0;
            while (ticks < 100 && !host.messages.includes("42")) {
                const before = remote.reads; host.update(); maxReads = Math.max(maxReads, remote.reads - before); ticks++;
            }
            assert(host.messages.includes("42"), `${backend}/${width}/${mode}: ${host.messages}`);
            if (mode === "large") {
                assert(ticks >= 4, "field scan monopolized one update");
                assert(maxReads < 600, `field scan performed ${maxReads} reads in one update`);
            }
        }
        cases++;
    }
}
console.log(JSON.stringify({ sharedFieldCursorCases: cases, grouped }));
