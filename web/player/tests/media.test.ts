import { Buffer } from "node:buffer";
import { describe, it, expect, vi } from "vite-plus/test";
import { AudioOutput, VideoOutput } from "../src/media";
function output(audio = new AudioOutput()) {
  const buffers: Float32Array[][] = [];
  const sources: {
    stop: ReturnType<typeof vi.fn>;
    disconnect: ReturnType<typeof vi.fn>;
    playbackRate: { value: number };
    start: ReturnType<typeof vi.fn>;
    connect: ReturnType<typeof vi.fn>;
  }[] = [];
  const gain = {
    gain: { value: 1 },
    connect: vi.fn(),
    disconnect: vi.fn(),
  };
  const context = {
    state: "running",
    sampleRate: 48000,
    currentTime: 0,
    createGain: vi.fn(() => gain),
    createBuffer(_channels: number, length: number) {
      const data = [new Float32Array(length), new Float32Array(length)];
      buffers.push(data);
      return {
        getChannelData: (channel: number) => data[channel],
        duration: length / 48000,
      };
    },
    createBufferSource() {
      const source = {
        buffer: null,
        playbackRate: { value: 1 },
        connect: vi.fn(),
        disconnect: vi.fn(),
        start: vi.fn(),
        stop: vi.fn(),
        onended: null,
      };
      sources.push(source);
      return source;
    },
    destination: {},
    resume: vi.fn(async () => {}),
    suspend: vi.fn(async () => {}),
    close: vi.fn(),
  };
  audio.context = context as unknown as AudioContext;
  void audio.ready();
  return { audio, buffers, sources, gain, context };
}
describe("bounded browser audio fallback", () => {
  it("applies a persisted volume to the shared output and saves live changes", () => {
    const storage = {
      getItem: vi.fn((key: string) =>
        key === "evenfall-volume" ? "0.37" : null,
      ),
      setItem: vi.fn(),
      removeItem: vi.fn(),
    };
    vi.stubGlobal("localStorage", storage);
    try {
      const { audio, gain, context, sources } = output(new AudioOutput());
      expect(gain.gain.value).toBe(0.37);
      expect(gain.connect).toHaveBeenCalledWith(context.destination);
      audio.send(new Int16Array([256, -256]));
      expect(sources[0].connect).toHaveBeenCalledWith(gain);
      audio.setVolume(0.62);
      expect(gain.gain.value).toBe(0.62);
      expect(storage.setItem).toHaveBeenCalledWith("evenfall-volume", "0.62");
      expect(new AudioOutput().volume).toBe(0.37);
    } finally {
      vi.unstubAllGlobals();
    }
  });
  /** The volume an earlier player stored under its own key moves once. */
  it("moves a volume stored under the legacy key", () => {
    const values = new Map([["zigba-volume", "0.25"]]);
    vi.stubGlobal("localStorage", {
      getItem: (key: string) => values.get(key) ?? null,
      setItem: (key: string, value: string) => values.set(key, value),
      removeItem: (key: string) => values.delete(key),
    });
    try {
      expect(new AudioOutput().volume).toBe(0.25);
      expect([...values]).toEqual([["evenfall-volume", "0.25"]]);
      expect(new AudioOutput().volume).toBe(0.25);
    } finally {
      vi.unstubAllGlobals();
    }
  });
  it("ignores corrupt volume preferences and works without storage", () => {
    vi.stubGlobal("localStorage", { getItem: () => "Infinity" });
    try {
      expect(new AudioOutput().volume).toBe(1);
      vi.stubGlobal("localStorage", {
        getItem: () => {
          throw new Error("storage denied");
        },
        setItem: () => {
          throw new Error("storage denied");
        },
      });
      const { audio, gain } = output(new AudioOutput());
      expect(() => audio.setVolume(0.24)).not.toThrow();
      expect(gain.gain.value).toBe(0.24);
    } finally {
      vi.unstubAllGlobals();
    }
  });
  it("routes worklet audio through the same gain and keeps volume through mute", async () => {
    const node = {
      port: { postMessage: vi.fn(), onmessage: null },
      connect: vi.fn(),
      disconnect: vi.fn(),
    };
    vi.stubGlobal("document", { baseURI: "http://localhost/" });
    vi.stubGlobal(
      "AudioWorkletNode",
      vi.fn(function () {
        return node;
      }),
    );
    try {
      const { audio, gain, context } = output();
      Object.assign(context, {
        audioWorklet: { addModule: vi.fn(async () => {}) },
      });
      audio.setVolume(0.37);
      await audio.play();
      await vi.waitFor(() => expect(node.connect).toHaveBeenCalledWith(gain));
      expect(gain.gain.value).toBe(0.37);
      await audio.mute();
      expect(context.suspend).toHaveBeenCalledOnce();
      expect(gain.gain.value).toBe(0.37);
      await audio.mute();
      expect(context.resume).toHaveBeenCalledTimes(2);
      expect(gain.gain.value).toBe(0.37);
    } finally {
      vi.unstubAllGlobals();
    }
  });
  it("changes pitch and buffer duration with speed, clearing old audio", () => {
    const { audio, sources } = output();
    audio.send(new Int16Array(960));
    audio.setSpeed(0.5);
    expect(sources[0].stop).toHaveBeenCalledOnce();
    audio.send(new Int16Array(960));
    audio.send(new Int16Array(960));
    expect(sources[1].playbackRate.value).toBe(0.5);
    expect(
      sources[2].start.mock.calls[0][0] - sources[1].start.mock.calls[0][0],
    ).toBeCloseTo(0.02);
    audio.setSpeed(2);
    audio.send(new Int16Array(960));
    expect(sources[3].playbackRate.value).toBe(2);
    audio.setSpeed(16);
    const recycle = vi.fn();
    audio.onRecycle = recycle;
    const skipped = new Int16Array(960);
    audio.send(skipped);
    expect(sources).toHaveLength(4);
    expect(recycle).toHaveBeenCalledWith(skipped);
  });
  it("uses the same DAC scale and saturation as the worklet", () => {
    const { audio, buffers } = output();
    audio.send(new Int16Array([1024, -1024, 256, -256]));
    expect([...buffers[0][0]]).toEqual([1, 0.5]);
    expect([...buffers[0][1]]).toEqual([-1, -0.5]);
  });
  it("limits pending sources and clears stale presentation on pause", () => {
    const { audio, sources } = output();
    for (let i = 0; i < 17; i++) audio.send(new Int16Array([1, 1]));
    for (const source of sources.slice(0, 16))
      expect(source.stop).toHaveBeenCalledOnce();
    expect(sources[16].stop).not.toHaveBeenCalled();
    audio.clear();
    expect(sources[16].stop).toHaveBeenCalledOnce();
  });
});

