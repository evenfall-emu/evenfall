-- Game assets are identified by SHA-1, as in No-Intro DATs, instead of SHA-256.
-- SHA-1 cannot be derived from SHA-256, so the catalog tables are recreated
-- empty: run the explicit catalog seed (POST /api/admin/seed) after applying.
-- Cloud saves keyed by a ROM SHA-256 are no longer addressable; their
-- playthrough rows are removed so they stop counting against the limit.
DELETE FROM `playthrough` WHERE length(`hash`) <> 40;--> statement-breakpoint
PRAGMA defer_foreign_keys = on;--> statement-breakpoint
DELETE FROM `versions`;--> statement-breakpoint
DELETE FROM `bases`;--> statement-breakpoint
CREATE TABLE `__new_bases` (
	`id` text PRIMARY KEY NOT NULL,
	`internal_name` text NOT NULL,
	`display_name` text NOT NULL,
	`sha1` text NOT NULL,
	`crc32` text NOT NULL,
	CONSTRAINT "bases_sha1_format" CHECK("__new_bases"."sha1" not glob '*[^0-9a-f]*' and length("__new_bases"."sha1") = 40),
	CONSTRAINT "bases_crc32_format" CHECK("__new_bases"."crc32" not glob '*[^0-9a-f]*' and length("__new_bases"."crc32") = 8)
);
--> statement-breakpoint
DROP TABLE `bases`;--> statement-breakpoint
ALTER TABLE `__new_bases` RENAME TO `bases`;--> statement-breakpoint
CREATE UNIQUE INDEX `bases_internal_name_unique` ON `bases` (`internal_name`);--> statement-breakpoint
CREATE UNIQUE INDEX `bases_sha1_unique` ON `bases` (`sha1`);--> statement-breakpoint
CREATE INDEX `bases_crc32_index` ON `bases` (`crc32`);--> statement-breakpoint
CREATE TABLE `__new_versions` (
	`id` text PRIMARY KEY NOT NULL,
	`game_id` text NOT NULL,
	`internal_name` text NOT NULL,
	`display_name` text NOT NULL,
	`output_sha1` text NOT NULL,
	`required_base_id` text,
	`source_repository_url` text,
	`source_revision` text,
	FOREIGN KEY (`game_id`) REFERENCES `games`(`id`) ON UPDATE cascade ON DELETE cascade,
	FOREIGN KEY (`required_base_id`) REFERENCES `bases`(`id`) ON UPDATE cascade ON DELETE restrict,
	CONSTRAINT "versions_output_sha1_format" CHECK("__new_versions"."output_sha1" not glob '*[^0-9a-f]*' and length("__new_versions"."output_sha1") = 40)
);
--> statement-breakpoint
DROP TABLE `versions`;--> statement-breakpoint
ALTER TABLE `__new_versions` RENAME TO `versions`;--> statement-breakpoint
CREATE UNIQUE INDEX `versions_game_internal_name_unique` ON `versions` (`game_id`,`internal_name`);--> statement-breakpoint
CREATE UNIQUE INDEX `versions_output_sha1_unique` ON `versions` (`output_sha1`);--> statement-breakpoint
CREATE INDEX `versions_game_id_index` ON `versions` (`game_id`);--> statement-breakpoint
CREATE INDEX `versions_required_base_id_index` ON `versions` (`required_base_id`);