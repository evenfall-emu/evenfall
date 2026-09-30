import { expect, test, type Page } from "@playwright/test";
import { build } from "vite-plus";
import { dashboardCss, fixtureBuild, playerCss } from "./support";
import { resolve } from "node:path";
import { rm, writeFile } from "node:fs/promises";

const outDir = "../player/dist/trainer-search-test";
const fixturePath = resolve("tests/browser/.trainer-search.fixture.tsx");

test.beforeAll(async () => {
  const dashboard = resolve("ui/pokemon-dashboard.tsx");
  const fixture = String.raw`
    import React, { useCallback, useEffect, useState } from "react";
    import { createRoot } from "react-dom/client";
    import { PokemonDashboard } from ${JSON.stringify(dashboard)};
    import ${JSON.stringify(playerCss)};
    import ${JSON.stringify(dashboardCss)};

    const zeroStats = { hp: 0, attack: 0, defense: 0, speed: 0, specialAttack: 0, specialDefense: 0 };
    const sample = (timeline) => ({ romHash: "fixture", timeline, sequence: 1, cycle: 1n });
    const member = (name, level, id) => ({
      id, species: 1, speciesName: name, nickname: name, language: 2, level,
      isEgg: false, gender: null, shiny: null, nature: null, types: null,
      abilities: [], heldItem: null,
      stats: { actual: { ...zeroStats }, base: null, ivs: { ...zeroStats }, evs: { ...zeroStats } },
      moves: [], artwork: null, warnings: [], slot: 0, hp: 1, maxHp: 1, status: 0, ailments: [],
    });
    const trainers = (version) => [
      {
        id: "alex-day", internalId: 10, name: "Alex", variant: "Day route",
        className: "Ace Trainer", portrait: { kind: "trainer", romHash: "fixture", trainerPic: 1 },
        team: [member("Daymon", version === 1 ? 12 : 21, "daymon")],
        averageLevel: version === 1 ? 12 : 21, warnings: [],
      },
      {
        id: "alex-night", internalId: 11, name: "Alex", variant: "Night rematch with a subtitle that wraps comfortably",
        className: "Ace Trainer", portrait: { kind: "trainer", romHash: "fixture", trainerPic: 2 },
        team: [member("Nightmon", version === 1 ? 13 : 22, "nightmon")],
        averageLevel: version === 1 ? 13 : 22, warnings: ["Synthetic preview"],
      },
    ];
    const resources = {
      catalog: {},
      linksFactory: { pokemon: (pokemon) => "https://example.test/dex?species=" + pokemon.species },
      artwork: () => {
        const pixels = new Uint8ClampedArray(16 * 16 * 4);
        for (let i = 0; i < pixels.length; i += 4) {
          pixels[i] = 70; pixels[i + 1] = 190; pixels[i + 2] = 210; pixels[i + 3] = 255;
        }
        return { width: 16, height: 16, pixels };
      },
    };
    function Fixture() {
      const [contextVersion, setContextVersion] = useState(1);
      const [timeline, setTimeline] = useState("one");
      const [mode, setMode] = useState("preview");
      const [, rerender] = useState(0);
      const search = useCallback((query) => {
        window.trainerSearchCalls.push({ query, contextVersion });
        if (query === "blocked") return { status: "unavailable", reason: "Trainer index unavailable" };
        const normalized = query.toLowerCase();
        return {
          status: "ready",
          data: trainers(contextVersion).filter((trainer) =>
            [trainer.name, trainer.variant, trainer.className].join(" ").toLowerCase().includes(normalized),
          ),
        };
      }, [contextVersion]);
      useEffect(() => {
        window.trainerFixture = {
          setContextVersion,
          setTimeline,
          setMode,
          rerender: () => rerender((value) => value + 1),
        };
      }, []);
      const liveOpponent = member("Live Enemy", 30, "live-enemy");
      const battle = mode === "unavailable"
        ? { status: "unavailable", sample: sample(timeline), reason: "Battle data unavailable now" }
        : mode === "live"
          ? {
              status: "ready", sample: sample(timeline), data: {
                active: true, kind: "trainer", format: "single", typeFlags: 0, types: [], turn: null,
                battleWeather: null, playerParty: [], opponentParty: [liveOpponent], battlers: [], warnings: [],
              },
            }
          : { status: "ready", sample: sample(timeline), data: { active: false } };
      return <div className="console-app"><PokemonDashboard
        gameName="Synthetic trainers"
        party={{ status: "idle" }} storage={{ status: "idle" }} context={{ status: "idle" }}
        battle={battle} resources={resources} trainerSearch={search}
      ><div className="game-display">Synthetic game screen</div></PokemonDashboard></div>;
    }
    window.trainerSearchCalls = [];
    window.mountTrainerFixture = () => {
      document.body.replaceChildren();
      const element = document.body.appendChild(document.createElement("div"));
      createRoot(element).render(<Fixture />);
    };
  `;
  await writeFile(fixturePath, fixture);
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
    '<!doctype html><html><head><meta name="viewport" content="width=device-width, initial-scale=1"><link rel="stylesheet" href="fixture.css"></head><body><script type="module" src="fixture.js"></script></body></html>',
  );
});

