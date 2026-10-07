// Golden Sun (GBA) battle and party bridge. Everything it shows is in work
// RAM, so the family reads no ROM data. Evidence for every address and field:
// docs/golden-sun.md.
import {
  at,
  createGame,
  defineFamily,
  locateWith,
  pointer,
  ram,
  ramArray,
  u8,
  type ManifestOf,
  type MemoryLayout,
  type MemoryReader,
} from "@evenfall/gba/bridge";
import { locatedFrom, locators, markers } from "./golden-sun.locators";

/** The order the game stores elements in, everywhere. */
export const goldenSunElements = [
  "venus",
  "mercury",
  "mars",
  "jupiter",
] as const;
export type GoldenSunElement = (typeof goldenSunElements)[number];

export type GoldenSunStatusKind =
  | "poison"
  | "venom"
  | "attack"
  | "defense"
  | "resist"
  | "delusion"
  | "stun"
  | "sleep"
  | "seal"
  | "regen"
  | "reflect"
  | "haunt"
  | "doom"
  | "curse";

export interface GoldenSunStatus {
  readonly kind: GoldenSunStatusKind;
  /** Turns left, or null for a status without a counter. */
  readonly turns: number | null;
  /** For stat changes: signed stages, each 12.5% of the stat. */
  readonly stage?: number;
}

/** One battle unit: a party member or an enemy, in the game's 0x14C-byte record. */
export interface GoldenSunUnit {
  /** Party: the character (0 Isaac … 7 Piers). Enemies: the battle slot, 0-5. */
  readonly slot: number;
  readonly name: string;
  readonly level: number;
  readonly hp: number;
  readonly maxHp: number;
  readonly pp: number;
  readonly maxPp: number;
  /** Current values, with equipment, class and battle changes applied. */
  readonly attack: number;
  readonly defense: number;
  readonly agility: number;
  readonly luck: number;
  /** Elemental power and resistance, in `goldenSunElements` order. */
  readonly elements: readonly {
    readonly power: number;
    readonly resist: number;
  }[];
  /** Djinn per element: how many the unit holds and how many are Set. */
  readonly djinn: readonly { readonly owned: number; readonly set: number }[];
  readonly statuses: readonly GoldenSunStatus[];
}

export interface GoldenSunPartyMember extends GoldenSunUnit {
  /** The character's own element. */
  readonly element: GoldenSunElement;
}

export interface GoldenSunPartyView {
  readonly members: readonly GoldenSunPartyMember[];
  readonly coins: number;
}

export type GoldenSunBattleView =
  | { readonly active: false }
  | { readonly active: true; readonly enemies: readonly GoldenSunUnit[] };

export interface GoldenSunViews {
  party: GoldenSunPartyView;
  battle: GoldenSunBattleView;
}

const RECORD = 0x14c;
/** The game's battles have at most six enemy slots (unit ids 0x80-0x85). */
const ENEMY_SLOTS = 6;
/** Characters 0-7 are Isaac, Garet, Ivan, Mia, Felix, Jenna, Sheba, Piers. */
const CHARACTER_ELEMENTS: readonly GoldenSunElement[] = [
  "venus",
  "mars",
  "jupiter",
  "mercury",
  "venus",
  "mars",
  "jupiter",
  "mercury",
];

const bitCount = (value: number) => {
  let count = 0;
  for (let v = value >>> 0; v; v = (v & (v - 1)) >>> 0) count++;
  return count;
};
const signed = (byte: number) => (byte << 24) >> 24;

/** Names are stored as printable ASCII, NUL-terminated, in 15 bytes. */
function readName(memory: MemoryReader, address: number) {
  let name = "";
  for (let i = 0; i < 15; i++) {
    const byte = memory.u8(address + i);
    if (byte === 0) break;
    name += byte >= 0x20 && byte < 0x7f ? String.fromCharCode(byte) : "?";
  }
  return name;
}

