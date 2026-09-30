import { expect, test, type Page } from "@playwright/test";
import type { Player } from "../../src/player";

const dialogName = "Rewind timeline";
async function prepare(page: Page, count = 5) {
  await page.goto("/");
  await page.getByRole("button", { name: "Open player" }).click();
  await page.getByLabel("Open ROM", { exact: true }).setInputFiles({
    name: "Rewind.gba",
    mimeType: "application/octet-stream",
    buffer: Buffer.from([42, 0, 160, 227, 254, 255, 255, 234]),
  });
  // Ready for quick slots; the phone layout has no quick bar to wait on.
  await expect
    .poll(() =>
      page.evaluate(() => {
        const { state } = (window as unknown as { player: Player }).player;
        return Boolean(state.playthrough) && !state.changing;
      }),
    )
    .toBe(true);
  await expect
    .poll(() =>
      page.evaluate(
        () => (window as unknown as { player: Player }).player.state.playing,
      ),
    )
    .toBe(true);
  await page.evaluate(async (count) => {
    const player = (window as unknown as { player: Player }).player;
    await player.pause();
    // Budgeted history keeps every capture, so space captures a second apart:
    // a stray presentation frame cannot add a checkpoint. The newest capture
    // is the live frame, leaving exactly `count` selectable checkpoints.
    await player.emulator.setRewindConfig(undefined, 60);
    for (let i = 0; i <= count; i++) {
      const cpu = await player.emulator.inspect();
      await player.emulator.runUntil(
        cpu.cycle + (i === 0 ? 1n : 60n) * 280896n,
      );
    }
  }, count);
}

for (const mode of ["desktop", "mobile", "fullscreen"] as const) {
  test(`rewind panel placement: ${mode}`, async ({ page }) => {
    await page.setViewportSize(
      mode === "mobile"
        ? { width: 390, height: 844 }
        : { width: 1280, height: 900 },
    );
    await prepare(page);
    if (mode === "fullscreen") {
      await page
        .locator("#game-display")
        .evaluate((node) => node.requestFullscreen());
      await expect
        .poll(() => page.evaluate(() => !!document.fullscreenElement))
        .toBe(true);
    }
    await page.getByLabel(/^Game screen/).focus();
    await page.keyboard.press("Shift+KeyR");
    const dialog = page.getByRole("dialog", { name: dialogName });
    await expect(dialog).toBeVisible();
    await expect(dialog.getByText("Loading…")).toHaveCount(0);
    await page.evaluate(() => document.fonts.ready);
    const panel = await dialog.boundingBox();
    const display = await page.locator("#game-display").boundingBox();
    // Desktop displays include a 1px frame; fullscreen and the phone layout
    // remove it.
    const inset = mode === "mobile" ? 5 : mode === "fullscreen" ? 10 : 11;
    expect(panel!.x - display!.x).toBeCloseTo(inset, 0);
    expect(display!.y + display!.height - panel!.y - panel!.height).toBeCloseTo(
      inset,
      0,
    );
    await page.screenshot({
      path: `../../artifacts/rewind-dialog/${process.env.REWIND_CAPTURE ?? "after"}-${mode}.png`,
    });
  });
}

test("rewind contains focus, blocks game shortcuts, and restores focus on cancellation", async ({
  page,
}) => {
  await prepare(page);
  const trigger = page.getByRole("button", { name: /^Rewind Shift/ });
  await trigger.click();
  const dialog = page.getByRole("dialog", { name: dialogName });
  const range = dialog.getByRole("slider");
  await expect(range).toBeFocused();
  const selected = Number(await range.inputValue());
  await range.press("ArrowLeft");
  await expect(range).toHaveValue(String(selected - 1));
  await range.press("KeyR");
  await range.press("Space");
  await expect(dialog).toBeVisible();
  // A press on the game surface is not a new cancellation gesture.
  await page.getByLabel(/^Game screen/).click({ position: { x: 5, y: 5 } });
  await expect(dialog).toBeVisible();
  expect(
    await page.evaluate(
      () => (window as unknown as { player: Player }).player.state.playing,
    ),
  ).toBe(false);
  await dialog.getByRole("button", { name: "Rewind here" }).focus();
  await page.keyboard.press("Tab");
  await expect(dialog.getByRole("button").first()).toBeFocused();
  await page.keyboard.press("Shift+Tab");
  await expect(
    dialog.getByRole("button", { name: "Rewind here" }),
  ).toBeFocused();
  await page.keyboard.press("Escape");
  await expect(dialog).toHaveCount(0);
  // Closing rewind returns to the game screen, including when opened by button.
  await expect(page.getByLabel(/^Game screen/)).toBeFocused();
  await page.getByLabel(/^Game screen/).focus();
  await page.keyboard.press("Shift+KeyR");
  await expect(range).toBeFocused();
  await dialog.getByRole("button", { name: /Cancel/ }).press("Space");
  await expect(dialog).toHaveCount(0);
  await expect(page.getByLabel(/^Game screen/)).toBeFocused();
  expect(
    await page.evaluate(
      () => (window as unknown as { player: Player }).player.state.playing,
    ),
  ).toBe(false);
});

