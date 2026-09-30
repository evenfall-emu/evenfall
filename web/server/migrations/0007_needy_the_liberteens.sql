ALTER TABLE `versions` ADD `visibility` text DEFAULT 'experimental' NOT NULL CHECK(`visibility` in ('featured', 'normal', 'experimental', 'hidden'));
