import { useId, useRef, useState, type ReactNode } from "react";
import { Tooltip } from "@base-ui/react/tooltip";

type TipProps = { label: string; children: ReactNode; content: ReactNode };

/** Descriptive, non-interactive content with hover, focus and tap access. */
export function InfoTip({ label, children, content }: TipProps) {
  const id = useId();
  const trigger = useRef<HTMLButtonElement>(null);
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
        ref={trigger}
        className="info-trigger"
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
          className="info-tooltip-positioner"
          positionMethod="fixed"
          anchor={() =>
            trigger.current?.closest("[data-tooltip-anchor]") ?? trigger.current
          }
          side="right"
          sideOffset={12}
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
            className="info-tooltip"
          >
            <Tooltip.Arrow className="info-tooltip-arrow" />
            <div className="info-tooltip-body">{content}</div>
          </Tooltip.Popup>
        </Tooltip.Positioner>
      </Tooltip.Portal>
    </Tooltip.Root>
  );
}