test("Escape resumes an initially playing game without changing its checkpoint history", async ({
  page,
}) => {
  await prepare(page);
  await page.evaluate(() =>
    (window as unknown as { player: Player }).player.play(),
  );
  await page.keyboard.press("Shift+KeyR");
  const dialog = page.getByRole("dialog", { name: dialogName });
  await expect(dialog.getByRole("slider")).toBeFocused();
  const before = await page.evaluate(async () => {
    const player = (window as unknown as { player: Player }).player;
    return (await player.emulator.rewindCheckpoints()).timeline;
  });
  await page.keyboard.press("Escape");
  await expect(dialog).toHaveCount(0);
  await expect
    .poll(() =>
      page.evaluate(
        () => (window as unknown as { player: Player }).player.state.playing,
      ),
    )
    .toBe(true);
  expect(
    await page.evaluate(async () => {
      const player = (window as unknown as { player: Player }).player;
      return (await player.emulator.rewindCheckpoints()).timeline;
    }),
  ).toBe(before);
  await expect(page.getByLabel(/^Game screen/)).toBeFocused();
});

test("Enter commits the selected checkpoint from the rewind slider", async ({
  page,
}) => {
  await prepare(page);
  const checkpoints = await page.evaluate(async () => {
    const player = (window as unknown as { player: Player }).player;
    return (await player.emulator.rewindCheckpoints()).checkpoints.map(
      (entry) => entry.cycle.toString(),
    );
  });
  expect(checkpoints.length).toBeGreaterThan(1);
  await page.getByRole("button", { name: /^Rewind Shift/ }).click();
  const dialog = page.getByRole("dialog", { name: dialogName });
  const range = dialog.getByRole("slider", { name: "Rewind checkpoint" });
  await expect(range).toBeFocused();
  await range.press("ArrowLeft");
  await expect(range).toHaveValue(String(checkpoints.length - 2));
  await range.press("Enter");
  await expect(dialog).toHaveCount(0);
  await expect
    .poll(() =>
      page.evaluate(async () => {
        const player = (window as unknown as { player: Player }).player;
        return (await player.emulator.inspect()).cycle.toString();
      }),
    )
    .toBe(checkpoints[checkpoints.length - 2]);
});

test("one checkpoint remains selectable and commits its exact cycle", async ({
  page,
}) => {
  await prepare(page, 1);
  const cycle = await page.evaluate(async () => {
    const player = (window as unknown as { player: Player }).player;
    return (
      await player.emulator.rewindCheckpoints()
    ).checkpoints[0].cycle.toString();
  });
  await page.getByRole("button", { name: /^Rewind Shift/ }).click();
  const dialog = page.getByRole("dialog", { name: dialogName });
  await expect(dialog.getByRole("slider")).toBeFocused();
  await expect(dialog.getByRole("slider")).toHaveValue("0");
  await expect(dialog.getByText("1 checkpoint available")).toBeVisible();
  await expect(dialog.getByText("Loading…")).toHaveCount(0);
  await dialog.screenshot({
    path: "../../artifacts/rewind-dialog/after-one-checkpoint.png",
  });
  await dialog.getByRole("slider").press("ArrowLeft");
  await dialog.getByRole("button", { name: "Rewind here" }).press("Space");
  await expect(dialog).toHaveCount(0);
  await expect
    .poll(() =>
      page.evaluate(async () => {
        const player = (window as unknown as { player: Player }).player;
        return (await player.emulator.inspect()).cycle.toString();
      }),
    )
    .toBe(cycle);
});
