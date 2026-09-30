import type { MemoryLayout, MemoryReader } from "@evenfall/gba/bridge";
import { decodePokemonText } from "./text";
import {
  statKeys,
  type BoxPokemon,
  type PartyRecord,
  type PokemonStats,
  type PokemonBattleStatStages,
} from "./model";

/** Elite Redux 2.65's packed, unencrypted ABI; not Emerald's encrypted records. */
export interface ReduxBox extends BoxPokemon {
  readonly shinyTier: number;
}
export interface ReduxParty extends ReduxBox, PartyRecord {}
export interface ReduxSettings {
  readonly difficulty: number;
  readonly evsEnabled: boolean;
  readonly randomizedAbilities: boolean;
  readonly randomizedInnates: boolean;
  readonly randomizedTypes: boolean;
}
export const reduxSettings: MemoryLayout<ReduxSettings> = {
  byteLength: 0xc8d,
  decode(memory, address) {
    const difficulty = memory.u16(address + 0xc88) >>> 12;
    if (difficulty > 3) throw new Error("Unknown Redux difficulty");
    const flags = memory.u16(address + 0xc8a);
    return {
      difficulty,
      evsEnabled: Boolean(memory.u8(address + 0xc8c) & 4),
      randomizedAbilities: Boolean(flags & 32),
      randomizedInnates: Boolean(flags & 8),
      randomizedTypes: Boolean(flags & 128),
    };
  },
};

export const reduxBox: MemoryLayout<ReduxBox> = {
  byteLength: 0x34,
  decode(memory, address) {
    const first = memory.u32(address + 8);
    const second = memory.u32(address + 12);
    const third = memory.u32(address + 16);
    const flags = memory.u32(address + 20);
    const species = third & 0xffff;
    const nature = (flags >>> 10) & 31;
    if (!species || nature > 24)
      throw new Error("Invalid Redux Pokémon identity/nature");
    const language = (flags >>> 16) & 7;
    const nickname = decodePokemonText(
      memory.bytes(address + 0x26, 12),
      language,
    );
    const trainerName = decodePokemonText(
      memory.bytes(address + 0x1f, 7),
      language,
    );
    const moveIds = [
      first & 0x7ff,
      second & 0x7ff,
      (second >>> 11) & 0x7ff,
      (third >>> 16) & 0x7ff,
    ];
    return {
      personality: memory.u32(address),
      trainerId: memory.u32(address + 4),
      species,
      nickname: nickname.text,
      trainerName: trainerName.text,
      language,
      textWarnings: [...nickname.warnings, ...trainerName.warnings],
      heldItem: flags & 0x3ff,
      experience: first >>> 11,
      friendship: (second >>> 22) & 255,
      hiddenNature: nature,
      moveIds,
      pp: [null, null, null, null],
      ppBonuses: 255,
      ivs: {
        hp: 31,
        attack: 31,
        defense: 31,
        speed: memory.u8(address + 0x32) & 32 ? 0 : 31,
        specialAttack: 31,
        specialDefense: 31,
      },
      evs: readStats(memory, address + 0x18, 1),
      isEgg: Boolean(flags & 0x8000),
      abilitySlot: flags >>> 30,
      shinyTier: (flags >>> 26) & 3,
    };
  },
};
export const reduxParty: MemoryLayout<ReduxParty> = {
  byteLength: 0x4c,
  decode(memory, address) {
    const level = memory.u8(address + 0x3c);
    const hp = memory.u16(address + 0x3e);
    const maxHp = memory.u16(address + 0x40);
    checkVitals(level, hp, maxHp);
    return {
      ...reduxBox.decode(memory, address),
      pp: [...memory.bytes(address + 0x34, 4)],
      level,
      hp,
      maxHp,
      status: memory.u32(address + 0x38),
      calculatedStats: readStats(memory, address + 0x40, 2),
    };
  },
};

export const reduxStorage = {
  byteLength: 0x9f78,
  decode(memory: MemoryReader, address: number) {
    const currentBox = memory.u8(address);
    if (currentBox >= 26) throw new Error("Invalid Redux box index");
    const boxes = Array.from({ length: 26 }, (_, box) => {
      const pokemon: { slot: number; record: ReduxBox }[] = [];
      for (let slot = 0; slot < 30; slot++) {
        const target = address + 4 + (box * 30 + slot) * reduxBox.byteLength;
        if (!memory.u16(target + 16)) continue;
        pokemon.push({
          slot,
          record: memory.read({ address: target, layout: reduxBox }),
        });
      }
      const name = decodePokemonText(
        memory.bytes(address + 0x9e74 + box * 9, 9),
      );
      if (name.warnings.length)
        throw new Error(`Redux box ${box + 1}: ${name.warnings.join("; ")}`);
      return { box, name: name.text, pokemon };
    });
    return { currentBox, boxes };
  },
};

export const reduxBattle = {
  byteLength: 0x68,
  decode(memory: MemoryReader, address: number) {
    const level = memory.u8(address + 0x4b);
    const hp = memory.u16(address + 0x36);
    const maxHp = memory.u16(address + 0x38);
    checkVitals(level, hp, maxHp);
    const stageKeys = [
      "attack",
      "defense",
      "speed",
      "specialAttack",
      "specialDefense",
      "accuracy",
      "evasion",
    ] as const;
    const stages = stageKeys.map((key, i) => {
      const value = memory.u8(address + 0x3d + i);
      if (value > 12) throw new Error("Invalid Redux battle stage");
      return [key, value - 6];
    });
    return {
      personality: memory.u32(address + 4),
      trainerId: memory.u32(address + 16),
      species: memory.u16(address + 20),
      experience: memory.u32(address),
      status: memory.u32(address + 8),
      volatileStatus: memory.u32(address + 12),
      moveIds: Array.from({ length: 4 }, (_, i) =>
        memory.u16(address + 0x20 + i * 2),
      ),
      abilityIds: Array.from({ length: 7 }, (_, i) =>
        memory.u16(address + 0x28 + i * 2),
      ),
      heldItem: memory.u16(address + 0x3a),
      typeIds: [...memory.bytes(address + 0x44, 3)].filter((id) => id !== 9),
      pp: [...memory.bytes(address + 0x47, 4)],
      ppBonuses: memory.u8(address + 0x5a),
      hiddenNature: memory.u8(address + 0x63),
      friendship: memory.u8(address + 0x4c),
      level,
      hp,
      maxHp,
      calculatedStats: {
        hp: maxHp,
        ...Object.fromEntries(
          statKeys
            .slice(1)
            .map((key, i) => [key, memory.u16(address + 0x16 + i * 2)]),
        ),
      } as PokemonStats,
      statStages: Object.fromEntries(stages) as PokemonBattleStatStages,
    };
  },
};

function checkVitals(level: number, hp: number, maxHp: number) {
  if (level < 1 || level > 100 || !maxHp || hp > maxHp)
    throw new Error("Invalid Redux level/HP");
}
function readStats(
  memory: MemoryReader,
  address: number,
  stride: 1 | 2,
): PokemonStats {
  return Object.fromEntries(
    statKeys.map((key, i) => [
      key,
      stride === 1 ? memory.u8(address + i) : memory.u16(address + i * 2),
    ]),
  ) as PokemonStats;
}
