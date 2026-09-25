import "fake-indexeddb/auto";
import { deleteDB } from "idb";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import { CloudSync, planQuickSlot, type CloudStatus } from "../src/cloud";
import {
  openLibrary,
  quickKind,
  quickSlotForKey,
  quickStateKey,
  type CloudJob,
  type Library,
  type QuickRemote,
} from "../src/library";

const hash = "a".repeat(40);
const ref = { hash, playthroughId: "default" };
const state = (marker: number) =>
  new Uint8Array([...new TextEncoder().encode("ZIGBAST1"), marker]);

async function gzip(bytes: Uint8Array) {
  return new Uint8Array(
    await new Response(
      new Blob([new Uint8Array(bytes)])
        .stream()
        .pipeThrough(new CompressionStream("gzip")),
    ).arrayBuffer(),
  );
}
async function gunzip(bytes: Uint8Array) {
  return new Uint8Array(
    await new Response(
      new Response(new Uint8Array(bytes)).body!.pipeThrough(
        new DecompressionStream("gzip"),
      ),
    ).arrayBuffer(),
  );
}
async function hex(bytes: Uint8Array) {
  return Array.from(
    new Uint8Array(
      await crypto.subtle.digest("SHA-256", new Uint8Array(bytes)),
    ),
    (byte) => byte.toString(16).padStart(2, "0"),
  ).join("");
}

type Stored = Omit<QuickRemote, "slot" | "uploadedAt"> & { body: Uint8Array };
/** Conditional-write semantics of the Worker's quick-slot routes. */
class FakeCloud {
  accounts = new Map<string, Map<number, Stored>>();
  session: string | null = "alice";
  puts: { account: string; slot: number; headers: Headers }[] = [];
  etags = 0;
  slots(account: string) {
    let slots = this.accounts.get(account);
    if (!slots) this.accounts.set(account, (slots = new Map()));
    return slots;
  }
  async seed(
    account: string,
    slot: number,
    data: Uint8Array,
    capturedAt: number,
  ) {
    const body = await gzip(data);
    const stored = {
      body,
      digest: await hex(body),
      etag: `"etag-${++this.etags}"`,
      revision: crypto.randomUUID(),
      capturedAt,
    };
    this.slots(account).set(slot, stored);
    return stored;
  }
  fetch = vi.fn(async (url: string, init: RequestInit = {}) => {
    if (url === "/api/auth/get-session")
      return Response.json(
        this.session ? { user: { id: this.session } } : null,
      );
    if (url.startsWith("/api/saves?") || url === "/api/saves")
      return Response.json({ saves: [], cursor: null });
    const headers = new Headers(init.headers);
    const account = headers.get("X-Zigba-Account")!;
    const listing = /^\/api\/saves\/[0-9a-f]{40}\/default\/quick$/.test(url);
    if (listing)
      return Response.json({
        limit: 10,
        slots: [...this.slots(account)].map(([slot, stored]) => ({
          slot,
          revision: stored.revision,
          digest: stored.digest,
          capturedAt: stored.capturedAt,
          etag: stored.etag,
          uploadedAt: new Date().toISOString(),
        })),
      });
    const slot = Number(/\/quick\/(\d+)$/.exec(url)?.[1]);
    if (!slot) throw new Error(`Unexpected request ${url}`);
    const current = this.slots(account).get(slot);
    const method = init.method ?? "GET";
    if (method === "GET" || method === "HEAD") {
      if (!current) return new Response(null, { status: 404 });
      return new Response(
        method === "GET" ? new Blob([new Uint8Array(current.body)]) : null,
        {
          headers: {
            ETag: current.etag,
            "X-Zigba-Digest": current.digest,
            "X-Zigba-Revision": current.revision,
            "X-Zigba-Uploaded-At": new Date().toISOString(),
          },
        },
      );
    }
    this.puts.push({ account, slot, headers });
    const ifMatch = headers.get("If-Match");
    if (ifMatch ? current?.etag !== ifMatch : current)
      return Response.json(
        { error: { code: "precondition_failed" } },
        { status: 412 },
      );
    const body = new Uint8Array(init.body as Uint8Array);
    const stored = {
      body,
      digest: headers.get("X-Zigba-Digest")!,
      etag: `"etag-${++this.etags}"`,
      revision: headers.get("X-Zigba-Revision")!,
      capturedAt: Number(headers.get("X-Zigba-Captured-At")),
    };
    this.slots(account).set(slot, stored);
    return Response.json({
      etag: stored.etag,
      revision: stored.revision,
      uploadedAt: new Date().toISOString(),
    });
  });
}

