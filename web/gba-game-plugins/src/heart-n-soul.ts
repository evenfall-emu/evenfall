import {
  definePreparedGame,
  array,
  pointer,
  ref,
  u8,
  type MemoryLayout,
  type GameArenaStatus,
  type GameStatusField,
} from "@evenfall/gba/bridge";
import {
  gen3BattlePokemon,
  gen3BoxPokemon,
  gen3PartyRecord,
} from "./gen3-record";
import { createGen3Catalog, type Gen3CatalogConfig } from "./catalog";
import { createPokemonArtwork } from "./artwork";
import {
  battleKind,
  detailedGameBattle,
  detailedGameParty,
  gameStorage,
  pokemonStorageLayout,
  type PokemonRules,
} from "./bridges";

const littleU16: MemoryLayout<number> = {
  byteLength: 2,
  decode: (memory, address) => memory.u16(address),
};
const littleU32: MemoryLayout<number> = {
  byteLength: 4,
  decode: (memory, address) => memory.u32(address),
};
const mainInBattle: MemoryLayout<boolean> = {
  byteLength: 1,
  decode: (memory, address) => Boolean(memory.u8(address) & 2),
};
const mapContext: MemoryLayout<{
  readonly x: number;
  readonly y: number;
  readonly mapGroup: number;
  readonly mapNumber: number;
}> = {
  byteLength: 0x34,
  decode: (memory, address) => ({
    x: memory.u16(address),
    y: memory.u16(address + 2),
    mapGroup: memory.u8(address + 4),
    mapNumber: memory.u8(address + 5),
  }),
};
const mapHeader: MemoryLayout<{
  readonly regionMapSection: number;
  readonly weather: number;
}> = {
  byteLength: 0x1c,
  decode: (memory, address) => ({
    regionMapSection: memory.u8(address + 0x14),
    weather: memory.u8(address + 0x16),
  }),
};
const weatherNames = [
  "None",
  "Sunny clouds",
  "Sunny",
  "Rain",
  "Snow",
  "Rain thunderstorm",
  "Horizontal fog",
  "Volcanic ash",
  "Sandstorm",
  "Diagonal fog",
  "Underwater",
  "Shade",
  "Drought",
  "Downpour",
  "Underwater bubbles",
  "Abnormal",
] as const;

// Table dimensions are from the exact release bytes, not a modern compiler's ABI.
export const heartNSoulCatalog: Gen3CatalogConfig = {
  speciesNames: { address: 0x08744798, count: 462, stride: 11 },
  species: { address: 0x0893bd40, count: 490, stride: 40 },
  moveNames: { address: 0x08745b74, count: 368, stride: 13 },
  moves: { address: 0x08937998, count: 368, stride: 12 },
  abilities: { address: 0x086e1e90, count: 82, stride: 13 },
  abilityDescriptions: {
    address: 0x086e22bc,
    count: 82,
    stride: 4,
    firstId: 0,
    maxLength: 32,
  },
  moveDescriptions: {
    address: 0x089658c8,
    count: 367,
    stride: 4,
    firstId: 1,
    maxLength: 64,
  },
  types: { address: 0x086e152c, count: 19, stride: 7 },
  natures: { address: 0x08965f48, count: 25, stride: 4 },
  items: { address: 0x088fe910, count: 398, stride: 44 },
  experience: { address: 0x0893b0a0, count: 6, stride: 404 },
  categoryOffset: 9,
  priorityOffset: 7,
  itemDescriptionOffset: 20,
  itemDescriptionMaxLength: 256,
  legacyStatsOffset: 26,
  legacyTypesOffset: 32,
  legacyAbilitiesOffset: 34,
  alternateTypesOffset: 36,
};
const recordFormat = {
  speciesLimit: 456,
  eggMask: 0x80000000,
  ivShift: 1,
  abilityOffset: 8,
  abilityShift: 0,
  hiddenNature: true,
} as const;

