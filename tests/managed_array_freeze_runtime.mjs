import {writeManagedObjectHeader} from './support/managed_type_fixture.mjs';
import {writeManagedArrayType} from './support/managed_type_fixture.mjs';
import assert from 'node:assert/strict';
import { readFile } from 'node:fs/promises';
import { SplitScriptHost } from './support/splitscript_host.mjs';
import { createMonoPeFixture } from './support/mono_pe_fixture.mjs';
import { createIl2cppPeFixture } from './support/il2cpp_pe_fixture.mjs';
const [wasm] = process.argv.slice(2);
const profiles = JSON.parse(await readFile(new URL('./fixtures/mono-pe-profiles.json', import.meta.url)));
let cases = 0;
for (const mono of [true, false]) for (const width of [32, 64]) {
    for (const mode of Array.from({length:25}, (_, i) => i+1)) {
        const wide = width === 64, bytes = width / 8;
        const fixture = mono ? createMonoPeFixture(profiles.builds.find(p => p.width === width && p.version === 'V2'))
            : createIl2cppPeFixture({width, version:[2022, 3, 0, 37029]});
        const memory = fixture.memory;
        const write = (at, bytes) => bytes.forEach((byte, i) => memory.set(at + BigInt(i), byte));
        const number = (at, size, value) => {
            const data = new Uint8Array(size), view = new DataView(data.buffer);
            if (size === 8) view.setBigUint64(0, BigInt(value), true);
            else if (size === 2) view.setUint16(0, Number(value), true);
            else if (size === 1) view.setUint8(0, Number(value));
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
        fields(klass, 0x50000n, [['instance', 0x10], ['strings', 0x20], ['nested', 0x10], ['leaves', 0x18], ['numbers', 0x20], ['pointers', 0x28], ['pairs', 0x30], ['operation', 0x40], ['staticHeader', 0x60], ['fixed', 0x40], ['header', 0x50], ['envelope', 0x80], ['fixedRoot', 0x90], ['matrix', 0xa0], ['headers', 0x70]]);
        fields(leafClass, 0x51000n, [['text', 0x10]]);
        const statics = mono ? 0x18000n : 0x16000n;
        const root = 0x70000n, leaf = 0x71000n;
        writeManagedObjectHeader(fixture,{mono,ptr},root,klass); writeManagedObjectHeader(fixture,{mono,ptr},leaf,leafClass);
        const strings = 0x80000n, nested = 0x82000n, inner = 0x84000n, empty = 0x86000n;
        const leaves = 0x88000n, numbers = 0x8a000n, pointers = 0x8c000n, pairs = 0x8e000n;
        const string = 0x90000n, tail = 0x92000n;
        ptr(statics + 0x10n, root); ptr(statics + 0x20n, strings);
        [nested, leaves, numbers, pointers, pairs].forEach((at, i) => ptr(root + 0x10n + BigInt(i * 8), at));
        ptr(leaf + 0x10n, tail);
        const text = (at, value) => {
            number(at + BigInt(bytes * 2), 4, value.length);
            for (let i = 0; i < value.length; i++) number(at + BigInt(bytes * 2 + 4 + i * 2), 2, value.charCodeAt(i));
        };
        const vector = (at, values, stride = bytes) => {
            const element = at === nested ? [0x0e] : at === leaves ? {kind:0x12,class:0x19000} : at === numbers ? 0x08 : at === pointers ? 0x19 : at === pairs || at === 0x300000n || at === 0xa0000n ? {kind:0x11,bytes:8} : 0x0e;
            writeManagedArrayType(fixture, {mono,width,ptr,number}, at, element);
            ptr(at + BigInt(2 * bytes), 0);
            ptr(at + BigInt(3 * bytes), values.length);
            values.forEach((value, i) => number(at + BigInt(4 * bytes + i * stride), stride, value));
        };
        text(string, 'seed'); text(tail, 'tail');
        vector(strings, [string, tail]); vector(nested, [inner, 0n, empty]);
        vector(inner, [string, 0n, tail]); vector(empty, []); vector(leaves, [leaf, 0n]);
        vector(numbers, [1, 2, 3], 4); vector(pointers, [0n, wide ? 0xffffffffffffffffn : 0xffffffffn]);
        vector(pairs, [0x0000000200000001n, 0x0000000400000003n], 8);
        number(statics + 0x40n, 4, 0);
        number(statics + 0x60n, 8, 0x0000000200000001n);
        number(statics + 0x80n, 8, 0x0000000200000001n);
        number(statics + 0x88n, 4, 0);
        number(statics + 0x90n, 8, 0x0000000200000001n);
        number(statics + 0xa0n, 8, 0x0000000200000001n);
        number(statics + 0xa8n, 8, 0x0000000400000003n);
        number(root + 0x40n, 8, 0x0000000200000001n);
        number(root + 0x50n, 8, 0x0000000200000001n);
        ptr(root + 0x70n, 0xa0000n);
        vector(0xa0000n, [0x0000000200000001n], 8);
        const host = await SplitScriptHost.instantiate(wasm);
        host.addProcess('game.exe', fixture.process); host.start();
        host.updateUntil(() => host.variables.has('ready'), `${mono}/${width}: seed`);
        host.update(2);
        const original = host.variables.get('snapshot');
        const envelope = host.variables.get('envelope');
        const fixedRoot = host.variables.get('fixedRoot');
        if (mode === 24) memory.delete(statics + 0x90n);
        if (mode === 25) number(statics + 0x88n, 4, 99);
        number(statics + 0x40n, 4, mode);
        const label = `${mono}/${width}/${mode}`;
        if (![20, 24, 25].includes(mode)) {
            assert.throws(() => host.update(), WebAssembly.RuntimeError, label);
            assert.equal(host.variables.get('completed'), '0', `${label}: mutation completed`);
            number(statics + 0x40n, 4, 0);
            host.update();
            assert.equal(host.variables.get('oldSnapshot'), original, `${label}: accepted graph was mutated`);
        } else if (mode === 20) {
            host.update();
            const norm = key => host.variables.get(key).replace(/\s/g, '');
            assert.equal(norm('local'), '[4,5,6,]', label);
            assert.equal(norm('native'), '[42,2,]', label);
            assert.equal(norm('empty'), '0', label);
            assert.equal(norm('calls'), '3', label);
            assert.equal(norm('finalLocal'), '[9,]', label);
        }
        if (mode === 24 || mode === 25) {
            host.update();
            assert.equal(host.variables.get('completed'), String(mode), label);
            assert.equal(host.variables.get('envelope'), envelope, `${label}: failed inline read lost accepted value`);
            assert.equal(host.variables.get('fixedRoot'), fixedRoot, `${label}: failed root lost accepted value`);
        }
        assert.equal(host.variables.get('old').replace(/\s/g, ''), '[1,2,3,]', label);
        cases++;
    }
}
console.log(JSON.stringify({managedArrayFreezeCases:cases}));
