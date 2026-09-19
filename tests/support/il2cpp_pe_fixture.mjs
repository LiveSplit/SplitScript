// Independent mapped Windows IL2CPP memory, including inline and indirect
// image starts. Never derive these offsets from the generated profile catalog.
export function createIl2cppPeFixture({ version = [2021, 3, 11, 23713], width = 64,
    playerVersion = version, storeVariant = 0, falseCandidate = false } = {}) {
    const wide = width === 64;
    const pointerBytes = width / 8;
    const [major, minor] = version;
    const inline = major <= 2020;
    const fieldCount = wide
        ? major === 2018 ? 0x114 : major <= 2020 || major === 6000 && minor >= 7 ? 0x11c : major === 2021 ? 0x120 : 0x124
        : major === 2018 ? 0xac : major <= 2021 ? 0xa8 : 0xac;
    const staticsOffset = major === 6000 && minor >= 7 ? (wide ? 0x98 : 0x4c)
        : major === 6000 && minor >= 5 ? (wide ? 0xa0 : 0x50) : (wide ? 0xb8 : 0x5c);
    const base = wide ? 0x100001000n : 0x1000n;
    const image = new Uint8Array(0x2000);
    const view = new DataView(image.buffer);
    const u16 = (at, value) => view.setUint16(at, value, true);
    const u32 = (at, value) => view.setUint32(at, value, true);
    const relative = (at, target) => u32(at, Number(BigInt.asUintN(32, target - base - BigInt(at + 4))));
    const memory = new Map();
    const write = (address, bytes) => {
        for (let i = 0; i < bytes.length; i++) {
            const at = address + BigInt(i);
            if (at >= base && at < base + BigInt(image.length)) image[Number(at - base)] = bytes[i];
            else memory.set(at, bytes[i]);
        }
    };
    const number = (address, size, value) => {
        const bytes = new Uint8Array(size);
        const data = new DataView(bytes.buffer);
        if (size === 8) data.setBigUint64(0, BigInt(value), true);
        else if (size === 2) data.setUint16(0, Number(value), true);
        else data.setUint32(0, Number(value), true);
        write(address, bytes);
    };
    const ptr = (address, value) => number(address, pointerBytes, value);
    const text = (address, value) => {
        const bytes = new Uint8Array(256);
        bytes.set(new TextEncoder().encode(value));
        write(address, bytes);
    };
    u16(0, 0x5a4d); u32(0x3c, 0x80); u32(0x80, 0x4550);
    u16(0x84, wide ? 0x8664 : 0x14c); u16(0x94, wide ? 0xf0 : 0xe0);
    u16(0x98, wide ? 0x20b : 0x10b); u32(0xd0, image.length);
    u32(wide ? 0x104 : 0xf4, 16);
    const assemblies = base + 0xc00n, typeInfo = base + 0xc20n;
    const assemblyCode = (at, valid) => {
        if (wide) {
            image.set([0x75, 0x10, 0x48, 0x8b, 0x1d, 0, 0, 0, 0, 0x48, 0x3b, 0x1d], at);
            relative(at + 5, assemblies);
            relative(at + 12, assemblies + (valid ? 8n : 16n));
        } else {
            image.set([0x75, 0x10, 0x8b, 0x35, 0, 0, 0, 0, 0x2b, 0xf9, 0x3b, 0x35], at);
            u32(at + 4, Number(assemblies));
            u32(at + 12, Number(assemblies + (valid ? 4n : 8n)));
        }
    };
    if (falseCandidate) assemblyCode(0x280, false);
    assemblyCode(0x300, true);
    image.set(new TextEncoder().encode("global-metadata.dat\0"), 0x600);
    if (wide) {
        image.set([0x48, 0x8d, 0x0d], 0x700); relative(0x703, base + 0x600n);
        image.set([0x48, 0xc1, 0xe9, 0x03], 0x710);
        image.set([0x48, 0x89, 0x05], 0x730); relative(0x733, typeInfo);
    } else {
        image.set([0x68, 0, 0, 0, 0, 0xe8], 0x700); u32(0x701, Number(base + 0x600n));
        const stores = [
            [[0xc1, 0xea, 2, 0x52, 0xe8, 0, 0, 0, 0, 0xa3], 10],
            [[0xc1, 0xea, 2, 0x52, 0xe8, 0, 0, 0, 0, 0x8b, 0x0d, 0, 0, 0, 0, 0xa3], 16],
            [[0xff, 0xb0, 0xf4, 0, 0, 0, 0xe8, 0, 0, 0, 0, 0xa3], 12],
        ];
        const [bytes, displacement] = stores[storeVariant];
        image.set(bytes, 0x730); u32(0x730 + displacement, Number(typeInfo));
    }
    const vector = 0x10000n, assembly = 0x11000n, managedImage = 0x12000n;
    const types = 0x13000n, klass = 0x14000n, fields = 0x15000n, statics = 0x16000n;
    ptr(assemblies, vector); ptr(assemblies + BigInt(pointerBytes), vector + BigInt(pointerBytes));
    ptr(vector, assembly); ptr(assembly, managedImage); ptr(typeInfo, types);
    // Measured profiles use the image name. An assembly-relative guess is wrong.
    ptr(assembly + BigInt(wide ? 0x18 : 0xc), 0x20000n); text(0x20000n, "Wrong-Assembly");
    ptr(managedImage + BigInt(pointerBytes), 0x21000n); text(0x21000n, "Assembly-CSharp");
    number(managedImage + BigInt(inline ? (wide ? 0x1c : 0x10) : (wide ? 0x18 : 0xc)), 4, 3);
    if (inline) number(managedImage + BigInt(wide ? 0x18 : 0xc), 4, 7);
    else { ptr(managedImage + BigInt(wide ? 0x28 : 0x18), 0x17000n); number(0x17000n, 4, 7); }
    ptr(types + BigInt(7 * pointerBytes), 0);
    ptr(types + BigInt(8 * pointerBytes), klass);
    ptr(types + BigInt(9 * pointerBytes), 0);
    ptr(klass + BigInt(wide ? 0x10 : 8), 0x22000n); text(0x22000n, "ProfileProbe");
    ptr(klass + BigInt(wide ? 0x18 : 0xc), 0x23000n); text(0x23000n, "");
    ptr(klass + BigInt(wide ? 0x58 : 0x2c), 0);
    ptr(klass + BigInt(wide ? 0x80 : 0x40), fields); number(klass + BigInt(fieldCount), 2, 1);
    ptr(fields, 0x24000n); text(0x24000n, "value"); number(fields + BigInt(wide ? 0x18 : 0xc), 4, 0x10);
    ptr(klass + BigInt(staticsOffset), statics); number(statics + 0x10n, 4, 42);
    const player = new Uint8Array(0x1000), playerBase = 0x500000n;
    if (playerVersion) {
        const data = new DataView(player.buffer);
        const word = (at, value) => data.setUint16(at, value, true);
        const dword = (at, value) => data.setUint32(at, value, true);
        word(0, 0x5a4d); dword(0x3c, 0x80); dword(0x80, 0x4550); word(0x98, 0x20b);
        dword(0x118, 0x200); dword(0x11c, 0x400);
        word(0x20e, 1); dword(0x210, 16); dword(0x214, 0x80000040);
        word(0x24e, 1); dword(0x254, 0x80000070);
        word(0x27e, 1); dword(0x284, 0xa0); dword(0x2a0, 0x300);
        dword(0x328, 0xfeef04bd);
        word(0x330, playerVersion[1]); word(0x332, playerVersion[0]);
        word(0x334, playerVersion[3]); word(0x336, playerVersion[2]);
    }
    let reads = 0;
    return { base, image, memory, assemblies, typeInfo, klass, version, width,
        get reads() { return reads; },
        process: {
            pointerSize: pointerBytes,
            modules: {
                "GameAssembly.dll": { address: base, size: BigInt(image.length) },
                ...(playerVersion ? { "UnityPlayer.dll": { address: playerBase, size: BigInt(player.length) } } : {}),
            },
            ranges: [{ address: base, bytes: image, flags: 5n }],
            read({ address, length, outputPointer, host }) {
                reads++;
                const output = host.bytes(outputPointer, length);
                if (playerVersion && address >= playerBase && address + BigInt(length) <= playerBase + BigInt(player.length)) {
                    output.set(player.subarray(Number(address - playerBase), Number(address - playerBase) + length));
                    return true;
                }
                for (let i = 0; i < length; i++) {
                    const at = address + BigInt(i);
                    const byte = at >= base && at < base + BigInt(image.length) ? image[Number(at - base)] : memory.get(at);
                    if (byte === undefined) return false;
                    output[i] = byte;
                }
                return true;
            },
        },
    };
}
