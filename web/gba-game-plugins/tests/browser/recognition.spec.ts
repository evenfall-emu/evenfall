// The player recognizing a build no committed manifest describes, from the
// game's locators. Skips unless ZIGBA_HNS_120_ROM names the user-supplied ROM.
import { test, expect, type Page } from "@playwright/test";
import { readFile } from "node:fs/promises";
import { createHash } from "node:crypto";

const hns120 = "b8a4b491ca67a76f079a8e7c19fd5512a2147dd2";

async function ready(page: Page) {
  await page.goto("/");
  await page.getByRole("button", { name: "Open player" }).click();
  await expect(page.getByLabel("Open ROM", { exact: true })).toBeEnabled();
}

const cachedBuilds = (page: Page) =>
  page.evaluate(
    () =>
      new Promise<string[]>((resolve, reject) => {
        const request = indexedDB.open("zigba-local");
        request.onerror = () => reject(request.error);
        request.onsuccess = () => {
          const keys = request.result
            .transaction("builds")
            .objectStore("builds")
            .getAllKeys();
          keys.onerror = () => reject(keys.error);
          keys.onsuccess = () => {
            request.result.close();
            resolve(keys.result as string[]);
          };
        };
      }),
  );

test("recognizes Heart & Soul 1.2.0 from its locators, then from the cache", async ({
  page,
}, testInfo) => {
  const rom = process.env.ZIGBA_HNS_120_ROM;
  test.skip(!rom, "Supply the hash-verified Heart & Soul 1.2.0 ROM");
  expect(
    createHash("sha1")
      .update(await readFile(rom!))
      .digest("hex"),
  ).toBe(hns120);
  const errors: string[] = [];
  page.on("pageerror", (error) => errors.push(error.message));
  await page.setViewportSize({ width: 1672, height: 941 });
  await ready(page);
  await page.getByLabel("Open ROM", { exact: true }).setInputFiles(rom!);
  const dashboard = page.locator('[data-game-layout="pokemon"]');
  await expect(dashboard).toBeVisible({ timeout: 30_000 });
  const badge = page.getByText("Detected build · not reviewed");
  await expect(badge).toBeVisible();
  await expect(badge).toHaveAttribute(
    "title",
    /Unconfirmed: rom\.catalog\.moveDescriptions\.count/,
  );
  expect(await cachedBuilds(page)).toEqual([
    expect.stringMatching(
      new RegExp(`^pokemon/heart-n-soul:${hns120}:[0-9a-f]{16}$`),
    ),
  ]);
  // Its views decode: a second of samples does not drop the build.
  await page.waitForTimeout(1500);
  await expect(dashboard).toBeVisible();
  await expect(page.getByText("could not read this build")).toHaveCount(0);
  await page.screenshot({
    path: testInfo.outputPath("recognized-desktop.png"),
  });

  // A new session finds the build in the cache.
  await page.reload();
  await ready(page);
  await page.getByLabel("Open ROM", { exact: true }).setInputFiles(rom!);
  await expect(dashboard).toBeVisible({ timeout: 10_000 });
  await expect(badge).toBeVisible();
  expect(errors).toEqual([]);
});
