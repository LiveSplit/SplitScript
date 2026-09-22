import * as vscode from 'vscode';
import type { ProvideCodeActionsSignature } from 'vscode-languageclient';
import {
    mapSelectionThroughEdits,
    type OffsetEdit,
} from './codeActionSelectionMapping';

export const RESTORE_CODE_ACTION_SELECTION =
    'splitscript.internal.restoreCodeActionSelection';

interface RestoreSelectionArguments {
    uri: string;
    start: number;
    end: number;
    subsequentCommand?: vscode.Command;
}

/** Adds a post-edit selection to every language-server code action. */
export async function provideCodeActionsWithMappedSelection(
    document: vscode.TextDocument,
    range: vscode.Range,
    context: vscode.CodeActionContext,
    token: vscode.CancellationToken,
    next: ProvideCodeActionsSignature,
): Promise<(vscode.Command | vscode.CodeAction)[] | null | undefined> {
    const actions = await next(document, range, context, token);
    if (actions === null || actions === undefined || token.isCancellationRequested) {
        return actions;
    }

    const source = document.getText();
    const selection = {
        start: document.offsetAt(range.start),
        end: document.offsetAt(range.end),
    };
    for (const action of actions) {
        if (!isCodeAction(action) || action.edit === undefined) {
            continue;
        }
        const edits = editsForDocument(document, action.edit);
        if (edits.length === 0) {
            continue;
        }
        const mapped = mapSelectionThroughEdits(source, selection, edits);
        action.command = {
            command: RESTORE_CODE_ACTION_SELECTION,
            title: 'Restore selection after edit',
            arguments: [{
                uri: document.uri.toString(),
                start: mapped.start,
                end: mapped.end,
                subsequentCommand: action.command,
            } satisfies RestoreSelectionArguments],
        };
    }
    return actions;
}

export async function restoreCodeActionSelection(
    arguments_: RestoreSelectionArguments,
): Promise<void> {
    if (arguments_.subsequentCommand !== undefined) {
        await vscode.commands.executeCommand(
            arguments_.subsequentCommand.command,
            ...(arguments_.subsequentCommand.arguments ?? []),
        );
    }

    const uri = vscode.Uri.parse(arguments_.uri);
    const editor = vscode.window.visibleTextEditors.find(candidate =>
        candidate.document.uri.toString() === uri.toString()
    );
    if (editor === undefined) {
        return;
    }
    const length = editor.document.getText().length;
    const start = editor.document.positionAt(Math.min(arguments_.start, length));
    const end = editor.document.positionAt(Math.min(arguments_.end, length));
    editor.selection = new vscode.Selection(start, end);
    editor.revealRange(editor.selection, vscode.TextEditorRevealType.InCenterIfOutsideViewport);
}

function isCodeAction(
    action: vscode.Command | vscode.CodeAction,
): action is vscode.CodeAction {
    return typeof action.command !== 'string';
}

function editsForDocument(
    document: vscode.TextDocument,
    workspaceEdit: vscode.WorkspaceEdit,
): OffsetEdit[] {
    const documentUri = document.uri.toString();
    return workspaceEdit.entries()
        .filter(([uri]) => uri.toString() === documentUri)
        .flatMap(([, edits]) => edits.map(edit => ({
            start: document.offsetAt(edit.range.start),
            end: document.offsetAt(edit.range.end),
            newText: edit.newText,
        })));
}