// GetTypeBySpecies/GetAbilityBySpecies compatibility sets in the pinned source.
const modernTypes = [
  24, 47, 55, 99, 154, 157, 160, 164, 192, 234, 278, 279, 312, 316, 367, 368,
  325, 421,
];
const fairyTypes = [
  39, 40, 35, 36, 122, 173, 174, 175, 176, 183, 184, 209, 210, 392, 393, 394,
  350, 355, 435, 446,
];
const legendaryAbilities = [144, 145, 146, 150, 243, 244, 245, 250, 249];
const modernShiny = [
  25, 26, 172, 134, 135, 136, 402, 214, 93, 94, 123, 282, 178, 46, 170, 171,
  145, 239, 83, 105, 231, 131, 72, 73,
];
const settings: MemoryLayout<PokemonRules> = {
  byteLength: 0x3db8,
  decode(memory, address) {
    const randomizationFlags = memory.u8(address + 0x3d9d);
    const typeFeatureFlags = memory.u8(address + 0x3da5);
    const modeFlags = memory.u8(address + 0x3da6);
    const shinyMode = memory.u8(address + 0x3da4) >>> 4;
    if (shinyMode > 4) throw new Error("Unknown HnS shiny chance setting");
    const typeOverrides: Record<number, "legacy" | "alternate"> = {};
    if (!(typeFeatureFlags & 32))
      for (const species of modernTypes) typeOverrides[species] = "legacy";
    if (!(typeFeatureFlags & 64))
      for (const species of fairyTypes) typeOverrides[species] = "legacy";
    if (typeFeatureFlags & 32 && typeFeatureFlags & 64)
      for (const species of [209, 210]) typeOverrides[species] = "alternate";
    const legacyAbilities =
      modeFlags & 4
        ? []
        : legendaryAbilities.map((species) => ({ species, slot: 0 }));
    if (!(typeFeatureFlags & 32))
      legacyAbilities.push({ species: 164, slot: 1 });
    return {
      legacyStats: !(typeFeatureFlags & 128),
      typeOverrides,
      legacyAbilities,
      typesUnavailable:
        randomizationFlags & 32
          ? "HnS randomized types are not resolved yet"
          : null,
      abilitiesUnavailable:
        randomizationFlags & 128
          ? "HnS randomized abilities are not resolved yet"
          : null,
      baseStatsUnavailable:
        memory.u8(address + 0x3da0) & 96
          ? "HnS base-stat equalizer is active; actual stats remain authoritative"
          : null,
      shinyThreshold: 8 << shinyMode,
      shinyRequiresPersonality: shinyMode !== 0,
      modernShinySpecies: memory.u8(address + 0x3da3) & 64 ? modernShiny : [],
    };
  },
};

/**
 * Heart & Soul 1.2.1 only, not the moving 1.2 branch or stock Emerald.
 * Source: Release-v1.2.1 include/pokemon.h and include/constants/species.h.
 * The widened Poké Ball bitfield moves Misc.isEgg to bit 31. Populated species
 * end at Kleavor (455); 456–461 are reserved and 462 is the egg sentinel.
 * Address/disassembly evidence and reproduction: docs/game-bridges.md.
 */
