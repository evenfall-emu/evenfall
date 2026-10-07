// The player recognizing a build no committed manifest describes, from the
// game's locators. Skips builds without a user-supplied ROM fixture.
import { test, expect, type Page } from "@playwright/test";
import { readFile } from "node:fs/promises";
import { createHash } from "node:crypto";
import type { Emulator } from "@evenfall/gba";

const builds = [
  {
    name: "Heart & Soul 1.2.0",
    family: "pokemon/heart-n-soul",
    hash: "b8a4b491ca67a76f079a8e7c19fd5512a2147dd2",
    rom: process.env.EVENFALL_HNS_120_ROM,
    unconfirmed: /Unconfirmed: rom\.catalog\.moveDescriptions\.count/,
  },
  {
    name: "Elite Redux 2.65.4.1b",
    family: "pokemon/elite-redux",
    hash: "2f3e17e55c6afe23da97ec0f96a067eeb142ad15",
    rom: process.env.EVENFALL_REDUX_26541_ROM,
    unconfirmed: null,
  },
];

async function ready(page: Page) {
  await page.goto("/");
  await page.getByRole("button", { name: "Open player" }).click();
  await expect(page.getByLabel("Open ROM", { exact: true })).toBeEnabled();
}

const cachedBuilds = (page: Page) =>
  page.evaluate(
    () =>
      new Promise<string[]>((resolve, reject) => {
        const request = indexedDB.open("evenfall-local");
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

for (const build of builds)
  test(`recognizes ${build.name} from its locators, then from the cache`, async ({
    page,
  }, testInfo) => {
    const rom = build.rom;
    test.skip(!rom, `Supply the hash-verified ${build.name} ROM`);
    expect(
      createHash("sha1")
        .update(await readFile(rom!))
        .digest("hex"),
    ).toBe(build.hash);
    const errors: string[] = [];
    page.on("pageerror", (error) => errors.push(error.message));
    await page.setViewportSize({ width: 1672, height: 941 });
    await ready(page);
    await page.getByLabel("Open ROM", { exact: true }).setInputFiles(rom!);
    const dashboard = page.locator('[data-game-layout="pokemon"]');
    await expect(dashboard).toBeVisible({ timeout: 30_000 });
    const badge = page.getByText("Detected build · not reviewed");
    await expect(badge).toBeVisible();
    if (build.unconfirmed)
      await expect(badge).toHaveAttribute("title", build.unconfirmed);
    else await expect(badge).not.toHaveAttribute("title", /Unconfirmed:/);
    expect(await cachedBuilds(page)).toEqual([
      expect.stringMatching(
        new RegExp(`^${build.family}:${build.hash}:[0-9a-f]{16}$`),
      ),
    ]);
    // watchBuild rejects a view after 20 unsuccessful samples. Wait for actual
    // captures, not wall time: the next capture starts after the prior decode.
    await page.evaluate(() => {
      const host = window as unknown as {
        emulator: Emulator;
        buildSamples: number;
      };
      host.buildSamples = 0;
      const capture = host.emulator.captureMemory.bind(host.emulator);
      host.emulator.captureMemory = async () => {
        const sample = await capture();
        if (sample) host.buildSamples++;
        return sample;
      };
    });
    await expect
      .poll(() =>
        page.evaluate(
          () => (window as unknown as { buildSamples: number }).buildSamples,
        ),
      )
      .toBeGreaterThan(20);
    await expect(dashboard).toBeVisible();
    await expect(page.getByText("could not read this build")).toHaveCount(0);
    await expect
      .poll(
        async () =>
          page.evaluate(async () => {
            const { emulator } = window as unknown as { emulator: Emulator };
            return Number((await emulator.inspect()).frames);
          }),
        { timeout: 20_000 },
      )
      .toBeGreaterThanOrEqual(600);
    expect(
      await page.evaluate(async () => {
        const { emulator } = window as unknown as { emulator: Emulator };
        return (await emulator.inspect()).fault;
      }),
    ).toBeNull();
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
