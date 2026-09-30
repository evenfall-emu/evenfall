import { createHash } from "node:crypto";
import { execFileSync } from "node:child_process";
import { gzipSync } from "node:zlib";
import { expect, test, type Page } from "@playwright/test";
import type { Emulator } from "@evenfall/gba";
import { advanceAutosave } from "./autosave";
import { stateAt } from "./state.fixture";

const rom = Buffer.from([42, 0, 160, 227, 254, 255, 255, 234]);
const hash = createHash("sha1").update(rom).digest("hex");
const file = {
  name: "Cloud adventure.gba",
  mimeType: "application/octet-stream",
  buffer: rom,
};
type Backup = { data: string; cycle: string };

async function checkpoint(page: Page, cycle = "1234567"): Promise<Backup> {
  await page.route("**/api/auth/get-session", (route) =>
    route.fulfill({ json: null }),
  );
  await page.goto("/");
  await expect(page.getByLabel("Open ROM", { exact: true })).toBeEnabled();
  return stateAt(rom, BigInt(cycle));
}

async function remote(page: Page, backup: Backup, playthroughId = "default") {
  const control = {
    online: true,
    account: "test-player",
    downloads: 0,
    uploads: 0,
    etag: '"remote-checkpoint"',
  };
  const bytes = gzipSync(Buffer.from(JSON.parse(backup.data)));
  await page.route("**/api/auth/get-session", (route) =>
    control.online
      ? route.fulfill({
          json: {
            user: {
              id: control.account,
              name: "Test player",
              email: "player@example.com",
            },
          },
        })
      : route.abort("internetdisconnected"),
  );
  await page.route("**/api/saves", (route) =>
    control.online
      ? route.fulfill({
          json: {
            saves:
              control.account === "test-player"
                ? [
                    {
                      hash,
                      playthroughId,
                      playthroughName: "Cloud run",
                      title: "Cloud adventure",
                      capturedAt: 1,
                      uploadedAt: "2026-09-22T18:31:00Z",
                    },
                  ]
                : [],
            cursor: null,
          },
        })
      : route.abort("internetdisconnected"),
  );
  await page.route("**/api/saves/*/*/*", (route) => {
    if (!control.online) return route.abort("internetdisconnected");
    if (route.request().method() === "PUT") {
      control.uploads++;
      return route.fulfill({ status: 500 });
    }
    expect(route.request().headers()["x-zigba-account"]).toBe("test-player");
    if (
      route.request().method() === "HEAD" &&
      new URL(route.request().url()).pathname !==
        `/api/saves/${hash}/${playthroughId}/resume`
    )
      return route.fulfill({ status: 404 });
    expect(new URL(route.request().url()).pathname).toBe(
      `/api/saves/${hash}/${playthroughId}/resume`,
    );
    if (route.request().method() === "HEAD")
      return route.fulfill({
        headers: {
          ETag: control.etag,
          "X-Zigba-Captured-At": "1",
          "X-Zigba-Uploaded-At": "2026-09-22T18:31:00Z",
        },
      });
    control.downloads++;
    return route.fulfill({
      body: bytes,
      headers: {
        "Content-Type": "application/octet-stream",
        ETag: control.etag,
        "X-Zigba-Digest": createHash("sha256").update(bytes).digest("hex"),
      },
    });
  });
  // Opening a run reconciles its quick slots; this fixture's runs have none.
  await page.route("**/api/saves/*/*/quick", (route) =>
    control.online
      ? route.fulfill({ json: { slots: [], limit: 10 } })
      : route.abort("internetdisconnected"),
  );
  return control;
}

async function recordRestores(page: Page) {
  await page.evaluate(() => {
    const host = window as unknown as {
      emulator: Emulator;
      loadedCycles: string[];
    };
    host.loadedCycles = [];
    const load = host.emulator.loadState.bind(host.emulator);
    host.emulator.loadState = async (data) => {
      const cpu = await load(data);
      host.loadedCycles.push(cpu.cycle.toString());
      return cpu;
    };
  });
}

