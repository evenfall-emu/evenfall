import { useEffect, useRef, useState, type ReactNode } from "react";
import { Radio } from "@base-ui/react/radio";
import { RadioGroup } from "@base-ui/react/radio-group";
import type { PatchFormat } from "../../patch";
import { Dialog, DialogActions } from "../../ui/dialog";
import { Icon } from "../../ui/icons";
import { FieldSelect } from "../../ui/select";
import { button, cx, focusRing } from "../../ui/styles";
import type {
  CatalogBase,
  CatalogGameSummary,
  CatalogVersion,
} from "../library/catalog";
import { ManageError, patchVersion, putVersion, uploadPatch } from "./api";
import { CheckboxRow, FormError, TextField } from "../../ui/fields";
import {
  releaseChanges,
  releaseDraft,
  releaseFields,
  shortHash,
  slug,
  visibilities,
  visibilityLabels,
  type ReleaseDraft,
  type Visibility,
} from "./model";
import { buildRelease, type RomSource } from "./verify";

/** Runs a catalog write, then refreshes what the page shows. */
export type Commit = (write: () => Promise<unknown>) => Promise<void>;

const MAX_PATCH_SIZE = 40 * 1024 * 1024;

const message = (error: unknown) =>
  error instanceof Error ? error.message : String(error);

const fieldErrors = (error: unknown) =>
  error instanceof ManageError ? error.fields : {};

type BaseRom =
  | { status: "loading" }
  /** `file` names the chosen file; null when it came from the Library. */
  | { status: "found"; bytes: Uint8Array; file: string | null }
  | { status: "missing" }
  | { status: "wrong"; file: string }
  | { status: "failed"; message: string };

type Build =
  | { status: "idle" }
  | { status: "working" }
  | { status: "built"; format: PatchFormat; outputSha1: string }
  | { status: "failed"; message: string };

/** A patch this device applied to its base, ready to upload. */
type VerifiedPatch = {
  baseId: string;
  bytes: Uint8Array;
  format: PatchFormat;
  outputSha1: string;
};

/**
 * Finds the base ROM on this device (or takes a chosen file), applies the
 * chosen patch to it, and names the output. `outputProblem` rejects an output
 * the caller cannot accept.
 */
function usePatchCheck(
  base: CatalogBase | undefined,
  roms: RomSource,
  outputProblem: (outputSha1: string) => string,
) {
  const [patch, setPatch] = useState<{ name: string; bytes: Uint8Array }>();
  const [patchError, setPatchError] = useState("");
  const [baseRom, setBaseRom] = useState<BaseRom>({ status: "loading" });
  const [build, setBuild] = useState<Build>({ status: "idle" });

  useEffect(() => {
    if (!base) return setBaseRom({ status: "missing" });
    let current = true;
    setBaseRom({ status: "loading" });
    roms.get(base.sha1).then(
      (bytes) =>
        current &&
        setBaseRom(
          bytes
            ? { status: "found", bytes, file: null }
            : { status: "missing" },
        ),
      () => current && setBaseRom({ status: "missing" }),
    );
    return () => {
      current = false;
    };
  }, [base?.sha1]);

  useEffect(() => {
    if (baseRom.status !== "found" || !patch)
      return setBuild({ status: "idle" });
    let current = true;
    setBuild({ status: "working" });
    // Let the "Applying" state paint; applying a large patch blocks briefly.
    const timer = setTimeout(() => {
      buildRelease(baseRom.bytes, patch.bytes).then(
        (built) => current && setBuild({ status: "built", ...built }),
        (error) =>
          current && setBuild({ status: "failed", message: message(error) }),
      );
    }, 0);
    return () => {
      current = false;
      clearTimeout(timer);
    };
  }, [baseRom, patch]);

  async function chooseBase(file: File) {
    if (!base) return;
    try {
      const { hash, rom } = await roms.inspect(file);
      setBaseRom(
        hash === base.sha1
          ? { status: "found", bytes: rom, file: file.name }
          : { status: "wrong", file: file.name },
      );
    } catch (error) {
      setBaseRom({ status: "failed", message: message(error) });
    }
  }

  async function choosePatch(file: File) {
    if (file.size > MAX_PATCH_SIZE) {
      setPatch(undefined);
      setPatchError("The patch is larger than 40 MiB.");
      return;
    }
    setPatchError("");
    setPatch({
      name: file.name,
      bytes: new Uint8Array(await file.arrayBuffer()),
    });
  }

  const problem =
    build.status === "built" ? outputProblem(build.outputSha1) : "";
  const verified: VerifiedPatch | null =
    base && patch && build.status === "built" && !problem
      ? {
          baseId: base.id,
          bytes: patch.bytes,
          format: build.format,
          outputSha1: build.outputSha1,
        }
      : null;
  return {
    base,
    patch,
    patchError,
    baseRom,
    build,
    problem,
    verified,
    chooseBase,
    choosePatch,
  };
}

