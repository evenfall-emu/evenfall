CREATE TABLE `bases` (
	`id` text PRIMARY KEY NOT NULL,
	`internal_name` text NOT NULL,
	`display_name` text NOT NULL,
	`sha256` text,
	CONSTRAINT "bases_sha256_format" CHECK("bases"."sha256" is null or ("bases"."sha256" not glob '*[^0-9a-f]*' and length("bases"."sha256") = 64))
);
--> statement-breakpoint
CREATE UNIQUE INDEX `bases_internal_name_unique` ON `bases` (`internal_name`);--> statement-breakpoint
CREATE UNIQUE INDEX `bases_sha256_unique` ON `bases` (`sha256`);--> statement-breakpoint
CREATE TABLE `games` (
	`id` text PRIMARY KEY NOT NULL,
	`internal_name` text NOT NULL,
	`display_name` text NOT NULL,
	`summary` text,
	`image_url` text
);
--> statement-breakpoint
CREATE UNIQUE INDEX `games_internal_name_unique` ON `games` (`internal_name`);--> statement-breakpoint
CREATE TABLE `versions` (
	`id` text PRIMARY KEY NOT NULL,
	`game_id` text NOT NULL,
	`internal_name` text NOT NULL,
	`display_name` text NOT NULL,
	`output_sha256` text NOT NULL,
	`required_base_id` text,
	`source_repository_url` text,
	`source_revision` text,
	FOREIGN KEY (`game_id`) REFERENCES `games`(`id`) ON UPDATE cascade ON DELETE cascade,
	FOREIGN KEY (`required_base_id`) REFERENCES `bases`(`id`) ON UPDATE cascade ON DELETE restrict,
	CONSTRAINT "versions_output_sha256_format" CHECK("versions"."output_sha256" not glob '*[^0-9a-f]*' and length("versions"."output_sha256") = 64)
);
--> statement-breakpoint
CREATE UNIQUE INDEX `versions_game_internal_name_unique` ON `versions` (`game_id`,`internal_name`);--> statement-breakpoint
CREATE UNIQUE INDEX `versions_output_sha256_unique` ON `versions` (`output_sha256`);--> statement-breakpoint
CREATE INDEX `versions_game_id_index` ON `versions` (`game_id`);--> statement-breakpoint
CREATE INDEX `versions_required_base_id_index` ON `versions` (`required_base_id`);