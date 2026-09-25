import { test, expect } from "@playwright/test";
import type { Emulator as SdkEmulator } from "@evenfall/gba";

/** Hooks that /debug.html exposes for console and browser tests. */
type DebugHooks = {
  zigbaDebug: {
    Emulator: typeof SdkEmulator;
    acquireSaveLock(key: string): Promise<(() => void) | null>;
  };
};
// Existing developer workflows now live at the separate debug entry point.
const rom = {
  name: "Loop.gba",
  mimeType: "application/octet-stream",
  buffer: Buffer.from([42, 0, 160, 227, 254, 255, 255, 234]),
};
test("fast-forward presents selected complete images while advancing every emulated frame", async ({
  page,
}) => {
  await page.addInitScript(() => {
    const original = CanvasRenderingContext2D.prototype.putImageData;
    (window as unknown as { canvasDraws: number }).canvasDraws = 0;
    CanvasRenderingContext2D.prototype.putImageData = function (
      this: CanvasRenderingContext2D,
      ...args: Parameters<typeof original>
    ) {
      (window as unknown as { canvasDraws: number }).canvasDraws++;
      return original.apply(this, args);
    } as typeof original;
  });
  await page.goto("/debug.html");
  await expect(page.getByRole("status").last()).toContainText("Open a game");
  await page.getByLabel("Open game").setInputFiles(rom);
  await expect(
    page.getByRole("heading", { name: "Loop", exact: true }),
  ).toBeVisible();
  await page.getByRole("button", { name: "Play", exact: true }).click();
  const counts = await page.evaluate(async () => {
    const host = window as unknown as {
      emulator: import("@evenfall/gba").Emulator;
      canvasDraws: number;
    };
    await host.emulator.setSpeed(16);
    let frames = 0,
      audioSamples = 0;
    const count = (event: Event) => {
      frames++;
      audioSamples += (event as CustomEvent).detail.audio.length;
    };
    host.emulator.addEventListener("frame", count);
    const start = host.canvasDraws;
    const before = await host.emulator.inspect();
    await new Promise((resolve) => setTimeout(resolve, 1000));
    const after = await host.emulator.inspect();
    host.emulator.removeEventListener("frame", count);
    return {
      frames,
      audioSamples,
      draws: host.canvasDraws - start,
      emulatedFrames: Number(after.cycle - before.cycle) / 280896,
    };
  });
  expect(counts.emulatedFrames).toBeGreaterThan(120);
  expect(counts.frames).toBeGreaterThan(5);
  expect(counts.frames).toBeLessThan(counts.emulatedFrames / 8);
  expect(counts.audioSamples).toBe(0);
  expect(counts.draws).toBeGreaterThan(0);
  expect(counts.draws).toBeLessThanOrEqual(counts.frames + 2);
  await page.getByRole("button", { name: "Pause", exact: true }).click();
});