type PatchCheck = ReturnType<typeof usePatchCheck>;

type StepState = "passed" | "failed" | "waiting" | "checking";

const stepStyles: Record<StepState, string> = {
  passed: "border-mint text-mint",
  failed: "border-danger text-danger",
  waiting: "border-line text-muted",
  checking: "border-control text-muted",
};

function ChainStep({
  state,
  title,
  children,
}: {
  state: StepState;
  title: string;
  children: ReactNode;
}) {
  return (
    <li className="relative flex gap-4 pb-5 last:pb-0 [&:not(:last-child)]:before:absolute [&:not(:last-child)]:before:top-10 [&:not(:last-child)]:before:bottom-1 [&:not(:last-child)]:before:left-[17px] [&:not(:last-child)]:before:border-l-2 [&:not(:last-child)]:before:border-dotted [&:not(:last-child)]:before:border-line">
      <span
        aria-hidden="true"
        className={cx(
          "grid size-9 shrink-0 place-items-center rounded-control border-2",
          stepStyles[state],
        )}
      >
        {state === "passed" ? (
          <Icon name="check" className="size-6" />
        ) : state === "failed" ? (
          <Icon name="x" className="size-6" />
        ) : state === "checking" ? (
          "…"
        ) : null}
      </span>
      <div className="min-w-0 flex-1 pt-1.5">
        <p className="font-bold text-heading">
          {title}
          <span className="sr-only">: {state}</span>
        </p>
        <div className="mt-1 text-1x5 break-words text-muted">{children}</div>
      </div>
    </li>
  );
}

/** A hidden file input and the button that opens it. */
function FileButton({
  label,
  accept,
  variant,
  disabled,
  onFile,
}: {
  label: string;
  accept: string;
  variant: "primary" | "outline";
  disabled?: boolean;
  onFile: (file: File) => void;
}) {
  const input = useRef<HTMLInputElement>(null);
  return (
    <>
      <button
        type="button"
        className={button(variant)}
        disabled={disabled}
        onClick={() => input.current?.click()}
      >
        {label}
      </button>
      <input
        ref={input}
        type="file"
        accept={accept}
        tabIndex={-1}
        aria-hidden="true"
        className="sr-only"
        onChange={(event) => {
          const file = event.target.files?.[0];
          event.target.value = "";
          if (file) onFile(file);
        }}
      />
    </>
  );
}

const size = (bytes: number) =>
  bytes >= 1_000_000
    ? `${(bytes / 1_000_000).toFixed(1)} MB`
    : `${Math.max(1, Math.round(bytes / 1_000))} KB`;

/**
 * The patch drop zone and the chain of checks that ends in the output SHA-1:
 * base ROM found, patch applies, output accepted.
 */
