import type {
  DecodeResult,
  MemoryLayout,
  MemoryReader,
} from "@evenfall/gba/bridge";
import type {
  BoxPokemon,
  PartyRecord,
  PokemonBattleStatStages,
  PokemonStats,
} from "./model";
import { decodePokemonText } from "./text";

export interface Gen3RecordFormat {
  readonly speciesLimit: number;
  readonly eggMask: number;
  readonly ivShift: 0 | 1;
  readonly abilityOffset: 4 | 8;
  readonly abilityShift: number;
  readonly hiddenNature: boolean;
  /**
   * False when the game stores records in the clear, with no checksum and
   * the substructures in their declared order (CFRU). Defaults to true.
   */
  readonly encrypted?: boolean;
  /**
   * The ability bit selects a hidden ability, and the personality's low bit
   * picks between the first two (CFRU): the slot is `2 * bit + (personality & 1)`.
   */
  readonly personalityAbility?: boolean;
}

export interface Gen3BattlePokemon {
  readonly species: number;
  readonly stats: PokemonStats;
  readonly moveIds: readonly number[];
  readonly pp: readonly number[];
  readonly ivs: PokemonStats;
  readonly isEgg: boolean;
  readonly abilitySlot: number;
  readonly statStages: PokemonBattleStatStages;
  readonly ability: number;
  readonly typeIds: readonly number[];
  readonly hp: number;
  readonly level: number;
  readonly friendship: number;
  readonly maxHp: number;
  readonly heldItem: number;
  readonly ppBonuses: number;
  readonly experience: number;
  readonly personality: number;
  readonly status: number;
  readonly volatileStatus: number;
  readonly trainerId: number;
}

/** Shared Gen III live battle record; addresses remain build-profile data. */
export function gen3BattlePokemon(format: {
  readonly speciesLimit: number;
  readonly moveLimit: number;
  readonly typeLimit: number;
  readonly abilityLimit: number;
  /**
   * Byte 0x18 holds a third type instead of the unused HP stage (CFRU), and
   * this type ID means it is empty.
   */
  readonly thirdTypeNone?: number;
  /** As in `Gen3RecordFormat`: bit 31 of the IV word selects a hidden ability. */
  readonly personalityAbility?: boolean;
}): MemoryLayout<DecodeResult<Gen3BattlePokemon>> {
  const thirdType = format.thirdTypeNone !== undefined;
  return {
    byteLength: 0x58,
    decode(memory, address) {
      const species = memory.u16(address);
      const moveIds = [0, 2, 4, 6].map((offset) =>
        memory.u16(address + 0x0c + offset),
      );
      const hp = memory.u16(address + 0x28);
      const level = memory.u8(address + 0x2a);
      const maxHp = memory.u16(address + 0x2c);
      const rawStages = memory.bytes(address + 0x18, 8);
      const ability = memory.u8(address + 0x20);
      const typeIds = [memory.u8(address + 0x21), memory.u8(address + 0x22)];
      if (thirdType && rawStages[0] !== format.thirdTypeNone)
        typeIds.push(rawStages[0]);
      if (
        !species ||
        species >= format.speciesLimit ||
        moveIds.some((move) => move >= format.moveLimit) ||
        typeIds.some((type) => type >= format.typeLimit) ||
        ability >= format.abilityLimit ||
        level < 1 ||
        level > 100 ||
        !maxHp ||
        hp > maxHp ||
        rawStages.subarray(thirdType ? 1 : 0).some((stage) => stage > 12)
      )
        return unavailable("Battle Pokémon is inconsistent or being updated");
      const ivWord = memory.u32(address + 0x14);
      const personality = memory.u32(address + 0x48);
      const stage = (index: number) => rawStages[index] - 6;
      return {
        status: "ready",
        data: {
          species,
          stats: {
            hp: maxHp,
            attack: memory.u16(address + 2),
            defense: memory.u16(address + 4),
            speed: memory.u16(address + 6),
            specialAttack: memory.u16(address + 8),
            specialDefense: memory.u16(address + 10),
          },
          moveIds,
          pp: [...memory.bytes(address + 0x24, 4)],
          ivs: statsFrom((index) => (ivWord >>> (index * 5)) & 31),
          isEgg: Boolean(ivWord & (1 << 30)),
          abilitySlot: format.personalityAbility
            ? 2 * (ivWord >>> 31) + (personality & 1)
            : ivWord >>> 31,
          statStages: {
            attack: stage(1),
            defense: stage(2),
            speed: stage(3),
            specialAttack: stage(4),
            specialDefense: stage(5),
            accuracy: stage(6),
            evasion: stage(7),
          },
          ability,
          typeIds,
          hp,
          level,
          friendship: memory.u8(address + 0x2b),
          maxHp,
          heldItem: memory.u16(address + 0x2e),
          ppBonuses: memory.u8(address + 0x3b),
          experience: memory.u32(address + 0x44),
          personality,
          status: memory.u32(address + 0x4c),
          volatileStatus: memory.u32(address + 0x50),
          trainerId: memory.u32(address + 0x54),
        },
      };
    },
  };
}

