import {writeManagedArrayType} from './support/managed_type_fixture.mjs';
import {writeGenericType} from './support/managed_type_fixture.mjs';
import assert from 'node:assert/strict';
import { SplitScriptHost } from './support/splitscript_host.mjs';
import { createKeyedCollectionFixture } from './support/keyed_collection_fixture.mjs';
const [wasm, kind] = process.argv.slice(2);
const dictionary = kind === 'map';
const layouts = {
    V1Cattrs: {32: [0x78, 0x68], 64: [0xb0, 0x9c]},
    V2: {32: [0x60, 0xa4], 64: [0x98, 0x100]},
    V3: {32: [0x60, 0x9c], 64: [0x98, 0x100]},
    il2cpp: {32: [0x40, 0xac], 64: [0x80, 0x124]},
};
let cases = 0;
for (const family of Object.keys(layouts)) for (const width of [32, 64])
for (const parallel of [false, true]) for (const mode of ['seed', 'reorder', 'mutate', 'duplicate', 'unreadable', 'freeze', 'comparison budget', 'shared budget', 'early mismatch']) {
    const f = createKeyedCollectionFixture({family, width, parallel, dictionary});
    const inner = createKeyedCollectionFixture({family, width, parallel: !parallel, dictionary: !dictionary, base: 0x400000n});
    for (const [at, value] of inner.memory) if (at >= 0x430000n) f.memory.set(at, value);
    const {number, ptr, memory, bytes} = f;
    const mono = family !== 'il2cpp', wide = width === 64, [fields, count] = layouts[family][width];
    ptr(0x14000n + BigInt(fields), 0x58000n); number(0x14000n + BigInt(count), mono ? 4 : 2, 1);
    ptr(0x58000n + BigInt(mono ? bytes : 0), 0x59000n);
    const name = new Uint8Array(256); name.set(new TextEncoder().encode('rows'));
    name.forEach((byte, i) => memory.set(0x59000n + BigInt(i), byte));
    number(0x58000n + BigInt(wide ? 0x18 : 0xc), 4, 0x10);
    ptr((mono ? 0x18000n : 0x16000n) + 0x10n, f.object);
    writeGenericType({mono, width, ptr, number, cachedClass: dictionary ? f.keyClass : f.valueClass}, dictionary ? f.keyType : f.valueType);
    if (dictionary) number(f.valueType + BigInt(bytes + 2), 1, 0x1d);
    number(inner.valueType + BigInt(bytes + 2), 1, 0x1d);
    const vector = (at, values) => {
        writeManagedArrayType(f, {mono,width,family,ptr,number}, at, 0x0e);
        ptr(at + BigInt(2 * bytes), 0); ptr(at + BigInt(3 * bytes), values.length);
        values.forEach((item, i) => ptr(at + BigInt((4 + i) * bytes), item));
    };
    const string = (at, text) => {
        number(at + BigInt(2 * bytes), 4, text.length);
        for (let i = 0; i < text.length; i++) number(at + BigInt(2 * bytes + 4 + i * 2), 2, text.charCodeAt(i));
    };
    const collection = (layout, object, base, isMap, isParallel, entries) => {
        ptr(object, mono ? layout.vtable : layout.root);
        number(object + BigInt(layout.outer.at(-2)[1]), 4, entries.length);
        number(object + BigInt(layout.outer.at(-1)[1]), 4, isMap && !isParallel ? 0 : entries.length);
        const arrays = [base, base + 0x10000n, base + 0x20000n];
        layout.outer.slice(1, isParallel ? -2 : 2).forEach((field, i) => {
            ptr(object + BigInt(field[1]), arrays[i]); vector(arrays[i], Array(entries.length).fill(0n));
        });
        entries.forEach(([key, value], i) => {
            const at = arrays[0] + BigInt(4 * bytes + i * layout.stride);
            number(at + BigInt(layout.hash), 4, isParallel ? 0x80000001 : 1);
            number(at + BigInt(layout.next), 4, -1);
            if (isMap) ptr(isParallel ? arrays[1] + BigInt((4 + i) * bytes) : at + BigInt(layout.key), key);
            ptr(isParallel ? arrays[isMap ? 2 : 1] + BigInt((4 + i) * bytes) : at + BigInt(layout.value), value);
        });
    };
    const a = inner.object, b = inner.object + 0x2000n;
    const row = 0x900000n, tailRow = row + 0x1000n, otherRow = row + 0x2000n;
    const text = 0xa00000n, tail = text + 0x1000n, other = text + 0x2000n;
    const firstKey = text + 0x3000n, secondKey = text + 0x4000n;
    string(text, 'seed'); string(tail, 'tail'); string(other, 'other');
    string(firstKey, 'first'); string(secondKey, 'second');
    vector(row, [text, 0n]); vector(tailRow, [tail]); vector(otherRow, [other]);
    const rowsA = [[firstKey, row], [secondKey, tailRow]];
    const rowsB = [[firstKey, row], [secondKey, otherRow]];
    collection(inner, a, 0xb00000n, !dictionary, !parallel, rowsA);
    collection(inner, b, 0xb40000n, !dictionary, !parallel, rowsB);
    collection(f, f.object, 0x80000n, dictionary, parallel, dictionary ? [[a, row], [b, tailRow]] : [[0n, a], [0n, b]]);
    number(0x6f000n, 4, 0);
    const host = await SplitScriptHost.instantiate(wasm);
    const label = `${kind}/${family}/${width}/${parallel}/${mode}`;
    host.addProcess('game.exe', f.process); host.start();
    host.updateUntil(() => host.variables.has('rows'), label); host.update(2);
    const before = host.variables.get('rows');
    assert.match(before, /seed/); assert.match(before, /other/);
    assert.equal(host.variables.get('equal'), 'true', label);
    if (mode === 'reorder') collection(inner, a, 0xb00000n, !dictionary, !parallel, [...rowsA].reverse());
    if (mode === 'mutate') string(other, 'changed');
    if (mode === 'duplicate') collection(inner, b, 0xb40000n, !dictionary, !parallel, [...rowsA].reverse());
    if (mode === 'unreadable') memory.delete(other + BigInt(2 * bytes + 4));
    if (mode === 'freeze') {
        number(0x6f000n, 4, 1);
        assert.throws(() => host.update(), WebAssembly.RuntimeError, label); cases++; continue;
    }
    if (['comparison budget', 'shared budget', 'early mismatch'].includes(mode)) {
        const large = 0xc00000n, different = 0xc10000n, differentRow = 0xd00000n;
        const length = mode === 'shared budget' ? 6000 : 17000;
        string(large, 'x'.repeat(length)); vector(row, [large, 0n]);
        if (mode === 'shared budget') {
            const third = inner.object + 0x4000n, thirdRow = 0xd10000n, thirdText = 0xc20000n;
            string(thirdText, 'third'); vector(thirdRow, [thirdText]);
            collection(inner, third, 0xb80000n, !dictionary, !parallel, [[firstKey, row], [secondKey, thirdRow]]);
            collection(f, f.object, 0x80000n, dictionary, parallel,
                dictionary ? [[a, row], [b, tailRow], [third, thirdRow]] : [[0n, a], [0n, b], [0n, third]]);
        }
        if (mode === 'early mismatch') {
            string(different, 'y' + 'x'.repeat(length - 1)); vector(differentRow, [different, 0n]);
            collection(inner, b, 0xb40000n, !dictionary, !parallel, [[firstKey, differentRow], [secondKey, otherRow]]);
        }
        host.update();
        assert.equal(host.variables.get('old'), before, label);
        if (mode === 'early mismatch') {
            assert.equal(host.variables.get('result'), 'ok', label);
            assert.equal(host.variables.get('equal'), 'false', label);
            // Ordinary script equality can still compare these large snapshots.
            host.update();
            assert.equal(host.variables.get('equal'), 'true', label);
        } else {
            assert.match(host.variables.get('result'), /comparison work budget/, label);
            assert.equal(host.variables.get('rows'), before, label);
            assert.equal(host.variables.get('equal'), 'true', label);
        }
        // Exhaustion belongs to this root only. Repair and retry on the same attachment.
        vector(row, [text, 0n]);
        collection(inner, b, 0xb40000n, !dictionary, !parallel, rowsB);
        collection(f, f.object, 0x80000n, dictionary, parallel, dictionary ? [[a, row], [b, tailRow]] : [[0n, a], [0n, b]]);
        host.update(2);
        assert.equal(host.variables.get('result'), 'ok', label);
        assert.equal(host.variables.get('rows'), before, label);
        assert.equal(host.variables.get('equal'), 'true', label);
        cases++; continue;
    }
    host.update();
    assert.equal(host.variables.get('old'), before, label);
    if (mode !== 'reorder') assert.equal(host.variables.get('rows'), mode === 'mutate' ? before.replace('other', 'changed') : before, label);
    assert.equal(host.variables.get('equal'), String(mode !== 'mutate'), label);
    assert.equal(host.variables.get('result') === 'ok', ['seed', 'reorder', 'mutate'].includes(mode), label);
    if (mode === 'duplicate') assert.match(host.variables.get('result'), /duplicate/, label);
    cases++;
}
console.log(JSON.stringify({managedNestedKeyedCases: cases, kind}));
