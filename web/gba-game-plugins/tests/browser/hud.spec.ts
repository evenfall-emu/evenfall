import { test, expect, type Page } from "@playwright/test";
import type { Emulator } from "@evenfall/gba";
import { build } from "vite";
import { fixtureBuild } from "./support";
import { resolve } from "node:path";
import { rm } from "node:fs/promises";

test.beforeAll(async () => {
  await build({
    ...fixtureBuild,
    build: {
      outDir: "../../zig-out/web/hud-test",
      emptyOutDir: true,
      lib: {
        entry: resolve("tests/browser/hud.fixture.tsx"),
        formats: ["es"],
        fileName: () => "fixture.js",
      },
    },
  });
});
test.afterAll(() =>
  rm("../../zig-out/web/hud-test", { recursive: true, force: true }),
);

const rom = {
  name: "Loop.gba",
  mimeType: "application/octet-stream",
  buffer: Buffer.from([42, 0, 160, 227, 254, 255, 255, 234]),
};
type HudWindow = typeof window & {
  emulator: Emulator;
  hudFixture: "party" | "empty" | "corrupt" | "error";
  hudOrder?: number[];
  captures: number;
  releaseCapture?: () => void;
};

async function open(page: Page, route = "/#/hud") {
  await page.goto(route.replace(/^\//, "/debug.html"));
  await expect(page.locator("footer")).toContainText("Open a game");
  await page.getByLabel("Open game").setInputFiles(rom);
  await expect(
    page.getByRole("button", { name: "Play", exact: true }),
  ).toBeEnabled();
}

test("HUD shares playback, releases keys, stops observing off-page, and Lab still pauses", async ({
  page,
}) => {
  await open(page);
  await expect(page.locator('[data-game-layout="pokemon"]')).toBeVisible();
  await expect(page.getByText("No bridge for this ROM")).toBeVisible();
  await page.evaluate(() => {
    const host = window as HudWindow;
    const capture = host.emulator.captureMemory.bind(host.emulator);
    host.captures = 0;
    host.emulator.captureMemory = () => {
      host.captures++;
      return capture();
    };
  });
  await page.getByRole("button", { name: "Play", exact: true }).click();
  await expect
    .poll(() => page.evaluate(() => (window as HudWindow).captures))
    .toBeGreaterThan(1);
  const before = await page.evaluate(async () => {
    const host = window as HudWindow;
    return (await host.emulator.captureMemory())!.timeline;
  });
  await page.getByRole("button", { name: "Sound on", exact: true }).click();
  await page.getByLabel("Game screen.", { exact: false }).focus();
  await page.keyboard.down("KeyX");
  await expect
    .poll(() =>
      page.evaluate(
        async () =>
          (await (window as HudWindow).emulator.readMemory(0x04000130, 1))[0],
      ),
    )
    .toBe(0xfe);
  await page.getByRole("link", { name: "Player", exact: true }).click();
  await expect(page.locator('[data-game-layout="generic"]')).toBeVisible();
  await expect(
    page.getByRole("button", { name: "Pause", exact: true }),
  ).toBeVisible();
  await expect(
    page.getByRole("button", { name: "Sound off", exact: true }),
  ).toBeVisible();
  await expect(page.getByRole("complementary", { name: "Party" })).toHaveCount(
    0,
  );
  const offPage = await page.evaluate(async () => {
    const host = window as HudWindow;
    const count = host.captures;
    const start = await host.emulator.inspect();
    // Advance enough real frames to expose a leaked HUD subscription.
    await new Promise<void>((resolve) => {
      let frames = 0;
      const listener = () => {
        if (++frames < 8) return;
        host.emulator.removeEventListener("frame", listener);
        resolve();
      };
      host.emulator.addEventListener("frame", listener);
    });
    const end = await host.emulator.inspect();
    return {
      captures: host.captures - count,
      advanced: end.cycle > start.cycle,
      keyinput: [...(await host.emulator.readMemory(0x04000130, 2))],
    };
  });
  expect(offPage).toEqual({ captures: 0, advanced: true, keyinput: [0xff, 3] });
  await page.keyboard.up("KeyX");
  await page.getByRole("link", { name: "HUD", exact: true }).click();
  await expect(page.getByText("No bridge for this ROM")).toBeVisible();
  await expect(
    page.getByRole("button", { name: "Pause", exact: true }),
  ).toBeVisible();
  expect(
    await page.evaluate(
      async () =>
        (await (window as HudWindow).emulator.captureMemory())!.timeline,
    ),
  ).toBe(before);
  await page.getByRole("link", { name: "Lab", exact: true }).click();
  await expect(
    page.getByRole("heading", { name: "Lab", exact: true }),
  ).toBeVisible();
  await page.getByRole("link", { name: "HUD", exact: true }).click();
  await expect(
    page.getByRole("button", { name: "Play", exact: true }),
  ).toBeVisible();
  await page.getByRole("button", { name: "Play", exact: true }).click();
  await expect(
    page.getByRole("button", { name: "Pause", exact: true }),
  ).toBeVisible();
});

test("HUD decodes snapshot fixtures, refreshes on step, and never keeps stale party cards", async ({
  page,
}) => {
  const errors: string[] = [];
  page.on("pageerror", (error) => errors.push(error.message));
  await open(page, "/");
  // RAM and catalogs are synthetic; ROM hash verification, coherent capture
  // identity, record decoding, subscriptions and UI are the actual implementation.
  const hash = await page.evaluate(async () => {
    const host = window as HudWindow;
    const capture = host.emulator.captureMemory.bind(host.emulator);
    const hash = (await capture())!.romHash;
    host.hudFixture = "party";
    let hold = true;
    host.emulator.captureMemory = async () => {
      if (hold) {
        hold = false;
        await new Promise<void>((resolve) => {
          host.releaseCapture = resolve;
        });
      }
      if (host.hudFixture === "error")
        throw new Error("Synthetic capture failure");
      const sample = (await capture())!;
      const bytes = new Uint8Array(0x40000);
      const data = new DataView(bytes.buffer);
      const iwram = new Uint8Array(0x8000);
      new DataView(iwram.buffer).setUint32(0x3740, 0x02030000, true);
      bytes[0x1d9e9] = host.hudFixture === "empty" ? 0 : 3;
      // Independent GAEM records (personality divisible by 24), HnS egg bit31.
      const members = [
        {
          personality: 24,
          species: 412,
          level: 26,
          hp: 53,
          maxHp: 62,
          status: 8,
          egg: false,
        },
        {
          personality: 48,
          species: 25,
          level: 41,
          hp: 0,
          maxHp: 120,
          status: 0,
          egg: false,
        },
        {
          personality: 72,
          species: 152,
          level: 5,
          hp: 11,
          maxHp: 17,
          status: 0,
          egg: true,
        },
      ];
      const ordered = (host.hudOrder ?? [0, 1, 2]).map(
        (member) => members[member],
      );
      ordered.forEach((mon, slot) => {
        const base = 0x1d9ec + slot * 100;
        const personality = mon.personality,
          owner = 0x12345678;
        data.setUint32(base, personality, true);
        data.setUint32(base + 4, owner, true);
        bytes[base + 8] = 0xff; // Empty nickname falls back to catalog name.
        bytes[base + 0x12] = 2;
        bytes[base + 0x14] = 0xff;
        bytes[base + 0x13] = mon.egg ? 6 : 2;
        const secure = new DataView(new ArrayBuffer(48));
        secure.setUint16(0, mon.species, true);
        secure.setUint32(40, mon.egg ? 0x80000000 : 0x40000000, true);
        let checksum = 0;
        for (let i = 0; i < 48; i += 2) checksum += secure.getUint16(i, true);
        data.setUint16(base + 0x1c, checksum & 0xffff, true);
        for (let i = 0; i < 48; i += 4)
          data.setUint32(
            base + 0x20 + i,
            secure.getUint32(i, true) ^ personality ^ owner,
            true,
          );
        data.setUint32(base + 0x50, mon.status, true);
        bytes[base + 0x54] = mon.level;
        data.setUint16(base + 0x56, mon.hp, true);
        data.setUint16(base + 0x58, mon.maxHp, true);
      });
      if (host.hudFixture === "corrupt") bytes[0x1d9ec + 0x1c] ^= 1;
      return {
        ...sample,
        regions: [
          { address: 0x02000000, bytes },
          { address: 0x03000000, bytes: iwram },
        ],
      };
    };
    return hash;
  });
  await page.evaluate(async (hash) => {
    const url = "/hud-test/fixture.js";
    const { mount } = await import(/* @vite-ignore */ url);
    mount((window as HudWindow).emulator, hash);
  }, hash);
  await expect(
    page.getByText("Waiting for a fresh memory snapshot…"),
  ).toBeVisible();
  await page.screenshot({ path: "../../artifacts/hud/loading.png" });
  await page.evaluate(() => (window as HudWindow).releaseCapture!());
  const team = page.getByRole("radiogroup", { name: "Current team" });
  await expect(team.getByRole("radio")).toHaveCount(3);
  const first = team.getByRole("radio").nth(0);
  await expect(first).toContainText("Species 412");
  await expect(first).toContainText("Level 26");
  await expect(first).toContainText("53 / 62");
  await expect(first).toContainText("Poison");
  await expect(first.getByRole("progressbar")).toHaveAttribute("value", "53");
  await expect(first.getByRole("progressbar")).toHaveAttribute("max", "62");
  await expect(team.getByRole("radio").nth(1)).toContainText("Fainted");
  await expect(team.getByRole("radio").nth(2)).toContainText("Egg");
  const inspector = page.locator('[aria-label="Selected Pokémon"]');
  await expect(inspector).toContainText("Species 412");
  await team.getByRole("radio").nth(1).click();
  await expect(inspector).toContainText("Species 25");
  await page.keyboard.press("ArrowDown");
  await expect(inspector).toContainText("Egg");
  await page.getByText("Snapshot details", { exact: true }).click();
  const cycle = page.locator(".bridge-sample dd").first();
  await expect(cycle).toHaveText("0");
  await page.evaluate(() => {
    (window as HudWindow).hudOrder = [2, 0, 1];
  });
  await page.getByRole("button", { name: "Frame +1", exact: true }).click();
  await expect(inspector).toContainText("Egg");
  await expect(inspector).toContainText("Slot 1");
  await expect(cycle).not.toHaveText("0");
  await page.getByText("Snapshot details", { exact: true }).click();
  await page.screenshot({
    path: "../../artifacts/hud/party-synthetic-desktop.png",
    fullPage: true,
  });
  await page.setViewportSize({ width: 390, height: 844 });
  await page.screenshot({
    path: "../../artifacts/hud/party-synthetic-mobile.png",
    fullPage: true,
  });
  expect(
    await page.evaluate(
      () => document.documentElement.scrollWidth <= innerWidth,
    ),
  ).toBe(true);
  await page.setViewportSize({ width: 1280, height: 900 });
  for (const [mode, text] of [
    ["empty", "No Pokémon in the party"],
    ["corrupt", "Party slot 1: Pokémon checksum mismatch"],
    ["error", "Synthetic capture failure"],
  ] as const) {
    await page.evaluate((mode) => {
      (window as HudWindow).hudFixture = mode;
    }, mode);
    await page.getByRole("button", { name: "Frame +1", exact: true }).click();
    await expect(page.getByText(text, { exact: true })).toBeVisible();
    await expect(team).toHaveCount(0);
    await page.screenshot({ path: `../../artifacts/hud/${mode}.png` });
  }
  await page.evaluate(() => {
    (window as HudWindow).hudFixture = "party";
  });
  await page.getByRole("button", { name: "Play", exact: true }).click();
  await expect(team.getByRole("radio")).toHaveCount(3);
  expect(errors).toEqual([]);
});

test("HUD is directly reachable and explains idle and unsupported ROM states", async ({
  page,
}) => {
  await page.goto("/debug.html#/hud");
  await expect(
    page.getByRole("heading", { name: "Load Heart & Soul" }),
  ).toBeVisible();
  await page.screenshot({ path: "../../artifacts/hud/idle.png" });
  await page.getByLabel("Open game").setInputFiles(rom);
  await expect(page.getByText("No bridge for this ROM")).toBeVisible();
  await expect(page.locator(".hud-hash")).toHaveText(/^[a-f0-9]{64}$/);
  await page.screenshot({ path: "../../artifacts/hud/unsupported.png" });
});
