import { useId, useState } from "react";
import { Menu } from "@base-ui/react/menu";
import { Tabs } from "@base-ui/react/tabs";
import { Icon } from "../../ui/icons";
import { FilterSelect } from "../../ui/select";
import { button, cx, focusRing, popup, popupItem } from "../../ui/styles";
import type { CatalogBase, CatalogVersion } from "../library/catalog";
import { Cover } from "../library/cover";
import {
  deleteGame,
  deleteVersion,
  ManageError,
  patchGame,
  patchVersion,
} from "./api";
import { TextField } from "../../ui/fields";
import { ConfirmDialog, TagEditor } from "./fields";
import {
  gameChanges,
  gameDraft,
  patchSummary,
  visibilities,
  visibilityLabels,
  type GameDraft,
  type ManageRow,
  type Visibility,
} from "./model";
import {
  AddReleaseDialog,
  EditReleaseDialog,
  UploadPatchDialog,
  type Commit,
} from "./release-dialogs";
import type { RomSource } from "./verify";

export type PanelTab = "details" | "releases";

const message = (error: unknown) =>
  error instanceof Error ? error.message : String(error);

const tabClass = cx(
  "relative flex h-12 cursor-pointer items-center gap-2 px-4 text-content hover:text-mint-hover data-active:text-mint",
  "after:absolute after:inset-x-2 after:bottom-0 after:h-1 after:rounded-t-control data-active:after:bg-mint",
  focusRing,
  "focus-visible:-outline-offset-2",
);

function GameForm({
  row,
  franchises,
  commit,
  onDelete,
}: {
  row: ManageRow;
  franchises: readonly string[];
  commit: Commit;
  onDelete: () => void;
}) {
  const { game } = row;
  const [draft, setDraft] = useState(() => gameDraft(game));
  const [busy, setBusy] = useState(false);
  const [error, setError] = useState("");
  const [errors, setErrors] = useState<Readonly<Record<string, string>>>({});
  const franchiseList = useId();
  const changes = gameChanges(game, draft);
  const dirty = Object.keys(changes).length > 0;
  const set =
    <K extends keyof GameDraft>(key: K) =>
    (value: GameDraft[K]) =>
      setDraft({ ...draft, [key]: value });

  async function save() {
    setBusy(true);
    setError("");
    setErrors({});
    try {
      await commit(() => patchGame(game.id, changes));
    } catch (failure) {
      setError(message(failure));
      if (failure instanceof ManageError) setErrors(failure.fields);
    } finally {
      setBusy(false);
    }
  }

  return (
    <form
      className="flex min-h-full flex-col"
      onSubmit={(event) => {
        event.preventDefault();
        if (dirty && !busy) void save();
      }}
    >
      <div className="flex-1 space-y-5 p-4 sm:p-6">
        <TextField
          label="Display name"
          value={draft.displayName}
          onChange={set("displayName")}
          error={errors.displayName}
          maxLength={160}
          required
          disabled={busy}
        />
        <TextField
          label="Summary"
          rows={2}
          value={draft.summary}
          onChange={set("summary")}
          hint="One line under the title in the Library."
          error={errors.summary}
          maxLength={2000}
          disabled={busy}
        />
        <TextField
          label="Description"
          rows={6}
          value={draft.description}
          onChange={set("description")}
          error={errors.description}
          maxLength={2000}
          disabled={busy}
        />
        <div className="grid grid-cols-[minmax(0,1fr)_auto] items-start gap-4">
          <TextField
            label="Cover image URL"
            type="url"
            value={draft.imageUrl}
            onChange={set("imageUrl")}
            placeholder="https://"
            hint={
              draft.imageUrl.trim()
                ? "Square artwork, hosted elsewhere."
                : "No cover image: the Library shows the title's initials."
            }
            error={errors.imageUrl}
            disabled={busy}
          />
          <div className="pt-8">
            <Cover
              key={draft.imageUrl}
              id={game.id}
              title={draft.displayName || game.displayName}
              imageUrl={draft.imageUrl.trim() || null}
              size="md"
            />
          </div>
        </div>
        <TextField
          label="Franchise"
          value={draft.franchise}
          onChange={set("franchise")}
          list={franchiseList}
          error={errors.franchise}
          maxLength={80}
          disabled={busy}
        />
        <datalist id={franchiseList}>
          {franchises.map((franchise) => (
            <option key={franchise} value={franchise} />
          ))}
        </datalist>
        <TagEditor
          tags={draft.tags}
          onChange={set("tags")}
          error={errors.tags}
          disabled={busy}
        />
        <TextField
          label="Internal name"
          value={draft.internalName}
          onChange={set("internalName")}
          hint="Unique across the catalog; players never see it."
          error={errors.internalName}
          maxLength={160}
          required
          disabled={busy}
        />
        <div className="border-t border-line pt-5">
          <button
            type="button"
            className={button("danger")}
            disabled={busy}
            onClick={onDelete}
          >
            Delete game…
          </button>
        </div>
      </div>
      <div className="sticky bottom-0 flex flex-wrap items-center gap-3 border-t border-line bg-canvas px-4 py-4 sm:px-6">
        <p
          role={error ? "alert" : "status"}
          className={cx(
            "min-w-0 flex-1 text-1x5 max-sm:basis-full",
            error ? "text-danger" : "text-muted",
          )}
        >
          {error || (dirty ? "Unsaved changes" : "")}
        </p>
        <button
          type="button"
          className={cx(button("outline"), "max-sm:w-full")}
          disabled={busy || !dirty}
          onClick={() => {
            setDraft(gameDraft(game));
            setError("");
            setErrors({});
          }}
        >
          Discard
        </button>
        <button
          type="submit"
          className={cx(button("primary"), "max-sm:w-full")}
          disabled={busy || !dirty}
        >
          {busy ? "Saving…" : "Save changes"}
        </button>
      </div>
    </form>
  );
}

