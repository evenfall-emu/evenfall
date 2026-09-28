import { putBase, putGame, putVersion, type CatalogDatabase } from "./catalog";
import type { BaseInput, GameInput, VersionInput } from "./types";
import romhacks from "./romhacks.json";

// Seed rows are management inputs, so they go through the same validation and
// writes as PUT /api/admin/*.
type Seed<T> = { id: string } & T;

export const seedGames: readonly Seed<GameInput>[] = romhacks.games;

export const seedBases: readonly Seed<BaseInput>[] = [
  {
    id: "pokemon-emerald-us",
    internalName: "pokemon/emerald-us",
    displayName: "Pokémon Emerald (USA, Europe)",
    // No-Intro "Pokemon - Emerald Version (USA, Europe)".
    sha1: "f3ae088181bf583e55daf962a92bb46f4f1d07b7",
    crc32: "1f1c08fb",
  },
  ...romhacks.bases.filter((base) => base.id !== "pokemon-emerald-us"),
];

export const seedVersions: readonly Seed<VersionInput>[] =
  romhacks.versions.map((version) => ({
    ...version,
    patch: version.patch && {
      ...version.patch,
      format: version.patch.format as NonNullable<
        VersionInput["patch"]
      >["format"],
    },
    visibility: version.visibility as NonNullable<VersionInput["visibility"]>,
  }));

/** Explicit, idempotent catalog seed. It is never called by a read request or startup. */
export async function seedCatalog(db: CatalogDatabase): Promise<void> {
  for (const { id, ...game } of seedGames) await putGame(db, id, game);
  for (const { id, ...base } of seedBases) await putBase(db, id, base);
  for (const { id, ...version } of seedVersions)
    await putVersion(db, id, version);
}
