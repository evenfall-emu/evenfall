import { useLayoutEffect, useMemo, useRef, useState } from "react";
import { Icon } from "../../ui/icons";
import { button, cx } from "../../ui/styles";
import type { LibraryAction, LibraryActionHandler } from "./actions";
import type { LibraryCatalog, LibraryDevice } from "./catalog";
import {
  DeleteDialog,
  DescriptionDialog,
  NewPlaythroughDialog,
  ProgressDialog,
  ReleaseNotesDialog,
  RenameDialog,
  UpdateDialog,
} from "./dialogs";
import { GameDetails } from "./game-details";
import { GameList } from "./game-list";
import {
  buildLibrary,
  filterGames,
  franchisesOf,
  noFilters,
  tagsOf,
  type LibraryGame,
  type PlaythroughEntry,
} from "./model";

type OpenDialog =
  | { kind: "update" | "notes" | "description" | "new" }
  | { kind: "progress" | "rename" | "delete"; run: PlaythroughEntry };

// Matches Tailwind's `md` breakpoint: below it, details replace the list.
const narrow = () => window.matchMedia("(max-width: 767.98px)").matches;
const experimentalKey = "evenfall-show-experimental";

/**
 * The game collection: a filterable list beside the selected game's details.
 * The host owns the selection, so it can live in the URL; on narrow screens a
 * selection shows the details instead of the list, and `onBack` returns.
 */
