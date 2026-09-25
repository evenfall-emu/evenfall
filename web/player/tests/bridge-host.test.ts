import { afterEach, expect, it, vi } from "vitest";
import {
  BridgeHost,
  defineGame,
  definePreparedGame,
  ref,
  u8,
} from "@evenfall/gba/bridge";
import { createHash } from "node:crypto";
import type { MemorySnapshot } from "@evenfall/gba";

const profile = {
  romHash: "a".repeat(40),
  source: {
    repository: "fixture",
    revision: "1",
    variant: "test",
    compiler: "none",
    options: [],
  },
  symbols: { count: ref(0x02000000, u8) },
};
const game = defineGame({
  id: "fixture",
  profiles: [profile],
  bridges: {
    party: {
      id: "party",
      decode: ({ memory, symbols }) => ({
        status: "ready",
        data: { count: memory.read(symbols.count) },
      }),
    },
    combat: {
      id: "combat",
      decode: ({ memory, symbols }) => ({
        status: "ready",
        data: { active: memory.read(symbols.count) > 2 },
      }),
    },
  },
});
function sample(sequence = 1, timeline = "first", count = 3): MemorySnapshot {
  return {
    romHash: profile.romHash,
    cycle: 90n,
    timeline,
    sequence,
    regions: [{ address: 0x02000000, bytes: new Uint8Array([count]) }],
  };
}
function worker() {
  return Object.assign(new EventTarget(), {
    captureMemory: vi
      .fn<() => Promise<MemorySnapshot | null>>()
      .mockResolvedValue(sample()),
  });
}
afterEach(() => vi.useRealTimers());

it("shares one capture and collection store, throttles demand and stops when unobserved", async () => {
  vi.useFakeTimers();
  const emulator = worker(),
    host = new BridgeHost(emulator);
  const store = host.observe(game);
  expect(host.observe(game)).toBe(store);
  expect(emulator.captureMemory).not.toHaveBeenCalled();
  const leaveA = store.subscribe(vi.fn()),
    leaveB = store.subscribe(vi.fn());
  await vi.advanceTimersByTimeAsync(0);
  expect(emulator.captureMemory).toHaveBeenCalledTimes(1);
  expect(store.getSnapshot()).toMatchObject({
    party: { status: "ready", data: { count: 3 }, sample: { sequence: 1 } },
    combat: {
      status: "ready",
      data: { active: true },
      sample: { sequence: 1 },
    },
  });
  emulator.captureMemory.mockResolvedValue(sample(2, "first", 1));
  for (let i = 0; i < 20; i++) emulator.dispatchEvent(new Event("frame"));
  await vi.advanceTimersByTimeAsync(49);
  expect(emulator.captureMemory).toHaveBeenCalledTimes(1);
  await vi.advanceTimersByTimeAsync(1);
  expect(store.getSnapshot().combat).toMatchObject({
    status: "ready",
    data: { active: false },
  });
  leaveA();
  leaveB();
  emulator.dispatchEvent(new Event("frame"));
  await vi.advanceTimersByTimeAsync(1000);
  expect(emulator.captureMemory).toHaveBeenCalledTimes(2);
  expect(store.getSnapshot().party.status).toBe("idle");
  host.dispose();
});

it("invalidates on replacement, rejects late replies and accepts new sequences at the same cycle", async () => {
  vi.useFakeTimers();
  const emulator = worker(),
    host = new BridgeHost(emulator);
  const store = host.observe(game);
  const leave = store.subscribe(vi.fn());
  await vi.advanceTimersByTimeAsync(0);
  let complete!: (sample: MemorySnapshot) => void;
  emulator.captureMemory.mockImplementationOnce(
    () =>
      new Promise((resolve) => {
        complete = resolve;
      }),
  );
  emulator.dispatchEvent(new Event("frame"));
  await vi.advanceTimersByTimeAsync(50);
  emulator.dispatchEvent(
    new CustomEvent("clear", { detail: { timeline: "replacement" } }),
  );
  expect(store.getSnapshot().party.status).toBe("loading");
  await vi.advanceTimersByTimeAsync(100);
  expect(emulator.captureMemory).toHaveBeenCalledTimes(2); // Still only one in flight.
  complete(sample(20));
  await Promise.resolve();
  expect(store.getSnapshot().party.status).toBe("loading");
  emulator.captureMemory.mockResolvedValue(sample(1, "replacement", 4));
  await vi.advanceTimersByTimeAsync(0);
  const current = store.getSnapshot();
  expect(current.party).toMatchObject({ status: "ready", data: { count: 4 } });
  // Pause/speed clear presentation but do not replace guest history.
  emulator.dispatchEvent(
    new CustomEvent("clear", { detail: { timeline: "replacement" } }),
  );
  expect(store.getSnapshot()).toBe(current);
  emulator.captureMemory.mockResolvedValue(sample(2, "replacement", 5));
  await vi.advanceTimersByTimeAsync(50);
  expect(store.getSnapshot().party).toMatchObject({
    status: "ready",
    data: { count: 5 },
  });
  const newest = store.getSnapshot();
  for (const stale of [
    sample(1, "replacement"),
    sample(2, "replacement"),
    sample(99, "first"),
  ]) {
    emulator.captureMemory.mockResolvedValue(stale);
    emulator.dispatchEvent(new Event("frame"));
    await vi.advanceTimersByTimeAsync(50);
    expect(store.getSnapshot()).toBe(newest);
  }
  leave();
  host.dispose();
});

