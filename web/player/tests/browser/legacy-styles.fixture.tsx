import React from "react";
import { createRoot } from "react-dom/client";
import "../../src/styles.css";

// A legacy page region, a Tailwind root with its own type size and a nested
// root inside it, and a root rendered straight into the body like a portal.
const root = document.createElement("div");
document.body.replaceChildren(root);
createRoot(root).render(
  <div className="console-app" data-view="player">
    <p id="legacy">Legacy panel text</p>
    <header data-ui className="text-2x" id="sized">
      Sized root
      <div data-ui id="nested">
        Nested root
      </div>
    </header>
  </div>,
);
const portal = document.createElement("div");
portal.dataset.ui = "";
portal.id = "portal";
portal.textContent = "Portaled popup";
document.body.append(portal);