async function localResume(page: Page, playthroughId = "default") {
  return page.evaluate(
    async ({ hash, playthroughId }) => {
      const request = indexedDB.open("zigba-local");
      const db = await new Promise<IDBDatabase>((resolve) => {
        request.onsuccess = () => resolve(request.result);
      });
      try {
        const read = db
          .transaction("resumes")
          .objectStore("resumes")
          .get([hash, playthroughId]);
        return await new Promise<Backup | null>((resolve) => {
          read.onsuccess = () =>
            resolve(
              read.result
                ? {
                    data: JSON.stringify(Array.from(read.result.data)),
                    cycle: read.result.cycle,
                  }
                : null,
            );
        });
      } finally {
        db.close();
      }
    },
    { hash, playthroughId },
  );
}

async function installLocal(page: Page, backup: Backup) {
  const savedAt = Date.parse("2026-09-24T18:00:00Z");
  await page.evaluate(
    async ({ hash, rom, backup, savedAt }) => {
      const request = indexedDB.open("zigba-local");
      const db = await new Promise<IDBDatabase>((resolve) => {
        request.onsuccess = () => resolve(request.result);
      });
      const tx = db.transaction(
        ["games", "roms", "resumes", "playthroughs"],
        "readwrite",
      );
      tx.objectStore("games").put(
        {
          hash,
          title: "Local adventure",
          romSize: rom.length,
          lastPlayed: savedAt,
          resumeSavedAt: savedAt,
          activePlaythroughId: "default",
        },
        hash,
      );
      tx.objectStore("roms").put(new Uint8Array(rom), hash);
      tx.objectStore("playthroughs").put(
        {
          hash,
          id: "default",
          name: "Local run",
          createdAt: 1,
          lastPlayed: savedAt,
          resumeSavedAt: savedAt,
        },
        [hash, "default"],
      );
      tx.objectStore("resumes").put(
        {
          data: new Uint8Array(JSON.parse(backup.data)),
          cycle: backup.cycle,
          savedAt,
          options: { rtcEpoch: 123n },
        },
        [hash, "default"],
      );
      await new Promise<void>((resolve, reject) => {
        tx.oncomplete = () => resolve();
        tx.onerror = () => reject(tx.error);
      });
      db.close();
    },
    { hash, rom: Array.from(rom), backup, savedAt },
  );
}

test("first connection discovers cloud progress, rejects the wrong ROM, and then resumes offline", async ({
  page,
  browser,
}) => {
  const saved = await checkpoint(page);
  const phone = await browser.newPage({
    baseURL: page.url(),
    viewport: { width: 390, height: 844 },
  });
  try {
    const control = await remote(phone, saved);
    await phone.goto("/");
    await expect(
      phone.getByRole("button", { name: "Resume adventure" }),
    ).toBeEnabled();
    await expect(phone.locator(".landing-resume")).toContainText(
      "Cloud adventure",
    );
    await expect(phone.locator(".landing-resume-detail")).toContainText(
      "Cloud checkpoint",
    );
    expect(control.downloads).toBe(0);
    await phone.evaluate(() => document.fonts.ready);
    await phone.screenshot({
      path: "../../artifacts/cloud-resume/home-mobile.png",
    });
    await phone.getByRole("button", { name: "Resume adventure" }).click();
    const dialog = phone.getByRole("dialog", { name: "Resume from cloud" });
    await expect(dialog).toContainText("never uploaded");
    await phone.screenshot({
      path: "../../artifacts/cloud-resume/choose-rom-mobile.png",
    });
    await expect(dialog.getByLabel("Choose ROM for cloud resume")).toBeHidden();
    const picker = dialog.getByRole("button", {
      name: "Choose ROM and resume",
    });
    await picker.focus();
    const choosing = phone.waitForEvent("filechooser");
    await picker.press("Enter");
    await (
      await choosing
    ).setFiles({
      ...file,
      buffer: Buffer.from([7, ...rom.subarray(1)]),
    });
    await expect(dialog.getByRole("alert")).toContainText("does not match");
    expect(await localResume(phone)).toBeNull();
    await recordRestores(phone);
    // Python is already required by the browser-test server. Keep ZIP creation
    // independent of the emulator's parser and never check in ROM archives.
    const zipped = execFileSync(
      "python3",
      [
        "-c",
        `
import io, sys, zipfile
output = io.BytesIO()
with zipfile.ZipFile(output, 'w', zipfile.ZIP_DEFLATED) as archive:
    archive.writestr('Cloud adventure.gba', sys.stdin.buffer.read())
sys.stdout.buffer.write(output.getvalue())
`,
      ],
      { input: rom },
    );
    await dialog
      .getByLabel("Choose ROM for cloud resume")
      .setInputFiles({ ...file, name: "Cloud adventure.zip", buffer: zipped });
    await expect(phone.getByLabel(/^Game screen/)).toBeVisible();
    expect(
      await phone.evaluate(
        () => (window as unknown as { loadedCycles: string[] }).loadedCycles,
      ),
    ).toEqual([saved.cycle]);
    expect((await localResume(phone))?.cycle).toBe(saved.cycle);
    expect(control.uploads).toBe(0);
    // Leaving the phone player persists progress, as any navigation does.
    await phone.getByRole("button", { name: "Back to Library" }).click();
    await expect
      .poll(async () => (await localResume(phone))?.cycle)
      .not.toBe(saved.cycle);
    const durable = await localResume(phone);
    await expect
      .poll(() =>
        phone.evaluate(() => Boolean(navigator.serviceWorker.controller)),
      )
      .toBe(true);
    control.online = false;
    await phone.context().setOffline(true);
    await phone.goto("/");
    await expect(
      phone.getByRole("button", { name: "Resume adventure" }),
    ).toBeEnabled();
    await recordRestores(phone);
    await phone.getByRole("button", { name: "Resume adventure" }).click();
    await expect(phone.getByLabel(/^Game screen/)).toBeVisible();
    expect(
      await phone.evaluate(
        () => (window as unknown as { loadedCycles: string[] }).loadedCycles,
      ),
    ).toEqual([durable!.cycle]);
  } finally {
    await phone.close();
  }
});

