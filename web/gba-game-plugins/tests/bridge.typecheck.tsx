// Compile-only author examples, checked against the real hook and decoders.
import { expectTypeOf } from "vitest";
import { useBridge } from "@evenfall/gba/bridge/react";
import {
  defineBridge,
  defineGame,
  definePreparedGame,
  type BridgeContext,
  type BuildProfile,
  type MemoryLayout,
  type MemoryRef,
} from "@evenfall/gba/bridge";
import { gameParty, type GamePartySymbols } from "../src/game-party";

interface Pokemon {
  species: number;
  hp: number;
  nickname: string;
}

declare const profile: BuildProfile<{
  gPlayerPartyCount: MemoryRef<number>;
  gPlayerParty: MemoryRef<readonly Pokemon[]>;
}>;

declare const generatedPartyProfile: BuildProfile<GamePartySymbols>;
const reusablePartyGame = defineGame({
  id: "pokemon/reusable-party",
  profiles: [generatedPartyProfile],
  bridges: { gameParty },
});

export function ReusablePartyPanel() {
  const state = useBridge(reusablePartyGame, "gameParty");
  if (state.status !== "ready") return null;
  expectTypeOf(state.data.party[0].slot).toEqualTypeOf<number>();
  expectTypeOf(state.data.party[0].species).toEqualTypeOf<number>();
  expectTypeOf(state.data.party[0].isEgg).toEqualTypeOf<boolean>();
  // @ts-expect-error The stock decoder does not yet expose localized names.
  state.data.party[0].nickname;
  return <span>{state.data.party[0]?.hp}</span>;
}

const partyGame = defineGame({
  id: "pokemon/party",
  profiles: [profile],
  bridges: {
    gameParty: {
      id: "gameParty",
      decode({ memory, symbols, sample }) {
        expectTypeOf(sample.cycle).toEqualTypeOf<bigint>();
        const count = memory.read(symbols.gPlayerPartyCount);
        const party = memory.read(symbols.gPlayerParty);
        expectTypeOf(count).toEqualTypeOf<number>();
        expectTypeOf(party).toEqualTypeOf<readonly Pokemon[]>();
        if (count > party.length) {
          return { status: "unavailable", reason: "Party is being updated" };
        }
        return { status: "ready", data: { party: party.slice(0, count) } };
      },
    },
    combat: {
      id: "combat",
      decode: () => ({ status: "ready", data: { active: true } }),
    },
  },
});

export function PartyPanel() {
  const state = useBridge(partyGame, "gameParty");
  expectTypeOf(partyGame.id).toEqualTypeOf<"pokemon/party">();
  const combat = useBridge(partyGame, "combat");
  if (combat.status === "ready") {
    expectTypeOf(combat.data.active).toEqualTypeOf<boolean>();
    // @ts-expect-error Sibling bridge outputs are not merged or widened.
    combat.data.party;
  }
  // @ts-expect-error Keys are checked within the specified game collection.
  useBridge(partyGame, "trainer");
  if (state.status !== "ready") {
    // @ts-expect-error There is no current data before successful decoding.
    state.data;
    return null;
  }

  expectTypeOf(state.data.party[0].hp).toEqualTypeOf<number>();
  expectTypeOf(state.data.party[0].nickname).toEqualTypeOf<string>();
  // @ts-expect-error Decoder output, not a global registry, defines the fields.
  state.data.money;
  // @ts-expect-error A decoded number must not become any or unknown.
  const hp: string = state.data.party[0].hp;
  // @ts-expect-error Published view data is deeply read-only.
  state.data.party[0].hp = 100;
  // @ts-expect-error Published collections cannot be changed by a panel.
  state.data.party.push({ species: 25, hp: 100, nickname: "Pika" });

  return <span>{state.data.party[0]?.nickname}</span>;
}

interface SaveBlock {
  trainer: { name: string; money: number };
}

declare const pointerProfile: BuildProfile<{
  gSaveBlock1Ptr: MemoryRef<MemoryRef<SaveBlock> | null>;
}>;
declare const pointerLayout: MemoryLayout<MemoryRef<SaveBlock> | null>;
// Constructed independently, as a generator would emit it. Synthetic addresses
// and identities are type fixtures, not a supported game profile.
const otherPointerBuild = {
  romHash: "b".repeat(40),
  source: {
    repository: "synthetic",
    revision: "fixture-v2",
    variant: "pointer-build",
    compiler: "synthetic",
    options: ["fixture-only"],
  },
  symbols: {
    gSaveBlock1Ptr: { address: 0x03000040, layout: pointerLayout },
  },
};