/** A record's header fields and its four 12-byte substructures in the clear. */
export interface DecryptedRecord {
  readonly personality: number;
  readonly trainerId: number;
  readonly flags: number;
  readonly language: number;
  readonly nickname: Uint8Array;
  readonly trainerName: Uint8Array;
  readonly blocks: readonly [DataView, DataView, DataView, DataView];
}

export function gen3BoxPokemon(
  format: Gen3RecordFormat,
): MemoryLayout<DecodeResult<BoxPokemon>> {
  return recordLayout(format, 80, false);
}

export function gen3PartyRecord(
  format: Gen3RecordFormat,
): MemoryLayout<DecodeResult<PartyRecord>> {
  return recordLayout(format, 100, true);
}

function recordLayout(
  format: Gen3RecordFormat,
  byteLength: 80 | 100,
  party: false,
): MemoryLayout<DecodeResult<BoxPokemon>>;
function recordLayout(
  format: Gen3RecordFormat,
  byteLength: 80 | 100,
  party: true,
): MemoryLayout<DecodeResult<PartyRecord>>;
function recordLayout(
  format: Gen3RecordFormat,
  byteLength: 80 | 100,
  party: boolean,
): MemoryLayout<DecodeResult<BoxPokemon | PartyRecord>> {
  return {
    byteLength,
    decode(memory, address) {
      const decrypted =
        format.encrypted === false
          ? plainGen3Record(memory, address)
          : decryptGen3Record(memory, address);
      if (decrypted.status === "unavailable") return decrypted;
      const decoded = decodeGen3Box(decrypted.data, format);
      if (decoded.status === "unavailable" || !party) return decoded;

      const hp = memory.u16(address + 0x56);
      const maxHp = memory.u16(address + 0x58);
      const level = memory.u8(address + 0x54);
      if (level < 1 || level > 100 || !maxHp || hp > maxHp)
        return unavailable("Party record is inconsistent or being updated");
      return {
        status: "ready",
        data: {
          ...decoded.data,
          level,
          hp,
          maxHp,
          status: memory.u32(address + 0x50),
          calculatedStats: {
            hp: maxHp,
            attack: memory.u16(address + 0x5a),
            defense: memory.u16(address + 0x5c),
            speed: memory.u16(address + 0x5e),
            specialAttack: memory.u16(address + 0x60),
            specialDefense: memory.u16(address + 0x62),
          },
        },
      };
    },
  };
}

