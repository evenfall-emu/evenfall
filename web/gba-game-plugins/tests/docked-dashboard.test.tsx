import type { ReactNode } from "react";
import { renderToStaticMarkup } from "react-dom/server";
import { expect, it } from "vite-plus/test";
import { PokemonDashboard } from "../ui/pokemon-dashboard";
import type { GameDock } from "../ui/layout";
import type { BridgeState } from "@evenfall/gba/bridge";
import type { PokemonBattleView, PokemonPartyMember } from "../src/model";

// What the dashboard hands the player's two-column dock (GameLayoutProps.docked).

const sample = { romHash: "test", timeline: "t", sequence: 1, cycle: 1n };
const idle = { status: "idle" } as const;
const zero = {
  hp: 0,
  attack: 0,
  defense: 0,
  speed: 0,
  specialAttack: 0,
  specialDefense: 0,
};
const member = (name: string): PokemonPartyMember => ({
  id: name,
  species: 1,
  speciesName: name,
  nickname: name,
  language: 2,
  level: 5,
  isEgg: false,
  gender: null,
  shiny: null,
  nature: null,
  types: null,
  abilities: [],
  heldItem: null,
  stats: { actual: null, base: null, ivs: zero, evs: zero },
  moves: [],
  artwork: null,
  warnings: [],
  slot: 0,
  hp: 1,
  maxHp: 1,
  status: 0,
  ailments: [],
});

/** The dock the dashboard asks for, and the page it renders from it. */
function dockFor(battle: BridgeState<PokemonBattleView>) {
  let dock: GameDock | undefined;
  const html = renderToStaticMarkup(
    <PokemonDashboard
      gameName="Route 29"
      party={{ status: "ready", sample, data: { party: [member("Mine")] } }}
      storage={idle}
      battle={battle}
      context={idle}
      resources={undefined}
      docked={(given) => {
        dock = given;
        return <div id="dock" />;
      }}
    >
      <p>Screen</p>
    </PokemonDashboard>,
  );
  const render = (node: ReactNode) => renderToStaticMarkup(<>{node}</>);
  return { dock: dock!, html, render };
}

it("hands its panels to the dock instead of laying out columns or placing the screen", () => {
  const { dock, html, render } = dockFor({
    status: "ready",
    sample,
    data: { active: false },
  });
  // The player places the screen; the layout neither places it nor its grid.
  expect(html).toBe('<div id="dock"></div>');
  expect(dock.panels.map((panel) => [panel.id, panel.label])).toEqual([
    ["team", "Team"],
    ["info", "Info"],
  ]);
  expect(dock.focus).toBe("team");
  expect(render(dock.heading)).toContain("Route 29");
  expect(render(dock.panels[0].content)).toContain("Mine");
  expect(render(dock.panels[1].content)).toContain("No active battle");
});

it("asks for the battle while one runs", () => {
  const { dock, render } = dockFor({
    status: "ready",
    sample,
    data: {
      active: true,
      kind: "wild",
      format: "single",
      typeFlags: 0,
      types: [],
      turn: null,
      battleWeather: null,
      playerParty: [member("Mine")],
      opponentParty: [member("Wild one")],
      battlers: [],
      warnings: [],
    },
  });
  expect(dock.panels.map((panel) => panel.label)).toEqual(["Team", "Battle"]);
  expect(dock.focus).toBe("info");
  expect(render(dock.panels[1].content)).toContain("Wild one");
});