test("import, play, refresh, checkpoint, isolated Lab, capture and narrow layout", async ({
  page,
}) => {
  const errors: string[] = [];
  page.on("pageerror", (e) => errors.push(e.message));
  await page.goto("/debug.html");
  await expect(page.getByRole("status").last()).toContainText("Open a game");
  await page.screenshot({ path: "test-results/empty-desktop.png" });
  await page.getByLabel("Open game").setInputFiles(rom);
  await expect(
    page.getByRole("heading", { name: "Loop", exact: true }),
  ).toBeVisible();
  await page.screenshot({ path: "test-results/loaded-desktop.png" });
  await page.getByRole("button", { name: "Play", exact: true }).click();
  await expect(
    page.getByRole("button", { name: "Pause", exact: true }),
  ).toBeVisible();
  await page.keyboard.press("KeyX");
  await page.screenshot({ path: "test-results/playing-desktop.png" });
  await page.getByRole("button", { name: "Pause", exact: true }).click();
  await page.getByText("Checkpoints & exports", { exact: true }).click();
  await page.getByLabel("Checkpoint name").fill("First checkpoint");
  await page.getByRole("button", { name: "Mark checkpoint" }).click();
  await page.getByRole("button", { name: "Prepare scenario" }).click();
  await expect(
    page.getByRole("button", { name: "Download manifest" }),
  ).toBeVisible();
  await page.getByRole("link", { name: "Lab", exact: true }).click();
  await page.getByRole("button", { name: "Loop", exact: true }).click();
  await page.getByRole("button", { name: "Run scenario", exact: true }).click();
  await expect(
    page.getByRole("status").filter({ hasText: /^passed$/ }),
  ).toBeVisible();
  await page.getByRole("link", { name: "Player", exact: true }).click();
  await page.getByRole("button", { name: "Capture bug", exact: true }).click();
  await page.getByLabel("What happened?").fill("Testing capture");
  await page.getByRole("button", { name: "Retain capture" }).click();
  await expect(
    page.getByRole("button", { name: "Download failure bundle" }),
  ).toBeVisible();
  await page.screenshot({ path: "test-results/capture-desktop.png" });
  await page.getByRole("button", { name: "Close", exact: true }).click();
  await page.reload();
  await expect(
    page.getByRole("button", { name: "Loop", exact: true }),
  ).toBeVisible();
  await page.getByRole("button", { name: "Loop", exact: true }).click();
  await expect(
    page.getByRole("heading", { name: "Loop", exact: true }),
  ).toBeVisible();
  await page.setViewportSize({ width: 390, height: 844 });
  await page.reload();
  await expect(page.getByRole("status").last()).toContainText("Open a game");
  await page.screenshot({ path: "test-results/empty-mobile.png" });
  await page.getByRole("button", { name: "Library", exact: true }).click();
  await page.screenshot({ path: "test-results/library-mobile.png" });
  await page.getByRole("button", { name: "Loop", exact: true }).click();
  await page.getByRole("button", { name: "Close", exact: true }).click();
  await page.screenshot({ path: "test-results/loaded-mobile.png" });
  expect(
    await page.evaluate(
      () => document.documentElement.scrollWidth <= window.innerWidth,
    ),
  ).toBe(true);
  expect(errors).toEqual([]);
});
test("worker cancellation, fault capture and save locking across tabs", async ({
  page,
  context,
}) => {
  await page.goto("/debug.html");
  const result = await page.evaluate(async () => {
    const { Emulator } = (window as unknown as DebugHooks).zigbaDebug;
    const a = await Emulator.create();
    const b = await Emulator.create();
    const rom = new Uint8Array([42, 0, 160, 227, 254, 255, 255, 234]);
    await a.load(rom, { rtc: false, saveType: "sram" });
    await a.runUntil(1000n);
    const before = await a.saveState();
    const { manifest, assets } = await a.exportScenario();
    manifest.checkpoints = [{ name: "long", at: "28089600000" }];
    manifest.limits.maxCycles = "28089600000";
    const running = b.runScenario(manifest, assets);
    setTimeout(() => void b.cancel(), 30);
    const cancelled = await running;
    const after = await a.saveState();
    await a.load(new Uint8Array([0, 0, 255, 239]), { rtc: false });
    let captured = false;
    a.addEventListener("capture", () => {
      captured = true;
    });
    await a.runUntil(1000n);
    a.terminate();
    b.terminate();
    return {
      status: cancelled.status,
      same: before.every((v: number, i: number) => v === after[i]),
      captured,
    };
  });
  expect(result).toEqual({ status: "cancelled", same: true, captured: true });
  await page.evaluate(async () => {
    const { acquireSaveLock } = (window as unknown as DebugHooks).zigbaDebug;
    (window as unknown as { release: () => void }).release =
      (await acquireSaveLock("lock-test"))!;
  });
  const second = await context.newPage();
  await second.goto("/debug.html");
  expect(
    await second.evaluate(async () => {
      const { acquireSaveLock } = (window as unknown as DebugHooks).zigbaDebug;
      return (await acquireSaveLock("lock-test")) === null;
    }),
  ).toBe(true);
  await page.close();
  await expect
    .poll(() =>
      second.evaluate(async () => {
        const { acquireSaveLock } = (window as unknown as DebugHooks)
          .zigbaDebug;
        const release = await acquireSaveLock("lock-test");
        release?.();
        return !!release;
      }),
    )
    .toBe(true);
});
test("battery save survives v1 upgrade, gameplay, refresh and duplicate ZIP import", async ({
  page,
}) => {
  const bytes = Buffer.alloc(272);
  [0xe3a0040e, 0xe3a0102a, 0xe5c01000, 0xeafffffe].forEach((n, i) =>
    bytes.writeUInt32LE(n, i * 4),
  );
  bytes.write("SRAM_V110", 256);
  const { createHash } = await import("node:crypto");
  const hash = createHash("sha256").update(bytes).digest("hex");
  await page.goto("/debug.html");
  await page.evaluate(async () => {
    const req = indexedDB.deleteDatabase("zigba-local");
    await new Promise<void>((resolve, reject) => {
      req.onsuccess = () => resolve();
      req.onerror = () => reject(req.error);
    });
  });
  // Seed the previous application's exact record shape before opening the new UI.
  await page.evaluate(async (hash) => {
    const req = indexedDB.open("zigba-local", 1);
    req.onupgradeneeded = () => req.result.createObjectStore("saves");
    const db = await new Promise<IDBDatabase>((resolve, reject) => {
      req.onsuccess = () => resolve(req.result);
      req.onerror = () => reject(req.error);
    });
    const tx = db.transaction("saves", "readwrite");
    const data = new Uint8Array(32768).fill(255);
    data[0] = 17;
    tx.objectStore("saves").put({ data, updatedAt: 1 }, hash);
    await new Promise<void>((resolve) => {
      tx.oncomplete = () => resolve();
    });
    db.close();
  }, hash);
  await page.reload();
  await expect(page.getByRole("status").last()).toContainText("Open a game");
  await page.getByLabel("Open game").setInputFiles({
    name: "Save.gba",
    mimeType: "application/octet-stream",
    buffer: bytes,
  });
  await expect(
    page.getByRole("button", { name: "Play", exact: true }),
  ).toBeEnabled();
  expect(
    await page.evaluate(async () => {
      return (
        await (
          window as unknown as {
            emulator: { exportSave(): Promise<Uint8Array> };
          }
        ).emulator.exportSave()
      )[0];
    }),
  ).toBe(17);
  await page.getByRole("button", { name: "Frame +1" }).click();
  await expect(
    page.getByRole("button", { name: "Play", exact: true }),
  ).toBeEnabled();
  await expect
    .poll(() =>
      page.evaluate(async () => {
        return (
          await (
            window as unknown as {
              emulator: { exportSave(): Promise<Uint8Array> };
            }
          ).emulator.exportSave()
        )[0];
      }),
    )
    .toBe(42);
  // Pause flushes the save before a controlled reload.
  await page.getByRole("button", { name: "Capture bug", exact: true }).click();
  await page.getByRole("button", { name: "Close", exact: true }).click();
  await page.reload();
  await page.getByRole("button", { name: "Save", exact: true }).click();
  await expect(
    page.getByRole("button", { name: "Play", exact: true }),
  ).toBeEnabled();
  expect(
    await page.evaluate(async () => {
      return (
        await (
          window as unknown as {
            emulator: { exportSave(): Promise<Uint8Array> };
          }
        ).emulator.exportSave()
      )[0];
    }),
  ).toBe(42);
  const { crc32 } = await import("node:zlib");
  const name = Buffer.from("Save.gba");
  const local = Buffer.alloc(30);
  local.writeUInt32LE(0x04034b50);
  local.writeUInt32LE(crc32(bytes), 14);
  local.writeUInt32LE(bytes.length, 18);
  local.writeUInt32LE(bytes.length, 22);
  local.writeUInt16LE(name.length, 26);
  const central = Buffer.alloc(46);
  central.writeUInt32LE(0x02014b50);
  central.writeUInt32LE(crc32(bytes), 16);
  central.writeUInt32LE(bytes.length, 20);
  central.writeUInt32LE(bytes.length, 24);
  central.writeUInt16LE(name.length, 28);
  const end = Buffer.alloc(22);
  end.writeUInt32LE(0x06054b50);
  end.writeUInt16LE(1, 8);
  end.writeUInt16LE(1, 10);
  end.writeUInt32LE(central.length + name.length, 12);
  end.writeUInt32LE(local.length + name.length + bytes.length, 16);
  const zipDrop = await page.evaluateHandle(
    (bytes) => {
      const transfer = new DataTransfer();
      transfer.items.add(
        new File([new Uint8Array(bytes)], "Copy.zip", {
          type: "application/zip",
        }),
      );
      return transfer;
    },
    [...Buffer.concat([local, name, bytes, central, name, end])],
  );
  await page.locator("main").dispatchEvent("drop", { dataTransfer: zipDrop });
  await zipDrop.dispose();
  await expect(
    page.getByRole("button", { name: "Play", exact: true }),
  ).toBeEnabled();
  expect(await page.locator(".game").count()).toBe(1);
  await expect(page.locator(".save-status")).toContainText("Autosave active");
  await page.locator(".game summary").click();
  await page
    .getByRole("button", { name: "Delete progress", exact: true })
    .click();
  await expect(
    page.getByRole("button", { name: "Play", exact: true }),
  ).toBeEnabled();
  expect(
    await page.evaluate(async () => {
      return (
        await (
          window as unknown as {
            emulator: { exportSave(): Promise<Uint8Array> };
          }
        ).emulator.exportSave()
      )[0];
    }),
  ).toBe(255);
});
test("storage failure leaves game and export controls available", async ({
  page,
}) => {
  await page.addInitScript(() => {
    Object.defineProperty(window, "indexedDB", {
      get() {
        throw new Error("Storage disabled for test");
      },
    });
  });
  await page.goto("/debug.html");
  await expect(page.locator(".save-status")).toContainText(
    "Storage unavailable",
  );
  await page.getByText("Boot options", { exact: true }).click();
  await page.getByLabel("Save type").selectOption("sram");
  await page.getByLabel("Open game").setInputFiles(rom);
  await expect(
    page.getByRole("button", { name: "Play", exact: true }),
  ).toBeEnabled();
  await page.getByText("Checkpoints & exports", { exact: true }).click();
  await expect(
    page.getByRole("button", { name: "Export save", exact: true }),
  ).toBeEnabled();
  await page.getByLabel("Import save", { exact: true }).setInputFiles({
    name: "Progress.sav",
    mimeType: "application/octet-stream",
    buffer: Buffer.alloc(32768, 17),
  });
  await expect(
    page.getByRole("button", { name: "Play", exact: true }),
  ).toBeEnabled();
  await expect
    .poll(() =>
      page.evaluate(async () => {
        const emulator = (
          window as unknown as {
            emulator: import("@evenfall/gba").Emulator;
          }
        ).emulator;
        return (await emulator.exportSave())[0];
      }),
    )
    .toBe(17);
  await page.getByRole("button", { name: "Frame +1" }).click();
  await expect(
    page.getByRole("button", { name: "Play", exact: true }),
  ).toBeEnabled();
  expect(
    await page.evaluate(async () =>
      String(
        (
          await (
            window as unknown as {
              emulator: { inspect(): Promise<{ cycle: bigint }> };
            }
          ).emulator.inspect()
        ).cycle,
      ),
    ),
  ).not.toBe("0");
});

