import type { ReactNode } from "react";
import { gameRegistry } from "./games";

/** Whether an installed plugin has a Dev / Debug HUD for this exact build. */
export function hasHud(romHash: string | undefined) {
  return gameRegistry.layoutForRom("huds", romHash) !== null;
}

/** The Dev / Debug page's layout: a plugin HUD for a supported ROM, else generic. */
export function GameExperience({
  romHash,
  playing,
  libraryOpen,
  children,
  forceEnhanced = false,
}: {
  romHash?: string;
  playing: boolean;
  libraryOpen: boolean;
  children: ReactNode;
  /** Keeps the diagnostics route useful for unsupported-profile states. */
  forceEnhanced?: boolean;
}) {
  const experience =
    gameRegistry.layoutForRom("huds", romHash) ??
    (forceEnhanced ? gameRegistry.anyLayout("huds") : null);
  if (experience) {
    const { Layout } = experience;
    return (
      <Layout key={experience.game.id} playing={playing}>
        {children}
      </Layout>
    );
  }
  return (
    <div
      className={`player-layout ${libraryOpen ? "" : "collapsed"}`}
      data-game-layout="generic"
    >
      {children}
    </div>
  );
}
