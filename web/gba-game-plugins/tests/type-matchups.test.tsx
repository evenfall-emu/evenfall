import { renderToStaticMarkup } from "react-dom/server";
import { describe, expect, it } from "vite-plus/test";
import {
  defaultTypeSystem,
  extendTypeSystem,
  pokemonTypes,
  standardTypeDefinition,
  type PokemonTypeSystem,
} from "../src/type-system";
import type {
  PokemonBattleBattler,
  PokemonBattleStatStages,
  PokemonMove,
  PokemonStats,
} from "../src/model";
import { PokemonInspector } from "../ui/pokemon-dashboard";
import { TypeBadge, TypeSystemContext } from "../ui/game-layout";

const type = (name: string, id = 0) => ({ id, name });
const hit = (attack: string, ...defender: string[]) =>
  defaultTypeSystem.effectiveness(
    type(attack),
    defender.map((t) => type(t)),
  );

describe("default type system", () => {
  it("parses Gen III abbreviations, mixed case and punctuation", () => {
    expect(defaultTypeSystem.key(type("ELECTR"))).toBe("electric");
    expect(defaultTypeSystem.key(type("Fight"))).toBe("fighting");
    expect(defaultTypeSystem.key(type("PSYCHC"))).toBe("psychic");
    expect(defaultTypeSystem.key(type("Fairy"))).toBe("fairy");
    expect(defaultTypeSystem.key(type("???"))).toBeNull();
    expect(defaultTypeSystem.key(type("SOUND"))).toBeNull();
  });

  it.each([
    ["GRND", "ground"],
    ["Drgn", "dragon"],
    ["STL", "steel"],
    ["Elec", "electric"],
    ["FLYNG", "flying"],
    ["Psych", "psychic"],
    ["WTR", "water"],
  ])("parses the hack spelling %s as %s", (name, key) => {
    expect(defaultTypeSystem.key(type(name))).toBe(key);
  });

  it("leaves ambiguous, too short or foreign names unmapped", () => {
    expect(defaultTypeSystem.key(type("FIR"))).toBeNull(); // fire or fairy
    expect(defaultTypeSystem.key(type("FI"))).toBeNull();
    expect(defaultTypeSystem.key(type("STELLAR"))).toBeNull();
    expect(defaultTypeSystem.key(type("FIREFLY"))).toBeNull();
  });

  it("parses every standard name and its truncations to at least four letters", () => {
    for (const key of pokemonTypes)
      for (let length = Math.min(4, key.length); length <= key.length; length++)
        expect(
          defaultTypeSystem.key(type(key.slice(0, length).toUpperCase())),
        ).toBe(key);
  });

  it("multiplies dual types into every tone", () => {
    expect(hit("NORMAL", "GHOST", "DARK")).toBe(0);
    expect(hit("GRASS", "FIRE", "FLYING")).toBe(0.25);
    expect(hit("FIRE", "WATER")).toBe(0.5);
    expect(hit("WATER", "NORMAL")).toBe(1);
    expect(hit("ELECTR", "WATER")).toBe(2);
    expect(hit("ICE", "DRAGON", "GROUND")).toBe(4);
    expect(hit("FIGHT", "NORMAL", "NORMAL")).toBe(2);
  });

  it("uses the modern chart, where Steel no longer resists Ghost or Dark", () => {
    expect(hit("GHOST", "STEEL")).toBe(1);
    expect(hit("DARK", "STEEL")).toBe(1);
    expect(hit("DRAGON", "FAIRY")).toBe(0);
  });

  it("treats unmapped types as neutral defenders and unknown attackers as unknown", () => {
    expect(hit("FIRE", "GRASS", "???")).toBe(2);
    expect(hit("FIRE", "???")).toBeNull();
    expect(hit("???", "GRASS")).toBeNull();
  });

  it("covers every standard type in its chart", () => {
    expect(Object.keys(standardTypeDefinition.chart).sort()).toEqual(
      [...pokemonTypes].sort(),
    );
  });
});

