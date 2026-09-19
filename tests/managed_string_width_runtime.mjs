import assert from "node:assert/strict";
import { readFile } from "node:fs/promises";
import { SplitScriptHost } from "./support/splitscript_host.mjs";
import { createMonoPeFixture } from "./support/mono_pe_fixture.mjs";
import { createIl2cppPeFixture } from "./support/il2cpp_pe_fixture.mjs";

const [wasmPath, observeErrors] = process.argv.slice(2);
const profiles = JSON.parse(await readFile(new URL("./fixtures/mono-pe-profiles.json", import.meta.url)));
let cases = 0;
const modes = ["empty", "ascii", "embedded NUL", "pair", "high surrogate", "low surrogate", "exact bound",
    "over bound", "negative length", "unreadable length", "unreadable payload", "null string", "null optional",
    "unreadable optional", "header overflow", "payload overflow", "last address"];
for (const backend of ["mono", "il2cpp"]) for (const width of [32, 64]) for (const mode of modes) {
    const wide = width === 64, bytes = width / 8, mono = backend === "mono";
    const profile = profiles.builds.find(p => p.width === width && p.version === "V2");
    const fixture = mono ? createMonoPeFixture(profile) : createIl2cppPeFixture({ width, version: [2022, 3, 0, 37029] });
    const memory = fixture.memory;
    const write = (at, size, value) => {
        const data = new Uint8Array(size), view = new DataView(data.buffer);
        if (size === 8) view.setBigUint64(0, BigInt(value), true);
        else if (size === 2) view.setUint16(0, value, true);
        else view.setUint32(0, Number(value) >>> 0, true);
        data.forEach((byte, i) => memory.set(at + BigInt(i), byte));
    };
    const ptr = (at, value) => write(at, bytes, value);
    const text = (at, value) => {
        const data = new Uint8Array(256); data.set(new TextEncoder().encode(value));
        data.forEach((byte, i) => memory.set(at + BigInt(i), byte));
    };
    const klass = 0x14000n, fields = 0x50000n, object = 0x70000n, string = 0x80000n;
    const statics = mono ? 0x18000n : 0x16000n;
    ptr(klass + BigInt(mono ? (wide ? 0x98 : 0x60) : (wide ? 0x80 : 0x40)), fields);
    write(klass + BigInt(mono ? (wide ? 0x100 : 0xa4) : (wide ? 0x124 : 0xac)), mono ? 4 : 2, 4);
    ["value", "instance", "text", "optional"].forEach((name, i) => {
        const field = fields + BigInt(i * (wide ? 0x20 : mono ? 0x10 : 0x14));
        const nameAddress = 0x60000n + BigInt(i * 256);
        ptr(field + BigInt(mono ? bytes : 0), nameAddress); text(nameAddress, name);
        write(field + BigInt(wide ? 0x18 : 0xc), 4, [0x10, 0x20, 0x10, 0x18][i]);
    });
    ptr(statics + 0x20n, object);
    const pointAt = address => {
        ptr(statics + 0x10n, address); ptr(object + 0x10n, address); ptr(object + 0x18n, address);
    };
    const payloadOffset = BigInt(2 * bytes + 4);
    const utf16 = (at, units) => {
        write(at + BigInt(2 * bytes), 4, units.length);
        units.forEach((unit, i) => write(at + payloadOffset + BigInt(2 * i), 2, unit));
    };
    pointAt(string); utf16(string, [0x73, 0x65, 0x65, 0x64]);
    const originalRead = fixture.process.read;
    const reads = [];
    fixture.process.read = request => {
        // Wasm i64 arguments cross the JS boundary as signed BigInts.
        const address = BigInt.asUintN(64, request.address);
        reads.push([address, request.length]);
        return originalRead({ ...request, address });
    };
    const host = await SplitScriptHost.instantiate(wasmPath);
    host.addProcess("game.exe", fixture.process); host.start();
    host.updateUntil(() => host.variables.get("snapshot") === "seed", `${backend}/${width}: seed`);
    host.update(2); reads.length = 0;
    let expected = "seed", optional = "some:seed";
    const success = new Map([
        ["empty", [[], ""]], ["ascii", [[65, 66], "AB"]], ["embedded NUL", [[65, 0, 66], "A\0B"]],
        ["pair", [[0xd83d, 0xde00], "😀"]], ["high surrogate", [[0xd800, 65], "�A"]],
        ["low surrogate", [[0xdc00], "�"]], ["exact bound", [Array(8).fill(65), "AAAAAAAA"]],
    ]);
    if (success.has(mode)) {
        const [units, value] = success.get(mode); utf16(string, units); expected = value; optional = `some:${value}`;
    }
    if (mode === "over bound") utf16(string, Array(9).fill(65));
    if (mode === "negative length") write(string + BigInt(2 * bytes), 4, -1);
    if (mode === "unreadable length") memory.delete(string + BigInt(2 * bytes));
    if (mode === "unreadable payload") memory.delete(string + payloadOffset + 2n);
    if (mode === "null string") pointAt(0n);
    if (mode === "null optional") { ptr(object + 0x18n, 0n); optional = "none"; }
    if (mode === "unreadable optional") { memory.delete(object + 0x18n); utf16(string, [110, 101, 119]); }
    const limit = wide ? 0xffffffffffffffffn : 0xffffffffn;
    if (mode === "header overflow") pointAt(limit - 4n);
    if (mode === "payload overflow") {
        const address = limit - payloadOffset; pointAt(address); write(address + BigInt(2 * bytes), 4, 8);
    }
    if (mode === "last address") {
        const address = limit - payloadOffset - 1n; pointAt(address); utf16(address, [65]); expected = "A"; optional = "some:A";
    }
    try { host.update(); }
    catch (error) { throw new Error(`${backend}/${width}/${mode}`, { cause: error }); }
    assert.equal(host.variables.get("snapshot"), expected, `${backend}/${width}/${mode}: snapshot`);
    assert.equal(host.variables.get("optional"), optional, `${backend}/${width}/${mode}: nullability/transaction`);
    assert.equal(host.variables.get("old"), "seed", `${backend}/${width}/${mode}: old snapshot mutated`);
    for (const field of ["static", "live"]) assert.equal(host.variables.get(field), mode === "unreadable optional" ? "new" : expected, `${backend}/${width}/${mode}/${field}`);
    if (observeErrors) {
        const error = success.has(mode) || mode === "null optional" || mode === "last address" ? "ok"
            : mode === "null string" ? "managed field contained a null string"
            : mode === "unreadable optional" ? "managed string field pointer could not be read"
            : "managed string could not be read within its declared maximum length";
        assert.equal(host.variables.get("result"), error, `${backend}/${width}/${mode}: snapshot error payload`);
    }
    for (const [address, length] of reads) assert(address + BigInt(length - 1) <= limit, `${mode}: read beyond target width`);
    if (mode === "over bound" || mode === "negative length") assert(!reads.some(([address]) => address === string + payloadOffset), "invalid string read its payload");
    cases++;
}
console.log(JSON.stringify({ managedStringCases: cases }));
