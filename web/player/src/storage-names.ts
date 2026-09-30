// Names under which the player keeps data in the browser. Earlier players
// used the names in LEGACY; those remain only so that storage-migration.ts can
// find and move what they stored.

/** The IndexedDB database holding games, ROMs and progress. */
export const LOCAL_DATABASE = "evenfall-local";

/** The localStorage key of the volume preference. */
export const VOLUME_KEY = "evenfall-volume";

/** The sessionStorage key that limits automatic update reloads. */
export const UPDATE_RELOAD_KEY = "evenfall-update-reloaded-at";

/**
 * The prefix of the service worker's per-version caches. A new version prunes
 * older caches by this prefix and the legacy one.
 */
export const OFFLINE_CACHE_PREFIX = "evenfall-player-";

/** The Web Locks prefix of a playthrough's single-writer lock. */
export const SAVE_LOCK_PREFIX = "evenfall-save-";

/** The Web Lock that serializes cloud sync across tabs. */
export const CLOUD_SYNC_LOCK = "evenfall-cloud-sync";

/** The Web Lock that lets one tab at a time move legacy data. */
export const STORAGE_MIGRATION_LOCK = "evenfall-storage-migration";

/**
 * Deployed identifiers: the names older players stored data under. The
 * migration reads them once and moves their data to the names above.
 */
export const LEGACY = {
  database: "zigba-local",
  volumeKey: "zigba-volume",
  offlineCachePrefix: "zigba-player-",
} as const;

/** Deployed identifier: the cloud protocol's headers, shared with the Worker. */
export const HEADER = {
  account: "X-Zigba-Account",
  rom: "X-Zigba-Rom",
  revision: "X-Zigba-Revision",
  capturedAt: "X-Zigba-Captured-At",
  digest: "X-Zigba-Digest",
  title: "X-Zigba-Title",
  playthroughName: "X-Zigba-Playthrough-Name",
  uploadedAt: "X-Zigba-Uploaded-At",
  romIncluded: "X-Zigba-Rom-Included",
} as const;
