/** Joins class names, skipping falsy entries. */
export function cx(...classes: (string | false | null | undefined)[]) {
  return classes.filter(Boolean).join(" ");
}

export const focusRing =
  "outline-none focus-visible:outline-2 focus-visible:outline-offset-2 focus-visible:outline-mint";

const buttonVariants = {
  /** The one main action in a region. */
  primary:
    "bg-mint text-on-accent hover:bg-mint-hover active:bg-mint-pressed disabled:border disabled:border-line disabled:bg-raised disabled:text-muted",
  /** Attention without urgency: updates and other offers. */
  accent:
    "bg-peach text-on-accent hover:bg-peach-hover active:bg-peach-pressed disabled:border disabled:border-line disabled:bg-raised disabled:text-muted",
  outline:
    "border border-control text-content hover:bg-hover active:bg-selected disabled:border-line disabled:text-muted",
  ghost: "text-content hover:bg-hover active:bg-selected disabled:text-muted",
};

const buttonSizes = {
  md: "h-11 gap-2 px-4 text-2x",
  lg: "h-12 gap-3 px-6 text-2x",
  icon: "size-11",
};

export function button(
  variant: keyof typeof buttonVariants = "outline",
  size: keyof typeof buttonSizes = "md",
) {
  return cx(
    "inline-flex shrink-0 cursor-pointer items-center justify-center rounded-control font-bold whitespace-nowrap select-none disabled:cursor-not-allowed",
    focusRing,
    buttonVariants[variant],
    buttonSizes[size],
  );
}

/** Bordered chip for tags and counts. */
export const chip =
  "inline-flex h-7 items-center rounded-control border px-2.5 text-1x5 whitespace-nowrap";

/** Floating panels: select lists, menus and combobox suggestions. */
export const popup =
  "rounded-panel border border-line bg-surface py-1 text-content shadow-popover outline-none";

export const popupItem =
  "flex min-h-10 cursor-pointer items-center gap-3 px-4 text-2x outline-none select-none data-highlighted:bg-hover data-selected:text-mint";