test("existing device progress remains an explicit alternative to a cloud checkpoint", async ({
  page,
}) => {
  const local = await checkpoint(page, "3456789");
  await installLocal(page, local);
  const cloud = await checkpoint(page, "1234567");
  const control = await remote(page, cloud);
  await page.goto("/");
  await expect(
    page.getByRole("button", { name: "Resume adventure" }),
  ).toBeEnabled();
  await expect(page.locator(".landing-resume-detail")).toContainText(
    "On this device",
  );
  await expect(page.locator(".landing-resume")).toContainText(
    "Local adventure",
  );
  await recordRestores(page);
  await page.getByRole("button", { name: "Resume adventure" }).click();
  const dialog = page.getByRole("dialog", { name: "Resume from cloud" });
  await expect(
    dialog.getByRole("button", { name: "Resume cloud checkpoint" }),
  ).toBeVisible();
  await expect(
    dialog.getByRole("button", { name: "Choose another ROM file" }),
  ).toBeVisible();
  await expect(dialog.getByLabel("Choose ROM for cloud resume")).toBeHidden();
  expect(await localResume(page)).toEqual(local);
  await page.evaluate(() => document.fonts.ready);
  await page.screenshot({
    path: "../../artifacts/cloud-resume/local-choice-desktop.png",
  });
  await dialog
    .getByRole("button", { name: "Continue on this device instead" })
    .click();
  await expect(page.getByLabel(/^Game screen/)).toBeVisible();
  expect(
    await page.evaluate(
      () => (window as unknown as { loadedCycles: string[] }).loadedCycles,
    ),
  ).toEqual([local.cycle]);
  expect(control.downloads).toBe(0);
});

