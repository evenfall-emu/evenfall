import { applyPatch } from "../../patch";
import type { CatalogVersion } from "./catalog";

/** Largest patch download; a patch never needs to exceed the ROM it builds. */
const MAX_PATCH_SIZE = 40 * 1024 * 1024;

export async function sha1(bytes: Uint8Array<ArrayBuffer>) {
  const digest = new Uint8Array(await crypto.subtle.digest("SHA-1", bytes));
  return Array.from(digest, (byte) => byte.toString(16).padStart(2, "0")).join(
    "",
  );
}

/**
 * Downloads a release's hosted patch and applies it to its base ROM. The
 * result is returned only when its SHA-1 is the release's, so a moved,
 * replaced or damaged patch never becomes a stored ROM.
 */
export async function patchRelease(version: CatalogVersion, base: Uint8Array) {
  const { patch } = version;
  if (!patch) throw new Error("This release has no hosted patch.");
  if (patch.size > MAX_PATCH_SIZE) throw new Error("The patch is too large.");
  let response: Response;
  try {
    response = await fetch(patch.url);
  } catch {
    throw new Error("The patch could not be downloaded. Try again later.");
  }
  if (!response.ok)
    throw new Error(
      `The patch could not be downloaded (HTTP ${response.status}).`,
    );
  const bytes = new Uint8Array(await response.arrayBuffer());
  if (bytes.length !== patch.size)
    throw new Error("The downloaded patch is not the one the catalog lists.");
  const rom = applyPatch(patch.format, base, bytes);
  if ((await sha1(rom)) !== version.outputSha1)
    throw new Error("The patched ROM does not match this release.");
  return rom;
}
