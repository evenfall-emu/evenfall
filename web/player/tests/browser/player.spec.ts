import { test, expect, type Page } from "@playwright/test";
import type { Emulator as SdkEmulator, Scenario } from "@evenfall/gba";

/** Hooks that /validation.html exposes for SDK and storage tests. */
type ValidationHooks = {
  zigbaValidation: {
    Emulator: typeof SdkEmulator;
    acquireSaveLock(key: string): Promise<(() => void) | null>;
  };
};
const rom = {
  name: "Loop.gba",
  mimeType: "application/octet-stream",
  buffer: Buffer.from([42, 0, 160, 227, 254, 255, 255, 234]),
};
async function ready(page: Page) {
  await page.goto("/");
  await page.getByRole("button", { name: "Open player" }).click();
  await expect(page.getByLabel("Open ROM", { exact: true })).toBeEnabled();
}
async function openRom(page: Page, file: typeof rom) {
  await page.getByLabel("Open ROM", { exact: true }).setInputFiles(file);
  await expect(
    page.getByRole("heading", {
      name: file.name.replace(/\.[^.]+$/, ""),
      exact: true,
    }),
  ).toBeVisible();
}
const playing = (page: Page) =>
  page.evaluate(() => window.player.state.playing);
const exportedSave = (page: Page) =>
  page.evaluate(async () => (await window.player.emulator.exportSave())[0]);
const register = (page: Page, index: number) =>
  page.evaluate(
    async (index) => (await window.player.emulator.inspect()).registers[index],
    index,
  );
const frames = (page: Page) =>
  page.evaluate(async () =>
    Number((await window.player.emulator.inspect()).frames),
  );
/** The SDK harness page; its load event waits for the validation run. */
async function sdkPage(page: Page) {
  await page.goto("/validation.html");
  await expect(page.locator("#result")).not.toHaveText("RUNNING");
}
/** A cartridge with an SRAM signature whose code runs before the signature. */
function sramRom(code: number[]) {
  const bytes = Buffer.alloc(272);
  code.forEach((n, i) => bytes.writeUInt32LE(n, i * 4));
  bytes.write("SRAM_V110", 256);
  return bytes;
}

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
  await ready(page);
  await openRom(page, rom);
  await expect.poll(() => playing(page)).toBe(true);
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
  await page.evaluate(() => window.player.pause());
});

test("worker cancellation, fault capture and save locking across tabs", async ({
  page,
  context,
}) => {
  await sdkPage(page);
  const result = await page.evaluate(async () => {
    const { Emulator } = (window as unknown as ValidationHooks).zigbaValidation;
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
    const { acquireSaveLock } = (window as unknown as ValidationHooks)
      .zigbaValidation;
    (window as unknown as { release: () => void }).release =
      (await acquireSaveLock("lock-test"))!;
  });
  const second = await context.newPage();
  await sdkPage(second);
  expect(
    await second.evaluate(async () => {
      const { acquireSaveLock } = (window as unknown as ValidationHooks)
        .zigbaValidation;
      return (await acquireSaveLock("lock-test")) === null;
    }),
  ).toBe(true);
  await page.close();
  await expect
    .poll(() =>
      second.evaluate(async () => {
        const { acquireSaveLock } = (window as unknown as ValidationHooks)
          .zigbaValidation;
        const release = await acquireSaveLock("lock-test");
        release?.();
        return !!release;
      }),
    )
    .toBe(true);
});

