import {writeVectorElementType} from './support/managed_type_fixture.mjs';
import assert from 'node:assert/strict';
import { SplitScriptHost } from './support/splitscript_host.mjs';
import { createKeyedCollectionFixture } from './support/keyed_collection_fixture.mjs';
const [wasm, kind] = process.argv.slice(2);
const dictionary = kind === 'map';
const layouts = {
    V1Cattrs: {32: [0x34, 0x38, 0x78, 0x68, null, null], 64: [0x50, 0x58, 0xb0, 0x9c, null, null]},
    V2: {32: [0x2c, 0x30, 0x60, 0xa4, 0x1e, 0x94], 64: [0x48, 0x50, 0x98, 0x100, 0x2a, 0xf0]},
    V3: {32: [0x2c, 0x30, 0x60, 0x9c, 0xf, 0x8c], 64: [0x48, 0x50, 0x98, 0x100, 0x1b, 0xf0]},
    il2cpp: {32: [8, 0xc, 0x40, 0xac], 64: [0x10, 0x18, 0x80, 0x124]},
};
let cases = 0;
for (const family of Object.keys(layouts)) for (const width of [32, 64])
for (const parallel of [false, true]) for (const mode of ['seed', 'mutate', 'duplicate', 'unreadable', 'null child', 'spare capacity', 'count overflow', 'freeze']) {
    const f = createKeyedCollectionFixture({family, width, parallel, dictionary});
    const {number, ptr, memory, object, bytes, outer, stride, hash, next, key, value} = f;
    const mono = family !== 'il2cpp', wide = width === 64;
    const [nameOffset, namespaceOffset, fieldsOffset, countOffset, kindOffset, genericOffset] = layouts[family][width];
    const name = (at, text) => {
        const data = new Uint8Array(256); data.set(new TextEncoder().encode(text));
        data.forEach((byte, i) => memory.set(at + BigInt(i), byte));
    };
    let nameIndex = 0;
    const fields = (owner, table, entries) => {
        ptr(owner + BigInt(fieldsOffset), table); number(owner + BigInt(countOffset), mono ? 4 : 2, entries.length);
        entries.forEach(([text, offset], i) => {
            const field = table + BigInt(i * (wide ? 32 : mono ? 16 : 20)), textAt = 0x600000n + BigInt(nameIndex++ * 256);
            ptr(field + BigInt(mono ? bytes : 0), textAt); name(textAt, text);
            number(field + BigInt(wide ? 0x18 : 0xc), 4, offset);
            if (text === '_items' || text === '_size') {
                const type = owner + 0x230000n + (text === '_items' ? 0n : 0x100n);
                ptr(field + BigInt(mono ? 0 : bytes), type);
                number(type + BigInt(bytes + 2), 1, text === '_items' ? 0x1d : 0x08);
                if (text === '_items') writeVectorElementType({mono, width, family, ptr, number}, type, owner + 0x240000n, owner === 0x310000n ? 0x0e : 0x15);
            }
        });
    };
    fields(0x14000n, 0x58000n, [['rows', 0x10]]);
    ptr((mono ? 0x18000n : 0x16000n) + 0x10n, object);
    number((dictionary ? f.keyType : f.valueType) + BigInt(bytes + 2), 1, 0x15);
    if (dictionary) number(f.valueType + BigInt(bytes + 2), 1, 0x15);
    // Separate closed runtime classes for List<String?> and List<List<String?>>.
    for (let i = 0; i < 2; i++) {
        const shift = BigInt(i * 0x1000), klass = 0x310000n + shift, definition = 0x320000n + shift;
        for (const at of [klass, definition]) for (let j = 0n; j < 0x200n; j++) memory.set(at + j, 0);
        ptr(klass + BigInt(nameOffset), 0x400000n + shift); name(0x400000n + shift, 'List`1');
        ptr(klass + BigInt(namespaceOffset), 0x400100n + shift); name(0x400100n + shift, 'System.Collections.Generic');
        fields(klass, 0x510000n + shift, [['_items', 2 * bytes], ['_size', 3 * bytes]]);
        if (mono && kindOffset !== null) {
            number(klass + BigInt(kindOffset), 1, 3);
            ptr(klass + BigInt(genericOffset), 0x350000n + shift); ptr(0x350000n + shift, definition);
            number(definition + BigInt(countOffset), 4, 2); number(klass + BigInt(countOffset), 4, 0x7fffffff);
        }
        ptr(0x390000n + shift, klass);
    }
    const vector = (at, values, capacity = values.length) => {
        ptr(at + BigInt(2 * bytes), 0); ptr(at + BigInt(3 * bytes), capacity);
        values.forEach((item, i) => ptr(at + BigInt((4 + i) * bytes), item));
    };
    const list = (at, backing, values, nested = false) => {
        ptr(at, (mono ? 0x390000n : 0x310000n) + (nested ? 0x1000n : 0n));
        ptr(at + BigInt(2 * bytes), backing); number(at + BigInt(3 * bytes), 4, values.length);
        vector(backing, values);
    };
    const string = (at, text) => {
        number(at + BigInt(2 * bytes), 4, text.length);
        for (let i = 0; i < text.length; i++) number(at + BigInt(2 * bytes + 4 + 2 * i), 2, text.charCodeAt(i));
    };
    const a = 0x700000n, b = 0x701000n;
    const first = 0x710000n, tail = 0x711000n, second = 0x712000n, other = 0x713000n;
    const seedText = 0x900000n, tailText = 0x901000n, otherText = 0x902000n;
    string(seedText, 'seed'); string(tailText, 'tail'); string(otherText, 'other');
    list(first, 0x810000n, [seedText, 0n]); list(tail, 0x811000n, [tailText]);
    list(second, 0x812000n, [seedText, 0n]); list(other, 0x813000n, [otherText]);
    list(a, 0x800000n, [first, tail], true); list(b, 0x801000n, [second, other], true);
    number(object + BigInt(outer.at(-2)[1]), 4, 2);
    number(object + BigInt(outer.at(-1)[1]), 4, dictionary && !parallel ? 0 : 2);
    const arrays = [0x80000n, 0x120000n, 0x180000n];
    outer.slice(1, parallel ? -2 : 2).forEach((field, i) => { ptr(object + BigInt(field[1]), arrays[i]); vector(arrays[i], [0n, 0n]); });
    for (let i = 0; i < 2; i++) {
        const at = arrays[0] + BigInt(4 * bytes + i * stride), nested = i === 0 ? a : b;
        number(at + BigInt(hash), 4, parallel ? 0x80000001 : 1); number(at + BigInt(next), 4, -1);
        if (dictionary) ptr(parallel ? arrays[1] + BigInt((4 + i) * bytes) : at + BigInt(key), nested);
        ptr(parallel ? arrays[dictionary ? 2 : 1] + BigInt((4 + i) * bytes) : at + BigInt(value), dictionary ? (i === 0 ? tail : first) : nested);
    }
    number(0x6f000n, 4, 0);
    const host = await SplitScriptHost.instantiate(wasm), label = `${kind}/${family}/${width}/${parallel}/${mode}`;
    host.addProcess('game.exe', f.process); host.start();
    host.updateUntil(() => host.variables.has('rows'), label); host.update(2);
    const before = host.variables.get('rows');
    assert.match(before, /seed/); assert.match(before, /other/);
    assert.equal(host.variables.get('contains'), 'true', label);
    assert.equal(host.variables.get('equal'), 'true', label);
    if (mode === 'mutate') string(otherText, 'changed');
    if (mode === 'duplicate') ptr(0x813000n + BigInt(4 * bytes), tailText);
    if (mode === 'unreadable') memory.delete(otherText + BigInt(2 * bytes + 4));
    if (mode === 'null child') ptr(0x801000n + BigInt(4 * bytes), 0);
    if (mode === 'spare capacity') vector(0x801000n, [second, other], 100);
    if (mode === 'count overflow') number(b + BigInt(3 * bytes), 4, 3);
    if (mode === 'freeze') {
        number(0x6f000n, 4, 1);
        assert.throws(() => host.update(), WebAssembly.RuntimeError, label); cases++; continue;
    }
    host.update();
    assert.equal(host.variables.get('old'), before, label);
    assert.equal(host.variables.get('rows'), mode === 'mutate' ? before.replace('other', 'changed') : before, label);
    assert.equal(host.variables.get('equal'), String(mode !== 'mutate'), label);
    assert.equal(host.variables.get('contains'), 'true', label);
    assert.equal(host.variables.get('result') === 'ok', ['seed', 'mutate', 'spare capacity'].includes(mode), label);
    if (mode === 'duplicate') assert.match(host.variables.get('result'), /duplicate/, label);
    cases++;
}
console.log(JSON.stringify({managedKeyedListCases: cases, kind}));
