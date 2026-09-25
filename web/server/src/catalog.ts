import { and, asc, count, eq, getTableColumns, inArray, ne } from "drizzle-orm";
import type { DrizzleD1Database } from "drizzle-orm/d1";
import {
  bases,
  gameTags,
  games,
  playthrough,
  versionSaveCompatibility,
  versions,
} from "./schema";
import * as schema from "./schema";
import type { Base, Version } from "./schema";
import type {
  BaseInput,
  CatalogGame,
  CatalogGameSummary,
  CatalogVersion,
  GameInput,
  VersionInput,
} from "./types";

export type CatalogDatabase = DrizzleD1Database<typeof schema>;

/** A write that breaks a catalog rule no column constraint can express. */
export class CatalogRuleError extends Error {}

function group<T>(
  rows: T[],
  key: (row: T) => string,
  value: (row: T) => string,
) {
  const grouped = new Map<string, string[]>();
  for (const row of rows)
    grouped.set(key(row), [...(grouped.get(key(row)) ?? []), value(row)]);
  return grouped;
}

async function tagsByGame(db: CatalogDatabase, gameId?: string) {
  const rows = await db
    .select()
    .from(gameTags)
    .where(gameId ? eq(gameTags.gameId, gameId) : undefined)
    .orderBy(asc(gameTags.gameId), asc(gameTags.position));
  return group(
    rows,
    (row) => row.gameId,
    (row) => row.tag,
  );
}

async function cloudPlaythroughsByGame(db: CatalogDatabase, gameId?: string) {
  const rows = await db
    .select({ gameId: versions.gameId, count: count() })
    .from(versions)
    .innerJoin(playthrough, eq(playthrough.hash, versions.outputSha1))
    .where(gameId ? eq(versions.gameId, gameId) : undefined)
    .groupBy(versions.gameId);
  return new Map(rows.map((row) => [row.gameId, row.count]));
}

async function compatibilityByVersion(db: CatalogDatabase, versionId?: string) {
  const rows = await db
    .select()
    .from(versionSaveCompatibility)
    .where(
      versionId ? eq(versionSaveCompatibility.versionId, versionId) : undefined,
    )
    .orderBy(asc(versionSaveCompatibility.fromVersionId));
  return group(
    rows,
    (row) => row.versionId,
    (row) => row.fromVersionId,
  );
}

function catalogVersion(
  version: Version,
  base: Base | null,
  compatibility: Map<string, string[]>,
): CatalogVersion {
  const { patchUrl, patchFormat, patchSize, ...rest } = version;
  return {
    ...rest,
    requiredBase: base,
    patch:
      patchUrl && patchFormat && patchSize
        ? { url: patchUrl, format: patchFormat, size: patchSize }
        : null,
    saveCompatibleFrom: compatibility.get(version.id) ?? [],
  };
}

export async function listGames(
  db: CatalogDatabase,
): Promise<CatalogGameSummary[]> {
  const [rows, tags, played] = await Promise.all([
    db
      .select({ ...getTableColumns(games), versionCount: count(versions.id) })
      .from(games)
      .leftJoin(versions, eq(versions.gameId, games.id))
      .groupBy(games.id)
      .orderBy(asc(games.displayName)),
    tagsByGame(db),
    cloudPlaythroughsByGame(db),
  ]);
  return rows.map((game) => ({
    ...game,
    tags: tags.get(game.id) ?? [],
    cloudPlaythroughs: played.get(game.id) ?? 0,
  }));
}

export async function getGame(
  db: CatalogDatabase,
  id: string,
): Promise<CatalogGame | null> {
  const game = await db.select().from(games).where(eq(games.id, id)).get();
  if (!game) return null;
  const [releases, tags, played, compatibility] = await Promise.all([
    db
      .select({ version: versions, base: bases })
      .from(versions)
      .leftJoin(bases, eq(versions.requiredBaseId, bases.id))
      .where(eq(versions.gameId, id))
      .orderBy(asc(versions.displayName)),
    tagsByGame(db, id),
    cloudPlaythroughsByGame(db, id),
    db
      .select()
      .from(versionSaveCompatibility)
      .innerJoin(versions, eq(versions.id, versionSaveCompatibility.versionId))
      .where(eq(versions.gameId, id))
      .orderBy(asc(versionSaveCompatibility.fromVersionId))
      .then((rows) =>
        group(
          rows,
          (row) => row.version_save_compatibility.versionId,
          (row) => row.version_save_compatibility.fromVersionId,
        ),
      ),
  ]);
  return {
    ...game,
    tags: tags.get(id) ?? [],
    cloudPlaythroughs: played.get(id) ?? 0,
    versions: releases.map(({ version, base }) =>
      catalogVersion(version, base, compatibility),
    ),
  };
}

