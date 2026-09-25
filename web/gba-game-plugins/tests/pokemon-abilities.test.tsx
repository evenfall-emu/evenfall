import { renderToStaticMarkup } from "react-dom/server";
import { expect, it } from "vitest";
import { createMemoryReader } from "@evenfall/gba/bridge";
import { eliteRedux, eliteReduxBeta2 } from "../src/elite-redux";
import { pokemonResources } from "./fixtures/pokemon";
import {
  PokemonDashboard,
  PokemonInspector,
  PokemonRoster,
} from "../ui/pokemon-dashboard";
import type {
  PokemonAbility,
  PokemonDetails,
  PokemonLinksFactory,
  PokemonStats,
} from "../src/model";

const stats: PokemonStats = {
  hp: 1,
  attack: 2,
  defense: 3,
  speed: 4,
  specialAttack: 5,
  specialDefense: 6,
};

const abilities: readonly PokemonAbility[] = [
  {
    id: 1,
    slot: 0,
    kind: "ability",
    state: "available",
    name: "Main Ability",
    description: "Main description",
  },
  {
    id: 2,
    slot: 1,
    kind: "innate",
    state: "locked",
    name: "Locked Innate",
    description: "Locked description",
    reason: "Unlocks later",
  },
  {
    id: 3,
    slot: 2,
    kind: "innate",
    state: "suppressed",
    name: "Suppressed Innate",
    description: "Suppressed description",
    reason: "Disabled in this mode",
  },
  {
    id: 4,
    slot: 3,
    kind: "extra",
    state: "unknown",
    name: "Unknown Extra",
    description: "Unknown description",
    reason: "State could not be decoded",
  },
  {
    id: 5,
    slot: 4,
    kind: "extra",
    state: "available",
    name: "Fifth Ability",
    description: "Fifth description",
  },
];

const pokemon: PokemonDetails = {
  personality: 1,
  trainerId: 2,
  species: 3,
  speciesName: "Testmon",
  nickname: "Testmon",
  trainerName: "Trainer",
  language: 2,
  level: 50,
  isEgg: false,
  gender: "genderless",
  shiny: false,
  nature: null,
  types: null,
  abilities,
  heldItem: null,
  friendship: 0,
  experience: { total: 0, levelStart: 0, nextLevel: null },
  stats: { actual: stats, base: stats, ivs: stats, evs: stats },
  moves: [],
  artwork: null,
  warnings: [],
};

const render = (value: PokemonDetails) =>
  renderToStaticMarkup(
    <PokemonInspector pokemon={value} artwork={() => ({})} />,
  );

it("opens the bridge's versioned Pokédex entry using species, not selection or national dex ID", () => {
  const resources = eliteRedux.prepare!({
    rom: createMemoryReader([]),
    romHash: eliteReduxBeta2.romHash,
  });
  const value = {
    ...pokemon,
    id: "preview:19",
    species: 1551,
    speciesName: "Rattata Alolan",
    nickname: "Rattata Alolan",
  };
  const html = renderToStaticMarkup(
    <PokemonInspector
      pokemon={value}
      artwork={() => ({})}
      linksFactory={resources.linksFactory}
    />,
  );
  expect(html).toContain(
    'href="https://forwardfeed.github.io/ER-nextdex/?v=2.65beta&amp;p=1551"',
  );
  expect(html).toContain('target="_blank"');
  expect(html).toContain('rel="noopener noreferrer"');
  expect(html).toContain(
    'aria-label="Open Rattata Alolan in Pokédex (new tab)"',
  );
});

it("omits links for absent factories, unavailable entries, eggs, and empty selections", () => {
  const factories: (PokemonLinksFactory | undefined)[] = [
    undefined,
    {},
    { pokemon: () => null },
  ];
  for (const linksFactory of factories) {
    expect(
      renderToStaticMarkup(
        <PokemonInspector
          pokemon={pokemon}
          artwork={() => ({})}
          linksFactory={linksFactory}
        />,
      ),
    ).not.toContain("<a ");
  }
  expect(
    renderToStaticMarkup(
      <PokemonInspector
        pokemon={{ ...pokemon, isEgg: true }}
        artwork={() => ({})}
        linksFactory={eliteReduxBeta2.linksFactory}
      />,
    ),
  ).not.toContain("<a ");
  expect(
    renderToStaticMarkup(
      <PokemonInspector
        artwork={() => ({})}
        linksFactory={{
          pokemon: () => {
            throw new Error("No selected Pokémon");
          },
        }}
      />,
    ),
  ).not.toContain("<a ");
});

