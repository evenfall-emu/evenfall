export type SaveType =
  | "auto"
  | "none"
  | "sram"
  | "flash64"
  | "flash64-atmel"
  | "flash128"
  | "eeprom512"
  | "eeprom8k";
export type StopReason =
  | "deadline"
  | "instruction_boundary_after_deadline"
  | "power_stopped"
  | "faulted";
export interface LoadOptions {
  bios?: Uint8Array;
  save?: Uint8Array;
  zipped?: boolean;
  saveType?: SaveType;
  rtc?: boolean;
  rtcEpoch?: bigint;
  sampleRate?: number;
}
export interface Snapshot {
  cycle: bigint;
  frames: bigint;
  registers: Uint32Array;
  cpsr: number;
  nextInstructionAddress: number;
  /** Lowercase hexadecimal SHA-1 of the raw ROM, after ZIP extraction. */
  romHash: string;
  rtcEnabled: boolean;
  saveType: SaveType;
  fault: string | null;
}
/** Owned EWRAM/IWRAM copies captured synchronously at one execution endpoint. */
export interface MemorySnapshot {
  readonly romHash: string;
  /** Replaced on load, reset, state load and rewind; independent of video. */
  readonly timeline: string;
  readonly sequence: number;
  readonly cycle: bigint;
  readonly regions: readonly {
    readonly address: number;
    readonly bytes: Uint8Array;
  }[];
}
export interface RewindCheckpoint {
  /** Stable only while the returned timeline remains current. */
  readonly index: number;
  /** Value accepted by rewind() to restore this checkpoint. */
  readonly steps: number;
  readonly frame: bigint;
  readonly cycle: bigint;
  readonly bytes: number;
}
export interface RewindTimeline {
  readonly timeline: string;
  readonly liveCycle: bigint;
  readonly liveFrame: bigint;
  /** Oldest to newest; only checkpoints strictly older than liveFrame. */
  readonly checkpoints: readonly RewindCheckpoint[];
}
export interface TimedInput {
  at: bigint;
  keys: number;
}
export interface RunResult {
  cycle: bigint;
  frames: bigint;
  reason: StopReason;
  inputsConsumed: number;
}
export interface FrameEvent {
  event: "frame";
  generation: number;
  workMs: number;
  /** Work averaged across these emulated frames, including omitted images. */
  emulatedWorkFrames: number;
  /** Identity and cycle of the last completed selected image. */
  videoFrameId: bigint;
  videoFrameCycle: bigint;
  pixels: Uint16Array;
  audio: Int16Array;
  /** Current WASM linear-memory size in bytes. */
  memoryBytes: number;
  /** Presentation buffers allocated so far; recycling keeps this bounded. */
  presentationAllocations: number;
  /** Cached at the configured diagnostics interval; inspect() is always fresh. */
  state: Snapshot;
}
export const KEYS: Readonly<
  Record<
    | "a"
    | "b"
    | "select"
    | "start"
    | "right"
    | "left"
    | "up"
    | "down"
    | "r"
    | "l",
    number
  >
>;
export const CYCLES_PER_FRAME: bigint;
export const SAVE_TYPES: readonly SaveType[];
export type CheatFormat =
  | "raw"
  | "gameshark"
  | "gameshark-decrypted"
  | "action-replay"
  | "action-replay-decrypted"
  | "codebreaker";