function PatchPanel({
  check,
  outputNote,
  disabled,
}: {
  check: PatchCheck;
  /** What an accepted output is, as "a new release". */
  outputNote: string;
  disabled?: boolean;
}) {
  const { base, baseRom, build, patch } = check;
  const baseName = base?.displayName ?? "The base ROM";
  const chooseBase = (
    <div className="mt-2">
      <FileButton
        label="Choose base ROM…"
        accept=".gba,.zip"
        variant="outline"
        disabled={disabled || !base}
        onFile={(file) => void check.chooseBase(file)}
      />
    </div>
  );
  return (
    <>
      <div
        className="mt-5 flex flex-wrap items-center gap-4 rounded-panel border-2 border-dashed border-line p-4"
        onDragOver={(event) => {
          if (event.dataTransfer.types.includes("Files"))
            event.preventDefault();
        }}
        onDrop={(event) => {
          event.preventDefault();
          const file = event.dataTransfer.files[0];
          if (file && !disabled) void check.choosePatch(file);
        }}
      >
        <Icon name="filePlus" className="size-8 shrink-0 text-mint" />
        <div className="min-w-0 flex-1">
          {patch ? (
            <>
              <p className="truncate">{patch.name}</p>
              <p className="text-1x5 text-muted">{size(patch.bytes.length)}</p>
            </>
          ) : (
            <>
              <p>Patch file</p>
              <p className="text-1x5 text-muted">
                BPS, UPS or IPS. Drop it here or choose it.
              </p>
            </>
          )}
        </div>
        <FileButton
          label={patch ? "Replace…" : "Choose patch…"}
          accept=".bps,.ups,.ips"
          variant={patch ? "outline" : "primary"}
          disabled={disabled}
          onFile={(file) => void check.choosePatch(file)}
        />
      </div>
      {check.patchError && (
        <p role="alert" className="mt-2 text-1x5 text-danger">
          {check.patchError}
        </p>
      )}
      <ol aria-label="Patch checks" className="mt-5">
        {baseRom.status === "found" ? (
          <ChainStep state="passed" title="Base ROM">
            {baseName} ·{" "}
            {baseRom.file ? `from ${baseRom.file}` : "in your Library"}
            <br />
            SHA-1 {base && shortHash(base.sha1)}
          </ChainStep>
        ) : baseRom.status === "loading" ? (
          <ChainStep state="checking" title="Base ROM">
            Looking for {baseName} on this device…
          </ChainStep>
        ) : baseRom.status === "missing" ? (
          <ChainStep state="waiting" title="Base ROM">
            {baseName} is not on this device.
            {chooseBase}
          </ChainStep>
        ) : (
          <ChainStep state="failed" title="Base ROM">
            {baseRom.status === "wrong"
              ? `${baseRom.file} is not ${baseName}.`
              : baseRom.message}
            {chooseBase}
          </ChainStep>
        )}
        {build.status === "built" ? (
          <ChainStep state="passed" title="Patch applies">
            {build.format === "ips"
              ? "IPS · this format carries no checksums"
              : `${build.format.toUpperCase()} · base and output checksums match`}
          </ChainStep>
        ) : build.status === "failed" ? (
          <ChainStep state="failed" title="Patch applies">
            {build.message}
          </ChainStep>
        ) : (
          <ChainStep
            state={build.status === "working" ? "checking" : "waiting"}
            title="Patch applies"
          >
            {build.status === "working"
              ? "Applying the patch…"
              : !patch
                ? "Choose the patch file."
                : "Waiting for the base ROM."}
          </ChainStep>
        )}
        {build.status === "built" ? (
          <ChainStep
            state={check.problem ? "failed" : "passed"}
            title="Output ROM"
          >
            SHA-1 {shortHash(build.outputSha1)}
            <br />
            {check.problem || outputNote}
          </ChainStep>
        ) : (
          <ChainStep state="waiting" title="Output ROM">
            Named by the SHA-1 of the patched ROM.
          </ChainStep>
        )}
      </ol>
      <p className="mt-5 text-1x5 text-muted">
        Checked on this device. Only the patch file is uploaded.
      </p>
    </>
  );
}

