import assert from "node:assert/strict";
import { execFileSync } from "node:child_process";
import { createHash } from "node:crypto";
import { mkdtemp, readFile, rm, writeFile } from "node:fs/promises";
import { tmpdir } from "node:os";
import { join } from "node:path";
import { crc32 } from "node:zlib";
import { afterAll, beforeAll, describe } from "vite-plus/test";
import { WasmCore, CYCLES_PER_FRAME as F } from "./core.js";
import { scalarWasm, wasm, wasmTest as test } from "./scripts/wasm-test.mjs";

const loop = new Uint8Array([0xfe, 0xff, 0xff, 0xea]);
const romSha = (bytes) => createHash("sha1").update(bytes).digest("hex");
const sha = (bytes) => createHash("sha256").update(bytes).digest("hex");
const asBytes = (array) =>
  new Uint8Array(array.buffer, array.byteOffset, array.byteLength);
function zip(image) {
  const name = Buffer.from("fixture.gba");
  const local = Buffer.alloc(30);
  local.writeUInt32LE(0x04034b50);
  local.writeUInt32LE(crc32(image), 14);
  local.writeUInt32LE(image.length, 18);
  local.writeUInt32LE(image.length, 22);
  local.writeUInt16LE(name.length, 26);
  const central = Buffer.alloc(46);
  central.writeUInt32LE(0x02014b50);
  central.writeUInt32LE(crc32(image), 16);
  central.writeUInt32LE(image.length, 20);
  central.writeUInt32LE(image.length, 24);
  central.writeUInt16LE(name.length, 28);
  const end = Buffer.alloc(22);
  end.writeUInt32LE(0x06054b50);
  end.writeUInt16LE(1, 8);
  end.writeUInt16LE(1, 10);
  end.writeUInt32LE(central.length + name.length, 12);
  end.writeUInt32LE(local.length + name.length + image.length, 16);
  return Buffer.concat([local, name, image, central, name, end]);
}

test("a module without the ef_ ABI is rejected by name", () => {
  for (const exports of [{}, { zb_version: () => 2 }, { ef_version: () => 1 }])
    assert.throws(() => new WasmCore(exports), /Unsupported WASM ABI/);
});

test("WASM ABI: bounded execution, zero-cycle inspection, input, and isolation", async () => {
  const a = await WasmCore.create(wasm),
    b = await WasmCore.create(wasm);
  try {
    a.load(loop);
    b.load(loop);
    const before = a.inspect();
    assert.equal(a.readMemory(0x08000000, 4).toString(), loop.toString());
    assert.deepEqual(a.inspect(), before);
    assert.throws(() => a.runUntil(F + 1n), /ExecutionBudgetExceeded/);
    const result = a.runUntil(1000n, [
      { at: 10n, keys: 1 },
      { at: 100n, keys: 0 },
      { at: 2000n, keys: 2 },
    ]);
    assert.equal(result.inputsConsumed, 2);
    assert.equal(result.reason, "instruction_boundary_after_deadline");
    assert.equal(b.inspect().cycle, 0n);
    a.setKeys(9);
    assert.equal(
      new DataView(a.readMemory(0x04000130, 2).buffer).getUint16(0, true) &
        1023,
      1023 ^ 9,
    );
    assert.throws(() => a.readMemory(0xffffffff, 2), /AddressOverflow/);
    assert.throws(() => a.setKeys(1024), /Invalid key mask/);
    assert.throws(() => a.runUntil(-1n), /Cycle/);
    assert.throws(() => a.runUntil(2000n, [{ at: 3n, keys: 0 }]), /past/);
  } finally {
    a.dispose();
    b.dispose();
  }
});

