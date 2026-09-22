export interface OffsetRange {
    start: number;
    end: number;
}

export interface OffsetEdit extends OffsetRange {
    newText: string;
}

/**
 * Maps the range that requested a code action through that action's edits.
 *
 * VS Code otherwise keeps the old selection offsets after applying an LSP
 * workspace edit. That is especially visible for wrappers: selecting `value`
 * and replacing it with `inspect(value)` leaves an arbitrary prefix selected.
 */
export function mapSelectionThroughEdits(
    source: string,
    selection: OffsetRange,
    inputEdits: readonly OffsetEdit[],
): OffsetRange {
    const edits = [...inputEdits].sort((left, right) =>
        left.start - right.start || left.end - right.end
    );
    if (selection.start === selection.end) {
        const cursor = mapCursor(source, selection.start, edits);
        return { start: cursor, end: cursor };
    }
    return {
        start: mapBoundary(source, selection, selection.start, false, edits),
        end: mapBoundary(source, selection, selection.end, true, edits),
    };
}

function mapBoundary(
    source: string,
    selection: OffsetRange,
    offset: number,
    endBoundary: boolean,
    edits: readonly OffsetEdit[],
): number {
    let delta = 0;
    for (const edit of edits) {
        if (offset < edit.start) {
            break;
        }
        if (offset > edit.end) {
            delta += edit.newText.length - (edit.end - edit.start);
            continue;
        }
        const overlapsSelection = selection.start < edit.end
            && edit.start < selection.end;
        if (overlapsSelection && edit.start <= offset && offset <= edit.end) {
            return edit.start + delta + mapCursorInsideReplacement(
                source.slice(edit.start, edit.end),
                edit.newText,
                offset - edit.start,
            );
        }
        if (offset === edit.start) {
            if (edit.start === edit.end && endBoundary) {
                return edit.start + delta + edit.newText.length;
            }
            return edit.start + delta;
        }
        return edit.start + delta + edit.newText.length;
    }
    return offset + delta;
}

function mapCursor(source: string, offset: number, edits: readonly OffsetEdit[]): number {
    let delta = 0;
    for (const edit of edits) {
        if (offset < edit.start) {
            break;
        }
        if (offset > edit.end) {
            delta += edit.newText.length - (edit.end - edit.start);
            continue;
        }
        if (edit.start === edit.end) {
            return edit.start + delta + edit.newText.length;
        }
        const oldText = source.slice(edit.start, edit.end);
        return edit.start + delta + mapCursorInsideReplacement(
            oldText,
            edit.newText,
            offset - edit.start,
        );
    }
    return offset + delta;
}

function mapCursorInsideReplacement(
    oldText: string,
    newText: string,
    relativeOffset: number,
): number {
    const wrappedAt = newText.indexOf(oldText);
    if (wrappedAt >= 0 && newText.indexOf(oldText, wrappedAt + 1) < 0) {
        return wrappedAt + relativeOffset;
    }

    const retainedAt = oldText.indexOf(newText);
    if (newText.length > 0
        && retainedAt >= 0
        && oldText.indexOf(newText, retainedAt + 1) < 0
    ) {
        return Math.max(0, Math.min(newText.length, relativeOffset - retainedAt));
    }

    let commonPrefix = 0;
    while (commonPrefix < oldText.length
        && commonPrefix < newText.length
        && oldText[commonPrefix] === newText[commonPrefix]
    ) {
        commonPrefix += 1;
    }
    if (relativeOffset <= commonPrefix) {
        return relativeOffset;
    }

    let commonSuffix = 0;
    while (commonSuffix < oldText.length - commonPrefix
        && commonSuffix < newText.length - commonPrefix
        && oldText[oldText.length - commonSuffix - 1]
            === newText[newText.length - commonSuffix - 1]
    ) {
        commonSuffix += 1;
    }
    if (relativeOffset >= oldText.length - commonSuffix) {
        return newText.length - (oldText.length - relativeOffset);
    }
    return newText.length;
}
