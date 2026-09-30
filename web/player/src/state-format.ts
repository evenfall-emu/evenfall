// Recognizes save states by their first eight bytes. The core checks
// everything else when it loads a state; the player only needs to tell its
// own states from those written by the core it used before.

/** The magic that starts every save state the current core writes. */
export const STATE_MAGIC = new TextEncoder().encode("EVENFALL");

/**
 * Deployed identifier: the magic of save states written by the player's
 * previous core. Browsers and the cloud still hold such resume points, quick
 * slots and shared states. The current core cannot load them, so the player
 * recognizes them, keeps them, and starts from the battery save instead.
 */
export const LEGACY_STATE_MAGIC = new TextEncoder().encode("ZIGBAST1");

const startsWith = (bytes: Uint8Array, magic: Uint8Array) =>
  bytes.length >= magic.length && magic.every((byte, i) => bytes[i] === byte);

/** True for a state the current core may load. */
export const isState = (bytes: Uint8Array) => startsWith(bytes, STATE_MAGIC);

/** True for a state written by the previous core. */
export const isLegacyState = (bytes: Uint8Array) =>
  startsWith(bytes, LEGACY_STATE_MAGIC);

/** A state from the previous core was found where a loadable one was needed. */
export class LegacyStateError extends Error {
  constructor(what: string) {
    super(
      `${what} was saved by an older version of Evenfall and can't be loaded by this one. It is kept unchanged.`,
    );
    this.name = "LegacyStateError";
  }
}

/** Throws LegacyStateError when `bytes` is a state from the previous core. */
export function requireCurrentState(bytes: Uint8Array, what: string) {
  if (isLegacyState(bytes)) throw new LegacyStateError(what);
}
