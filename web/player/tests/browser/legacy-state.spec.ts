// States saved by the player's previous core start with ZIGBAST1, which the
// current core cannot load. The player says so, starts from the battery save
// and keeps the old bytes.
import { createHash } from "node:crypto";
import { gzipSync } from "node:zlib";
import { expect, test, type Page } from "@playwright/test";
import type { Emulator } from "@evenfall/gba";
import { advanceAutosave } from "./autosave";

// A loop with an SRAM marker, so the game has a 32 KiB battery save.
const sramRom = {
  name: "Legacy.gba",
  mimeType: "application/octet-stream",
  buffer: Buffer.concat([
    Buffer.from([60, 0, 160, 227, 254, 255, 255, 234]),
    Buffer.from("SRAM_V113\0\0\0"),
  ]),
};
const legacyState = [...Buffer.from("ZIGBAST1"), 1, 2, 3, 4];
const battery = Array.from({ length: 32 * 1024 }, (_, i) => (i * 7) & 255);

type Records = {
  resume?: Record<string, unknown>;
  save?: Record<string, unknown>;
  quick?: Record<string, unknown>;
};

/** Reads or replaces the stored records of the default playthrough. */
function storage(page: Page, hash: string, write?: Records) {
  return page.evaluate(
    async ({ hash, write }) => {
      const request = indexedDB.open("zigba-local");
      const db = await new Promise<IDBDatabase>((resolve, reject) => {
        request.onsuccess = () => resolve(request.result);
        request.onerror = () => reject(request.error);
      });
      const stores = ["resumes", "saves", "assets"];
      const tx = db.transaction(stores, write ? "readwrite" : "readonly");
      const key = [hash, "default"];
      const quickKey = `quick-state:${JSON.stringify([hash, "default", 1])}`;
      const bytes = (value: unknown) =>
        value instanceof Uint8Array ? Array.from(value) : value;
      const plain = (record: Record<string, unknown> | undefined) =>
        record &&
        Object.fromEntries(
          Object.entries(record).map(([name, value]) => [
            name,
            value && typeof value === "object" && "data" in value
              ? { ...value, data: bytes((value as { data: unknown }).data) }
              : bytes(value),
          ]),
        );
      const revive = (record: Record<string, unknown>) => ({
        ...record,
        data: new Uint8Array(record.data as number[]),
      });
      const read = (store: string, id: IDBValidKey) =>
        new Promise<Record<string, unknown> | undefined>((resolve, reject) => {
          const get = tx.objectStore(store).get(id);
          get.onsuccess = () => resolve(get.result);
          get.onerror = () => reject(get.error);
        });
      if (write?.resume)
        tx.objectStore("resumes").put(revive(write.resume), key);
      if (write?.save) tx.objectStore("saves").put(revive(write.save), key);
      if (write?.quick)
        tx.objectStore("assets").put(revive(write.quick), quickKey);
      const result = {
        resume: plain(await read("resumes", key)),
        save: plain(await read("saves", key)),
        quick: plain(await read("assets", quickKey)),
      };
      await new Promise<void>((resolve, reject) => {
        tx.oncomplete = () => resolve();
        tx.onerror = () => reject(tx.error);
      });
      db.close();
      return result;
    },
    { hash, write },
  );
}

const emulator = (page: Page) =>
  page.evaluate(async () => {
    const instance = (window as unknown as { emulator: Emulator }).emulator;
    const cpu = await instance.inspect();
    return {
      hash: cpu.romHash,
      cycle: Number(cpu.cycle),
      battery: Array.from((await instance.exportSave()).subarray(0, 16)),
    };
  });

async function importGame(page: Page) {
  await page.goto("/");
  await page.getByRole("button", { name: "Open player" }).click();
  await page.getByLabel("Open ROM", { exact: true }).setInputFiles(sramRom);
  await expect(
    page.getByRole("heading", { name: "Legacy", exact: true }),
  ).toBeVisible();
  await expect
    .poll(async () => (await emulator(page)).cycle)
    .toBeGreaterThan(0);
  await page.getByLabel(/^Game screen/).focus();
  await page.keyboard.press("Space");
  const { hash } = await emulator(page);
  await expect
    .poll(async () => (await storage(page, hash)).resume)
    .toBeDefined();
  return hash;
}

