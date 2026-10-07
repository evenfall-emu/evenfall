// Complete Fire Red Upgrade (CFRU), the engine Pokémon Unbound and other
// FireRed hacks are built on. CFRU is compiled C inserted into a FireRed 1.0
// (BPRE) ROM, not a decompilation build: FireRed's globals keep their RAM
// addresses, and the tables CFRU expands move to free space, which its
// locators find through FireRed's own code. Evidence: docs/profiles.md.
import {
  createGame,
  defineFamily,
  isWorkRam,
  locateWith,
  pointer,
  ram,
  ramArray,
  ref,
  u8,
  type DecodeResult,
  type GameStatusField,
  type ManifestOf,
  type MemoryLayout,
  type MemoryReader,
  type RomContext,
  type View,
  type ViewContext,
} from "@evenfall/gba/bridge";
import { createPokemonArtwork, type PokemonArtworkConfig } from "./artwork";
import {
  battleKind,
  detailedGameBattle,
  detailedGameParty,
  gameStorage,
  type PokemonRules,
  type StoredRecords,
} from "./bridges";
import {
  createGen3Catalog,
  type Gen3CatalogConfig,
  type PokemonCatalog,
  type SpeciesMetadata,
} from "./catalog";
import {
  gen3Probes,
  mediaView,
  type Gen3Views,
  type PokemonContextView,
} from "./gen3";
import {
  decodeGen3Box,
  gen3BattlePokemon,
  gen3PartyRecord,
  type Gen3RecordFormat,
} from "./gen3-record";
import { locatedFrom, locators, markers, romLocators } from "./cfru.locators";
import type { BoxPokemon, PokemonBattleView } from "./model";
import { decodePokemonText } from "./text";

/** Per-build ROM data the CFRU resources read (part of the manifest). */
export interface CfruRom {
  readonly catalog: Gen3CatalogConfig;
  readonly artwork: Omit<PokemonArtworkConfig, "romHash">;
  /** The expanded PC's tables of box and box-name pointers into RAM. */
  readonly storage: {
    readonly boxes: number;
    readonly names: number;
    readonly boxCount: number;
  };
  /** FireRed's map section names, the first naming `firstSection`. */
  readonly mapNames: {
    readonly address: number;
    readonly firstSection: number;
    readonly count: number;
  };
  /** A Pokémon is shiny when its shiny value is below this. */
  readonly shinyThreshold: number;
}

/**
 * CFRU's party record: FireRed's 100 bytes in the clear, a Poké Ball byte in
 * the growth substructure, and the hidden-ability flag where FireRed keeps
 * the ability bit. The species bound is the reviewed build's species count.
 */
export const cfruRecordFormat: Gen3RecordFormat = {
  speciesLimit: 1294,
  eggMask: 1 << 30,
  ivShift: 0,
  abilityOffset: 4,
  abilityShift: 31,
  hiddenNature: false,
  encrypted: false,
  personalityAbility: true,
};
/** `struct BattlePokemon`: a third type at 0x18, 0x14 (TYPE_BLANK) when empty. */
export const cfruBattleFormat = {
  speciesLimit: 1294,
  moveLimit: 923,
  typeLimit: 24,
  abilityLimit: 255,
  thirdTypeNone: 0x14,
  personalityAbility: true,
} as const;
/** CFRU's BaseStats keep the hidden ability in FireRed's padding byte. */
const HIDDEN_ABILITY_OFFSET = 26;
const BOX_CAPACITY = 30;
const BOX_NAME_LENGTH = 9;

/**
 * CFRU's 58-byte PC record (`struct CompressedPokemon`, packed): the party
 * record's header, its growth substructure without the last byte, four
 * 10-bit moves packed into five bytes, the EVs and the first eight bytes of
 * Misc. It keeps no PP, so withdrawing restores full PP; PP reads as unknown.
 */
export function cfruBoxPokemon(
  format: Gen3RecordFormat,
): MemoryLayout<DecodeResult<BoxPokemon>> {
  return {
    byteLength: 58,
    decode(memory, address) {
      const flags = memory.u8(address + 0x13);
      if (!(flags & 2) || flags & 1)
        return unavailable("Empty or bad-egg record");
      const record = memory.bytes(address, 58);
      const parts = new Uint8Array(48);
      const view = new DataView(parts.buffer);
      parts.set(record.subarray(0x1c, 0x27), 0);
      let moves = 0;
      for (let i = 4; i >= 0; i--) moves = moves * 256 + record[0x27 + i];
      for (let slot = 0; slot < 4; slot++)
        view.setUint16(
          12 + slot * 2,
          Math.floor(moves / 1024 ** slot) % 1024,
          true,
        );
      parts.set(record.subarray(0x2c, 0x32), 24);
      parts.set(record.subarray(0x32, 0x3a), 36);
      const header = new DataView(record.buffer, record.byteOffset);
      const block = (kind: number) => new DataView(parts.buffer, kind * 12, 12);
      const decoded = decodeGen3Box(
        {
          personality: header.getUint32(0, true),
          trainerId: header.getUint32(4, true),
          flags,
          language: record[0x12],
          nickname: record.slice(8, 18),
          trainerName: record.slice(0x14, 0x1b),
          blocks: [block(0), block(1), block(2), block(3)],
        },
        format,
      );
      if (decoded.status !== "ready") return decoded;
      return {
        status: "ready",
        data: { ...decoded.data, pp: [null, null, null, null] },
      };
    },
  };
}

