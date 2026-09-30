import type { CatalogGameSummary, CatalogVersion } from "../library/catalog";
import { newestFirst } from "../library/model";
import type { AdminCatalog, GameInput, VersionInput } from "./api";

export type Visibility = CatalogVersion["visibility"];

/** Most to least public. */
export const visibilities = [
  "featured",
  "normal",
  "experimental",
  "hidden",
] as const satisfies readonly Visibility[];

export const visibilityLabels: Record<Visibility, string> = {
  featured: "Featured",
  normal: "Normal",
  experimental: "Experimental",
  hidden: "Hidden",
};

/** A game and every one of its releases, newest first. */
export type ManageRow = {
  game: CatalogGameSummary;
  releases: CatalogVersion[];
};

export function manageRows(catalog: AdminCatalog): ManageRow[] {
  const releases = new Map<string, CatalogVersion[]>();
  for (const version of catalog.versions)
    releases.set(version.gameId, [
      ...(releases.get(version.gameId) ?? []),
      version,
    ]);
  return catalog.games.map((game) => ({
    game,
    releases: (releases.get(game.id) ?? []).sort(newestFirst),
  }));
}

const anyRelease =
  (test: (version: CatalogVersion) => boolean) => (row: ManageRow) =>
    row.releases.some(test);

/**
 * What a game still holds back from players, or lacks. Each is a filter; a
 * game matches when any of its releases does.
 */
export const findings = {
  experimental: {
    label: "Experimental",
    matches: anyRelease((version) => version.visibility === "experimental"),
  },
  hidden: {
    label: "Hidden",
    matches: anyRelease((version) => version.visibility === "hidden"),
  },
  "no-releases": {
    label: "No releases",
    matches: (row: ManageRow) => row.releases.length === 0,
  },
  "no-patch": {
    label: "No patch",
    matches: anyRelease((version) => !version.patch),
  },
  "no-cover": {
    label: "No cover image",
    matches: (row: ManageRow) => !row.game.imageUrl,
  },
  "no-date": {
    label: "No release date",
    matches: anyRelease((version) => !version.releasedAt),
  },
  "no-source": {
    label: "No source link",
    matches: anyRelease((version) => !version.sourceRepositoryUrl),
  },
} satisfies Record<
  string,
  { label: string; matches: (row: ManageRow) => boolean }
>;

export type Finding = keyof typeof findings;

export const findingKeys = Object.keys(findings) as Finding[];

export function countFindings(rows: readonly ManageRow[]) {
  return Object.fromEntries(
    findingKeys.map((key) => [
      key,
      rows.filter((row) => findings[key].matches(row)).length,
    ]),
  ) as Record<Finding, number>;
}

/** Lowercase without accents, so "pokemon" finds "Pokémon". */
const fold = (text: string) =>
  text
    .normalize("NFD")
    .replace(/\p{Diacritic}/gu, "")
    .toLocaleLowerCase();

/** Rows matching the finding and every word of the query. */
export function filterRows(
  rows: readonly ManageRow[],
  query: string,
  finding: Finding | null,
) {
  const words = fold(query).split(/\s+/).filter(Boolean);
  return rows.filter((row) => {
    if (finding && !findings[finding].matches(row)) return false;
    const text = fold(
      [
        row.game.displayName,
        row.game.id,
        row.game.franchise ?? "",
        ...row.game.tags,
      ].join(" "),
    );
    return words.every((word) => text.includes(word));
  });
}

/** How many releases have each visibility, most public first, omitting zeros. */
export function visibilityCounts(releases: readonly CatalogVersion[]) {
  return visibilities
    .map((visibility) => ({
      visibility,
      count: releases.filter((version) => version.visibility === visibility)
        .length,
    }))
    .filter((entry) => entry.count > 0);
}

/**
 * A catalog ID from free text: lowercase letters and digits with single `-`
 * or `.` separators, at most 80 characters, as the server requires.
 */
