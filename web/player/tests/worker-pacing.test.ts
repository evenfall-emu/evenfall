import { afterEach, expect, it, vi } from "vite-plus/test";

const frameCycles = 280896n;

afterEach(() => {
  vi.unstubAllGlobals();
  vi.doUnmock("../../web/core.js");
  vi.doUnmock("../../web/devtools/journal.js");
  vi.doUnmock("../../web/devtools/scenario.js");
  vi.resetModules();
});

async function worker(frameWorkMs = 5, messageDelayMs = 0) {
  let now = 0,
    cycle = 0n,
    nextId = 0,
    nesting = 0;
  let acknowledge = true,
    fault = false;
  let videoInterval = 1;
  const tasks = new Map<
    number,
    { at: number; nesting: number; run: () => void }
  >();
  const events: { event?: string; generation?: number; reason?: string }[] = [];
  const scope = { onmessage: async (_event: { data: object }) => {} };
  const enqueue = (run: () => void, delay: number, level: number) => {
    const id = ++nextId;
    tasks.set(id, { at: now + delay, nesting: level, run });
    return id;
  };
  vi.stubGlobal("self", scope);
  vi.stubGlobal("performance", { now: () => now });
  // HTML timer nesting rules apply in Workers too. A Node timer mock alone
  // misses the four-millisecond idle gap that caused the browser regression.
  vi.stubGlobal("setTimeout", (run: () => void, delay = 0) =>
    enqueue(run, Math.max(nesting > 5 ? 4 : 0, delay), nesting + 1),
  );
  vi.stubGlobal("clearTimeout", (id: number) => tasks.delete(id));
  vi.stubGlobal(
    "MessageChannel",
    class {
      port1 = { onmessage: (_event: { data: number }) => {} };
      port2 = {
        postMessage: (data: number) =>
          enqueue(() => this.port1.onmessage({ data }), messageDelayMs, 0),
      };
    },
  );
  vi.stubGlobal("postMessage", (data: (typeof events)[number]) => {
    events.push(data);
    if (data.event === "frame" && acknowledge)
      enqueue(
        () => void scope.onmessage({ data: { ack: data.generation } }),
        0,
        0,
      );
  });
  vi.stubGlobal("fetch", async () => ({
    ok: true,
    arrayBuffer: async () => new ArrayBuffer(0),
  }));
  const core = {
    loaded: true,
    recorder: { snapshotIntervalCycles: 0n },
    setAudioOutput: vi.fn(),
    setVideoInterval: vi.fn((interval: number) => {
      videoInterval = interval;
    }),
    videoFrameId: () => cycle / frameCycles / BigInt(videoInterval),
    videoFrameCycle: () =>
      (cycle / frameCycles / BigInt(videoInterval)) *
      BigInt(videoInterval) *
      frameCycles,
    presentationFrame: (pixels: Uint16Array) => pixels,
    wasm: {
      memory: { buffer: new ArrayBuffer(65536) },
      zb_cycle: () => cycle,
      zb_frames: () => cycle / frameCycles,
    },
    requireLoaded() {},
    load() {},
    inspect: () => ({ cycle, frames: cycle / frameCycles, fault }),
    frame: (pixels: Uint16Array) => pixels,
    runUntil: vi.fn((deadline: bigint) => {
      // A known five-millisecond frame has capacity for 200 FPS. At 4x,
      // pacing should use that capacity, rather than add another 4 ms/frame.
      now += (frameWorkMs * Number(deadline - cycle)) / Number(frameCycles);
      cycle = deadline;
      return { reason: fault ? "faulted" : "deadline" };
    }),
    drainAudio: vi.fn((audio: Int16Array) => audio.subarray(0, 0)),
  };
  vi.doMock("../../web/core.js", () => ({
    CYCLES_PER_FRAME: frameCycles,
    WasmCore: { create: async () => core },
  }));
  vi.doMock("../../web/devtools/journal.js", () => ({
    GameplayRecording: class {
      constructor() {
        return core;
      }
    },
  }));
  vi.doMock("../../web/devtools/scenario.js", () => ({
    sha256: async () => "test-wasm",
    runScenario() {},
  }));
  await import("../../web/worker.js");
  const command = (method: string, ...args: unknown[]) =>
    scope.onmessage({ data: { id: ++nextId, method, args } });
  await command("init", "test.wasm");
  await command("load", new Uint8Array(), {});
  return {
    command,
    core,
    events,
    get frames() {
      return Number(cycle / frameCycles);
    },
    set acknowledge(value: boolean) {
      acknowledge = value;
    },
    set fault(value: boolean) {
      fault = value;
    },
    advance(ms: number) {
      const end = now + ms;
      let turns = 0;
      while (tasks.size) {
        const [id, task] = [...tasks].sort(
          (a, b) => a[1].at - b[1].at || a[0] - b[0],
        )[0];
        if (task.at > end) break;
        if (++turns > 10000) throw new Error("Worker did not yield");
        tasks.delete(id);
        now = Math.max(now, task.at);
        nesting = task.nesting;
        task.run();
        nesting = 0;
      }
      now = Math.max(now, end);
    },
  };
}