/** The box fields of a record whose substructures are in the clear. */
export function decodeGen3Box(
  record: DecryptedRecord,
  format: Gen3RecordFormat,
): DecodeResult<BoxPokemon> {
  const [growth, attacks, evBlock, misc] = record.blocks;
  const species = growth.getUint16(0, true);
  const isEgg = Boolean(misc.getUint32(4, true) & format.eggMask);
  if (
    !species ||
    species >= format.speciesLimit ||
    isEgg !== Boolean(record.flags & 4)
  )
    return unavailable("Pokémon record is inconsistent or being updated");

  const rawNature = growth.getUint8(10) & 31;
  // HnS reserves 26 for personality nature. The other out-of-range values are
  // corrupt/unverified, rather than aliases for a valid nature.
  if (format.hiddenNature && rawNature !== 26 && rawNature > 24)
    return unavailable("Hidden nature is outside its verified range");

  const nickname = decodePokemonText(record.nickname, record.language);
  const trainerName = decodePokemonText(record.trainerName, record.language);
  const ivWord = misc.getUint32(4, true);
  const ivs = statsFrom(
    (index) => (ivWord >>> (format.ivShift + index * 5)) & 31,
  );
  const evs = statsFrom((index) => evBlock.getUint8(index));
  const abilityBit =
    (misc.getUint32(format.abilityOffset, true) >>> format.abilityShift) & 1;
  return {
    status: "ready",
    data: {
      personality: record.personality,
      trainerId: record.trainerId,
      species,
      nickname: nickname.text,
      trainerName: trainerName.text,
      language: record.language,
      textWarnings: [...nickname.warnings, ...trainerName.warnings],
      heldItem: growth.getUint16(2, true),
      experience: growth.getUint32(4, true),
      friendship: growth.getUint8(9),
      hiddenNature: format.hiddenNature && rawNature !== 26 ? rawNature : null,
      moveIds: [0, 2, 4, 6].map((offset) => attacks.getUint16(offset, true)),
      pp: [8, 9, 10, 11].map((offset) => attacks.getUint8(offset)),
      ppBonuses: growth.getUint8(8),
      ivs,
      evs,
      isEgg,
      abilitySlot: format.personalityAbility
        ? 2 * abilityBit + (record.personality & 1)
        : abilityBit,
    },
  };
}

function statsFrom(read: (index: number) => number): PokemonStats {
  return {
    hp: read(0),
    attack: read(1),
    defense: read(2),
    speed: read(3),
    specialAttack: read(4),
    specialDefense: read(5),
  };
}

/** Lowest common codec used by the legacy basic party projection. */
export function decryptGen3Record(
  memory: MemoryReader,
  address: number,
): DecodeResult<DecryptedRecord> {
  const flags = memory.u8(address + 0x13);
  if (!(flags & 2) || flags & 1) return unavailable("Empty or bad-egg record");
  const personality = memory.u32(address);
  const trainerId = memory.u32(address + 4);
  const key = personality ^ trainerId;
  const plain = new DataView(new ArrayBuffer(48));
  let checksum = 0;
  for (let offset = 0; offset < 48; offset += 4) {
    const word = (memory.u32(address + 0x20 + offset) ^ key) >>> 0;
    plain.setUint32(offset, word, true);
    checksum = (checksum + (word & 0xffff) + (word >>> 16)) & 0xffff;
  }
  if (checksum !== memory.u16(address + 0x1c))
    return unavailable("Pokémon checksum mismatch");

  const remaining = [0, 1, 2, 3];
  const positions: number[] = [];
  let rank = personality % 24;
  for (const [slot, divisor] of [6, 2, 1, 1].entries()) {
    const selected = Math.floor(rank / divisor);
    positions[remaining.splice(selected, 1)[0]] = slot * 12;
    rank %= divisor;
  }
  const block = (kind: number) =>
    new DataView(plain.buffer, positions[kind], 12);
  return {
    status: "ready",
    data: {
      personality,
      trainerId,
      flags,
      language: memory.u8(address + 0x12),
      nickname: memory.bytes(address + 8, 10),
      trainerName: memory.bytes(address + 0x14, 7),
      blocks: [block(0), block(1), block(2), block(3)],
    },
  };
}

/**
 * A record stored in the clear: CFRU removes the XOR and the substructure
 * shuffle, and reuses the checksum field, so nothing checks the data.
 */
function plainGen3Record(
  memory: MemoryReader,
  address: number,
): DecodeResult<DecryptedRecord> {
  const flags = memory.u8(address + 0x13);
  if (!(flags & 2) || flags & 1) return unavailable("Empty or bad-egg record");
  const plain = memory.bytes(address + 0x20, 48);
  const block = (kind: number) =>
    new DataView(plain.buffer, plain.byteOffset + kind * 12, 12);
  return {
    status: "ready",
    data: {
      personality: memory.u32(address),
      trainerId: memory.u32(address + 4),
      flags,
      language: memory.u8(address + 0x12),
      nickname: memory.bytes(address + 8, 10),
      trainerName: memory.bytes(address + 0x14, 7),
      blocks: [block(0), block(1), block(2), block(3)],
    },
  };
}

function unavailable(reason: string): DecodeResult<never> {
  return { status: "unavailable", reason };
}
