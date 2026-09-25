import type { MemoryReader } from "@evenfall/gba/bridge";
import type { PokemonCatalog, SpeciesMetadata, MoveMetadata } from "./catalog";
import { statKeys, type PokemonStats, type DescribedValue } from "./model";
import { decodePokemonText } from "./text";

/** Addresses/counts vary by exact build; the 2.65 table ABI is shared. */
export interface ReduxCatalogConfig {
  readonly species: number;
  readonly speciesNames: number;
  readonly speciesCount: number;
  readonly moves: number;
  readonly moveNames: number;
  readonly moveDescriptions: number;
  readonly moveCount: number;
  readonly abilities: number;
  readonly abilityCount: number;
  readonly items: number;
  readonly itemCount: number;
  readonly types: number;
  readonly natures: number;
  readonly experience: number;
}
export interface ReduxSpecies extends SpeciesMetadata {
  readonly innates: readonly number[];
}
export interface ReduxCatalog extends PokemonCatalog {
  species(id: number): ReduxSpecies;
  abilityFlags(id: number): {
    readonly unsuppressable: boolean;
    readonly blocksSuppression: boolean;
  };
  holdEffect(id: number): number;
}

export function createReduxCatalog(
  rom: MemoryReader,
  config: ReduxCatalogConfig,
  romHash?: string,
): ReduxCatalog {
  const entry = (base: number, id: number, count: number, stride: number) => {
    if (!Number.isInteger(id) || id < 0 || id >= count)
      throw new RangeError(
        `Invalid Redux catalog ID ${id} at 0x${base.toString(16)}`,
      );
    return base + id * stride;
  };
  function text(address: number, limit = 1024) {
    if (address < 0x08000000 || address >= 0x0a000000)
      throw new Error("Invalid Redux ROM text pointer");
    const bytes: number[] = [];
    for (let i = 0; i < limit; i++) {
      const byte = rom.u8(address + i);
      bytes.push(byte);
      if (byte === 255) return decodePokemonText(Uint8Array.from(bytes));
    }
    return { text: "", warnings: ["Unterminated Redux ROM text"] };
  }
  function name(address: number, limit = 32) {
    const result = text(address, limit);
    if (result.warnings.length) throw new Error(result.warnings.join("; "));
    return result.text;
  }
  function described(
    id: number,
    nameAddress: number,
    descriptionAddress: number,
  ): DescribedValue {
    const result = text(descriptionAddress);
    return {
      id,
      name: name(nameAddress),
      description: result.warnings.length ? null : result.text,
      warnings: result.warnings,
    };
  }
  const speciesCache = new Map<number, ReduxSpecies>();
  const movesCache = new Map<number, MoveMetadata>();
  const abilityCache = new Map<number, DescribedValue>();
  const catalog: ReduxCatalog = {
    species(id) {
      const cached = speciesCache.get(id);
      if (cached) return cached;
      const address = entry(config.species, id, config.speciesCount, 44);
      const value: ReduxSpecies = {
        id,
        name: name(rom.u32(config.speciesNames + id * 4)),
        base: Object.fromEntries(
          statKeys.map((key, i) => [key, rom.u8(address + i)]),
        ) as PokemonStats,
        legacyBase: null,
        legacyTypes: null,
        alternateTypes: null,
        legacyAbilities: null,
        types: [rom.u8(address + 6), rom.u8(address + 7)],
        abilities: [0, 1, 2].map((i) => rom.u16(address + 24 + i * 2)),
        innates: [0, 1, 2].map((i) => rom.u16(address + 34 + i * 2)),
        genderRatio: rom.u8(address + 18),
        growth: rom.u8(address + 21),
      };
      speciesCache.set(id, Object.freeze(value));
      return value;
    },
    move(id) {
      const cached = movesCache.get(id);
      if (cached) return cached;
      const address = entry(config.moves, id, config.moveCount, 24);
      const category = rom.u8(address + 17);
      if (category > 2) throw new Error("Invalid Redux move category");
      const priority = rom.u8(address + 16);
      const splitFlag = (rom.u8(address + 19) >>> 1) & 7;
      const value: MoveMetadata = {
        ...(id
          ? described(
              id,
              config.moveNames + id * 13,
              rom.u32(config.moveDescriptions + (id - 1) * 4),
            )
          : { id, name: "—" }),
        type: catalog.type(rom.u8(address + 11)),
        category: splitFlag
          ? null
          : (["physical", "special", "status"] as const)[category],
        priority: priority > 127 ? priority - 256 : priority,
        power: rom.u8(address + 10),
        accuracy: rom.u8(address + 13),
        pp: rom.u8(address + 14),
      };
      movesCache.set(id, Object.freeze(value));
      return value;
    },
    type(id) {
      return { id, name: name(entry(config.types, id, 20, 7), 7) };
    },
    ability(id) {
      const cached = abilityCache.get(id);
      if (cached) return cached;
      const address = entry(config.abilities, id, config.abilityCount, 192);
      const value = Object.freeze(
        described(id, rom.u32(address), rom.u32(address + 8)),
      );
      abilityCache.set(id, value);
      return value;
    },
    abilityFlags(id) {
      // Release disassembly, not modern GCC's different bitfield packing (+0xb6).
      const flags = rom.u16(
        entry(config.abilities, id, config.abilityCount, 192) + 0xb8,
      );
      return {
        unsuppressable: Boolean(flags & 0x800),
        blocksSuppression: Boolean(flags & 0x2000),
      };
    },
    holdEffect(id) {
      return rom.u8(entry(config.items, id, config.itemCount, 40) + 13);
    },
    item(id) {
      const address = entry(config.items, id, config.itemCount, 40);
      return {
        ...described(id, rom.u32(address), rom.u32(address + 16)),
        artwork: romHash ? { kind: "item", romHash, item: id } : undefined,
      };
    },
    nature(id) {
      return { id, name: name(rom.u32(entry(config.natures, id, 25, 4)), 16) };
    },
    experience(growth, level) {
      if (!Number.isInteger(level) || level < 0 || level > 100)
        throw new Error("Invalid Redux experience level");
      return rom.u32(entry(config.experience, growth, 8, 404) + level * 4);
    },
  };
  return Object.freeze(catalog);
}
