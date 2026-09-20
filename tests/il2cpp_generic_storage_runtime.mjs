import assert from 'node:assert/strict';
import {readFile} from 'node:fs/promises';
import {SplitScriptHost} from './support/splitscript_host.mjs';
import {createIl2cppPeFixture} from './support/il2cpp_pe_fixture.mjs';

const [wasm] = process.argv.slice(2);
const profiles = JSON.parse(await readFile(new URL('./fixtures/il2cpp-pe-profiles.json', import.meta.url))).builds;
// Independent offsets from the audited source and PDB layouts, x86 then x64.
const layouts = {
    '2018.4.36': [[0xbe, 2, 0x84], [0x126, 2, 0xec]],
    '2019.4.41': [[0xba, 2, 0x80], [0x12e, 2, 0xf4]],
    '2020.1.18': [[0xba, 2, 0x80], [0x12e, 2, 0xf4]],
    '2021.3.11': [[0x17, 128, 0x80], [0x2b, 128, 0xf8]],
    '2022.3.0': [[0x17, 128, 0x80], [0x2b, 128, 0xf8]],
    '2023.1.0': [[0x17, 128, 0x80], [0x2b, 128, 0xf8]],
    '2023.1.22': [[0x17, 128, 0x80], [0x2b, 128, 0xf8]],
    '6000.2.12': [[0x17, 128, 0x80], [0x2b, 128, 0xf8]],
    '6000.3.21': [[0x17, 128, 0x80], [0x2b, 128, 0xf8]],
    '6000.5.10': [[0x17, 128, 0x80], [0x2b, 128, 0xf8]],
    '6000.7.0': [[0x17, 128, 0x80], [0x2b, 128, 0xf0]],
};
let cases = 0;
for (const [choice, profile] of profiles.entries()) {
    const layout = layouts[profile.version.slice(0, 3).join('.')]?.[profile.width === 64 ? 1 : 0];
    assert(layout, `Missing independent layout for ${profile.name}`);
    for (const mode of ['missing fact', 'invalid fact', 'reference', 'value', 'unrelated flags', 'unreadable flags', 'null descriptor', 'null class', 'unreadable size', 'zero size', 'oversized', 'work budget', 'reference budget', 'descriptor overflow', 'class overflow']) {
        const fixture = createIl2cppPeFixture(profile), {memory} = fixture;
        const bytes = profile.width / 8, limit = (1n << BigInt(profile.width)) - 1n;
        const number = (at, count, value) => {
            let n = BigInt.asUintN(count * 8, BigInt(value));
            for (let i = 0; i < count; i++, n >>= 8n) memory.set(at + BigInt(i), Number(n & 255n));
        };
        const type = 0x70000n, generic = 0x71000n, klass = 0x72000n;
        const cached = generic + BigInt((profile.version[0] === 6000 && profile.version[1] >= 7 ? 2 : 3) * bytes);
        number(0x60000n, 4, choice); number(0x60008n, 8, type);
        const seed = () => {
            number(0x60004n, 4, 4); number(0x60010n, 1, 0);
            number(type, bytes, generic); number(cached, bytes, klass);
            if (layout) {
                number(klass + BigInt(layout[0]), 1, layout[1]);
                number(klass + BigInt(layout[2]), 4, 2 * bytes + 24);
            }
        };
        seed();
        if (layout) {
            const [flag, mask, size] = layout;
            if (mode === 'missing fact' || mode === 'invalid fact') number(0x60010n, 1, mode === 'missing fact' ? 1 : 2);
            if (mode === 'reference' || mode === 'reference budget' || mode === 'unrelated flags') {
                number(klass + BigInt(flag), 1, mode === 'unrelated flags' ? 255 ^ mask : 0);
                memory.delete(klass + BigInt(size)); // Reference storage needs no instance size.
            }
            if (mode === 'unreadable flags') memory.delete(klass + BigInt(flag));
            if (mode === 'unreadable size') memory.delete(klass + BigInt(size));
            if (mode === 'zero size') number(klass + BigInt(size), 4, 2 * bytes);
            if (mode === 'oversized') number(klass + BigInt(size), 4, 2 * bytes + 1025);
            if (mode === 'null descriptor') number(type, bytes, 0);
            if (mode === 'null class') number(cached, bytes, 0);
            if (mode === 'descriptor overflow') number(type, bytes, limit - 1n);
            if (mode === 'class overflow') number(cached, bytes, limit - 1n);
            if (mode === 'work budget' || mode === 'reference budget') number(0x60004n, 4, 3);
        }
        const original = fixture.process.read;
        let metadataReads = 0;
        fixture.process.read = request => {
            const address = BigInt.asUintN(64, request.address);
            assert(address + BigInt(request.length - 1) <= limit, 'overflowing host read');
            if (address >= type && address < klass + 0x1000n) metadataReads++;
            return original({...request, address});
        };
        const host = await SplitScriptHost.instantiate(wasm);
        host.addProcess('game.exe', fixture.process); host.start();
        const label = `${profile.name}: ${mode}`;
        host.updateUntil(() => host.variables.has('result'), label);
        const success = ['reference', 'value', 'unrelated flags', 'reference budget'].includes(mode);
        assert.equal(host.variables.get('result') === 'ok', success, `${label}: ${host.variables.get('result')}`);
        if (success) assert.equal(host.variables.get('size'), mode === 'value' ? '24' : '0', label);
        else {
            if (mode === 'missing fact' || mode === 'invalid fact') {
                assert.match(host.variables.get('result'), mode === 'missing fact' ? /lacks value type metadata/ : /metadata offset is invalid/, label);
                assert.equal(metadataReads, 0, label);
            }
            if (mode === 'work budget') {
                assert.match(host.variables.get('result'), /work limit exceeded/, label);
                assert.equal(metadataReads, 3, label);
            }
            seed(); host.variables.delete('result');
            host.updateUntil(() => host.variables.get('result') === 'ok', `${label}: repair`);
            assert.equal(host.variables.get('size'), '24', label);
        }
        cases++;
    }
}
console.log(JSON.stringify({il2cppGenericStorageCases: cases}));
