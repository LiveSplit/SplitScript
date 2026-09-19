import assert from 'node:assert/strict';
import { SplitScriptHost } from './support/splitscript_host.mjs';
import { createKeyedCollectionFixture } from './support/keyed_collection_fixture.mjs';

const [wasm] = process.argv.slice(2);
const fields = {
    V1Cattrs: {32: [0x78, 0x68], 64: [0xb0, 0x9c]},
    V2: {32: [0x60, 0xa4], 64: [0x98, 0x100]},
    V3: {32: [0x60, 0x9c], 64: [0x98, 0x100]},
    il2cpp: {32: [0x40, 0xac], 64: [0x80, 0x124]},
};
let cases = 0;
for (const family of Object.keys(fields)) for (const width of [32, 64]) {
    const f = createKeyedCollectionFixture({family, width, dictionary: false});
    const {number, ptr, memory, bytes, object, outer, stride, hash, next, value} = f;
    const mono = family !== 'il2cpp', [fieldOffset, countOffset] = fields[family][width];
    ptr(0x14000n + BigInt(fieldOffset), 0x58000n);
    number(0x14000n + BigInt(countOffset), mono ? 4 : 2, 3);
    ['rows', 'codes', 'floats'].forEach((text, i) => {
        const field = 0x58000n + BigInt(i * (width === 64 ? 32 : mono ? 16 : 20));
        const textAt = 0x59000n + BigInt(i * 256), offset = 0x10 + i * bytes;
        ptr(field + BigInt(mono ? bytes : 0), textAt);
        const name = new Uint8Array(256); name.set(new TextEncoder().encode(text));
        name.forEach((byte, j) => memory.set(textAt + BigInt(j), byte));
        number(field + BigInt(width === 64 ? 0x18 : 0xc), 4, offset);
        ptr((mono ? 0x18000n : 0x16000n) + BigInt(offset), object);
    });
    number(0x50400n + BigInt(bytes + 2), 1, 0x08); // System.Int32
    number(object + BigInt(outer[2][1]), 4, 2);
    number(object + BigInt(outer[3][1]), 4, 2);
    ptr(object + BigInt(outer[1][1]), 0x80000n);
    ptr(0x80000n + BigInt(2 * bytes), 0);
    ptr(0x80000n + BigInt(3 * bytes), 2);
    for (let i = 0; i < 2; i++) {
        const at = 0x80000n + BigInt(4 * bytes + i * stride);
        number(at + BigInt(hash), 4, 1);
        number(at + BigInt(next), 4, -1);
        number(at + BigInt(value), 4, i + 1);
    }
    const host = await SplitScriptHost.instantiate(wasm), label = `${family}/${width}`;
    host.addProcess('game.exe', f.process); host.start();
    host.updateUntil(() => host.variables.has('wrong'), label);
    assert.equal(host.variables.get('rows').replace(/\s/g, ''), 'Set{1,2,}', label);
    assert.equal(host.variables.get('codes'), '2', label);
    // The same cached shape must be checked against each requested decoder.
    assert.match(host.variables.get('wrong'), /member type is incompatible with its schema/, label);
    cases++;
}
console.log(JSON.stringify({managedScalarSetCases: cases}));
