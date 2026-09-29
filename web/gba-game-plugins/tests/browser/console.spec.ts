// The player with this package's dashboards, driven by user-supplied ROMs and
// states. Each test skips unless its ZIGBA_* fixture variables are set.
import { test, expect, type Page } from "@playwright/test";
import { readFile } from "node:fs/promises";
import { createHash } from "node:crypto";
import type { Emulator } from "@evenfall/gba";

test.use({ hasTouch: true });

const sha = (bytes: Uint8Array) =>
  createHash("sha256").update(bytes).digest("hex");
const romSha = (bytes: Uint8Array) =>
  createHash("sha1").update(bytes).digest("hex");
async function ready(page: Page) {
  await page.goto("/");
  await page.getByRole("button", { name: "Open player" }).click();
  await expect(page.getByLabel("Open ROM", { exact: true })).toBeEnabled();
}
async function importState(page: Page, path: string) {
  await page.getByRole("button", { name: "More options" }).click();
  await expect(page.getByLabel("Import state", { exact: true })).toBeEnabled();
  await page.getByLabel("Import state", { exact: true }).setInputFiles(path);
  await expect(
    page.getByRole("dialog", { name: "Saves and options" }),
  ).toHaveCount(0);
}
async function runOneFrame(page: Page) {
  await page.evaluate(async () => {
    const emulator = (window as unknown as { emulator: Emulator }).emulator;
    const cpu = await emulator.inspect();
    await emulator.runUntil(cpu.cycle + 280896n);
  });
}

async function screenFillsColumn(page: Page) {
  const geometry = await page.locator(".console-player").evaluate((player) => {
    const display = player.querySelector(".game-display")!;
    const canvas = display.querySelector("canvas")!;
    const container = player.querySelector(".game-viewport")!;
    const style = getComputedStyle(container);
    return {
      available:
        container.clientWidth -
        parseFloat(style.paddingLeft) -
        parseFloat(style.paddingRight),
      availableHeight:
        container.clientHeight -
        parseFloat(style.paddingTop) -
        parseFloat(style.paddingBottom),
      display: display.getBoundingClientRect().width,
      displayHeight: display.getBoundingClientRect().height,
      canvas: canvas.getBoundingClientRect().width,
      ratio:
        canvas.getBoundingClientRect().width /
        canvas.getBoundingClientRect().height,
    };
  });
  expect(geometry.display).toBeLessThanOrEqual(geometry.available + 1);
  expect(geometry.displayHeight).toBeLessThanOrEqual(
    geometry.availableHeight + 1,
  );
  expect(geometry.canvas).toBeGreaterThan(0);
  expect(geometry.display - geometry.canvas).toBeCloseTo(12, 0);
  expect(geometry.ratio).toBeCloseTo(1.5, 2);
  expect(
    await page.evaluate(
      () => document.documentElement.scrollHeight <= innerHeight,
    ),
  ).toBe(true);
  for (const element of await page
    .locator(".game-display, .quick-bar, .touch-pad")
    .all()) {
    const box = await element.boundingBox();
    expect(box!.y).toBeGreaterThanOrEqual(0);
    expect(box!.y + box!.height).toBeLessThanOrEqual(
      page.viewportSize()!.height + 1,
    );
  }
}

