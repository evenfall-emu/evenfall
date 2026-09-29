import { wasm, wasmTest as test } from "../scripts/wasm-test.mjs";
import assert from "node:assert/strict";
import { writeFile, mkdtemp, rm } from "node:fs/promises";
import { tmpdir } from "node:os";
import { join } from "node:path";
import { spawnSync } from "node:child_process";
import { WasmCore } from "../core.js";
import { GameplayRecording } from "./journal.js";
import { runScenario, sha256 } from "./scenario.js";
import { replayFailureBundleAsync } from "./capture.js";
const rom = new Uint8Array([42, 0, 160, 227, 254, 255, 255, 234]);
const config = {
  rtc: false,
  rtcEpoch: 0n,
  saveType: "none",
  sampleRate: 48000,
};
test("gameplay journal reproduces machine state and both CLI formats", async () => {
  const core = await WasmCore.create(wasm),
    replay = await WasmCore.create(wasm);
  const dir = await mkdtemp(join(tmpdir(), "zigba-player-"));
  try {
    core.load(rom, config);
    const recording = new GameplayRecording(core, {
      ...config,
      build: { wasmSha256: await sha256(wasm) },
    });
    recording.setKeys(1);
    recording.runUntil(1000n);
    recording.drainAudio();
    recording.setKeys(0);
    recording.runUntil(10000n);
    recording.drainAudio();
    recording.checkpoint("finish");
    const item = await recording.scenario();
    const state = core.saveState();
    const result = await runScenario(replay, item.manifest, item.assets);
    assert.equal(result.status, "passed");
    assert.deepEqual(replay.saveState(), state);
    await writeFile(join(dir, "scenario.json"), JSON.stringify(item.manifest));
    for (const [name, bytes] of Object.entries(item.assets))
      await writeFile(join(dir, item.manifest.assets[name].path), bytes);
    const bundle = recording.recorder.capture();
    await writeFile(join(dir, "failure.json"), JSON.stringify(bundle));
    for (const args of [
      [
        "scripts/run-scenario.mjs",
        join(dir, "scenario.json"),
        join(dir, "result"),
      ],
      [
        "scripts/replay-capture.mjs",
        join(dir, "failure.json"),
        join(dir, "scenario.json"),
      ],
    ]) {
      const child = spawnSync(process.execPath, args, {
        encoding: "utf8",
        timeout: 30000,
      });
      assert.equal(child.status, 0, child.stderr);
    }
  } finally {
    core.dispose();
    replay.dispose();
    await rm(dir, { recursive: true, force: true });
  }
});
test("recording on/off preserves exact input timing, audio and state", async () => {
  const results = [];
  for (const enabled of [false, true]) {
    const core = await WasmCore.create(wasm);
    core.load(rom, config);
    const recorder = new GameplayRecording(core, config);
    recorder.enabled = enabled;
    const audio = [];
    for (let i = 0; i < 100; i++) {
      recorder.setKeys(i % 2);
      recorder.runUntil(core.inspect().cycle + 70224n);
      audio.push(...recorder.drainAudio());
    }
    results.push({ state: core.saveState(), audio });
    core.dispose();
  }
  assert.deepEqual(results[0].state, results[1].state);
  assert.deepEqual(results[0].audio, results[1].audio);
});
test("input cap keeps capture active and exports only complete prefix; new segment starts from state", async () => {
  const core = await WasmCore.create(wasm);
  core.load(rom, config);
  const recording = new GameplayRecording(core, config);
  try {
    for (let i = 0; i < 1026; i++) recording.setKeys(i % 2);
    assert.equal(recording.status().inputs, 1024);
    assert.equal(recording.status().full, true);
    recording.runUntil(1000n);
    const item = await recording.scenario();
    assert.equal(item.manifest.inputs.length, 1024);
    assert.equal(item.manifest.limits.maxCycles, "0");
    assert.equal(recording.recorder.capture().history.replayable, true);
    recording.start();
    assert.ok((await recording.scenario()).assets.state);
    recording.enabled = false;
    recording.runUntil(2000n);
    await assert.rejects(recording.scenario(), /missing history/);
  } finally {
    core.dispose();
  }
});
test("long scenario and replay yield cooperatively and return cancellation distinctly", async () => {
  const core = await WasmCore.create(wasm);
  core.load(rom, config);
  const recording = new GameplayRecording(core, config);
  recording.runUntil(1000n);
  recording.drainAudio();
  const item = await recording.scenario();
  try {
    item.manifest.checkpoints = [{ name: "long", at: "28089600000" }];
    item.manifest.limits.maxCycles = "28089600000";
    const controller = new AbortController();
    let yields = 0;
    const result = await runScenario(core, item.manifest, item.assets, {
      signal: controller.signal,
      yieldControl: async () => {
        if (++yields === 2) controller.abort();
      },
    });
    assert.equal(result.status, "cancelled");
    assert.equal(yields, 2);
    core.load(rom, config);
    recording.start();
    for (let i = 0; i < 20; i++) recording.setKeys(i % 2);
    const bundle = recording.recorder.capture();
    const cancel = new AbortController();
    setTimeout(() => cancel.abort(), 0);
    assert.equal(
      (
        await replayFailureBundleAsync(core, bundle, {
          identity: bundle.identity,
          signal: cancel.signal,
        })
      ).status,
      "cancelled",
    );
  } finally {
    core.dispose();
  }
});

