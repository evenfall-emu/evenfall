import React from "react";
import { createRoot } from "react-dom/client";
import { LibraryPage, type LibraryEntry } from "../../src/library-page";
import { AppHeader } from "../../src/app-header";
import "../../src/console.css";
import "../../src/about.css";

// In-memory UI fixtures, never a catalog or a source of player installations.
async function unavailable() {
  await new Promise((resolve) => setTimeout(resolve, 400));
  throw new Error("Preview only — no installer or game is connected.");
}
const entries: LibraryEntry[] = [
  {
    id: "heart-soul",
    title: "Heart & Soul",
    status: "installed",
    lastPlayed: 2000,
    resume: true,
    onPlay: unavailable,
  },
  {
    id: "emerald",
    title: "Pokémon Emerald",
    status: "installed",
    lastPlayed: 1000,
    onPlay: unavailable,
  },
  {
    id: "import",
    title: "My imported game",
    status: "installed",
    lastPlayed: 500,
    onPlay: unavailable,
  },
  {
    id: "redux",
    title: "Elite Redux",
    status: "ready",
    requiredBase: "Pokémon Emerald · US 1.0",
    onPlay: unavailable,
  },
  {
    id: "seaglass",
    title: "Emerald Seaglass",
    status: "ready",
    requiredBase: "Pokémon Emerald · US 1.0",
    onPlay: unavailable,
  },
  {
    id: "gaia",
    title: "Gaia",
    status: "missing-base",
    requiredBase: "Pokémon FireRed · US 1.0",
    onImportBase: unavailable,
  },
  {
    id: "radical",
    title: "Radical Red",
    status: "missing-base",
    requiredBase: "Pokémon FireRed · US 1.0",
    onImportBase: unavailable,
  },
  {
    id: "unbound",
    title: "Unbound",
    status: "missing-base",
    requiredBase: "Pokémon FireRed · US 1.0",
    description:
      "A new journey, with familiar roots. Discover a fan-made adventure with its own story, places to explore, and challenges to take on.\n\nBring your original base game, choose a release, and make this adventure your own.",
    onImportBase: unavailable,
  },
];
const root =
  document.getElementById("root") ??
  document.body.appendChild(document.createElement("div"));
const addGame = () =>
  alert("UI preview only. Open the real Player to add a game.");
createRoot(root).render(
  <div className="library-preview">
    <p className="library-preview-label">
      UI preview · sample catalog states · no installation or activity service
    </p>
    <AppHeader
      view="library"
      onNavigate={() => location.assign("/")}
      onAddGame={addGame}
      onAccount={() => location.assign("/?account=1")}
    />
    <LibraryPage entries={entries} onAddGame={addGame} />
  </div>,
);