test("Redux trainer search uses real ROM teams and portraits with an inactive-battle observation", async ({
  page,
}) => {
  const rom = process.env.ZIGBA_REDUX_ROM;
  const state = process.env.ZIGBA_REDUX_STATE;
  test.skip(!rom || !state, "User-supplied Redux ROM and snapshot required");
  await page.setViewportSize({ width: 1672, height: 1100 });
  await ready(page);
  await page.getByLabel("Open ROM", { exact: true }).setInputFiles(rom!);
  await importState(page, state!);
  // The supplied save is a battle. Change only captured copies, never guest memory,
  // to exercise out-of-combat composition and changing settings with real ROM data.
  await page.evaluate(() => {
    const emulator = (window as unknown as { emulator: Emulator }).emulator;
    const original = emulator.captureMemory.bind(emulator);
    emulator.captureMemory = async () => {
      const snapshot = await original();
      if (!snapshot) return snapshot;
      return {
        ...snapshot,
        regions: snapshot.regions.map((region) => {
          const bytes = region.bytes.slice();
          if (region.address === 0x03000000) bytes[0x3859] &= ~2;
          return { ...region, bytes };
        }),
      };
    };
    emulator.dispatchEvent(new Event("frame"));
  });
  // This page is narrower than 17:10, so the panels share the player's dock;
  // outside a battle it opens the team, and trainer search is under Info.
  await page.getByRole("tab", { name: "Info", exact: true }).click();
  const input = page.getByRole("combobox", { name: "Search trainers" });
  await expect(input).toBeVisible();
  await input.fill("May");
  await expect(page.getByRole("option").first()).toContainText(
    "Route 103 · Your starter: Mudkip",
  );
  await expect(page.getByRole("option").nth(3)).toContainText("Rustboro");
  await expect(page.getByRole("option").nth(9)).toContainText(
    "Meteor Falls · Partner",
  );
  await expect(page.getByRole("option").nth(15)).toContainText(
    "Lilycove / Route 103 rematch",
  );
  await expect
    .poll(() =>
      page
        .locator(".trainer-search-dropdown img")
        .evaluateAll(
          (images) =>
            images.length >= 18 &&
            images.every(
              (image) => (image as HTMLImageElement).naturalWidth === 64,
            ),
        ),
    )
    .toBe(true);
  await page.evaluate(() => document.fonts.ready);
  await page.locator(".trainer-search-dropdown").screenshot({
    path: "../../artifacts/hud/trainer-search-may.png",
  });
  await input.fill("May partner");
  await expect(page.getByRole("option")).toHaveCount(3);
  await page.locator(".trainer-search-dropdown").screenshot({
    path: "../../artifacts/hud/trainer-search-may-partner.png",
  });
  await input.fill("roxa");
  await expect(page.getByRole("option")).toHaveCount(5);
  await expect(page.getByRole("option").first()).toContainText(
    "Initial encounter",
  );
  await expect(page.getByRole("option").last()).toContainText(
    "Doubles · Legends",
  );
  await expect
    .poll(() =>
      page
        .locator(".trainer-search-dropdown img")
        .evaluateAll(
          (images) =>
            images.length === 5 &&
            images.every(
              (image) => (image as HTMLImageElement).naturalWidth === 64,
            ),
        ),
    )
    .toBe(true);
  await page.evaluate(() => document.fonts.ready);
  await page
    .locator(".trainer-search-dropdown")
    .screenshot({ path: "../../artifacts/hud/trainer-search-dropdown.png" });
  await page.getByRole("option").first().click();
  const preview = page.locator(".trainer-opponent-column");
  await expect(page.getByRole("option")).toHaveCount(0);
  await expect(preview.locator(".roster-card")).toHaveCount(5);
  await expect(preview).toContainText("Anorith");
  await expect(
    preview.getByRole("button", { name: "Stat: Attack, total 49" }),
  ).toBeVisible();
  await expect(
    preview.getByRole("button", { name: "Item: Focus Sash" }),
  ).toBeVisible();
  await expect
    .poll(() =>
      preview
        .locator(".roster-card img,.selected-summary img")
        .evaluateAll(
          (images) =>
            images.length === 6 &&
            images.every(
              (image) => (image as HTMLImageElement).naturalWidth === 64,
            ),
        ),
    )
    .toBe(true);
  await page.getByRole("button", { name: "Dismiss notification" }).click();
  await input.fill("Roxanne");
  await page.screenshot({
    path: "../../artifacts/hud/trainer-search-desktop.png",
    fullPage: true,
  });
  await input.press("Escape");
  await expect(input).toHaveValue("Roxanne");
  await expect(page.getByRole("option")).toHaveCount(0);
  await preview.screenshot({
    path: "../../artifacts/hud/trainer-preview-desktop.png",
  });
  await preview.getByRole("button", { name: "Stat: Attack, total 49" }).focus();
  await expect(
    page
      .getByRole("tooltip")
      .locator("dt", { hasText: /^EV$/ })
      .locator("+ dd"),
  ).toHaveText("0");
  await page.keyboard.press("Escape");
  await page.setViewportSize({ width: 390, height: 844 });
  // Phones open the game panels in a sheet below the screen.
  await page.getByRole("button", { name: "Show Team and Info" }).click();
  await page.getByRole("tab", { name: "Info", exact: true }).click();
  await input.scrollIntoViewIfNeeded();
  await input.click();
  await expect(page.getByRole("option")).toHaveCount(5);
  const bounds = await page.locator(".trainer-search-dropdown").boundingBox();
  expect(bounds!.y).toBeGreaterThanOrEqual(0);
  expect(bounds!.y + bounds!.height).toBeLessThanOrEqual(844);
  expect(
    await page.evaluate(
      () => document.documentElement.scrollWidth <= innerWidth,
    ),
  ).toBe(true);
  await page.screenshot({
    path: "../../artifacts/hud/trainer-search-mobile.png",
  });
  await page.getByRole("option").last().click();
  await expect(preview.locator(".selected-trainer-summary")).toContainText(
    "Doubles · Legends",
  );
  await expect(preview.locator(".roster-card")).toHaveCount(6);
  await page.mouse.move(0, 0);
  await page.keyboard.press("Escape");
  await expect(page.getByRole("tooltip")).toHaveCount(0);
  await preview.screenshot({
    path: "../../artifacts/hud/trainer-preview-mobile.png",
  });
});