it("uses available emulation throughput at 4x despite nested browser timers", async () => {
  const emulation = await worker();
  await emulation.command("play");
  emulation.advance(1000);
  expect(emulation.frames).toBeGreaterThanOrEqual(59);
  expect(emulation.frames).toBeLessThanOrEqual(61);
  await emulation.command("setSpeed", 4);
  const start = emulation.frames;
  emulation.advance(1000);
  expect(emulation.frames - start).toBeGreaterThanOrEqual(195);
  await emulation.command("setSpeed", 0.5);
  const boosted = emulation.frames;
  emulation.advance(1000);
  expect(emulation.frames - boosted).toBeGreaterThanOrEqual(29);
  expect(emulation.frames - boosted).toBeLessThanOrEqual(31);
});

it("cancels scheduled work on pause and faults", async () => {
  const emulation = await worker();
  await emulation.command("setSpeed", 4);
  await emulation.command("play");
  emulation.advance(100);
  await emulation.command("pause");
  const paused = emulation.frames;
  emulation.advance(100);
  expect(emulation.frames).toBe(paused);
  await emulation.command("play");
  emulation.fault = true;
  emulation.advance(100);
  expect(emulation.events.filter((event) => event.event === "stopped")).toEqual(
    [expect.objectContaining({ reason: "faulted" })],
  );
  const stopped = emulation.frames;
  emulation.advance(100);
  expect(emulation.frames).toBe(stopped);
});

it("keeps the presentation queue bounded when the host stops acknowledging", async () => {
  const emulation = await worker();
  emulation.advance(0);
  emulation.acknowledge = false;
  await emulation.command("setSpeed", 4);
  await emulation.command("play");
  emulation.advance(1000);
  expect(emulation.frames).toBe(8);
});

it("sustains 8x with a fast core despite nested browser timer limits", async () => {
  const emulation = await worker(1.5);
  await emulation.command("setSpeed", 8);
  await emulation.command("play");
  emulation.advance(2000);
  expect(emulation.frames).toBeGreaterThanOrEqual(954);
  expect(emulation.frames).toBeLessThanOrEqual(957);
  await emulation.command("pause");
  const paused = emulation.frames;
  emulation.advance(100);
  expect(emulation.frames).toBe(paused);
});

it("ignores a queued pump message after pause and restart", async () => {
  const emulation = await worker(1.5, 10);
  await emulation.command("setSpeed", 8);
  await emulation.command("play");
  emulation.advance(5);
  await emulation.command("pause");
  await emulation.command("play");
  emulation.advance(5);
  expect(emulation.frames).toBe(0);
  emulation.advance(5);
  expect(emulation.frames).toBe(1);
});

it("uses whole-frame silent execution and wall-time recovery intervals at 16x", async () => {
  const emulation = await worker(0.5);
  await emulation.command("setSpeed", 16);
  await emulation.command("play");
  expect(emulation.core.setAudioOutput).toHaveBeenLastCalledWith(false);
  expect(emulation.core.setVideoInterval).toHaveBeenLastCalledWith(16);
  expect(emulation.core.recorder.snapshotIntervalCycles).toBe(
    frameCycles * 960n,
  );
  emulation.advance(1000);
  expect(emulation.frames).toBeGreaterThanOrEqual(955);
  expect(emulation.frames).toBeLessThanOrEqual(957);
  expect(emulation.core.runUntil).toHaveBeenCalledTimes(emulation.frames);
  expect(emulation.core.drainAudio).not.toHaveBeenCalled();
  expect(
    emulation.events.filter((event) => event.event === "frame").length,
  ).toBeLessThanOrEqual(62);
  await emulation.command("setSpeed", 2);
  expect(emulation.core.setAudioOutput).toHaveBeenLastCalledWith(true);
  emulation.advance(100);
  expect(emulation.core.drainAudio).toHaveBeenCalled();
  await emulation.command("pause");
  expect(emulation.core.setAudioOutput).toHaveBeenLastCalledWith(true);
  expect(emulation.core.recorder.snapshotIntervalCycles).toBe(
    frameCycles * 60n,
  );
});
