// Independent MonoClass by-value offsets for the fixture ABI families.
const byValueOffsets = {
    V1: {32: 0x84, 64: 0xc8}, V1Cattrs: {32: 0x88, 64: 0xd0},
    V2: {32: 0x70, 64: 0xb8}, V3: {32: 0x70, 64: 0xb8},
};
export function writeVectorElementType({mono, width, family, ptr, number, fixture}, vectorType, elementClass, kind, valueBytes = 8) {
    const bytes = width / 8;
    const elementType = mono ? elementClass + BigInt(byValueOffsets[family][width]) : elementClass;
    number(vectorType + BigInt(bytes + 2), 1, 0x1d);
    ptr(vectorType, mono ? elementClass : elementType);
    number(elementType + BigInt(bytes + 2), 1, kind);
    if (mono && kind === 0x11) {
        ptr(elementType, elementClass);
        number(elementClass + BigInt(width === 64 ? 0x1c : 0x10), 4, 2 * bytes + valueBytes);
    }
    if (!mono && kind === 0x11) writeIl2cppPlainType(fixture, {ptr, number}, elementType, elementClass + 0x20000000n, valueBytes);
    if (kind === 0x15) writeGenericType({mono, width, ptr, number, cachedClass: elementClass}, elementType);
    return elementType;
}

const plainTypes = new WeakMap();
// The public collection fixtures use Unity 2022.3. Its data union stores a
// metadata definition handle, distinct from both the type and runtime class.
export function writeIl2cppPlainType(fixture, {ptr, number}, type, klass, valueBytes) {
    const p = BigInt(fixture.width / 8);
    let registered = plainTypes.get(fixture);
    if (!registered) { registered = new Map(); plainTypes.set(fixture, registered); }
    if (!registered.has(type)) registered.set(type, 10 + registered.size);
    const index = registered.get(type), definition = i => 0x200000n + BigInt(i) * 88n;
    ptr(fixture.klass + 4n * p, definition(8));
    number(fixture.klass + 5n * p + 2n, 1, 0x12);
    ptr(fixture.klass + 13n * p, definition(8));
    number(0x12000n + (p === 8n ? 0x18n : 0xcn), 4, 3 + registered.size);
    ptr(0x13000n + BigInt(index) * p, klass);
    // The attachment walk must also be able to inspect this newly registered
    // class. Preserve names on entry classes whose field layout is tested.
    if (Array.from({length: Number(p)}, (_, i) => fixture.memory.get(klass + 2n * p + BigInt(i)) ?? 0).every(byte => byte === 0)) {
        ptr(klass + 2n * p, 0x2fff00n); ptr(klass + 3n * p, 0x2fff00n);
        for (let i = 0n; i < 256n; i++) fixture.memory.set(0x2fff00n + i, 0);
    }
    ptr(type, definition(index)); number(type + p + 2n, 1, 0x11);
    ptr(klass + 4n * p, definition(index));
    number(klass + 5n * p + 2n, 1, 0x11);
    ptr(klass + 13n * p, definition(index));
    number(klass + (p === 8n ? 0xf8n : 0x80n), 4, Number(2n * p) + valueBytes);
    return klass;
}

export function writeGenericType({mono, width, ptr, number, cachedClass}, typeAddress, value = false, valueBytes = 16) {
    const bytes = width / 8;
    number(typeAddress + BigInt(bytes + 2), 1, 0x15);
    const klass = cachedClass ?? typeAddress + 0x10000000n, descriptor = klass + 0x400n;
    const flags = klass + BigInt(mono ? (width === 64 ? 0x20 : 0x14) : (width === 64 ? 0x2b : 0x17));
    const size = klass + BigInt(mono ? (width === 64 ? 0x1c : 0x10) : (width === 64 ? 0xf8 : 0x80));
    const cached = descriptor + BigInt((mono ? 4 : 3) * bytes);
    ptr(typeAddress, descriptor); ptr(cached, klass);
    number(flags, 1, value ? (mono ? 4 : 128) : 0);
    number(size, 4, 2 * bytes + valueBytes);
    return {klass, descriptor, flags, size, cached};
}
