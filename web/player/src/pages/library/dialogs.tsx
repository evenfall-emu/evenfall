import { useState, type ReactNode } from "react";
import { Checkbox } from "@base-ui/react/checkbox";
import { Radio } from "@base-ui/react/radio";
import { RadioGroup } from "@base-ui/react/radio-group";
import { Dialog, DialogActions } from "../../ui/dialog";
import { Icon, type IconName } from "../../ui/icons";
import { FieldSelect } from "../../ui/select";
import { button, cx, focusRing } from "../../ui/styles";
import { Cover } from "./cover";
import { formatMoment } from "./format";
import {
  saveCompatible,
  type LibraryGame,
  type PlaythroughEntry,
} from "./model";

type DialogProps = {
  game: LibraryGame;
  busy: boolean;
  onClose: () => void;
};

function Warning({ title, children }: { title: string; children: ReactNode }) {
  return (
    <div className="mt-5 flex items-start gap-4 rounded-panel border border-warning/70 bg-warning-surface p-4 text-warning">
      <Icon name="alertFilled" className="size-10 shrink-0" />
      <div>
        <p className="font-bold">{title}</p>
        <p className="mt-1 text-1x5 opacity-85">{children}</p>
      </div>
    </div>
  );
}

function Fact({
  icon,
  title,
  detail,
}: {
  icon: IconName;
  title: string;
  detail?: string;
}) {
  return (
    <li className="flex items-start gap-4">
      <Icon name={icon} className="mt-0.5 size-7 shrink-0" />
      <div>
        <p>{title}</p>
        {detail && <p className="text-1x5 text-muted">{detail}</p>}
      </div>
    </li>
  );
}

function Confirm({
  checked,
  onChange,
  children,
}: {
  checked: boolean;
  onChange: (checked: boolean) => void;
  children: ReactNode;
}) {
  return (
    <label className="mt-5 flex cursor-pointer items-center gap-4">
      <Checkbox.Root
        checked={checked}
        onCheckedChange={onChange}
        className={cx(
          "grid size-7 shrink-0 place-items-center rounded-control border-2 border-control text-on-accent data-checked:border-mint data-checked:bg-mint",
          focusRing,
        )}
      >
        <Checkbox.Indicator>
          <Icon name="check" className="size-5" />
        </Checkbox.Indicator>
      </Checkbox.Root>
      {children}
    </label>
  );
}

/** Carries one playthrough's in-game save to the latest release. */
export function UpdateDialog({
  game,
  busy,
  onClose,
  onConfirm,
  onAddFile,
}: DialogProps & {
  onConfirm: (run: PlaythroughEntry) => void;
  /** Chooses the latest release's ROM when this device lacks it. */
  onAddFile: () => void;
}) {
  const [runId, setRunId] = useState(game.updatable[0]?.id ?? "");
  const [understood, setUnderstood] = useState(false);
  const run = game.updatable.find((item) => item.id === runId);
  const verified = run ? saveCompatible(run.release, game.latest) : false;
  const { installed } = game.latest;
  return (
    <Dialog
      open
      onOpenChange={(open) => !open && onClose()}
      busy={busy}
      leading={
        <Cover
          id={game.id}
          title={game.title}
          imageUrl={game.imageUrl}
          size="md"
        />
      }
      title={`${game.title} · ${game.latest.label}`}
    >
      <div className="mt-6">
        <FieldSelect
          label="Playthrough to update"
          value={runId}
          disabled={busy}
          options={game.updatable.map((item) => ({
            value: item.id,
            label: `${item.name} · ${item.release.label}`,
          }))}
          onChange={(value) => {
            setRunId(value);
            setUnderstood(false);
          }}
          hint={
            <p className="mt-3 flex items-center gap-3 text-1x5 text-muted">
              <Icon name="info" className="size-6 shrink-0 text-mint" />
              Only playthroughs on a different release appear.
            </p>
          }
        />
      </div>
      {!verified && (
        <Warning title="Save compatibility not verified">
          Your in-game save may not work correctly in {game.latest.label}.
        </Warning>
      )}
      <ul className="mt-5 space-y-4">
        <Fact
          icon="copy"
          title="Copy the in-game save."
          detail="Quick states and rewind stay on the old release."
        />
        <Fact icon="history" title="Old progress remains available." />
      </ul>
      {!verified && installed && (
        <Confirm checked={understood} onChange={setUnderstood}>
          I understand compatibility is unverified.
        </Confirm>
      )}
      {!installed && (
        <p className="mt-5 flex items-start gap-3 text-1x5">
          <Icon name="filePlus" className="size-6 shrink-0 text-warning" />
          Add the {game.latest.label} ROM to this device first. It stays on this
          device.
        </p>
      )}
      <DialogActions>
        <button
          className={button("outline", "lg")}
          disabled={busy}
          onClick={onClose}
        >
          Cancel
        </button>
        {installed ? (
          <button
            className={button("primary", "lg")}
            disabled={busy || !run || (!verified && !understood)}
            onClick={() => run && onConfirm(run)}
          >
            Copy save &amp; play
          </button>
        ) : (
          <button
            className={button("primary", "lg")}
            disabled={busy}
            onClick={onAddFile}
          >
            Choose file…
          </button>
        )}
      </DialogActions>
    </Dialog>
  );
}

