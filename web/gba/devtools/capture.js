// Optional host-side recording. The ordinary core pays no cost unless wrapped.
import { CYCLES_PER_FRAME } from "../core.js";

const json = (value) =>
  JSON.parse(
    JSON.stringify(value, (_, item) =>
      typeof item === "bigint"
        ? String(item)
        : ArrayBuffer.isView(item)
          ? Array.from(item)
          : item,
    ),
  );
const encode = (bytes) => {
  let out = "";
  for (let i = 0; i < bytes.length; i += 8192)
    out += String.fromCharCode(...bytes.subarray(i, i + 8192));
  return btoa(out);
};
const decode = (value) => Uint8Array.from(atob(value), (c) => c.charCodeAt(0));
const errorInfo = (error) => ({
  name: error?.name ?? "Error",
  message: String(error?.message ?? error),
});
function positive(value, label) {
  if (!Number.isSafeInteger(value) || value <= 0)
    throw new RangeError(`Invalid ${label}`);
  return value;
}

const validCycle = (value) =>
  typeof value === "bigint" && value >= 0n && value <= 0xffffffffffffffffn;
const validKeys = (value) =>
  Number.isInteger(value) && value >= 0 && value <= 1023;
const empty = Object.freeze([]);
const noInputCycleTypes = Object.freeze({
  deadline: "bigint",
  inputDeadline: "bigint",
  events: empty,
});
// Only ordinary ABI inputs take this path. Unusual/rejected arguments keep the
// general JSON conversion below, including their original cycle argument types.
function copyCall(method, args) {
  if (method !== "runUntil") {
    if (args.length === 0) return empty;
    if (method === "setKeys" && validKeys(args[0])) return args.slice();
    return null;
  }
  if (
    !validCycle(args[0]) ||
    !validCycle(args[2]) ||
    !Array.isArray(args[1]) ||
    args[1].length > 1024
  )
    return null;
  const events = args[1].length ? [] : empty;
  for (const event of args[1]) {
    if (!event || Object.getPrototypeOf(event) !== Object.prototype)
      return null;
    const fields = Object.getOwnPropertyDescriptors(event);
    if (
      Object.keys(fields).length !== 2 ||
      !validCycle(fields.at?.value) ||
      !validKeys(fields.keys?.value)
    )
      return null;
    events.push({ at: fields.at.value, keys: fields.keys.value });
  }
  return [args[0], events, args[2]];
}

export class FailureRecorder {
  constructor(core, options = {}) {
    const started = performance.now();
    this.overheadMs = 0;
    this.executionMs = 0;
    this.core = core;
    this.identity = json({
      ...options.identity,
      romHash: core.inspect().romHash,
    });
    this.maxSnapshots = positive(options.maxSnapshots ?? 3, "maxSnapshots");
    this.maxSnapshotBytes = positive(
      options.maxSnapshotBytes ?? 16 * 1024 * 1024,
      "maxSnapshotBytes",
    );
    this.maxHistoryBytes = positive(
      options.maxHistoryBytes ?? 4 * 1024 * 1024,
      "maxHistoryBytes",
    );
    this.historyBytes = 0;
    this.maxOperations = positive(
      options.maxOperations ?? 4096,
      "maxOperations",
    );
    this.snapshotIntervalCycles = BigInt(
      options.snapshotIntervalCycles ?? CYCLES_PER_FRAME * 60n,
    );
    if (this.snapshotIntervalCycles <= 0n)
      throw new RangeError("Invalid snapshotIntervalCycles");
    this.onCapture = options.onCapture;
    this.snapshots = [];
    this.operationRing = [];
    this.operationHead = 0;
    this.operationCount = 0;
    this.sequence = 0;
    this.snapshotBytes = 0;
    this.droppedOperations = 0;
    this.droppedSnapshots = 0;
    this.budgetSnapshots = 0;
    this.snapshotError = null;
    this.lastCapture = null;
    this.snapshotMs = 0;
    this.snapshotMaxMs = 0;
    this.takeSnapshot();
    this.overheadMs = performance.now() - started;
  }