type ReleaseDialog =
  | { kind: "add" }
  | { kind: "edit" | "patch" | "delete"; version: CatalogVersion };

function ReleaseItem({
  version,
  onVisibility,
  onDialog,
  disabled,
}: {
  version: CatalogVersion;
  onVisibility: (visibility: Visibility) => void;
  onDialog: (dialog: ReleaseDialog) => void;
  disabled: boolean;
}) {
  return (
    <li className="rounded-panel border border-line bg-surface p-4">
      <div className="flex items-start gap-3">
        <div className="min-w-0 flex-1">
          <p className="font-bold break-words">{version.displayName}</p>
          <p className="mt-1 text-1x5 break-words text-muted">
            {version.requiredBase?.displayName ?? "No base ROM"}
          </p>
          <p className="mt-1 text-1x5">
            {version.patch ? (
              patchSummary(version.patch)
            ) : (
              <span className="text-warning">No patch</span>
            )}
            {" · "}
            {version.releasedAt ?? (
              <span className="text-warning">No release date</span>
            )}
          </p>
        </div>
        <Menu.Root>
          <Menu.Trigger
            className={button("outline", "icon")}
            aria-label={`More options for ${version.displayName}`}
            disabled={disabled}
          >
            <Icon name="dotsVertical" className="size-6" />
          </Menu.Trigger>
          <Menu.Portal>
            <Menu.Positioner align="end" sideOffset={4} className="z-50">
              <Menu.Popup data-ui className={cx(popup, "min-w-56 font-sans")}>
                <Menu.Item
                  className={popupItem}
                  onClick={() => onDialog({ kind: "edit", version })}
                >
                  Edit details…
                </Menu.Item>
                <Menu.Item
                  className={popupItem}
                  onClick={() => onDialog({ kind: "patch", version })}
                >
                  {version.patch ? "Replace patch…" : "Upload patch…"}
                </Menu.Item>
                <Menu.Item
                  className={cx(popupItem, "text-danger")}
                  onClick={() => onDialog({ kind: "delete", version })}
                >
                  Delete…
                </Menu.Item>
              </Menu.Popup>
            </Menu.Positioner>
          </Menu.Portal>
        </Menu.Root>
      </div>
      <FilterSelect
        label="Visibility"
        value={version.visibility}
        options={visibilities.map((value) => ({
          value,
          label: visibilityLabels[value],
        }))}
        onChange={(value) => onVisibility(value as Visibility)}
        disabled={disabled}
        className="mt-3 w-full"
      />
    </li>
  );
}

/**
 * One game's management: its details form and its releases. Every write goes
 * through `commit`, which refreshes the page's catalog afterwards.
 */
