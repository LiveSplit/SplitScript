import * as path from 'node:path';

import * as vscode from 'vscode';

import type { RuntimeSnapshot } from './runtimeProtocol';

export class RuntimeViewProvider implements vscode.TreeDataProvider<vscode.TreeItem>, vscode.Disposable {
    private readonly changes = new vscode.EventEmitter<vscode.TreeItem | undefined>();
    private snapshot: RuntimeSnapshot | undefined;

    public readonly onDidChangeTreeData = this.changes.event;

    public update(snapshot: RuntimeSnapshot | undefined): void {
        this.snapshot = snapshot;
        this.changes.fire(undefined);
    }

    public getTreeItem(element: vscode.TreeItem): vscode.TreeItem {
        return element;
    }

    public getChildren(): vscode.TreeItem[] {
        const snapshot = this.snapshot;
        if (snapshot === undefined) {
            return [];
        }
        return [
            item('Program', path.basename(snapshot.program), 'file-code'),
            item(
                'Execution',
                words(snapshot.status),
                snapshot.status === 'paused' ? 'debug-pause' : 'debug-continue',
            ),
            item(
                'Timer State',
                words(snapshot.timer.state),
                'watch',
                snapshot.timer.state === 'notRunning' ? 'timerNotRunning' : 'timerStarted',
            ),
            item('Game Time', formatDuration(snapshot.timer.gameTimeSeconds)),
            item('Game Time State', words(snapshot.timer.gameTimeState)),
            item('Split Index', String(snapshot.timer.splitIndex)),
        ];
    }

    public dispose(): void {
        this.changes.dispose();
    }
}

function item(
    label: string,
    description: string,
    icon?: string,
    contextValue?: string,
): vscode.TreeItem {
    const value = new vscode.TreeItem(label, vscode.TreeItemCollapsibleState.None);
    value.description = description;
    value.tooltip = `${label}: ${description}`;
    value.contextValue = contextValue;
    if (icon !== undefined) {
        value.iconPath = new vscode.ThemeIcon(icon);
    }
    return value;
}

function formatDuration(seconds: number): string {
    const sign = seconds < 0 ? '-' : '';
    const absolute = Math.abs(seconds);
    const hours = Math.floor(absolute / 3_600);
    const minutes = Math.floor(absolute / 60) % 60;
    const remainder = absolute % 60;
    return `${sign}${hours}:${String(minutes).padStart(2, '0')}:${remainder.toFixed(3).padStart(6, '0')}`;
}

function words(value: string): string {
    return value.replace(/[A-Z]/g, letter => ` ${letter.toLowerCase()}`)
        .replace(/^./, first => first.toUpperCase());
}
