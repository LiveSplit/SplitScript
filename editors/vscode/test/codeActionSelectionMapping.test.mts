import assert from 'node:assert/strict';
import test from 'node:test';

import { mapSelectionThroughEdits } from '../src/codeActionSelectionMapping.ts';

test('a selected expression remains selected inside a wrapping code-action edit', () => {
    const source = 'if edition == Edition.BaseGame {}';
    const start = source.indexOf('edition');
    const end = source.indexOf(' {}');
    const expression = source.slice(start, end);
    const mapped = mapSelectionThroughEdits(
        source,
        { start, end },
        [{ start, end, newText: `inspect(${expression})` }],
    );

    assert.deepEqual(mapped, {
        start: start + 'inspect('.length,
        end: start + 'inspect('.length + expression.length,
    });
});

test('a cursor follows retained text through wrapping and unwrapping edits', () => {
    const source = 'return edition';
    const start = source.indexOf('edition');
    const cursor = start + 3;
    const wrapped = mapSelectionThroughEdits(
        source,
        { start: cursor, end: cursor },
        [{ start, end: source.length, newText: 'inspect(edition)' }],
    );
    assert.deepEqual(wrapped, {
        start: cursor + 'inspect('.length,
        end: cursor + 'inspect('.length,
    });

    const inspected = 'return inspect(edition)';
    const inspectStart = inspected.indexOf('inspect');
    const operandCursor = inspected.indexOf('edition') + 3;
    const unwrapped = mapSelectionThroughEdits(
        inspected,
        { start: operandCursor, end: operandCursor },
        [{ start: inspectStart, end: inspected.length, newText: 'edition' }],
    );
    assert.deepEqual(unwrapped, {
        start: inspectStart + 3,
        end: inspectStart + 3,
    });
});

test('edits before an untouched selection shift both selection boundaries', () => {
    const source = 'first second';
    const start = source.indexOf('second');
    const mapped = mapSelectionThroughEdits(
        source,
        { start, end: source.length },
        [{ start: 0, end: 'first'.length, newText: 'longerFirst' }],
    );
    const delta = 'longerFirst'.length - 'first'.length;
    assert.deepEqual(mapped, {
        start: start + delta,
        end: source.length + delta,
    });
});
