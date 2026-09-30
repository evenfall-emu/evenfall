import type {
  CatalogBase,
  CatalogVersion,
  LibraryCatalog,
  LibraryDevice,
} from "./catalog";
import { formatTitle } from "./format";

/**
 * - `installed`: a release ROM is on this device.
 * - `ready`: the base ROM is here and the latest release's patch is hosted, so
 *   the game plays at once.
 * - `needs-file`: the base ROM, or the release ROM itself, must be added.
 */
export type Availability = "installed" | "ready" | "needs-file";

export type Release = {
  id: string;
  /** SHA-1 of the release's ROM, which identifies its playthroughs. */
  hash: string;
  /** "v2.6"; empty for a ROM the catalog does not know. */
  label: string;
  installed: boolean;
  /** A hosted patch can build this release from a base ROM on this device. */
  installable: boolean;
  /** Milliseconds since the epoch; null when the catalog has no date. */
  releasedAt: number | null;
  releaseNotes: string | null;
  sourceUrl: string | null;
  patchAvailable: boolean;
  requiredBase: CatalogBase | null;
  /** Release IDs whose in-game saves are verified to load in this one. */
  saveCompatibleFrom: readonly string[];
};

/**
 * - `synced`: the cloud holds this device's latest checkpoint.
 * - `unsynced`: local changes are waiting to upload.
 * - `cloud-newer`: another device uploaded since this one last synced.
 * - `device-only`: no cloud copy, and none pending (cloud backups off).
 * - `cloud-only`: a cloud checkpoint with no copy on this device.
 */
export type SyncState =
  | "synced"
  | "unsynced"
  | "cloud-newer"
  | "device-only"
  | "cloud-only";

export type PlaythroughEntry = {
  id: string;
  name: string;
  release: Release;
  lastPlayed: number;
  sync: SyncState;
  /** Set when both copies exist and the cloud changed since this device synced. */
  conflict?: {
    deviceLastPlayed: number;
    cloudCapturedAt: number;
    cloudUploadedAt: number;
  };
};

/** A ROM file this device may add for a game, identified by its SHA-1. */
export type WantedFile = { sha1: string; title: string };

export type LibraryGame = {
  /** The catalog ID, or the ROM SHA-1 for a game the catalog does not know. */
  id: string;
  title: string;
  fullTitle: string;
  visibility: CatalogVersion["visibility"] | null;
  summary: string | null;
  description: string | null;
  imageUrl: string | null;
  franchise: string | null;
  tags: readonly string[];
  /** Cloud playthroughs across accounts, the measure of popularity. */
  cloudPlaythroughs: number;
  availability: Availability;
  /** Newest first. */
  releases: readonly Release[];
  latest: Release;
  /** What `needs-file` is waiting for, e.g. "Pokémon Emerald (US)". */
  missingFile?: string;
  /** Files Choose file accepts: any release ROM, or the base a patch needs. */
  wantedFiles: readonly WantedFile[];
  /** Most recently played first. */
  playthroughs: readonly PlaythroughEntry[];
  lastPlayed?: number;
  /** Playthroughs on an older release, offered the latest by Update. */
  updatable: readonly PlaythroughEntry[];
};

/** A release's date for ordering; undated releases sort after dated ones. */
const dated = (release: Release) => release.releasedAt ?? -Infinity;
const versionNames = new Intl.Collator("en", {
  numeric: true,
  sensitivity: "base",
});

/** Newest first: dated releases by date, then undated ones by version name. */
export function newestFirst(a: CatalogVersion, b: CatalogVersion) {
  const date = (version: CatalogVersion) =>
    version.releasedAt ? Date.parse(version.releasedAt) : -Infinity;
  return (
    date(b) - date(a) ||
    versionNames.compare(b.internalName, a.internalName) ||
    a.id.localeCompare(b.id)
  );
}

function release(version: CatalogVersion, roms: Set<string>): Release {
  return {
    id: version.id,
    hash: version.outputSha1,
    label: /^\d/.test(version.displayName)
      ? `v${version.displayName}`
      : version.displayName,
    installed: roms.has(version.outputSha1),
    installable: Boolean(
      version.patch &&
      version.requiredBase &&
      roms.has(version.requiredBase.sha1),
    ),
    releasedAt: version.releasedAt ? Date.parse(version.releasedAt) : null,
    releaseNotes: version.releaseNotes,
    sourceUrl: version.sourceRepositoryUrl,
    patchAvailable: Boolean(version.patch),
    requiredBase: version.requiredBase,
    saveCompatibleFrom: version.saveCompatibleFrom,
  };
}