test("a resume point from the previous core starts from the battery save and is kept", async ({
  page,
}) => {
  await page.clock.install();
  const hash = await importGame(page);
  const { resume } = await storage(page, hash);
  await storage(page, hash, {
    resume: { ...resume, data: legacyState, previous: undefined },
    save: { data: battery, updatedAt: 1 },
  });

  await page.goto("/");
  const resumeButton = page.getByRole("button", { name: "Resume adventure" });
  await expect(resumeButton).toBeEnabled();
  await page.evaluate(() => {
    (window as unknown as { emulator: Emulator }).emulator.play =
      async () => {};
  });
  await resumeButton.click();
  await expect(page.getByLabel(/^Game screen/)).toBeVisible();
  await expect(
    page.getByText(/saved by an older version of Evenfall/),
  ).toBeVisible();
  await expect(
    page.getByRole("dialog", { name: "Resume needs attention" }),
  ).toHaveCount(0);
  expect(await emulator(page)).toMatchObject({
    hash,
    battery: battery.slice(0, 16),
  });

  // Autosave writes a current resume point and keeps the old one aside.
  await page.evaluate(async () => {
    const instance = (window as unknown as { emulator: Emulator }).emulator;
    await instance.runUntil((await instance.inspect()).cycle + 1000n);
  });
  await advanceAutosave(page);
  const stored = await storage(page, hash);
  expect((stored.resume!.data as number[]).slice(0, 8)).toEqual([
    ...Buffer.from("EVENFALL"),
  ]);
  expect((stored.resume!.legacy as { data: number[] }).data).toEqual(
    legacyState,
  );
});

test("a quick slot from the previous core is refused with an explanation and kept", async ({
  page,
}) => {
  const hash = await importGame(page);
  await storage(page, hash, { quick: { data: legacyState, savedAt: 1 } });
  const before = (await emulator(page)).cycle;
  await page.getByLabel(/^Game screen/).focus();
  await page.keyboard.press("Control+Digit1");
  await expect(page.locator(".console-notice")).toContainText(
    "Quick slot 1 was saved by an older version of Evenfall",
  );
  expect((await emulator(page)).cycle).toBeGreaterThanOrEqual(before);
  expect((await storage(page, hash)).quick?.data).toEqual(legacyState);
});

test("a cloud resume point from the previous core restores the cloud battery save", async ({
  page,
}) => {
  const hash = createHash("sha1").update(sramRom.buffer).digest("hex");
  const bodies = {
    resume: gzipSync(Buffer.from(legacyState)),
    battery: gzipSync(Buffer.from(battery)),
  };
  const downloads: string[] = [];
  await page.route("**/api/auth/get-session", (route) =>
    route.fulfill({
      json: { user: { id: "old-player", name: "Old", email: "o@x.test" } },
    }),
  );
  await page.route("**/api/saves", (route) =>
    route.fulfill({
      json: {
        saves: [
          {
            hash,
            playthroughId: "default",
            playthroughName: "Old run",
            title: "Legacy",
            capturedAt: 1,
            uploadedAt: "2026-09-22T18:31:00Z",
          },
        ],
        cursor: null,
      },
    }),
  );
  await page.route("**/api/saves/*/*/quick", (route) =>
    route.fulfill({ json: { slots: [], limit: 10 } }),
  );
  await page.route("**/api/saves/*/*/*", (route) => {
    const kind = new URL(route.request().url()).pathname.split("/").at(-1)!;
    if (route.request().method() === "PUT")
      return route.fulfill({ status: 500 });
    const body = bodies[kind as keyof typeof bodies];
    if (!body) return route.fulfill({ status: 404, json: {} });
    if (route.request().method() === "GET") downloads.push(kind);
    return route.fulfill({
      body,
      headers: {
        ETag: `"${kind}"`,
        "X-Zigba-Captured-At": "1",
        "X-Zigba-Uploaded-At": "2026-09-22T18:31:00Z",
        "X-Zigba-Digest": createHash("sha256").update(body).digest("hex"),
      },
    });
  });
  await page.goto("/");
  await page.getByRole("button", { name: "Resume adventure" }).click();
  const dialog = page.getByRole("dialog", { name: "Resume from cloud" });
  await dialog.getByLabel("Choose ROM for cloud resume").setInputFiles(sramRom);
  await expect(page.getByLabel(/^Game screen/)).toBeVisible();
  await expect(page.locator(".console-notice")).toContainText(
    "Restored the cloud in-game save instead",
  );
  expect(downloads).toEqual(["resume", "battery"]);
  expect(await emulator(page)).toMatchObject({
    hash,
    battery: battery.slice(0, 16),
  });
  expect((await storage(page, hash)).save?.data).toEqual(battery);
});
