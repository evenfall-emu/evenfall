import "fake-indexeddb/auto";
import { deleteDB } from "idb";
import { afterEach, beforeEach, expect, it, vi } from "vitest";
import { CloudSync, type CloudStatus } from "../src/cloud";
import { openLibrary, type Library } from "../src/library";

let library: Library;
let cloud: CloudSync;
let status: CloudStatus;
const listing = (hash: string, uploadedAt: string, capturedAt = 1) => ({
  hash: hash.repeat(64),
  playthroughId: "default",
  playthroughName: "First adventure",
  title: `Game ${hash}`,
  uploadedAt,
  capturedAt,
});

beforeEach(async () => {
  await deleteDB("zigba-local");
  library = await openLibrary();
  vi.stubGlobal("window", new EventTarget());
  cloud = new CloudSync(library, (value) => {
    status = value;
  });
});
afterEach(() => {
  cloud.close();
  library.close();
  vi.unstubAllGlobals();
  vi.restoreAllMocks();
});

it("discovers across every page while uploads are off, using upload time rather than device clocks", async () => {
  await library.setCloudIdentity("alice", false);
  const fetcher = vi.fn(async (url: string, init: RequestInit) => {
    expect(init.headers).toEqual({ "X-Zigba-Account": "alice" });
    expect(init.cache).toBe("no-store");
    if (url === "/api/saves")
      return Response.json({
        saves: [listing("1", "2026-09-20T10:00:00Z", 9_999_999_999_999)],
        cursor: "next page",
      });
    if (url === "/api/saves?cursor=next%20page")
      return Response.json({ saves: [], cursor: "last" });
    expect(url).toBe("/api/saves?cursor=last");
    return Response.json({
      saves: [listing("f", "2026-09-22T12:00:00Z", 7)],
      cursor: null,
    });
  });
  vi.stubGlobal("fetch", fetcher);
  await cloud.setIdentity("alice");
  await vi.waitFor(() => expect(status.discovering).toBe(false));
  expect(status.enabled).toBe(false);
  expect(status.latest).toEqual({
    ...listing("f", "2026-09-22T12:00:00Z", 7),
    accountId: "alice",
    uploadedAt: Date.parse("2026-09-22T12:00:00Z"),
  });
  expect(fetcher).toHaveBeenCalledTimes(3);
  expect(await library.cloudJobs("alice")).toEqual([]);
});

it("discovers separate playthroughs of one game and account usage without downloading their saves", async () => {
  const older = listing("a", "2026-09-20T10:00:00Z");
  const newer = {
    ...listing("a", "2026-09-22T12:00:00Z"),
    playthroughId: "15b285c2-f844-451f-85e6-c5bbcc98dd09",
    playthroughName: "Challenge run",
  };
  vi.stubGlobal("fetch", async (url: string) => {
    expect(url).toMatch(/^\/api\/saves(?:\?cursor=next)?$/);
    return Response.json({
      saves: url.endsWith("next") ? [newer] : [older],
      cursor: url.endsWith("next") ? null : "next",
      usage: { playthroughs: 99, limit: 100 },
    });
  });
  await cloud.setIdentity("alice");
  await vi.waitFor(() => expect(status.discovering).toBe(false));
  expect(status.resumes?.map((run) => run.playthroughName)).toEqual([
    "Challenge run",
    "First adventure",
  ]);
  expect(status.latest?.playthroughId).toBe(newer.playthroughId);
  expect(status.usage).toEqual({ playthroughs: 99, limit: 100 });
  await cloud.setIdentity(null);
  expect(status.resumes).toEqual([]);
  expect(status.usage).toBeUndefined();
});

it("does not publish an incomplete listing as the latest backup", async () => {
  vi.stubGlobal(
    "fetch",
    vi.fn(async (url: string) =>
      url === "/api/saves"
        ? Response.json({
            saves: [listing("a", "2026-09-22T12:00:00Z")],
            cursor: "broken",
          })
        : new Response("unavailable", { status: 503 }),
    ),
  );
  await cloud.setIdentity("alice");
  await vi.waitFor(() =>
    expect(status.discoveryError).toContain("Local play still works"),
  );
  expect(status.latest).toBeUndefined();
  expect(status.discovering).toBe(false);
});

