import { createHash } from "node:crypto";
import { expect, test, type Page } from "@playwright/test";

const loop = {
  name: "Loop.gba",
  mimeType: "application/octet-stream",
  buffer: Buffer.from([42, 0, 160, 227, 254, 255, 255, 234]),
};
const hash = createHash("sha1").update(loop.buffer).digest("hex");
type Stored = {
  body: Buffer;
  etag: string;
  revision: string;
  capturedAt: number;
};

/** Signed-in fixture with the Worker's conditional quick-slot semantics. */
async function cloud(page: Page) {
  const slots = new Map<number, Stored>();
  const puts: { slot: number; ifMatch?: string; ifNoneMatch?: string }[] = [];
  let etags = 0;
  await page.route("**/api/auth/get-session", (route) =>
    route.fulfill({
      json: { user: { id: "quick-player", name: "Quick", email: "q@x.test" } },
    }),
  );
  await page.route("**/api/saves", (route) =>
    route.fulfill({ json: { saves: [], cursor: null } }),
  );
  await page.route("**/api/saves/*/*/*", (route) =>
    route.fulfill({ status: 404, json: { error: { code: "save_not_found" } } }),
  );
  await page.route("**/api/saves/*/*/quick", (route) =>
    route.fulfill({
      json: {
        limit: 10,
        slots: [...slots].map(([slot, stored]) => ({
          slot,
          revision: stored.revision,
          capturedAt: stored.capturedAt,
          etag: stored.etag,
          digest: createHash("sha256").update(stored.body).digest("hex"),
          uploadedAt: new Date().toISOString(),
        })),
      },
    }),
  );
  await page.route("**/api/saves/*/*/quick/*", async (route) => {
    const request = route.request();
    expect(request.headers()["x-zigba-account"]).toBe("quick-player");
    const slot = Number(new URL(request.url()).pathname.split("/").at(-1));
    const current = slots.get(slot);
    if (request.method() !== "PUT") {
      if (!current) return route.fulfill({ status: 404, json: {} });
      return route.fulfill({
        body: current.body,
        headers: {
          ETag: current.etag,
          "X-Zigba-Digest": createHash("sha256")
            .update(current.body)
            .digest("hex"),
        },
      });
    }
    const headers = request.headers();
    puts.push({
      slot,
      ifMatch: headers["if-match"],
      ifNoneMatch: headers["if-none-match"],
    });
    if (headers["if-match"] ? current?.etag !== headers["if-match"] : current)
      return route.fulfill({
        status: 412,
        json: { error: { code: "precondition_failed" } },
      });
    const stored = {
      body: request.postDataBuffer()!,
      etag: `"quick-${++etags}"`,
      revision: headers["x-zigba-revision"],
      capturedAt: Number(headers["x-zigba-captured-at"]),
    };
    slots.set(slot, stored);
    return route.fulfill({
      json: {
        etag: stored.etag,
        revision: stored.revision,
        uploadedAt: new Date().toISOString(),
      },
    });
  });
  return {
    slots,
    puts,
    /** Another device replaces a slot; the body stays a valid state. */
    replace(slot: number, capturedAt: number) {
      const current = slots.get(slot)!;
      slots.set(slot, {
        ...current,
        etag: `"quick-${++etags}"`,
        revision: crypto.randomUUID(),
        capturedAt,
      });
      return slots.get(slot)!;
    },
  };
}

async function localSlot(page: Page, slot: number) {
  return page.evaluate(
    async ({ key }) => {
      const request = indexedDB.open("zigba-local");
      const db = await new Promise<IDBDatabase>((resolve) => {
        request.onsuccess = () => resolve(request.result);
      });
      try {
        const read = db.transaction("assets").objectStore("assets").get(key);
        return await new Promise<{
          savedAt: number;
          cloudRevision?: string;
        }>((resolve) => {
          read.onsuccess = () => resolve(read.result);
        });
      } finally {
        db.close();
      }
    },
    { key: `quick-state:${JSON.stringify([hash, "default", slot])}` },
  );
}

test("quick slots back up on save, sync silently, and ask once about true divergence", async ({
  page,
}) => {
  const server = await cloud(page);
  await page.goto("/");
  await page.getByRole("button", { name: "Enter player" }).click();
  await page.getByLabel("Open ROM", { exact: true }).setInputFiles(loop);
  await expect(
    page.getByRole("button", { name: "How to play", exact: true }),
  ).toBeEnabled();
  const canvas = page.getByLabel(/^Game screen/);
  await canvas.focus();

  // Local-first save of slot 10 through Digit0, then an explicit upload.
  await page.keyboard.press("Shift+Digit0");
  await expect(page.locator(".console-notice")).toContainText(
    "Saved slot 10 · backing up",
  );
  await expect.poll(() => server.puts.length).toBe(1);
  expect(server.puts[0]).toMatchObject({ slot: 10, ifNoneMatch: "*" });

  // Another device changes slot 10 while this one saves it too.
  const other = server.replace(10, 1_000);
  await page.keyboard.press("Shift+Digit0");
  await page.evaluate(() => window.dispatchEvent(new Event("online")));
  const dialog = page.getByRole("dialog", { name: "Quick slots changed" });
  await expect(dialog).toContainText("Slot 10");
  // The newest save (this device) is preselected; nothing was overwritten.
  await expect(
    dialog.getByRole("radio", { name: /This device/ }),
  ).toBeChecked();
  expect(server.slots.get(10)?.etag).toBe(other.etag);
  await page.screenshot({ path: "../artifacts/quick-cloud/divergence.png" });
  await dialog.getByRole("button", { name: "Decide later" }).click();
  await expect(dialog).toHaveCount(0);
  // Asked once: reconnecting again keeps the reminder only in the menu.
  await page.evaluate(() => window.dispatchEvent(new Event("online")));
  await page.waitForTimeout(500);
  await expect(dialog).toHaveCount(0);
  await page.getByRole("button", { name: "More options" }).click();
  const menu = page.getByRole("dialog", { name: "Saves and options" });
  await expect(menu.locator(".slot-row")).toHaveCount(10);
  await expect(menu.locator(".slot-row").nth(9)).toContainText("differs");
  await menu.locator(".slot-row").nth(9).scrollIntoViewIfNeeded();
  await page.screenshot({ path: "../artifacts/quick-cloud/menu.png" });
  await page.getByRole("button", { name: /Resolve 1 changed slot/ }).click();
  await expect(dialog).toBeVisible();
  await dialog.getByRole("button", { name: "Keep selected copies" }).click();
  await expect(dialog).toHaveCount(0);
  await expect
    .poll(() => server.puts.at(-1))
    .toMatchObject({ slot: 10, ifMatch: other.etag });
  const kept = server.slots.get(10)!;
  expect(kept.etag).not.toBe(other.etag);

  // A cloud-only change replaces the unchanged local slot without asking.
  const newer = server.replace(10, 2_000);
  const uploads = server.puts.length;
  await page.evaluate(() => window.dispatchEvent(new Event("online")));
  await expect
    .poll(async () => (await localSlot(page, 10)).cloudRevision)
    .toBe(newer.revision);
  expect((await localSlot(page, 10)).savedAt).toBe(2_000);
  await expect(dialog).toHaveCount(0);
  expect(server.puts).toHaveLength(uploads);
  await page.keyboard.press("Control+Digit0");
  await expect(page.locator(".console-notice")).toContainText("Loaded slot 10");
});
