import { createHash } from "node:crypto";
import { describe, expect, it } from "vite-plus/test";
import type {
  CatalogGameSummary,
  CatalogVersion,
} from "../src/pages/library/catalog";
import { crc32, PatchError } from "../src/patch";
import {
  countFindings,
  filterRows,
  gameChanges,
  gameDraft,
  manageRows,
  releaseChanges,
  releaseDraft,
  slug,
  visibilityCounts,
} from "../src/pages/manage/model";
import { buildRelease, patchFormat } from "../src/pages/manage/verify";

const sha = (byte: number) => byte.toString(16).padStart(2, "0").repeat(20);

function game(
  id: string,
  extra: Partial<CatalogGameSummary> = {},
): CatalogGameSummary {
  return {
    id,
    internalName: id,
    displayName: id,
    summary: null,
    description: null,
    imageUrl: null,
    franchise: null,
    tags: [],
    cloudPlaythroughs: 0,
    versionCount: 0,
    ...extra,
  };
}

function version(
  id: string,
  gameId: string,
  extra: Partial<CatalogVersion> = {},
): CatalogVersion {
  return {
    id,
    gameId,
    internalName: id,
    displayName: id,
    outputSha1: sha(id.length),
    requiredBaseId: null,
    requiredBase: null,
    sourceRepositoryUrl: "https://creator.example",
    sourceRevision: null,
    releasedAt: "2026-01-01",
    releaseNotes: null,
    patch: { url: "https://patches.example/p.bps", format: "bps", size: 10 },
    visibility: "normal",
    saveCompatibleFrom: [],
    ...extra,
  };
}

const catalog = {
  games: [
    game("pokemon-unbound", {
      displayName: "Pokémon Unbound",
      franchise: "Pokémon",
      imageUrl: "https://images.example/unbound.png",
    }),
    game("elite-redux", { displayName: "Elite Redux", franchise: "Pokémon" }),
    game("vision-quest", {
      displayName: "Vision Quest",
      franchise: "Fire Emblem",
      tags: ["Story"],
    }),
  ],
  versions: [
    version("unbound-2", "pokemon-unbound"),
    // Undated releases order by version name, numerically: 2.10 is newest.
    version("er-2.9", "elite-redux", {
      internalName: "2.9",
      releasedAt: null,
      visibility: "experimental",
    }),
    version("er-2.10", "elite-redux", {
      internalName: "2.10",
      releasedAt: null,
      patch: null,
      visibility: "hidden",
    }),
  ],
  bases: [],
};
const rows = manageRows(catalog);
const ids = (list: typeof rows) => list.map((row) => row.game.id);

describe("manageRows", () => {
  it("lists every game with its releases newest first, including none", () => {
    expect(
      rows.map((row) => [row.game.id, row.releases.map((item) => item.id)]),
    ).toEqual([
      ["pokemon-unbound", ["unbound-2"]],
      ["elite-redux", ["er-2.10", "er-2.9"]],
      ["vision-quest", []],
    ]);
  });
});

describe("findings", () => {
  it("counts a game once when any of its releases matches", () => {
    expect(countFindings(rows)).toEqual({
      experimental: 1,
      hidden: 1,
      "no-releases": 1,
      "no-patch": 1,
      "no-cover": 2,
      "no-date": 1,
      "no-source": 0,
    });
  });

  it("filters by finding and by every word of the query, ignoring accents", () => {
    expect(ids(filterRows(rows, "", "no-patch"))).toEqual(["elite-redux"]);
    expect(ids(filterRows(rows, "pokemon", null))).toEqual([
      "pokemon-unbound",
      "elite-redux",
    ]);
    expect(ids(filterRows(rows, "pokemon redux", null))).toEqual([
      "elite-redux",
    ]);
    expect(ids(filterRows(rows, "story", "no-releases"))).toEqual([
      "vision-quest",
    ]);
    expect(ids(filterRows(rows, "unbound", "no-cover"))).toEqual([]);
  });

  it("summarizes visibility most public first, without empty levels", () => {
    expect(visibilityCounts(rows[1].releases)).toEqual([
      { visibility: "experimental", count: 1 },
      { visibility: "hidden", count: 1 },
    ]);
  });
});

