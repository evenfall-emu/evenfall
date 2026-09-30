import "fake-indexeddb/auto";
import {
  afterEach,
  beforeEach,
  describe,
  expect,
  it,
  vi,
} from "vite-plus/test";
import { deleteDB, openDB, type IDBPDatabase } from "idb";
import { openLibrary, quickStateKey } from "../src/library";
import { FENCE_VERSION, LegacyStorageBlocked } from "../src/storage-migration";

const LEGACY = "zigba-local";
const CURRENT = "evenfall-local";
const ref = { hash: "a".repeat(40), playthroughId: "default" };
const rom = (marker: number) => new Uint8Array([marker, 0, 160, 227]);

/** An earlier player's version 8 database, with a record in every store. */
async function legacyLibrary(roms = 1): Promise<IDBPDatabase> {
  const db = await openDB(LEGACY, 8, {
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
  for (let i = 0; i < roms; i++) {
    const hash = i ? `${i}`.repeat(40) : ref.hash;
    await db.put(
      "games",
      {
        hash,
        title: `Game ${i}`,
        romSize: 4,
        lastPlayed: i,
        activePlaythroughId: "default",
      },
      hash,
    );
    await db.put("roms", rom(i), hash);
    await db.put(
      "playthroughs",
      { hash, id: "default", name: "Run", createdAt: 1, lastPlayed: i },
      [hash, "default"],
    );
  }
  await db.put("saves", { data: new Uint8Array([42]), updatedAt: 1 }, [
    ref.hash,
    "default",
  ]);
  await db.put(
    "resumes",
    { data: new Uint8Array([7]), options: {}, cycle: "5", savedAt: 2 },
    [ref.hash, "default"],
  );
  await db.put(
    "assets",
    { data: new Uint8Array([9]), savedAt: 3 },
    quickStateKey(ref, 1),
  );
  await db.put(
    "cloud",
    {
      accountId: "alice",
      ...ref,
      kind: "resume",
      revision: "r1",
      capturedAt: 2,
      lastInputAt: 0,
    },
    ["alice", ref.hash, "default", "resume"],
  );
  await db.put("builds", { review: [] }, `game:${ref.hash}:v1`);
  return db;
}

async function recordCount(name: string) {
  const db = await openDB(name);
  let count = 0;
  for (const store of db.objectStoreNames) count += await db.count(store);
  db.close();
  return count;
}

beforeEach(async () => {
  await deleteDB(CURRENT);
  await deleteDB(LEGACY);
});
afterEach(() => vi.restoreAllMocks());

describe("legacy storage migration", () => {
  /**
   * An earlier player's library, saves, checkpoint, quick slot, outbox and
   * build cache all appear under the new name, and the legacy database is
   * left fenced above every older player's version, with nothing in it.
   */
  it("moves an earlier player's data once and leaves the old database fenced and empty", async () => {
    (await legacyLibrary()).close();
    const lib = await openLibrary();
    expect(lib.db!.name).toBe(CURRENT);
    expect((await lib.list()).map((game) => game.title)).toEqual(["Game 0"]);
    expect(await lib.rom(ref.hash)).toEqual(rom(0));
    expect(await lib.getSave(ref)).toEqual(new Uint8Array([42]));
    expect((await lib.getResume(ref))?.cycle).toBe("5");
    expect((await lib.getQuickSlot(ref, 1))?.data).toEqual(new Uint8Array([9]));
    expect((await lib.cloudJobs("alice")).map((job) => job.revision)).toEqual([
      "r1",
    ]);
    expect(await lib.playthroughs(ref.hash)).toHaveLength(1);
    expect(await lib.builds.get(`game:${ref.hash}:v1`)).toEqual({ review: [] });
    lib.close();

    const legacy = await openDB(LEGACY);
    expect(legacy.version).toBe(FENCE_VERSION);
    legacy.close();
    expect(await recordCount(LEGACY)).toBe(0);

    // A second load finds nothing to move and keeps everything.
    const again = await openLibrary();
    expect(await again.rom(ref.hash)).toEqual(rom(0));
    again.close();
  });

  /** A browser that never ran an earlier player gets an empty library. */
  it("starts a fresh install empty and fences the legacy name against older players", async () => {
    const lib = await openLibrary();
    expect(await lib.list()).toEqual([]);
    await lib.put({
      hash: "b".repeat(40),
      title: "New",
      rom: rom(1),
      lastPlayed: 1,
    });
    expect((await lib.list()).map((game) => game.title)).toEqual(["New"]);
    lib.close();
    // An older player opening its database now fails instead of starting an
    // empty library of its own next to the current one.
    await expect(openDB(LEGACY, 8)).rejects.toMatchObject({
      name: "VersionError",
    });
    expect(await recordCount(LEGACY)).toBe(0);
  });

  /**
   * An older tab keeps its connection open. Like the earlier player, it closes
   * on `versionchange`; the move then proceeds, and the older tab can neither
   * write through its closed connection nor reopen the database.
   */
  it("closes an older tab's connection before moving, and keeps it from writing again", async () => {
    const oldTab = await legacyLibrary();
    let closed = false;
    oldTab.addEventListener("versionchange", () => {
      oldTab.close();
      closed = true;
    });
    const lib = await openLibrary();
    expect(closed).toBe(true);
    expect(await lib.rom(ref.hash)).toEqual(rom(0));
    expect(() => oldTab.transaction("saves", "readwrite")).toThrow();
    await expect(openDB(LEGACY, 8)).rejects.toMatchObject({
      name: "VersionError",
    });
    lib.close();
  });

  /** An older tab that ignores `versionchange` blocks the move; nothing moves or is lost. */
  it("waits for an older tab that stays open, then moves on the next load", async () => {
    const oldTab = await legacyLibrary();
    await expect(openLibrary()).rejects.toBeInstanceOf(LegacyStorageBlocked);
    expect(await oldTab.get("roms", ref.hash)).toEqual(rom(0));
    oldTab.close();
    await new Promise((resolve) => setTimeout(resolve, 0));
    const lib = await openLibrary();
    expect(await lib.rom(ref.hash)).toEqual(rom(0));
    lib.close();
  });

  /**
   * ROMs move one at a time: a full quota part-way leaves each ROM in exactly
   * one database, never two copies of the library, and the next load finishes.
   */
  it("never holds two copies of the ROMs, and resumes after running out of quota", async () => {
    (await legacyLibrary(3)).close();
    const put = Object.getOwnPropertyDescriptor(
      IDBObjectStore.prototype,
      "put",
    )!.value as IDBObjectStore["put"];
    let written = 0;
    vi.spyOn(IDBObjectStore.prototype, "put").mockImplementation(function (
      this: IDBObjectStore,
      ...args: [unknown, IDBValidKey?]
    ) {
      if (
        this.name === "roms" &&
        this.transaction.db.name === CURRENT &&
        ++written === 2
      )
        throw new DOMException("Storage is full", "QuotaExceededError");
      return put.apply(this, args);
    });
    await expect(openLibrary()).rejects.toMatchObject({
      name: "QuotaExceededError",
    });
    const roms = async (name: string) => {
      const db = await openDB(name);
      const count = db.objectStoreNames.contains("roms")
        ? await db.count("roms")
        : 0;
      db.close();
      return count;
    };
    expect((await roms(CURRENT)) + (await roms(LEGACY))).toBe(3);
    expect(await roms(CURRENT)).toBe(1);

    vi.restoreAllMocks();
    const lib = await openLibrary();
    expect(await lib.list()).toHaveLength(3);
    lib.close();
    expect(await roms(CURRENT)).toBe(3);
    expect(await roms(LEGACY)).toBe(0);
  });
});
