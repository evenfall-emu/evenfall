import { useLayoutEffect, useRef, useState } from "react";
import { StatusTip } from "./info-tip";
import "./library-page.css";

/** Display metadata only. ROM bytes and session ownership stay with the host. */
export type LibraryEntry = {
  id: string;
  title: string;
  description?: string;
  logo?: string;
  lastPlayed?: number;
  romSize?: number;
  version?: string;
  authorUrl?: string;
  playthroughs?: readonly {
    id: string;
    name: string;
    localSavedAt?: number;
    cloudUploadedAt?: number;
    onPlay: () => void | Promise<void>;
  }[];
  /** Omit until a service supplies genuine new-playthrough activity. */
  newRunsPastWeek?: number;
} & (
  | {
      status: "installed";
      resume?: boolean;
      onPlay: () => void | Promise<void>;
    }
  | {
      status: "cloud";
      onPlay: () => void | Promise<void>;
    }
  | {
      status: "ready";
      requiredBase: string;
      onPlay: () => void | Promise<void>;
    }
  | {
      status: "missing-base";
      requiredBase: string;
      onImportBase: (file: File) => void | Promise<void>;
    }
);

const statusText = {
  installed: "Installed",
  cloud: "Cloud checkpoint · ROM needed",
  ready: "Base ROM ready",
  "missing-base": "Base ROM required",
};

function LibraryIcon({
  name,
}: {
  name:
    | LibraryEntry["status"]
    | "search"
    | "chevron"
    | "back"
    | "plus"
    | "people";
}) {
  return (
    <svg
      viewBox="0 0 24 24"
      fill="none"
      stroke="currentColor"
      strokeWidth="1.6"
      strokeLinecap="square"
      aria-hidden="true"
    >
      {name === "installed" && (
        <>
          <rect x="4" y="4" width="16" height="16" rx="1" />
          <path d="m8 12 3 3 6-6" />
        </>
      )}
      {name === "search" && (
        <>
          <circle cx="10" cy="10" r="6" />
          <path d="m15 15 6 6" />
        </>
      )}
      {name === "chevron" && <path d="m9 5 7 7-7 7" />}
      {name === "back" && <path d="m15 5-7 7 7 7" />}
      {name === "plus" && <path d="M12 4v16M4 12h16" />}
      {name === "ready" && <path d="M12 2v13m-5-5 5 5 5-5M3 15v6h18v-6" />}
      {name === "cloud" && (
        <path d="M20.88 18.09A5 5 0 0 0 18 9h-1.26A8 8 0 1 0 4 16.25m8-3.25v8m-4-4 4-4 4 4" />
      )}
      {name === "missing-base" && (
        <path d="M13 21H4V3h10l5 5v4M14 3v6h5m0 6v8m-4-4h8" />
      )}
      {name === "people" && (
        <>
          <circle cx="9" cy="7" r="3" />
          <path d="M3 22v-5a6 6 0 0 1 12 0v5M17 4a3 3 0 0 1 0 6m1 3a5 5 0 0 1 4 5v4" />
        </>
      )}
    </svg>
  );
}

function Artwork({ entry }: { entry: LibraryEntry }) {
  const [failed, setFailed] = useState(false);
  const words = entry.title
    .trim()
    .split(/[^\p{L}\p{N}]+/u)
    .filter(Boolean);
  const initials =
    words
      .slice(0, 2)
      .map((word) => Array.from(word)[0])
      .join("")
      .toUpperCase() || "?";
  return (
    <span className="library-artwork" aria-hidden="true">
      {entry.logo && !failed ? (
        <img src={entry.logo} alt="" onError={() => setFailed(true)} />
      ) : (
        initials
      )}
    </span>
  );
}

