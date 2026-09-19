import assert from 'node:assert/strict';
import { readFile } from 'node:fs/promises';
import { SplitScriptHost } from './support/splitscript_host.mjs';
import { createMonoPeFixture } from './support/mono_pe_fixture.mjs';
import { createIl2cppPeFixture } from './support/il2cpp_pe_fixture.mjs';
const [wasm] = process.argv.slice(2);
const profiles = JSON.parse(await readFile(new URL('./fixtures/mono-pe-profiles.json', import.meta.url)));
let cases = 0;
for (const mono of [true, false]) for (const width of [32, 64]) {
    for (const mode of ['shared children', 'mutate', 'empty', 'self cycle', 'array cycle', 'unreadable child', 'failed sibling', 'depth boundary', 'depth overflow', 'object boundary', 'object overflow']) {
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
        const klass = 0x14000n;
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
        fields(klass, 0x50000n, [['instance', 0x10], ['children', 0x10], ['value', 0x18]]);
        const root = 0x70000n, child = 0x71000n, rootArray = 0x80000n, childArray = 0x88000n;
        const vector = (at, values) => {
            ptr(at + BigInt(2 * bytes), 0); ptr(at + BigInt(3 * bytes), values.length);
            values.forEach((value, index) => ptr(at + BigInt((4 + index) * bytes), value));
        };
        const node = (at, array, value) => { ptr(at + 0x10n, array); number(at + 0x18n, 4, value); };
        ptr((mono ? 0x18000n : 0x16000n) + 0x10n, root);
        node(root, rootArray, 1); node(child, childArray, 2);
        vector(rootArray, [child, 0n, child]); vector(childArray, []);
        const host = await SplitScriptHost.instantiate(wasm);
        host.addProcess('game.exe', fixture.process); host.start();
        host.updateUntil(() => host.variables.has('tree'), `${mono}/${width}: seed`); host.update(2);
        const normalize = text => text.replace(/\s/g, '');
        const value = (number, children = []) => `ProfileProbe{children:[${children.map(child => child === null ? 'None,' : `Some(${child},),`).join('')}],value:${number},}`;
        const seed = value(1, [value(2), null, value(2)]);
        assert.equal(normalize(host.variables.get('tree')), seed);
        let expected = seed, ok = false, count = 3;
        if (mode === 'shared children') ok = true;
        if (mode === 'mutate') { node(child, childArray, 3); expected = value(1, [value(3), null, value(3)]); ok = true; }
        if (mode === 'empty') { vector(rootArray, []); expected = value(1); ok = true; count = 1; }
        if (mode === 'self cycle') vector(rootArray, [root]);
        if (mode === 'array cycle') vector(rootArray, [rootArray]);
        if (mode === 'unreadable child') memoryDelete(child + 0x18n);
        if (mode === 'failed sibling') { node(child, childArray, 9); memoryDelete(rootArray + BigInt(6 * bytes)); }
        if (mode.startsWith('depth')) {
            const count = mode === 'depth boundary' ? 31 : 32;
            vector(rootArray, [0x100000n]);
            let nested = value(2);
            for (let i = 0; i < count; i++) {
                const at = 0x100000n + BigInt(i * 0x1000), array = at + 0x400n;
                node(at, array, 2); vector(array, i + 1 < count ? [at + 0x1000n] : []);
                if (i) nested = value(2, [nested]);
            }
            if (mode === 'depth boundary') { expected = value(1, [nested]); ok = true; }
        }
        if (mode.startsWith('object')) {
            const count = mode === 'object boundary' ? 511 : 512;
            vector(rootArray, Array(count).fill(child));
            if (mode === 'object boundary') { expected = value(1, Array(count).fill(value(2))); ok = true; }
        }
        host.update();
        const label = `${mono}/${width}/${mode}`;
        if (mode !== 'depth boundary') assert.equal(normalize(host.variables.get('tree')), expected, label);
        assert.equal(host.variables.get('count'), String(mode === 'depth boundary' ? 32 : mode === 'object boundary' ? 512 : count), `${label}: complete owned traversal`);
        assert.equal(normalize(host.variables.get('old')), seed, `${label}: old changed`);
        assert.equal(host.variables.get('result') === 'ok', ok, `${label}: result`);
        assert.equal(host.variables.get('equal'), String(!['mutate', 'empty', 'depth boundary', 'object boundary'].includes(mode)), `${label}: structural snapshot equality`);
        if (mode.includes('cycle')) assert.match(host.variables.get('result'), /cycle/, label);
        if (mode.includes('overflow')) assert.match(host.variables.get('result'), /object\/depth limit/, label);
        cases++;
        function memoryDelete(at) { fixture.memory.delete(at); }
    }
}
console.log(JSON.stringify({managedArrayTreeCases:cases}));