/** The single "release" of a ROM the catalog does not know. */
function unlisted(hash: string, installed: boolean): Release {
  return {
    id: hash,
    hash,
    label: "",
    installed,
    installable: false,
    releasedAt: null,
    releaseNotes: null,
    sourceUrl: null,
    patchAvailable: false,
    requiredBase: null,
    saveCompatibleFrom: [],
  };
}

function playthroughs(
  releases: Map<string, Release>,
  device: LibraryDevice,
): PlaythroughEntry[] {
  const local = device.playthroughs
    .filter((run) => releases.has(run.hash))
    .map((run): PlaythroughEntry => {
      const cloud = device.cloud.find(
        (item) => item.hash === run.hash && item.playthroughId === run.id,
      );
      const job = device.jobs.find(
        (item) =>
          item.kind === "resume" &&
          item.hash === run.hash &&
          item.playthroughId === run.id,
      );
      const unsynced = Boolean(job && job.revision !== job.uploadedRevision);
      // A cloud copy this device never synced with, or one replaced since.
      const changed = Boolean(cloud && cloud.etag !== job?.baseEtag);
      return {
        id: run.id,
        name: run.name,
        release: releases.get(run.hash)!,
        lastPlayed: run.lastPlayed,
        sync: unsynced
          ? "unsynced"
          : !cloud
            ? "device-only"
            : changed
              ? "cloud-newer"
              : "synced",
        ...(cloud && changed
          ? {
              conflict: {
                deviceLastPlayed: run.lastPlayed,
                cloudCapturedAt: cloud.capturedAt,
                cloudUploadedAt: cloud.uploadedAt,
              },
            }
          : {}),
      };
    });
  const remote = device.cloud
    .filter(
      (item) =>
        releases.has(item.hash) &&
        !local.some(
          (run) =>
            run.id === item.playthroughId && run.release.hash === item.hash,
        ),
    )
    .map((item): PlaythroughEntry => ({
      id: item.playthroughId,
      name: item.playthroughName,
      release: releases.get(item.hash)!,
      lastPlayed: item.capturedAt,
      sync: "cloud-only",
    }));
  return [...local, ...remote].sort(
    (a, b) => b.lastPlayed - a.lastPlayed || a.name.localeCompare(b.name),
  );
}

/**
 * Joins the catalog with this device's ROMs, playthroughs and cloud copies.
 * ROMs and cloud playthroughs the catalog does not list as a release become
 * games of their own. Their title is the catalog's base name, else the dump
 * name in `dumps` (SHA-1 to No-Intro name), else the stored title.
 */
export function buildLibrary(
  catalog: LibraryCatalog,
  device: LibraryDevice,
  dumps: Readonly<Record<string, string>> = {},
  showExperimental = false,
): LibraryGame[] {
  const roms = new Set(device.games.map((game) => game.hash));
  const publicVersions = catalog.versions.filter(
    (version) =>
      version.patch &&
      version.visibility !== "hidden" &&
      (showExperimental ||
        version.visibility === "featured" ||
        version.visibility === "normal"),
  );
  const listed = catalog.games.flatMap((game): LibraryGame[] => {
    const versions = publicVersions
      .filter((version) => version.gameId === game.id)
      .map((version) => ({ version, release: release(version, roms) }))
      .sort((a, b) => newestFirst(a.version, b.version));
    const newest = versions[0]?.version;
    if (!newest) return [];
    const releases = versions.map((item) => item.release);
    const latest = releases[0];
    const runs = playthroughs(
      new Map(releases.map((item) => [item.hash, item])),
      device,
    );
    const base = newest.requiredBase;
    const baseHere = Boolean(base && roms.has(base.sha1));
    const availability: Availability = releases.some((item) => item.installed)
      ? "installed"
      : newest.patch && (!base || baseHere)
        ? "ready"
        : "needs-file";
    // A base only helps when a patch can turn it into the release.
    const wantsBase = Boolean(newest.patch && base && !baseHere);
    return [
      {
        id: game.id,
        title: formatTitle(game.displayName, game.franchise),
        fullTitle: game.displayName,
        visibility: newest.visibility ?? "experimental",
        summary: game.summary,
        description: game.description ?? null,
        imageUrl: game.imageUrl,
        franchise: game.franchise,
        tags: game.tags,
        cloudPlaythroughs: game.cloudPlaythroughs,
        availability,
        releases,
        latest,
        ...(availability === "needs-file"
          ? {
              missingFile: wantsBase
                ? base!.displayName
                : `the ${game.displayName} ${latest.label} ROM`,
            }
          : {}),
        wantedFiles: [
          ...releases.map((item) => ({
            sha1: item.hash,
            title: game.displayName,
          })),
          ...(wantsBase
            ? [{ sha1: base!.sha1, title: base!.displayName }]
            : []),
        ],
        playthroughs: runs,
        lastPlayed: runs[0]?.lastPlayed,
        updatable:
          availability === "installed"
            ? runs.filter((run) => run.release.id !== latest.id)
            : [],
      },
    ];
  });
  const outputs = new Set(publicVersions.map((version) => version.outputSha1));
  const bases = new Map(
    publicVersions.flatMap((version) =>
      version.requiredBase
        ? [[version.requiredBase.sha1, version.requiredBase.displayName]]
        : [],
    ),
  );
  const titles = new Map<string, string>();
  for (const { hash, title } of [...device.cloud, ...device.games])
    if (!outputs.has(hash))
      titles.set(
        hash,
        bases.get(hash) ??
          (Object.hasOwn(dumps, hash) ? dumps[hash] : undefined) ??
          title,
      );
  const unknown = [...titles].map(([hash, title]): LibraryGame => {
    const installed = roms.has(hash);
    const only = unlisted(hash, installed);
    const runs = playthroughs(new Map([[hash, only]]), device);
    return {
      id: hash,
      title,
      fullTitle: title,
      visibility: null,
      summary: null,
      description: null,
      imageUrl: null,
      franchise: null,
      tags: [],
      cloudPlaythroughs: 0,
      availability: installed ? "installed" : "needs-file",
      releases: [only],
      latest: only,
      ...(installed ? {} : { missingFile: `the ${title} ROM` }),
      wantedFiles: [{ sha1: hash, title }],
      playthroughs: runs,
      lastPlayed: runs[0]?.lastPlayed,
      updatable: [],
    };
  });
  return [...listed, ...unknown];
}

