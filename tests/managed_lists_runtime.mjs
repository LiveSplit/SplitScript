import assert from 'node:assert/strict';
import { readFile } from 'node:fs/promises';
import { SplitScriptHost } from './support/splitscript_host.mjs';
import { createMonoPeFixture } from './support/mono_pe_fixture.mjs';
import { createIl2cppPeFixture } from './support/il2cpp_pe_fixture.mjs';

const [wasm] = process.argv.slice(2);
const profiles = JSON.parse(await readFile(new URL('./fixtures/mono-pe-profiles.json', import.meta.url)));
const modes = ['seed', 'mutate', 'empty', 'large spare capacity', 'null empty backing',
    'negative count', 'count exceeds capacity', 'element budget', 'null backing', 'null list',
    'indexed string', 'unreadable size', 'unreadable backing slot', 'unreadable live slot', 'unreadable string',
    'null nested list', 'class cycle', 'shared element budget', 'capacity overflow',
    'list field overflow', 'backing bounds', 'runtime class replacement', 'cached layout',
    'reattach', 'retry discovery', 'freeze outer', 'freeze nested', 'freeze snapshot',
    'local arrays mutable', 'freeze inline', 'depth boundary', 'depth overflow', 'object boundary', 'object overflow',
    'torn size', 'torn backing', 'replacement items type', 'replacement size type'];