test("a recorded session replays in an isolated worker, and a failed assertion captures a replayable failure", async ({
  page,
}) => {
  const { createHash } = await import("node:crypto");
  const manifest: Scenario = {
    version: 2,
    name: "Assertion fixture",
    assets: {
      rom: {
        path: "rom.gba",
        sha1: createHash("sha1").update(rom.buffer).digest("hex"),
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
  await sdkPage(page);
  const result = await page.evaluate(async (manifest) => {
    const { Emulator } = (window as unknown as ValidationHooks).zigbaValidation;
    const rom = new Uint8Array([42, 0, 160, 227, 254, 255, 255, 234]);
    // Each run gets its own worker, as a host keeps tests off the live session.
    const isolated = async <T>(run: (worker: SdkEmulator) => Promise<T>) => {
      const worker = await Emulator.create();
      try {
        return await run(worker);
      } finally {
        worker.terminate();
      }
    };
    const session = await Emulator.create();
    try {
      await session.load(rom, { rtc: false });
      await session.runUntil(1000n);
      await session.checkpoint("First checkpoint");
      await session.runUntil(2000n);
      const recorded = await session.exportScenario("Loop");
      const replayed = await isolated((worker) =>
        worker.runScenario(recorded.manifest, recorded.assets),
      );
      const manual = await session.capture("Testing capture");
      const failed = await isolated((worker) =>
        worker.runScenario(manifest, { rom }),
      );
      const baseline = structuredClone(manifest);
      for (const checkpoint of baseline.checkpoints) delete checkpoint.expect;
      const unasserted = await isolated((worker) =>
        worker.runScenario(baseline, { rom }),
      );
      const reproduced = await isolated(async (worker) => {
        await worker.load(rom, {
          ...manifest.config,
          rtcEpoch: BigInt(manifest.config.rtcEpoch),
        });
        return worker.replay(failed.capture!);
      });
      return {
        recorded: recorded.manifest.checkpoints.map(({ name }) => name),
        replayed: replayed.status,
        manual: {
          format: manual.format,
          framebuffer: manual.artifacts.framebuffer?.length,
        },
        failed: failed.status,
        failures: failed.failures,
        capture: failed.capture?.format,
        unasserted: unasserted.status,
        checkpoint: unasserted.checkpoints?.[0]?.name,
        reproduced: reproduced.status,
      };
    } finally {
      session.terminate();
    }
  }, manifest);
  expect(result.recorded).toContain("First checkpoint");
  expect(result).toMatchObject({
    replayed: "passed",
    manual: { format: "zigba-failure", framebuffer: 240 * 160 },
    failed: "failed",
    capture: "zigba-failure",
    unasserted: "passed",
    checkpoint: "r0 check",
    reproduced: "passed",
  });
  expect(result.failures?.join("\n")).toContain(
    "r0 check: r0 expected 43, got 42",
  );
});

test("an older library is dropped; a new battery save survives gameplay, refresh and duplicate ZIP import", async ({
  page,
}) => {
  // Reads the boot-time SRAM byte into r2, then writes 42 to it.
  const bytes = sramRom([
    0xe3a0040e, 0xe5d02000, 0xe3a0102a, 0xe5c01000, 0xeafffffe,
  ]);
  const { createHash } = await import("node:crypto");
  const hash = createHash("sha1").update(bytes).digest("hex");
  await page.goto("/");
  await expect
    .poll(() => page.evaluate(() => window.player?.state.ready))
    .toBe(true);
  await page.evaluate(async () => {
    const req = indexedDB.deleteDatabase("zigba-local");
    await new Promise<void>((resolve, reject) => {
      req.onsuccess = () => resolve();
      req.onerror = () => reject(req.error);
    });
  });
  // Seed a version-1 save; libraries before version 7 are keyed by SHA-256
  // and start empty.
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
  await page.goto("/");
  await page.getByRole("button", { name: "Open player" }).click();
  await openRom(page, {
    name: "Save.gba",
    mimeType: "application/octet-stream",
    buffer: bytes,
  });
  // The dropped save leaves erased SRAM.
  await expect.poll(() => register(page, 2)).toBe(255);
  await expect.poll(() => exportedSave(page)).toBe(42);
  // Pause flushes the save before a controlled reload.
  await page.evaluate(() => window.player.pause());
  await page.goto("/");
  await page.getByRole("button", { name: "Resume adventure" }).click();
  await expect(
    page.getByRole("heading", { name: "Save", exact: true }),
  ).toBeVisible();
  // Reset boots from the stored battery save, so the game reads its own write.
  await page.getByRole("button", { name: /^Reset/ }).click();
  await expect.poll(() => register(page, 2)).toBe(42);
  expect(await exportedSave(page)).toBe(42);
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
  await page
    .locator(".console-app")
    .dispatchEvent("drop", { dataTransfer: zipDrop });
  await zipDrop.dispose();
  await expect
    .poll(() =>
      page.evaluate(() =>
        window.player.state.games.map((game) => [game.hash, game.title]),
      ),
    )
    .toEqual([[hash, "Save"]]);
  await expect(
    page.getByRole("heading", { name: "Save", exact: true }),
  ).toBeVisible();
  await expect(page.locator(".save-indicator")).toContainText(
    "Autosave active",
  );
  await expect.poll(() => exportedSave(page)).toBe(42);
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
  await ready(page);
  // An SRAM cartridge that never writes its save.
  await openRom(page, {
    name: "Progress.gba",
    mimeType: "application/octet-stream",
    buffer: sramRom([0xeafffffe]),
  });
  await expect(page.locator(".save-indicator")).toContainText(
    "Save unavailable",
  );
  await page.getByRole("button", { name: "More options" }).click();
  await expect(
    page.getByRole("button", { name: "Export battery save", exact: true }),
  ).toBeEnabled();
  await page.getByLabel("Import battery save", { exact: true }).setInputFiles({
    name: "Progress.sav",
    mimeType: "application/octet-stream",
    buffer: Buffer.alloc(32768, 17),
  });
  await expect.poll(() => exportedSave(page)).toBe(17);
  await expect.poll(() => frames(page)).toBeGreaterThan(0);
  await page.getByRole("button", { name: "More options" }).click();
  const downloading = page.waitForEvent("download");
  await page
    .getByRole("button", { name: "Export battery save", exact: true })
    .click();
  const exported = await (await downloading).createReadStream();
  const chunks: Buffer[] = [];
  for await (const chunk of exported) chunks.push(chunk as Buffer);
  const save = Buffer.concat(chunks);
  expect(save.length).toBe(32768);
  expect(save.every((byte) => byte === 17)).toBe(true);
});

test("battery import ignores an emulator trailer and keeps both flash banks", async ({
  page,
}) => {
  await ready(page);
  const flashRom = Buffer.alloc(272);
  flashRom.writeUInt32LE(0xeafffffe, 0);
  flashRom.write("FLASH1M_V103", 256);
  await openRom(page, {
    name: "Flash.gba",
    mimeType: "application/octet-stream",
    buffer: flashRom,
  });
  const save = Buffer.alloc(128 * 1024 + 16, 0xff);
  save[0x1234] = 0x35;
  save[0x10042] = 0x7a;
  save.fill(0x55, 128 * 1024);

  await page.getByRole("button", { name: "More options" }).click();
  await page.getByLabel("Import battery save", { exact: true }).setInputFiles({
    name: "Flash.sav",
    mimeType: "application/octet-stream",
    buffer: save,
  });
  await expect(page.locator(".save-indicator")).toContainText(
    "Autosave active",
  );
  await expect
    .poll(() =>
      page.evaluate(async () => {
        const data = await window.player.emulator.exportSave();
        return [data.length, data[0x1234], data[0x10042]];
      }),
    )
    .toEqual([128 * 1024, 0x35, 0x7a]);
});

test("library lists metadata and reads only the selected ROM", async ({
  page,
}) => {
  const { createHash } = await import("node:crypto");
  const second = Buffer.from(rom.buffer);
  second[0] = 43;
  const games = [rom.buffer, second].map((bytes, index) => ({
    hash: createHash("sha1").update(bytes).digest("hex"),
    title: `Game ${index + 1}`,
    rom: [...bytes],
    lastPlayed: index + 1,
  }));
  await page.goto("/");
  await expect
    .poll(() => page.evaluate(() => window.player?.state.ready))
    .toBe(true);
  // The page has opened the current library; write two games directly.
  await page.evaluate(async (games) => {
    const request = indexedDB.open("zigba-local");
    const db = await new Promise<IDBDatabase>((resolve, reject) => {
      request.onsuccess = () => resolve(request.result);
      request.onerror = () => reject(request.error);
    });
    const tx = db.transaction(["games", "roms", "playthroughs"], "readwrite");
    for (const { rom, ...game } of games) {
      tx.objectStore("games").put(
        { ...game, romSize: rom.length, activePlaythroughId: "default" },
        game.hash,
      );
      tx.objectStore("roms").put(new Uint8Array(rom), game.hash);
      tx.objectStore("playthroughs").put(
        {
          hash: game.hash,
          id: "default",
          name: "Playthrough 1",
          createdAt: game.lastPlayed,
          lastPlayed: game.lastPlayed,
        },
        [game.hash, "default"],
      );
    }
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
  await page.goto("/");
  // Home offers the most recent game from metadata alone.
  await expect(page.locator(".landing-resume")).toContainText("Game 2");
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
  await page.getByRole("button", { name: "Resume adventure" }).click();
  await expect(page.getByLabel(/^Game screen/)).toBeVisible();
  await expect(
    page.getByRole("heading", { name: "Game 2", exact: true }),
  ).toBeVisible();
  await expect.poll(() => register(page, 0)).toBe(43);
  expect(await reads()).toEqual([games[1].hash]);
});

test("keyboard focus, focus loss, touch release and visibility pause", async ({
  page,
}) => {
  await ready(page);
  await openRom(page, rom);
  await expect.poll(() => playing(page)).toBe(true);
  const keys = () =>
    page.evaluate(async () => {
      const bytes = await window.player.emulator.readMemory(0x04000130, 2);
      return 1023 ^ (bytes[0] | (bytes[1] << 8));
    });
  await page.getByLabel(/^Game screen/).focus();
  await page.keyboard.down("KeyX");
  await expect.poll(keys).toBe(1);
  await page.evaluate(() => window.dispatchEvent(new Event("blur")));
  await expect.poll(keys).toBe(0);
  await page.keyboard.up("KeyX");
  await page.evaluate(() => {
    const input = document.createElement("input");
    input.setAttribute("aria-label", "Typing fixture");
    document.body.append(input);
  });
  await page.getByLabel("Typing fixture").fill("x");
  await page.keyboard.press("KeyX");
  expect(await keys()).toBe(0);
  await page.getByLabel("Typing fixture").evaluate((node) => node.remove());
  await page.setViewportSize({ width: 390, height: 844 });
  await page.getByRole("button", { name: "How to play", exact: true }).click();
  await page.getByRole("button", { name: "Show touch controls" }).click();
  await page.getByRole("button", { name: "Close dialog" }).click();
  const a = page
    .getByLabel("Touch controls", { exact: true })
    .getByRole("button", { name: "A", exact: true });
  const box = await a.boundingBox();
  await page.mouse.move(box!.x + box!.width / 2, box!.y + box!.height / 2);
  await page.mouse.down();
  await expect.poll(keys).toBe(1);
  await page.mouse.up();
  await expect.poll(keys).toBe(0);
  await expect.poll(() => playing(page)).toBe(true);
  await page.evaluate(() => {
    Object.defineProperty(document, "hidden", {
      configurable: true,
      get: () => true,
    });
    document.dispatchEvent(new Event("visibilitychange"));
  });
  await expect.poll(() => playing(page)).toBe(false);
  await expect(page.locator(".pause-overlay")).toBeVisible();
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
  await ready(page);
  await openRom(page, {
    name: "Wave.gba",
    mimeType: "application/octet-stream",
    buffer: bytes,
  });
  await expect.poll(() => playing(page)).toBe(true);
  await expect
    .poll(() =>
      page.evaluate(
        () =>
          (window as unknown as { audioEvidence: { nonzero: boolean } })
            .audioEvidence.nonzero,
      ),
    )
    .toBe(true);
  await page.getByRole("button", { name: "Mute sound", exact: true }).click();
  await expect(
    page.getByRole("button", { name: "Unmute sound", exact: true }),
  ).toHaveAttribute("aria-pressed", "true");
  await page.evaluate(() => window.player.pause());
});

test("browser SDK validation passes", async ({ page }) => {
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
  await ready(page);
  const before = Math.floor(Date.now() / 1000);
  await openRom(page, {
    name: "Clock.gba",
    mimeType: "application/octet-stream",
    buffer: bytes,
  });
  const config = await page.evaluate(async () => {
    const emulator = window.player.emulator;
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
});

test("boot options override cartridge clock detection and pin a fixed epoch", async ({
  page,
}) => {
  const bytes = Buffer.alloc(256);
  bytes.writeUInt32LE(0xeafffffe, 0);
  bytes.write("AXVE", 0xac);
  await sdkPage(page);
  const result = await page.evaluate(
    async (bytes) => {
      const { Emulator } = (window as unknown as ValidationHooks)
        .zigbaValidation;
      const emulator = await Emulator.create();
      try {
        const rom = new Uint8Array(bytes);
        await emulator.load(rom, { rtc: false, rtcEpoch: 946684800n });
        const pinned = (await emulator.exportScenario()).manifest.config;
        const off = (await emulator.inspect()).rtcEnabled;
        await emulator.load(rom, { rtc: true });
        return { pinned, off, on: (await emulator.inspect()).rtcEnabled };
      } finally {
        emulator.terminate();
      }
    },
    [...bytes],
  );
  expect(result.pinned.rtc).toBe(false);
  expect(result.pinned.rtcEpoch).toBe("946684800");
  expect(result.off).toBe(false);
  expect(result.on).toBe(true);
});

test("the player imports and plays a dropped ROM", async ({ page }) => {
  await ready(page);
  const transfer = await page.evaluateHandle(
    (bytes) => {
      const dt = new DataTransfer();
      dt.items.add(new File([new Uint8Array(bytes)], "Dropped.GBA"));
      return dt;
    },
    [...rom.buffer],
  );
  const app = page.locator(".console-app");
  await app.dispatchEvent("dragenter", { dataTransfer: transfer });
  await expect(page.locator(".drop-message")).toBeVisible();
  await page
    .getByLabel(/^Game screen/)
    .dispatchEvent("drop", { dataTransfer: transfer });
  await expect(page.locator(".drop-message")).toHaveCount(0);
  await transfer.dispose();
  await expect(
    page.getByRole("heading", { name: "Dropped", exact: true }),
  ).toBeVisible();
  await expect.poll(() => playing(page)).toBe(true);
  const started = await frames(page);
  await expect.poll(() => frames(page)).toBeGreaterThan(started);
  expect(
    await page.evaluate(async () =>
      window.player.state.games.map((game) => game.title),
    ),
  ).toEqual(["Dropped"]);
  await page.evaluate(() => window.player.pause());
});

test("speed choices and hold-Tab boost change pacing, not machine timing", async ({
  page,
}) => {
  await ready(page);
  await openRom(page, rom);
  await expect.poll(() => playing(page)).toBe(true);
  const speed = page.getByRole("button", { name: "Playback speed" });
  const choose = async (value: string) => {
    await speed.click();
    await page.getByRole("button", { name: new RegExp(`^${value}×`) }).click();
    await expect(speed).toHaveText(`${value}×`);
    await expect(
      page.getByRole("dialog", { name: "Playback speed" }),
    ).toHaveCount(0);
  };
  const measure = async () => {
    const start = await frames(page);
    await page.waitForTimeout(500);
    return (await frames(page)) - start;
  };
  const canvas = page.getByLabel(/^Game screen/);
  await choose("0.5");
  await page.waitForTimeout(100);
  const slow = await measure();
  await choose("1");
  await page.waitForTimeout(100);
  const normal = await measure();
  await canvas.focus();
  await page.keyboard.down("Tab");
  await expect(speed).toHaveText("16×");
  const boosted = await measure();
  expect(slow).toBeGreaterThan(0);
  expect(normal).toBeGreaterThan(slow * 1.4);
  expect(boosted).toBeGreaterThan(slow * 2);
  expect(boosted).toBeGreaterThan(normal * 1.5);
  await page.keyboard.up("Tab");
  await expect(speed).toHaveText("1×");
  await choose("4");
  await page.waitForTimeout(100);
  expect(await measure()).toBeGreaterThan(normal * 1.5);
  await choose("0.5");
  await canvas.focus();
  await page.keyboard.down("Tab");
  await expect(speed).toHaveText("16×");
  await page.evaluate(() => window.dispatchEvent(new Event("blur")));
  await expect(speed).toHaveText("0.5×");
  await page.keyboard.up("Tab");
  expect(await playing(page)).toBe(true);
});

test("playback speed never changes emulated results", async ({ page }) => {
  await sdkPage(page);
  const parity = await page.evaluate(async () => {
    const { Emulator } = (window as unknown as ValidationHooks).zigbaValidation;
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
  await sdkPage(page);
  const result = await page.evaluate(async () => {
    const { Emulator } = (window as unknown as ValidationHooks).zigbaValidation;
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
