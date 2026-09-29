import { wasm, wasmTest as test } from "../scripts/wasm-test.mjs";
import assert from "node:assert/strict";
import { WasmCore, CYCLES_PER_FRAME as F } from "../core.js";
import { FailureRecorder, replayFailureBundle } from "./capture.js";

const loop = new Uint8Array([0xfe, 0xff, 0xff, 0xea]);
const identity = {
  build: "test-build",
  config: { rtcEpoch: "0", sampleRate: 48000, bios: null },
};
async function pair(rom = loop) {
  const a = await WasmCore.create(wasm),
    b = await WasmCore.create(wasm);
  a.load(rom);
  b.load(rom);
  return [a, b];
}

test("manual bundle owns artifacts and replays exact calls and consumed inputs on real WASM", async () => {
  const [a, b] = await pair();
  try {
    const recorder = new FailureRecorder(a, { identity });
    recorder.setKeys(2);
    recorder.runUntil(F / 4n, [
      { at: 13n, keys: 1 },
      { at: F, keys: 0 },
    ]);
    recorder.drainAudio();
    recorder.runUntil(F / 2n);
    const expected = a.saveState();
    const bundle = JSON.parse(
      JSON.stringify(recorder.capture("manual", { note: "inspect this" })),
    );
    assert.equal(bundle.history.replayable, true);
    assert.equal(bundle.operations[1].consumedInputs.length, 1);
    assert.equal(bundle.operations[1].consumedInputs[0].at, "13");
    assert.equal(bundle.identity.romHash, a.inspect().romHash);
    replayFailureBundle(b, bundle, { identity });
    assert.deepEqual(b.saveState(), expected);
    a.runUntil(F);
    assert.deepEqual(bundle.artifacts.cpu.cycle, String(b.inspect().cycle));
    assert.throws(
      () =>
        replayFailureBundle(b, bundle, {
          identity: { ...identity, build: "wrong" },
        }),
      /identity mismatch/,
    );
  } finally {
    a.dispose();
    b.dispose();
  }
});

test("snapshot and operation eviction retain coherent replay or explicitly report loss", async () => {
  const [a, b] = await pair();
  try {
    const recorder = new FailureRecorder(a, {
      identity,
      snapshotIntervalCycles: 10n,
      maxSnapshots: 2,
      maxOperations: 4,
    });
    for (let target = 30n; target <= 300n; target += 30n)
      recorder.runUntil(target);
    recorder.setKeys(8);
    const bundle = recorder.capture();
    assert.equal(bundle.snapshots.length, 2);
    assert.ok(bundle.history.droppedSnapshots > 0);
    assert.ok(
      bundle.operations.every(
        (op) => op.sequence >= bundle.snapshots[0].nextOperation,
      ),
    );
    replayFailureBundle(b, bundle, { identity });
    assert.deepEqual(b.saveState(), a.saveState());
    const limited = new FailureRecorder(a, {
      maxOperations: 1,
      snapshotIntervalCycles: F,
    });
    limited.setKeys(1);
    limited.setKeys(2);
    assert.equal(limited.capture().history.replayable, true);
    assert.equal(limited.capture().history.budgetSnapshots, 1);
    replayFailureBundle(b, limited.capture());
    assert.deepEqual(b.saveState(), a.saveState());
    const tiny = new FailureRecorder(a, { maxSnapshotBytes: 1 });
    assert.match(
      tiny.capture().history.snapshotError.message,
      /maxSnapshotBytes/,
    );
  } finally {
    a.dispose();
    b.dispose();
  }
});

test("fault retains preceding snapshot and automatically emits a replayable bundle", async () => {
  const [a, b] = await pair(new Uint8Array([0, 0, 0xff, 0xef]));
  try {
    let captured;
    const recorder = new FailureRecorder(a, {
      onCapture: (value) => {
        captured = value;
      },
    });
    assert.equal(recorder.runUntil(F).reason, "faulted");
    assert.equal(captured.reason, "fault");
    assert.equal(captured.snapshots.length, 1);
    assert.ok(captured.artifacts.cpu.fault);
    assert.equal(
      replayFailureBundle(b, captured).inspection.fault,
      a.inspect().fault,
    );
  } finally {
    a.dispose();
    b.dispose();
  }
});

