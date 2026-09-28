import { expect, test, type Page } from "@playwright/test";
declare global {
  interface Window {
    player: import("../../src/player").Player;
  }
}

const loop = {
  name: "Two runs.gba",
  mimeType: "application/octet-stream",
  buffer: Buffer.from([42, 0, 160, 227, 254, 255, 255, 234]),
};
const selectedRun = (page: Page) =>
  page.getByRole("combobox", { name: "Playthrough" });
async function chooseRun(page: Page, name: string) {
  await selectedRun(page).click();
  await page.getByRole("option", { name, exact: true }).click();
}
async function openNewRun(page: Page) {
  await selectedRun(page).click();
  await page.getByRole("option", { name: /New playthrough/ }).click();
}

test("Reset retains progress, while playthroughs isolate checkpoints and quick slots", async ({
  page,
}) => {
  // A native prompt is not an accessible in-page naming form.
  page.on("dialog", (dialog) => dialog.dismiss());
  await page.goto("/");
  await page.getByRole("button", { name: "Open player" }).click();
  await page.getByLabel("Open ROM", { exact: true }).setInputFiles(loop);
  await expect(page.getByRole("heading", { name: "Two runs" })).toBeVisible();
  await expect(selectedRun(page)).toHaveText(/Playthrough 1/);
  await expect(page.getByRole("button", { name: "Reset" })).toBeEnabled();

  const first = await page.evaluate(async () => {
    const player = window.player;
    await player.pause();
    await player.emulator.runUntil(
      (await player.emulator.inspect()).cycle + 280896n * 4n,
    );
    player.latest = await player.emulator.inspect();
    await player.checkpointState();
    return {
      id: player.activeRef!.playthroughId,
      cycle: Number(player.latest.cycle),
    };
  });
  await page.getByRole("button", { name: /Quick save/ }).click();
  await expect(page.locator(".sr-only[role=status]")).toContainText(
    "Saved slot 1",
  );
  await page.getByRole("button", { name: /Reset/ }).click();
  await expect(page.getByRole("button", { name: /Reset/ })).toBeEnabled();
  const afterReset = await page.evaluate(async () => {
    const player = window.player;
    return {
      id: player.activeRef!.playthroughId,
      cycle: Number((await player.emulator.inspect()).cycle),
      checkpoint: Number(
        (await player.library.getResume(player.activeRef!))!.cycle,
      ),
    };
  });
  expect(afterReset.id).toBe(first.id);
  expect(afterReset.cycle).toBeLessThan(first.cycle);
  expect(afterReset.checkpoint).toBe(first.cycle);

  const beforeSwitch = await page.evaluate(async () => {
    const player = window.player;
    await player.emulator.runUntil(
      (await player.emulator.inspect()).cycle + 280896n * 2n,
    );
    await player.checkpointState();
    return Number((await player.emulator.inspect()).cycle);
  });

  await openNewRun(page);
  const create = page.getByRole("dialog", { name: "New playthrough" });
  await expect(create).toBeVisible();
  await create.getByLabel("Playthrough name").fill("Second adventure");
  await create.getByRole("button", { name: "Create playthrough" }).click();
  await expect(create).toBeHidden();
  await expect(selectedRun(page)).not.toContainText("Playthrough 1");
  await expect(selectedRun(page)).toContainText("Second adventure");
  await page.getByRole("button", { name: "Rename playthrough" }).click();
  const rename = page.getByRole("dialog", { name: "Rename playthrough" });
  await expect(rename.getByLabel("Playthrough name")).toHaveValue(
    "Second adventure",
  );
  await rename.getByLabel("Playthrough name").fill("Second renamed");
  await rename.getByRole("button", { name: "Save name" }).click();
  await expect(rename).toBeHidden();
  await expect(selectedRun(page)).toContainText("Second renamed");
  const second = await page.evaluate(async () => {
    const player = window.player;
    return {
      id: player.activeRef!.playthroughId,
      cycle: Number((await player.emulator.inspect()).cycle),
    };
  });
  expect(second.id).not.toBe(first.id);
  expect(second.cycle).toBeLessThan(beforeSwitch);
  await page.getByRole("button", { name: /Quick load/ }).click();
  await expect(page.locator(".sr-only[role=status]")).toContainText(
    "Quick slot 1 is empty",
  );

  await chooseRun(page, "Playthrough 1");
  await expect
    .poll(() =>
      page.evaluate(async () => {
        const player = window.player;
        return Number((await player.emulator.inspect()).cycle);
      }),
    )
    .toBe(beforeSwitch);
  await page.getByRole("button", { name: /Quick load/ }).click();
  await expect(page.locator(".sr-only[role=status]")).toContainText(
    "Loaded slot 1",
  );

  await chooseRun(page, "Second renamed");
  await expect(selectedRun(page)).toContainText("Second renamed");
  await expect(page.getByRole("button", { name: "Reset" })).toBeEnabled();
  const secondResume = await page.evaluate(async () => {
    const player = window.player;
    await player.pause();
    await player.emulator.runUntil(
      (await player.emulator.inspect()).cycle + 280896n * 3n,
    );
    await player.checkpointState();
    return Number((await player.emulator.inspect()).cycle);
  });
  await page.goto("/");
  await page.getByRole("button", { name: "Resume adventure" }).click();
  await expect(selectedRun(page)).toContainText("Second renamed");
  expect(
    await page.evaluate(async () => {
      const player = window.player;
      return Number((await player.emulator.inspect()).cycle);
    }),
  ).toBeGreaterThanOrEqual(secondResume);
});

