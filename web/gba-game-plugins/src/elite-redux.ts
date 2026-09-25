import {
  definePreparedGame,
  array,
  at,
  pointer,
  ref,
  u8,
  type BuildProfile,
  type DecodeResult,
  type MemoryLayout,
  type GameArenaStatus,
} from "@evenfall/gba/bridge";
import {
  ailments,
  applyStatStages,
  battleKind,
  pokemonDetails,
  type PokemonRules,
} from "./bridges";
import { createPokemonArtwork, type PokemonArtworkConfig } from "./artwork";
import {
  createReduxCatalog,
  type ReduxCatalog,
  type ReduxCatalogConfig,
} from "./redux-catalog";
import {
  createReduxTrainerSearch,
  type ReduxTrainerCatalogConfig,
  type ReduxTrainerContext,
} from "./redux-trainers";
import {
  reduxParty,
  reduxBattle,
  reduxSettings,
  reduxStorage,
  type ReduxBox,
  type ReduxParty,
  type ReduxSettings,
} from "./redux-record";
import type {
  PokemonAbility,
  PokemonBattleView,
  PokemonDetails,
  PokemonLinksFactory,
  PokemonPartyMember,
  PokemonPartyView,
  PokemonStorageView,
  PokemonStats,
} from "./model";

const u16: MemoryLayout<number> = { byteLength: 2, decode: (m, a) => m.u16(a) };
const u32: MemoryLayout<number> = { byteLength: 4, decode: (m, a) => m.u32(a) };
const inBattle: MemoryLayout<boolean> = {
  byteLength: 1,
  decode: (m, a) => Boolean(m.u8(a) & 2),
};
const symbols265 = {
  playerCount: ref(0x02039905, u8),
  playerParty: array(0x02039908, reduxParty, 6),
  enemyParty: array(0x02039ad0, reduxParty, 6),
  settings: ref(0x03003410, pointer(reduxSettings)),
  thirdBadge: ref(
    0x0300340c,
    pointer({
      byteLength: 0x1ae2,
      decode: (memory, address) => Boolean(memory.u8(address + 0x1ae1) & 8),
    }),
  ),
  storage: ref(0x03003414, pointer(reduxStorage)),
  battleActive: ref(0x03003859, inBattle),
  battleFlags: ref(0x0201c3f4, u32),
  battlerCount: ref(0x0201c484, u8),
  partyIndexes: array(0x0201c488, u16, 4),
  battlers: array(0x0201c50c, reduxBattle, 4),
  absent: ref(0x0201c6db, u8),
  statuses3: array(0x0201c7ac, u32, 4),
  neutralizingGas: ref(0x0201cb69, u8),
  enigmaBerries: array(
    0x0201ca28,
    { byteLength: 0x1c, decode: (m, a) => m.u8(a + 7) },
    4,
  ),
};

