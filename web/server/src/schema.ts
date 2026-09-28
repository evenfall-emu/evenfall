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
    description: text("description"),
    imageUrl: text("image_url"),
    franchise: text("franchise"),
  },
  (table) => [uniqueIndex("games_internal_name_unique").on(table.internalName)],
);

// A game's tags, in display order.
export const gameTags = sqliteTable(
  "game_tags",
  {
    gameId: text("game_id").notNull(),
    tag: text("tag").notNull(),
    position: integer("position").notNull(),
  },
  (table) => [
    primaryKey({ columns: [table.gameId, table.tag] }),
    foreignKey({ columns: [table.gameId], foreignColumns: [games.id] })
      .onUpdate("cascade")
      .onDelete("cascade"),
    index("game_tags_tag_index").on(table.tag),
  ],
);

// A base is one No-Intro dump. Its SHA-1 and CRC32 are the DAT's; patch
// footers (BPS, UPS) name their source by CRC32.
export const bases = sqliteTable(
  "bases",
  {
    id: text("id").primaryKey(),
    internalName: text("internal_name").notNull(),
    displayName: text("display_name").notNull(),
    sha1: text("sha1").notNull(),
    crc32: text("crc32").notNull(),
  },
  (table) => [
    uniqueIndex("bases_internal_name_unique").on(table.internalName),
    uniqueIndex("bases_sha1_unique").on(table.sha1),
    index("bases_crc32_index").on(table.crc32),
    check(
      "bases_sha1_format",
      sql`${table.sha1} not glob '*[^0-9a-f]*' and length(${table.sha1}) = 40`,
    ),
    check(
      "bases_crc32_format",
      sql`${table.crc32} not glob '*[^0-9a-f]*' and length(${table.crc32}) = 8`,
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
    outputSha1: text("output_sha1").notNull(),
    requiredBaseId: text("required_base_id"),
    sourceRepositoryUrl: text("source_repository_url"),
    sourceRevision: text("source_revision"),
    // A calendar date (YYYY-MM-DD); unknown for some releases.
    releasedAt: text("released_at"),
    releaseNotes: text("release_notes"),
    // The hosted patch that turns the required base into this release. Its
    // integrity follows from the base SHA-1 in and the output SHA-1 out.
    patchUrl: text("patch_url"),
    patchFormat: text("patch_format", { enum: ["bps", "ups", "ips"] }),
    patchSize: integer("patch_size"),
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
    uniqueIndex("versions_output_sha1_unique").on(table.outputSha1),
    index("versions_game_id_index").on(table.gameId),
    index("versions_required_base_id_index").on(table.requiredBaseId),
    check(
      "versions_output_sha1_format",
      sql`${table.outputSha1} not glob '*[^0-9a-f]*' and length(${table.outputSha1}) = 40`,
    ),
    check(
      "versions_released_at_format",
      sql`${table.releasedAt} is null or ${table.releasedAt} glob '[0-9][0-9][0-9][0-9]-[01][0-9]-[0-3][0-9]'`,
    ),
    check(
      "versions_patch_complete",
      sql`(${table.patchUrl} is null) = (${table.patchFormat} is null) and (${table.patchUrl} is null) = (${table.patchSize} is null)`,
    ),
    check(
      "versions_patch_base",
      sql`${table.patchUrl} is null or ${table.requiredBaseId} is not null`,
    ),
    check(
      "versions_patch_format",
      sql`${table.patchFormat} is null or ${table.patchFormat} in ('bps', 'ups', 'ips')`,
    ),
    check(
      "versions_patch_size",
      sql`${table.patchSize} is null or ${table.patchSize} > 0`,
    ),
  ],
);

// Earlier versions of the same game whose in-game saves are verified to load
// in a version.
export const versionSaveCompatibility = sqliteTable(
  "version_save_compatibility",
  {
    versionId: text("version_id").notNull(),
    fromVersionId: text("from_version_id").notNull(),
  },
  (table) => [
    primaryKey({ columns: [table.versionId, table.fromVersionId] }),
    foreignKey({ columns: [table.versionId], foreignColumns: [versions.id] })
      .onUpdate("cascade")
      .onDelete("cascade"),
    foreignKey({
      columns: [table.fromVersionId],
      foreignColumns: [versions.id],
    })
      .onUpdate("cascade")
      .onDelete("cascade"),
    index("version_save_compatibility_from_index").on(table.fromVersionId),
    check(
      "version_save_compatibility_distinct",
      sql`${table.versionId} <> ${table.fromVersionId}`,
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
    // Counts a game's cloud playthroughs across accounts.
    index("playthrough_hash_idx").on(table.hash),
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
