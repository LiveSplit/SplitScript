import assert from 'node:assert/strict';
import { readFile } from 'node:fs/promises';
import { SplitScriptHost } from './support/splitscript_host.mjs';
import { createMonoPeFixture } from './support/mono_pe_fixture.mjs';
import { createIl2cppPeFixture } from './support/il2cpp_pe_fixture.mjs';

const [wasm, backend] = process.argv.slice(2);
const mono = backend === 'mono';
const profiles = JSON.parse(await readFile(new URL('./fixtures/mono-pe-profiles.json', import.meta.url)));
const modes = ['derived', 'direct', 'renamed', 'reversed members', 'signed hash', 'entry boundary',
    'wrong namespace', 'wrong name', 'cycle', 'missing buckets', 'missing backing', 'missing count',
    'missing member', 'duplicate field', 'duplicate member', 'outer overlap', 'inner overlap',
    'negative offset', 'outer header', 'inner header', 'negative size', 'zero stride', 'oversized stride',
    'integer overrun', 'reference overrun', 'wrong count type', 'wrong backing type', 'wrong hash type',
    'wrong next type', 'unsupported member type', 'null field type', 'unreadable type', 'null element class',
    'null field array', 'unreadable field', 'huge field count', 'unreadable generic', 'null definition',
    'object overflow', 'fields overflow', 'instance size overflow', 'unreadable size', 'retry', 'null type data', 'plain element type', 'array element type',
    'payload null type', 'payload unreadable type', 'payload unsupported type', 'payload type overflow',
    'null generic data', 'missing kind', 'missing data', 'missing field type', 'missing instance size', 'missing cached class'];
