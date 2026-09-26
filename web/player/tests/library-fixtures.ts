import type {
  CatalogBase,
  CatalogVersion,
  LibraryCatalog,
  LibraryDevice,
} from "../src/pages/library/catalog";

// A catalog and a device in the shapes the catalog API and the local database
// return, for Library tests.

/** Recognizable placeholder digests; fixtures only. */
const hex = (byte: number, bytes: number) =>
  byte.toString(16).padStart(2, "0").repeat(bytes);
const sha = (byte: number) => hex(byte, 20);

const bases = {
  emerald: {
    id: "pokemon-emerald-us",
    internalName: "pokemon/emerald-us",
    displayName: "Pokémon Emerald (US)",
    sha1: sha(0xe0),
    crc32: hex(0xe0, 4),
  },
  fireRed: {
    id: "pokemon-firered-us",
    internalName: "pokemon/firered-us-1.0",
    displayName: "Pokémon FireRed (US 1.0)",
    sha1: sha(0xf0),
    crc32: hex(0xf0, 4),
  },
  fireEmblem: {
    id: "fire-emblem-us",
    internalName: "fire-emblem/fe7-us",
    displayName: "Fire Emblem (US)",
    sha1: sha(0xfe),
    crc32: hex(0xfe, 4),
  },
  lostAge: {
    id: "golden-sun-lost-age-us",
    internalName: "golden-sun/lost-age-us",
    displayName: "Golden Sun: The Lost Age (US)",
    sha1: sha(0x65),
    crc32: hex(0x65, 4),
  },
} satisfies Record<string, CatalogBase>;

const notes =
  "Preview data. Release notes appear here once the catalog provides them.";

function version(
  gameId: string,
  displayName: string,
  output: number,
  base: CatalogBase,
  releasedAt: string | null,
  extra: Partial<CatalogVersion> = {},
): CatalogVersion {
  return {
    id: `${gameId}-${displayName}`,
    gameId,
    internalName: `v${displayName}`,
    displayName,
    outputSha1: sha(output),
    requiredBaseId: base.id,
    requiredBase: base,
    sourceRepositoryUrl: null,
    sourceRevision: null,
    releasedAt,
    releaseNotes: notes,
    patch: {
      url: `https://example.invalid/patches/${gameId}-${displayName}.bps`,
      format: "bps",
      size: 4_194_304,
    },
    saveCompatibleFrom: [],
    ...extra,
  };
}

export const previewCatalog: LibraryCatalog = {
  games: [
    {
      id: "elite-redux",
      internalName: "pokemon/elite-redux",
      displayName: "Elite Redux",
      summary:
        "A reimagining of Pokémon Emerald with higher difficulty, expanded Pokémon, and modern quality of life features.",
      imageUrl: null,
      versionCount: 3,
      franchise: "Pokémon",
      tags: [
        "Pokémon",
        "Difficulty Hack",
        "Expanded abilities",
        "Randomizer",
        "Quality of life",
      ],
      cloudPlaythroughs: 412,
    },
    {
      id: "emerald-seaglass",
      internalName: "pokemon/emerald-seaglass",
      displayName: "Emerald Seaglass",
      summary:
        "Pokémon Emerald with its original story intact and a lighter touch of modern conveniences.",
      imageUrl: null,
      versionCount: 1,
      franchise: "Pokémon",
      tags: [
        "Pokémon",
        "Vanilla+",
        "Quality of life",
        "Physical/special split",
      ],
      cloudPlaythroughs: 96,
    },
    {
      id: "vision-quest",
      internalName: "fire-emblem/vision-quest",
      displayName: "Fire Emblem: Vision Quest",
      summary:
        "A complete new campaign built on Fire Emblem, with its own cast, maps and story.",
      imageUrl: null,
      versionCount: 1,
      franchise: "Fire Emblem",
      tags: ["Fire Emblem", "New campaign", "Story", "Tactics"],
      cloudPlaythroughs: 131,
    },
    {
      id: "golden-sun-reloaded",
      internalName: "golden-sun/reloaded",
      displayName: "Golden Sun: Reloaded",
      summary: "The Lost Age rebalanced for a sharper, longer challenge.",
      imageUrl: null,
      versionCount: 1,
      franchise: "Golden Sun",
      tags: ["Golden Sun", "Difficulty Hack", "Rebalance"],
      cloudPlaythroughs: 27,
    },
    {
      id: "unbound",
      internalName: "pokemon/unbound",
      displayName: "Pokémon Unbound",
      summary:
        "A new region and story built on Pokémon FireRed, with selectable difficulty.",
      imageUrl: null,
      versionCount: 1,
      franchise: "Pokémon",
      tags: ["Pokémon", "New region", "Difficulty options", "Quality of life"],
      cloudPlaythroughs: 587,
    },
  ],
  versions: [
    version("elite-redux", "2.4", 0x24, bases.emerald, "2026-05-02", {
      sourceRepositoryUrl: "https://github.com/Elite-Redux/eliteredux-source",
    }),
    version("elite-redux", "2.5", 0x25, bases.emerald, "2026-07-18", {
      sourceRepositoryUrl: "https://github.com/Elite-Redux/eliteredux-source",
    }),
    version("elite-redux", "2.6", 0x26, bases.emerald, "2026-09-20", {
      sourceRepositoryUrl: "https://github.com/Elite-Redux/eliteredux-source",
    }),
    version("emerald-seaglass", "1.2", 0x12, bases.emerald, "2026-03-11"),
    version("vision-quest", "1.0", 0x10, bases.fireEmblem, "2025-12-01"),
    version("golden-sun-reloaded", "1.1", 0x11, bases.lostAge, "2026-02-14"),
    version("unbound", "2.1.1", 0x21, bases.fireRed, "2025-11-04"),
  ],
};