/** Battle statuses at +0x130…+0x141 (docs/golden-sun.md). */
function readStatuses(memory: MemoryReader, address: number) {
  const byte = (offset: number) => memory.u8(address + offset);
  const statuses: GoldenSunStatus[] = [];
  const toxin = byte(0x131);
  if (toxin === 1) statuses.push({ kind: "poison", turns: null });
  else if (toxin === 2) statuses.push({ kind: "venom", turns: null });
  for (const [kind, offset] of [
    ["attack", 0x132],
    ["defense", 0x134],
    ["resist", 0x136],
  ] as const) {
    const turns = byte(offset);
    const stage = signed(byte(offset + 1));
    if (turns && stage) statuses.push({ kind, turns, stage });
  }
  for (const [kind, offset] of [
    ["delusion", 0x138],
    ["stun", 0x13b],
    ["sleep", 0x13c],
    ["seal", 0x13d],
    ["regen", 0x13e],
    ["reflect", 0x13f],
  ] as const) {
    const turns = byte(offset);
    if (turns) statuses.push({ kind, turns });
  }
  if (byte(0x140)) statuses.push({ kind: "haunt", turns: null });
  const doom = byte(0x141);
  if (doom) statuses.push({ kind: "doom", turns: doom });
  if (byte(0x130)) statuses.push({ kind: "curse", turns: null });
  return statuses;
}

type UnitRecord = Omit<GoldenSunUnit, "slot">;
const unitRecord: MemoryLayout<UnitRecord> = {
  byteLength: RECORD,
  decode: (memory, address) => ({
    name: readName(memory, address),
    level: memory.u8(address + 0x0f),
    maxHp: memory.u16(address + 0x34),
    maxPp: memory.u16(address + 0x36),
    hp: memory.u16(address + 0x38),
    pp: memory.u16(address + 0x3a),
    attack: memory.u16(address + 0x3c),
    defense: memory.u16(address + 0x3e),
    agility: memory.u16(address + 0x40),
    luck: memory.u8(address + 0x42),
    elements: goldenSunElements.map((_, i) => ({
      power: memory.u16(address + 0x48 + i * 4),
      resist: memory.u16(address + 0x4a + i * 4),
    })),
    djinn: goldenSunElements.map((_, i) => ({
      owned: bitCount(memory.u32(address + 0xf8 + i * 4)),
      set: bitCount(memory.u32(address + 0x108 + i * 4)),
    })),
    statuses: readStatuses(memory, address),
  }),
};

/** The enemy block the game allocates per battle: six records. */
const enemyBlock: MemoryLayout<readonly UnitRecord[]> = {
  byteLength: RECORD * ENEMY_SLOTS,
  decode: (memory, address) =>
    Array.from({ length: ENEMY_SLOTS }, (_, slot) =>
      unitRecord.decode(memory, address + slot * RECORD),
    ),
};

const partyOrder: MemoryLayout<readonly number[]> = {
  byteLength: 8,
  decode: (memory, address) =>
    Array.from({ length: 8 }, (_, i) => memory.u8(address + i)),
};
const u32: MemoryLayout<number> = {
  byteLength: 4,
  decode: (memory, address) => memory.u32(address),
};

const sources = locateWith(
  {
    party: ramArray(unitRecord, 8),
    /** Story flags 0-7: which characters have joined. */
    partyFlags: ram(u8),
    partyOrder: ram(partyOrder),
    coins: ram(u32),
    /** The battle's enemy block, or null outside battle. */
    enemies: ram(pointer(enemyBlock)),
  },
  locators,
);

const plausible = (unit: UnitRecord) =>
  unit.level >= 1 &&
  unit.level <= 99 &&
  unit.maxHp > 0 &&
  unit.hp <= unit.maxHp &&
  unit.pp <= unit.maxPp;

/**
 * Golden Sun's battle units: the party, the coins and the enemies. Golden Sun
 * and The Lost Age share the unit record and its routines; their addresses
 * differ, and the locators carry both games' code.
 */