let cases = 0;
const monoLayouts = {
    V1Cattrs: {32: [0x34, 0x38, 0x24, 0x78, 0x68, null, null], 64: [0x50, 0x58, 0x30, 0xb0, 0x9c, null, null]},
    V2: {32: [0x2c, 0x30, 0x20, 0x60, 0xa4, 0x1e, 0x94], 64: [0x48, 0x50, 0x30, 0x98, 0x100, 0x2a, 0xf0]},
    V3: {32: [0x2c, 0x30, 0x20, 0x60, 0x9c, 0xf, 0x8c], 64: [0x48, 0x50, 0x30, 0x98, 0x100, 0x1b, 0xf0]},
};
for (const family of mono ? Object.keys(monoLayouts) : ['il2cpp']) for (const width of [32, 64]) for (const dictionary of [true, false]) for (const parallel of [false, true]) for (const mode of modes) {
    if (mode.startsWith('payload ') && (mono || !parallel)) continue;
    if (mono && ['plain element type', 'array element type', 'null generic data', 'missing cached class'].includes(mode)) continue;
    if (family === 'V1Cattrs' && ['unreadable generic', 'null definition'].includes(mode)) continue;
    const wide = width === 64, bytes = width / 8, header = 2 * bytes;
    const fixture = mono ? createMonoPeFixture(profiles.builds.find(p => p.width === width && p.version === family))
        : createIl2cppPeFixture({width, version: [2022, 3, 0, 37029]});
    const memory = fixture.memory;
    const write = (at, data) => data.forEach((byte, i) => memory.set(at + BigInt(i), byte));
    const number = (at, size, value) => {
        const data = new Uint8Array(size), view = new DataView(data.buffer);
        if (size === 8) view.setBigUint64(0, BigInt.asUintN(64, BigInt(value)), true);
        else if (size === 4) view.setUint32(0, Number(value) >>> 0, true);
        else if (size === 2) view.setUint16(0, Number(value), true);
        else view.setUint8(0, Number(value));
        write(at, data);
    };
    const ptr = (at, value) => number(at, bytes, value);
    const text = (at, value) => { const data = new Uint8Array(256); data.set(new TextEncoder().encode(value)); write(at, data); };
    // Independent ABI facts, not sourced from the generated profile under test.
    const [nameOffset, namespaceOffset, parentOffset, fieldsOffset, countOffset, kindOffset, genericOffset] = mono
        ? monoLayouts[family][width] : wide ? [0x10, 0x18, 0x58, 0x80, 0x124] : [8, 0xc, 0x2c, 0x40, 0xac];
    const instanceSizeOffset = mono ? (wide ? 0x1c : 0x10) : (wide ? 0xf8 : 0x80);
    const fieldStride = wide ? 32 : mono ? 16 : 20;
    const valueOffset = wide ? 0x18 : 0xc;
    const typeKindOffset = bytes + 2;
    const root = 0x30000n, owner = 0x31000n, definition = 0x32000n, entry = 0x33000n, entryDefinition = 0x34000n;
    const generic = 0x35000n, entryGeneric = 0x36000n, outerFields = 0x37000n, innerFields = 0x38000n;
    const object = 0x70000n, vtable = 0x71000n;
    const vectorType = 0x50000n, elementType = 0x50100n, intType = 0x50200n, hashType = 0x50300n, stringType = 0x50400n;
    for (const klass of [root, owner, definition, entry, entryDefinition]) {
        for (let i = 0n; i < 0x200n; i++) memory.set(klass + i, 0);
    }
    let textIndex = 0;
    const named = (klass, name, namespace) => {
        const at = 0x40000n + BigInt(textIndex++ * 512);
        ptr(klass + BigInt(nameOffset), at); text(at, name);
        ptr(klass + BigInt(namespaceOffset), at + 256n); text(at + 256n, namespace);
    };
    named(root, 'DerivedCollection', 'Game'); named(owner, dictionary ? 'Dictionary`2' : 'HashSet`1', 'System.Collections.Generic');
    ptr(root + BigInt(parentOffset), owner);
    number(vectorType + BigInt(typeKindOffset), 1, 0x1d);
    ptr(vectorType, mono ? entry : elementType);
    number(elementType + BigInt(typeKindOffset), 1, 0x15); ptr(elementType, entryGeneric);
    ptr(entryGeneric + BigInt(3 * bytes), entry);
    number(intType + BigInt(typeKindOffset), 1, 0x08);
    number(hashType + BigInt(typeKindOffset), 1, mode === 'signed hash' ? 0x08 : 0x09);
    number(stringType + BigInt(typeKindOffset), 1, 0x0e);
    const names = parallel
        ? (dictionary ? ['table', 'linkSlots', 'keySlots', 'valueSlots', 'touchedSlots', 'count'] : ['table', 'links', 'slots', 'touched', 'count'])
        : dictionary ? ['_buckets', '_entries', '_count', '_freeCount'] : ['_buckets', '_slots', '_count', '_lastIndex'];
    if (mode === 'renamed' && !parallel) names.forEach((name, i) => names[i] = dictionary ? name.slice(1) : `m${name}`);
    const outer = names.map((name, i) => [name, header + i * bytes, i >= names.length - 2 ? intType : vectorType]);
    if (!mono && parallel) {
        const payloadVector = 0x50500n;
        number(payloadVector + BigInt(typeKindOffset), 1, 0x1d);
        ptr(payloadVector, stringType);
        for (let i = 2; i < outer.length - 2; i++) outer[i][2] = payloadVector;
        if (mode === 'payload null type') ptr(payloadVector, 0);
        if (mode === 'payload unreadable type') memory.delete(stringType + BigInt(typeKindOffset));
        if (mode === 'payload unsupported type') number(stringType + BigInt(typeKindOffset), 1, 0x10);
        if (mode === 'payload type overflow') ptr(payloadVector, (1n << BigInt(width)) - 1n);
    }
    const inner = parallel ? [['HashCode', header, hashType], ['Next', header + 4, intType]]
        : [['hashCode', header, hashType], ['next', header + 4, intType], ...(dictionary ? [['key', header + 8, stringType]] : []), ['value', header + 8 + (dictionary ? bytes : 0), stringType]];
    let stride = parallel ? 8 : 8 + (dictionary ? 2 : 1) * bytes;
    if (mode === 'reversed members') { [inner[0][1], inner[1][1]] = [inner[1][1], inner[0][1]]; inner.reverse(); }
    if (mode === 'entry boundary' && !parallel) { stride = 1024; inner.at(-1)[1] = header + stride - bytes; }
    if (mode === 'missing buckets') outer[0][0] = 'unrelated';
    if (mode === 'missing backing') outer[1][0] = 'unrelated';
    if (mode === 'missing count') outer.at(-1)[0] = 'unrelated';
    if (mode === 'missing member') inner[1][0] = 'unrelated';
    if (mode === 'duplicate field') outer.push([...outer[1]]);
    if (mode === 'duplicate member') inner.push([...inner[1]]);
    if (mode === 'outer overlap') outer[1][1] = outer[0][1] + 1;
    if (mode === 'inner overlap') inner[1][1] = inner[0][1] + 1;
    if (mode === 'negative offset') outer[0][1] = -1;
    if (mode === 'outer header') outer[0][1] = bytes;
    if (mode === 'inner header') inner[0][1] = bytes;
    if (mode === 'negative size') stride = -header - 1;
    if (mode === 'zero stride') stride = 0;
    if (mode === 'oversized stride') stride = 1025;
    if (mode === 'integer overrun') inner[1][1] = header + stride - 3;
    if (mode === 'reference overrun') inner.at(-1)[1] = header + stride - (parallel ? 3 : bytes - 1);
    if (mode === 'wrong count type') outer.at(-1)[2] = stringType;
    if (mode === 'wrong backing type') outer[1][2] = stringType;
    if (mode === 'wrong hash type') inner[0][2] = stringType;
    if (mode === 'wrong next type') inner[1][2] = hashType;
    if (mode === 'unsupported member type') number(stringType + BigInt(typeKindOffset), 1, 0x10);
    if (mode === 'unsupported member type' && parallel) inner[0][2] = stringType;
    if (mode === 'null field type') outer[1][2] = 0n;
    let nameIndex = 0;
    const fields = (klass, counted, at, members) => {
        ptr(klass + BigInt(fieldsOffset), at);
        number(counted + BigInt(countOffset), mono ? 4 : 2, members.length);
        if (mono && kindOffset !== null) {
            number(klass + BigInt(kindOffset), 1, 3);
            const descriptor = klass === owner ? generic : 0x39000n;
            ptr(klass + BigInt(genericOffset), descriptor); ptr(descriptor, counted);
            number(klass + BigInt(countOffset), 4, 0x7fffffff);
        }
        members.forEach(([name, offset, type], i) => {
            const field = at + BigInt(i * fieldStride), nameAddress = 0x44000n + BigInt(nameIndex++ * 256);
            ptr(field + BigInt(mono ? bytes : 0), nameAddress); text(nameAddress, name);
            ptr(field + BigInt(mono ? 0 : bytes), type);
            number(field + BigInt(valueOffset), 4, offset);
        });
    };
    fields(owner, mono && kindOffset !== null ? definition : owner, outerFields, outer);
    fields(entry, mono && kindOffset !== null ? entryDefinition : entry, innerFields, inner);
    number(entry + BigInt(instanceSizeOffset), 4, stride + header);
    ptr(object, mono ? vtable : root); ptr(vtable, root);
    number(0x60000n, 8, object); number(0x60008n, 1, dictionary ? 0 : 1);
    number(0x6000an, 1, family === 'V1Cattrs' ? 1 : family === 'V3' ? 3 : 2);
    number(0x60009n, 1, ['missing kind', 'missing data', 'missing field type', 'missing instance size', 'missing cached class'].indexOf(mode) + 1);
    if (mode === 'null type data') ptr(vectorType, 0);
    if (mode === 'plain element type') number(elementType + BigInt(typeKindOffset), 1, 0x11);
    if (mode === 'array element type') number(elementType + BigInt(typeKindOffset), 1, 0x1d);
    if (mode === 'null generic data') ptr(elementType, 0);
    const limit = (1n << BigInt(width)) - 1n;
    if (mode === 'direct') { ptr(object, mono ? vtable : owner); ptr(vtable, owner); }
    if (mode === 'wrong namespace') text(0x40300n, 'Fake.Collections');
    if (mode === 'wrong name') text(0x40200n, 'Fake');
    if (mode === 'cycle') { text(0x40200n, 'Other'); ptr(owner + BigInt(parentOffset), root); }
    if (mode === 'unreadable type') memory.delete(vectorType + BigInt(typeKindOffset));
    if (mode === 'null element class') ptr(mono ? vectorType : entryGeneric + BigInt(3 * bytes), 0);
    if (mode === 'null field array') ptr(entry + BigInt(fieldsOffset), 0);
    if (mode === 'unreadable field') memory.delete(innerFields + BigInt(valueOffset));
    if (mode === 'huge field count') number((mono && kindOffset !== null ? entryDefinition : entry) + BigInt(countOffset), mono ? 4 : 2, 4097);
    if (mode === 'unreadable generic') memory.delete(mono ? 0x39000n : entryGeneric + BigInt(3 * bytes));
    if (mode === 'null definition') ptr(mono ? 0x39000n : elementType, 0);
    if (mode === 'object overflow') number(0x60000n, 8, limit - 1n);
    if (mode === 'fields overflow') ptr(entry + BigInt(fieldsOffset), limit - 1n);
    if (mode === 'instance size overflow') ptr(mono ? vectorType : entryGeneric + BigInt(3 * bytes), limit - 1n);
    if (mode === 'unreadable size' || mode === 'retry') memory.delete(entry + BigInt(instanceSizeOffset));
    const originalRead = fixture.process.read;
    fixture.process.read = request => {
        const address = BigInt.asUintN(64, request.address);
        assert(address + BigInt(Math.max(request.length, 1) - 1) <= limit, `${family}/${width}/${mode}: overflowing host read`);
        return originalRead({...request, address});
    };
    const host = await SplitScriptHost.instantiate(wasm);
    host.addProcess('game.exe', fixture.process); host.start();
    const label = `${family}/${width}/${dictionary ? 'map' : 'set'}/${parallel ? 'parallel' : 'entries'}/${mode}`;
    host.updateUntil(() => host.variables.has('result'), label);
    const success = ['derived', 'direct', 'renamed', 'reversed members', 'signed hash', 'entry boundary'].includes(mode);
    assert.equal(host.variables.get('result') === 'ok', success, `${label}: ${host.variables.get('result')}`);
    if (success) {
        assert.equal(BigInt(host.variables.get('class')), mode === 'direct' ? owner : root, label);
        assert.equal(BigInt(host.variables.get('owner')), owner, label);
        assert.equal(host.variables.get('parallel'), String(parallel), label);
        assert.equal(host.variables.get('stride'), String(stride), label);
        const values = name => (host.variables.get(name).match(/0x[0-9a-f]+|[0-9]+/gi) ?? []).map(v => BigInt(v).toString());
        assert.deepEqual(values('fields'), outer.map(v => String(v[1])), label);
        const ordered = parallel ? ['HashCode', 'Next'] : dictionary ? ['hashCode', 'next', 'key', 'value'] : ['hashCode', 'next', 'value'];
        const payloads = !mono && parallel ? (dictionary ? 2 : 1) : 0;
        assert.deepEqual(values('members'), [...ordered.map(name => String(inner.find(v => v[0] === name)[1] - header)), ...Array(payloads).fill('0')], label);
        assert.deepEqual(values('types'), [...ordered.map(name => String(inner.find(v => v[0] === name)[2])), ...Array(payloads).fill(String(stringType))], label);
    }
    if (mode === 'retry') {
        number(entry + BigInt(instanceSizeOffset), 4, stride + header);
        host.updateUntil(() => host.variables.get('result') === 'ok', label);
    }
    cases++;
}
console.log(JSON.stringify({backend, keyedLayoutCases: cases}));
