import {writeVectorElementType} from './support/managed_type_fixture.mjs';
import assert from 'node:assert/strict';
import { readFile } from 'node:fs/promises';
import { SplitScriptHost } from './support/splitscript_host.mjs';
import { createMonoPeFixture } from './support/mono_pe_fixture.mjs';
import { createIl2cppPeFixture } from './support/il2cpp_pe_fixture.mjs';
const [wasm, backend] = process.argv.slice(2);
const mono = backend === 'mono';
const profiles = JSON.parse(await readFile(new URL('./fixtures/mono-pe-profiles.json', import.meta.url)));
const typeModes = ['element null', 'element unreadable', 'element unsupported', 'element overflow', 'items class', 'items string', 'items multidimensional array', 'items generic',
    'size unsigned', 'size long', 'null items type', 'null size type', 'unreadable type pointer',
    'unreadable type kind', 'overflowing type'];
const modes = [...typeModes, 'derived', 'direct', 'replacement', 'late object', 'null header', 'null class', 'unreadable object', 'unreadable name', 'unreadable parent', 'cycle', 'wrong namespace', 'wrong name', 'null fields', 'missing items', 'missing size', 'duplicate items', 'duplicate size', 'negative offset', 'header offset', 'overlap', 'large count', 'unreadable field', 'null name', 'unreadable generic', 'null generic', 'object overflow', 'field span overflow', 'negative count', 'unreadable count', 'unreadable namespace', 'null required name', 'null definition'];
let cases = 0;
for (const width of [32, 64]) for (const mode of modes) {
    if (!mono && ['null class', 'unreadable generic', 'null generic', 'negative count', 'null definition'].includes(mode)) continue;
    const wide = width === 64, bytes = width / 8;
    const fixture = mono ? createMonoPeFixture(profiles.builds.find(p => p.width === width && p.version === 'V2'))
        : createIl2cppPeFixture({width, version:[2022, 3, 0, 37029]});
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
    const root = 0x30000n, list = 0x31000n, definition = 0x32000n, fields = 0x33000n, generic = 0x35000n;
    const object = 0x70000n, vtable = 0x71000n;
    const nameOffset = mono ? (wide ? 0x48 : 0x2c) : (wide ? 0x10 : 8);
    const namespaceOffset = mono ? (wide ? 0x50 : 0x30) : (wide ? 0x18 : 0xc);
    const parentOffset = mono ? (wide ? 0x30 : 0x20) : (wide ? 0x58 : 0x2c);
    const fieldsOffset = mono ? (wide ? 0x98 : 0x60) : (wide ? 0x80 : 0x40);
    const countOffset = mono ? (wide ? 0x100 : 0xa4) : (wide ? 0x124 : 0xac);
    const stride = wide ? 32 : mono ? 16 : 20;
    const valueOffset = wide ? 0x18 : 0xc;
    const kindOffset = wide ? 0x2a : 0x1e;
    const genericOffset = wide ? 0xf0 : 0x94;
    for (const klass of [root, list, definition]) {
        for (let i = 0n; i < 0x200n; i++) memory.set(klass + i, 0);
    }
    ptr(root + BigInt(nameOffset), 0x40000n); text(0x40000n, 'DerivedList');
    ptr(root + BigInt(namespaceOffset), 0x41000n); text(0x41000n, 'Game');
    ptr(root + BigInt(parentOffset), list);
    ptr(list + BigInt(nameOffset), 0x42000n); text(0x42000n, 'List`1');
    ptr(list + BigInt(namespaceOffset), 0x43000n); text(0x43000n, 'System.Collections.Generic');
    ptr(list + BigInt(fieldsOffset), fields);
    if (mono) {
        number(list + BigInt(kindOffset), 1, 3);
        ptr(list + BigInt(genericOffset), generic); ptr(generic, definition);
        number(list + BigInt(countOffset), 4, 0x7fffffff); // Inflated count must be ignored.
    }
    const counted = mono ? definition : list;
    const entries = [['ignored', 0x80], ['_items', 2 * bytes], ['_size', 3 * bytes]];
    if (mode === 'missing items') entries[1][0] = 'items';
    if (mode === 'missing size') entries[2][0] = 'size';
    if (mode === 'duplicate items') entries.push(['_items', 0x90]);
    if (mode === 'duplicate size') entries.push(['_size', 0x90]);
    if (mode === 'negative offset') entries[1][1] = -1;
    if (mode === 'header offset') entries[1][1] = bytes;
    if (mode === 'overlap') entries[2][1] = 2 * bytes + 1;
    number(counted + BigInt(countOffset), mono ? 4 : 2, mode === 'large count' ? 4097 : entries.length);
    const itemsType = 0x54000n, sizeType = 0x54100n;
    const elementType = writeVectorElementType({mono, width, family: "V2", ptr, number}, itemsType, 0x55000n, 0x0e);
    number(sizeType + BigInt(bytes + 2), 1, 0x08);
    entries.forEach(([name, offset], i) => {
        const field = fields + BigInt(i * stride), at = 0x44000n + BigInt(i * 256);
        ptr(field + BigInt(mono ? bytes : 0), at); text(at, name);
        number(field + BigInt(valueOffset), 4, offset);
        // The ignored field intentionally has no readable type metadata.
        if (name === '_items' || name === '_size') {
            ptr(field + BigInt(mono ? 0 : bytes), name === '_items' ? itemsType : sizeType);
        }
    });
    ptr(object, mono ? vtable : root); ptr(vtable, root); number(0x60000n, 8, object);
    if (mode === 'direct') { ptr(object, mono ? vtable : list); ptr(vtable, list); }
    if (mode === 'late object') number(0x60000n, 8, 0);
    if (mode === 'null header') ptr(object, 0);
    if (mode === 'null class') ptr(vtable, 0);
    if (mode === 'unreadable object') memory.delete(object);
    if (mode === 'unreadable name') memory.delete(0x42000n);
    if (mode === 'unreadable parent') memory.delete(root + BigInt(parentOffset));
    if (mode === 'cycle') { text(0x42000n, 'Other'); ptr(list + BigInt(parentOffset), root); }
    if (mode === 'wrong namespace') text(0x43000n, 'Game.Collections');
    if (mode === 'wrong name') text(0x42000n, 'List');
    if (mode === 'null fields') ptr(list + BigInt(fieldsOffset), 0);
    if (mode === 'unreadable field') memory.delete(fields + BigInt(stride + valueOffset));
    if (mode === 'null name') ptr(fields + BigInt(mono ? bytes : 0), 0);
    if (mode === 'unreadable generic') memory.delete(generic);
    if (mode === 'null generic') ptr(list + BigInt(genericOffset), 0);
    const limit = (1n << BigInt(width)) - 1n;
    if (mode === 'object overflow') number(0x60000n, 8, limit - 1n);
    if (mode === 'field span overflow') ptr(list + BigInt(fieldsOffset), limit - 1n);
    if (mode === 'negative count') number(counted + BigInt(countOffset), 4, -1);
    if (mode === 'unreadable count') memory.delete(counted + BigInt(countOffset));
    if (mode === 'unreadable namespace') memory.delete(0x43000n);
    if (mode === 'null required name') ptr(fields + BigInt(stride + (mono ? bytes : 0)), 0);
    if (mode === 'null definition') ptr(generic, 0);
    const itemsFieldType = fields + BigInt(stride + (mono ? 0 : bytes));
    const sizeFieldType = fields + BigInt(2 * stride + (mono ? 0 : bytes));
    const itemsKinds = {'items class': 0x12, 'items string': 0x0e, 'items multidimensional array': 0x14, 'items generic': 0x15};
    if (mode in itemsKinds) number(itemsType + BigInt(bytes + 2), 1, itemsKinds[mode]);
    if (mode === 'size unsigned') number(sizeType + BigInt(bytes + 2), 1, 0x09);
    if (mode === 'size long') number(sizeType + BigInt(bytes + 2), 1, 0x0a);
    if (mode === 'null items type') ptr(itemsFieldType, 0);
    if (mode === 'null size type') ptr(sizeFieldType, 0);
    if (mode === 'unreadable type pointer') memory.delete(itemsFieldType);
    if (mode === 'unreadable type kind') memory.delete(itemsType + BigInt(bytes + 2));
    if (mode === 'overflowing type') ptr(itemsFieldType, limit - 1n);
    if (mode === 'element null') ptr(itemsType, 0);
    if (mode === 'element unreadable') memory.delete(elementType + BigInt(bytes + 2));
    if (mode === 'element unsupported') number(elementType + BigInt(bytes + 2), 1, 0x10);
    if (mode === 'element overflow') ptr(itemsType, limit - 1n);
    const originalRead = fixture.process.read;
    fixture.process.read = request => {
        const address = BigInt.asUintN(64, request.address);
        assert(address + BigInt(Math.max(request.length, 1) - 1) <= limit, `${backend}/${width}/${mode}: overflowing host read`);
        return originalRead({...request, address});
    };
    const host = await SplitScriptHost.instantiate(wasm);
    host.addProcess('game.exe', fixture.process); host.start();
    host.updateUntil(() => host.variables.has('result'), `${backend}/${width}/${mode}`);
    const label = `${backend}/${width}/${mode}`;
    const success = ['derived', 'direct', 'replacement', 'null name'].includes(mode);
    assert.equal(host.variables.get('result') === 'ok', success, `${label}: ${host.variables.get('result')}`);
    if (success) {
        assert.equal(BigInt(host.variables.get('class')), mode === 'direct' ? list : root, label);
        assert.equal(BigInt(host.variables.get('owner')), list, label);
        assert.equal(host.variables.get('items'), String(2 * bytes), label);
        assert.equal(host.variables.get('size'), String(3 * bytes), label);
    }
    if (typeModes.includes(mode)) {
        if (mode in itemsKinds || mode.startsWith('size ')) {
            assert.match(host.variables.get('result'), /invalid backing or count field types/, label);
        }
        // Failed discovery must remain retryable when metadata materializes.
        ptr(itemsFieldType, itemsType); ptr(sizeFieldType, sizeType);
        writeVectorElementType({mono, width, family: "V2", ptr, number}, itemsType, 0x55000n, 0x0e);
        number(sizeType + BigInt(bytes + 2), 1, 0x08);
        host.updateUntil(() => host.variables.get('result') === 'ok', label);
    }
    if (mode === 'late object' || mode === 'replacement') {
        number(0x60000n, 8, object);
        ptr(object, mono ? vtable : list); ptr(vtable, list);
        host.updateUntil(() => host.variables.get('result') === 'ok' && BigInt(host.variables.get('class')) === list, label);
    }
    cases++;
}
console.log(JSON.stringify({backend, listLayoutCases:cases}));