test("Reset boots the current battery save and preserves play/pause state", async ({
  page,
}) => {
  const rom = Buffer.alloc(272);
  [0xe3a0040e, 0xe3a0102a, 0xe5c01000, 0xeafffffe].forEach((opcode, index) =>
    rom.writeUInt32LE(opcode, index * 4),
  );
  rom.write("SRAM_V110", 256);
  await page.goto("/");
  await page.getByRole("button", { name: "Open player" }).click();
  await page.getByLabel("Open ROM", { exact: true }).setInputFiles({
    name: "Battery.gba",
    mimeType: "application/octet-stream",
    buffer: rom,
  });
  await expect(page.getByRole("button", { name: "Reset" })).toBeEnabled();
  await expect
    .poll(() =>
      page.evaluate(
        async () =>
          (
            await (
              window as unknown as {
                emulator: import("@evenfall/gba").Emulator;
              }
            ).emulator.exportSave()
          )[0],
      ),
    )
    .toBe(42);
  await page.getByLabel(/^Game screen/).focus();
  await page.keyboard.press("Space");
  await expect(page.getByRole("button", { name: "Reset" })).toBeEnabled();
  const before = await page.evaluate(async () => {
    const player = window.player;
    return Number((await player.emulator.inspect()).cycle);
  });
  await page.getByRole("button", { name: "Reset" }).click();
  await expect(page.getByRole("button", { name: "Reset" })).toBeEnabled();
  const after = await page.evaluate(async () => {
    const player = window.player;
    return {
      cycle: Number((await player.emulator.inspect()).cycle),
      save: (await player.emulator.exportSave())[0],
      playing: player.state.playing,
    };
  });
  expect(after.cycle).toBeLessThan(before);
  expect(after.save).toBe(42);
  expect(after.playing).toBe(false);

  await page.evaluate(async () => {
    const player = window.player;
    await player.play();
  });
  await page.getByRole("button", { name: "Reset" }).click();
  await expect
    .poll(() =>
      page.evaluate(async () => {
        const player = window.player;
        return player.state.playing;
      }),
    )
    .toBe(true);
});

