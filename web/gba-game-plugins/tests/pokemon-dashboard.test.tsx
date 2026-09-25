import { expect, it } from "vitest";
import { renderToStaticMarkup } from "react-dom/server";
import { ArenaStatus } from "../ui/pokemon-dashboard";
import { TypeBadge } from "../ui/game-layout";
import type { GameArenaStatus } from "@evenfall/gba/bridge";

it.each([
  ["ELECTR", "electric"],
  ["Electric", "electric"],
  ["FIGHT", "fighting"],
  ["Fighting", "fighting"],
  ["PSYCHC", "psychic"],
  ["NORMAL", "normal"],
])(
  "colors the ROM type label %s as %s without changing its text",
  (type, color) => {
    expect(renderToStaticMarkup(<TypeBadge type={type} />)).toBe(
      `<span class="type-badge type-${color}">${type}</span>`,
    );
  },
);

it("renders game-defined arena fields without a weather/terrain vocabulary in the UI", () => {
  const arena: GameArenaStatus = {
    title: "Custom arena",
    fields: [
      { id: "terrain", label: "Terrain", value: "Electric · 3 turns" },
      { id: "shield", label: "Crystal shield", value: "Player +2" },
      {
        id: "trainer",
        label: "Trainer",
        value: null,
        reason: "No verified trainer symbol",
      },
    ],
  };
  const html = renderToStaticMarkup(<ArenaStatus arena={arena} />);
  expect(html).toContain("<dt>Terrain</dt><dd>Electric · 3 turns</dd>");
  expect(html).toContain("<dt>Crystal shield</dt><dd>Player +2</dd>");
  expect(html).toContain('title="No verified trainer symbol"');
  expect(html).toContain("<dt>Trainer</dt><dd>Unavailable</dd>");
  expect(html).not.toContain("Weather");
  expect(
    renderToStaticMarkup(
      <ArenaStatus arena={{ title: "Exploring", fields: [] }} />,
    ),
  ).not.toContain("Crystal shield");
});
