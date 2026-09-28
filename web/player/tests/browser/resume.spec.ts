import { expect, test, type Page } from "@playwright/test";
import type { Emulator } from "@evenfall/gba";

const loopRom = (marker: number, name: string) => ({
  name: `${name}.gba`,
  mimeType: "application/octet-stream",
  buffer: Buffer.from([marker, 0, 160, 227, 254, 255, 255, 234]),
});

async function enterPlayer(page: Page) {
  await page.goto("/");
  await page.getByRole("button", { name: "Open player" }).click();
  await expect(page.getByLabel(/^Game screen/)).toBeVisible();
}

async function importAndCheckpoint(page: Page, marker: number, name: string) {
  await page
    .getByLabel("Open ROM", { exact: true })
    .setInputFiles(loopRom(marker, name));
  await expect(page.getByRole("heading", { name, exact: true })).toBeVisible();
  await expect(page.getByLabel("Open ROM", { exact: true })).toBeEnabled();
  await expect
    .poll(() =>
      page.evaluate(async () =>
        Number(
          (
            await (
              window as unknown as { emulator: Emulator }
            ).emulator.inspect()
          ).cycle,
        ),
      ),
    )
    .toBeGreaterThan(0);
  await expect(page.getByLabel("Open ROM", { exact: true })).toBeEnabled();
  await page.getByLabel(/^Game screen/).focus();
  await page.keyboard.press("Space");
  await expect
    .poll(() =>
      page.evaluate(async () => {
        const emulator = (window as unknown as { emulator: Emulator }).emulator;
        const before = (await emulator.inspect()).cycle;
        await new Promise((resolve) => setTimeout(resolve, 100));
        return (await emulator.inspect()).cycle === before;
      }),
    )
    .toBe(true);
  const identity = await page.evaluate(async () => {
    const instance = (window as unknown as { emulator: Emulator }).emulator;
    return {
      hash: (await instance.inspect()).romHash,
      batteryBytes: (await instance.exportSave()).byteLength,
    };
  });
  await expect
    .poll(() =>
      page.evaluate(async (hash) => {
        const request = indexedDB.open("zigba-local");
        const db = await new Promise<IDBDatabase>((resolve, reject) => {
          request.onsuccess = () => resolve(request.result);
          request.onerror = () => reject(request.error);
        });
        const result = await new Promise<boolean>((resolve, reject) => {
          const get = db
            .transaction("resumes")
            .objectStore("resumes")
            .get([hash, "default"]);
          get.onsuccess = () => resolve(!!get.result);
          get.onerror = () => reject(get.error);
        });
        db.close();
        return result;
      }, identity.hash),
    )
    .toBe(true);
  const state = await page.evaluate(async (hash) => {
    const request = indexedDB.open("zigba-local");
    const db = await new Promise<IDBDatabase>((resolve, reject) => {
      request.onsuccess = () => resolve(request.result);
      request.onerror = () => reject(request.error);
    });
    const result = await new Promise<number[]>((resolve, reject) => {
      const get = db
        .transaction("resumes")
        .objectStore("resumes")
        .get([hash, "default"]);
      get.onsuccess = () => resolve(Array.from(get.result.data));
      get.onerror = () => reject(get.error);
    });
    db.close();
    return result;
  }, identity.hash);
  return { ...identity, state };
}

async function preventPlayback(page: Page) {
  await page.evaluate(() => {
    const emulator = (window as unknown as { emulator: Emulator }).emulator;
    emulator.play = async () => {};
  });
}

async function replaceResume(
  page: Page,
  hash: string,
  data?: number[],
  options = { retainCurrentAsPrevious: false },
) {
  await page.evaluate(
    async ({ hash, data, options }) => {
      const request = indexedDB.open("zigba-local");
      const db = await new Promise<IDBDatabase>((resolve, reject) => {
        request.onsuccess = () => resolve(request.result);
        request.onerror = () => reject(request.error);
      });
      const tx = db.transaction("resumes", "readwrite");
      const store = tx.objectStore("resumes");
      if (data) {
        const get = store.get([hash, "default"]);
        const record = await new Promise<Record<string, unknown>>(
          (resolve, reject) => {
            get.onsuccess = () => resolve(get.result);
            get.onerror = () => reject(get.error);
          },
        );
        const { previous: _previous, ...generation } = record;
        store.put(
          {
            ...generation,
            data: new Uint8Array(data),
            previous: options.retainCurrentAsPrevious ? generation : undefined,
          },
          [hash, "default"],
        );
      } else store.delete([hash, "default"]);
      await new Promise<void>((resolve, reject) => {
        tx.oncomplete = () => resolve();
        tx.onerror = () => reject(tx.error);
      });
      db.close();
    },
    { hash, data, options },
  );
}

