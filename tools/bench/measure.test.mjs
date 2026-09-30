import assert from "node:assert/strict";
import test from "node:test";
import { parseObservation, roundOrder, summarize } from "./measure.mjs";

const line = (fields = {}) =>
  `noise\nbenchmark: ${JSON.stringify({
    schema: 1,
    emulator: "example",
    frames: 10,
    seconds: 0.5,
    cycle: "2808960",
    boot: "skip-bios",
    video: "full",
    audio: "exact",
    framebufferHash: "0123456789abcdef",
    settings: {},
    ...fields,
  })}\n`;
const plan = { frames: 10, boot: "skip-bios" };

test("accepts a harness line that matches the plan", () => {
  assert.equal(parseObservation(line(), plan).seconds, 0.5);
  assert.equal(
    parseObservation(line({ cycle: "2810191" }), plan).cycle,
    "2810191",
  );
});

test("rejects observations that are not comparable", () => {
  assert.throws(() => parseObservation("no line", plan), /one benchmark line/);
  assert.throws(() => parseObservation(line() + line(), plan), /found 2/);
  assert.throws(
    () => parseObservation(line({ boot: "hle" }), plan),
    /booted hle/,
  );
  assert.throws(
    () => parseObservation(line({ frames: 11 }), plan),
    /11 frames/,
  );
  assert.throws(
    () => parseObservation(line({ audio: "high-speed" }), plan),
    /policy/,
  );
  assert.throws(
    () => parseObservation(line({ seconds: 0 }), plan),
    /loop time/,
  );
  // Stopping early or more than a scanline late is different guest time.
  assert.throws(
    () => parseObservation(line({ cycle: "2808959" }), plan),
    /outside/,
  );
  assert.throws(
    () => parseObservation(line({ cycle: "2810192" }), plan),
    /outside/,
  );
});

test("rotates the emulator order across rounds and workloads", () => {
  const emulators = ["a", "b", "c"];
  assert.deepEqual(roundOrder(emulators, 0, 0), ["a", "b", "c"]);
  assert.deepEqual(roundOrder(emulators, 1, 0), ["b", "c", "a"]);
  assert.deepEqual(roundOrder(emulators, 1, 1), ["c", "a", "b"]);
});

test("summarizes per-round ratios against the baseline and compares images", () => {
  const observation = (
    workload,
    emulator,
    round,
    seconds,
    hash,
    warmup = false,
  ) => ({ workload, emulator, round, seconds, framebufferHash: hash, warmup });
  const observations = [
    observation("w", "a", 0, 9, "aa", true),
    observation("w", "b", 0, 1, "aa", true),
    observation("w", "a", 0, 1, "aa"),
    observation("w", "b", 0, 2, "aa"),
    observation("w", "a", 1, 2, "aa"),
    observation("w", "b", 1, 2, "aa"),
    observation("v", "a", 0, 1, "x"),
    observation("v", "b", 0, 4, "y"),
    observation("v", "a", 1, 1, "x"),
    observation("v", "b", 1, 4, "z"),
  ];
  const summary = summarize({
    observations,
    emulators: ["a", "b"],
    baseline: "a",
    workloads: [
      { name: "w", frames: 60 },
      { name: "v", frames: 60 },
    ],
  });
  const [w, v] = summary.rows;
  assert.equal(w.rounds, 2);
  assert.equal(w.results.a.timeRatio, 1);
  assert.ok(Math.abs(w.results.b.timeRatio - Math.SQRT2) < 1e-12); // ratios 2 and 1
  assert.deepEqual(w.results.b.timeRatioRange, [1, 2]);
  assert.equal(w.results.b.medianSeconds, 2);
  assert.equal(w.results.a.fps, 60 / 1.5);
  assert.deepEqual(w.image, { status: "agree", groups: [["a", "b"]] });
  assert.equal(v.results.b.deterministic, false);
  assert.deepEqual(v.image, { status: "differ", groups: [["a"], ["b"]] });
  assert.ok(
    Math.abs(summary.geomeanTimeRatio.b - Math.sqrt(Math.SQRT2 * 4)) < 1e-12,
  );
});
