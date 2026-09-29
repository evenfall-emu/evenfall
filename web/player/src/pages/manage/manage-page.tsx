import {
  useCallback,
  useEffect,
  useLayoutEffect,
  useMemo,
  useRef,
  useState,
} from "react";
import { Dialog, DialogActions } from "../../ui/dialog";
import { Icon, type IconName } from "../../ui/icons";
import { button, chip, cx, focusRing } from "../../ui/styles";
import { Cover } from "../library/cover";
import { loadAdminCatalog, putGame, type AdminCatalog } from "./api";
import { FormError, TextField } from "./fields";
import { GamePanel, type PanelTab } from "./game-panel";
import {
  countFindings,
  filterRows,
  findingKeys,
  findings,
  manageRows,
  slug,
  visibilityCounts,
  visibilityLabels,
  type Finding,
  type ManageRow,
  type Visibility,
} from "./model";
import type { Commit } from "./release-dialogs";
import type { RomSource } from "./verify";

/** Whether the signed-in account may use this page, as `/api/me` says. */
export type ManageAccess = "signed-out" | "checking" | "denied" | "admin";

const message = (error: unknown) =>
  error instanceof Error ? error.message : String(error);

const visibilityChip: Record<Visibility, string> = {
  featured: "border-peach/60 text-peach",
  normal: "border-mint/60 text-mint",
  experimental: "border-warning/60 text-warning",
  hidden: "border-line text-muted",
};

/** The gaps the Missing column shows, as icons. */
const gapIcons: { finding: Finding; icon: IconName }[] = [
  { finding: "no-patch", icon: "filePlus" },
  { finding: "no-cover", icon: "image" },
  { finding: "no-date", icon: "calendar" },
  { finding: "no-source", icon: "link" },
];

function useAdminCatalog(enabled: boolean) {
  const [catalog, setCatalog] = useState<AdminCatalog | null>(null);
  const [error, setError] = useState("");
  const reload = useCallback(async (signal?: AbortSignal) => {
    try {
      setCatalog(await loadAdminCatalog(signal));
      setError("");
    } catch (failure) {
      if (!signal?.aborted) setError(message(failure));
    }
  }, []);
  useEffect(() => {
    if (!enabled) return;
    const request = new AbortController();
    void reload(request.signal);
    return () => request.abort();
  }, [enabled, reload]);
  return { catalog, error, reload };
}

function Notice({
  title,
  children,
}: {
  title: string;
  children?: React.ReactNode;
}) {
  return (
    <main
      data-ui
      aria-label="Manage catalog"
      className="grid min-h-0 flex-1 place-items-center bg-canvas p-8 font-sans text-2x text-content"
    >
      <div className="max-w-xl space-y-4 text-center">
        <h1 className="text-3x font-bold text-heading">{title}</h1>
        {children}
      </div>
    </main>
  );
}