test("exceptions and assertions capture without replacing original exception", async () => {
  const [a, b] = await pair();
  try {
    const recorder = new FailureRecorder(a, {
      onCapture: () => {
        throw new Error("sink unavailable");
      },
    });
    assert.throws(() => recorder.setKeys(1024), /Invalid key mask/);
    assert.equal(recorder.lastCapture.reason, "exception");
    assert.equal(
      recorder.lastCapture.captureCallbackError.message,
      "sink unavailable",
    );
    replayFailureBundle(b, recorder.lastCapture);
    assert.deepEqual(a.saveState(), b.saveState());
    assert.throws(
      () => recorder.assert(false, "pixel mismatch", { x: 3 }),
      /pixel mismatch/,
    );
    assert.equal(recorder.lastCapture.reason, "assertion");
    assert.equal(recorder.lastCapture.details.x, 3);
    // Replay reproduces rejected argument types rather than normalizing them.
    assert.throws(() => recorder.runUntil(10), /bigint/);
    replayFailureBundle(b, recorder.lastCapture);
    assert.throws(
      () => recorder.runUntil(10n, [{ at: "1", keys: 1 }]),
      /bigint/,
    );
    replayFailureBundle(b, recorder.lastCapture);
  } finally {
    a.dispose();
    b.dispose();
  }
});

test("byte budget and unserializable calls invalidate history without hiding the core error", async () => {
  const [a, b] = await pair();
  try {
    const recorder = new FailureRecorder(a, { maxHistoryBytes: 1024 });
    const huge = Array.from({ length: 1025 }, () => ({ at: 0n, keys: 0 }));
    assert.throws(() => recorder.runUntil(10n, huge), /1024/);
    const bundle = recorder.lastCapture;
    assert.equal(bundle.history.replayable, false);
    assert.equal(bundle.history.snapshotError.name, "RecordingGap");
    assert.ok(JSON.stringify(bundle.operations).length * 2 <= 1024);
    assert.throws(
      () => replayFailureBundle(b, bundle),
      /No replayable snapshot/,
    );
    assert.deepEqual(
      recorder.capture("manual", { data: "x".repeat(1025) }).details,
      { truncated: true },
    );
  } finally {
    a.dispose();
    b.dispose();
  }
});

test("replay rejects final-state divergence even at identical cycle", async () => {
  const [a, b] = await pair();
  try {
    const recorder = new FailureRecorder(a);
    recorder.setKeys(1);
    const bundle = recorder.capture();
    bundle.operations[0].args[0] = 2;
    assert.throws(
      () => replayFailureBundle(b, bundle),
      /machine state diverged/,
    );
  } finally {
    a.dispose();
    b.dispose();
  }
});

test("budget-triggered snapshots preserve a replay anchor through an imminent guest fault", async () => {
  for (const limits of [{ maxOperations: 1 }, { maxHistoryBytes: 1400 }]) {
    const [a, b] = await pair(new Uint8Array([0, 0, 0xff, 0xef]));
    try {
      const recorder = new FailureRecorder(a, {
        ...limits,
        snapshotIntervalCycles: F * 60n,
      });
      for (let i = 0; i < 20; i++) recorder.setKeys(i);
      assert.equal(recorder.runUntil(F).reason, "faulted");
      const bundle = recorder.lastCapture;
      assert.equal(bundle.history.replayable, true);
      assert.ok(bundle.history.budgetSnapshots > 0);
      assert.ok(bundle.operations.length <= (limits.maxOperations ?? 4096));
      assert.equal(
        replayFailureBundle(b, bundle).inspection.fault,
        a.inspect().fault,
      );
    } finally {
      a.dispose();
      b.dispose();
    }
  }
});