async function storedResume(page: Page, hash: string) {
  return page.evaluate(async (key) => {
    const request = indexedDB.open("zigba-local");
    const db = await new Promise<IDBDatabase>((resolve, reject) => {
      request.onsuccess = () => resolve(request.result);
      request.onerror = () => reject(request.error);
    });
    const result = await new Promise<number[] | undefined>(
      (resolve, reject) => {
        const get = db
          .transaction("resumes")
          .objectStore("resumes")
          .get([key, "default"]);
        get.onsuccess = () =>
          resolve(get.result ? Array.from(get.result.data) : undefined);
        get.onerror = () => reject(get.error);
      },
    );
    db.close();
    return result;
  }, hash);
}

test("resumes the most recent game at its exact durable state without requiring battery data", async ({
  page,
}) => {
  await enterPlayer(page);
  await importAndCheckpoint(page, 41, "Older");
  const recent = await importAndCheckpoint(page, 42, "Recent");
  expect(recent.batteryBytes).toBe(0);

  await page.goto("/");
  await expect(
    page.getByRole("button", { name: "Resume adventure" }),
  ).toBeEnabled();
  await preventPlayback(page);
  await page.evaluate(() => {
    const host = window as unknown as {
      emulator: Emulator;
      resumeLoadEntered: boolean;
      releaseResumeLoad: () => void;
    };
    const loadState = host.emulator.loadState.bind(host.emulator);
    let release = () => {};
    const gate = new Promise<void>((resolve) => {
      release = resolve;
    });
    host.releaseResumeLoad = release;
    host.emulator.loadState = async (bytes) => {
      host.resumeLoadEntered = true;
      await gate;
      return loadState(bytes);
    };
  });
  await page.getByRole("button", { name: "Resume adventure" }).click();
  await expect
    .poll(() =>
      page.evaluate(
        () =>
          (window as unknown as { resumeLoadEntered?: boolean })
            .resumeLoadEntered,
      ),
    )
    .toBe(true);
  await expect(page.getByLabel("Open ROM", { exact: true })).toBeDisabled();
  const bootCycle = await page.evaluate(async () => {
    const emulator = (window as unknown as { emulator: Emulator }).emulator;
    return (await emulator.inspect()).cycle.toString();
  });
  await page.keyboard.press("KeyX");
  await page.keyboard.press("Space");
  await page.waitForTimeout(100);
  expect(
    await page.evaluate(async () => {
      const emulator = (window as unknown as { emulator: Emulator }).emulator;
      return (await emulator.inspect()).cycle.toString();
    }),
  ).toBe(bootCycle);
  await page.evaluate(() =>
    (
      window as unknown as { releaseResumeLoad: () => void }
    ).releaseResumeLoad(),
  );
  await expect
    .poll(() =>
      page.evaluate(async () => {
        const emulator = (window as unknown as { emulator: Emulator }).emulator;
        return {
          hash: (await emulator.inspect()).romHash,
          state: Array.from(await emulator.saveState()),
        };
      }),
    )
    .toEqual({ hash: recent.hash, state: recent.state });
});

test("recovers the previous generation when the latest checkpoint is corrupt", async ({
  page,
}) => {
  await enterPlayer(page);
  const saved = await importAndCheckpoint(page, 50, "Recovery");
  await replaceResume(page, saved.hash, [9, 8, 7], {
    retainCurrentAsPrevious: true,
  });
  await page.goto("/");
  await expect(
    page.getByRole("button", { name: "Resume adventure" }),
  ).toBeEnabled();
  await preventPlayback(page);
  await page.getByRole("button", { name: "Resume adventure" }).click();
  await expect(page.getByLabel(/^Game screen/)).toBeVisible();
  expect(
    await page.evaluate(async () =>
      Array.from(
        await (
          window as unknown as { emulator: Emulator }
        ).emulator.saveState(),
      ),
    ),
  ).toEqual(saved.state);
  await expect(
    page.getByRole("dialog", { name: "Resume needs attention" }),
  ).toHaveCount(0);
});