let library: Library;
let cloud: CloudSync;
let status: CloudStatus;
let server: FakeCloud;
const online = { value: true };

beforeEach(async () => {
  await deleteDB("zigba-local");
  library = await openLibrary();
  server = new FakeCloud();
  online.value = true;
  vi.stubGlobal("window", new EventTarget());
  vi.stubGlobal("navigator", {
    get onLine() {
      return online.value;
    },
    locks: {
      request: async (_name: string, operation: () => Promise<unknown>) =>
        operation(),
    },
  });
  vi.stubGlobal("fetch", server.fetch);
  await library.put({
    hash,
    title: "Game",
    rom: new Uint8Array([1]),
    lastPlayed: 1,
  });
  cloud = new CloudSync(library, (value) => {
    status = value;
  });
  await cloud.setIdentity("alice");
});
afterEach(() => {
  cloud.close();
  library.close();
  vi.unstubAllGlobals();
  vi.restoreAllMocks();
});

async function job(account: string, slot: number) {
  return (await library.cloudJobs(account)).find(
    (item) => item.kind === quickKind(slot),
  );
}
async function localBytes(slot: number) {
  return (await library.getQuickSlot(ref, slot))?.data;
}

describe("planQuickSlot", () => {
  const remote: QuickRemote = {
    slot: 1,
    revision: "cloud-2",
    digest: "d",
    capturedAt: 5,
    etag: '"e2"',
    uploadedAt: "",
  };
  const synced: CloudJob = {
    accountId: "alice",
    ...ref,
    kind: "quick/1",
    revision: "r1",
    uploadedRevision: "r1",
    baseEtag: '"e1"',
    capturedAt: 1,
    lastInputAt: 0,
  };
  const local = {
    data: state(1),
    savedAt: 1,
    cloudRevision: "r1",
    accountId: "alice",
  };
  it("uploads only-local changes, downloads only-cloud changes, and flags true divergence", () => {
    const unchanged = { ...remote, revision: "r1", etag: '"e1"' };
    expect(planQuickSlot("alice", local, synced, unchanged).type).toBe("none");
    // Pending local revision on an unchanged cloud copy: the outbox uploads it.
    const pending = { ...synced, revision: "r2" };
    const edited = { ...local, cloudRevision: "r2" };
    expect(
      planQuickSlot("alice", edited, pending, { ...remote, etag: '"e1"' }),
    ).toEqual({ type: "none" });
    expect(planQuickSlot("alice", local, synced, remote)).toEqual({
      type: "download",
    });
    expect(planQuickSlot("alice", undefined, undefined, remote)).toEqual({
      type: "download",
    });
    expect(planQuickSlot("alice", edited, pending, remote)).toEqual({
      type: "conflict",
    });
    // Unsynced legacy/offline slots have no common base with an existing copy.
    expect(
      planQuickSlot("alice", { data: state(1), savedAt: 0 }, undefined, remote),
    ).toEqual({ type: "conflict" });
    expect(
      planQuickSlot(
        "alice",
        { data: state(1), savedAt: 0 },
        undefined,
        undefined,
      ),
    ).toEqual({ type: "claim" });
    // A lost acknowledgement is not a conflict.
    expect(
      planQuickSlot(
        "alice",
        { ...edited, cloudRevision: "cloud-2" },
        pending,
        remote,
      ),
    ).toEqual({ type: "adopt" });
    // A conflict flag without any cloud change since the base is re-queued.
    expect(
      planQuickSlot(
        "alice",
        edited,
        { ...pending, conflict: true },
        { ...remote, etag: '"e1"' },
      ),
    ).toEqual({ type: "claim", baseEtag: '"e1"' });
  });
  it("never offers another account's slot for upload automatically", () => {
    const foreign = { ...local, accountId: "bob" };
    expect(planQuickSlot("alice", foreign, undefined, undefined)).toEqual({
      type: "none",
    });
    expect(planQuickSlot("alice", foreign, undefined, remote)).toEqual({
      type: "conflict",
    });
  });
  it("maps Digit1-9 and Digit0 to slots 1-10", () => {
    expect(
      ["Digit1", "Digit9", "Digit0", "KeyA", "Numpad1"].map(quickSlotForKey),
    ).toEqual([1, 9, 10, undefined, undefined]);
  });
});

