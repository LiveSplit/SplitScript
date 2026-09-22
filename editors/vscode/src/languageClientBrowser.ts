import * as vscode from 'vscode';
import {
    LanguageClient,
    LanguageClientOptions,
    MessageTransports,
    ServerOptions,
} from 'vscode-languageclient/browser';
import {
    BrowserMessageReader,
    BrowserMessageWriter,
} from 'vscode-jsonrpc/browser';
import { errorMessage } from './paths';
import { documentationMarkdownTrust } from './documentationMarkdown';
import { provideDocumentFormattingEdits } from './formatting';
import { provideCodeActionsWithMappedSelection } from './codeActionSelection';

export class BrowserLanguageClientController implements vscode.Disposable {
    private client: LanguageClient | undefined;
    private worker: Worker | undefined;

    public constructor(private readonly context: vscode.ExtensionContext) {}

    public async start(): Promise<void> {
        const serverOptions: ServerOptions = async () => {
            const worker = await this.createWorker();
            this.worker = worker;
            // VS Code's web extension host can expose a Worker from a different
            // realm, so vscode-languageclient's `instanceof Worker` dispatch is
            // not reliable here. Construct the transport explicitly.
            return workerTransports(worker);
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
            this.stopWorker();
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
                this.stopWorker();
            }
        } else {
            this.stopWorker();
        }
    }

    public dispose(): void {
        void this.stop();
    }

    private async createWorker(): Promise<Worker> {
        this.stopWorker();
        const workerUri = vscode.Uri.joinPath(
            this.context.extensionUri,
            'dist',
            'web',
            'embeddedLanguageServerWorker.js',
        );
        const worker = new Worker(workerUri.toString(true));
        const moduleUri = vscode.Uri.joinPath(
            this.context.extensionUri,
            'dist',
            'splitscript_vscode_wasm.wasm',
        );
        const moduleBytes = await vscode.workspace.fs.readFile(moduleUri);
        const ownedModuleBytes = new Uint8Array(moduleBytes.length);
        ownedModuleBytes.set(moduleBytes);
        try {
            worker.postMessage(
                { kind: 'initialize', moduleBytes: ownedModuleBytes.buffer },
                [ownedModuleBytes.buffer],
            );
            await waitUntilReady(worker);
            return worker;
        } catch (error) {
            worker.terminate();
            throw error;
        }
    }

    private stopWorker(): void {
        this.worker?.terminate();
        this.worker = undefined;
    }
}

function workerTransports(worker: Worker): MessageTransports {
    return {
        reader: new BrowserMessageReader(worker),
        writer: new BrowserMessageWriter(worker),
    };
}

function waitUntilReady(worker: Worker): Promise<void> {
    return new Promise((resolve, reject) => {
        const onMessage = (event: MessageEvent<unknown>): void => {
            const message = event.data;
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
        const onError = (event: ErrorEvent): void => {
            cleanup();
            reject(new Error(event.message || 'embedded language-server worker failed'));
        };
        const cleanup = (): void => {
            worker.removeEventListener('message', onMessage);
            worker.removeEventListener('error', onError);
        };
        worker.addEventListener('message', onMessage);
        worker.addEventListener('error', onError);
    });
}
