import * as vscode from 'vscode';

const extensionId = 'LiveSplit.splitscript';

export async function run(): Promise<void> {
    const extension = vscode.extensions.getExtension(extensionId);
    assert(extension !== undefined, `extension ${extensionId} is not installed`);
    await extension.activate();

    const workspace = vscode.workspace.workspaceFolders?.[0];
    assert(workspace !== undefined, 'the web test has no virtual workspace');
    assert(
        workspace.uri.scheme === 'vscode-test-web',
        `expected a vscode-test-web workspace, got ${workspace.uri.scheme}`,
    );

    const script = vscode.Uri.joinPath(workspace.uri, 'web.split');
    const document = await vscode.workspace.openTextDocument(script);
    const sourceEditor = await vscode.window.showTextDocument(document);

    const hoverPosition = document.positionAt(document.getText().indexOf('double') + 1);
    await hoverAt(script, hoverPosition);

    await vscode.commands.executeCommand('splitscript.restartLanguageServer');
    await hoverAt(script, hoverPosition);

    const inspectedText = 'double(21)';
    const inspectedStart = document.getText().indexOf(inspectedText);
    const inspectedRange = new vscode.Range(
        document.positionAt(inspectedStart),
        document.positionAt(inspectedStart + inspectedText.length),
    );
    sourceEditor.selection = new vscode.Selection(inspectedRange.start, inspectedRange.end);
    const inspectAction = await waitFor(async () => {
        const actions = await vscode.commands.executeCommand<readonly vscode.CodeAction[]>(
            'vscode.executeCodeActionProvider',
            script,
            inspectedRange,
            vscode.CodeActionKind.RefactorRewrite.value,
        );
        return actions?.find(action =>
            action.kind?.value === 'refactor.rewrite.inspect.add'
        );
    }, 'the language server returned no inspect refactoring');
    assert(inspectAction.edit !== undefined, 'the inspect refactoring has no workspace edit');
    assert(inspectAction.command !== undefined, 'the inspect refactoring cannot restore selection');
    assert(await vscode.workspace.applyEdit(inspectAction.edit), 'could not apply inspect refactoring');
    await vscode.commands.executeCommand(
        inspectAction.command.command,
        ...(inspectAction.command.arguments ?? []),
    );
    assert(
        document.getText(sourceEditor.selection) === inspectedText,
        'the inspect refactoring did not preserve the selected expression',
    );
    const restore = new vscode.WorkspaceEdit();
    const rewrittenStart = document.getText().indexOf('inspect(double(21))');
    restore.replace(
        script,
        new vscode.Range(
            document.positionAt(rewrittenStart),
            document.positionAt(rewrittenStart + 'inspect(double(21))'.length),
        ),
        inspectedText,
    );
    assert(await vscode.workspace.applyEdit(restore), 'could not restore inspect test source');

    const printPosition = document.positionAt(document.getText().indexOf('print') + 1);
    const printHovers = await hoverAt(script, printPosition);
    const printMarkdown = printHovers.flatMap(hover => hover.contents)
        .filter(
            (content): content is vscode.MarkdownString =>
                content instanceof vscode.MarkdownString,
        )
        .map(content => content.value)
        .join('\n');
    assert(
        printMarkdown.includes(
            'command:splitscript.openDocumentation?%5B%22%2Fstdlib%2Ffunctions%2Fprint.md%22%5D',
        ),
        'catalog hover does not link to its exact compiler-owned documentation page',
    );
    const printDocumentationUri = vscode.Uri.parse(
        'splitscript-docs:/stdlib/functions/print.md',
    );
    assert(
        !vscode.workspace.textDocuments.some(
            candidate => candidate.uri.toString() === printDocumentationUri.toString(),
        ),
        'print documentation was unexpectedly open before the contextual command',
    );
    sourceEditor.selection = new vscode.Selection(printPosition, printPosition);
    await vscode.commands.executeCommand('splitscript.openSymbolDocumentation');
    await waitFor(
        async () => vscode.workspace.textDocuments.some(
            candidate => candidate.uri.toString() === printDocumentationUri.toString(),
        ) ? true : undefined,
        'the contextual documentation command did not open the symbol page',
    );
    await vscode.commands.executeCommand(
        'splitscript.openDocumentation',
        '/stdlib/functions/print.md',
    );
    const printDocumentation = await vscode.workspace.openTextDocument(
        printDocumentationUri,
    );
    assert(
        printDocumentation.getText().includes('# print'),
        'the documentation command did not open the requested symbol page',
    );

    await vscode.commands.executeCommand('splitscript.openDocumentation');
    await waitFor(
        async () => (vscode.window.tabGroups.all.length >= 2 ? true : undefined),
        'standard-library documentation did not open beside the source editor',
    );
    await vscode.window.showTextDocument(document, vscode.ViewColumn.One);

    const durationDocs = vscode.Uri.parse(
        'splitscript-docs:/stdlib/types/Duration/index.md',
    );
    const documentation = await vscode.workspace.openTextDocument(durationDocs);
    assert(documentation.languageId === 'markdown', 'virtual documentation is not Markdown');
    assert(
        documentation.getText().includes('# Duration')
            && documentation.getText().includes(
                '[fromSeconds](methods/fromSeconds.md)',
            ),
        'the bundled language server returned incomplete standard-library documentation',
    );
    const documentationIndex = await vscode.workspace.openTextDocument(
        vscode.Uri.parse('splitscript-docs:/index.md'),
    );
    assert(
        documentationIndex.getText().includes(
            '[Duration](stdlib/types/Duration/index.md)',
        ),
        'the documentation index does not use virtual-document-relative Markdown links',
    );
    assert(
        !documentationIndex.getText().includes('Duration.fromSeconds'),
        'the documentation index includes members that belong on type pages',
    );
    const methodDocs = await vscode.workspace.openTextDocument(
        vscode.Uri.parse(
            'splitscript-docs:/stdlib/types/Duration/methods/fromSeconds.md',
        ),
    );
    assert(
        methodDocs.getText().includes(
            '[SplitScript reference](../../../../index.md) / [Duration](../index.md) / fromSeconds',
        ),
        'the documentation method does not follow its logical owner hierarchy',
    );
    const asyncDocs = await vscode.workspace.openTextDocument(
        vscode.Uri.parse('splitscript-docs:/language/async.md'),
    );
    const asyncMarkdown = asyncDocs.getText();
    for (const keyword of ['fn', 'async', 'let', 'await', 'return']) {
        assert(
            asyncMarkdown.includes(`href="${keyword}.md"`),
            `the async example does not link the ${keyword} keyword`,
        );
    }
    assert(
        asyncMarkdown.includes('[`await`](await.md)')
            && asyncMarkdown.includes('[`retry`](retry.md)'),
        'rustdoc-style prose links did not resolve to language pages',
    );

    const output = script.with({ path: script.path.replace(/\.split$/, '.wasm') });
    await vscode.commands.executeCommand('splitscript.buildRelease');
    const release = await readWasm(output);

    await vscode.commands.executeCommand('splitscript.startDebugWatch');
    try {
        const debug = await waitFor(async () => {
            const bytes = await tryRead(output);
            return bytes !== undefined && !bytesEqual(bytes, release) ? bytes : undefined;
        }, 'debug watch did not replace the release module');

        const closingBrace = document.getText().lastIndexOf('}');
        const edit = new vscode.WorkspaceEdit();
        edit.insert(
            script,
            document.positionAt(closingBrace),
            '    debug print("web watch rebuild")\n',
        );
        assert(await vscode.workspace.applyEdit(edit), 'could not edit the virtual document');
        assert(await document.save(), 'could not save the virtual document');

        await waitFor(async () => {
            const bytes = await tryRead(output);
            return bytes !== undefined && !bytesEqual(bytes, debug) ? true : undefined;
        }, 'saving did not rebuild the debug module');
    } finally {
        await vscode.commands.executeCommand('splitscript.stopDebugWatch');
    }
}

