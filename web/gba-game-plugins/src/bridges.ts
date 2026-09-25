import {
  defineBridge,
  at,
  type BridgeContext,
  type DecodeResult,
  type MemoryLayout,
  type MemoryReader,
  type MemoryRef,
  type MemoryArrayRef,
} from "@evenfall/gba/bridge";
import type { PokemonCatalog } from "./catalog";
import {
  statKeys,
  type BoxPokemon,
  type PartyRecord,
  type PokemonArtworkRef,
  type PokemonItemArtworkRef,
  type PokemonTrainerArtworkRef,
  type PokemonBattleKind,
  type PokemonBattleStatStages,
  type PokemonBattleView,
  type PokemonDetails,
  type PokemonImage,
  type PokemonLinksFactory,
  type PokemonPartyMember,
  type PokemonPartyView,
  type PokemonStats,
  type PokemonStorageView,
} from "./model";
import type { Gen3BattlePokemon } from "./gen3-record";
import { decodePokemonText } from "./text";

/** A profile converts runtime flags to these semantic choices, not UI-specific fields. */
export interface PokemonRules {
  readonly legacyStats: boolean;
  readonly typeOverrides: Readonly<Record<number, "legacy" | "alternate">>;
  readonly legacyAbilities: readonly {
    readonly species: number;
    readonly slot: number;
  }[];
  readonly typesUnavailable: string | null;
  readonly abilitiesUnavailable: string | null;
  readonly baseStatsUnavailable: string | null;
  readonly shinyThreshold: number;
  readonly shinyRequiresPersonality: boolean;
  readonly modernShinySpecies: readonly number[];
}
export interface PokemonResources {
  readonly catalog: PokemonCatalog;
  readonly artwork: (
    ref: PokemonArtworkRef | PokemonItemArtworkRef | PokemonTrainerArtworkRef,
  ) => PokemonImage;
  readonly linksFactory?: PokemonLinksFactory;
}
export type PokemonPartySymbols = {
  readonly gPlayerPartyCount: MemoryRef<number>;
  readonly gPlayerParty: MemoryArrayRef<DecodeResult<PartyRecord>>;
  readonly rules: MemoryRef<MemoryRef<PokemonRules> | null>;
};

export type PokemonBattleSymbols = PokemonPartySymbols & {
  readonly battleActive: MemoryRef<boolean>;
  readonly gEnemyParty: MemoryArrayRef<DecodeResult<PartyRecord>>;
  readonly gBattleTypeFlags: MemoryRef<number>;
  readonly gBattlersCount: MemoryRef<number>;
  readonly gBattlerPartyIndexes: MemoryArrayRef<number>;
  readonly gBattleMons: MemoryArrayRef<DecodeResult<Gen3BattlePokemon>>;
  readonly gAbsentBattlerFlags: MemoryRef<number>;
};