export const goldenSunFamily = defineFamily<GoldenSunViews>()({
  id: "golden-sun",
  sources,
  // No ROM data; an object rather than undefined, which means "not prepared".
  resources: () => ({}),
  views: {
    party: ({ memory, symbols }) => {
      const flags = memory.read(symbols.partyFlags);
      const count = bitCount(flags);
      if (!count)
        return { status: "unavailable", reason: "No one has joined yet" };
      const order = memory.read(symbols.partyOrder).slice(0, count);
      if (order.some((pc) => pc > 7 || !(flags & (1 << pc))))
        return { status: "unavailable", reason: "The party order is updating" };
      const members: GoldenSunPartyMember[] = [];
      for (const pc of order) {
        const unit = memory.read(at(symbols.party, pc));
        if (!plausible(unit))
          return {
            status: "unavailable",
            reason: "A party record is not valid",
          };
        members.push({ ...unit, slot: pc, element: CHARACTER_ELEMENTS[pc] });
      }
      return {
        status: "ready",
        data: { members, coins: memory.read(symbols.coins) },
      };
    },
    battle: ({ memory, symbols }) => {
      const block = memory.read(symbols.enemies);
      if (!block) return { status: "ready", data: { active: false } };
      // Unused slots are zeroed; defeated enemies keep their record at 0 HP.
      const enemies = memory
        .read(block)
        .map((unit, slot) => ({ ...unit, slot }))
        .filter((unit) => unit.name && unit.maxHp > 0);
      if (!enemies.length || !enemies.every(plausible))
        return { status: "unavailable", reason: "The battle is setting up" };
      return { status: "ready", data: { active: true, enemies } };
    },
  },
  detect: { gameCodes: ["AGSE", "AGFE"], markers },
  locatedFrom,
});

/** The Golden Age hack the user supplied: a binary patch of Golden Sun (USA, Europe). */
export const goldenAge1: ManifestOf<typeof goldenSunFamily> = {
  family: "golden-sun",
  romHash: "1864e0f1ae44e07f10c2d0ac826e6104c69ca5e9",
  source: {
    repository: "Binary patch of Golden Sun (USA, Europe); no published source",
    revision: "Golden Age 1",
    variant: "16 MiB patched ROM, header Golden_Sun_AAGSE01",
    compiler: "Retail Golden Sun code, patched in place",
    options: [
      "Verified by disassembly and live save states; not a matched rebuild",
    ],
  },
  addresses: {
    party: 0x02000500,
    partyFlags: 0x02000040,
    partyOrder: 0x02000438,
    coins: 0x02000250,
    enemies: 0x03001f28,
  },
  rom: undefined,
  provenance: { method: "reviewed" },
};

/**
 * Reviewed builds. The Lost Age (USA, Europe) and other builds whose code
 * still matches a game's locators are recognized in the player.
 */
export const goldenSun = createGame(goldenSunFamily, [goldenAge1]);

/** Living units by descending agility, which is how the game orders a round's actions. */
export function estimatedTurnOrder(
  party: readonly GoldenSunUnit[],
  enemies: readonly GoldenSunUnit[],
) {
  return [
    ...party.map((unit) => ({ unit, side: "party" as const })),
    ...enemies.map((unit) => ({ unit, side: "enemy" as const })),
  ]
    .filter(({ unit }) => unit.hp > 0)
    .sort((a, b) => b.unit.agility - a.unit.agility);
}

/**
 * How a unit's resistance to each element compares with its others: weak or
 * resistant when 15 or more below or above their mean.
 */
export function elementalAffinity(unit: GoldenSunUnit) {
  const mean =
    unit.elements.reduce((sum, e) => sum + e.resist, 0) / unit.elements.length;
  return unit.elements.map(({ resist }) =>
    resist <= mean - 15 ? "weak" : resist >= mean + 15 ? "resists" : "neutral",
  );
}

/** Djinn on Standby per element across the party, which summons spend. */
export function standbyDjinn(party: readonly GoldenSunUnit[]) {
  return goldenSunElements.map((_, i) =>
    party.reduce(
      (sum, unit) => sum + unit.djinn[i].owned - unit.djinn[i].set,
      0,
    ),
  );
}