/** Add a verified build here, not a copy of the game/bridge/UI. Different ABIs need a new layout family. */
export interface EliteReduxBuild extends BuildProfile<typeof symbols265> {
  readonly layout: "redux-2.65";
  readonly catalog: ReduxCatalogConfig;
  readonly artwork: Omit<PokemonArtworkConfig, "romHash">;
  readonly trainers: ReduxTrainerCatalogConfig;
  readonly linksFactory?: PokemonLinksFactory;
}
export const eliteReduxBeta2: EliteReduxBuild = {
  layout: "redux-2.65",
  romHash: "d86e6ac909e4abdef9159ed20b75a684a9b2ee2fe874f388122e00a5749d2d37",
  source: {
    repository: "https://github.com/Elite-Redux/eliteredux-source",
    revision: "06da575972cfed20d471f8bb5b90ea9e4736b16a",
    variant: "v2.65 Beta2 (1048)",
    compiler: "Release compiler unrecorded; ARM ABI probes GCC 16.2.0",
    options: [
      "Exact release ROM tables, RAM and disassembly checked; no matched rebuild",
    ],
  },
  symbols: symbols265,
  linksFactory: {
    pokemon: (pokemon) =>
      pokemon.isEgg
        ? null
        : `https://forwardfeed.github.io/ER-nextdex/?v=2.65beta&p=${pokemon.species}`,
  },
  catalog: {
    species: 0x08bf5b0c,
    speciesNames: 0x089c16c8,
    speciesCount: 2902,
    moves: 0x08bebfac,
    moveNames: 0x089c7a24,
    moveDescriptions: 0x08ec1724,
    moveCount: 1032,
    abilities: 0x08fbc4d8,
    abilityCount: 1034,
    items: 0x08ab9674,
    itemCount: 1023,
    types: 0x08517074,
    natures: 0x08ec2804,
    experience: 0x08bf4e6c,
  },
  trainers: {
    trainers: 0x089a2ffc,
    trainerCount: 932,
    classes: 0x089be5b4,
    classCount: 73,
    portraitCount: 98,
    hellSkills: 0x0850da44,
    variants: {
      265: { label: "Initial encounter", order: 0 },
      770: { label: "Rematch · Singles · No legends", order: 1 },
      771: { label: "Rematch · Singles · Legends", order: 2 },
      772: { label: "Rematch · Doubles · No legends", order: 3 },
      773: { label: "Rematch · Doubles · Legends", order: 4 },
      // Pinned map scripts select each rival's teams by the player's starter.
      // Each group lists May then Brendan, both in Mudkip/Treecko/Torchic order.
      ...Object.fromEntries(
        [
          { location: "Route 103", trainers: [529, 532, 535, 520, 523, 526] },
          { location: "Rustboro", trainers: [600, 768, 769, 593, 592, 599] },
          { location: "Route 110", trainers: [530, 533, 536, 521, 524, 527] },
          {
            location: "Meteor Falls · Partner",
            trainers: [87, 63, 70, 122, 104, 113],
          },
          { location: "Route 119", trainers: [531, 534, 537, 522, 525, 528] },
          {
            location: "Lilycove / Route 103 rematch",
            trainers: [664, 665, 666, 661, 662, 663],
          },
        ].flatMap(({ location, trainers }, encounter) =>
          trainers.map((id, slot) => [
            id,
            {
              label: `${location} · Your starter: ${["Mudkip", "Treecko", "Torchic"][slot % 3]}`,
              order: encounter * 3 + (slot % 3),
            },
          ]),
        ),
      ),
    },
  },
  artwork: {
    speciesCount: 2902,
    eggSpecies: 2901,
    unownSpecies: 201,
    unownFormsStart: 1603,
    iconTable: 0x08e1ffa4,
    iconPaletteIndices: 0x08e25a54,
    iconPalettes: 0x08e2684c,
    iconPaletteCount: 7,
    frontTable: 0x08955cac,
    palettes: 0x0857c66c,
    shinyPalettes: 0x0858211c,
    modernShinyPalettes: 0x0858211c,
    itemCount: 1023,
    itemIconTable: 0x08ae4b40,
    spindaSpecies: 327,
    deoxysSpecies: 386,
    trainerCount: 98,
    trainerFrontTable: 0x08963d64,
    trainerPalettes: 0x08964074,
  },
};

