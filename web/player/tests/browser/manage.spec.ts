import { expect, test, type Page, type Request } from "@playwright/test";
import type {
  CatalogBase,
  CatalogGameSummary,
  CatalogVersion,
} from "../../src/pages/library/catalog";
import { bps, loop, mockCatalog, romBytes, sha1 } from "./catalog.fixture";

const mapleBase: CatalogBase = {
  id: "maple-us",
  internalName: "maple/us",
  displayName: "Maple (US)",
  sha1: sha1(40),
  crc32: "00000000",
};

function game(
  id: string,
  displayName: string,
  extra: Partial<CatalogGameSummary> = {},
): CatalogGameSummary {
  return {
    id,
    internalName: id,
    displayName,
    summary: `${displayName}, a test catalog entry.`,
    description: null,
    imageUrl: null,
    franchise: "Trees",
    tags: [],
    cloudPlaythroughs: 0,
    versionCount: 1,
    ...extra,
  };
}

function version(
  id: string,
  gameId: string,
  outputSha1: string,
  extra: Partial<CatalogVersion> = {},
): CatalogVersion {
  return {
    id,
    gameId,
    internalName: "1",
    displayName: "1",
    outputSha1,
    requiredBaseId: mapleBase.id,
    requiredBase: mapleBase,
    sourceRepositoryUrl: "https://creator.example/release",
    sourceRevision: null,
    releasedAt: "2026-02-01",
    releaseNotes: null,
    patch: {
      url: `https://example.invalid/${id}.bps`,
      format: "bps",
      size: 1024,
    },
    visibility: "normal",
    saveCompatibleFrom: [],
    ...extra,
  };
}

type Write = { method: string; path: string; body: unknown; request: Request };

/**
 * An in-memory `/api/admin/*` with the server's read shapes. Writes change
 * the store and are recorded in order.
 */
async function mockAdmin(page: Page) {
  const games = [
    game("birch-quest", "Birch Quest", {
      imageUrl: "https://images.invalid/birch.png",
    }),
    game("maple-remix", "Maple Remix", { franchise: "Leaves" }),
    game("rowan-draft", "Rowan Draft", { versionCount: 0 }),
  ];
  const versions = [
    version("birch-quest-1", "birch-quest", sha1(42)),
    version("maple-remix-1", "maple-remix", sha1(41), {
      visibility: "experimental",
      releasedAt: null,
    }),
  ];
  const writes: Write[] = [];
  await page.route("**/api/admin/**", async (route) => {
    const request = route.request();
    const path = new URL(request.url()).pathname.replace(/^\/api\/admin/, "");
    const method = request.method();
    if (method === "GET") {
      if (path === "/games") return route.fulfill({ json: { games } });
      if (path === "/versions") return route.fulfill({ json: { versions } });
      if (path === "/bases")
        return route.fulfill({ json: { bases: [mapleBase] } });
      return route.fulfill({ status: 404, json: {} });
    }
    const json = request.headers()["content-type"]?.includes("json");
    const body = json ? request.postDataJSON() : request.postDataBuffer();
    writes.push({ method, path, body, request });
    const [, kind, id, patch] = path.split("/");
    if (kind === "games") {
      const index = games.findIndex((item) => item.id === id);
      if (method === "DELETE") games.splice(index, 1);
      else if (method === "PUT")
        games.push({ ...game(id, ""), ...body, versionCount: 0 });
      else Object.assign(games[index], body);
    } else if (kind === "versions") {
      const index = versions.findIndex((item) => item.id === id);
      if (patch)
        Object.assign(versions[index], {
          patch: {
            url: "https://patches.invalid/new.bps",
            format: request.headers()["x-patch-format"],
            size: (body as Buffer).length,
          },
        });
      else if (method === "PUT")
        versions.push({ ...body, id, requiredBase: mapleBase });
      else if (method === "DELETE") versions.splice(index, 1);
      else Object.assign(versions[index], body);
    }
    return route.fulfill(
      method === "DELETE" ? { status: 204 } : { json: { ok: true } },
    );
  });
  return writes;
}