test("coherent RAM samples are owned, zero-cycle and identify timeline replacement", async () => {
  const core = await WasmCore.create(wasm);
  try {
    assert.throws(() => core.captureMemory(), /NoSession/);
    core.load(loop);
    core.runUntil(1000n);
    const state = core.saveState();
    const first = core.captureMemory();
    assert.equal(first.cycle, core.currentCycle());
    assert.equal(first.romHash, romSha(loop));
    assert.deepEqual(
      first.regions.map((r) => [r.address, r.bytes.length]),
      [
        [0x02000000, 0x40000],
        [0x03000000, 0x8000],
      ],
    );
    first.regions[0].bytes.fill(0xa5);
    const second = core.captureMemory();
    assert.equal(second.regions[0].bytes[0], 0);
    assert.equal(second.timeline, first.timeline);
    assert.equal(second.sequence, first.sequence + 1);
    assert.deepEqual(core.saveState(), state);
    assert.throws(() => core.loadState(new Uint8Array()));
    assert.equal(core.captureMemory().timeline, first.timeline);
    core.loadState(state);
    const restored = core.captureMemory();
    assert.notEqual(restored.timeline, first.timeline);
    assert.equal(restored.sequence, 1);
    assert.equal(restored.cycle, first.cycle);
    core.resetWithSave(new Uint8Array());
    assert.notEqual(core.captureMemory().timeline, restored.timeline);
    core.setRewindConfig(undefined, 1, "off");
    for (let i = 0; i < 3; i++) {
      core.runUntil(core.currentCycle() + F);
      core.drainAudio();
    }
    const later = core.captureMemory();
    core.rewind();
    const rewound = core.captureMemory();
    assert.notEqual(rewound.timeline, later.timeline);
    core.load(loop);
    assert.notEqual(core.captureMemory().timeline, rewound.timeline);
  } finally {
    core.dispose();
  }
});

test("cheap execution probes and cached media identity survive loads, resets and faults", async () => {
  const core = await WasmCore.create(wasm);
  try {
    assert.throws(() => core.currentCycle(), /NoSession/);
    assert.throws(() => core.hasFault(), /NoSession/);
    core.load(loop);
    core.runUntil(F / 4n);
    const state = core.saveState();
    assert.equal(core.currentCycle(), core.inspect().cycle);
    assert.equal(core.hasFault(), false);
    assert.deepEqual(core.saveState(), state);
    assert.throws(() => core.load(new Uint8Array()));
    assert.equal(core.inspect().romHash, romSha(loop));
    core.resetWithSave(new Uint8Array());
    assert.equal(core.currentCycle(), 0n);
    assert.equal(core.inspect().romHash, romSha(loop));
    const faultRom = new Uint8Array([0, 0, 255, 239]);
    core.load(faultRom);
    core.runUntil(F);
    assert.equal(core.hasFault(), true);
    assert.equal(core.currentCycle(), core.inspect().cycle);
    assert.equal(core.inspect().romHash, romSha(faultRom));
    core.load(loop);
    assert.equal(core.hasFault(), false);
    assert.equal(core.inspect().romHash, romSha(loop));
  } finally {
    core.dispose();
  }
});

test("host audio suppression survives quickload and reset without changing state bytes", async () => {
  const core = await WasmCore.create(wasm);
  try {
    assert.throws(() => core.setAudioOutput(false), /NoSession/);
    core.load(loop);
    core.runUntil(F);
    const state = core.saveState();
    core.setAudioOutput(false);
    assert.deepEqual(core.saveState(), state);
    assert.equal(core.drainAudio().length, 0);
    assert.throws(() => core.setAudioOutput(1), /boolean/);
    core.loadState(state);
    core.runUntil(2n * F);
    assert.equal(core.drainAudio().length, 0);
    core.resetWithSave(new Uint8Array());
    core.runUntil(F);
    assert.equal(core.drainAudio().length, 0);
    core.setAudioOutput(true);
    core.runUntil(2n * F);
    assert.ok(core.drainAudio().length > 0);
  } finally {
    core.dispose();
  }
});

