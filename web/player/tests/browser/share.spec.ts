import { createHash } from "node:crypto";
import { gunzipSync, gzipSync } from "node:zlib";
import { expect, test, type Page } from "@playwright/test";
import type { Emulator } from "@evenfall/gba";
declare global {
  interface Window {
    player: import("../../src/player").Player;
  }
}

const rom = Buffer.from([42, 0, 160, 227, 254, 255, 255, 234]);
const hash = createHash("sha1").update(rom).digest("hex");
const file = {
  name: "Share source.gba",
  mimeType: "application/octet-stream",
  buffer: rom,
};
const shareId = "Ab-_0123456789abcdefghij";
const selectedRun = (page: Page) =>
  page.getByRole("combobox", { name: "Playthrough" });

async function signedIn(page: Page, account: string | null) {
  await page.route("**/api/auth/get-session", (route) =>
    route.fulfill({
      json: account
        ? { user: { id: account, name: "Tester", email: "t@example.com" } }
        : null,
    }),
  );
  await page.route("**/api/saves", (route) =>
    route.fulfill({ json: { saves: [], cursor: null } }),
  );
  await page.route("**/api/saves/*/*/quick", (route) =>
    route.fulfill({ json: { slots: [], limit: 10 } }),
  );
  await page.route("**/api/saves/*/*/*", (route) =>
    route.fulfill({ status: route.request().method() === "HEAD" ? 404 : 500 }),
  );
}

/** A state far past boot, so a fresh boot cannot pass for a restored one. */
async function farState(page: Page) {
  await signedIn(page, null);
  await page.goto("/");
  await expect(page.getByLabel("Open ROM", { exact: true })).toBeEnabled();
  return page.evaluate(async (rom) => {
    const emulator = (window as unknown as { emulator: Emulator }).emulator;
    await emulator.load(new Uint8Array(rom), { rtcEpoch: 123n });
    await emulator.runUntil(100_000_000n);
    return {
      data: Array.from(await emulator.saveState()),
      cycle: (await emulator.inspect()).cycle.toString(),
    };
  }, Array.from(rom));
}

async function serveShare(
  page: Page,
  state: number[],
  playthroughName: string,
) {
  const body = gzipSync(Buffer.from(state));
  const reads = { count: 0 };
  await page.route(`**/api/shares/${shareId}`, (route) => {
    reads.count++;
    expect(route.request().headers()["cookie"]).toBeUndefined();
    return route.fulfill({
      body,
      headers: {
        "Content-Type": "application/octet-stream",
        "X-Zigba-Rom": hash,
        "X-Zigba-Title": encodeURIComponent("Shared adventure · 1.0"),
        "X-Zigba-Playthrough-Name": encodeURIComponent(playthroughName),
        "X-Zigba-Digest": createHash("sha256").update(body).digest("hex"),
      },
    });
  });
  return reads;
}

async function runState(page: Page) {
  return page.evaluate(async (hash) => {
    const player = window.player;
    const resume = await player.library.getResume(player.activeRef!);
    return {
      hash: player.activeRef?.hash,
      name: player.state.playthrough?.name,
      names: (await player.library.playthroughs(hash))
        .map((run) => run.name)
        .sort(),
      stored: [resume?.cycle, resume?.previous?.cycle],
      live: (await player.emulator.inspect()).cycle.toString(),
    };
  }, hash);
}

test("Share uploads the exact live state and hands back a copyable link", async ({
  page,
}) => {
  await signedIn(page, "test-player");
  const posts: { headers: Record<string, string>; body: Buffer }[] = [];
  const url = `https://play.example/share/state/${shareId}`;
  await page.route("**/api/shares", (route) => {
    posts.push({
      headers: route.request().headers(),
      body: route.request().postDataBuffer()!,
    });
    return route.fulfill({
      status: 201,
      json: { id: shareId, url },
    });
  });
  await page.context().grantPermissions(["clipboard-read", "clipboard-write"]);
  await page.goto("/");
  await expect(page.getByLabel("Open ROM", { exact: true })).toBeEnabled();
  await page.getByLabel("Open ROM", { exact: true }).setInputFiles(file);
  await expect(
    page.getByRole("heading", { name: "Share source" }),
  ).toBeVisible();
  await expect
    .poll(() => page.evaluate(() => window.player.state.playing))
    .toBe(true);

  await page.getByRole("button", { name: "Share state" }).click();
  const dialog = page.getByRole("dialog", { name: "Share state" });
  await expect(dialog.getByLabel("Share link")).toHaveValue(url);
  await expect(dialog).toContainText("Share source · Playthrough 1");
  await expect(dialog).toContainText("It works for at least 24 hours.");
  expect(posts).toHaveLength(1);
  const [{ headers, body }] = posts;
  expect(headers["x-zigba-account"]).toBe("test-player");
  expect(headers["x-zigba-rom"]).toBe(hash);
  expect(headers["content-type"]).toBe("application/octet-stream");
  expect(decodeURIComponent(headers["x-zigba-playthrough-name"])).toBe(
    "Playthrough 1",
  );
  expect(decodeURIComponent(headers["x-zigba-title"])).toBe("Share source");
  expect(headers["x-zigba-digest"]).toBe(
    createHash("sha256").update(body).digest("hex"),
  );
  // Sharing paused play; the upload is the machine as it stands now.
  const live = await page.evaluate(async () => {
    const player = window.player;
    return {
      playing: player.state.playing,
      state: Array.from(await player.emulator.saveState()),
    };
  });
  expect(live.playing).toBe(false);
  expect(gunzipSync(body)).toEqual(Buffer.from(live.state));

  await dialog.getByRole("button", { name: "Copy link" }).click();
  await expect(dialog.getByRole("button", { name: "Copied" })).toBeVisible();
  expect(await page.evaluate(() => navigator.clipboard.readText())).toBe(url);
  await dialog.getByRole("button", { name: "Close dialog" }).click();
  await expect(dialog).toBeHidden();
  await expect
    .poll(() => page.evaluate(() => window.player.state.playing))
    .toBe(true);
});