it("reuses canvas storage and preserves all 32768 clamped RGB conversions", async () => {
  const { draw } = await import("../src/media");
  const image = { data: new Uint8ClampedArray(38400 * 4) };
  const ctx = { createImageData: vi.fn(() => image), putImageData: vi.fn() };
  const canvas = {
    getContext: vi.fn(() => ctx),
  } as unknown as HTMLCanvasElement;
  const pixels = Uint16Array.from({ length: 38400 }, (_, i) => i | 0x8000);
  const expected = new Uint8ClampedArray(38400 * 4);
  for (let i = 0; i < pixels.length; i++) {
    const p = pixels[i];
    expected[i * 4] = ((p & 31) * 255) / 31;
    expected[i * 4 + 1] = (((p >> 5) & 31) * 255) / 31;
    expected[i * 4 + 2] = (((p >> 10) & 31) * 255) / 31;
    expected[i * 4 + 3] = 255;
  }
  draw(canvas, pixels);
  // Compare every byte without Vitest's expensive typed-array deep traversal.
  expect(Buffer.from(image.data).equals(Buffer.from(expected))).toBe(true);
  pixels.fill(0);
  draw(canvas, pixels);
  expect(ctx.createImageData).toHaveBeenCalledOnce();
  expect(ctx.putImageData).toHaveBeenCalledTimes(2);
  expect([...image.data.slice(0, 8)]).toEqual([0, 0, 0, 255, 0, 0, 0, 255]);
});

it("presents only the newest frame per refresh and returns every owned buffer", () => {
  let refresh: FrameRequestCallback | undefined;
  const request = vi.fn((callback: FrameRequestCallback) => {
    refresh = callback;
    return 0;
  });
  const cancel = vi.fn(() => {
    refresh = undefined;
  });
  vi.stubGlobal("requestAnimationFrame", request);
  vi.stubGlobal("cancelAnimationFrame", cancel);
  try {
    const image = { data: new Uint8ClampedArray(38400 * 4) };
    const ctx = { createImageData: () => image, putImageData: vi.fn() };
    const canvas = { getContext: () => ctx } as unknown as HTMLCanvasElement;
    const recycled: Uint16Array[] = [];
    const video = new VideoOutput((pixels) => recycled.push(pixels));
    const frames = Array.from({ length: 8 }, (_, i) =>
      new Uint16Array(38400).fill(i === 7 ? 0x7fff : 0),
    );
    for (const frame of frames) video.send(canvas, frame);
    expect(request).toHaveBeenCalledOnce();
    expect(ctx.putImageData).not.toHaveBeenCalled();
    // Ownership requires the same buffers, not just identical pixel contents.
    expect(recycled).toHaveLength(7);
    frames.slice(0, 7).forEach((frame, i) => expect(recycled[i]).toBe(frame));
    refresh!(0);
    expect(ctx.putImageData).toHaveBeenCalledOnce();
    expect([...image.data.slice(0, 4)]).toEqual([255, 255, 255, 255]);
    expect(recycled).toHaveLength(8);
    frames.forEach((frame, i) => expect(recycled[i]).toBe(frame));

    // No refresh while hidden: replacing frames still retains only one.
    for (const frame of frames) video.send(canvas, frame);
    video.clear();
    expect(cancel).toHaveBeenCalledWith(0);
    expect(refresh).toBeUndefined();
    expect(recycled).toHaveLength(16);
    [...frames, ...frames].forEach((frame, i) =>
      expect(recycled[i]).toBe(frame),
    );
    video.clear();
    expect(recycled).toHaveLength(16);
    expect(ctx.putImageData).toHaveBeenCalledOnce();
    video.send(canvas, frames[0]);
    video.flush();
    expect([...image.data.slice(0, 4)]).toEqual([0, 0, 0, 255]);
    expect(refresh).toBeUndefined();
    video.flush();
    expect(ctx.putImageData).toHaveBeenCalledTimes(2);
  } finally {
    vi.unstubAllGlobals();
  }
});
