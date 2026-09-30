import type { NamedValue } from "./model";

/** The standard eighteen types. Games may add others through their type system. */
export const pokemonTypes = [
  "normal",
  "fighting",
  "flying",
  "poison",
  "ground",
  "rock",
  "bug",
  "ghost",
  "steel",
  "fire",
  "water",
  "grass",
  "electric",
  "psychic",
  "ice",
  "dragon",
  "dark",
  "fairy",
] as const;
export type StandardPokemonType = (typeof pokemonTypes)[number];
/** A standard type or one a game adds, in lowercase letters. */
export type PokemonTypeKey = string;
/** 1 lets an override make a standard pair neutral again. */
export type TypeMultiplier = 0 | 0.5 | 1 | 2;

export interface PokemonTypeInfo {
  /** Only needed for types the dashboard stylesheet lacks. */
  readonly color?: string;
  /**
   * Matches the type's normalized ROM name (lowercase letters only). Without
   * one, any abbreviation of the key of three letters or more matches.
   */
  readonly pattern?: RegExp;
}

/** Plain data a game overrides; everything not listed is neutral or unmapped. */
export interface PokemonTypeDefinition {
  /** Known keys, which the ROM's type names are parsed into. */
  readonly types: Readonly<Record<string, PokemonTypeInfo>>;
  /** Game type IDs with a fixed meaning, checked before names. Null means no type. */
  readonly ids: Readonly<Record<number, PokemonTypeKey | null>>;
  /** Attacking key → defending key → multiplier; absent pairs are neutral. */
  readonly chart: Readonly<
    Record<string, Readonly<Partial<Record<PokemonTypeKey, TypeMultiplier>>>>
  >;
}

export interface PokemonTypeSystem extends PokemonTypeDefinition {
  /** The key of a game's type, or null for types without matchups such as `???`. */
  key(type: NamedValue): PokemonTypeKey | null;
  /**
   * The product of the attacking type against each distinct defending type,
   * from the chart alone: no abilities, items, weather or move exceptions.
   * Unmapped defending types count as neutral; null when the attacking type or
   * every defending type is unmapped.
   */
  effectiveness(
    attack: NamedValue,
    defender: readonly NamedValue[],
  ): number | null;
}

const standardChart: PokemonTypeDefinition["chart"] = {
  normal: { rock: 0.5, ghost: 0, steel: 0.5 },
  fighting: {
    normal: 2,
    flying: 0.5,
    poison: 0.5,
    rock: 2,
    bug: 0.5,
    ghost: 0,
    steel: 2,
    psychic: 0.5,
    ice: 2,
    dark: 2,
    fairy: 0.5,
  },
  flying: {
    fighting: 2,
    rock: 0.5,
    bug: 2,
    steel: 0.5,
    grass: 2,
    electric: 0.5,
  },
  poison: {
    poison: 0.5,
    ground: 0.5,
    rock: 0.5,
    ghost: 0.5,
    steel: 0,
    grass: 2,
    fairy: 2,
  },
  ground: {
    flying: 0,
    poison: 2,
    rock: 2,
    bug: 0.5,
    steel: 2,
    fire: 2,
    grass: 0.5,
    electric: 2,
  },
  rock: {
    fighting: 0.5,
    flying: 2,
    ground: 0.5,
    bug: 2,
    steel: 0.5,
    fire: 2,
    ice: 2,
  },
  bug: {
    fighting: 0.5,
    flying: 0.5,
    poison: 0.5,
    ghost: 0.5,
    steel: 0.5,
    fire: 0.5,
    grass: 2,
    psychic: 2,
    dark: 2,
    fairy: 0.5,
  },
  ghost: { normal: 0, ghost: 2, psychic: 2, dark: 0.5 },
  steel: {
    rock: 2,
    steel: 0.5,
    fire: 0.5,
    water: 0.5,
    electric: 0.5,
    ice: 2,
    fairy: 2,
  },
  fire: {
    rock: 0.5,
    bug: 2,
    steel: 2,
    fire: 0.5,
    water: 0.5,
    grass: 2,
    ice: 2,
    dragon: 0.5,
  },
  water: { ground: 2, rock: 2, fire: 2, water: 0.5, grass: 0.5, dragon: 0.5 },
  grass: {
    flying: 0.5,
    poison: 0.5,
    ground: 2,
    rock: 2,
    bug: 0.5,
    steel: 0.5,
    fire: 0.5,
    water: 2,
    grass: 0.5,
    dragon: 0.5,
  },
  electric: {
    flying: 2,
    ground: 0,
    water: 2,
    grass: 0.5,
    electric: 0.5,
    dragon: 0.5,
  },
  psychic: { fighting: 2, poison: 2, steel: 0.5, psychic: 0.5, dark: 0 },
  ice: {
    flying: 2,
    ground: 2,
    steel: 0.5,
    fire: 0.5,
    water: 0.5,
    grass: 2,
    ice: 0.5,
    dragon: 2,
  },
  dragon: { steel: 0.5, dragon: 2, fairy: 0 },
  dark: { fighting: 0.5, ghost: 2, psychic: 2, dark: 0.5, fairy: 0.5 },
  fairy: {
    fighting: 2,
    poison: 0.5,
    steel: 0.5,
    fire: 0.5,
    dragon: 2,
    dark: 2,
  },
};