function ReleaseForm({
  draft,
  onChange,
  others,
  errors,
  disabled,
}: {
  draft: ReleaseDraft;
  onChange: (draft: ReleaseDraft) => void;
  /** The game's other releases, which may load this one's saves. */
  others: readonly CatalogVersion[];
  errors: Readonly<Record<string, string>>;
  disabled: boolean;
}) {
  const set =
    <K extends keyof ReleaseDraft>(key: K) =>
    (value: ReleaseDraft[K]) =>
      onChange({ ...draft, [key]: value });
  return (
    <div className="space-y-5">
      <TextField
        label="Label"
        value={draft.displayName}
        onChange={set("displayName")}
        hint="What players see, as 2.7 or 1.0 Beta 2."
        error={errors.displayName}
        maxLength={160}
        required
        disabled={disabled}
      />
      <TextField
        label="Internal name"
        value={draft.internalName}
        onChange={set("internalName")}
        placeholder={draft.displayName}
        hint="Unique within the game; orders undated releases. Defaults to the label."
        error={errors.internalName}
        maxLength={160}
        disabled={disabled}
      />
      <TextField
        label="Release date"
        type="date"
        value={draft.releasedAt}
        onChange={set("releasedAt")}
        error={errors.releasedAt}
        disabled={disabled}
      />
      <TextField
        label="Source link"
        type="url"
        value={draft.sourceRepositoryUrl}
        onChange={set("sourceRepositoryUrl")}
        placeholder="https://"
        hint="The creator's project or release page."
        error={errors.sourceRepositoryUrl}
        disabled={disabled}
      />
      <TextField
        label="Source revision"
        value={draft.sourceRevision}
        onChange={set("sourceRevision")}
        hint="A tag or commit, when the source is a repository."
        error={errors.sourceRevision}
        maxLength={160}
        disabled={disabled}
      />
      <TextField
        label="Release notes"
        rows={5}
        value={draft.releaseNotes}
        onChange={set("releaseNotes")}
        error={errors.releaseNotes}
        maxLength={20_000}
        disabled={disabled}
      />
      {others.length > 0 && (
        <fieldset>
          <legend className="mb-2 font-bold text-heading">
            Loads saves from
          </legend>
          <p className="mb-3 text-1x5 text-muted">
            Earlier releases whose in-game saves are verified to load in this
            one.
          </p>
          <div className="space-y-3">
            {others.map((other) => (
              <CheckboxRow
                key={other.id}
                checked={draft.saveCompatibleFrom.includes(other.id)}
                disabled={disabled}
                onChange={(checked) =>
                  set("saveCompatibleFrom")(
                    checked
                      ? [...draft.saveCompatibleFrom, other.id]
                      : draft.saveCompatibleFrom.filter(
                          (id) => id !== other.id,
                        ),
                  )
                }
              >
                {other.displayName}
              </CheckboxRow>
            ))}
          </div>
          {errors.saveCompatibleFrom && (
            <p className="mt-2 text-1x5 text-danger">
              {errors.saveCompatibleFrom}
            </p>
          )}
        </fieldset>
      )}
    </div>
  );
}

const visibilityNotes: Record<Visibility, string> = {
  hidden: "Only managers see it.",
  experimental: "Players who show experimental releases see it.",
  normal: "Reviewed: every player sees it.",
  featured: "Reviewed, and reserved for future placement.",
};

function VisibilityChoice({
  value,
  onChange,
  disabled,
}: {
  value: Visibility;
  onChange: (value: Visibility) => void;
  disabled: boolean;
}) {
  return (
    <fieldset>
      <legend className="mb-3 font-bold text-heading">Visibility</legend>
      <RadioGroup
        aria-label="Visibility"
        value={value}
        onValueChange={(next) => onChange(next as Visibility)}
        disabled={disabled}
        className="space-y-2"
      >
        {[...visibilities].reverse().map((option) => (
          <label
            key={option}
            className="flex cursor-pointer items-center gap-4 rounded-panel border border-line px-4 py-3 hover:bg-hover has-data-checked:border-mint has-data-checked:bg-selected"
          >
            <Radio.Root
              value={option}
              className={cx(
                "grid size-7 shrink-0 place-items-center rounded-full border-2 border-control data-checked:border-mint",
                focusRing,
              )}
            >
              <Radio.Indicator className="size-3.5 rounded-full bg-mint" />
            </Radio.Root>
            <div className="min-w-0">
              <p className="font-bold">{visibilityLabels[option]}</p>
              <p className="text-1x5 text-muted">{visibilityNotes[option]}</p>
            </div>
          </label>
        ))}
      </RadioGroup>
    </fieldset>
  );
}

function Steps({ current }: { current: 1 | 2 }) {
  return (
    <ol aria-label="Steps" className="mt-5 flex gap-2">
      {(["Patch", "Details"] as const).map((label, index) => {
        const step = index + 1;
        return (
          <li
            key={label}
            aria-current={step === current ? "step" : undefined}
            className={cx(
              "flex flex-1 items-center gap-3 border-b-4 pb-2 text-1x5",
              step === current
                ? "border-mint text-mint"
                : step < current
                  ? "border-mint/40 text-content"
                  : "border-line text-muted",
            )}
          >
            <span aria-hidden="true">{step}</span>
            {label}
          </li>
        );
      })}
    </ol>
  );
}

/**
 * Adds a release in two steps: verify the patch on this device, then
 * describe the release. The release is written, then its patch uploaded; a
 * failed upload leaves a patchless release that players cannot see, and
 * adding again retries it.
 */
