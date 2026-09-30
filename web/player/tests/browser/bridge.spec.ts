import { test, expect } from "@playwright/test";
import { build, defaultClientConditions } from "vite-plus";
import { resolve } from "node:path";
import { rm } from "node:fs/promises";

test.beforeAll(async () => {
  await build({
    configFile: false,
    logLevel: "error",
    resolve: { conditions: ["evenfall-source", ...defaultClientConditions] },
    define: { "process.env.NODE_ENV": '"development"' },
    build: {
      outDir: "dist/bridge-test",
      emptyOutDir: true,
      lib: {
        entry: resolve("tests/browser/bridge.fixture.tsx"),
        formats: ["es"],
        fileName: () => "fixture.js",
      },
    },
  });
});

test.afterAll(() => rm("dist/bridge-test", { recursive: true, force: true }));

test("worker capture is coherent and owned; React subscriptions follow step and state replacement", async ({
  page,
}) => {
  await page.goto("/");
  await expect(page.getByRole("status").last()).toContainText("Open a game");
  const result = await page.evaluate(async () => {
    const emulator = (
      window as unknown as {
        emulator: import("@evenfall/gba").Emulator;
      }
    ).emulator;
    const unloaded = await emulator.captureMemory();
    await emulator.load(new Uint8Array([42, 0, 160, 227, 254, 255, 255, 234]));
    const before = await emulator.saveState();
    const first = (await emulator.captureMemory())!;
    first.regions[0].bytes[0] = 99;
    const second = (await emulator.captureMemory())!;
    const after = await emulator.saveState();
    const url = "/bridge-test/fixture.js";
    const { mount } = await import(/* @vite-ignore */ url);
    (window as unknown as { unmountBridge: () => void }).unmountBridge = mount(
      emulator,
      first.romHash,
    );
    return {
      unloaded,
      cycle: first.cycle.toString(),
      sameCycle: first.cycle === second.cycle,
      sameTimeline: first.timeline === second.timeline,
      nextSequence: second.sequence - first.sequence,
      regions: second.regions.map(({ address, bytes }) => [
        address,
        bytes.length,
      ]),
      owned: second.regions[0].bytes[0],
      unchanged: before.every((byte, i) => after[i] === byte),
    };
  });
  expect(result).toEqual({
    unloaded: null,
    cycle: "0",
    sameCycle: true,
    sameTimeline: true,
    nextSequence: 1,
    regions: [
      [0x02000000, 0x40000],
      [0x03000000, 0x8000],
    ],
    owned: 0,
    unchanged: true,
  });
  const probes = page.getByLabel("Bridge probe");
  await expect(probes).toHaveCount(2);
  await expect(probes.first()).toHaveText(/^0:0:/);
  const initial = await probes.first().textContent();
  expect(await probes.last().textContent()).toBe(initial);
  await page.evaluate(async () => {
    const emulator = (
      window as unknown as {
        emulator: import("@evenfall/gba").Emulator;
      }
    ).emulator;
    await emulator.setSpeed(8);
    await emulator.pause();
  });
  await expect(probes.first()).toHaveText(initial!);
  await page.evaluate(() =>
    (
      window as unknown as {
        emulator: import("@evenfall/gba").Emulator;
      }
    ).emulator.step(),
  );
  await expect(probes.first()).not.toHaveText(initial!);
  const stepped = await probes.first().textContent();
  expect(stepped!.split(":")[2]).toBe(initial!.split(":")[2]);
  await page.evaluate(async () => {
    const emulator = (
      window as unknown as {
        emulator: import("@evenfall/gba").Emulator;
      }
    ).emulator;
    await emulator.loadState(await emulator.saveState());
  });
  await expect(probes.first()).not.toHaveText(stepped!);
  await expect(probes.first()).toHaveText(/^0:\d+:/);
  expect((await probes.first().textContent())!.split(":")[2]).not.toBe(
    stepped!.split(":")[2],
  );
  await page.evaluate(() =>
    (window as unknown as { unmountBridge: () => void }).unmountBridge(),
  );
  await expect(probes).toHaveCount(0);
});
