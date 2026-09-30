// Moves an earlier player's local data into the current database, once.
//
// Earlier players kept everything in the legacy database, at version 8 or
// lower. Opening it at FENCE_VERSION first closes every older tab's
// connection (their code closes on `versionchange`) and makes their later
// opens fail with a VersionError, so no older tab can write to it again, or
// recreate it empty. The fenced database then stays, as an empty marker.
//
// Records move store by store: a batch is written to the current database,
// then deleted from the legacy one. ROMs and other large values move one
// record at a time, so at most one extra copy exists at any moment and the
// storage quota is never asked to hold the library twice. An interrupted move
// resumes on the next load: a record still in the legacy database is written
// again, which is idempotent.
import { openDB, type IDBPDatabase } from "idb";
import { LEGACY, STORAGE_MIGRATION_LOCK } from "./storage-names";

/** Above every version an older player opens, so their opens fail. */
export const FENCE_VERSION = 9;

/** Stores an older player may hold; the current schema has each of them. */
const STORES = [
  "games",
  "playthroughs",
  "saves",
  "resumes",
  "cloud",
  "builds",
  "scenarios",
  "reports",
  "roms",
  "assets",
] as const;
/** Stores whose values can be ROM-sized; these move one record at a time. */
const LARGE = new Set<string>(["roms", "assets", "scenarios", "reports"]);
const BATCH = 256;

export class LegacyStorageBlocked extends Error {
  constructor() {
    super(
      "Close older Evenfall tabs to move saved games to the new storage. Export saves until then.",
    );
    this.name = "LegacyStorageBlocked";
  }
}

/**
 * Moves every record of the legacy database into `target`, which has the
 * current schema, and leaves the legacy database fenced and empty. Returns
 * the number of records moved.
 */
export async function migrateLegacyStorage(
  target: IDBPDatabase,
): Promise<number> {
  const run = () => moveAll(target);
  return typeof navigator !== "undefined" && navigator.locks
    ? navigator.locks.request(STORAGE_MIGRATION_LOCK, run)
    : run();
}

async function moveAll(target: IDBPDatabase) {
  const legacy = await openFenced();
  let moved = 0;
  try {
    for (const name of STORES) {
      if (!legacy.objectStoreNames.contains(name)) continue;
      moved += await moveStore(legacy, target, name);
    }
  } finally {
    legacy.close();
  }
  return moved;
}

function openFenced(): Promise<IDBPDatabase> {
  let rejectBlocked: (error: Error) => void = () => {};
  const blocked = new Promise<never>((_, reject) => {
    rejectBlocked = reject;
  });
  let gaveUp = false;
  const opened = openDB(LEGACY.database, FENCE_VERSION, {
    upgrade(db, oldVersion) {
      // Libraries before version 7 keyed games by SHA-256; those players
      // started them empty, and so does the move.
      if (oldVersion > 0 && oldVersion < 7)
        for (const name of [...db.objectStoreNames]) db.deleteObjectStore(name);
    },
    blocked() {
      gaveUp = true;
      rejectBlocked(new LegacyStorageBlocked());
    },
    blocking() {
      void opened.then((db) => db.close());
    },
  });
  // If an older tab closes after we gave up, release the late connection.
  void opened.then((db) => gaveUp && db.close()).catch(() => {});
  return Promise.race([opened, blocked]);
}

async function moveStore(
  legacy: IDBPDatabase,
  target: IDBPDatabase,
  name: string,
) {
  const limit = LARGE.has(name) ? 1 : BATCH;
  let moved = 0;
  for (;;) {
    const batch: [IDBValidKey, unknown][] = [];
    const read = legacy.transaction(name);
    let cursor = await read.store.openCursor();
    while (cursor) {
      batch.push([cursor.primaryKey, cursor.value]);
      if (batch.length === limit) break;
      cursor = await cursor.continue();
    }
    await read.done;
    if (!batch.length) return moved;
    const write = target.transaction(name, "readwrite");
    await Promise.all([
      ...batch.map(([key, value]) => write.store.put(value, key)),
      write.done,
    ]);
    const remove = legacy.transaction(name, "readwrite");
    await Promise.all([
      ...batch.map(([key]) => remove.store.delete(key)),
      remove.done,
    ]);
    moved += batch.length;
  }
}
