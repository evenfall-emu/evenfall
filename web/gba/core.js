// Synchronous adapter for a single WASM instance. Works in Workers and Node.
export const CYCLES_PER_FRAME = 280896n;
export const KEYS = Object.freeze({
  a: 1,
  b: 2,
  select: 4,
  start: 8,
  right: 16,
  left: 32,
  up: 64,
  down: 128,
  r: 256,
  l: 512,
});
export const SAVE_TYPES = Object.freeze([
  "auto",
  "none",
  "sram",
  "flash64",
  "flash64-atmel",
  "flash128",
  "eeprom512",
  "eeprom8k",
]);
const reasons = [
  "deadline",
  "instruction_boundary_after_deadline",
  "power_stopped",
  "faulted",
];
const text = new TextDecoder();

function uint(value, max, label) {
  if (!Number.isInteger(value) || value < 0 || value > max)
    throw new RangeError(`Invalid ${label}`);
  return value;
}
function cycle(value) {
  if (typeof value !== "bigint" || value < 0n || value > 0xffffffffffffffffn)
    throw new RangeError("Cycle must be an unsigned 64-bit bigint");
  return value;
}
function bytes(value, label) {
  if (!(value instanceof Uint8Array))
    throw new TypeError(`${label} must be Uint8Array`);
  return value;
}

// Node's fetch has no file: support, so packaged binaries are read from disk
// there. process.getBuiltinModule is a runtime lookup, invisible to bundlers.
async function readWasm(source) {
  const href = String(source);
  if (
    href.startsWith("file:") &&
    typeof globalThis.process?.getBuiltinModule === "function"
  )
    return globalThis.process
      .getBuiltinModule("node:fs/promises")
      .readFile(new URL(href));
  const response = await fetch(source);
  if (!response.ok) throw new Error(`WASM fetch: ${response.status}`);
  return response.arrayBuffer();
}

