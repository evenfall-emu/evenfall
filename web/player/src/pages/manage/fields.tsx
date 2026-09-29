import { useId, useState, type ReactNode } from "react";
import { Checkbox } from "@base-ui/react/checkbox";
import { Dialog, DialogActions } from "../../ui/dialog";
import { Icon } from "../../ui/icons";
import { button, chip, cx, focusRing } from "../../ui/styles";

export const inputClass =
  "h-12 w-full min-w-0 rounded-control border border-line bg-canvas px-3 text-2x [color-scheme:dark] placeholder:text-muted focus:border-mint focus:outline-none disabled:text-muted aria-invalid:border-danger";

const labelClass = "mb-2 block font-bold text-heading";

/** A hint under a field, or its error in its place. */
function Note({
  id,
  hint,
  error,
}: {
  id: string;
  hint?: ReactNode;
  error?: string;
}) {
  if (!error && !hint) return null;
  return (
    <p
      id={id}
      className={cx("mt-2 text-1x5", error ? "text-danger" : "text-muted")}
    >
      {error ?? hint}
    </p>
  );
}

/** A labelled single-line or multiline text input. */
export function TextField({
  label,
  value,
  onChange,
  hint,
  error,
  rows,
  ...input
}: {
  label: string;
  value: string;
  onChange: (value: string) => void;
  hint?: ReactNode;
  error?: string;
  /** Makes the field multiline. */
  rows?: number;
  type?: string;
  placeholder?: string;
  maxLength?: number;
  disabled?: boolean;
  required?: boolean;
  autoFocus?: boolean;
  list?: string;
}) {
  const id = useId();
  const common = {
    id,
    value,
    "aria-describedby": error || hint ? `${id}-note` : undefined,
    "aria-invalid": error ? true : undefined,
    ...input,
  };
  return (
    <div>
      <label htmlFor={id} className={labelClass}>
        {label}
      </label>
      {rows ? (
        <textarea
          {...common}
          rows={rows}
          onChange={(event) => onChange(event.target.value)}
          className={cx(inputClass, "h-auto py-2 leading-6")}
        />
      ) : (
        <input
          {...common}
          onChange={(event) => onChange(event.target.value)}
          className={inputClass}
        />
      )}
      <Note id={`${id}-note`} hint={hint} error={error} />
    </div>
  );
}

/** Tags as removable chips; Enter or a comma adds the typed one. */
export function TagEditor({
  tags,
  onChange,
  disabled,
  error,
}: {
  tags: readonly string[];
  onChange: (tags: string[]) => void;
  disabled?: boolean;
  error?: string;
}) {
  const id = useId();
  const [draft, setDraft] = useState("");
  function add() {
    const tag = draft.trim();
    setDraft("");
    if (
      tag &&
      !tags.some((item) => item.toLocaleLowerCase() === tag.toLocaleLowerCase())
    )
      onChange([...tags, tag]);
  }
  return (
    <div>
      <label htmlFor={id} className={labelClass}>
        Tags
      </label>
      <div
        className={cx(
          "flex flex-wrap items-center gap-2 rounded-control border bg-canvas p-2 focus-within:border-mint",
          error ? "border-danger" : "border-line",
        )}
      >
        {tags.map((tag) => (
          <span
            key={tag}
            className={cx(chip, "h-9 gap-1 border-mint/50 pr-1 text-content")}
          >
            {tag}
            <button
              type="button"
              aria-label={`Remove tag ${tag}`}
              disabled={disabled}
              onClick={() => onChange(tags.filter((item) => item !== tag))}
              className={cx(
                "grid size-7 cursor-pointer place-items-center rounded-control hover:bg-hover",
                focusRing,
              )}
            >
              <Icon name="x" className="size-4" />
            </button>
          </span>
        ))}
        <input
          id={id}
          value={draft}
          disabled={disabled || tags.length >= 16}
          maxLength={40}
          placeholder={tags.length >= 16 ? "16 tags at most" : "Add tag…"}
          aria-describedby={`${id}-note`}
          onChange={(event) => setDraft(event.target.value)}
          onBlur={add}
          onKeyDown={(event) => {
            if (event.key === "Enter" || event.key === ",") {
              event.preventDefault();
              add();
            } else if (event.key === "Backspace" && !draft && tags.length)
              onChange(tags.slice(0, -1));
          }}
          className="h-9 min-w-32 flex-1 bg-transparent px-1 text-1x5 placeholder:text-muted focus:outline-none"
        />
      </div>
      <Note
        id={`${id}-note`}
        hint="Enter adds a tag. Up to 16."
        error={error}
      />
    </div>
  );
}

export function CheckboxRow({
  checked,
  onChange,
  disabled,
  children,
}: {
  checked: boolean;
  onChange: (checked: boolean) => void;
  disabled?: boolean;
  children: ReactNode;
}) {
  return (
    <label className="flex cursor-pointer items-center gap-4">
      <Checkbox.Root
        checked={checked}
        onCheckedChange={onChange}
        disabled={disabled}
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

/** An error from the last attempt, above a dialog's actions. */
export function FormError({ message }: { message: string }) {
  if (!message) return null;
  return (
    <p
      role="alert"
      className="mt-5 flex items-start gap-3 rounded-panel border border-danger/60 bg-danger-surface p-3 text-1x5 text-danger"
    >
      <Icon name="alert" className="size-6 shrink-0" />
      {message}
    </p>
  );
}

/** Asks before an action that removes catalog records. */
export function ConfirmDialog({
  title,
  subtitle,
  action,
  busy,
  error,
  onClose,
  onConfirm,
  children,
}: {
  title: string;
  subtitle?: string;
  action: string;
  busy: boolean;
  error: string;
  onClose: () => void;
  onConfirm: () => void;
  children: ReactNode;
}) {
  return (
    <Dialog
      open
      onOpenChange={(open) => !open && onClose()}
      busy={busy}
      title={title}
      subtitle={subtitle}
    >
      <div className="mt-5 space-y-3">{children}</div>
      <FormError message={error} />
      <DialogActions>
        <button className={button("outline")} disabled={busy} onClick={onClose}>
          Cancel
        </button>
        <button
          className={button("danger")}
          disabled={busy}
          onClick={onConfirm}
        >
          {busy ? "Deleting…" : action}
        </button>
      </DialogActions>
    </Dialog>
  );
}