function AddGameDialog({
  catalog,
  franchises,
  commit,
  onClose,
  onCreated,
}: {
  catalog: AdminCatalog;
  franchises: readonly string[];
  commit: Commit;
  onClose: () => void;
  onCreated: (id: string) => void;
}) {
  const [name, setName] = useState("");
  const [id, setId] = useState<string | null>(null);
  const [franchise, setFranchise] = useState("");
  const [summary, setSummary] = useState("");
  const [busy, setBusy] = useState(false);
  const [error, setError] = useState("");
  const [errors, setErrors] = useState<Record<string, string>>({});
  const gameId = id ?? slug(name);

  async function create() {
    const problems: Record<string, string> = {};
    if (!name.trim()) problems.displayName = "Enter the game's name.";
    if (!gameId || slug(gameId) !== gameId)
      problems.id =
        "Use lowercase letters and digits, separated by single - or . characters.";
    else if (catalog.games.some((game) => game.id === gameId))
      problems.id = "Another game has this ID.";
    setErrors(problems);
    setError("");
    if (Object.keys(problems).length) return;
    setBusy(true);
    try {
      await commit(() =>
        putGame(gameId, {
          internalName: gameId,
          displayName: name.trim(),
          summary: summary.trim() || null,
          description: null,
          imageUrl: null,
          franchise: franchise.trim() || null,
          tags: [],
        }),
      );
      onCreated(gameId);
    } catch (failure) {
      setError(message(failure));
    } finally {
      setBusy(false);
    }
  }

  return (
    <Dialog
      open
      onOpenChange={(open) => !open && onClose()}
      busy={busy}
      title="Add game"
      subtitle="Players see it once it has a release with a patch."
    >
      <form
        className="mt-5 space-y-5"
        onSubmit={(event) => {
          event.preventDefault();
          void create();
        }}
      >
        <TextField
          label="Name"
          value={name}
          onChange={setName}
          error={errors.displayName}
          maxLength={160}
          required
          autoFocus
          disabled={busy}
        />
        <TextField
          label="Game ID"
          value={gameId}
          onChange={setId}
          hint="Permanent: it is the game's address, /library/<ID>."
          error={errors.id}
          maxLength={80}
          disabled={busy}
        />
        <TextField
          label="Franchise"
          value={franchise}
          onChange={setFranchise}
          list="manage-franchises"
          maxLength={80}
          disabled={busy}
        />
        <datalist id="manage-franchises">
          {franchises.map((item) => (
            <option key={item} value={item} />
          ))}
        </datalist>
        <TextField
          label="Summary"
          rows={2}
          value={summary}
          onChange={setSummary}
          hint="One line under the title in the Library."
          maxLength={2000}
          disabled={busy}
        />
        <FormError message={error} />
        <DialogActions>
          <button
            type="button"
            className={button("outline")}
            disabled={busy}
            onClick={onClose}
          >
            Cancel
          </button>
          <button type="submit" className={button("primary")} disabled={busy}>
            {busy ? "Adding…" : "Add game"}
          </button>
        </DialogActions>
      </form>
    </Dialog>
  );
}

function ReleaseChips({ row }: { row: ManageRow }) {
  if (!row.releases.length)
    return <span className="text-1x5 text-warning">No releases</span>;
  return (
    <ul aria-label="Releases" className="flex flex-wrap gap-2">
      {visibilityCounts(row.releases).map(({ visibility, count }) => (
        <li key={visibility} className={cx(chip, visibilityChip[visibility])}>
          {visibilityLabels[visibility]}
          {count > 1 && ` ×${count}`}
        </li>
      ))}
    </ul>
  );
}

function Gaps({ row }: { row: ManageRow }) {
  const gaps = gapIcons.filter(({ finding }) => findings[finding].matches(row));
  if (!gaps.length) return <span className="text-1x5 text-muted">Nothing</span>;
  return (
    <ul aria-label="Missing" className="flex gap-2 text-warning">
      {gaps.map(({ finding, icon }) => (
        <li key={finding} title={findings[finding].label}>
          <Icon name={icon} className="size-6" />
          <span className="sr-only">{findings[finding].label}</span>
        </li>
      ))}
    </ul>
  );
}

/** Below `md`, releases and gaps move under the name instead of columns. */
function GameRow({
  row,
  selected,
  onSelect,
}: {
  row: ManageRow;
  selected: boolean;
  onSelect: () => void;
}) {
  const { game } = row;
  return (
    <tr
      className={cx(
        "relative border-b border-line hover:bg-hover",
        selected && "bg-selected",
      )}
    >
      <td className="py-3 pr-4 pl-2">
        <div className="flex items-center gap-4">
          <Cover
            id={game.id}
            title={game.displayName}
            imageUrl={game.imageUrl}
            size="sm"
          />
          <div className="min-w-0">
            <button
              aria-current={selected ? "true" : undefined}
              onClick={onSelect}
              className={cx(
                "cursor-pointer rounded-control text-left font-bold break-words after:absolute after:inset-0",
                selected ? "text-mint" : "text-content",
                focusRing,
              )}
            >
              {game.displayName}
            </button>
            <p className="mt-1 text-1x5 text-muted">
              {game.franchise ?? "No franchise"}
            </p>
            <div className="mt-2 flex flex-wrap items-center gap-3 md:hidden">
              <ReleaseChips row={row} />
              <Gaps row={row} />
            </div>
          </div>
        </div>
      </td>
      <td className="py-3 pr-4 max-md:hidden">
        <ReleaseChips row={row} />
      </td>
      <td className="py-3 pr-4 max-md:hidden">
        <Gaps row={row} />
      </td>
      <td className="py-3 pr-2 text-right tabular-nums">
        {game.cloudPlaythroughs}
      </td>
    </tr>
  );
}

