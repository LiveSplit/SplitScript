import assert from 'node:assert/strict';
import { SplitScriptHost } from './support/splitscript_host.mjs';
import { createKeyedCollectionFixture } from './support/keyed_collection_fixture.mjs';

const [wasm] = process.argv.slice(2);
const modes = ['wrong value schema', 'seed', 'mutate', 'empty', 'shared empty backing', 'all deleted', 'large capacity', 'duplicate values',
    'unreadable value', 'unreadable string', 'null set', 'scan budget', 'shared scan budget',
    'shared element budget', 'comparison budget', 'byte budget', 'backing cycle', 'torn count', 'torn backing', 'cached layout',
    'retry', 'freeze clear', 'freeze insert', 'freeze remove', 'freeze absent', 'freeze child',
    'freeze snapshot', 'local mutable'];
const fieldLayouts = {
    V1Cattrs: {32: [0x78, 0x68], 64: [0xb0, 0x9c]},
    V2: {32: [0x60, 0xa4], 64: [0x98, 0x100]},
    V3: {32: [0x60, 0x9c], 64: [0x98, 0x100]},
    il2cpp: {32: [0x40, 0xac], 64: [0x80, 0x124]},
};
let cases = 0;
for (const family of Object.keys(fieldLayouts)) for (const width of [32, 64])
for (const parallel of [false, true]) for (const mode of modes) {
    if (parallel && family !== 'il2cpp' && mode.startsWith('wrong ')) continue;
    const f = createKeyedCollectionFixture({family, width, parallel, dictionary: false});
    const {memory, number, ptr, object, bytes, outer, stride, hash, next, value} = f;
    const mono = family !== 'il2cpp', wide = width === 64;
    const [fieldsOffset, countOffset] = fieldLayouts[family][width];
    ptr(0x14000n + BigInt(fieldsOffset), 0x58000n);
    const fields = [['rows', 0x10], ['sets', 0x18], ['instance', 0x20], ['values', 0x10], ['score', 0x18], ['wrongValues', 0x28]];
    number(0x14000n + BigInt(countOffset), mono ? 4 : 2, fields.length);
    fields.forEach(([name, offset], i) => {
        const field = 0x58000n + BigInt(i * (wide ? 32 : mono ? 16 : 20));
        const address = 0x59000n + BigInt(i * 256);
        ptr(field + BigInt(mono ? bytes : 0), address);
        const data = new Uint8Array(256); data.set(new TextEncoder().encode(name));
        data.forEach((byte, j) => memory.set(address + BigInt(j), byte));
        number(field + BigInt(wide ? 0x18 : 0xc), 4, offset);
    });
    // HashSet values are SZARRAY references, with nullable String elements.
    number(0x50400n + BigInt(bytes + 2), 1, 0x1d);
    ptr((mono ? 0x18000n : 0x16000n) + 0x28n, object);
    const arrays = [0x80000n, 0x120000n, 0x180000n];
    const row = 0x220000n, tailRow = 0x221000n, text = 0x230000n, tail = 0x231000n;
    const vector = (at, values, capacity = values.length) => {
        ptr(at + BigInt(2 * bytes), 0); ptr(at + BigInt(3 * bytes), capacity);
        values.forEach((item, i) => ptr(at + BigInt(4 * bytes + i * bytes), item));
    };
    const string = (at, text) => {
        number(at + BigInt(2 * bytes), 4, text.length);
        for (let i = 0; i < text.length; i++) number(at + BigInt(2 * bytes + 4 + 2 * i), 2, text.charCodeAt(i));
    };
    const first = object + BigInt(outer.at(-2)[1]), second = object + BigInt(outer.at(-1)[1]);
    const counts = (touched, live) => { number(first, 4, parallel ? touched : live); number(second, 4, parallel ? live : touched); };
    const backing = outer.slice(1, parallel ? -2 : 2).map(field => object + BigInt(field[1]));
    backing.forEach((field, i) => { ptr(field, arrays[i]); vector(arrays[i], [], 6); });
    const valueSlot = i => parallel ? arrays[1] + BigInt(4 * bytes + i * bytes) : arrays[0] + BigInt(4 * bytes + i * stride + value);
    const marker = (i, live) => {
        const at = arrays[0] + BigInt(4 * bytes + i * stride);
        number(at + BigInt(hash), 4, parallel ? (live ? 0x80000042 : 0x42) : (live ? 0x123 : 0xffffffff));
        number(at + BigInt(next), 4, -1);
    };
    counts(4, 2);
    for (let i = 0; i < 4; i++) marker(i, i === 0 || i === 3);
    ptr(valueSlot(0), row);
    ptr(valueSlot(3), tailRow);
    string(text, 'seed'); string(tail, 'tail'); vector(row, [text, 0n, tail]); vector(tailRow, []);
    const statics = mono ? 0x18000n : 0x16000n;
    ptr(statics + 0x10n, object); ptr(statics + 0x18n, 0x240000n); ptr(statics + 0x20n, 0x250000n);
    vector(0x240000n, [object, 0n]); ptr(0x250010n, object); number(0x250018n, 4, 42);
    number(0x6f000n, 4, 0);
    const limit = (1n << BigInt(width)) - 1n;
    const reads = [], originalRead = f.process.read;
    let armed = false;
    f.process.read = request => {
        const address = BigInt.asUintN(64, request.address);
        assert(address + BigInt(Math.max(request.length, 1) - 1) <= limit, `${mode}: overflowing host read`);
        reads.push(address);
        const result = originalRead({...request, address});
        if (armed && address === text + BigInt(2 * bytes + 4)) {
            armed = false;
            if (mode === 'torn count') counts(4, 1);
            if (mode === 'torn backing') ptr(backing[0], 0);
        }
        return result;
    };
    const host = await SplitScriptHost.instantiate(wasm);
    const label = `${family}/${width}/${parallel ? 'parallel' : 'entries'}/${mode}`;
    host.addProcess('game.exe', f.process); host.start();
    host.updateUntil(() => host.variables.has('rows'), label); host.update(2);
    const normalize = value => value.replace(/\s/g, '');
    const before = Object.fromEntries(['rows', 'sets', 'tree'].map(key => [key, normalize(host.variables.get(key))]));
    assert.equal(before.rows, 'Set{[Some("seed",),None,Some("tail",),],[],}', label);
    assert(before.sets.includes(before.rows), `${label}: nested set`);
    assert(before.tree.includes(before.rows), `${label}: snapshot set`);
    reads.length = 0;
    let success = ['seed', 'mutate', 'empty', 'shared empty backing', 'all deleted', 'large capacity', 'cached layout', 'local mutable'].includes(mode);
    if (mode === 'mutate') string(text, 'new');
    if (mode === 'empty') counts(0, 0);
    if (mode === 'shared empty backing') {
        counts(0, 0); vector(arrays[0], []);
        backing.forEach(field => ptr(field, arrays[0]));
    }
    if (mode === 'all deleted') { counts(4, 0); marker(0, false); marker(3, false); }
    if (mode === 'large capacity') arrays.forEach(at => ptr(at + BigInt(3 * bytes), 65536));
    if (mode === 'duplicate values') vector(tailRow, [text, 0n, tail]);
    if (mode === 'unreadable value' || mode === 'retry') memory.delete(row + BigInt(4 * bytes));
    if (mode === 'unreadable string') memory.delete(text + BigInt(2 * bytes + 4));
    if (mode === 'null set') ptr(statics + 0x10n, 0);
    if (mode === 'scan budget') counts(4097, 2);
    if (mode === 'shared scan budget') {
        counts(2049, 2);
        arrays.forEach(at => ptr(at + BigInt(3 * bytes), 2049));
        for (let i = 4; i < 2049; i++) marker(i, false);
        vector(0x240000n, [object, object]); success = true;
    }
    if (mode === 'shared element budget') { vector(row, Array(8192).fill(0n)); vector(tailRow, Array(8192).fill(0n)); }
    if (mode === 'comparison budget') {
        counts(182, 182);
        arrays.forEach(at => ptr(at + BigInt(3 * bytes), 182));
        for (let i = 0; i < 182; i++) {
            marker(i, true);
            const at = 0x300000n + BigInt(i * 256);
            string(at, `value ${i}`); vector(at + 0x100000n, [at]); ptr(valueSlot(i), at + 0x100000n);
        }
    }
    if (mode === 'byte budget') number(text + BigInt(2 * bytes), 4, 524288);
    if (mode === 'backing cycle') ptr(valueSlot(0), arrays[0]);
    if (mode.startsWith('torn')) { armed = true; string(text, 'new'); }
    if (mode === 'cached layout') { memory.delete(0x40200n); memory.delete(0x37000n); }
    if (mode === 'local mutable') number(0x6f000n, 4, 7);
    if (mode.startsWith('freeze')) {
        number(0x6f000n, 4, ['freeze clear', 'freeze insert', 'freeze remove', 'freeze absent', 'freeze child', 'freeze snapshot'].indexOf(mode) + 1);
        assert.throws(() => host.update(), WebAssembly.RuntimeError, label); cases++; continue;
    }
    if (mode.startsWith('wrong ')) { number(0x6f000n, 4, mode === 'wrong value schema' ? 8 : 9); success = true; }
    host.update();
    if (mode.startsWith('wrong ')) assert.match(host.variables.get('wrong'), /member type is incompatible with its schema/, label);
    assert.equal(normalize(host.variables.get('old')), before.rows, `${label}: immutable old snapshot`);
    for (const field of ['rows', 'sets', 'tree']) {
        let expected = before[field];
        if (mode === 'mutate') expected = expected.replaceAll('seed', 'new');
        if (['empty', 'shared empty backing', 'all deleted'].includes(mode)) expected = expected.replaceAll(before.rows, 'Set{}');
        assert.equal(normalize(host.variables.get(field)), expected, `${label}: ${field}`);
    }
    assert.equal(host.variables.get('result') === 'ok', success, `${label}: ${host.variables.get('result')}`);
    if (mode === 'local mutable') assert.equal(host.variables.get('local'), '1', label);
    if (mode === 'retry') {
        ptr(row + BigInt(4 * bytes), text); string(text, 'new');
        host.updateUntil(() => host.variables.get('result') === 'ok', label);
        assert(normalize(host.variables.get('rows')).includes('"new"'), label);
    }
    cases++;
}
console.log(JSON.stringify({managedSetCases: cases}));