test("turning recording off invalidates stale replay anchors but still captures faults", async () => {
  const core = await WasmCore.create(wasm);
  const captures = [];
  try {
    core.load(new Uint8Array([0, 0, 255, 239]), config);
    const journal = new GameplayRecording(core, {
      ...config,
      onCapture: (bundle) => captures.push(bundle),
    });
    journal.enabled = false;
    journal.runUntil(1000n);
    assert.equal(captures.length, 1);
    assert.equal(captures[0].history.replayable, false);
    assert.equal(captures[0].history.snapshotError.name, "RecordingDisabled");
    assert.ok(captures[0].artifacts.cpu.fault);
  } finally {
    core.dispose();
  }
});

test("boot receipts pin detected RTC and the requested automatic save policy", async () => {
  const core = await WasmCore.create(wasm),
    replay = await WasmCore.create(wasm);
  try {
    const image = new Uint8Array(272);
    image.set(rom);
    image.set(new TextEncoder().encode("SRAM_V110"), 256);
    core.load(image, { rtc: true, rtcEpoch: 0n, saveType: "auto" });
    assert.equal(core.inspect().rtcEnabled, true);
    assert.equal(core.inspect().saveType, "sram");
    const journal = new GameplayRecording(core, { saveType: "auto" });
    journal.runUntil(1000n);
    journal.drainAudio();
    const item = await journal.scenario();
    assert.equal(item.manifest.config.rtc, true);
    assert.equal(item.manifest.config.saveType, "auto");
    const result = await runScenario(replay, item.manifest, item.assets);
    assert.equal(result.status, "passed");
    assert.deepEqual(core.saveState(), replay.saveState());
  } finally {
    core.dispose();
    replay.dispose();
  }
});

test("scheduled inputs crossed by a bounded slice remain exportable and deterministic", async () => {
  const core = await WasmCore.create(wasm),
    replay = await WasmCore.create(wasm);
  try {
    core.load(rom, config);
    const journal = new GameplayRecording(core, config);
    const events = [
      { at: 101n, keys: 1 },
      { at: 104n, keys: 0 },
      { at: 10000n, keys: 4 },
    ];
    const first = journal.runUntil(100n, events, 1000n);
    journal.drainAudio();
    journal.runUntil(1000n, events.slice(first.inputsConsumed));
    journal.drainAudio();
    const item = await journal.scenario();
    assert.deepEqual(item.manifest.inputs, [
      { at: "101", keys: 1 },
      { at: "104", keys: 0 },
    ]);
    assert.equal(
      (await runScenario(replay, item.manifest, item.assets)).status,
      "passed",
    );
    assert.deepEqual(core.saveState(), replay.saveState());
  } finally {
    core.dispose();
    replay.dispose();
  }
});