test("snapshot round trip, failed loads, and owned output buffers", async () => {
  const core = await WasmCore.create(wasm);
  try {
    core.load(loop);
    core.runUntil(F);
    core.drainAudio();
    const saved = core.saveState();
    const before = core.inspect();
    core.runUntil(2n * F);
    const expected = core.saveState();
    core.loadState(saved);
    assert.deepEqual(core.inspect(), before);
    core.runUntil(2n * F);
    assert.deepEqual(core.saveState(), expected);
    const corrupt = saved.slice();
    corrupt[0] ^= 1;
    const live = core.saveState();
    assert.throws(() => core.loadState(corrupt));
    assert.deepEqual(core.saveState(), live);
    assert.throws(
      () => core.load(loop, { bios: new Uint8Array(10) }),
      /InvalidBiosSize/,
    );
    assert.deepEqual(core.saveState(), live);
    const owned = core.frame();
    core.load(loop);
    assert.equal(owned.length, 38400);
    core.dispose();
    assert.throws(() => core.inspect(), /NoSession/);
  } finally {
    core.dispose();
  }
});

test("rewind bundles restore a branch with its history and replace observations only on success", async () => {
  const core = await WasmCore.create(wasm);
  try {
    assert.throws(() => core.saveRewindBundle(), /NoSession/);
    core.load(loop);
    core.setRewindConfig(undefined, 2);
    for (let i = 0; i < 7; i++) {
      core.runUntil(core.currentCycle() + F);
      core.drainAudio();
    }
    const live = core.saveState();
    const infos = Array.from({ length: 3 }, (_, i) => core.rewindInfo(i));
    assert.deepEqual(
      infos.map((info) => info.frame),
      [2n, 4n, 6n],
    );
    const observation = core.captureMemory();
    const bundle = core.saveRewindBundle();
    assert.deepEqual(core.saveState(), live);
    assert.equal(core.timeline, observation.timeline);
    core.rewind(2);
    core.setKeys(1);
    core.runUntil(core.currentCycle() + F);
    const branch = core.saveRewindBundle();
    const branchedObservation = core.captureMemory();
    const corrupt = bundle.slice();
    corrupt[corrupt.length - 1] ^= 1;
    assert.throws(() => core.loadRewindBundle(corrupt), /InvalidRewindBundle/);
    assert.equal(core.timeline, branchedObservation.timeline);
    assert.deepEqual(core.saveRewindBundle(), branch);
    assert.equal(core.loadRewindBundle(bundle).frames, 7n);
    assert.deepEqual(core.saveState(), live);
    assert.equal(core.rewindDepth(), 3);
    assert.deepEqual(
      Array.from({ length: 3 }, (_, i) => core.rewindInfo(i)),
      infos,
    );
    const restoredObservation = core.captureMemory();
    assert.notEqual(restoredObservation.timeline, branchedObservation.timeline);
    assert.equal(restoredObservation.sequence, 1);
    core.runUntil(core.currentCycle() + F);
    assert.equal(core.rewindDepth(), 4);
    assert.equal(core.rewind(3).frames, 2n);
    core.loadState(live);
    assert.equal(core.rewindDepth(), 0); // The existing state-only API still clears history.
  } finally {
    core.dispose();
  }
});

