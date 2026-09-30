import { wasm, wasmTest as test } from "./scripts/wasm-test.mjs";
import assert from "node:assert/strict";
import { Worker } from "node:worker_threads";
import { Emulator, CYCLES_PER_FRAME as F } from "./zigba.js";

// Only the browser transport is adapted. Exercise the real SDK, worker,
// recorder and WASM, including structured clones and transferred output.
class BrowserWorker {
  constructor(url) {
    const bootstrap = `
      import { parentPort } from 'node:worker_threads';
      globalThis.self = globalThis;
      globalThis.postMessage = (data, transfer) => parentPort.postMessage(data, transfer);
      await import(${JSON.stringify(String(url))});
      parentPort.on('message', data => self.onmessage({data}));
    `;
    this.worker = new Worker(
      new URL(`data:text/javascript,${encodeURIComponent(bootstrap)}`),
    );
    this.worker.on("message", (data) => this.onmessage?.({ data }));
    this.worker.on("error", (error) => this.onerror?.(error));
  }
  postMessage(data, transfer) {
    this.worker.postMessage(data, transfer);
  }
  terminate() {
    return this.worker.terminate();
  }
}

test("SDK bundle import requires pause, preserves failures and resets presentation and recording on success", async () => {
  const previousWorker = globalThis.Worker;
  globalThis.Worker = BrowserWorker;
  let emulator;
  try {
    emulator = await Emulator.create({
      wasmUrl: `data:application/wasm;base64,${wasm.toString("base64")}`,
    });
    const events = [];
    emulator.addEventListener("clear", (event) => events.push(event.detail));
    emulator.addEventListener("frame", (event) => events.push(event.detail));
    await emulator.load(new Uint8Array([0xfe, 0xff, 0xff, 0xea]));
    await emulator.setRewindConfig(undefined, 2);
    await emulator.runUntil(7n * F, [{ at: 6n * F, keys: 1 }]);
    const live = await emulator.saveState();
    const timeline = await emulator.rewindCheckpoints();
    const bundle = await emulator.saveRewindBundle();
    assert.deepEqual(await emulator.saveState(), live);
    assert.deepEqual(
      (await emulator.rewindCheckpoints()).checkpoints.map((p) => p.frame),
      [2n, 4n, 6n],
    );

    await emulator.play();
    await assert.rejects(
      emulator.loadRewindBundle(bundle),
      /PauseBeforeLoadingRewindBundle/,
    );
    await emulator.pause();
    await emulator.rewind(2);
    await emulator.setKeys(2);
    await emulator.runUntil((await emulator.inspect()).cycle + F);
    const branch = await emulator.saveRewindBundle();
    const oldObservation = await emulator.captureMemory();
    const corrupt = bundle.slice();
    corrupt[corrupt.length - 1] ^= 1;
    events.length = 0;
    await assert.rejects(
      emulator.loadRewindBundle(corrupt),
      /InvalidRewindBundle/,
    );
    assert.equal(events.length, 0);
    assert.deepEqual(await emulator.saveRewindBundle(), branch);
    assert.equal(
      (await emulator.captureMemory()).timeline,
      oldObservation.timeline,
    );

    const restored = await emulator.loadRewindBundle(bundle);
    assert.equal(restored.frames, 7n);
    assert.ok(bundle.byteLength > 0); // Import does not detach caller storage.
    assert.deepEqual(await emulator.saveState(), live);
    assert.deepEqual(
      events.map((event) => event.event),
      ["clear", "frame"],
    );
    assert.equal(events[1].audio.length, 0);
    assert.equal(events[1].state.cycle, restored.cycle);
    const observation = await emulator.captureMemory();
    assert.notEqual(observation.timeline, oldObservation.timeline);
    assert.notEqual(observation.timeline, timeline.timeline);
    assert.equal(observation.sequence, 1);
    assert.equal(events[0].timeline, observation.timeline);
    assert.equal(await emulator.rewindDepth(), 3);
    assert.equal((await emulator.recording()).inputs, 0);
    const scenario = await emulator.exportScenario("Restored branch");
    assert.deepEqual(scenario.assets.state, live); // Never replay from cold boot.
    assert.deepEqual(scenario.manifest.inputs, []);
    assert.equal((await emulator.rewind(3)).frames, 2n);
  } finally {
    await emulator?.dispose();
    globalThis.Worker = previousWorker;
  }
});

test("SDK cheats are recorded into exported scenarios, kept by reset and restored by rewind", async () => {
  const previousWorker = globalThis.Worker;
  globalThis.Worker = BrowserWorker;
  let emulator;
  try {
    emulator = await Emulator.create({
      wasmUrl: `data:application/wasm;base64,${wasm.toString("base64")}`,
    });
    assert.deepEqual(await emulator.parseCheat("82025BC4 E0FF"), {
      format: "codebreaker",
      code: "82025BC4 E0FF",
    });
    await emulator.load(new Uint8Array([0xfe, 0xff, 0xff, 0xea]));
    await emulator.setRewindConfig(undefined, 1);
    await emulator.runUntil(2n * F);
    const money = [{ format: "raw", code: "02025E90:000F423F" }];
    const at = String((await emulator.inspect()).cycle);
    assert.deepEqual(
      await emulator.setCheats([{ code: "02025e90:000f423f" }]),
      money,
    );
    await emulator.runUntil(4n * F);
    const read = async () =>
      new DataView((await emulator.readMemory(0x02025e90, 4)).buffer).getUint32(
        0,
        true,
      );
    assert.equal(await read(), 999999);
    const { manifest } = await emulator.exportScenario("cheats");
    assert.deepEqual(manifest.cheats, [{ at, cheats: money }]);
    await emulator.rewind(3);
    assert.deepEqual(await emulator.cheats(), []);
    await emulator.setCheats(money);
    await emulator.reset();
    assert.deepEqual(await emulator.cheats(), money);
    const fresh = await emulator.exportScenario("after reset");
    assert.deepEqual(fresh.manifest.cheats, [{ at: "0", cheats: money }]);
    await assert.rejects(
      emulator.setCheats([{ code: "nonsense" }]),
      /InvalidCheat/,
    );
  } finally {
    await emulator?.dispose();
    globalThis.Worker = previousWorker;
  }
});
