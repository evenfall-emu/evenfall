import type { ReactNode } from "react";
import { renderToStaticMarkup } from "react-dom/server";
import { expect, it } from "vite-plus/test";
import type { BridgeState } from "@evenfall/gba/bridge";
import { GoldenSunLayout } from "../ui/golden-sun-dashboard";
import type { GameDock } from "../ui/layout";
import type {
  GoldenSunBattleView,
  GoldenSunPartyMember,
  GoldenSunPartyView,
  GoldenSunUnit,
} from "../src/golden-sun";

const sample = { romHash: "t", timeline: "t", sequence: 1, cycle: 1n };
const unit = (name: string, agility: number): GoldenSunUnit => ({
  slot: 0,
  name,
  level: 21,
  hp: 10,
  maxHp: 10,
  pp: 0,
  maxPp: 0,
  attack: 1,
  defense: 1,
  agility,
  luck: 1,
  elements: [100, 72, 72, 48].map((resist) => ({ power: 85, resist })),
  djinn: Array.from({ length: 4 }, () => ({ owned: 0, set: 0 })),
  statuses: [],
});
const isaac: GoldenSunPartyMember = {
  ...unit("Isaac", 151),
  element: "venus",
};
const party: BridgeState<GoldenSunPartyView> = {
  status: "ready",
  sample,
  data: { members: [isaac], coins: 60612 },
};

/** The dock the layout asks for, rendered to markup. */
function dockFor(battle: BridgeState<GoldenSunBattleView>) {
  let dock: GameDock | undefined;
  renderToStaticMarkup(
    <GoldenSunLayout
      party={party}
      battle={battle}
      docked={(given) => {
        dock = given;
        return null;
      }}
    >
      <p>Screen</p>
    </GoldenSunLayout>,
  );
  const render = (node: ReactNode) => renderToStaticMarkup(<>{node}</>);
  return { dock: dock!, render };
}

it("opens the party outside battle and the battle panel while one runs", () => {
  expect(
    dockFor({ status: "ready", sample, data: { active: false } }).dock.focus,
  ).toBe("party");
  const { dock, render } = dockFor({
    status: "ready",
    sample,
    data: { active: true, enemies: [unit("Orc Brute1", 90)] },
  });
  expect(dock.focus).toBe("battle");
  expect(render(dock.heading)).toContain("60,612");
  const battle = render(dock.panels[1].content);
  expect(battle).toContain("Orc Brute1");
  // The enemy resists Venus and is weak to Jupiter; the turn order is by agility.
  expect(battle).toMatch(/gs-resists[^>]*>85\/100</);
  expect(battle).toMatch(/gs-weak[^>]*>85\/48</);
  const turns = battle.slice(battle.indexOf("Estimated turn order"));
  expect(turns.indexOf("Isaac")).toBeLessThan(turns.indexOf("Orc Brute1"));
});