test("rewind parts restore a bundle's history and copy out only unknown payloads", async () => {
  const core = await WasmCore.create(wasm);
  try {
    core.load(loop);
    core.setRewindConfig(undefined, 2);
    const run = (frames) => {
      for (let i = 0; i < frames; i++) {
        core.runUntil(core.currentCycle() + F);
        core.drainAudio();
      }
    };
    run(7);
    const first = core.saveRewindParts();
    assert.deepEqual(first.state, core.saveState());
    assert.deepEqual([...first.payloads.keys()], first.keys);
    const store = new Map(first.payloads);
    run(4);
    const next = core.saveRewindParts(store.keys());
    // Checkpoints at frames 8 and 10 against the same anchor.
    assert.equal(next.payloads.size, 2);
    for (const [key, value] of next.payloads) store.set(key, value);
    const bundle = core.saveRewindBundle();
    core.loadState(first.state);
    const partial = new Map(store);
    partial.delete(next.keys[0]);
    assert.throws(
      () => core.loadRewindParts(next.state, next.index, partial),
      /InvalidRewindBundle/,
    );
    assert.equal(core.rewindDepth(), 0);
    assert.equal(
      core.loadRewindParts(next.state, next.index, store).frames,
      11n,
    );
    assert.deepEqual(core.saveRewindBundle(), bundle);
    assert.throws(() => core.saveRewindParts(["not-a-key"]), /hex/);
  } finally {
    core.dispose();
  }
});

test("maximum-budget bundle can replace resident history with a 32 MiB ROM", async () => {
  const core = await WasmCore.create(wasm);
  try {
    const rom = new Uint8Array(32 * 1024 * 1024);
    rom.set(loop);
    core.load(rom);
    core.setRewindConfig(64 * 1024 * 1024, 1, "off");
    const count = Math.ceil((64 * 1024 * 1024) / core.saveState().length) + 2;
    for (let i = 0; i < count; i++) {
      core.runUntil(core.currentCycle() + F);
      core.drainAudio();
    }
    const live = core.saveState(),
      depth = core.rewindDepth();
    const bundle = core.saveRewindBundle();
    assert.ok(bundle.length > 63 * 1024 * 1024);
    const beforeImport = core.wasm.memory.buffer.byteLength;
    // Repeated transfers must not keep prior WASM output/input allocations alive.
    for (let i = 0; i < 4; i++) {
      core.loadRewindBundle(bundle);
      if (i === 0) {
        // Upload reuses the released export; validation needs scratch, not
        // another history-sized allocation.
        assert.ok(
          core.wasm.memory.buffer.byteLength - beforeImport < 8 * 1024 * 1024,
        );
      }
      assert.equal(core.rewindDepth(), depth);
      assert.deepEqual(core.saveState(), live);
      assert.deepEqual(core.saveRewindBundle(), bundle);
    }
    assert.ok(core.wasm.memory.buffer.byteLength < 300 * 1024 * 1024);
    const warm = core.wasm.memory.buffer.byteLength;
    const corrupt = bundle.slice();
    corrupt[corrupt.length - 1] ^= 1;
    assert.throws(() => core.loadRewindBundle(corrupt), /InvalidRewindBundle/);
    assert.deepEqual(core.saveRewindBundle(), bundle);
    assert.equal(core.wasm.memory.buffer.byteLength, warm);
    assert.throws(
      () => core.upload(new Uint8Array(68 * 1024 * 1024 + 1)),
      /InputTooLarge/,
    );
    assert.deepEqual(core.saveState(), live);

    // A fresh instance has no freed, separately allocated old history to hide
    // a leak: captures must reuse the adopted bundle's evicted payload space.
    const resumed = await WasmCore.create(wasm);
    try {
      resumed.load(rom);
      resumed.loadRewindBundle(bundle);
      const imported = resumed.wasm.memory.buffer.byteLength;
      const oldest = resumed.rewindInfo(0).frame;
      for (let i = 0; i < count * 2; i++) {
        resumed.runUntil(resumed.currentCycle() + F);
        resumed.drainAudio();
      }
      assert.ok(
        resumed.wasm.memory.buffer.byteLength - imported < 8 * 1024 * 1024,
      );
      assert.equal(resumed.rewindInfo(0).frame, oldest);
      assert.equal(resumed.rewind(resumed.rewindDepth() - 1).frames, oldest);
      resumed.loadRewindBundle(bundle);
      assert.deepEqual(resumed.saveState(), live);
    } finally {
      resumed.dispose();
    }
  } finally {
    core.dispose();
  }
});