export function createEliteReduxGame(
  builds: readonly [EliteReduxBuild, ...EliteReduxBuild[]],
) {
  return definePreparedGame(({ rom, romHash }) => {
    const build = builds.find((build) => build.romHash === romHash);
    if (!build || build.layout !== "redux-2.65")
      throw new Error("Unsupported Redux build/layout");
    const catalog = createReduxCatalog(rom, build.catalog, romHash);
    return {
      catalog,
      linksFactory: build.linksFactory,
      artwork: createPokemonArtwork(rom, { ...build.artwork, romHash }),
      searchTrainers: createReduxTrainerSearch(
        rom,
        catalog,
        build.trainers,
        romHash,
      ),
    };
  })({
    id: "pokemon/elite-redux",
    profiles: builds,
    bridges: {
      gameTrainerContext: {
        id: "gameTrainerContext",
        decode({ memory, symbols }): DecodeResult<ReduxTrainerContext> {
          const settings = memory.read(symbols.settings);
          const badge = memory.read(symbols.thirdBadge);
          if (!settings || !badge)
            return unavailable("Trainer preview context is not initialized");
          const { difficulty, evsEnabled } = memory.read(settings);
          let highestLevel = 0;
          // The game scans all six slots, including fainted Pokémon, but not eggs.
          for (let slot = 0; slot < 6; slot++) {
            const record = at(symbols.playerParty, slot);
            if (
              !memory.u16(record.address + 16) ||
              memory.u32(record.address + 20) & 0x8000
            )
              continue;
            highestLevel = Math.max(highestLevel, memory.read(record).level);
          }
          if (!highestLevel)
            return unavailable(
              "Trainer previews need a non-egg player Pokémon",
            );
          return {
            status: "ready",
            data: {
              difficulty,
              evsEnabled,
              highestLevel,
              hellLevelBonus: difficulty === 3 && memory.read(badge) ? 5 : 0,
            },
          };
        },
      },
      gameParty: {
        id: "gameParty",
        decode({
          memory,
          symbols,
          resources,
          sample,
        }): DecodeResult<PokemonPartyView> {
          const count = memory.read(symbols.playerCount);
          if (count > 6) throw new Error("Invalid Redux party count");
          if (!count) return { status: "ready", data: { party: [] } };
          const settings = memory.read(symbols.settings);
          if (!settings)
            return unavailable("Redux settings are not initialized");
          const options = memory.read(settings);
          return {
            status: "ready",
            data: {
              party: Array.from({ length: count }, (_, slot) =>
                partyMember(
                  memory.read(at(symbols.playerParty, slot)),
                  slot,
                  resources.catalog,
                  options,
                  sample.romHash,
                  false,
                ),
              ),
            },
          };
        },
      },
      gameBattle: {
        id: "gameBattle",
        decode({
          memory,
          symbols,
          resources,
          sample,
        }): DecodeResult<PokemonBattleView> {
          if (!memory.read(symbols.battleActive))
            return { status: "ready", data: { active: false } };
          const settings = memory.read(symbols.settings);
          if (!settings)
            return unavailable("Redux settings are not initialized");
          const options = memory.read(settings);
          const playerCount = memory.read(symbols.playerCount);
          if (playerCount < 1 || playerCount > 6)
            throw new Error("Invalid Redux battle party count");
          const player = Array.from({ length: playerCount }, (_, i) =>
            memory.read(at(symbols.playerParty, i)),
          );
          // gEnemyPartyCount is not maintained in ordinary trainer battles.
          const enemy = Array.from({ length: 6 }, (_, i) => {
            const record = at(symbols.enemyParty, i);
            return memory.u16(record.address + 16) ? memory.read(record) : null;
          });
          const playerParty = player.map((mon, slot) =>
            partyMember(
              mon,
              slot,
              resources.catalog,
              options,
              sample.romHash,
              false,
            ),
          );
          const opponentParty = enemy.flatMap((mon, slot) =>
            mon
              ? [
                  partyMember(
                    mon,
                    slot,
                    resources.catalog,
                    options,
                    sample.romHash,
                    true,
                  ),
                ]
              : [],
          );
          if (!opponentParty.length)
            throw new Error("Redux opponent party is empty during battle");
          const count = memory.read(symbols.battlerCount);
          if (count !== 2 && count !== 4)
            throw new Error("Invalid Redux battler count");
          const absent = memory.read(symbols.absent);
          const battlers: Extract<
            PokemonBattleView,
            { active: true }
          >["battlers"][number][] = [];
          for (let battler = 0; battler < count; battler++) {
            if (absent & (1 << battler)) continue;
            const isEnemy = Boolean(battler & 1);
            const partySlot = memory.read(at(symbols.partyIndexes, battler));
            const original = (isEnemy ? enemy : player)[partySlot];
            const live = memory.read(at(symbols.battlers, battler));
            if (
              !original ||
              original.personality !== live.personality ||
              original.trainerId !== live.trainerId ||
              original.species !== live.species
            )
              throw new Error("Redux battler does not match its party record");
            const detail = details(
              { ...original, ...live },
              resources.catalog,
              {
                ...options,
                randomizedTypes: false,
                randomizedAbilities: false,
                randomizedInnates: false,
              },
              sample.romHash,
              isEnemy,
              live.level,
              live.calculatedStats,
            );
            const statuses3 = memory.read(at(symbols.statuses3, battler));
            const gas = Boolean(memory.read(symbols.neutralizingGas) & 1);
            const acid = Boolean(statuses3 & (1 << 16));
            const holdEffect =
              live.heldItem === 136
                ? memory.read(at(symbols.enigmaBerries, battler))
                : resources.catalog.holdEffect(live.heldItem);
            const shield =
              live.abilityIds
                .slice(0, 4)
                .some(
                  (id) => resources.catalog.abilityFlags(id).blocksSuppression,
                ) ||
              (holdEffect === 62 && !(statuses3 & (1 << 17)));
            // Persistent suppression only. Mold Breaker depends on the attack being evaluated.
            const abilities: PokemonAbility[] = live.abilityIds
              .slice(0, options.difficulty === 3 ? 7 : 4)
              .flatMap((id, slot): PokemonAbility[] => {
                if (!id) {
                  const locked = detail.abilities?.find(
                    (ability) =>
                      ability.slot === slot && ability.state === "locked",
                  );
                  return locked && !options.randomizedInnates ? [locked] : [];
                }
                const suppressed =
                  (gas || acid) &&
                  !shield &&
                  !resources.catalog.abilityFlags(id).unsuppressable;
                return [
                  {
                    ...resources.catalog.ability(id),
                    slot,
                    kind:
                      slot === 0 ? "ability" : slot < 4 ? "innate" : "extra",
                    state: suppressed ? "suppressed" : "available",
                    ...(suppressed
                      ? {
                          reason: [
                            gas ? "Neutralizing Gas" : null,
                            acid ? "Gastro Acid" : null,
                          ]
                            .filter(Boolean)
                            .join(" and "),
                        }
                      : {}),
                  },
                ];
              });
            battlers.push({
              ...detail,
              battler,
              side: isEnemy ? "opponent" : "player",
              partySlot,
              hp: live.hp,
              maxHp: live.maxHp,
              status: live.status,
              volatileStatus: live.volatileStatus,
              ailments: reduxAilments(live.status),
              statStages: live.statStages,
              stageAdjustedStats: applyStatStages(
                live.calculatedStats,
                live.statStages,
              ),
              types: [...new Set(live.typeIds)].map((id) =>
                resources.catalog.type(id),
              ),
              abilities,
            });
          }
          const typeFlags = memory.read(symbols.battleFlags);
          return {
            status: "ready",
            data: {
              active: true,
              kind: battleKind(typeFlags),
              format: typeFlags & 1 ? "double" : "single",
              typeFlags,
              types: [battleKind(typeFlags)],
              playerParty,
              opponentParty,
              battlers,
              turn: null,
              battleWeather: null,
              warnings: ["Battle turn/weather are not verified for this build"],
            },
          };
        },
      },
      gameStorage: {
        id: "gameStorage",
        decode({
          memory,
          symbols,
          resources,
          sample,
        }): DecodeResult<PokemonStorageView> {
          const root = memory.read(symbols.storage),
            settings = memory.read(symbols.settings);
          if (!root || !settings)
            return unavailable("Redux storage/settings are not initialized");
          const options = memory.read(settings);
          const storage = memory.read(root);
          const boxes = storage.boxes.map(({ box, name, pokemon }) => ({
            box,
            name,
            pokemon: pokemon.map(({ slot, record }) => ({
              ...details(
                record,
                resources.catalog,
                options,
                sample.romHash,
                false,
              ),
              slot,
            })),
          }));
          return {
            status: "ready",
            data: {
              currentBox: storage.currentBox,
              occupied: boxes.reduce(
                (count, box) => count + box.pokemon.length,
                0,
              ),
              boxes,
            },
          };
        },
      },
      gameContext: {
        id: "gameContext",
        decode({ memory, symbols }): DecodeResult<{ arena: GameArenaStatus }> {
          const settings = memory.read(symbols.settings);
          if (!settings)
            return unavailable("Redux settings are not initialized");
          const options = memory.read(settings);
          const active = memory.read(symbols.battleActive);
          return {
            status: "ready",
            data: {
              arena: {
                title: "Elite Redux",
                fields: [
                  {
                    id: "difficulty",
                    label: "Difficulty",
                    value: ["Easy", "Ace", "Elite", "Hell"][options.difficulty],
                  },
                  {
                    id: "battle",
                    label: "Battle",
                    value: active
                      ? battleKind(memory.read(symbols.battleFlags)).replace(
                          /^./,
                          (letter) => letter.toUpperCase(),
                        )
                      : "Overworld",
                  },
                  {
                    id: "weather",
                    label: "Weather",
                    value: null,
                    reason: "Weather is not verified for this build",
                  },
                ],
              },
            },
          };
        },
      },
    },
  });
}
export const eliteRedux = createEliteReduxGame([eliteReduxBeta2]);

