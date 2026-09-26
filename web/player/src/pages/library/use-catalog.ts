import { useEffect, useState } from "react";
import type {
  CatalogGameSummary,
  CatalogVersion,
  LibraryCatalog,
} from "./catalog";

const storageKey = "evenfall-catalog";
const empty: LibraryCatalog = { games: [], versions: [] };

function cached(): LibraryCatalog {
  try {
    const stored = JSON.parse(localStorage.getItem(storageKey) ?? "null");
    return Array.isArray(stored?.games) && Array.isArray(stored?.versions)
      ? stored
      : empty;
  } catch {
    return empty;
  }
}

async function get<T>(path: string, signal: AbortSignal): Promise<T> {
  const response = await fetch(path, { signal });
  if (!response.ok) throw new Error(`${path} answered ${response.status}`);
  return response.json();
}

/**
 * The public catalog, fetched once per mount. The last catalog read stays in
 * localStorage, so the Library shows it at once and while offline.
 */
export function useCatalog() {
  const [catalog, setCatalog] = useState(cached);
  const [offline, setOffline] = useState(false);
  useEffect(() => {
    const request = new AbortController();
    Promise.all([
      get<{ games: CatalogGameSummary[] }>("/api/games", request.signal),
      get<{ versions: CatalogVersion[] }>("/api/versions", request.signal),
    ]).then(
      ([{ games }, { versions }]) => {
        const next = { games, versions };
        setCatalog(next);
        setOffline(false);
        try {
          localStorage.setItem(storageKey, JSON.stringify(next));
        } catch {
          // A full or disabled store only costs the offline copy.
        }
      },
      () => {
        if (!request.signal.aborted) setOffline(true);
      },
    );
    return () => request.abort();
  }, []);
  return { catalog, offline };
}

/**
 * SHA-1 to No-Intro dump name for every GBA dump. The table is its own chunk,
 * loaded once `wanted` is first true; it is empty until then.
 */
export function useDumpNames(wanted: boolean) {
  const [names, setNames] = useState<Readonly<Record<string, string>>>({});
  const loaded = Object.keys(names).length > 0;
  useEffect(() => {
    if (!wanted || loaded) return;
    let current = true;
    import("./no-intro-gba.json").then(
      (table) => current && setNames(table.default.names),
      () => {},
    );
    return () => {
      current = false;
    };
  }, [wanted, loaded]);
  return names;
}
