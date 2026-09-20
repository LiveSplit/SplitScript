import assert from 'node:assert/strict';
import { SplitScriptHost } from './support/splitscript_host.mjs';
import { createKeyedCollectionFixture } from './support/keyed_collection_fixture.mjs';
import { writeGenericType, writeManagedClassType, writeVectorElementType } from './support/managed_type_fixture.mjs';

const [wasm, backend] = process.argv.slice(2);
const mono = backend === 'mono';
const publicProof = process.argv[4] === '--public';
// Independent ABI facts; the fixture factory supplies module identities only.
const layouts = {
    V1Cattrs: {32: [0x24, 0x68], 64: [0x30, 0x9c]},
    V2: {32: [0x20, 0xa4], 64: [0x30, 0x100]},
    V3: {32: [0x20, 0x9c], 64: [0x30, 0x100]},
    il2cpp: {32: [0x2c, 0xac], 64: [0x58, 0x124]},
};
const modes = ['ordinary', 'generic', 'wrong family', 'wrong namespace', 'parent cycle',
    'wrong leaf', 'unreadable leaf', 'wrong width', 'null type', 'null class',
    'null generic cache', 'unreadable generic cache', 'generic value', 'budget', 'wrong class'];
let cases = 0;
for (const family of mono ? ['V1Cattrs', 'V2', 'V3'] : ['il2cpp']) for (const width of [32, 64])
for (const shape of (publicProof ? [0, 1, 2, 3, 4, 5] : [0, 1, 2, 3, 4])) for (const parallel of [false, true]) for (const mode of modes) {
    if (parallel && (shape === 0 || shape === 3 || shape === 5)) continue;
    if (mode === 'wrong class' && shape !== 5) continue;
    if (publicProof && ['wrong width', 'budget'].includes(mode)) continue;
    const f = createKeyedCollectionFixture({family, width, dictionary: shape !== 2, parallel});
    const {memory, ptr, number, bytes} = f;
    const options = {mono, width, family, ptr, number, fixture: f};
    const text = (at, value) => {
        const data = new Uint8Array(256); data.set(new TextEncoder().encode(value));
        data.forEach((byte, i) => memory.set(at + BigInt(i), byte));
    };
    const asList = target => {
        const {base} = target;
        text(base + 0x40200n, 'List`1');
        const inflated = mono && family !== 'V1Cattrs';
        number((inflated ? base + 0x32000n : target.owner) + BigInt(layouts[family][width][1]), mono ? 4 : 2, 2);
        const stride = width === 64 ? 32 : mono ? 16 : 20;
        const offset = width === 64 ? 0x18 : 0xc;
        for (const [i, name] of ['_items', '_size'].entries()) {
            text(base + 0x44000n + BigInt(i * 256), name);
            ptr(base + 0x37000n + BigInt(i * stride + (mono ? 0 : bytes)), base + (i ? 0x50200n : 0x50000n));
            number(base + 0x37000n + BigInt(i * stride + offset), 4, (2 + i) * bytes);
        }
        return writeVectorElementType(options, base + 0x50000n, base + 0x3a000n, 0x0e);
    };
    let leaf = f.valueType;
    if (shape === 0 || shape === 3 || shape === 5) leaf = asList(f);
    if (shape === 5) writeManagedClassType(f, options, leaf, 0x14000n);
    if (shape === 4) {
        const child = createKeyedCollectionFixture({family, width, base: 0x100000n});
        for (const [address, value] of child.memory) if (address >= 0x130000n) memory.set(address, value);
        leaf = asList(child);
        writeManagedClassType(f, options, f.valueType, child.root);
    }
    const rootType = 0x5d000n;
    writeManagedClassType(f, options, rootType, f.root);
    // Store vector element metadata at the root class's by-value type. This
    // tests array<List<String>> and array<Map<String,List<String>>> as graphs.
    let contractType = rootType;
    if (shape === 3 || shape === 4) {
        contractType = 0x5e000n;
        writeVectorElementType({...options, referenceClass:f.root}, contractType, f.root, 0x12);
    }
    let resolvedType = (shape === 3 || shape === 4)
        ? (mono ? f.root + BigInt(({V1Cattrs:{32:0x88,64:0xd0},V2:{32:0x70,64:0xb8},V3:{32:0x70,64:0xb8}})[family][width]) : f.root)
        : rootType;
    number(0x60000n, 8, contractType); number(0x60008n, 1, shape);
    number(0x60010n, 4, bytes); number(0x60014n, 4, 4096);
    // No live collection, backing vector, or child object exists in memory.
    const forbidden = [f.object, f.vtable, 0x170000n, 0x171000n];
    for (const at of forbidden) for (let i = 0n; i < 64n; i++) memory.delete(at + i);
    let publicArrayType;
    if (publicProof) {
        const fieldsOffset = mono ? ({V1Cattrs:{32:0x78,64:0xb0},V2:{32:0x60,64:0x98},V3:{32:0x60,64:0x98}})[family][width] : width === 64 ? 0x80 : 0x40;
        const byval = mono ? fieldsOffset + 4 * bytes : 4 * bytes;
        // The outer array is empty; only its declared type graph can establish
        // the recursively nested collection contract.
        const proxy = 0xe0000n, array = 0xc0000n, arrayClass = 0xc1000n, table = 0xc2000n;
        if (mono) {
            const proxyType = proxy + BigInt(byval);
            for (let i = 0n; i < BigInt(2 * bytes); i++) memory.set(proxyType + i, memory.get(contractType + i) ?? 0);
            contractType = proxyType;
            if (shape !== 3 && shape !== 4) resolvedType = proxyType;
        }
        publicArrayType = arrayClass + BigInt(byval);
        number(publicArrayType + BigInt(bytes + 2), 1, 0x1d);
        ptr(publicArrayType, mono ? proxy : contractType);
        ptr(array, mono ? table : arrayClass); ptr(table, arrayClass);
        ptr(array + BigInt(2 * bytes), 0); ptr(array + BigInt(3 * bytes), 0);
        ptr(0x14000n + BigInt(fieldsOffset), 0x58000n);
        number(0x14000n + BigInt(layouts[family][width][1]), mono ? 4 : 2, 6);
        ['lists','maps','sets','vectors','nested','classes'].forEach((name, i) => {
            const field = 0x58000n + BigInt(i * (width === 64 ? 32 : mono ? 16 : 20));
            const label = 0x59000n + BigInt(i * 256);
            ptr(field + BigInt(mono ? bytes : 0), label); text(label, name);
            number(field + BigInt(width === 64 ? 0x18 : 0xc), 4, 0x10 + i * bytes);
            ptr((mono ? 0x18000n : 0x16000n) + BigInt(0x10 + i * bytes), array);
        });
    }
    const pristine = new Map(memory);
    if (mode === 'generic' || mode.includes('generic')) {
        const generic = writeGenericType({...options, cachedClass:f.root}, resolvedType, mode === 'generic value');
        if (mode === 'null generic cache') ptr(generic.cached, 0);
        if (mode === 'unreadable generic cache') memory.delete(generic.cached);
    }
    if (mode === 'wrong family') text(0x40200n, 'Other`1');
    if (mode === 'wrong namespace') text(0x40300n, 'Game');
    if (mode === 'parent cycle') {
        text(0x40200n, 'Other`1'); ptr(f.owner + BigInt(layouts[family][width][0]), f.root);
    }
    if (mode === 'wrong class') writeManagedClassType(f, options, leaf, f.owner);
    if (mode === 'wrong leaf') number(leaf + BigInt(bytes + 2), 1, 0x08);
    if (mode === 'unreadable leaf') memory.delete(leaf + BigInt(bytes + 2));
    if (mode === 'wrong width') number(0x60010n, 4, bytes === 4 ? 8 : 4);
    if (mode === 'null type') { number(0x60000n, 8, 0); if (publicProof) ptr(publicArrayType, 0); }
    if (mode === 'null class') ptr(resolvedType, 0);
    if (mode === 'budget') number(0x60014n, 4, 1);
    const read = f.process.read;
    f.process.read = request => {
        const address = BigInt.asUintN(64, request.address), end = address + BigInt(request.length);
        assert(end <= 1n << BigInt(width), 'overflowing host read');
        assert(!forbidden.some(at => address < at + 64n && end > at), 'metadata proof read a live object');
        return read({...request, address});
    };
    const label = `${family}/${width}/shape${shape}/${parallel ? 'parallel' : 'entries'}/${mode}`;
    const host = await SplitScriptHost.instantiate(wasm);
    host.addProcess('game.exe', f.process); host.start();
    host.updateUntil(() => host.variables.has('result'), label);
    assert.equal(host.variables.get('result') === 'ok', ['ordinary', 'generic'].includes(mode), `${label}: ${host.variables.get('result')}`);
    if (!publicProof) assert(Number(host.variables.get('work')) <= (mode === 'budget' ? 2 : 4097), `${label}: unbounded work`);
    if (!['ordinary', 'generic'].includes(mode)) {
        memory.clear(); for (const [address, value] of pristine) memory.set(address, value);
        host.variables.delete('result'); host.updateUntil(() => host.variables.has('result'), `${label}/repair`);
        assert.equal(host.variables.get('result'), 'ok', `${label}/repair`);
    }
    if (publicProof && mode === 'ordinary') {
        // A successful proof is specific to its source schema, even when the
        // containing array layout was already cached on this attachment.
        number(0x60008n, 1, shape === 2 ? 1 : 2);
        host.variables.delete('result'); host.updateUntil(() => host.variables.has('result'), `${label}/other schema`);
        assert.notEqual(host.variables.get('result'), 'ok', `${label}: proof leaked across schemas`);
        number(0x60008n, 1, shape);
        const kindAddress = leaf + BigInt(bytes + 2), savedKind = memory.get(kindAddress);
        memory.delete(kindAddress);
        host.variables.delete('result'); host.updateUntil(() => host.variables.has('result'), `${label}/cached proof`);
        assert.equal(host.variables.get('result'), 'ok', `${label}: successful proof was not cached`);
        host.setProcessOpen('game.exe', false); host.update(2);
        host.addProcess('game.exe', f.process); host.variables.delete('result');
        host.updateUntil(() => host.variables.has('result'), `${label}/reattach`);
        assert.notEqual(host.variables.get('result'), 'ok', `${label}: proof survived reattachment`);
        memory.set(kindAddress, savedKind);
        host.variables.delete('result'); host.updateUntil(() => host.variables.has('result'), `${label}/reattach repair`);
        assert.equal(host.variables.get('result'), 'ok', `${label}: failed proof prevented repair`);
    }
    if (publicProof) assert.equal(host.variables.get('length'), '0', `${label}: repaired empty root`);
    cases++;
}
console.log(`${backend}: ${cases} ${publicProof ? 'public empty-root' : 'metadata-only'} nested collection contracts and repairs passed`);
