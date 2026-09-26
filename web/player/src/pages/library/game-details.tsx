import type { ReactNode, Ref } from "react";
import { Menu } from "@base-ui/react/menu";
import { Icon, PixelIcon, type IconName } from "../../ui/icons";
import { button, chip, cx, popup, popupItem } from "../../ui/styles";
import { Cover } from "./cover";
import { formatAgo } from "./format";
import type { LibraryGame, PlaythroughEntry, SyncState } from "./model";

const syncStatus: Record<
  SyncState,
  { icon: IconName; text: string; className?: string }
> = {
  synced: { icon: "cloud", text: "Cloud synced" },
  unsynced: {
    icon: "alert",
    text: "Local changes not backed up",
    className: "text-warning",
  },
  "cloud-newer": {
    icon: "cloud",
    text: "Newer progress in the cloud",
    className: "text-info",
  },
  "device-only": { icon: "monitor", text: "On this device only" },
  "cloud-only": { icon: "cloud", text: "Cloud backup · Not downloaded here" },
};

function Playthrough({
  run,
  now,
  disabled,
  onPlay,
  onRename,
  onExport,
  onDelete,
}: {
  run: PlaythroughEntry;
  now: number;
  disabled: boolean;
  onPlay: () => void;
  onRename: () => void;
  onExport: () => void;
  onDelete: () => void;
}) {
  const status = syncStatus[run.sync];
  return (
    <li className="grid grid-cols-[minmax(0,1fr)_auto_auto] items-center gap-x-3 gap-y-2 rounded-panel border border-line bg-surface px-3 py-3 sm:px-4 2xl:grid-cols-[minmax(0,5fr)_minmax(0,6fr)_auto_auto]">
      <div className="min-w-0">
        <p className="truncate font-bold">{run.name}</p>
        <p className="text-1x5 text-muted">
          {run.release.label && `${run.release.label} · `}Last played{" "}
          {formatAgo(run.lastPlayed, now)}
        </p>
      </div>
      <p className="flex items-center gap-3 text-1x5 max-2xl:col-start-1 max-2xl:row-start-2">
        <Icon
          name={status.icon}
          className={cx("size-6 shrink-0", status.className)}
        />
        {status.text}
      </p>
      <button
        className={cx(
          button("primary", "lg"),
          "max-2xl:row-span-2 max-sm:h-11 max-sm:px-3",
        )}
        aria-label={`Play ${run.name}`}
        disabled={disabled}
        onClick={onPlay}
      >
        <PixelIcon name="playSmall" />
        Play
      </button>
      {/* A cloud-only run has nothing on this device to manage yet. */}
      {run.sync === "cloud-only" ? (
        <span className="size-11 max-2xl:row-span-2 max-sm:size-10" />
      ) : (
        <Menu.Root>
          <Menu.Trigger
            className={cx(
              button("outline", "icon"),
              "max-2xl:row-span-2 max-sm:size-10",
            )}
            aria-label={`More options for ${run.name}`}
            disabled={disabled}
          >
            <Icon name="dotsVertical" className="size-6" />
          </Menu.Trigger>
          <Menu.Portal>
            <Menu.Positioner align="end" sideOffset={4} className="z-50">
              <Menu.Popup data-ui className={cx(popup, "min-w-56 font-sans")}>
                <Menu.Item className={popupItem} onClick={onRename}>
                  Rename…
                </Menu.Item>
                <Menu.Item className={popupItem} onClick={onExport}>
                  Export in-game save
                </Menu.Item>
                <Menu.Item
                  className={cx(popupItem, "text-danger")}
                  onClick={onDelete}
                >
                  Delete…
                </Menu.Item>
              </Menu.Popup>
            </Menu.Positioner>
          </Menu.Portal>
        </Menu.Root>
      )}
    </li>
  );
}

/** One offer box: an icon, a line of text and its action. */
function Offer({
  icon,
  iconClass,
  children,
  action,
}: {
  icon: IconName;
  iconClass: string;
  children: ReactNode;
  action: ReactNode;
}) {
  return (
    <div className="flex items-center gap-4 rounded-panel border border-line bg-surface py-2 pr-2 pl-4">
      <Icon name={icon} className={cx("size-7 shrink-0", iconClass)} />
      <div className="min-w-0 flex-1">{children}</div>
      {action}
    </div>
  );
}

