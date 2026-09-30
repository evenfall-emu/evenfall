import { createHash } from "node:crypto";
import type { Page } from "@playwright/test";
import { crc32 } from "../../src/patch";
import type {
  CatalogGameSummary,
  CatalogVersion,
} from "../../src/pages/library/catalog";

export const romBytes = (marker: number) =>
  Buffer.from([marker, 0, 160, 227, 254, 255, 255, 234]);
export const loop = (marker: number, name: string) => ({
  name: `${name}.gba`,
  mimeType: "application/octet-stream",
  buffer: romBytes(marker),
});
export const sha1 = (marker: number) =>
  createHash("sha1").update(romBytes(marker)).digest("hex");

function game(
  id: string,
  displayName: string,
  franchise: string,
  tags: string[],
  cloudPlaythroughs: number,
): CatalogGameSummary {
  return {
    id,
    internalName: id,
    displayName,
    summary: `${displayName}, a test catalog entry.`,
    description: null,
    imageUrl: null,
    franchise,
    tags,
    cloudPlaythroughs,
    versionCount: 1,
  };
}

function version(
  gameId: string,
  displayName: string,
  outputSha1: string,
  releasedAt: string,
  extra: Partial<CatalogVersion> = {},
): CatalogVersion {
  return {
    id: `${gameId}-${displayName}`,
    gameId,
    internalName: displayName,
    displayName,
    outputSha1,
    requiredBaseId: "shared-source",
    requiredBase: {
      id: "shared-source",
      internalName: "shared/source",
      displayName: "the Shared Source ROM",
      sha1: sha1(50),
      crc32: "00000000",
    },
    sourceRepositoryUrl: null,
    sourceRevision: null,
    releasedAt,
    releaseNotes: null,
    patch: {
      url: `https://example.invalid/${gameId}-${displayName}.bps`,
      format: "bps",
      size: 1024,
    },
    visibility: "normal",
    saveCompatibleFrom: [],
    ...extra,
  };
}

const mapleBase = {
  id: "maple-us",
  internalName: "maple/us",
  displayName: "Maple (US)",
  sha1: sha1(40),
  crc32: "00000000",
};

const u32 = (value: number) =>
  [0, 8, 16, 24].map((shift) => (value >>> shift) & 0xff);

/** A BPS patch whose one action writes the whole target from the patch. */
export function bps(source: Buffer, target: Buffer) {
  // Each size fits one 7-bit group, which BPS ends with the high bit.
  const body = [
    ...Buffer.from("BPS1"),
    0x80 | source.length,
    0x80 | target.length,
    0x80,
    0x80 | ((target.length - 1) << 2) | 1,
    ...target,
    ...u32(crc32(source)),
    ...u32(crc32(target)),
  ];
  return Buffer.from([...body, ...u32(crc32(new Uint8Array(body)))]);
}

export const maplePatch = bps(romBytes(40), romBytes(41));
const birchPatch = bps(romBytes(50), romBytes(43));

/**
 * Serves a catalog whose ROMs are loop ROMs: Birch Quest v1 is marker 42 and
 * v2 marker 43, patched from marker 50. Cedar Saga is marker 44, and Maple
 * Remix is marker 41, patched from marker 40 by `maplePatch`.
 */
export async function mockCatalog(
  page: Page,
  fillers = 0,
  experimental = false,
) {
  const oaks = Array.from({ length: fillers }, (_, index) =>
    game(`oak-${index + 1}`, `Oak ${index + 1}`, "Trees", [], 0),
  );
  const games = [
    {
      ...game("birch-quest", "Birch Quest", "Trees", ["Adventure", "Hard"], 3),
      summary: "Find your way through an unfamiliar forest.",
      description:
        "Explore the deep woods with a new cast and discover what waits beyond the old trail.",
    },
    game("cedar-saga", "Cedar Saga", "Trees", ["Story"], 9),
    game("maple-remix", "Maple Remix", "Leaves", ["Hard"], 1),
    ...oaks,
  ];
  const versions = [
    version("birch-quest", "1", sha1(42), "2026-01-01T00:00:00.000Z"),
    version("birch-quest", "2", sha1(43), "2026-06-01T00:00:00.000Z", {
      releaseNotes: "Faster text.",
      sourceRepositoryUrl: "https://creator.example/birch-quest/release",
      patch: {
        url: "https://example.invalid/birch-quest-2.bps",
        format: "bps",
        size: birchPatch.length,
      },
    }),
    version("cedar-saga", "1", sha1(44), "2026-03-01T00:00:00.000Z", {
      visibility: experimental ? "experimental" : "normal",
    }),
    version("maple-remix", "1", sha1(41), "2026-02-01T00:00:00.000Z", {
      requiredBaseId: mapleBase.id,
      requiredBase: mapleBase,
      patch: {
        url: "https://example.invalid/maple.bps",
        format: "bps",
        size: maplePatch.length,
      },
    }),
    ...oaks.map((oak) =>
      version(
        oak.id,
        "1",
        createHash("sha1").update(oak.id).digest("hex"),
        "2025-01-01T00:00:00.000Z",
      ),
    ),
  ];
  await page.route("**/api/games", (route) =>
    route.fulfill({ json: { games } }),
  );
  await page.route("**/api/versions", (route) =>
    route.fulfill({ json: { versions } }),
  );
  await page.route("https://example.invalid/maple.bps", (route) =>
    route.fulfill({
      body: maplePatch,
      headers: { "access-control-allow-origin": "*" },
    }),
  );
  await page.route("https://example.invalid/birch-quest-2.bps", (route) =>
    route.fulfill({
      body: birchPatch,
      headers: { "access-control-allow-origin": "*" },
    }),
  );
}
