import type { DecodeResult, MemoryReader } from "@evenfall/gba/bridge";
import type { ReduxCatalog } from "./redux-catalog";
import { searchKey } from "./search";
import { decodePokemonText } from "./text";
import {
  statKeys,
  type PokemonStats,
  type PokemonPartyMember,
  type PokemonTrainerPreview,
} from "./model";

export interface ReduxTrainerContext {
  readonly difficulty: number;
  readonly highestLevel: number;
  readonly hellLevelBonus: number;
  readonly evsEnabled: boolean;
}

/** Release-specific roots; struct layouts belong to the Redux 2.65 family. */
export interface ReduxTrainerCatalogConfig {
  readonly trainers: number;
  readonly trainerCount: number;
  readonly classes: number;
  readonly classCount: number;
  readonly portraitCount: number;
  readonly hellSkills: number;
  /** Verified encounter labels and progression within a trainer's name group. */
  readonly variants?: Readonly<
    Record<number, { readonly label: string; readonly order: number }>
  >;
}

interface Trainer {
  readonly internalId: number;
  readonly name: string;
  readonly className: string;
  readonly pic: number;
  readonly parties: readonly { count: number; address: number }[];
  readonly skills: readonly { event: number; value: number }[];
  readonly variant: string | null;
}

/** Lazy ROM index plus pure, context-resolved search. Never executes or mutates the game. */
export function createReduxTrainerSearch(
  rom: MemoryReader,
  catalog: ReduxCatalog,
  config: ReduxTrainerCatalogConfig,
  romHash: string,
): (
  query: string,
  context: ReduxTrainerContext,
) => DecodeResult<readonly PokemonTrainerPreview[]> {
  let index: readonly Trainer[] | undefined;

  function text(address: number, length: number) {
    if (address < 0x08000000 || address >= 0x0a000000)
      throw new Error("Invalid trainer text pointer");
    const bytes = rom.bytes(address, length);
    if (!bytes.includes(255)) throw new Error("Unterminated trainer text");
    const value = decodePokemonText(bytes);
    if (value.warnings.length) throw new Error(value.warnings.join("; "));
    return value.text;
  }

  function trainers(): readonly Trainer[] {
    if (index) return index;
    const entries: Trainer[] = [];
    for (let internalId = 1; internalId < config.trainerCount; internalId++) {
      const address = config.trainers + internalId * 0x38;
      const parties = [0x18, 0x20, 0x28].map((offset) => {
        const count = rom.u8(address + offset);
        const pointer = rom.u32(address + offset + 4);
        if (
          count > 6 ||
          (count &&
            (pointer < 0x08000000 || pointer >= 0x0a000000 || pointer % 2))
        )
          throw new Error(`Invalid trainer party ${internalId}`);
        return { count, address: pointer };
      });
      if (parties.every((party) => !party.count)) continue;
      const trainerClass = rom.u8(address + 1);
      const pic = rom.u8(address + 3);
      if (trainerClass >= config.classCount || pic >= config.portraitCount)
        throw new Error(`Invalid trainer metadata ${internalId}`);
      entries.push({
        internalId,
        parties,
        pic,
        name: text(rom.u32(address + 4), 32),
        className: text(config.classes + trainerClass * 13, 13),
        skills: Array.from({ length: 4 }, (_, slot) => {
          const address = config.hellSkills + internalId * 24 + slot * 6;
          return { event: rom.u16(address), value: rom.u16(address + 2) };
        }).filter((skill) => skill.event !== 0),
        variant: config.variants?.[internalId]?.label ?? null,
      });
    }
    // IDs are allocation order, not chronology. Keep unverified encounters in
    // ID order after verified ones; the shared UI preserves this bridge order.
    entries.sort(
      (a, b) =>
        a.name.localeCompare(b.name, "en") ||
        (config.variants?.[a.internalId]?.order ?? Number.MAX_SAFE_INTEGER) -
          (config.variants?.[b.internalId]?.order ?? Number.MAX_SAFE_INTEGER) ||
        a.internalId - b.internalId,
    );
    const counts = new Map<string, number>();
    for (const trainer of entries)
      counts.set(trainer.name, (counts.get(trainer.name) ?? 0) + 1);
    index = entries.map((trainer) => ({
      ...trainer,
      variant:
        trainer.variant ??
        ((counts.get(trainer.name) ?? 0) > 1
          ? `Encounter #${trainer.internalId}`
          : null),
    }));
    return index;
  }

  function member(
    address: number,
    slot: number,
    trainer: Trainer,
    context: ReduxTrainerContext,
  ): PokemonPartyMember {
    const speciesId = rom.u16(address + 2);
    const species = catalog.species(speciesId);
    const rawOffset = rom.u8(address);
    const offset = rawOffset > 127 ? rawOffset - 256 : rawOffset;
    const level = Math.min(
      100,
      Math.max(1, context.highestLevel + offset) +
        (context.difficulty === 3 ? context.hellLevelBonus : 0),
    );
    const nature = rom.u8(address + 0x17);
    const abilitySlot = rom.u8(address + 6);
    const zeroSpeed = rom.u8(address + 0x18);
    if (
      nature >= 25 ||
      abilitySlot >= 3 ||
      zeroSpeed > 1 ||
      rom.u8(address + 0x19) > 1
    )
      throw new Error(
        "Unsupported trainer Pokémon nature/ability/Alpha record",
      );
    const evOrder = [
      "hp",
      "attack",
      "defense",
      "specialAttack",
      "specialDefense",
      "speed",
    ] as const;
    const evs = Object.fromEntries(
      evOrder.map((key, i) => [
        key,
        context.difficulty === 0 || !context.evsEnabled
          ? 0
          : rom.u8(address + 0x11 + i),
      ]),
    ) as PokemonStats;
    const ivs = Object.fromEntries(
      statKeys.map((key) => [key, key === "speed" && zeroSpeed ? 0 : 31]),
    ) as PokemonStats;
    const actual = Object.fromEntries(
      statKeys.map((key, i) => {
        const value = Math.floor(
          ((species.base[key] * 2 + ivs[key] + Math.floor(evs[key] / 4)) *
            level) /
            100,
        );
        if (key === "hp")
          return [
            key,
            speciesId === 292 || speciesId === 2140 ? 1 : value + level + 10,
          ];
        const raised = Math.floor(nature / 5),
          lowered = nature % 5;
        const factor =
          raised === lowered
            ? 10
            : raised === i - 1
              ? 11
              : lowered === i - 1
                ? 9
                : 10;
        return [key, Math.floor(((value + 5) * factor) / 10)];
      }),
    ) as PokemonStats;
    const heldItem = rom.u16(address + 4);
    const mainAbility = species.abilities[abilitySlot] || species.abilities[0];
    const extras =
      context.difficulty === 3
        ? [232, 233, 234].map(
            (event) =>
              trainer.skills.find((skill) => skill.event === event)?.value ?? 0,
          )
        : [];
    const artwork = {
      romHash,
      species: speciesId,
      personality: null,
      isEgg: false,
      palette: "normal",
    } as const;
    return {
      id: `${romHash}:trainer:${trainer.internalId}:slot:${slot}`,
      slot,
      species: speciesId,
      speciesName: species.name,
      nickname: "",
      language: 2,
      level,
      isEgg: false,
      gender: null,
      shiny: false,
      nature: catalog.nature(nature),
      types: [...new Set(species.types)].map((id) => catalog.type(id)),
      abilities: [mainAbility, ...species.innates, ...extras].flatMap(
        (id, slot) =>
          id
            ? [
                {
                  ...catalog.ability(id),
                  slot,
                  kind:
                    slot === 0
                      ? ("ability" as const)
                      : slot < 4
                        ? ("innate" as const)
                        : ("extra" as const),
                  state: "available" as const,
                },
              ]
            : [],
      ),
      heldItem: heldItem ? catalog.item(heldItem) : null,
      hp: actual.hp,
      maxHp: actual.hp,
      status: 0,
      ailments: [],
      stats: { actual, base: species.base, ivs, evs },
      moves: Array.from({ length: 4 }, (_, slot) => {
        const id = rom.u16(address + 8 + slot * 2);
        if (!id) return null;
        const move = catalog.move(id);
        const pp = move.pp + Math.floor((move.pp * 3) / 5);
        return { ...move, slot, pp, maxPp: pp };
      }),
      artwork: {
        icon: { ...artwork, kind: "icon" },
        front: { ...artwork, kind: "front" },
      },
      warnings: [],
    };
  }

  return (query, context) => {
    try {
      if (
        !Number.isInteger(context.difficulty) ||
        context.difficulty < 0 ||
        context.difficulty > 3 ||
        !Number.isInteger(context.highestLevel) ||
        context.highestLevel < 1 ||
        context.highestLevel > 100 ||
        ![0, 5].includes(context.hellLevelBonus)
      )
        return {
          status: "unavailable",
          reason: "Trainer preview context is not initialized",
        };
      const tokens = searchKey(query).split(" ").filter(Boolean);
      if (!tokens.length) return { status: "ready", data: [] };
      const partyIndex = Math.max(0, context.difficulty - 1);
      const data = trainers()
        .filter((trainer) => {
          const searchable = searchKey(
            `${trainer.name} ${trainer.className} ${trainer.variant ?? ""}`,
          );
          return (
            trainer.parties[partyIndex].count &&
            tokens.every((token) => searchable.includes(token))
          );
        })
        .map((trainer): PokemonTrainerPreview => {
          const party = trainer.parties[partyIndex];
          const team = Array.from({ length: party.count }, (_, slot) =>
            member(party.address + slot * 0x1c, slot, trainer, context),
          );
          return {
            id: `${romHash}:trainer:${trainer.internalId}`,
            internalId: trainer.internalId,
            name: trainer.name,
            variant: trainer.variant,
            className: trainer.className,
            portrait: { kind: "trainer", romHash, trainerPic: trainer.pic },
            team,
            averageLevel:
              team.reduce((sum, mon) => sum + mon.level, 0) / team.length,
            warnings:
              context.difficulty === 3
                ? trainer.skills
                    .filter((skill) => skill.event < 232 || skill.event > 234)
                    .map(
                      (skill) =>
                        `Hell ${skill.event === 231 ? "Eviolite skill" : `battle rule #${skill.event}`} is a battle-only effect, not included in these starting stats.`,
                    )
                : [],
          };
        });
      return { status: "ready", data };
    } catch (error) {
      return {
        status: "unavailable",
        reason: error instanceof Error ? error.message : String(error),
      };
    }
  };
}
