// Linux shapes specified independently of the generated profile catalog.
const layouts = {
    V1: [0x58, 0x3d0, 0x40, 0x48, 0xa0, 0xf0, 0x8c, 0xf8, 0x18],
    V1Cattrs: [0x58, 0x3d0, 0x48, 0x50, 0xa8, 0xf8, 0x94, 0x100, 0x18],
    V2: [0x60, 0x4c0, 0x40, 0x48, 0x90, 0xc8, 0xf8, 0x100, 0x54],
    V3: [0x60, 0x4d0, 0x40, 0x48, 0x90, 0xc8, 0xf8, 0x100, 0x54],
};
export function createMonoElfFixture({ version = 'V3', runtimeId, playerId, playerVersion,
    malformedId = false, falseCandidate = false, machine = 62, elfClass = 2,
    signature = true, exportPresent = true, backward = false } = {}) {
    const base = 0x100001000n, playerBase = 0x500000n;
    const elf = (base, identity) => {
        const bytes = new Uint8Array(0x2000), view = new DataView(bytes.buffer);
        const u16 = (at, value) => view.setUint16(at, value, true);
        const u32 = (at, value) => view.setUint32(at, value, true);
        const u64 = (at, value) => view.setBigUint64(at, BigInt(value), true);
        bytes.set([0x7f, 0x45, 0x4c, 0x46, 2, 1, 1]);
        u16(16, 3); u16(18, 62); u32(20, 1); u64(32, 0x80);
        u16(52, 64); u16(54, 56); u16(56, identity ? 3 : 2);
        const segment = (at, kind, offset, size) => {
            u32(at, kind); u64(at + 8, offset); u64(at + 16, offset);
            u64(at + 32, size); u64(at + 40, size);
        };
        segment(0x80, 1, 0, bytes.length); segment(0xb8, 2, 0x200, 96);
        [[4, base + 0x300n], [5, base + 0x400n], [6, base + 0x480n], [10, 0x40], [11, 24]].forEach(([tag, value], i) => {
            u64(0x200 + i * 16, tag); u64(0x208 + i * 16, value);
        });
        u32(0x300, 1); u32(0x304, 2); u32(0x308, 1);
        bytes.set(new TextEncoder().encode('mono_assembly_foreach\0'), 0x401);
        u32(0x498, 1); bytes[0x49c] = 0x12; u16(0x49e, 1); u64(0x4a0, 0x500);
        if (identity) {
            const id = Uint8Array.from(identity.match(/../g), byte => parseInt(byte, 16));
            segment(0xf0, 4, 0x900, 16 + ((id.length + 3) & ~3));
            u32(0x900, 4); u32(0x904, id.length); u32(0x908, 3);
            bytes.set([71, 78, 85, 0], 0x90c); bytes.set(id, 0x910);
        }
        return {bytes, view, u16, u32, u64};
    };
    const runtime = elf(base, runtimeId), player = elf(playerBase, playerId);
    runtime.u16(18, machine); runtime.bytes[4] = elfClass;
    if (malformedId) { runtime.u16(56, 3); runtime.u32(0xf0, 4); runtime.u64(0x100, 0x3000); runtime.u64(0x110, 32); }
    if (!exportPresent) runtime.u32(0x308, 0);
    const instruction = (at, target) => {
        runtime.bytes.set([0x48, 0x8b, 0x3d], at);
        runtime.u32(at + 3, Number(BigInt.asUintN(32, target - (base + BigInt(at + 7)))));
    };
    if (falseCandidate) instruction(0x500, base + 0x3000n);
    if (signature) instruction(falseCandidate ? 0x510 : 0x500, base + (backward ? 0x450n : 0x700n));
    if (playerVersion) player.bytes.set(new TextEncoder().encode('\0' + playerVersion + '\0'), 0x1000);
    const memory = new Map();
    const write = (at, bytes) => bytes.forEach((byte, i) => memory.set(at + BigInt(i), byte));
    const number = (at, value, width = 8) => {
        const bytes = new Uint8Array(width), view = new DataView(bytes.buffer);
        if (width === 8) view.setBigUint64(0, BigInt(value), true);
        else view.setUint32(0, value, true);
        write(at, bytes);
    };
    const text = (at, value) => { const bytes = new Uint8Array(256); bytes.set(new TextEncoder().encode(value)); write(at, bytes); };
    const [assemblyImage, cache, name, namespace, fields, info, fieldCount, next, size] = layouts[version];
    const old = version === 'V1' || version === 'V1Cattrs';
    runtime.u64(backward ? 0x450 : 0x700, 0x10000);
    number(0x10000n, 0x11000); number(0x10008n, 0);
    number(0x11000n + BigInt(assemblyImage), 0x12000); number(0x11010n, 0x20000);
    text(0x20000n, 'Assembly-CSharp');
    number(0x12000n + BigInt(cache + 0x18), 1, 4); number(0x12000n + BigInt(cache + 0x20), 0x13000);
    number(0x13000n, 0x14000); number(0x14028n, 0); number(0x14000n + BigInt(next), 0);
    number(0x14000n + BigInt(name), 0x21000); number(0x14000n + BigInt(namespace), 0x22000);
    if (version === 'V1Cattrs') number(0x14040n, 0x12000);
    if (!old) write(0x14000n + BigInt(version === 'V2' ? 0x24 : 0x1b), Uint8Array.of(1));
    text(0x21000n, 'ProfileProbe'); text(0x22000n, '');
    number(0x14000n + BigInt(fields), 0x15000); number(0x14000n + BigInt(fieldCount), 1, 4);
    number(0x15008n, 0x23000); text(0x23000n, 'value'); number(0x15018n, 0x10, 4);
    number(0x14000n + BigInt(info), 0x16000); number(0x16008n, 0x17000);
    if (old) number(0x17000n + BigInt(size), 0x18000);
    else { number(0x14000n + BigInt(size), 3, 4); number(0x17000n + BigInt((version === 'V2' ? 0x40 : 0x48) + 24), 0x18000); }
    number(0x18010n, 42, 4);
    let reads = 0, playerReads = 0;
    const hasPlayer = playerId !== undefined || playerVersion !== undefined;
    return {runtime, player, get reads() {return reads;}, get playerReads() {return playerReads;},
        process: {
            modules: {
                [old ? 'libmono.so' : 'libmonobdwgc-2.0.so']: {address: base, size: BigInt(runtime.bytes.length)},
                ...(hasPlayer ? {'UnityPlayer.so': {address: playerBase, size: BigInt(player.bytes.length)}} : {}),
            },
            read({address, length, outputPointer, host}) {
                reads++;
                const output = host.bytes(outputPointer, length);
                for (const [start, image, isPlayer] of [[base, runtime.bytes, false], [playerBase, player.bytes, true]]) {
                    if (address >= start && address + BigInt(length) <= start + BigInt(image.length)) {
                        if (isPlayer) playerReads++;
                        output.set(image.subarray(Number(address - start), Number(address - start) + length)); return true;
                    }
                }
                for (let i = 0; i < length; i++) { const byte = memory.get(address + BigInt(i)); if (byte === undefined) return false; output[i] = byte; }
                return true;
            },
        },
    };
}