test("storage failure keeps a supplied ROM playable without attaching the prior save", async ({
  page,
}) => {
  await page.goto("/");
  await page.getByRole("button", { name: "Open player" }).click();
  await page.getByLabel("Open ROM", { exact: true }).setInputFiles(loop);
  await expect(page.getByRole("button", { name: "Reset" })).toBeEnabled();
  const oldHash = await page.evaluate(() => window.player.state.game!.hash);
  await page.evaluate(() => {
    window.player.library.put = async () => {
      throw new Error("QuotaExceededError");
    };
  });
  await page.getByLabel("Open ROM", { exact: true }).setInputFiles({
    ...loop,
    name: "Transient.gba",
    buffer: Buffer.from([43, 0, 160, 227, 254, 255, 255, 234]),
  });
  await expect(page.getByRole("heading", { name: "Transient" })).toBeVisible();
  await expect(page.locator(".save-indicator")).toContainText(
    "Save unavailable",
  );
  const transient = await page.evaluate(async (oldHash) => {
    const player = window.player;
    await player.pause();
    return {
      gameHash: player.state.game?.hash,
      cpuHash: (await player.emulator.inspect()).romHash,
      oldHashStillListed: player.state.games.some(
        (game) => game.hash === oldHash,
      ),
      lock: Boolean(player.releaseLock),
      run: player.activeRef,
      romBytes: (await player.emulator.exportRom()).length,
    };
  }, oldHash);
  expect(transient.gameHash).toBe(transient.cpuHash);
  expect(transient.gameHash).not.toBe(oldHash);
  expect(transient.oldHashStillListed).toBe(true);
  expect(transient.lock).toBe(false);
  expect(transient.run).toBeUndefined();
  expect(transient.romBytes).toBe(loop.buffer.length);
});

test("a corrupt target run cannot replace the current machine or writer", async ({
  page,
}) => {
  await page.goto("/");
  await expect(page.getByLabel("Open ROM", { exact: true })).toBeEnabled();
  await page.getByLabel("Open ROM", { exact: true }).setInputFiles(loop);
  await expect(page.getByRole("button", { name: "Reset" })).toBeEnabled();
  const result = await page.evaluate(async () => {
    const player = window.player;
    await player.pause();
    const original = player.activeRef!;
    const before = Array.from(await player.emulator.saveState());
    const run = await player.library.createPlaythrough(
      original.hash,
      "Corrupt",
    );
    await player.library.saveResume(
      { hash: original.hash, playthroughId: run.id },
      new Uint8Array([1, 2, 3]),
      {},
      99n,
    );
    await player.library.selectPlaythrough(original);
    let rejected = false;
    try {
      await player.switchPlaythrough(run.id);
    } catch {
      rejected = true;
    }
    return {
      rejected,
      original,
      selected: player.activeRef,
      before,
      after: Array.from(await player.emulator.saveState()),
      hasWriter: Boolean(player.releaseLock),
      corrupt: Array.from(
        (await player.library.getResume({
          hash: original.hash,
          playthroughId: run.id,
        }))!.data,
      ),
    };
  });
  expect(result.rejected).toBe(true);
  expect(result.selected).toEqual(result.original);
  expect(result.after).toEqual(result.before);
  expect(result.hasWriter).toBe(true);
  expect(result.corrupt).toEqual([1, 2, 3]);
});

