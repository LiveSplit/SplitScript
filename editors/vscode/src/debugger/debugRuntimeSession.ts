import * as path from 'node:path';

import * as vscode from 'vscode';

import {
    compilerServiceProtocolVersion,
    type EmbeddedCompileResponse,
} from '../embeddedCompiler';
import type { EmbeddedCompilerClient } from '../compilerTasks';
import { RuntimeClient } from './runtimeClient';
import type {
    ProcessMemoryRange,
    RuntimeLogMessage,
    RuntimeMemoryRead,
    RuntimeMemoryTarget,
    RuntimeSnapshot,
} from './runtimeProtocol';

export interface SplitScriptLaunchConfiguration extends vscode.DebugConfiguration {
    type: 'splitscript';
    request: 'launch';
    program: string;
    hotReload?: boolean;
}

export interface DebugRuntimeSessionCallbacks {
    snapshot(snapshot: RuntimeSnapshot | undefined): void;
    log(message: RuntimeLogMessage): void;
    failure(error: Error): void;
    memoryReset(): void;
}

export class DebugRuntimeSession implements vscode.Disposable {
    private readonly runtime: RuntimeClient;
    private compiler: EmbeddedCompilerClient | undefined;
    private programUri: vscode.Uri | undefined;
    private hotReload = true;
    private reloadChain = Promise.resolve();
    private readonly saveSubscription: vscode.Disposable;
    private programWatcher: vscode.FileSystemWatcher | undefined;
    private fileReloadTimer: ReturnType<typeof setTimeout> | undefined;
    private stopped = false;
    private suppressSaveReload = false;

    public constructor(
        workerPath: string,
        nativeModulePath: string,
        private readonly createCompiler: () => Promise<EmbeddedCompilerClient>,
        private readonly callbacks: DebugRuntimeSessionCallbacks,
    ) {
        this.runtime = new RuntimeClient(workerPath, nativeModulePath, callbacks);
        this.saveSubscription = vscode.workspace.onDidSaveTextDocument(document => {
            if (
                this.hotReload
                && !this.suppressSaveReload
                && this.programUri !== undefined
                && document.uri.toString() === this.programUri.toString()
            ) {
                this.scheduleFileReload(this.programUri);
            }
        });
    }

    public async launch(configuration: SplitScriptLaunchConfiguration): Promise<void> {
        if (!vscode.workspace.isTrusted) {
            throw new Error('Auto Splitter debugging requires a trusted workspace.');
        }
        const uri = programUri(configuration.program);
        if (uri.scheme !== 'file') {
            throw new Error('Auto Splitter debugging currently requires a local file.');
        }
        const sourceProgram = uri.path.toLowerCase().endsWith('.split');
        this.hotReload = configuration.hotReload ?? true;
        this.compiler = sourceProgram ? await this.createCompiler() : undefined;
        const artifact = await this.buildArtifact(uri);
        this.programUri = uri;
        this.callbacks.memoryReset();
        await this.runtime.launch(artifact, uri.fsPath);
        if (this.hotReload) {
            this.watchProgramFile(uri);
        }
    }

    public reload(): Promise<void> {
        const uri = this.programUri;
        if (uri === undefined || this.stopped) {
            return Promise.resolve();
        }
        const operation = this.reloadChain.then(async () => {
            this.callbacks.log(runtimeLog('debug', `Rebuilding ${uri.fsPath}`));
            const artifact = await this.buildArtifact(uri);
            this.callbacks.snapshot(undefined);
            this.callbacks.memoryReset();
            await this.runtime.launch(artifact, uri.fsPath);
            this.callbacks.log(runtimeLog('info', `Reloaded ${uri.fsPath}`));
        });
        this.reloadChain = operation.catch(() => {});
        return operation;
    }

    public timerCommand(command: 'start' | 'reset'): void {
        this.runtime.timerCommand(command);
    }

    public pause(): Promise<void> {
        return this.runtime.setPaused(true);
    }

    public resume(): Promise<void> {
        return this.runtime.setPaused(false);
    }

    public setSetting(key: string, value: boolean | string): void {
        this.runtime.setSetting(key, value);
    }

    public clearSettings(): void {
        this.runtime.clearSettings();
    }

    public resetStatistics(): void {
        this.runtime.resetStatistics();
    }

    public readMemory(
        target: RuntimeMemoryTarget,
        offset: number,
        count: number,
    ): Promise<RuntimeMemoryRead> {
        return this.runtime.readMemory(target, offset, count);
    }

    public listProcessMemoryRanges(handle: string): Promise<ProcessMemoryRange[]> {
        return this.runtime.listProcessMemoryRanges(handle);
    }

    public async stop(): Promise<void> {
        if (this.stopped) {
            return;
        }
        this.stopped = true;
        this.saveSubscription.dispose();
        this.programWatcher?.dispose();
        this.programWatcher = undefined;
        if (this.fileReloadTimer !== undefined) {
            clearTimeout(this.fileReloadTimer);
            this.fileReloadTimer = undefined;
        }
        this.compiler?.dispose();
        this.compiler = undefined;
        await this.runtime.terminate();
        this.callbacks.snapshot(undefined);
    }

