import { expect, test } from "@playwright/test";
import { build } from "vite-plus";
import { dashboardCss, fixtureBuild, playerCss } from "./support";
import { resolve } from "node:path";
import { rm, writeFile } from "node:fs/promises";

const outDir = "../../zig-out/web/selection-test";
const fixturePath = resolve("tests/browser/.selection.fixture.tsx");

declare global {
  interface Window {
    selectionFixture: { reverse(): void; reset(): void; duplicates(): void };
  }
}

test.beforeAll(async () => {
  await writeFile(
    fixturePath,
    `
    import React, { useState } from "react";
    import { createRoot } from "react-dom/client";
    import { PokemonDashboard } from ${JSON.stringify(resolve("ui/pokemon-dashboard.tsx"))};
    import ${JSON.stringify(playerCss)};
    import ${JSON.stringify(dashboardCss)};
    const stats = { hp: 12, attack: 23, defense: 34, speed: 45, specialAttack: 56, specialDefense: 67 };
    const member = (name, slot, id = name) => ({
      id, species: 1, speciesName: name, nickname: name, language: 2, level: 12 + slot,
      isEgg: false, gender: null, shiny: null, nature: null, types: null, abilities: [], heldItem: null,
      stats: { actual: stats, base: null, ivs: stats, evs: stats }, moves: [], artwork: null,
      warnings: [], slot, hp: 12, maxHp: 24, status: 0, ailments: [],
    });
    const initial = [member("Alpha", 0), member("Beta", 1), member("Gamma", 2), member("Delta", 3), member("Epsilon", 4)];
    function Fixture() {
      const [party, setParty] = useState(initial);
      const [timeline, setTimeline] = useState(1);
      window.selectionFixture = {
        reverse: () => setParty((p) => [...p].reverse()),
        reset: () => setTimeline((t) => t + 1),
        duplicates: () => setParty([member("Twin one", 0, "twin"), member("Twin two", 1, "twin")]),
      };
      const sample = { timeline, sequence: 1, cycle: 1n, romHash: "fixture" };
      return <div className="console-app"><PokemonDashboard gameName="Selection fixture"
          party={{ status: "ready", sample, data: { party } }}
          storage={{ status: "ready", data: { boxes: [{ pokemon: [member("PC Pikachu", 0), { ...member("Pikachu", 0, "poke-pal"), nickname: "Poké Pal" }] }] } }}
          battle={{ status: "ready", data: { active: true, kind: "wild", playerParty: party,
            opponentParty: [member("Enemy", 0)], battlers: [{ side: "player", partySlot: 1 }] } }}
          context={{ status: "idle" }} resources={undefined}
        ><div className="game-display">Synthetic game screen</div></PokemonDashboard></div>;
    }
    createRoot(document.getElementById("root")).render(<Fixture />);
  `,
  );
  await build({
    ...fixtureBuild,
    build: {
      outDir,
      emptyOutDir: true,
      lib: {
        entry: fixturePath,
        formats: ["es"],
        fileName: () => "fixture.js",
        cssFileName: "fixture",
      },
    },
  });
  await writeFile(
    resolve(outDir, "index.html"),
    '<!doctype html><html><head><meta name="viewport" content="width=device-width, initial-scale=1"><link rel="stylesheet" href="fixture.css"></head><body><div id="root"></div><script type="module" src="fixture.js"></script></body></html>',
  );
});

test.afterAll(() =>
  Promise.all([
    rm(outDir, { recursive: true, force: true }),
    rm(fixturePath, { force: true }),
  ]),
);

test("roster keeps spatial wrapping, identity, filtering and responsive panels", async ({
  page,
}) => {
  page.on("pageerror", (error) => {
    throw error;
  });
  await page.setViewportSize({ width: 1440, height: 1000 });
  await page.goto("/selection-test/index.html");
  const cards = page.locator(".player-roster .roster-card");
  await expect(cards.nth(1)).toHaveAttribute("aria-pressed", "true");
  await cards.nth(1).focus();
  for (const [key, index] of [
    ["ArrowDown", 3],
    ["ArrowDown", 0],
    ["ArrowUp", 3],
    ["End", 4],
    ["ArrowRight", 0],
    ["ArrowLeft", 4],
    ["Home", 0],
  ] as const) {
    await page.keyboard.press(key);
    await expect(cards.nth(index)).toBeFocused();
    await expect(cards.nth(index)).toHaveAttribute("aria-pressed", "true");
  }
  await cards.nth(2).click();
  await page.evaluate(() => window.selectionFixture.reverse());
  await expect(cards.first()).toContainText("Epsilon");
  await expect(
    page.locator('.player-roster .roster-card[aria-pressed="true"]'),
  ).toContainText("Gamma");
  await expect(page.locator(".dashboard-team")).toBeVisible();
  await expect(page.locator(".dashboard-info")).toBeVisible();
  await page.screenshot({
    path: `../../artifacts/selection-${process.env.SELECTION_STAGE ?? "after"}-desktop.png`,
  });
  const search = page.getByRole("searchbox", { name: "Search team and PC" });
  await search.fill("PC");
  await expect(cards).toHaveCount(1);
  await expect(cards.first()).toContainText("Pc Pikachu");
  await cards.first().press("ArrowDown");
  await expect(cards.first()).toBeFocused();
  await page.screenshot({
    path: `../../artifacts/selection-${process.env.SELECTION_STAGE ?? "after"}-filtered.png`,
  });
  await search.fill("missing");
  await expect(cards).toHaveCount(0);
  await expect(page.getByText("No matching Pokémon")).toBeVisible();
  // A nickname matches without its accents.
  await search.fill("poke pal");
  await expect(cards).toHaveCount(1);
  await expect(cards.first()).toContainText("Poké Pal");
  await search.fill("");
  await expect(
    page.locator('.player-roster .roster-card[aria-pressed="true"]'),
  ).toContainText("Beta");
  await cards.filter({ hasText: "Gamma" }).click();
  await page.evaluate(() => window.selectionFixture.reset());
  await expect(
    page.locator('.player-roster .roster-card[aria-pressed="true"]'),
  ).toContainText("Beta");
  await page.evaluate(() => window.selectionFixture.duplicates());
  await cards.nth(1).click();
  await expect(cards.nth(0)).toHaveAttribute("aria-pressed", "false");
  await expect(cards.nth(1)).toHaveAttribute("aria-pressed", "true");
  await page.setViewportSize({ width: 390, height: 844 });
  const info = page.getByRole("button", { name: "Info", exact: true });
  await info.focus();
  await info.press("Enter");
  await expect(info).toBeFocused();
  await expect(page.locator(".dashboard-team")).toBeHidden();
  await expect(page.locator(".dashboard-info")).toBeVisible();
  await page.keyboard.press("Tab");
  await expect(page.locator(".opponent-roster .roster-card")).toBeFocused();
  await page.screenshot({
    path: `../../artifacts/selection-${process.env.SELECTION_STAGE ?? "after"}-mobile.png`,
  });
  await page.setViewportSize({ width: 1440, height: 1000 });
  await expect(page.locator(".dashboard-team")).toBeVisible();
  await expect(page.locator(".dashboard-info")).toBeVisible();
});
