import { useId, useState, type ReactNode } from "react";
import { Dialog, DialogActions } from "../../ui/dialog";
import { FormError, labelClass, Note } from "../../ui/fields";
import { Icon } from "../../ui/icons";
import { button, chip, cx, focusRing } from "../../ui/styles";

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