test("budgeted engine rewind restores earlier frames and rejects bad configs", async () => {
  const core = await WasmCore.create(wasm);
  try {
    core.load(loop);
    assert.equal(core.rewindDepth(), 0);
    assert.throws(() => core.rewind(1), /RewindEmpty/);
    assert.throws(
      () => core.setRewindConfig(64 * 1024 * 1024 + 1),
      /rewind budget/,
    );
    assert.throws(
      () => core.setRewindConfig(undefined, 0),
      /InvalidRewindConfig/,
    );
    assert.throws(
      () => core.setRewindConfig(undefined, 1, "zstd"),
      /rewind compression/,
    );
    assert.throws(
      () => core.setRewindConfig(undefined, 1, 2),
      /rewind compression/,
    );
    assert.equal(core.rewindDepth(), 0);
    core.setRewindConfig(undefined, 1);
    for (let i = 0; i < 3; i++) {
      core.runUntil(core.currentCycle() + F);
      core.drainAudio();
    }
    assert.equal(core.rewindDepth(), 3);
    const info = core.rewindInfo(2);
    assert.equal(info.frame, 3n);
    // Default 'delta' compression stores a fraction of the raw state bytes.
    assert.ok(info.bytes > 0 && info.bytes < core.saveState().length / 4);
    assert.throws(() => core.rewindInfo(3), /InvalidRewindSteps/);
    const live = core.saveState();
    const depth = core.rewindDepth();
    const preview = core.rewindFrame(0);
    assert.equal(preview.length, 240 * 160);
    assert.deepEqual(core.saveState(), live);
    assert.equal(core.rewindDepth(), depth);
    assert.throws(() => core.rewindFrame(3), /InvalidRewindSteps/);
    const memoryAfterWarmup = core.wasm.memory.buffer.byteLength;
    for (let i = 0; i < 128; i++)
      assert.throws(() => core.rewindFrame(3), /InvalidRewindSteps/);
    assert.equal(
      core.wasm.memory.buffer.byteLength,
      memoryAfterWarmup,
      "failed previews must reuse the output allocation, not leak it",
    );
    core.runUntil(core.currentCycle() + F);
    core.drainAudio();
    const atFour = core.saveState();
    assert.equal(core.rewind().frames, 3n);
    assert.equal(core.rewindDepth(), 3);
    core.runUntil(core.currentCycle() + F);
    assert.deepEqual(core.saveState(), atFour);
    // Walking back through delta entries lands on each earlier frame exactly.
    assert.equal(core.rewind(3).frames, 1n);
    assert.throws(() => core.rewind(0), /InvalidRewindSteps/);
    // Explicit 'off' stores whole states and still rewinds exactly.
    core.setRewindConfig(undefined, 1, "off");
    assert.equal(core.rewindDepth(), 0);
    core.runUntil(core.currentCycle() + F);
    core.drainAudio();
    assert.equal(core.rewindInfo(0).bytes, core.saveState().length);
  } finally {
    core.dispose();
  }
});

test("internal worker yields preserve input timestamps crossed by an instruction", async () => {
  const a = await WasmCore.create(wasm),
    b = await WasmCore.create(wasm);
  try {
    a.load(loop);
    b.load(loop);
    const events = [{ at: F / 4n + 1n, keys: 1 }];
    a.runUntil(F / 2n, events);
    const first = b.runUntil(F / 4n, events, F / 2n);
    assert.equal(first.inputsConsumed, 1);
    b.runUntil(F / 2n, events.slice(first.inputsConsumed));
    assert.deepEqual(a.saveState(), b.saveState());
  } finally {
    a.dispose();
    b.dispose();
  }
});

