import { parentPort } from 'node:worker_threads';

import { GuestMemory } from './asr/memory';
import { neutralImport } from './asr/neutralImports';
import { loadNativeProcessBridge, ProcessHost } from './asr/process';
import { SettingsHost } from './asr/settings';
import { DebuggerTimer } from './asr/timer';
import { WasiExit, WasiHost } from './asr/wasi';
import type {
    RuntimeLogMessage,
    RuntimeRequest,
    RuntimeResponse,
    RuntimeMemoryRead,
    RuntimeMemoryTarget,
    RuntimeSnapshot,
    SettingMapSnapshot,
} from './runtimeProtocol';
import { TickStatistics } from './runtimeStatistics';

const port = parentPort;
if (port === null) {
    throw new Error('the ASR runtime must run in a Node worker');
}
const workerPort = port;
const SNAPSHOT_INTERVAL_MILLISECONDS = 200;
const RETAINED_TICK_SAMPLES = 2_048;
let host: RuntimeHost | undefined;

workerPort.on('message', (message: RuntimeRequest) => {
    if (message.type === 'launch') {
        if (host !== undefined) {
            fail(new Error('this runtime worker has already launched a module'));
            return;
        }
        host = new RuntimeHost(
            message.program,
            message.settings,
            message.nativeModulePath,
            message.paused ?? false,
        );
        void host.launch(message.wasm).catch(fail);
    } else if (message.type === 'setExecution') {
        if (host === undefined) {
            requestFailure(message.requestId, 'the ASR runtime is not running');
        } else {
            host.setPaused(message.requestId, message.paused);
        }
    } else if (message.type === 'timerCommand') {
        host?.timerCommand(message.command);
    } else if (message.type === 'setSetting') {
        host?.setSetting(message.key, message.value);
    } else if (message.type === 'clearSettings') {
        host?.clearSettings();
    } else if (message.type === 'resetStatistics') {
        host?.resetStatistics();
    } else if (message.type === 'readMemory') {
        if (host === undefined) {
            requestFailure(message.requestId, 'the ASR runtime has not loaded its memory yet');
        } else {
            host.readMemory(
                message.requestId,
                message.target,
                message.offset,
                message.count,
            );
        }
    } else if (message.type === 'listProcessMemoryRanges') {
        if (host === undefined) {
            requestFailure(message.requestId, 'the ASR runtime is not running');
        } else {
            host.listProcessMemoryRanges(message.requestId, message.handle);
        }
    } else if (message.type === 'shutdown') {
        host?.dispose();
        host = undefined;
        workerPort.postMessage({ type: 'stopped' } satisfies RuntimeResponse);
    }
});

class RuntimeHost {
    private readonly memory = new GuestMemory();
    private readonly timer: DebuggerTimer;
    private readonly settings: SettingsHost;
    private readonly wasi: WasiHost;
    private readonly processes: ProcessHost | undefined;
    private readonly nativeBridgeError: string | undefined;
    private status: RuntimeSnapshot['status'] = 'starting';
    private tickRateHz = 120;
    private tickCount = 0;
    private readonly tickStatistics = new TickStatistics(RETAINED_TICK_SAMPLES);
    private lastSnapshotTime = 0;
    private initialize: (() => void) | undefined;
    private update: (() => void) | undefined;
    private tickTimer: NodeJS.Timeout | undefined;

    public constructor(
        private readonly program: string,
        initialSettings: SettingMapSnapshot | undefined,
        nativeModulePath: string | undefined,
        private readonly initiallyPaused: boolean,
    ) {
        this.timer = new DebuggerTimer(
            () => {},
            message => this.emitLog(message),
        );
        this.settings = new SettingsHost(this.memory, initialSettings);
        this.wasi = new WasiHost(this.memory, message => this.emitLog(message));
        let processes: ProcessHost | undefined;
        let nativeBridgeError: string | undefined;
        if (nativeModulePath !== undefined) {
            try {
                processes = new ProcessHost(
                    this.memory,
                    loadNativeProcessBridge(nativeModulePath),
                    message => this.emitLog(message),
                );
            } catch (error) {
                nativeBridgeError = error instanceof Error ? error.message : String(error);
            }
        }
        this.processes = processes;
        this.nativeBridgeError = nativeBridgeError;
    }