  currentCycle() {
    return this.core.currentCycle?.() ?? this.core.inspect().cycle;
  }
  hasFault() {
    return this.core.hasFault?.() ?? Boolean(this.core.inspect().fault);
  }
  // Materialize an owned, JSON-compatible view only for inspection/export.
  get operations() {
    const records = [];
    for (let i = 0; i < this.operationCount; i++)
      records.push(
        this.operationRing[(this.operationHead + i) % (this.maxOperations + 1)],
      );
    return json(records);
  }
  clearHistory() {
    this.operationRing = [];
    this.operationHead = 0;
    this.operationCount = 0;
    this.historyBytes = 0;
  }
  appendOperation(operation) {
    this.operationRing[
      (this.operationHead + this.operationCount) % (this.maxOperations + 1)
    ] = operation;
    this.operationCount++;
    this.historyBytes += operation.recordBytes;
  }

  takeSnapshot(deferUntilCanonical = false) {
    // Periodic anchors can wait; budget/manual snapshots reconstruct exactly.
    if (deferUntilCanonical && this.core.canSnapshot?.() === false)
      return false;
    const started = performance.now();
    try {
      // Core snapshots already belong to the caller.
      const state = this.core.saveState();
      if (state.length > this.maxSnapshotBytes)
        throw new Error("Snapshot exceeds maxSnapshotBytes");
      const cycle = this.currentCycle();
      this.snapshots.push({
        cycle: String(cycle),
        nextOperation: this.sequence,
        state,
      });
      this.snapshotBytes += state.length;
      while (
        this.snapshots.length > this.maxSnapshots ||
        this.snapshotBytes > this.maxSnapshotBytes
      ) {
        this.snapshotBytes -= this.snapshots.shift().state.length;
        this.droppedSnapshots++;
      }
      this.lastSnapshotCycle = cycle;
      this.snapshotError = null;
      this.prune();
      return true;
    } catch (error) {
      this.snapshotError = errorInfo(error);
      return false;
    } finally {
      this.snapshotMs = performance.now() - started;
      this.snapshotMaxMs = Math.max(this.snapshotMaxMs, this.snapshotMs);
    }
  }

  prune() {
    const oldest = this.snapshots[0]?.nextOperation;
    while (
      this.operationCount &&
      (this.operationCount > this.maxOperations ||
        this.historyBytes > this.maxHistoryBytes ||
        (oldest !== undefined &&
          this.operationRing[this.operationHead].sequence < oldest))
    ) {
      const removed = this.operationRing[this.operationHead];
      this.operationRing[this.operationHead] = undefined;
      this.operationHead = (this.operationHead + 1) % (this.maxOperations + 1);
      this.operationCount--;
      this.historyBytes -= removed.recordBytes;
      this.droppedOperations++;
      while (
        this.snapshots.length &&
        this.snapshots[0].nextOperation <= removed.sequence
      ) {
        this.snapshotBytes -= this.snapshots.shift().state.length;
        this.droppedSnapshots++;
      }
    }
  }

