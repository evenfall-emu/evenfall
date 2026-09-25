import { expect, test } from "@playwright/test";

test("email sign-in stays optional and reports a generic delivery result", async ({
  page,
}) => {
  await page.route("**/api/auth/get-session", (route) =>
    route.fulfill({ json: null }),
  );
  await page.route("**/api/auth/sign-in/magic-link", async (route) => {
    expect(route.request().postDataJSON()).toEqual({
      email: "player@example.com",
      callbackURL: "/?account=1",
      errorCallbackURL: "/?account=1",
    });
    await route.fulfill({ json: { status: true } });
  });
  await page.goto("/");
  await page.getByRole("button", { name: "Account", exact: true }).click();
  const dialog = page.getByRole("dialog", { name: "Account", exact: true });
  await expect(
    dialog.getByRole("button", { name: "Continue with Discord" }),
  ).toBeEnabled();
  await dialog.getByLabel("Email address").fill("player@example.com");
  await dialog.getByRole("button", { name: "Email me a sign-in link" }).click();
  await expect(dialog.getByRole("status")).toContainText("Check your inbox");
  await expect(dialog).toContainText(
    "ROMs stay on this device. Cloud backups are on by default",
  );
  await page.evaluate(() => document.fonts.ready);
  await page.screenshot({ path: "../artifacts/account-email-desktop.png" });
  await page.getByRole("button", { name: "Close dialog" }).click();
  await page.getByRole("button", { name: "Enter player" }).click();
  await expect(page.getByLabel(/^Game screen/)).toBeVisible();
});

test("toolbar cloud upload runs backup for an active game only when enabled", async ({
  page,
}) => {
  await page.route("**/api/auth/get-session", (route) =>
    route.fulfill({
      json: {
        user: {
          id: "toolbar-player",
          name: "Test Player",
          email: "player@example.com",
        },
      },
    }),
  );
  await page.route("**/api/saves", (route) =>
    route.fulfill({ json: { saves: [], cursor: null } }),
  );
  await page.goto("/");
  await page.getByRole("button", { name: "Enter player" }).click();
  const backup = page.locator(".console-toolbar").getByRole("button", {
    name: "Back up now",
  });
  await expect(backup).toBeDisabled();
  expect(
    await backup.evaluate((button) =>
      button.nextElementSibling?.getAttribute("aria-label"),
    ),
  ).toBe("Fullscreen");
  await page.getByLabel("Open ROM", { exact: true }).setInputFiles({
    name: "Local adventure.gba",
    mimeType: "application/octet-stream",
    buffer: Buffer.from([42, 0, 160, 227, 254, 255, 255, 234]),
  });
  await expect(backup).toBeEnabled();
  await expect(backup.locator("svg path")).toHaveCount(2);
  await page.evaluate(() => {
    window.player.backupCloud = async () => {
      document.body.dataset.backupCalled = "yes";
    };
  });
  await backup.click();
  await expect(page.locator("body")).toHaveAttribute(
    "data-backup-called",
    "yes",
  );
  await page.evaluate(() => document.fonts.ready);
  await page.locator(".player-tools").screenshot({
    path: "../artifacts/backup-toolbar-desktop.png",
  });
  await page.setViewportSize({ width: 390, height: 844 });
  await page.locator(".player-tools").screenshot({
    path: "../artifacts/backup-toolbar-mobile.png",
  });
  await page.evaluate(() => window.player.cloud.enable(false));
  await expect(backup).toBeDisabled();
  await page.locator(".player-tools").screenshot({
    path: "../artifacts/backup-toolbar-disabled-mobile.png",
  });
});

test("cloud backups default on, an opt-out survives reload, and sign-out clears the session", async ({
  page,
}) => {
  await page.setViewportSize({ width: 390, height: 844 });
  await page.route("**/api/auth/get-session", (route) =>
    route.fulfill({
      json: {
        user: {
          id: "test-player",
          name: "Test player",
          email: "player@example.com",
        },
      },
    }),
  );
  await page.route("**/api/saves", (route) =>
    route.fulfill({ json: { saves: [], cursor: null } }),
  );
  await page.route("**/api/auth/sign-out", (route) =>
    route.fulfill({ json: { success: true } }),
  );
  await page.goto("/?account=1");
  const dialog = page.getByRole("dialog", { name: "Account", exact: true });
  await expect(dialog).toContainText("player@example.com");
  await dialog.getByRole("button", { name: "Disable cloud backups" }).click();
  await expect(
    dialog.getByRole("button", { name: "Enable cloud backups on this device" }),
  ).toBeVisible();
  await page.goto("/?account=1");
  await expect(
    dialog.getByRole("button", { name: "Enable cloud backups on this device" }),
  ).toBeVisible();
  await page.evaluate(() => document.fonts.ready);
  await page.screenshot({ path: "../artifacts/account-signed-in-mobile.png" });
  await dialog.getByRole("button", { name: "Sign out", exact: true }).click();
  await expect(dialog.getByLabel("Email address")).toBeVisible();
  await expect(dialog).not.toContainText("player@example.com");
});

