import {
  defineBridge,
  at,
  type BridgeContext,
  type DecodeResult,
  type MemoryLayout,
  type MemoryRef,
  type MemoryArrayRef,
} from "@evenfall/gba/bridge";
import { decryptGen3Record } from "./gen3-record";

export interface PartyPokemon {
  readonly personality: number;
  /** Internal species id, not a National Dex number or a localized name. */
  readonly species: number;
  readonly level: number;
  readonly hp: number;
  readonly maxHp: number;
  readonly status: number;
  readonly isEgg: boolean;
}

/** Shared view data: a team UI consumes this, never a ROM's memory layout. */
export interface PokemonParty {
  readonly party: readonly (PartyPokemon & { readonly slot: number })[];
}

export type GamePartySymbols = {
  readonly gPlayerPartyCount: MemoryRef<number>;
  readonly gPlayerParty: MemoryArrayRef<DecodeResult<PartyPokemon>>;
};

/** Reusable across builds whose profile supplies this semantic symbol contract. */
export const gameParty = defineBridge({
  id: "gameParty",
  decode({ memory, symbols }: BridgeContext<GamePartySymbols>) {
    const count = memory.read(symbols.gPlayerPartyCount);
    if (
      !Number.isInteger(count) ||
      count < 0 ||
      count > symbols.gPlayerParty.length
    )
      return {
        status: "unavailable",
        reason: "Party count is outside its declared capacity",
      };
    const party: PokemonParty["party"][number][] = [];
    for (let slot = 0; slot < count; slot++) {
      const pokemon = memory.read(at(symbols.gPlayerParty, slot));
      if (pokemon.status === "unavailable")
        return {
          status: "unavailable",
          reason: `Party slot ${slot + 1}: ${pokemon.reason}`,
        };
      party.push({ ...pokemon.data, slot });
    }
    return { status: "ready", data: { party } };
  },
});

/**
 * Stock 100-byte party record, not a promise about ROM hacks or their addresses.
 * Source facts: pret/pokefirered c75f352304d529f6ba92d4f74b9cf8b5c3810788,
 * include/pokemon.h and src/pokemon.c:GetSubstruct/DecryptBoxMon/GetBoxMonData.
 * Internal species bounds: include/constants/species.h (NUM_SPECIES = 412).
 * A build profile must independently verify this layout before selecting it.
 */
export const stockPartyPokemon = gen3PartyPokemon({
  speciesLimit: 412,
  eggMask: 0x40000000,
});

/** The two verified formats share encryption/stride, but not species or egg bits. */
export function gen3PartyPokemon({
  speciesLimit,
  eggMask,
}: {
  readonly speciesLimit: number;
  readonly eggMask: number;
}): MemoryLayout<DecodeResult<PartyPokemon>> {
  return {
    byteLength: 100,
    decode(memory, address) {
      const decrypted = decryptGen3Record(memory, address);
      if (decrypted.status === "unavailable") return decrypted;
      const { personality, flags, blocks } = decrypted.data;
      const species = blocks[0].getUint16(0, true);
      const isEgg = Boolean(blocks[3].getUint32(4, true) & eggMask);
      const level = memory.u8(address + 0x54);
      const hp = memory.u16(address + 0x56);
      const maxHp = memory.u16(address + 0x58);
      if (
        !species ||
        species >= speciesLimit || // Egg/display-only ids are not stored species.
        level < 1 ||
        level > 100 ||
        !maxHp ||
        hp > maxHp ||
        isEgg !== Boolean(flags & 4)
      )
        return {
          status: "unavailable",
          reason: "Party record is inconsistent or being updated",
        };
      return {
        status: "ready",
        data: {
          personality,
          species,
          level,
          hp,
          maxHp,
          status: memory.u32(address + 0x50),
          isEgg,
        },
      };
    },
  };
}
