// The Pokémon Gen III family (pokeemerald and hacks built on it): what its
// overlay needs, the sources those views read, and the standard views and ROM
// resources. A game spreads `pokemonGen3(format)` and replaces what differs.
import {
  createGame,
  defineFamily,
  pointer,
  ram,
  ramArray,
  type DecodeResult,
  type GameArenaStatus,
  type MemoryLayout,
  type MemoryRef,
  type RamArraySource,
  type RamSource,
  type RomContext,
  type View,
} from "@evenfall/gba/bridge";
import { createPokemonArtwork, type PokemonArtworkConfig } from "./artwork";
import {
  battleKind,
  detailedGameBattle,
  detailedGameParty,
  gameStorage,
  pokemonStorageLayout,
  type PokemonResources,
  type PokemonRules,
  type StoredRecords,
} from "./bridges";
import { defaultTypeSystem, type PokemonTypeSystem } from "./type-system";
import { createGen3Catalog, type Gen3CatalogConfig } from "./catalog";
import {
  gen3BattlePokemon,
  gen3BoxPokemon,
  gen3PartyRecord,
  type Gen3BattlePokemon,
  type Gen3RecordFormat,
} from "./gen3-record";
import type {
  PartyRecord,
  PokemonBattleView,
  PokemonLinksFactory,
  PokemonPartyView,
  PokemonStorageView,
  PokemonTrainerSearch,
} from "./model";

/** ROM-bound helpers the overlay calls; they never enter a memory snapshot. */
export interface PokemonMedia {
  readonly artwork: PokemonResources["artwork"];
  readonly links: PokemonLinksFactory | null;
  readonly typeSystem: PokemonTypeSystem;
}

/** Arena facts for the overlay; games add the location details they verify. */
export interface PokemonContextView {
  readonly arena: GameArenaStatus;
  readonly location?: {
    readonly mapGroup: number;
    readonly mapNumber: number;
    readonly x: number;
    readonly y: number;
    readonly regionMapSection: number;
    readonly name: string | null;
  };
  /** Configured map weather, not the live battle weather effect. */
  readonly weather?: { readonly id: number; readonly name: string | null };
  readonly warnings?: readonly string[];
}

/** Everything the Pokémon overlay renders. `trainers` is optional per game. */
export interface Gen3Views {
  readonly party: PokemonPartyView;
  readonly storage: PokemonStorageView;
  readonly battle: PokemonBattleView;
  readonly context: PokemonContextView;
  readonly media: PokemonMedia;
  readonly trainers?: PokemonTrainerSearch;
}

/** The sources the standard Gen III views read, named as in the decompilation. */
export type Gen3Sources = {
  readonly gPlayerPartyCount: RamSource<number>;
  readonly gPlayerParty: RamArraySource<DecodeResult<PartyRecord>>;
  readonly gEnemyParty: RamArraySource<DecodeResult<PartyRecord>>;
  /** Pointer to the save block holding the game's option flags. */
  readonly rules: RamSource<MemoryRef<PokemonRules> | null>;
  readonly storage: RamSource<MemoryRef<DecodeResult<StoredRecords>> | null>;
  readonly battleActive: RamSource<boolean>;
  readonly gBattleTypeFlags: RamSource<number>;
  readonly gBattlersCount: RamSource<number>;
  readonly gBattlerPartyIndexes: RamArraySource<number>;
  readonly gBattleMons: RamArraySource<DecodeResult<Gen3BattlePokemon>>;
  readonly gAbsentBattlerFlags: RamSource<number>;
};

/** Layout facts that differ between Gen III games but not between their builds. */
export interface Gen3Format {
  readonly record: Gen3RecordFormat;
  readonly battleLimits: {
    readonly speciesLimit: number;
    readonly moveLimit: number;
    readonly typeLimit: number;
    readonly abilityLimit: number;
  };
  readonly storage: Parameters<typeof pokemonStorageLayout>[1];
  /** Decodes the game's option flags into the rules the views apply. */
  readonly rules: MemoryLayout<PokemonRules>;
}

/** Per-build ROM data the standard resources read (part of the manifest). */
export interface Gen3Rom {
  readonly catalog: Gen3CatalogConfig;
  readonly artwork: Omit<PokemonArtworkConfig, "romHash">;
}