/** A signed-in account whose `/api/me` says whether it is an admin. */
async function signedIn(page: Page, admin: boolean) {
  await page.route("**/api/auth/get-session", (route) =>
    route.fulfill({
      json: { user: { id: "manager", name: "M", email: "m@example.com" } },
    }),
  );
  await page.route("**/api/me", (route) =>
    route.fulfill({ json: { id: "manager", admin } }),
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

const gameNames = (page: Page) =>
  page
    .getByRole("table")
    .getByRole("row")
    .locator("td:first-child button")
    .allTextContents();

test.beforeEach(async ({ page }) => {
  await page.setViewportSize({ width: 1440, height: 900 });
  await mockCatalog(page);
});

test("Manage is only offered to admin accounts", async ({ page }) => {
  const writes = await mockAdmin(page);
  // Not on its own page either: only an admin account sees the tab.
  const tab = page.getByRole("button", { name: "Manage", exact: true });
  let adminReads = 0;
  page.on("request", (request) => {
    if (request.url().includes("/api/admin/")) adminReads++;
  });
  await page.route("**/api/me", (route) =>
    route.fulfill({ status: 401, json: {} }),
  );
  await page.goto("/manage");
  await expect(
    page.getByRole("heading", { name: "Sign in to manage the catalog" }),
  ).toBeVisible();
  await expect(tab).toHaveCount(0);

  await page.unroute("**/api/me");
  await signedIn(page, false);
  await page.goto("/library/maple-remix");
  await expect(
    page.getByRole("heading", { level: 2, name: "Maple Remix" }),
  ).toBeVisible();
  await expect(page.getByRole("button", { name: "Manage" })).toHaveCount(0);
  await expect(
    page.getByRole("button", { name: /^Edit .* in Manage$/ }),
  ).toHaveCount(0);
  await page.goto("/manage");
  await expect(
    page.getByRole("heading", {
      name: "This account cannot manage the catalog",
    }),
  ).toBeVisible();
  await expect(tab).toHaveCount(0);
  // The phone row, where Manage is an icon, too.
  await page.setViewportSize({ width: 390, height: 844 });
  await expect(page.getByRole("button", { name: "Account" })).toBeVisible();
  await expect(tab).toHaveCount(0);
  expect(adminReads).toBe(0);
  expect(writes).toEqual([]);
});

test("An admin finds gaps and edits a game, saving only what changed", async ({
  page,
}) => {
  await signedIn(page, true);
  const writes = await mockAdmin(page);
  let publicReads = 0;
  await page.route("**/api/games", (route) => {
    publicReads++;
    return route.fallback();
  });

  // The Library's Edit opens the same game in Manage.
  await page.goto("/library/maple-remix");
  await page
    .getByRole("button", { name: "Edit Maple Remix in Manage" })
    .click();
  await expect(page).toHaveURL(/\/manage\/maple-remix$/);
  await expect(
    page.getByRole("button", { name: "Manage", exact: true }),
  ).toHaveAttribute("aria-current", "page");
  await page.getByRole("button", { name: "Close game" }).click();
  await expect(page).toHaveURL(/\/manage$/);

  // Hidden, patchless and release-less games are all listed.
  expect(await gameNames(page)).toEqual([
    "Birch Quest",
    "Maple Remix",
    "Rowan Draft",
  ]);
  const filters = page.getByRole("group", { name: "Show only" });
  await filters.getByRole("button", { name: "No releases 1" }).click();
  expect(await gameNames(page)).toEqual(["Rowan Draft"]);
  await filters.getByRole("button", { name: "No releases 1" }).click();
  await filters.getByRole("button", { name: "No cover image 2" }).click();
  await page.getByRole("searchbox", { name: "Search games" }).fill("maple");
  expect(await gameNames(page)).toEqual(["Maple Remix"]);

  await page.getByRole("button", { name: "Maple Remix" }).click();
  await expect(page).toHaveURL(/\/manage\/maple-remix$/);
  const panel = page.getByRole("complementary", { name: "Game" });
  const save = panel.getByRole("button", { name: "Save changes" });
  await expect(save).toBeDisabled();
  await panel.getByLabel("Summary").fill("Remixed leaves, all year.");
  await panel.getByLabel("Cover image URL").fill("not a url");
  await panel.getByLabel("Tags").fill("Seasonal");
  await panel.getByLabel("Tags").press("Enter");
  await expect(panel.getByText("Unsaved changes")).toBeVisible();
  await panel.getByLabel("Cover image URL").fill("");
  const reads = publicReads;
  await save.click();
  await expect(save).toBeDisabled();
  expect(writes.map(({ method, path, body }) => [method, path, body])).toEqual([
    [
      "PATCH",
      "/games/maple-remix",
      { summary: "Remixed leaves, all year.", tags: ["Seasonal"] },
    ],
  ]);
  expect(writes[0].request.headers()["content-type"]).toBe("application/json");
  // The saved values come back, and the Library's catalog is reloaded.
  await expect(panel.getByLabel("Summary")).toHaveValue(
    "Remixed leaves, all year.",
  );
  expect(publicReads).toBeGreaterThan(reads);
});

test("A release is added only after its patch builds a new ROM here", async ({
  page,
}) => {
  await signedIn(page, true);
  const writes = await mockAdmin(page);
  // Store Maple's base ROM (marker 40) in the Library.
  await page.goto("/");
  await page.getByRole("button", { name: "Open player" }).click();
  await expect(page.getByLabel("Open ROM", { exact: true })).toBeEnabled();
  await page
    .getByLabel("Open ROM", { exact: true })
    .setInputFiles(loop(40, "Maple base"));
  await expect(
    page.getByRole("heading", { name: "Maple base", exact: true }),
  ).toBeVisible();

  await page.getByRole("button", { name: "Manage", exact: true }).click();
  await page.getByRole("button", { name: "Maple Remix" }).click();
  const panel = page.getByRole("complementary", { name: "Game" });
  await panel.getByRole("tab", { name: /Releases/ }).click();
  await panel.getByRole("button", { name: "Add release" }).click();
  const dialog = page.getByRole("dialog", { name: "Add release" });
  const checks = dialog.getByRole("list", { name: "Patch checks" });
  await expect(checks).toContainText("Maple (US) · in your Library");
  const cont = dialog.getByRole("button", { name: "Continue" });

  const choosePatch = async (name: string, buffer: Buffer) => {
    const picker = page.waitForEvent("filechooser");
    await dialog
      .getByRole("button", { name: /^(Choose patch|Replace)…$/ })
      .click();
    await (
      await picker
    ).setFiles({
      name,
      mimeType: "application/octet-stream",
      buffer,
    });
  };
  // A patch for another base fails; one that builds an existing release's
  // ROM (marker 41 is Maple Remix 1) is refused.
  await choosePatch("wrong.bps", bps(romBytes(47), romBytes(46)));
  await expect(checks).toContainText("This patch is for a different base ROM.");
  await expect(cont).toBeDisabled();
  await choosePatch("same.bps", bps(romBytes(40), romBytes(41)));
  await expect(checks).toContainText(
    "Already the output of 1 (maple-remix-1).",
  );
  await expect(cont).toBeDisabled();

  const patch = bps(romBytes(40), romBytes(46));
  await choosePatch("maple-2.bps", patch);
  await expect(checks).toContainText(
    `SHA-1 ${sha1(46).slice(0, 8)}…${sha1(46).slice(-6)}`,
  );
  await cont.click();
  await dialog.getByLabel("Label", { exact: true }).fill("2 Beta");
  await expect(dialog.getByLabel("Release ID")).toHaveValue(
    "maple-remix-2-beta",
  );
  await dialog.getByLabel("Release date").fill("2026-09-01");
  await dialog.getByRole("button", { name: "Add release" }).click();
  await expect(dialog).toBeHidden();

  expect(writes.map(({ method, path }) => `${method} ${path}`)).toEqual([
    "PUT /versions/maple-remix-2-beta",
    "PUT /versions/maple-remix-2-beta/patch",
  ]);
  expect(writes[0].body).toEqual({
    gameId: "maple-remix",
    displayName: "2 Beta",
    internalName: "2 Beta",
    releasedAt: "2026-09-01",
    sourceRepositoryUrl: null,
    sourceRevision: null,
    releaseNotes: null,
    saveCompatibleFrom: [],
    outputSha1: sha1(46),
    requiredBaseId: "maple-us",
    patch: null,
    visibility: "experimental",
  });
  expect(writes[1].request.headers()["x-patch-format"]).toBe("bps");
  expect(writes[1].body).toEqual(patch);
  const releases = panel.getByRole("list", { name: "Releases" });
  await expect(releases.getByRole("listitem")).toHaveCount(2);
  await expect(releases.getByRole("listitem").first()).toContainText("2 Beta");
});
