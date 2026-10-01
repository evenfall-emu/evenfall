import "fake-indexeddb/auto";
import { describe, it, expect, vi } from "vite-plus/test";
import { openDB, deleteDB } from "idb";
import {
  openLibrary,
  Library,
  quickStateKey,
  playthroughLockKey,
} from "../src/library";
describe("library storage", () => {
  const ref = (hash: string) => ({ hash, playthroughId: "default" });
  it("deduplicates imports and keeps the previous save", async () => {
    await deleteDB("evenfall-local");
    const lib = await openLibrary();
    await lib.put({
      hash: "abc",
      title: "First",
      rom: new Uint8Array([1]),
      lastPlayed: 1,
    });
    await lib.save(ref("abc"), new Uint8Array([42]));
    await lib.put({
      hash: "abc",
      title: "Duplicate",
      rom: new Uint8Array([1]),
      lastPlayed: 2,
    });
    expect(await lib.list()).toEqual([
      {
        hash: "abc",
        title: "First",
        romSize: 1,
        lastPlayed: 2,
        activePlaythroughId: "default",
      },
    ]);
    expect(await lib.db!.getAll("games")).toEqual(await lib.list());
    expect(await lib.rom("abc")).toEqual(new Uint8Array([1]));
    await lib.save(ref("abc"), new Uint8Array([43]));
    expect((await lib.db!.get("saves", ["abc", "default"])).previous).toEqual(
      new Uint8Array([42]),
    );
    expect(await lib.getSave(ref("abc"))).toEqual(new Uint8Array([43]));
    lib.close();
  });
  it("stores ROMs apart from metadata and scenario assets apart from library ROMs", async () => {
    await deleteDB("evenfall-local");
    const lib = await openLibrary();
    const first = new Uint8Array([1, 2, 3]);
    const second = new Uint8Array([4, 5]);
    await lib.put({ hash: "first", title: "First", rom: first, lastPlayed: 1 });
    await lib.put({
      hash: "second",
      title: "Second",
      rom: second,
      lastPlayed: 2,
    });
    await lib.addAsset("first", first);
    expect(
      (await lib.db!.getAll("games")).every(
        (game) => !Object.hasOwn(game, "rom"),
      ),
    ).toBe(true);
    expect(await lib.rom("first")).toEqual(first);
    expect(await lib.asset("second")).toEqual(second);

    // A selection updates recency without writing another ROM-sized value.
    const put = vi.spyOn(IDBObjectStore.prototype, "put");
    await lib.put({
      hash: "first",
      title: "Import title",
      rom: first,
      lastPlayed: 3,
    });
    expect(
      put.mock.contexts.map((store) => (store as IDBObjectStore).name),
    ).toEqual(["games"]);
    put.mockRestore();
    expect((await lib.list())[0]).toEqual({
      hash: "first",
      title: "First",
      romSize: 3,
      lastPlayed: 3,
      activePlaythroughId: "default",
    });
    // A retained asset keeps its own record, independent of the library ROM.
    expect(await lib.db!.get("assets", "first")).toEqual(first);
    lib.close();
  });
  it("starts a legacy library keyed by ROM SHA-256 (version 6 and older) empty", async () => {
    await deleteDB("evenfall-local");
    await deleteDB("zigba-local");
    const old = await openDB("zigba-local", 6, {
      upgrade(db) {
        for (const name of ["games", "roms", "saves", "assets", "scenarios"])
          db.createObjectStore(name);
        db.createObjectStore("cloud").createIndex("hash", "hash");
      },
    });
    const hash = "a".repeat(64);
    await old.put("games", { hash, title: "Old", romSize: 1 }, hash);
    await old.put("roms", new Uint8Array([1]), hash);
    await old.put("saves", { data: new Uint8Array([42]) }, [hash, "default"]);
    await old.put("scenarios", { name: "Old scenario" }, "scenario");
    old.close();
    const lib = await openLibrary();
    expect(lib.db!.version).toBe(2);
    expect(await lib.list()).toEqual([]);
    expect(await lib.rom(hash)).toBeUndefined();
    expect(await lib.getSave(ref(hash))).toBeUndefined();
    expect(await lib.db!.getAll("scenarios")).toEqual([]);
    expect(await lib.playthroughs(hash)).toEqual([]);
    expect(await lib.cloudJobs("alice")).toEqual([]);
    lib.close();
  });
  it("moves a legacy version 7 library, which had no recognized-build cache", async () => {
    await deleteDB("evenfall-local");
    await deleteDB("zigba-local");
    const old = await openDB("zigba-local", 7, {
      upgrade(db) {
        for (const name of ["games", "roms", "saves", "resumes", "assets"])
          db.createObjectStore(name);
        db.createObjectStore("playthroughs").createIndex("hash", "hash");
        const cloud = db.createObjectStore("cloud");
        cloud.createIndex("accountId", "accountId");
        cloud.createIndex("hash", "hash");
      },
    });
    await old.put(
      "games",
      { hash: "abc", title: "Kept", romSize: 1, lastPlayed: 1 },
      "abc",
    );
    await old.put("roms", new Uint8Array([1]), "abc");
    await old.put(
      "playthroughs",
      { hash: "abc", id: "default", name: "Run", createdAt: 1, lastPlayed: 1 },
      ["abc", "default"],
    );
    await old.put("saves", { data: new Uint8Array([42]) }, ["abc", "default"]);
    old.close();
    const lib = await openLibrary();
    expect(lib.db!.version).toBe(2);
    expect((await lib.list()).map((game) => game.title)).toEqual(["Kept"]);
    expect(await lib.rom("abc")).toEqual(new Uint8Array([1]));
    expect(await lib.getSave(ref("abc"))).toEqual(new Uint8Array([42]));
    expect(await lib.builds.get("game:abc:v1")).toBeUndefined();
    await lib.builds.set("game:abc:v1", { review: [] });
    expect(await lib.builds.get("game:abc:v1")).toEqual({ review: [] });
    lib.close();
  });
  it("reports unavailable storage explicitly", async () => {
    const lib = new Library(null, "quota");
    await expect(lib.save(ref("x"), new Uint8Array())).rejects.toMatchObject({
      _tag: "StorageError",
    });
  });
  it("publishes resume bytes with metadata, retains one recovery generation, and keeps them separate from quick slots", async () => {
    await deleteDB("evenfall-local");
    const lib = await openLibrary();
    await lib.put({
      hash: "game",
      title: "Game",
      rom: new Uint8Array([1]),
      lastPlayed: 10,
    });
    await lib.addAsset("quick-state:game:1", new Uint8Array([99]));
    const now = vi.spyOn(Date, "now");
    now.mockReturnValue(100);
    await lib.saveResume(
      ref("game"),
      new Uint8Array([1, 2]),
      { saveType: "sram", rtcEpoch: 42n, sampleRate: 48000 },
      123n,
    );
    now.mockReturnValue(200);
    await lib.saveResume(
      ref("game"),
      new Uint8Array([3, 4]),
      { saveType: "sram", rtcEpoch: 42n, sampleRate: 48000 },
      456n,
    );
    now.mockRestore();
    expect(await lib.getResume(ref("game"))).toEqual({
      data: new Uint8Array([3, 4]),
      options: { saveType: "sram", rtcEpoch: 42n, sampleRate: 48000 },
      cycle: "456",
      savedAt: 200,
      previous: {
        data: new Uint8Array([1, 2]),
        options: { saveType: "sram", rtcEpoch: 42n, sampleRate: 48000 },
        cycle: "123",
        savedAt: 100,
      },
    });
    expect((await lib.list())[0].resumeSavedAt).toBe(200);
    expect(await lib.asset("quick-state:game:1")).toEqual(new Uint8Array([99]));
    lib.close();
  });
  /**
   * Each autosave writes one compressed state. The previous generation is a
   * record of its own, replaced only once it is a minute old, and an earlier
   * player's record with an inline previous generation still reads and moves
   * it out on the next save.
   */
  it("stores the latest state compressed and replaces the previous generation at most once a minute", async () => {
    await deleteDB("evenfall-local");
    const lib = await openLibrary();
    await lib.put({
      hash: "game",
      title: "Game",
      rom: new Uint8Array([1]),
      lastPlayed: 1,
    });
    const state = (marker: number) => {
      const bytes = new Uint8Array(64 * 1024);
      bytes.set(new TextEncoder().encode("EVENFALL"));
      bytes[100] = marker;
      return bytes;
    };
    await lib.db!.put(
      "resumes",
      {
        data: state(1),
        options: {},
        cycle: "1",
        savedAt: 0,
        previous: { data: state(0), options: {}, cycle: "0", savedAt: 0 },
      },
      ["game", "default"],
    );
    expect((await lib.getResume(ref("game")))?.previous?.data).toEqual(
      state(0),
    );
    const now = vi.spyOn(Date, "now");
    const save = async (at: number, marker: number) => {
      now.mockReturnValue(at);
      return (
        await lib.saveResume(ref("game"), state(marker), {}, BigInt(marker))
      ).written;
    };
    // The earlier record moves its latest generation out as the previous one.
    expect((await save(5_000, 2)).previous).toBe(64 * 1024);
    const third = await save(10_000, 3);
    expect(third.state).toBeLessThan(1024);
    expect(third.previous).toBe(0);
    const stored = await lib.db!.get("resumes", ["game", "default"]);
    expect(stored.data).toBeUndefined();
    expect(stored.previous).toBeUndefined();
    let checkpoint = await lib.getResume(ref("game"));
    expect(checkpoint?.data).toEqual(state(3));
    expect(checkpoint?.previous?.data).toEqual(state(1));
    // The previous generation keeps its own save time, 0.
    expect((await save(59_000, 4)).previous).toBe(0);
    // A minute after it, the save being replaced becomes the previous one,
    // already compressed.
    const rotated = (await save(60_000, 5)).previous;
    expect(rotated).toBeGreaterThan(0);
    expect(rotated).toBeLessThan(1024);
    now.mockRestore();
    checkpoint = await lib.getResume(ref("game"));
    expect(checkpoint?.previous?.data).toEqual(state(4));
    await lib.deletePlaythrough(ref("game"));
    expect(await lib.db!.count("resumes")).toBe(0);
    lib.close();
  });
  /**
   * A resume point from the previous core (ZIGBAST1) survives any number of
   * later autosaves in `legacy`, never becomes the recovery generation, and
   * never enters the cloud outbox's upload source.
   */
  it("keeps a previous-core resume point through later saves without uploading it", async () => {
    await deleteDB("evenfall-local");
    const lib = await openLibrary();
    await lib.put({
      hash: "game",
      title: "Game",
      rom: new Uint8Array([1]),
      lastPlayed: 1,
    });
    await lib.setCloudIdentity("alice");
    const old = new Uint8Array([...new TextEncoder().encode("ZIGBAST1"), 7]);
    const current = (marker: number) =>
      new Uint8Array([...new TextEncoder().encode("EVENFALL"), marker]);
    // Earlier players stored raw states, and kept older generations inline.
    await lib.db!.put(
      "resumes",
      { data: old, options: { saveType: "sram" }, cycle: "5", savedAt: 1 },
      ["game", "default"],
    );
    await lib.db!.put(
      "cloud",
      {
        accountId: "alice",
        hash: "game",
        playthroughId: "default",
        kind: "resume",
        revision: "r",
        capturedAt: 1,
        lastInputAt: 1,
      },
      ["alice", "game", "default", "resume"],
    );
    const pending = (await lib.cloudJobs("alice")).find(
      (job) => job.kind === "resume",
    )!;
    expect(await lib.cloudSource(pending)).toBeUndefined();
    for (const marker of [1, 2, 3])
      await lib.saveResume(ref("game"), current(marker), {}, BigInt(marker), 1);
    const checkpoint = await lib.getResume(ref("game"));
    expect(checkpoint?.data).toEqual(current(3));
    // Saves less than a minute apart keep the first previous generation.
    expect(checkpoint?.previous?.data).toEqual(current(1));
    expect(checkpoint?.legacy).toMatchObject({
      data: old,
      options: { saveType: "sram" },
      cycle: "5",
    });
    const job = (await lib.cloudJobs("alice")).find(
      (item) => item.kind === "resume",
    )!;
    expect(await lib.cloudSource(job)).toEqual(current(3));
    lib.close();
  });
  it("does not publish resume metadata or bytes when their transaction fails", async () => {
    await deleteDB("evenfall-local");
    const lib = await openLibrary();
    await lib.put({
      hash: "game",
      title: "Game",
      rom: new Uint8Array([1]),
      lastPlayed: 1,
    });
    await lib.saveResume(ref("game"), new Uint8Array([1]), {}, 1n);
    const original = IDBObjectStore.prototype.put;
    const put = vi
      .spyOn(IDBObjectStore.prototype, "put")
      .mockImplementation(function (this: IDBObjectStore, value, key) {
        if (this.name === "games" && value.resumeSavedAt)
          throw new DOMException("No space", "QuotaExceededError");
        return original.call(this, value, key);
      });
    await expect(
      lib.saveResume(ref("game"), new Uint8Array([2]), {}, 2n),
    ).rejects.toThrow("No space");
    put.mockRestore();
    expect((await lib.getResume(ref("game")))?.data).toEqual(
      new Uint8Array([1]),
    );
    expect((await lib.list())[0].resumeSavedAt).toBeDefined();
    lib.close();
  });
  it("rejects cloud restore persistence after account changes without replacing either local save", async () => {
    await deleteDB("evenfall-local");
    const lib = await openLibrary();
    await lib.put({
      hash: "game",
      title: "Game",
      rom: new Uint8Array([1]),
      lastPlayed: 1,
    });
    await lib.setCloudIdentity("alice", true);
    await lib.save(ref("game"), new Uint8Array([17]), 10);
    await lib.saveResume(ref("game"), new Uint8Array([23]), {}, 42n, 10);
    await lib.setCloudIdentity("bob", true);
    await expect(
      lib.save(ref("game"), new Uint8Array([99]), 20, "alice"),
    ).rejects.toThrow("Account changed");
    await expect(
      lib.saveResume(ref("game"), new Uint8Array([98]), {}, 99n, 20, "alice"),
    ).rejects.toThrow("Account changed");
    expect(await lib.getSave(ref("game"))).toEqual(new Uint8Array([17]));
    expect((await lib.getResume(ref("game")))?.data).toEqual(
      new Uint8Array([23]),
    );
    expect(await lib.cloudJobs("bob")).toEqual([]);
    lib.close();
  });
  it("isolates two runs of one ROM including history, jobs and acknowledgements", async () => {
    await deleteDB("evenfall-local");
    const lib = await openLibrary();
    await lib.put({
      hash: "game",
      title: "Game",
      rom: new Uint8Array([7]),
      lastPlayed: 1,
    });
    const first = { hash: "game", playthroughId: "default" };
    const second = { hash: "game", playthroughId: "second" };
    await lib.setCloudIdentity("alice");
    await lib.save(first, new Uint8Array([11]), 1);
    await lib.saveResume(first, new Uint8Array([12]), {}, 1n, 1);
    await lib.createPlaythrough("game", "Alternate", "second");
    await lib.save(second, new Uint8Array([21]), 2);
    await lib.save(second, new Uint8Array([22]), 3);
    await lib.saveResume(second, new Uint8Array([23]), {}, 2n, 3);
    expect(await lib.getSave(first)).toEqual(new Uint8Array([11]));
    expect(await lib.getSave(second)).toEqual(new Uint8Array([22]));
    expect((await lib.db!.get("saves", ["game", "second"])).previous).toEqual(
      new Uint8Array([21]),
    );
    expect((await lib.getResume(first))?.data).toEqual(new Uint8Array([12]));
    expect((await lib.getResume(second))?.data).toEqual(new Uint8Array([23]));
    expect((await lib.list())[0].resumeSavedAt).toBe(
      (await lib.getResume(second))?.savedAt,
    );
    const jobs = await lib.cloudJobs("alice");
    expect(jobs.map((j) => `${j.playthroughId}:${j.kind}`).sort()).toEqual([
      "default:battery",
      "default:resume",
      "second:battery",
      "second:resume",
    ]);
    const secondJob = jobs.find(
      (j) => j.playthroughId === "second" && j.kind === "battery",
    )!;
    expect(await lib.cloudSource(secondJob)).toEqual(new Uint8Array([22]));
    expect(await lib.beginCloud(secondJob, "digest", true)).toBe(true);
    await lib.acknowledgeCloud(secondJob, secondJob.revision, '"second"', 100);
    expect(
      (await lib.cloudJobs("alice")).find(
        (j) => j.playthroughId === "second" && j.kind === "battery",
      )?.baseEtag,
    ).toBe('"second"');
    expect(
      (await lib.cloudJobs("alice")).find(
        (j) => j.playthroughId === "default" && j.kind === "battery",
      )?.baseEtag,
    ).toBeUndefined();
    await lib.selectPlaythrough(first);
    expect((await lib.list())[0].resumeSavedAt).toBe(
      (await lib.getResume(first))?.savedAt,
    );
    expect(playthroughLockKey(first)).not.toBe(playthroughLockKey(second));
    expect(quickStateKey(first, 1)).not.toBe(quickStateKey(second, 1));
    await expect(
      lib.save({ hash: "game", playthroughId: "missing" }, new Uint8Array([1])),
    ).rejects.toThrow();
    lib.close();
  });

  /**
   * Each autosave writes only the payloads it adds and deletes those its
   * index drops, in the resume point's transaction. An index naming a payload
   * that is neither stored nor supplied stores the state without history.
   */
  it("stores rewind payloads once beside the resume point and prunes dropped ones", async () => {
    await deleteDB("evenfall-local");
    const lib = await openLibrary();
    await lib.put({
      hash: "game",
      title: "Game",
      rom: new Uint8Array([1]),
      lastPlayed: 1,
    });
    const other = { hash: "game", playthroughId: "other" };
    await lib.createPlaythrough("game", "Other", "other");
    const bytes = (n: number) => new Uint8Array([n]);
    const history = (index: number, keys: string[], fresh: string[]) => ({
      index: bytes(index),
      keys,
      payloads: new Map(fresh.map((key) => [key, bytes(key.length)])),
    });
    const put = vi.spyOn(IDBObjectStore.prototype, "put");
    const first = await lib.saveResume(
      ref("game"),
      bytes(1),
      {},
      1n,
      0,
      undefined,
      history(1, ["a", "bb"], ["a", "bb"]),
    );
    expect([...first.rewindKeys]).toEqual(["a", "bb"]);
    await lib.saveResume(
      other,
      bytes(9),
      {},
      1n,
      0,
      undefined,
      history(9, ["a"], ["a"]),
    );
    put.mockClear();
    const second = await lib.saveResume(
      ref("game"),
      bytes(2),
      {},
      2n,
      0,
      undefined,
      history(2, ["bb", "ccc"], ["ccc"]),
    );
    expect([...second.rewindKeys].sort()).toEqual(["bb", "ccc"]);
    // The new payload, not the one already stored.
    expect(
      put.mock.calls.filter(
        (_, i) => (put.mock.contexts[i] as IDBObjectStore).name === "rewind",
      ),
    ).toEqual([[bytes(3), ["game", "default", "ccc"]]]);
    expect(second.written).toMatchObject({
      index: 1,
      payloads: 1,
      payloadCount: 1,
      deleted: 1,
    });
    put.mockRestore();
    const resume = await lib.getResume(ref("game"));
    expect(resume?.rewind).toEqual(bytes(2));
    expect(resume?.previous?.rewind).toBeUndefined();
    expect(await lib.rewindPayloads(ref("game"))).toEqual(
      new Map([
        ["bb", bytes(2)],
        ["ccc", bytes(3)],
      ]),
    );

    const missing = await lib.saveResume(
      ref("game"),
      bytes(3),
      {},
      3n,
      0,
      undefined,
      history(3, ["ccc", "dddd"], []),
    );
    expect(missing.rewindKeys.size).toBe(0);
    expect((await lib.getResume(ref("game")))?.rewind).toBeUndefined();
    expect(await lib.rewindKeys(ref("game"))).toEqual(new Set());

    await lib.saveResume(
      ref("game"),
      bytes(4),
      {},
      4n,
      0,
      undefined,
      history(4, ["a"], ["a"]),
    );
    await lib.deletePlaythrough(ref("game"));
    expect(await lib.rewindKeys(ref("game"))).toEqual(new Set());
    expect(await lib.rewindPayloads(other)).toEqual(new Map([["a", bytes(1)]]));
    lib.close();
  });
  it("adds the rewind store to a version 1 database and keeps its data", async () => {
    await deleteDB("evenfall-local");
    const v1 = await openDB("evenfall-local", 1, {
      upgrade(db) {
        for (const name of [
          "saves",
          "games",
          "roms",
          "resumes",
          "assets",
          "scenarios",
          "reports",
          "builds",
        ])
          db.createObjectStore(name);
        const cloud = db.createObjectStore("cloud");
        cloud.createIndex("accountId", "accountId");
        cloud.createIndex("hash", "hash");
        db.createObjectStore("playthroughs").createIndex("hash", "hash");
      },
    });
    await v1.put("roms", new Uint8Array([5]), "kept");
    v1.close();
    const lib = await openLibrary();
    expect(lib.db!.version).toBe(2);
    expect(lib.db!.objectStoreNames.contains("rewind")).toBe(true);
    expect(await lib.rom("kept")).toEqual(new Uint8Array([5]));
    lib.close();
  });
  it("deletes one local playthrough and its slots without touching another run", async () => {
    await deleteDB("evenfall-local");
    const lib = await openLibrary();
    await lib.put({
      hash: "game",
      title: "Game",
      rom: new Uint8Array([7]),
      lastPlayed: 1,
    });
    const first = ref("game");
    const second = { hash: "game", playthroughId: "second" };
    await lib.setCloudIdentity("alice");
    await lib.save(first, new Uint8Array([11]));
    await lib.saveResume(first, new Uint8Array([12]), {}, 1n);
    await lib.addAsset(quickStateKey(first, 1), new Uint8Array([13]));
    await lib.createPlaythrough("game", "Second", "second");
    await lib.save(second, new Uint8Array([21]));
    await lib.saveResume(second, new Uint8Array([22]), {}, 2n);
    await lib.addAsset(quickStateKey(second, 1), new Uint8Array([23]));

    expect(await lib.deletePlaythrough(second)).toEqual({
      next: expect.objectContaining({ id: "default" }),
      created: false,
    });
    expect(await lib.getPlaythrough(second)).toBeUndefined();
    expect(await lib.getSave(second)).toBeUndefined();
    expect(await lib.getResume(second)).toBeUndefined();
    expect(await lib.asset(quickStateKey(second, 1))).toBeUndefined();
    expect(
      (await lib.cloudJobs("alice")).every(
        (job) => job.playthroughId === "default",
      ),
    ).toBe(true);
    expect((await lib.list())[0]).toMatchObject({
      activePlaythroughId: "default",
      resumeSavedAt: (await lib.getResume(first))!.savedAt,
    });
    expect(await lib.getSave(first)).toEqual(new Uint8Array([11]));
    expect(await lib.asset(quickStateKey(first, 1))).toEqual(
      new Uint8Array([13]),
    );
    lib.close();
  });

  it("replaces the last deleted playthrough with a fresh identity, not its old save", async () => {
    await deleteDB("evenfall-local");
    const lib = await openLibrary();
    await lib.put({
      hash: "game",
      title: "Game",
      rom: new Uint8Array([7]),
      lastPlayed: 1,
    });
    const first = ref("game");
    await lib.saveResume(first, new Uint8Array([12]), {}, 1n);
    const { next, created } = await lib.deletePlaythrough(first);
    expect(created).toBe(true);
    expect(next.id).not.toBe(first.playthroughId);
    expect(next.name).toBe("Playthrough 1");
    expect(await lib.playthroughs("game")).toEqual([next]);
    expect(await lib.getResume(first)).toBeUndefined();
    expect((await lib.list())[0]).toMatchObject({
      activePlaythroughId: next.id,
    });
    expect((await lib.list())[0].resumeSavedAt).toBeUndefined();
    lib.close();
  });

  it("keeps the last playthrough and its save if deletion fails midway", async () => {
    await deleteDB("evenfall-local");
    const lib = await openLibrary();
    await lib.put({
      hash: "game",
      title: "Game",
      rom: new Uint8Array([7]),
      lastPlayed: 1,
    });
    const first = ref("game");
    await lib.saveResume(first, new Uint8Array([12]), {}, 1n);
    await lib.addAsset(quickStateKey(first, 1), new Uint8Array([13]));
    const original = IDBObjectStore.prototype.delete;
    const deletion = vi
      .spyOn(IDBObjectStore.prototype, "delete")
      .mockImplementation(function (this: IDBObjectStore, key) {
        if (this.name === "assets")
          throw new DOMException("Storage failed", "QuotaExceededError");
        return original.call(this, key);
      });
    await expect(lib.deletePlaythrough(first)).rejects.toThrow(
      "Storage failed",
    );
    deletion.mockRestore();
    expect((await lib.playthroughs("game")).map((run) => run.id)).toEqual([
      "default",
    ]);
    expect((await lib.list())[0].activePlaythroughId).toBe("default");
    expect((await lib.getResume(first))?.data).toEqual(new Uint8Array([12]));
    expect(await lib.asset(quickStateKey(first, 1))).toEqual(
      new Uint8Array([13]),
    );
    lib.close();
  });

  it("defaults cloud on for accounts, retains explicit opt-out and forgets one account only", async () => {
    await deleteDB("evenfall-local");
    const lib = await openLibrary();
    const ref = { hash: "orphan", playthroughId: "default" };
    await lib.ensurePlaythrough("orphan");
    expect(await lib.setCloudIdentity("alice")).toEqual({
      accountId: "alice",
      enabled: true,
    });
    await lib.save(ref, new Uint8Array([1]));
    await lib.setCloudIdentity("bob", false);
    await lib.setCloudIdentity("alice");
    await lib.setCloudIdentity("bob", true);
    await lib.save(ref, new Uint8Array([2]));
    await lib.setCloudIdentity("bob", false);
    await lib.setCloudIdentity("alice");
    await lib.forgetCloudAccount("alice");
    expect(await lib.cloudIdentity()).toEqual({
      accountId: null,
      enabled: false,
    });
    expect(await lib.cloudJobs("alice")).toEqual([]);
    expect(await lib.getSave(ref)).toEqual(new Uint8Array([2]));
    expect(await lib.cloudJobs("bob")).toHaveLength(1);
    expect(await lib.setCloudIdentity("alice")).toEqual({
      accountId: "alice",
      enabled: true,
    });
    expect(await lib.setCloudIdentity("bob")).toEqual({
      accountId: "bob",
      enabled: false,
    });
    await lib.forgetCloudAccount("alice");
    expect(await lib.cloudIdentity()).toEqual({
      accountId: "bob",
      enabled: false,
    });
    lib.close();
  });
});
