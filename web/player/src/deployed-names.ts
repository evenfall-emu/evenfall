// Deployed identifiers. Players already in use have written data under these
// names into browsers, and the deployed Worker answers to the same header
// names. They keep their original spelling on purpose: renaming one strands
// what was stored under the old name or breaks tabs still running an older
// version, so a rename needs a data migration, not an edit here.

/** Deployed identifier: the IndexedDB database holding games and progress. */
export const LOCAL_DATABASE = "zigba-local";

/** Deployed identifier: the localStorage key of the volume preference. */
export const VOLUME_KEY = "zigba-volume";

/** Deployed identifier: the sessionStorage key that limits automatic update reloads. */
export const UPDATE_RELOAD_KEY = "zigba-update-reloaded-at";

/**
 * Deployed identifier: the prefix of the service worker's per-version caches.
 * A new version prunes older caches by this prefix.
 */
export const OFFLINE_CACHE_PREFIX = "zigba-player-";

/** Deployed identifier: the Web Locks prefix of a playthrough's single-writer lock. */
export const SAVE_LOCK_PREFIX = "zigba-save-";

/** Deployed identifier: the Web Lock that serializes cloud sync across tabs. */
export const CLOUD_SYNC_LOCK = "zigba-cloud-sync";

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
