import { expect, test } from "@playwright/test";

test("popover focus, outside dismissal and game input isolation", async ({
  page,
}) => {
  await page.goto("/");
  await expect(page.getByLabel("Open ROM", { exact: true })).toBeEnabled();
  await page.getByLabel("Open ROM", { exact: true }).setInputFiles({
    name: "Overlay loop.gba",
    mimeType: "application/octet-stream",
    buffer: Buffer.from([42, 0, 160, 227, 254, 255, 255, 234]),
  });
  const speed = page.getByRole("button", { name: "Playback speed" });
  await speed.click();
  await page.keyboard.down("KeyX");
  await expect
    .poll(() =>
      page.evaluate(async () => {
        const bytes = await window.player.emulator.readMemory(0x04000130, 2);
        return new DataView(bytes.buffer).getUint16(0, true) & 1023;
      }),
    )
    .toBe(1023);
  await page.keyboard.up("KeyX");
  await page.getByRole("button", { name: "4×", exact: true }).click();
  await expect(speed).toHaveText("4×");
  await expect(speed).toBeFocused();
  const more = page.getByRole("button", { name: "More options" });
  await more.click();
  const options = page.getByRole("dialog", { name: "Saves and options" });
  await expect(
    options.getByLabel("Import state", { exact: true }),
  ).toBeFocused();
  await page.keyboard.press("Escape");
  await expect(options).toHaveCount(0);
  await expect(more).toBeFocused();
  await more.click();
  await page
    .getByRole("heading", { name: "Overlay loop", exact: true })
    .click();
  await expect(options).toHaveCount(0);
  // Test dialog focus restoration without gameplay's intentional canvas focus
  // on resume. The account suite separately covers closing while playing.
  await page.evaluate(() => window.player.pause());
  await page.getByRole("button", { name: "Account", exact: true }).click();
  const account = page.getByRole("dialog", { name: "Account", exact: true });
  await expect(account).toBeVisible();
  await expect(
    account.getByRole("button", { name: "Close dialog" }),
  ).toBeFocused();
  await page.keyboard.press("Shift+Tab");
  await expect
    .poll(() =>
      account.evaluate((node) => node.contains(document.activeElement)),
    )
    .toBe(true);
  await page.mouse.click(2, 2);
  await expect(account).toHaveCount(0);
  await expect(
    page.getByRole("button", { name: "Account", exact: true }),
  ).toBeFocused();
  expect(await page.evaluate(() => window.player.state.playing)).toBe(false);
});

test("cloud restore blocks Escape and backdrop dismissal until settled", async ({
  page,
}) => {
  await page.route("**/api/auth/get-session", (route) =>
    route.fulfill({ json: null }),
  );
  await page.goto("/");
  await expect(page.getByLabel("Open ROM", { exact: true })).toBeEnabled();
  await page.evaluate(() => {
    const player = window.player;
    player.cloud.discover = async () => {};
    player.restoreCloud = async () =>
      new Promise((_resolve, reject) => {
        Object.assign(window, {
          failRestore: () => reject(new Error("Fixture restore failed")),
        });
      });
    player.update({
      cloud: {
        ...player.state.cloud,
        accountId: "fixture",
        latest: {
          accountId: "fixture",
          hash: "1".repeat(40),
          playthroughId: "default",
          playthroughName: "Test run",
          title: "Cloud fixture",
          capturedAt: 1,
          uploadedAt: 1,
        },
      },
    });
  });
  await page.getByRole("button", { name: "Resume adventure" }).click();
  const dialog = page.getByRole("dialog", { name: "Resume from cloud" });
  await dialog.getByLabel("Choose ROM for cloud resume").setInputFiles({
    name: "fixture.gba",
    mimeType: "application/octet-stream",
    buffer: Buffer.from([0]),
  });
  await expect(dialog).toHaveAttribute("aria-busy", "true");
  await expect(
    dialog.getByRole("button", { name: "Close dialog" }),
  ).toBeDisabled();
  await page.keyboard.press("Escape");
  await page.mouse.click(2, 2);
  await expect(dialog).toBeVisible();
  await page.evaluate(() =>
    (window as unknown as { failRestore(): void }).failRestore(),
  );
  await expect(dialog.getByRole("alert")).toHaveText("Fixture restore failed");
  await page.keyboard.press("Escape");
  await expect(dialog).toHaveCount(0);
});

for (const width of [1280, 390]) {
  test(`app overlay appearance at ${width}`, async ({ page }) => {
    await page.setViewportSize({ width, height: 844 });
    await page.route("**/api/auth/get-session", (route) =>
      route.fulfill({ json: null }),
    );
    await page.goto("/");
    await page.getByRole("button", { name: "Account", exact: true }).click();
    await expect(
      page.getByRole("dialog", { name: "Account", exact: true }),
    ).toBeVisible();
    await page.evaluate(() => document.fonts.ready);
    const capture = async (name: string) =>
      page.screenshot({
        path: `../../artifacts/app-overlays/${process.env.OVERLAY_PHASE ?? "final"}-${width}-${name}.png`,
      });
    await capture("account");
    await page.getByRole("button", { name: "Close dialog" }).click();
    await page.getByRole("button", { name: "Open player" }).click();
    // Phones keep the toolbar in the player menu.
    if (width < 650)
      await page.getByRole("button", { name: "Player menu" }).click();
    await page.getByRole("button", { name: "Playback speed" }).click();
    await expect(
      page.getByRole("dialog", { name: "Playback speed" }),
    ).toBeVisible();
    await capture("speed");
    await page.getByRole("button", { name: "More options" }).click();
    await expect(
      page.getByRole("dialog", { name: "Playback speed" }),
    ).toHaveCount(0);
    await expect(
      page.getByRole("dialog", { name: "Saves and options" }),
    ).toBeVisible();
    await capture("more");
    await page.keyboard.press("Escape");
    await page
      .getByRole("button", { name: "How to play", exact: true })
      .click();
    await expect(
      page.getByRole("dialog", { name: "How to play" }),
    ).toBeVisible();
    await capture("help");
  });
}
