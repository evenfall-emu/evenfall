import type {
  CatalogGameSummary,
  CatalogPatch,
  CatalogVersion,
} from "../../../../server/src/types";
import type { CloudJob, Playthrough } from "../../library";
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
  /** SHA-1 of every ROM on this device: installed releases and base ROMs. */
  roms: string[];
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