it("drops captures across unsubscribe/rejoin and shuts down on worker termination", async () => {
  vi.useFakeTimers();
  const emulator = worker(),
    host = new BridgeHost(emulator),
    store = host.observe(game);
  let complete!: (sample: MemorySnapshot) => void;
  emulator.captureMemory.mockImplementationOnce(
    () =>
      new Promise((resolve) => {
        complete = resolve;
      }),
  );
  const leave = store.subscribe(vi.fn());
  await vi.advanceTimersByTimeAsync(0);
  leave();
  emulator.dispatchEvent(
    new CustomEvent("clear", { detail: { timeline: "second" } }),
  );
  const rejoined = store.subscribe(vi.fn());
  await vi.advanceTimersByTimeAsync(100);
  expect(emulator.captureMemory).toHaveBeenCalledTimes(1);
  complete(sample());
  await Promise.resolve();
  expect(store.getSnapshot().party.status).toBe("loading");
  emulator.captureMemory.mockResolvedValue(sample(1, "second", 6));
  await vi.advanceTimersByTimeAsync(0);
  expect(store.getSnapshot().party).toMatchObject({
    status: "ready",
    data: { count: 6 },
  });
  emulator.dispatchEvent(new Event("frame"));
  emulator.dispatchEvent(new Event("close"));
  await vi.advanceTimersByTimeAsync(1000);
  expect(store.getSnapshot().party.status).toBe("idle");
  expect(emulator.captureMemory).toHaveBeenCalledTimes(2);
  rejoined();
  host.dispose();
});

it("reports absent, unsupported and failed captures without a paused polling loop", async () => {
  vi.useFakeTimers();
  const emulator = worker(),
    host = new BridgeHost(emulator),
    store = host.observe(game);
  emulator.captureMemory.mockResolvedValue(null);
  const leave = store.subscribe(vi.fn());
  await vi.advanceTimersByTimeAsync(1000);
  expect(store.getSnapshot().party.status).toBe("idle");
  expect(emulator.captureMemory).toHaveBeenCalledTimes(1);
  emulator.captureMemory.mockResolvedValue({
    ...sample(),
    romHash: "b".repeat(40),
  });
  emulator.dispatchEvent(
    new CustomEvent("clear", { detail: { timeline: "first" } }),
  );
  await vi.advanceTimersByTimeAsync(0);
  expect(store.getSnapshot().party).toEqual({
    status: "unsupported",
    romHash: "b".repeat(40),
  });
  emulator.captureMemory.mockRejectedValueOnce(new Error("SessionBusy"));
  emulator.dispatchEvent(new Event("frame"));
  await vi.advanceTimersByTimeAsync(50);
  expect(store.getSnapshot().party).toEqual({
    status: "error",
    message: "SessionBusy",
  });
  emulator.captureMemory.mockResolvedValue(sample(2));
  emulator.dispatchEvent(new Event("frame"));
  await vi.advanceTimersByTimeAsync(50);
  expect(store.getSnapshot().party.status).toBe("ready");
  leave();
  host.dispose();
});

it("cannot subscribe to a worker terminated before the first consumer mounts", async () => {
  vi.useFakeTimers();
  const emulator = worker(),
    host = new BridgeHost(emulator);
  emulator.dispatchEvent(new Event("close"));
  const store = host.observe(game);
  const leave = store.subscribe(vi.fn());
  await vi.advanceTimersByTimeAsync(100);
  expect(store.getSnapshot().party.status).toBe("idle");
  expect(emulator.captureMemory).not.toHaveBeenCalled();
  leave();
  host.dispose();
});

it("retains sample identity when observed game collections are ambiguous", async () => {
  vi.useFakeTimers();
  const emulator = worker(),
    host = new BridgeHost(emulator);
  const first = host.observe(game);
  const second = host.observe(defineGame({ ...game, id: "ambiguous" }));
  const leaveA = first.subscribe(vi.fn()),
    leaveB = second.subscribe(vi.fn());
  await vi.advanceTimersByTimeAsync(0);
  expect(emulator.captureMemory).toHaveBeenCalledTimes(1);
  expect(first.getSnapshot().party).toEqual({
    status: "error",
    message: "ROM matches multiple game definitions",
    sample: {
      romHash: profile.romHash,
      timeline: "first",
      sequence: 1,
      cycle: 90n,
    },
  });
  expect(second.getSnapshot().combat).toEqual(first.getSnapshot().party);
  const state = first.getSnapshot().party;
  if (state.status === "error")
    expect(Object.isFrozen(state.sample)).toBe(true);
  leaveA();
  leaveB();
  host.dispose();
});