    public async launch(bytes: ArrayBuffer): Promise<void> {
        const module = await WebAssembly.compile(bytes);
        const moduleImports = WebAssembly.Module.imports(module);
        const unsupportedImports: string[] = [];
        const imports = this.createImports(moduleImports, unsupportedImports);
        const instance = await WebAssembly.instantiate(module, imports);
        const memory = instance.exports.memory;
        if (memory instanceof WebAssembly.Memory) this.memory.bind(memory);
        const update = instance.exports.update;
        this.update = typeof update === 'function' ? update as () => void : undefined;
        this.initialize = initialEntryPoint(instance.exports, this.update !== undefined);
        this.status = this.initiallyPaused ? 'paused' : 'running';
        workerPort.postMessage({ type: 'ready', unsupportedImports } satisfies RuntimeResponse);
        if (unsupportedImports.length > 0) {
            this.emitLog({
                source: 'runtime',
                level: 'warning',
                message: `Using unavailable-host stubs for: ${unsupportedImports.join(', ')}`,
            });
        }
        if (unsupportedImports.some(name => name.startsWith('env.process_'))
            && this.nativeBridgeError !== undefined) {
            this.emitLog({
                source: 'runtime',
                level: 'error',
                message: `Could not load the native process bridge: ${this.nativeBridgeError}`,
            });
        }
        this.emitLog({
            source: 'runtime',
            level: 'info',
            message: `Loaded ${this.program}`,
        });
        this.emitSnapshot(true);
        if (
            this.status === 'running'
            && (this.initialize !== undefined || this.update !== undefined)
        ) {
            this.scheduleTick(0);
        }
    }

    public setPaused(requestId: number, paused: boolean): void {
        try {
            if (paused) {
                if (this.status === 'running') {
                    this.status = 'paused';
                    if (this.tickTimer !== undefined) {
                        clearTimeout(this.tickTimer);
                        this.tickTimer = undefined;
                    }
                } else if (this.status !== 'paused') {
                    throw new Error('the ASR runtime is not ready to pause');
                }
            } else if (this.status === 'paused') {
                this.status = 'running';
            } else if (this.status !== 'running') {
                throw new Error('the ASR runtime is not ready to resume');
            }

            this.emitSnapshot(true);
            workerPort.postMessage({
                type: 'executionChanged',
                requestId,
                paused,
            } satisfies RuntimeResponse);
            if (
                !paused
                && this.tickTimer === undefined
                && (this.initialize !== undefined || this.update !== undefined)
            ) {
                this.scheduleTick(0);
            }
        } catch (error) {
            requestFailure(requestId, error instanceof Error ? error.message : String(error));
        }
    }

    public timerCommand(command: 'start' | 'reset'): void {
        if (command === 'start') {
            this.timer.start();
        } else {
            this.timer.reset();
        }
        this.emitSnapshot(true);
    }

    public setSetting(key: string, value: boolean | string): void {
        this.settings.set(key, value);
        this.emitSnapshot(true);
    }

    public clearSettings(): void {
        this.settings.clear();
        this.emitSnapshot(true);
    }

    public resetStatistics(): void {
        this.tickStatistics.reset();
        this.emitSnapshot(true);
    }

    public readMemory(
        requestId: number,
        target: RuntimeMemoryTarget,
        offset: number,
        count: number,
    ): void {
        try {
            const result = this.readMemoryTarget(target, offset, count);
            const bytes = ownedBytes(result.bytes);
            workerPort.postMessage({
                type: 'memoryRead',
                requestId,
                address: result.address,
                bytes: bytes.buffer,
                unreadableBytes: result.unreadableBytes,
            } satisfies RuntimeResponse, [bytes.buffer]);
        } catch (error) {
            requestFailure(requestId, error instanceof Error ? error.message : String(error));
        }
    }

    public listProcessMemoryRanges(requestId: number, handle: string): void {
        try {
            const processes = this.processes;
            if (processes === undefined) throw new Error('the native process bridge is unavailable');
            const ranges = processes.memoryRanges(handle);
            workerPort.postMessage({
                type: 'processMemoryRanges',
                requestId,
                ranges,
            } satisfies RuntimeResponse);
        } catch (error) {
            requestFailure(requestId, error instanceof Error ? error.message : String(error));
        }
    }

    public dispose(): void {
        this.status = 'trapped';
        if (this.tickTimer !== undefined) {
            clearTimeout(this.tickTimer);
            this.tickTimer = undefined;
        }
        this.processes?.dispose();
        this.wasi.dispose();
    }

    private scheduleTick(delay: number): void {
        this.tickTimer = setTimeout(() => this.tick(), delay);
    }