test("naming dialogs contain focus and cancel without changing the adventure or playback mode", async ({
  page,
}) => {
  await page.goto("/");
  await expect(page.getByLabel("Open ROM", { exact: true })).toBeEnabled();
  await page.getByLabel("Open ROM", { exact: true }).setInputFiles(loop);
  const create = selectedRun(page);
  await expect(create).toBeEnabled();
  await expect
    .poll(() => page.evaluate(() => window.player.state.playing))
    .toBe(true);
  await openNewRun(page);
  const dialog = page.getByRole("dialog", { name: "New playthrough" });
  const input = dialog.getByLabel("Playthrough name");
  await expect(input).toBeFocused();
  await expect
    .poll(() => page.evaluate(() => window.player.state.playing))
    .toBe(false);
  await input.pressSequentially("Unsaved adventure");
  await input.press("Tab");
  await expect(
    dialog.getByRole("button", { name: "Cancel", exact: true }),
  ).toBeFocused();
  await page.keyboard.press("Tab");
  await expect(
    dialog.getByRole("button", { name: "Create playthrough" }),
  ).toBeFocused();
  await page.keyboard.press("Tab");
  await expect(
    dialog.getByRole("button", { name: "Close dialog" }),
  ).toBeFocused();
  expect(await page.evaluate(() => window.player.state.boosting)).toBe(false);
  await page.keyboard.press("Escape");
  await expect(dialog).toBeHidden();
  await expect(create).toBeFocused();
  await expect
    .poll(() => page.evaluate(() => window.player.state.playing))
    .toBe(true);
  await expect(selectedRun(page)).toContainText("Playthrough 1");
  await selectedRun(page).click();
  await expect(page.getByRole("option")).toHaveCount(2);
  await page.keyboard.press("Escape");

  await page.evaluate(() => window.player.pause());
  const rename = page.getByRole("button", { name: "Rename playthrough" });
  await rename.click();
  const edit = page.getByRole("dialog", { name: "Rename playthrough" });
  await expect(edit.getByLabel("Playthrough name")).toBeFocused();
  await expect(edit.getByLabel("Playthrough name")).toHaveValue(
    "Playthrough 1",
  );
  expect(
    await edit
      .getByLabel("Playthrough name")
      .evaluate((element: HTMLInputElement) =>
        element.value.slice(element.selectionStart!, element.selectionEnd!),
      ),
  ).toBe("Playthrough 1");
  await edit.getByLabel("Playthrough name").fill("Discard this name");
  await edit.getByRole("button", { name: "Cancel", exact: true }).click();
  await expect(edit).toBeHidden();
  await expect(rename).toBeFocused();
  expect(await page.evaluate(() => window.player.state.playing)).toBe(false);
  await expect(selectedRun(page)).toContainText("Playthrough 1");
});

test("the playthrough selector handles keys without driving or boosting the game", async ({
  page,
}) => {
  await page.goto("/");
  await expect(page.getByLabel("Open ROM", { exact: true })).toBeEnabled();
  await page.getByLabel("Open ROM", { exact: true }).setInputFiles(loop);
  await expect(selectedRun(page)).toBeEnabled();
  await page.getByRole("button", { name: "Fullscreen" }).focus();
  await page.evaluate(() => {
    window.player.keyboard = 64;
  });
  await selectedRun(page).focus();
  expect(await page.evaluate(() => window.player.keyboard)).toBe(0);
  await page.keyboard.press("ArrowDown");
  expect(await page.evaluate(() => window.player.keyboard)).toBe(0);
  await page.keyboard.press("Escape");
  await expect
    .poll(() => page.evaluate(() => window.player.state.playing))
    .toBe(true);
  await selectedRun(page).focus();
  await page.keyboard.press("Tab");
  await expect(
    page.getByRole("button", { name: "Rename playthrough" }),
  ).toBeFocused();
  expect(await page.evaluate(() => window.player.state.boosting)).toBe(false);
});

