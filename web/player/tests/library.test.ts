import "fake-indexeddb/auto";
import { describe, it, expect, vi } from "vitest";
import { openDB, deleteDB } from "idb";
import {
  openLibrary,
  Library,
  quickStateKey,
  playthroughLockKey,
  type CloudJob,
} from "../src/library";
describe("library storage", () => {
  const ref = (hash: string) => ({ hash, playthroughId: "default" });
  it("upgrades v1, deduplicates imports, removes entries without deleting saves", async () => {
    await deleteDB("zigba-local");
    const old = await openDB("zigba-local", 1, {
      upgrade(db) {
        db.createObjectStore("saves");
      },
    });
    await old.put("saves", { data: new Uint8Array([42]), updatedAt: 1 }, "abc");
    old.close();
    const lib = await openLibrary();
    expect(await lib.getSave(ref("abc"))).toEqual(new Uint8Array([42]));
    await lib.put({
      hash: "abc",
      title: "First",
      rom: new Uint8Array([1]),
      lastPlayed: 1,
    });
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
    await lib.rename("abc", "Renamed");
    expect((await lib.list())[0].title).toBe("Renamed");
    await lib.save(ref("abc"), new Uint8Array([43]));
    expect((await lib.db!.get("saves", ["abc", "default"])).previous).toEqual(
      new Uint8Array([42]),
    );
    await lib.remove("abc");
    expect(await lib.list()).toHaveLength(0);
    expect(await lib.rom("abc")).toBeUndefined();
    expect(await lib.getSave(ref("abc"))).toEqual(new Uint8Array([43]));
    lib.close();
  });
  it("splits v2 ROMs from metadata without losing games, saves or scenario assets", async () => {
    await deleteDB("zigba-local");
    const old = await openDB("zigba-local", 2, {
      upgrade(db) {
        for (const name of ["games", "saves", "assets", "scenarios", "reports"])
          db.createObjectStore(name);
      },
    });
    const first = new Uint8Array([1, 2, 3]);
    const second = new Uint8Array([4, 5]);
    await old.put(
      "games",
      { hash: "first", title: "Renamed", rom: first, lastPlayed: 1 },
      "first",
    );
    await old.put(
      "games",
      { hash: "second", title: "Second", rom: second, lastPlayed: 2 },
      "second",
    );
    await old.put(
      "saves",
      { data: new Uint8Array([42]), previous: new Uint8Array([41]) },
      "first",
    );
    await old.put("assets", first, "first");
    await old.put("scenarios", { name: "Retained scenario" }, "scenario");
    await old.put("reports", { note: "Retained report" }, "report");
    old.close();
    const lib = await openLibrary();
    expect(lib.db!.version).toBe(6);
    expect(await lib.list()).toEqual([
      {
        hash: "second",
        title: "Second",
        romSize: 2,
        lastPlayed: 2,
        activePlaythroughId: "default",
      },
      {
        hash: "first",
        title: "Renamed",
        romSize: 3,
        lastPlayed: 1,
        activePlaythroughId: "default",
      },
    ]);
    expect(
      (await lib.db!.getAll("games")).every(
        (game) => !Object.hasOwn(game, "rom"),
      ),
    ).toBe(true);
    expect(await lib.rom("first")).toEqual(first);
    expect(await lib.rom("second")).toEqual(second);
    expect(await lib.asset("second")).toEqual(second);
    expect(await lib.db!.get("saves", ["first", "default"])).toEqual({
      data: new Uint8Array([42]),
      previous: new Uint8Array([41]),
    });
    expect(await lib.scenarios()).toEqual([{ name: "Retained scenario" }]);
    expect(await lib.reports()).toEqual([{ note: "Retained report" }]);

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
      title: "Renamed",
      romSize: 3,
      lastPlayed: 3,
      activePlaythroughId: "default",
    });
    await lib.remove("first");
    expect(await lib.rom("first")).toBeUndefined();
    // An explicitly retained scenario asset is independent of the library ROM.
    expect(await lib.asset("first")).toEqual(first);
    expect(await lib.getSave(ref("first"))).toEqual(new Uint8Array([42]));
    lib.close();
  });
  it("keeps the old library intact when a ROM migration write fails", async () => {
    await deleteDB("zigba-local");
    const old = await openDB("zigba-local", 2, {
      upgrade(db) {
        db.createObjectStore("games");
      },
    });
    const game = {
      hash: "abc",
      title: "Old",
      rom: new Uint8Array([1]),
      lastPlayed: 1,
    };
    await old.put("games", game, "abc");
    const later = { ...game, hash: "xyz", rom: new Uint8Array([2]) };
    await old.put("games", later, "xyz");
    old.close();
    const original = IDBObjectStore.prototype.put;
    const put = vi
      .spyOn(IDBObjectStore.prototype, "put")
      .mockImplementation(function (this: IDBObjectStore, value, key) {
        if (this.name === "roms" && key === "xyz")
          throw new DOMException("No space", "QuotaExceededError");
        return original.call(this, value, key);
      });
    try {
      await expect(openLibrary()).rejects.toThrow();
    } finally {
      put.mockRestore();
    }
    const intact = await openDB("zigba-local", 2);
    expect(await intact.getAll("games")).toEqual([game, later]);
    expect(intact.objectStoreNames.contains("roms")).toBe(false);
    intact.close();
    const retried = await openLibrary();
    expect(await retried.rom("abc")).toEqual(game.rom);
    expect(await retried.rom("xyz")).toEqual(later.rom);
    retried.close();
  });
  it("upgrades v5 saves, orphan progress, quick slots and in-flight cloud metadata atomically", async () => {
    await deleteDB("zigba-local");
    const old = await openDB("zigba-local", 5, {
      upgrade(db) {
        for (const name of [
          "games",
          "roms",
          "saves",
          "resumes",
          "assets",
          "scenarios",
          "reports",
        ])
          db.createObjectStore(name);
        const cloud = db.createObjectStore("cloud");
        cloud.createIndex("accountId", "accountId");
        cloud.createIndex("hash", "hash");
      },
    });
    await old.put(
      "games",
      {
        hash: "game",
        title: "Game",
        romSize: 1,
        lastPlayed: 9,
        resumeSavedAt: 5,
      },
      "game",
    );
    await old.put("roms", new Uint8Array([7]), "game");
    await old.put(
      "saves",
      {
        data: new Uint8Array([8]),
        previous: new Uint8Array([6]),
        updatedAt: 4,
        cloudRevision: "battery-rev",
      },
      "game",
    );
    await old.put(
      "resumes",
      {
        data: new Uint8Array([9]),
        options: {},
        cycle: "12",
        savedAt: 5,
        cloudRevision: "resume-rev",
        previous: {
          data: new Uint8Array([7]),
          options: {},
          cycle: "11",
          savedAt: 3,
        },
      },
      "game",
    );
    await old.put(
      "saves",
      { data: new Uint8Array([44]), updatedAt: 1 },
      "orphan",
    );
    await old.put(
      "resumes",
      { data: new Uint8Array([45]), options: {}, cycle: "13", savedAt: 6 },
      "orphan",
    );
    await old.put("assets", new Uint8Array([31]), "quick-state:game:2");
    await old.put("cloud", { accountId: "alice", enabled: true }, "identity");
    await old.put("cloud", false, "enabled:bob");
    const job: Omit<CloudJob, "playthroughId"> = {
      accountId: "alice",
      hash: "game",
      kind: "resume",
      revision: "resume-rev",
      capturedAt: 5,
      lastInputAt: 4,
      baseEtag: '"old"',
      flight: { revision: "resume-rev", digest: "digest" },
      attempts: 3,
      retryAt: 11,
    };
    await old.put("cloud", job, ["alice", "game", "resume"]);
    old.close();
    const fail = vi.spyOn(IDBObjectStore.prototype, "put");
    const original = fail.getMockImplementation()!;
    fail.mockImplementation(function (this: IDBObjectStore, value, key) {
      if (this.name === "assets" && String(key).startsWith("quick-state:["))
        throw new DOMException("No space", "QuotaExceededError");
      return original.call(this, value, key);
    });
    try {
      await expect(openLibrary()).rejects.toThrow();
    } finally {
      fail.mockRestore();
    }
    const intact = await openDB("zigba-local", 5);
    expect(await intact.get("saves", "game")).toMatchObject({
      cloudRevision: "battery-rev",
    });
    expect(await intact.get("cloud", ["alice", "game", "resume"])).toEqual(job);
    expect(await intact.get("assets", "quick-state:game:2")).toEqual(
      new Uint8Array([31]),
    );
    intact.close();
    const lib = await openLibrary();
    expect(await lib.getSave(ref("game"))).toEqual(new Uint8Array([8]));
    expect(await lib.getSave(ref("orphan"))).toEqual(new Uint8Array([44]));
    expect((await lib.getPlaythrough(ref("orphan")))?.resumeSavedAt).toBe(6);
    expect((await lib.getResume(ref("game")))?.previous?.data).toEqual(
      new Uint8Array([7]),
    );
    expect(await lib.asset(quickStateKey(ref("game"), 2))).toEqual(
      new Uint8Array([31]),
    );
    expect(await lib.asset("quick-state:game:2")).toBeUndefined();
    expect(await lib.rom("game")).toEqual(new Uint8Array([7]));
    expect(await lib.cloudJobs("alice")).toEqual([
      { ...job, playthroughId: "default" },
    ]);
    expect(await lib.cloudSource({ ...job, playthroughId: "default" })).toEqual(
      new Uint8Array([9]),
    );
    expect(await lib.setCloudIdentity("bob")).toEqual({
      accountId: "bob",
      enabled: false,
    });
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
    await lib.remove("game");
    expect(await lib.getResume(ref("game"))).toBeDefined();
    expect(await lib.asset("quick-state:game:1")).toEqual(new Uint8Array([99]));
    await lib.put({
      hash: "game",
      title: "Game",
      rom: new Uint8Array([1]),
      lastPlayed: 11,
    });
    await lib.deleteSave(ref("game"));
    expect(await lib.getResume(ref("game"))).toBeUndefined();
    expect((await lib.list())[0].resumeSavedAt).toBeUndefined();
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
  it("isolates two runs of one ROM including history, jobs, acknowledgements and deletion", async () => {
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
    await lib.deleteSave(second);
    expect(await lib.getSave(second)).toBeUndefined();
    expect(await lib.getSave(first)).toEqual(new Uint8Array([11]));
    expect((await lib.cloudJobs("alice")).map((j) => j.playthroughId)).toEqual([
      "default",
      "default",
    ]);
    expect((await lib.list())[0].resumeSavedAt).toBeUndefined();
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