it("discards an old account's delayed listing after switching accounts", async () => {
  let release!: (response: Response) => void;
  const delayed = new Promise<Response>((resolve) => {
    release = resolve;
  });
  const fetcher = vi.fn(async (_url: string, init: RequestInit) =>
    new Headers(init.headers).get("X-Zigba-Account") === "alice"
      ? delayed
      : Response.json({
          saves: [listing("b", "2026-09-21T10:00:00Z")],
          cursor: null,
        }),
  );
  vi.stubGlobal("fetch", fetcher);
  await cloud.setIdentity("alice");
  await cloud.setIdentity("bob");
  await vi.waitFor(() => expect(status.latest?.accountId).toBe("bob"));
  const old = Response.json({
    saves: [listing("a", "2026-09-22T12:00:00Z")],
    cursor: "must-not-fetch",
  });
  release(old);
  await vi.waitFor(() => expect(old.bodyUsed).toBe(true));
  await new Promise((resolve) => setTimeout(resolve, 30));
  expect(status.latest?.hash).toBe("b".repeat(64));
  expect(fetcher).toHaveBeenCalledTimes(2);
  await cloud.setIdentity(null);
  expect(status.latest).toBeUndefined();
});

it("rejects a selected account's download before fetching bytes if the session changed", async () => {
  const fetcher = vi.fn(async (url: string) =>
    url === "/api/auth/get-session"
      ? Response.json({ user: { id: "bob" } })
      : Response.json({ saves: [], cursor: null }),
  );
  vi.stubGlobal("fetch", fetcher);
  await cloud.setIdentity("alice");
  await vi.waitFor(() => expect(status.discovering).toBe(false));
  await expect(
    cloud.download(
      { hash: "a".repeat(64), playthroughId: "default" },
      "resume",
      "alice",
    ),
  ).rejects.toThrow("Account changed");
  await vi.waitFor(() => expect(status.discovering).toBe(false));
  expect(fetcher.mock.calls.some(([url]) => url.endsWith("/resume"))).toBe(
    false,
  );
  expect(status.accountId).toBe("bob");
});

it("downloads the selected playthrough rather than another run of the same game", async () => {
  const bytes = new Uint8Array(
    await new Response(
      new Blob([new Uint8Array([9, 17, 43])])
        .stream()
        .pipeThrough(new CompressionStream("gzip")),
    ).arrayBuffer(),
  );
  const digest = Array.from(
    new Uint8Array(await crypto.subtle.digest("SHA-256", bytes)),
    (byte) => byte.toString(16).padStart(2, "0"),
  ).join("");
  const target = {
    hash: "d".repeat(64),
    playthroughId: "15b285c2-f844-451f-85e6-c5bbcc98dd09",
  };
  vi.stubGlobal("fetch", async (url: string) => {
    if (url === "/api/auth/get-session")
      return Response.json({ user: { id: "alice" } });
    if (url === "/api/saves") return Response.json({ saves: [], cursor: null });
    expect(url).toBe(
      `/api/saves/${target.hash}/${target.playthroughId}/resume`,
    );
    return new Response(bytes, {
      headers: { ETag: '"second-run"', "X-Zigba-Digest": digest },
    });
  });
  await cloud.setIdentity("alice");
  const remote = await cloud.download(target, "resume", "alice");
  expect(remote).toEqual({
    accountId: "alice",
    data: new Uint8Array([9, 17, 43]),
    etag: '"second-run"',
  });
});

it("refuses account deletion when the session changed from the account shown in the dialog", async () => {
  vi.stubGlobal("navigator", {
    locks: {
      request: async (_name: string, operation: () => Promise<void>) =>
        operation(),
    },
  });
  vi.stubGlobal("fetch", async (url: string, init?: RequestInit) => {
    expect(init?.method).not.toBe("DELETE");
    return url === "/api/auth/get-session"
      ? Response.json({ user: { id: "bob" } })
      : Response.json({ saves: [], cursor: null });
  });
  await cloud.setIdentity("alice");
  await expect(cloud.deleteAccount("alice")).rejects.toThrow("Account changed");
  expect((await library.cloudIdentity()).accountId).toBe("bob");
});

