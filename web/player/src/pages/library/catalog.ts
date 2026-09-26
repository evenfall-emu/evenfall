import type {
  CatalogGameSummary,
  CatalogPatch,
  CatalogVersion,
} from "../../../../server/src/types";
import type { CloudJob, Game, Playthrough } from "../../library";
import type { CloudResume } from "../../cloud";

export type { CatalogGameSummary, CatalogPatch, CatalogVersion };
export type CatalogBase = NonNullable<CatalogVersion["requiredBase"]>;

/** The catalog as the Library reads it, in the server's response types. */
export type LibraryCatalog = {
  /** `GET /api/games` → `{ games }` */
  games: CatalogGameSummary[];
  /** `GET /api/versions` → `{ versions }` */
  versions: CatalogVersion[];
};

/** What this browser holds, in the shapes the local database and cloud use. */
export type LibraryDevice = {
  /** The `games` store: every ROM on this device, by SHA-1. */
  games: Game[];
  /** The `playthroughs` store. */
  playthroughs: Playthrough[];
  /** Cloud checkpoints discovered for the signed-in account. */
  cloud: CloudResume[];
  /** Resume jobs from the cloud outbox. */
  jobs: Pick<
    CloudJob,
    | "hash"
    | "playthroughId"
    | "kind"
    | "revision"
    | "uploadedRevision"
    | "baseEtag"
  >[];
};