it("renders every equipped ability with its semantic state and no active claim", () => {
  const html = render(pokemon);

  for (const ability of abilities) expect(html).toContain(ability.name);
  expect(html.match(/aria-label="(?:Ability|Innate|Extra):/g)).toHaveLength(5);
  expect(html).toContain("Innate: Locked Innate (Locked)");
  expect(html).toContain("Innate: Suppressed Innate (Suppressed)");
  expect(html).toContain("Extra: Unknown Extra (Unknown)");
  expect(html).not.toContain("Active");
});

it("distinguishes unknown ability data from a known empty ability list", () => {
  expect(render({ ...pokemon, abilities: null })).toContain(
    "<span>Ability</span><strong>Unavailable</strong>",
  );
  expect(render({ ...pokemon, abilities: [] })).toContain(
    "<span>Ability</span><strong>None</strong>",
  );
});

it("colors frostbite aliases ice-blue without replacing the bridge's label", () => {
  for (const ailment of ["Frostbite", "FROSTBYTE", "Frozen", "frz"]) {
    const value = { ...pokemon, hp: 1, maxHp: 1, ailments: [ailment] };
    expect(render(value)).toContain(
      `<span class="condition affected" style="background-color:#98d9da;color:#122127">${ailment}</span>`,
    );
  }
});

it("uses common status colors while retaining the existing abbreviations", () => {
  for (const [ailment, label, color] of [
    ["Burn", "BRN", "#f67843"],
    ["burnt", "burnt", "#f67843"],
    ["BRN", "BRN", "#f67843"],
    ["Paralysis", "PAR", "#f4d628"],
    ["para", "para", "#f4d628"],
    ["PARALYZED", "PARALYZED", "#f4d628"],
    ["Freeze", "FRZ", "#98d9da"],
    ["Bleed", "Bleed", "#ff786a"],
    ["bleeding", "bleeding", "#ff786a"],
    ["Sleep", "SLP", "#ffffff"],
    ["asleep", "asleep", "#ffffff"],
    ["SLP", "SLP", "#ffffff"],
  ]) {
    const value = { ...pokemon, hp: 1, maxHp: 1, ailments: [ailment] };
    expect(render(value)).toContain(
      `<span class="condition affected" style="background-color:${color};color:#122127">${label}</span>`,
    );
  }
});

it("keeps poison and unknown statuses on the fallback, with fainting taking precedence", () => {
  for (const [ailment, label] of [
    ["Poison", "PSN"],
    ["Bad poison", "TOX"],
    ["Moonburn", "Moonburn"],
  ]) {
    const value = { ...pokemon, hp: 1, maxHp: 1, ailments: [ailment] };
    expect(render(value)).toContain(
      `<span class="condition affected">${label}</span>`,
    );
  }
  const fainted = { ...pokemon, hp: 0, maxHp: 1, ailments: ["Frostbite"] };
  expect(render(fainted)).toContain(
    '<span class="condition affected">FNT</span>',
  );
  const healthy = { ...pokemon, hp: 1, maxHp: 1, ailments: [] };
  expect(render(healthy)).not.toContain('class="condition');
});

it("shows six ordered totals with only an arrow for non-neutral active battle stages", () => {
  // Deliberately not the standard ratio: the UI must honor the bridge, not redo game math.
  const value = {
    ...pokemon,
    stageAdjustedStats: { ...stats, attack: 77, defense: 1 },
    statStages: {
      attack: 2,
      defense: -1,
      speed: 0,
      specialAttack: 0,
      specialDefense: 0,
      accuracy: 0,
      evasion: 0,
    },
  };
  const active = renderToStaticMarkup(
    <PokemonInspector pokemon={value} artwork={() => ({})} active />,
  );
  expect(active.match(/aria-label="Stat: [^"]+"/g)).toEqual([
    'aria-label="Stat: HP, total 1"',
    'aria-label="Stat: Attack, total 77, stage +2"',
    'aria-label="Stat: Defense, total 1, stage -1"',
    'aria-label="Stat: Special Attack, total 5"',
    'aria-label="Stat: Special Defense, total 6"',
    'aria-label="Stat: Speed, total 4"',
  ]);
  expect(active).toContain('class="stage-up" aria-hidden="true">↑</span>');
  expect(active).toContain('class="stage-down" aria-hidden="true">↓</span>');
  expect(active).not.toContain("↑2");
  expect(active).not.toContain("<th>Base</th>");
  expect(render(value)).toContain("<strong>2</strong>");
  expect(render(value)).not.toContain("77</strong>");
  expect(render(value)).not.toContain('class="stage-up"');
  expect(
    render({ ...pokemon, stats: { ...pokemon.stats, actual: null } }),
  ).toContain('aria-label="Stat: Attack, total unavailable"');
});

it("keeps status in the inspector but removes it from compact party previews", () => {
  const value = {
    ...pokemon,
    hp: 1,
    maxHp: 1,
    ailments: ["Frostbite"],
    status: 4096,
    slot: 0,
  };
  const html = renderToStaticMarkup(
    <PokemonRoster party={[value]} artwork={() => ({})} label="Party" />,
  );
  expect(html.match(/>Frostbite</g)).toHaveLength(1);
  expect(html.match(/<ol\b[^]*?<\/ol>/)?.[0]).not.toContain("Frostbite");
});

it("renders held-item artwork through the shared resource callback instead of an empty square", () => {
  const value: PokemonDetails = {
    ...pokemon,
    heldItem: {
      id: 1,
      name: "Master Ball",
      artwork: { kind: "item", item: 1, romHash: "test" },
    },
  };
  const html = renderToStaticMarkup(
    <PokemonInspector
      pokemon={value}
      artwork={(ref) => (ref?.kind === "item" ? { src: "item.png" } : {})}
    />,
  );
  expect(html).toContain('class="item-sprite"');
  expect(html).toContain('src="item.png"');
  expect(html).not.toContain("item-outline");
  const unavailable = render(value);
  expect(unavailable).toContain("Master Ball");
  expect(unavailable).not.toContain('class="item-sprite"');
  expect(unavailable).not.toContain("item-outline");
});

it("renders the shared dashboard from semantic bridge states", () => {
  const idle = { status: "idle" } as const;
  const html = renderToStaticMarkup(
    <PokemonDashboard
      gameName="Another Pokémon Game"
      party={idle}
      storage={idle}
      battle={idle}
      context={idle}
      resources={undefined}
    >
      <p>Game screen</p>
    </PokemonDashboard>,
  );

  expect(html).toContain("Another Pokémon Game");
  expect(html).toContain("Game screen");
});

it("passes resource link factories to both selected battle Pokémon without choosing URLs in the UI", () => {
  const member = {
    ...pokemon,
    hp: 1,
    maxHp: 1,
    status: 0,
    ailments: [],
    slot: 0,
  };
  const idle = { status: "idle" } as const;
  const html = renderToStaticMarkup(
    <PokemonDashboard
      gameName="Another Pokémon Game"
      party={idle}
      storage={idle}
      battle={{
        status: "ready",
        sample: { romHash: "test", timeline: "one", sequence: 1, cycle: 0n },
        data: {
          active: true,
          kind: "trainer",
          format: "single",
          typeFlags: 0,
          types: [],
          turn: null,
          battleWeather: null,
          playerParty: [member],
          opponentParty: [{ ...member, species: 25 }],
          battlers: [],
          warnings: [],
        },
      }}
      context={idle}
      resources={{
        ...pokemonResources(),
        linksFactory: {
          pokemon: (value) => `https://example.com/pokemon/${value.species}`,
        },
      }}
    >
      <p>Game screen</p>
    </PokemonDashboard>,
  );
  expect(html).toContain('href="https://example.com/pokemon/3"');
  expect(html).toContain('href="https://example.com/pokemon/25"');
});