describe("game type systems", () => {
  const extended = extendTypeSystem(standardTypeDefinition, {
    types: {
      sound: { color: "#123456" },
      light: { pattern: /^(light|lght|lite)$/ },
      fire: { pattern: /^(fire|flame)$/ },
    },
    ids: { 9: null, 30: "sound" },
    chart: { sound: { normal: 2 }, fire: { water: 2, dragon: 1 } },
  });

  it("adds types, maps IDs before names and merges chart rows", () => {
    expect(extended.key(type("SOUND"))).toBe("sound");
    expect(extended.key(type("SND"))).toBe("sound");
    expect(extended.key(type("LITE"))).toBe("light");
    expect(extended.key(type("FLAME"))).toBe("fire");
    expect(extended.key(type("FIR"))).toBeNull();
    expect(extended.key(type("Renamed", 30))).toBe("sound");
    expect(extended.key(type("FIRE", 9))).toBeNull();
    expect(extended.effectiveness(type("SOUND"), [type("NORMAL")])).toBe(2);
    expect(extended.effectiveness(type("FIRE"), [type("WATER")])).toBe(2);
    expect(extended.effectiveness(type("FIRE"), [type("GRASS")])).toBe(2);
    expect(extended.effectiveness(type("FIRE"), [type("DRAGON")])).toBe(1);
    expect(hit("FIRE", "WATER")).toBe(0.5);
  });

  it("colors added types inline and standard ones by class", () => {
    const badge = (system: PokemonTypeSystem, name: string) =>
      renderToStaticMarkup(
        <TypeSystemContext value={system}>
          <TypeBadge type={type(name)} />
        </TypeSystemContext>,
      );
    expect(badge(extended, "SOUND")).toBe(
      '<span class="type-badge type-sound" style="background:#123456">SOUND</span>',
    );
    expect(badge(extended, "FIRE")).toBe(
      '<span class="type-badge type-fire">FIRE</span>',
    );
    expect(badge(defaultTypeSystem, "SOUND")).toBe(
      '<span class="type-badge">SOUND</span>',
    );
  });
});

describe("move matchups in the inspector", () => {
  const stats: PokemonStats = {
    hp: 1,
    attack: 1,
    defense: 1,
    speed: 1,
    specialAttack: 1,
    specialDefense: 1,
  };
  const stages: PokemonBattleStatStages = {
    attack: 0,
    defense: 0,
    speed: 0,
    specialAttack: 0,
    specialDefense: 0,
    accuracy: 0,
    evasion: 0,
  };
  const move = (
    slot: number,
    name: string,
    moveType: string,
    category: PokemonMove["category"] = "special",
    power = 90,
  ): PokemonMove => ({
    id: slot + 1,
    slot,
    name,
    type: type(moveType),
    category,
    pp: 10,
    maxPp: 10,
    power,
    accuracy: 100,
  });
  const battler = (
    name: string,
    types: string[],
    hp = 10,
  ): PokemonBattleBattler => ({
    species: 1,
    speciesName: name,
    nickname: name,
    language: 2,
    level: 50,
    isEgg: false,
    gender: null,
    shiny: null,
    nature: null,
    types: types.map((t, id) => type(t, id)),
    abilities: null,
    heldItem: null,
    stats: { actual: stats, base: null, ivs: stats, evs: stats },
    moves: [],
    artwork: null,
    warnings: [],
    battler: 1,
    side: "opponent",
    partySlot: 0,
    hp,
    maxHp: 10,
    status: 0,
    volatileStatus: 0,
    ailments: [],
    statStages: stages,
    stageAdjustedStats: stats,
  });
  const attacker = {
    ...battler("Attacker", ["WATER"]),
    moves: [
      move(0, "SURF", "WATER"),
      move(1, "THUNDERBOLT", "ELECTR"),
      move(2, "EARTHQUAKE", "GROUND", "physical"),
      move(3, "THUNDER WAVE", "ELECTR", "status", 0),
    ],
  };
  const rows = (targets: PokemonBattleBattler[]) =>
    renderToStaticMarkup(
      <PokemonInspector
        pokemon={attacker}
        artwork={() => ({})}
        targets={targets}
      />,
    )
      .split("<tr")
      .slice(2);

  it("colors each damaging move by its matchup and labels it", () => {
    const [surf, bolt, quake, wave] = rows([
      battler("Gyarados", ["WATER", "FLYING"]),
    ]);
    expect(surf).toContain('class="matchup-half"');
    expect(surf).toContain(">×½</span>");
    expect(bolt).toContain('class="matchup-quadruple"');
    expect(bolt).toContain("×4 against Gyarados");
    expect(quake).toContain('class="matchup-immune"');
    expect(quake).toContain(">×0</span>");
    expect(wave).not.toContain("matchup");
  });

  it("keeps neutral rows plain and shows the best result among standing foes", () => {
    const [surf, bolt] = rows([
      battler("Marill", ["WATER"]),
      battler("Golem", ["ROCK", "GROUND"]),
      battler("Fainted", ["WATER", "FLYING"], 0),
    ]);
    expect(surf).toContain('class="matchup-quadruple"');
    expect(surf).toContain("×4 against Golem");
    expect(bolt).toContain('class="matchup-double"');
    expect(bolt).toContain("×2 against Marill");
    expect(rows([battler("Rattata", ["NORMAL"])])[0]).not.toContain(
      'class="matchup',
    );
  });

  it("does not highlight without opposing battlers", () => {
    expect(rows([]).join("")).not.toContain("matchup");
  });
});