export function LibraryPage({
  catalog,
  dumpNames,
  device,
  now = Date.now(),
  busy = false,
  selectedId,
  onSelect,
  onBack,
  onAddGame,
  onAction,
}: {
  catalog: LibraryCatalog;
  /** SHA-1 to No-Intro name, titling ROMs the catalog does not list. */
  dumpNames?: Readonly<Record<string, string>>;
  device: LibraryDevice;
  now?: number;
  busy?: boolean;
  /** The game whose details show; without one, the first listed game. */
  selectedId?: string;
  onSelect: (gameId: string) => void;
  onBack: () => void;
  onAddGame: () => void;
  onAction: LibraryActionHandler;
}) {
  const [showExperimental, setShowExperimental] = useState(() => {
    try {
      return localStorage.getItem(experimentalKey) === "true";
    } catch {
      return false;
    }
  });
  const games = useMemo(
    () => buildLibrary(catalog, device, dumpNames, showExperimental),
    [catalog, device, dumpNames, showExperimental],
  );
  const franchises = useMemo(() => franchisesOf(games), [games]);
  const tags = useMemo(() => tagsOf(games), [games]);
  const [filters, setFilters] = useState(noFilters);
  const visible = useMemo(() => filterGames(games, filters), [games, filters]);
  // Back to the list keeps the last game selected, so focus can return to it.
  const lastSelected = useRef(selectedId);
  if (selectedId !== undefined) lastSelected.current = selectedId;
  const selected =
    games.find((game) => game.id === lastSelected.current) ?? visible[0];
  const detailOpen = selectedId !== undefined;
  const [dialog, setDialog] = useState<OpenDialog | null>(null);
  const [working, setWorking] = useState(false);
  const pending = useRef(false);
  const [error, setError] = useState("");
  const disabled = busy || working;

  const pageHeading = useRef<HTMLHeadingElement>(null);
  const detailHeading = useRef<HTMLHeadingElement>(null);
  const selectedButton = useRef<HTMLButtonElement>(null);
  const listScroll = useRef(0);
  const fileInput = useRef<HTMLInputElement>(null);
  const fileFor = useRef<LibraryGame>(null);

  useLayoutEffect(() => {
    pageHeading.current?.focus({ preventScroll: true });
  }, []);
  // Narrow screens swap panes; keep the reader's place in each.
  useLayoutEffect(() => {
    if (!narrow()) return;
    if (detailOpen) {
      window.scrollTo(0, 0);
      detailHeading.current?.focus({ preventScroll: true });
    } else {
      window.scrollTo(0, listScroll.current);
      selectedButton.current?.focus({ preventScroll: true });
    }
  }, [detailOpen]);

  async function run(action: LibraryAction) {
    if (disabled || pending.current) return false;
    pending.current = true;
    setWorking(true);
    setError("");
    try {
      await onAction(action);
      return true;
    } catch (failure) {
      setError(failure instanceof Error ? failure.message : String(failure));
      return false;
    } finally {
      pending.current = false;
      setWorking(false);
    }
  }

  async function finish(action: LibraryAction) {
    if (await run(action)) setDialog(null);
  }

  function select(game: LibraryGame) {
    setError("");
    if (narrow()) listScroll.current = window.scrollY;
    onSelect(game.id);
  }

  function install(game: LibraryGame, play = false) {
    void run({ type: "install", versionId: game.latest.id, play });
  }

  function play(game: LibraryGame, entry?: PlaythroughEntry) {
    const release = game.releases.find((item) => item.installed);
    if (entry?.conflict) {
      onSelect(game.id);
      setDialog({ kind: "progress", run: entry });
    } else if (entry)
      void run({
        type: "play",
        hash: entry.release.hash,
        playthroughId: entry.id,
      });
    else if (release) void run({ type: "play", hash: release.hash });
    else install(game, true);
  }

  function addFile(game: LibraryGame) {
    fileFor.current = game;
    fileInput.current?.click();
  }

  function changeExperimental(show: boolean) {
    if (
      !show &&
      selectedId &&
      !buildLibrary(catalog, device, dumpNames).some(
        (game) => game.id === selectedId,
      )
    )
      onBack();
    setShowExperimental(show);
    try {
      localStorage.setItem(experimentalKey, String(show));
    } catch {
      // A disabled store makes the setting last only for this visit.
    }
  }

  const ref = (entry: PlaythroughEntry) => ({
    hash: entry.release.hash,
    playthroughId: entry.id,
  });
  const exportSave = (entry: PlaythroughEntry) =>
    void run({ type: "export-save", ...ref(entry), name: entry.name });
  const conflicted = dialog?.kind === "progress" ? dialog.run : undefined;

  return (
    <main
      data-ui
      aria-label="Library"
      className="grid min-h-0 flex-1 grid-cols-1 bg-canvas font-sans text-2x text-content md:grid-cols-[minmax(360px,5fr)_minmax(0,7fr)]"
      onKeyDown={(event) => {
        if (event.key === "Escape" && detailOpen && !dialog && narrow()) {
          event.preventDefault();
          onBack();
        }
      }}
    >
      <h1 ref={pageHeading} tabIndex={-1} className="sr-only">
        Library
      </h1>
      <section
        aria-label="Games"
        className={cx(
          "min-h-0 border-line md:overflow-y-auto md:border-r",
          detailOpen && "max-md:hidden",
        )}
      >
        <GameList
          games={visible}
          total={games.length}
          filters={filters}
          onFiltersChange={setFilters}
          franchises={franchises}
          tags={tags}
          selectedId={selected?.id}
          selectedRef={selectedButton}
          disabled={disabled}
          showExperimental={showExperimental}
          onShowExperimentalChange={changeExperimental}
          onSelect={select}
          onAddGame={onAddGame}
          onPlay={(game) => play(game, game.playthroughs[0])}
          onInstall={(game) => install(game)}
          onAddFile={addFile}
        />
      </section>
      <section
        aria-label="Game details"
        className={cx(
          "min-h-0 md:overflow-y-auto",
          !detailOpen && "max-md:hidden",
        )}
      >
        {selected ? (
          <GameDetails
            key={selected.id}
            game={selected}
            now={now}
            headingRef={detailHeading}
            disabled={disabled}
            onPlay={(entry) => play(selected, entry)}
            onInstall={() => install(selected)}
            onAddFile={() => addFile(selected)}
            onUpdate={() => setDialog({ kind: "update" })}
            onReleaseNotes={() => setDialog({ kind: "notes" })}
            onDescription={() => setDialog({ kind: "description" })}
            onNewPlaythrough={() => setDialog({ kind: "new" })}
            onRename={(entry) => setDialog({ kind: "rename", run: entry })}
            onExport={exportSave}
            onDelete={(entry) => setDialog({ kind: "delete", run: entry })}
          />
        ) : (
          <p className="p-8 text-muted">Choose a game to see its details.</p>
        )}
      </section>
      {error && (
        <div
          role="alert"
          className="fixed bottom-4 left-1/2 z-60 flex w-max max-w-[calc(100vw-32px)] -translate-x-1/2 items-center gap-3 rounded-panel border border-danger/60 bg-danger-surface py-2 pr-2 pl-4 text-danger shadow-popover"
        >
          <Icon name="alert" className="size-6 shrink-0" />
          <p>{error}</p>
          <button
            className={button("ghost", "icon")}
            aria-label="Dismiss error"
            onClick={() => setError("")}
          >
            <Icon name="x" className="size-5" />
          </button>
        </div>
      )}
      <input
        ref={fileInput}
        type="file"
        accept=".gba,.zip"
        aria-label="Game file"
        tabIndex={-1}
        className="sr-only"
        onChange={(event) => {
          const file = event.target.files?.[0];
          const game = fileFor.current;
          event.target.value = "";
          // An open Update dialog stays, and copies once the ROM is here.
          if (file && game)
            void run({ type: "add-file", file, wanted: game.wantedFiles });
        }}
      />
      {selected && dialog?.kind === "update" && (
        <UpdateDialog
          game={selected}
          busy={working}
          onClose={() => setDialog(null)}
          onAddFile={() => addFile(selected)}
          onConfirm={(entry) =>
            void finish({
              type: "update",
              from: ref(entry),
              toHash: selected.latest.hash,
              name: entry.name,
            })
          }
        />
      )}
      {selected && conflicted?.conflict && (
        <ProgressDialog
          game={selected}
          run={{ ...conflicted, conflict: conflicted.conflict }}
          now={now}
          busy={working}
          onClose={() => setDialog(null)}
          onChoose={(progress) =>
            void finish({ type: "play", ...ref(conflicted), progress })
          }
          onExport={() => exportSave(conflicted)}
        />
      )}
      {selected && dialog?.kind === "notes" && (
        <ReleaseNotesDialog
          game={selected}
          busy={working}
          onClose={() => setDialog(null)}
        />
      )}
      {selected && dialog?.kind === "description" && (
        <DescriptionDialog game={selected} onClose={() => setDialog(null)} />
      )}
      {selected && dialog?.kind === "new" && (
        <NewPlaythroughDialog
          game={selected}
          busy={working}
          onClose={() => setDialog(null)}
          onCreate={(name, hash) =>
            void finish({ type: "new-playthrough", hash, name })
          }
        />
      )}
      {selected && dialog?.kind === "rename" && (
        <RenameDialog
          game={selected}
          run={dialog.run}
          busy={working}
          onClose={() => setDialog(null)}
          onRename={(name) =>
            void finish({
              type: "rename-playthrough",
              ...ref(dialog.run),
              name,
            })
          }
        />
      )}
      {selected && dialog?.kind === "delete" && (
        <DeleteDialog
          game={selected}
          run={dialog.run}
          busy={working}
          onClose={() => setDialog(null)}
          onDelete={() =>
            void finish({ type: "delete-playthrough", ...ref(dialog.run) })
          }
        />
      )}
    </main>
  );
}