test("account deletion needs one confirmation, signs out, and keeps local progress", async ({
  page,
}) => {
  let deleted = false;
  let attempts = 0;
  await page.route("**/api/auth/get-session", (route) =>
    route.fulfill({
      json: deleted
        ? null
        : {
            user: {
              id: "delete-player",
              name: "Test player",
              email: "player@example.com",
            },
          },
    }),
  );
  await page.route("**/api/saves", (route) =>
    route.fulfill({
      json: { saves: [], cursor: null, usage: { playthroughs: 4, limit: 100 } },
    }),
  );
  await page.route("**/api/account", async (route) => {
    expect(route.request().method()).toBe("DELETE");
    expect(route.request().headers()["x-zigba-account"]).toBe("delete-player");
    attempts++;
    deleted = true;
    await route.fulfill({
      status: 202,
      json: { accountDeleted: true, cloudDeletion: "pending" },
    });
  });
  await page.goto("/");
  await expect(page.getByLabel("Open ROM", { exact: true })).toBeEnabled();
  await page.getByLabel("Open ROM", { exact: true }).setInputFiles({
    name: "Local adventure.gba",
    mimeType: "application/octet-stream",
    buffer: Buffer.from([42, 0, 160, 227, 254, 255, 255, 234]),
  });
  await expect(
    page.getByRole("heading", { name: "Local adventure", exact: true }),
  ).toBeVisible();
  await page.getByRole("button", { name: "Account", exact: true }).click();
  const dialog = page.getByRole("dialog", { name: "Account", exact: true });
  await dialog
    .getByRole("button", { name: "Delete my account", exact: true })
    .click();
  await expect(dialog).toContainText(
    "Local games and saves stay on this device",
  );
  await dialog.getByRole("button", { name: "Cancel", exact: true }).click();
  expect(attempts).toBe(0);
  await dialog
    .getByRole("button", { name: "Delete my account", exact: true })
    .click();
  await page.setViewportSize({ width: 390, height: 844 });
  await page.screenshot({
    path: "../artifacts/playthrough-work/delete-account-mobile.png",
  });
  await dialog
    .getByRole("button", { name: "Delete account permanently", exact: true })
    .click();
  await expect(dialog).toContainText(
    "Account deleted. Cloud-save cleanup continues automatically",
  );
  await expect(dialog.getByLabel("Email address")).toBeVisible();
  expect(attempts).toBe(1);
  await page.reload();
  await expect(
    page.getByRole("button", { name: "Resume adventure" }),
  ).toBeEnabled();
});

test("closing Account cancels a pending Discord navigation", async ({
  page,
}) => {
  await page.route("**/api/auth/get-session", (route) =>
    route.fulfill({ json: null }),
  );
  let finish!: () => void;
  const waiting = new Promise<void>((resolve) => {
    finish = resolve;
  });
  let requested!: () => void;
  const pending = new Promise<void>((resolve) => {
    requested = resolve;
  });
  await page.route("**/api/auth/sign-in/social", async (route) => {
    expect(route.request().postDataJSON()).toEqual({
      provider: "discord",
      callbackURL: "/?account=1",
      errorCallbackURL: "/?account=1",
    });
    requested();
    await waiting;
    await route.fulfill({
      json: { url: "https://discord.com/oauth2/authorize" },
    });
  });
  let navigated = false;
  await page.route("https://discord.com/**", async (route) => {
    navigated = true;
    await route.fulfill({ contentType: "text/html", body: "Discord" });
  });
  await page.goto("/?account=1");
  await page.getByRole("button", { name: "Continue with Discord" }).click();
  await pending;
  await page.getByRole("button", { name: "Close dialog" }).click();
  finish();
  await page.waitForTimeout(300);
  expect(navigated).toBe(false);
  await expect(
    page.getByRole("button", { name: "Enter player" }),
  ).toBeVisible();
});

test("an expired link explains how to retry without hiding sign-in options", async ({
  page,
}) => {
  await page.setViewportSize({ width: 360, height: 640 });
  await page.route("**/api/auth/get-session", (route) =>
    route.fulfill({ json: null }),
  );
  await page.goto("/?account=1&error=INVALID_TOKEN");
  const dialog = page.getByRole("dialog", { name: "Account", exact: true });
  await expect(dialog.getByRole("alert")).toContainText(
    "expired or already been used",
  );
  await expect(dialog.getByLabel("Email address")).toBeVisible();
  await page.evaluate(() => document.fonts.ready);
  await page.screenshot({ path: "../artifacts/account-expired-mobile.png" });
});

test("failed login and offline backend do not block local gameplay", async ({
  page,
}) => {
  await page.route("**/api/auth/get-session", (route) =>
    route.fulfill({
      status: 503,
      json: { error: { code: "auth_unavailable" } },
    }),
  );
  await page.goto("/?account=1&error=discord_email_not_verified");
  const dialog = page.getByRole("dialog", { name: "Account", exact: true });
  await expect(dialog.getByRole("alert")).toContainText("unavailable");
  await page.evaluate(() => document.fonts.ready);
  await page.screenshot({
    path: "../artifacts/account-unavailable-desktop.png",
  });
  await page.getByRole("button", { name: "Close dialog" }).click();
  await page.getByRole("button", { name: "Enter player" }).click();
  await expect(page.getByLabel(/^Game screen/)).toBeVisible();
});