/** This device's side of the preview, with times relative to `now`. */
export function previewDevice(now = Date.now()): LibraryDevice {
  const at = (daysAgo: number, hours: number, minutes: number) => {
    const date = new Date(now);
    date.setDate(date.getDate() - daysAgo);
    date.setHours(hours, minutes, 0, 0);
    return date.getTime();
  };
  const main = now - 2 * 3_600_000;
  const randomizer = at(3, 14, 20);
  const seaglass = at(1, 18, 40);
  const rom = (hash: string, title: string, lastPlayed = 0) => ({
    hash,
    title,
    romSize: 16_777_216,
    lastPlayed,
  });
  const run = (hash: string, id: string, name: string, lastPlayed: number) => ({
    hash,
    id,
    name,
    createdAt: lastPlayed - 30 * 86_400_000,
    lastPlayed,
    resumeSavedAt: lastPlayed,
  });
  const cloud = (
    hash: string,
    playthroughId: string,
    playthroughName: string,
    title: string,
    capturedAt: number,
    uploadedAt: number,
    etag: string,
  ) => ({
    accountId: "preview",
    hash,
    playthroughId,
    playthroughName,
    title,
    capturedAt,
    uploadedAt,
    etag,
  });
  return {
    games: [
      rom(bases.emerald.sha1, "Pokemon Emerald"),
      rom(bases.fireRed.sha1, "Pokemon FireRed"),
      rom(bases.fireEmblem.sha1, "Fire Emblem"),
      rom(sha(0x25), "Elite Redux 2.5", main),
      rom(sha(0x12), "Emerald Seaglass", seaglass),
    ],
    playthroughs: [
      run(sha(0x25), "main", "Main adventure", main),
      run(sha(0x25), "randomizer", "Randomizer run", randomizer),
      run(sha(0x12), "seaglass", "Playthrough 1", seaglass),
    ],
    cloud: [
      cloud(
        sha(0x25),
        "main",
        "Main adventure",
        "Elite Redux",
        main,
        main + 60_000,
        "main-2",
      ),
      // Another device played on after this one last synced.
      cloud(
        sha(0x25),
        "randomizer",
        "Randomizer run",
        "Elite Redux",
        at(1, 21, 10),
        now - 25 * 60_000,
        "randomizer-4",
      ),
      cloud(
        sha(0x24),
        "challenge",
        "Challenge run",
        "Elite Redux",
        at(5, 19, 30),
        at(5, 19, 32),
        "challenge-1",
      ),
      cloud(
        sha(0x12),
        "seaglass",
        "Playthrough 1",
        "Emerald Seaglass",
        seaglass,
        seaglass + 60_000,
        "seaglass-7",
      ),
    ],
    jobs: [
      {
        hash: sha(0x25),
        playthroughId: "main",
        kind: "resume",
        revision: "main-r2",
        uploadedRevision: "main-r2",
        baseEtag: "main-2",
      },
      {
        hash: sha(0x25),
        playthroughId: "randomizer",
        kind: "resume",
        revision: "randomizer-r9",
        uploadedRevision: "randomizer-r8",
        baseEtag: "randomizer-3",
      },
      {
        hash: sha(0x12),
        playthroughId: "seaglass",
        kind: "resume",
        revision: "seaglass-r7",
        uploadedRevision: "seaglass-r7",
        baseEtag: "seaglass-7",
      },
    ],
  };
}
