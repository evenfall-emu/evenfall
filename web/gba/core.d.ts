import type {
  Cheat,
  CheatFormat,
  CheatInput,
  LoadOptions,
  MemorySnapshot,
  RewindParts,
  RunResult,
  Snapshot,
  TimedInput,
} from "./emulator.js";

export {
  KEYS,
  CYCLES_PER_FRAME,
  SAVE_TYPES,
  CHEAT_FORMATS,
} from "./emulator.js";
export type {
  Cheat,
  CheatFormat,
  CheatInput,
  LoadOptions,
  MemorySnapshot,
  RunResult,
  SaveType,
  Snapshot,
  StopReason,
  TimedInput,
} from "./emulator.js";

export type StepReason =
  | "executed"
  | "halted"
  | "power_stopped"
  | "faulted"
  /** A DMA burst owns the bus; the CPU waits until running lets it end. */
  | "dma_stall";
export type RewindCompression = "off" | "fast" | "delta";

/**
 * Synchronous adapter for one WebAssembly instance, for Node, Workers and
 * other headless hosts. Calls block the calling thread; drive one instance
 * from one caller at a time. Methods other than `load` throw `NoSession`
 * before a ROM is loaded. Returned arrays are owned copies.
 */
/**
 * Whether this engine supports WebAssembly SIMD (simd128). The default core is
 * `evenfall-gba.wasm` (SIMD) where it does, `evenfall-gba-scalar.wasm` otherwise.
 */
export const wasmSimd: boolean;

export class WasmCore {
  /**
   * Instantiate the core. Accepts WASM bytes or a URL. Without an argument it
   * loads the package's `evenfall-gba.wasm`, or `evenfall-gba-scalar.wasm` on an
   * engine without WebAssembly SIMD: fetched in browsers, read from disk in
   * Node. Rejects a binary whose `ef_version` ABI does not match this module.
   */
  static create(
    source?: URL | string | Uint8Array | ArrayBuffer,
  ): Promise<WasmCore>;
  constructor(exports: WebAssembly.Exports);
  /** Raw instance exports. The `ef_*` ABI is internal and versioned. */
  readonly wasm: WebAssembly.Exports;
  /** True between a successful `load` and `dispose`. */
  readonly loaded: boolean;
  /** Replaced on load, reset, state load and rewind; null before load. */
  readonly timeline: string | null;

  /** Load a raw ROM or single-ROM ZIP. Failure keeps the previous machine. */
  load(image: Uint8Array, options?: LoadOptions): Snapshot;
  currentCycle(): bigint;
  hasFault(): boolean;
  inspect(): Snapshot;
  /** Held-button mask built from `KEYS`, applied at the current cycle. */
  setKeys(mask: number): void;
  /**
   * Run to `deadline`, applying absolute-cycle inputs in order. Instructions
   * are atomic, so the returned cycle can pass the deadline. At most 1,024
   * inputs; inputs after `inputDeadline` are left unconsumed.
   */
  runUntil(
    deadline: bigint,
    events?: readonly TimedInput[],
    inputDeadline?: bigint,
  ): RunResult;
  /**
   * Replace the active cheats at the current cycle; they apply at the first
   * instruction boundary at or after each later VBlank start. An empty list
   * removes them. Returns the canonical active list.
   */
  setCheats(cheats: readonly CheatInput[]): Cheat[];
  /** The active cheats. States hold none, so loads and rewinds keep them. */
  cheats(): Cheat[];
  /** Detect and normalize one code; needs no loaded game. */
  parseCheat(code: string, format?: CheatFormat | "auto"): Cheat;
  step(): Snapshot & { reason: StepReason };
  /** Bus-visible bytes; at most 1 MiB. Does not advance time. */
  readMemory(address: number, length: number): Uint8Array;
  /** Owned EWRAM and IWRAM copies from one execution endpoint. */
  captureMemory(): MemorySnapshot;
  /** Exact-cycle 240×160 BGR555 framebuffer, optionally into `output`. */
  frame(output?: Uint16Array): Uint16Array;
  /** False while frame skipping has left pixels non-canonical. */
  canSnapshot(): boolean;
  /** Render one complete image per `interval` frames (1–16). */
  setVideoInterval(interval: number): void;
  videoFrameId(): bigint;
  videoFrameCycle(): bigint;
  /** Last selected image, without reconstructing skipped frames. */
  presentationFrame(output?: Uint16Array): Uint16Array;
  /** Interleaved signed stereo samples produced since the last drain. */
  drainAudio(output?: Int16Array): Int16Array;
  /** Host-only preference; the emulated sound hardware keeps running. */
  setAudioOutput(enabled: boolean): void;
  exportRom(): Uint8Array;
  /** Raw cartridge save bytes, interoperable with the native frontend. */
  exportSave(): Uint8Array;
  /** Reboot the resident ROM/BIOS with replacement save bytes. */
  resetWithSave(save: Uint8Array): Snapshot;
  /** Portable, versioned machine state for the loaded ROM/BIOS. */
  saveState(): Uint8Array;
  /** Validates fully before replacing the machine; clears rewind history. */
  loadState(state: Uint8Array): Snapshot;
  /** Live state plus retained rewind history and policy. */
  saveRewindBundle(): Uint8Array;
  loadRewindBundle(bundle: Uint8Array): Snapshot;
  /** A bundle split for incremental storage; copies only unknown payloads. */
  saveRewindParts(known?: Iterable<string>, minGap?: number): RewindParts;
  loadRewindParts(
    state: Uint8Array,
    index: Uint8Array,
    payloads: ReadonlyMap<string, Uint8Array>,
  ): Snapshot;
  /** Enable budgeted rewind (defaults 15 MiB, 60 frames, "delta"). */
  setRewindConfig(
    budgetBytes?: number,
    intervalFrames?: number,
    compression?: RewindCompression,
  ): void;
  /** Tell the capture cadence about realtime playback speed (0.5–16). */
  setRewindSpeed(multiplier: number): void;
  rewind(steps?: number): Snapshot;
  rewindDepth(): number;
  rewindInfo(index: number): { frame: bigint; cycle: bigint; bytes: number };
  /** Decode one retained checkpoint's framebuffer without changing state. */
  rewindFrame(index: number): Uint16Array;
  /** Release the machine; `load` may be called again. */
  dispose(): void;
}