export function AddReleaseDialog({
  game,
  releases,
  bases,
  versions,
  roms,
  commit,
  onClose,
}: {
  game: CatalogGameSummary;
  /** This game's releases, newest first. */
  releases: readonly CatalogVersion[];
  bases: readonly CatalogBase[];
  /** Every release in the catalog, whose IDs and outputs are taken. */
  versions: readonly CatalogVersion[];
  roms: RomSource;
  commit: Commit;
  onClose: () => void;
}) {
  const [step, setStep] = useState<1 | 2>(1);
  const [baseId, setBaseId] = useState(
    releases.find((release) => release.requiredBaseId)?.requiredBaseId ??
      bases[0]?.id ??
      "",
  );
  const check = usePatchCheck(
    bases.find((base) => base.id === baseId),
    roms,
    (outputSha1) => {
      const owner = versions.find(
        (version) => version.outputSha1 === outputSha1,
      );
      return owner
        ? `Already the output of ${owner.displayName} (${owner.id}).`
        : "";
    },
  );
  const [ready, setReady] = useState<VerifiedPatch | null>(null);
  const [draft, setDraft] = useState(releaseDraft);
  const [id, setId] = useState<string | null>(null);
  const derivedId = draft.displayName.trim()
    ? `${game.id}-${slug(draft.displayName)}`
    : "";
  const releaseId = id ?? derivedId;
  const [visibility, setVisibility] = useState<Visibility>("experimental");
  const [busy, setBusy] = useState("");
  const [error, setError] = useState("");
  const [errors, setErrors] = useState<Readonly<Record<string, string>>>({});
  // A release written by an earlier attempt whose upload failed.
  const saved = useRef<string | null>(null);

  async function add() {
    if (!ready) return;
    const problems: Record<string, string> = {};
    if (!draft.displayName.trim()) problems.displayName = "Enter a label.";
    if (!releaseId || slug(releaseId) !== releaseId)
      problems.id =
        "Use lowercase letters and digits, separated by single - or . characters.";
    else if (
      releaseId !== saved.current &&
      versions.some((version) => version.id === releaseId)
    )
      problems.id = "Another release has this ID.";
    setErrors(problems);
    setError("");
    if (Object.keys(problems).length) return;
    let written = false;
    try {
      await commit(async () => {
        setBusy("Saving release…");
        await putVersion(releaseId, {
          gameId: game.id,
          ...releaseFields(draft),
          outputSha1: ready.outputSha1,
          requiredBaseId: ready.baseId,
          patch: null,
          visibility,
        });
        written = true;
        saved.current = releaseId;
        setBusy("Uploading patch…");
        await uploadPatch(releaseId, ready.bytes, ready.format);
      });
      onClose();
    } catch (failure) {
      setErrors(fieldErrors(failure));
      setError(
        written
          ? `The release was saved without its patch: ${message(failure)} Add it again to retry the upload.`
          : message(failure),
      );
    } finally {
      setBusy("");
    }
  }

  return (
    <Dialog
      open
      onOpenChange={(open) => !open && onClose()}
      busy={Boolean(busy)}
      title="Add release"
      subtitle={game.displayName}
    >
      <Steps current={step} />
      {step === 1 ? (
        <>
          {bases.length > 0 ? (
            <div className="mt-5">
              <FieldSelect
                label="Base ROM"
                value={baseId}
                options={bases.map((base) => ({
                  value: base.id,
                  label: base.displayName,
                }))}
                onChange={setBaseId}
              />
            </div>
          ) : (
            <FormError message="The catalog has no base ROMs. Add one through the admin API first." />
          )}
          <PatchPanel check={check} outputNote="A new release." />
          <DialogActions>
            <button className={button("outline")} onClick={onClose}>
              Cancel
            </button>
            <button
              className={button("primary")}
              disabled={!check.verified}
              onClick={() => {
                setReady(check.verified);
                setStep(2);
              }}
            >
              Continue
            </button>
          </DialogActions>
        </>
      ) : (
        <>
          <p className="mt-5 text-1x5 text-muted">
            Output SHA-1 {ready && shortHash(ready.outputSha1)} ·{" "}
            {ready?.format.toUpperCase()} patch verified
          </p>
          <div className="mt-5 space-y-5">
            <ReleaseForm
              draft={draft}
              onChange={setDraft}
              others={releases}
              errors={errors}
              disabled={Boolean(busy)}
            />
            <TextField
              label="Release ID"
              value={releaseId}
              onChange={setId}
              hint="Permanent. Derived from the game and label."
              error={errors.id}
              maxLength={80}
              disabled={Boolean(busy)}
            />
            <VisibilityChoice
              value={visibility}
              onChange={setVisibility}
              disabled={Boolean(busy)}
            />
          </div>
          <FormError message={error} />
          <DialogActions>
            <button
              className={button("outline")}
              disabled={Boolean(busy)}
              onClick={() => setStep(1)}
            >
              Back
            </button>
            <button
              className={button("primary")}
              disabled={Boolean(busy)}
              onClick={() => void add()}
            >
              {busy || "Add release"}
            </button>
          </DialogActions>
        </>
      )}
    </Dialog>
  );
}

