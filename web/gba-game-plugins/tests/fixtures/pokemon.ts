import type { PokemonCatalog } from "../../src/catalog";
import type { PokemonResources, PokemonRules } from "../../src/bridges";
import type { PokemonStats } from "../../src/model";

const stats = (values: readonly number[]): PokemonStats => ({
  hp: values[0],
  attack: values[1],
  defense: values[2],
  speed: values[3],
  specialAttack: values[4],
  specialDefense: values[5],
});

/** Deliberately small, synthetic catalog for bridge and browser fixtures. */
export function pokemonResources(): PokemonResources {
  const catalog: PokemonCatalog = {
    species: (id) => ({
      id,
      name: `Species ${id}`,
      base: stats([10, 20, 30, 40, 50, 60]),
      legacyBase: stats([0, 21, 0, 41, 0, 61]),
      types: [1, 2],
      legacyTypes: [3, 3],
      alternateTypes: [4, 5],
      abilities: [6, 7],
      legacyAbilities: [8, 9],
      genderRatio: id === 3 ? 255 : id === 2 ? 254 : 128,
      growth: 0,
    }),
    move: (id) => ({
      id,
      name: `Move ${id}`,
      type: { id: id % 3, name: `Type ${id % 3}` },
      category: (["physical", "special", "status"] as const)[id % 3],
      pp: 10 * id,
      power: id,
      accuracy: 100,
    }),
    type: (id) => ({ id, name: `Type ${id}` }),
    ability: (id) => ({ id, name: `Ability ${id}` }),
    item: (id) => ({ id, name: `Item ${id}` }),
    nature: (id) => ({ id, name: `Nature ${id}` }),
    experience: (_growth, level) => level * 100,
  };
  return {
    catalog,
    artwork: () => ({
      width: 1,
      height: 1,
      pixels: new Uint8ClampedArray(4),
    }),
  };
}

export const pokemonRules: PokemonRules = {
  legacyStats: false,
  typeOverrides: {},
  legacyAbilities: [],
  typesUnavailable: null,
  abilitiesUnavailable: null,
  baseStatsUnavailable: null,
  shinyThreshold: 8,
  shinyRequiresPersonality: false,
  modernShinySpecies: [],
};