/** Explicit cheat formats; requests may also pass `"auto"`. */
export const CHEAT_FORMATS: readonly CheatFormat[];
/** A cheat to activate. `format` defaults to `"auto"`, which detects it. */
export interface CheatInput {
  code: string;
  format?: CheatFormat | "auto";
}
/** An active cheat: its resolved format and canonical code text. */
export interface Cheat {
  format: CheatFormat;
  code: string;
}
export interface EmulatorOptions {
  /** Minimum interval for frame-event CPU snapshots. Default 0 (every frame). */
  diagnosticsIntervalMs?: number;
  /**
   * Module worker script. Default: the package's `worker.js`, which bundlers
   * emit automatically. Must be same-origin with the page.
   */
  workerUrl?: URL | string;
  /**
   * WebAssembly binary fetched by the worker. Default: the package's
   * `evenfall-gba.wasm`, or `evenfall-gba-scalar.wasm` on an engine without
   * WebAssembly SIMD.
   */
  wasmUrl?: URL | string;
}
/** An isolated emulator session; each owns one Worker and one WASM instance. */
export class Emulator extends EventTarget {
  /** Start a Worker and instantiate the WASM core. Use instead of `new`. */
  static create(options?: EmulatorOptions): Promise<Emulator>;
  private constructor(worker: Worker);
  load(image: Uint8Array, options?: LoadOptions): Promise<Snapshot>;
  /** Request 0.5–16× pacing; actual speed depends on the host. Above 2×,
   * host audio production is suppressed and complete video frames are selected
   * for presentation while all hardware timing continues. Explicit saves and
   * frame reads reconstruct exact-cycle pixels when needed. */
  setSpeed(multiplier: number): Promise<number>;
  play(): Promise<void>;
  pause(): Promise<Snapshot | null>;
  reset(): Promise<Snapshot>;
  /** Reboot resident ROM/BIOS with replacement cartridge save bytes. */
  resetWithSave(save: Uint8Array): Promise<Snapshot>;
  inspect(): Promise<Snapshot>;
  readMemory(address: number, length: number): Promise<Uint8Array>;
  /** Null when no ROM is loaded. Does not advance the machine or read MMIO. */
  captureMemory(): Promise<MemorySnapshot | null>;
  setKeys(mask: number): Promise<void>;
  /** Replace the active cheats at the current cycle, effective from the next
   * VBlank start; allowed while playing and recorded. An empty list removes
   * them. Resolves to the canonical active list. */
  setCheats(cheats: readonly CheatInput[]): Promise<Cheat[]>;
  /** The active cheats; states, rewinds and resets can change them. */
  cheats(): Promise<Cheat[]>;
  /** Detect and normalize one code without changing the session. */
  parseCheat(code: string, format?: CheatFormat | "auto"): Promise<Cheat>;
  runUntil(deadline: bigint, inputs?: TimedInput[]): Promise<RunResult>;
  step(): Promise<
    Snapshot & { reason: "executed" | "halted" | "power_stopped" | "faulted" }
  >;
  frame(): Promise<Uint16Array>;
  saveState(): Promise<Uint8Array>;
  loadState(bytes: Uint8Array): Promise<Snapshot>;
  /** Owned opaque bytes: exact live state, retained rewind history and policy.
   * Does not advance time or consume history. No ROM/BIOS contents included. */
  saveRewindBundle(): Promise<Uint8Array>;
  /** Atomically restore state AND history against the loaded ROM/BIOS. Requires
   * paused playback. Failure preserves both; success replaces the timeline and
   * clears host presentation. Unlike loadState(), keeps imported checkpoints.
   * Playback speed/output preferences stay local; input bytes are not detached. */
  loadRewindBundle(bytes: Uint8Array): Promise<Snapshot>;
  /** Enable byte-budgeted checkpoint history (disabled at load). Older history
   * thins out as the budget fills. Reconfiguring clears history. Defaults:
   * 15 MiB, a capture every 60 frames at 1x, delta compression. */
  setRewindConfig(
    budgetBytes?: number,
    intervalFrames?: number,
    compression?: "off" | "fast" | "delta",
  ): Promise<{ depth: number }>;
  /** Restore earlier checkpoints while paused; replaces the bridge timeline. */
  rewind(steps?: number): Promise<Snapshot>;
  rewindDepth(): Promise<number>;
  /** Read the available checkpoint-granular timeline without changing it. */
  rewindCheckpoints(): Promise<RewindTimeline>;
  /** Read exact checkpoint pixels without changing the live machine/history. */
  rewindFrame(index: number): Promise<Uint16Array>;
  exportSave(): Promise<Uint8Array>;
  /** Relinquish frame-event storage after use. Supplied views are detached. */
  recyclePresentation(pixels?: Uint16Array, audio?: Int16Array): void;
  dispose(): Promise<void>;
  /** Immediately abort even an unresponsive worker; unsaved progress is lost. */
  terminate(message?: string): void;
}
export interface Scenario {
  version: 2;
  name: string;
  /** Input assets, identified by the SHA-1 of their raw bytes. */
  assets: Record<string, { path?: string; sha1: string }>;
  config: {
    boot: "hle" | "bios";
    saveType: SaveType;
    rtc: boolean;
    rtcEpoch: string;
    sampleRate: number;
  };
  inputs: { at: string; keys: number }[];
  /** Cheat-set changes, applied once execution reaches `at`. */
  cheats?: { at: string; cheats: CheatInput[] }[];
  checkpoints: {
    name: string;
    at: string;
    expect?: Record<string, unknown>;
    memory?: {
      name: string;
      address: number;
      length: number;
      sha256?: string;
    }[];
  }[];
  limits: {
    maxCycles: string;
    maxRuns: number;
    chunkCycles: string;
    maxAudioBytes: number;
  };
}
export interface RecordingStatus {
  enabled: boolean;
  inputs: number;
  full: boolean;
  gap: boolean;
  history: number;
  snapshots: number;
  snapshotError: unknown;
  costMs: number;
  overheadMs: number;
  snapshotMs: number;
  snapshotMaxMs: number;
  historyCycles: string;
  historyBytes: number;
  checkpoints: { name: string; at: string }[];
}
export interface FailureBundle {
  format: "evenfall-failure";
  version: 2;
  identity: {
    romHash: string;
    biosSha1?: string;
    config?: Scenario["config"];
    manifest?: Scenario;
  };
  history: { replayable: boolean; audioReplay: string; snapshotError: unknown };
  artifacts: { framebuffer?: number[]; cpu?: Record<string, unknown> };
  [key: string]: unknown;
}
export interface ScenarioExport {
  manifest: Scenario;
  assets: Record<string, Uint8Array>;
}
export interface LabReport {
  status: string;
  name?: string;
  manifest?: Scenario;
  checkpoints?: Record<string, unknown>[];
  failures?: string[];
  capture?: FailureBundle;
  [key: string]: unknown;
}
export interface Emulator {
  recording(enabled?: boolean): Promise<RecordingStatus>;
  newSegment(): Promise<RecordingStatus>;
  checkpoint(name: string): Promise<RecordingStatus>;
  capture(note?: string): Promise<FailureBundle>;
  exportScenario(name?: string): Promise<ScenarioExport>;
  runScenario(
    manifest: Scenario,
    assets: Record<string, Uint8Array>,
  ): Promise<LabReport>;
  replay(bundle: FailureBundle): Promise<LabReport>;
  cancel(): Promise<void>;
  exportRom(): Promise<Uint8Array>;
}

