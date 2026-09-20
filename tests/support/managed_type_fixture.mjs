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
    if (kind === 0x15) writeGenericType({mono, width, ptr, number, cachedClass: elementClass}, elementType);
    return elementType;
}

export function writeGenericType({mono, width, ptr, number, cachedClass}, typeAddress, value = false, valueBytes = 16) {
    const bytes = width / 8;
    number(typeAddress + BigInt(bytes + 2), 1, 0x15);
    const klass = cachedClass ?? typeAddress + 0x10000000n, descriptor = klass + 0x400n;
    const flags = klass + BigInt(width === 64 ? 0x20 : 0x14);
    if (mono) {
        ptr(typeAddress, descriptor); ptr(descriptor + BigInt(4 * bytes), klass);
        number(flags, 1, value ? 4 : 0);
        number(flags - 4n, 4, 2 * bytes + valueBytes);
    }
    return {klass, descriptor, flags};
}