it("saves locally first, then uploads that slot with If-None-Match and records the acknowledged base", async () => {
  const { queued } = await library.saveQuickSlot(ref, 3, state(3));
  expect(queued).toBe(true);
  expect(await localBytes(3)).toEqual(state(3));
  expect((await library.quickSlots(ref, "alice"))[2].sync).toBe("pending");
  await cloud.flush();
  expect(server.puts).toHaveLength(1);
  expect(server.puts[0].slot).toBe(3);
  expect(server.puts[0].headers.get("If-None-Match")).toBe("*");
  expect(await gunzip(server.slots("alice").get(3)!.body)).toEqual(state(3));
  const acknowledged = await job("alice", 3);
  expect(acknowledged?.uploadedRevision).toBe(acknowledged?.revision);
  expect(acknowledged?.baseEtag).toBe(server.slots("alice").get(3)!.etag);
  expect((await library.quickSlots(ref, "alice"))[2].sync).toBe("synced");
  // A repeated save replaces the one pending revision and waits out pacing.
  await library.saveQuickSlot(ref, 3, state(4));
  await library.saveQuickSlot(ref, 3, state(5));
  expect(
    (await library.cloudJobs("alice")).filter(
      (item) => item.kind === "quick/3",
    ),
  ).toHaveLength(1);
  await cloud.flush();
  expect(server.puts).toHaveLength(1);
});

it("silently replaces an unchanged local slot with a newer cloud save from another device", async () => {
  await library.saveQuickSlot(ref, 1, state(1));
  await cloud.flush();
  await server.seed("alice", 1, state(9), 1234);
  await cloud.syncQuick(ref);
  expect(await localBytes(1)).toEqual(state(9));
  expect((await library.getQuickSlot(ref, 1))?.savedAt).toBe(1234);
  expect(status.quickConflicts).toEqual([]);
  expect(server.puts).toHaveLength(1);
  expect((await library.quickSlots(ref, "alice"))[0].sync).toBe("synced");
});

it("discovers and downloads every cloud slot for a playthrough on a new device", async () => {
  await server.seed("alice", 1, state(1), 10);
  await server.seed("alice", 10, state(10), 20);
  await cloud.syncQuick(ref);
  expect(await localBytes(1)).toEqual(state(1));
  expect(await localBytes(10)).toEqual(state(10));
  expect(
    (await library.quickSlots(ref, "alice")).map((slot) => slot.sync),
  ).toEqual(["synced", ...Array(8).fill(undefined), "synced"]);
  expect(server.puts).toHaveLength(0);
});