test("new HnS console reads the real wild battle and clears opponents on timeline replacement", async ({
  page,
}) => {
  const rom = process.env.ZIGBA_HNS_ROM;
  const battle = process.env.ZIGBA_HNS_BATTLE;
  const party = process.env.ZIGBA_HNS_STATE;
  test.skip(
    !rom || !battle || !party,
    "Supply the hash-verified HnS ROM, battle.state and game2.state",
  );
  expect(romSha(await readFile(rom!))).toBe(
    "a4be81f8330d7f223a3ab847c3da822fe6f8a2e0",
  );
  expect(sha(await readFile(battle!))).toBe(
    "5515f43a8119632270b2f64a447917fd826bc7cf16655f3276bbb0f3c4d42e54",
  );
  expect(sha(await readFile(party!))).toBe(
    "71817992f9053560808493c79c3e65b5acba2b4499375cc52d76806b2107fb5f",
  );
  const errors: string[] = [];
  page.on("pageerror", (error) => errors.push(error.message));
  await page.setViewportSize({ width: 1672, height: 941 });
  await ready(page);
  await page.getByLabel("Open ROM", { exact: true }).setInputFiles(rom!);
  await expect(page.locator('[data-game-layout="pokemon"]')).toBeVisible();
  await importState(page, battle!);
  const team = page.getByRole("complementary", {
    name: "Your Pokémon",
    exact: true,
  });
  const opponent = page.getByRole("complementary", {
    name: "Opponent",
    exact: true,
  });
  await expect(team.getByRole("listitem")).toHaveCount(3);
  await expect(team.getByRole("listitem").first()).toContainText("Totodile");
  await expect(team.getByRole("listitem").first()).toContainText("26 / 33");
  await expect(opponent.getByRole("listitem")).toHaveCount(1);
  await expect(opponent.getByRole("listitem").first()).toContainText("Hoppip");
  await expect(opponent.getByRole("listitem").first()).toContainText("16 / 16");
  await expect(
    opponent.getByRole("region", { name: "Selected Pokémon" }),
  ).toContainText("In battle");
  await expect(page.locator(".scene-heading")).toContainText("Route 29");
  await expect(page.locator(".scene-heading")).toContainText("Wild battle");
  await expect(page.getByLabel("Arena status")).toContainText(
    "WeatherUnavailable",
  );
  await expect(page.locator(".scene-heading")).not.toContainText("Turn");
  await expect(page.getByText("OK", { exact: true })).toHaveCount(0);
  await expect
    .poll(() =>
      page
        .locator(".roster-card img,.selected-summary img")
        .evaluateAll(
          (images) =>
            images.length === 6 &&
            images.every(
              (image) => (image as HTMLImageElement).naturalWidth === 64,
            ),
        ),
    )
    .toBe(true);
  await page.evaluate(() => document.fonts.ready);
  await page.getByRole("button", { name: "Dismiss notification" }).click();
  await page.mouse.move(0, 0);
  expect(
    await page.evaluate(
      () => document.documentElement.scrollWidth <= innerWidth,
    ),
  ).toBe(true);
  await page.screenshot({
    path: "../../artifacts/hud/reboot-battle-desktop.png",
    fullPage: true,
  });
  for (const width of [2048, 1100, 900, 390]) {
    await page.setViewportSize({ width, height: 941 });
    await screenFillsColumn(page);
    expect(
      await page.evaluate(
        () => document.documentElement.scrollWidth <= innerWidth,
      ),
    ).toBe(true);
    if (width === 2048 || width === 900)
      await page.screenshot({
        path: `../../artifacts/hud/player-width-${width}.png`,
        fullPage: true,
      });
  }
  await page.setViewportSize({ width: 1672, height: 941 });
  const ability = team.getByRole("button", { name: "Ability: TORRENT" });
  await ability.hover();
  await expect(page.getByRole("tooltip")).toContainText(
    "Ups WATER moves in a pinch.",
  );
  await page.getByRole("tooltip").hover();
  await expect(page.getByRole("tooltip")).toBeVisible();
  await page.screenshot({
    path: "../../artifacts/hud/ability-tooltip.png",
  });
  await expect(page.getByRole("tooltip")).toBeVisible();
  await page.keyboard.press("Escape");
  await expect(page.getByRole("tooltip")).toHaveCount(0);
  await ability.focus();
  await expect(ability).toHaveAttribute("aria-describedby", /.+/);
  await expect(page.getByRole("tooltip")).toBeVisible();
  await page.keyboard.press("Tab");
  await expect(
    team.getByRole("button", { name: "Move: Scratch" }),
  ).toBeFocused();
  await expect(page.getByRole("tooltip")).toContainText("Power 40");
  await page.screenshot({
    path: "../../artifacts/hud/move-tooltip-battle.png",
  });
  await expect(page.getByRole("tooltip")).toBeVisible();
  await page.keyboard.press("Escape");
  await team.getByRole("listitem").nth(1).getByRole("button").click();
  await expect(
    team.getByRole("region", { name: "Selected Pokémon" }),
  ).toContainText("KEEN EYE");
  await expect(
    team
      .getByRole("region", { name: "Selected Pokémon" })
      .getByText("In battle", { exact: true }),
  ).toHaveCount(0);
  await page.keyboard.press("End");
  await expect(
    team.getByRole("region", { name: "Selected Pokémon" }),
  ).toContainText("CHLOROPHYLL");
  await page.getByLabel("Search team and PC").fill("totodile");
  await expect(team.getByRole("listitem")).toHaveCount(1);
  await expect(
    team.getByRole("region", { name: "Selected Pokémon" }),
  ).toContainText("TORRENT");
  await page.getByLabel("Search team and PC").fill("");
  await importState(page, battle!);
  await expect(
    team.getByRole("region", { name: "Selected Pokémon" }),
  ).toContainText("TORRENT");
  await page.setViewportSize({ width: 390, height: 844 });
  await page.evaluate(() => window.scrollTo(0, 0));
  expect(
    await page.evaluate(
      () => document.documentElement.scrollWidth <= innerWidth,
    ),
  ).toBe(true);
  const screenBox = await page.locator(".game-display").boundingBox();
  const teamBox = await team.boundingBox();
  expect(screenBox!.y + screenBox!.height).toBeLessThan(teamBox!.y);
  await page.screenshot({
    path: "../../artifacts/hud/reboot-battle-mobile.png",
    fullPage: true,
  });
  await team.getByRole("button", { name: "Move: Water Gun" }).tap();
  await expect(page.getByRole("tooltip")).toContainText("Special");
  await expect(page.getByRole("tooltip")).toBeVisible();
  const tipBox = await page.getByRole("tooltip").boundingBox();
  expect(tipBox!.x).toBeGreaterThanOrEqual(0);
  expect(tipBox!.x + tipBox!.width).toBeLessThanOrEqual(390);
  expect(tipBox!.y).toBeGreaterThanOrEqual(0);
  expect(tipBox!.y + tipBox!.height).toBeLessThanOrEqual(844);
  await page.screenshot({
    path: "../../artifacts/hud/move-tooltip-mobile.png",
  });
  await page.touchscreen.tap(2, 2);
  await expect(page.getByRole("tooltip")).toHaveCount(0);
  await importState(page, party!);
  await page.getByRole("button", { name: "Info", exact: true }).click();
  await expect(opponent).toContainText("Not in battle");
  await expect(opponent.getByRole("button", { name: /Hoppip/ })).toHaveCount(0);
  await page.getByRole("button", { name: "Team", exact: true }).click();
  await expect(team.getByRole("listitem").first()).toContainText("29 / 33");
  await expect(page.getByLabel("Arena status")).toHaveText("Map weatherNone");
  await page.setViewportSize({ width: 1672, height: 941 });
  await page.evaluate(() => window.scrollTo(0, 0));
  await page.screenshot({
    path: "../../artifacts/hud/reboot-party-desktop.png",
    fullPage: true,
  });
  // Visual edge states use a synthetic capture overlay, never guest writes.
  // The screenshots above are unmodified observations of the supplied states.
  await importState(page, battle!);
  await page.evaluate(() => {
    const emulator = (window as unknown as { emulator: Emulator }).emulator;
    const capture = emulator.captureMemory.bind(emulator);
    emulator.captureMemory = async () => {
      const sample = await capture();
      if (!sample) return sample;
      const ram = sample.regions.find(
        (region) => region.address === 0x02000000,
      )!.bytes;
      const view = new DataView(ram.buffer, ram.byteOffset, ram.byteLength);
      view.setUint16(0x143c + 0x2e, 1, true); // Master Ball, verified catalog text.
      view.setUint16(0x143c + 0x0c, 98, true); // Quick Attack: priority +1.
      ram[0x143c + 0x19] = 8; // Attack +2.
      ram[0x143c + 0x1a] = 5; // Defense -1.
      view.setUint32(0x143c + 0x4c, 8, true); // Poison, player live status.
      view.setUint16(0x143c + 0x58 + 0x28, 0, true); // Fainted opponent.
      return sample;
    };
  });
  await runOneFrame(page);
  const inspector = team.getByRole("region", { name: "Selected Pokémon" });
  await expect(
    inspector.getByRole("button", { name: "Stat: Attack, total 36, stage +2" }),
  ).toHaveText("Atk36↑");
  await expect(
    inspector.getByRole("button", {
      name: "Stat: Defense, total 13, stage -1",
    }),
  ).toHaveText("Def13↓");
  await expect(inspector.getByRole("table", { name: "Moves" })).toContainText(
    "+1",
  );
  await expect(inspector.locator(".condition")).toHaveText("PSN");
  await expect(opponent.locator(".selected-tags .condition")).toHaveText("FNT");
  await expect(page.locator(".roster-card .condition")).toHaveCount(0);
  await inspector.getByRole("button", { name: "Move: Quick Attack" }).hover();
  const tooltip = page.getByRole("tooltip");
  await expect(tooltip).toContainText("Quick Attack");
  await expect(tooltip).toContainText("NORMAL");
  await expect(tooltip).toContainText("Power 40");
  await expect(tooltip).toContainText("Physical");
  await expect(tooltip).toContainText("An extremely fast attack");
  await expect(tooltip).toContainText("Priority +1");
  await page.screenshot({
    path: "../../artifacts/hud/move-tooltip-desktop-synthetic.png",
  });
  await expect(tooltip).toBeVisible();
  await page.keyboard.press("Escape");
  const item = inspector.getByRole("button", { name: "Item: MASTER BALL" });
  await expect
    .poll(() =>
      item
        .locator("img")
        .evaluate((image: HTMLImageElement) => [
          image.naturalWidth,
          image.naturalHeight,
        ]),
    )
    .toEqual([24, 24]);
  await item.hover();
  await expect(tooltip).toContainText("without fail.");
  await expect(tooltip.locator(".item-sprite")).toBeVisible();
  await page.screenshot({
    path: "../../artifacts/hud/reboot-item-stages-synthetic.png",
  });
  await expect(tooltip).toBeVisible();
  expect(errors).toEqual([]);
});

