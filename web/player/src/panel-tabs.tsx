import type { ReactNode, RefObject } from "react";
import type { GamePanel } from "./games";
import { cx } from "./ui/styles";

/**
 * A game layout's panels as tabs, for the phone player's sheet and the
 * two-column player's dock. Every panel stays mounted to keep its state.
 */
export function PanelTabs({
  panels,
  panel,
  onPanel,
  tabRef,
  className,
  panelClassName,
  trailing,
}: {
  panels: readonly GamePanel[];
  panel?: string;
  onPanel: (id: string) => void;
  tabRef?: RefObject<HTMLButtonElement | null>;
  className?: string;
  panelClassName?: string;
  /** After the tabs, such as a close button. */
  trailing?: ReactNode;
}) {
  const current =
    panels.find((candidate) => candidate.id === panel) ?? panels[0];
  return (
    <div className={cx("flex flex-col", className)}>
      <div
        data-ui
        role="tablist"
        className="flex h-11 shrink-0 border-b border-line font-sans text-2x"
      >
        {panels.map((candidate) => {
          const selected = candidate.id === current?.id;
          return (
            <button
              key={candidate.id}
              ref={selected ? tabRef : undefined}
              role="tab"
              aria-selected={selected}
              className={cx(
                "flex-1 cursor-pointer border-b-3 outline-none focus-visible:bg-hover",
                selected
                  ? "border-peach text-heading"
                  : "border-transparent text-muted",
              )}
              onClick={() => onPanel(candidate.id)}
            >
              {candidate.label}
            </button>
          );
        })}
        {trailing}
      </div>
      {panels.map((candidate) => (
        <div
          key={candidate.id}
          role="tabpanel"
          aria-label={candidate.label}
          hidden={candidate.id !== current?.id}
          className={cx(
            "min-h-0 flex-1 overflow-y-auto overscroll-contain",
            panelClassName,
          )}
        >
          {candidate.content}
        </div>
      ))}
    </div>
  );
}
