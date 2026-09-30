CREATE TABLE `account_deletions` (
	`user_id` text PRIMARY KEY NOT NULL,
	`requested_at` integer NOT NULL,
	`last_swept_at` integer
);
--> statement-breakpoint
CREATE TABLE `playthrough` (
	`user_id` text NOT NULL,
	`hash` text NOT NULL,
	`id` text NOT NULL,
	`name` text NOT NULL,
	PRIMARY KEY(`user_id`, `hash`, `id`)
);
--> statement-breakpoint
CREATE INDEX `playthrough_user_idx` ON `playthrough` (`user_id`);