export type ShowFilter = "all" | Exclude<Availability, "needs-file">;
/** Latest release first, most recently played first, or most launched first. */
export type SortOrder = "updated" | "played" | "popularity";

export type LibraryFilters = {
  query: string;
  show: ShowFilter;
  /** Empty for every franchise. */
  franchise: string;
  /** A game must carry every selected tag. */
  tags: readonly string[];
  sort: SortOrder;
};

export const noFilters: LibraryFilters = {
  query: "",
  show: "all",
  franchise: "",
  tags: [],
  sort: "played",
};

const order: Record<SortOrder, (a: LibraryGame, b: LibraryGame) => number> = {
  updated: (a, b) => dated(b.latest) - dated(a.latest) || 0,
  // Unplayed games follow every played one.
  played: (a, b) =>
    (b.lastPlayed ?? -Infinity) - (a.lastPlayed ?? -Infinity) || 0,
  popularity: (a, b) => b.cloudPlaythroughs - a.cloudPlaythroughs,
};

/**
 * A search key: case, accents and punctuation folded away, so "poke" finds
 * "Pokémon" and "emblem vision" finds "Fire Emblem: Vision Quest".
 */
function searchKey(text: string): string {
  return text
    .normalize("NFKD")
    .replace(/\p{M}/gu, "")
    .toLowerCase()
    .replace(/[^\p{L}\p{N}]+/gu, " ")
    .trim();
}

export function filterGames(
  games: readonly LibraryGame[],
  filters: LibraryFilters,
): LibraryGame[] {
  const query = searchKey(filters.query);
  return games
    .filter(
      (game) =>
        searchKey(game.fullTitle).includes(query) &&
        (filters.show === "all" || game.availability === filters.show) &&
        (!filters.franchise || game.franchise === filters.franchise) &&
        filters.tags.every((tag) => game.tags.includes(tag)),
    )
    .sort(
      (a, b) =>
        order[filters.sort](a, b) ||
        a.title.localeCompare(b.title) ||
        a.id.localeCompare(b.id),
    );
}

export function franchisesOf(games: readonly LibraryGame[]) {
  return [...new Set(games.flatMap((game) => game.franchise ?? []))].sort(
    (a, b) => a.localeCompare(b),
  );
}

export function tagsOf(games: readonly LibraryGame[]) {
  return [...new Set(games.flatMap((game) => game.tags))].sort((a, b) =>
    a.localeCompare(b),
  );
}

/** Whether a save from `from` is verified to load in `to`. */
export function saveCompatible(from: Release, to: Release) {
  return from.id === to.id || to.saveCompatibleFrom.includes(from.id);
}