export const detailedGameBattle = defineBridge({
  id: "gameBattle",
  decode({
    memory,
    symbols,
    sample,
    resources,
  }: BridgeContext<
    PokemonBattleSymbols,
    PokemonResources
  >): DecodeResult<PokemonBattleView> {
    if (!memory.read(symbols.battleActive))
      return { status: "ready", data: { active: false } };

    const rulesRef = memory.read(symbols.rules);
    if (!rulesRef) return unavailable("Game settings are not initialized");
    const rules = memory.read(rulesRef);
    const playerCount = memory.read(symbols.gPlayerPartyCount);
    if (!playerCount || playerCount > symbols.gPlayerParty.length)
      return unavailable("Player party count is invalid during battle");
    const playerParty = decodeParty(
      memory,
      symbols.gPlayerParty,
      Array.from({ length: playerCount }, (_, slot) => slot),
      resources.catalog,
      rules,
      sample.romHash,
      "Player party",
    );
    if (playerParty.status !== "ready") return playerParty;

    const typeFlags = memory.read(symbols.gBattleTypeFlags);
    const kind = battleKind(typeFlags);
    const battlersCount = memory.read(symbols.gBattlersCount);
    if (battlersCount !== 2 && battlersCount !== 4)
      return unavailable("Battle has an invalid battler count");
    const opponentSlots =
      kind === "wild"
        ? [
            ...new Set(
              Array.from({ length: battlersCount / 2 }, (_, index) =>
                memory.read(at(symbols.gBattlerPartyIndexes, index * 2 + 1)),
              ),
            ),
          ].sort((a, b) => a - b)
        : Array.from(
            { length: symbols.gEnemyParty.length },
            (_, slot) => slot,
          ).filter(
            (slot) =>
              memory.u8(
                symbols.gEnemyParty.address +
                  slot * symbols.gEnemyParty.element.byteLength +
                  0x13,
              ) & 2,
          );
    if (!opponentSlots.length)
      return unavailable("Opponent party has no occupied battle slots");
    if (opponentSlots.some((slot) => slot >= symbols.gEnemyParty.length))
      return unavailable("Opponent battler has an invalid party slot");
    const opponentParty = decodeParty(
      memory,
      symbols.gEnemyParty,
      opponentSlots,
      resources.catalog,
      rules,
      sample.romHash,
      "Opponent party",
    );
    if (opponentParty.status !== "ready") return opponentParty;

    const absent = memory.read(symbols.gAbsentBattlerFlags);
    const battlers: Extract<
      PokemonBattleView,
      { active: true }
    >["battlers"][number][] = [];
    for (let battler = 0; battler < battlersCount; battler++) {
      if (absent & (1 << battler)) continue;
      const side = battler & 1 ? "opponent" : "player";
      const partySlot = memory.read(at(symbols.gBattlerPartyIndexes, battler));
      const partyRef =
        side === "player" ? symbols.gPlayerParty : symbols.gEnemyParty;
      if (
        side === "player"
          ? partySlot >= playerCount
          : !opponentSlots.includes(partySlot)
      )
        return unavailable(`Battler ${battler} has an invalid party slot`);
      const source = memory.read(at(partyRef, partySlot));
      if (source.status !== "ready")
        return unavailable(`Battler ${battler}: ${source.reason}`);
      const live = memory.read(at(symbols.gBattleMons, battler));
      if (live.status !== "ready")
        return unavailable(`Battler ${battler}: ${live.reason}`);
      if (
        source.data.personality !== live.data.personality ||
        source.data.trainerId !== live.data.trainerId ||
        source.data.species !== live.data.species
      )
        return unavailable(
          `Battler ${battler} does not match its linked party record`,
        );
      const record: BoxPokemon = {
        ...source.data,
        species: live.data.species,
        heldItem: live.data.heldItem,
        experience: live.data.experience,
        friendship: live.data.friendship,
        moveIds: live.data.moveIds,
        pp: live.data.pp,
        ppBonuses: live.data.ppBonuses,
        ivs: live.data.ivs,
        isEgg: live.data.isEgg,
        abilitySlot: live.data.abilitySlot,
      };
      const details = pokemonDetails(
        record,
        resources.catalog,
        {
          ...rules,
          typesUnavailable: null,
          abilitiesUnavailable: null,
        },
        sample.romHash,
        live.data.level,
        live.data.stats,
      );
      battlers.push({
        ...details,
        types: [...new Set(live.data.typeIds)].map((id) =>
          resources.catalog.type(id),
        ),
        abilities: [
          {
            ...resources.catalog.ability(live.data.ability),
            slot: 0,
            kind: "ability",
            state: "available",
          },
        ],
        battler,
        side,
        partySlot,
        hp: live.data.hp,
        maxHp: live.data.maxHp,
        status: live.data.status,
        volatileStatus: live.data.volatileStatus,
        ailments: ailments(live.data.status),
        statStages: live.data.statStages,
        stageAdjustedStats: applyStatStages(
          live.data.stats,
          live.data.statStages,
        ),
      });
    }
    return {
      status: "ready",
      data: {
        active: true,
        kind,
        format: typeFlags & 1 ? "double" : "single",
        typeFlags,
        types: battleTypes(typeFlags),
        turn: null,
        battleWeather: null,
        playerParty: playerParty.data,
        opponentParty: opponentParty.data,
        battlers,
        warnings: [
          "Battle turn counter is not verified for this build",
          "Battle weather global is not verified for this build",
        ],
      },
    };
  },
});

