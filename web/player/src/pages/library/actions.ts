/**
 * Everything the Library asks its host to do. The page only describes intent;
 * the host owns ROM bytes, the running session and cloud transfers.
 */
export type LibraryAction =
  /** Continue a playthrough, or the most recent one, or start a first one. */
  | {
      type: "play";
      gameId: string;
      playthroughId?: string;
      /** Which copy to continue after a cloud conflict. */
      progress?: "device" | "cloud";
    }
  /** Download the release's patch and apply it to the base ROM on this device. */
  | { type: "install"; gameId: string; versionId: string }
  /** A base ROM, or the release ROM itself, chosen by the user. */
  | { type: "add-file"; gameId: string; versionId: string; file: File }
  | { type: "new-playthrough"; gameId: string; versionId: string; name: string }
  /** Copy the in-game save into the target release and play it there. */
  | { type: "update"; gameId: string; playthroughId: string; versionId: string }
  | { type: "export-save"; gameId: string; playthroughId: string }
  | { type: "rename-playthrough"; gameId: string; playthroughId: string }
  | { type: "delete-playthrough"; gameId: string; playthroughId: string };

export type LibraryActionHandler = (
  action: LibraryAction,
) => void | Promise<void>;