test("shared ZIP importer rejects malformed lengths without trapping the WASM instance", async () => {
  const core = await WasmCore.create(wasm);
  try {
    core.load(zip(loop));
    assert.equal(core.inspect().romHash, romSha(loop));
    for (const offset of [16, 20, 24, 42]) {
      const malformed = zip(loop);
      const centralOffset = 30 + "fixture.gba".length + loop.length;
      malformed.writeUInt32LE(0xfffffff0, centralOffset + offset);
      assert.throws(() => core.load(malformed));
      assert.equal(core.inspect().romHash, romSha(loop));
    }
    const badCrc = zip(loop);
    badCrc.writeUInt32LE(0, 30 + "fixture.gba".length + loop.length + 16);
    assert.throws(() => core.load(badCrc), /ZipCrcMismatch/);
    assert.throws(
      () => core.load(new Uint8Array(), { zipped: false }),
      /InvalidRomSize/,
    );
  } finally {
    core.dispose();
  }
});

test("STOP and unsupported guest code remain explicit terminal results", async () => {
  const core = await WasmCore.create(wasm);
  try {
    core.load(new Uint8Array([0, 0, 3, 0xef]));
    assert.equal(core.runUntil(F).reason, "power_stopped");
    const at = core.inspect().cycle;
    assert.equal(core.runUntil(at + F).reason, "power_stopped");
    assert.equal(core.inspect().cycle, at);
    core.load(new Uint8Array([0, 0, 0xff, 0xef]));
    assert.equal(core.runUntil(F).reason, "faulted");
    const fault = core.inspect();
    assert.ok(fault.fault);
    assert.equal(core.runUntil(fault.cycle + F).reason, "faulted");
    assert.deepEqual(core.inspect(), fault);
    assert.throws(() => core.saveState(), /MachineFaulted/);
  } finally {
    core.dispose();
  }
});

test("a DMA burst that holds the bus reports dma_stall while video runs on", async () => {
  // Direct Sound DMA1/DMA2 refill FIFO A/B from IWRAM while timers 0 and 1
  // overflow every cycle, then the CPU counts in r2.
  const rom = Buffer.from(
    "0103a0e38010a0e3b418c0e1fb1ca0e3b218c0e13c109fe5bc1080e538109fe5" +
      "c01080e534109fe5b61cc0e124109fe5c81080e528109fe5cc1080e51c109fe5" +
      "b21dc0e11c109fe5001180e5041180e5012082e2fdffffea00200003a0000004" +
      "40b70000a4000004ffff8000",
    "hex",
  );
  const core = await WasmCore.create(wasm);
  try {
    core.load(new Uint8Array(rom));
    // The call that reaches the in-access limit returns about 2^24 cycles
    // late; the CPU is stalled from then on.
    assert.equal(
      core.runUntil(F).reason,
      "instruction_boundary_after_deadline",
    );
    const stalled = core.inspect();
    assert.ok(stalled.cycle > 1n << 24n);
    const step = core.step();
    assert.equal(step.reason, "dma_stall");
    assert.equal(step.cycle, stalled.cycle);
    assert.throws(() => core.saveState());
    const deadline = stalled.cycle + F;
    const run = core.runUntil(deadline);
    assert.ok(run.cycle >= deadline && run.cycle < deadline + 16n);
    const after = core.inspect();
    assert.equal(after.frames, stalled.frames + 1n);
    assert.deepEqual(after.registers, stalled.registers);
    assert.equal(after.nextInstructionAddress, stalled.nextInstructionAddress);
    assert.equal(core.step().reason, "dma_stall");
  } finally {
    core.dispose();
  }
});

const extra = process.env.EVENFALL_TEST_ROMS
  ? JSON.parse(process.env.EVENFALL_TEST_ROMS)
  : [];