it("keeps uploads off and offers retry when deletion loses its connection", async () => {
  vi.stubGlobal("navigator", {
    locks: {
      request: async (_name: string, operation: () => Promise<void>) =>
        operation(),
    },
  });
  vi.stubGlobal("fetch", async (url: string) => {
    if (url === "/api/account") throw new TypeError("Failed to fetch");
    return url === "/api/auth/get-session"
      ? Response.json({ user: { id: "alice" } })
      : Response.json({ saves: [], cursor: null });
  });
  await library.setCloudIdentity("alice", true);
  await cloud.setIdentity("alice");
  await expect(cloud.deleteAccount("alice")).rejects.toThrow("Please retry");
  expect(await library.cloudIdentity()).toEqual({
    accountId: "alice",
    enabled: false,
  });
  expect(status.message).toContain("needs another attempt");
});

it("forgets only cloud metadata after a confirmed account deletion, preserving the local run", async () => {
  const ref = { hash: "f".repeat(64), playthroughId: "default" };
  let confirmed = false;
  vi.stubGlobal("navigator", {
    locks: {
      request: async (_name: string, operation: () => Promise<void>) =>
        operation(),
    },
  });
  vi.stubGlobal("fetch", async (url: string, init?: RequestInit) => {
    if (url === "/api/account") {
      expect(init?.method).toBe("DELETE");
      expect(new Headers(init?.headers).get("X-Zigba-Account")).toBe("alice");
      expect((await library.cloudIdentity()).enabled).toBe(false);
      return Response.json(
        { accountDeleted: confirmed, cloudDeletion: "pending" },
        { status: 202 },
      );
    }
    return url === "/api/auth/get-session"
      ? Response.json({ user: { id: "alice" } })
      : Response.json({ saves: [], cursor: null });
  });
  await library.put({
    hash: ref.hash,
    title: "Kept locally",
    rom: new Uint8Array([3]),
    lastPlayed: 1,
  });
  await cloud.setIdentity("alice");
  await library.saveResume(ref, new Uint8Array([7, 9]), {}, 23n);
  await expect(cloud.deleteAccount("alice")).rejects.toThrow("not confirmed");
  expect(await library.cloudJobs("alice")).toHaveLength(1);
  confirmed = true;
  await cloud.deleteAccount("alice");
  expect(await library.cloudJobs("alice")).toEqual([]);
  expect(status.accountId).toBeNull();
  expect(status.enabled).toBe(false);
  expect(await library.rom(ref.hash)).toEqual(new Uint8Array([3]));
  expect((await library.getResume(ref))?.data).toEqual(new Uint8Array([7, 9]));
});