  invoke(method, args, executionArgs = args) {
    const started = performance.now();
    const before = this.currentCycle();
    const operation = { sequence: this.sequence, method, beforeCycle: before };
    let copied;
    try {
      copied = copyCall(method, args);
      operation.args = copied ?? json(args);
      if (method === "runUntil")
        operation.cycleTypes =
          copied && !copied[1].length
            ? noInputCycleTypes
            : {
                deadline: typeof args[0],
                inputDeadline: typeof args[2],
                events: Array.isArray(args[1])
                  ? args[1].map((event) => typeof event?.at)
                  : [],
              };
    } catch (error) {
      operation.recordingError = errorInfo(error);
    }
    // Reserve enough space for the run result and the worst-case consumed input
    // copy before execution: a fault cannot be snapshotted afterward.
    // Conservative UTF-16 export bounds for fixed-shape ABI records. This also
    // covers the consumed-event copy and avoids serializing successful calls.
    const estimatedBytes = copied
      ? method === "runUntil"
        ? 1024 + copied[1].length * 256
        : 320
      : JSON.stringify(json(operation)).length * 2 +
        1024 +
        (method === "runUntil"
          ? JSON.stringify(operation.args?.[1] ?? []).length * 2
          : 0);
    if (
      this.operationCount &&
      (this.operationCount + 1 > this.maxOperations ||
        this.historyBytes + estimatedBytes > this.maxHistoryBytes) &&
      !this.hasFault()
    ) {
      if (this.takeSnapshot()) {
        this.budgetSnapshots++;
        // Rebase history onto the new anchor, freeing old history before the call.
        while (this.snapshots.length > 1) {
          this.snapshotBytes -= this.snapshots.shift().state.length;
          this.droppedSnapshots++;
        }
        this.prune();
      }
    }
    this.sequence++;
    let result, caught;
    const executionStarted = performance.now();
    try {
      result = this.core[method](...executionArgs);
    } catch (error) {
      caught = error;
      operation.error = errorInfo(error);
    }
    const executionMs = performance.now() - executionStarted;
    try {
      operation.afterCycle = this.currentCycle();
      if (method === "runUntil" && result) {
        operation.result = copied ? { ...result } : json(result);
        // The ABI exposes consumption, not the exact instruction boundary for each input.
        operation.consumedInputs = copied
          ? result.inputsConsumed
            ? copied[1].slice(0, result.inputsConsumed)
            : empty
          : json(args[1].slice(0, result.inputsConsumed));
        operation.applicationCycleRange = [before, operation.afterCycle];
      }
    } catch (error) {
      operation.inspectionError = errorInfo(error);
    }
    operation.recordBytes =
      copied && !caught && !operation.inspectionError
        ? estimatedBytes
        : JSON.stringify(json(operation)).length * 2;
    if (
      operation.recordingError ||
      operation.recordBytes > this.maxHistoryBytes
    ) {
      // A gap invalidates every older snapshot. Never advertise an incomplete replay.
      this.droppedSnapshots += this.snapshots.length;
      this.snapshots = [];
      this.snapshotBytes = 0;
      this.snapshotError = {
        name: "RecordingGap",
        message: "Operation exceeds history budget or is not serializable",
      };
      delete operation.args;
      delete operation.consumedInputs;
      delete operation.cycleTypes;
      operation.recordingError = this.snapshotError;
      operation.recordBytes = JSON.stringify(json(operation)).length * 2;
    }
    this.appendOperation(operation);
    this.prune();
    if (caught || result?.reason === "faulted") {
      this.capture(
        caught ? "exception" : "fault",
        caught ? errorInfo(caught) : result,
      );
    } else if (
      operation.afterCycle !== undefined &&
      operation.afterCycle - (this.lastSnapshotCycle ?? before) >=
        this.snapshotIntervalCycles
    ) {
      this.takeSnapshot(true);
    }
    this.executionMs += executionMs;
    this.overheadMs += performance.now() - started - executionMs;
    if (caught) throw caught;
    return result;
  }

  runUntil(deadline, events = empty, inputDeadline = deadline) {
    return this.invoke("runUntil", [deadline, events, inputDeadline]);
  }
  setKeys(mask) {
    return this.invoke("setKeys", [mask]);
  }
  step() {
    return this.invoke("step", empty);
  }
  // Destination storage is host scratch, never a replay argument.
  drainAudio(output) {
    const invalid =
      output !== undefined &&
      (!(output instanceof Int16Array) || output.length < 16384);
    return this.invoke(
      "drainAudio",
      invalid ? [output] : empty,
      output === undefined ? empty : [output],
    );
  }
  inspect() {
    return this.core.inspect();
  }
  readMemory(address, length) {
    return this.core.readMemory(address, length);
  }
  frame() {
    return this.core.frame();
  }
  exportSave() {
    return this.core.exportSave();
  }
  saveState() {
    return this.core.saveState();
  }

  assert(condition, message = "Assertion failed", details = {}) {
    if (condition) return;
    this.capture("assertion", { message, ...details });
    throw new Error(message);
  }

