// A mapped PE, not a disk image. File offsets intentionally differ from RVAs.
// Reused by build-identity and measured-runtime-profile fixtures.
export function createPeDebugFixture({ pointerSize = 8, entries = 2, age = 7,
    guid = Uint8Array.from({ length: 16 }, (_, index) => index * 17),
    base = pointerSize === 8 ? 0x100001000n : 0x1000n } = {}) {
    const bytes = new Uint8Array(0x4000);
    const view = new DataView(bytes.buffer);
    const u16 = (offset, value) => view.setUint16(offset, value, true);
    const u32 = (offset, value) => view.setUint32(offset, value, true);
    const pe = 0x80;
    const optional = pe + 0x18;
    const directories = optional + (pointerSize === 4 ? 0x60 : 0x70);
    const table = 0x400;
    const record = 0x3000;
    const entry = table + (entries - 1) * 28;
    u16(0, 0x5a4d);
    u32(0x3c, pe);
    u32(pe, 0x4550);
    u16(pe + 0x14, pointerSize === 4 ? 0xe0 : 0xf0);
    u16(optional, pointerSize === 4 ? 0x10b : 0x20b);
    u32(optional + 0x38, bytes.length);
    u32(directories - 4, 16);
    u32(directories + 6 * 8, table);
    u32(directories + 6 * 8 + 4, entries * 28);
    u32(entry + 12, 2);
    u32(entry + 16, 24);
    u32(entry + 20, record);
    u32(entry + 24, 0xffff0000); // PointerToRawData must never be followed.
    u32(record, 0x53445352);
    bytes.set(guid, record + 4);
    u32(record + 20, age);
    return { bytes, view, u16, u32, base, pe, optional, directories, table,
        entry, record, guid, age, mappedSize: BigInt(bytes.length) };
}
