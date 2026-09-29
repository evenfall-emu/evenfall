// `@evenfall/gba/bridge/react`: React bindings for a session's BridgeHost.
// React is an optional peer dependency; nothing else in the SDK imports it.
import {
  createContext,
  createElement,
  useCallback,
  useContext,
  useSyncExternalStore,
  type ReactNode,
} from "react";
import type {
  BridgeMap,
  GameData,
  GameDefinition,
  SymbolTable,
  UseBridge,
} from "./contract.js";
import type { BridgeHost } from "./host.js";

const HostContext = createContext<BridgeHost | undefined>(undefined);
const idle = Object.freeze({ status: "idle" as const });
const noSubscribe = () => () => {};

/** Undefined while the host session is initializing; hooks report idle. */
export function BridgeProvider({
  host,
  children,
}: {
  host: BridgeHost | undefined;
  children?: ReactNode;
}) {
  return createElement(HostContext.Provider, { value: host }, children);
}

/** Subscribe to one bridge of `game`; the key is checked against that game. */
export const useBridge: UseBridge = (game, key) => {
  const host = useContext(HostContext);
  const store = host?.observe(game);
  const subscribe = useCallback(
    (listener: () => void) => store?.subscribe(listener, key) ?? noSubscribe(),
    [store, key],
  );
  return useSyncExternalStore(
    subscribe,
    () => (store ? store.getSnapshot()[key] : idle),
    () => idle,
  );
};

/** ROM-only resources paired with an observed decoder, never a fresh memory read. */
export function useBridgeResources<
  Id extends string,
  Symbols extends SymbolTable,
  Resources,
  Bridges extends BridgeMap<Symbols, Resources>,
>(
  game: GameDefinition<Id, Symbols, Bridges, Resources>,
  key: keyof Bridges,
): Resources | undefined {
  const host = useContext(HostContext);
  const store = host?.observe(game);
  const subscribe = useCallback(
    (listener: () => void) => store?.subscribe(listener, key) ?? noSubscribe(),
    [store, key],
  );
  return useSyncExternalStore(
    subscribe,
    () => store?.getResources(),
    () => undefined,
  );
}

/**
 * Every view of `game` at once, for an overlay: `const views = useViews(game)`.
 * Each entry is a BridgeState; views a game does not define are absent.
 */
export function useViews<
  Id extends string,
  Symbols extends SymbolTable,
  Resources,
  Bridges extends BridgeMap<Symbols, Resources>,
>(
  game: GameDefinition<Id, Symbols, Bridges, Resources>,
): GameData<Bridges> | undefined {
  const host = useContext(HostContext);
  const store = host?.observe(game);
  const subscribe = useCallback(
    (listener: () => void) => store?.subscribe(listener) ?? noSubscribe(),
    [store],
  );
  return useSyncExternalStore(
    subscribe,
    () => store?.getSnapshot(),
    () => undefined,
  );
}