export async function listVersions(
  db: CatalogDatabase,
): Promise<CatalogVersion[]> {
  const [rows, compatibility] = await Promise.all([
    db
      .select({ version: versions, base: bases })
      .from(versions)
      .leftJoin(bases, eq(versions.requiredBaseId, bases.id))
      .orderBy(asc(versions.displayName)),
    compatibilityByVersion(db),
  ]);
  return rows.map(({ version, base }) =>
    catalogVersion(version, base, compatibility),
  );
}

export async function getVersion(
  db: CatalogDatabase,
  id: string,
): Promise<CatalogVersion | null> {
  const [row, compatibility] = await Promise.all([
    db
      .select({ version: versions, base: bases })
      .from(versions)
      .leftJoin(bases, eq(versions.requiredBaseId, bases.id))
      .where(eq(versions.id, id))
      .get(),
    compatibilityByVersion(db, id),
  ]);
  return row ? catalogVersion(row.version, row.base, compatibility) : null;
}

export async function listBases(db: CatalogDatabase): Promise<Base[]> {
  return db.select().from(bases).orderBy(asc(bases.displayName));
}

export async function getBase(
  db: CatalogDatabase,
  id: string,
): Promise<Base | null> {
  return (await db.select().from(bases).where(eq(bases.id, id)).get()) ?? null;
}

/** Full-record upsert of a game and its tags, in one atomic batch. */
export async function putGame(
  db: CatalogDatabase,
  id: string,
  input: GameInput,
) {
  const { tags, ...row } = input;
  await db.batch([
    db
      .insert(games)
      .values({ id, ...row })
      .onConflictDoUpdate({ target: games.id, set: row }),
    db.delete(gameTags).where(eq(gameTags.gameId, id)),
    ...(tags.length
      ? [
          db
            .insert(gameTags)
            .values(
              tags.map((tag, position) => ({ gameId: id, tag, position })),
            ),
        ]
      : []),
  ] as const);
}

export async function putBase(
  db: CatalogDatabase,
  id: string,
  input: BaseInput,
) {
  await db
    .insert(bases)
    .values({ id, ...input })
    .onConflictDoUpdate({ target: bases.id, set: input });
}

/**
 * Full-record upsert of a version, its patch and the versions whose saves it
 * accepts, in one atomic batch. Compatibility stays within one game.
 */
export async function putVersion(
  db: CatalogDatabase,
  id: string,
  input: VersionInput,
) {
  const { patch, saveCompatibleFrom, ...row } = input;
  if (saveCompatibleFrom.includes(id))
    throw new CatalogRuleError("A version cannot list itself as compatible");
  if (saveCompatibleFrom.length) {
    const listed = await db
      .select({ gameId: versions.gameId })
      .from(versions)
      .where(inArray(versions.id, saveCompatibleFrom));
    // Unknown IDs fail the foreign key in the batch below.
    if (listed.some((version) => version.gameId !== row.gameId))
      throw new CatalogRuleError(
        "Save-compatible versions must belong to the same game",
      );
  }
  const columns = {
    ...row,
    patchUrl: patch?.url ?? null,
    patchFormat: patch?.format ?? null,
    patchSize: patch?.size ?? null,
  };
  await db.batch([
    db
      .insert(versions)
      .values({ id, ...columns })
      .onConflictDoUpdate({ target: versions.id, set: columns }),
    db
      .delete(versionSaveCompatibility)
      .where(eq(versionSaveCompatibility.versionId, id)),
    // A version moved to another game leaves its old game's lists.
    db
      .delete(versionSaveCompatibility)
      .where(
        and(
          eq(versionSaveCompatibility.fromVersionId, id),
          inArray(
            versionSaveCompatibility.versionId,
            db
              .select({ id: versions.id })
              .from(versions)
              .where(ne(versions.gameId, row.gameId)),
          ),
        ),
      ),
    ...(saveCompatibleFrom.length
      ? [
          db.insert(versionSaveCompatibility).values(
            saveCompatibleFrom.map((fromVersionId) => ({
              versionId: id,
              fromVersionId,
            })),
          ),
        ]
      : []),
  ] as const);
}