export function LibraryPage({
  entries,
  onAddGame,
  busy = false,
}: {
  entries: readonly LibraryEntry[];
  onAddGame: () => void;
  busy?: boolean;
}) {
  const [query, setQuery] = useState("");
  const [filter, setFilter] = useState("all");
  const [sort, setSort] = useState("recent");
  const [selectedId, setSelectedId] = useState<string>();
  const [detailOpen, setDetailOpen] = useState(false);
  const listPane = useRef<HTMLDivElement>(null);
  const pageHeading = useRef<HTMLHeadingElement>(null);
  const detailHeading = useRef<HTMLHeadingElement>(null);
  const selectedButton = useRef<HTMLButtonElement>(null);
  const listScroll = useRef(0);
  const baseInput = useRef<HTMLInputElement>(null);
  const [working, setWorking] = useState(false);
  const pending = useRef(false);
  const [error, setError] = useState("");
  const disabled = busy || working;
  const visible = entries
    .filter(
      (entry) =>
        (filter === "all" ||
          (filter === "installed"
            ? entry.status === "installed"
            : entry.status !== "installed")) &&
        entry.title
          .toLocaleLowerCase()
          .includes(query.trim().toLocaleLowerCase()),
    )
    .sort(
      (a, b) =>
        (sort === "recent" ? (b.lastPlayed ?? 0) - (a.lastPlayed ?? 0) : 0) ||
        a.title.localeCompare(b.title) ||
        a.id.localeCompare(b.id),
    );
  const selected =
    visible.find((entry) => entry.id === selectedId) ?? visible[0];

  useLayoutEffect(() => {
    pageHeading.current?.focus();
  }, []);
  useLayoutEffect(() => {
    if (!window.matchMedia("(max-width: 800px)").matches) return;
    if (detailOpen) detailHeading.current?.focus({ preventScroll: true });
    else if (listPane.current) {
      listPane.current.scrollTop = listScroll.current;
      selectedButton.current?.focus({ preventScroll: true });
    }
  }, [detailOpen]);

  async function run(action: () => void | Promise<void>) {
    if (disabled || pending.current) return;
    pending.current = true;
    setWorking(true);
    setError("");
    try {
      await action();
    } catch (error) {
      setError(error instanceof Error ? error.message : String(error));
    } finally {
      pending.current = false;
      setWorking(false);
    }
  }

  function playButton(entry: LibraryEntry) {
    const label =
      entry.status === "missing-base"
        ? "Add base ROM"
        : entry.status === "ready"
          ? "Install & play"
          : entry.status === "cloud"
            ? "Resume from cloud"
            : entry.resume
              ? "Continue"
              : "Play";
    return (
      <button
        className="library-primary"
        disabled={disabled}
        aria-label={`${label} ${entry.title}`}
        onClick={() => {
          if (entry.status === "missing-base") {
            setSelectedId(entry.id);
            setDetailOpen(true);
            setError("");
          } else void run(entry.onPlay);
        }}
      >
        {working ? "Opening…" : label}
        {entry.status === "installed" && entry.resume && (
          <span aria-hidden="true">▶</span>
        )}
      </button>
    );
  }

  return (
    <div
      className={`library-page ${detailOpen ? "library-show-detail" : ""}`}
      onKeyDown={(event) => {
        if (
          event.key === "Escape" &&
          detailOpen &&
          !disabled &&
          window.matchMedia("(max-width: 800px)").matches
        ) {
          event.preventDefault();
          setDetailOpen(false);
        }
      }}
    >
      <main className="library-main" aria-label="Library">
        <button
          className="library-back"
          aria-label="Back to library"
          disabled={disabled}
          onClick={() => setDetailOpen(false)}
        >
          <LibraryIcon name="back" />
          Library
        </button>
        <div className="library-controls">
          <h1 ref={pageHeading} tabIndex={-1}>
            Library
          </h1>
          <label className="library-search">
            <LibraryIcon name="search" />
            <input
              type="search"
              aria-label="Search games"
              placeholder="Search games…"
              value={query}
              disabled={disabled}
              onChange={(event) => {
                setQuery(event.target.value);
                setError("");
              }}
            />
          </label>
          <label className="library-filter">
            <span>Show:</span>
            <select
              aria-label="Show games"
              value={filter}
              disabled={disabled}
              onChange={(event) => {
                setFilter(event.target.value);
                setError("");
              }}
            >
              <option value="all">All games</option>
              <option value="installed">Installed</option>
              <option value="not-installed">Not installed</option>
            </select>
          </label>
          <label className="library-sort">
            <span>Sort:</span>
            <select
              aria-label="Sort games"
              value={sort}
              disabled={disabled}
              onChange={(event) => setSort(event.target.value)}
            >
              <option value="recent">Recently played</option>
              <option value="title">Title</option>
            </select>
          </label>
        </div>
        {error && (
          <p className="library-error" role="alert">
            {error}
          </p>
        )}
        <div className="library-content">
          <div className="library-list-pane" ref={listPane}>
            <ul className="library-list" aria-label="Games">
              {visible.map((entry) => (
                <li
                  key={entry.id}
                  className={entry.id === selected?.id ? "selected" : ""}
                >
                  <button
                    className="library-pick"
                    aria-label={`View ${entry.title}`}
                    aria-current={
                      entry.id === selected?.id ? "true" : undefined
                    }
                    disabled={disabled}
                    onClick={(event) => {
                      selectedButton.current = event.currentTarget;
                      listScroll.current = listPane.current?.scrollTop ?? 0;
                      setSelectedId(entry.id);
                      setError("");
                      setDetailOpen(true);
                    }}
                  >
                    <Artwork key={`${entry.id}:${entry.logo}`} entry={entry} />
                    <span className="library-row-title">
                      <h3>{entry.title}</h3>
                      {entry.lastPlayed && (
                        <small>
                          Played{" "}
                          {new Date(entry.lastPlayed).toLocaleDateString(
                            undefined,
                            { month: "short", day: "numeric" },
                          )}
                        </small>
                      )}
                    </span>
                    <span className="library-mobile-chevron">
                      <LibraryIcon name="chevron" />
                    </span>
                  </button>
                  <StatusTip
                    label={statusText[entry.status]}
                    className={`library-status-${entry.status}`}
                  >
                    <LibraryIcon name={entry.status} />
                  </StatusTip>
                  <div className="library-row-action">{playButton(entry)}</div>
                </li>
              ))}
            </ul>
            {!visible.length && (
              <div className="library-empty">
                <h2>
                  {entries.length
                    ? "No matching games"
                    : "Your next adventure starts here."}
                </h2>
                <p>
                  {entries.length
                    ? "Try another search or filter."
                    : "Add a .gba game or a single-game ZIP to begin."}
                </p>
                {entries.length ? (
                  <button
                    onClick={() => {
                      setQuery("");
                      setFilter("all");
                    }}
                  >
                    Clear filters
                  </button>
                ) : (
                  <button
                    className="library-primary"
                    disabled={disabled}
                    onClick={onAddGame}
                  >
                    Add game
                  </button>
                )}
              </div>
            )}
          </div>
          <section className="library-detail" aria-label="Game details">
            {selected ? (
              <>
                <div className="library-detail-body" key={selected.id}>
                  <header className="library-detail-title">
                    <Artwork
                      key={`${selected.id}:${selected.logo}`}
                      entry={selected}
                    />
                    <div>
                      <h2 ref={detailHeading} tabIndex={-1}>
                        {selected.title}
                      </h2>
                      <p className={`library-status-${selected.status}`}>
                        <LibraryIcon name={selected.status} />
                        {statusText[selected.status]}
                      </p>
                    </div>
                  </header>
                  <p className="library-description">
                    {selected.description ??
                      "No description is available for this game."}
                  </p>
                  {selected.playthroughs &&
                    selected.playthroughs.length > 0 && (
                      <section
                        className="library-playthroughs"
                        aria-label="Playthroughs"
                      >
                        <h3>Playthroughs</h3>
                        {selected.playthroughs.map((playthrough) => (
                          <button
                            key={playthrough.id}
                            disabled={disabled}
                            onClick={() => void run(playthrough.onPlay)}
                          >
                            <strong>{playthrough.name}</strong>
                            <span>
                              {playthrough.localSavedAt
                                ? `On this device · ${new Date(playthrough.localSavedAt).toLocaleString()}`
                                : playthrough.cloudUploadedAt
                                  ? "Cloud only"
                                  : "No checkpoint yet"}
                              {playthrough.cloudUploadedAt
                                ? ` · Cloud · ${new Date(playthrough.cloudUploadedAt).toLocaleString()}`
                                : ""}
                            </span>
                          </button>
                        ))}
                      </section>
                    )}
                  {selected.newRunsPastWeek !== undefined && (
                    <p className="library-activity">
                      <LibraryIcon name="people" />
                      {selected.newRunsPastWeek} new runs started · past 7 days
                    </p>
                  )}
                  {(selected.status === "ready" ||
                    selected.status === "missing-base") && (
                    <div className="library-base">
                      <p>
                        <span>Required base</span>
                        <strong>{selected.requiredBase}</strong>
                      </p>
                      {selected.status === "missing-base" && (
                        <div
                          className="library-base-drop"
                          onDragOver={(event) => {
                            event.preventDefault();
                            event.stopPropagation();
                          }}
                          onDragEnter={(event) => {
                            event.preventDefault();
                            event.stopPropagation();
                          }}
                          onDrop={(event) => {
                            event.preventDefault();
                            event.stopPropagation();
                            const file = event.dataTransfer.files[0];
                            if (file)
                              void run(() => selected.onImportBase(file));
                          }}
                        >
                          <LibraryIcon name="missing-base" />
                          <span>Drop .gba or .zip</span>
                          <button
                            className="library-primary"
                            disabled={disabled}
                            onClick={() => baseInput.current?.click()}
                          >
                            Choose file
                          </button>
                        </div>
                      )}
                    </div>
                  )}
                  <details className="library-release">
                    <summary>
                      {selected.status === "installed"
                        ? "Game details"
                        : "Release details"}
                    </summary>
                    <dl>
                      {selected.version ? (
                        <>
                          <dt>Version</dt>
                          <dd>{selected.version}</dd>
                        </>
                      ) : (
                        selected.status !== "installed" && (
                          <dd>No release metadata supplied.</dd>
                        )
                      )}
                      {selected.lastPlayed ? (
                        <>
                          <dt>Last played</dt>
                          <dd>
                            {new Date(selected.lastPlayed).toLocaleString()}
                          </dd>
                        </>
                      ) : null}
                      {selected.romSize !== undefined && (
                        <>
                          <dt>File size</dt>
                          <dd>
                            {selected.romSize < 1048576
                              ? `${(selected.romSize / 1024).toFixed(1)} KiB`
                              : `${(selected.romSize / 1048576).toFixed(1)} MiB`}
                          </dd>
                        </>
                      )}
                    </dl>
                  </details>
                  {selected.authorUrl &&
                    /^https?:\/\//i.test(selected.authorUrl) && (
                      <a
                        className="library-author"
                        href={selected.authorUrl}
                        target="_blank"
                        rel="noopener noreferrer"
                      >
                        ↗ Author website
                      </a>
                    )}
                </div>
                <div
                  className={`library-detail-action ${selected.status === "missing-base" ? "library-base-action" : ""}`}
                >
                  {selected.status === "missing-base" ? (
                    <button
                      className="library-primary"
                      disabled={disabled}
                      onClick={() => baseInput.current?.click()}
                    >
                      {working ? "Opening…" : "Choose base ROM"}
                    </button>
                  ) : (
                    playButton(selected)
                  )}
                </div>
                {selected.status === "missing-base" && (
                  <input
                    ref={baseInput}
                    type="file"
                    className="file-input"
                    aria-label="Base ROM file"
                    accept=".gba,.zip"
                    disabled={disabled}
                    tabIndex={-1}
                    onChange={(event) => {
                      const file = event.target.files?.[0];
                      event.target.value = "";
                      if (file) void run(() => selected.onImportBase(file));
                    }}
                  />
                )}
              </>
            ) : (
              <p className="library-detail-placeholder">
                Choose a game to see its details.
              </p>
            )}
          </section>
        </div>
      </main>
    </div>
  );
}
