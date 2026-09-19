// Independently specified Windows Mono memory shapes. Do not build these
// from the profile JSON or the generated standard-library descriptors.
const layouts = {
    V1: {
        32: [0x40, 0x2a0, 0x24, 0x30, 0x34, 0x74, 0xa4, 0x64, 0xa8, 0xc],
        64: [0x58, 0x3d0, 0x30, 0x48, 0x50, 0xa8, 0xf8, 0x94, 0x100, 0x18],
    },
    V1Cattrs: {
        32: [0x40, 0x2a0, 0x24, 0x34, 0x38, 0x78, 0xa8, 0x68, 0xac, 0xc],
        64: [0x58, 0x3d0, 0x30, 0x50, 0x58, 0xb0, 0x100, 0x9c, 0x108, 0x18],
    },
    V2: {
        32: [0x44, 0x354, 0x20, 0x2c, 0x30, 0x60, 0x84, 0xa4, 0xa8, 0x38],
        64: [0x60, 0x4c0, 0x30, 0x48, 0x50, 0x98, 0xd0, 0x100, 0x108, 0x5c],
    },
    V3: {
        32: [0x48, 0x35c, 0x20, 0x2c, 0x30, 0x60, 0x7c, 0x9c, 0xa0, 0x38],
        64: [0x60, 0x4d0, 0x30, 0x48, 0x50, 0x98, 0xd0, 0x100, 0x108, 0x5c],
    },
};

export function codeViewGuid(canonical) {
    const bytes = Uint8Array.from(canonical.replaceAll("-", "").match(/../g), hex => parseInt(hex, 16));
    return Uint8Array.from([3, 2, 1, 0, 5, 4, 7, 6, 8, 9, 10, 11, 12, 13, 14, 15], i => bytes[i]);
}

