import {
  useRef,
  useState,
  useSyncExternalStore,
  type ReactNode,
  type RefObject,
} from "react";
import { Dialog } from "@base-ui/react/dialog";
import type { HandheldHud } from "./games";
import { TouchControls } from "./touch-controls";
import { Icon } from "./ui/icons";
import { button, cx, focusRing } from "./ui/styles";

export type HandheldOrientation = "portrait" | "landscape";

// Phones: a narrow portrait or a short landscape page. Tablets and desktops
// keep the dashboard layouts.
const phone = {
  portrait: "(orientation: portrait) and (max-width: 650px)",
  landscape: "(orientation: landscape) and (max-height: 500px)",
} as const;

function watch(queries: readonly string[]) {
  return (change: () => void) => {
    const lists = queries.map((query) => matchMedia(query));
    lists.forEach((list) => list.addEventListener("change", change));
    return () =>
      lists.forEach((list) => list.removeEventListener("change", change));
  };
}
const watchPhone = watch(Object.values(phone));

/** The phone layout the page needs, or undefined for the dashboard. */
export function useHandheld(): HandheldOrientation | undefined {
  return useSyncExternalStore(watchPhone, () =>
    matchMedia(phone.portrait).matches
      ? "portrait"
      : matchMedia(phone.landscape).matches
        ? "landscape"
        : undefined,
  );
}

const watchTouch = watch(["(pointer: coarse)"]);
function watchGamepads(change: () => void) {
  addEventListener("gamepadconnected", change);
  addEventListener("gamepaddisconnected", change);
  return () => {
    removeEventListener("gamepadconnected", change);
    removeEventListener("gamepaddisconnected", change);
  };
}

/**
 * Whether on-screen controls start shown: a touch screen with no standard
 * gamepad connected. A gamepad connecting hides them; unplugging it brings
 * them back.
 */
export function useTouchByDefault() {
  const touch = useSyncExternalStore(
    watchTouch,
    () => matchMedia("(pointer: coarse)").matches,
  );
  const gamepad = useSyncExternalStore(watchGamepads, () =>
    [...(navigator.getGamepads?.() ?? [])].some(
      (pad) => pad?.connected && pad.mapping === "standard",
    ),
  );
  return touch && !gamepad;
}

const barButton = cx(button("ghost", "icon"), "pointer-events-auto");

/**
 * The phone player. Portrait stacks a one-line bar, the screen at full width,
 * the game's HUD line and the controls. Landscape gives the screen the full
 * height, puts the controls in the gutters and the bar's buttons in the top
 * corners. The game's panels open in a sheet over the controls, or inline in
 * portrait when the controls are hidden.
 */
export function HandheldPlayer({
  orientation,
  title,
  subtitle,
  playing,
  canPlay,
  onBack,
  onTogglePlay,
  onRewind,
  menu,
  menuOpen,
  onMenuOpenChange,
  hud,
  controls,
  container,
  children,
}: {
  orientation: HandheldOrientation;
  title: string;
  subtitle?: string;
  playing: boolean;
  canPlay: boolean;
  onBack: () => void;
  onTogglePlay: () => void;
  onRewind: () => void;
  /** The player's tools and options, shown from the bar's menu button. */
  menu: ReactNode;
  menuOpen: boolean;
  onMenuOpenChange: (open: boolean) => void;
  hud?: HandheldHud;
  controls: boolean;
  /** Where sheets render; the fullscreen element while there is one. */
  container?: RefObject<HTMLElement | null>;
  /** The screen. */
  children: ReactNode;
}) {
  const [hudOpen, setHudOpen] = useState(false);
  const [panel, setPanel] = useState(hud?.panels[0]?.id);
  const portrait = orientation === "portrait";
  const inline = portrait && !controls && hud;
  return (
    <main
      className="handheld relative min-h-0 flex-1 overflow-hidden bg-canvas"
      data-orientation={orientation}
      data-controls={controls || undefined}
    >
      <header
        data-ui
        className={cx(
          "handheld-bar z-10 flex items-center gap-1 font-sans text-content",
          portrait
            ? "h-11 border-b border-line px-1"
            : "pointer-events-none absolute inset-x-0 top-0 h-11 px-0.5",
        )}
      >
        <button
          className={barButton}
          aria-label="Back to Library"
          onClick={onBack}
        >
          <Icon name="chevronLeft" className="size-6" />
        </button>
        {portrait ? (
          <div className="min-w-0 flex-1 leading-none">
            <h1 className="truncate text-1x5 text-heading">{title}</h1>
            {subtitle && (
              <p className="mt-1 truncate text-[8px] leading-none text-muted">
                {subtitle}
              </p>
            )}
          </div>
        ) : (
          <>
            <h1 className="sr-only">{title}</h1>
            {hud && (
              <button
                className={cx(
                  "pointer-events-auto h-9 cursor-pointer rounded-control border border-control bg-surface/80 px-3 text-1x5 text-content hover:bg-hover active:bg-selected",
                  focusRing,
                )}
                aria-haspopup="dialog"
                onClick={() => {
                  setPanel(hud.panels[0]?.id);
                  setHudOpen(true);
                }}
              >
                {hud.panels[0]?.label ?? "Game"}
              </button>
            )}
            <span className="flex-1" />
          </>
        )}
        <button
          className={barButton}
          aria-label={playing ? "Pause" : "Play"}
          disabled={!canPlay}
          onClick={onTogglePlay}
        >
          <Icon name={playing ? "pause" : "play"} className="size-6" />
        </button>
        <button
          className={barButton}
          aria-label="Player menu"
          aria-haspopup="dialog"
          onClick={() => onMenuOpenChange(true)}
        >
          <Icon name="dotsVertical" className="size-6" />
        </button>
      </header>
      <div className="handheld-screen min-h-0 min-w-0">{children}</div>
      {hud && portrait && !inline && (
        <button
          className="handheld-hud h-12 w-full min-w-0 cursor-pointer border-0 border-b border-line bg-surface px-2 text-left"
          aria-label={`Show ${hud.panels.map((item) => item.label).join(" and ")}`}
          aria-haspopup="dialog"
          onClick={() => setHudOpen(true)}
        >
          {hud.summary}
        </button>
      )}
      {controls ? (
        <div
          className={cx(
            "handheld-controls",
            portrait ? "min-h-0" : "absolute inset-0",
          )}
        >
          <TouchControls orientation={orientation} onRewind={onRewind} />
        </div>
      ) : (
        inline && (
          <HudPanels
            hud={inline}
            panel={panel}
            onPanel={setPanel}
            className="min-h-0"
          />
        )
      )}
      {hud && (
        <Sheet
          open={hudOpen && !inline}
          onOpenChange={setHudOpen}
          side={portrait ? "bottom" : "left"}
          title="Game info"
          container={container}
        >
          {(initialFocus) => (
            <HudPanels
              hud={hud}
              panel={panel}
              onPanel={setPanel}
              tabRef={initialFocus}
              className="min-h-0 flex-1"
              trailing={
                <Dialog.Close
                  aria-label="Close game info"
                  className={button("ghost", "icon")}
                >
                  <Icon name="x" className="size-6" />
                </Dialog.Close>
              }
            />
          )}
        </Sheet>
      )}
      <Sheet
        open={menuOpen}
        onOpenChange={onMenuOpenChange}
        side={portrait ? "bottom" : "right"}
        title="Player menu"
        container={container}
      >
        {(initialFocus) => (
          <>
            <div
              data-ui
              className="flex h-11 shrink-0 items-center border-b border-line pl-4 font-sans text-2x text-heading"
            >
              <Dialog.Title className="flex-1">Player</Dialog.Title>
              <Dialog.Close
                ref={initialFocus}
                aria-label="Close menu"
                className={button("ghost", "icon")}
              >
                <Icon name="x" className="size-6" />
              </Dialog.Close>
            </div>
            <div className="handheld-menu min-h-0 flex-1 overflow-y-auto">
              {menu}
            </div>
          </>
        )}
      </Sheet>
    </main>
  );
}