describe.skipIf(!wasm)(
  "native/WASM canonical state, pixels, audio, memory, and save parity",
  () => {
    let directory, file;
    beforeAll(async () => {
      const root = new URL("../..", import.meta.url);
      execFileSync(
        "cargo",
        [
          "build",
          "--locked",
          "--release",
          "-p",
          "evenfall-gba",
          "--bin",
          "session_probe",
        ],
        {
          cwd: root,
          timeout: 120000,
        },
      );
      directory = await mkdtemp(join(tmpdir(), "evenfall-web-test-"));
      file = join(directory, "loop.gba");
      await writeFile(file, loop);
    });
    afterAll(() => directory && rm(directory, { recursive: true }));
    // The SIMD module (the default) and the scalar fallback must both equal
    // the native core.
    const modules = [["simd128", wasm]];
    if (scalarWasm) modules.push(["scalar", scalarWasm]);
    for (const [variant, module] of modules)
      for (const rom of [null, ...extra])
        test(`${variant}: ${rom ?? "loop.gba"}`, async () => {
          const path = rom ?? file;
          const core = await WasmCore.create(module);
          try {
            core.load(await readFile(path), {
              zipped: path.toLowerCase().endsWith(".zip"),
            });
            const audio = createHash("sha256");
            for (let target = F / 4n; target <= 120n * F; target += F / 4n) {
              const result = core.runUntil(target);
              assert.notEqual(result.reason, "faulted");
              assert.notEqual(result.reason, "power_stopped");
              audio.update(asBytes(core.drainAudio()));
            }
            const state = core.inspect();
            const actual = {
              cycle: String(state.cycle),
              frames: String(state.frames),
              pc: state.nextInstructionAddress,
              cpsr: state.cpsr,
              state: sha(core.saveState()),
              ewram: sha(core.readMemory(0x02000000, 256 * 1024)),
              frame: sha(asBytes(core.frame())),
              audio: audio.digest("hex"),
              save: sha(core.exportSave()),
            };
            const native = JSON.parse(
              execFileSync(
                "../../target/release/session_probe",
                [path, "120"],
                {
                  cwd: new URL(".", import.meta.url),
                  timeout: 120000,
                  encoding: "utf8",
                },
              ),
            );
            assert.deepEqual(actual, native);
          } finally {
            core.dispose();
          }
        });
  },
);

test("resident save restoration matches a fresh boot and preserves state on failure", async () => {
  const a = await WasmCore.create(wasm),
    b = await WasmCore.create(wasm);
  try {
    const bios = new Uint8Array(16384);
    bios.set(loop);
    for (const options of [{}, { bios }]) {
      const save = new Uint8Array(32768).fill(0xa5);
      a.load(loop, options);
      a.runUntil(F);
      a.drainAudio();
      const before = a.saveState();
      assert.throws(
        () => a.resetWithSave(new Uint8Array(1)),
        /InvalidSaveFileSize/,
      );
      assert.deepEqual(a.saveState(), before);
      a.resetWithSave(save);
      b.load(loop, { ...options, save });
      assert.deepEqual(a.saveState(), b.saveState());
      assert.deepEqual(a.exportSave(), save);
      a.runUntil(F);
      b.runUntil(F);
      assert.deepEqual(a.drainAudio(), b.drainAudio());
      assert.deepEqual(a.saveState(), b.saveState());
    }
  } finally {
    a.dispose();
    b.dispose();
  }
});

test("WASM accepts a flash save with an opaque trailer and exports only cartridge bytes", async () => {
  const core = await WasmCore.create(wasm);
  try {
    const rom = new Uint8Array(64);
    rom.set(loop);
    rom.set(Buffer.from("FLASH1M_V103"), 4);
    const save = new Uint8Array(128 * 1024 + 16).fill(0xff);
    save[0x1234] = 0x35;
    save[0x10042] = 0x7a;
    save.fill(0x55, 128 * 1024);
    core.load(rom, { save });
    assert.equal(core.inspect().saveType, "flash128");
    assert.deepEqual(core.exportSave(), save.subarray(0, 128 * 1024));
    core.resetWithSave(save);
    assert.deepEqual(core.exportSave(), save.subarray(0, 128 * 1024));
    assert.throws(
      () => core.resetWithSave(save.subarray(0, 128 * 1024 - 1)),
      /InvalidSaveFileSize/,
    );
    assert.deepEqual(core.exportSave(), save.subarray(0, 128 * 1024));
  } finally {
    core.dispose();
  }
});

