import type { ComponentType, ReactNode } from "react";

/**
 * Props the ZiGBA player gives a game layout (frontend/src/games.ts): current
 * playback state and the self-contained player, which is rendered once.
 */
export interface GameLayoutProps {
  readonly playing: boolean;
  /** How the player knows the loaded build. */
  readonly build?: BuildInfo;
  readonly children: ReactNode;
}

export interface BuildInfo {
  /** Reviewed: a committed manifest. Located: recognized from the game's locators. */
  readonly provenance: "reviewed" | "located";
  /** For a located build, values found only by inference or kept from the template. */
  readonly review: readonly string[];
}

/** Layout components keyed by game definition `id`. */
export type GameLayouts = Readonly<
  Record<string, ComponentType<GameLayoutProps>>
>;
