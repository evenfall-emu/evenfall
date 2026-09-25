import type { ComponentType, ReactNode } from "react";

/**
 * Props the ZiGBA player gives a game layout (frontend/src/games.ts): current
 * playback state and the self-contained player, which is rendered once.
 */
export interface GameLayoutProps {
  readonly playing: boolean;
  readonly children: ReactNode;
}

/** Layout components keyed by game definition `id`. */
export type GameLayouts = Readonly<
  Record<string, ComponentType<GameLayoutProps>>
>;