test("Home resumes the last local run, while Player resolves the selected other run", async ({
  page,
}) => {
  const a = await checkpoint(page, "3456789");
  const b = await checkpoint(page, "6789012");
  await installLocal(page, a);
  const bId = await page.evaluate(
    async ({ hash, b }) => {
      const library = window.player.library;
      const run = await library.createPlaythrough(hash, "Run B");
      const ref = { hash, playthroughId: run.id };
      await library.saveResume(
        ref,
        new Uint8Array(JSON.parse(b.data)),
        { rtcEpoch: 123n },
        BigInt(b.cycle),
      );
      await library.selectPlaythrough(ref);
      return run.id;
    },
    { hash, b },
  );
  const control = await remote(page, await checkpoint(page, "1234567"));
  await page.goto("/");
  await expect(page.locator(".landing-resume")).toContainText("Run B");
  await recordRestores(page);
  await page.getByRole("button", { name: "Resume adventure" }).click();
  await expect(page.getByLabel(/^Game screen/)).toBeVisible();
  expect(
    await page.evaluate(() => window.player.activeRef!.playthroughId),
  ).toBe(bId);
  expect(
    await page.evaluate(
      () => (window as unknown as { loadedCycles: string[] }).loadedCycles,
    ),
  ).toEqual([b.cycle]);
  expect(control.downloads).toBe(0);

  await page.getByRole("combobox", { name: "Playthrough" }).click();
  await page.getByRole("option", { name: "Local run" }).click();
  const dialog = page.getByRole("dialog", { name: "Resume from cloud" });
  await expect(dialog).toContainText("Local adventure · Local run");
  await dialog
    .getByRole("button", { name: "Continue on this device instead" })
    .click();
  await expect(
    page.getByRole("combobox", { name: "Playthrough" }),
  ).toContainText("Local run");
  expect(
    await page.evaluate(() => window.player.activeRef!.playthroughId),
  ).toBe("default");
  expect(
    await page.evaluate(
      () => (window as unknown as { loadedCycles: string[] }).loadedCycles,
    ),
  ).toEqual([b.cycle, a.cycle]);
  expect(control.downloads).toBe(0);

  await page.getByRole("combobox", { name: "Playthrough" }).click();
  await page.getByRole("option", { name: "Run B" }).click();
  await expect(
    page.getByRole("combobox", { name: "Playthrough" }),
  ).toContainText("Run B");
  await page.getByRole("combobox", { name: "Playthrough" }).click();
  await page.getByRole("option", { name: "Local run" }).click();
  await expect(
    page.getByRole("dialog", { name: "Resume from cloud" }),
  ).toBeVisible();
});

test("an unchanged known cloud version never interrupts local resume, even when local was saved later", async ({
  page,
}) => {
  const local = await checkpoint(page, "3456789");
  await installLocal(page, local);
  const control = await remote(page, await checkpoint(page, "1234567"));
  await page.evaluate(async (hash) => {
    const request = indexedDB.open("zigba-local");
    const db = await new Promise<IDBDatabase>((resolve) => {
      request.onsuccess = () => resolve(request.result);
    });
    const tx = db.transaction("cloud", "readwrite");
    tx.objectStore("cloud").put(
      {
        accountId: "test-player",
        hash,
        playthroughId: "default",
        kind: "resume",
        revision: "local-pending",
        capturedAt: 10,
        lastInputAt: 0,
        baseEtag: '"remote-checkpoint"',
        uploadedRevision: "cloud-previous",
      },
      ["test-player", hash, "default", "resume"],
    );
    await new Promise<void>((resolve) => {
      tx.oncomplete = () => resolve();
    });
    db.close();
  }, hash);
  await page.goto("/");
  await page.getByRole("button", { name: "Resume adventure" }).click();
  await expect(page.getByLabel(/^Game screen/)).toBeVisible();
  await expect(
    page.getByRole("dialog", { name: "Resume from cloud" }),
  ).toHaveCount(0);
  expect(control.downloads).toBe(0);
  expect((await localResume(page))?.cycle).toBe(local.cycle);
  await page.getByRole("button", { name: "More options" }).click();
  await page.getByRole("button", { name: "Restore cloud resume" }).click();
  const dialog = page.getByRole("dialog", { name: "Resume from cloud" });
  await expect(dialog).toContainText("Local adventure · Local run");
  await dialog
    .getByRole("button", { name: "Continue on this device instead" })
    .click();
  expect(control.downloads).toBe(0);
});

test("a cloud checkpoint changed after the choice cannot replace local progress", async ({
  page,
}) => {
  const local = await checkpoint(page, "3456789");
  await installLocal(page, local);
  const control = await remote(page, await checkpoint(page));
  await page.goto("/");
  await page.getByRole("button", { name: "Resume adventure" }).click();
  const dialog = page.getByRole("dialog", { name: "Resume from cloud" });
  await expect(dialog).toBeVisible();
  control.etag = '"new-remote-checkpoint"';
  await dialog.getByRole("button", { name: "Resume cloud checkpoint" }).click();
  await expect(dialog.getByRole("alert")).toContainText(
    "changed since you chose it",
  );
  expect(await localResume(page)).toEqual(local);
});

