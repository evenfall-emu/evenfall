CREATE TABLE `account_limits` (
	`user_id` text PRIMARY KEY NOT NULL,
	`quick_slots` integer,
	CONSTRAINT "account_limits_quick_slots_range" CHECK("account_limits"."quick_slots" is null or "account_limits"."quick_slots" between 0 and 99)
);