/**
 * Attaches or replaces a release's patch. The patch must build exactly the
 * release's output from its base; a release without a base takes the chosen
 * one.
 */
export function UploadPatchDialog({
  version,
  bases,
  roms,
  commit,
  onClose,
}: {
  version: CatalogVersion;
  bases: readonly CatalogBase[];
  roms: RomSource;
  commit: Commit;
  onClose: () => void;
}) {
  const [baseId, setBaseId] = useState(
    version.requiredBaseId ?? bases[0]?.id ?? "",
  );
  const check = usePatchCheck(
    bases.find((base) => base.id === baseId),
    roms,
    (outputSha1) =>
      outputSha1 === version.outputSha1
        ? ""
        : `This release is ${shortHash(version.outputSha1)}; this patch builds another ROM.`,
  );
  const [busy, setBusy] = useState(false);
  const [error, setError] = useState("");

  async function upload() {
    const verified = check.verified;
    if (!verified) return;
    setBusy(true);
    setError("");
    try {
      await commit(async () => {
        if (version.requiredBaseId !== verified.baseId)
          await patchVersion(version.id, { requiredBaseId: verified.baseId });
        await uploadPatch(version.id, verified.bytes, verified.format);
      });
      onClose();
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
      title={version.patch ? "Replace patch" : "Upload patch"}
      subtitle={version.displayName}
    >
      {!version.requiredBaseId && (
        <div className="mt-5">
          <FieldSelect
            label="Base ROM"
            value={baseId}
            options={bases.map((base) => ({
              value: base.id,
              label: base.displayName,
            }))}
            onChange={setBaseId}
          />
        </div>
      )}
      <PatchPanel
        check={check}
        outputNote="Matches this release."
        disabled={busy}
      />
      <FormError message={error} />
      <DialogActions>
        <button className={button("outline")} disabled={busy} onClick={onClose}>
          Cancel
        </button>
        <button
          className={button("primary")}
          disabled={busy || !check.verified}
          onClick={() => void upload()}
        >
          {busy ? "Uploading…" : "Upload patch"}
        </button>
      </DialogActions>
    </Dialog>
  );
}

export function EditReleaseDialog({
  version,
  others,
  commit,
  onClose,
}: {
  version: CatalogVersion;
  others: readonly CatalogVersion[];
  commit: Commit;
  onClose: () => void;
}) {
  const [draft, setDraft] = useState(() => releaseDraft(version));
  const [busy, setBusy] = useState(false);
  const [error, setError] = useState("");
  const [errors, setErrors] = useState<Readonly<Record<string, string>>>({});
  const changes = releaseChanges(version, draft);
  const dirty = Object.keys(changes).length > 0;

  async function save() {
    if (!draft.displayName.trim()) {
      setErrors({ displayName: "Enter a label." });
      return;
    }
    setBusy(true);
    setError("");
    setErrors({});
    try {
      await commit(() => patchVersion(version.id, changes));
      onClose();
    } catch (failure) {
      setErrors(fieldErrors(failure));
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
      title="Edit release"
      subtitle={`${version.displayName} · ${version.id}`}
    >
      <div className="mt-5">
        <ReleaseForm
          draft={draft}
          onChange={setDraft}
          others={others}
          errors={errors}
          disabled={busy}
        />
      </div>
      <FormError message={error} />
      <DialogActions>
        <button className={button("outline")} disabled={busy} onClick={onClose}>
          Cancel
        </button>
        <button
          className={button("primary")}
          disabled={busy || !dirty}
          onClick={() => void save()}
        >
          {busy ? "Saving…" : "Save changes"}
        </button>
      </DialogActions>
    </Dialog>
  );
}