    private tick(): void {
        this.tickTimer = undefined;
        const update = this.update;
        const initialize = this.initialize;
        if (this.status !== 'running' || (initialize === undefined && update === undefined)) {
            return;
        }
        const started = performance.now();
        try {
            this.initialize = undefined;
            initialize?.();
            update?.();
        } catch (error) {
            if (error instanceof WasiExit && error.code === 0) {
                const duration = performance.now() - started;
                this.tickCount += 1;
                this.tickStatistics.record(duration);
                this.processes?.dispose();
                this.wasi.dispose();
                this.emitSnapshot(true);
                return;
            }
            this.dispose();
            this.emitSnapshot(true);
            fail(error);
            return;
        }
        const duration = performance.now() - started;
        this.tickCount += 1;
        this.tickStatistics.record(duration);
        this.settings.consumeChanged();
        this.processes?.consumeChanged();
        this.emitSnapshot(false);
        if (update !== undefined) {
            this.scheduleTick(1_000 / this.tickRateHz);
        } else {
            this.emitSnapshot(true);
        }
    }

    private createImports(
        entries: WebAssembly.ModuleImportDescriptor[],
        unsupported: string[],
    ): WebAssembly.Imports {
        const available = this.availableEnvImports();
        const wasi = this.wasi.imports();
        const imports: Record<string, Record<string, WebAssembly.ImportValue>> = {};
        for (const entry of entries) {
            if (entry.kind !== 'function') {
                throw new Error(`unsupported import ${entry.module}.${entry.name} (${entry.kind})`);
            }
            const namespace = imports[entry.module] ??= {};
            const exact = entry.module === 'env'
                ? available[entry.name]
                : entry.module === 'wasi_snapshot_preview1'
                    ? wasi[entry.name]
                    : undefined;
            if (exact !== undefined) {
                namespace[entry.name] = exact;
            } else {
                const qualified = `${entry.module}.${entry.name}`;
                unsupported.push(qualified);
                namespace[entry.name] = neutralImport(qualified);
            }
        }
        return imports;
    }

    private availableEnvImports(): Record<string, WebAssembly.ImportValue> {
        return {
            ...this.settings.imports(),
            ...this.processes?.imports(),
            timer_get_state: () => this.timer.stateNumber(),
            timer_current_split_index: () => this.timer.currentSplitIndex(),
            timer_segment_splitted: (index: bigint) => this.timer.segmentSplitted(index),
            timer_start: () => this.timer.start(),
            timer_split: () => this.timer.split(),
            timer_skip_split: () => this.timer.skipSplit(),
            timer_undo_split: () => this.timer.undoSplit(),
            timer_reset: () => this.timer.reset(),
            timer_set_variable: (
                namePointer: number,
                nameLength: number,
                valuePointer: number,
                valueLength: number,
            ) => this.timer.setVariable(
                this.memory.readString(namePointer, nameLength),
                this.memory.readString(valuePointer, valueLength),
            ),
            timer_set_game_time: (seconds: bigint, nanoseconds: number) => {
                this.timer.setGameTime(seconds, nanoseconds);
            },
            timer_pause_game_time: () => this.timer.pauseGameTime(),
            timer_resume_game_time: () => this.timer.resumeGameTime(),
            runtime_set_tick_rate: (ticksPerSecond: number) => {
                if (!Number.isFinite(ticksPerSecond) || ticksPerSecond <= 0) {
                    throw new WebAssembly.RuntimeError('the tick rate needs to be finite and larger than 0');
                }
                this.tickRateHz = ticksPerSecond;
                this.emitLog({
                    source: 'runtime',
                    level: 'debug',
                    message: `New Tick Rate: ${ticksPerSecond}`,
                });
                this.emitSnapshot(false);
            },
            runtime_print_message: (pointer: number, length: number) => {
                this.emitLog({
                    source: 'autoSplitter',
                    level: 'info',
                    message: this.memory.readString(pointer, length),
                });
            },
            runtime_get_os: (pointer: number, lengthPointer: number) => {
                return this.memory.writeHostString(pointer, lengthPointer, asrOperatingSystem());
            },
            runtime_get_arch: (pointer: number, lengthPointer: number) => {
                return this.memory.writeHostString(pointer, lengthPointer, asrArchitecture());
            },
        };
    }

