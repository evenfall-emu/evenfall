import { test } from "node:test";
import assert from "node:assert/strict";
import { PresentationPool } from "./presentation.js";
import { WasmCore } from "./core.js";
import { GameplayRecording } from "./devtools/journal.js";
import { replayFailureBundle } from "./devtools/capture.js";
import { wasm, wasmTest } from "./scripts/wasm-test.mjs";

test("presentation buffers are bounded and reused only after explicit transfer back", () => {
  const pool = new PresentationPool();
  for (let i = 0; i < 100; i++) {
    const pixels = pool.takePixels(),
      audio = pool.takeAudio();
    assert.equal(pool.allocations, 2);
    pixels[0] = i;
    audio[0] = -i;
    const received = structuredClone(
      { pixels, audio },
      { transfer: [pixels.buffer, audio.buffer] },
    );
    assert.equal(pixels.byteLength, 0);
    assert.equal(received.pixels[0], i);
    pool.release(
      structuredClone(
        { pixels: received.pixels.buffer, audio: received.audio.buffer },
        { transfer: [received.pixels.buffer, received.audio.buffer] },
      ),
    );
  }
  for (let i = 0; i < 10; i++)
    pool.release({
      pixels: new ArrayBuffer(76800),
      audio: new ArrayBuffer(131072),
    });
  pool.release({ pixels: new ArrayBuffer(10), audio: new ArrayBuffer(10) });
  assert.equal(pool.pixels.length, 4);
  assert.equal(pool.audio.length, 4);
});

wasmTest(
  "pooled audio keeps exact PCM and replay operations independent of output storage",
  async () => {
    const a = await WasmCore.create(wasm),
      b = await WasmCore.create(wasm);
    const config = {
      saveType: "none",
      rtc: false,
      rtcEpoch: 0n,
      sampleRate: 48000,
    };
    // Self-contained ARM program: enable channel 2, set stereo gain, trigger
    // a square wave, then loop. No generated/external probe ROM is required.
    const words = [],
      literals = [];
    const load = (register, value) => {
      literals.push({ at: words.length, register, value });
      words.push(0);
    };
    const store = (offset) =>
      words.push(0xe1c010b0 | ((offset & 0xf0) << 4) | (offset & 15));
    load(0, 0x04000000);
    words.push(0xe3a01080);
    store(0x84);
    load(1, 0x2277);
    store(0x80);
    words.push(0xe3a01002);
    store(0x82);
    load(1, 0xf080);
    store(0x68);
    load(1, 0x8700);
    store(0x6c);
    words.push(0xeafffffe);
    for (const { at, register, value } of literals) {
      words[at] =
        0xe59f0000 | (register << 12) | (words.length * 4 - at * 4 - 8);
      words.push(value);
    }
    const rom = new Uint8Array(words.length * 4);
    words.forEach((word, i) =>
      new DataView(rom.buffer).setUint32(i * 4, word, true),
    );
    try {
      a.load(rom, config);
      b.load(rom, config);
      const recording = new GameplayRecording(a, config);
      const pool = new PresentationPool();
      let audible = false;
      for (let i = 1n; i <= 8n; i++) {
        recording.runUntil(i * 70224n);
        b.runUntil(i * 70224n);
        const audio = pool.takeAudio();
        const actual = recording.drainAudio(audio);
        audible ||= actual.some((sample) => sample !== 0);
        assert.deepEqual(actual, b.drainAudio());
        pool.release({ audio: audio.buffer });
      }
      assert.ok(audible, "Fixture must exercise nonzero PCM");
      const capture = recording.recorder.capture("manual");
      assert.ok(
        capture.operations
          .filter((op) => op.method === "drainAudio")
          .every((op) => op.args.length === 0),
      );
      const expected = a.saveState();
      replayFailureBundle(b, capture, { identity: capture.identity });
      assert.deepEqual(b.saveState(), expected);
      assert.deepEqual(a.frame(pool.takePixels()), b.frame());

      // Fast-forward suppresses only host output. Exact portable states and
      // replayed failure evidence must survive an automatic snapshot rollover.
      a.setAudioOutput(false);
      for (let i = 0; i < 125; i++) {
        const deadline = a.currentCycle() + 280896n;
        recording.runUntil(deadline);
        b.runUntil(deadline);
        assert.equal(a.drainAudio().length, 0);
        assert.ok(b.drainAudio().length > 0);
        if (i % 16 === 0) {
          assert.deepEqual(a.saveState(), b.saveState());
          assert.deepEqual(a.frame(), b.frame());
        }
      }
      const silentCapture = recording.recorder.capture("manual");
      assert.ok(silentCapture.snapshots.length > 1);
      replayFailureBundle(b, silentCapture, {
        identity: silentCapture.identity,
      });
      assert.deepEqual(a.saveState(), b.saveState());
      a.setAudioOutput(true);
      b.setAudioOutput(false);
      b.setAudioOutput(true);
      for (let i = 0; i < 8; i++) {
        const deadline = a.currentCycle() + 70224n;
        a.runUntil(deadline);
        b.runUntil(deadline);
        assert.deepEqual(a.drainAudio(), b.drainAudio());
      }
    } finally {
      a.dispose();
      b.dispose();
    }
  },
);