export function slug(text: string) {
  return fold(text)
    .replace(/[^a-z0-9.]+/g, "-")
    .replace(/([.-])[.-]+/g, "$1")
    .slice(0, 80)
    .replace(/^[.-]+|[.-]+$/g, "");
}

/** "BPS · 1.2 MB" */
export function patchSummary(patch: NonNullable<CatalogVersion["patch"]>) {
  const size =
    patch.size >= 1_000_000
      ? `${(patch.size / 1_000_000).toFixed(1)} MB`
      : `${Math.max(1, Math.round(patch.size / 1_000))} KB`;
  return `${patch.format.toUpperCase()} · ${size}`;
}

/** The first and last characters of a SHA-1, as "f3ae0881…1d07b7". */
export const shortHash = (hash: string) =>
  `${hash.slice(0, 8)}…${hash.slice(-6)}`;

/** A form's text: an empty optional field is stored as null. */
const text = (value: string) => value.trim() || null;

/** The fields that differ, compared as JSON values. */
function changed<T extends object>(current: T, next: T): Partial<T> {
  return Object.fromEntries(
    Object.entries(next).filter(
      ([key, value]) =>
        JSON.stringify(value) !== JSON.stringify(current[key as keyof T]),
    ),
  ) as Partial<T>;
}

/** A game's editable fields as the form holds them. */
export type GameDraft = {
  displayName: string;
  internalName: string;
  summary: string;
  description: string;
  imageUrl: string;
  franchise: string;
  tags: string[];
};

export const gameDraft = (game: CatalogGameSummary): GameDraft => ({
  displayName: game.displayName,
  internalName: game.internalName,
  summary: game.summary ?? "",
  description: game.description ?? "",
  imageUrl: game.imageUrl ?? "",
  franchise: game.franchise ?? "",
  tags: [...game.tags],
});

export const gameFields = (draft: GameDraft): GameInput => ({
  displayName: draft.displayName.trim(),
  internalName: draft.internalName.trim(),
  summary: text(draft.summary),
  description: text(draft.description),
  imageUrl: text(draft.imageUrl),
  franchise: text(draft.franchise),
  tags: draft.tags,
});

/** What saving the draft would change on the game. */
export const gameChanges = (game: CatalogGameSummary, draft: GameDraft) =>
  changed(gameFields(gameDraft(game)), gameFields(draft));

/** A release's descriptive fields; its identity and patch are set apart. */
export type ReleaseDraft = {
  displayName: string;
  internalName: string;
  releasedAt: string;
  sourceRepositoryUrl: string;
  sourceRevision: string;
  releaseNotes: string;
  saveCompatibleFrom: string[];
};

export const releaseDraft = (version?: CatalogVersion): ReleaseDraft => ({
  displayName: version?.displayName ?? "",
  internalName: version?.internalName ?? "",
  releasedAt: version?.releasedAt ?? "",
  sourceRepositoryUrl: version?.sourceRepositoryUrl ?? "",
  sourceRevision: version?.sourceRevision ?? "",
  releaseNotes: version?.releaseNotes ?? "",
  saveCompatibleFrom: [...(version?.saveCompatibleFrom ?? [])],
});

export const releaseFields = (draft: ReleaseDraft) =>
  ({
    displayName: draft.displayName.trim(),
    // The internal name defaults to the label.
    internalName: draft.internalName.trim() || draft.displayName.trim(),
    releasedAt: text(draft.releasedAt),
    sourceRepositoryUrl: text(draft.sourceRepositoryUrl),
    sourceRevision: text(draft.sourceRevision),
    releaseNotes: text(draft.releaseNotes),
    saveCompatibleFrom: draft.saveCompatibleFrom,
  }) satisfies Partial<VersionInput>;

/** What saving the draft would change on the release. */
export const releaseChanges = (version: CatalogVersion, draft: ReleaseDraft) =>
  changed(releaseFields(releaseDraft(version)), releaseFields(draft));
