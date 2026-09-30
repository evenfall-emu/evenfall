import { wasmSimd } from "./core.js";
export { KEYS, CYCLES_PER_FRAME, SAVE_TYPES, CHEAT_FORMATS } from "./core.js";

/** An isolated emulator session. All methods serialize through its Worker. */
export class Emulator extends EventTarget {
  static async create({
    diagnosticsIntervalMs = 0,
    workerUrl,
    // The SIMD build where the engine supports it, the scalar build otherwise.
    wasmUrl = wasmSimd
      ? new URL("./evenfall-gba.wasm", import.meta.url)
      : new URL("./evenfall-gba-scalar.wasm", import.meta.url),
  } = {}) {
    // Keep these literal `new Worker(new URL(...))` and `new URL(...)` forms:
    // bundlers (Vite, webpack, Parcel) recognize them and emit the worker and
    // WASM with the application. Explicit URLs bypass that for static hosts.
    const worker =
      workerUrl === undefined
        ? new Worker(new URL("./worker.js", import.meta.url), {
            type: "module",
          })
        : new Worker(workerUrl, { type: "module" });
    const emulator = new Emulator(worker);
    try {
      await emulator.request("init", [
        String(wasmUrl),
        { diagnosticsIntervalMs },
      ]);
      return emulator;
    } catch (error) {
      emulator.terminate();
      throw error;
    }
  }
  /** Use Emulator.create(); the constructor adopts an already started Worker. */
  constructor(worker) {
    super();
    this.worker = worker;
    this.pending = new Map();
    this.nextId = 1;
    this.closed = false;
    this.worker.onmessage = ({ data }) => {
      if (data.event) {
        try {
          this.dispatchEvent(new CustomEvent(data.event, { detail: data }));
        } finally {
          if (data.event === "frame")
            this.worker.postMessage({ ack: data.generation });
        }
      } else {
        const handler = this.pending.get(data.id);
        if (!handler) return;
        this.pending.delete(data.id);
        if (data.error) handler.reject(new Error(data.error));
        else handler.resolve(data.result);
      }
    };
    this.worker.onerror = (event) => {
      const message = event.message || "Emulator worker failed";
      this.dispatchEvent(new CustomEvent("error", { detail: { message } }));
      this.terminate(message);
    };
  }
  request(method, args = []) {
    if (this.closed) return Promise.reject(new Error("SessionDisposed"));
    if (this.pending.size >= 128)
      return Promise.reject(new Error("CommandQueueFull"));
    const id = this.nextId++;
    return new Promise((resolve, reject) => {
      this.pending.set(id, { resolve, reject });
      try {
        this.worker.postMessage({ id, method, args });
      } catch (error) {
        this.pending.delete(id);
        reject(error);
      }
    });
  }
  recording(enabled) {
    return this.request("recording", [enabled]);
  }
  newSegment() {
    return this.request("segment");
  }
  checkpoint(name) {
    return this.request("checkpoint", [name]);
  }
  capture(note = "") {
    return this.request("capture", [note]);
  }
  exportScenario(name) {
    return this.request("exportScenario", [name]);
  }
  runScenario(manifest, assets) {
    return this.request("scenario", [manifest, assets]);
  }
  replay(bundle) {
    return this.request("replay", [bundle]);
  }
  cancel() {
    return this.request("cancel");
  }
  exportRom() {
    return this.request("exportRom");
  }
  load(image, options = {}) {
    return this.request("load", [image, options]);
  }
  setSpeed(multiplier) {
    return this.request("setSpeed", [multiplier]);
  }
  play() {
    return this.request("play");
  }
  pause() {
    return this.request("pause");
  }
  reset() {
    return this.request("reset");
  }
  resetWithSave(save) {
    return this.request("resetWithSave", [save]);
  }
  inspect() {
    return this.request("inspect");
  }
  readMemory(address, length) {
    return this.request("readMemory", [address, length]);
  }
  captureMemory() {
    return this.request("captureMemory");
  }
  setKeys(mask) {
    return this.request("setKeys", [mask]);
  }
  setCheats(cheats) {
    return this.request("setCheats", [cheats]);
  }
  cheats() {
    return this.request("cheats");
  }
  parseCheat(code, format = "auto") {
    return this.request("parseCheat", [code, format]);
  }
  runUntil(deadline, inputs = []) {
    return this.request("runUntil", [deadline, inputs]);
  }
  step() {
    return this.request("step");
  }
  frame() {
    return this.request("frame");
  }
  saveState() {
    return this.request("saveState");
  }
  loadState(bytes) {
    return this.request("loadState", [bytes]);
  }
  saveRewindBundle() {
    return this.request("saveRewindBundle");
  }
  loadRewindBundle(bytes) {
    return this.request("loadRewindBundle", [bytes]);
  }
  setRewindConfig(budgetBytes, intervalFrames, compression) {
    return this.request("rewindConfig", [
      budgetBytes,
      intervalFrames,
      compression,
    ]);
  }
  rewind(steps = 1) {
    return this.request("rewind", [steps]);
  }
  rewindDepth() {
    return this.request("rewindDepth");
  }
  rewindCheckpoints() {
    return this.request("rewindCheckpoints");
  }
  rewindFrame(index) {
    return this.request("rewindFrame", [index]);
  }
  exportSave() {
    return this.request("exportSave");
  }
  /** Explicitly relinquish presentation storage; supplied views are detached. */
  recyclePresentation(pixels, audio) {
    if (this.closed) return;
    const buffers = {};
    if (pixels?.buffer.byteLength) buffers.pixels = pixels.buffer;
    if (audio?.buffer.byteLength) buffers.audio = audio.buffer;
    const transfer = [...new Set(Object.values(buffers))];
    if (transfer.length)
      this.worker.postMessage({ recycle: buffers }, transfer);
  }
  async dispose() {
    if (this.closed) return;
    try {
      await this.request("dispose");
    } finally {
      this.terminate();
    }
  }
  terminate(message = "SessionDisposed") {
    if (this.closed) return;
    this.closed = true;
    this.worker.terminate();
    for (const { reject } of this.pending.values()) reject(new Error(message));
    this.pending.clear();
    this.dispatchEvent(
      new CustomEvent("close", { detail: { event: "close", message } }),
    );
  }
}
