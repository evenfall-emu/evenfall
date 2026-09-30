// An earlier player kept its library in the legacy IndexedDB database. The
// first load of this version moves it, while a tab of the earlier player is
// still open on the same origin.
import { expect, test } from "@playwright/test";

const hash = "c".repeat(40);

test("moves an earlier player's library while its tab is open, and fences that tab", async ({
  context,
}) => {
  const oldTab = await context.newPage();
  // Any same-origin document serves as the earlier player's tab.
  await oldTab.goto("/manifest.webmanifest");
  await oldTab.evaluate(async (hash) => {
    const db = await new Promise<IDBDatabase>((resolve, reject) => {
      const request = indexedDB.open("zigba-local", 8);
      request.onupgradeneeded = () => {
        const db = request.result;
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
      };
      request.onsuccess = () => resolve(request.result);
      request.onerror = () => reject(request.error);
    });
    const tx = db.transaction(["games", "roms", "playthroughs"], "readwrite");
    tx.objectStore("games").put(
      {
        hash,
        title: "Carried over",
        romSize: 8,
        lastPlayed: 1,
        activePlaythroughId: "default",
      },
      hash,
    );
    tx.objectStore("roms").put(
      new Uint8Array([42, 0, 160, 227, 254, 255, 255, 234]),
      hash,
    );
    tx.objectStore("playthroughs").put(
      { hash, id: "default", name: "Old run", createdAt: 1, lastPlayed: 1 },
      [hash, "default"],
    );
    await new Promise((resolve) => (tx.oncomplete = resolve));
    // The earlier player closes its connection on versionchange.
    const host = window as unknown as { legacyClosed: boolean };
    host.legacyClosed = false;
    db.onversionchange = () => {
      db.close();
      host.legacyClosed = true;
    };
  }, hash);

  const page = await context.newPage();
  await page.goto("/library");
  await expect(page.getByRole("main", { name: "Library" })).toContainText(
    "Carried over",
  );
  expect(
    await oldTab.evaluate(
      () => (window as unknown as { legacyClosed: boolean }).legacyClosed,
    ),
  ).toBe(true);
  // The earlier player can't reopen its database, so it can't write to it.
  expect(
    await oldTab.evaluate(
      () =>
        new Promise<string>((resolve) => {
          const request = indexedDB.open("zigba-local", 8);
          request.onsuccess = () => resolve("opened");
          request.onerror = () => resolve(request.error!.name);
        }),
    ),
  ).toBe("VersionError");
  // Nothing is left behind: the ROM exists once, in the new database.
  const counts = await page.evaluate(async () => {
    const count = (name: string) =>
      new Promise<number>((resolve, reject) => {
        const request = indexedDB.open(name);
        request.onsuccess = () => {
          const db = request.result;
          if (!db.objectStoreNames.contains("roms")) return resolve(0);
          const get = db.transaction("roms").objectStore("roms").count();
          get.onsuccess = () => {
            db.close();
            resolve(get.result);
          };
        };
        request.onerror = () => reject(request.error);
      });
    return {
      legacy: await count("zigba-local"),
      current: await count("evenfall-local"),
    };
  });
  expect(counts).toEqual({ legacy: 0, current: 1 });
});
