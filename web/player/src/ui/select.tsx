import type { ReactNode } from "react";
import { Select as BaseSelect } from "@base-ui/react/select";
import { Icon } from "./icons";
import { cx, focusRing, popup, popupItem } from "./styles";

export type SelectOption = { value: string; label: string };

function Options({
  options,
  compact = false,
}: {
  options: readonly SelectOption[];
  compact?: boolean;
}) {
  return (
    <BaseSelect.Portal>
      <BaseSelect.Positioner
        alignItemWithTrigger={false}
        sideOffset={4}
        className="z-50 outline-none"
      >
        <BaseSelect.Popup
          data-ui
          className={cx(
            popup,
            "max-h-(--available-height) min-w-(--anchor-width) overflow-y-auto font-sans",
            compact && "[&_[role=option]]:text-1x5",
          )}
        >
          <BaseSelect.List>
            {options.map((option) => (
              <BaseSelect.Item
                key={option.value}
                value={option.value}
                className={popupItem}
              >
                <BaseSelect.ItemText className="flex-1">
                  {option.label}
                </BaseSelect.ItemText>
                <BaseSelect.ItemIndicator>
                  <Icon name="check" className="size-5" />
                </BaseSelect.ItemIndicator>
              </BaseSelect.Item>
            ))}
          </BaseSelect.List>
        </BaseSelect.Popup>
      </BaseSelect.Positioner>
    </BaseSelect.Portal>
  );
}

const trigger = cx(
  "flex h-12 min-w-0 cursor-pointer items-center gap-2 rounded-control border border-line bg-canvas px-3 text-left hover:border-control data-popup-open:border-mint disabled:cursor-not-allowed disabled:text-muted",
  focusRing,
);

/**
 * A compact select whose visible label shares the box with its value, as in
 * "Show: All". The label is also the accessible name. In a container narrower
 * than a row of three filters (640px), the label stacks above the value.
 */
export function FilterSelect({
  label,
  value,
  options,
  onChange,
  disabled,
  className,
}: {
  label: string;
  value: string;
  options: readonly SelectOption[];
  onChange: (value: string) => void;
  disabled?: boolean;
  className?: string;
}) {
  return (
    <BaseSelect.Root
      items={options}
      value={value}
      disabled={disabled}
      onValueChange={(next) => next !== null && onChange(next)}
    >
      <BaseSelect.Label className="sr-only">{label}</BaseSelect.Label>
      <BaseSelect.Trigger
        className={cx(trigger, "text-1x5 max-sm:px-2.5", className)}
      >
        <span className="flex min-w-0 flex-1 @max-[640px]:flex-col @min-[640px]:items-center @min-[640px]:gap-2">
          <span
            aria-hidden="true"
            className="shrink-0 @max-[640px]:leading-5 @max-[640px]:text-muted"
          >
            {label}
            <span className="@max-[640px]:hidden">:</span>
          </span>
          <BaseSelect.Value className="min-w-0 truncate @max-[640px]:leading-5" />
        </span>
        <BaseSelect.Icon className="shrink-0">
          <Icon name="chevronDown" className="size-5" />
        </BaseSelect.Icon>
      </BaseSelect.Trigger>
      <Options options={options} compact />
    </BaseSelect.Root>
  );
}

/** A full-width form select with its label above. */
export function FieldSelect({
  label,
  value,
  options,
  onChange,
  disabled,
  hint,
}: {
  label: string;
  value: string;
  options: readonly SelectOption[];
  onChange: (value: string) => void;
  disabled?: boolean;
  hint?: ReactNode;
}) {
  return (
    <BaseSelect.Root
      items={options}
      value={value}
      disabled={disabled}
      onValueChange={(next) => next !== null && onChange(next)}
    >
      <BaseSelect.Label className="mb-2 block font-bold text-heading">
        {label}
      </BaseSelect.Label>
      <BaseSelect.Trigger className={cx(trigger, "w-full text-2x")}>
        <BaseSelect.Value className="min-w-0 flex-1 truncate" />
        <BaseSelect.Icon className="shrink-0">
          <Icon name="chevronDown" className="size-5" />
        </BaseSelect.Icon>
      </BaseSelect.Trigger>
      {hint}
      <Options options={options} />
    </BaseSelect.Root>
  );
}
