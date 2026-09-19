import {createMonoElfFixture} from './mono_elf_fixture.mjs';

// Mach-O uses the same independently specified Unix Mono metadata shapes.
// Replace only the mapped library, platform module names, and instructions.
export function createMonoMachFixture({version = 'V3', arm = false, uuid, playerVersion,
    mode = 'valid', machine, malformedUuid = false, base = 0x100001000n} = {}) {
    const fixture = createMonoElfFixture({version, playerVersion});
    const originalBase = 0x100001000n, origin = 0x100000000n;
    const {runtime} = fixture, {bytes, u16, u32, u64} = runtime;
    bytes.fill(0);
    u32(0, 0xfeedfacf); u32(4, machine ?? (arm ? 0x100000c : 0x1000007)); u32(12, 6);
    u32(16, uuid ? 3 : 2); u32(20, 152 + 24 + (uuid ? 24 : 0));
    u32(32, 0x19); u32(36, 152); u64(56, origin); u64(64, bytes.length);
    u64(72, 0); u64(80, bytes.length); u32(88, 5); u32(92, 5); u32(96, 1);
    u64(136, origin + 0x400n); u64(144, 0x1c00); u32(152, 0x400); u32(168, 0x80000400);
    u32(184, 2); u32(188, 24); u32(192, 0x1a00); u32(196, 2); u32(200, 0x1c00); u32(204, 0x40);
    if (uuid) { u32(208, 0x1b); u32(212, 24); bytes.set(Uint8Array.from(uuid.replaceAll('-', '').match(/../g), byte => parseInt(byte, 16)), 216); }
    const exportOffset = mode === 'negative page' ? 0x1500 : mode === 'unaligned' ? 0x501 : mode === 'truncated' ? 0x1ffc : 0x500;
    const slot = ['positive page', 'false candidate'].includes(mode) ? 0x1700 : mode === 'wide load' ? 0x1ff8 : 0x700;
    u32(0x1a10, 1); bytes[0x1a14] = 0x0f; bytes[0x1a15] = 1; u64(0x1a18, origin + BigInt(exportOffset));
    bytes.set(new TextEncoder().encode('_mono_assembly_foreach\0'), 0x1c01);
    u64(slot, 0x10000);
    const instruction = (at, target, overridePages) => {
        if (!arm) { bytes.set([0x48, 0x8b, 0x3d], at); u32(at + 3, Number(BigInt.asUintN(32, target - base - BigInt(at + 7)))); return; }
        const page = (base + BigInt(at)) & ~0xfffn;
        const targetPage = mode === 'wide load' ? page : target & ~0xfffn;
        const pages = overridePages ?? ((targetPage - page) / 4096n);
        const bits = Number(BigInt.asUintN(21, pages));
        u32(at, (0x90000017 | ((bits & 3) << 29) | ((bits >>> 2) << 5)) >>> 0);
        const offset = Number(target - targetPage);
        u32(at + 4, (0xf94002e0 | ((offset / 8) << 10)) >>> 0);
    };
    if (mode === 'truncated') u32(exportOffset, 0x90000017);
    else if (mode !== 'missing') {
        if (mode === 'false candidate') instruction(exportOffset, base + 0x3000n);
        instruction(exportOffset + (mode === 'false candidate' ? 16 : 0), base + BigInt(slot));
    }
    if (mode === 'wrong ADRP register') u32(exportOffset, 0x90000016);
    if (mode === 'wrong LDR register') u32(exportOffset + 4, 0xf94002c0);
    if (mode === 'wrong destination') u32(exportOffset + 4, 0xf94002e1);
    if (mode === 'wrong width') u32(exportOffset + 4, 0xb94002e0);
    if (mode === 'not ADRP') u32(exportOffset, 0x10000017);
    if (mode === 'not unsigned LDR') u32(exportOffset + 4, 0xf84002e0);
    if (mode === 'maximum positive page') instruction(exportOffset, base + BigInt(slot), 0xfffffn);
    if (mode === 'maximum negative page') instruction(exportOffset, base + BigInt(slot), -0x100000n);
    if (mode === 'outside load') u32(exportOffset + 4, 0xf97ffee0);
    if (mode === 'missing export') u32(0x1a10, 0);
    const old = version === 'V1' || version === 'V1Cattrs';
    fixture.process.modules = {
        [old ? 'libmono.0.dylib' : 'libmonobdwgc-2.0.dylib']: {address: base, size: BigInt(bytes.length)},
        ...(playerVersion ? {'UnityPlayer.dylib': {address: 0x500000n, size: 0x2000n}} : {}),
    };
    const originalRead = fixture.process.read;
    fixture.process.read = request => {
        request = {...request, address: BigInt.asUintN(64, request.address)};
        if (malformedUuid && request.address === base + 216n && request.length === 16) return false;
        if (mode === 'unreadable instructions' && request.address >= base + BigInt(exportOffset) && request.address < base + BigInt(exportOffset + 0x100)) return false;
        if (request.address >= base && request.address + BigInt(request.length) <= base + BigInt(bytes.length)) {
            return originalRead({...request, address: originalBase + (request.address - base)});
        }
        return originalRead(request);
    };
    return fixture;
}
