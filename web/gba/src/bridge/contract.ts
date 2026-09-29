/**
 * Read-only game definitions and synchronous snapshot dispatch.
 *
 * BridgeHost (./host) owns live capture; ./react provides useBridge. Published
 * as `@evenfall/gba/bridge` and `@evenfall/gba/bridge/react`.
 */

/** A fixed-size layout derived from the target game's build, not host C sizes. */
export interface MemoryLayout<T> {
  readonly byteLength: number;
  readonly decode: (memory: MemoryReader, address: number) => T;
}

/** A guest address with its pointee layout. This is never a WASM pointer. */
export interface MemoryRef<T> {
  readonly address: number;
  readonly layout: MemoryLayout<T>;
}

/**
 * Synchronous access to ONE captured sample (currently work RAM in the host).
 * Reads never fetch newer bytes or run the emulator. Invalid/uncaptured ranges
 * throw diagnostic errors; primitive reads are unaligned little-endian reads.
 */
export interface MemoryReader {
  u8(address: number): number;
  u16(address: number): number;
  u32(address: number): number;
  /** Returns an owned copy, not mutable access to the captured sample. */
  bytes(address: number, length: number): Uint8Array;
  /** Decoded values are owned by the caller; pointer layouts return refs. */
  read<T>(reference: MemoryRef<T>): T;
}

export type SymbolTable = Readonly<Record<string, MemoryRef<unknown>>>;

/** Generated/reviewed against the exact ROM; symbol names stay literal keys. */
export interface BuildProfile<Symbols extends SymbolTable> {
  /** Lowercase hexadecimal SHA-1 of the raw ROM, as No-Intro DATs list it. */
  readonly romHash: string;
  readonly source: {
    readonly repository: string;
    readonly revision: string;
    readonly variant: string;
    /** Compiler/toolchain identity and effective flags/defines for this ROM. */
    readonly compiler: string;
    readonly options: readonly string[];
  };
  readonly symbols: Symbols;
}

export interface BridgeSample {
  readonly romHash: string;
  /** Changes on game replacement, reset, state load and rewind, not speed. */
  readonly timeline: string;
  /** Monotonic within this timeline, including multiple samples at one cycle. */
  readonly sequence: number;
  /** Actual quiescent instruction/DMA endpoint, not a presentation timestamp. */
  readonly cycle: bigint;
}

export interface BridgeContext<
  Symbols extends SymbolTable,
  Resources = undefined,
> {
  readonly memory: MemoryReader;
  readonly symbols: Readonly<Symbols>;
  readonly sample: BridgeSample;
  /** Prepared once from the identity-checked ROM; never part of live state. */
  readonly resources: Resources;
}

/** Unavailable is normal during boot/transitions; it is not an empty party. */
export type DecodeResult<Data> =
  | { readonly status: "ready"; readonly data: Data }
  | { readonly status: "unavailable"; readonly reason: string };

export interface BridgeDefinition<
  Id extends string,
  Symbols extends SymbolTable,
  Result extends DecodeResult<unknown>,
  Resources = undefined,
> {
  readonly id: Id;
  /** Pure, synchronous decoding; no I/O, execution, writes or retained state. */
  readonly decode: (context: BridgeContext<Symbols, Resources>) => Result;
}

/** Reusable bridge, independent of any ROM identity or game collection. */
export function defineBridge<
  const Id extends string,
  Symbols extends SymbolTable,
  Result extends DecodeResult<unknown>,
  Resources = undefined,
>(
  definition: BridgeDefinition<Id, Symbols, Result, Resources>,
): BridgeDefinition<Id, Symbols, Result, Resources> {
  return definition;
}

export type BridgeMap<
  Symbols extends SymbolTable,
  Resources = undefined,
> = Readonly<
  Record<
    string,
    BridgeDefinition<string, Symbols, DecodeResult<unknown>, Resources>
  >
>;

