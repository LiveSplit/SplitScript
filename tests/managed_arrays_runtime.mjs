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
    for (const mode of ['cached metadata', 'wrong element kind', 'wrong empty nested kind', 'wrong inline width', 'wrong depth', 'non-array class', 'unreadable class', 'type cycle', 'type depth overflow', 'compatible class replacement', 'torn class', 'seed', 'mutate', 'replace', 'empty', 'null array', 'null row', 'null element', 'null leaf', 'bounds', 'unreadable header', 'unreadable slot', 'unreadable child', 'failed sibling', 'cycle', 'count overflow', 'address overflow', 'shared element budget', 'inline element budget']) {
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
        fields(klass, 0x50000n, [['instance', 0x10], ['strings', 0x20], ['nested', 0x10], ['leaves', 0x18], ['numbers', 0x20], ['pointers', 0x28], ['pairs', 0x30]]);
        fields(leafClass, 0x51000n, [['text', 0x10]]);
        const statics = mono ? 0x18000n : 0x16000n;
        const root = 0x70000n, leaf = 0x71000n;
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
            const element = at === nested ? [0x0e] : at === leaves ? 0x12 : at === numbers ? 0x08 : at === pointers ? 0x19 : at === pairs || at === 0x300000n || at === 0xa0000n ? {kind:0x11,bytes:8} : 0x0e;
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
        const originalArray = writeManagedArrayType(fixture, {mono,width,ptr,number}, nested, [0x0e]);
        const replacementClass = originalArray.klass + 0x10000000n;
        const replacementType = replacementClass + originalArray.type - originalArray.klass;
        const replacementVtable = replacementClass + 0x800n;
        for (let i = 0n; i < 0x200n; i++) memory.set(replacementClass + i, memory.get(originalArray.klass + i) ?? 0);
        ptr(replacementVtable, replacementClass);
        const replacementHeader = mono ? replacementVtable : replacementClass;
        let armedClassChange = false, replacementActive = false;
        const reads = [], read = fixture.process.read;
        fixture.process.read = request => {
            const address = BigInt.asUintN(64, request.address);
            reads.push({ ...request, address });
            if (armedClassChange && address === nested + BigInt(4 * bytes)) {
                replacementActive = !replacementActive;
                ptr(nested, replacementActive ? replacementHeader : mono ? originalArray.vtable : originalArray.klass);
            }
            return read({ ...request, address });
        };
        const host = await SplitScriptHost.instantiate(wasm);
        host.addProcess('game.exe', fixture.process); host.start();
        host.updateUntil(() => host.variables.has('nested'), `${mono}/${width}: seed`);
        host.update(2); reads.length = 0;
        const normalize = value => value.replace(/\s/g, '');
        const seedNested = '[Some([Some("seed",),None,Some("tail",),],),None,Some([],),]';
        let expected = seedNested;
        if (mode === 'wrong element kind') writeManagedArrayType(fixture, {mono,width,ptr,number}, inner, 0x08);
        if (mode === 'wrong empty nested kind') {
            vector(nested, []); writeManagedArrayType(fixture, {mono,width,ptr,number}, nested, [0x08]);
        }
        if (mode === 'wrong inline width') writeManagedArrayType(fixture, {mono,width,ptr,number}, pairs, {kind:0x11,bytes:4});
        if (mode === 'wrong depth') writeManagedArrayType(fixture, {mono,width,ptr,number}, nested, 0x0e);
        if (mode === 'non-array class') {
            const scalar = originalArray.element.element;
            ptr(nested, mono ? scalar.vtable : scalar.klass);
        }
        if (mode === 'unreadable class') ptr(nested, 0x2e000000n);
        if (mode === 'type cycle') {
            ptr(replacementType, mono ? replacementClass : replacementType); ptr(nested, replacementHeader);
        }
        if (mode === 'type depth overflow') {
            let shape = 0x0e; for (let i = 0; i < 64; i++) shape = [shape];
            writeManagedArrayType(fixture, {mono,width,ptr,number}, nested, shape);
        }
        if (mode === 'compatible class replacement') ptr(nested, replacementHeader);
        if (mode === 'torn class') armedClassChange = true;
        if (mode === 'mutate') { text(string, 'new'); expected = seedNested.replace('seed', 'new'); }
        if (mode === 'replace') { vector(inner, [tail]); expected = '[Some([Some("tail",),],),None,Some([],),]'; }
        if (mode === 'empty') { vector(nested, []); expected = '[]'; }
        if (mode === 'null array') ptr(root + 0x10n, 0);
        if (mode === 'null row') { ptr(nested + BigInt(4 * bytes), 0); expected = '[None,None,Some([],),]'; }
        if (mode === 'null element') { ptr(inner + BigInt(4 * bytes), 0); expected = '[Some([None,None,Some("tail",),],),None,Some([],),]'; }
        if (mode === 'null leaf') ptr(leaves + BigInt(4 * bytes), 0);
        if (mode === 'bounds') ptr(nested + BigInt(2 * bytes), 0xdead);
        if (mode === 'unreadable header') memory.delete(nested + BigInt(2 * bytes));
        if (mode === 'unreadable slot') memory.delete(nested + BigInt(4 * bytes));
        if (mode === 'unreadable child') memory.delete(string + BigInt(2 * bytes + 4));
        if (mode === 'failed sibling') { text(string, 'new'); memory.delete(root + 0x18n); }
        if (mode === 'cycle') ptr(nested + BigInt(4 * bytes), nested);
        if (mode === 'count overflow') ptr(nested + BigInt(3 * bytes), wide ? 0x100000000n : 0x80000000n);
        const limit = wide ? 0xffffffffffffffffn : 0xffffffffn;
        if (mode === 'address overflow') {
            const at = limit - BigInt(4 * bytes) + 1n;
            ptr(root + 0x10n, at); ptr(at + BigInt(2 * bytes), 0); ptr(at + BigInt(3 * bytes), 1);
        }
        if (mode === 'shared element budget') {
            vector(nested, [0x100000n, 0x200000n]);
            vector(0x100000n, Array(8192).fill(0)); vector(0x200000n, Array(8192).fill(0));
        }
        if (mode === 'inline element budget') {
            ptr(root + 0x30n, 0x300000n);
            vector(0x300000n, Array(8192).fill(0x0000000200000001n), 8);
        }
        host.update();
        const label = `${mono ? 'mono' : 'il2cpp'}/${width}/${mode}`;
        assert.equal(normalize(host.variables.get('nested')), expected, label);
        assert.equal(normalize(host.variables.get('old')), seedNested, `${label}: old changed`);
        assert.equal(normalize(host.variables.get('numbers')), '[1,2,3,]', label);
        assert.equal(normalize(host.variables.get('pairs')), '[[1,2,],[3,4,],]', label);
        assert.equal(normalize(host.variables.get('pointers')), `[0,${limit},]`, label);
        assert.equal(normalize(host.variables.get('leaves')), mode === 'null leaf' ? '[None,None,]' : '[Some(Leaf{text:"tail",},),None,]', label);
        const successful = ['cached metadata', 'compatible class replacement', 'seed', 'mutate', 'replace', 'empty', 'null row', 'null element', 'null leaf'].includes(mode);
        assert.equal(host.variables.get('result') === 'ok', successful, `${label}: result`);
        if (mode.startsWith('wrong ')) assert.match(host.variables.get('result'), /incompatible/, label);
        if (mode === 'non-array class') assert.match(host.variables.get('result'), /not a vector/, label);
        if (mode === 'type cycle') assert.match(host.variables.get('result'), /cycle/, label);
        if (mode === 'type depth overflow') assert.match(host.variables.get('result'), /depth limit/, label);
        if (mode === 'torn class') assert.match(host.variables.get('result'), /class changed/, label);
        if (mode.startsWith('wrong ')) {
            const rejected = mode === 'wrong inline width' ? pairs : mode === 'wrong element kind' ? inner : nested;
            assert(!reads.some(({address}) => address === rejected + BigInt(4 * bytes)), `${label}: rejected type read payload`);
        }
        if (mode === 'cycle') assert.match(host.variables.get('result'), /cycle/, label);
        if (mode.includes('budget')) assert.match(host.variables.get('result'), /budget/, label);
        for (const { address, length } of reads) assert(address + BigInt(Math.max(length, 1) - 1) <= limit, `${label}: overflowing read`);
        if (mode === 'count overflow') assert(!reads.some(read => read.address === nested + BigInt(4 * bytes)), `${label}: invalid count read payload`);
        if (mode === 'shared element budget') assert(!reads.some(read => read.address === 0x200000n + BigInt(4 * bytes)), `${label}: child replenished budget`);
        assert(reads.length < (mode.includes('budget') ? 17000 : mode === 'type depth overflow' ? 700 : 200), `${label}: excessive host reads`);
        if (mode === 'inline element budget') assert(!reads.some(read => read.address === 0x300000n + BigInt(4 * bytes + 8191 * 8)), `${label}: inline children replenished budget`);
        if (mode.startsWith('wrong ') || ['non-array class', 'unreadable class', 'type cycle', 'type depth overflow', 'torn class'].includes(mode)) {
            armedClassChange = false;
            writeManagedArrayType(fixture, {mono,width,ptr,number}, nested, [0x0e]);
            writeManagedArrayType(fixture, {mono,width,ptr,number}, inner, 0x0e);
            writeManagedArrayType(fixture, {mono,width,ptr,number}, pairs, {kind:0x11,bytes:8});
            ptr(nested + BigInt(3 * bytes), 3);
            host.updateUntil(() => host.variables.get('result') === 'ok', `${label}: metadata repair`);
            assert.equal(normalize(host.variables.get('nested')), seedNested, label);
        }
        if (mode === 'cached metadata') {
            assert(!reads.some(({address}) => address >= 0x30000000n && address < 0x30100000n && (!mono || address % 0x1000n !== 0x800n)), `${label}: cached layout reread type metadata`);
            host.setProcessOpen('game.exe', false); host.update(2);
            host.addProcess('game.exe', fixture.process); host.variables.delete('result'); reads.length = 0;
            host.updateUntil(() => host.variables.get('result') === 'ok', `${label}: reattach`);
            assert(reads.some(({address}) => address === originalArray.type + BigInt(bytes + 2)), `${label}: reused previous attachment metadata`);
        }
        cases++;
    }
}
console.log(JSON.stringify({managedArrayCases:cases}));
