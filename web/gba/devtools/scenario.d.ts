import type { WasmCore } from "../core.js";
import type { FailureBundle, Scenario, StopReason } from "../emulator.js";
import type { RecorderOptions } from "./capture.js";

export type { Scenario } from "../emulator.js";

/** JSON-compatible copy: bigints become strings, typed arrays plain arrays. */
export function jsonSafe(value: unknown): any;
/** Lowercase hexadecimal SHA-256 through Web Crypto, for observed outputs. */
export function sha256(bytes: BufferSource): Promise<string>;
/** Lowercase hexadecimal SHA-1 through Web Crypto, for input assets. */
export function sha1(bytes: BufferSource): Promise<string>;
/** Throw a descriptive error unless `manifest` is a valid version-2 scenario. */
export function validateScenario(manifest: unknown): Scenario;

export interface ScenarioOptions {
  /** Build identity recorded in the result and failure bundles. */
  build?: unknown;
  /** Failure-recorder limits, or false to disable capture overhead. */
  capture?: false | Omit<RecorderOptions, "identity" | "onCapture">;
  signal?: AbortSignal;
  /** Awaited every few runs so long scenarios can yield to the host. */
  yieldControl?: () => Promise<void>;
  onProgress?: (checkpoint: ScenarioCheckpoint) => void;
  /** Receives owned artifact bytes (frame, audio, save, memory). */
  onArtifact?: (name: string, bytes: Uint8Array) => void | Promise<void>;
  /** Also emit per-checkpoint frame and audio artifacts. */
  allArtifacts?: boolean;
}

export interface ScenarioCheckpoint {
  name: string;
  requestedCycle: string;
  cycle: string;
  reason: StopReason;
  fault: string | null;
  cpu: {
    registers: number[];
    cpsr: number;
    nextInstructionAddress: number;
  };
  frameSha256: string;
  audioSha256: string;
  audioBytes: number;
  saveSha256: string;
  stateSha256?: string;
  memory: {
    name: string;
    address: number;
    length: number;
    sha256: string;
  }[];
}

export interface ScenarioResult {
  version: 2;
  name: string;
  build: unknown;
  manifest: Scenario;
  status: "passed" | "failed" | "cancelled";
  initial?: Record<string, unknown>;
  checkpoints: ScenarioCheckpoint[];
  runs: number;
  inputsConsumed: number;
  failures: string[];
  artifacts: { name: string; size: number; sha256: string }[];
  capture?: FailureBundle;
  captureError?: string;
}

/**
 * Load `assets` (raw bytes keyed by manifest asset name, identity-checked)
 * into `core` and run the scenario deterministically. Validation and asset
 * errors reject; execution and checkpoint failures resolve as "failed".
 */
export function runScenario(
  core: WasmCore,
  manifest: Scenario,
  assets: Record<string, Uint8Array>,
  options?: ScenarioOptions,
): Promise<ScenarioResult>;
