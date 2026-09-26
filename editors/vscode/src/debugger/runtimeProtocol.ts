export type TimerState = 'notRunning' | 'running' | 'paused' | 'ended';
export type GameTimeState = 'notInitialized' | 'paused' | 'running';

export interface TimerSnapshot {
    state: TimerState;
    gameTimeSeconds: number;
    gameTimeState: GameTimeState;
    splitIndex: number;
    variables: Readonly<Record<string, string>>;
}

export type SettingValueSnapshot =
    | { type: 'map'; value: SettingMapSnapshot }
    | { type: 'list'; value: SettingValueSnapshot[] }
    | { type: 'bool'; value: boolean }
    | { type: 'i64'; value: string }
    | { type: 'f64'; value: number }
    | { type: 'string'; value: string };

export type SettingMapSnapshot = Array<{
    key: string;
    value: SettingValueSnapshot;
}>;

export interface SettingWidgetBase {
    key: string;
    description: string;
    tooltip?: string;
}

export type SettingWidgetSnapshot =
    | (SettingWidgetBase & { type: 'title'; headingLevel: number })
    | (SettingWidgetBase & { type: 'bool'; defaultValue: boolean })
    | (SettingWidgetBase & {
        type: 'choice';
        defaultOptionKey: string;
        options: Array<{ key: string; description: string }>;
    })
    | (SettingWidgetBase & {
        type: 'fileSelect';
        filters: Array<
            | { type: 'name'; description?: string; pattern: string }
            | { type: 'mime'; mime: string }
        >;
    })
    | (SettingWidgetBase & { type: 'textInput'; defaultValue: string });

export interface SettingsSnapshot {
    widgets: SettingWidgetSnapshot[];
    map: SettingMapSnapshot;
    handleCount: number;
}

export interface ProcessSnapshot {
    handle: string;
    pid: number;
    path?: string;
    isOpen: boolean;
}

export interface ProcessMemoryRange {
    address: string;
    size: string;
    flags: string;
}

export type RuntimeMemoryTarget =
    | { kind: 'wasm' }
    | {
        kind: 'process';
        handle: string;
    };

export interface RuntimeMemoryRead {
    address: string;
    bytes: Uint8Array;
    unreadableBytes: number;
}

export interface RuntimeSnapshot {
    status: 'starting' | 'running' | 'paused' | 'trapped';
    program: string;
    tickRateHz: number;
    tickCount: number;
    sampledTickCount: number;
    retainedTickCount: number;
    averageTickMilliseconds: number;
    slowestTickMilliseconds: number;
    memoryBytes: number;
    timer: TimerSnapshot;
    settings: SettingsSnapshot;
    processes: ProcessSnapshot[];
}

export interface RuntimeLaunchMessage {
    type: 'launch';
    wasm: ArrayBuffer;
    program: string;
    settings?: SettingMapSnapshot;
    nativeModulePath?: string;
    paused?: boolean;
}

export interface RuntimeSetExecutionMessage {
    type: 'setExecution';
    requestId: number;
    paused: boolean;
}

export interface RuntimeTimerCommandMessage {
    type: 'timerCommand';
    command: 'start' | 'reset';
}

export interface RuntimeSetSettingMessage {
    type: 'setSetting';
    key: string;
    value: boolean | string;
}

export interface RuntimeClearSettingsMessage {
    type: 'clearSettings';
}

export interface RuntimeResetStatisticsMessage {
    type: 'resetStatistics';
}

export interface RuntimeReadMemoryMessage {
    type: 'readMemory';
    requestId: number;
    target: RuntimeMemoryTarget;
    offset: number;
    count: number;
}

export interface RuntimeListProcessMemoryRangesMessage {
    type: 'listProcessMemoryRanges';
    requestId: number;
    handle: string;
}

export interface RuntimeShutdownMessage {
    type: 'shutdown';
}

export type RuntimeRequest =
    | RuntimeLaunchMessage
    | RuntimeSetExecutionMessage
    | RuntimeTimerCommandMessage
    | RuntimeSetSettingMessage
    | RuntimeClearSettingsMessage
    | RuntimeResetStatisticsMessage
    | RuntimeReadMemoryMessage
    | RuntimeListProcessMemoryRangesMessage
    | RuntimeShutdownMessage;

export interface RuntimeReadyMessage {
    type: 'ready';
    unsupportedImports: string[];
}

export interface RuntimeSnapshotMessage {
    type: 'snapshot';
    snapshot: RuntimeSnapshot;
}

export interface RuntimeLogMessage {
    type: 'log';
    timestamp: string;
    source: 'runtime' | 'autoSplitter';
    level: 'debug' | 'info' | 'warning' | 'error';
    message: string;
}

export interface RuntimeFailureMessage {
    type: 'failure';
    message: string;
    stack?: string;
}

export interface RuntimeStoppedMessage {
    type: 'stopped';
}

export interface RuntimeExecutionChangedMessage {
    type: 'executionChanged';
    requestId: number;
    paused: boolean;
}

export interface RuntimeMemoryReadMessage {
    type: 'memoryRead';
    requestId: number;
    address: string;
    bytes: ArrayBuffer;
    unreadableBytes: number;
}

export interface RuntimeProcessMemoryRangesMessage {
    type: 'processMemoryRanges';
    requestId: number;
    ranges: ProcessMemoryRange[];
}

export interface RuntimeRequestFailureMessage {
    type: 'requestFailure';
    requestId: number;
    message: string;
}

export type RuntimeResponse =
    | RuntimeReadyMessage
    | RuntimeSnapshotMessage
    | RuntimeLogMessage
    | RuntimeFailureMessage
    | RuntimeExecutionChangedMessage
    | RuntimeMemoryReadMessage
    | RuntimeProcessMemoryRangesMessage
    | RuntimeRequestFailureMessage
    | RuntimeStoppedMessage;
