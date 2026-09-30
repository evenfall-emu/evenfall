import type { ReactNode } from "react";
import { Dialog as BaseDialog } from "@base-ui/react/dialog";
import { Icon } from "./icons";
import { button } from "./styles";

/**
 * A modal panel with a title row and a close button. While `busy`, the dialog
 * cannot be dismissed, so a pending action always settles inside it.
 */
export function Dialog({
  open,
  onOpenChange,
  title,
  subtitle,
  leading,
  busy = false,
  children,
}: {
  open: boolean;
  onOpenChange: (open: boolean) => void;
  title: ReactNode;
  subtitle?: ReactNode;
  /** Artwork or an icon before the title. */
  leading?: ReactNode;
  busy?: boolean;
  children: ReactNode;
}) {
  return (
    <BaseDialog.Root
      open={open}
      onOpenChange={(next, details) => {
        if (busy) details.cancel();
        else onOpenChange(next);
      }}
    >
      <BaseDialog.Portal>
        <BaseDialog.Backdrop
          data-ui
          className="fixed inset-0 z-40 bg-canvas/75 transition-opacity duration-150 data-ending-style:opacity-0 data-starting-style:opacity-0"
        />
        <BaseDialog.Popup
          data-ui
          aria-busy={busy}
          className="fixed inset-0 z-50 m-auto h-fit max-h-[calc(100dvh-32px)] w-[calc(100vw-32px)] max-w-[640px] overflow-y-auto rounded-panel border border-mint/60 bg-surface p-5 font-sans text-2x text-content shadow-dialog transition-opacity duration-150 data-ending-style:opacity-0 data-starting-style:opacity-0 sm:p-6"
        >
          <div className="flex items-start gap-4">
            {leading}
            <div className="min-w-0 flex-1 self-center">
              <BaseDialog.Title className="text-3x font-bold text-heading">
                {title}
              </BaseDialog.Title>
              {subtitle && <p className="mt-1">{subtitle}</p>}
            </div>
            <BaseDialog.Close
              aria-label="Close dialog"
              disabled={busy}
              className={`${button("ghost", "icon")} -mt-1 -mr-2`}
            >
              <Icon name="x" className="size-7" />
            </BaseDialog.Close>
          </div>
          {children}
        </BaseDialog.Popup>
      </BaseDialog.Portal>
    </BaseDialog.Root>
  );
}

/** Two equal-width actions at the foot of a dialog: dismissal, then the main action. */
export function DialogActions({ children }: { children: ReactNode }) {
  return <div className="mt-6 grid grid-cols-2 gap-4">{children}</div>;
}