export class WasmCore {
  static async create(
    source = new URL("./evenfall-gba.wasm", import.meta.url),
  ) {
    const data =
      source instanceof Uint8Array || source instanceof ArrayBuffer
        ? source
        : await readWasm(source);
    const { instance } = await WebAssembly.instantiate(data, {});
    return new WasmCore(instance.exports);
  }
  constructor(exports) {
    this.wasm = exports;
    this.loaded = false;
    this.timeline = null;
    this.sampleSequence = 0;
    if (exports.zb_version() !== 1) throw new Error("Unsupported WASM ABI");
  }
  replaceTimeline() {
    this.timeline = crypto.randomUUID();
    this.sampleSequence = 0;
  }
  check(result) {
    if (result < 0)
      throw new Error(
        text.decode(
          new Uint8Array(
            this.wasm.memory.buffer,
            this.wasm.zb_error_ptr(),
            this.wasm.zb_error_len(),
          ),
        ),
      );
    return result;
  }
  requireLoaded() {
    if (!this.loaded) throw new Error("NoSession");
  }
  upload(data) {
    this.check(this.wasm.zb_prepare(data.length));
    if (data.length)
      new Uint8Array(
        this.wasm.memory.buffer,
        this.wasm.zb_input_ptr(),
        data.length,
      ).set(data);
  }
  output() {
    const length = this.wasm.zb_output_len();
    return length
      ? new Uint8Array(
          this.wasm.memory.buffer,
          this.wasm.zb_output_ptr(),
          length,
        ).slice()
      : new Uint8Array();
  }
  load(image, options = {}) {
    bytes(image, "image");
    const bios = bytes(options.bios ?? new Uint8Array(), "bios");
    const save = bytes(options.save ?? new Uint8Array(), "save");
    const saveType =
      options.saveType === undefined
        ? -1
        : SAVE_TYPES.indexOf(options.saveType);
    if (options.saveType !== undefined && saveType === -1)
      throw new Error("Invalid save type");
    const rtc =
      options.rtc === undefined
        ? -1
        : options.rtc === true
          ? 1
          : options.rtc === false
            ? 0
            : NaN;
    if (!Number.isInteger(rtc)) throw new Error("Invalid RTC option");
    const epoch = options.rtcEpoch ?? 0n;
    cycle(epoch);
    const sampleRate = uint(options.sampleRate ?? 48000, 192000, "sample rate");
    if (image.length + bios.length + save.length > 41 * 1024 * 1024)
      throw new RangeError("InputTooLarge");
    this.check(this.wasm.zb_prepare(image.length + bios.length + save.length));
    const payload =
      image.length + bios.length + save.length
        ? new Uint8Array(
            this.wasm.memory.buffer,
            this.wasm.zb_input_ptr(),
            image.length + bios.length + save.length,
          )
        : new Uint8Array();
    payload.set(image);
    payload.set(bios, image.length);
    payload.set(save, image.length + bios.length);
    const zipped = options.zipped ?? (image[0] === 0x50 && image[1] === 0x4b);
    this.check(
      this.wasm.zb_load(
        image.length,
        bios.length,
        save.length,
        Number(zipped),
        saveType,
        rtc,
        epoch,
        sampleRate,
      ),
    );
    this.loaded = true;
    this.romHash = undefined;
    this.replaceTimeline();
    return this.inspect();
  }
  // Cheap probes for hosts that do not need a copied register file or fault text.
  currentCycle() {
    this.requireLoaded();
    return this.wasm.zb_cycle();
  }
  hasFault() {
    this.requireLoaded();
    return this.check(this.wasm.zb_fault()) === 1;
  }
  inspect() {
    this.requireLoaded();
    const w = this.wasm;
    const ptr = w.zb_cpu_ptr();
    const cpu = new Uint32Array(w.memory.buffer, ptr, 18).slice();
    const hash = (this.romHash ??= Array.from(
      new Uint8Array(w.memory.buffer, w.zb_rom_hash_ptr(), 20),
      (b) => b.toString(16).padStart(2, "0"),
    ).join(""));
    let fault = null;
    if (this.check(w.zb_fault()) === 1) fault = text.decode(this.output());
    return {
      cycle: w.zb_cycle(),
      frames: w.zb_frames(),
      registers: cpu.slice(0, 16),
      cpsr: cpu[16],
      nextInstructionAddress: cpu[17],
      romHash: hash,
      rtcEnabled:
        typeof w.zb_rtc_enabled === "function"
          ? Boolean(w.zb_rtc_enabled())
          : undefined,
      saveType: SAVE_TYPES[w.zb_save_type()],
      fault,
    };
  }
  setKeys(mask) {
    this.requireLoaded();
    this.check(this.wasm.zb_keys(uint(mask, 1023, "key mask")));
  }
  runUntil(deadline, events = [], inputDeadline = deadline) {
    this.requireLoaded();
    cycle(deadline);
    cycle(inputDeadline);
    if (events.length > 1024)
      throw new RangeError("At most 1024 events per bounded run");
    const packet = new Uint8Array(events.length * 16);
    const view = new DataView(packet.buffer);
    let previous = this.wasm.zb_cycle();
    for (let i = 0; i < events.length; i++) {
      const { at, keys } = events[i];
      cycle(at);
      if (at < previous)
        throw new RangeError(
          "Input events must be ordered and not in the past",
        );
      previous = at;
      view.setBigUint64(i * 16, at, true);
      view.setUint32(i * 16 + 8, uint(keys, 1023, "key mask"), true);
    }
    this.upload(packet);
    const status = this.check(
      this.wasm.zb_run_until(deadline, events.length, inputDeadline),
    );
    return {
      cycle: this.wasm.zb_cycle(),
      frames: this.wasm.zb_frames(),
      reason: reasons[status],
      inputsConsumed: this.wasm.zb_inputs_consumed(),
    };
  }
  step() {
    this.requireLoaded();
    return {
      reason: ["executed", "halted", "power_stopped", "faulted"][
        this.check(this.wasm.zb_step())
      ],
      ...this.inspect(),
    };
  }
  readMemory(address, length) {
    this.requireLoaded();
    this.check(
      this.wasm.zb_read_memory(
        uint(address, 0xffffffff, "address"),
        uint(length, 1048576, "length"),
      ),
    );
    return this.output();
  }
  // One synchronous worker turn, no execution between these copied reads.
  // Deliberately RAM-only: MMIO/save protocols and ROM caching are separate.
  captureMemory() {
    const { cycle, romHash } = this.inspect();
    return {
      cycle,
      romHash,
      timeline: this.timeline,
      sequence: ++this.sampleSequence,
      regions: [
        { address: 0x02000000, bytes: this.readMemory(0x02000000, 0x40000) },
        { address: 0x03000000, bytes: this.readMemory(0x03000000, 0x8000) },
      ],
    };
  }
  frame(output) {
    this.requireLoaded();
    // Explicit observations retain their exact-cycle meaning during skips.
    if (this.wasm.zb_canonical_frame)
      this.check(this.wasm.zb_canonical_frame());
    const frame = new Uint16Array(
      this.wasm.memory.buffer,
      this.wasm.zb_frame_ptr(),
      240 * 160,
    );
    if (output === undefined) return frame.slice();
    if (!(output instanceof Uint16Array) || output.length !== frame.length)
      throw new RangeError("Invalid frame buffer");
    output.set(frame);
    return output;
  }
  canSnapshot() {
    this.requireLoaded();
    return this.wasm.zb_frame_canonical
      ? Boolean(this.wasm.zb_frame_canonical())
      : true;
  }
  setVideoInterval(interval) {
    this.requireLoaded();
    uint(interval, 16, "video interval");
    if (interval < 1) throw new RangeError("Invalid video interval");
    this.check(this.wasm.zb_video_interval(interval));
  }
  videoFrameId() {
    this.requireLoaded();
    return this.wasm.zb_video_frame_id();
  }
  videoFrameCycle() {
    this.requireLoaded();
    return this.wasm.zb_video_frame_cycle();
  }
  // Cheap, stable last selected image for presentation; does not reconstruct.
  presentationFrame(output) {
    this.requireLoaded();
    const frame = new Uint16Array(
      this.wasm.memory.buffer,
      this.wasm.zb_present_frame_ptr(),
      240 * 160,
    );
    if (output === undefined) return frame.slice();
    if (!(output instanceof Uint16Array) || output.length !== frame.length)
      throw new RangeError("Invalid frame buffer");
    output.set(frame);
    return output;
  }
  drainAudio(output) {
    this.requireLoaded();
    // Validate before draining so a bad destination cannot discard samples.
    if (
      output !== undefined &&
      (!(output instanceof Int16Array) || output.length < 16384)
    )
      throw new RangeError("Invalid audio buffer");
    const count = this.check(this.wasm.zb_drain_audio());
    const samples = new Int16Array(
      this.wasm.memory.buffer,
      this.wasm.zb_audio_ptr(),
      count * 2,
    );
    if (output === undefined) return samples.slice();
    output.set(samples);
    return output.subarray(0, samples.length);
  }
  // Host-only preference: portable states retain the same sound circuitry.
  setAudioOutput(enabled) {
    this.requireLoaded();
    if (typeof enabled !== "boolean")
      throw new TypeError("Audio output must be boolean");
    this.check(this.wasm.zb_audio_output(Number(enabled)));
  }
  exportRom() {
    this.requireLoaded();
    this.check(this.wasm.zb_export_rom());
    return this.output();
  }
  exportSave() {
    this.requireLoaded();
    this.check(this.wasm.zb_export_save());
    return this.output();
  }
  resetWithSave(save) {
    this.requireLoaded();
    this.upload(bytes(save, "save"));
    this.check(this.wasm.zb_reset_save());
    this.replaceTimeline();
    return this.inspect();
  }
  saveState() {
    this.requireLoaded();
    this.check(this.wasm.zb_write_state());
    return this.output();
  }
  loadState(state) {
    this.requireLoaded();
    this.upload(bytes(state, "state"));
    this.check(this.wasm.zb_load_state());
    this.replaceTimeline();
    return this.inspect();
  }
  saveRewindBundle() {
    this.requireLoaded();
    this.check(this.wasm.zb_write_rewind_bundle());
    return this.output();
  }
  loadRewindBundle(bundle) {
    this.requireLoaded();
    this.upload(bytes(bundle, "rewind bundle"));
    this.check(this.wasm.zb_load_rewind_bundle());
    this.replaceTimeline();
    return this.inspect();
  }
  // Budget-bounded engine rewind history. The byte budget bounds stored
  // history (0 disables); resolution coarsens with age. The interval is the
  // nominal 1x capture cadence in frames; compression is 'delta' (anchored
  // word-run deltas), 'fast' (level-1 deflate) or 'off'. Stored bytes stay
  // opaque to callers.
  setRewindConfig(
    budgetBytes = 15 * 1024 * 1024,
    intervalFrames = 60,
    compression = "delta",
  ) {
    this.requireLoaded();
    const budget = uint(budgetBytes, 64 * 1024 * 1024, "rewind budget");
    const interval = uint(intervalFrames, 65535, "rewind interval");
    const mode = { off: 0, fast: 1, delta: 2 }[compression];
    if (!Number.isInteger(mode))
      throw new RangeError("Invalid rewind compression");
    this.check(this.wasm.zb_rewind_config(budget, interval, mode));
  }
  // Private host-policy notification. Q8 avoids per-frame clock input while
  // keeping capture cadence in Zig.
  setRewindSpeed(multiplier) {
    this.requireLoaded();
    if (!Number.isFinite(multiplier) || multiplier < 0.5 || multiplier > 16)
      throw new RangeError("Invalid rewind speed");
    this.check(this.wasm.zb_rewind_speed(Math.round(multiplier * 256)));
  }
  rewind(steps = 1) {
    this.requireLoaded();
    this.check(this.wasm.zb_rewind(uint(steps, 1024, "rewind steps")));
    this.replaceTimeline();
    return this.inspect();
  }
  rewindDepth() {
    this.requireLoaded();
    return this.wasm.zb_rewind_depth();
  }
  rewindInfo(index) {
    this.requireLoaded();
    this.check(
      this.wasm.zb_rewind_info(uint(index, 0xffffffff, "rewind index")),
    );
    const out = this.output();
    const view = new DataView(out.buffer, out.byteOffset, out.byteLength);
    return {
      frame: view.getBigUint64(0, true),
      cycle: view.getBigUint64(8, true),
      bytes: Number(view.getBigUint64(16, true)),
    };
  }
  rewindFrame(index) {
    this.requireLoaded();
    this.check(
      this.wasm.zb_rewind_frame(uint(index, 0xffffffff, "rewind index")),
    );
    const out = this.output();
    if (out.byteLength !== 240 * 160 * 2)
      throw new Error("Invalid rewind frame");
    return new Uint16Array(out.buffer, out.byteOffset, 240 * 160).slice();
  }
  dispose() {
    this.wasm.zb_dispose();
    this.loaded = false;
    this.timeline = null;
  }
}
