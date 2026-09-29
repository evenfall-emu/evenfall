import {
  decodeGame,
  resolveGame,
  type BridgeMap,
  type BridgeState,
  type BuildProfile,
  type GameData,
  type GameDefinition,
  type MemoryReader,
  type SymbolTable,
} from "./contract.js";
import { createMemoryReader } from "./memory.js";
import type { MemorySnapshot } from "@evenfall/gba";

export interface BridgeStore<Data, Resources = undefined> {
  /** Omit key to observe the full collection; hooks subscribe to one decoder. */
  readonly subscribe: (listener: () => void, key?: keyof Data) => () => void;
  readonly getSnapshot: () => Data;
  readonly getResources: () => Resources | undefined;
}

type PendingState = Extract<
  BridgeState<never>,
  { status: "idle" | "loading" | "error" }
>;
interface Observation {
  readonly game: {
    readonly id: string;
    readonly profiles: readonly BuildProfile<SymbolTable>[];
  };
  needsRom(hash: string): boolean;
  reset(state: PendingState): void;
  decode(
    sample: MemorySnapshot,
    memory: MemoryReader,
    rom?: MemoryReader,
  ): void;
}

/** One per emulator session. Trusted decoders run outside the emulator worker. */
export class BridgeHost {
  private readonly stores = new WeakMap<object, unknown>();
  private readonly active = new Set<Observation>();
  private timer?: ReturnType<typeof setTimeout>;
  private inFlight = false;
  private dirty = false;
  private epoch = 0;
  private timeline: string | null | undefined;
  private sequence = 0;
  private lastStart = -Infinity;
  private closed = false;
  private rom?: { hash: string; memory: MemoryReader };

  constructor(
    private readonly emulator: EventTarget & {
      captureMemory(): Promise<MemorySnapshot | null>;
      exportRom?(): Promise<Uint8Array>;
    },
  ) {
    this.emulator.addEventListener("close", this.dispose);
  }

  observe<
    Id extends string,
    Symbols extends SymbolTable,
    Resources,
    Bridges extends BridgeMap<Symbols, Resources>,
  >(
    game: GameDefinition<Id, Symbols, Bridges, Resources>,
  ): BridgeStore<GameData<Bridges>, Resources> {
    // The identity-keyed cache always stores the type created for this game.
    const cached = this.stores.get(game) as
      | BridgeStore<GameData<Bridges>, Resources>
      | undefined;
    if (cached) return cached;
    const listeners = new Set<{ listener: () => void; key?: keyof Bridges }>();
    const observedKeys = () =>
      new Set<keyof Bridges>(
        [...listeners].flatMap(({ key }) =>
          key === undefined ? Object.keys(game.bridges) : [key],
        ),
      );
    let state: GameData<Bridges>;
    let prepared: { hash: string; value: Resources } | undefined;
    let resources: Resources | undefined;
    const publish = (next: GameData<Bridges>) => {
      state = next;
      listeners.forEach(({ listener }) => listener());
    };
    const observation: Observation = {
      game,
      needsRom: (hash) =>
        !!game.prepare && game.profiles.some((p) => p.romHash === hash),
      reset: (value) => {
        resources = undefined;
        if (value.status === "idle") prepared = undefined;
        const keys = observedKeys();
        publish(
          Object.freeze(
            Object.fromEntries(
              Object.keys(game.bridges).map((key) => [
                key,
                Object.freeze(keys.has(key) ? value : { status: "idle" }),
              ]),
            ),
          ) as GameData<Bridges>,
        );
      },
      decode: (sample, memory, rom) => {
        const profile = game.profiles.find((p) => p.romHash === sample.romHash);
        if (prepared?.hash !== sample.romHash) prepared = undefined;
        if (game.prepare && profile && rom && prepared?.hash !== sample.romHash)
          prepared = {
            hash: sample.romHash,
            value: game.prepare({ rom, romHash: profile.romHash }),
          };
        resources =
          profile && prepared?.hash === sample.romHash
            ? prepared.value
            : undefined;
        publish(decodeGame(game, sample, memory, resources, observedKeys()));
      },
    };
    observation.reset({ status: "idle" });
    const store: BridgeStore<GameData<Bridges>, Resources> = {
      getSnapshot: () => state,
      getResources: () => resources,
      subscribe: (listener, key) => {
        if (this.closed) return () => {};
        const subscription = { listener, key };
        listeners.add(subscription);
        if (listeners.size === 1) {
          observation.reset({ status: "loading" });
          this.active.add(observation);
          if (this.active.size === 1) {
            this.emulator.addEventListener("frame", this.request);
            this.emulator.addEventListener("clear", this.clear);
          }
        }
        this.request();
        return () => {
          listeners.delete(subscription);
          if (!listeners.size) {
            this.active.delete(observation);
            observation.reset({ status: "idle" });
            if (!this.active.size) this.detach();
          } else {
            const keys = observedKeys();
            publish(
              Object.freeze(
                Object.fromEntries(
                  Object.entries(state).map(([key, value]) => [
                    key,
                    keys.has(key) ? value : Object.freeze({ status: "idle" }),
                  ]),
                ),
              ) as GameData<Bridges>,
            );
          }
        };
      },
    };
    this.stores.set(game, store);
    return store;
  }