test("naming forms validate Unicode names and preserve failed edits for retry on mobile", async ({
  page,
}) => {
  await page.setViewportSize({ width: 1280, height: 800 });
  await page.goto("/");
  await expect(page.getByLabel("Open ROM", { exact: true })).toBeEnabled();
  await page.getByLabel("Open ROM", { exact: true }).setInputFiles(loop);
  await openNewRun(page);
  const create = page.getByRole("dialog", { name: "New playthrough" });
  const name = create.getByLabel("Playthrough name");
  await expect(name).toBeFocused();
  await page.evaluate(() => document.fonts.ready);
  await page.screenshot({
    path: "../artifacts/playthrough-dialogs/new-desktop.png",
  });
  await name.fill("   ");
  await name.press("Enter");
  await expect(create.getByRole("alert")).toHaveText(
    "Give your playthrough a name.",
  );
  await expect(name).toHaveAttribute("aria-invalid", "true");
  await name.fill("🌙".repeat(161));
  await name.press("Enter");
  await expect(create.getByRole("alert")).toHaveText(
    "Use 160 characters or fewer.",
  );
  await page.setViewportSize({ width: 360, height: 480 });
  await create
    .getByRole("button", { name: "Create playthrough" })
    .scrollIntoViewIfNeeded();
  const bounds = await create.boundingBox();
  expect(bounds!.x).toBeGreaterThanOrEqual(0);
  expect(bounds!.x + bounds!.width).toBeLessThanOrEqual(360);
  expect(bounds!.y).toBeGreaterThanOrEqual(0);
  expect(bounds!.y + bounds!.height).toBeLessThanOrEqual(480);
  const titleBounds = await create
    .getByRole("heading", { name: "New playthrough" })
    .boundingBox();
  expect(titleBounds!.y).toBeGreaterThanOrEqual(bounds!.y);
  await page.screenshot({
    path: "../artifacts/playthrough-dialogs/validation-mobile.png",
  });
  await name.fill("🌙".repeat(160));
  await name.press("Enter");
  await expect(create).toBeHidden();
  const selected = selectedRun(page);
  await expect(selected).toContainText("🌙".repeat(160));

  await page.setViewportSize({ width: 390, height: 640 });
  await page.getByRole("button", { name: "Rename playthrough" }).click();
  const rename = page.getByRole("dialog", { name: "Rename playthrough" });
  await rename.getByLabel("Playthrough name").fill("  Moonlight run  ");
  // Fail one browser storage write, without replacing the Library or Player.
  await page.evaluate(async () => {
    await window.player.pause();
    const put = IDBObjectStore.prototype.put;
    IDBObjectStore.prototype.put = function (...args) {
      if (this.name === "playthroughs") {
        IDBObjectStore.prototype.put = put;
        throw new DOMException(
          "Storage full. Free space and try again.",
          "QuotaExceededError",
        );
      }
      return put.apply(this, args);
    };
  });
  await rename.getByRole("button", { name: "Save name" }).click();
  await expect(rename.getByRole("alert")).toContainText("Storage full");
  await expect(rename.getByLabel("Playthrough name")).toHaveValue(
    "  Moonlight run  ",
  );
  await expect(rename.getByRole("button", { name: "Save name" })).toBeEnabled();
  await page.screenshot({
    path: "../artifacts/playthrough-dialogs/retry-mobile.png",
  });
  await rename.getByRole("button", { name: "Save name" }).click();
  await expect(rename).toBeHidden();
  await expect(selected).toContainText("Moonlight run");
  await page.getByRole("button", { name: "Rename playthrough" }).click();
  await expect(rename.getByLabel("Playthrough name")).toHaveValue(
    "Moonlight run",
  );
  await page.screenshot({
    path: "../artifacts/playthrough-dialogs/rename-mobile.png",
  });
  await page.mouse.click(4, 4);
  await expect(rename).toBeHidden();
});

