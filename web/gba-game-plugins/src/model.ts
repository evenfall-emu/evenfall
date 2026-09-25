import type { DecodeResult } from "@evenfall/gba/bridge";

/** Game-independent Pokémon concepts. IDs remain in the matched game's namespace. */
export const statKeys = [
  "hp",
  "attack",
  "defense",
  "speed",
  "specialAttack",
  "specialDefense",
] as const;
export type PokemonStat = (typeof statKeys)[number];
export type PokemonStats = Readonly<Record<PokemonStat, number>>;
export interface NamedValue {
  readonly id: number;
  readonly name: string;
}
export interface DescribedValue extends NamedValue {
  /** Missing/null when this profile has no verified description. */
  readonly description?: string | null;
  readonly warnings?: readonly string[];
}
export type MoveCategory = "physical" | "special" | "status";
export interface PokemonMove extends DescribedValue {
  readonly slot: number;
  readonly type: NamedValue;
  /** Null means this game's format has no verified category semantics. */
  readonly category: MoveCategory | null;
  /** Null means this game's catalog has no verified priority field. */
  readonly priority?: number | null;
  /** Some games do not retain current PP in PC storage. */
  readonly pp: number | null;
  readonly maxPp: number;
  readonly power: number;
  readonly accuracy: number;
}

export interface PokemonItemArtworkRef {
  readonly kind: "item";
  readonly romHash: string;
  readonly item: number;
}
export interface PokemonItem extends DescribedValue {
  readonly artwork?: PokemonItemArtworkRef | null;
}

/** Equipped abilities, not the species' list of alternative selectable abilities. */
export interface PokemonAbility extends DescribedValue {
  readonly slot: number;
  readonly kind: "ability" | "innate" | "extra";
  /** Available means equipped, not that its conditional effect is firing right now. */
  readonly state: "available" | "locked" | "suppressed" | "unknown";
  readonly reason?: string;
}

/** Decoded identity, not a globally unique key: cloned Pokémon can share both values. */
export interface PokemonIdentity {
  readonly personality: number;
  readonly trainerId: number;
}

export interface BoxPokemon extends PokemonIdentity {
  readonly species: number;
  readonly nickname: string;
  readonly trainerName: string;
  readonly language: number;
  readonly textWarnings: readonly string[];
  readonly heldItem: number;
  readonly experience: number;
  readonly friendship: number;
  readonly hiddenNature: number | null;
  readonly moveIds: readonly number[];
  readonly pp: readonly (number | null)[];
  readonly ppBonuses: number;
  readonly ivs: PokemonStats;
  readonly evs: PokemonStats;
  readonly isEgg: boolean;
  readonly abilitySlot: number;
}

export interface PartyRecord extends BoxPokemon {
  readonly level: number;
  readonly hp: number;
  readonly maxHp: number;
  readonly status: number;
  readonly calculatedStats: PokemonStats;
}

/** Small serializable key; pixel buffers stay in the ROM resource module, not snapshots. */
export interface PokemonArtworkRef {
  readonly romHash: string;
  readonly species: number;
  readonly personality: number | null;
  readonly isEgg: boolean;
  readonly palette: "normal" | "shiny" | "modern-shiny";
  readonly kind: "icon" | "front";
}
export interface PokemonImage {
  readonly width: number;
  readonly height: number;
  /** Owned RGBA pixels, outside the published plain-data bridge observation. */
  readonly pixels: Uint8ClampedArray;
}

export interface PokemonTrainerArtworkRef {
  readonly kind: "trainer";
  readonly romHash: string;
  readonly trainerPic: number;
}

/** A resolved starting party, not a live battle or a declaration of eligibility. */
export interface PokemonTrainerPreview {
  readonly id: string;
  readonly internalId: number | string;
  readonly name: string;
  readonly variant: string | null;
  readonly className: string;
  readonly portrait: PokemonTrainerArtworkRef | null;
  readonly team: readonly PokemonPartyMember[];
  readonly averageLevel: number;
  readonly warnings: readonly string[];
}