  capture(reason = "manual", details = {}) {
    const artifacts = {},
      artifactErrors = {};
    for (const [name, read] of Object.entries({
      cpu: () => this.core.inspect(),
      framebuffer: () => this.core.frame(),
      batterySave: () => this.core.exportSave(),
    })) {
      try {
        artifacts[name] = json(read());
      } catch (error) {
        artifactErrors[name] = errorInfo(error);
      }
    }
    if (!artifacts.cpu?.fault) {
      try {
        const state = this.core.saveState();
        if (state.length <= this.maxSnapshotBytes)
          artifacts.stateBase64 = encode(state);
        else
          artifactErrors.state = {
            message: "Final state exceeds maxSnapshotBytes",
          };
      } catch (error) {
        artifactErrors.state = errorInfo(error);
      }
    }
    let capturedDetails;
    try {
      capturedDetails = json(details);
      if (JSON.stringify(capturedDetails).length * 2 > this.maxHistoryBytes)
        capturedDetails = { truncated: true };
    } catch (error) {
      capturedDetails = { serializationError: errorInfo(error) };
    }
    const bundle = {
      format: "zigba-failure",
      version: 2,
      identity: json(this.identity),
      reason: String(reason).slice(0, 1024),
      details: capturedDetails,
      snapshots: this.snapshots.map((s) => ({
        cycle: s.cycle,
        nextOperation: s.nextOperation,
        stateBase64: encode(s.state),
      })),
      operations: this.operations,
      artifacts,
      artifactErrors,
      history: {
        replayable: this.snapshots.length > 0,
        droppedOperations: this.droppedOperations,
        droppedSnapshots: this.droppedSnapshots,
        budgetSnapshots: this.budgetSnapshots,
        snapshotError: this.snapshotError,
        audioReplay:
          "Snapshots restore machine audio state but reset the host resampler; use cold-boot scenarios for exact output sample comparisons.",
        inputTiming:
          "setKeys applies at beforeCycle in the worker. runUntil records requested cycles and consumed inputs; their application lies within the recorded run interval.",
        coldBootHistoryComplete:
          this.snapshots[0]?.nextOperation === 0 &&
          this.snapshots[0]?.cycle === "0",
        limits: {
          maxSnapshots: this.maxSnapshots,
          maxSnapshotBytes: this.maxSnapshotBytes,
          maxOperations: this.maxOperations,
          maxHistoryBytes: this.maxHistoryBytes,
        },
      },
    };
    this.lastCapture = bundle;
    if (this.onCapture) {
      try {
        this.onCapture(bundle);
      } catch (error) {
        bundle.captureCallbackError = errorInfo(error);
      }
    }
    return bundle;
  }
}

/** Validate the fields consumed by the browser inspector before rendering them. */
export function validateFailureBundle(bundle) {
  if (bundle?.format !== "zigba-failure" || bundle.version !== 2)
    throw new Error("Unsupported failure bundle");
  if (!bundle.identity || !/^[a-f0-9]{40}$/.test(bundle.identity.romHash))
    throw new Error("Invalid failure ROM identity");
  if (
    !bundle.history ||
    typeof bundle.history.replayable !== "boolean" ||
    typeof bundle.history.audioReplay !== "string"
  )
    throw new Error("Invalid failure history");
  if (
    !Array.isArray(bundle.snapshots) ||
    !Array.isArray(bundle.operations) ||
    !bundle.artifacts ||
    typeof bundle.artifacts !== "object"
  )
    throw new Error("Invalid failure evidence");
  const pixels = bundle.artifacts.framebuffer;
  if (
    pixels !== undefined &&
    (!Array.isArray(pixels) ||
      pixels.length !== 38400 ||
      pixels.some((p) => !Number.isInteger(p) || p < 0 || p > 65535))
  )
    throw new Error("Invalid failure framebuffer");
  for (const snapshot of bundle.snapshots) {
    if (
      typeof snapshot.stateBase64 !== "string" ||
      !/^(0|[1-9][0-9]*)$/.test(snapshot.cycle) ||
      !Number.isSafeInteger(snapshot.nextOperation) ||
      snapshot.nextOperation < 0
    )
      throw new Error("Invalid failure snapshot");
  }
  return bundle;
}

