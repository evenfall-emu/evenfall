-- Catalog fields the Library shows: a game's franchise and tags, and each
-- version's release date, notes, hosted patch and verified save
-- compatibility. Existing rows keep their data; the new fields start empty.
PRAGMA defer_foreign_keys = on;--> statement-breakpoint
ALTER TABLE `games` ADD `franchise` text;--> statement-breakpoint
CREATE TABLE `__new_versions` (
	`id` text PRIMARY KEY NOT NULL,
	`game_id` text NOT NULL,
	`internal_name` text NOT NULL,
	`display_name` text NOT NULL,
	`output_sha1` text NOT NULL,
	`required_base_id` text,
	`source_repository_url` text,
	`source_revision` text,
	`released_at` text,
	`release_notes` text,
	`patch_url` text,
	`patch_format` text,
	`patch_size` integer,
	FOREIGN KEY (`game_id`) REFERENCES `games`(`id`) ON UPDATE cascade ON DELETE cascade,
	FOREIGN KEY (`required_base_id`) REFERENCES `bases`(`id`) ON UPDATE cascade ON DELETE restrict,
	CONSTRAINT "versions_output_sha1_format" CHECK("__new_versions"."output_sha1" not glob '*[^0-9a-f]*' and length("__new_versions"."output_sha1") = 40),
	CONSTRAINT "versions_released_at_format" CHECK("__new_versions"."released_at" is null or "__new_versions"."released_at" glob '[0-9][0-9][0-9][0-9]-[01][0-9]-[0-3][0-9]'),
	CONSTRAINT "versions_patch_complete" CHECK(("__new_versions"."patch_url" is null) = ("__new_versions"."patch_format" is null) and ("__new_versions"."patch_url" is null) = ("__new_versions"."patch_size" is null)),
	CONSTRAINT "versions_patch_base" CHECK("__new_versions"."patch_url" is null or "__new_versions"."required_base_id" is not null),
	CONSTRAINT "versions_patch_format" CHECK("__new_versions"."patch_format" is null or "__new_versions"."patch_format" in ('bps', 'ups', 'ips')),
	CONSTRAINT "versions_patch_size" CHECK("__new_versions"."patch_size" is null or "__new_versions"."patch_size" > 0)
);
--> statement-breakpoint
INSERT INTO `__new_versions`("id", "game_id", "internal_name", "display_name", "output_sha1", "required_base_id", "source_repository_url", "source_revision") SELECT "id", "game_id", "internal_name", "display_name", "output_sha1", "required_base_id", "source_repository_url", "source_revision" FROM `versions`;--> statement-breakpoint
DROP TABLE `versions`;--> statement-breakpoint
ALTER TABLE `__new_versions` RENAME TO `versions`;--> statement-breakpoint
CREATE UNIQUE INDEX `versions_game_internal_name_unique` ON `versions` (`game_id`,`internal_name`);--> statement-breakpoint
CREATE UNIQUE INDEX `versions_output_sha1_unique` ON `versions` (`output_sha1`);--> statement-breakpoint
CREATE INDEX `versions_game_id_index` ON `versions` (`game_id`);--> statement-breakpoint
CREATE INDEX `versions_required_base_id_index` ON `versions` (`required_base_id`);--> statement-breakpoint
CREATE TABLE `game_tags` (
	`game_id` text NOT NULL,
	`tag` text NOT NULL,
	`position` integer NOT NULL,
	PRIMARY KEY(`game_id`, `tag`),
	FOREIGN KEY (`game_id`) REFERENCES `games`(`id`) ON UPDATE cascade ON DELETE cascade
);
--> statement-breakpoint
CREATE INDEX `game_tags_tag_index` ON `game_tags` (`tag`);--> statement-breakpoint
CREATE TABLE `version_save_compatibility` (
	`version_id` text NOT NULL,
	`from_version_id` text NOT NULL,
	PRIMARY KEY(`version_id`, `from_version_id`),
	FOREIGN KEY (`version_id`) REFERENCES `versions`(`id`) ON UPDATE cascade ON DELETE cascade,
	FOREIGN KEY (`from_version_id`) REFERENCES `versions`(`id`) ON UPDATE cascade ON DELETE cascade,
	CONSTRAINT "version_save_compatibility_distinct" CHECK("version_save_compatibility"."version_id" <> "version_save_compatibility"."from_version_id")
);
--> statement-breakpoint
CREATE INDEX `version_save_compatibility_from_index` ON `version_save_compatibility` (`from_version_id`);--> statement-breakpoint
CREATE INDEX `playthrough_hash_idx` ON `playthrough` (`hash`);
