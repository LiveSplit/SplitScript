import assert from 'node:assert/strict';
import {readFile} from 'node:fs/promises';
import {SplitScriptHost} from './support/splitscript_host.mjs';
import {createIl2cppPeFixture} from './support/il2cpp_pe_fixture.mjs';

const [wasm] = process.argv.slice(2);
const profiles = JSON.parse(await readFile(new URL('./fixtures/il2cpp-pe-profiles.json', import.meta.url))).builds;
let cases = 0;
for (const [choice, profile] of profiles.entries()) {
    for (const mode of ['value', 'reference', 'sparse distant', 'another image', 'primitive anchor', 'uninitialized',
        'wrong kind', 'wrong definition', 'unreadable definition', 'work budget', 'empty assemblies',
        'malformed vector', 'negative image start', 'oversized image', 'null table',
        'out of range', 'null input', 'class overflow', 'type overflow', 'unaligned handle',
        'negative index', 'index zero']) {
        const f = createIl2cppPeFixture(profile), {memory} = f;
        const bytes = profile.width / 8, p = BigInt(bytes), handles = profile.version[0] >= 2021;
        if (handles && mode === 'negative index' || !handles && mode === 'unaligned handle') continue;
        const limit = (1n << BigInt(profile.width)) - 1n;
        const number = (at, size, value) => {
            let v = BigInt.asUintN(size * 8, BigInt(value));
            for (let i = 0; i < size; i++, v >>= 8n) memory.set(at + BigInt(i), Number(v & 255n));
        };
        const write = (at, size, value) => {
            if (at >= f.base && at < f.base + BigInt(f.image.length)) {
                let v = BigInt(value);
                for (let i = 0; i < size; i++, v >>= 8n) f.image[Number(at - f.base) + i] = Number(v & 255n);
            } else number(at, size, value);
        };
        const ptr = (at, value) => write(at, bytes, value);
        const type = 0x70000n, targetClass = 0x72000n, secondClass = 0x74000n;
        const definition = index => 0x90000n + BigInt(index) * 88n;
        // Deliberately nonzero padding proves old indices use only the low i32.
        const data = index => handles ? definition(index) : BigInt(index) | (bytes === 8 ? 0xabcdef1200000000n : 0n);
        const seedClass = (klass, index, kind) => {
            ptr(klass + 4n * p, data(index));
            number(klass + 5n * p + 2n, 1, kind);
            ptr(klass + 13n * p, definition(index));
        };
        const target = mode === 'sparse distant' ? 10000 : mode === 'index zero' ? 0 : 10;
        const start = mode === 'index zero' ? 0 : 7;
        const countAt = 0x12000n + BigInt(handles ? bytes === 8 ? 0x18 : 0xc : bytes === 8 ? 0x1c : 0x10);
        const startAt = handles ? 0x17000n : 0x12000n + BigInt(bytes === 8 ? 0x18 : 0xc);
        const seed = () => {
            number(0x60000n, 4, choice); number(0x60004n, 4, 128);
            number(0x60008n, 8, type); number(0x60018n, 8, f.assemblies); number(0x60020n, 8, f.typeInfo);
            number(countAt, 4, Math.max(target, 9) - start + 1); number(startAt, 4, start);
            ptr(f.assemblies, 0x10000n); ptr(f.assemblies + p, 0x10000n + p); ptr(f.typeInfo, 0x13000n);
            for (let i = start; i <= Math.max(target, 9); i++) ptr(0x13000n + BigInt(i) * p, 0);
            // Large sparse tables overlap fixture addresses: restore objects last.
            ptr(0x10000n, 0x11000n); ptr(0x11000n, 0x12000n);
            if (handles) ptr(0x12000n + BigInt(bytes === 8 ? 0x28 : 0x18), 0x17000n);
            number(countAt, 4, Math.max(target, 9) - start + 1); number(startAt, 4, start);
            ptr(0x13000n + 8n * p, f.klass); seedClass(f.klass, 8, 0x12);
            ptr(0x13000n + 9n * p, secondClass); seedClass(secondClass, 9, 0x12);
            ptr(0x13000n + BigInt(target) * p, targetClass); seedClass(targetClass, target, mode === 'reference' ? 0x12 : 0x11);
            ptr(type, data(target)); number(type + p + 2n, 1, mode === 'reference' ? 0x12 : 0x11);
        };
        seed();
        if (mode === 'another image') {
            ptr(f.assemblies + p, 0x10000n + 2n * p);
            ptr(0x10000n + p, 0x18000n); ptr(0x18000n, 0x19000n);
            number(countAt, 4, 3);
            number(0x19000n + (countAt - 0x12000n), 4, 1);
            if (handles) {
                ptr(0x19000n + BigInt(bytes === 8 ? 0x28 : 0x18), 0x1a000n);
                number(0x1a000n, 4, 10);
            } else number(0x19000n + (startAt - 0x12000n), 4, 10);
        }
        if (mode === 'primitive anchor') number(f.klass + 5n * p + 2n, 1, 0x08);
        if (mode === 'uninitialized') ptr(0x13000n + BigInt(target) * p, 0);
        if (mode === 'wrong kind') number(targetClass + 5n * p + 2n, 1, 0x12);
        if (mode === 'wrong definition') ptr(targetClass + 13n * p, 0);
        if (mode === 'unreadable definition') memory.delete(targetClass + 13n * p);
        if (mode === 'work budget') number(0x60004n, 4, 12);
        if (mode === 'empty assemblies') ptr(f.assemblies + p, 0x10000n);
        if (mode === 'malformed vector') ptr(f.assemblies + p, 0x10001n);
        if (mode === 'negative image start') number(startAt, 4, -1);
        if (mode === 'oversized image') number(countAt, 4, 1048577);
        if (mode === 'null table') ptr(f.typeInfo, 0);
        if (mode === 'out of range') ptr(type, data(100000));
        if (mode === 'null input') number(0x60008n, 8, 0);
        if (mode === 'class overflow') ptr(0x13000n + BigInt(target) * p, limit - 1n);
        if (mode === 'type overflow') number(0x60008n, 8, limit - 1n);
        if (mode === 'unaligned handle') ptr(type, definition(target) + 1n);
        if (mode === 'negative index') ptr(type, 0xffffffffn);
        const read = f.process.read;
        f.process.read = r => {
            const address = BigInt.asUintN(64, r.address);
            assert(address + BigInt(r.length - 1) <= limit, 'overflowing host read');
            return read({...r, address});
        };
        const host = await SplitScriptHost.instantiate(wasm);
        host.addProcess('game.exe', f.process); host.start();
        const label = `${profile.name}: ${mode}`;
        host.updateUntil(() => host.variables.has('result'), label);
        const success = ['value', 'reference', 'sparse distant', 'another image', 'primitive anchor', 'index zero'].includes(mode);
        assert.equal(host.variables.get('result') === 'ok', success, `${label}: ${host.variables.get('result')}`);
        if (success) assert.equal(host.variables.get('class'), String(targetClass), label);
        else {
            if (mode === 'work budget') assert.match(host.variables.get('result'), /work limit exceeded/, label);
            seed(); host.variables.delete('result');
            host.updateUntil(() => host.variables.get('result') === 'ok', `${label}: repair`);
            assert.equal(host.variables.get('class'), String(targetClass), label);
        }
        cases++;
    }
}
console.log(JSON.stringify({il2cppPlainStorageCases: cases}));