export function createMonoPeFixture({ version, width, guid, age = 1, exact = true,
    signatureVariant = 0, falseCandidate = false, exportOffset = 0x500, playerVersion } = {}) {
    const pointerBytes = width / 8;
    const wide = width === 64;
    const old = version === "V1" || version === "V1Cattrs";
    const base = wide ? 0x100001000n : 0x1000n;
    const image = new Uint8Array(0x1000);
    const view = new DataView(image.buffer);
    const u16 = (at, value) => view.setUint16(at, value, true);
    const u32 = (at, value) => view.setUint32(at, value, true);
    const memory = new Map();
    const write = (address, bytes) => {
        for (let i = 0; i < bytes.length; i++) {
            const at = address + BigInt(i);
            if (at >= base && at < base + BigInt(image.length)) image[Number(at - base)] = bytes[i];
            else memory.set(at, bytes[i]);
        }
    };
    const number = (address, bytes, value) => {
        const data = new Uint8Array(bytes);
        const target = new DataView(data.buffer);
        if (bytes === 8) target.setBigUint64(0, BigInt(value), true);
        else target.setUint32(0, Number(value), true);
        write(address, data);
    };
    const ptr = (at, value) => number(at, pointerBytes, value);
    const int = (at, value) => number(at, 4, value);
    const string = (at, text) => {
        const data = new Uint8Array(256);
        data.set(new TextEncoder().encode(text));
        write(at, data);
    };
    const optional = 0x98;
    const directory = optional + (wide ? 0x70 : 0x60);
    u16(0, 0x5a4d); u32(0x3c, 0x80); u32(0x80, 0x4550);
    u16(0x84, wide ? 0x8664 : 0x14c); u16(0x94, wide ? 0xf0 : 0xe0);
    u16(optional, wide ? 0x20b : 0x10b); u32(optional + 0x38, image.length);
    u32(directory - 4, 16); u32(directory, 0x200); u32(directory + 4, 0x100);
    u32(0x214, 1); u32(0x218, 1); u32(0x21c, 0x300); u32(0x220, 0x310); u32(0x224, 0x320);
    u32(0x300, exportOffset); u32(0x310, 0x400); u16(0x320, 0);
    image.set(new TextEncoder().encode("mono_assembly_foreach\0"), 0x400);
    const assemblies = base + 0x700n;
    const instruction = (at, target) => {
        if (wide) {
            image.set([0x48, 0x8b, 0x0d], at);
            u32(at + 3, Number(BigInt.asUintN(32, target - (base + BigInt(at + 7)))));
        } else {
            image.set(signatureVariant ? [0x8b, 0x0d] : [0xff, 0x35], at);
            u32(at + 2, Number(target));
        }
    };
    if (falseCandidate) instruction(exportOffset, base + 0x2000n);
    instruction(exportOffset + (falseCandidate ? 16 : 0), assemblies);
    if (guid !== undefined) {
        u32(directory + 48, 0x800); u32(directory + 52, 28);
        u32(0x80c, 2); u32(0x810, 24); u32(0x814, 0x900);
        u32(0x900, 0x53445352); image.set(codeViewGuid(guid), 0x904); u32(0x914, age);
    }
    const [assemblyImage, classCache, parent, name, namespace, fields, runtimeInfo, fieldCount, next, size] = layouts[version][width];
    const link = 0x10000n, assembly = 0x11000n, managedImage = 0x12000n;
    const table = 0x13000n, klass = 0x14000n, fieldTable = 0x15000n;
    const info = 0x16000n, vtable = 0x17000n, statics = 0x18000n;
    ptr(assemblies, link); ptr(link, assembly); ptr(link + BigInt(pointerBytes), 0);
    ptr(assembly + BigInt(assemblyImage), managedImage);
    // Measured profiles deliberately use image naming; the fallback's assembly
    // name is wrong so silently ignoring exact identity fails attachment.
    ptr(assembly + BigInt(wide ? 0x10 : 0x8), 0x20000n);
    ptr(managedImage + BigInt(version === "V3" ? (wide ? 0x30 : 0x1c) : (wide ? 0x28 : 0x18)), 0x21000n);
    string(0x20000n, exact ? "Wrong-Assembly" : "Assembly-CSharp");
    string(0x21000n, "Assembly-CSharp");
    int(managedImage + BigInt(classCache + (wide ? 0x18 : 0xc)), 1);
    ptr(managedImage + BigInt(classCache + (wide ? 0x20 : 0x14)), table);
    ptr(table, klass); ptr(klass + BigInt(parent), 0); ptr(klass + BigInt(next), 0);
    ptr(klass + BigInt(name), 0x22000n); ptr(klass + BigInt(namespace), 0x23000n);
    if (version === "V1Cattrs") ptr(klass + BigInt(wide ? 0x48 : 0x30), managedImage);
    string(0x22000n, "ProfileProbe"); string(0x23000n, "");
    ptr(klass + BigInt(fields), fieldTable); int(klass + BigInt(fieldCount), 1);
    ptr(fieldTable + BigInt(pointerBytes), 0x24000n); string(0x24000n, "value");
    int(fieldTable + BigInt(wide ? 0x18 : 0xc), 0x10);
    ptr(klass + BigInt(runtimeInfo), info); ptr(info + BigInt(pointerBytes), vtable);
    if (old) ptr(vtable + BigInt(size), statics);
    else {
        int(klass + BigInt(size), 3);
        const slots = version === "V2" ? (wide ? 0x40 : 0x28) : (wide ? 0x48 : 0x2c);
        ptr(vtable + BigInt(slots + 3 * pointerBytes), statics);
    }
    int(statics + 0x10n, 42);
    const player = new Uint8Array(0x1000);
    const playerBase = 0x500000n;
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
    }
    let reads = 0;
    return { base, image, assemblies, memory, version, width,
        get reads() { return reads; },
        process: {
            pointerSize: pointerBytes,
            modules: {
                [old ? "mono.dll" : "mono-2.0-bdwgc.dll"]: { address: base, size: BigInt(image.length) },
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
                    const byte = at >= base && at < base + BigInt(image.length)
                        ? image[Number(at - base)] : memory.get(at);
                    if (byte === undefined) return false;
                    output[i] = byte;
                }
                return true;
            },
        },
    };
}