test("Share asks a signed-out player to sign in and uploads nothing", async ({
  page,
}) => {
  await signedIn(page, null);
  let posts = 0;
  await page.route("**/api/shares", (route) => {
    posts++;
    return route.fulfill({ status: 500 });
  });
  await page.goto("/");
  await expect(page.getByLabel("Open ROM", { exact: true })).toBeEnabled();
  await page.getByLabel("Open ROM", { exact: true }).setInputFiles(file);
  await expect(
    page.getByRole("heading", { name: "Share source" }),
  ).toBeVisible();
  await page.getByRole("button", { name: "Share state" }).click();
  const dialog = page.getByRole("dialog", { name: "Share state" });
  await expect(dialog).toContainText("Sign in to create share links");
  await dialog.getByRole("button", { name: "Sign in" }).click();
  await expect(page.getByRole("dialog", { name: "Account" })).toBeVisible();
  expect(posts).toBe(0);
});

test("A share link starts at once as a new playthrough when nothing needs asking", async ({
  page,
}) => {
  const state = await farState(page);
  await page.getByLabel("Open ROM", { exact: true }).setInputFiles(file);
  await expect(selectedRun(page)).toHaveText(/Playthrough 1/);
  const reads = await serveShare(page, state.data, "Speedrun 🌙");

  await page.goto(`/share/state/${shareId}`);
  await expect(page).toHaveURL(/\/player$/);
  await expect(selectedRun(page)).toHaveText(/Speedrun 🌙/);
  const run = await runState(page);
  expect(run.hash).toBe(hash);
  expect(run.names).toEqual(["Playthrough 1", "Speedrun 🌙"]);
  expect(run.stored).toContain(state.cycle);
  expect(BigInt(run.live)).toBeGreaterThanOrEqual(BigInt(state.cycle));
  await expect
    .poll(() => page.evaluate(() => window.player.state.playing))
    .toBe(true);
  expect(reads.count).toBe(1);
  // The link was replaced in history, so Back cannot start it again.
  await page.goBack();
  await expect(page).not.toHaveURL(/\/share\//);
  expect(reads.count).toBe(1);
});

test("A share link asks for the ROM, then for a new name once the name is taken", async ({
  page,
}) => {
  const state = await farState(page);
  await serveShare(page, state.data, "Shared run");

  // A fresh device has no ROM: choose it, then start under the shared name.
  await page.goto(`/share/state/${shareId}`);
  let dialog = page.getByRole("dialog", { name: "Shared state" });
  await expect(dialog).toContainText("Shared adventure · 1.0 · Shared run");
  await expect(dialog.getByLabel("Playthrough name")).toHaveCount(0);
  const chooser = page.waitForEvent("filechooser");
  await dialog.getByRole("button", { name: "Choose ROM and start" }).click();
  await (await chooser).setFiles(file);
  await expect(page).toHaveURL(/\/player$/);
  await expect(
    page.getByRole("heading", { name: "Shared adventure · 1.0" }),
  ).toBeVisible();
  let run = await runState(page);
  expect(run.names).toEqual(["Shared run"]);
  expect(run.stored).toContain(state.cycle);

  // The same link again: the ROM is stored, but the name now exists.
  await page.goto(`/share/state/${shareId}`);
  dialog = page.getByRole("dialog", { name: "Shared state" });
  const name = dialog.getByLabel("Playthrough name");
  await expect(name).toHaveValue("Shared run");
  await expect(dialog).toContainText("already have a playthrough");
  await name.fill("  shared RUN ");
  await dialog.getByRole("button", { name: "Start playthrough" }).click();
  await expect(dialog.getByRole("alert")).toHaveText(
    "A playthrough with this name already exists for this game.",
  );
  await expect(page).toHaveURL(/\/share\/state\//);
  await name.fill("Shared run, take 2");
  await dialog.getByRole("button", { name: "Start playthrough" }).click();
  await expect(page).toHaveURL(/\/player$/);
  run = await runState(page);
  expect(run.name).toBe("Shared run, take 2");
  expect(run.names).toEqual(["Shared run", "Shared run, take 2"]);
  expect(run.stored).toContain(state.cycle);
});

test("An expired share link explains itself and returns Home", async ({
  page,
}) => {
  await signedIn(page, null);
  await page.route(`**/api/shares/${shareId}`, (route) =>
    route.fulfill({
      status: 404,
      json: { error: { code: "share_not_found", message: "gone" } },
    }),
  );
  await page.goto(`/share/state/${shareId}`);
  const dialog = page.getByRole("dialog", { name: "Shared state" });
  await expect(dialog.getByRole("alert")).toHaveText(
    "This share link has expired or does not exist.",
  );
  await dialog.getByRole("button", { name: "Go to Home" }).click();
  await expect(page).toHaveURL(/\/$/);
  await expect(dialog).toBeHidden();
});