// The caller supplies the original ROM/BIOS/config by loading the destination core.
// No load operation or ROM/BIOS payload is embedded in a bundle.
function* replayOperations(core, bundle, { identity } = {}) {
  validateFailureBundle(bundle);
  if (core.inspect().romHash !== bundle.identity.romHash)
    throw new Error("Replay ROM identity mismatch");
  for (const [key, value] of Object.entries(bundle.identity)) {
    if (
      key !== "romHash" &&
      JSON.stringify(identity?.[key]) !== JSON.stringify(value)
    ) {
      throw new Error(`Replay identity mismatch: ${key}`);
    }
  }
  const snapshot = bundle.snapshots.at(-1);
  if (!bundle.history.replayable || !snapshot)
    throw new Error("No replayable snapshot retained");
  core.loadState(decode(snapshot.stateBase64));
  const results = [];
  let expectedSequence = snapshot.nextOperation;
  for (const operation of bundle.operations.filter(
    (op) => op.sequence >= snapshot.nextOperation,
  )) {
    if (operation.sequence !== expectedSequence++)
      throw new Error("Incomplete operation history");
    if (String(core.inspect().cycle) !== operation.beforeCycle)
      throw new Error("Replay cycle diverged before operation");
    const args = operation.args.slice();
    if (operation.method === "runUntil") {
      if (operation.cycleTypes?.deadline === "bigint")
        args[0] = BigInt(args[0]);
      if (operation.cycleTypes?.inputDeadline === "bigint")
        args[2] = BigInt(args[2]);
      if (Array.isArray(args[1]))
        args[1] = args[1].map((event, index) =>
          operation.cycleTypes?.events[index] === "bigint"
            ? { ...event, at: BigInt(event.at) }
            : event,
        );
    } else if (!["setKeys", "step", "drainAudio"].includes(operation.method))
      throw new Error("Unsupported replay operation");
    let result, caught;
    try {
      result = core[operation.method](...args);
    } catch (error) {
      caught = error;
    }
    if (
      JSON.stringify(caught ? errorInfo(caught) : undefined) !==
      JSON.stringify(operation.error)
    ) {
      throw new Error("Replay exception diverged");
    }
    if (String(core.inspect().cycle) !== operation.afterCycle)
      throw new Error("Replay cycle diverged after operation");
    if (
      operation.result &&
      JSON.stringify(json(result)) !== JSON.stringify(operation.result)
    )
      throw new Error("Replay result diverged");
    yield operation.sequence;
    results.push({
      sequence: operation.sequence,
      result,
      error: caught ? errorInfo(caught) : undefined,
    });
  }
  const inspection = core.inspect();
  if (JSON.stringify(json(inspection)) !== JSON.stringify(bundle.artifacts.cpu))
    throw new Error("Replay final CPU diverged");
  if (
    bundle.artifacts.stateBase64 &&
    encode(core.saveState()) !== bundle.artifacts.stateBase64
  )
    throw new Error("Replay final machine state diverged");
  for (const [name, value] of [
    ["framebuffer", core.frame()],
    ["batterySave", core.exportSave()],
  ]) {
    if (
      bundle.artifacts[name] &&
      JSON.stringify(json(value)) !== JSON.stringify(bundle.artifacts[name])
    )
      throw new Error(`Replay final ${name} diverged`);
  }
  return { inspection, results };
}

export function replayFailureBundle(core, bundle, options = {}) {
  const iterator = replayOperations(core, bundle, options);
  let next;
  do {
    next = iterator.next();
  } while (!next.done);
  return next.value;
}
export async function replayFailureBundleAsync(core, bundle, options = {}) {
  const iterator = replayOperations(core, bundle, options);
  let next,
    count = 0;
  do {
    if (options.signal?.aborted) return { status: "cancelled" };
    next = iterator.next();
    if (++count % 4 === 0)
      await new Promise((resolve) => setTimeout(resolve, 0));
  } while (!next.done);
  return { status: "passed", ...next.value };
}
