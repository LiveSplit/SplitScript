import { Worker } from 'node:worker_threads';
import * as vscode from 'vscode';
import {
    LanguageClient,
    LanguageClientOptions,
    ServerOptions,
} from 'vscode-languageclient/node';
import { PortMessageReader, PortMessageWriter } from 'vscode-jsonrpc/node';
import { errorMessage } from './paths';
import { documentationMarkdownTrust } from './documentationMarkdown';
import { provideDocumentFormattingEdits } from './formatting';
import { provideCodeActionsWithMappedSelection } from './codeActionSelection';

export class LanguageClientController implements vscode.Disposable {
    private client: LanguageClient | undefined;
    private worker: Worker | undefined;

    public constructor(private readonly context: vscode.ExtensionContext) {}

    public async start(): Promise<void> {
        const serverOptions: ServerOptions = async () => {
            const worker = await this.createWorker();
            this.worker = worker;
            return {
                reader: new PortMessageReader(worker),
                writer: new PortMessageWriter(worker),
            };
        };
        let client: LanguageClient;
        const clientOptions: LanguageClientOptions = {
            documentSelector: [{ language: 'splitscript' }],
            markdown: documentationMarkdownTrust,
            middleware: {
                provideCodeActions: provideCodeActionsWithMappedSelection,
                provideDocumentFormattingEdits: (document, options, token) =>
                    provideDocumentFormattingEdits(
                        document,
                        options,
                        token,
                        (method, params, requestToken) => client.sendRequest(
                            method,
                            params,
                            requestToken,
                        ),
                    ),
            },
        };
        client = new LanguageClient(
            'splitscript',
            'SplitScript Language Server',
            serverOptions,
            clientOptions,
        );
        this.client = client;
        try {
            await client.start();
        } catch (error) {
            if (this.client === client) {
                this.client = undefined;
            }
            await this.stopWorker();
            const choice = await vscode.window.showErrorMessage(
                `Could not start the bundled SplitScript language server: ${errorMessage(error)}`,
                'Restart Language Server',
            );
            if (choice === 'Restart Language Server') {
                await this.restart();
            }
        }
    }

    public async restart(): Promise<void> {
        await this.stop();
        await this.start();
    }

    public async sendRequest<TResult>(method: string, params?: unknown): Promise<TResult> {
        const client = this.client;
        if (client === undefined) {
            throw new Error('the SplitScript language server is not running');
        }
        return client.sendRequest<TResult>(method, params);
    }

    public async stop(): Promise<void> {
        const running = this.client;
        this.client = undefined;
        if (running !== undefined) {
            try {
                await running.stop();
            } finally {
                await this.stopWorker();
            }
        } else {
            await this.stopWorker();
        }
    }

    public dispose(): void {
        void this.stop();
    }

    private async createWorker(): Promise<Worker> {
        await this.stopWorker();
        const moduleUri = vscode.Uri.joinPath(
            this.context.extensionUri,
            'dist',
            'splitscript_vscode_wasm.wasm',
        );
        const moduleBytes = await vscode.workspace.fs.readFile(moduleUri);
        const ownedModuleBytes = new Uint8Array(moduleBytes.length);
        ownedModuleBytes.set(moduleBytes);
        const worker = new Worker(
            this.context.asAbsolutePath('dist/embeddedLanguageServerNodeWorker.js'),
            {
                workerData: { moduleBytes: ownedModuleBytes.buffer },
                transferList: [ownedModuleBytes.buffer],
            },
        );
        try {
            await waitUntilReady(worker);
            return worker;
        } catch (error) {
            await worker.terminate();
            throw error;
        }
    }

    private async stopWorker(): Promise<void> {
        const worker = this.worker;
        this.worker = undefined;
        if (worker !== undefined) {
            await worker.terminate();
        }
    }
}

function waitUntilReady(worker: Worker): Promise<void> {
    return new Promise((resolve, reject) => {
        const onMessage = (message: unknown): void => {
            if (
                typeof message === 'object'
                && message !== null
                && 'kind' in message
                && message.kind === 'ready'
            ) {
                cleanup();
                resolve();
            }
        };
        const onError = (error: Error): void => {
            cleanup();
            reject(error);
        };
        const onExit = (code: number): void => {
            cleanup();
            reject(new Error(`embedded language-server worker exited with code ${code}`));
        };
        const cleanup = (): void => {
            worker.off('message', onMessage);
            worker.off('error', onError);
            worker.off('exit', onExit);
        };
        worker.on('message', onMessage);
        worker.once('error', onError);
        worker.once('exit', onExit);
    });
}