test("HnS 1.2.1 game2: real names, artwork, moves, selection and timeline replacement", async ({
  page,
}) => {
  const rom = process.env.ZIGBA_HNS_ROM;
  const state = process.env.ZIGBA_HNS_STATE;
  test.skip(
    !rom || !state,
    "Set ZIGBA_HNS_ROM and ZIGBA_HNS_STATE to the documented local fixtures",
  );
  expect(sha(await readFile(rom!))).toBe(
    "7c4f90c8b68b64d4639a37306d5302a11cd4d7c44005114ec2faa2b28b210c3d",
  );
  expect(sha(await readFile(state!))).toBe(
    "71817992f9053560808493c79c3e65b5acba2b4499375cc52d76806b2107fb5f",
  );
  const errors: string[] = [];
  page.on("pageerror", (error) => errors.push(error.message));
  await page.setViewportSize({ width: 1672, height: 941 });
  await ready(page);
  await page.getByLabel("Open ROM", { exact: true }).setInputFiles(rom!);
  await expect(page.locator('[data-game-layout="pokemon"]')).toBeVisible();
  await importState(page, state!);
  const team = page.getByRole("complementary", {
    name: "Your Pokémon",
    exact: true,
  });
  const cards = team.getByRole("list", { name: "Current team" });
  await expect(cards.getByRole("listitem")).toHaveCount(3);
  for (const [slot, name, hp] of [
    [0, "Totodile", "29 / 33"],
    [1, "Pidgey", "16 / 16"],
    [2, "Hoppip", "14 / 14"],
  ] as const) {
    await expect(cards.getByRole("listitem").nth(slot)).toContainText(name);
    await expect(cards.getByRole("listitem").nth(slot)).toContainText(hp);
  }
  const details = team.getByRole("region", { name: "Selected Pokémon" });
  await expect(details.locator(".selected-name h3")).toHaveAttribute(
    "title",
    /^MODEST nature/,
  );
  await expect(details).toContainText("TORRENT");
  await expect(
    details.getByRole("button", { name: "Move: Scratch" }),
  ).toBeVisible();
  const waterGun = details
    .getByRole("row")
    .filter({ has: page.getByRole("button", { name: "Move: Water Gun" }) });
  await expect(waterGun.locator(".move-category")).toHaveAttribute(
    "aria-label",
    "special",
  );
  await expect(cards.locator("img")).toHaveCount(3);
  await expect
    .poll(() =>
      cards
        .locator("img")
        .evaluateAll((images) =>
          images.every(
            (image) => (image as HTMLImageElement).naturalWidth === 64,
          ),
        ),
    )
    .toBe(true);
  await expect
    .poll(() =>
      details
        .locator(".selected-summary img")
        .evaluate((image) => (image as HTMLImageElement).naturalWidth),
    )
    .toBe(64);
  // IVs and EVs are in each stat's details; the starter has a perfect value.
  const training: string[] = [];
  for (const stat of await details
    .getByRole("group", { name: "Stats" })
    .getByRole("button")
    .all()) {
    await stat.focus();
    await expect(page.getByRole("tooltip")).toBeVisible();
    training.push((await page.getByRole("tooltip").textContent()) ?? "");
    await page.keyboard.press("Escape");
    await expect(page.getByRole("tooltip")).toHaveCount(0);
  }
  expect(training).toHaveLength(6);
  expect(training.join("\n")).toMatch(/(IV|EV)31(?!\d)/);
  await page.evaluate(() => document.fonts.ready);
  await page.screenshot({
    path: "../../artifacts/hud/hns-game2-desktop.png",
    fullPage: true,
  });
  await cards.getByRole("listitem").nth(1).getByRole("button").click();
  await expect(details).toContainText("KEEN EYE");
  await expect(
    details.getByRole("button", { name: "Move: Tackle" }),
  ).toBeVisible();
  await page.keyboard.press("End");
  await expect(details).toContainText("CHLOROPHYLL");
  await expect(
    details.getByRole("button", { name: "Move: Splash" }),
  ).toBeVisible();
  // Reload the same state: same Pokémon identity, different timeline => reset focus.
  await importState(page, state!);
  await expect(details).toContainText("TORRENT");
  await runOneFrame(page);
  await expect(cards.getByRole("listitem")).toHaveCount(3);
  await page.setViewportSize({ width: 390, height: 844 });
  expect(
    await page.evaluate(
      () => document.documentElement.scrollWidth <= innerWidth,
    ),
  ).toBe(true);
  await page.evaluate(() => window.scrollTo(0, 0));
  await page.screenshot({
    path: "../../artifacts/hud/hns-game2-mobile.png",
    fullPage: true,
  });
  expect(errors).toEqual([]);
});

