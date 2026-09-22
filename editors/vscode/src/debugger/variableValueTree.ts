export interface VariableValueLine {
    readonly text: string;
    readonly children: readonly VariableValueLine[];
}

export interface VariableValuePresentation {
    readonly summary: string;
    readonly children: readonly VariableValueLine[];
}

/**
 * Converts the compiler's indentation-based structural Debug output into a
 * tree suitable for VS Code's single-line tree rows. This intentionally does
 * not interpret values: quoted strings, map keys, and future Debug formats
 * remain exact text instead of being mistaken for protocol data.
 */
export function presentVariableValue(value: string): VariableValuePresentation {
    const lines = value.replaceAll('\r\n', '\n').replaceAll('\r', '\n').split('\n');
    if (lines.length === 1) return { summary: value, children: [] };

    const first = lines[0]!.trim();
    const children: MutableVariableValueLine[] = [];
    const stack: Array<{ indent: number; children: MutableVariableValueLine[] }> = [
        { indent: -1, children },
    ];

    for (const sourceLine of lines.slice(1)) {
        const text = sourceLine.trim();
        if (text === '' || isClosingDelimiter(text)) continue;

        const indent = indentation(sourceLine);
        while (stack.length > 1 && indent <= stack.at(-1)!.indent) stack.pop();
        const node: MutableVariableValueLine = {
            text: text.endsWith(',') ? text.slice(0, -1) : text,
            children: [],
        };
        stack.at(-1)!.children.push(node);
        stack.push({ indent, children: node.children });
    }

    return {
        summary: structuralSummary(first, lines.at(-1)?.trim()),
        children,
    };
}

interface MutableVariableValueLine {
    text: string;
    children: MutableVariableValueLine[];
}

function indentation(line: string): number {
    let width = 0;
    for (const character of line) {
        if (character === ' ') width += 1;
        else if (character === '\t') width += 4;
        else break;
    }
    return width;
}

function isClosingDelimiter(text: string): boolean {
    return /^[}\])],?$/.test(text);
}

function structuralSummary(first: string, last: string | undefined): string {
    const closing = last?.replace(/,$/, '');
    if (first.endsWith('{') && closing === '}') return `${first} … }`;
    if (first.endsWith('[') && closing === ']') return `${first}…]`;
    if (first.endsWith('(') && closing === ')') return `${first}…)`;
    return `${first} …`;
}
