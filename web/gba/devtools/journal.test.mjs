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
  const dir = await mkdtemp(join(tmpdir(), "evenfall-player-"));
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
test("recorded cheat changes reproduce through scenarios, captures and states", async () => {
  const core = await WasmCore.create(wasm),
    replay = await WasmCore.create(wasm),
    other = await WasmCore.create(wasm);
  const F = 280896n;
  const word = (target, address) =>
    new DataView(target.readMemory(address, 4).buffer).getUint32(0, true);
  try {
    core.load(rom, config);
    const recording = new GameplayRecording(core, {
      ...config,
      build: { wasmSha256: await sha256(wasm) },
    });
    const advance = (frames) => {
      const end = core.currentCycle() + frames * F;
      while (core.currentCycle() < end) {
        const next = core.currentCycle() + F / 4n;
        recording.runUntil(next < end ? next : end);
        recording.drainAudio();
      }
    };
    advance(1n);
    // Pokémon Emerald (USA) "Infinite Money", published encrypted: it
    // decrypts to 04225E90 000F423F, 999999 at 02025E90.
    assert.deepEqual(recording.setCheats([{ code: "C051CCF6 975E8DA1" }]), [
      { format: "action-replay", code: "C051CCF6 975E8DA1" },
    ]);
    advance(2n);
    assert.equal(word(core, 0x02025e90), 999999);
    // Set a counting code just before a VBlank start: a replay that
    // applied the change later would miss one addition.
    let vblank = (core.currentCycle() / F) * F + 197120n;
    if (vblank <= core.currentCycle() + 40n) vblank += F;
    while (core.currentCycle() < vblank - 40n) {
      const next = core.currentCycle() + F / 4n;
      recording.runUntil(next < vblank - 40n ? next : vblank - 40n);
      recording.drainAudio();
    }
    assert.ok(core.currentCycle() < vblank);
    recording.setCheats([
      { format: "action-replay-decrypted", code: "84225E90 00000001" },
    ]);
    advance(2n);
    // Clear it just after the next VBlank start, far from the one after.
    recording.runUntil(core.currentCycle() + 1000n);
    assert.equal(word(core, 0x02025e90), 999999 + 3);
    recording.setCheats([]);
    advance(1n);
    recording.checkpoint("end");
    const item = await recording.scenario();
    assert.equal(item.manifest.cheats.length, 3);
    const result = await runScenario(replay, item.manifest, item.assets);
    assert.equal(result.status, "passed", result.failures.join("; "));
    assert.deepEqual(replay.saveState(), core.saveState());

    const bundle = recording.recorder.capture();
    other.load(rom, config);
    const replayed = await replayFailureBundleAsync(other, bundle, {
      identity: bundle.identity,
    });
    assert.equal(replayed.status, "passed");

    // States carry the active cheats; a plain state clears them.
    const plain = core.saveState();
    core.setCheats([{ code: "02025E90:00000007" }]);
    const withCheats = core.saveState();
    assert.equal(
      new TextDecoder().decode(withCheats.subarray(0, 8)),
      "EVGBCHT1",
    );
    replay.loadState(withCheats);
    assert.deepEqual(replay.cheats(), [
      { format: "raw", code: "02025E90:00000007" },
    ]);
    replay.loadState(plain);
    assert.deepEqual(replay.cheats(), []);
    assert.throws(
      () =>
        core.setCheats([
          {
            code: "00000000 1802603C 00002100 00000000",
            format: "action-replay-decrypted",
          },
        ]),
      /InvalidCheat: cheat 1: line 1: ROM patch codes are not supported/,
    );
    assert.deepEqual(core.parseCheat("9E6AC862 823AB7A8 46B7D9E4 A709E9E1"), {
      format: "gameshark",
      code: "9E6AC862 823AB7A8\n46B7D9E4 A709E9E1",
    });
  } finally {
    core.dispose();
    replay.dispose();
    other.dispose();
  }
});