test("Elite Redux Pokédex links follow selection and open a separate tab", async ({
  page,
  context,
}) => {
  const rom = process.env.ZIGBA_REDUX_ROM;
  const state = process.env.ZIGBA_REDUX_STATE;
  test.skip(!rom || !state, "Supply the exact Redux Beta2 ROM and redux.state");
  await context.route("https://forwardfeed.github.io/ER-nextdex/**", (route) =>
    route.fulfill({
      contentType: "text/html",
      body: "<title>Pokédex destination</title>",
    }),
  );
  await page.setViewportSize({ width: 1672, height: 941 });
  await ready(page);
  await page.getByLabel("Open ROM", { exact: true }).setInputFiles(rom!);
  await importState(page, state!);
  await page.getByRole("button", { name: "Dismiss notification" }).click();
  const team = page.getByRole("complementary", {
    name: "Your Pokémon",
    exact: true,
  });
  const inspector = team.getByRole("region", { name: "Selected Pokémon" });
  const link = inspector.getByRole("link");
  await expect(link).toHaveAccessibleName("Open Frigibax in Pokédex (new tab)");
  await expect(link).toHaveAttribute(
    "href",
    "https://forwardfeed.github.io/ER-nextdex/?v=2.65beta&p=996",
  );
  await expect(
    page
      .getByRole("complementary", { name: "Opponent", exact: true })
      .getByRole("link"),
  ).toHaveAttribute(
    "href",
    "https://forwardfeed.github.io/ER-nextdex/?v=2.65beta&p=258",
  );
  await page.evaluate(() => document.fonts.ready);
  await inspector.screenshot({
    path: "../../artifacts/hud/pokedex-link-desktop.png",
  });
  await link.focus();
  await expect(link).toBeFocused();
  const popupPromise = page.waitForEvent("popup");
  await link.press("Enter");
  const popup = await popupPromise;
  await expect(popup).toHaveURL(
    "https://forwardfeed.github.io/ER-nextdex/?v=2.65beta&p=996",
  );
  expect(await popup.evaluate(() => window.opener)).toBeNull();
  await popup.close();
  await team.getByRole("listitem").nth(1).getByRole("button").click();
  await expect(link).toHaveAccessibleName("Open Klink in Pokédex (new tab)");
  await expect(link).toHaveAttribute(
    "href",
    "https://forwardfeed.github.io/ER-nextdex/?v=2.65beta&p=599",
  );
  await page.setViewportSize({ width: 390, height: 844 });
  await link.scrollIntoViewIfNeeded();
  await expect(link).toBeInViewport();
  const linkBox = await link.boundingBox();
  const nameBox = await inspector
    .getByRole("heading", { name: "Klink", exact: true })
    .boundingBox();
  expect(linkBox!.x).toBeGreaterThan(nameBox!.x + nameBox!.width);
  expect(linkBox!.x + linkBox!.width).toBeLessThan(390);
  await inspector.screenshot({
    path: "../../artifacts/hud/pokedex-link-mobile.png",
  });
});

test("Elite Redux Beta3 opens as a reviewed build with its own ability records", async ({
  page,
}) => {
  const rom = process.env.ZIGBA_REDUX_BETA3_ROM,
    state = process.env.ZIGBA_REDUX_BETA3_STATE;
  test.skip(!rom || !state, "Supply the exact Redux Beta3 ROM and its state");
  expect(romSha(await readFile(rom!))).toBe(
    "5b38a681b6924ef3e0bb9f6cfbf22426c4c4e523",
  );
  expect(sha(await readFile(state!))).toBe(
    "74a207b19b09a228cef36f5122d7f229945d8693192eaf1b75796a347445c359",
  );
  const errors: string[] = [];
  page.on("pageerror", (error) => errors.push(error.message));
  await page.setViewportSize({ width: 1672, height: 941 });
  await ready(page);
  await page.getByLabel("Open ROM", { exact: true }).setInputFiles(rom!);
  await expect(page.locator('[data-game-layout="pokemon"]')).toBeVisible();
  await expect(page.getByText("Detected build · not reviewed")).toHaveCount(0);
  await importState(page, state!);
  const team = page.getByRole("complementary", {
    name: "Your Pokémon",
    exact: true,
  });
  await expect(team.getByRole("listitem")).toHaveCount(6);
  await expect(team.getByRole("listitem").first()).toContainText("Zoroark");
  await expect(team.getByRole("listitem").first()).toContainText("110 / 110");
  await expect(team.getByRole("listitem").last()).toContainText("Pawmot");
  const inspector = team.getByRole("region", { name: "Selected Pokémon" });
  // Names from 196-byte records; Beta2's 192-byte stride reads other abilities.
  await expect(
    inspector.getByRole("button", { name: "Innate: Illusion" }),
  ).toBeVisible();
  await expect(page.getByLabel("Arena status")).toContainText("Ace");
  await page.evaluate(() => document.fonts.ready);
  await page.mouse.move(0, 0);
  await page.screenshot({
    path: "../../artifacts/hud/redux-beta3-desktop.png",
    fullPage: true,
  });
  expect(errors).toEqual([]);
});