/** Lowercase letters only, so `PSYCHC`, `Psychc` and `Psy-chc` compare equal. */
export const typeNameKey = (name: string) =>
  name.toLowerCase().replace(/[^a-z]/g, "");

/**
 * The key's first letter, then any of its other letters in order: truncations
 * such as `electr` and `fight` and dropped letters such as `psychc` and `grnd`.
 */
export const abbreviationPattern = (key: PokemonTypeKey) =>
  new RegExp(`^${key[0]}${key.slice(1).replace(/[a-z]/g, "$&?")}$`);

export function createTypeSystem(
  definition: PokemonTypeDefinition,
): PokemonTypeSystem {
  const keys = Object.keys(definition.types);
  const explicit = keys.flatMap((key) => {
    const pattern = definition.types[key].pattern;
    return pattern ? [{ key, pattern }] : [];
  });
  const abbreviations = keys.map((key) => ({
    key,
    pattern: abbreviationPattern(key),
  }));
  // A name two types could abbreviate, such as `fir`, stays unmapped.
  const unique = (matches: readonly { key: PokemonTypeKey }[]) =>
    matches.length === 1 ? matches[0].key : null;
  const byName = new Map<string, PokemonTypeKey | null>();
  const parse = (name: string): PokemonTypeKey | null => {
    if (Object.hasOwn(definition.types, name)) return name;
    const matched = explicit.filter(({ pattern }) => pattern.test(name));
    if (matched.length) return unique(matched);
    return name.length < 3
      ? null
      : unique(abbreviations.filter(({ pattern }) => pattern.test(name)));
  };
  const key = (type: NamedValue): PokemonTypeKey | null => {
    if (Object.hasOwn(definition.ids, type.id)) return definition.ids[type.id];
    const name = typeNameKey(type.name);
    if (!byName.has(name)) byName.set(name, parse(name));
    return byName.get(name)!;
  };
  const system: PokemonTypeSystem = {
    ...definition,
    key,
    effectiveness(attack, defender) {
      const attacking = key(attack);
      const defending = new Set(defender.flatMap((type) => key(type) ?? []));
      if (attacking === null || !defending.size) return null;
      const row = definition.chart[attacking] ?? {};
      let product = 1;
      for (const type of defending) product *= row[type] ?? 1;
      return product;
    },
  };
  return Object.freeze(system);
}

/** The standard types and the modern (Generation VI onward) chart. */
export const standardTypeDefinition: PokemonTypeDefinition = {
  types: Object.fromEntries(pokemonTypes.map((type) => [type, {}])),
  ids: {},
  chart: standardChart,
};
export const defaultTypeSystem = createTypeSystem(standardTypeDefinition);

/**
 * A game's type system: its additions and replacements over `base`. Type
 * entries and chart rows merge per key, so a pattern keeps the type's color
 * and one changed pair keeps the rest of the row.
 */
export function extendTypeSystem(
  base: PokemonTypeDefinition,
  overrides: Partial<PokemonTypeDefinition>,
): PokemonTypeSystem {
  const chart: Record<string, PokemonTypeDefinition["chart"][string]> = {
    ...base.chart,
  };
  for (const [attack, row] of Object.entries(overrides.chart ?? {}))
    chart[attack] = { ...chart[attack], ...row };
  const types: Record<string, PokemonTypeInfo> = { ...base.types };
  for (const [key, info] of Object.entries(overrides.types ?? {}))
    types[key] = { ...types[key], ...info };
  return createTypeSystem({
    types,
    ids: { ...base.ids, ...overrides.ids },
    chart,
  });
}