test("v2 library migrates to metadata and reads only the selected ROM", async ({
  page,
}) => {
  const { createHash } = await import("node:crypto");
  const second = Buffer.from(rom.buffer);
  second[0] = 43;
  const games = [rom.buffer, second].map((bytes, index) => ({
    hash: createHash("sha256").update(bytes).digest("hex"),
    title: `Game ${index + 1}`,
    rom: [...bytes],
    lastPlayed: index + 1,
  }));
  await page.goto("/debug.html");
  await expect(page.getByRole("status").last()).toContainText("Open a game");
  await page.evaluate(async (games) => {
    await new Promise<void>((resolve, reject) => {
      const request = indexedDB.deleteDatabase("zigba-local");
      request.onsuccess = () => resolve();
      request.onerror = () => reject(request.error);
    });
    const request = indexedDB.open("zigba-local", 2);
    request.onupgradeneeded = () => {
      for (const name of ["games", "saves", "assets", "scenarios", "reports"])
        request.result.createObjectStore(name);
    };
    const db = await new Promise<IDBDatabase>((resolve, reject) => {
      request.onsuccess = () => resolve(request.result);
      request.onerror = () => reject(request.error);
    });
    const tx = db.transaction("games", "readwrite");
    for (const game of games)
      tx.objectStore("games").put(
        { ...game, rom: new Uint8Array(game.rom) },
        game.hash,
      );
    await new Promise<void>((resolve, reject) => {
      tx.oncomplete = () => resolve();
      tx.onabort = () => reject(tx.error);
    });
    db.close();
  }, games);
  await page.addInitScript(() => {
    const reads: string[] = [];
    const get = IDBObjectStore.prototype.get;
    IDBObjectStore.prototype.get = function (key) {
      if (this.name === "roms") reads.push(String(key));
      return get.call(this, key);
    };
    (window as unknown as { romReads: string[] }).romReads = reads;
  });
  await page.reload();
  await expect(
    page.getByRole("button", { name: "Game 1", exact: true }),
  ).toBeVisible();
  await expect(
    page.getByRole("button", { name: "Game 2", exact: true }),
  ).toBeVisible();
  const reads = () =>
    page.evaluate(() => (window as unknown as { romReads: string[] }).romReads);
  expect(await reads()).toEqual([]);
  const metadata = await page.evaluate(async () => {
    const request = indexedDB.open("zigba-local");
    const db = await new Promise<IDBDatabase>((resolve) => {
      request.onsuccess = () => resolve(request.result);
    });
    const records = db.transaction("games").objectStore("games").getAll();
    const result = await new Promise<{ romSize: number; rom?: Uint8Array }[]>(
      (resolve) => {
        records.onsuccess = () => resolve(records.result);
      },
    );
    db.close();
    return result.map((game) => ({
      size: game.romSize,
      containsRom: Object.hasOwn(game, "rom"),
    }));
  });
  expect(metadata).toEqual([
    { size: 8, containsRom: false },
    { size: 8, containsRom: false },
  ]);
  await page.getByRole("button", { name: "Game 2", exact: true }).click();
  await expect(
    page.getByRole("heading", { name: "Game 2", exact: true }),
  ).toBeVisible();
  await page.getByRole("button", { name: "Frame +1" }).click();
  await expect(
    page.getByRole("button", { name: "Play", exact: true }),
  ).toBeEnabled();
  expect(await reads()).toEqual([games[1].hash]);
  expect(
    await page.evaluate(async () => {
      const emulator = (
        window as unknown as {
          emulator: import("@evenfall/gba").Emulator;
        }
      ).emulator;
      return (await emulator.inspect()).registers[0];
    }),
  ).toBe(43);
  const entry = page
    .locator(".game")
    .filter({ has: page.getByRole("button", { name: "Game 2", exact: true }) });
  await entry.locator("summary").click();
  await page.getByLabel("Rename Game 2", { exact: true }).fill("Renamed");
  await entry.getByRole("button", { name: "Rename", exact: true }).click();
  await expect(
    page.getByRole("button", { name: "Renamed", exact: true }),
  ).toBeVisible();
  expect(await reads()).toEqual([games[1].hash]);
  await page
    .locator(".game")
    .filter({ has: page.getByRole("button", { name: "Renamed", exact: true }) })
    .getByRole("button", { name: "Remove game", exact: true })
    .click();
  await expect(
    page.getByRole("button", { name: "Renamed", exact: true }),
  ).toHaveCount(0);
  await expect(
    page.getByRole("button", { name: "Game 1", exact: true }),
  ).toBeVisible();
  expect(await reads()).toEqual([games[1].hash]);
});

