import assert from "node:assert/strict";
import { SplitScriptHost } from "./support/splitscript_host.mjs";
import { createElfIdentityFixture, createMachIdentityFixture } from "./support/unix_identity_fixture.mjs";

const [wasmPath, format] = process.argv.slice(2);
if (!wasmPath || !["elf", "mach"].includes(format)) {
    throw new Error("usage: node tests/unix_identity_runtime.mjs <module.wasm> <elf|mach>");
}
let tested = 0;
async function run(name, fixture, expected) {
    const host = await SplitScriptHost.instantiate(wasmPath, {
        operatingSystem: format === "elf" ? "linux" : "macos",
    });
    let reads = 0;
    host.addProcess("game.exe", {
        modules: { runtime: { address: fixture.base, size: fixture.mappedSize } },
        read({ address, length, outputPointer, host }) {
            const offset = address - fixture.base;
            assert(offset >= 0n && offset + BigInt(length) <= fixture.mappedSize,
                `${name}: out-of-mapping read`);
            assert(address + BigInt(length) <= 0x10000000000000000n, `${name}: overflowing read`);
            assert(++reads <= (fixture.maxReads ?? 100), `${name}: excessive metadata work`);
            if (Number(offset) === fixture.failAt) return false;
            if (offset + BigInt(length) > BigInt(fixture.bytes.length)) return false;
            host.bytes(outputPointer, length).set(fixture.bytes.subarray(Number(offset), Number(offset) + length));
            return true;
        },
    });
    host.start();
    host.updateUntil(() => host.messages.length > 0, name);
    if (expected === "found") {
        assert.deepEqual(host.messages, ["found", ...Array.from(fixture.identity, String)], name);
    } else if (expected === "none") {
        assert.deepEqual(host.messages, ["none"], name);
    } else {
        assert.equal(host.messages.length, 1, name);
        assert.match(host.messages[0], /^error:/, name);
    }
    tested++;
}