test("deferred records own mutable inputs and results and keep the version 2 export shape", async () => {
  const [a, b] = await pair();
  try {
    const recorder = new FailureRecorder(a);
    const event = { at: 13n, keys: 1 };
    const events = [event, { at: F, keys: 0 }];
    const result = recorder.runUntil(F / 4n, events);
    const recordedCycle = String(result.cycle);
    event.at = 10000n;
    event.keys = 0;
    events.length = 0;
    result.cycle = 0n;
    result.inputsConsumed = 999;
    recorder.drainAudio(new Int16Array(16384));
    const view = recorder.operations;
    view[0].args[1][0].keys = 8;
    const bundle = recorder.capture();
    assert.equal(bundle.version, 2);
    assert.deepEqual(bundle.operations[0].args, [
      String(F / 4n),
      [
        { at: "13", keys: 1 },
        { at: String(F), keys: 0 },
      ],
      String(F / 4n),
    ]);
    assert.equal(bundle.operations[0].result.cycle, recordedCycle);
    assert.equal(bundle.operations[0].result.inputsConsumed, 1);
    assert.equal(bundle.operations[1].args.length, 0);
    for (const operation of bundle.operations)
      assert.ok(JSON.stringify(operation).length * 2 <= operation.recordBytes);
    replayFailureBundle(b, JSON.parse(JSON.stringify(bundle)));
    assert.deepEqual(a.saveState(), b.saveState());
  } finally {
    a.dispose();
    b.dispose();
  }
});

test("rejected audio destinations replay their exception without consuming samples", async () => {
  const [a, b] = await pair();
  try {
    const recorder = new FailureRecorder(a);
    recorder.runUntil(F / 4n);
    for (const output of [new Int16Array(1), new Uint16Array(16384)]) {
      assert.throws(() => recorder.drainAudio(output), /Invalid audio buffer/);
      replayFailureBundle(b, recorder.lastCapture);
      assert.deepEqual(a.saveState(), b.saveState());
    }
    assert.deepEqual(recorder.drainAudio(), b.drainAudio());
  } finally {
    a.dispose();
    b.dispose();
  }
});

test("ring wrap preserves order and budget rebasing; failed snapshots expose a replay gap", async () => {
  const [a, b] = await pair();
  try {
    const recorder = new FailureRecorder(a, { maxOperations: 4 });
    for (let i = 0; i < 101; i++) recorder.setKeys(i);
    assert.equal(recorder.operationCount, 1);
    assert.equal(recorder.operations[0].sequence, 100);
    assert.equal(recorder.capture().history.budgetSnapshots, 25);
    replayFailureBundle(b, recorder.capture());
    assert.deepEqual(a.saveState(), b.saveState());
    const saveState = a.saveState;
    a.saveState = () => {
      throw new Error("snapshot unavailable");
    };
    try {
      for (let i = 0; i < 12; i++) recorder.setKeys(i);
      const bundle = recorder.capture();
      assert.equal(bundle.history.replayable, false);
      assert.deepEqual(
        bundle.operations.map((op) => op.sequence),
        [109, 110, 111, 112],
      );
      assert.equal(recorder.operationCount, 4);
      assert.ok(recorder.historyBytes <= recorder.maxHistoryBytes);
    } finally {
      a.saveState = saveState;
    }
  } finally {
    a.dispose();
    b.dispose();
  }
});

test("periodic snapshots defer omitted pixels but budget/manual recovery stays exact without policy replay", async () => {
  const [a, b] = await pair();
  try {
    const recorder = new FailureRecorder(a, {
      snapshotIntervalCycles: F,
      maxOperations: 4,
    });
    a.setVideoInterval(16);
    a.setAudioOutput(false);
    for (let frame = 0; frame < 3; frame++)
      recorder.runUntil(a.currentCycle() + F);
    assert.equal(a.canSnapshot(), false);
    assert.ok(BigInt(recorder.snapshots.at(-1).cycle) < a.currentCycle());
    assert.equal(recorder.snapshotError, null);
    for (let i = 0; i < 5; i++) recorder.setKeys(i);
    assert.ok(recorder.budgetSnapshots > 0);
    recorder.runUntil(a.currentCycle() + F);
    const at = a.currentCycle();
    assert.equal(a.canSnapshot(), false);
    const bundle = recorder.capture("manual");
    assert.equal(a.currentCycle(), at);
    assert.equal(bundle.history.replayable, true);
    assert.deepEqual(bundle.artifactErrors, {});
    replayFailureBundle(b, bundle);
    assert.deepEqual(b.saveState(), a.saveState());
  } finally {
    a.dispose();
    b.dispose();
  }
});