    public dispose(): void {
        void this.stop();
    }

    private async buildArtifact(uri: vscode.Uri): Promise<Uint8Array> {
        if (uri.path.toLowerCase().endsWith('.wasm')) {
            const file = await vscode.workspace.fs.readFile(uri);
            const artifact = new Uint8Array(file.length);
            artifact.set(file);
            if (!WebAssembly.validate(artifact)) {
                throw new Error(`The WebAssembly module at ${uri.fsPath} is not valid.`);
            }
            return artifact;
        }
        if (!uri.path.toLowerCase().endsWith('.split')) {
            throw new Error('The debug program must be a .split or .wasm file.');
        }
        const compiler = this.compiler;
        if (compiler === undefined) {
            throw new Error('the embedded debug compiler is not initialized');
        }
        const document = await vscode.workspace.openTextDocument(uri);
        if (document.isDirty) {
            this.suppressSaveReload = true;
            try {
                if (!await document.save()) {
                    throw new Error(`Could not save ${uri.fsPath} before compiling it.`);
                }
            } finally {
                this.suppressSaveReload = false;
            }
        }
        // A filesystem watcher may observe an external write before VS Code has
        // refreshed an already-open TextDocument. Read the saved source from
        // disk so hot reload always compiles the change that triggered it.
        const savedSource = new TextDecoder().decode(await vscode.workspace.fs.readFile(uri));
        const response = await compiler.compile({
            protocolVersion: compilerServiceProtocolVersion,
            uri: uri.toString(),
            sourcePath: uri.fsPath,
            revision: document.version,
            source: savedSource,
            profile: 'debug',
        });
        this.reportDiagnostics(response);
        if (response.artifact === undefined) {
            throw new Error(`SplitScript debug compilation failed for ${uri.fsPath}.`);
        }
        return response.artifact;
    }

    private reportDiagnostics(response: EmbeddedCompileResponse): void {
        for (const diagnostic of response.diagnostics) {
            const level = diagnostic.severity === 'error'
                ? 'error'
                : diagnostic.severity === 'warning'
                    ? 'warning'
                    : 'info';
            this.callbacks.log(runtimeLog(
                level,
                `${diagnostic.severity}[${diagnostic.code}]: ${diagnostic.message}`,
            ));
        }
    }

    private watchProgramFile(uri: vscode.Uri): void {
        this.programWatcher?.dispose();
        const watcher = vscode.workspace.createFileSystemWatcher(
            new vscode.RelativePattern(path.dirname(uri.fsPath), '*'),
            false,
            false,
            true,
        );
        const changed = (changedUri: vscode.Uri): void => {
            if (sameFile(changedUri, uri)) {
                this.scheduleFileReload(uri);
            }
        };
        watcher.onDidChange(changed);
        // Compilers commonly replace their output atomically, which appears as
        // deletion followed by creation rather than an in-place change.
        watcher.onDidCreate(changed);
        this.programWatcher = watcher;
    }

    private scheduleFileReload(uri: vscode.Uri): void {
        if (this.fileReloadTimer !== undefined) {
            clearTimeout(this.fileReloadTimer);
        }
        // File watchers may report while a compiler is still replacing the
        // module and often emit several events for one build.
        this.fileReloadTimer = setTimeout(() => {
            this.fileReloadTimer = undefined;
            this.runFileHotReload(uri);
        }, 100);
    }

    private runFileHotReload(uri: vscode.Uri): void {
        const openDocument = vscode.workspace.textDocuments.find(
            document => sameFile(document.uri, uri),
        );
        if (openDocument?.isDirty) {
            this.callbacks.log(runtimeLog(
                'warning',
                `Skipped hot reload for ${uri.fsPath} because it has unsaved editor changes.`,
            ));
            return;
        }
        this.runHotReload();
    }

    private runHotReload(): void {
        void this.reload().catch(error => {
            this.callbacks.log(runtimeLog(
                'error',
                `Hot reload failed: ${asError(error).message}`,
            ));
        });
    }
}

function sameFile(left: vscode.Uri, right: vscode.Uri): boolean {
    const leftPath = path.resolve(left.fsPath);
    const rightPath = path.resolve(right.fsPath);
    return process.platform === 'win32'
        ? leftPath.toLowerCase() === rightPath.toLowerCase()
        : leftPath === rightPath;
}

function programUri(program: string): vscode.Uri {
    return /^[a-z][a-z0-9+.-]*:\/\//i.test(program)
        ? vscode.Uri.parse(program)
        : vscode.Uri.file(program);
}

function runtimeLog(
    level: RuntimeLogMessage['level'],
    message: string,
): RuntimeLogMessage {
    return {
        type: 'log',
        timestamp: new Date().toISOString(),
        source: 'runtime',
        level,
        message,
    };
}

function asError(error: unknown): Error {
    return error instanceof Error ? error : new Error(String(error));
}
