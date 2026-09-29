import type { GameInput, VersionInput } from "../../../../server/src/types";
import type { PatchFormat } from "../../patch";
import type {
  CatalogBase,
  CatalogGameSummary,
  CatalogVersion,
} from "../library/catalog";

export type { GameInput, VersionInput };

/** Every catalog record, including hidden, patchless and release-less ones. */
export type AdminCatalog = {
  /** `GET /api/admin/games` */
  games: CatalogGameSummary[];
  /** `GET /api/admin/versions` */
  versions: CatalogVersion[];
  /** `GET /api/admin/bases` */
  bases: CatalogBase[];
};

/** A request the server rejected, with its per-field validation issues. */
export class ManageError extends Error {
  constructor(
    message: string,
    readonly status: number,
    /** Field name to message, from the server's validation issues. */
    readonly fields: Readonly<Record<string, string>> = {},
  ) {
    super(message);
  }
}

type ErrorBody = {
  error?: {
    message?: string;
    issues?: { path?: (string | number)[]; message?: string }[];
  };
};

async function request<T>(
  method: string,
  path: string,
  body?: { json: unknown } | { bytes: Uint8Array; format: PatchFormat },
  signal?: AbortSignal,
): Promise<T> {
  let response: Response;
  try {
    response = await fetch(path, {
      method,
      credentials: "same-origin",
      cache: "no-store",
      signal,
      headers:
        body && "json" in body
          ? { "Content-Type": "application/json" }
          : body
            ? {
                "Content-Type": "application/octet-stream",
                "X-Patch-Format": body.format,
              }
            : undefined,
      body:
        body && "json" in body
          ? JSON.stringify(body.json)
          : (body?.bytes as BodyInit | undefined),
    });
  } catch (error) {
    if (signal?.aborted) throw error;
    throw new ManageError(
      "Could not reach the server. Check your connection.",
      0,
    );
  }
  if (response.status === 204) return undefined as T;
  const payload = (await response.json().catch(() => ({}))) as T & ErrorBody;
  if (response.ok) return payload;
  const fields: Record<string, string> = {};
  for (const issue of payload.error?.issues ?? []) {
    const field = issue.path?.[0];
    if (typeof field === "string" && issue.message && !(field in fields))
      fields[field] = issue.message;
  }
  throw new ManageError(
    response.status === 401
      ? "Sign in with an admin account to manage the catalog."
      : (payload.error?.message ?? `The server answered ${response.status}.`),
    response.status,
    fields,
  );
}

export async function loadAdminCatalog(
  signal?: AbortSignal,
): Promise<AdminCatalog> {
  const [{ games }, { versions }, { bases }] = await Promise.all([
    request<{ games: CatalogGameSummary[] }>(
      "GET",
      "/api/admin/games",
      undefined,
      signal,
    ),
    request<{ versions: CatalogVersion[] }>(
      "GET",
      "/api/admin/versions",
      undefined,
      signal,
    ),
    request<{ bases: CatalogBase[] }>(
      "GET",
      "/api/admin/bases",
      undefined,
      signal,
    ),
  ]);
  return { games, versions, bases };
}

const games = (id: string) => `/api/admin/games/${encodeURIComponent(id)}`;
const versions = (id: string) =>
  `/api/admin/versions/${encodeURIComponent(id)}`;

/** Creates a game, or fully replaces one with the same ID. */
export const putGame = (id: string, input: GameInput) =>
  request("PUT", games(id), { json: input });

/** Changes only the given fields of an existing game. */
export const patchGame = (id: string, changes: Partial<GameInput>) =>
  request("PATCH", games(id), { json: changes });

/** Deletes a game and, through the database, its releases. */
export const deleteGame = (id: string) => request("DELETE", games(id));

export const putVersion = (id: string, input: VersionInput) =>
  request("PUT", versions(id), { json: input });

export const patchVersion = (id: string, changes: Partial<VersionInput>) =>
  request("PATCH", versions(id), { json: changes });

export const deleteVersion = (id: string) => request("DELETE", versions(id));

/** Stores the patch bytes and points the release at them. */
export const uploadPatch = (
  id: string,
  bytes: Uint8Array,
  format: PatchFormat,
) => request("PUT", `${versions(id)}/patch`, { bytes, format });
