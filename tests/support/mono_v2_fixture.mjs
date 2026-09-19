const encoder = new TextEncoder();

export function createMonoV2Fixture({ className, fields, parent }) {
    const moduleBase = 0x1000n;
    const assemblyList = moduleBase + 0x700n;
    const link = 0x8100n;
    const assembly = 0x8200n;
    const assemblyName = 0x8300n;
    const imageAddress = 0x8400n;
    const classTable = 0x9000n;
    const classAddress = 0x9100n;
    const classNameAddress = 0x9300n;
    const classNamespace = 0x9320n;
    const fieldTable = 0x9400n;
    const runtimeInfo = 0x9500n;
    const vtables = 0x9600n;
    const staticTable = 0x9700n;

    const moduleImage = new Uint8Array(0x1000);
    const moduleView = new DataView(moduleImage.buffer);
    const writeModuleU16 = (offset, value) => moduleView.setUint16(offset, value, true);
    const writeModuleU32 = (offset, value) => moduleView.setUint32(offset, value, true);

    // A minimal PE32+ image exporting mono_assembly_foreach. Its first
    // instruction references the assembly-list global with RIP-relative
    // addressing, exactly like the supported Windows Mono family.
    writeModuleU16(0x00, 0x5a4d);
    writeModuleU32(0x3c, 0x80);
    writeModuleU32(0x80, 0x00004550);
    writeModuleU16(0x98, 0x020b);
    writeModuleU32(0x108, 0x200);
    writeModuleU32(0x10c, 0x100);
    writeModuleU32(0x214, 1);
    writeModuleU32(0x218, 1);
    writeModuleU32(0x21c, 0x300);
    writeModuleU32(0x220, 0x310);
    writeModuleU32(0x224, 0x320);
    writeModuleU32(0x300, 0x500);
    writeModuleU32(0x310, 0x400);
    writeModuleU16(0x320, 0);
    moduleImage.set(encoder.encode("mono_assembly_foreach\0"), 0x400);
    moduleImage.set([0x48, 0x8b, 0x0d], 0x500);
    writeModuleU32(0x503, Number(assemblyList - (moduleBase + 0x507n)));

    const memory = new Map();
    const writeBytes = (address, bytes) => {
        bytes.forEach((value, index) => {
            const target = address + BigInt(index);
            if (target >= moduleBase && target < moduleBase + BigInt(moduleImage.length)) {
                moduleImage[Number(target - moduleBase)] = value;
            } else {
                memory.set(target, value);
            }
        });
    };
    const writeNumber = (address, size, write) => {
        const buffer = new ArrayBuffer(size);
        write(new DataView(buffer));
        writeBytes(address, new Uint8Array(buffer));
    };
    const writeU32 = (address, value) => writeNumber(
        address,
        4,
        view => view.setUint32(0, value, true),
    );
    const writeI32 = (address, value) => writeNumber(
        address,
        4,
        view => view.setInt32(0, value, true),
    );
    const writeU64 = (address, value) => writeNumber(
        address,
        8,
        view => view.setBigUint64(0, value, true),
    );
    const writeF64 = (address, value) => writeNumber(
        address,
        8,
        view => view.setFloat64(0, value, true),
    );
    const writeUtf8 = (address, value, capacity = 256) => {
        const bytes = new Uint8Array(capacity);
        bytes.set(encoder.encode(value));
        writeBytes(address, bytes);
    };

    writeU64(assemblyList, link);
    writeU64(link, assembly);
    writeU64(link + 8n, 0n);
    writeU64(assembly + 0x10n, assemblyName);
    writeU64(assembly + 0x60n, imageAddress);
    writeUtf8(assemblyName, "Assembly-CSharp");

    const classCache = imageAddress + 0x4c0n;
    writeI32(classCache + 0x18n, 1);
    writeU64(classCache + 0x20n, classTable);
    writeU64(classTable, classAddress);
    writeBytes(classAddress + 0x2an, Uint8Array.of(1));
    writeU64(classAddress + 0x30n, 0n);
    writeU64(classAddress + 0x48n, classNameAddress);
    writeU64(classAddress + 0x50n, classNamespace);
    writeU32(classAddress + 0x5cn, 1);
    writeU64(classAddress + 0x98n, fieldTable);
    writeU64(classAddress + 0xd0n, runtimeInfo);
    writeI32(classAddress + 0x100n, fields.length);
    writeU64(classAddress + 0x108n, 0n);
    writeUtf8(classNameAddress, className);
    writeUtf8(classNamespace, "");

    const fieldOffsets = new Map();
    fields.forEach(({ name, offset }, index) => {
        const field = fieldTable + BigInt(index) * 0x20n;
        const nameAddress = 0xa000n + BigInt(index) * 0x100n;
        writeU64(field + 0x8n, nameAddress);
        writeU32(field + 0x18n, offset);
        writeUtf8(nameAddress, name);
        fieldOffsets.set(name, BigInt(offset));
    });

    writeU64(runtimeInfo + 8n, vtables);
    writeU64(vtables + 0x48n, staticTable);

    let parentStaticTable;
    let parentFieldOffsets;
    if (parent !== undefined) {
        const parentClass = 0x9800n;
        const parentFieldTable = 0x9900n;
        const parentClassName = 0x9a00n;
        const parentNamespace = 0x9a80n;
        const parentRuntimeInfo = 0x9b00n;
        const parentVtables = 0x9c00n;
        parentStaticTable = 0x9d00n;
        parentFieldOffsets = new Map();

        writeU64(classAddress + 0x30n, parentClass);
        writeBytes(parentClass + 0x2an, Uint8Array.of(1));
        writeU64(parentClass + 0x30n, 0n);
        writeU64(parentClass + 0x48n, parentClassName);
        writeU64(parentClass + 0x50n, parentNamespace);
        writeU32(parentClass + 0x5cn, 1);
        writeU64(parentClass + 0x98n, parentFieldTable);
        writeU64(parentClass + 0xd0n, parentRuntimeInfo);
        writeI32(parentClass + 0x100n, parent.fields.length);
        writeU64(parentClass + 0x108n, 0n);
        writeUtf8(parentClassName, parent.className);
        writeUtf8(parentNamespace, parent.namespace ?? "");

        parent.fields.forEach(({ name, offset }, index) => {
            const field = parentFieldTable + BigInt(index) * 0x20n;
            const nameAddress = 0xb000n + BigInt(index) * 0x100n;
            writeU64(field + 0x8n, nameAddress);
            writeU32(field + 0x18n, offset);
            writeUtf8(nameAddress, name);
            parentFieldOffsets.set(name, BigInt(offset));
        });
        writeU64(parentRuntimeInfo + 8n, parentVtables);
        writeU64(parentVtables + 0x48n, parentStaticTable);
    }

    return {
        staticTable,
        fieldOffsets,
        parentStaticTable,
        parentFieldOffsets,
        writeBytes,
        writeI32,
        writeU32,
        writeU64,
        writeF64,
        process: {
            modules: {
                "mono-2.0-bdwgc.dll": {
                    address: moduleBase,
                    size: BigInt(moduleImage.length),
                },
            },
            ranges: [{ address: moduleBase, bytes: moduleImage, flags: 5n }],
            read({ address, outputPointer, length, host }) {
                const moduleEnd = moduleBase + BigInt(moduleImage.length);
                if (address >= moduleBase && address + BigInt(length) <= moduleEnd) {
                    const offset = Number(address - moduleBase);
                    host.bytes(outputPointer, length).set(
                        moduleImage.subarray(offset, offset + length),
                    );
                    return true;
                }

                const output = host.bytes(outputPointer, length);
                for (let index = 0; index < length; index += 1) {
                    const value = memory.get(address + BigInt(index));
                    if (value === undefined) return false;
                    output[index] = value;
                }
                return true;
            },
        },
    };
}
