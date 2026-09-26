import type { WantedFile } from "./model";

/**
 * Everything the Library asks its host to do, resolved to ROM SHA-1s and
 * playthrough IDs. The host owns ROM bytes, the running session and cloud
 * transfers.
 */
export type LibraryAction =
  /**
   * Continue a playthrough, or the ROM's active one when no ID is given.
   * After a cloud conflict, `progress` names the copy to continue.
   */
  | {
      type: "play";
      hash: string;
      playthroughId?: string;
      progress?: "device" | "cloud";
    }
  /**
   * Download a release's patch and apply it to the base ROM here, then play
   * the release when `play` is set.
   */
  | { type: "install"; versionId: string; play?: boolean }
  /** Store a ROM file the user chose, if it is one of the wanted files. */
  | { type: "add-file"; file: File; wanted: readonly WantedFile[] }
  | { type: "new-playthrough"; hash: string; name: string }
  /** Copy a playthrough's in-game save to a new run on another release. */
  | {
      type: "update";
      from: { hash: string; playthroughId: string };
      toHash: string;
      name: string;
    }
  | { type: "export-save"; hash: string; playthroughId: string; name: string }
  | {
      type: "rename-playthrough";
      hash: string;
      playthroughId: string;
      name: string;
    }
  | { type: "delete-playthrough"; hash: string; playthroughId: string };

export type LibraryActionHandler = (
  action: LibraryAction,
) => void | Promise<void>;