describe("slug", () => {
  it("makes IDs the server accepts", () => {
    const serverId = /^[a-z0-9]+(?:[.-][a-z0-9]+)*$/;
    for (const [text, expected] of [
      ["Pokémon Elite Redux", "pokemon-elite-redux"],
      ["2.65 Beta3 (1048)", "2.65-beta3-1048"],
      ["  --v1..0--  ", "v1.0"],
      ["R.O.W.E.", "r.o.w.e"],
      ["Fire Emblem: The Last Promise", "fire-emblem-the-last-promise"],
    ]) {
      expect(slug(text)).toBe(expected);
      expect(slug(text)).toMatch(serverId);
    }
    expect(slug("x".repeat(79) + "-y")).toMatch(serverId);
    expect(slug("x".repeat(79) + "-y").length).toBeLessThanOrEqual(80);
    expect(slug("???")).toBe("");
  });
});

describe("drafts", () => {
  it("sends only changed game fields, clearing emptied ones to null", () => {
    const current = game("vision-quest", {
      displayName: "Vision Quest",
      summary: "A new campaign.",
      franchise: "Fire Emblem",
      tags: ["Story"],
    });
    const draft = gameDraft(current);
    expect(gameChanges(current, draft)).toEqual({});
    expect(
      gameChanges(current, {
        ...draft,
        summary: "   ",
        imageUrl: " https://images.example/vq.png ",
        tags: ["Story", "Hard"],
      }),
    ).toEqual({
      summary: null,
      imageUrl: "https://images.example/vq.png",
      tags: ["Story", "Hard"],
    });
  });

  it("defaults a release's internal name to its label", () => {
    const current = version("er-3", "elite-redux", {
      internalName: "3.0",
      displayName: "3.0",
    });
    const draft = releaseDraft(current);
    expect(releaseChanges(current, draft)).toEqual({});
    expect(
      releaseChanges(current, {
        ...draft,
        displayName: "3.0 Final",
        internalName: "",
        releasedAt: "",
      }),
    ).toEqual({
      displayName: "3.0 Final",
      internalName: "3.0 Final",
      releasedAt: null,
    });
  });
});

const u32 = (value: number) =>
  [0, 8, 16, 24].map((shift) => (value >>> shift) & 0xff);

/** A BPS patch whose one action writes the whole target from the patch. */
function bps(source: Uint8Array, target: Uint8Array) {
  const body = [
    ...new TextEncoder().encode("BPS1"),
    0x80 | source.length,
    0x80 | target.length,
    0x80,
    0x80 | ((target.length - 1) << 2) | 1,
    ...target,
    ...u32(crc32(source)),
    ...u32(crc32(target)),
  ];
  return new Uint8Array([...body, ...u32(crc32(new Uint8Array(body)))]);
}

describe("buildRelease", () => {
  const base = new Uint8Array([1, 2, 3, 4, 5, 6, 7, 8]);
  const target = new Uint8Array([8, 7, 6, 5, 4, 3, 2, 1]);
  const targetSha1 = createHash("sha1").update(target).digest("hex");

  it("names the ROM the patch builds from its base", async () => {
    await expect(buildRelease(base, bps(base, target))).resolves.toEqual({
      format: "bps",
      outputSha1: targetSha1,
    });
  });

  it("rejects a patch for another base and a file that is not a patch", async () => {
    await expect(
      buildRelease(new Uint8Array(8), bps(base, target)),
    ).rejects.toThrow("This patch is for a different base ROM.");
    await expect(buildRelease(base, target)).rejects.toBeInstanceOf(PatchError);
  });

  it("reads the format from the header, not the file name", () => {
    const encode = (text: string) => new TextEncoder().encode(text);
    expect(patchFormat(encode("UPS1…"))).toBe("ups");
    expect(patchFormat(encode("PATCH…"))).toBe("ips");
    expect(patchFormat(encode("BPS1"))).toBe("bps");
    expect(patchFormat(encode("PK\x03\x04"))).toBeNull();
  });
});