test("battery-only local progress remains an explicit alternative to cloud restore", async ({
  page,
}) => {
  const saved = await checkpoint(page);
  await page.evaluate(
    async ({ hash, rom }) => {
      const library = window.player.library;
      await library.put({
        hash,
        rom: new Uint8Array(rom),
        title: "Battery only",
        lastPlayed: 1,
      });
      await library.save(
        { hash, playthroughId: "default" },
        new Uint8Array(512).fill(17),
      );
    },
    { hash, rom: Array.from(rom) },
  );
  const control = await remote(page, saved);
  await page.goto("/");
  await expect(page.locator(".landing-resume")).toContainText("Battery only");
  await page.getByRole("button", { name: "Resume adventure" }).click();
  const dialog = page.getByRole("dialog", { name: "Resume from cloud" });
  await expect(dialog).toContainText("In-game save available");
  await dialog
    .getByRole("button", { name: "Continue on this device instead" })
    .click();
  expect(control.downloads).toBe(0);
  expect(await localResume(page)).toBeNull();
  expect(
    await page.evaluate(
      async (hash) =>
        Array.from(
          (await window.player.library.getSave({
            hash,
            playthroughId: "default",
          }))!,
        ),
      hash,
    ),
  ).toEqual(Array(512).fill(17));
});

test("restoring another cloud playthrough keeps this device's existing run intact", async ({
  page,
}) => {
  const local = await checkpoint(page, "3456789");
  await installLocal(page, local);
  const cloud = await checkpoint(page, "1234567");
  const cloudId = "15b285c2-f844-451f-85e6-c5bbcc98dd09";
  await remote(page, cloud, cloudId);
  await page.goto("/");
  await expect(page.locator(".landing-resume-detail")).toContainText(
    "On this device",
  );
  await page.getByRole("button", { name: "Resume adventure" }).click();
  const selected = page.getByRole("combobox", { name: "Playthrough" });
  await expect(selected).toContainText("Local run");
  await recordRestores(page);
  await selected.click();
  await page.getByRole("option", { name: "Cloud run · cloud" }).click();
  await page.getByRole("button", { name: "Resume cloud checkpoint" }).click();
  await expect(page.getByLabel(/^Game screen/)).toBeVisible();
  await expect(selected).toContainText("Cloud run");
  expect(
    await page.evaluate(() => window.player.activeRef!.playthroughId),
  ).toBe(cloudId);
  // The local run progressed while it played; the restore left it alone.
  const kept = (await localResume(page))!;
  expect(Number(kept.cycle)).toBeGreaterThanOrEqual(Number(local.cycle));
  expect(kept.cycle).not.toBe(cloud.cycle);
  expect((await localResume(page, cloudId))?.cycle).toBe(cloud.cycle);
  await selected.click();
  await page.getByRole("option", { name: "Local run" }).click();
  await expect(selected).toContainText("Local run");
  await expect
    .poll(() =>
      page.evaluate(
        () => (window as unknown as { loadedCycles: string[] }).loadedCycles,
      ),
    )
    .toEqual([cloud.cycle, kept.cycle]);
});

test("Player can choose a cloud-only playthrough of the loaded game", async ({
  page,
}) => {
  const local = await checkpoint(page, "3456789");
  await installLocal(page, local);
  const cloudId = "15b285c2-f844-451f-85e6-c5bbcc98dd09";
  await remote(page, await checkpoint(page), cloudId);
  await page.goto("/");
  await page.getByRole("button", { name: "Resume adventure" }).click();
  await expect(
    page.getByRole("combobox", { name: "Playthrough" }),
  ).toContainText("Local run");
  await page.getByRole("combobox", { name: "Playthrough" }).click();
  await expect(
    page.getByRole("option", { name: "Cloud run · cloud" }),
  ).toBeVisible();
  await page.locator(".playthrough-select-popup").screenshot({
    path: "../../artifacts/cloud-resume/player-cloud-menu-desktop.png",
  });
  await page.getByRole("option", { name: "Cloud run · cloud" }).click();
  const dialog = page.getByRole("dialog", { name: "Resume from cloud" });
  await expect(dialog).toContainText("Local adventure · Cloud run");
  await expect(
    dialog.getByRole("button", { name: "Continue on this device instead" }),
  ).toHaveCount(0);
  await dialog.getByRole("button", { name: "Resume cloud checkpoint" }).click();
  await expect(
    page.getByRole("combobox", { name: "Playthrough" }),
  ).toContainText("Cloud run");
  expect(Number((await localResume(page))?.cycle)).toBeGreaterThanOrEqual(
    Number(local.cycle),
  );
});