test("direct Lab route inspects a failed assertion, records a baseline and replays failure", async ({
  page,
}, testInfo) => {
  const { createHash } = await import("node:crypto");
  const manifest = {
    version: 1,
    name: "Assertion fixture",
    assets: {
      rom: {
        path: "rom.gba",
        sha256: createHash("sha256").update(rom.buffer).digest("hex"),
      },
    },
    config: {
      boot: "hle",
      saveType: "none",
      rtc: false,
      rtcEpoch: "0",
      sampleRate: 48000,
    },
    inputs: [],
    checkpoints: [
      {
        name: "r0 check",
        at: "1000",
        expect: { cpu: { registers: { "0": 43 } } },
      },
    ],
    limits: {
      maxCycles: "1000",
      maxRuns: 100,
      chunkCycles: "70224",
      maxAudioBytes: 1048576,
    },
  };
  const errors: string[] = [];
  page.on("pageerror", (e) => errors.push(e.message));
  await page.goto("/debug.html#/lab");
  await expect(
    page.getByRole("heading", { name: "Scenarios", exact: true }),
  ).toBeVisible();
  await page.getByLabel("Import manifest or failure").setInputFiles({
    name: "scenario.json",
    mimeType: "application/json",
    buffer: Buffer.from(JSON.stringify(manifest)),
  });
  await page.getByLabel("Choose rom").setInputFiles(rom);
  await page.getByRole("button", { name: "Run scenario", exact: true }).click();
  await expect(
    page.getByRole("status").filter({ hasText: /^failed$/ }),
  ).toBeVisible();
  await expect(
    page.getByText("r0 check: r0 expected 43, got 42", { exact: false }),
  ).toBeVisible();
  await page.screenshot({
    path: testInfo.outputPath("lab-failure-desktop.png"),
    fullPage: true,
  });
  await page.setViewportSize({ width: 390, height: 844 });
  await page.screenshot({
    path: testInfo.outputPath("lab-failure-mobile.png"),
    fullPage: true,
  });
  expect(
    await page.evaluate(
      () => document.documentElement.scrollWidth <= innerWidth,
    ),
  ).toBe(true);
  const downloadPromise = page.waitForEvent("download");
  await page
    .getByRole("button", { name: "Download failure bundle", exact: true })
    .click();
  const failure = await downloadPromise;
  const failurePath = testInfo.outputPath("failure.json");
  await failure.saveAs(failurePath);
  await page
    .getByRole("button", { name: "Record baseline", exact: true })
    .click();
  await expect(
    page.getByRole("heading", {
      name: "Assertion fixture — regression baseline",
      exact: true,
    }),
  ).toBeVisible();
  await page.getByRole("button", { name: "Run scenario", exact: true }).click();
  await expect(
    page.getByRole("status").filter({ hasText: /^passed$/ }),
  ).toBeVisible();
  await page
    .getByLabel("Import manifest or failure")
    .setInputFiles(failurePath);
  await page
    .getByRole("button", { name: "Replay failure", exact: true })
    .click();
  await expect(
    page.getByRole("status").filter({ hasText: /^passed$/ }),
  ).toBeVisible();
  expect(errors).toEqual([]);
});