export const heartNSoul = definePreparedGame(({ rom, romHash }) => ({
  catalog: createGen3Catalog(rom, heartNSoulCatalog, romHash),
  artwork: createPokemonArtwork(rom, {
    romHash,
    speciesCount: 490,
    eggSpecies: 462,
    unownSpecies: 201,
    unownFormsStart: 463,
    iconTable: 0x08952b3c,
    iconPaletteIndices: 0x089532e4,
    iconPalettes: 0x089534d0,
    frontTable: 0x08732678,
    palettes: 0x0872a28c,
    shinyPalettes: 0x0872b1dc,
    modernShinyPalettes: 0x0872c12c,
    itemCount: 398,
    itemIconTable: 0x08907368,
    spindaSpecies: 308,
    deoxysSpecies: 410,
  }),
}))({
  id: "pokemon/heart-n-soul",
  profiles: [
    {
      romHash:
        "7c4f90c8b68b64d4639a37306d5302a11cd4d7c44005114ec2faa2b28b210c3d",
      source: {
        repository: "https://github.com/PokemonHnS-Development/pokemonHnS",
        revision: "4aa8fa894db4c890b3d1100abd6aab50f44c22b1",
        variant: "Release-v1.2.1",
        compiler:
          "Release compiler unpublished; source prescribes devkitARM r65",
        options: [
          "Release binary verified by disassembly; not a matched local rebuild",
        ],
      },
      symbols: {
        gPlayerPartyCount: ref(0x0201d9e9, u8),
        gPlayerParty: array(0x0201d9ec, gen3PartyRecord(recordFormat), 6),
        gEnemyParty: array(0x0201de9c, gen3PartyRecord(recordFormat), 6),
        rules: ref(0x03003740, pointer(settings)),
        storage: ref(
          0x03003748,
          pointer(
            pokemonStorageLayout(gen3BoxPokemon(recordFormat), {
              byteLength: 0x8d34,
              boxesOffset: 4,
              boxCount: 15,
              capacity: 30,
              namesOffset: 0x8ca4,
              nameLength: 9,
            }),
          ),
        ),
        battleActive: ref(0x030019ad, mainInBattle),
        gBattleTypeFlags: ref(0x020003a0, littleU32),
        gBattlersCount: ref(0x02001420, u8),
        gBattlerPartyIndexes: array(0x02001422, littleU16, 4),
        gBattleMons: array(
          0x0200143c,
          gen3BattlePokemon({
            speciesLimit: 456,
            moveLimit: 368,
            typeLimit: 19,
            abilityLimit: 82,
          }),
          4,
        ),
        gAbsentBattlerFlags: ref(0x020015c8, u8),
        mapContext: ref(0x03003740, pointer(mapContext)),
        mapHeader: ref(0x0200c220, mapHeader),
      },
    },
  ],
  bridges: {
    gameParty: detailedGameParty,
    gameStorage,
    gameBattle: detailedGameBattle,
    gameContext: {
      id: "gameContext",
      decode({ memory, symbols }) {
        const root = memory.read(symbols.mapContext);
        if (!root)
          return {
            status: "unavailable" as const,
            reason: "Map save data is not initialized",
          };
        const location = memory.read(root);
        const header = memory.read(symbols.mapHeader);
        const locationName =
          location.mapGroup === 0 && location.mapNumber === 11
            ? "Route 29"
            : null;
        const weatherName = weatherNames[header.weather] ?? null;
        const warnings: string[] = [];
        if (!locationName)
          warnings.push("This map id does not have a verified display name");
        if (!weatherName)
          warnings.push("This map weather id does not have a verified name");
        const inBattle = memory.read(symbols.battleActive);
        const fields: GameStatusField[] = [];
        if (inBattle) {
          const kind = battleKind(memory.read(symbols.gBattleTypeFlags));
          fields.push(
            {
              id: "battle",
              label: "Battle",
              value: `${kind[0].toUpperCase()}${kind.slice(1)} battle`,
            },
            kind === "wild"
              ? { id: "opponent", label: "Opponent", value: "Wild Pokémon" }
              : {
                  id: "opponent",
                  label: "Opponent",
                  value: null,
                  reason:
                    "Trainer/opponent identity is not verified for this build",
                },
            {
              id: "weather",
              label: "Weather",
              value: null,
              reason: "Live battle weather is not verified for this build",
            },
          );
        } else {
          fields.push(
            weatherName === null
              ? {
                  id: "weather",
                  label: "Map weather",
                  value: null,
                  reason: "Unknown map weather ID",
                }
              : { id: "weather", label: "Map weather", value: weatherName },
          );
        }
        const arena: GameArenaStatus = {
          title:
            locationName ?? `Map ${location.mapGroup}:${location.mapNumber}`,
          fields,
        };
        return {
          status: "ready" as const,
          data: {
            arena,
            location: {
              mapGroup: location.mapGroup,
              mapNumber: location.mapNumber,
              x: location.x,
              y: location.y,
              regionMapSection: header.regionMapSection,
              name: locationName,
            },
            /** Configured map weather, not the unverified live weather effect. */
            weather: {
              id: header.weather,
              name: weatherName,
            },
            warnings,
          },
        };
      },
    },
  },
});
