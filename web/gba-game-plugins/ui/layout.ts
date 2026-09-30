import type { ComponentType, ReactNode } from "react";

/**
 * Props the Evenfall player gives a game layout (web/player/src/games.ts): current
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
  /**
   * Set on landscape pages narrower than 17:10: return `docked(dock)` instead
   * of placing `children`; the player places the heading and screen in one
   * column and the panels as tabs in one column beside it.
   */
  readonly docked?: (dock: GameDock) => ReactNode;
}

/** One of a layout's panels, shown as a tab. */
export interface GamePanel {
  readonly id: string;
  readonly label: string;
  readonly content: ReactNode;
}

/** A layout's contribution to the phone player. */
export interface HandheldHud {
  /** One non-interactive line under the screen; tapping it opens `panels`. */
  readonly summary: ReactNode;
  /** Tabs of the sheet (portrait) or drawer (landscape), in order. */
  readonly panels: readonly GamePanel[];
}

/** A layout's contribution to the two-column player. */
export interface GameDock {
  /** Above the screen, such as where the game takes place. */
  readonly heading?: ReactNode;
  /** Tabs of the column beside the screen, in order. */
  readonly panels: readonly GamePanel[];
  /** The panel the game's state calls for; the dock opens it whenever it changes. */
  readonly focus?: string;
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