test("Elite Redux uses the shared layout with four described abilities and real battle data", async ({
  page,
}) => {
  const rom = process.env.ZIGBA_REDUX_ROM,
    state = process.env.ZIGBA_REDUX_STATE;
  test.skip(!rom || !state, "Supply the exact Redux Beta2 ROM and redux.state");
  expect(romSha(await readFile(rom!))).toBe(
    "f1323126bc043f3ce931bcc670957930559eb515",
  );
  expect(sha(await readFile(state!))).toBe(
    "bf403592558c7a6cdc0deb493f8ab26f8796dfec829f0a2031fcbbc44af66d23",
  );
  // Category artwork must work without any third-party network access.
  await page.route("https://**", (route) => route.abort());
  const errors: string[] = [];
  page.on("pageerror", (error) => errors.push(error.message));
  await page.setViewportSize({ width: 1672, height: 941 });
  await ready(page);
  await page.getByLabel("Open ROM", { exact: true }).setInputFiles(rom!);
  await expect(page.locator('[data-game-layout="pokemon"]')).toBeVisible();
  await importState(page, state!);
  const team = page.getByRole("complementary", {
    name: "Your Pokémon",
    exact: true,
  });
  const opponent = page.getByRole("complementary", {
    name: "Opponent",
    exact: true,
  });
  await expect(team.getByRole("listitem")).toHaveCount(4);
  await expect(opponent.getByRole("listitem")).toHaveCount(2);
  await expect(team.getByRole("listitem").first()).toContainText("Frigibax");
  await expect(team.getByRole("listitem").first()).toContainText("62 / 62");
  await expect(opponent.getByRole("listitem").first()).toContainText("Mudkip");
  await expect(opponent.getByRole("listitem").first()).toContainText("58 / 58");
  const inspector = team.getByRole("region", { name: "Selected Pokémon" });
  await expect(inspector.locator(".ability-list button")).toHaveCount(4);
  await expect(page.locator(".scene-heading")).toContainText("Elite Redux");
  await expect(page.locator(".scene-heading")).not.toContainText("Turn");
  await expect
    .soft(page.getByLabel("Arena status"))
    .toContainText("BattleTrainer");
  await expect
    .poll(() =>
      page
        .locator(".roster-card img,.selected-summary img")
        .evaluateAll(
          (images) =>
            images.length === 8 &&
            images.every(
              (img) => (img as HTMLImageElement).naturalWidth === 64,
            ),
        ),
    )
    .toBe(true);
  await page.getByRole("button", { name: "Dismiss notification" }).click();
  await page.evaluate(() => document.fonts.ready);
  await page.mouse.move(0, 0);
  await screenFillsColumn(page);
  await page.screenshot({
    path: "../../artifacts/hud/redux-battle-desktop.png",
    fullPage: true,
  });
  const ability = inspector.getByRole("button", {
    name: "Innate: Thermal Exchange",
  });
  await ability.focus();
  await expect(page.getByRole("tooltip")).toContainText(
    "Boosts Attack by one stage",
  );
  await page.screenshot({
    path: "../../artifacts/hud/redux-ability-tooltip.png",
  });
  await page.keyboard.press("Escape");
  await team.getByRole("listitem").nth(1).getByRole("button").click();
  await expect(inspector).toContainText("Klink");
  for (const index of [0, 3]) {
    const trigger = inspector.locator(".ability-list button").nth(index);
    await trigger.hover();
    const borders = await trigger.evaluate((element) => {
      const style = getComputedStyle(element);
      return [
        style.borderTopColor,
        style.borderRightColor,
        style.borderBottomColor,
        style.borderLeftColor,
      ];
    });
    expect.soft(borders).toEqual(Array(4).fill("rgb(89, 183, 232)"));
    await trigger.focus();
    await page.keyboard.press("Escape");
  }
  await expect
    .soft(inspector.getByText(/^electr$/i).first())
    .toHaveCSS("background-color", "rgb(244, 214, 40)");
  for (const category of ["physical", "special", "status"]) {
    await inspector
      .locator(`tr:has(.move-category.${category}) .info-trigger`)
      .first()
      .focus();
    const tip = page.getByRole("tooltip");
    // Inline SVG glyphs: no network request and a fixed 3:2 icon grid.
    const glyph = tip.locator(`.move-category.${category} svg`);
    await expect(glyph).toHaveAttribute("viewBox", "0 0 30 20");
    const size = await glyph.boundingBox();
    expect(size).not.toBeNull();
    expect(size!.width / size!.height).toBeCloseTo(1.5, 1);
    const contained = await tip
      .locator(`.move-category.${category} i`)
      .evaluate((icon) => {
        const box = icon.getBoundingClientRect();
        const glyph = icon.querySelector("svg")!.getBoundingClientRect();
        return (
          glyph.top >= box.top &&
          glyph.bottom <= box.bottom &&
          glyph.left >= box.left &&
          glyph.right <= box.right
        );
      });
    expect
      .soft(contained, `${category} glyph stays inside its icon slot`)
      .toBe(true);
    await page.screenshot({
      path: `../../artifacts/hud/redux-move-${category}.png`,
    });
    await page.keyboard.press("Escape");
  }
  await team.getByRole("listitem").first().getByRole("button").click();
  await page.setViewportSize({ width: 390, height: 844 });
  await page.evaluate(() => window.scrollTo(0, 0));
  await screenFillsColumn(page);
  expect(
    await page.evaluate(
      () => document.documentElement.scrollWidth <= innerWidth,
    ),
  ).toBe(true);
  await page.screenshot({
    path: "../../artifacts/hud/redux-battle-mobile.png",
    fullPage: true,
  });
  await ability.tap();
  await expect(page.getByRole("tooltip")).toContainText("Fire");
  await expect(page.getByRole("tooltip")).toBeVisible();
  const bounds = await page.getByRole("tooltip").boundingBox();
  expect(bounds!.x).toBeGreaterThanOrEqual(0);
  expect(bounds!.x + bounds!.width).toBeLessThanOrEqual(390);
  await page.keyboard.press("Escape");
  // Synthetic observation overlay: exercise non-default badges without guest writes.
  await page.evaluate(() => {
    const emulator = (window as unknown as { emulator: Emulator }).emulator;
    const capture = emulator.captureMemory.bind(emulator);
    emulator.captureMemory = async () => {
      const sample = await capture();
      if (!sample) return sample;
      const ram = sample.regions.find(
        (region) => region.address === 0x02000000,
      )!.bytes;
      const view = new DataView(ram.buffer, ram.byteOffset, ram.byteLength);
      ram[0x29c48 + 0xc89] = (ram[0x29c48 + 0xc89] & 15) | 0x20; // Elite: last innate locked at Lv20.
      view.setUint16(0x1c50c + 0x2e, 0, true);
      view.setUint32(0x1c7ac, 1 << 16, true); // Gastro Acid suppresses other abilities.
      view.setUint32(0x1c50c + 8, 0x1000, true); // Frostbite, in the copied observation only.
      view.setUint16(0x1c50c + 0x3a, 1, true); // Master Ball item artwork.
      return sample;
    };
  });
  await runOneFrame(page);
  await expect(inspector.locator(".ability-list .suppressed")).toHaveCount(3);
  await expect(inspector.locator(".ability-list .locked")).toHaveCount(1);
  await expect(inspector.locator(".condition")).toHaveText("Frostbite");
  await expect(inspector.locator(".condition")).toHaveCSS(
    "background-color",
    "rgb(152, 217, 218)",
  );
  await expect(inspector.locator(".condition")).toHaveCSS(
    "color",
    "rgb(18, 33, 39)",
  );
  await expect(page.locator(".roster-card .condition")).toHaveCount(0);
  const item = inspector.getByRole("button", { name: "Item: Master Ball" });
  await expect
    .poll(() =>
      item
        .locator("img")
        .evaluate((image: HTMLImageElement) => [
          image.naturalWidth,
          image.naturalHeight,
        ]),
    )
    .toEqual([24, 24]);
  expect(
    await inspector.locator(".ability-list button").evaluateAll((buttons) =>
      buttons.every((button) => {
        const name = button
          .querySelector(".ability-name")!
          .getBoundingClientRect();
        const state = button.querySelector("small")!.getBoundingClientRect();
        return (
          state.top >= name.bottom &&
          state.right <= button.getBoundingClientRect().right
        );
      }),
    ),
  ).toBe(true);
  await inspector.screenshot({
    path: "../../artifacts/hud/redux-ability-states-mobile-synthetic.png",
  });
  await item.tap();
  await expect(page.getByRole("tooltip").locator(".item-sprite")).toBeVisible();
  await page.screenshot({
    path: "../../artifacts/hud/redux-item-mobile-synthetic.png",
  });
  expect(errors).toEqual([]);
});

