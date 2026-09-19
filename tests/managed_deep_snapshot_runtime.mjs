import assert from 'node:assert/strict';
import { readFile } from 'node:fs/promises';
import { SplitScriptHost } from './support/splitscript_host.mjs';
import { createMonoPeFixture } from './support/mono_pe_fixture.mjs';
import { createIl2cppPeFixture } from './support/il2cpp_pe_fixture.mjs';

const [wasm, optionalLeft] = process.argv.slice(2);
const profiles = JSON.parse(await readFile(new URL('./fixtures/mono-pe-profiles.json', import.meta.url)));
let cases = 0;
for (const mono of [true, false]) for (const width of [32, 64]) {
    for (const mode of ['shared', 'mutate', 'replace', 'null child', 'unreadable child', 'unreadable string', 'cycle', 'failed sibling']) {
        const wide = width === 64, bytes = width / 8;
        const fixture = mono ? createMonoPeFixture(profiles.builds.find(p => p.width === width && p.version === 'V2'))
            : createIl2cppPeFixture({width, version:[2022, 3, 0, 37029]});
        const memory = fixture.memory;
        const write = (at, bytes) => bytes.forEach((byte, i) => memory.set(at + BigInt(i), byte));
        const number = (at, size, value) => {
            const data = new Uint8Array(size), view = new DataView(data.buffer);
            if (size === 8) view.setBigUint64(0, BigInt(value), true);
            else if (size === 2) view.setUint16(0, Number(value), true);
            else view.setUint32(0, Number(value), true);
            write(at, data);
        };
        const ptr = (at, value) => number(at, bytes, value);
        const name = (at, value) => { const data = new Uint8Array(256); data.set(new TextEncoder().encode(value)); write(at, data); };
        const klass = 0x14000n, leafClass = 0x19000n;
        for (let offset = 0n; offset < 0x200n; offset++) memory.set(leafClass + offset, memory.get(klass + offset) ?? 0);
        const className = mono ? (wide ? 0x48 : 0x2c) : (wide ? 0x10 : 8);
        ptr(leafClass + BigInt(className), 0x25000n); name(0x25000n, 'Leaf');
        if (mono) {
            ptr(klass + BigInt(wide ? 0x108 : 0xa8), leafClass);
            ptr(leafClass + BigInt(wide ? 0x108 : 0xa8), 0);
        } else ptr(0x13000n + BigInt(9 * bytes), leafClass);
        const fieldsOffset = mono ? (wide ? 0x98 : 0x60) : (wide ? 0x80 : 0x40);
        const countOffset = mono ? (wide ? 0x100 : 0xa4) : (wide ? 0x124 : 0xac);
        let nameIndex = 0;
        const fields = (owner, at, entries) => {
            ptr(owner + BigInt(fieldsOffset), at);
            number(owner + BigInt(countOffset), mono ? 4 : 2, entries.length);
            entries.forEach(([text, offset], index) => {
                const field = at + BigInt(index * (wide ? 32 : mono ? 16 : 20));
                const textAddress = 0x60000n + BigInt(nameIndex++ * 256);
                ptr(field + BigInt(mono ? bytes : 0), textAddress); name(textAddress, text);
                number(field + BigInt(wide ? 0x18 : 0xc), 4, offset);
            });
        };
        fields(klass, 0x50000n, [['instance', 0x10], ['left', 0x10], ['right', 0x18]]);
        fields(leafClass, 0x51000n, [['text', 0x10]]);
        const staticSlot = (mono ? 0x18000n : 0x16000n) + 0x10n;
        const root = 0x70000n, leaf = 0x71000n, otherLeaf = 0x72000n, string = 0x80000n, otherString = 0x82000n;
        ptr(staticSlot, root); ptr(root + 0x10n, leaf); ptr(root + 0x18n, leaf);
        ptr(leaf + 0x10n, string); ptr(otherLeaf + 0x10n, otherString);
        const text = (at, value) => {
            number(at + BigInt(bytes * 2), 4, value.length);
            for (let i = 0; i < value.length; i++) number(at + BigInt(bytes * 2 + 4 + i * 2), 2, value.charCodeAt(i));
        };
        text(string, 'seed'); text(otherString, 'two');
        const reads = [], read = fixture.process.read;
        fixture.process.read = request => { reads.push(request.address); return read(request); };
        const host = await SplitScriptHost.instantiate(wasm);
        host.addProcess('game.exe', fixture.process); host.start();
        host.updateUntil(() => host.variables.get('left') === 'seed', `${mono}/${width}: seed`);
        host.update(2); reads.length = 0;
        let left = 'seed', right = 'seed';
        if (mode === 'mutate') { text(string, 'new'); left = right = 'new'; }
        if (mode === 'replace') { ptr(root + 0x18n, otherLeaf); right = 'two'; }
        if (mode === 'null child') { ptr(root + 0x10n, 0); if (optionalLeft) left = 'none'; }
        if (mode === 'unreadable child') memory.delete(root + 0x10n);
        if (mode === 'unreadable string') memory.delete(string + BigInt(bytes * 2 + 4));
        if (mode === 'cycle') ptr(root + 0x10n, root);
        if (mode === 'failed sibling') { text(string, 'new'); memory.delete(root + 0x18n); }
        host.update();
        const label = `${mono ? 'mono' : 'il2cpp'}/${width}/${mode}`;
        assert.equal(host.variables.get('left'), left, label);
        assert.equal(host.variables.get('right'), right, label);
        assert.equal(host.variables.get('old'), 'seed', `${label}: previous nested value changed`);
        const result = host.variables.get('result');
        if (['shared', 'mutate', 'replace'].includes(mode) || (optionalLeft && mode === 'null child')) assert.equal(result, 'ok', label);
        else assert.match(result, /managed/, `${label}: failure message missing`);
        if (mode === 'cycle') assert.match(result, /cycle/, label);
        assert(reads.length < 100, `${label}: unbounded traversal`);
        assert(reads.every(address => address === staticSlot || address >= root), `${label}: repeated metadata discovery`);
        cases++;
    }
}
console.log(JSON.stringify({deepSnapshotCases:cases}));