export interface GameDefinition<
  Id extends string,
  Symbols extends SymbolTable,
  Bridges extends BridgeMap<Symbols, Resources>,
  Resources = undefined,
> {
  readonly id: Id;
  readonly profiles: readonly [
    BuildProfile<Symbols>,
    ...BuildProfile<Symbols>[],
  ];
  /** Synchronous, host-owned ROM preparation, separate from snapshot decoding. */
  readonly prepare?: (context: {
    readonly rom: MemoryReader;
    readonly romHash: string;
  }) => Resources;
  readonly bridges: Bridges;
}

/** Infer symbols from builds and each bridge's output from its decoder. */
export function defineGame<
  const Id extends string,
  Symbols extends SymbolTable,
  Bridges extends BridgeMap<NoInfer<Symbols>>,
>(
  game: GameDefinition<Id, Symbols, Bridges>,
): GameDefinition<Id, Symbols, Bridges> {
  return validateGame(game);
}

/** Fix resource types before inferring each decoder's context and output. */
export function definePreparedGame<Resources>(
  prepare: (context: {
    readonly rom: MemoryReader;
    readonly romHash: string;
  }) => Resources,
) {
  return function <
    const Id extends string,
    Symbols extends SymbolTable,
    Bridges extends BridgeMap<NoInfer<Symbols>, Resources>,
  >(
    game: GameDefinition<Id, Symbols, Bridges, Resources> & {
      readonly prepare?: never;
    },
  ): GameDefinition<Id, Symbols, Bridges, Resources> {
    return validateGame({ ...game, prepare });
  };
}

function validateGame<
  Game extends GameIdentity & {
    readonly bridges: Readonly<Record<string, { readonly id: string }>>;
  },
>(game: Game): Game {
  if (!game.id.trim() || !game.profiles.length)
    throw new Error("Invalid game definition");
  const hashes = new Set<string>();
  for (const profile of game.profiles) {
    if (!/^[a-f0-9]{40}$/.test(profile.romHash))
      throw new Error("Invalid ROM hash");
    if (hashes.has(profile.romHash)) throw new Error("Duplicate build hash");
    hashes.add(profile.romHash);
  }
  if (!Object.keys(game.bridges).length) throw new Error("Game has no bridges");
  for (const [key, bridge] of Object.entries(game.bridges)) {
    if (!key.trim() || key !== bridge.id)
      throw new Error("Bridge key must match its id");
  }
  return game;
}

// Selection only needs identity; the returned union retains each game's types.
export interface GameIdentity {
  readonly id: string;
  readonly profiles: readonly BuildProfile<SymbolTable>[];
}

export function resolveGame<const Games extends readonly GameIdentity[]>(
  games: Games,
  romHash: string,
): Games[number] | null {
  let match: Games[number] | null = null;
  const ids = new Set<string>();
  for (const game of games) {
    if (ids.has(game.id)) throw new Error("Duplicate game id");
    ids.add(game.id);
    if (!game.profiles.some((profile) => profile.romHash === romHash)) continue;
    if (match) throw new Error("ROM matches multiple game definitions");
    match = game;
  }
  return match;
}

// Published data is plain records, arrays, primitives and functions (such as a
// search bound to ROM resources). Authors must not return class instances,
// Maps or typed arrays.
type ReadonlyData<T> = T extends (...args: never[]) => unknown
  ? T
  : { readonly [Key in keyof T]: ReadonlyData<T[Key]> };
type DecodedData<Result> = Result extends { status: "ready"; data: infer Data }
  ? Data
  : never;

export type BridgeState<Data> =
  | { readonly status: "idle" }
  | { readonly status: "unsupported"; readonly romHash: string }
  | { readonly status: "loading" }
  | {
      readonly status: "unavailable";
      readonly sample: BridgeSample;
      readonly reason: string;
    }
  | {
      readonly status: "ready";
      readonly sample: BridgeSample;
      readonly data: ReadonlyData<Data>;
    }
  | {
      readonly status: "error";
      readonly message: string;
      readonly sample?: BridgeSample;
      readonly address?: number;
      readonly path?: string;
    };

