import { useState, useSyncExternalStore, type ReactNode } from "react";
import type { GameDock } from "./games";
import { PanelTabs } from "./panel-tabs";

// Landscape pages narrower than 17:10. There, columns on both sides of the
// screen leave it limited by the page's width with height to spare; one
// column of tabs lets it grow to the page's height. Phones take precedence.
const narrow = "(orientation: landscape) and (max-aspect-ratio: 17/10)";

function watchNarrow(change: () => void) {
  const list = matchMedia(narrow);
  list.addEventListener("change", change);
  return () => list.removeEventListener("change", change);
}

/** Whether a game layout gets the two-column player instead of its own. */
export function useDocked() {
  return useSyncExternalStore(watchNarrow, () => matchMedia(narrow).matches);
}

/**
 * The two-column player: the game layout's heading above the screen, and
 * its panels as tabs in a column beside it, 340 to 480 px wide. The tab the
 * game calls for opens whenever that changes; otherwise the chosen tab stays.
 */
export function DockedPlayer({
  dock,
  children,
}: {
  dock: GameDock;
  /** The screen. */
  children: ReactNode;
}) {
  const [panel, setPanel] = useState(dock.focus ?? dock.panels[0]?.id);
  const [focus, setFocus] = useState(dock.focus);
  if (dock.focus !== focus) {
    setFocus(dock.focus);
    if (dock.focus !== undefined) setPanel(dock.focus);
  }
  return (
    <main className="docked grid min-h-0 flex-1 grid-cols-[minmax(0,1fr)_clamp(340px,30vw,480px)] grid-rows-[minmax(0,1fr)] gap-3 px-2.5 py-3">
      <div className="flex min-h-0 min-w-0 flex-col">
        {dock.heading}
        {children}
      </div>
      <PanelTabs
        panels={dock.panels}
        panel={panel}
        onPanel={setPanel}
        className="docked-panels min-h-0 min-w-0"
        panelClassName="pt-2"
      />
    </main>
  );
}