/**
 * Catalog management: every game with what it still lacks, beside the
 * selected game's details and releases. The selection lives in the URL; below
 * the `xl` breakpoint the game replaces the table.
 */
export default function ManagePage({
  access,
  selectedId,
  onSelect,
  onClose,
  onAccount,
  onChanged,
  roms,
}: {
  access: ManageAccess;
  selectedId?: string;
  onSelect: (gameId: string) => void;
  onClose: () => void;
  onAccount: () => void;
  /** The public catalog changed. */
  onChanged: () => void;
  roms: RomSource;
}) {
  const { catalog, error, reload } = useAdminCatalog(access === "admin");
  const rows = useMemo(() => (catalog ? manageRows(catalog) : []), [catalog]);
  const counts = useMemo(() => countFindings(rows), [rows]);
  const franchises = useMemo(
    () =>
      [...new Set(rows.flatMap((row) => row.game.franchise ?? []))].sort(
        (a, b) => a.localeCompare(b),
      ),
    [rows],
  );
  const [query, setQuery] = useState("");
  const [finding, setFinding] = useState<Finding | null>(null);
  const visible = useMemo(
    () => filterRows(rows, query, finding),
    [rows, query, finding],
  );
  const [tab, setTab] = useState<PanelTab>("details");
  const [adding, setAdding] = useState(false);
  const selected = rows.find((row) => row.game.id === selectedId);
  const heading = useRef<HTMLHeadingElement>(null);

  useLayoutEffect(() => {
    heading.current?.focus({ preventScroll: true });
  }, [catalog === null]);

  const commit: Commit = async (write) => {
    try {
      await write();
    } finally {
      onChanged();
      await reload();
    }
  };

  if (access === "signed-out")
    return (
      <Notice title="Sign in to manage the catalog">
        <p className="text-muted">Managing needs an admin account.</p>
        <button className={button("primary")} onClick={onAccount}>
          Account
        </button>
      </Notice>
    );
  if (access === "checking") return <Notice title="Checking your account…" />;
  if (access === "denied")
    return (
      <Notice title="This account cannot manage the catalog">
        <p className="text-muted">
          An operator grants admin rights to an account.
        </p>
      </Notice>
    );
  if (!catalog)
    return (
      <Notice title={error ? "The catalog did not load" : "Loading catalog…"}>
        {error && (
          <>
            <p className="text-muted">{error}</p>
            <button className={button("outline")} onClick={() => void reload()}>
              Try again
            </button>
          </>
        )}
      </Notice>
    );

  const open = selectedId !== undefined;
  return (
    <main
      data-ui
      aria-label="Manage catalog"
      className={cx(
        "grid min-h-0 flex-1 grid-cols-1 bg-canvas font-sans text-2x text-content",
        open && "xl:grid-cols-[minmax(0,1fr)_600px]",
      )}
    >
      <section
        aria-label="Games"
        className={cx("min-h-0 md:overflow-y-auto", open && "max-xl:hidden")}
      >
        <div className="space-y-5 p-4 sm:p-6 lg:p-8">
          <header>
            <h1
              ref={heading}
              tabIndex={-1}
              className="text-4x font-bold text-heading outline-none"
            >
              Manage catalog
            </h1>
            <p className="mt-2 text-1x5 text-muted">
              {catalog.games.length} games · {catalog.versions.length} releases
              · {catalog.bases.length} base ROMs
            </p>
          </header>
          <div className="flex flex-wrap gap-3">
            <div className="relative min-w-60 flex-1">
              <Icon
                name="search"
                className="pointer-events-none absolute top-3 left-4 size-6 text-muted"
              />
              <input
                type="search"
                value={query}
                onChange={(event) => setQuery(event.target.value)}
                placeholder="Search games…"
                aria-label="Search games"
                className="h-12 w-full rounded-control border border-line bg-canvas pr-3 pl-13 text-2x placeholder:text-muted focus:border-mint focus:outline-none"
              />
            </div>
            <button
              className={button("primary")}
              onClick={() => setAdding(true)}
            >
              <Icon name="plus" className="size-6" />
              Add game
            </button>
          </div>
          <div
            role="group"
            aria-label="Show only"
            className="flex flex-wrap gap-2"
          >
            {findingKeys
              .filter((key) => counts[key] > 0 || key === finding)
              .map((key) => (
                <button
                  key={key}
                  aria-pressed={key === finding}
                  onClick={() => setFinding(key === finding ? null : key)}
                  className={cx(
                    chip,
                    "h-10 cursor-pointer gap-3 px-3",
                    key === finding
                      ? "border-mint bg-selected text-mint"
                      : "border-line text-content hover:bg-hover",
                    focusRing,
                  )}
                >
                  {findings[key].label}
                  <span className={key === finding ? "" : "text-muted"}>
                    {counts[key]}
                  </span>
                </button>
              ))}
          </div>
          {error && (
            <p role="alert" className="text-1x5 text-danger">
              Refreshing failed: {error}
            </p>
          )}
          {visible.length ? (
            <table className="w-full border-collapse text-left">
              <caption className="sr-only">
                {visible.length === rows.length
                  ? "Games"
                  : `${visible.length} of ${rows.length} games`}
              </caption>
              <thead className="text-1x5 text-muted">
                <tr className="border-b border-line">
                  <th scope="col" className="py-3 pr-4 pl-2 font-normal">
                    Game
                  </th>
                  <th
                    scope="col"
                    className="py-3 pr-4 font-normal max-md:hidden"
                  >
                    Releases
                  </th>
                  <th
                    scope="col"
                    className="py-3 pr-4 font-normal max-md:hidden"
                  >
                    Missing
                  </th>
                  <th scope="col" className="py-3 pr-2 text-right font-normal">
                    Cloud plays
                  </th>
                </tr>
              </thead>
              <tbody>
                {visible.map((row) => (
                  <GameRow
                    key={row.game.id}
                    row={row}
                    selected={row.game.id === selectedId}
                    onSelect={() => onSelect(row.game.id)}
                  />
                ))}
              </tbody>
            </table>
          ) : (
            <div className="space-y-3 rounded-panel border border-dashed border-line p-6 text-center">
              <p>No games match.</p>
              <button
                className={button("outline")}
                onClick={() => {
                  setQuery("");
                  setFinding(null);
                }}
              >
                Show all games
              </button>
            </div>
          )}
        </div>
      </section>
      {open && (
        <aside
          aria-label="Game"
          className="min-h-0 border-line md:overflow-y-auto xl:border-l"
        >
          {selected ? (
            <GamePanel
              key={selected.game.id}
              row={selected}
              tab={tab}
              onTabChange={setTab}
              bases={catalog.bases}
              versions={catalog.versions}
              franchises={franchises}
              roms={roms}
              commit={commit}
              onClose={onClose}
              onDeleted={onClose}
            />
          ) : (
            <div className="space-y-4 p-6">
              <p>No game has the ID {selectedId}.</p>
              <button className={button("outline")} onClick={onClose}>
                Back to all games
              </button>
            </div>
          )}
        </aside>
      )}
      {adding && (
        <AddGameDialog
          catalog={catalog}
          franchises={franchises}
          commit={commit}
          onClose={() => setAdding(false)}
          onCreated={(id) => {
            setAdding(false);
            setTab("releases");
            onSelect(id);
          }}
        />
      )}
    </main>
  );
}