export function GameDetails({
  game,
  now,
  headingRef,
  disabled,
  onPlay,
  onInstall,
  onAddFile,
  onUpdate,
  onReleaseNotes,
  onNewPlaythrough,
  onRename,
  onExport,
  onDelete,
}: {
  game: LibraryGame;
  now: number;
  headingRef: Ref<HTMLHeadingElement>;
  disabled: boolean;
  onPlay: (run: PlaythroughEntry) => void;
  onInstall: () => void;
  onAddFile: () => void;
  onUpdate: () => void;
  onReleaseNotes: () => void;
  onNewPlaythrough: () => void;
  onRename: (run: PlaythroughEntry) => void;
  onExport: (run: PlaythroughEntry) => void;
  onDelete: (run: PlaythroughEntry) => void;
}) {
  const { latest } = game;
  return (
    <article className="space-y-5 p-4 sm:p-6 lg:p-8">
      <header className="grid grid-cols-[auto_minmax(0,1fr)] gap-x-4 gap-y-3 sm:gap-x-6">
        <div className="sm:row-span-3">
          <Cover
            id={game.id}
            title={game.title}
            imageUrl={game.imageUrl}
            size="lg"
          />
        </div>
        <div className="min-w-0">
          <div className="flex items-start gap-1">
            <h2
              ref={headingRef}
              tabIndex={-1}
              className="mr-1 min-w-0 flex-1 text-3x font-bold break-words text-heading outline-none sm:mr-2 sm:flex-initial sm:text-4x"
            >
              {game.title}
            </h2>
            {latest.sourceUrl && /^https?:\/\//i.test(latest.sourceUrl) && (
              <a
                href={latest.sourceUrl}
                target="_blank"
                rel="noopener noreferrer"
                className={cx(
                  button("ghost", "icon"),
                  "text-mint max-sm:size-9",
                )}
                aria-label="Project page"
                title="Project page"
              >
                <Icon name="externalLink" className="size-7" />
              </a>
            )}
            {latest.releaseNotes && (
              <button
                className={cx(button("ghost", "icon"), "max-sm:size-9")}
                aria-label={`Release notes for ${latest.label}`}
                title="Release notes"
                onClick={onReleaseNotes}
              >
                <Icon name="releaseNotes" className="size-7" />
              </button>
            )}
          </div>
          {game.summary && (
            <p className="mt-2 max-sm:text-1x5">{game.summary}</p>
          )}
        </div>
        {game.tags.length > 0 && (
          <ul
            aria-label={`${game.title} tags`}
            className="col-span-2 flex flex-wrap gap-2 sm:col-span-1 sm:col-start-2"
          >
            {game.tags.map((tag) => (
              <li key={tag} className={cx(chip, "h-8 border-mint/50 px-3")}>
                {tag}
              </li>
            ))}
          </ul>
        )}
      </header>

      {game.availability === "installed" ? (
        <p className="flex items-center gap-3">
          <Icon name="check" className="size-7 text-mint" />
          Available locally
        </p>
      ) : game.availability === "ready" ? (
        <Offer
          icon="download"
          iconClass="text-mint"
          action={
            <button
              className={button("primary")}
              disabled={disabled}
              onClick={onInstall}
            >
              Install
            </button>
          }
        >
          <p className="font-bold">Ready to play {latest.label}</p>
          <p className="text-1x5 text-muted">
            Your {latest.requiredBase?.displayName ?? "base ROM"} is on this
            device; the patch installs in one step.
          </p>
        </Offer>
      ) : (
        <Offer
          icon="filePlus"
          iconClass="text-warning"
          action={
            <button
              className={button("primary")}
              disabled={disabled}
              onClick={onAddFile}
            >
              Choose file…
            </button>
          }
        >
          <p className="font-bold">Needs {game.missingFile}</p>
          <p className="text-1x5 text-muted">
            Add your own copy
            {latest.label ? ` to install ${latest.label}` : " to play"}. It
            stays on this device.
          </p>
        </Offer>
      )}

      {game.updatable.length > 0 && (
        <Offer
          icon="download"
          iconClass="text-peach"
          action={
            <button
              className={button("accent")}
              disabled={disabled}
              onClick={onUpdate}
            >
              Update…
            </button>
          }
        >
          <p className="font-bold text-peach">{latest.label} available</p>
        </Offer>
      )}

      <section aria-labelledby={`${game.id}-playthroughs`} className="pt-3">
        <div className="mb-4 flex items-center justify-between gap-4">
          <h3
            id={`${game.id}-playthroughs`}
            className="text-2x font-bold text-heading xl:text-3x"
          >
            Your playthroughs
          </h3>
          {game.availability === "installed" && (
            <button
              className={button("primary")}
              disabled={disabled}
              onClick={onNewPlaythrough}
            >
              <Icon name="plus" className="size-6" />
              <span>
                New<span className="max-sm:sr-only"> playthrough</span>
              </span>
            </button>
          )}
        </div>
        {game.playthroughs.length ? (
          <ul className="space-y-2">
            {game.playthroughs.map((run) => (
              <Playthrough
                key={run.id}
                run={run}
                now={now}
                disabled={disabled}
                onPlay={() => onPlay(run)}
                onRename={() => onRename(run)}
                onExport={() => onExport(run)}
                onDelete={() => onDelete(run)}
              />
            ))}
          </ul>
        ) : (
          <p className="rounded-panel border border-dashed border-line px-4 py-6 text-center text-muted">
            {game.availability === "needs-file"
              ? `Add ${game.missingFile} to start playing.`
              : "No playthroughs yet. Start one to begin."}
          </p>
        )}
      </section>
    </article>
  );
}