test("keyboard focus, focus loss, touch release and visibility pause", async ({
  page,
}) => {
  await page.goto("/debug.html");
  await expect(page.getByRole("status").last()).toContainText("Open a game");
  await page.getByLabel("Open game").setInputFiles(rom);
  await expect(
    page.getByRole("button", { name: "Play", exact: true }),
  ).toBeEnabled();
  const keys = () =>
    page.evaluate(async () => {
      const bytes = await (
        window as unknown as {
          emulator: { readMemory(a: number, n: number): Promise<Uint8Array> };
        }
      ).emulator.readMemory(0x04000130, 2);
      return 1023 ^ (bytes[0] | (bytes[1] << 8));
    });
  await page.locator("canvas").first().focus();
  await page.keyboard.down("KeyX");
  await expect.poll(keys).toBe(1);
  await page.evaluate(() => window.dispatchEvent(new Event("blur")));
  await expect.poll(keys).toBe(0);
  await page.keyboard.up("KeyX");
  await page.getByLabel("Search games").fill("x");
  await page.keyboard.press("KeyX");
  expect(await keys()).toBe(0);
  await page.setViewportSize({ width: 390, height: 844 });
  await page.getByRole("button", { name: "Close", exact: true }).click();
  const a = page.getByRole("button", { name: "A", exact: true });
  const box = await a.boundingBox();
  await page.mouse.move(box!.x + box!.width / 2, box!.y + box!.height / 2);
  await page.mouse.down();
  await expect.poll(keys).toBe(1);
  await page.mouse.up();
  await expect.poll(keys).toBe(0);
  await page.getByRole("button", { name: "Play", exact: true }).click();
  await expect(
    page.getByRole("button", { name: "Pause", exact: true }),
  ).toBeVisible();
  await page.evaluate(() => {
    Object.defineProperty(document, "hidden", {
      configurable: true,
      get: () => true,
    });
    document.dispatchEvent(new Event("visibilitychange"));
  });
  await expect(
    page.getByRole("button", { name: "Play", exact: true }),
  ).toBeVisible();
});

