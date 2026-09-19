import assert from 'node:assert/strict';
import {SplitScriptHost} from './support/splitscript_host.mjs';
const wasm = process.argv[2], target = '_mono_assembly_foreach';
let cases = 0;
for (const arm of [false, true]) for (const origin of [0n, 0x100000000n]) {
    const modes = ['valid', 'weak', 'bundle', 'executable', 'negative slide', 'pagezero',
        'large commands', 'large sections', 'large symbols', 'cancel',
        'magic', 'mach32', 'big endian', 'universal', 'filetype', 'command limit', 'table bounds',
        'command truncated', 'command alignment', 'command zero', 'command past end', 'count mismatch',
        'symtab missing', 'symtab repeated', 'symtab size', 'segment truncated', 'segment count',
        'section count', 'section table', 'section before segment', 'section past segment',
        'file larger than memory', 'virtual overflow', 'file overflow', 'high mapping',
        'header missing', 'header repeated', 'header truncated', 'ambiguous mapping',
        'symbols missing', 'symbol limit', 'symbols unmapped', 'symbols cross segment',
        'strings missing', 'strings unmapped', 'strings cross segment', 'table virtual before header',
        'table outside module', 'name outside strings', 'name zero', 'name unterminated',
        'undefined', 'local', 'private', 'indirect', 'absolute', 'debug', 'section zero', 'section missing',
        'nonexecutable segment', 'noninstruction section', 'unbacked code', 'symbol before section', 'symbol after section',
        'symbol outside module', 'unreadable symbol', 'unreadable string'];
    for (const mode of modes) {
        const bytes = new Uint8Array(0x20000), view = new DataView(bytes.buffer);
        const u16 = (at, value) => view.setUint16(at, value, true);
        const u32 = (at, value) => view.setUint32(at, value, true);
        const u64 = (at, value) => view.setBigUint64(at, BigInt(value), true);
        const base = mode === 'negative slide' ? 0x1000n : 0x700001000n;
        bytes.set([0xcf, 0xfa, 0xed, 0xfe]); u32(4, arm ? 0x100000c : 0x1000007); u32(12, 6);
        let cursor = 32, count = 0;
        const segment = (vm, file, size, nsects = 0, protection = 1) => {
            const at = cursor, commandSize = 72 + 80 * nsects;
            u32(at, 0x19); u32(at + 4, commandSize); u64(at + 24, vm);
            u64(at + 32, size); u64(at + 40, file); u64(at + 48, size);
            u32(at + 56, protection); u32(at + 60, protection); u32(at + 64, nsects);
            for (let i = 0; i < nsects; i++) {
                const section = at + 72 + 80 * i;
                u64(section + 32, vm + 0x1000n); u64(section + 40, 0x1000);
                u32(section + 48, Number(file) + 0x1000); u32(section + 64, 0x80000400);
            }
            cursor += commandSize; count++; return at;
        };
        if (mode === 'pagezero') { const at = segment(0n, 0, origin || 0x1000n); u64(at + 48, 0); u32(at + 60, 0); }
        const header = segment(origin, 0, 0x4000);
        const nsections = mode === 'large sections' ? 192 : 1;
        const code = segment(origin + 0x8000n, 0x4000, 0x4000, nsections, 5);
        const link = segment(origin + 0x10000n, 0x8000, 0x8000);
        if (mode === 'large commands') for (let i = 0; i < 192; i++) { u32(cursor, 0x40); u32(cursor + 4, 8); cursor += 8; count++; }
        const symtab = cursor, n = ['large symbols', 'cancel'].includes(mode) ? 192 : 2;
        u32(cursor, 2); u32(cursor + 4, 24); u32(cursor + 8, 0x8000); u32(cursor + 12, n);
        u32(cursor + 16, 0xe000); u32(cursor + 20, 0x100); cursor += 24; count++;
        const named = 0x10000 + (n - 1) * 16;
        for (let i = 1; i < n; i++) {
            const at = 0x10000 + i * 16;
            u32(at, i === n - 1 ? 0x20 : 1); bytes[at + 4] = 0x0f;
            bytes[at + 5] = nsections; u64(at + 8, origin + 0x9120n);
        }
        bytes.set(new TextEncoder().encode('other\0'), 0x16001);
        bytes.set(new TextEncoder().encode(target + '\0'), 0x16020);
        if (mode === 'symtab repeated') { bytes.copyWithin(cursor, symtab, symtab + 24); cursor += 24; count++; }
        if (mode === 'segment count') { for (let i = 0; i < 1022; i++) segment(origin + 0x18000n, 0x10000, 1); u64(header + 32, bytes.length); u64(header + 48, bytes.length); }
        u32(16, count); u32(20, cursor - 32);
        if (mode === 'weak') u16(named + 6, 0x80);
        if (mode === 'bundle') u32(12, 8);
        if (mode === 'executable') u32(12, 2);
        if (mode === 'magic') u32(0, 0);
        if (mode === 'mach32') u32(0, 0xfeedface);
        if (mode === 'big endian') u32(0, 0xcffaedfe);
        if (mode === 'universal') u32(0, 0xcafebabe);
        if (mode === 'filetype') u32(12, 1);
        if (mode === 'command limit') u32(16, 4097);
        if (mode === 'table bounds') u32(20, bytes.length);
        if (mode === 'command truncated') u32(20, 7);
        if (mode === 'command alignment') u32(header + 4, 73);
        if (mode === 'command zero') u32(header + 4, 0);
        if (mode === 'command past end') u32(header + 4, cursor);
        if (mode === 'count mismatch') u32(16, count - 1);
        if (mode === 'symtab missing') u32(symtab, 0x40);
        if (mode === 'symtab size') u32(symtab + 4, 16);
        if (mode === 'segment truncated') u32(header + 4, 64);
        if (mode === 'section count') u32(code + 64, 256);
        if (mode === 'section table') u32(code + 64, 2);
        if (mode === 'section before segment') u64(code + 72 + 32, origin);
        if (mode === 'section past segment') u64(code + 72 + 40, 0x4000);
        if (mode === 'file larger than memory') u64(link + 48, 0x8001);
        if (mode === 'virtual overflow') u64(link + 24, 0xfffffffffffffff0n);
        if (mode === 'file overflow') u64(link + 40, 0xfffffffffffffff0n);
        if (mode === 'high mapping') u32(link + 68, 1);
        if (mode === 'header missing') u64(header + 40, 1);
        if (mode === 'header repeated') u64(link + 40, 0);
        if (mode === 'header truncated') u64(header + 48, 32);
        if (mode === 'ambiguous mapping') { u64(link + 40, 0x4000); u32(symtab + 8, 0x5000); }
        if (mode === 'symbols missing') u32(symtab + 12, 0);
        if (mode === 'symbol limit') u32(symtab + 12, 1048577);
        if (mode === 'symbols unmapped') u32(symtab + 8, 0x20000);
        if (mode === 'symbols cross segment') u32(symtab + 8, 0xfff0);
        if (mode === 'strings missing') u32(symtab + 20, 0);
        if (mode === 'strings unmapped') u32(symtab + 16, 0x20000);
        if (mode === 'strings cross segment') u32(symtab + 16, 0xfff0);
        if (mode === 'table virtual before header') u64(link + 24, origin ? origin - 1n : 0xffffffffffffffffn);
        if (mode === 'table outside module') u64(link + 24, origin + 0x20000n);
        if (mode === 'name outside strings') u32(named, 0x100);
        if (mode === 'name zero') u32(named, 0);
        if (mode === 'name unterminated') u32(symtab + 20, 0x20 + target.length);
        for (const [label, type] of Object.entries({undefined: 1, local: 0x0e, private: 0x1f, indirect: 0x0b, absolute: 3, debug: 0xef})) if (mode === label) bytes[named + 4] = type;
        if (mode === 'section zero') bytes[named + 5] = 0;
        if (mode === 'section missing') bytes[named + 5] = 2;
        if (mode === 'unbacked code') u64(code + 48, 0x1000);
        if (mode === 'nonexecutable segment') u32(code + 60, 1);
        if (mode === 'noninstruction section') u32(code + 72 + 64, 0);
        if (mode === 'symbol before section') u64(named + 8, origin + 0x8fffn);
        if (mode === 'symbol after section') u64(named + 8, origin + 0xa000n);
        if (mode === 'symbol outside module') { u64(code + 24, origin + 0x20000n); u64(code + 72 + 32, origin + 0x21000n); u64(named + 8, origin + 0x21120n); }
        const label = `${arm ? 'arm64' : 'x64'}/${origin}/${mode}`;
        const host = await SplitScriptHost.instantiate(wasm, {operatingSystem: 'macos'});
        let reads = 0, maxReads = 0;
        host.addProcess('game.exe', {
            modules: {runtime: {address: base, size: BigInt(bytes.length)}},
            read({address, length, outputPointer, host}) {
                const offset = address - base;
                assert(offset >= 0n && offset + BigInt(length) <= BigInt(bytes.length), `${label}: read outside module`);
                assert(++reads < 10000, `${label}: excessive reads`);
                if ((mode === 'unreadable symbol' && offset === BigInt(named)) || (mode === 'unreadable string' && offset === 0x16020n)) return false;
                host.bytes(outputPointer, length).set(bytes.subarray(Number(offset), Number(offset) + length)); return true;
            },
        });
        host.start();
        let ticks = 0;
        while (!host.variables.has('address') && !host.variables.has('error') && ticks < 50) {
            const before = reads; host.update(); maxReads = Math.max(maxReads, reads - before); ticks++;
            if (mode === 'cancel' && ticks === 2) break;
        }
        assert(maxReads < 400, `${label}: ${maxReads} reads per update`);
        if (mode === 'cancel') {
            assert.equal(host.variables.size, 0, label);
            host.setProcessOpen('game.exe', false); host.update(3);
            const before = reads; host.update(3); assert.equal(reads, before, label);
            host.setProcessOpen('game.exe', true);
            host.updateUntil(() => host.variables.has('address'), label);
        }
        const success = ['valid', 'weak', 'bundle', 'executable', 'negative slide', 'pagezero', 'large commands', 'large sections', 'large symbols', 'cancel'].includes(mode);
        if (success) {
            assert.equal(BigInt(host.variables.get('address')), base + 0x9120n, label);
            if (mode.startsWith('large')) assert(ticks >= 3, `${label}: missing yields`);
        } else assert(host.variables.get('error')?.length > 0, `${label}: missing error`);
        if (mode === 'segment count') assert.match(host.variables.get('error'), /segment count exceeds/, label);
        cases++;
    }
}
console.log(JSON.stringify({machExportCases: cases}));
