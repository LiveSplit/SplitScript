import { Worker } from 'node:worker_threads';

import type {
    ProcessMemoryRange,
    RuntimeLogMessage,
    RuntimeMemoryRead,
    RuntimeMemoryTarget,
    RuntimeRequest,
    RuntimeResponse,
    RuntimeSnapshot,
    SettingMapSnapshot,
} from './runtimeProtocol';

export interface RuntimeClientCallbacks {
    snapshot(snapshot: RuntimeSnapshot): void;
    log(message: RuntimeLogMessage): void;
    failure(error: Error): void;
}

export class RuntimeClient {
    private worker: Worker | undefined;
    private intentionalTermination = false;
    private settings: SettingMapSnapshot | undefined;
    private paused = false;
    private nextRequestId = 1;
    private readonly memoryReads = new Map<
        number,
        { resolve(result: RuntimeMemoryRead): void; reject(error: Error): void }
    >();
    private readonly processMemoryRanges = new Map<
        number,
        { resolve(ranges: ProcessMemoryRange[]): void; reject(error: Error): void }
    >();
    private readonly executionChanges = new Map<
        number,
        { resolve(): void; reject(error: Error): void }
    >();

    public constructor(
        private readonly workerPath: string,
        private readonly nativeModulePath: string,
        private readonly callbacks: RuntimeClientCallbacks,
    ) {}

    public async launch(wasm: Uint8Array, program: string): Promise<void> {
        await this.terminate();
        const worker = new Worker(this.workerPath);
        this.worker = worker;
        this.intentionalTermination = false;

        const ready = new Promise<void>((resolve, reject) => {
            const onMessage = (message: RuntimeResponse) => {
                if (message.type === 'ready') {
                    cleanup();
                    resolve();
                } else if (message.type === 'failure') {
                    cleanup();
                    reject(runtimeError(message));
                }
            };
            const onError = (error: Error) => {
                cleanup();
                reject(error);
            };
            const cleanup = () => {
                worker.off('message', onMessage);
                worker.off('error', onError);
            };
            worker.on('message', onMessage);
            worker.on('error', onError);
        });

        worker.on('message', message => this.handleMessage(message as RuntimeResponse));
        worker.on('error', error => this.handleFailure(error));
        worker.on('exit', code => {
            if (this.worker === worker) {
                this.worker = undefined;
            }
            if (!this.intentionalTermination && code !== 0) {
                this.handleFailure(new Error(`ASR runtime worker exited with code ${code}`));
            }
        });

        const owned = new Uint8Array(wasm.length);
        owned.set(wasm);
        this.post({
            type: 'launch',
            wasm: owned.buffer,
            program,
            settings: this.settings,
            nativeModulePath: this.nativeModulePath,
            paused: this.paused,
        }, [owned.buffer]);
        await ready;
    }

    public setPaused(paused: boolean): Promise<void> {
        if (this.worker === undefined) {
            return Promise.reject(new Error('the ASR runtime worker is not running'));
        }
        this.paused = paused;
        const requestId = this.nextRequestId++;
        return new Promise((resolve, reject) => {
            this.executionChanges.set(requestId, { resolve, reject });
            try {
                this.post({ type: 'setExecution', requestId, paused });
            } catch (error) {
                this.executionChanges.delete(requestId);
                reject(asError(error));
            }
        });
    }

    public timerCommand(command: 'start' | 'reset'): void {
        if (this.worker === undefined) {
            return;
        }
        this.post({ type: 'timerCommand', command });
    }

    public setSetting(key: string, value: boolean | string): void {
        if (this.worker !== undefined) this.post({ type: 'setSetting', key, value });
    }

    public clearSettings(): void {
        if (this.worker !== undefined) this.post({ type: 'clearSettings' });
    }

    public resetStatistics(): void {
        if (this.worker !== undefined) this.post({ type: 'resetStatistics' });
    }

    public readMemory(
        target: RuntimeMemoryTarget,
        offset: number,
        count: number,
    ): Promise<RuntimeMemoryRead> {
        if (this.worker === undefined) {
            return Promise.reject(new Error('the ASR runtime worker is not running'));
        }
        const requestId = this.nextRequestId++;
        return new Promise((resolve, reject) => {
            this.memoryReads.set(requestId, { resolve, reject });
            try {
                this.post({ type: 'readMemory', requestId, target, offset, count });
            } catch (error) {
                this.memoryReads.delete(requestId);
                reject(asError(error));
            }
        });
    }

