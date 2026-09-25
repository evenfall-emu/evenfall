import type { CatalogDatabase } from "./catalog";
import {
  bases,
  games,
  versions,
  type NewBase,
  type NewGame,
  type NewVersion,
} from "./schema";

export const seedGames: readonly NewGame[] = [
  {
    id: "heart-n-soul",
    internalName: "pokemon/heart-n-soul",
    displayName: "Pokémon Heart & Soul",
    summary: null,
    imageUrl: null,
  },
  {
    id: "elite-redux",
    internalName: "pokemon/elite-redux",
    displayName: "Pokémon Elite Redux",
    summary: null,
    imageUrl: null,
  },
];

export const seedBases: readonly NewBase[] = [
  {
    id: "pokemon-emerald-us",
    internalName: "pokemon/emerald-us",
    displayName: "Pokémon Emerald (US)",
    // No exact playable base-ROM SHA-256 is established in this repository.
    sha256: null,
  },
];

export const seedVersions: readonly NewVersion[] = [
  {
    id: "heart-n-soul-1.2.1",
    gameId: "heart-n-soul",
    internalName: "Release-v1.2.1",
    displayName: "1.2.1",
    outputSha256:
      "7c4f90c8b68b64d4639a37306d5302a11cd4d7c44005114ec2faa2b28b210c3d",
    requiredBaseId: "pokemon-emerald-us",
    sourceRepositoryUrl: "https://github.com/PokemonHnS-Development/pokemonHnS",
    sourceRevision: "4aa8fa894db4c890b3d1100abd6aab50f44c22b1",
  },
  {
    id: "elite-redux-2.65-beta2",
    gameId: "elite-redux",
    internalName: "v2.65-beta2-1048",
    displayName: "2.65 Beta2 (1048)",
    outputSha256:
      "d86e6ac909e4abdef9159ed20b75a684a9b2ee2fe874f388122e00a5749d2d37",
    requiredBaseId: "pokemon-emerald-us",
    sourceRepositoryUrl: "https://github.com/Elite-Redux/eliteredux-source",
    sourceRevision: "06da575972cfed20d471f8bb5b90ea9e4736b16a",
  },
];

/** Explicit, idempotent catalog seed. It is never called by a read request or startup. */
export async function seedCatalog(db: CatalogDatabase): Promise<void> {
  for (const game of seedGames) {
    await db
      .insert(games)
      .values(game)
      .onConflictDoUpdate({
        target: games.id,
        set: {
          internalName: game.internalName,
          displayName: game.displayName,
          summary: game.summary,
          imageUrl: game.imageUrl,
        },
      });
  }
  for (const base of seedBases) {
    await db
      .insert(bases)
      .values(base)
      .onConflictDoUpdate({
        target: bases.id,
        set: {
          internalName: base.internalName,
          displayName: base.displayName,
          sha256: base.sha256,
        },
      });
  }
  for (const version of seedVersions) {
    await db
      .insert(versions)
      .values(version)
      .onConflictDoUpdate({
        target: versions.id,
        set: {
          gameId: version.gameId,
          internalName: version.internalName,
          displayName: version.displayName,
          outputSha256: version.outputSha256,
          requiredBaseId: version.requiredBaseId,
          sourceRepositoryUrl: version.sourceRepositoryUrl,
          sourceRevision: version.sourceRevision,
        },
      });
  }
}
