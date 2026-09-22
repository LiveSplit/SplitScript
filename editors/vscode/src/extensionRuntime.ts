import * as vscode from 'vscode';
import { CompilerTaskController } from './compilerTasks';
import { DocumentationReferenceController } from './documentationReference';
import {
    RESTORE_CODE_ACTION_SELECTION,
    restoreCodeActionSelection,
} from './codeActionSelection';

export interface LanguageClientLifecycle extends vscode.Disposable {
    start(): Promise<void>;
    restart(): Promise<void>;
    stop(): Promise<void>;
    sendRequest<TResult>(method: string, params?: unknown): Promise<TResult>;
}

/** Shared activation wiring for the Node and browser extension hosts. */
export class ExtensionRuntime {
    public constructor(
        private readonly compilerTasks: CompilerTaskController,
        private readonly languageClient: LanguageClientLifecycle,
    ) {}

    public async activate(context: vscode.ExtensionContext): Promise<void> {
        const documentation = new DocumentationReferenceController(this.languageClient);
        context.subscriptions.push(
            this.compilerTasks,
            this.languageClient,
            documentation,
            vscode.commands.registerCommand(
                RESTORE_CODE_ACTION_SELECTION,
                restoreCodeActionSelection,
            ),
            vscode.commands.registerCommand(
                'splitscript.restartLanguageServer',
                async () => this.languageClient.restart(),
            ),
            vscode.commands.registerCommand(
                'splitscript.buildRelease',
                async () => this.compilerTasks.buildRelease(),
            ),
            vscode.commands.registerCommand(
                'splitscript.startDebugWatch',
                async () => this.compilerTasks.startDebugWatch(),
            ),
            vscode.commands.registerCommand(
                'splitscript.stopDebugWatch',
                async () => this.compilerTasks.stopDebugWatch(true),
            ),
        );
        await this.compilerTasks.initialize();
        await this.languageClient.start();
        documentation.register();
    }

    public async deactivate(): Promise<void> {
        await this.compilerTasks.stopDebugWatch(false);
        await this.languageClient.stop();
    }
}
