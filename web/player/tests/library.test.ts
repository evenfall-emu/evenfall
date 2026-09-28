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
    await deleteDB("zigba-local");
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
    await deleteDB("zigba-local");
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
  it("starts a library keyed by ROM SHA-256 (version 6 and older) empty", async () => {
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
    expect(lib.db!.version).toBe(8);
    expect(await lib.list()).toEqual([]);
    expect(await lib.rom(hash)).toBeUndefined();
    expect(await lib.getSave(ref(hash))).toBeUndefined();
    expect(await lib.db!.getAll("scenarios")).toEqual([]);
    expect(await lib.playthroughs(hash)).toEqual([]);
    expect(await lib.cloudJobs("alice")).toEqual([]);
    lib.close();
  });
  it("keeps a version 7 library and adds the recognized-build cache", async () => {
    await deleteDB("zigba-local");
    const first = await openLibrary();
    await first.put({
      hash: "abc",
      title: "Kept",
      rom: new Uint8Array([1]),
      lastPlayed: 1,
    });
    await first.save(ref("abc"), new Uint8Array([42]));
    first.close();
    // Downgrade the schema to version 7: same stores, no build cache.
    const raw = await openDB("zigba-local");
    const stores = new Map<string, Map<IDBValidKey, unknown>>();
    for (const name of raw.objectStoreNames) {
      const entries = new Map<IDBValidKey, unknown>();
      for (const key of await raw.getAllKeys(name))
        entries.set(key, await raw.get(name, key));
      stores.set(name, entries);
    }
    raw.close();
    await deleteDB("zigba-local");
    const old = await openDB("zigba-local", 7, {
      upgrade(db) {
        for (const name of stores.keys()) {
          if (name === "builds") continue;
          const store = db.createObjectStore(name);
          if (name === "cloud") {
            store.createIndex("accountId", "accountId");
            store.createIndex("hash", "hash");
          }
          if (name === "playthroughs") store.createIndex("hash", "hash");
        }
      },
    });
    for (const [name, entries] of stores)
      if (name !== "builds")
        for (const [key, value] of entries) await old.put(name, value, key);
    old.close();
    const lib = await openLibrary();
    expect(lib.db!.version).toBe(8);
    expect((await lib.list()).map((game) => game.title)).toEqual(["Kept"]);
    expect(await lib.rom("abc")).toEqual(new Uint8Array([1]));
    expect(await lib.getSave(ref("abc"))).toEqual(new Uint8Array([42]));
    expect(await lib.builds.get("game:abc:v1")).toBeUndefined();
    await lib.builds.set("game:abc:v1", { review: [] });
    expect(await lib.builds.get("game:abc:v1")).toEqual({ review: [] });
    await lib.builds.delete("game:abc:v1");
    expect(await lib.builds.get("game:abc:v1")).toBeUndefined();
    lib.close();
  });
  it("reports unavailable storage explicitly", async () => {
    const lib = new Library(null, "quota");
    await expect(lib.save(ref("x"), new Uint8Array())).rejects.toMatchObject({
      _tag: "StorageError",
    });
  });
  it("publishes resume bytes with metadata, retains one recovery generation, and keeps them separate from quick slots", async () => {
    await deleteDB("zigba-local");
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
  it("does not publish resume metadata or bytes when their transaction fails", async () => {
    await deleteDB("zigba-local");
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
    await deleteDB("zigba-local");
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
    await deleteDB("zigba-local");
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

  it("deletes one local playthrough and its slots without touching another run", async () => {
    await deleteDB("zigba-local");
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
    await deleteDB("zigba-local");
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
    await deleteDB("zigba-local");
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
    await deleteDB("zigba-local");
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