function preparedWorker() {
  const bytes = new Uint8Array([17, 91, 3]);
  const romHash = createHash("sha1").update(bytes).digest("hex");
  const prepare = vi.fn(
    ({
      rom,
    }: {
      rom: import("@evenfall/gba/bridge").MemoryReader;
      romHash: string;
    }) => ({ value: rom.u8(0x08000001) }),
  );
  const unused = vi.fn(() => ({ status: "ready" as const, data: 0 }));
  const game = definePreparedGame(prepare)({
    id: "prepared",
    profiles: [{ ...profile, romHash }],
    bridges: {
      party: {
        id: "party",
        decode: ({ resources }) => ({ status: "ready", data: resources.value }),
      },
      storage: { id: "storage", decode: unused },
    },
  });
  let sequence = 0;
  const emulator = Object.assign(new EventTarget(), {
    captureMemory: vi.fn(async () => ({ ...sample(++sequence), romHash })),
    exportRom: vi.fn(async () => bytes.slice()),
  });
  const host = new BridgeHost(emulator),
    store = host.observe(game);
  return { emulator, host, store, prepare, unused, bytes, romHash };
}

// SHA-256 is native asynchronous work, so resource tests use real timers.
it("prepares verified ROM resources once and only decodes subscribed keys", async () => {
  const { emulator, host, store, prepare, unused } = preparedWorker();
  try {
    const leave = store.subscribe(() => {}, "party");
    await vi.waitFor(() =>
      expect(store.getSnapshot().party).toMatchObject({
        status: "ready",
        data: 91,
      }),
    );
    const resources = store.getResources();
    expect(unused).not.toHaveBeenCalled();
    expect(store.getSnapshot().storage.status).toBe("idle");
    const storage = store.subscribe(() => {}, "storage");
    await vi.waitFor(() => expect(unused).toHaveBeenCalledTimes(1));
    storage();
    expect(store.getSnapshot().storage.status).toBe("idle");
    emulator.dispatchEvent(new Event("frame"));
    await vi.waitFor(() =>
      expect(emulator.captureMemory).toHaveBeenCalledTimes(3),
    );
    expect(unused).toHaveBeenCalledTimes(1);
    expect(prepare).toHaveBeenCalledTimes(1);
    expect(emulator.exportRom).toHaveBeenCalledTimes(1);
    expect(store.getResources()).toBe(resources);
    leave();
    expect(store.getResources()).toBeUndefined();
    const rejoin = store.subscribe(() => {}, "party");
    await vi.waitFor(() => expect(prepare).toHaveBeenCalledTimes(2));
    expect(store.getResources()).not.toBe(resources);
    rejoin();
  } finally {
    host.dispose();
  }
});

it("rejects a mismatched ROM export without preparing or publishing resources", async () => {
  const { emulator, host, store, prepare } = preparedWorker();
  emulator.exportRom.mockResolvedValue(new Uint8Array([8, 9]));
  try {
    store.subscribe(() => {}, "party");
    await vi.waitFor(() =>
      expect(store.getSnapshot().party).toMatchObject({
        status: "error",
        message: "ROM changed during resource preparation",
      }),
    );
    expect(prepare).not.toHaveBeenCalled();
    expect(store.getResources()).toBeUndefined();
  } finally {
    host.dispose();
  }
});

it.each(["replacement", "unsubscribe", "close"])(
  "drops in-flight preparation on %s",
  async (action) => {
    const { emulator, host, store, prepare, bytes } = preparedWorker();
    let release!: (value: typeof bytes) => void;
    emulator.exportRom.mockImplementationOnce(
      () =>
        new Promise((resolve) => {
          release = resolve;
        }),
    );
    try {
      const leave = store.subscribe(() => {}, "party");
      await vi.waitFor(() =>
        expect(emulator.exportRom).toHaveBeenCalledTimes(1),
      );
      if (action === "replacement") {
        emulator.captureMemory.mockImplementation(async () => ({
          ...sample(1, "new"),
          romHash: "b".repeat(40),
        }));
        emulator.dispatchEvent(
          new CustomEvent("clear", { detail: { timeline: "new" } }),
        );
      } else if (action === "unsubscribe") leave();
      else emulator.dispatchEvent(new Event("close"));
      release(bytes);
      await vi.waitFor(() =>
        expect(store.getSnapshot().party.status).toBe(
          action === "replacement" ? "unsupported" : "idle",
        ),
      );
      expect(prepare).not.toHaveBeenCalled();
      expect(store.getResources()).toBeUndefined();
    } finally {
      host.dispose();
    }
  },
);