test("Redux battle totals include stages and reset with the loaded state", async ({
  page,
}) => {
  const rom = process.env.ZIGBA_REDUX_ROM;
  const baseline = process.env.ZIGBA_REDUX_STATE;
  const buffed = process.env.ZIGBA_REDUX_STAGE_STATE;
  test.skip(
    !rom || !baseline || !buffed,
    "Supply Redux ROM, original and stage-buffed states",
  );
  expect(romSha(await readFile(rom!))).toBe(
    "f1323126bc043f3ce931bcc670957930559eb515",
  );
  expect(sha(await readFile(baseline!))).toBe(
    "bf403592558c7a6cdc0deb493f8ab26f8796dfec829f0a2031fcbbc44af66d23",
  );
  expect(sha(await readFile(buffed!))).toBe(
    "d092f7bd0f323b1720321f16b72291753748c03fba63454fe8d19455ec7b472d",
  );
  await page.setViewportSize({ width: 1672, height: 941 });
  await ready(page);
  await page.getByLabel("Open ROM", { exact: true }).setInputFiles(rom!);
  await expect(page.locator('[data-game-layout="pokemon"]')).toBeVisible();
  const inspector = page
    .getByRole("complementary", { name: "Opponent", exact: true })
    .getByRole("region", { name: "Selected Pokémon" });
  const stat = (name: string) =>
    inspector.getByRole("button", { name: new RegExp(`^Stat: ${name},`) });
  const total = (name: string) => stat(name).locator(".stat-value");
  const expectCompactStats = async () => {
    const grid = inspector.getByRole("group", { name: "Stats", exact: true });
    expect((await grid.boundingBox())!.height).toBeLessThanOrEqual(60);
    for (const button of await grid.getByRole("button").all()) {
      expect((await button.boundingBox())!.height).toBeLessThanOrEqual(28);
      const label = (await button.locator(".stat-label").boundingBox())!;
      const value = (await button.locator(".stat-value").boundingBox())!;
      expect(label.x + label.width).toBeLessThanOrEqual(value.x);
      expect(
        Math.abs(label.y + label.height / 2 - value.y - value.height / 2),
      ).toBeLessThan(1);
    }
  };
  await importState(page, baseline!);
  await expect(total("Attack")).toHaveText("41");
  await expect(total("Defense")).toHaveText("31");
  await importState(page, buffed!);
  await page.getByRole("button", { name: "Dismiss notification" }).click();
  await expect(inspector).toContainText("Mudkip");
  await expect(total("Attack")).toHaveText("61↑");
  await expect(total("Defense")).toHaveText("46↑");
  await expect(total("HP")).toHaveText("58");
  await expect(total("Speed")).toHaveText("29");
  await page.evaluate(() => document.fonts.ready);
  await expectCompactStats();
  await inspector.screenshot({
    path: "../../artifacts/hud/redux-stage-buff.png",
  });
  await stat("Attack").hover();
  const tip = page.getByRole("tooltip");
  const detail = (label: string) =>
    tip
      .locator("dt")
      .filter({ hasText: new RegExp(`^${label}$`) })
      .locator("+ dd");
  // Independent release facts: species 258's base Attack byte is 75; Redux IVs are 31;
  // this trainer record has zero EVs. The supplied state has raw Attack 41 at +1.
  await expect(detail("Base")).toHaveText("75");
  await expect(detail("IV")).toHaveText("31");
  await expect(detail("EV")).toHaveText("0");
  await expect(detail("Before stages")).toHaveText("41");
  await expect(detail("Battle stage")).toHaveText("+1");
  await expect(detail("Total")).toHaveText("61");
  await expect(tip).toContainText("other battle modifiers excluded");
  await page.screenshot({
    path: "../../artifacts/hud/redux-stat-tooltip-desktop.png",
  });
  await stat("Attack").focus();
  await page.keyboard.press("Escape");
  await expect(tip).toHaveCount(0);
  await page.keyboard.press("Tab");
  await expect(stat("Defense")).toBeFocused();
  await expect(detail("Total")).toHaveText("46");
  await page.keyboard.press("Escape");
  await page.setViewportSize({ width: 390, height: 844 });
  await page.getByRole("button", { name: "Info", exact: true }).click();
  await expectCompactStats();
  await inspector.screenshot({
    path: "../../artifacts/hud/redux-stage-buff-mobile.png",
  });
  const cells = await inspector
    .getByRole("group", { name: "Stats", exact: true })
    .getByRole("button")
    .evaluateAll((buttons) =>
      buttons.map((button) => {
        const rect = button.getBoundingClientRect();
        return { x: rect.x, y: rect.y, width: rect.width };
      }),
    );
  expect(cells).toHaveLength(6);
  expect(new Set(cells.slice(0, 3).map((cell) => cell.y)).size).toBe(1);
  expect(new Set(cells.slice(3).map((cell) => cell.y)).size).toBe(1);
  expect(cells[3].y).toBeGreaterThan(cells[0].y);
  expect(new Set(cells.map((cell) => cell.x)).size).toBe(3);
  await stat("Attack").tap();
  await expect(detail("Base")).toHaveText("75");
  const bounds = await tip.boundingBox();
  expect(bounds!.x).toBeGreaterThanOrEqual(0);
  expect(bounds!.x + bounds!.width).toBeLessThanOrEqual(390);
  expect(bounds!.y).toBeGreaterThanOrEqual(0);
  expect(bounds!.y + bounds!.height).toBeLessThanOrEqual(844);
  await page.screenshot({
    path: "../../artifacts/hud/redux-stat-tooltip-mobile.png",
  });
  await page.touchscreen.tap(2, 2);
  await expect(tip).toHaveCount(0);
  expect(
    await page.evaluate(
      () => document.documentElement.scrollWidth <= innerWidth,
    ),
  ).toBe(true);
  // Large totals at the narrowest three-column desktop size: just above the
  // tablet breakpoint and wider than 17:10, where the side columns are at
  // their 300 px minimum. Observation only.
  await importState(page, baseline!);
  await expect(total("Attack")).toHaveText("41");
  await expect(total("Defense")).toHaveText("31");
  await importState(page, buffed!);
  await expect(total("Attack")).toHaveText("61↑");
  await page.setViewportSize({ width: 1001, height: 580 });
  await page.evaluate(() => {
    const emulator = (window as unknown as { emulator: Emulator }).emulator;
    const capture = emulator.captureMemory.bind(emulator);
    emulator.captureMemory = async () => {
      const sample = await capture();
      if (!sample) return sample;
      const ram = sample.regions.find(
        (region) => region.address === 0x02000000,
      )!.bytes;
      const view = new DataView(ram.buffer, ram.byteOffset, ram.byteLength);
      const enemy = 0x1c50c + 0x68;
      view.setUint16(enemy + 0x16, 511, true);
      view.setUint16(enemy + 0x18, 809, true);
      view.setUint16(enemy + 0x38, 250, true);
      ram[enemy + 0x3d] = 12; // Attack +6, total 2044.
      return sample;
    };
  });
  await runOneFrame(page);
  await expect(total("Attack")).toHaveText("2044↑");
  await expect(total("Defense")).toHaveText("1213↑");
  await expectCompactStats();
  const contained = await inspector
    .locator(".pokemon-stat-grid button")
    .evaluateAll((buttons) =>
      buttons.every((button) => {
        const box = button.getBoundingClientRect();
        const label = button
          .querySelector(".stat-label")!
          .getBoundingClientRect();
        const value = button
          .querySelector(".stat-value")!
          .getBoundingClientRect();
        return (
          (label.right <= value.left || label.bottom <= value.top) &&
          Array.from(
            button.querySelectorAll(".stat-label,.stat-value > *"),
          ).every((child) => {
            const rect = child.getBoundingClientRect();
            return (
              rect.left >= box.left &&
              rect.right <= box.right &&
              rect.top >= box.top &&
              rect.bottom <= box.bottom
            );
          })
        );
      }),
    );
  expect(contained).toBe(true);
  await inspector.screenshot({
    path: "../../artifacts/hud/redux-stats-large-totals-synthetic.png",
  });
});