let cases = 0;
// Independent ABI facts: name, namespace, fields, field count, generic kind,
// and inflated generic descriptor. Older Mono classes store their count directly.
const monoLayouts = {
    V1: {32: [0x30, 0x34, 0x74, 0x64, null, null], 64: [0x48, 0x50, 0xa8, 0x94, null, null]},
    V1Cattrs: {32: [0x34, 0x38, 0x78, 0x68, null, null], 64: [0x50, 0x58, 0xb0, 0x9c, null, null]},
    V2: {32: [0x2c, 0x30, 0x60, 0xa4, 0x1e, 0x94], 64: [0x48, 0x50, 0x98, 0x100, 0x2a, 0xf0]},
    V3: {32: [0x2c, 0x30, 0x60, 0x9c, 0xf, 0x8c], 64: [0x48, 0x50, 0x98, 0x100, 0x1b, 0xf0]},
};
for (const backend of [...Object.keys(monoLayouts), 'il2cpp']) for (const width of [32, 64]) for (const mode of modes) {
    const mono = backend !== 'il2cpp';
    const wide = width === 64, bytes = width / 8;
    const fixture = mono ? createMonoPeFixture(profiles.builds.find(p => p.width === width && p.version === backend))
        : createIl2cppPeFixture({width, version: [2022, 3, 0, 37029]});
    const memory = fixture.memory;
    const write = (at, data) => data.forEach((byte, i) => memory.set(at + BigInt(i), byte));
    const number = (at, size, value) => {
        const data = new Uint8Array(size), view = new DataView(data.buffer);
        if (size === 8) view.setBigUint64(0, BigInt(value), true);
        else if (size === 4) view.setUint32(0, Number(value), true);
        else if (size === 2) view.setUint16(0, Number(value), true);
        else view.setUint8(0, Number(value));
        write(at, data);
    };
    const ptr = (at, value) => number(at, bytes, value);
    const name = (at, value) => { const data = new Uint8Array(256); data.set(new TextEncoder().encode(value)); write(at, data); };
    const [nameOffset, namespaceOffset, fieldsOffset, countOffset, kindOffset, genericOffset] = mono
        ? monoLayouts[backend][width] : (wide ? [0x10, 0x18, 0x80, 0x124] : [8, 0xc, 0x40, 0xac]);
    let nameIndex = 0;
    const fields = (owner, at, entries) => {
        ptr(owner + BigInt(fieldsOffset), at);
        number(owner + BigInt(countOffset), mono ? 4 : 2, entries.length);
        entries.forEach(([text, offset], index) => {
            const field = at + BigInt(index * (wide ? 32 : mono ? 16 : 20));
            const textAddress = 0x60000n + BigInt(nameIndex++ * 256);
            ptr(field + BigInt(mono ? bytes : 0), textAddress); name(textAddress, text);
            number(field + BigInt(wide ? 0x18 : 0xc), 4, offset);
            if (text === '_items' || text === '_size') {
                const type = 0x55000n + (text === '_items' ? 0n : 0x100n);
                ptr(field + BigInt(mono ? 0 : bytes), type);
                number(type + BigInt(bytes + 2), 1, text === '_items' ? 0x1d : 0x08);
            }
        });
    };
    fields(0x14000n, 0x50000n, [['rows', 0x10], ['vectors', 0x18], ['nested', 0x20], ['instance', 0x28],
        ['children', 0x10], ['tags', 0x18], ['value', 0x20], ['numbers', 0x30], ['pointers', 0x38], ['records', 0x40]]);
    const listClass = 0x31000n, definition = 0x32000n, generic = 0x35000n, vtable = 0x39000n;
    for (const klass of [listClass, definition]) for (let i = 0n; i < 0x200n; i++) memory.set(klass + i, 0);
    ptr(listClass + BigInt(nameOffset), 0x40000n); name(0x40000n, 'List`1');
    ptr(listClass + BigInt(namespaceOffset), 0x41000n); name(0x41000n, 'System.Collections.Generic');
    fields(listClass, 0x51000n, [['_items', 2 * bytes], ['_size', 3 * bytes]]);
    if (mono && kindOffset !== null) {
        number(listClass + BigInt(kindOffset), 1, 3);
        ptr(listClass + BigInt(genericOffset), generic); ptr(generic, definition);
        number(definition + BigInt(countOffset), 4, 2);
        number(listClass + BigInt(countOffset), 4, 0x7fffffff);
    }
    ptr(vtable, listClass);
    const rows = 0x70000n, innerList = 0x71000n, nested = 0x72000n, empty = 0x73000n;
    const root = 0x74000n, children = 0x75000n, child = 0x78000n;
    const rowsArray = 0x80000n, row = 0x81000n, vectors = 0x82000n, strings = 0x83000n;
    const nestedArray = 0x84000n, childrenArray = 0x85000n, string = 0x90000n, tail = 0x92000n;
    const vector = (at, values, capacity = values.length, stride = bytes) => {
        ptr(at + BigInt(2 * bytes), 0); ptr(at + BigInt(3 * bytes), capacity);
        values.forEach((value, index) => number(at + BigInt(4 * bytes + index * stride), stride, value));
    };
    const list = (at, array, count) => {
        ptr(at, mono ? vtable : listClass);
        ptr(at + BigInt(2 * bytes), array); number(at + BigInt(3 * bytes), 4, count);
    };
    const text = (at, value) => {
        number(at + BigInt(2 * bytes), 4, value.length);
        for (let i = 0; i < value.length; i++) number(at + BigInt(2 * bytes + 4 + i * 2), 2, value.charCodeAt(i));
    };
    const node = (at, kids, value) => {
        ptr(at + 0x10n, kids); ptr(at + 0x18n, innerList); number(at + 0x20n, 4, value);
    };
    const statics = mono ? 0x18000n : 0x16000n;
    [rows, vectors, nested, root].forEach((value, i) => ptr(statics + 0x10n + BigInt(i * 8), value));
    text(string, 'seed'); text(tail, 'tail');
    list(rows, rowsArray, 1); vector(rowsArray, [row], 2); // The spare slot is deliberately unreadable.
    vector(row, [string, 0n, tail]);
    vector(vectors, [innerList, 0n]); list(innerList, strings, 2); vector(strings, [string, tail]);
    list(nested, nestedArray, 2); vector(nestedArray, [innerList, empty]); list(empty, 0n, 0);
    list(children, childrenArray, 2); vector(childrenArray, [child, 0n]); node(root, children, 1); node(child, empty, 2);
    ptr(statics + 0x30n, 0x76000n); list(0x76000n, 0x86000n, 3); vector(0x86000n, [1, -2, 3], 3, 4);
    ptr(statics + 0x38n, 0x77000n); list(0x77000n, 0x87000n, 2); vector(0x87000n, [0n, wide ? 0xffffffffffffffffn : 0xffffffffn]);
    ptr(statics + 0x40n, 0x79000n); list(0x79000n, 0x88000n, 2); vector(0x88000n, [0x0000000200000001n, 0x0000000400000003n], 2, 8);
    const limit = (1n << BigInt(width)) - 1n, reads = [], read = fixture.process.read;
    let armed = false;
    fixture.process.read = request => {
        const address = BigInt.asUintN(64, request.address);
        assert(address + BigInt(Math.max(request.length, 1) - 1) <= limit, `${mono}/${width}/${mode}: overflowing host read`);
        reads.push({address, length: request.length});
        if (armed && address === rowsArray + BigInt(4 * bytes)) {
            armed = false;
            if (mode === 'torn size') number(rows + BigInt(3 * bytes), 4, 0);
            if (mode === 'torn backing') ptr(rows + BigInt(2 * bytes), 0x8a000n);
        }
        return read({...request, address});
    };
    const host = await SplitScriptHost.instantiate(wasm);
    number(0x6f000n, 4, 0);
    host.addProcess('game.exe', fixture.process); host.start();
    const label = `${backend}/${width}/${mode}`;
    host.updateUntil(() => host.variables.has('rows'), label);
    host.update(2);
    const normalize = text => text.replace(/\s/g, '');
    const before = Object.fromEntries(['rows', 'vectors', 'nested', 'tree'].map(key => [key, normalize(host.variables.get(key))]));
    assert.equal(before.rows, '[[Some("seed",),None,Some("tail",),],]', label);
    assert.equal(before.vectors, '[Some(["seed","tail",],),None,]', label);
    assert.equal(before.nested, '[["seed","tail",],[],]', label);
    assert(before.tree.includes('children:[Some(ProfileProbe{children:[]'), label);
    assert.equal(normalize(host.variables.get('numbers')), '[1,-2,3,]', label);
    assert.equal(normalize(host.variables.get('records')), '[Record{pair:[1,2,],},Record{pair:[3,4,],},]', label);
    assert.equal(normalize(host.variables.get('pointers')), `[0,${wide ? '18446744073709551615' : '4294967295'},]`, label);
    reads.length = 0;
    let success = false;
    if (mode.startsWith('torn')) { text(string, 'new'); vector(0x8a000n, [row]); armed = true; }
    if (mode === 'seed') success = true;
    if (mode === 'mutate') { text(string, 'new'); success = true; }
    if (mode === 'empty') { number(rows + BigInt(3 * bytes), 4, 0); success = true; }
    if (mode === 'large spare capacity') { ptr(rowsArray + BigInt(3 * bytes), 65536); success = true; }
    if (mode === 'null empty backing') { list(rows, 0n, 0); success = true; }
    if (mode === 'negative count') number(rows + BigInt(3 * bytes), 4, -1);
    if (mode === 'count exceeds capacity') number(rows + BigInt(3 * bytes), 4, 3);
    if (mode === 'element budget') { number(rows + BigInt(3 * bytes), 4, 16385); ptr(rowsArray + BigInt(3 * bytes), 16385); }
    if (mode === 'null backing') ptr(rows + BigInt(2 * bytes), 0);
    if (mode === 'null list') ptr(statics + 0x10n, 0);
    if (mode === 'unreadable size') memory.delete(rows + BigInt(3 * bytes));
    if (mode === 'unreadable backing slot') memory.delete(rows + BigInt(2 * bytes));
    if (mode === 'unreadable live slot') memory.delete(rowsArray + BigInt(4 * bytes));
    if (mode === 'unreadable string') memory.delete(string + BigInt(2 * bytes + 4));
    if (mode === 'indexed string') { vector(row, [...Array(12).fill(0n), string]); memory.delete(string + BigInt(2 * bytes + 4)); }
    if (mode === 'null nested list') ptr(nestedArray + BigInt(4 * bytes), 0);
    if (mode === 'class cycle') ptr(childrenArray + BigInt(4 * bytes), root);
    if (mode === 'shared element budget') {
        list(rows, rowsArray, 2); vector(rowsArray, [0x100000n, 0x200000n]);
        vector(0x100000n, Array(8192).fill(0n)); vector(0x200000n, Array(8192).fill(0n));
    }
    if (mode === 'capacity overflow') ptr(rowsArray + BigInt(3 * bytes), limit);
    if (mode === 'list field overflow') { ptr(statics + 0x10n, limit - BigInt(bytes)); ptr(limit - BigInt(bytes), mono ? vtable : listClass); }
    if (mode === 'backing bounds') ptr(rowsArray + BigInt(2 * bytes), 0xdeadn);
    if (mode.startsWith('depth')) {
        const count = mode === 'depth boundary' ? 20 : 21;
        vector(childrenArray, [0x100000n]); list(children, childrenArray, 1);
        for (let i = 0; i < count; i++) {
            const at = 0x100000n + BigInt(i * 0x1000), kids = at + 0x400n, array = at + 0x800n;
            node(at, i + 1 === count ? empty : kids, 2);
            list(kids, array, 1); vector(array, [at + 0x1000n]);
        }
        success = mode === 'depth boundary';
    }
    if (mode.startsWith('object')) {
        const count = mode === 'object boundary' ? 254 : 255;
        list(children, childrenArray, count); vector(childrenArray, Array(count).fill(child));
        success = mode === 'object boundary';
    }
    if (mode === 'local arrays mutable') { number(0x6f000n, 4, 4); success = true; }
    if (mode === 'cached layout') {
        memory.delete(0x40000n); memory.delete(0x51000n); success = true;
    }
    if (mode === 'reattach') success = true;
    if (mode === 'runtime class replacement' || mode === 'retry discovery' || mode.startsWith('replacement ')) {
        const replacement = 0x36000n;
        for (let i = 0n; i < 0x200n; i++) memory.set(replacement + i, memory.get(listClass + i) ?? 0);
        fields(replacement, 0x52000n, [['_items', 0x40], ['_size', 0x48]]);
        ptr(0x39100n, replacement); ptr(rows, mono ? 0x39100n : replacement);
        ptr(rows + 0x40n, rowsArray); number(rows + 0x48n, 4, 0);
        success = mode === 'runtime class replacement';
        if (mode === 'retry discovery') memory.delete(0x52000n + BigInt(wide ? 0x18 : 0xc));
        if (mode.startsWith('replacement ')) {
            const slot = mode === 'replacement items type' ? 0 : 1;
            const field = 0x52000n + BigInt(slot * (wide ? 32 : mono ? 16 : 20));
            ptr(field + BigInt(mono ? 0 : bytes), 0x55200n);
            number(0x55200n + BigInt(bytes + 2), 1, slot === 0 ? 0x12 : 0x09);
        }
    }
    if (mode.startsWith('freeze')) {
        number(0x6f000n, 4, mode === 'freeze inline' ? 5 : ['freeze outer', 'freeze nested', 'freeze snapshot'].indexOf(mode) + 1);
        assert.throws(() => host.update(), WebAssembly.RuntimeError, label);
        cases++;
        continue;
    }
    host.update();
    assert.equal(normalize(host.variables.get('old')), before.rows, `${label}: old snapshot`);
    for (const key of ['rows', 'vectors', 'nested', 'tree']) {
        if (key === 'tree' && ['depth boundary', 'object boundary'].includes(mode)) {
            assert.notEqual(normalize(host.variables.get(key)), before[key], label);
            continue;
        }
        let expected = before[key];
        if (mode === 'mutate' || (mode.startsWith('torn') && key !== 'rows')) {
            // Each state field is an independent materialization root.
            expected = expected.replaceAll('seed', 'new');
        }
        if (key === 'rows' && ['empty', 'null empty backing', 'runtime class replacement'].includes(mode)) expected = '[]';
        assert.equal(normalize(host.variables.get(key)), expected, `${label}: ${key}`);
    }
    const directSuccess = success || ['null nested list', 'class cycle', 'depth overflow', 'object overflow', 'torn size', 'torn backing'].includes(mode);
    assert.equal(host.variables.get('result') === 'ok', directSuccess, `${label}: ${host.variables.get('result')}`);
    if (mode === 'indexed string') assert.match(host.variables.get('result'), /^\[0\]: \[12\]: managed string/, label);
    if (mode === 'class cycle') assert.match(host.variables.get('treeResult'), /^children: \[0\]: /, label);
    if (mode === 'unreadable string') assert.match(host.variables.get('result'), /^\[0\]: \[0\]: managed string/, label);
    if (!directSuccess) assert(host.variables.get('result').length > 0, `${label}: error payload was lost`);
    if (mode === 'local arrays mutable') assert.equal(host.variables.get('local'), '2', label);
    if (mode === 'large spare capacity' || mode === 'seed') {
        assert(!reads.some(r => r.address >= rowsArray + BigInt(5 * bytes) && r.address < row), `${label}: spare capacity was read`);
    }
    if (mode === 'retry discovery') {
        number(0x52000n + BigInt(wide ? 0x18 : 0xc), 4, 0x40);
        host.updateUntil(() => host.variables.get('result') === 'ok' && normalize(host.variables.get('rows')) === '[]', label);
    }
    if (mode.startsWith('replacement ')) {
        assert.match(host.variables.get('result'), /invalid backing or count field types/, label);
        number(0x55200n + BigInt(bytes + 2), 1, mode === 'replacement items type' ? 0x1d : 0x08);
        host.updateUntil(() => host.variables.get('result') === 'ok' && normalize(host.variables.get('rows')) === '[]', label);
    }
    if (mode === 'reattach') {
        host.setProcessOpen('game.exe', false); host.update(3);
        fields(listClass, 0x51000n, [['_items', 0x40], ['_size', 0x48]]);
        for (const [object, array, count] of [[rows, rowsArray, 0], [innerList, strings, 2],
            [nested, nestedArray, 2], [empty, 0n, 0], [children, childrenArray, 2],
            [0x76000n, 0x86000n, 3], [0x77000n, 0x87000n, 2], [0x79000n, 0x88000n, 2]]) {
            ptr(object + 0x40n, array); number(object + 0x48n, 4, count);
        }
        host.setProcessOpen('game.exe', true);
        host.updateUntil(() => normalize(host.variables.get('rows')) === '[]', `${label}: fresh layout cache`);
    }
    cases++;
}
console.log(JSON.stringify({managedListCases: cases}));
