import { createHash } from "node:crypto";
import { gunzipSync } from "node:zlib";
import { expect, test, type Page, type Request } from "@playwright/test";
declare global {
  interface Window {
    player: import("../../src/player").Player;
  }
}

const rom = Buffer.from([42, 0, 160, 227, 254, 255, 255, 234]);
const hash = createHash("sha1").update(rom).digest("hex");
const file = {
  name: "Bug source.gba",
  mimeType: "application/octet-stream",
  buffer: rom,
};

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

/** Answers report uploads with `status` in turn, recording each request. */
async function serveReports(page: Page, statuses: number[]) {
  const posts: Request[] = [];
  await page.route("**/api/reports", (route) => {
    posts.push(route.request());
    const status = statuses.shift() ?? 201;
    return route.fulfill(
      status === 201
        ? { status, json: { id: "Report-0123456789abcde" } }
        : { status, json: { error: { code: "failed", message: "Failed" } } },
    );
  });
  return posts;
}

async function fields(request: Request) {
  return new Response(new Uint8Array(request.postDataBuffer()!), {
    headers: { "Content-Type": request.headers()["content-type"] },
  }).formData();
}

async function openGame(page: Page) {
  await page.goto("/");
  await expect(page.getByLabel("Open ROM", { exact: true })).toBeEnabled();
  await page.getByLabel("Open ROM", { exact: true }).setInputFiles(file);
  await expect(page.getByRole("heading", { name: "Bug source" })).toBeVisible();
  await expect
    .poll(() => page.evaluate(() => window.player.state.playing))
    .toBe(true);
}

test("sends an anonymous report with the paused game's state and resumes on close", async ({
  page,
}) => {
  await signedIn(page, null);
  const posts = await serveReports(page, [201]);
  await openGame(page);

  await page.getByRole("button", { name: "Report a bug" }).click();
  const dialog = page.getByRole("dialog", { name: "Report a bug" });
  const send = dialog.getByRole("button", { name: "Send report" });
  await expect(send).toBeDisabled();
  // Signed out, there is no account to include.
  await expect(
    dialog.getByRole("checkbox", { name: /Include my account/ }),
  ).toHaveCount(0);
  await expect(
    dialog.getByRole("checkbox", { name: /Attach this game’s state/ }),
  ).toBeChecked();
  await dialog.getByLabel("What happened?").fill("  The screen froze  ");
  await send.click();
  await expect(dialog.getByRole("status")).toHaveText(
    "Thank you. Your report was sent.",
  );
  await expect(dialog).toContainText("Report-0123456789abcde");

  expect(posts).toHaveLength(1);
  const [post] = posts;
  // Anonymous reports leave the session cookie behind.
  expect(post.headers()["cookie"]).toBeUndefined();
  const form = await fields(post);
  expect(form.get("message")).toBe("The screen froze");
  expect(form.get("rom")).toBe(hash);
  expect(form.get("title")).toBe("Bug source");
  expect(form.get("includeAccount")).toBeNull();
  // The report pauses play, so its state is the machine as it stands now.
  const live = await page.evaluate(async () => ({
    playing: window.player.state.playing,
    state: Array.from(await window.player.emulator.saveState()),
    battery: Array.from(await window.player.emulator.exportSave()),
  }));
  expect(live.playing).toBe(false);
  const state = form.get("state") as File;
  expect(gunzipSync(Buffer.from(await state.arrayBuffer()))).toEqual(
    Buffer.from(live.state),
  );
  const battery = form.get("battery");
  if (live.battery.length)
    expect(Buffer.from(await (battery as File).arrayBuffer())).toEqual(
      Buffer.from(live.battery),
    );
  else expect(battery).toBeNull();

  await dialog.getByRole("button", { name: "Close", exact: true }).click();
  await expect(dialog).toBeHidden();
  await expect
    .poll(() => page.evaluate(() => window.player.state.playing))
    .toBe(true);
});

test("keeps the message after a failure and sends the account only on request", async ({
  page,
}) => {
  await signedIn(page, "test-player");
  const posts = await serveReports(page, [429, 201]);
  await openGame(page);

  await page.getByRole("button", { name: "Report a bug" }).click();
  const dialog = page.getByRole("dialog", { name: "Report a bug" });
  await dialog.getByLabel("What happened?").fill("Sound crackles");
  await dialog
    .getByRole("checkbox", { name: /Attach this game’s state/ })
    .uncheck();
  const account = dialog.getByRole("checkbox", { name: /Include my account/ });
  await expect(account).not.toBeChecked();
  await account.check();
  await dialog.getByRole("button", { name: "Send report" }).click();
  await expect(dialog.getByRole("alert")).toHaveText(
    "Too many reports from this connection. Try again later.",
  );
  await expect(dialog.getByLabel("What happened?")).toHaveValue(
    "Sound crackles",
  );
  await dialog.getByRole("button", { name: "Send report" }).click();
  await expect(dialog.getByRole("status")).toHaveText(
    "Thank you. Your report was sent.",
  );

  expect(posts).toHaveLength(2);
  const form = await fields(posts[1]);
  expect(form.get("message")).toBe("Sound crackles");
  expect(form.get("includeAccount")).toBe("1");
  // Unchecked, the report carries no game at all.
  for (const name of ["rom", "title", "state", "battery"])
    expect(form.get(name)).toBeNull();
});