    public listProcessMemoryRanges(handle: string): Promise<ProcessMemoryRange[]> {
        if (this.worker === undefined) {
            return Promise.reject(new Error('the ASR runtime worker is not running'));
        }
        const requestId = this.nextRequestId++;
        return new Promise((resolve, reject) => {
            this.processMemoryRanges.set(requestId, { resolve, reject });
            try {
                this.post({ type: 'listProcessMemoryRanges', requestId, handle });
            } catch (error) {
                this.processMemoryRanges.delete(requestId);
                reject(asError(error));
            }
        });
    }

    public async terminate(): Promise<void> {
        const worker = this.worker;
        if (worker === undefined) {
            return;
        }
        this.intentionalTermination = true;
        this.worker = undefined;
        this.rejectRequests(new Error('the ASR runtime worker stopped before completing the request'));
        const stopped = new Promise<void>(resolve => {
            const onMessage = (message: RuntimeResponse) => {
                if (message.type === 'stopped') {
                    worker.off('message', onMessage);
                    resolve();
                }
            };
            worker.on('message', onMessage);
            setTimeout(() => {
                worker.off('message', onMessage);
                resolve();
            }, 250).unref();
        });
        worker.postMessage({ type: 'shutdown' } satisfies RuntimeRequest);
        await stopped;
        await worker.terminate();
    }

    public dispose(): void {
        void this.terminate();
    }

    private post(message: RuntimeRequest, transfer: readonly ArrayBuffer[] = []): void {
        const worker = this.worker;
        if (worker === undefined) {
            throw new Error('the ASR runtime worker is not running');
        }
        worker.postMessage(message, [...transfer]);
    }

    private handleMessage(message: RuntimeResponse): void {
        if (message.type === 'snapshot') {
            this.settings = message.snapshot.settings.map;
            this.callbacks.snapshot(message.snapshot);
        } else if (message.type === 'log') {
            this.callbacks.log(message);
        } else if (message.type === 'failure') {
            this.handleFailure(runtimeError(message));
        } else if (message.type === 'memoryRead') {
            const pending = this.memoryReads.get(message.requestId);
            this.memoryReads.delete(message.requestId);
            pending?.resolve({
                address: message.address,
                bytes: new Uint8Array(message.bytes),
                unreadableBytes: message.unreadableBytes,
            });
        } else if (message.type === 'processMemoryRanges') {
            const pending = this.processMemoryRanges.get(message.requestId);
            this.processMemoryRanges.delete(message.requestId);
            pending?.resolve(message.ranges);
        } else if (message.type === 'executionChanged') {
            const pending = this.executionChanges.get(message.requestId);
            this.executionChanges.delete(message.requestId);
            this.paused = message.paused;
            pending?.resolve();
        } else if (message.type === 'requestFailure') {
            const error = new Error(message.message);
            const memoryRead = this.memoryReads.get(message.requestId);
            this.memoryReads.delete(message.requestId);
            memoryRead?.reject(error);
            const ranges = this.processMemoryRanges.get(message.requestId);
            this.processMemoryRanges.delete(message.requestId);
            ranges?.reject(error);
            const execution = this.executionChanges.get(message.requestId);
            this.executionChanges.delete(message.requestId);
            execution?.reject(error);
        }
    }

    private handleFailure(error: Error): void {
        this.rejectRequests(error);
        if (!this.intentionalTermination) {
            this.callbacks.failure(error);
        }
    }

    private rejectRequests(error: Error): void {
        for (const pending of this.memoryReads.values()) pending.reject(error);
        this.memoryReads.clear();
        for (const pending of this.processMemoryRanges.values()) pending.reject(error);
        this.processMemoryRanges.clear();
        for (const pending of this.executionChanges.values()) pending.reject(error);
        this.executionChanges.clear();
    }
}

function runtimeError(message: Extract<RuntimeResponse, { type: 'failure' }>): Error {
    const error = new Error(message.message);
    error.name = 'AsrRuntimeError';
    if (message.stack !== undefined) {
        error.stack = message.stack;
    }
    return error;
}

function asError(error: unknown): Error {
    return error instanceof Error ? error : new Error(String(error));
}
