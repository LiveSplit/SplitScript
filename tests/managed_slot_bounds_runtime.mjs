import assert from 'node:assert/strict';
import { readFile } from 'node:fs/promises';
import { SplitScriptHost } from './support/splitscript_host.mjs';
import { createMonoPeFixture } from './support/mono_pe_fixture.mjs';
import { createIl2cppPeFixture } from './support/il2cpp_pe_fixture.mjs';

const [wasm] = process.argv.slice(2);
const profiles = JSON.parse(await readFile(new URL('./fixtures/mono-pe-profiles.json', import.meta.url)));
const modes = ['addition overflow', 'scalar span overflow', 'scalar last address',
    'reference span overflow', 'reference last address', 'string span overflow', 'string last address',
    'static addition overflow', 'static span overflow', 'static last address', 'static invalid cancellation'];
let cases = 0;
for (const mono of [true, false]) for (const width of [32, 64]) for (const mode of modes) {
    const wide = width === 64, bytes = width / 8, limit = (1n << BigInt(width)) - 1n;
    const fixture = mono ? createMonoPeFixture(profiles.builds.find(p => p.width === width && p.version === 'V2'))
        : createIl2cppPeFixture({width, version:[2022, 3, 0, 37029]});
    const memory = fixture.memory;
    const write = (at, size, value) => {
        const data = new Uint8Array(size), view = new DataView(data.buffer);
        if (size === 8) view.setBigUint64(0, BigInt(value), true);
        else if (size === 2) view.setUint16(0, Number(value), true);
        else view.setUint32(0, Number(value), true);
        data.forEach((byte, i) => memory.set(at + BigInt(i), byte));
    };
    const ptr = (at, value) => write(at, bytes, value);
    const name = (at, value) => {
        const data = new Uint8Array(256); data.set(new TextEncoder().encode(value));
        data.forEach((byte, i) => memory.set(at + BigInt(i), byte));
    };
    const klass = 0x14000n, fields = 0x50000n, root = 0x70000n, string = 0x90000n;
    const normalStatics = mono ? 0x18000n : 0x16000n;
    const staticPointerSlot = mono ? 0x17000n + BigInt((wide ? 0x40 : 0x28) + 3 * bytes)
        : klass + BigInt(wide ? 0xb8 : 0x5c);
    ptr(klass + BigInt(mono ? (wide ? 0x98 : 0x60) : (wide ? 0x80 : 0x40)), fields);
    write(klass + BigInt(mono ? (wide ? 0x100 : 0xa4) : (wide ? 0x124 : 0xac)), mono ? 4 : 2, 5);
    ['instance', 'counter', 'value', 'next', 'text'].forEach((text, i) => {
        const field = fields + BigInt(i * (wide ? 32 : mono ? 16 : 20));
        const nameAddress = 0x60000n + BigInt(i * 256);
        ptr(field + BigInt(mono ? bytes : 0), nameAddress); name(nameAddress, text);
        write(field + BigInt(wide ? 0x18 : 0xc), 4, [0x10, 0x20, 0x10, 0x20, 0x30][i]);
    });
    const object = (at, value) => {
        write(at + 0x10n, 8, value); ptr(at + 0x20n, 0); ptr(at + 0x30n, string);
    };
    const statics = at => { ptr(at + 0x10n, root); write(at + 0x20n, 8, 55); };
    statics(normalStatics); object(root, 41);
    write(string + BigInt(2 * bytes), 4, 1); write(string + BigInt(2 * bytes + 4), 2, 120);
    // These bytes are deliberately readable. An unchecked x64 addition must
    // not turn an invalid high object/table into a successful low-memory read.
    for (let at = 0n; at < 256n; at++) memory.set(at, 99);
    const reads = [], read = fixture.process.read;
    fixture.process.read = request => {
        const address = BigInt.asUintN(64, request.address);
        reads.push([address, request.length]);
        return read({...request, address});
    };
    const label = `${mono ? 'mono' : 'il2cpp'}/${width}/${mode}`;
    let initialTable = normalStatics;
    if (mode === 'static addition overflow' || mode === 'static invalid cancellation') initialTable = limit - 7n;
    if (mode === 'static span overflow') initialTable = limit - 0x20n - 3n;
    if (mode === 'static last address') initialTable = limit - 0x20n - 7n;
    if (initialTable !== normalStatics) { statics(initialTable); ptr(staticPointerSlot, initialTable); }
    const host = await SplitScriptHost.instantiate(wasm);
    host.addProcess('game.exe', fixture.process); host.start();
    if (mode === 'static addition overflow' || mode === 'static span overflow' || mode === 'static invalid cancellation') {
        for (let i = 0; i < 300; i++) {
            const before = reads.length; host.update();
            assert(reads.length - before < 500, `${label}: unbounded retry`);
        }
        assert.equal(host.variables.get('static'), undefined, `${label}: invalid static storage published state`);
        if (mode === 'static addition overflow' || mode === 'static invalid cancellation') {
            if (mode === 'static invalid cancellation') {
                host.setProcessOpen('game.exe', false);
                const before = reads.length; host.update(5);
                assert.equal(host.detaches.length, 1, `${label}: pending binding did not detach`);
                assert.equal(reads.length, before, `${label}: closed process still read`);
                host.setProcessOpen('game.exe', true);
            }
            ptr(staticPointerSlot, normalStatics);
            host.updateUntil(() => host.variables.get('static') === '55', `${label}: did not rediscover corrected storage`);
        }
    } else {
        host.updateUntil(() => host.variables.get('snapshot') === '41', `${label}: seed`);
        host.update(2); reads.length = 0;
        let value = 41, snapshot = 41, text = 'x', next = 0;
        let at;
        if (mode === 'addition overflow') at = limit - 7n;
        if (mode === 'scalar span overflow') at = limit - 0x10n - 3n;
        if (mode === 'scalar last address') { at = limit - 0x10n - 7n; value = 99; }
        if (mode === 'reference span overflow') { at = limit - 0x20n - BigInt(bytes - 2); value = 99; }
        if (mode === 'reference last address') { at = limit - 0x20n - BigInt(bytes - 1); value = 99; }
        if (mode === 'string span overflow') { at = limit - 0x30n - BigInt(bytes - 2); value = 99; }
        if (mode === 'string last address') { at = limit - 0x30n - BigInt(bytes - 1); value = snapshot = 99; text = 'y'; }
        if (at !== undefined) {
            object(at, 99); ptr(normalStatics + 0x10n, at);
            write(string + BigInt(2 * bytes + 4), 2, 121);
        }
        host.update();
        assert.equal(host.variables.get('live'), String(value), label);
        assert.equal(host.variables.get('next'), String(next), label);
        assert.equal(host.variables.get('text'), text, label);
        assert.equal(host.variables.get('snapshot'), String(snapshot), label);
        assert.equal(host.variables.get('old'), '41', `${label}: old changed`);
        assert.equal(host.variables.get('static'), '55', label);
        if (snapshot === 99 || mode === 'static last address') assert.equal(host.variables.get('result'), 'ok', label);
        else assert.match(host.variables.get('result'), /managed/, `${label}: failure was lost`);
    }
    for (const [address, length] of reads) {
        assert(address >= 0x1000n, `${label}: invalid address wrapped into low memory`);
        assert(address + BigInt(Math.max(1, length) - 1) <= limit, `${label}: host read crossed target width`);
    }
    cases++;
}
console.log(JSON.stringify({managedSlotCases:cases}));
