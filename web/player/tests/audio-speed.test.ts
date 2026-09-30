import { readFileSync } from "node:fs";
import vm from "node:vm";
import { describe, expect, it } from "vite-plus/test";

interface Processor {
  port: { onmessage: (event: { data: object }) => void };
  process(inputs: unknown[], outputs: Float32Array[][]): boolean;
  queued: number;
}
function processor(speed: number, outputSampleRate = 80) {
  let Processor!: new (options: object) => Processor;
  vm.runInNewContext(
    readFileSync(
      new URL("../../web/audio-worklet.js", import.meta.url),
      "utf8",
    ),
    {
      AudioWorkletProcessor: class {
        port = { postMessage() {} };
      },
      sampleRate: outputSampleRate,
      Int16Array,
      registerProcessor: (_: string, constructor: typeof Processor) => {
        Processor = constructor;
      },
    },
  );
  const audio = new Processor({});
  audio.port.onmessage({ data: { speed } });
  audio.port.onmessage({
    data: {
      samples: Int16Array.from(
        { length: 128 },
        (_, i) => Math.floor(i / 2) * (i % 2 ? -8 : 8),
      ),
    },
  });
  return audio;
}
describe("audio worklet playback speed", () => {
  for (const speed of [0.5, 1, 1.5, 4, 8]) {
    it(`consumes stereo audio at ${speed}× with fractional interpolation`, () => {
      const audio = processor(speed);
      const left = new Float32Array(4),
        right = new Float32Array(4);
      audio.process([], [[left, right]]);
      expect([...left]).toEqual([0, 1, 2, 3].map((i) => (i * speed * 8) / 512));
      expect([...right]).toEqual(
        [0, 1, 2, 3].map((i) => (i === 0 ? 0 : (-i * speed * 8) / 512)),
      );
      expect(audio.queued).toBe(64 - 4 * speed);
      audio.port.onmessage({ data: { clear: true } });
      expect(audio.queued).toBe(0);
    });
  }
});

it("returns transferred input only after copying PCM into the worklet ring", () => {
  const audio = processor(1);
  const returned: unknown[] = [];
  (
    audio.port as unknown as { postMessage: (value: unknown) => void }
  ).postMessage = (value) => returned.push(value);
  const samples = new Int16Array([123, -123]);
  audio.port.onmessage({ data: { samples, recycle: true } });
  expect(returned).toEqual([{ recycled: samples.buffer }]);
  samples.fill(0);
  expect(audio.queued).toBe(65);
  const left = new Float32Array(65),
    right = new Float32Array(65);
  audio.process([], [[left, right]]);
  expect(left[64]).toBe(123 / 512);
  expect(right[64]).toBe(-123 / 512);
});

it("can prime and play 8x audio at the real device sample rate", () => {
  for (const sampleRate of [44100, 48000, 96000]) {
    const audio = processor(8, sampleRate);
    const priming = Math.ceil(sampleRate * 0.05 * 8);
    audio.port.onmessage({
      data: { samples: new Int16Array((priming - 64) * 2) },
    });
    expect(audio.queued).toBe(priming);
    audio.process([], [[new Float32Array(128), new Float32Array(128)]]);
    expect(audio.queued).toBe(priming - 128 * 8);
  }
});