export const detailedGameParty = defineBridge({
  id: "gameParty",
  decode({
    memory,
    symbols,
    sample,
    resources,
  }: BridgeContext<
    PokemonPartySymbols,
    PokemonResources
  >): DecodeResult<PokemonPartyView> {
    const count = memory.read(symbols.gPlayerPartyCount);
    if (count > symbols.gPlayerParty.length)
      return unavailable("Party count is outside its declared capacity");
    if (!count) return { status: "ready", data: { party: [] } };
    const rulesRef = memory.read(symbols.rules);
    if (!rulesRef) return unavailable("Game settings are not initialized");
    const rules = memory.read(rulesRef);
    const party: PokemonPartyView["party"][number][] = [];
    for (let slot = 0; slot < count; slot++) {
      const record = memory.read(at(symbols.gPlayerParty, slot));
      if (record.status !== "ready")
        return unavailable(`Party slot ${slot + 1}: ${record.reason}`);
      const mon = record.data;
      party.push({
        ...pokemonDetails(
          mon,
          resources.catalog,
          rules,
          sample.romHash,
          mon.level,
          mon.calculatedStats,
        ),
        slot,
        hp: mon.hp,
        maxHp: mon.maxHp,
        status: mon.status,
        ailments: ailments(mon.status),
      });
    }
    return { status: "ready", data: { party } };
  },
});

export interface StoredRecords {
  readonly currentBox: number;
  readonly boxes: readonly {
    readonly box: number;
    readonly name: string;
    readonly pokemon: readonly {
      readonly slot: number;
      readonly record: BoxPokemon;
    }[];
  }[];
}
export type PokemonStorageSymbols = {
  readonly storage: MemoryRef<MemoryRef<DecodeResult<StoredRecords>> | null>;
  readonly rules: MemoryRef<MemoryRef<PokemonRules> | null>;
};
export const gameStorage = defineBridge({
  id: "gameStorage",
  decode({
    memory,
    symbols,
    sample,
    resources,
  }: BridgeContext<
    PokemonStorageSymbols,
    PokemonResources
  >): DecodeResult<PokemonStorageView> {
    const root = memory.read(symbols.storage),
      rulesRef = memory.read(symbols.rules);
    if (!root || !rulesRef)
      return unavailable("Pokémon storage is not initialized");
    const storage = memory.read(root);
    if (storage.status !== "ready") return storage;
    const rules = memory.read(rulesRef);
    const boxes = storage.data.boxes.map(({ box, name, pokemon }) => ({
      box,
      name,
      pokemon: pokemon.map(({ slot, record }) => ({
        ...pokemonDetails(record, resources.catalog, rules, sample.romHash),
        slot,
      })),
    }));
    return {
      status: "ready",
      data: {
        currentBox: storage.data.currentBox,
        boxes,
        occupied: boxes.reduce((n, box) => n + box.pokemon.length, 0),
      },
    };
  },
});

/** Format-specific storage geometry is supplied by a verified build profile. */
export function pokemonStorageLayout(
  record: MemoryLayout<DecodeResult<BoxPokemon>>,
  layout: {
    readonly byteLength: number;
    readonly boxesOffset: number;
    readonly boxCount: number;
    readonly capacity: number;
    readonly namesOffset: number;
    readonly nameLength: number;
  },
): MemoryLayout<DecodeResult<StoredRecords>> {
  return {
    byteLength: layout.byteLength,
    decode(memory, address) {
      const currentBox = memory.u8(address);
      if (currentBox >= layout.boxCount)
        return unavailable("Current box is outside storage capacity");
      const boxes: StoredRecords["boxes"][number][] = [];
      for (let box = 0; box < layout.boxCount; box++) {
        const pokemon: StoredRecords["boxes"][number]["pokemon"][number][] = [];
        for (let slot = 0; slot < layout.capacity; slot++) {
          const target =
            address +
            layout.boxesOffset +
            (box * layout.capacity + slot) * record.byteLength;
          // An empty slot has no species flag. Corrupt occupied records are never silently skipped.
          if (!(memory.u8(target + 0x13) & 2)) continue;
          const result = memory.read({ address: target, layout: record });
          if (result.status !== "ready")
            return unavailable(
              `Box ${box + 1}, slot ${slot + 1}: ${result.reason}`,
            );
          pokemon.push({ slot, record: result.data });
        }
        boxes.push({
          box,
          name: decodePokemonText(
            memory.bytes(
              address + layout.namesOffset + box * layout.nameLength,
              layout.nameLength,
            ),
          ).text,
          pokemon,
        });
      }
      return { status: "ready", data: { currentBox, boxes } };
    },
  };
}

