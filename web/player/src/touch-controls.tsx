import {
  useEffect,
  useRef,
  useState,
  useSyncExternalStore,
  type CSSProperties,
  type PointerEvent,
  type ReactNode,
} from "react";
import { quickState } from "./console-player";
import { player } from "./player";
import {
  defaultControlLayouts,
  dpadKeys,
  isGameButton,
  KEY,
  type ControlId,
  type ControlLayout,
} from "./touch-model";
import { Icon, type IconName } from "./ui/icons";
import { cx } from "./ui/styles";

/** One finger on the surface: the control it went down on and what it presses. */
interface Touch {
  readonly control: ControlId;
  readonly element: HTMLElement;
  keys: number;
}

const controlUnder = (x: number, y: number) =>
  document.elementFromPoint(x, y)?.closest<HTMLElement>("[data-control]")
    ?.dataset.control;

// Sizes follow the surface (a size container), so one layout fits phones of
// neighbouring shapes.
const sizes = {
  portrait:
    "[--dpad:clamp(120px,min(40cqw,34cqh),180px)] [--face:clamp(56px,min(19cqw,16cqh),84px)] [--shoulder:min(22cqw,96px)] [--action:min(10cqw,40px)]",
  landscape:
    "[--dpad:clamp(110px,min(17cqw,42cqh),170px)] [--face:clamp(52px,min(8.5cqw,20cqh),80px)] [--shoulder:min(10cqw,96px)] [--action:min(4.5cqw,40px)]",
};
const face =
  "border-2 border-control bg-raised/85 text-heading shadow-popover data-pressed:border-mint data-pressed:bg-mint data-pressed:text-on-accent";

/**
 * On-screen GBA controls for touch screens. Every finger is tracked on its
 * own: the D-pad follows a finger anywhere once touched, and a finger slides
 * between face, shoulder and Start/Select buttons. Fast-forward holds;
 * save, load and rewind act on a tap.
 */
