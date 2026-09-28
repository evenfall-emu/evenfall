import { renderToStaticMarkup } from "react-dom/server";
import { expect, it, vi } from "vite-plus/test";
import { PokemonDashboard, PokemonRoster } from "../ui/pokemon-dashboard";
import type { BridgeState } from "@evenfall/gba/bridge";
import type {
  PokemonBattleView,
  PokemonPartyMember,
  PokemonTrainerSearch,
} from "../src/model";

const sample = {
  romHash: "test",
  timeline: "timeline-1",
  sequence: 1,
  cycle: 1n,
};
const idle = { status: "idle" } as const;

const member = (name: string, id = name): PokemonPartyMember => ({
  id,
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
  stats: {
    actual: null,
    base: null,
    ivs: {
      hp: 0,
      attack: 0,
      defense: 0,
      speed: 0,
      specialAttack: 0,
      specialDefense: 0,
    },
    evs: {
      hp: 0,
      attack: 0,
      defense: 0,
      speed: 0,
      specialAttack: 0,
      specialDefense: 0,
    },
  },
  moves: [],
  artwork: null,
  warnings: [],
  slot: 0,
  hp: 1,
  maxHp: 1,
  status: 0,
  ailments: [],
});

const renderDashboard = (
  battle: BridgeState<PokemonBattleView>,
  trainerSearch?: PokemonTrainerSearch,
) =>
  renderToStaticMarkup(
    <PokemonDashboard
      gameName="Test game"
      party={idle}
      storage={idle}
      battle={battle}
      context={idle}
      resources={undefined}
      trainerSearch={trainerSearch}
    >
      <p>Screen</p>
    </PokemonDashboard>,
  );

it("replaces only the ready no-combat opponent panel with trainer search", () => {
  const search = vi.fn<PokemonTrainerSearch>(() => ({
    status: "ready",
    data: [],
  }));
  const html = renderDashboard(
    { status: "ready", sample, data: { active: false } },
    search,
  );

  expect(html).toContain('aria-label="Trainer search"');
  expect(html).toContain("Find a trainer to preview their team and details.");
  expect(html).not.toContain("No active battle");
  expect(search).toHaveBeenCalledOnce();
  expect(search).toHaveBeenCalledWith("");
});

it("shows a connector-provided unavailable reason without fabricating results", () => {
  const html = renderDashboard(
    { status: "ready", sample, data: { active: false } },
    () => ({ status: "unavailable", reason: "Trainer index is not ready" }),
  );
  expect(html).toContain("Trainer index is not ready");
  expect(html).not.toContain("Find a trainer to preview");
});

it("keeps live battle and unavailable battle states ahead of trainer previews", () => {
  const search = vi.fn<PokemonTrainerSearch>(() => ({
    status: "ready",
    data: [],
  }));
  const opponent = member("Live Enemy");
  const live: BridgeState<PokemonBattleView> = {
    status: "ready",
    sample,
    data: {
      active: true,
      kind: "trainer",
      format: "single",
      typeFlags: 0,
      types: [],
      turn: null,
      battleWeather: null,
      playerParty: [],
      opponentParty: [opponent],
      battlers: [],
      warnings: [],
    },
  };
  const liveHtml = renderDashboard(live, search);
  expect(liveHtml).toContain("Live Enemy");
  expect(liveHtml).not.toContain('aria-label="Trainer search"');
  expect(search).not.toHaveBeenCalled();

  const unavailableHtml = renderDashboard(
    { status: "unavailable", sample, reason: "Battle context changed" },
    search,
  );
  expect(unavailableHtml).toContain("Battle context changed");
  expect(unavailableHtml).not.toContain('aria-label="Trainer search"');
  expect(search).not.toHaveBeenCalled();
});

it("leaves games without a trainer search connector unchanged", () => {
  const html = renderDashboard({
    status: "ready",
    sample,
    data: { active: false },
  });
  expect(html).toContain("No active battle");
  expect(html).not.toContain('aria-label="Trainer search"');
});

it("uses preview ids before absent live personality fields for roster selection", () => {
  const html = renderToStaticMarkup(
    <PokemonRoster
      party={[
        member("First", "preview:first"),
        member("Second", "preview:second"),
      ]}
      artwork={() => ({})}
      opponent
      label="Preview"
    />,
  );
  expect(html).toContain("First");
  expect(html).toContain("Second");
  expect(html.match(/aria-pressed="true"/g)).toHaveLength(1);
});