export function pokemonDetails(
  record: BoxPokemon,
  catalog: PokemonCatalog,
  rules: PokemonRules,
  romHash: string,
  partyLevel?: number,
  actual: PokemonStats | null = null,
): PokemonDetails {
  const species = catalog.species(record.species);
  const warnings = [...record.textWarnings];
  for (const reason of [
    rules.typesUnavailable,
    rules.abilitiesUnavailable,
    rules.baseStatsUnavailable,
  ])
    if (reason) warnings.push(reason);
  let level = partyLevel ?? 1;
  if (partyLevel === undefined)
    while (
      level < 100 &&
      record.experience >= catalog.experience(species.growth, level + 1)
    )
      level++;
  const typeSource = rules.typeOverrides[record.species];
  const typeIds =
    typeSource === "legacy"
      ? species.legacyTypes
      : typeSource === "alternate"
        ? species.alternateTypes
        : species.types;
  const abilities = rules.legacyAbilities.some(
    ({ species, slot }) =>
      species === record.species && slot === record.abilitySlot,
  )
    ? species.legacyAbilities
    : species.abilities;
  const shinyValue =
    (record.trainerId >>> 16) ^
    (record.trainerId & 0xffff) ^
    (record.personality >>> 16) ^
    (record.personality & 0xffff);
  const shiny =
    shinyValue < rules.shinyThreshold &&
    (!rules.shinyRequiresPersonality || record.personality !== 0);
  const palette = shiny
    ? rules.modernShinySpecies.includes(record.species)
      ? "modern-shiny"
      : "shiny"
    : "normal";
  const art = {
    romHash,
    species: record.species,
    personality: record.personality,
    isEgg: record.isEgg,
    palette,
  } as const;
  const base = Object.fromEntries(
    statKeys.map((key) => [
      key,
      rules.legacyStats && species.legacyBase?.[key]
        ? species.legacyBase[key]
        : species.base[key],
    ]),
  ) as unknown as PokemonStats;
  return {
    species: record.species,
    speciesName: species.name,
    personality: record.personality,
    trainerId: record.trainerId,
    nickname: record.nickname,
    trainerName: record.trainerName,
    language: record.language,
    level,
    isEgg: record.isEgg,
    gender:
      species.genderRatio === 255
        ? "genderless"
        : species.genderRatio === 254
          ? "female"
          : (record.personality & 255) < species.genderRatio
            ? "female"
            : "male",
    shiny,
    nature: catalog.nature(record.hiddenNature ?? record.personality % 25),
    types:
      rules.typesUnavailable || !typeIds
        ? null
        : [...new Set(typeIds)].map((id) => catalog.type(id)),
    abilities:
      rules.abilitiesUnavailable || !abilities
        ? null
        : [
            {
              ...catalog.ability(abilities[record.abilitySlot]),
              slot: 0,
              kind: "ability",
              state: "available",
            },
          ],
    heldItem: record.heldItem ? catalog.item(record.heldItem) : null,
    friendship: record.friendship,
    experience: {
      total: record.experience,
      levelStart: catalog.experience(species.growth, level),
      nextLevel:
        level === 100 ? null : catalog.experience(species.growth, level + 1),
    },
    stats: {
      actual,
      base: rules.baseStatsUnavailable ? null : base,
      ivs: record.ivs,
      evs: record.evs,
    },
    moves: record.moveIds.map((id, slot) => {
      if (!id) return null;
      const move = catalog.move(id);
      return {
        ...move,
        slot,
        pp: record.pp[slot],
        maxPp:
          move.pp +
          Math.floor((move.pp * ((record.ppBonuses >>> (slot * 2)) & 3)) / 5),
      };
    }),
    artwork: {
      icon: { ...art, kind: "icon" },
      front: { ...art, kind: "front" },
    },
    warnings,
  };
}