  private clear = (event: Event) => {
    const { timeline } = (event as CustomEvent<{ timeline: string | null }>)
      .detail;
    if (timeline !== this.timeline) {
      this.epoch++;
      this.timeline = timeline;
      this.sequence = 0;
      if (timeline === null) this.rom = undefined;
      this.active.forEach((store) =>
        store.reset({ status: timeline === null ? "idle" : "loading" }),
      );
    }
    this.request();
  };

  private request = () => {
    if (this.closed || !this.active.size) return;
    this.dirty = true;
    this.schedule();
  };

  private schedule() {
    if (!this.dirty || this.inFlight || this.timer !== undefined) return;
    this.timer = setTimeout(
      () => void this.capture(),
      Math.max(0, 50 - (performance.now() - this.lastStart)),
    );
  }

  private async capture() {
    this.timer = undefined;
    this.inFlight = true;
    this.dirty = false;
    this.lastStart = performance.now();
    const epoch = this.epoch;
    let captured: MemorySnapshot | null = null;
    try {
      const sample = await this.emulator.captureMemory();
      captured = sample;
      if (epoch !== this.epoch) return;
      if (!sample) {
        this.rom = undefined;
        this.active.forEach((store) => store.reset({ status: "idle" }));
        return;
      }
      // Clear events identify replacement before its reply; old replies cannot
      // restore data. On first subscription we learn identity from the capture.
      if (this.timeline === undefined) this.timeline = sample.timeline;
      if (sample.timeline !== this.timeline || sample.sequence <= this.sequence)
        return;
      this.sequence = sample.sequence;
      resolveGame(
        [...this.active].map((store) => store.game),
        sample.romHash,
      );
      if (this.rom?.hash !== sample.romHash) this.rom = undefined;
      if (
        !this.rom &&
        [...this.active].some((store) => store.needsRom(sample.romHash))
      ) {
        if (!this.emulator.exportRom)
          throw new Error("ROM export is unavailable");
        const bytes = await this.emulator.exportRom();
        const digest = new Uint8Array(
          await crypto.subtle.digest("SHA-1", bytes.slice().buffer),
        );
        const hash = Array.from(digest, (byte) =>
          byte.toString(16).padStart(2, "0"),
        ).join("");
        if (epoch !== this.epoch) return;
        if (hash !== sample.romHash)
          throw new Error("ROM changed during resource preparation");
        this.rom = {
          hash,
          memory: createMemoryReader([{ address: 0x08000000, bytes }]),
        };
      }
      const memory = createMemoryReader(sample.regions);
      this.active.forEach((store) =>
        store.decode(sample, memory, this.rom?.memory),
      );
    } catch (error) {
      if (epoch === this.epoch) {
        const stamp =
          captured &&
          Object.freeze({
            romHash: captured.romHash,
            timeline: captured.timeline,
            sequence: captured.sequence,
            cycle: captured.cycle,
          });
        this.active.forEach((store) =>
          store.reset({
            status: "error",
            message: error instanceof Error ? error.message : String(error),
            ...(stamp ? { sample: stamp } : {}),
          }),
        );
      }
    } finally {
      this.inFlight = false;
      this.schedule();
    }
  }

  private detach() {
    this.epoch++;
    this.rom = undefined;
    this.dirty = false;
    this.timeline = undefined;
    this.sequence = 0;
    clearTimeout(this.timer);
    this.timer = undefined;
    this.emulator.removeEventListener("frame", this.request);
    this.emulator.removeEventListener("clear", this.clear);
  }

  readonly dispose = () => {
    if (this.closed) return;
    this.closed = true;
    this.detach();
    this.rom = undefined;
    this.emulator.removeEventListener("close", this.dispose);
    this.active.forEach((store) => store.reset({ status: "idle" }));
    this.active.clear();
  };
}