    private readMemoryTarget(
        target: RuntimeMemoryTarget,
        offset: number,
        count: number,
    ): RuntimeMemoryRead {
        if (!Number.isSafeInteger(offset)) throw new Error('the memory offset must be a safe integer');
        if (!Number.isSafeInteger(count) || count < 0 || count > 128 * 1_024) {
            throw new Error('the memory read size must be between 0 and 131072 bytes');
        }
        if (offset < 0) throw new Error('memory before the selected region is unavailable');

        if (target.kind === 'wasm') {
            const available = this.memory.byteLength();
            const readable = Math.max(0, Math.min(count, available - offset));
            const bytes = readable === 0
                ? new Uint8Array()
                : this.memory.readBytes(offset, readable);
            return {
                address: `0x${offset.toString(16)}`,
                bytes,
                unreadableBytes: count - readable,
            };
        }

        // Hex Editor interprets its baseAddress query as the initial file offset,
        // so process-view offsets are absolute virtual addresses. The mapping picked
        // in the UI is only the starting point; the view itself spans the process.
        const address = BigInt(offset);
        if (address > 0xffff_ffff_ffff_ffffn) {
            throw new Error('the process memory address exceeds 64 bits');
        }
        let bytes: Uint8Array<ArrayBufferLike> = new Uint8Array();
        if (count > 0) {
            const processes = this.processes;
            if (processes === undefined) throw new Error('the native process bridge is unavailable');
            try {
                bytes = processes.readMemory(
                    target.handle,
                    address.toString(),
                    count,
                );
            } catch {
                // DAP represents inaccessible memory as unreadable bytes, not a failed request.
            }
        }
        return {
            address: `0x${address.toString(16)}`,
            bytes,
            unreadableBytes: count - bytes.byteLength,
        };
    }

    private emitSnapshot(force: boolean): void {
        const now = performance.now();
        if (!force && now - this.lastSnapshotTime < SNAPSHOT_INTERVAL_MILLISECONDS) {
            return;
        }
        this.lastSnapshotTime = now;
        const statistics = this.tickStatistics.snapshot();
        workerPort.postMessage({
            type: 'snapshot',
            snapshot: {
                status: this.status,
                program: this.program,
                tickRateHz: this.tickRateHz,
                tickCount: this.tickCount,
                sampledTickCount: statistics.sampleCount,
                retainedTickCount: statistics.retainedSampleCount,
                averageTickMilliseconds: statistics.averageMilliseconds,
                slowestTickMilliseconds: statistics.slowestMilliseconds,
                memoryBytes: this.memory.byteLength(),
                timer: this.timer.snapshot(),
                settings: this.settings.snapshot(),
                processes: this.processes?.snapshot() ?? [],
            },
        } satisfies RuntimeResponse);
    }

    private emitLog(message: Omit<RuntimeLogMessage, 'type' | 'timestamp'>): void {
        workerPort.postMessage({
            type: 'log',
            timestamp: new Date().toISOString(),
            ...message,
        } satisfies RuntimeResponse);
    }
}

function fail(error: unknown): void {
    workerPort.postMessage({
        type: 'failure',
        message: error instanceof Error ? error.message : String(error),
        stack: error instanceof Error ? error.stack : undefined,
    } satisfies RuntimeResponse);
}

function requestFailure(requestId: number, message: string): void {
    workerPort.postMessage({
        type: 'requestFailure',
        requestId,
        message,
    } satisfies RuntimeResponse);
}

function ownedBytes(bytes: Uint8Array): Uint8Array<ArrayBuffer> {
    const owned = new Uint8Array(bytes.byteLength);
    owned.set(bytes);
    return owned;
}

function parseU64(value: string, description: string): bigint {
    const parsed = BigInt(value);
    if (parsed < 0n || parsed > 0xffff_ffff_ffff_ffffn) {
        throw new Error(`the ${description} must be an unsigned 64-bit integer`);
    }
    return parsed;
}

function initialEntryPoint(
    exports: WebAssembly.Exports,
    hasUpdateLoop: boolean,
): (() => void) | undefined {
    const initialize = exportedFunction(exports._initialize);
    const start = exportedFunction(exports._start);
    if (hasUpdateLoop) return initialize ?? start;
    if (initialize === undefined) return start;
    if (start === undefined || start === initialize) return initialize;
    return () => {
        initialize();
        start();
    };
}

function exportedFunction(value: WebAssembly.ExportValue | undefined): (() => void) | undefined {
    return typeof value === 'function' ? value as () => void : undefined;
}

function asrOperatingSystem(): string {
    switch (process.platform) {
        case 'win32': return 'windows';
        case 'darwin': return 'macos';
        default: return process.platform;
    }
}

function asrArchitecture(): string {
    switch (process.arch) {
        case 'x64': return 'x86_64';
        case 'ia32': return 'x86';
        case 'arm64': return 'aarch64';
        default: return process.arch;
    }
}
