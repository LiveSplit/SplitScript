import {writeManagedObjectHeader} from './support/managed_type_fixture.mjs';
import assert from 'node:assert/strict';
import { readFile } from 'node:fs/promises';
import { SplitScriptHost } from './support/splitscript_host.mjs';
import { createMonoPeFixture } from './support/mono_pe_fixture.mjs';
import { createIl2cppPeFixture } from './support/il2cpp_pe_fixture.mjs';

const [wasm, policy] = process.argv.slice(2);
const boundedPaths = policy === '--bounded-paths';
const workBudget = policy === '--work-budget';
const profiles = JSON.parse(await readFile(new URL('./fixtures/mono-pe-profiles.json', import.meta.url)));
let cases = 0;
for (const mono of [true, false]) for (const width of [32, 64]) {
    const modes = workBudget ? ['work below limit', 'work overflow']
        : ['null', 'chain', 'depth boundary', 'depth overflow', 'shared tree', 'object overflow', 'self cycle', 'ancestor cycle', 'unreadable optional'];
    for (const mode of modes) {
        const wide = width === 64, bytes = width / 8;
        const fixture = mono ? createMonoPeFixture(profiles.builds.find(p => p.width === width && p.version === 'V2'))
            : createIl2cppPeFixture({width, version:[2022, 3, 0, 37029]});
        const memory = fixture.memory;
        const number = (at, size, value) => {
            const data = new Uint8Array(size), view = new DataView(data.buffer);
            if (size === 8) view.setBigUint64(0, BigInt(value), true);
            else if (size === 2) view.setUint16(0, Number(value), true);
            else view.setUint32(0, Number(value), true);
            data.forEach((byte, i) => memory.set(at + BigInt(i), byte));
        };
        const ptr = (at, value) => number(at, bytes, value);
        const text = (at, value) => {
            const data = new Uint8Array(256); data.set(new TextEncoder().encode(value));
            data.forEach((byte, i) => memory.set(at + BigInt(i), byte));
        };
        const klass = 0x14000n, fields = 0x50000n, root = 0x70000n, string = 0x90000n;
        const staticSlot = (mono ? 0x18000n : 0x16000n) + 0x10n;
        ptr(klass + BigInt(mono ? (wide ? 0x98 : 0x60) : (wide ? 0x80 : 0x40)), fields);
        const names = ['instance', 'left', 'right', 'text'];
        if (workBudget) names.push(...Array.from({length:14}, (_, i) => `value${i}`));
        number(klass + BigInt(mono ? (wide ? 0x100 : 0xa4) : (wide ? 0x124 : 0xac)), mono ? 4 : 2, names.length);
        names.forEach((name, i) => {
            const field = fields + BigInt(i * (wide ? 32 : mono ? 16 : 20));
            const nameAddress = 0x60000n + BigInt(256 * i);
            ptr(field + BigInt(mono ? bytes : 0), nameAddress); text(nameAddress, name);
            number(field + BigInt(wide ? 0x18 : 0xc), 4, i < 4 ? [0x10, 0x10, 0x18, 0x20][i] : 0x30 + 4 * (i - 4));
        });
        ptr(staticSlot, root);
        const headerSlots = new Set();
        const chain = (depth, shared) => {
            for (let i = 0; i < depth; i++) {
                const object = root + BigInt(i * 256), next = i + 1 < depth ? object + 256n : 0n;
                const header = writeManagedObjectHeader(fixture,{mono,ptr},object);
                headerSlots.add(object);
                if (mono) headerSlots.add(header.vtable);
                ptr(object + 0x10n, next); ptr(object + 0x18n, shared ? next : 0n); ptr(object + 0x20n, string);
                if (workBudget) for (let j = 0; j < 14; j++) number(object + 0x30n + BigInt(4 * j), 4, j);
            }
        };
        chain(1, false);
        number(string + BigInt(2 * bytes), 4, 1); number(string + BigInt(2 * bytes + 4), 2, 120);
        let reads = 0, headerReads = 0;
        const read = fixture.process.read;
        fixture.process.read = request => {
            reads++;
            if (headerSlots.has(request.address)) headerReads++;
            return read(request);
        };
        const host = await SplitScriptHost.instantiate(wasm);
        host.addProcess('game.exe', fixture.process); host.start();
        host.updateUntil(() => host.variables.get('count') === '1', `${mono}/${width}: seed`);
        host.update(2); reads = headerReads = 0;
        let count = 1, failed = false;
        if (mode === 'chain') { chain(3, false); count = 3; }
        if (mode === 'depth boundary') { chain(64, false); count = 64; }
        if (mode === 'depth overflow') { chain(65, false); failed = true; }
        if (mode === 'shared tree') { chain(10, true); count = 1023; }
        if (mode === 'object overflow') { chain(11, true); failed = true; }
        if (mode === 'work below limit') { chain(9, true); count = 511; }
        if (mode === 'work overflow') { chain(10, true); failed = true; }
        if (mode === 'self cycle') { ptr(root + 0x10n, root); failed = true; }
        if (mode === 'ancestor cycle') { chain(8, false); ptr(root + 7n * 256n + 0x10n, root + 256n); failed = true; }
        if (mode === 'unreadable optional') { memory.delete(root + 0x10n); failed = true; }
        number(string + BigInt(2 * bytes + 4), 2, 121);
        host.update();
        const label = `${mono ? 'mono' : 'il2cpp'}/${width}/${mode}`;
        assert.equal(host.variables.get('count'), String(failed ? 1 : count), label);
        assert.equal(host.variables.get('old'), '1', `${label}: old mutated`);
        assert.equal(host.variables.get('text'), failed ? 'x' : 'y', label);
        if (failed) assert.match(host.variables.get('result'), /managed/, label);
        else assert.equal(host.variables.get('result'), 'ok', label);
        if (boundedPaths && mode === 'depth overflow') {
            const result = host.variables.get('result');
            assert(Buffer.byteLength(result) <= 4096, label);
            assert(Buffer.byteLength(result) > 3900, `${label}: lost inner context`);
            assert.match(result, /depth/, label);
            assert(result.startsWith('left_context_'), `${label}: source alias missing`);
        }
        if (mode === 'work overflow') assert.match(host.variables.get('result'), /work limit/, label);
        // One state root and one explicit root, each bounded to 1,024 objects,
        // check the header before and after reading fields. Mono adds a vtable read.
        assert(headerReads <= 2 * 1024 * 2 * (mono ? 2 : 1), `${label}: unbounded header checks`);
        assert(reads - headerReads < (workBudget ? 40000 : 11000), `${label}: unbounded object traversal`);
        cases++;
    }
}
console.log(JSON.stringify({recursiveSnapshotCases:cases}));