test("nonzero wave audio reaches the bounded browser output and sound can be muted", async ({
  page,
}) => {
  // Same register program as probe/wave_ramp.c, emitted as a tiny ARM fixture.
  const writes: number[][] = [];
  for (let h = 0; h < 8; h++) {
    const lo = ((4 * h) % 16 << 4) | (4 * h + 1) % 16;
    const hi = ((4 * h + 2) % 16 << 4) | (4 * h + 3) % 16;
    writes.push([0x90 + h * 2, (hi << 8) | lo]);
  }
  writes.push(
    [0x84, 0x80],
    [0x80, 0x4477],
    [0x82, 2],
    [0x70, 0xc0],
    [0x72, 0x2000],
    [0x74, 0x8700],
  );
  const code = [0xe3a00301];
  const literals = (2 + writes.length * 2) * 4;
  for (let i = 0; i < writes.length; i++) {
    const [offset, value] = writes[i];
    const pc = code.length * 4 + 8;
    code.push(
      (0xe59f1000 | (literals + i * 4 - pc)) >>> 0,
      (0xe1c010b0 | ((offset & 0xf0) << 4) | (offset & 15)) >>> 0,
    );
  }
  code.push(0xeafffffe, ...writes.map((w) => w[1]));
  const bytes = Buffer.alloc(code.length * 4);
  code.forEach((n, i) => bytes.writeUInt32LE(n, i * 4));
  await page.addInitScript(() => {
    const evidence = { buffers: 0, nonzero: false };
    const post = MessagePort.prototype.postMessage;
    MessagePort.prototype.postMessage = function (
      message: unknown,
      options?: StructuredSerializeOptions | Transferable[],
    ) {
      const samples = (message as { samples?: Int16Array })?.samples;
      evidence.nonzero ||=
        samples instanceof Int16Array && samples.some((v) => v !== 0);
      return post.call(
        this,
        message,
        Array.isArray(options) ? { transfer: options } : options,
      );
    };
    (window as unknown as { audioEvidence: typeof evidence }).audioEvidence =
      evidence;
    const start = AudioBufferSourceNode.prototype.start;
    AudioBufferSourceNode.prototype.start = function (
      ...args: Parameters<AudioBufferSourceNode["start"]>
    ) {
      evidence.buffers++;
      evidence.nonzero ||= !!this.buffer
        ?.getChannelData(0)
        .some((v) => v !== 0);
      return start.apply(this, args);
    };
  });
  await page.goto("/debug.html");
  await expect(page.getByRole("status").last()).toContainText("Open a game");
  await page.getByLabel("Open game").setInputFiles({
    name: "Wave.gba",
    mimeType: "application/octet-stream",
    buffer: bytes,
  });
  await page.getByRole("button", { name: "Play", exact: true }).click();
  await expect
    .poll(() =>
      page.evaluate(
        () =>
          (window as unknown as { audioEvidence: { nonzero: boolean } })
            .audioEvidence.nonzero,
      ),
    )
    .toBe(true);
  await page.getByRole("button", { name: "Sound on", exact: true }).click();
  await expect(
    page.getByRole("button", { name: "Sound off", exact: true }),
  ).toHaveAttribute("aria-pressed", "true");
  await page.getByRole("button", { name: "Pause", exact: true }).click();
});

test("legacy browser SDK validation remains runnable against the new player", async ({
  page,
}) => {
  await page.goto("/validation.html");
  await expect(page.locator("#result")).toHaveAttribute("data-result", "pass", {
    timeout: 30000,
  });
  await expect(page.locator("#result")).toContainText(
    "PASS AudioWorklet stereo rendering",
  );
  await expect(page.locator("#result")).toContainText(
    "PASS browser integration complete",
  );
});

test("player detects RTC cartridges and pins the current boot clock in exports", async ({
  page,
}) => {
  const bytes = Buffer.alloc(256);
  bytes.writeUInt32LE(0xeafffffe, 0);
  bytes.write("AXVE", 0xac);
  await page.goto("/debug.html");
  await expect(page.getByRole("status").last()).toContainText("Open a game");
  const before = Math.floor(Date.now() / 1000);
  await page.getByLabel("Open game").setInputFiles({
    name: "Clock.gba",
    mimeType: "application/octet-stream",
    buffer: bytes,
  });
  await expect(
    page.getByRole("button", { name: "Play", exact: true }),
  ).toBeEnabled();
  const config = await page.evaluate(async () => {
    const emulator = (
      window as unknown as {
        emulator: import("@evenfall/gba").Emulator;
      }
    ).emulator;
    return {
      cpu: await emulator.inspect(),
      config: (await emulator.exportScenario()).manifest.config,
    };
  });
  expect(config.cpu.rtcEnabled).toBe(true);
  expect(config.config.rtc).toBe(true);
  expect(Number(config.config.rtcEpoch)).toBeGreaterThanOrEqual(before);
  expect(Number(config.config.rtcEpoch)).toBeLessThanOrEqual(
    Math.floor(Date.now() / 1000),
  );
  await page.getByText("Boot options", { exact: true }).click();
  await page.getByLabel("Cartridge clock", { exact: true }).selectOption("off");
  await page.getByLabel("Fixed clock epoch (UTC seconds)").fill("946684800");
  await page.getByRole("button", { name: "Clock", exact: true }).click();
  await expect(
    page.getByRole("button", { name: "Play", exact: true }),
  ).toBeEnabled();
  const pinned = await page.evaluate(async () => {
    const emulator = (
      window as unknown as {
        emulator: import("@evenfall/gba").Emulator;
      }
    ).emulator;
    return (await emulator.exportScenario()).manifest.config;
  });
  expect(pinned.rtc).toBe(false);
  expect(pinned.rtcEpoch).toBe("946684800");
  await page.getByLabel("Cartridge clock", { exact: true }).selectOption("on");
  await page.getByRole("button", { name: "Clock", exact: true }).click();
  await expect(
    page.getByRole("button", { name: "Play", exact: true }),
  ).toBeEnabled();
  expect(
    await page.evaluate(
      async () =>
        (
          await (
            window as unknown as {
              emulator: import("@evenfall/gba").Emulator;
            }
          ).emulator.inspect()
        ).rtcEnabled,
    ),
  ).toBe(true);
});