export function GamePanel({
  row,
  tab,
  onTabChange,
  bases,
  versions,
  franchises,
  roms,
  commit,
  onClose,
  onDeleted,
}: {
  row: ManageRow;
  tab: PanelTab;
  onTabChange: (tab: PanelTab) => void;
  bases: readonly CatalogBase[];
  versions: readonly CatalogVersion[];
  franchises: readonly string[];
  roms: RomSource;
  commit: Commit;
  onClose: () => void;
  onDeleted: () => void;
}) {
  const { game, releases } = row;
  const [dialog, setDialog] = useState<ReleaseDialog | null>(null);
  const [deletingGame, setDeletingGame] = useState(false);
  const [busy, setBusy] = useState(false);
  const [error, setError] = useState("");
  const close = () => {
    setDialog(null);
    setDeletingGame(false);
    setError("");
  };

  async function run(write: () => Promise<unknown>, after = close) {
    setBusy(true);
    setError("");
    try {
      await commit(write);
      after();
    } catch (failure) {
      setError(message(failure));
    } finally {
      setBusy(false);
    }
  }

  return (
    <div className="flex min-h-full flex-col">
      <header className="flex items-start gap-4 px-4 pt-4 sm:px-6 sm:pt-6">
        <Cover
          id={game.id}
          title={game.displayName}
          imageUrl={game.imageUrl}
          size="md"
        />
        <div className="min-w-0 flex-1">
          <h2 className="text-3x font-bold break-words text-heading">
            {game.displayName}
          </h2>
          <p className="mt-1 text-1x5 break-all text-muted">{game.id}</p>
        </div>
        <button
          className={cx(button("ghost", "icon"), "-mt-1 -mr-2")}
          aria-label="Close game"
          onClick={onClose}
        >
          <Icon name="x" className="size-7" />
        </button>
      </header>
      <Tabs.Root
        value={tab}
        onValueChange={(value) => onTabChange(value as PanelTab)}
        className="flex flex-1 flex-col"
      >
        <Tabs.List className="mx-4 mt-4 flex border-b border-line sm:mx-6">
          <Tabs.Tab value="details" className={tabClass}>
            Details
          </Tabs.Tab>
          <Tabs.Tab value="releases" className={tabClass}>
            Releases
            <span className="text-1x5 text-muted">{releases.length}</span>
          </Tabs.Tab>
        </Tabs.List>
        <Tabs.Panel value="details" className="flex-1 outline-none">
          <GameForm
            key={JSON.stringify(gameDraft(game))}
            row={row}
            franchises={franchises}
            commit={commit}
            onDelete={() => setDeletingGame(true)}
          />
        </Tabs.Panel>
        <Tabs.Panel
          value="releases"
          className="space-y-4 p-4 outline-none sm:p-6"
        >
          <div className="flex flex-wrap items-center gap-3">
            <p className="min-w-0 flex-1 basis-60 text-1x5 text-muted">
              {releases.length
                ? "Newest first. Visibility changes save at once."
                : "No releases yet: players cannot see this game."}
            </p>
            <button
              className={button("primary")}
              disabled={busy}
              onClick={() => setDialog({ kind: "add" })}
            >
              <Icon name="plus" className="size-6" />
              Add release
            </button>
          </div>
          {error && !dialog && (
            <p role="alert" className="text-1x5 text-danger">
              {error}
            </p>
          )}
          <ul aria-label="Releases" className="space-y-3">
            {releases.map((version) => (
              <ReleaseItem
                key={version.id}
                version={version}
                disabled={busy}
                onDialog={(next) => {
                  setError("");
                  setDialog(next);
                }}
                onVisibility={(visibility) =>
                  void run(
                    () => patchVersion(version.id, { visibility }),
                    () => {},
                  )
                }
              />
            ))}
          </ul>
        </Tabs.Panel>
      </Tabs.Root>
      {dialog?.kind === "add" && (
        <AddReleaseDialog
          game={game}
          releases={releases}
          bases={bases}
          versions={versions}
          roms={roms}
          commit={commit}
          onClose={close}
        />
      )}
      {dialog?.kind === "edit" && (
        <EditReleaseDialog
          version={dialog.version}
          others={releases.filter((other) => other.id !== dialog.version.id)}
          commit={commit}
          onClose={close}
        />
      )}
      {dialog?.kind === "patch" && (
        <UploadPatchDialog
          version={dialog.version}
          bases={bases}
          roms={roms}
          commit={commit}
          onClose={close}
        />
      )}
      {dialog?.kind === "delete" && (
        <ConfirmDialog
          title="Delete release"
          subtitle={`${game.displayName} · ${dialog.version.displayName}`}
          action="Delete release"
          busy={busy}
          error={error}
          onClose={close}
          onConfirm={() => void run(() => deleteVersion(dialog.version.id))}
        >
          <p>This removes the release from the catalog.</p>
          <p className="text-1x5 text-muted">
            Players who installed it keep their ROM and saves. The patch file
            stays in storage.
          </p>
        </ConfirmDialog>
      )}
      {deletingGame && (
        <ConfirmDialog
          title="Delete game"
          subtitle={game.displayName}
          action="Delete game"
          busy={busy}
          error={error}
          onClose={close}
          onConfirm={() =>
            void run(
              () => deleteGame(game.id),
              () => {
                close();
                onDeleted();
              },
            )
          }
        >
          <p>
            This removes the game
            {releases.length === 1
              ? " and its release"
              : releases.length > 1
                ? ` and its ${releases.length} releases`
                : ""}{" "}
            from the catalog.
          </p>
          <p className="text-1x5 text-muted">
            Players who installed it keep their ROMs and saves. Patch files stay
            in storage.
          </p>
        </ConfirmDialog>
      )}
    </div>
  );
}