if (format === "elf") {
    const cases = [
        ["build ID", () => {}, "found"],
        ["no headers", f => f.u16(f.sizes + 4, 0), "none"],
        ["no notes", f => f.u32(f.noteEntry, 0), "none"],
        ["other note type", f => f.u32(f.buildNote + 8, 7), "none"],
        ["other note name", f => f.u32(f.buildNote + 12, 0), "none"],
        ["wrong magic", f => f.u32(0, 0), "error"],
        ["big endian", f => { f.bytes[5] = 2; }, "error"],
        ["wrong class", f => { f.bytes[4] = 3; }, "error"],
        ["wrong ident version", f => { f.bytes[6] = 2; }, "error"],
        ["wrong image version", f => f.u32(20, 2), "error"],
        ["relocatable file", f => f.u16(16, 1), "error"],
        ["truncated ident", f => { f.mappedSize = 15n; }, "error"],
        ["truncated header", f => { f.mappedSize = 48n; }, "error"],
        ["short declared header", f => f.u16(f.sizes, 12), "error"],
        ["oversized declared header", f => f.u16(f.sizes, 0xffff), "error"],
        ["short entry", f => f.u16(f.sizes + 2, 31), "error"],
        ["extended count", f => f.u16(f.sizes + 4, 0xffff), "error"],
        ["bounded header count", f => f.u16(f.sizes + 4, 1025), "error"],
        ["table past end", f => f.wide ? f.u64(32, 0xfffffffffffffff0n) : f.u32(28, 0xfffffff0), "error"],
        ["table crosses end", f => { f.mappedSize = BigInt(f.table + f.entrySize); }, "error"],
        ["no header load segment", f => f.u32(f.table, 0), "error"],
        ["short header load segment", f => f.wide ? f.u64(f.table + 32, 0) : f.u32(f.table + 16, 0), "error"],
        ["unreadable segment", f => { f.failAt = f.table; }, "error"],
        ["note outside mapping", f => f.noteAddress(0x3fffn), "error"],
        ["note size overflow", f => f.noteLength(f.wide ? 0xffffffffffffffffn : 0xffffffff), "error"],
        ["partial note header", f => f.noteLength(8), "error"],
        ["truncated note data", f => f.noteLength(f.noteSize - 1), "error"],
        ["overflowing name length", f => f.u32(f.note, 0xffffffff), "error"],
        ["overflowing data length", f => f.u32(f.note + 4, 0xffffffff), "error"],
        ["unreadable note", f => { f.failAt = f.note; }, "error"],
        ["unreadable GNU name", f => { f.failAt = f.buildNote + 12; }, "error"],
        ["unreadable ID byte", f => { f.failAt = f.buildNote + 17; }, "error"],
        ["host address overflow", f => { f.base = 0xfffffffffffffff8n; }, "error"],
    ];
    for (const pointerSize of [4, 8]) {
        for (const [name, change, expected] of cases) {
            const fixture = createElfIdentityFixture({ pointerSize });
            change(fixture);
            await run(`${name} / ELF${pointerSize * 8}`, fixture, expected);
        }
        for (const idLength of [1, 16, 32, 0, 33]) {
            await run(`ID length ${idLength} / ELF${pointerSize * 8}`,
                createElfIdentityFixture({ pointerSize, idLength }), idLength > 0 && idLength <= 32 ? "found" : "error");
        }
        for (const [base, preferredBase] of [[0x400000n, 0x400000n], [0x1000n, 0x400000n]]) {
            await run(`load bias / ELF${pointerSize * 8}`, createElfIdentityFixture({
                pointerSize, base, preferredBase, entryPadding: 8,
            }), "found");
        }
        const beforeHeader = createElfIdentityFixture({ pointerSize, preferredBase: 0x400000n });
        beforeHeader.noteAddress(0x3fffffn);
        await run(`notes before load base / ELF${pointerSize * 8}`, beforeHeader, "error");

        const several = createElfIdentityFixture({ pointerSize });
        several.u16(several.sizes + 4, 3);
        several.segment(several.noteEntry, 4, 0, 0x1000n, 12);
        several.segment(several.noteEntry + several.entrySize, 4, 0, BigInt(several.note), several.noteSize);
        await run(`several note segments / ELF${pointerSize * 8}`, several, "found");

        const late = createElfIdentityFixture({ pointerSize });
        late.bytes.fill(0, late.note);
        const lateNote = late.note + 17 * 12;
        late.u32(lateNote, 4); late.u32(lateNote + 4, late.identity.length); late.u32(lateNote + 8, 3);
        late.bytes.set([71, 78, 85, 0], lateNote + 12);
        late.bytes.set(late.identity, lateNote + 16);
        late.noteLength(17 * 12 + 16 + late.identity.length);
        await run(`late GNU note / ELF${pointerSize * 8}`, late, "found");

        const bounded = createElfIdentityFixture({ pointerSize, imageSize: 0x20000 });
        bounded.bytes.fill(0, bounded.note);
        bounded.noteLength(4097 * 12);
        bounded.maxReads = 4200;
        await run(`bounded note count / ELF${pointerSize * 8}`, bounded, "error");
    }
} else {
    const cases = [
        ["UUID", () => {}, "found"],
        ["no UUID", f => f.u32(f.uuid, 7), "none"],
        ["empty command table", f => { f.u32(16, 0); f.u32(20, 0); }, "none"],
        ["32-bit image", f => f.u32(0, 0xfeedface), "error"],
        ["big endian image", f => f.u32(0, 0xcffaedfe), "error"],
        ["universal file", f => f.u32(0, 0xbebafeca), "error"],
        ["truncated header", f => { f.mappedSize = 31n; }, "error"],
        ["oversized table", f => f.u32(20, 0xfffffff0), "error"],
        ["too many commands", f => f.u32(16, 4097), "error"],
        ["command beyond table", f => f.u32(20, 4), "error"],
        ["zero command size", f => f.u32(36, 0), "error"],
        ["short command size", f => f.u32(36, 4), "error"],
        ["unaligned command size", f => f.u32(36, 10), "error"],
        ["command crosses table", f => f.u32(36, 0x1000), "error"],
        ["truncated UUID", f => f.u32(f.uuid + 4, 16), "error"],
        ["count exceeds table", f => { f.u32(f.uuid, 7); f.u32(16, 3); }, "error"],
        ["trailing table bytes", f => { f.u32(f.uuid, 7); f.u32(16, 1); }, "error"],
        ["unreadable UUID", f => { f.failAt = f.uuid + 8; }, "error"],
        ["host address overflow", f => { f.base = 0xfffffffffffffff8n; }, "error"],
    ];
    for (const cpu of [0x1000007, 0x100000c]) {
        for (const [name, change, expected] of cases) {
            const fixture = createMachIdentityFixture({ cpu });
            change(fixture);
            await run(`${name} / CPU ${cpu}`, fixture, expected);
        }
        await run(`late UUID / CPU ${cpu}`, createMachIdentityFixture({ cpu, commands: 18 }), "found");
    }
}
console.log(JSON.stringify({ format, identityCases: tested }));
