import { useId, type Ref } from "react";
import { Combobox } from "@base-ui/react/combobox";
import { Icon, PixelIcon } from "../../ui/icons";
import { FilterSelect } from "../../ui/select";
import { button, chip, cx, focusRing, popup, popupItem } from "../../ui/styles";
import { Cover } from "./cover";
import type {
  LibraryFilters,
  LibraryGame,
  ShowFilter,
  SortOrder,
} from "./model";

const showOptions = [
  { value: "all", label: "All" },
  { value: "installed", label: "Installed" },
  { value: "ready", label: "Ready" },
];
const sortOptions = [
  { value: "updated", label: "Recently updated" },
  { value: "played", label: "Played" },
  { value: "popularity", label: "Popularity" },
];

function TagFilter({
  tags,
  value,
  onChange,
  disabled,
}: {
  tags: readonly string[];
  value: readonly string[];
  onChange: (tags: string[]) => void;
  disabled: boolean;
}) {
  const id = useId();
  return (
    <Combobox.Root
      items={tags}
      multiple
      autoHighlight
      value={[...value]}
      onValueChange={(next) => onChange(next)}
      disabled={disabled}
    >
      <div className="flex min-h-14 items-center gap-4 rounded-control border border-mint/40 py-2 pr-2 pl-4 focus-within:border-mint">
        <label htmlFor={id} className="shrink-0 text-muted">
          Tags
        </label>
        <Combobox.InputGroup className="flex min-w-0 flex-1 cursor-text flex-wrap items-center gap-1.5">
          <Combobox.Chips className="contents">
            {value.map((tag) => (
              <Combobox.Chip
                key={tag}
                aria-label={tag}
                className={cx(
                  chip,
                  "gap-1 border-mint/60 bg-selected pr-1 outline-none data-highlighted:border-mint",
                )}
              >
                {tag}
                <Combobox.ChipRemove
                  aria-label={`Remove ${tag}`}
                  className="grid size-5 cursor-pointer place-items-center rounded-control hover:bg-hover"
                >
                  <Icon name="x" className="size-4" />
                </Combobox.ChipRemove>
              </Combobox.Chip>
            ))}
            <Combobox.Input
              id={id}
              placeholder={value.length ? "" : "Find tags…"}
              className="h-9 min-w-24 flex-1 bg-transparent text-2x outline-none placeholder:text-muted"
            />
          </Combobox.Chips>
        </Combobox.InputGroup>
      </div>
      <Combobox.Portal>
        <Combobox.Positioner sideOffset={4} className="z-50 outline-none">
          <Combobox.Popup
            data-ui
            className={cx(
              popup,
              "max-h-[min(var(--available-height),320px)] w-(--anchor-width) overflow-y-auto font-sans",
            )}
          >
            <Combobox.Empty className="px-4 py-2 text-muted empty:hidden">
              No matching tags.
            </Combobox.Empty>
            <Combobox.List>
              {(tag: string) => (
                <Combobox.Item key={tag} value={tag} className={popupItem}>
                  <span className="flex-1">{tag}</span>
                  <Combobox.ItemIndicator>
                    <Icon name="check" className="size-5" />
                  </Combobox.ItemIndicator>
                </Combobox.Item>
              )}
            </Combobox.List>
          </Combobox.Popup>
        </Combobox.Positioner>
      </Combobox.Portal>
    </Combobox.Root>
  );
}

/** The first tags and a count of the rest: two when the row has room, else one. */
function Tags({ tags }: { tags: readonly string[] }) {
  const more = (shown: number) => tags.length - shown;
  return (
    <span className="mt-2 flex min-w-0 gap-2 overflow-hidden">
      {tags.slice(0, 2).map((tag, index) => (
        <span
          key={tag}
          className={cx(
            chip,
            "border-mint/50",
            index > 0 && "@max-[380px]:hidden",
          )}
        >
          {tag}
        </span>
      ))}
      {more(1) > 0 && (
        <span
          className={cx(chip, "border-line text-muted @min-[380px]:hidden")}
        >
          +{more(1)}
        </span>
      )}
      {more(2) > 0 && (
        <span
          className={cx(chip, "border-line text-muted @max-[380px]:hidden")}
        >
          +{more(2)}
        </span>
      )}
    </span>
  );
}