/**
 * React hook contract, implemented by `useBridge` in `@evenfall/gba/bridge/react`.
 * The key is checked against this game's bridges, never a global string lookup.
 */
export type UseBridge = <
  Id extends string,
  Symbols extends SymbolTable,
  Resources,
  Bridges extends BridgeMap<Symbols, Resources>,
  Key extends keyof Bridges,
>(
  game: GameDefinition<Id, Symbols, Bridges, Resources>,
  key: Key,
) =>
  | BridgeState<DecodedData<ReturnType<NonNullable<Bridges[Key]>["decode"]>>>
  // An optional view a game does not define has no state.
  | (undefined extends Bridges[Key] ? undefined : never);

/** One state per bridge; an optional view a game leaves out is absent. */
export type GameData<Bridges extends BridgeMap<never, never>> = {
  readonly [Key in keyof Bridges]: BridgeState<
    DecodedData<ReturnType<NonNullable<Bridges[Key]>["decode"]>>
  >;
};

/** One accepted sample, one matched layout, all bridges. Scheduling stays in the host. */
export function decodeGame<
  Id extends string,
  Symbols extends SymbolTable,
  Resources,
  Bridges extends BridgeMap<Symbols, Resources>,
>(
  game: GameDefinition<Id, Symbols, Bridges, Resources>,
  sample: BridgeSample,
  memory: MemoryReader,
  resources?: Resources,
  keys: ReadonlySet<keyof Bridges> = new Set(Object.keys(game.bridges)),
): GameData<Bridges> {
  const profile = game.profiles.find(
    (candidate) => candidate.romHash === sample.romHash,
  );
  // The capture may also carry owned bytes; publish only its identity.
  const { romHash, timeline, sequence, cycle } = sample;
  const stamp = Object.freeze({ romHash, timeline, sequence, cycle });
  const states = Object.fromEntries(
    Object.entries(game.bridges).map(([key, bridge]) => {
      if (!keys.has(key)) return [key, Object.freeze({ status: "idle" })];
      if (!profile)
        return [
          key,
          Object.freeze({ status: "unsupported", romHash: stamp.romHash }),
        ];
      try {
        if (game.prepare && resources === undefined)
          return [
            key,
            copyData({
              status: "unavailable",
              reason: "ROM resources are not prepared",
              sample: stamp,
            }),
          ];
        const result = bridge.decode({
          memory,
          symbols: profile.symbols,
          sample: stamp,
          resources: resources as Resources,
        });
        if (result.status !== "ready" && result.status !== "unavailable")
          throw new Error("Invalid decode result");
        return [key, copyData({ ...result, sample: stamp })];
      } catch (error) {
        return [
          key,
          Object.freeze({
            status: "error",
            sample: stamp,
            message: error instanceof Error ? error.message : String(error),
          }),
        ];
      }
    }),
  );
  // Object iteration erases keys; every entry above preserves its decoder/key pair.
  return Object.freeze(states) as GameData<Bridges>;
}

/** Deep-copies and freezes records and arrays; functions pass through as is. */
function copyData<T>(value: T): ReadonlyData<T> {
  function copy(value: unknown, depth: number): unknown {
    if (value === null || typeof value !== "object") return value;
    if (depth > 64)
      throw new Error("Bridge data is cyclic or too deeply nested");
    if (Array.isArray(value))
      return Object.freeze(value.map((item) => copy(item, depth + 1)));
    const prototype = Object.getPrototypeOf(value);
    if (prototype !== Object.prototype && prototype !== null)
      throw new Error("Bridge data must contain plain records and arrays");
    return Object.freeze(
      Object.fromEntries(
        Object.entries(value).map(([key, item]) => [
          key,
          copy(item, depth + 1),
        ]),
      ),
    );
  }
  return copy(value, 0) as ReadonlyData<T>;
}