/**
 * The expanded PC: the current box from `gPokemonStoragePtr`, and each box's
 * records and name at the RAM addresses the ROM's tables give.
 */
function cfruStorage(
  boxes: readonly { readonly records: number; readonly name: number }[],
  record: MemoryLayout<DecodeResult<BoxPokemon>>,
): MemoryLayout<DecodeResult<StoredRecords>> {
  return {
    byteLength: 1,
    decode(memory, address) {
      const currentBox = memory.u8(address);
      if (currentBox >= boxes.length)
        return unavailable("Current box is outside storage capacity");
      const stored: StoredRecords["boxes"][number][] = [];
      for (const [box, { records, name }] of boxes.entries()) {
        const pokemon: StoredRecords["boxes"][number]["pokemon"][number][] = [];
        for (let slot = 0; slot < BOX_CAPACITY; slot++) {
          const target = records + slot * record.byteLength;
          if (!(memory.u8(target + 0x13) & 2)) continue;
          const result = memory.read({ address: target, layout: record });
          if (result.status !== "ready")
            return unavailable(
              `Box ${box + 1}, slot ${slot + 1}: ${result.reason}`,
            );
          pokemon.push({ slot, record: result.data });
        }
        stored.push({
          box,
          name: decodePokemonText(memory.bytes(name, BOX_NAME_LENGTH)).text,
          pokemon,
        });
      }
      return { status: "ready", data: { currentBox, boxes: stored } };
    },
  };
}

/** CFRU has no option flags the views apply; the shiny bound is ROM data. */
function cfruRules(shinyThreshold: number): MemoryLayout<PokemonRules> {
  return {
    byteLength: 1,
    decode: () => ({
      legacyStats: false,
      typeOverrides: {},
      legacyAbilities: [],
      typesUnavailable: null,
      abilitiesUnavailable: null,
      baseStatsUnavailable: null,
      shinyThreshold,
      shinyRequiresPersonality: false,
      modernShinySpecies: [],
    }),
  };
}

/**
 * The Gen III catalog with CFRU's long item names and its ability rule
 * (`GetMonAbility`): slots 0 and
 * 1 follow the personality's low bit, an empty second ability falls back to
 * the first, and slots 2 and 3 are the hidden ability when the species has
 * one, else the same as 0 and 1.
 */
function cfruCatalog(
  rom: MemoryReader,
  config: Gen3CatalogConfig,
  romHash: string,
): PokemonCatalog {
  const catalog = createGen3Catalog(
    rom,
    { ...config, itemNamePointers: true },
    romHash,
  );
  const species = new Map<number, SpeciesMetadata>();
  return Object.freeze({
    ...catalog,
    species(id: number) {
      const cached = species.get(id);
      if (cached) return cached;
      const base = catalog.species(id);
      const hidden = rom.u8(
        config.species.address +
          id * config.species.stride +
          HIDDEN_ABILITY_OFFSET,
      );
      const [first, second] = base.abilities;
      const normal = [first, second || first];
      const value = Object.freeze({
        ...base,
        abilities: Object.freeze([
          ...normal,
          ...normal.map((ability) => hidden || ability),
        ]),
      });
      species.set(id, value);
      return value;
    },
  });
}

const u16: MemoryLayout<number> = { byteLength: 2, decode: (m, a) => m.u16(a) };
const u32: MemoryLayout<number> = { byteLength: 4, decode: (m, a) => m.u32(a) };
/** gMain.inBattle, bit 1 of byte 0x439. */
const mainInBattle: MemoryLayout<boolean> = {
  byteLength: 1,
  decode: (memory, address) => Boolean(memory.u8(address) & 2),
};
/** SaveBlock1's position and location: x, y, then map group and number. */
const mapLocation: MemoryLayout<{
  readonly x: number;
  readonly y: number;
  readonly mapGroup: number;
  readonly mapNumber: number;
}> = {
  byteLength: 6,
  decode: (memory, address) => ({
    x: memory.u16(address),
    y: memory.u16(address + 2),
    mapGroup: memory.u8(address + 4),
    mapNumber: memory.u8(address + 5),
  }),
};
/** gMapHeader.regionMapSectionId. */
const mapSection: MemoryLayout<number> = {
  byteLength: 0x1c,
  decode: (memory, address) => memory.u8(address + 0x14),
};

