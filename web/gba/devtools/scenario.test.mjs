import { test } from "node:test";
import assert from "node:assert/strict";
import { WasmCore } from "../core.js";
import { runScenario, sha1, sha256, validateScenario } from "./scenario.js";
import { wasm, wasmTest } from "../scripts/wasm-test.mjs";
const rom = new Uint8Array([42, 0, 160, 227, 254, 255, 255, 234]);
async function manifest(image = rom) {
  return {
    version: 2,
    name: "fixture",
    assets: { rom: { sha1: await sha1(image) } },
    config: {
      boot: "hle",
      saveType: "none",
      rtc: false,
      rtcEpoch: "0",
      sampleRate: 48000,
    },
    limits: {
      maxCycles: "280896",
      maxRuns: 100,
      chunkCycles: "16384",
      maxAudioBytes: 1048576,
    },
    inputs: [
      { at: "100", keys: 1 },
      { at: "200", keys: 0 },
    ],
    checkpoints: [
      {
        name: "done",
        at: "280896",
        expect: { cpu: { registers: { 0: 42 } } },
        memory: [
          {
            name: "ROM",
            address: 0x08000000,
            length: 8,
            sha256: await sha256(rom),
          },
        ],
      },
    ],
  };
}
test("strict manifest rejects ambiguous timing, unknown fields, unbounded work and inconsistent BIOS", async () => {
  const m = await manifest();
  validateScenario(m);
  for (const mutate of [
    (m) => (m.inputs[0].at = 100),
    (m) => (m.config.clock = "wall"),
    (m) => (m.config.boot = "bios"),
    (m) => (m.limits.chunkCycles = "70225"),
    (m) => (m.limits.maxRuns = 0),
    (m) => (m.checkpoints[0].at = "18446744073709551616"),
    (m) => m.inputs.reverse(),
    (m) => (m.checkpoints[0].memory[0].address = 0xffffffff),
  ]) {
    const copy = structuredClone(m);
    mutate(copy);
    assert.throws(() => validateScenario(copy));
  }
});
wasmTest(
  "real WASM scenario is repeatable, checks independent CPU/ROM oracle and keeps owned artifacts",
  async () => {
    const core = await WasmCore.create(wasm);
    try {
      const m = await manifest();
      const a = await runScenario(core, m, { rom });
      assert.equal(a.status, "passed", JSON.stringify(a.failures));
      assert.equal(a.inputsConsumed, 2);
      const b = await runScenario(core, m, { rom });
      assert.deepEqual(a.checkpoints, b.checkpoints);
      assert.ok(a.checkpoints[0].audioBytes > 0);
      m.checkpoints[0].expect = { ...a.checkpoints[0] };
      for (const k of [
        "name",
        "requestedCycle",
        "audioBytes",
        "memory",
        "fault",
      ])
        delete m.checkpoints[0].expect[k];
      // CPU expectations use sparse register object, not a typed-array serialization.
      m.checkpoints[0].expect.cpu = { registers: { 0: 42 } };
      assert.equal((await runScenario(core, m, { rom })).status, "passed");
      m.checkpoints[0].expect.cpu.registers[0] = 43;
      const artifacts = [];
      const bad = await runScenario(
        core,
        m,
        { rom },
        { onArtifact: (name, data) => artifacts.push({ name, data }) },
      );
      assert.equal(bad.status, "failed");
      assert.ok(bad.capture);
      assert.equal(artifacts.length, 4);
      const first = artifacts[0].data.slice();
      core.load(rom);
      assert.deepEqual(artifacts[0].data, first);
      await assert.rejects(
        runScenario(core, m, { rom: new Uint8Array([1]) }),
        /SHA-1 mismatch/,
      );
    } finally {
      core.dispose();
    }
  },
);
wasmTest(
  "fault, STOP, run budget and audio budget report failures without hanging",
  async () => {
    const core = await WasmCore.create(wasm);
    try {
      for (const [image, reason] of [
        [new Uint8Array([0, 0, 3, 239]), "power_stopped"],
        [new Uint8Array([0, 0, 255, 239]), "faulted"],
      ]) {
        const m = await manifest(image);
        m.inputs = [];
        m.checkpoints[0].expect = {};
        m.checkpoints[0].memory = [];
        const bad = await runScenario(core, m, { rom: image });
        assert.equal(bad.status, "failed");
        assert.ok(bad.capture);
        m.checkpoints[0].expect.reason = reason;
        assert.equal(
          (await runScenario(core, m, { rom: image })).status,
          "passed",
        );
      }
      const m = await manifest();
      m.limits.maxRuns = 1;
      assert.match(
        (await runScenario(core, m, { rom })).failures[0],
        /run budget/,
      );
      m.limits.maxRuns = 100;
      m.limits.maxAudioBytes = 0;
      assert.match(
        (await runScenario(core, m, { rom })).failures[0],
        /audio budget/,
      );
    } finally {
      core.dispose();
    }
  },
);
wasmTest(
  "inputs crossed at checkpoint boundaries remain deterministic across execution chunk sizes",
  async () => {
    const core = await WasmCore.create(wasm);
    try {
      const m = await manifest();
      m.inputs = [
        { at: "101", keys: 7 },
        { at: "104", keys: 0 },
      ];
      m.checkpoints = [
        { name: "edge", at: "100" },
        { name: "end", at: "1000" },
      ];
      const a = await runScenario(core, m, { rom }, { capture: false });
      assert.equal(a.status, "passed", JSON.stringify(a.failures));
      assert.equal(a.inputsConsumed, 2);
      m.limits.chunkCycles = "1";
      m.limits.maxRuns = 10000;
      const b = await runScenario(core, m, { rom }, { capture: false });
      assert.equal(b.status, "passed", JSON.stringify(b.failures));
      assert.deepEqual(a.checkpoints, b.checkpoints);
      const before = core.saveState();
      await assert.rejects(
        runScenario(core, m, { rom: new Uint8Array([1]) }),
        /SHA-1 mismatch/,
      );
      assert.deepEqual(core.saveState(), before);
      const undeclared = await manifest();
      await assert.rejects(
        runScenario(core, undeclared, { rom, state: new Uint8Array() }),
        /Undeclared state/,
      );
    } finally {
      core.dispose();
    }
  },
);
wasmTest(
  "pinned initial state restores a deterministic checkpoint and rejects past inputs",
  async () => {
    const core = await WasmCore.create(wasm);
    try {
      core.load(rom, {
        saveType: "none",
        rtc: false,
        rtcEpoch: 0n,
        sampleRate: 48000,
      });
      core.runUntil(1000n);
      core.drainAudio();
      const state = core.saveState();
      const m = await manifest();
      m.assets.state = { sha1: await sha1(state) };
      m.inputs = [];
      const a = await runScenario(core, m, { rom, state }, { capture: false });
      const b = await runScenario(core, m, { rom, state }, { capture: false });
      assert.equal(a.status, "passed");
      assert.deepEqual(a.checkpoints, b.checkpoints);
      m.inputs = [{ at: "0", keys: 1 }];
      assert.match(
        (await runScenario(core, m, { rom, state })).failures[0],
        /predates/,
      );
    } finally {
      core.dispose();
    }
  },
);
wasmTest(
  "cycle zero and same-cycle inputs preserve order through closely spaced checkpoints",
  async () => {
    const core = await WasmCore.create(wasm);
    try {
      const m = await manifest();
      m.inputs = [
        { at: "0", keys: 1 },
        { at: "0", keys: 7 },
        { at: "1", keys: 3 },
        { at: "2", keys: 9 },
      ];
      m.checkpoints = [
        {
          name: "zero",
          at: "0",
          memory: [{ name: "keypad", address: 0x04000130, length: 2 }],
        },
        { name: "one", at: "1" },
        { name: "two", at: "2" },
        { name: "done", at: "100" },
      ];
      const a = await runScenario(core, m, { rom }, { capture: false });
      assert.equal(a.status, "passed", JSON.stringify(a.failures));
      assert.equal(a.inputsConsumed, 4);
      assert.equal(a.checkpoints[0].cycle, "0");
      // KEYINPUT is active low; the second event at cycle zero wins.
      assert.equal(
        a.checkpoints[0].memory[0].sha256,
        await sha256(new Uint8Array([248, 3])),
      );
      assert.equal(
        new DataView(core.readMemory(0x04000130, 2).buffer).getUint16(0, true) &
          1023,
        1023 ^ 9,
      );
      assert.ok(BigInt(a.checkpoints[1].cycle) >= 1n);
      assert.ok(BigInt(a.checkpoints[2].cycle) >= 2n);
    } finally {
      core.dispose();
    }
  },
);