test("deleting a playthrough requires confirmation and keeps the other adventure intact", async ({
  page,
}) => {
  await page.goto("/");
  await expect(page.getByLabel("Open ROM", { exact: true })).toBeEnabled();
  await page.getByLabel("Open ROM", { exact: true }).setInputFiles(loop);
  await expect(selectedRun(page)).toBeEnabled();
  const first = await page.evaluate(async () => {
    const player = window.player;
    await player.pause();
    await player.emulator.runUntil(
      (await player.emulator.inspect()).cycle + 280896n * 3n,
    );
    player.latest = await player.emulator.inspect();
    await player.checkpointState();
    return {
      id: player.activeRef!.playthroughId,
      cycle: Number(player.latest.cycle),
    };
  });
  await openNewRun(page);
  const create = page.getByRole("dialog", { name: "New playthrough" });
  await create.getByLabel("Playthrough name").fill("Delete me");
  await create.getByRole("button", { name: "Create playthrough" }).click();
  await expect(create).toBeHidden();
  const second = await page.evaluate(async () => {
    const player = window.player;
    const ref = player.activeRef!;
    await player.library.addAsset(
      `quick-state:${JSON.stringify([ref.hash, ref.playthroughId, 1])}`,
      new Uint8Array([7]),
    );
    return ref;
  });
  const preservedCycle = await page.evaluate(
    async ({ hash, id }) =>
      Number(
        (await window.player.library.getResume({ hash, playthroughId: id }))!
          .cycle,
      ),
    { hash: second.hash, id: first.id },
  );
  await page.evaluate(() => {
    const emulator = window.player.emulator;
    const load = emulator.loadState.bind(emulator);
    emulator.loadState = async (bytes) => {
      const cpu = await load(bytes);
      (window as unknown as { restoredCycle: number }).restoredCycle = Number(
        cpu.cycle,
      );
      return cpu;
    };
    const boot = emulator.load.bind(emulator);
    emulator.load = async (...args) => {
      const cpu = await boot(...args);
      (window as unknown as { bootCycle: number }).bootCycle = Number(
        cpu.cycle,
      );
      return cpu;
    };
  });
  await selectedRun(page).click();
  await expect(
    page.getByRole("option", { name: /New playthrough/ }),
  ).toBeVisible();
  await page.evaluate(() => document.fonts.ready);
  await page.screenshot({
    path: "../artifacts/playthrough-selector-desktop.png",
  });
  await page.keyboard.press("Escape");

  const trash = page.getByRole("button", {
    name: "Delete playthrough",
    exact: true,
  });
  await trash.click();
  const confirm = page.getByRole("dialog", { name: "Delete playthrough?" });
  await expect(confirm).toContainText("Delete me");
  await expect(confirm).toContainText("Any cloud backup remains");
  await expect(confirm.getByRole("button", { name: "Cancel" })).toBeFocused();
  await confirm.getByRole("button", { name: "Cancel" }).click();
  await expect(confirm).toBeHidden();
  expect(
    await page.evaluate(() => window.player.activeRef!.playthroughId),
  ).toBe(second.playthroughId);

  await trash.click();
  await page.setViewportSize({ width: 390, height: 640 });
  await page.screenshot({ path: "../artifacts/playthrough-delete-mobile.png" });
  await confirm.getByRole("button", { name: "Delete playthrough" }).click();
  await expect(confirm).toBeHidden();
  await expect(selectedRun(page)).toContainText("Playthrough 1");
  await page.locator(".player-tools").screenshot({
    path: "../artifacts/playthrough-toolbar-mobile.png",
  });
  await selectedRun(page).click();
  await expect(
    page.getByRole("option", { name: /New playthrough/ }),
  ).toBeVisible();
  await page.locator(".playthrough-select-popup").screenshot({
    path: "../artifacts/playthrough-selector-mobile.png",
  });
  await page.keyboard.press("Escape");
  const result = await page.evaluate(
    async ({ second }) => {
      const player = window.player;
      return {
        active: player.activeRef!.playthroughId,
        restoredCycle: (window as unknown as { restoredCycle: number })
          .restoredCycle,
        deleted: await player.library.getPlaythrough(second),
        save: await player.library.getResume(second),
        slot: await player.library.asset(
          `quick-state:${JSON.stringify([second.hash, second.playthroughId, 1])}`,
        ),
      };
    },
    { second },
  );
  expect(result.active).toBe(first.id);
  expect(result.restoredCycle).toBe(preservedCycle);
  expect(result.deleted).toBeUndefined();
  expect(result.save).toBeUndefined();
  expect(result.slot).toBeUndefined();

  await trash.click();
  await confirm.getByRole("button", { name: "Delete playthrough" }).click();
  await expect(confirm).toBeHidden();
  await expect(selectedRun(page)).toContainText("Playthrough 1");
  const fresh = await page.evaluate(
    async ({ hash, id }) => ({
      id: window.player.activeRef!.playthroughId,
      bootCycle: (window as unknown as { bootCycle: number }).bootCycle,
      oldResume: await window.player.library.getResume({
        hash,
        playthroughId: id,
      }),
    }),
    { hash: second.hash, id: first.id },
  );
  expect(fresh.id).not.toBe(first.id);
  expect(fresh.bootCycle).toBe(0);
  expect(fresh.oldResume).toBeUndefined();
  await page.evaluate(() => window.player.checkpointState());
  await page.goto("/");
  await page.getByRole("button", { name: "Resume adventure" }).click();
  await expect(selectedRun(page)).toBeVisible();
  expect(
    await page.evaluate(() => window.player.activeRef!.playthroughId),
  ).toBe(fresh.id);
});
