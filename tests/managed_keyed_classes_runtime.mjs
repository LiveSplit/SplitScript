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
for (const parallel of [false, true]) for (const active of [false, true]) for (const mode of ['seed', 'mutate', 'conditional', 'duplicate', 'unreadable', 'cycle', 'freeze']) {
    const f = createKeyedCollectionFixture({family, width, parallel, dictionary});
    const {number, ptr, memory, object, outer, bytes, stride, hash, next, key, value} = f;
    const mono = family !== 'il2cpp', wide = width === 64, [fields, count] = layouts[family][width];
    ptr(0x14000n + BigInt(fields), 0x58000n);
    const members = [['rows', 0x10], ['anchor', 0x18], ['label', 0x10], ['children', 0x18], ...(active ? [['inactive', 0x20]] : [])];
    number(0x14000n + BigInt(count), mono ? 4 : 2, members.length);
    members.forEach(([name, offset], i) => {
        const field = 0x58000n + BigInt(i * (wide ? 32 : mono ? 16 : 20));
        const address = 0x59000n + BigInt(i * 256);
        ptr(field + BigInt(mono ? bytes : 0), address);
        const text = new Uint8Array(256); text.set(new TextEncoder().encode(name));
        text.forEach((byte, j) => memory.set(address + BigInt(j), byte));
        number(field + BigInt(wide ? 0x18 : 0xc), 4, offset);
    });
    number((dictionary ? f.keyType : f.valueType) + BigInt(bytes + 2), 1, 0x12);
    const statics = mono ? 0x18000n : 0x16000n;
    ptr(statics + 0x10n, object); ptr(statics + 0x18n, 0x200000n);
    number(object + BigInt(outer.at(-2)[1]), 4, 2);
    number(object + BigInt(outer.at(-1)[1]), 4, dictionary && !parallel ? 0 : 2);
    const arrays = [0x80000n, 0x120000n, 0x180000n];
    outer.slice(1, parallel ? -2 : 2).forEach((field, i) => {
        ptr(object + BigInt(field[1]), arrays[i]);
        ptr(arrays[i] + BigInt(2 * bytes), 0); ptr(arrays[i] + BigInt(3 * bytes), 2);
    });
    const slot = i => parallel ? arrays[1] + BigInt((4 + i) * bytes) : arrays[0] + BigInt(4 * bytes + i * stride + (dictionary ? key : value));
    const vector = (at, values) => {
        ptr(at + BigInt(2 * bytes), 0); ptr(at + BigInt(3 * bytes), values.length);
        values.forEach((item, i) => ptr(at + BigInt((4 + i) * bytes), item));
    };
    const string = (at, text) => {
        number(at + BigInt(2 * bytes), 4, text.length);
        for (let i = 0; i < text.length; i++) number(at + BigInt(2 * bytes + 4 + i * 2), 2, text.charCodeAt(i));
    };
    const node = (at, label, children) => { ptr(at + 0x10n, label); ptr(at + 0x18n, children); if (active) ptr(at + 0x20n, at + 0x800n); };
    for (let i = 0; i < 2; i++) {
        const at = arrays[0] + BigInt(4 * bytes + i * stride);
        number(at + BigInt(hash), 4, parallel ? 0x80000001 : 1); number(at + BigInt(next), 4, -1);
        ptr(slot(i), 0x200000n + BigInt(i * 0x1000));
        if (dictionary) ptr(parallel ? arrays[2] + BigInt((4 + i) * bytes) : at + BigInt(value), 0x220000n);
    }
    // Distinct root allocations and strings compare through recursive children.
    node(0x200000n, 0x220000n, 0x230000n); node(0x201000n, 0x221000n, 0x231000n);
    string(0x220000n, 'root'); string(0x221000n, 'root');
    vector(0x230000n, [0x202000n, 0n]); vector(0x231000n, [0x203000n, 0n]);
    node(0x202000n, 0x222000n, 0x232000n); node(0x203000n, 0x223000n, 0x233000n);
    string(0x222000n, 'first'); string(0x223000n, 'second'); vector(0x232000n, []); vector(0x233000n, []);
    for (let i = 0; i < 4; i++) string(0x200800n + BigInt(i * 0x1000), 'conditional');
    number(0x6f000n, 4, 0); number(0x6f004n, 1, active ? 1 : 0);
    const host = await SplitScriptHost.instantiate(wasm);
    const label = `${kind}/${family}/${width}/${parallel}/${active}/${mode}`;
    host.addProcess('game.exe', f.process); host.start();
    host.updateUntil(() => host.variables.has('rows'), label); host.update(2);
    const before = host.variables.get('rows');
    assert.match(before, /first/); assert.match(before, /second/);
    assert.equal(host.variables.get('contains'), 'true', label);
    assert.equal(host.variables.get('equal'), 'true', label);
    if (mode === 'mutate') string(0x222000n, 'changed');
    if (mode === 'conditional') string(0x202800n, 'changed');
    if (mode === 'duplicate') string(0x223000n, 'first');
    if (mode === 'unreadable') memory.delete(0x203018n);
    if (mode === 'cycle') vector(0x231000n, [0x201000n]);
    if (mode === 'freeze') {
        number(0x6f000n, 4, 1);
        assert.throws(() => host.update(), WebAssembly.RuntimeError, label); cases++; continue;
    }
    host.update();
    assert.equal(host.variables.get('old'), before, label);
    assert.equal(host.variables.get('rows'), mode === 'mutate' ? before.replace('first', 'changed') : mode === 'conditional' && active ? before.replace('conditional', 'changed') : before, label);
    assert.equal(host.variables.get('result') === 'ok', ['seed', 'mutate', 'conditional'].includes(mode), label);
    assert.equal(host.variables.get('equal'), String(mode !== 'mutate' && !(mode === 'conditional' && active)), label);
    if (mode === 'duplicate') assert.match(host.variables.get('result'), /duplicate/, label);
    cases++;
}
console.log(JSON.stringify({managedClassCollectionCases: cases, kind}));
