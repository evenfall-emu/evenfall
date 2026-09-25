import { asc, count, eq } from "drizzle-orm";
import type { DrizzleD1Database } from "drizzle-orm/d1";
import { bases, games, versions } from "./schema";
import * as schema from "./schema";
import type {
  CatalogBase,
  CatalogGame,
  CatalogGameSummary,
  CatalogVersion,
} from "./types";

export type CatalogDatabase = DrizzleD1Database<typeof schema>;

function publicBase(
  base: typeof bases.$inferSelect | null,
): CatalogBase | null {
  return base && { ...base, verified: base.sha256 !== null };
}

export async function listGames(
  db: CatalogDatabase,
): Promise<CatalogGameSummary[]> {
  const rows = await db
    .select({
      id: games.id,
      internalName: games.internalName,
      displayName: games.displayName,
      summary: games.summary,
      imageUrl: games.imageUrl,
      versionCount: count(versions.id),
    })
    .from(games)
    .leftJoin(versions, eq(versions.gameId, games.id))
    .groupBy(games.id)
    .orderBy(asc(games.displayName));
  return rows;
}

export async function getGame(
  db: CatalogDatabase,
  id: string,
): Promise<CatalogGame | null> {
  const game = await db.select().from(games).where(eq(games.id, id)).get();
  if (!game) return null;
  const releases = await db
    .select({ version: versions, base: bases })
    .from(versions)
    .leftJoin(bases, eq(versions.requiredBaseId, bases.id))
    .where(eq(versions.gameId, id))
    .orderBy(asc(versions.displayName));
  return {
    ...game,
    versions: releases.map(({ version, base }) => ({
      ...version,
      requiredBase: publicBase(base),
    })),
  };
}

export async function listVersions(
  db: CatalogDatabase,
): Promise<CatalogVersion[]> {
  const rows = await db
    .select({ version: versions, base: bases })
    .from(versions)
    .leftJoin(bases, eq(versions.requiredBaseId, bases.id))
    .orderBy(asc(versions.displayName));
  return rows.map(({ version, base }) => ({
    ...version,
    requiredBase: publicBase(base),
  }));
}

export async function getVersion(
  db: CatalogDatabase,
  id: string,
): Promise<CatalogVersion | null> {
  const row = await db
    .select({ version: versions, base: bases })
    .from(versions)
    .leftJoin(bases, eq(versions.requiredBaseId, bases.id))
    .where(eq(versions.id, id))
    .get();
  return row ? { ...row.version, requiredBase: publicBase(row.base) } : null;
}

export async function listBases(db: CatalogDatabase): Promise<CatalogBase[]> {
  const rows = await db.select().from(bases).orderBy(asc(bases.displayName));
  return rows.map((base) => ({ ...base, verified: base.sha256 !== null }));
}

export async function getBase(
  db: CatalogDatabase,
  id: string,
): Promise<CatalogBase | null> {
  const base = await db.select().from(bases).where(eq(bases.id, id)).get();
  return publicBase(base ?? null);
}