it("asks once about true divergence, keeps both copies until chosen, then applies the choice", async () => {
  await library.saveQuickSlot(ref, 2, state(1));
  await cloud.flush();
  const cloudCopy = await server.seed("alice", 2, state(8), 50);
  vi.spyOn(Date, "now").mockReturnValue(Date.now() + 60_000);
  await library.saveQuickSlot(ref, 2, state(2));
  // The conditional upload fails; neither side is overwritten.
  await cloud.flush();
  expect(server.puts.at(-1)?.headers.get("If-Match")).not.toBe(cloudCopy.etag);
  expect(server.slots("alice").get(2)?.etag).toBe(cloudCopy.etag);
  expect(await localBytes(2)).toEqual(state(2));
  expect(status.quickConflicts).toMatchObject([
    { slot: 2, cloudSavedAt: 50, prompted: false, etag: cloudCopy.etag },
  ]);
  expect(status.quickConflicts![0].localSavedAt).toBeGreaterThan(50);
  await cloud.markQuickPrompted(status.quickConflicts!);
  expect(status.quickConflicts![0].prompted).toBe(true);
  // Later syncs keep the divergence visible without re-asking.
  await cloud.syncQuick(ref);
  expect(status.quickConflicts).toMatchObject([{ slot: 2, prompted: true }]);
  expect(await localBytes(2)).toEqual(state(2));

  await cloud.resolveQuick(ref, [{ slot: 2, keep: "local" }]);
  expect(server.puts.at(-1)?.headers.get("If-Match")).toBe(cloudCopy.etag);
  expect(await gunzip(server.slots("alice").get(2)!.body)).toEqual(state(2));
  expect(status.quickConflicts).toEqual([]);
});

it("replaces the local copy only when the user keeps the cloud side of a divergence", async () => {
  await library.saveQuickSlot(ref, 4, state(1));
  await server.seed("alice", 4, state(7), 99);
  await cloud.syncQuick(ref);
  expect(status.quickConflicts).toMatchObject([{ slot: 4 }]);
  expect(await localBytes(4)).toEqual(state(1));
  const puts = server.puts.length;
  await cloud.resolveQuick(ref, [{ slot: 4, keep: "cloud" }]);
  expect(await localBytes(4)).toEqual(state(7));
  expect(server.puts).toHaveLength(puts);
  expect(status.quickConflicts).toEqual([]);
});

it("queues offline saves without network use and uploads them on reconnect", async () => {
  await cloud.start();
  online.value = false;
  await library.saveQuickSlot(ref, 5, state(5));
  server.fetch.mockClear();
  await cloud.flush();
  await cloud.syncQuick(ref);
  expect(server.fetch).not.toHaveBeenCalled();
  expect((await job("alice", 5))?.uploadedRevision).toBeUndefined();
  online.value = true;
  window.dispatchEvent(new Event("online"));
  await vi.waitFor(async () =>
    expect((await job("alice", 5))?.uploadedRevision).toBeDefined(),
  );
  expect(await gunzip(server.slots("alice").get(5)!.body)).toEqual(state(5));
});

it("does not upload, retag, or overwrite another account's slot after switching accounts", async () => {
  online.value = false;
  await library.saveQuickSlot(ref, 6, state(6));
  online.value = true;
  server.session = "bob";
  await cloud.setIdentity("bob");
  await cloud.flush();
  await cloud.syncQuick(ref);
  expect(server.puts).toHaveLength(0);
  expect(await job("bob", 6)).toBeUndefined();
  expect((await job("alice", 6))?.accountId).toBe("alice");
  // Bob's own cloud copy is a divergence, not a silent overwrite.
  await server.seed("bob", 6, state(66), 1);
  await cloud.syncQuick(ref);
  expect(await localBytes(6)).toEqual(state(6));
  expect(status.quickConflicts).toMatchObject([{ accountId: "bob", slot: 6 }]);
  // A session that changed underneath the device cannot receive Alice's job.
  server.session = "carol";
  await cloud.syncQuick(ref);
  expect(server.puts).toHaveLength(0);
});

it("does not queue uploads while cloud backups are disabled, and removes slots with the playthrough", async () => {
  await library.setCloudIdentity("alice", false);
  const { queued } = await library.saveQuickSlot(ref, 7, state(7));
  expect(queued).toBe(false);
  expect(await job("alice", 7)).toBeUndefined();
  await library.saveQuickSlot(ref, 10, state(10));
  await library.deletePlaythrough(ref);
  expect(await library.asset(quickStateKey(ref, 10))).toBeUndefined();
  expect(await library.asset(quickStateKey(ref, 7))).toBeUndefined();
});