function ProgressChoice({
  value,
  icon,
  iconClass,
  title,
  detail,
}: {
  value: "device" | "cloud";
  icon: IconName;
  iconClass?: string;
  title: string;
  detail: string;
}) {
  return (
    <label className="flex cursor-pointer items-center gap-4 rounded-panel border border-line px-4 py-3 hover:bg-hover has-data-checked:border-mint has-data-checked:bg-selected">
      <Radio.Root
        value={value}
        className={cx(
          "grid size-7 shrink-0 place-items-center rounded-full border-2 border-control data-checked:border-mint",
          focusRing,
        )}
      >
        <Radio.Indicator className="size-3.5 rounded-full bg-mint" />
      </Radio.Root>
      <Icon name={icon} className={cx("size-9 shrink-0", iconClass)} />
      <div className="min-w-0">
        <p className="font-bold">{title}</p>
        <p className="text-1x5 text-muted">{detail}</p>
      </div>
    </label>
  );
}

/** Both copies of a run changed: the user picks the one to continue. */
export function ProgressDialog({
  game,
  run,
  now,
  busy,
  onClose,
  onChoose,
  onExport,
}: DialogProps & {
  run: PlaythroughEntry & {
    conflict: NonNullable<PlaythroughEntry["conflict"]>;
  };
  now: number;
  onChoose: (progress: "device" | "cloud") => void;
  onExport: () => void;
}) {
  const [choice, setChoice] = useState<"device" | "cloud" | null>(null);
  const { conflict } = run;
  return (
    <Dialog
      open
      onOpenChange={(open) => !open && onClose()}
      busy={busy}
      title="Choose progress"
      subtitle={[game.title, run.name, run.release.label]
        .filter(Boolean)
        .join(" · ")}
    >
      <p className="mt-5">
        Cloud progress changed since this device last synced. Choose which
        progress to play.
      </p>
      <RadioGroup
        aria-label="Progress to play"
        value={choice}
        onValueChange={(value) => setChoice(value as "device" | "cloud")}
        disabled={busy}
        className="mt-4 space-y-3"
      >
        <ProgressChoice
          value="device"
          icon="monitor"
          iconClass="text-mint"
          title="This device"
          detail={`Last played ${formatMoment(conflict.deviceLastPlayed, now)}`}
        />
        <ProgressChoice
          value="cloud"
          icon="cloud"
          title="Cloud backup"
          detail={`Last played ${formatMoment(conflict.cloudCapturedAt, now)} · Uploaded ${formatMoment(conflict.cloudUploadedAt, now)}`}
        />
      </RadioGroup>
      <Warning title="Using cloud progress replaces this device’s copy of this run.">
        Export local progress first to keep it.
      </Warning>
      <button
        className={cx(
          "mt-4 inline-flex cursor-pointer items-center gap-3 rounded-control py-1 text-mint underline underline-offset-4 hover:text-mint-hover",
          focusRing,
        )}
        disabled={busy}
        onClick={onExport}
      >
        <Icon name="fileExport" className="size-7 text-content" />
        Export local save
      </button>
      <DialogActions>
        <button
          className={button("outline", "lg")}
          disabled={busy}
          onClick={onClose}
        >
          Cancel
        </button>
        <button
          className={button("primary", "lg")}
          disabled={busy || !choice}
          onClick={() => choice && onChoose(choice)}
        >
          Use selected progress
        </button>
      </DialogActions>
    </Dialog>
  );
}

export function ReleaseNotesDialog({ game, busy, onClose }: DialogProps) {
  return (
    <Dialog
      open
      onOpenChange={(open) => !open && onClose()}
      busy={busy}
      title="Release notes"
      subtitle={`${game.title} · ${game.latest.label}`}
    >
      <p className="mt-5 whitespace-pre-line">{game.latest.releaseNotes}</p>
    </Dialog>
  );
}