test("large uploads, normalized exports and repeated restores stay bounded", async () => {
  const core = await WasmCore.create(wasm);
  try {
    const rom = new Uint8Array(32 * 1024 * 1024);
    rom.set(loop);
    core.load(rom, { saveType: "sram", zipped: false });
    assert.ok(core.wasm.memory.buffer.byteLength <= 96 * 1024 * 1024);
    const exported = core.exportRom();
    const save = new Uint8Array(32768).fill(0x5a);
    for (let i = 0; i < 5; i++) {
      core.resetWithSave(save);
      assert.deepEqual(core.exportSave(), save);
      core.saveState();
      core.exportRom();
    }
    assert.ok(core.wasm.memory.buffer.byteLength <= 96 * 1024 * 1024);
    for (let i = 0; i < 3; i++)
      core.load(rom, { saveType: "sram", zipped: false });
    assert.ok(core.wasm.memory.buffer.byteLength <= 160 * 1024 * 1024);
    rom[0] ^= 1; // Caller data and exported data never alias resident media.
    assert.equal(core.readMemory(0x08000000, 1)[0], loop[0]);
    assert.equal(exported[0], loop[0]);
    const live = core.saveState();
    assert.throws(
      () => core.load(rom, { save: new Uint8Array(1) }),
      /InvalidSaveFileSize/,
    );
    assert.deepEqual(core.saveState(), live);
  } finally {
    core.dispose();
  }
});

test("selective WASM publication stays stable while instant state/frame reads remain exact", async () => {
  const a = await WasmCore.create(wasm),
    b = await WasmCore.create(wasm);
  const words = [
    0xe59f0020, 0xe3a01000, 0xe1c010b0, 0xe59f0018, 0xe59f1018, 0xe1d020b0,
    0xe1c120b0, 0xeafffffc, 0, 0, 0x04000000, 0x04000130, 0x05000000,
  ];
  const rom = new Uint8Array(words.length * 4);
  words.forEach((word, i) =>
    new DataView(rom.buffer).setUint32(i * 4, word, true),
  );
  try {
    a.load(rom);
    b.load(rom);
    b.setVideoInterval(16);
    b.setAudioOutput(false);
    assert.throws(() => b.setVideoInterval(0), /video interval/);
    assert.throws(() => b.setVideoInterval(17), /video interval/);
    for (let frame = 0; frame < 5; frame++) {
      a.setKeys(frame);
      b.setKeys(frame);
      const deadline = a.currentCycle() + F;
      assert.deepEqual(b.runUntil(deadline), a.runUntil(deadline));
    }
    assert.equal(b.canSnapshot(), false);
    const pixels = b.presentationFrame(),
      id = b.videoFrameId(),
      at = b.currentCycle();
    assert.deepEqual(b.saveState(), a.saveState());
    assert.equal(b.currentCycle(), at);
    assert.deepEqual(b.frame(), a.frame());
    assert.deepEqual(b.presentationFrame(), pixels);
    assert.equal(b.videoFrameId(), id);
    const deadline = at + F / 4n;
    assert.deepEqual(b.runUntil(deadline), a.runUntil(deadline));
    assert.equal(b.canSnapshot(), false);
    b.setVideoInterval(1);
    assert.equal(b.canSnapshot(), true);
    assert.deepEqual(b.saveState(), a.saveState());
    const snapshot = b.saveState();
    b.setVideoInterval(8);
    b.loadState(snapshot);
    assert.deepEqual(b.saveState(), snapshot);
    b.resetWithSave(new Uint8Array());
    assert.equal(b.currentCycle(), 0n);
  } finally {
    a.dispose();
    b.dispose();
  }
});