test("a corrupt resume remains available for retry and cannot be replaced by the boot fallback", async ({
  page,
}) => {
  await enterPlayer(page);
  const saved = await importAndCheckpoint(page, 43, "Corrupt");
  const corrupt = [1, 2, 3, 4];
  await page.evaluate(
    async ({ hash, corrupt }) => {
      const request = indexedDB.open("zigba-local");
      const db = await new Promise<IDBDatabase>((resolve, reject) => {
        request.onsuccess = () => resolve(request.result);
        request.onerror = () => reject(request.error);
      });
      const tx = db.transaction("resumes", "readwrite");
      const store = tx.objectStore("resumes");
      const get = store.get([hash, "default"]);
      const record = await new Promise<Record<string, unknown>>(
        (resolve, reject) => {
          get.onsuccess = () => resolve(get.result);
          get.onerror = () => reject(get.error);
        },
      );
      store.put(
        { ...record, data: new Uint8Array(corrupt), previous: undefined },
        [hash, "default"],
      );
      await new Promise<void>((resolve, reject) => {
        tx.oncomplete = () => resolve();
        tx.onerror = () => reject(tx.error);
      });
      db.close();
    },
    { hash: saved.hash, corrupt },
  );

  await page.goto("/");
  await page.getByRole("button", { name: "Resume adventure" }).click();
  const recovery = page.getByRole("dialog", { name: "Resume needs attention" });
  await expect(recovery).toBeVisible();
  await page.screenshot({
    path: "../artifacts/resume-recovery/desktop-corrupt.png",
  });
  await page.waitForTimeout(5500);
  expect(
    await page.evaluate(async (hash) => {
      const request = indexedDB.open("zigba-local");
      const db = await new Promise<IDBDatabase>((resolve, reject) => {
        request.onsuccess = () => resolve(request.result);
        request.onerror = () => reject(request.error);
      });
      const result = await new Promise<number[]>((resolve, reject) => {
        const get = db
          .transaction("resumes")
          .objectStore("resumes")
          .get([hash, "default"]);
        get.onsuccess = () => resolve(Array.from(get.result.data));
        get.onerror = () => reject(get.error);
      });
      db.close();
      return result;
    }, saved.hash),
  ).toEqual(corrupt);
  await replaceResume(page, saved.hash, saved.state);
  await preventPlayback(page);
  await recovery.getByRole("button", { name: "Retry checkpoint" }).click();
  await expect(recovery).toBeHidden();
  await expect(page.getByLabel(/^Game screen/)).toBeVisible();
});

test("missing resume data does not silently start from the battery save", async ({
  page,
}) => {
  await enterPlayer(page);
  const saved = await importAndCheckpoint(page, 44, "Missing");
  await page.evaluate(async (hash) => {
    const request = indexedDB.open("zigba-local");
    const db = await new Promise<IDBDatabase>((resolve, reject) => {
      request.onsuccess = () => resolve(request.result);
      request.onerror = () => reject(request.error);
    });
    const tx = db.transaction("resumes", "readwrite");
    tx.objectStore("resumes").delete([hash, "default"]);
    await new Promise<void>((resolve, reject) => {
      tx.oncomplete = () => resolve();
      tx.onerror = () => reject(tx.error);
    });
    db.close();
  }, saved.hash);

  await page.goto("/");
  await page.getByRole("button", { name: "Resume adventure" }).click();
  const recovery = page.getByRole("dialog", { name: "Resume needs attention" });
  await expect(recovery).toBeVisible();
  await page.setViewportSize({ width: 390, height: 844 });
  await page.screenshot({
    path: "../artifacts/resume-recovery/mobile-missing.png",
  });
  await page.waitForTimeout(5500);
  expect(
    await page.evaluate(async (hash) => {
      const request = indexedDB.open("zigba-local");
      const db = await new Promise<IDBDatabase>((resolve, reject) => {
        request.onsuccess = () => resolve(request.result);
        request.onerror = () => reject(request.error);
      });
      const result = await new Promise<boolean>((resolve, reject) => {
        const get = db
          .transaction("resumes")
          .objectStore("resumes")
          .get([hash, "default"]);
        get.onsuccess = () => resolve(!!get.result);
        get.onerror = () => reject(get.error);
      });
      db.close();
      return result;
    }, saved.hash),
  ).toBe(false);
  await preventPlayback(page);
  await recovery
    .getByRole("button", { name: "Start from in-game save" })
    .click();
  await expect(recovery).toBeHidden();
  await expect(page.getByLabel(/^Game screen/)).toBeVisible();
});

