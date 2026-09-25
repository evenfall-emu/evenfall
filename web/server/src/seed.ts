import { putBase, putGame, putVersion, type CatalogDatabase } from "./catalog";
import type { BaseInput, GameInput, VersionInput } from "./types";

// Seed rows are management inputs, so they go through the same validation and
// writes as PUT /api/admin/*.
type Seed<T> = { id: string } & T;

export const seedGames: readonly Seed<GameInput>[] = [
  {
    id: "heart-n-soul",
    internalName: "pokemon/heart-n-soul",
    displayName: "Pokémon Heart & Soul",
    summary: null,
    imageUrl: null,
    franchise: "Pokémon",
    tags: ["Pokémon", "Remake"],
  },
  {
    id: "elite-redux",
    internalName: "pokemon/elite-redux",
    displayName: "Pokémon Elite Redux",
    summary: null,
    imageUrl: null,
    franchise: "Pokémon",
    tags: ["Pokémon", "Difficulty Hack"],
  },
];

export const seedBases: readonly Seed<BaseInput>[] = [
  {
    id: "pokemon-emerald-us",
    internalName: "pokemon/emerald-us",
    displayName: "Pokémon Emerald (USA, Europe)",
    // No-Intro "Pokemon - Emerald Version (USA, Europe)".
    sha1: "f3ae088181bf583e55daf962a92bb46f4f1d07b7",
    crc32: "1f1c08fb",
  },
];

export const seedVersions: readonly Seed<VersionInput>[] = [
  {
    id: "heart-n-soul-1.2.1",
    gameId: "heart-n-soul",
    internalName: "Release-v1.2.1",
    displayName: "1.2.1",
    outputSha1: "a4be81f8330d7f223a3ab847c3da822fe6f8a2e0",
    requiredBaseId: "pokemon-emerald-us",
    sourceRepositoryUrl: "https://github.com/PokemonHnS-Development/pokemonHnS",
    sourceRevision: "4aa8fa894db4c890b3d1100abd6aab50f44c22b1",
    releasedAt: null,
    releaseNotes: null,
    patch: null,
    saveCompatibleFrom: [],
  },
  {
    id: "elite-redux-2.65-beta2",
    gameId: "elite-redux",
    internalName: "v2.65-beta2-1048",
    displayName: "2.65 Beta2 (1048)",
    outputSha1: "f1323126bc043f3ce931bcc670957930559eb515",
    requiredBaseId: "pokemon-emerald-us",
    sourceRepositoryUrl: "https://github.com/Elite-Redux/eliteredux-source",
    sourceRevision: "06da575972cfed20d471f8bb5b90ea9e4736b16a",
    releasedAt: null,
    releaseNotes: null,
    patch: null,
    saveCompatibleFrom: [],
  },
];

/** Explicit, idempotent catalog seed. It is never called by a read request or startup. */
export async function seedCatalog(db: CatalogDatabase): Promise<void> {
  for (const { id, ...game } of seedGames) await putGame(db, id, game);
  for (const { id, ...base } of seedBases) await putBase(db, id, base);
  for (const { id, ...version } of seedVersions)
    await putVersion(db, id, version);
}
