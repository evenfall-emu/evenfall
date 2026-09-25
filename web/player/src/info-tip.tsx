import { useId, useRef, useState, type ReactNode } from "react";
import { Tooltip } from "@base-ui/react/tooltip";

/** A status label with hover, focus and tap access to its full text. */
export function StatusTip({
  label,
  children,
  className = "",
}: {
  label: string;
  children: ReactNode;
  className?: string;
}) {
  const id = useId();
  const pressed = useRef(false);
  const [open, setOpen] = useState(false);
  return (
    <Tooltip.Root
      open={open}
      triggerId={id}
      onOpenChange={(next, details) => {
        // A hover-close queued before a tap must not dismiss its description.
        if (pressed.current && details.reason === "trigger-hover") return;
        if (!next) pressed.current = false;
        setOpen(next);
      }}
    >
      <Tooltip.Trigger
        id={id}
        className={`library-status ${className}`}
        aria-label={label}
        aria-describedby={open ? `${id}-description` : undefined}
        delay={0}
        closeDelay={100}
        closeOnClick={false}
        // Base UI ignores touch hover; these buttons only reveal descriptions.
        onClick={() => {
          pressed.current = true;
          setOpen(true);
        }}
        onFocus={() => setOpen(true)}
      >
        {children}
      </Tooltip.Trigger>
      <Tooltip.Portal>
        <Tooltip.Positioner
          className="library-tooltip-positioner"
          positionMethod="fixed"
          side="top"
          sideOffset={3}
          collisionPadding={12}
          collisionAvoidance={{
            side: "flip",
            align: "shift",
            fallbackAxisSide: "start",
          }}
          arrowPadding={16}
        >
          <Tooltip.Popup
            id={`${id}-description`}
            role="tooltip"
            hidden={!open}
            className="library-tooltip"
          >
            {label}
          </Tooltip.Popup>
        </Tooltip.Positioner>
      </Tooltip.Portal>
    </Tooltip.Root>
  );
}
