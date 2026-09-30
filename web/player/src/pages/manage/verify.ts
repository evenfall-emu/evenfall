import { applyPatch, PatchError, type PatchFormat } from "../../patch";
import { sha1 } from "../library/install";

/** ROMs this device can supply: the Library's, by SHA-1, or a chosen file. */
export interface RomSource {
  get(sha1: string): Promise<Uint8Array | undefined>;
  /** Reads a ROM or zipped ROM file and names it by SHA-1. */
  inspect(file: File): Promise<{ hash: string; rom: Uint8Array }>;
}

const magics: readonly [PatchFormat, string][] = [
  ["bps", "BPS1"],
  ["ups", "UPS1"],
  ["ips", "PATCH"],
];

/** The format a patch's header names, or null for any other file. */
export function patchFormat(bytes: Uint8Array): PatchFormat | null {
  const head = String.fromCharCode(...bytes.subarray(0, 5));
  return magics.find(([, magic]) => head.startsWith(magic))?.[0] ?? null;
}

/**
 * Applies a patch to its base on this device and names the ROM it builds.
 * BPS and UPS also check the base's CRC32 and their output's; IPS checks
 * nothing, so the output SHA-1 is its only evidence.
 */
export async function buildRelease(base: Uint8Array, patch: Uint8Array) {
  const format = patchFormat(patch);
  if (!format)
    throw new PatchError("This file is not a BPS, UPS or IPS patch.");
  const rom = applyPatch(format, base, patch);
  return { format, outputSha1: await sha1(rom) };
}