/** Bound to the current game's coherent context by its connector, not by the UI. */
export type PokemonTrainerSearch = (
  query: string,
) => DecodeResult<readonly PokemonTrainerPreview[]>;

/** Pure game-owned URL factories, outside live snapshots. Null means no link. */
export interface PokemonLinksFactory {
  readonly pokemon?: (pokemon: PokemonDetails) => string | null;
  readonly ability?: (ability: PokemonAbility) => string | null;
  readonly move?: (move: PokemonMove) => string | null;
}

/** Both party and storage share details; storage never fabricates current HP/stats. */
export interface PokemonDetails extends Partial<PokemonIdentity> {
  /** Stable selection key for records without generated/live Pokémon identity. */
  readonly id?: string;
  readonly species: number;
  readonly speciesName: string;
  readonly nickname: string;
  readonly trainerName?: string;
  readonly language: number;
  readonly level: number;
  readonly isEgg: boolean;
  readonly gender: "male" | "female" | "genderless" | null;
  readonly shiny: boolean | null;
  readonly nature: NamedValue | null;
  readonly types: readonly NamedValue[] | null;
  readonly abilities: readonly PokemonAbility[] | null;
  readonly heldItem: PokemonItem | null;
  readonly friendship?: number;
  readonly experience?: {
    readonly total: number;
    readonly levelStart: number;
    readonly nextLevel: number | null;
  };
  readonly stats: {
    /** Calculated party or raw live battle stats, before battle stages. */
    readonly actual: PokemonStats | null;
    readonly base: PokemonStats | null;
    readonly ivs: PokemonStats;
    readonly evs: PokemonStats;
  };
  readonly moves: readonly (PokemonMove | null)[];
  readonly artwork: {
    readonly icon: PokemonArtworkRef;
    readonly front: PokemonArtworkRef;
  } | null;
  /** Explain unresolved mode/language metadata rather than substituting stock values. */
  readonly warnings: readonly string[];
}

export interface PokemonPartyMember extends PokemonDetails {
  readonly slot: number;
  readonly hp: number;
  readonly maxHp: number;
  readonly status: number;
  readonly ailments: readonly string[];
}
export interface PokemonPartyView {
  readonly party: readonly PokemonPartyMember[];
}

export type PokemonBattleKind =
  | "wild"
  | "trainer"
  | "safari"
  | "tutorial"
  | "link"
  | "other";
export type PokemonBattleStatStages = Readonly<
  Record<
    | "attack"
    | "defense"
    | "speed"
    | "specialAttack"
    | "specialDefense"
    | "accuracy"
    | "evasion",
    number
  >
>;
export interface PokemonBattleBattler extends PokemonDetails {
  readonly battler: number;
  readonly side: "player" | "opponent";
  readonly partySlot: number;
  readonly hp: number;
  readonly maxHp: number;
  readonly status: number;
  readonly volatileStatus: number;
  readonly ailments: readonly string[];
  /** Signed stage deltas: neutral is 0, and the verified range is −6…+6. */
  readonly statStages: PokemonBattleStatStages;
  /** Raw live stats with stage ratios applied; HP unchanged. Not a damage calculation. */
  readonly stageAdjustedStats: PokemonStats;
}
export type PokemonBattleView =
  | { readonly active: false }
  | {
      readonly active: true;
      readonly kind: PokemonBattleKind;
      readonly format: "single" | "double";
      readonly typeFlags: number;
      readonly types: readonly string[];
      readonly turn: number | null;
      readonly battleWeather: NamedValue | null;
      readonly playerParty: readonly PokemonPartyMember[];
      readonly opponentParty: readonly PokemonPartyMember[];
      readonly battlers: readonly PokemonBattleBattler[];
      readonly warnings: readonly string[];
    };
export interface PokemonStorageView {
  readonly currentBox: number;
  readonly boxes: readonly {
    readonly box: number;
    readonly name: string;
    readonly pokemon: readonly (PokemonDetails & { readonly slot: number })[];
  }[];
  readonly occupied: number;
}
