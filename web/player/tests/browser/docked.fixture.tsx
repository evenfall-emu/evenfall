// The two-column player around the Player's screen markup, with panels and
// a focus the test changes, and whether the page's shape calls for it.
import { useState } from "react";
import { createRoot } from "react-dom/client";
import { DockedPlayer, useDocked } from "../../src/docked";
import "../../src/styles.css";

declare global {
  interface Window {
    dockFixture: { focus(id: string | undefined): void; rerender(): void };
  }
}

function Screen() {
  return (
    <div className="player-stage">
      <section className="console-player" aria-label="Game player">
        <div className="game-viewport">
          <div className="game-display">
            <canvas width="240" height="160" aria-label="Game screen" />
          </div>
        </div>
        <div className="quick-bar">
          <button>Quick save</button>
          <button>Quick load</button>
          <button>Rewind</button>
          <button>Reset</button>
        </div>
      </section>
    </div>
  );
}

function Fixture() {
  const [focus, setFocus] = useState<string | undefined>("battle");
  const [, setRenders] = useState(0);
  window.dockFixture = {
    focus: setFocus,
    rerender: () => setRenders((count) => count + 1),
  };
  const docked = useDocked();
  return (
    <div className="console-app" data-view="player">
      <div id="docked" data-docked={docked} style={{ height: 150 }} />
      <DockedPlayer
        dock={{
          heading: <h1>Route 29</h1>,
          panels: [
            {
              id: "team",
              label: "Team",
              content: <input aria-label="Search team" />,
            },
            { id: "battle", label: "Battle", content: <p>Wild Pidgey</p> },
          ],
          focus,
        }}
      >
        <Screen />
      </DockedPlayer>
    </div>
  );
}

const root = document.createElement("div");
document.body.replaceChildren(root);
createRoot(root).render(<Fixture />);