async function mountFixture(page: Page) {
  await page.goto("/trainer-search-test/index.html");
  await page.waitForFunction(() => "mountTrainerFixture" in window);
  await page.evaluate(() =>
    (
      window as typeof window & { mountTrainerFixture: () => void }
    ).mountTrainerFixture(),
  );
}

test.afterAll(() =>
  Promise.all([
    rm(outDir, { recursive: true, force: true }),
    rm(fixturePath, { force: true }),
  ]),
);

test("trainer search selects exact variants, refreshes context, and yields to battle state", async ({
  page,
}) => {
  page.on("pageerror", (error) => {
    throw error;
  });
  await page.setViewportSize({ width: 1440, height: 1000 });
  await mountFixture(page);

  const input = page.getByRole("combobox", { name: "Search trainers" });
  await expect(input).toBeVisible();
  await input.focus();
  await input.fill("Alex");
  const options = page.getByRole("option");
  await expect(options).toHaveCount(2);
  await expect(options.nth(0)).toContainText("Day route");
  await expect(options.nth(1)).toContainText("Night rematch");
  // Base UI keeps Home/End available for editing text, as an editable
  // combobox should; arrows navigate suggestions without moving focus.
  await input.press("End");
  await expect(options.nth(0)).toHaveAttribute("data-highlighted");
  await input.press("ArrowDown");
  await expect(options.nth(1)).toHaveAttribute("data-highlighted");
  await input.press("Enter");
  await expect(input).toBeFocused();
  await expect(input).toHaveValue("Alex");
  await expect(input).toHaveAttribute("aria-expanded", "false");

  const summary = page.locator(".selected-trainer-summary");
  await expect(summary).toContainText("Night rematch");
  await expect(
    page.getByText("Nightmon", { exact: true }).first(),
  ).toBeVisible();
  await expect(page.getByText("Lv13", { exact: true }).first()).toBeVisible();
  await expect(
    page.getByRole("link", { name: "Open Nightmon in Pokédex (new tab)" }),
  ).toHaveAttribute("href", "https://example.test/dex?species=1");
  await page.getByRole("button", { name: "Stat: Attack, total 0" }).focus();
  await expect(
    page.getByRole("tooltip").locator("dt", { hasText: "EV" }).locator("+ dd"),
  ).toHaveText("0");

  await input.fill("Nobody");
  await expect(page.getByText("No trainers match this search.")).toBeVisible();
  await expect(summary).toContainText("Night rematch");
  await page.evaluate(() =>
    (
      window as typeof window & {
        trainerFixture: { setContextVersion: (value: number) => void };
      }
    ).trainerFixture.setContextVersion(2),
  );
  await expect(page.getByText("Lv22", { exact: true }).first()).toBeVisible();

  const callsBefore = await page.evaluate(
    () =>
      (window as typeof window & { trainerSearchCalls: unknown[] })
        .trainerSearchCalls.length,
  );
  await page.evaluate(() =>
    (
      window as typeof window & { trainerFixture: { rerender: () => void } }
    ).trainerFixture.rerender(),
  );
  await expect
    .poll(() =>
      page.evaluate(
        () =>
          (window as typeof window & { trainerSearchCalls: unknown[] })
            .trainerSearchCalls.length,
      ),
    )
    .toBe(callsBefore);

  await input.fill("Alex");
  await expect(options).toHaveCount(2);
  const dropdown = page.locator(".trainer-search-dropdown");
  const box = await dropdown.boundingBox();
  expect(box).not.toBeNull();
  expect(box!.x).toBeGreaterThanOrEqual(0);
  expect(box!.x + box!.width).toBeLessThanOrEqual(1440);
  expect(box!.y + box!.height).toBeLessThanOrEqual(1000);
  await page.screenshot({ path: "../../artifacts/trainer-search-ui.png" });
  await page.getByText("Synthetic game screen").click();
  await expect(dropdown).toHaveCount(0);

  await page.evaluate(() =>
    (
      window as typeof window & {
        trainerFixture: { setTimeline: (value: string) => void };
      }
    ).trainerFixture.setTimeline("two"),
  );
  await expect(input).toHaveValue("");
  await expect(summary).toHaveCount(0);

  await page.evaluate(() =>
    (
      window as typeof window & {
        trainerFixture: { setMode: (value: string) => void };
      }
    ).trainerFixture.setMode("live"),
  );
  await expect(
    page.getByRole("combobox", { name: "Search trainers" }),
  ).toHaveCount(0);
  await expect(
    page.getByText("Live Enemy", { exact: true }).first(),
  ).toBeVisible();

  await page.evaluate(() =>
    (
      window as typeof window & {
        trainerFixture: { setMode: (value: string) => void };
      }
    ).trainerFixture.setMode("unavailable"),
  );
  await expect(page.getByText("Battle data unavailable now")).toBeVisible();
  await expect(
    page.getByRole("combobox", { name: "Search trainers" }),
  ).toHaveCount(0);
});