test("cloud restore preserves the running adventure when its local checkpoint fails", async ({
  page,
}) => {
  const cloud = await checkpoint(page);
  const cloudId = "15b285c2-f844-451f-85e6-c5bbcc98dd09";
  const control = await remote(page, cloud, cloudId);
  await page.goto("/");
  await expect(page.getByLabel("Open ROM", { exact: true })).toBeEnabled();
  await page.getByLabel("Open ROM", { exact: true }).setInputFiles(file);
  await expect(page.getByRole("button", { name: "Reset" })).toBeEnabled();
  await expect
    .poll(() => page.evaluate(() => window.player.state.cloud.resumes?.length))
    .toBe(1);
  const result = await page.evaluate(async () => {
    const player = window.player;
    await player.pause();
    await player.emulator.runUntil(
      (await player.emulator.inspect()).cycle + 280896n * 7n,
    );
    const original = player.activeRef;
    const writer = player.releaseLock;
    const before = JSON.stringify(
      Array.from(await player.emulator.saveState()),
    );
    player.library.saveResume = async () => {
      throw new DOMException("Storage full", "QuotaExceededError");
    };
    let error = "";
    try {
      await player.restoreCloud("resume", player.state.cloud.resumes![0]);
    } catch (failure) {
      error = String(failure);
    }
    return {
      error,
      original,
      selected: player.activeRef,
      before,
      after: JSON.stringify(Array.from(await player.emulator.saveState())),
      writerRetained: Boolean(writer) && player.releaseLock === writer,
    };
  });
  expect(result.error).toContain("Local progress could not be saved");
  expect(control.downloads).toBe(0);
  expect(result.selected).toEqual(result.original);
  expect(result.after).toEqual(result.before);
  expect(result.writerRetained).toBe(true);
});

for (const failure of ["incompatible checkpoint", "local storage failure"])
  test(`${failure} cannot overwrite existing local progress on an autosave tick`, async ({
    page,
  }) => {
    await page.clock.install();
    const local = await checkpoint(page);
    await installLocal(page, local);
    await remote(
      page,
      failure === "incompatible checkpoint"
        ? { data: JSON.stringify([1, 2, 3]), cycle: "0" }
        : local,
    );
    await page.goto("/");
    await expect(page.locator(".landing-resume-detail")).toContainText(
      "On this device",
    );
    await page.getByRole("button", { name: "Resume adventure" }).click();
    const dialog = page.getByRole("dialog", { name: "Resume from cloud" });
    if (failure === "local storage failure")
      await page.evaluate(() => {
        const put = IDBObjectStore.prototype.put;
        IDBObjectStore.prototype.put = function (...args) {
          if (this.name === "resumes")
            throw new DOMException("Storage full", "QuotaExceededError");
          return put.apply(this, args);
        };
      });
    await dialog
      .getByRole("button", { name: "Resume cloud checkpoint" })
      .click();
    await expect(dialog.getByRole("alert")).toBeVisible();
    await advanceAutosave(page);
    expect(await localResume(page)).toEqual(local);
    expect(
      await page.evaluate(async (hash) => {
        const request = indexedDB.open("zigba-local");
        const db = await new Promise<IDBDatabase>((resolve) => {
          request.onsuccess = () => resolve(request.result);
        });
        try {
          const read = db
            .transaction("cloud")
            .objectStore("cloud")
            .get(["test-player", hash, "default", "resume"]);
          return await new Promise<unknown>((resolve) => {
            read.onsuccess = () => resolve(read.result);
          });
        } finally {
          db.close();
        }
      }, hash),
    ).toBeUndefined(); // A failed restore must not advance the outbox's remote base.
    await expect(
      dialog.getByRole("button", { name: "Continue on this device instead" }),
    ).toBeEnabled();
  });

test("a different account at download time cannot launch the previously selected backup", async ({
  page,
}) => {
  const saved = await checkpoint(page);
  const control = await remote(page, saved);
  await page.goto("/");
  await expect(
    page.getByRole("button", { name: "Resume adventure" }),
  ).toBeEnabled();
  await page.getByRole("button", { name: "Resume adventure" }).click();
  await expect(
    page.getByRole("dialog", { name: "Resume from cloud" }),
  ).toBeVisible();
  control.account = "different-player";
  await page.getByLabel("Choose ROM for cloud resume").setInputFiles(file);
  await expect(
    page.getByRole("dialog", { name: "Resume from cloud" }),
  ).toBeHidden();
  await expect(page.getByRole("button", { name: "Open player" })).toBeEnabled();
  expect(control.downloads).toBe(0);
  expect(await localResume(page)).toBeNull();
});