const u8: MemoryLayout<number> = { byteLength: 1, decode: (m, a) => m.u8(a) };
const u16: MemoryLayout<number> = { byteLength: 2, decode: (m, a) => m.u16(a) };
const u32: MemoryLayout<number> = { byteLength: 4, decode: (m, a) => m.u32(a) };
/** gMain.inBattle. */
const mainInBattle: MemoryLayout<boolean> = {
  byteLength: 1,
  decode: (memory, address) => Boolean(memory.u8(address) & 2),
};

export function gen3Sources(format: Gen3Format): Gen3Sources {
  const party = gen3PartyRecord(format.record);
  return {
    gPlayerPartyCount: ram(u8),
    gPlayerParty: ramArray(party, 6),
    gEnemyParty: ramArray(party, 6),
    rules: ram(pointer(format.rules)),
    storage: ram(
      pointer(
        pokemonStorageLayout(gen3BoxPokemon(format.record), format.storage),
      ),
    ),
    battleActive: ram(mainInBattle),
    gBattleTypeFlags: ram(u32),
    gBattlersCount: ram(u8),
    gBattlerPartyIndexes: ramArray(u16, 4),
    gBattleMons: ramArray(gen3BattlePokemon(format.battleLimits), 4),
    gAbsentBattlerFlags: ram(u8),
  };
}

/** Artwork, links and types for the overlay, from any Pokémon resources. */
export const mediaView: View<{}, PokemonResources, PokemonMedia> = ({
  resources,
}) => ({
  status: "ready",
  data: {
    artwork: resources.artwork,
    links: resources.linksFactory ?? null,
    typeSystem: resources.typeSystem ?? defaultTypeSystem,
  },
});

/** Battle or overworld, without map details; games replace it when they know more. */
export const battleContextView: View<
  Pick<Gen3Sources, "battleActive" | "gBattleTypeFlags">,
  unknown,
  PokemonContextView
> = ({ memory, symbols }) => {
  const kind = memory.read(symbols.battleActive)
    ? battleKind(memory.read(symbols.gBattleTypeFlags))
    : null;
  return {
    status: "ready",
    data: {
      arena: {
        title: kind
          ? `${kind[0].toUpperCase()}${kind.slice(1)} battle`
          : "Overworld",
        fields: [],
      },
    },
  };
};

/**
 * ROM checks for any Gen III build: well-known catalog entries decode and the
 * first species' artwork renders. IDs are the national ones every hack keeps.
 */
export const gen3Probes = {
  species: ({ catalog }: PokemonResources) => {
    for (let id = 1; id <= 25; id++) catalog.species(id);
  },
  moves: ({ catalog }: PokemonResources) => {
    for (let id = 1; id <= 20; id++) catalog.move(id);
  },
  abilities: ({ catalog }: PokemonResources) => {
    for (let id = 1; id <= 20; id++) catalog.ability(id);
  },
  items: ({ catalog }: PokemonResources) => {
    for (let id = 1; id <= 10; id++) catalog.item(id);
  },
  natures: ({ catalog }: PokemonResources) => {
    for (let id = 0; id < 25; id++) catalog.nature(id);
  },
  artwork: ({ artwork }: PokemonResources, romHash: string) => {
    for (const kind of ["icon", "front"] as const)
      artwork({
        romHash,
        species: 1,
        personality: 0,
        isEgg: false,
        palette: "normal",
        kind,
      });
  },
};

/** Any game whose views meet the Gen III contract, whatever its sources. */
export type Gen3Game = ReturnType<typeof createGame<Gen3Views, any, any, any>>;

/** The family itself: standard sources for `format`, views and resources. */
export function pokemonGen3(format: Gen3Format) {
  return defineFamily<Gen3Views, Gen3Rom>()({
    id: "pokemon/gen3",
    sources: gen3Sources(format),
    resources: ({ rom, romHash, config }: RomContext<Gen3Rom>) => ({
      catalog: createGen3Catalog(rom, config.catalog, romHash),
      artwork: createPokemonArtwork(rom, { ...config.artwork, romHash }),
    }),
    views: {
      party: detailedGameParty.decode,
      storage: gameStorage.decode,
      battle: detailedGameBattle.decode,
      context: battleContextView,
      media: mediaView,
    },
    probes: gen3Probes,
    // Emerald's game code, which hacks built on it keep.
    detect: { gameCodes: ["BPEE"] },
  });
}
