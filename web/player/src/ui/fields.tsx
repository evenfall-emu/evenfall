import { useId, type ReactNode } from "react";
import { Checkbox } from "@base-ui/react/checkbox";
import { Icon } from "./icons";
import { cx, focusRing } from "./styles";

const inputClass =
  "h-12 w-full min-w-0 rounded-control border border-line bg-canvas px-3 text-2x [color-scheme:dark] placeholder:text-muted focus:border-mint focus:outline-none disabled:text-muted aria-invalid:border-danger";

export const labelClass = "mb-2 block font-bold text-heading";

/** A hint under a field, or its error in its place. */
export function Note({
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