const party = gen3PartyRecord(cfruRecordFormat);
/** FireRed's globals at the addresses CFRU keeps, named as in pokefirered. */
const cfruSources = locateWith(
  {
    gPlayerPartyCount: ram(u8),
    gPlayerParty: ramArray(party, 6),
    gEnemyParty: ramArray(party, 6),
    gSaveBlock1Ptr: ram(pointer(mapLocation)),
    /** Read only to wait for a loaded save; CFRU keeps no rules there. */
    gSaveBlock2Ptr: ram(pointer(u8)),
    /** The current box, at the start of PokemonStorage. */
    gPokemonStoragePtr: ram(pointer(u8)),
    battleActive: ram(mainInBattle),
    gBattleTypeFlags: ram(u32),
    gBattlersCount: ram(u8),
    gBattlerPartyIndexes: ramArray(u16, 4),
    gBattleMons: ramArray(gen3BattlePokemon(cfruBattleFormat), 4),
    gAbsentBattlerFlags: ram(u8),
    gMapHeader: ram(mapSection),
  },
  locators,
);
type CfruSources = typeof cfruSources;

function cfruResources({ rom, romHash, config }: RomContext<CfruRom>) {
  const boxes = Array.from({ length: config.storage.boxCount }, (_, box) => {
    const records = rom.u32(config.storage.boxes + box * 4);
    const name = rom.u32(config.storage.names + box * 4);
    if (!isWorkRam(records) || !isWorkRam(name))
      throw new RangeError(`PC box ${box + 1} is outside work RAM`);
    return { records, name };
  });
  const { mapNames } = config;
  return {
    catalog: cfruCatalog(rom, config.catalog, romHash),
    artwork: createPokemonArtwork(rom, { ...config.artwork, romHash }),
    rules: cfruRules(config.shinyThreshold),
    storage: cfruStorage(boxes, cfruBoxPokemon(cfruRecordFormat)),
    /** The name GetMapName gives a map section, or null for none. */
    mapName(section: number): string | null {
      const index = section - mapNames.firstSection;
      if (index < 0 || index >= mapNames.count) return null;
      const name = decodePokemonText(
        rom.bytes(rom.u32(mapNames.address + index * 4), 32),
      );
      return name.warnings.length ? null : name.text;
    },
  };
}
type CfruResources = ReturnType<typeof cfruResources>;

/** The shared views' symbols, with the rules and PC layouts this ROM defines. */
function shared({
  memory,
  symbols,
  sample,
  resources,
}: ViewContext<CfruSources, CfruResources>) {
  return {
    memory,
    sample,
    resources,
    symbols: {
      ...symbols,
      rules: ref(symbols.gSaveBlock2Ptr.address, pointer(resources.rules)),
      storage: ref(
        symbols.gPokemonStoragePtr.address,
        pointer(resources.storage),
      ),
    },
  };
}

/** FireRed's battle type flags as CFRU defines them, by bit. */
const battleFlagNames = [
  "double",
  "link",
  "master",
  "trainer",
  "oak-tutorial",
  "link-established",
  "multi",
  "safari",
  "battle-tower",
  "old-man-tutorial",
  "roamer",
  "e-reader-trainer",
  "kyogre-groudon",
  "scripted-wild",
  "regi",
  "ghost",
  "poke-dude",
  "scripted-wild-2",
  "legendary",
  "trainer-tower",
  null,
  "two-opponents",
  "in-game-partner",
  "ring-challenge",
  "mock-battle",
  "battle-sands",
  "battle-circus",
  "benjamin-butterfree",
  "camomons",
  "mega-brawl",
  "dynamax",
  "shadow-warrior",
] as const;

const battleView: View<CfruSources, CfruResources, PokemonBattleView> = (
  context,
) => {
  const result = detailedGameBattle.decode(shared(context));
  if (result.status !== "ready" || !result.data.active) return result;
  const { typeFlags } = result.data;
  return {
    status: "ready",
    data: {
      ...result.data,
      types: battleFlagNames.filter(
        (name, bit): name is NonNullable<typeof name> =>
          name !== null && Boolean(typeFlags & (2 ** bit)),
      ),
    },
  };
};

