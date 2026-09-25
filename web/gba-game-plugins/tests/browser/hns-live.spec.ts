import { test, expect } from "@playwright/test";
import { readFile } from "node:fs/promises";
import { createHash } from "node:crypto";

// Optional user-owned fixtures. Neither the ROM nor state is part of the repo.
test("HnS 1.2.1 game2: real names, artwork, moves, selection and timeline replacement", async ({
  page,
}) => {
  const rom = process.env.ZIGBA_HNS_ROM;
  const state = process.env.ZIGBA_HNS_STATE;
  test.skip(
    !rom || !state,
    "Set ZIGBA_HNS_ROM and ZIGBA_HNS_STATE to the documented local fixtures",
  );
  const sha = (bytes: Uint8Array) =>
    createHash("sha256").update(bytes).digest("hex");
  expect(sha(await readFile(rom!))).toBe(
    "7c4f90c8b68b64d4639a37306d5302a11cd4d7c44005114ec2faa2b28b210c3d",
  );
  expect(sha(await readFile(state!))).toBe(
    "71817992f9053560808493c79c3e65b5acba2b4499375cc52d76806b2107fb5f",
  );
  const errors: string[] = [];
  page.on("pageerror", (error) => errors.push(error.message));
  await page.setViewportSize({ width: 1440, height: 1000 });
  await page.goto("/debug.html");
  await expect(page.locator("footer")).toContainText("Open a game");
  await page.getByLabel("Open game").setInputFiles(rom!);
  await expect(page.locator('[data-game-layout="pokemon"]')).toBeVisible();
  await page.getByLabel("Import state", { exact: true }).setInputFiles(state!);
  const team = page.getByRole("list", { name: "Current team" });
  await expect(team.getByRole("listitem")).toHaveCount(3);
  for (const [slot, name, hp] of [
    [0, "TOTODILE", "29 / 33"],
    [1, "PIDGEY", "16 / 16"],
    [2, "HOPPIP", "14 / 14"],
  ] as const) {
    await expect(team.getByRole("listitem").nth(slot)).toContainText(name);
    await expect(team.getByRole("listitem").nth(slot)).toContainText(hp);
  }
  const details = page.getByRole("region", { name: "Selected Pokémon" });
  await expect(details).toContainText("MODEST");
  await expect(details).toContainText("TORRENT");
  await expect(details).toContainText("SCRATCH");
  await expect(details).toContainText("WATER GUN");
  await expect(details.locator(".move-category.special")).toHaveText("special");
  await expect(team.locator("img")).toHaveCount(3);
  await expect
    .poll(() =>
      team
        .locator("img")
        .evaluateAll((images) =>
          images.every(
            (image) => (image as HTMLImageElement).naturalWidth === 32,
          ),
        ),
    )
    .toBe(true);
  await expect
    .poll(() =>
      details
        .locator("img")
        .evaluate((image) => (image as HTMLImageElement).naturalWidth),
    )
    .toBe(64);
  await page.screenshot({
    path: "../../artifacts/hud/hns-game2-desktop.png",
    fullPage: true,
  });
  await team.getByRole("button").nth(1).click();
  await expect(details).toContainText("KEEN EYE");
  await expect(details).toContainText("TACKLE");
  await page.keyboard.press("End");
  await expect(details).toContainText("CHLOROPHYLL");
  await expect(details).toContainText("SPLASH");
  // Reload the same state: same Pokémon identity, different timeline => reset focus.
  await page.getByLabel("Import state", { exact: true }).setInputFiles(state!);
  await expect(details).toContainText("TORRENT");
  await page.getByRole("button", { name: "Frame +1", exact: true }).click();
  await expect(team.getByRole("listitem")).toHaveCount(3);
  const overlayInsidePlayer = () =>
    page.evaluate(() => {
      const overlay = document
        .querySelector(".performance")!
        .getBoundingClientRect();
      const player = document.querySelector(".bezel")!.getBoundingClientRect();
      return (
        overlay.left >= player.left &&
        overlay.right <= player.right &&
        overlay.top >= player.top &&
        overlay.bottom <= player.bottom
      );
    });
  expect(await overlayInsidePlayer()).toBe(true);
  await page.setViewportSize({ width: 390, height: 844 });
  await details.getByText("IVs & EVs", { exact: true }).click();
  await expect(details.getByRole("table")).toContainText("31");
  expect(
    await page.evaluate(
      () => document.documentElement.scrollWidth <= innerWidth,
    ),
  ).toBe(true);
  expect(await overlayInsidePlayer()).toBe(true);
  await page.evaluate(() => window.scrollTo(0, 0));
  await page.screenshot({
    path: "../../artifacts/hud/hns-game2-mobile.png",
    fullPage: true,
  });
  expect(errors).toEqual([]);
});