async function hoverAt(
    uri: vscode.Uri,
    position: vscode.Position,
): Promise<readonly vscode.Hover[]> {
    return waitFor(async () => {
        const hovers = await vscode.commands.executeCommand<readonly vscode.Hover[]>(
            'vscode.executeHoverProvider',
            uri,
            position,
        );
        return hovers !== undefined && hovers.length > 0 ? hovers : undefined;
    }, 'the browser language server returned no hover');
}

async function readWasm(uri: vscode.Uri): Promise<Uint8Array> {
    const bytes = await waitFor(
        () => tryRead(uri),
        `compiler did not create ${uri.toString(true)}`,
    );
    assert(
        bytes.length >= 4
            && bytes[0] === 0
            && bytes[1] === 0x61
            && bytes[2] === 0x73
            && bytes[3] === 0x6d,
        'compiler output is not a WebAssembly module',
    );
    return bytes;
}

async function tryRead(uri: vscode.Uri): Promise<Uint8Array | undefined> {
    try {
        return await vscode.workspace.fs.readFile(uri);
    } catch {
        return undefined;
    }
}

async function waitFor<T>(
    query: () => Promise<T | undefined>,
    failure: string,
    timeoutMilliseconds = 30_000,
): Promise<T> {
    const deadline = Date.now() + timeoutMilliseconds;
    while (Date.now() < deadline) {
        const value = await query();
        if (value !== undefined) {
            return value;
        }
        await new Promise(resolve => setTimeout(resolve, 50));
    }
    throw new Error(failure);
}

function bytesEqual(left: Uint8Array, right: Uint8Array): boolean {
    return left.length === right.length && left.every((byte, index) => byte === right[index]);
}

function assert(condition: unknown, message: string): asserts condition {
    if (!condition) {
        throw new Error(message);
    }
}