/** The map section's name from the ROM, and the battle kind. */
const contextView: View<CfruSources, CfruResources, PokemonContextView> = ({
  memory,
  symbols,
  resources,
}) => {
  const saveRef = memory.read(symbols.gSaveBlock1Ptr);
  if (!saveRef) return unavailable("Map save data is not initialized");
  const location = memory.read(saveRef);
  const section = memory.read(symbols.gMapHeader);
  const name = resources.mapName(section);
  const fields: GameStatusField[] = [];
  if (memory.read(symbols.battleActive)) {
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
            reason: "Trainer identity is not read for CFRU builds",
          },
    );
  }
  return {
    status: "ready",
    data: {
      arena: {
        title: name ?? `Map ${location.mapGroup}:${location.mapNumber}`,
        fields,
      },
      location: { ...location, regionMapSection: section, name },
      warnings: name ? [] : ["This map section has no name in the ROM"],
    },
  };
};

/** CFRU on FireRed: the Gen III views over plain records and the expanded PC. */
export const cfruFamily = defineFamily<Gen3Views, CfruRom>()({
  id: "pokemon/cfru",
  sources: cfruSources,
  resources: cfruResources,
  views: {
    party: (context) => detailedGameParty.decode(shared(context)),
    storage: (context) => gameStorage.decode(shared(context)),
    battle: battleView,
    context: contextView,
    media: mediaView,
  },
  probes: gen3Probes,
  romLocators,
  // FireRed's game code alone would match vanilla FireRed and other hacks.
  detect: { gameCodes: ["BPRE"], markers },
  locatedFrom,
});

/** Pokémon Unbound 2.1.1.1, the release ROM. */
export const unbound2111: ManifestOf<typeof cfruFamily> = {
  family: "pokemon/cfru",
  romHash: "b4776b82a4c7915d0fadeaa27e013523f99dfd94",
  source: {
    repository: "https://github.com/Skeli789/Complete-Fire-Red-Upgrade",
    revision: "Unbound's own CFRU revision is unpublished",
    variant: "Pokémon Unbound 2.1.1.1",
    compiler: "Unpublished",
    options: [
      "Release binary verified against public CFRU sources and a battle state",
    ],
  },
  addresses: {
    gPlayerPartyCount: 0x02024029,
    gPlayerParty: 0x02024284,
    gEnemyParty: 0x0202402c,
    gSaveBlock1Ptr: 0x03005008,
    gSaveBlock2Ptr: 0x0300500c,
    gPokemonStoragePtr: 0x03005010,
    battleActive: 0x03003529,
    gBattleTypeFlags: 0x02022b4c,
    gBattlersCount: 0x02023bcc,
    gBattlerPartyIndexes: 0x02023bce,
    gBattleMons: 0x02023be4,
    gAbsentBattlerFlags: 0x02023d70,
    gMapHeader: 0x02036dfc,
  },
  rom: {
    catalog: {
      speciesNames: { address: 0x0966a98c, count: 1294, stride: 11 },
      species: { address: 0x099e0c9c, count: 1294, stride: 28 },
      moveNames: { address: 0x08a40a10, count: 923, stride: 13 },
      moves: { address: 0x08a769af, count: 923, stride: 12 },
      abilities: { address: 0x08a36398, count: 255, stride: 17 },
      abilityDescriptions: {
        address: 0x0896de04,
        count: 255,
        stride: 4,
        firstId: 0,
        maxLength: 48,
      },
      moveDescriptions: {
        address: 0x0899f194,
        count: 922,
        stride: 4,
        firstId: 1,
        maxLength: 128,
      },
      types: { address: 0x08a4ead4, count: 24, stride: 7 },
      natures: { address: 0x08463e60, count: 25, stride: 4 },
      items: { address: 0x08876200, count: 729, stride: 44 },
      experience: { address: 0x08a79560, count: 6, stride: 1024 },
      categoryOffset: 10,
      priorityOffset: 7,
      itemDescriptionOffset: 20,
      itemDescriptionMaxLength: 256,
    },
    artwork: {
      speciesCount: 1294,
      eggSpecies: 412,
      unownSpecies: 201,
      unownFormsStart: 413,
      iconTable: 0x09a217fc,
      iconPaletteIndices: 0x09a212ec,
      iconPalettes: 0x083d4038,
      frontTable: 0x09a1d5b4,
      palettes: 0x09a32bdc,
      shinyPalettes: 0x09a3f77c,
      modernShinyPalettes: 0x09a3f77c,
      itemCount: 729,
      itemIconTable: 0x0825a554,
      spindaSpecies: 308,
      deoxysSpecies: 410,
    },
    storage: { boxes: 0x08a6cb2c, names: 0x08a6cb90, boxCount: 25 },
    mapNames: { address: 0x083f1cac, firstSection: 0x58, count: 0x6d },
    shinyThreshold: 16,
  },
  provenance: { method: "reviewed" },
};

/**
 * Reviewed builds. Other CFRU hacks are recognized in the player from the
 * locators, with Unbound 2.1.1.1 as their template.
 */
export const fireRedUpgrade = createGame(cfruFamily, [unbound2111]);

function unavailable(reason: string): DecodeResult<never> {
  return { status: "unavailable", reason };
}
