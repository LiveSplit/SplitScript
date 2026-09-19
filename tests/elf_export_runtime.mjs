import assert from 'node:assert/strict';
import { SplitScriptHost } from './support/splitscript_host.mjs';

const [wasm] = process.argv.slice(2);
const target = 'mono_assembly_foreach';
let cases = 0;
for (const gnu of [false, true]) for (const origin of [0n, 0x400000n]) for (const relocated of [false, true]) {
    const modes = ['valid', 'weak', 'protected', 'absolute', 'fixed executable', 'large headers', 'large dynamic', 'large chain',
        'magic', 'elf32', 'endian', 'ident version', 'image version', 'image type', 'header size',
        'entry size', 'header count', 'table overflow', 'missing load', 'missing dynamic', 'duplicate dynamic',
        'dynamic size', 'dynamic limit', 'dynamic overflow', 'dynamic terminator', 'missing strings',
        'symbol stride', 'conflicting tag', 'missing hash', 'hash count', 'hash limit', 'hash overflow',
        'string span', 'symbol span', 'name offset', 'unterminated name', 'undefined', 'hidden', 'local', 'ifunc',
        'reserved section', 'symbol outside', 'unreadable', 'empty bucket', 'bad chain', 'cycle', 'cancel'];
    for (const mode of modes) {
        if (gnu && ['cycle', 'cancel'].includes(mode)) continue;
        const bytes = new Uint8Array(0x20000), view = new DataView(bytes.buffer);
        const u16 = (at, value) => view.setUint16(at, value, true);
        const u32 = (at, value) => view.setUint32(at, value, true);
        const u64 = (at, value) => view.setBigUint64(at, BigInt(value), true);
        const base = mode === 'fixed executable' ? (origin || 0x400000n) : 0x100001000n;
        const preferred = mode === 'fixed executable' ? base : origin;
        const pointer = offset => (relocated ? base : preferred) + BigInt(offset);
        bytes.set([0x7f, 0x45, 0x4c, 0x46, 2, 1, 1]);
        u16(16, preferred === 0n ? 3 : 2); u16(18, 62); u32(20, 1);
        u64(32, 0x80); u16(52, 64); u16(54, 56); u16(56, mode === 'large headers' ? 192 : 2);
        const segment = (at, type, offset, size) => {
            u32(at, type); u64(at + 8, type === 1 ? 0 : 0x1800);
            u64(at + 16, preferred + BigInt(offset)); u64(at + 32, size); u64(at + 40, size);
        };
        segment(0x80, 1, 0, bytes.length);
        const tags = [[gnu ? 0x6ffffef5 : 4, pointer(0x6000)], [5, pointer(0x9000)],
            [6, pointer(0x10000)], [10, 0x100], [11, 24]];
        if (mode === 'conflicting tag') tags.push([11, 8]);
        const padding = mode === 'large dynamic' ? 192 : 0;
        for (let i = 0; i < padding; i++) { u64(0x4000 + i * 16, 0x70000001); }
        tags.forEach(([tag, value], i) => { u64(0x4000 + (i + padding) * 16, tag); u64(0x4008 + (i + padding) * 16, value); });
        segment(0xb8, 2, 0x4000, (tags.length + padding + 1) * 16);
        bytes.set(new TextEncoder().encode('other\0'), 0x9001);
        bytes.set(new TextEncoder().encode(target + '\0'), 0x9020);
        const n = mode === 'large chain' ? 192 : 1;
        let hash = 5381;
        for (const byte of new TextEncoder().encode(target)) hash = (Math.imul(hash, 33) + byte) >>> 0;
        let chains;
        if (gnu) {
            u32(0x6000, 1); u32(0x6004, 1); u32(0x6008, 1); u32(0x600c, 5);
            u64(0x6010, 0xffffffffffffffffn); u32(0x6018, 1); chains = 0x601c;
            for (let i = 0; i < n; i++) u32(chains + i * 4, (hash & 0xfffffffe) | (i === n - 1 ? 1 : 0));
        } else {
            u32(0x6000, 1); u32(0x6004, n + 1); u32(0x6008, 1); chains = 0x600c;
            for (let i = 1; i <= n; i++) u32(chains + i * 4, i === n ? 0 : i + 1);
        }
        for (let i = 1; i <= n; i++) {
            const at = 0x10000 + i * 24;
            u32(at, i === n ? 0x20 : 1); bytes[at + 4] = 0x12; u16(at + 6, 1);
            u64(at + 8, preferred + 0x18000n); u64(at + 16, 8);
        }
        const symbol = 0x10000 + n * 24;
        if (mode === 'weak') bytes[symbol + 4] = 0x22;
        if (mode === 'protected') bytes[symbol + 5] = 3;
        if (mode === 'absolute') { u16(symbol + 6, 0xfff1); u64(symbol + 8, base + 0x18000n); }
        if (mode === 'magic') bytes[0] = 0;
        if (mode === 'elf32') bytes[4] = 1;
        if (mode === 'endian') bytes[5] = 2;
        if (mode === 'ident version') bytes[6] = 2;
        if (mode === 'image version') u32(20, 0);
        if (mode === 'image type') u16(16, 1);
        if (mode === 'header size') u16(52, 48);
        if (mode === 'entry size') u16(54, 48);
        if (mode === 'header count') u16(56, 1025);
        if (mode === 'table overflow') u64(32, 0xffffffffffffffffn);
        if (mode === 'missing load') u32(0x80, 0);
        if (mode === 'missing dynamic') u32(0xb8, 0);
        if (mode === 'duplicate dynamic') { u16(56, 3); segment(0xf0, 2, 0x4000, 96); }
        if (mode === 'dynamic size') u64(0xb8 + 32, 95);
        if (mode === 'dynamic limit') u64(0xb8 + 32, 65552);
        if (mode === 'dynamic overflow') u64(0xb8 + 16, 0xfffffffffffffff0n);
        if (mode === 'dynamic terminator') u64(0x4050, 1);
        if (mode === 'missing strings') u64(0x4018, 0);
        if (mode === 'symbol stride') u64(0x4048, 16);
        if (mode === 'missing hash') u64(0x4000, 42);
        if (mode === 'hash count') u32(0x6000, 0);
        if (mode === 'hash limit') u32(0x6000, 1048577);
        if (mode === 'hash overflow') u64(0x4008, 0xffffffffffffffffn);
        if (mode === 'string span') u64(0x4038, 0xffffffffffffffffn);
        if (mode === 'symbol span') u64(0x4028, pointer(bytes.length - 24));
        if (mode === 'name offset') u32(symbol, 0x100);
        if (mode === 'unterminated name') u64(0x4038, 0x20 + target.length);
        if (mode === 'undefined') u16(symbol + 6, 0);
        if (mode === 'hidden') bytes[symbol + 5] = 2;
        if (mode === 'local') bytes[symbol + 4] = 2;
        if (mode === 'ifunc') bytes[symbol + 4] = 0x1a;
        if (mode === 'reserved section') u16(symbol + 6, 0xffff);
        if (mode === 'symbol outside') u64(symbol + 8, preferred + BigInt(bytes.length));
        if (mode === 'empty bucket') u32(gnu ? 0x6018 : 0x6008, 0);
        if (mode === 'bad chain') u32(gnu ? 0x6018 : 0x6008, 1048576);
        if (mode === 'cycle' || mode === 'cancel') { u32(symbol, 1); u32(chains + 4, 1); }
        let reads = 0, maxReads = 0;
        const host = await SplitScriptHost.instantiate(wasm, {operatingSystem: 'linux'});
        const label = `${gnu ? 'gnu' : 'sysv'}/${origin}/${relocated}/${mode}`;
        host.addProcess('game.exe', {
            modules: {runtime: {address: base, size: BigInt(bytes.length)}},
            read({address, length, outputPointer, host}) {
                const offset = address - base;
                assert(offset >= 0n && offset + BigInt(length) <= BigInt(bytes.length), `${label}: out-of-mapping read`);
                assert(++reads < 100000, `${label}: excessive work`);
                if (mode === 'unreadable' && offset === 0x10018n) return false;
                host.bytes(outputPointer, length).set(bytes.subarray(Number(offset), Number(offset) + length));
                return true;
            },
        });
        host.start();
        let ticks = 0;
        while (!host.variables.has('address') && !host.variables.has('error') && ticks < 400) {
            const before = reads; host.update(); maxReads = Math.max(maxReads, reads - before); ticks++;
            if (mode === 'cancel' && ticks === 2) break;
        }
        assert(maxReads < 800, `${label}: ${maxReads} reads in one update`);
        if (mode === 'cancel') {
            assert.equal(host.variables.size, 0, label);
            host.setProcessOpen('game.exe', false); host.update(3);
            const before = reads; host.update(3); assert.equal(reads, before, label);
            u32(symbol, 0x20); u32(chains + 4, 0);
            host.setProcessOpen('game.exe', true);
            host.updateUntil(() => host.variables.has('address'), label);
        }
        const success = ['valid', 'weak', 'protected', 'absolute', 'fixed executable', 'large headers', 'large dynamic', 'large chain', 'cancel'].includes(mode);
        if (success) {
            assert.equal(BigInt(host.variables.get('address')), base + 0x18000n, label);
            if (mode.startsWith('large')) assert(ticks >= 3, `${label}: no cooperative yield`);
        } else assert(host.variables.get('error')?.length > 0, `${label}: missing failure`);
        cases++;
    }
}
console.log(JSON.stringify({elfExportCases: cases}));
