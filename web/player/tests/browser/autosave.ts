import { expect, type Page } from "@playwright/test";

/** Install page.clock before loading the player, then observe a real timer tick. */
export async function advanceAutosave(page: Page) {
  await page.evaluate(() => {
    const host = window as typeof window & {
      autosaveAttempts: Promise<void>[];
    };
    host.autosaveAttempts = [];
    const persist = window.player.persist.bind(window.player);
    window.player.persist = (...args) => {
      const attempt = persist(...args);
      host.autosaveAttempts.push(attempt);
      return attempt;
    };
  });
  await page.clock.fastForward(5_000);
  // Negative storage assertions are meaningful only after persist was called
  // by the interval and all its asynchronous work has settled.
  const attempts = await page.evaluate(async () => {
    const host = window as typeof window & {
      autosaveAttempts: Promise<void>[];
    };
    await Promise.all(host.autosaveAttempts);
    return host.autosaveAttempts.length;
  });
  expect(attempts).toBeGreaterThan(0);
}
