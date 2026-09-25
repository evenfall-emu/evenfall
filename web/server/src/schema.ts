import { sql } from "drizzle-orm";
import {
  check,
  foreignKey,
  index,
  sqliteTable,
  text,
  integer,
  primaryKey,
  uniqueIndex,
} from "drizzle-orm/sqlite-core";

export const games = sqliteTable(
  "games",
  {
    id: text("id").primaryKey(),
    internalName: text("internal_name").notNull(),
    displayName: text("display_name").notNull(),
    summary: text("summary"),
    imageUrl: text("image_url"),
  },
  (table) => [uniqueIndex("games_internal_name_unique").on(table.internalName)],
);

export const bases = sqliteTable(
  "bases",
  {
    id: text("id").primaryKey(),
    internalName: text("internal_name").notNull(),
    displayName: text("display_name").notNull(),
    sha256: text("sha256"),
  },
  (table) => [
    uniqueIndex("bases_internal_name_unique").on(table.internalName),
    uniqueIndex("bases_sha256_unique").on(table.sha256),
    check(
      "bases_sha256_format",
      sql`${table.sha256} is null or (${table.sha256} not glob '*[^0-9a-f]*' and length(${table.sha256}) = 64)`,
    ),
  ],
);

export const versions = sqliteTable(
  "versions",
  {
    id: text("id").primaryKey(),
    gameId: text("game_id").notNull(),
    internalName: text("internal_name").notNull(),
    displayName: text("display_name").notNull(),
    outputSha256: text("output_sha256").notNull(),
    requiredBaseId: text("required_base_id"),
    sourceRepositoryUrl: text("source_repository_url"),
    sourceRevision: text("source_revision"),
  },
  (table) => [
    foreignKey({ columns: [table.gameId], foreignColumns: [games.id] })
      .onUpdate("cascade")
      .onDelete("cascade"),
    foreignKey({ columns: [table.requiredBaseId], foreignColumns: [bases.id] })
      .onUpdate("cascade")
      .onDelete("restrict"),
    uniqueIndex("versions_game_internal_name_unique").on(
      table.gameId,
      table.internalName,
    ),
    uniqueIndex("versions_output_sha256_unique").on(table.outputSha256),
    index("versions_game_id_index").on(table.gameId),
    index("versions_required_base_id_index").on(table.requiredBaseId),
    check(
      "versions_output_sha256_format",
      sql`${table.outputSha256} not glob '*[^0-9a-f]*' and length(${table.outputSha256}) = 64`,
    ),
  ],
);

export type Game = typeof games.$inferSelect;
export type NewGame = typeof games.$inferInsert;
export type Base = typeof bases.$inferSelect;
export type NewBase = typeof bases.$inferInsert;
export type Version = typeof versions.$inferSelect;

// Retained independently of Better Auth's user cascade. An interrupted delete
// remains gated and the scheduled sweep can finish it without a session.
export const accountDeletions = sqliteTable("account_deletions", {
  userId: text("user_id").primaryKey(),
  requestedAt: integer("requested_at").notNull(),
  lastSweptAt: integer("last_swept_at"),
});

export const playthrough = sqliteTable(
  "playthrough",
  {
    userId: text("user_id").notNull(),
    hash: text("hash").notNull(),
    id: text("id").notNull(),
    name: text("name").notNull(),
  },
  (table) => [
    primaryKey({ columns: [table.userId, table.hash, table.id] }),
    index("playthrough_user_idx").on(table.userId),
  ],
);

// Optional per-account overrides. A missing row or null column uses the
// Worker's configured default; operators set rows directly in D1.
export const accountLimits = sqliteTable(
  "account_limits",
  {
    userId: text("user_id").primaryKey(),
    quickSlots: integer("quick_slots"),
  },
  (table) => [
    check(
      "account_limits_quick_slots_range",
      sql`${table.quickSlots} is null or ${table.quickSlots} between 0 and 99`,
    ),
  ],
);

export type NewVersion = typeof versions.$inferInsert;
