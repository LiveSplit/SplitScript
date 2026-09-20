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
    if (kind === 0x1d) {
        const childClass = elementClass + 0x800n;
        const childType = mono ? childClass + BigInt(byValueOffsets[family][width]) : childClass;
        ptr(elementType, mono ? childClass : childType);
        number(childType + BigInt(bytes + 2), 1, 0x0e);
    }
    if (mono && kind === 0x11) {
        ptr(elementType, elementClass);
        number(elementClass + BigInt(width === 64 ? 0x1c : 0x10), 4, 2 * bytes + valueBytes);
    }
    if (!mono && kind === 0x11) writeIl2cppPlainType(fixture, {ptr, number}, elementType, elementClass + 0x20000000n, valueBytes);
    if (kind === 0x15) writeGenericType({mono, width, ptr, number, cachedClass: elementClass}, elementType);
    return elementType;
}

const plainTypes = new WeakMap();
const arrayTypes = new WeakMap();
// Explicit object headers and runtime type graphs for owned-array fixtures.
// A nested one-element JS array denotes SZARRAY; objects describe inline values.
export function writeManagedArrayType(fixture, {mono, width, family = 'V2', ptr, number}, object, element) {
    let state = arrayTypes.get(fixture);
    if (!state) { state = {types: new Map(), next: 0x30000000n}; arrayTypes.set(fixture, state); }
    const p = BigInt(width / 8);
    const typeOf = shape => {
        const key = JSON.stringify(shape);
        if (state.types.has(key)) return state.types.get(key);
        const klass = state.next; state.next += 0x1000n;
        const type = klass + (mono ? BigInt(byValueOffsets[family][width]) : 4n * p);
        const kind = Array.isArray(shape) ? 0x1d : typeof shape === 'number' ? shape : shape.kind;
        const result = {klass, type, kind, vtable: klass + 0x800n};
        state.types.set(key, result);
        number(type + p + 2n, 1, kind); ptr(result.vtable, klass);
        if (kind === 0x1d) {
            const child = typeOf(shape[0]);
            ptr(type, mono ? child.klass : child.type); result.element = child;
        } else if (kind === 0x11) {
            if (mono) {
                ptr(type, klass); number(klass + (p === 8n ? 0x1cn : 0x10n), 4, Number(2n * p) + shape.bytes);
            } else writeIl2cppPlainType(fixture, {ptr, number}, type, klass, shape.bytes);
        } else if (kind === 0x15) writeGenericType({mono, width, ptr, number, cachedClass: klass}, type, shape.value ?? false, shape.bytes ?? 16);
        return result;
    };
    const array = typeOf([element]);
    ptr(object, mono ? array.vtable : array.klass);
    return array;
}

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

// Explicit object headers for schema snapshots; the Mono vtable is independent
// of the class's metadata storage and of array type fixtures.
export function writeManagedObjectHeader(fixture, {mono, ptr}, object, klass = 0x14000n) {
    const vtable = klass + 0x18000000n;
    if (mono) ptr(vtable, klass);
    ptr(object, mono ? vtable : klass);
    return {klass, vtable, header: mono ? vtable : klass};
}