it("uploads only the selected run with bounded private labels, without sending ROM bytes", async () => {
  const hash = "c".repeat(64);
  const ref = { hash, playthroughId: "15b285c2-f844-451f-85e6-c5bbcc98dd09" };
  vi.stubGlobal("navigator", {
    onLine: true,
    locks: {
      request: async (_name: string, operation: () => Promise<void>) =>
        operation(),
    },
  });
  let uploaded = false;
  vi.stubGlobal(
    "fetch",
    vi.fn(async (url: string, init?: RequestInit) => {
      if (url === "/api/auth/get-session")
        return Response.json({ user: { id: "alice" } });
      if (url === "/api/saves")
        return Response.json({ saves: [], cursor: null });
      expect(url).toBe(`/api/saves/${hash}/${ref.playthroughId}/resume`);
      expect(init?.method).toBe("PUT");
      const headers = new Headers(init?.headers);
      expect(decodeURIComponent(headers.get("X-Zigba-Title")!)).toBe(
        "🌙".repeat(160),
      );
      expect(decodeURIComponent(headers.get("X-Zigba-Playthrough-Name")!)).toBe(
        "Challenge run",
      );
      const decoded = await new Response(
        new Blob([init!.body as BlobPart])
          .stream()
          .pipeThrough(new DecompressionStream("gzip")),
      ).arrayBuffer();
      expect(new Uint8Array(decoded)).toEqual(new Uint8Array([21, 32, 43]));
      uploaded = true;
      return Response.json({
        revision: headers.get("X-Zigba-Revision"),
        etag: '"confirmed"',
        uploadedAt: new Date().toISOString(),
      });
    }),
  );
  await library.put({
    hash,
    title: ` \n${"🌙".repeat(161)}`,
    rom: new Uint8Array([99, 88]),
    lastPlayed: 1,
  });
  await library.setCloudIdentity("alice", true);
  await cloud.setIdentity("alice");
  await library.saveResume(
    { hash, playthroughId: "default" },
    new Uint8Array([5, 6]),
    {},
    2n,
  );
  await library.createPlaythrough(hash, "Challenge run", ref.playthroughId);
  await library.saveResume(ref, new Uint8Array([21, 32, 43]), {}, 8n);
  await cloud.flush(ref);
  expect(uploaded).toBe(true);
  expect(status.message).toBe("Cloud backup confirmed.");
  const jobs = await library.cloudJobs("alice");
  expect(
    jobs.find((job) => job.playthroughId === "default")?.uploadedRevision,
  ).toBeUndefined();
  expect(
    jobs.find((job) => job.playthroughId === ref.playthroughId)?.baseEtag,
  ).toBe('"confirmed"');
});

it("keeps an over-quota run local while continuing backups of existing playthroughs", async () => {
  const hash = "e".repeat(64);
  const existing = { hash, playthroughId: "default" };
  const extra = { hash, playthroughId: "15b285c2-f844-451f-85e6-c5bbcc98dd09" };
  vi.stubGlobal("navigator", {
    onLine: true,
    locks: {
      request: async (_name: string, operation: () => Promise<void>) =>
        operation(),
    },
  });
  vi.stubGlobal("fetch", async (url: string, init?: RequestInit) => {
    if (url === "/api/auth/get-session")
      return Response.json({ user: { id: "alice" } });
    if (url === "/api/saves") return Response.json({ saves: [], cursor: null });
    if (init?.method === "HEAD") return new Response(null, { status: 404 });
    if (url.includes(extra.playthroughId))
      return Response.json(
        { error: { code: "playthrough_limit" } },
        { status: 409 },
      );
    expect(url).toBe(`/api/saves/${hash}/default/resume`);
    expect(new Headers(init?.headers).get("If-Match")).toBe('"existing"');
    return Response.json({
      etag: '"updated"',
      revision: new Headers(init?.headers).get("X-Zigba-Revision"),
      uploadedAt: new Date().toISOString(),
    });
  });
  await library.put({
    hash,
    title: "Game",
    rom: new Uint8Array([1]),
    lastPlayed: 1,
  });
  await cloud.setIdentity("alice");
  await library.adoptCloudBase("alice", existing, "resume", '"existing"');
  await library.saveResume(
    existing,
    new Uint8Array([3, 4]),
    {},
    17n,
    Date.now(),
  );
  await library.createPlaythrough(hash, "Run 101", extra.playthroughId);
  await library.saveResume(extra, new Uint8Array([6, 7]), {}, 23n, Date.now());
  await cloud.flush();
  const jobs = await library.cloudJobs("alice");
  expect(jobs.find((job) => job.playthroughId === "default")?.baseEtag).toBe(
    '"updated"',
  );
  expect(
    jobs.find((job) => job.playthroughId === extra.playthroughId)?.conflict,
  ).not.toBe(true);
  expect((await library.getResume(extra))?.data).toEqual(
    new Uint8Array([6, 7]),
  );
  expect(status.message).toContain("100 cloud playthroughs");
  await expect(cloud.flush(extra)).rejects.toThrow("100 cloud playthroughs");
});