export function GameList({
  games,
  total,
  filters,
  onFiltersChange,
  franchises,
  tags,
  selectedId,
  selectedRef,
  disabled,
  showExperimental,
  onShowExperimentalChange,
  onSelect,
  onAddGame,
  onPlay,
  onInstall,
  onAddFile,
}: {
  games: readonly LibraryGame[];
  total: number;
  filters: LibraryFilters;
  onFiltersChange: (filters: LibraryFilters) => void;
  franchises: readonly string[];
  tags: readonly string[];
  selectedId?: string;
  selectedRef: Ref<HTMLButtonElement>;
  disabled: boolean;
  showExperimental: boolean;
  onShowExperimentalChange: (show: boolean) => void;
  onSelect: (game: LibraryGame) => void;
  onAddGame: () => void;
  onPlay: (game: LibraryGame) => void;
  onInstall: (game: LibraryGame) => void;
  onAddFile: (game: LibraryGame) => void;
}) {
  const set = (change: Partial<LibraryFilters>) =>
    onFiltersChange({ ...filters, ...change });
  return (
    <>
      <div className="space-y-4 px-3 pt-4 pb-2 sm:px-6 sm:pt-6">
        <div className="@container flex gap-3">
          <label className="relative min-w-0 flex-1">
            <Icon
              name="search"
              className="pointer-events-none absolute top-1/2 left-4 size-6 -translate-y-1/2 text-muted"
            />
            <input
              type="search"
              aria-label="Search games"
              placeholder="Search games…"
              value={filters.query}
              disabled={disabled}
              onChange={(event) => set({ query: event.target.value })}
              className="h-12 w-full rounded-control border border-line bg-canvas pr-3 pl-13 text-2x placeholder:text-muted focus:border-mint focus:outline-none disabled:text-muted"
            />
          </label>
          <button
            className={button("primary", "lg")}
            aria-label="Add game"
            disabled={disabled}
            onClick={onAddGame}
          >
            <Icon name="plus" className="size-6" />
            <span className="@max-[400px]:hidden">Add game</span>
          </button>
        </div>
        <div className="@container flex gap-2 *:flex-auto sm:gap-3">
          <FilterSelect
            label="Show"
            value={filters.show}
            options={showOptions}
            disabled={disabled}
            onChange={(show) => set({ show: show as ShowFilter })}
          />
          <FilterSelect
            label="Franchise"
            value={filters.franchise || "all"}
            options={[
              { value: "all", label: "All" },
              ...franchises.map((name) => ({ value: name, label: name })),
            ]}
            disabled={disabled}
            onChange={(franchise) =>
              set({ franchise: franchise === "all" ? "" : franchise })
            }
          />
          <FilterSelect
            label="Sort"
            value={filters.sort}
            options={sortOptions}
            disabled={disabled}
            onChange={(sort) => set({ sort: sort as SortOrder })}
          />
        </div>
        <TagFilter
          tags={tags}
          value={filters.tags}
          disabled={disabled}
          onChange={(next) => set({ tags: next })}
        />
        <details className="rounded-control border border-line bg-surface px-4 py-2 text-1x5">
          <summary className="cursor-pointer text-mint focus-visible:outline-2 focus-visible:outline-mint">
            Library settings
          </summary>
          <label className="mt-3 flex cursor-pointer items-start gap-3 pb-1">
            <input
              type="checkbox"
              className="mt-1 size-5 accent-mint"
              checked={showExperimental}
              disabled={disabled}
              onChange={(event) =>
                onShowExperimentalChange(event.target.checked)
              }
            />
            <span>
              Show experimental romhacks
              <span className="mt-1 block text-muted">
                These patches work, but their release details need review.
              </span>
            </span>
          </label>
        </details>
      </div>
      {games.length ? (
        <ul aria-label="Games" className="px-3 pb-6 sm:px-6">
          {games.map((game) => {
            const selected = game.id === selectedId;
            return (
              <li
                key={game.id}
                className="border-b border-line/70 py-2 last:border-b-0"
              >
                <div
                  className={cx(
                    "flex items-center gap-1 rounded-panel border-2 p-2 sm:gap-3",
                    selected
                      ? "border-peach bg-surface"
                      : "border-transparent hover:bg-surface/60",
                  )}
                >
                  <button
                    ref={selected ? selectedRef : undefined}
                    aria-label={`View ${game.title}`}
                    aria-current={selected || undefined}
                    disabled={disabled}
                    onClick={() => onSelect(game)}
                    className={cx(
                      "flex min-w-0 flex-1 cursor-pointer items-center gap-3 rounded-control text-left disabled:cursor-not-allowed sm:gap-4",
                      focusRing,
                    )}
                  >
                    <Cover
                      id={game.id}
                      title={game.title}
                      imageUrl={game.imageUrl}
                      size="sm"
                    />
                    <span className="@container min-w-0 flex-1">
                      <span className="line-clamp-2 font-bold">
                        {game.title}
                      </span>
                      {game.visibility === "experimental" && (
                        <span className="mt-1 inline-block text-1x5 text-warning">
                          Experimental
                        </span>
                      )}
                      <Tags tags={game.tags} />
                    </span>
                  </button>
                  {game.availability === "installed" ? (
                    <span
                      className="grid size-11 shrink-0 place-items-center text-mint"
                      title="On this device"
                    >
                      <Icon name="check" className="size-8" />
                      <span className="sr-only">On this device</span>
                    </span>
                  ) : game.availability === "ready" ? (
                    <button
                      className={button("ghost", "icon")}
                      aria-label={`Install ${game.title}`}
                      title="Ready: install"
                      disabled={disabled}
                      onClick={() => onInstall(game)}
                    >
                      <Icon name="download" className="size-7" />
                    </button>
                  ) : (
                    <button
                      className={button("ghost", "icon")}
                      aria-label={`Add ${game.missingFile} for ${game.title}`}
                      title={`Add ${game.missingFile}`}
                      disabled={disabled}
                      onClick={() => onAddFile(game)}
                    >
                      <Icon name="filePlus" className="size-7" />
                    </button>
                  )}
                  {game.availability === "needs-file" ? (
                    <span className="size-11 shrink-0" />
                  ) : (
                    <button
                      className={button("ghost", "icon")}
                      aria-label={`Play ${game.title}`}
                      title="Play"
                      disabled={disabled}
                      onClick={() => onPlay(game)}
                    >
                      <PixelIcon name="play" />
                    </button>
                  )}
                </div>
              </li>
            );
          })}
        </ul>
      ) : (
        <div className="px-6 py-12 text-center">
          <h2 className="text-3x font-bold text-heading">
            {total ? "No matching games" : "Your library is empty"}
          </h2>
          <p className="mt-2 text-muted">
            {total
              ? "Try another search, filter or tag."
              : "Add a .gba game or a single-game ZIP to begin."}
          </p>
          {total > 0 && (
            <button
              className={cx(button("outline"), "mt-5")}
              onClick={() =>
                onFiltersChange({
                  ...filters,
                  query: "",
                  show: "all",
                  franchise: "",
                  tags: [],
                })
              }
            >
              Clear filters
            </button>
          )}
        </div>
      )}
    </>
  );
}
