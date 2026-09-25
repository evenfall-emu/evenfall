import type {
  CatalogBase,
  CatalogVersion,
  LibraryCatalog,
  LibraryDevice,
} from "./catalog";

/**
 * - `installed`: a release ROM is on this device.
 * - `ready`: the base ROM is here and the latest release's patch is hosted, so
 *   the game plays at once.
 * - `needs-file`: the base ROM, or the release ROM itself, must be added.
 */
export type Availability = "installed" | "ready" | "needs-file";

export type Release = {
  id: string;
  /** "v2.6" */
  label: string;
  installed: boolean;
  /** Milliseconds since the epoch; null when the catalog has no date. */
  releasedAt: number | null;
  releaseNotes: string | null;
  sourceUrl: string | null;
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

export type LibraryGame = {
  id: string;
  title: string;
  summary: string | null;
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
  /** Most recently played first. */
  playthroughs: readonly PlaythroughEntry[];
  lastPlayed?: number;
  /** Playthroughs on an older release, offered the latest by Update. */
  updatable: readonly PlaythroughEntry[];
};

/** A release's date for ordering; undated releases sort after dated ones. */
const dated = (release: Release) => release.releasedAt ?? -Infinity;

function release(version: CatalogVersion, roms: Set<string>): Release {
  return {
    id: version.id,
    label: `v${version.displayName}`,
    installed: roms.has(version.outputSha1),
    releasedAt: version.releasedAt ? Date.parse(version.releasedAt) : null,
    releaseNotes: version.releaseNotes,
    sourceUrl: version.sourceRepositoryUrl,
    requiredBase: version.requiredBase,
    saveCompatibleFrom: version.saveCompatibleFrom,
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
        !local.some((run) => run.id === item.playthroughId),
    )
    .map(
      (item): PlaythroughEntry => ({
        id: item.playthroughId,
        name: item.playthroughName,
        release: releases.get(item.hash)!,
        lastPlayed: item.capturedAt,
        sync: "cloud-only",
      }),
    );
  return [...local, ...remote].sort(
    (a, b) => b.lastPlayed - a.lastPlayed || a.name.localeCompare(b.name),
  );
}

/** Joins the catalog with this device's ROMs, playthroughs and cloud copies. */
export function buildLibrary(
  catalog: LibraryCatalog,
  device: LibraryDevice,
): LibraryGame[] {
  const roms = new Set(device.roms);
  return catalog.games.flatMap((game) => {
    const versions = catalog.versions
      .filter((version) => version.gameId === game.id)
      .map((version) => ({ version, release: release(version, roms) }))
      .sort(
        (a, b) =>
          dated(b.release) - dated(a.release) ||
          a.version.id.localeCompare(b.version.id),
      );
    const newest = versions[0]?.version;
    if (!newest) return [];
    const releases = versions.map((item) => item.release);
    const latest = releases[0];
    const runs = playthroughs(
      new Map(versions.map((item) => [item.version.outputSha1, item.release])),
      device,
    );
    const base = newest.requiredBase;
    const availability: Availability = releases.some((item) => item.installed)
      ? "installed"
      : newest.patch && (!base || roms.has(base.sha1))
        ? "ready"
        : "needs-file";
    return [
      {
        id: game.id,
        title: game.displayName,
        summary: game.summary,
        imageUrl: game.imageUrl,
        franchise: game.franchise,
        tags: game.tags,
        cloudPlaythroughs: game.cloudPlaythroughs,
        availability,
        releases,
        latest,
        ...(availability === "needs-file"
          ? { missingFile: base?.displayName ?? `${game.displayName} ROM` }
          : {}),
        playthroughs: runs,
        lastPlayed: runs[0]?.lastPlayed,
        updatable:
          availability === "installed"
            ? runs.filter((run) => run.release.id !== latest.id)
            : [],
      },
    ];
  });
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

export function filterGames(
  games: readonly LibraryGame[],
  filters: LibraryFilters,
): LibraryGame[] {
  const query = filters.query.trim().toLocaleLowerCase();
  return games
    .filter(
      (game) =>
        game.title.toLocaleLowerCase().includes(query) &&
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
