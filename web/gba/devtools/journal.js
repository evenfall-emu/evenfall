import { FailureRecorder } from "./capture.js";
import { sha1, validateScenario } from "./scenario.js";

/** Unbounded time, bounded input count. Independent of the failure history ring. */
export class GameplayRecording {
  constructor(core, options) {
    this.core = core;
    this.options = options;
    this.enabled = true;
    this.costMs = 0;
    this.start(true);
  }
  start(coldBoot = false) {
    this.inputs = [];
    this.checkpoints = [];
    this.full = false;
    this.gap = false;
    this.end = undefined;
    this.initialState = coldBoot ? undefined : this.core.saveState();
    this.initialSave = this.core.exportSave();
    this.recorder = new FailureRecorder(this.core, {
      identity: {
        config: this.config(),
        build: this.options.build,
        biosSha1: this.options.biosSha1,
      },
      onCapture: this.options.onCapture,
    });
  }
  config() {
    return {
      boot: this.options.bios ? "bios" : "hle",
      saveType: this.options.saveType ?? "auto",
      rtc: this.options.rtc ?? this.core.inspect().rtcEnabled,
      rtcEpoch: String(this.options.rtcEpoch ?? 0n),
      sampleRate: this.options.sampleRate ?? 48000,
    };
  }
  invoke(method, args, executionArgs = args) {
    if (!this.enabled) {
      this.gap = true;
      // An older snapshot cannot reproduce execution that was not recorded.
      this.recorder.snapshots = [];
      this.recorder.snapshotBytes = 0;
      this.recorder.clearHistory();
      this.recorder.snapshotError = {
        name: "RecordingDisabled",
        message: "Execution continued while recording was off",
      };
      try {
        const result = this.core[method](...executionArgs);
        if (result?.reason === "faulted")
          this.recorder.capture("fault", result);
        return result;
      } catch (error) {
        this.recorder.capture("exception", {
          message: String(error?.message ?? error),
        });
        throw error;
      }
    }
    const start = performance.now();
    try {
      return this.recorder[method](...executionArgs);
    } finally {
      this.costMs += performance.now() - start;
    }
  }
  appendInput(at, keys) {
    if (!this.enabled || this.full) return;
    this.inputs.push({ at: String(at), keys });
    if (this.inputs.length === 1024) {
      this.full = true;
      this.end = String(at);
    }
  }
  setKeys(keys) {
    const at = this.recorder.currentCycle();
    const result = this.invoke("setKeys", [keys]);
    this.appendInput(at, keys);
    return result;
  }
  runUntil(...args) {
    let result;
    try {
      result = this.invoke("runUntil", args);
    } catch (error) {
      this.gap = true;
      throw error;
    }
    // Retain only inputs actually consumed by this bounded call. Scheduled
    // inputs keep their requested cycles, as required by the CLI format.
    for (let i = 0; i < result.inputsConsumed; i++)
      this.appendInput(args[1][i].at, args[1][i].keys);
    return result;
  }
  step() {
    return this.invoke("step", []);
  }
  drainAudio(output) {
    return this.invoke("drainAudio", [], output === undefined ? [] : [output]);
  }
  checkpoint(name) {
    if (!name?.trim() || this.checkpoints.some((p) => p.name === name))
      throw new Error("Use a unique checkpoint name");
    this.checkpoints.push({ name, at: String(this.recorder.currentCycle()) });
    return this.status();
  }
  status() {
    return {
      enabled: this.enabled,
      inputs: this.inputs.length,
      full: this.full,
      gap: this.gap,
      history: this.recorder.operationCount,
      snapshots: this.recorder.snapshots.length,
      snapshotError: this.recorder.snapshotError,
      costMs: this.costMs,
      overheadMs: this.recorder.overheadMs,
      snapshotMs: this.recorder.snapshotMs,
      snapshotMaxMs: this.recorder.snapshotMaxMs,
      historyCycles: String(
        this.recorder.currentCycle() -
          BigInt(
            this.recorder.snapshots[0]?.cycle ?? this.recorder.currentCycle(),
          ),
      ),
      historyBytes: this.recorder.historyBytes,
      checkpoints: this.checkpoints,
    };
  }
  async scenario(name = "Recorded gameplay") {
    if (this.gap)
      throw new Error("Recording has missing history. Start a new segment.");
    const assets = { rom: this.core.exportRom() };
    if (this.options.bios) assets.bios = this.options.bios;
    if (this.initialSave.length) assets.save = this.initialSave;
    if (this.initialState) assets.state = this.initialState;
    const descriptors = {};
    for (const [key, bytes] of Object.entries(assets))
      descriptors[key] = {
        path: `${key}.${key === "rom" ? "gba" : key === "save" ? "sav" : "bin"}`,
        sha1: await sha1(bytes),
      };
    const end = this.end ?? String(this.recorder.currentCycle());
    const checkpoints = this.checkpoints.filter(
      (p) => BigInt(p.at) <= BigInt(end),
    );
    if (!checkpoints.length || checkpoints.at(-1).at !== end)
      checkpoints.push({
        name: checkpoints.some((p) => p.name === "End")
          ? "Recording end"
          : "End",
        at: end,
      });
    const manifest = {
      version: 2,
      name,
      assets: descriptors,
      config: this.config(),
      inputs: this.inputs.slice(),
      checkpoints,
      limits: {
        maxCycles: end,
        maxRuns: 1000000,
        chunkCycles: "70224",
        maxAudioBytes: 268435456,
      },
    };
    validateScenario(manifest);
    return { manifest, assets };
  }
}