test("main area accepts dropped ROMs and exposes one-second performance diagnostics", async ({
  page,
}, testInfo) => {
  await page.goto("/debug.html");
  await expect(page.getByRole("status").last()).toContainText("Open a game");
  const stats = page.getByLabel("Player performance");
  await expect(stats).toContainText("0.0 FPS");
  const drop = async (name: string, bytes: Buffer) => {
    const transfer = await page.evaluateHandle(
      ({ name, bytes }) => {
        const dt = new DataTransfer();
        dt.items.add(new File([new Uint8Array(bytes)], name));
        return dt;
      },
      { name, bytes: [...bytes] },
    );
    await page
      .locator("main")
      .dispatchEvent("dragenter", { dataTransfer: transfer });
    await expect(page.locator(".drop-hint")).toBeVisible();
    await page
      .locator("canvas")
      .first()
      .dispatchEvent("drop", { dataTransfer: transfer });
    await expect(page.locator(".drop-hint")).toHaveCount(0);
    await transfer.dispose();
  };
  await drop("Bad.txt", rom.buffer);
  await expect(page.getByRole("status").last()).toContainText("Drop one .gba");
  await expect(
    page.getByRole("button", { name: "Play", exact: true }),
  ).toBeDisabled();
  await drop("Dropped.GBA", rom.buffer);
  await expect(
    page.getByRole("heading", { name: "Dropped", exact: true }),
  ).toBeVisible();
  await expect(
    page.getByRole("button", { name: "Play", exact: true }),
  ).toBeEnabled();
  await page.getByRole("button", { name: "Play", exact: true }).click();
  await expect
    .poll(async () => Number(await page.getByTestId("fps").textContent()))
    .toBeGreaterThan(0);
  await expect
    .poll(async () => Number(await page.getByTestId("frame-ms").textContent()))
    .toBeGreaterThan(0);
  const cadence = await page.evaluate(async () => {
    const emulator = (
      window as unknown as {
        emulator: import("@evenfall/gba").Emulator;
      }
    ).emulator;
    let frames = 0;
    const states = new Set<string>();
    const changes: number[] = [];
    const listener = (event: Event) => {
      frames++;
      states.add(String((event as CustomEvent).detail.state.cycle));
    };
    const observer = new MutationObserver(() =>
      changes.push(performance.now()),
    );
    observer.observe(document.querySelector(".performance")!, {
      subtree: true,
      characterData: true,
    });
    emulator.addEventListener("frame", listener);
    await new Promise((resolve) => setTimeout(resolve, 2300));
    emulator.removeEventListener("frame", listener);
    observer.disconnect();
    return {
      frames,
      states: states.size,
      intervals: changes.slice(1).map((t, i) => t - changes[i]),
      updates: changes.length,
    };
  });
  expect(cadence.frames).toBeGreaterThan(10);
  expect(cadence.states).toBeLessThanOrEqual(4);
  expect(cadence.updates).toBeGreaterThanOrEqual(1);
  expect(cadence.updates).toBeLessThanOrEqual(3);
  for (const interval of cadence.intervals)
    expect(interval).toBeGreaterThan(850);
  await page.screenshot({
    path: testInfo.outputPath("performance-desktop.png"),
  });
  await page.getByRole("button", { name: "Fullscreen", exact: true }).click();
  await expect
    .poll(() => page.evaluate(() => !!document.fullscreenElement))
    .toBe(true);
  await expect(stats).toBeInViewport();
  await page.screenshot({
    path: testInfo.outputPath("performance-fullscreen.png"),
  });
  await page.evaluate(() => document.exitFullscreen());
  await page.evaluate(() => window.scrollTo(0, document.body.scrollHeight));
  // Performance belongs to the player, not a floating viewport overlay that
  // would cover game-specific panels when the player scrolls out of view.
  await expect(stats).not.toBeInViewport();
  await page.evaluate(() => window.scrollTo(0, 0));
  await page.setViewportSize({ width: 390, height: 844 });
  await page.getByRole("button", { name: "Close", exact: true }).click();
  await expect(stats).toBeInViewport();
  await page.screenshot({
    path: testInfo.outputPath("performance-mobile.png"),
  });
  await page.getByRole("button", { name: "Pause", exact: true }).click();
  await expect(page.getByTestId("fps")).toHaveText("0.0");
  await expect(page.getByTestId("frame-ms")).toHaveText("0.00");
});