test("trainer search visual and dismissal states", async ({ page }) => {
  page.on("pageerror", (error) => {
    throw error;
  });
  await page.setViewportSize({ width: 1440, height: 1000 });
  await mountFixture(page);
  const input = page.getByRole("combobox", { name: "Search trainers" });
  await input.fill("Alex");
  await expect(page.getByRole("option")).toHaveCount(2);
  await page.evaluate(() => document.fonts.ready);
  await page.screenshot({
    path: "../../artifacts/trainer-after-desktop.png",
  });
  await input.press("Escape");
  await expect(input).toHaveAttribute("aria-expanded", "false");
  await input.click();
  await expect(page.getByRole("option")).toHaveCount(2);
  await page.getByRole("option").nth(1).click();
  await expect(page.locator(".selected-trainer-summary")).toContainText(
    "Night rematch",
  );
  await input.fill("blocked");
  await expect(page.locator(".trainer-search-message")).toHaveText(
    "Trainer index unavailable",
  );
  await input.press("Escape");
  await expect(input).toHaveValue("blocked");
  await page.setViewportSize({ width: 390, height: 844 });
  await page.getByRole("button", { name: "Info", exact: true }).click();
  await input.evaluate((element) =>
    element.scrollIntoView({ block: "center" }),
  );
  await input.fill("Alex");
  await expect(page.getByRole("option")).toHaveCount(2);
  const popup = await page.locator(".trainer-search-dropdown").boundingBox();
  expect(popup!.x).toBeGreaterThanOrEqual(0);
  expect(popup!.x + popup!.width).toBeLessThanOrEqual(390);
  expect(popup!.y).toBeGreaterThanOrEqual(0);
  expect(popup!.y + popup!.height).toBeLessThanOrEqual(844);
  await page.screenshot({
    path: "../../artifacts/trainer-after-mobile.png",
  });
});
