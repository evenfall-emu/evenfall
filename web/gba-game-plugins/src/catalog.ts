import type { MemoryReader } from "@evenfall/gba/bridge";
import {
  statKeys,
  type NamedValue,
  type DescribedValue,
  type PokemonItem,
  type PokemonStats,
  type MoveCategory,
} from "./model";
import { decodePokemonText } from "./text";

export interface RomTable {
  readonly address: number;
  readonly count: number;
  readonly stride: number;
}
/** A ROM pointer table whose first entry need not correspond to ID zero. */
export interface DescriptionTable extends RomTable {
  readonly firstId: number;
  readonly maxLength: number;
}
export interface SpeciesMetadata extends NamedValue {
  readonly base: PokemonStats;
  readonly legacyBase: PokemonStats | null;
  readonly types: readonly number[];
  readonly legacyTypes: readonly number[] | null;
  readonly alternateTypes: readonly number[] | null;
  readonly abilities: readonly number[];
  readonly legacyAbilities: readonly number[] | null;
  readonly genderRatio: number;
  readonly growth: number;
}
export interface MoveMetadata extends DescribedValue {
  readonly type: NamedValue;
  readonly category: MoveCategory | null;
  readonly priority?: number | null;
  readonly pp: number;
  readonly power: number;
  readonly accuracy: number;
}
/** Shared semantic catalog; another Pokémon generation can supply another reader. */
export interface PokemonCatalog {
  species(id: number): SpeciesMetadata;
  move(id: number): MoveMetadata;
  type(id: number): NamedValue;
  ability(id: number): DescribedValue;
  item(id: number): PokemonItem;
  nature(id: number): NamedValue;
  experience(growth: number, level: number): number;
}

/** Gen III table layout facts; addresses, dimensions and fork extensions are data. */
export interface Gen3CatalogConfig {
  readonly speciesNames: RomTable;
  readonly species: RomTable;
  readonly moveNames: RomTable;
  readonly moves: RomTable;
  readonly abilities: RomTable;
  readonly moveDescriptions?: DescriptionTable;
  readonly abilityDescriptions?: DescriptionTable;
  readonly types: RomTable;
  readonly natures: RomTable;
  readonly items: RomTable;
  readonly experience: RomTable;
  readonly categoryOffset: number | null;
  readonly priorityOffset?: number;
  readonly itemDescriptionOffset?: number;
  readonly itemDescriptionMaxLength?: number;
  /**
   * An item name whose fourth byte is 0x08 or 0x09 is a pointer to the full
   * name instead, for names longer than the record's 14 bytes (CFRU).
   */
  readonly itemNamePointers?: boolean;
  readonly legacyStatsOffset?: number;
  readonly legacyTypesOffset?: number;
  readonly alternateTypesOffset?: number;
  readonly legacyAbilitiesOffset?: number;
}