test("a read-only tab can resume but never overwrites the lock owner's checkpoint", async ({
  page,
  context,
}) => {
  await enterPlayer(page);
  const owner = await importAndCheckpoint(page, 45, "Locked");
  const second = await context.newPage();
  await second.goto("/");
  await expect(
    second.getByRole("button", { name: "Resume adventure" }),
  ).toBeEnabled();
  await preventPlayback(second);
  await second.getByRole("button", { name: "Resume adventure" }).click();
  await expect
    .poll(() =>
      second.evaluate(async () => {
        const emulator = (window as unknown as { emulator: Emulator }).emulator;
        try {
          return (await emulator.inspect()).romHash;
        } catch {
          return "";
        }
      }),
    )
    .toBe(owner.hash);
  await second.evaluate(async () => {
    const emulator = (window as unknown as { emulator: Emulator }).emulator;
    const cycle = (await emulator.inspect()).cycle;
    await emulator.runUntil(cycle + 1000n);
  });
  await second.waitForTimeout(5500);
  expect(
    await second.evaluate(async (hash) => {
      const request = indexedDB.open("zigba-local");
      const db = await new Promise<IDBDatabase>((resolve, reject) => {
        request.onsuccess = () => resolve(request.result);
        request.onerror = () => reject(request.error);
      });
      const result = await new Promise<number[]>((resolve, reject) => {
        const get = db
          .transaction("resumes")
          .objectStore("resumes")
          .get([hash, "default"]);
        get.onsuccess = () => resolve(Array.from(get.result.data));
        get.onerror = () => reject(get.error);
      });
      db.close();
      return result;
    }, owner.hash),
  ).toEqual(owner.state);
  await second.close();
});

test("trying another game after a failed resume cannot overwrite the failed game's checkpoint", async ({
  page,
}) => {
  await enterPlayer(page);
  const healthy = await importAndCheckpoint(page, 46, "Healthy");
  const failed = await importAndCheckpoint(page, 47, "Failed");
  // Switching games flushes Healthy once more. Resume must load that current
  // generation, not the earlier checkpoint returned by importAndCheckpoint.
  const healthyCurrent = await storedResume(page, healthy.hash);
  expect(healthyCurrent).toBeDefined();
  const corrupt = [9, 8, 7];
  await replaceResume(page, failed.hash, corrupt);

  await page.goto("/");
  await expect(
    page.getByRole("button", { name: "Resume adventure" }),
  ).toBeEnabled();
  await preventPlayback(page);
  await page.getByRole("button", { name: "Resume adventure" }).click();
  const recovery = page.getByRole("dialog", { name: "Resume needs attention" });
  await expect(recovery).toBeVisible();
  await recovery.getByRole("button", { name: "Close" }).click();
  await page.getByRole("button", { name: "Library", exact: true }).click();
  await page.getByRole("button", { name: "View Healthy" }).click();
  await page.getByRole("button", { name: "Play Healthy" }).click();
  await expect(page.getByLabel(/^Game screen/)).toBeVisible();
  await expect
    .poll(() =>
      page.evaluate(async () => {
        const emulator = (window as unknown as { emulator: Emulator }).emulator;
        return Array.from(await emulator.saveState());
      }),
    )
    .toEqual(healthyCurrent);
  expect(await storedResume(page, failed.hash)).toEqual(corrupt);
});

test("a missing target checkpoint pauses the previously running machine", async ({
  page,
}) => {
  await enterPlayer(page);
  const missing = await importAndCheckpoint(page, 48, "Missing target");
  await importAndCheckpoint(page, 49, "Running source");
  await replaceResume(page, missing.hash);
  await page.getByRole("button", { name: "Library", exact: true }).click();
  await page.getByRole("button", { name: "View Missing target" }).click();
  await page.evaluate(async () => {
    await (window as unknown as { emulator: Emulator }).emulator.play();
  });
  const cycle = async () =>
    page.evaluate(async () =>
      Number(
        (await (window as unknown as { emulator: Emulator }).emulator.inspect())
          .cycle,
      ),
    );
  const running = await cycle();
  await expect.poll(cycle).toBeGreaterThan(running);
  await page.getByRole("button", { name: "Play Missing target" }).click();
  await expect(
    page.getByRole("dialog", { name: "Resume needs attention" }),
  ).toBeVisible();
  const stopped = await cycle();
  await page.waitForTimeout(200);
  expect(await cycle()).toBe(stopped);
  expect(await storedResume(page, missing.hash)).toBeUndefined();
});
