import {writeGenericType} from './support/managed_type_fixture.mjs';
import assert from 'node:assert/strict';
import { SplitScriptHost } from './support/splitscript_host.mjs';
import { createKeyedCollectionFixture } from './support/keyed_collection_fixture.mjs';
const [wasm] = process.argv.slice(2);
const layouts = {
    V1Cattrs: {32: [0x78, 0x68], 64: [0xb0, 0x9c]},
    V2: {32: [0x60, 0xa4], 64: [0x98, 0x100]},
    V3: {32: [0x60, 0x9c], 64: [0x98, 0x100]},
    il2cpp: {32: [0x40, 0xac], 64: [0x80, 0x124]},
};
let cases = 0;
for (const family of Object.keys(layouts)) for (const width of [32, 64])
for (const parallel of [false, true]) for (const mode of ['generic', 'generic short schema', 'seed', 'mutate', 'duplicate', 'unreadable', 'freeze', 'short schema', 'long schema']) {
    const f = createKeyedCollectionFixture({family, width, parallel, inline: true});
    const {number, ptr, memory, object, outer, bytes, stride, hash, next, key, value} = f;
    const mono = family !== 'il2cpp', wide = width === 64, [fields, count] = layouts[family][width];
    if (mode.startsWith('generic')) writeGenericType({mono, width, ptr, number, cachedClass: f.valueClass}, f.valueType, true);
    ptr(0x14000n + BigInt(fields), 0x58000n);
    number(0x14000n + BigInt(count), mono ? 4 : 2, 3);
    ['rows', 'shortRows', 'longRows'].forEach((text, i) => {
        const field = 0x58000n + BigInt(i * (wide ? 32 : mono ? 16 : 20));
        const textAt = 0x59000n + BigInt(i * 256), offset = 0x10 + i * bytes;
        ptr(field + BigInt(mono ? bytes : 0), textAt);
        const name = new Uint8Array(256); name.set(new TextEncoder().encode(text));
        name.forEach((byte, j) => memory.set(textAt + BigInt(j), byte));
        number(field + BigInt(wide ? 0x18 : 0xc), 4, offset);
        ptr((mono ? 0x18000n : 0x16000n) + BigInt(offset), object);
    });
    number(object + BigInt(outer.at(-2)[1]), 4, 2);
    number(object + BigInt(outer.at(-1)[1]), 4, parallel ? 2 : 0);
    const arrays = [0x80000n, 0x120000n, 0x180000n];
    outer.slice(1, parallel ? -2 : 2).forEach((field, i) => {
        ptr(object + BigInt(field[1]), arrays[i]);
        ptr(arrays[i] + BigInt(2 * bytes), 0); ptr(arrays[i] + BigInt(3 * bytes), 2);
    });
    const keySlot = i => parallel ? arrays[1] + BigInt(4 * bytes + i) : arrays[0] + BigInt(4 * bytes + i * stride + key);
    const valueSlot = i => parallel ? arrays[2] + BigInt(4 * bytes + i * 16) : arrays[0] + BigInt(4 * bytes + i * stride + value);
    for (let i = 0; i < 2; i++) {
        const at = arrays[0] + BigInt(4 * bytes + i * stride);
        number(at + BigInt(hash), 4, parallel ? 0x80000001 : 1); number(at + BigInt(next), 4, -1);
        number(keySlot(i), 1, i === 0 ? 255 : 1);
        for (let j = 0; j < 4; j++) number(valueSlot(i) + BigInt(j * 4), 4, i === 0 ? j - 1 : j + 10);
    }
    number(0x6f000n, 4, 0);
    const host = await SplitScriptHost.instantiate(wasm);
    const label = `${family}/${width}/${parallel}/${mode}`;
    host.addProcess('game.exe', f.process); host.start();
    host.updateUntil(() => host.variables.has('rows'), label); host.update(2);
    const normalize = value => value.replace(/\s/g, '');
    const before = normalize(host.variables.get('rows'));
    assert.equal(before, 'Map{255:Record{values:[-1,0,1,2,],},1:Record{values:[10,11,12,13,],},}', label);
    if (mode === 'mutate') number(valueSlot(0), 4, 99);
    if (mode === 'duplicate') number(keySlot(1), 1, 255);
    if (mode === 'unreadable') memory.delete(valueSlot(1) + 15n);
    if (mode === 'freeze') {
        number(0x6f000n, 4, 1);
        assert.throws(() => host.update(), WebAssembly.RuntimeError, label); cases++; continue;
    }
    if (mode.endsWith('schema')) number(0x6f000n, 4, mode.endsWith('short schema') ? 2 : 3);
    host.update();
    if (mode.endsWith('schema')) assert.match(host.variables.get('wrong'), /width is incompatible/, label);
    assert.equal(normalize(host.variables.get('old')), before, label);
    assert.equal(normalize(host.variables.get('rows')), mode === 'mutate' ? before.replace('-1', '99') : before, label);
    assert.equal(host.variables.get('result') === 'ok', ['generic', 'generic short schema', 'seed', 'mutate', 'short schema', 'long schema'].includes(mode), label);
    cases++;
}
console.log(JSON.stringify({managedInlineMapCases: cases}));