export function DescriptionDialog({
  game,
  onClose,
}: Omit<DialogProps, "busy">) {
  return (
    <Dialog
      open
      onOpenChange={(open) => !open && onClose()}
      title={game.title}
      subtitle={game.franchise ?? undefined}
    >
      <p className="mt-5 whitespace-pre-line">{game.description}</p>
      {game.latest.sourceUrl && /^https?:\/\//i.test(game.latest.sourceUrl) && (
        <a
          href={game.latest.sourceUrl}
          target="_blank"
          rel="noopener noreferrer"
          className={cx(
            "mt-5 inline-flex items-center gap-2 text-mint underline underline-offset-4 hover:text-mint-hover",
            focusRing,
          )}
        >
          Creator's page <Icon name="externalLink" className="size-5" />
        </a>
      )}
    </Dialog>
  );
}

function NameField({
  value,
  disabled,
  onChange,
}: {
  value: string;
  disabled: boolean;
  onChange: (value: string) => void;
}) {
  return (
    <label className="mt-6 block">
      <span className="mb-2 block font-bold text-heading">Name</span>
      <input
        value={value}
        maxLength={64}
        required
        autoFocus
        disabled={disabled}
        onChange={(event) => onChange(event.target.value)}
        className="h-12 w-full rounded-control border border-line bg-canvas px-3 text-2x focus:border-mint focus:outline-none"
      />
    </label>
  );
}

export function NewPlaythroughDialog({
  game,
  busy,
  onClose,
  onCreate,
}: DialogProps & {
  onCreate: (name: string, hash: string) => void;
}) {
  const releases = game.releases.filter((item) => item.installed);
  const [name, setName] = useState(
    `Playthrough ${game.playthroughs.length + 1}`,
  );
  const [hash, setHash] = useState(releases[0]?.hash ?? "");
  const trimmed = name.trim();
  return (
    <Dialog
      open
      onOpenChange={(open) => !open && onClose()}
      busy={busy}
      title="New playthrough"
      subtitle={game.title}
    >
      <form
        onSubmit={(event) => {
          event.preventDefault();
          if (trimmed && hash) onCreate(trimmed, hash);
        }}
      >
        <NameField value={name} disabled={busy} onChange={setName} />
        {releases.length > 1 && (
          <div className="mt-5">
            <FieldSelect
              label="Release"
              value={hash}
              disabled={busy}
              options={releases.map((item) => ({
                value: item.hash,
                label:
                  item === game.latest ? `${item.label} (latest)` : item.label,
              }))}
              onChange={setHash}
            />
          </div>
        )}
        <DialogActions>
          <button
            type="button"
            className={button("outline", "lg")}
            disabled={busy}
            onClick={onClose}
          >
            Cancel
          </button>
          <button
            type="submit"
            className={button("primary", "lg")}
            disabled={busy || !trimmed || !hash}
          >
            Create &amp; play
          </button>
        </DialogActions>
      </form>
    </Dialog>
  );
}

export function RenameDialog({
  game,
  run,
  busy,
  onClose,
  onRename,
}: DialogProps & {
  run: PlaythroughEntry;
  onRename: (name: string) => void;
}) {
  const [name, setName] = useState(run.name);
  const trimmed = name.trim();
  return (
    <Dialog
      open
      onOpenChange={(open) => !open && onClose()}
      busy={busy}
      title="Rename playthrough"
      subtitle={game.title}
    >
      <form
        onSubmit={(event) => {
          event.preventDefault();
          if (trimmed) onRename(trimmed);
        }}
      >
        <NameField value={name} disabled={busy} onChange={setName} />
        <DialogActions>
          <button
            type="button"
            className={button("outline", "lg")}
            disabled={busy}
            onClick={onClose}
          >
            Cancel
          </button>
          <button
            type="submit"
            className={button("primary", "lg")}
            disabled={busy || !trimmed || trimmed === run.name}
          >
            Rename
          </button>
        </DialogActions>
      </form>
    </Dialog>
  );
}

export function DeleteDialog({
  game,
  run,
  busy,
  onClose,
  onDelete,
}: DialogProps & { run: PlaythroughEntry; onDelete: () => void }) {
  return (
    <Dialog
      open
      onOpenChange={(open) => !open && onClose()}
      busy={busy}
      title="Delete playthrough?"
      subtitle={`${game.title} · ${run.name}`}
    >
      <p className="mt-5">
        This removes the playthrough&rsquo;s in-game save, resume point and
        quick states from this device.
      </p>
      <p className="mt-3 text-1x5 text-muted">
        When online and signed in, its cloud backup is also deleted. Offline or
        signed out, the cloud backup stays.
      </p>
      <DialogActions>
        <button
          className={button("outline", "lg")}
          disabled={busy}
          onClick={onClose}
        >
          Cancel
        </button>
        <button
          className={button("danger", "lg")}
          disabled={busy}
          onClick={onDelete}
        >
          Delete
        </button>
      </DialogActions>
    </Dialog>
  );
}
