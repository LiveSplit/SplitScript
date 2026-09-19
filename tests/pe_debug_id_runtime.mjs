import assert from "node:assert/strict";
import { SplitScriptHost } from "./support/splitscript_host.mjs";
import { createPeDebugFixture } from "./support/pe_debug_fixture.mjs";

const wasmPath = process.argv[2];
if (!wasmPath) throw new Error("usage: node tests/pe_debug_id_runtime.mjs <module.wasm>");

const cases = [
    ["PDB 7.0", () => {}, "found"],
    ["no debug slot", f => f.u32(f.directories - 4, 6), "none"],
    ["absent directory", f => { f.u32(f.directories + 48, 0); f.u32(f.directories + 52, 0); }, "none"],
    ["non-CodeView record", f => f.u32(f.entry + 12, 13), "none"],
    ["old NB10 record", f => { f.u32(f.record, 0x3031424e); f.u32(f.entry + 16, 16); }, "none"],
    ["old record before PDB 7.0", f => {
        f.u32(f.table + 12, 2); f.u32(f.table + 16, 16);
        f.u32(f.table + 20, 0x2000); f.u32(0x2000, 0x3031424e);
    }, "found"],
    ["bad DOS", f => f.u16(0, 0), "error"],
    ["bad PE", f => f.u32(f.pe, 0), "error"],
    ["bad optional magic", f => f.u16(f.optional, 0), "error"],
    ["truncated DOS mapping", f => { f.mappedSize = 0x20n; }, "error"],
    ["PE offset outside mapping", f => f.u32(0x3c, 0xfffffff0), "error"],
    ["optional size outside mapping", f => f.u16(f.pe + 20, 0xffff), "error"],
    ["empty optional header", f => f.u16(f.pe + 20, 0), "error"],
    ["truncated optional header", f => f.u16(f.pe + 20, 0x50), "error"],
    ["truncated debug slot", f => f.u16(f.pe + 20, f.directories - f.optional + 48), "error"],
    ["empty image size", f => f.u32(f.optional + 0x38, 0), "error"],
    ["headers outside image", f => f.u32(f.optional + 0x38, 0x100), "error"],
    ["directory outside image", f => f.u32(f.optional + 0x38, f.table + 27), "error"],
    ["directory outside mapping", f => { f.mappedSize = BigInt(f.table + 27); }, "error"],
    ["RVA addition overflow", f => f.u32(f.directories + 48, 0xfffffff0), "error"],
    ["partial directory entry", f => f.u32(f.directories + 52, 29), "error"],
    ["zero directory RVA", f => f.u32(f.directories + 48, 0), "error"],
    ["empty directory size", f => f.u32(f.directories + 52, 0), "error"],
    ["zero record RVA", f => f.u32(f.entry + 20, 0), "error"],
    ["record outside image", f => f.u32(f.entry + 20, 0x3ff0), "error"],
    ["record outside mapping", f => { f.mappedSize = BigInt(f.record + 23); }, "error"],
    ["truncated signature", f => f.u32(f.entry + 16, 3), "error"],
    ["truncated RSDS", f => f.u32(f.entry + 16, 23), "error"],
    ["unreadable directory", f => { f.failAt = f.table; }, "error"],
    ["unreadable GUID", f => { f.failAt = f.record + 4; }, "error"],
    ["unreadable age", f => { f.failAt = f.record + 20; }, "error"],
    ["host address overflow", f => { f.base = 0xffffffffffffffe0n; }, "error"],
    ["bounded directory work", f => {
        f.u32(f.directories + 52, 4097 * 28);
        f.u32(f.optional + 0x38, 0x200000);
        f.mappedSize = 0x200000n;
    }, "error"],
];

let tested = 0;
async function run(name, fixture, expected) {
    const host = await SplitScriptHost.instantiate(wasmPath);
    const reads = [];
    host.addProcess("game.exe", {
        modules: { "runtime.dll": { address: fixture.base, size: fixture.mappedSize } },
        read({ address, length, outputPointer, host }) {
            const offset = address - fixture.base;
            assert(offset >= 0n && offset + BigInt(length) <= fixture.mappedSize,
                `${name}: attempted an out-of-mapping read`);
            assert(address + BigInt(length) <= 0x10000000000000000n,
                `${name}: attempted an overflowing read`);
            reads.push([Number(offset), length]);
            if (Number(offset) === fixture.failAt) return false;
            if (offset + BigInt(length) > BigInt(fixture.bytes.length)) return false;
            host.bytes(outputPointer, length).set(fixture.bytes.subarray(Number(offset), Number(offset) + length));
            return true;
        },
    });
    host.start();
    host.updateUntil(() => host.messages.length > 0, name);
    if (expected === "found") {
        assert.deepEqual(host.messages, ["found", ...Array.from(fixture.guid, String),
            String(fixture.age), "true", "false"], name);
    } else if (expected === "none") {
        assert.deepEqual(host.messages, ["none"], name);
    } else {
        assert.equal(host.messages.length, 1, name);
        assert.match(host.messages[0], /^error:/, name);
    }
    assert(reads.length <= 64, `${name}: metadata work was not bounded`);
    tested++;
}

for (const pointerSize of [4, 8]) {
    for (const [name, change, expected] of cases) {
        const fixture = createPeDebugFixture({ pointerSize });
        change(fixture);
        await run(`${name} / PE${pointerSize * 8}`, fixture, expected);
    }
    // Do not assume a fixed 16-entry table. Ignore unrelated preceding entries.
    await run(`late CodeView / PE${pointerSize * 8}`,
        createPeDebugFixture({ pointerSize, entries: 18, age: 0x89abcdef }), "found");
}
console.log(JSON.stringify({ peDebugIdentityCases: tested }));