export function createGen3Catalog(
  rom: MemoryReader,
  config: Gen3CatalogConfig,
  romHash?: string,
): PokemonCatalog {
  function entry(table: RomTable, id: number): number {
    if (!Number.isInteger(id) || id < 0 || id >= table.count)
      throw new RangeError(
        `Catalog id ${id} outside table at 0x${table.address.toString(16)}`,
      );
    return table.address + id * table.stride;
  }
  function text(address: number, length: number): string {
    const result = decodePokemonText(rom.bytes(address, length));
    if (result.warnings.length)
      throw new Error(
        `Catalog text at 0x${address.toString(16)}: ${result.warnings.join(", ")}`,
      );
    return result.text;
  }
  function named(
    table: RomTable,
    id: number,
    length = table.stride,
  ): NamedValue {
    return Object.freeze({ id, name: text(entry(table, id), length) });
  }
  function itemName(id: number): NamedValue {
    const address = entry(config.items, id);
    const high = rom.u8(address + 3);
    if (config.itemNamePointers && (high === 0x08 || high === 0x09))
      return Object.freeze({ id, name: text(rom.u32(address), 32) });
    return named(config.items, id, 14);
  }
  function terminatedText(address: number, maxLength: number) {
    const bytes: number[] = [];
    for (let offset = 0; offset < maxLength; offset++) {
      const byte = rom.u8(address + offset);
      bytes.push(byte);
      if (byte === 0xff) return decodePokemonText(Uint8Array.from(bytes));
    }
    return {
      text: "",
      warnings: [`Text exceeds verified ${maxLength}-byte bound`],
    };
  }
  function description(table: DescriptionTable | undefined, id: number) {
    if (!table || id < table.firstId)
      return {
        description: null,
        warnings: ["No verified description for this entry"],
      };
    const result = terminatedText(
      rom.u32(entry(table, id - table.firstId)),
      table.maxLength,
    );
    return {
      description: result.warnings.length ? null : result.text,
      warnings: result.warnings,
    };
  }
  const stats = (address: number): PokemonStats =>
    Object.freeze(
      Object.fromEntries(statKeys.map((key, i) => [key, rom.u8(address + i)])),
    ) as PokemonStats;
  const pair = (address: number): readonly number[] =>
    Object.freeze([rom.u8(address), rom.u8(address + 1)]);
  // At most one entry per declared table ID; immutable data, no per-sample ROM copying.
  const speciesCache = new Map<number, SpeciesMetadata>();
  const movesCache = new Map<number, MoveMetadata>();
  const catalog: PokemonCatalog = {
    species(id) {
      const cached = speciesCache.get(id);
      if (cached) return cached;
      const address = entry(config.species, id);
      const value = Object.freeze({
        ...named(config.speciesNames, id),
        base: stats(address),
        legacyBase:
          config.legacyStatsOffset === undefined
            ? null
            : stats(address + config.legacyStatsOffset),
        types: pair(address + 6),
        legacyTypes:
          config.legacyTypesOffset === undefined
            ? null
            : pair(address + config.legacyTypesOffset),
        alternateTypes:
          config.alternateTypesOffset === undefined
            ? null
            : pair(address + config.alternateTypesOffset),
        abilities: pair(address + 22),
        legacyAbilities:
          config.legacyAbilitiesOffset === undefined
            ? null
            : pair(address + config.legacyAbilitiesOffset),
        genderRatio: rom.u8(address + 16),
        growth: rom.u8(address + 19),
      });
      speciesCache.set(id, value);
      return value;
    },
    move(id) {
      const cached = movesCache.get(id);
      if (cached) return cached;
      const address = entry(config.moves, id);
      const categoryId =
        config.categoryOffset === null
          ? null
          : rom.u8(address + config.categoryOffset);
      if (categoryId !== null && categoryId > 2)
        throw new Error(`Invalid move category ${categoryId}`);
      const value: MoveMetadata = Object.freeze({
        ...named(config.moveNames, id),
        ...description(config.moveDescriptions, id),
        type: catalog.type(rom.u8(address + 2)),
        category:
          categoryId === null
            ? null
            : (["physical", "special", "status"] as const)[categoryId],
        priority:
          config.priorityOffset === undefined
            ? null
            : signedByte(rom.u8(address + config.priorityOffset)),
        pp: rom.u8(address + 4),
        power: rom.u8(address + 1),
        accuracy: rom.u8(address + 3),
      });
      movesCache.set(id, value);
      return value;
    },
    type: (id) => named(config.types, id),
    ability: (id) =>
      Object.freeze({
        ...named(config.abilities, id),
        ...description(config.abilityDescriptions, id),
      }),
    item(id) {
      const address = entry(config.items, id);
      if (config.itemDescriptionOffset === undefined)
        return Object.freeze({
          ...itemName(id),
          artwork: romHash
            ? { kind: "item" as const, romHash, item: id }
            : undefined,
          description: null,
          warnings: ["Item descriptions are not verified for this game"],
        });
      const pointer = rom.u32(address + config.itemDescriptionOffset);
      const result = terminatedText(
        pointer,
        config.itemDescriptionMaxLength ?? 256,
      );
      return Object.freeze({
        ...itemName(id),
        artwork: romHash
          ? { kind: "item" as const, romHash, item: id }
          : undefined,
        description: result.warnings.length ? null : result.text,
        warnings: result.warnings,
      });
    },
    nature: (id) =>
      Object.freeze({ id, name: text(rom.u32(entry(config.natures, id)), 16) }),
    experience(growth, level) {
      if (!Number.isInteger(level) || level < 0 || level > 100)
        throw new RangeError("Invalid experience level");
      return rom.u32(entry(config.experience, growth) + level * 4);
    },
  };
  return Object.freeze(catalog);
}

function signedByte(value: number): number {
  return value < 0x80 ? value : value - 0x100;
}
