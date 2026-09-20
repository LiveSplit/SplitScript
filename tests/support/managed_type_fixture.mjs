// Independent MonoClass by-value offsets for the fixture ABI families.
const byValueOffsets = {
    V1: {32: 0x84, 64: 0xc8}, V1Cattrs: {32: 0x88, 64: 0xd0},
    V2: {32: 0x70, 64: 0xb8}, V3: {32: 0x70, 64: 0xb8},
};
export function writeVectorElementType({mono, width, family, ptr, number}, vectorType, elementClass, kind, valueBytes = 8) {
    const bytes = width / 8;
    const elementType = mono ? elementClass + BigInt(byValueOffsets[family][width]) : elementClass;
    number(vectorType + BigInt(bytes + 2), 1, 0x1d);
    ptr(vectorType, mono ? elementClass : elementType);
    number(elementType + BigInt(bytes + 2), 1, kind);
    if (mono && kind === 0x11) {
        ptr(elementType, elementClass);
        number(elementClass + BigInt(width === 64 ? 0x1c : 0x10), 4, 2 * bytes + valueBytes);
    }
    return elementType;
}