test("visible key guide, speed slider and hold-Tab boost preserve focus and machine timing", async ({
  page,
}, testInfo) => {
  await page.goto("/debug.html");
  await expect(page.getByRole("status").last()).toContainText("Open a game");
  const guide = page.getByLabel("Keyboard controls");
  await expect(guide).toBeVisible();
  for (const name of ["Move", "Start", "Select", "Hold for 16×"])
    await expect(guide).toContainText(name);
  const speed = page.getByRole("slider", { name: "Speed", exact: true });
  await expect(speed).toHaveValue("1");
  await speed.focus();
  await page.keyboard.press("Home");
  await expect(speed).toHaveValue("0.5");
  await page.getByLabel("Open game").setInputFiles(rom);
  await page.getByRole("button", { name: "Play", exact: true }).click();
  const frames = () =>
    page.evaluate(async () =>
      Number(
        (
          await (
            window as unknown as {
              emulator: import("@evenfall/gba").Emulator;
            }
          ).emulator.inspect()
        ).frames,
      ),
    );
  const start = await frames();
  await page.waitForTimeout(500);
  const slow = (await frames()) - start;
  await speed.fill("1");
  await page.waitForTimeout(100);
  const normalStart = await frames();
  await page.waitForTimeout(500);
  const normal = (await frames()) - normalStart;
  await page.locator("canvas").first().focus();
  await page.keyboard.down("Tab");
  await expect(page.getByLabel("Effective speed")).toHaveText("16× BOOST");
  const boostedStart = await frames();
  await page.waitForTimeout(500);
  const boosted = (await frames()) - boostedStart;
  expect(slow).toBeGreaterThan(0);
  expect(normal).toBeGreaterThan(slow * 1.4);
  expect(boosted).toBeGreaterThan(slow * 2);
  expect(boosted).toBeGreaterThan(normal * 1.5);
  await page.keyboard.up("Tab");
  await expect(page.getByLabel("Effective speed")).toHaveText("1×");
  await speed.fill("4");
  await expect(page.getByLabel("Effective speed")).toHaveText("4×");
  const sliderStart = await frames();
  await page.waitForTimeout(500);
  expect((await frames()) - sliderStart).toBeGreaterThan(normal * 1.5);
  await speed.fill("0.5");
  await page.locator("canvas").first().focus();
  await expect(page.getByLabel("Effective speed")).toHaveText("0.5×");
  await page.keyboard.down("Tab");
  await page.evaluate(() => window.dispatchEvent(new Event("blur")));
  await expect(page.getByLabel("Effective speed")).toHaveText("0.5×");
  await page.keyboard.up("Tab");
  await page.getByLabel("Search games").focus();
  await page.keyboard.down("Tab");
  await expect(page.getByLabel("Effective speed")).toHaveText("0.5×");
  await page.keyboard.up("Tab");
  await expect(page.getByLabel("Search games")).not.toBeFocused();
  await page.locator("canvas").first().focus();
  await page.keyboard.down("Tab");
  await page.getByRole("link", { name: "Lab", exact: true }).click();
  await page.keyboard.up("Tab");
  await page.getByRole("link", { name: "Player", exact: true }).click();
  await expect(page.getByLabel("Effective speed")).toHaveText("0.5×");
  await expect(
    page.getByRole("button", { name: "Play", exact: true }),
  ).toBeVisible();
  await speed.focus();
  await page.keyboard.press("End");
  await expect(speed).toHaveValue("16");
  await expect(page.getByLabel("Effective speed")).toHaveText("16×");
  await guide.scrollIntoViewIfNeeded();
  await page.screenshot({ path: testInfo.outputPath("controls-desktop.png") });
  await page.setViewportSize({ width: 390, height: 844 });
  await page.getByRole("button", { name: "Close", exact: true }).click();
  await guide.scrollIntoViewIfNeeded();
  await page.screenshot({ path: testInfo.outputPath("controls-mobile.png") });
  expect(
    await page.evaluate(
      () => document.documentElement.scrollWidth <= innerWidth,
    ),
  ).toBe(true);
  const parity = await page.evaluate(async () => {
    const { Emulator } = (window as unknown as DebugHooks).zigbaDebug;
    const a = await Emulator.create(),
      b = await Emulator.create();
    try {
      const rom = new Uint8Array([42, 0, 160, 227, 254, 255, 255, 234]);
      await a.load(rom);
      await b.load(rom);
      await a.setSpeed(0.5);
      await b.setSpeed(16);
      await a.runUntil(2808960n);
      await b.runUntil(2808960n);
      const left = await a.saveState(),
        right = await b.saveState();
      let rejected = false;
      try {
        await a.setSpeed(0);
      } catch {
        rejected = true;
      }
      return {
        equal:
          left.length === right.length &&
          left.every((n: number, i: number) => n === right[i]),
        rejected,
      };
    } finally {
      a.terminate();
      b.terminate();
    }
  });
  expect(parity).toEqual({ equal: true, rejected: true });
});

test("recycles worker presentation buffers across play, pause and save restoration", async ({
  page,
}) => {
  await page.goto("/debug.html");
  const result = await page.evaluate(async () => {
    const { Emulator } = (window as unknown as DebugHooks).zigbaDebug;
    const emulator = await Emulator.create();
    const errors: string[] = [];
    emulator.addEventListener("error", (event: Event) =>
      errors.push((event as CustomEvent).detail.message),
    );
    let frames = 0,
      allocations = 0,
      audioSamples = 0;
    emulator.addEventListener("frame", (event: Event) => {
      const { pixels, audio, presentationAllocations } = (event as CustomEvent)
        .detail;
      frames++;
      audioSamples += audio.length;
      allocations = presentationAllocations;
      emulator.recyclePresentation(pixels, audio);
    });
    try {
      await emulator.load(
        new Uint8Array([42, 0, 160, 227, 254, 255, 255, 234]),
        { saveType: "sram", rtc: false },
      );
      await emulator.setSpeed(16);
      for (let session = 0; session < 2; session++) {
        const start = frames;
        await emulator.play();
        const deadline = performance.now() + 10000;
        while (frames - start < 30) {
          if (performance.now() > deadline) throw new Error("Frames stopped");
          await new Promise((resolve) => setTimeout(resolve, 10));
        }
        await emulator.pause();
        await emulator.resetWithSave(new Uint8Array(32768).fill(0x5a));
      }
      const save = await emulator.exportSave();
      const recording = await emulator.recording();
      return {
        errors,
        allocations,
        audioSamples,
        frames,
        saved: save.every((byte: number) => byte === 0x5a),
        recording: recording.enabled,
      };
    } finally {
      await emulator.dispose();
    }
  });
  expect(result.errors).toEqual([]);
  expect(result.frames).toBeGreaterThanOrEqual(60);
  expect(result.allocations).toBeLessThanOrEqual(6);
  expect(result.audioSamples).toBe(0);
  expect(result.saved).toBe(true);
  expect(result.recording).toBe(true);
});