const rules: PokemonRules = {
  legacyStats: false,
  typeOverrides: {},
  legacyAbilities: [],
  typesUnavailable: null,
  abilitiesUnavailable: null,
  baseStatsUnavailable: null,
  shinyThreshold: 0,
  shinyRequiresPersonality: false,
  modernShinySpecies: [],
};

export function reduxAbilities(
  record: ReduxBox,
  level: number,
  catalog: ReduxCatalog,
  settings: ReduxSettings,
  enemy: boolean,
): readonly PokemonAbility[] | null {
  if (!enemy && (settings.randomizedAbilities || settings.randomizedInnates))
    return null;
  const species = catalog.species(record.species);
  const main = species.abilities[record.abilitySlot] || species.abilities[0];
  const thresholds =
    settings.difficulty === 3
      ? [10, 17, 24]
      : settings.difficulty === 2
        ? [1, 17, 24]
        : [0, 0, 0];
  return [main, ...species.innates].flatMap((id, slot): PokemonAbility[] => {
    if (!id) return [];
    const locked = !enemy && slot > 0 && level < thresholds[slot - 1];
    return [
      {
        ...catalog.ability(id),
        slot,
        kind: slot ? "innate" : "ability",
        state: locked ? "locked" : "available",
        ...(locked
          ? {
              reason: `Unlocks at level ${thresholds[slot - 1]} on this difficulty`,
            }
          : {}),
      },
    ];
  });
}
function details(
  record: ReduxBox,
  catalog: ReduxCatalog,
  settings: ReduxSettings,
  romHash: string,
  enemy: boolean,
  level?: number,
  actual: PokemonStats | null = null,
): PokemonDetails {
  const value = pokemonDetails(
    { ...record, abilitySlot: 0 },
    catalog,
    {
      ...rules,
      typesUnavailable:
        !enemy && settings.randomizedTypes
          ? "Randomized party types are not resolved; live battle types remain authoritative"
          : null,
      abilitiesUnavailable:
        !enemy && (settings.randomizedAbilities || settings.randomizedInnates)
          ? "Randomized party abilities are not resolved; live battle abilities remain authoritative"
          : null,
    },
    romHash,
    level,
    actual,
  );
  return {
    ...value,
    abilities: reduxAbilities(record, value.level, catalog, settings, enemy),
    shiny: record.shinyTier !== 0,
    artwork:
      record.shinyTier > 1
        ? null
        : value.artwork && {
            icon: {
              ...value.artwork.icon,
              palette: record.shinyTier ? "shiny" : "normal",
            },
            front: {
              ...value.artwork.front,
              palette: record.shinyTier ? "shiny" : "normal",
            },
          },
    warnings: [
      ...value.warnings,
      ...(record.shinyTier > 1
        ? ["Rare/legendary shiny artwork is not verified"]
        : []),
    ],
  };
}
function partyMember(
  record: ReduxParty,
  slot: number,
  catalog: ReduxCatalog,
  settings: ReduxSettings,
  romHash: string,
  enemy: boolean,
): PokemonPartyMember {
  return {
    ...details(
      record,
      catalog,
      settings,
      romHash,
      enemy,
      record.level,
      record.calculatedStats,
    ),
    slot,
    hp: record.hp,
    maxHp: record.maxHp,
    status: record.status,
    ailments: reduxAilments(record.status),
  };
}
function reduxAilments(status: number) {
  return [
    ...ailments(status),
    ...(status & 0x1000 ? ["Frostbite"] : []),
    ...(status & 0x2000 ? ["Bleed"] : []),
  ];
}
function unavailable(reason: string) {
  return { status: "unavailable", reason } as const;
}
