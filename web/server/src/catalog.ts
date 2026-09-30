import {
  and,
  asc,
  count,
  eq,
  getTableColumns,
  inArray,
  isNotNull,
  ne,
} from "drizzle-orm";
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

// Public catalog reads never disclose hidden or unpatchable releases. The
// Experimental setting only changes what the player displays, not this API.
const publicVersion = and(
  isNotNull(versions.patchUrl),
  ne(versions.visibility, "hidden"),
);

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
    .where(and(gameId ? eq(versions.gameId, gameId) : undefined, publicVersion))
    .groupBy(versions.gameId);
  return new Map(rows.map((row) => [row.gameId, row.count]));
}

async function compatibilityByVersion(
  db: CatalogDatabase,
  versionId?: string,
  publicOnly = true,
) {
  const rows = await db
    .select()
    .from(versionSaveCompatibility)
    .where(
      and(
        versionId
          ? eq(versionSaveCompatibility.versionId, versionId)
          : undefined,
        publicOnly
          ? inArray(
              versionSaveCompatibility.fromVersionId,
              db
                .select({ id: versions.id })
                .from(versions)
                .where(publicVersion),
            )
          : undefined,
      ),
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
  publicOnly = true,
): Promise<CatalogGameSummary[]> {
  const [rows, tags, played] = await Promise.all([
    db
      .select({ ...getTableColumns(games), versionCount: count(versions.id) })
      .from(games)
      [publicOnly ? "innerJoin" : "leftJoin"](
        versions,
        and(
          eq(versions.gameId, games.id),
          publicOnly ? publicVersion : undefined,
        ),
      )
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
  publicOnly = true,
): Promise<CatalogGame | null> {
  const game = await db.select().from(games).where(eq(games.id, id)).get();
  if (!game) return null;
  const [releases, tags, played, compatibility] = await Promise.all([
    db
      .select({ version: versions, base: bases })
      .from(versions)
      .leftJoin(bases, eq(versions.requiredBaseId, bases.id))
      .where(
        and(eq(versions.gameId, id), publicOnly ? publicVersion : undefined),
      )
      .orderBy(asc(versions.displayName)),
    tagsByGame(db, id),
    cloudPlaythroughsByGame(db, id),
    db
      .select()
      .from(versionSaveCompatibility)
      .innerJoin(versions, eq(versions.id, versionSaveCompatibility.versionId))
      .where(
        and(
          eq(versions.gameId, id),
          publicOnly
            ? inArray(
                versionSaveCompatibility.fromVersionId,
                db
                  .select({ id: versions.id })
                  .from(versions)
                  .where(publicVersion),
              )
            : undefined,
        ),
      )
      .orderBy(asc(versionSaveCompatibility.fromVersionId))
      .then((rows) =>
        group(
          rows,
          (row) => row.version_save_compatibility.versionId,
          (row) => row.version_save_compatibility.fromVersionId,
        ),
      ),
  ]);
  if (publicOnly && !releases.length) return null;
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
  publicOnly = true,
): Promise<CatalogVersion[]> {
  const [rows, compatibility] = await Promise.all([
    db
      .select({ version: versions, base: bases })
      .from(versions)
      .leftJoin(bases, eq(versions.requiredBaseId, bases.id))
      .where(publicOnly ? publicVersion : undefined)
      .orderBy(asc(versions.displayName)),
    compatibilityByVersion(db, undefined, publicOnly),
  ]);
  return rows.map(({ version, base }) =>
    catalogVersion(version, base, compatibility),
  );
}

export async function getVersion(
  db: CatalogDatabase,
  id: string,
  publicOnly = true,
): Promise<CatalogVersion | null> {
  const [row, compatibility] = await Promise.all([
    db
      .select({ version: versions, base: bases })
      .from(versions)
      .leftJoin(bases, eq(versions.requiredBaseId, bases.id))
      .where(and(eq(versions.id, id), publicOnly ? publicVersion : undefined))
      .get(),
    compatibilityByVersion(db, id, publicOnly),
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
  const values = { ...row, description: input.description ?? null };
  await db.batch([
    db
      .insert(games)
      .values({ id, ...values })
      .onConflictDoUpdate({ target: games.id, set: values }),
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
    visibility: row.visibility ?? "experimental",
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