/** Standard HnS/Redux stage ratios, applied to validated signed stages (−6…+6).
 * Keep the raw stats untouched; move/ability/item/field modifiers are contextual.
 */
export function applyStatStages(
  stats: PokemonStats,
  stages: PokemonBattleStatStages,
): PokemonStats {
  return Object.fromEntries(
    statKeys.map((key) => {
      if (key === "hp") return [key, stats.hp];
      const stage = stages[key];
      return [
        key,
        Math.floor(
          (stats[key] * (stage >= 0 ? 2 + stage : 2)) /
            (stage >= 0 ? 2 : 2 - stage),
        ),
      ];
    }),
  ) as PokemonStats;
}

export function ailments(status: number): string[] {
  const values: string[] = [];
  if (status & 7) values.push("Sleep");
  for (const [mask, name] of [
    [8, "Poison"],
    [16, "Burn"],
    [32, "Freeze"],
    [64, "Paralysis"],
    [128, "Bad poison"],
  ] as const)
    if (status & mask) values.push(name);
  return values;
}

function decodeParty(
  memory: MemoryReader,
  partyRef: MemoryArrayRef<DecodeResult<PartyRecord>>,
  slots: readonly number[],
  catalog: PokemonCatalog,
  rules: PokemonRules,
  romHash: string,
  label: string,
): DecodeResult<PokemonPartyMember[]> {
  const party: PokemonPartyMember[] = [];
  for (const slot of slots) {
    const result = memory.read(at(partyRef, slot));
    if (result.status !== "ready")
      return unavailable(`${label} slot ${slot + 1}: ${result.reason}`);
    const mon = result.data;
    party.push({
      ...pokemonDetails(
        mon,
        catalog,
        rules,
        romHash,
        mon.level,
        mon.calculatedStats,
      ),
      slot,
      hp: mon.hp,
      maxHp: mon.maxHp,
      status: mon.status,
      ailments: ailments(mon.status),
    });
  }
  return { status: "ready", data: party };
}

const battleFlagNames = [
  [0, "double"],
  [1, "link"],
  [2, "master"],
  [3, "trainer"],
  [4, "first-battle"],
  [5, "link-in-battle"],
  [6, "multi"],
  [7, "safari"],
  [8, "battle-tower"],
  [9, "tutorial"],
  [10, "roamer"],
  [11, "e-reader-trainer"],
  [12, "kyogre-groudon"],
  [13, "legendary"],
  [14, "regi"],
  [15, "two-opponents"],
  [16, "dome"],
  [17, "palace"],
  [18, "arena"],
  [19, "factory"],
  [20, "pike"],
  [21, "pyramid"],
  [22, "in-game-partner"],
  [23, "tower-link-multi"],
  [24, "recorded"],
  [25, "recorded-link"],
  [26, "trainer-hill"],
  [27, "secret-base"],
  [28, "ho-oh"],
  [29, "kyogre"],
  [30, "rayquaza"],
  [31, "recorded-master"],
] as const;

function battleTypes(flags: number): string[] {
  return battleFlagNames
    .filter(([bit]) => flags & (2 ** bit))
    .map(([, name]) => name);
}

export function battleKind(flags: number): PokemonBattleKind {
  if (flags & (1 << 1)) return "link";
  if (flags & (1 << 7)) return "safari";
  if (flags & ((1 << 4) | (1 << 9))) return "tutorial";
  if (flags & (1 << 3)) return "trainer";
  if ((flags & ~((1 << 0) | (1 << 2))) === 0) return "wild";
  return "other";
}
function unavailable(reason: string): DecodeResult<never> {
  return { status: "unavailable", reason };
}
