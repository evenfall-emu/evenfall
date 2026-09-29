import type { WasmCore } from "../core.js";
import type {
  FailureBundle,
  RunResult,
  Snapshot,
  TimedInput,
} from "../zigba.js";

export type { FailureBundle } from "../zigba.js";

export interface RecorderOptions {
  /** Build/boot identity stored in every bundle and checked on replay. */
  identity?: Record<string, unknown>;
  /** Retained snapshots (default 3). */
  maxSnapshots?: number;
  /** Raw snapshot budget in bytes (default 16 MiB). */
  maxSnapshotBytes?: number;
  /** Serialized operation-history budget in bytes (default 4 MiB). */
  maxHistoryBytes?: number;
  /** Retained operations (default 4,096). */
  maxOperations?: number;
  /** Periodic snapshot interval (default 60 frames). */
  snapshotIntervalCycles?: bigint | number;
  /** Called with every automatic or manual capture. */
  onCapture?: (bundle: FailureBundle) => void;
}

/**
 * Bounded failure recording around an already loaded `WasmCore`. Route
 * `runUntil`, `step`, `setKeys` and `drainAudio` through the recorder; faults
 * and thrown execution errors capture automatically.
 */
export class FailureRecorder {
  constructor(core: WasmCore, options?: RecorderOptions);
  readonly core: WasmCore;
  /** The most recent bundle, if any. */
  readonly lastCapture?: FailureBundle;
  snapshotIntervalCycles: bigint;
  currentCycle(): bigint;
  hasFault(): boolean;
  runUntil(
    deadline: bigint,
    events?: readonly TimedInput[],
    inputDeadline?: bigint,
  ): RunResult;
  setKeys(mask: number): void;
  step(): ReturnType<WasmCore["step"]>;
  drainAudio(output?: Int16Array): Int16Array;
  inspect(): Snapshot;
  readMemory(address: number, length: number): Uint8Array;
  frame(): Uint16Array;
  exportSave(): Uint8Array;
  saveState(): Uint8Array;
  /** Capture and throw when `condition` is falsy. */
  assert(
    condition: unknown,
    message?: string,
    details?: Record<string, unknown>,
  ): asserts condition;
  capture(reason?: string, details?: Record<string, unknown>): FailureBundle;
}

/** Validate the fields a viewer reads before rendering an untrusted bundle. */
export function validateFailureBundle(bundle: unknown): FailureBundle;

export interface ReplayOptions {
  /** Identity of the destination core; every recorded key must match. */
  identity?: Record<string, unknown>;
}
export interface ReplayResult {
  inspection: Snapshot;
  results: {
    sequence: number;
    result: unknown;
    error?: { name: string; message: string };
  }[];
}

/**
 * Restore the last retained snapshot into `core` (already loaded with the
 * same ROM/BIOS) and replay recorded operations, verifying every result.
 */
export function replayFailureBundle(
  core: WasmCore,
  bundle: FailureBundle,
  options?: ReplayOptions,
): ReplayResult;
export function replayFailureBundleAsync(
  core: WasmCore,
  bundle: FailureBundle,
  options?: ReplayOptions & { signal?: AbortSignal },
): Promise<{ status: "cancelled" } | ({ status: "passed" } & ReplayResult)>;