export interface EmulatorEventMap {
  frame: CustomEvent<FrameEvent>;
  clear: CustomEvent<{
    event: "clear";
    generation: number;
    timeline: string | null;
  }>;
  close: CustomEvent<{ event: "close"; message: string }>;
  capture: CustomEvent<{ event: "capture"; bundle: FailureBundle }>;
  progress: CustomEvent<{
    event: "progress";
    checkpoint: Record<string, unknown>;
  }>;
  artifact: CustomEvent<{ event: "artifact"; name: string; bytes: Uint8Array }>;
  error: CustomEvent<{ event: "error"; message: string }>;
  stopped: CustomEvent<{
    event: "stopped";
    reason: StopReason;
    state: Snapshot;
  }>;
}
export interface Emulator {
  addEventListener<K extends keyof EmulatorEventMap>(
    type: K,
    listener: (event: EmulatorEventMap[K]) => void,
    options?: boolean | AddEventListenerOptions,
  ): void;
  addEventListener(
    type: string,
    listener: EventListenerOrEventListenerObject | null,
    options?: boolean | AddEventListenerOptions,
  ): void;
  removeEventListener<K extends keyof EmulatorEventMap>(
    type: K,
    listener: (event: EmulatorEventMap[K]) => void,
    options?: boolean | EventListenerOptions,
  ): void;
  removeEventListener(
    type: string,
    listener: EventListenerOrEventListenerObject | null,
    options?: boolean | EventListenerOptions,
  ): void;
}
