export function createElfIdentityFixture({ pointerSize = 8, preferredBase = 0n,
    base = 0x100001000n, idLength = 20, entryPadding = 0, imageSize = 0x4000 } = {}) {
    const bytes = new Uint8Array(imageSize);
    const view = new DataView(bytes.buffer);
    const u16 = (at, value) => view.setUint16(at, value, true);
    const u32 = (at, value) => view.setUint32(at, value, true);
    const u64 = (at, value) => view.setBigUint64(at, BigInt(value), true);
    const wide = pointerSize === 8;
    const table = 0x80;
    const entrySize = (wide ? 56 : 32) + entryPadding;
    const note = 0x800;
    const identity = Uint8Array.from({ length: idLength }, (_, index) => (index * 13) & 255);
    bytes.set([0x7f, 0x45, 0x4c, 0x46, wide ? 2 : 1, 1, 1]);
    u16(16, preferredBase === 0n ? 3 : 2);
    u16(18, wide ? 62 : 3);
    u32(20, 1);
    if (wide) u64(32, table); else u32(28, table);
    const sizes = wide ? 52 : 40;
    u16(sizes, wide ? 64 : 52);
    u16(sizes + 2, entrySize);
    u16(sizes + 4, 2);
    const segment = (at, kind, fileOffset, address, fileSize) => {
        u32(at, kind);
        if (wide) {
            u64(at + 8, fileOffset); u64(at + 16, address); u64(at + 32, fileSize);
        } else {
            u32(at + 4, Number(fileOffset)); u32(at + 8, Number(address)); u32(at + 16, fileSize);
        }
    };
    segment(table, 1, 0, preferredBase, bytes.length);
    const noteEntry = table + entrySize;
    const noteSize = 20 + 16 + ((idLength + 3) & ~3);
    // p_offset deliberately disagrees with the mapped virtual address.
    segment(noteEntry, 4, 0x1800, preferredBase + BigInt(note), noteSize);
    // An unrelated note has both a padded name and padded descriptor.
    u32(note, 3); u32(note + 4, 1); u32(note + 8, 99);
    bytes.set([65, 66, 67, 0, 42, 0, 0, 0], note + 12);
    const buildNote = note + 20;
    u32(buildNote, 4); u32(buildNote + 4, idLength); u32(buildNote + 8, 3);
    bytes.set([71, 78, 85, 0], buildNote + 12);
    bytes.set(identity, buildNote + 16);
    const noteAddress = value => wide ? u64(noteEntry + 16, value) : u32(noteEntry + 8, Number(value));
    const noteLength = value => wide ? u64(noteEntry + 32, value) : u32(noteEntry + 16, value);
    return { bytes, base, u16, u32, u64, wide, table, sizes, entrySize,
        note, noteEntry, noteSize, buildNote, noteAddress, noteLength, identity, segment,
        mappedSize: BigInt(bytes.length) };
}

export function createMachIdentityFixture({ cpu = 0x1000007, commands = 2 } = {}) {
    const bytes = new Uint8Array(0x4000);
    const view = new DataView(bytes.buffer);
    const u32 = (at, value) => view.setUint32(at, value, true);
    const identity = Uint8Array.from({ length: 16 }, (_, index) => 255 - index * 7);
    identity[0] = cpu === 0x100000c ? 0xaa : 0xbb;
    u32(0, 0xfeedfacf); u32(4, cpu); u32(12, 6);
    u32(16, commands); u32(20, (commands - 1) * 8 + 24);
    for (let index = 0; index < commands - 1; index++) {
        u32(32 + index * 8, 0x7); u32(36 + index * 8, 8);
    }
    const uuid = 32 + (commands - 1) * 8;
    u32(uuid, 0x1b); u32(uuid + 4, 24);
    bytes.set(identity, uuid + 8);
    return { bytes, base: 0x100001000n, u32, identity, uuid, mappedSize: BigInt(bytes.length) };
}