/** The game's panels as tabs; every panel stays mounted to keep its state. */
function HudPanels({
  hud,
  panel,
  onPanel,
  tabRef,
  className,
  trailing,
}: {
  hud: HandheldHud;
  panel?: string;
  onPanel: (id: string) => void;
  tabRef?: RefObject<HTMLButtonElement | null>;
  className?: string;
  /** After the tabs, such as a close button. */
  trailing?: ReactNode;
}) {
  const current =
    hud.panels.find((candidate) => candidate.id === panel) ?? hud.panels[0];
  return (
    <div className={cx("flex flex-col", className)}>
      <div
        data-ui
        role="tablist"
        className="flex h-11 shrink-0 border-b border-line font-sans text-2x"
      >
        {hud.panels.map((candidate) => {
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
      {hud.panels.map((candidate) => (
        <div
          key={candidate.id}
          role="tabpanel"
          aria-label={candidate.label}
          hidden={candidate.id !== current?.id}
          className="handheld-panel min-h-0 flex-1 overflow-y-auto overscroll-contain p-2"
        >
          {candidate.content}
        </div>
      ))}
    </div>
  );
}

/**
 * A panel that leaves the screen visible: from the bottom, below the screen,
 * in portrait; from a side, over the controls' gutter, in landscape.
 */
function Sheet({
  open,
  onOpenChange,
  side,
  title,
  container,
  children,
}: {
  open: boolean;
  onOpenChange: (open: boolean) => void;
  side: "bottom" | "left" | "right";
  title: string;
  container?: RefObject<HTMLElement | null>;
  children: (initialFocus: RefObject<HTMLButtonElement | null>) => ReactNode;
}) {
  // Focus a tab or the close button, never a search field that would raise
  // the keyboard.
  const initialFocus = useRef<HTMLButtonElement>(null);
  return (
    <Dialog.Root open={open} onOpenChange={onOpenChange}>
      <Dialog.Portal container={container}>
        {/* Closing sheets ignore pointers, so the next tap reaches the game. */}
        <Dialog.Backdrop className="fixed inset-0 z-40 data-ending-style:pointer-events-none" />
        <Dialog.Popup
          aria-label={title}
          initialFocus={initialFocus}
          data-side={side}
          className={cx(
            "handheld-sheet fixed z-50 flex flex-col overflow-hidden border-line bg-canvas shadow-dialog transition-transform duration-150 data-ending-style:pointer-events-none",
            side === "bottom" &&
              "inset-x-0 bottom-0 top-[calc(44px+100vw/1.5)] border-t data-ending-style:translate-y-full data-starting-style:translate-y-full",
            side === "left" &&
              "inset-y-0 left-0 w-[min(440px,50vw)] border-r data-ending-style:-translate-x-full data-starting-style:-translate-x-full",
            side === "right" &&
              "inset-y-0 right-0 w-[min(440px,50vw)] border-l data-ending-style:translate-x-full data-starting-style:translate-x-full",
          )}
        >
          {children(initialFocus)}
        </Dialog.Popup>
      </Dialog.Portal>
    </Dialog.Root>
  );
}
