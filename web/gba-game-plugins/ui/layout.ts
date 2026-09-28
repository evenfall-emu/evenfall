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
  /**
   * Set on phones: return `handheld(hud)` instead of placing `children`; the
   * player places the screen and controls and shows the HUD around them.
   */
  readonly handheld?: (hud: HandheldHud) => ReactNode;
}

/** A layout's contribution to the phone player. */
export interface HandheldHud {
  /** One non-interactive line under the screen; tapping it opens `panels`. */
  readonly summary: ReactNode;
  /** Tabs of the sheet (portrait) or drawer (landscape), in order. */
  readonly panels: readonly {
    readonly id: string;
    readonly label: string;
    readonly content: ReactNode;
  }[];
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