const trainerBridge = defineBridge({
  id: "trainer",
  decode({ memory, symbols }: BridgeContext<typeof pointerProfile.symbols>) {
    // @ts-expect-error Only symbols in this profile are available.
    symbols.gPlayerParty;
    const blockRef = memory.read(symbols.gSaveBlock1Ptr);
    // @ts-expect-error A pointer is not automatically decoded as its target.
    blockRef?.trainer;
    // @ts-expect-error Null must be checked before following a pointer.
    memory.read(blockRef);
    if (blockRef === null) {
      return { status: "unavailable", reason: "Save block is not initialized" };
    }
    const block = memory.read(blockRef);
    expectTypeOf(block).toEqualTypeOf<SaveBlock>();
    // @ts-expect-error There is no mutation capability in bridge context.
    memory.write(symbols.gSaveBlock1Ptr, 0);
    return { status: "ready", data: block.trainer };
  },
});

const trainerGame = defineGame({
  id: "pokemon/trainer",
  profiles: [pointerProfile, otherPointerBuild],
  bridges: { trainer: trainerBridge },
});

export function TrainerPanel() {
  const state = useBridge(trainerGame, "trainer");
  // @ts-expect-error There is no untyped global string lookup.
  useBridge("pokemon/trainer");
  if (state.status !== "ready") return null;
  expectTypeOf(state.data.money).toEqualTypeOf<number>();
  // @ts-expect-error Another bridge must not leak its output into this one.
  state.data.party;
  return <span>{state.data.name}</span>;
}

defineBridge({
  id: "invalid/async",
  // @ts-expect-error Decoding must finish against one synchronous snapshot.
  async decode() {
    return { status: "ready", data: { count: 1 } };
  },
});

defineGame({
  id: "invalid/profiles",
  // @ts-expect-error At least one verified build is required.
  profiles: [],
  bridges: {},
});

// @ts-expect-error The bridge id is required, not inferred from game identity.
defineBridge({
  decode: () => ({ status: "ready", data: {} }),
});

defineGame({
  id: "invalid/mixed-symbols",
  // @ts-expect-error Builds sharing a decoder must share its symbol contract.
  profiles: [profile, pointerProfile],
  bridges: {},
});

defineGame({
  id: "invalid/bridge-symbols",
  profiles: [profile],
  bridges: {
    // @ts-expect-error A reusable bridge cannot require symbols this game lacks.
    trainer: trainerBridge,
  },
});

const phaseGame = defineGame({
  id: "pokemon/phase",
  profiles: [profile],
  bridges: {
    phase: {
      id: "phase",
      decode({ memory, symbols }) {
        if (memory.read(symbols.gPlayerPartyCount) === 0) {
          return { status: "ready", data: { phase: "boot" as const } };
        }
        return {
          status: "ready",
          data: {
            phase: "playing" as const,
            party: memory.read(symbols.gPlayerParty),
          },
        };
      },
    },
  },
});

export function PhasePanel() {
  const state = useBridge(phaseGame, "phase");
  if (state.status !== "ready") return null;
  expectTypeOf(state.data.phase).toEqualTypeOf<"boot" | "playing">();
  if (state.data.phase === "playing") {
    expectTypeOf(state.data.party[0].species).toEqualTypeOf<number>();
    return <span>{state.data.party.length}</span>;
  }
  return null;
}

const preparedGame = definePreparedGame(({ rom, romHash }) => ({
  title: `${romHash}:${rom.u8(0x08000000)}`,
}))({
  id: "prepared",
  profiles: [profile],
  bridges: {
    named: {
      id: "named",
      decode({ resources, memory, symbols }) {
        expectTypeOf(resources.title).toEqualTypeOf<string>();
        // @ts-expect-error Resources do not widen to any.
        resources.artwork;
        return {
          status: "ready",
          data: {
            name: resources.title,
            count: memory.read(symbols.gPlayerPartyCount),
          },
        };
      },
    },
    other: {
      id: "other",
      decode: () => ({ status: "ready", data: { flag: true } }),
    },
  },
});

export function PreparedPanel() {
  const state = useBridge(preparedGame, "named");
  if (state.status !== "ready") return null;
  expectTypeOf(state.data.name).toEqualTypeOf<string>();
  expectTypeOf(state.data.count).toEqualTypeOf<number>();
  // @ts-expect-error Sibling data does not leak.
  state.data.flag;
  // @ts-expect-error Prepared games still check keys.
  useBridge(preparedGame, "missing");
  // @ts-expect-error Published data remains immutable.
  state.data.name = "changed";
  return null;
}

const needsCatalog = defineBridge({
  id: "catalog",
  decode({
    resources,
  }: BridgeContext<typeof profile.symbols, { catalog: string }>) {
    return { status: "ready", data: resources.catalog };
  },
});
defineGame({
  id: "missing-resources",
  profiles: [profile],
  bridges: {
    // @ts-expect-error An unprepared game cannot register a resource-requiring bridge.
    catalog: needsCatalog,
  },
});
definePreparedGame(() => ({ title: "not a catalog" }))({
  id: "wrong-resources",
  profiles: [profile],
  bridges: {
    // @ts-expect-error Prepared resources must satisfy reusable decoders.
    catalog: needsCatalog,
  },
});