export function TouchControls({
  orientation,
  onRewind,
  layout = defaultControlLayouts[orientation],
}: {
  orientation: "portrait" | "landscape";
  /** Opens the rewind timeline. */
  onRewind: () => void;
  layout?: ControlLayout;
}) {
  const state = useSyncExternalStore(player.subscribe, player.snapshot);
  const touches = useRef(new Map<number, Touch>());
  const sent = useRef({ keys: 0, fast: false });
  const [pressed, setPressed] = useState(sent.current);
  // Release everything this surface holds when it goes away.
  useEffect(
    () => () => {
      touches.current.clear();
      player.touch = 0;
      player.keys();
      if (sent.current.fast) player.boost(false);
    },
    [],
  );
  function sync() {
    let keys = 0;
    let fast = false;
    for (const touch of touches.current.values()) {
      keys |= touch.keys;
      fast ||= touch.control === "fast";
    }
    const previous = sent.current;
    // Compare with player.touch: releaseKeys (on play, blur or focus loss)
    // clears it under a finger that is still down.
    if (keys === player.touch && fast === previous.fast) return;
    if (keys & ~previous.keys || (fast && !previous.fast))
      navigator.vibrate?.(10);
    if (fast !== previous.fast) player.boost(fast);
    sent.current = { keys, fast };
    player.touch = keys;
    player.keys();
    setPressed(sent.current);
  }
  function track(touch: Touch, x: number, y: number) {
    if (touch.control === "dpad") {
      const box = touch.element.getBoundingClientRect();
      touch.keys = dpadKeys(
        x - (box.left + box.width / 2),
        y - (box.top + box.height / 2),
        box.width / 2,
      );
    } else if (isGameButton(touch.control)) {
      const under = controlUnder(x, y);
      touch.keys = isGameButton(under) ? KEY[under] : 0;
    }
  }
  function release(event: PointerEvent) {
    if (touches.current.delete(event.pointerId)) sync();
  }
  const place = (id: ControlId): CSSProperties => ({
    left: `${layout[id].x}%`,
    top: `${layout[id].y}%`,
  });
  const button = (
    id: ControlId,
    label: string,
    className: string,
    content: ReactNode,
    onClick?: () => void,
    disabled = false,
  ) => (
    <button
      type="button"
      tabIndex={-1}
      disabled={disabled}
      data-control={id}
      data-pressed={
        (isGameButton(id)
          ? pressed.keys & KEY[id]
          : id === "fast" && pressed.fast) || undefined
      }
      aria-label={label}
      onClick={onClick}
      className={cx(
        "pointer-events-auto absolute grid -translate-1/2 cursor-pointer touch-none place-items-center",
        className,
      )}
      style={place(id)}
    >
      {content}
    </button>
  );
  const action = (
    id: "save" | "load" | "rewind" | "fast",
    label: string,
    icon: IconName,
    onClick?: () => void,
    disabled = false,
  ) =>
    button(
      id,
      label,
      "size-(--action) rounded-control border border-line bg-surface/80 text-content active:bg-selected disabled:cursor-not-allowed disabled:text-muted/50 data-pressed:border-mint data-pressed:bg-mint data-pressed:text-on-accent",
      <Icon name={icon} className="size-5" />,
      onClick,
      disabled,
    );
  // As the quick bar: slots need a playthrough, rewind a game.
  const noSlots = !state.playthrough || state.changing;
  const quick = (restore: boolean) => () =>
    void quickState(1, restore).catch(player.error);
  const arm = (keys: number) => pressed.keys & keys;
  return (
    <div
      data-ui
      aria-label="Touch controls"
      role="group"
      className={cx(
        "pointer-events-none relative size-full font-sans select-none [container-type:size] [-webkit-touch-callout:none]",
        sizes[orientation],
      )}
      onContextMenu={(event) => event.preventDefault()}
      onPointerDown={(event) => {
        const element = (event.target as Element).closest<HTMLElement>(
          "[data-control]",
        );
        const control = element?.dataset.control as ControlId | undefined;
        if (!element || !control) return;
        if (control !== "dpad" && control !== "fast" && !isGameButton(control))
          return;
        event.preventDefault();
        element.setPointerCapture(event.pointerId);
        const touch: Touch = { control, element, keys: 0 };
        touches.current.set(event.pointerId, touch);
        track(touch, event.clientX, event.clientY);
        sync();
      }}
      onPointerMove={(event) => {
        const touch = touches.current.get(event.pointerId);
        if (!touch) return;
        track(touch, event.clientX, event.clientY);
        sync();
      }}
      onPointerUp={release}
      onPointerCancel={release}
      onLostPointerCapture={release}
    >
      <div
        data-control="dpad"
        role="group"
        aria-label="Directional pad"
        className="pointer-events-auto absolute size-(--dpad) -translate-1/2 touch-none"
        style={place("dpad")}
      >
        <svg viewBox="0 0 90 90" className="size-full" aria-hidden="true">
          <path
            d="M31 1h28v30h30v28H59v30H31V59H1V31h30Z"
            className="fill-raised/85 stroke-control"
            strokeWidth="2"
            strokeLinejoin="round"
          />
          {(
            [
              [KEY.up, "M32 2h26v28H32Z", "M45 9l7 9H38Z"],
              [KEY.down, "M32 60h26v28H32Z", "M45 81l7-9H38Z"],
              [KEY.left, "M2 32h28v26H2Z", "M9 45l9-7v14Z"],
              [KEY.right, "M60 32h28v26H60Z", "M81 45l-9-7v14Z"],
            ] as const
          ).map(([key, cell, arrow]) => (
            <g key={key}>
              {arm(key) ? <path d={cell} className="fill-mint" /> : null}
              <path
                d={arrow}
                className={arm(key) ? "fill-on-accent" : "fill-muted"}
              />
            </g>
          ))}
        </svg>
      </div>
      {button("b", "B", cx("size-(--face) rounded-full text-3x", face), "B")}
      {button("a", "A", cx("size-(--face) rounded-full text-3x", face), "A")}
      {button(
        "l",
        "L",
        cx("h-10 w-(--shoulder) rounded-full text-2x", face),
        "L",
      )}
      {button(
        "r",
        "R",
        cx("h-10 w-(--shoulder) rounded-full text-2x", face),
        "R",
      )}
      {button(
        "select",
        "Select",
        cx("h-8 w-19 rounded-full text-1x5", face),
        "SELECT",
      )}
      {button(
        "start",
        "Start",
        cx("h-8 w-19 rounded-full text-1x5", face),
        "START",
      )}
      {action("save", "Quick save", "saveState", quick(false), noSlots)}
      {action("load", "Quick load", "loadState", quick(true), noSlots)}
      {action(
        "rewind",
        "Rewind",
        "rewind",
        onRewind,
        !state.game || state.changing,
      )}
      {action("fast", "Fast-forward while held", "fastForward")}
    </div>
  );
}
