import {
  useCallback,
  useEffect,
  useLayoutEffect,
  useRef,
  useState,
  useSyncExternalStore,
} from "react";
import type { RewindTimeline as Timeline } from "@evenfall/gba";
import { gameInputBlocked, player } from "./player";
import { QUICK_SLOTS, quickSlotForKey } from "./library";
import { EmptyScreen } from "./empty-screen";
import { RewindTimeline } from "./rewind-timeline";
import { requireCurrentState } from "./state-format";

/**
 * One owned operation: slots are ROM- and playthrough-specific and never
 * overwrite battery saves. Saving commits locally first; with cloud backup on,
 * it also queues that slot's replaceable upload. Loading never waits for sync.
 */
export async function quickState(slot: number, restore: boolean) {
  const game = player.state.game;
  const ref = player.activeRef;
  if (!game || !ref || player.state.changing || slot < 1 || slot > QUICK_SLOTS)
    return;
  const resume = player.state.playing;
  let queued = false;
  player.update({ changing: true });
  try {
    await player.pause(true);
    if (restore) {
      const quick = await player.library.getQuickSlot(ref, slot);
      if (!quick) throw new Error(`Quick slot ${slot} is empty.`);
      requireCurrentState(quick.data, `Quick slot ${slot}`);
      player.latest = await player.emulator.loadState(quick.data);
      player.update({ message: `Loaded slot ${slot}` });
      await player.checkpointState();
    } else {
      ({ queued } = await player.library.saveQuickSlot(
        ref,
        slot,
        await player.emulator.saveState(),
      ));
      player.update({
        message: queued
          ? `Saved slot ${slot} · backing up`
          : `Saved slot ${slot} on this device`,
      });
      await player.refreshQuickSlots();
    }
  } finally {
    player.update({ changing: false });
  }
  if (queued) void player.cloud.flush();
  if (resume) await player.play();
}

/**
 * The open rewind timeline, if any. Owned above the display so on-screen
 * controls outside it can open the timeline too.
 */
export function useRewindTimeline() {
  const rewindTimeline = useSyncExternalStore(
    player.subscribe,
    () => player.state.rewindTimeline,
  );
  const [timeline, setTimeline] = useState<Timeline>();
  const [selected, setSelected] = useState(0);
  useEffect(() => {
    if (!rewindTimeline) setTimeline(undefined);
  }, [rewindTimeline]);
  const open = useCallback(() => {
    void player
      .openRewindTimeline()
      .then((next) => {
        if (!next) return;
        setSelected(next.checkpoints.length - 1);
        setTimeline(next);
      })
      .catch(player.error);
  }, []);
  const close = useCallback(
    (checkpoint?: Timeline["checkpoints"][number]) => {
      const open = timeline;
      setTimeline(undefined);
      void player
        .closeRewindTimeline(
          open && checkpoint
            ? {
                timeline: open.timeline,
                index: checkpoint.index,
                steps: checkpoint.steps,
              }
            : undefined,
        )
        .catch(player.error);
    },
    [timeline],
  );
  return { timeline, selected, setSelected, open, close };
}
export type RewindTimelineControl = ReturnType<typeof useRewindTimeline>;

/** Self-contained display and play controls; a game layout only places this child. */
export function ConsolePlayer({
  onHelp,
  rewind,
}: {
  onHelp: () => void;
  rewind: RewindTimelineControl;
}) {
  const state = useSyncExternalStore(player.subscribe, player.snapshot);
  const {
    timeline,
    selected,
    setSelected,
    open: openTimeline,
    close: closeTimeline,
  } = rewind;
  const displayRef = useRef<HTMLDivElement>(null);
  // A new key mounts a fresh canvas, which a failed WebGPU context can't reuse.
  const [canvasKey, setCanvasKey] = useState(0);
  const screenRef = useCallback((canvas: HTMLCanvasElement | null) => {
    const screen = player.attachScreen(canvas, () =>
      setCanvasKey((key) => key + 1),
    );
    if (screen && player.state.game)
      void player.emulator
        .frame()
        .then((pixels) => screen.show(pixels))
        .catch(player.error);
  }, []);
  // Attached during the commit that shows the Player, so a key pressed as the
  // page appears is never missed.
  useLayoutEffect(() => {
    let rewindHeld = false;
    const releaseRewind = () => {
      rewindHeld = false;
    };
    function key(event: KeyboardEvent) {
      if (event.type === "keyup") {
        if (event.code === "KeyR") releaseRewind();
        return;
      }
      if (
        event.defaultPrevented ||
        event.altKey ||
        event.metaKey ||
        gameInputBlocked(event.target)
      )
        return;
      const slot = quickSlotForKey(event.code);
      if (slot && (event.ctrlKey || event.shiftKey)) {
        event.preventDefault();
        void quickState(slot, event.ctrlKey).catch(player.error);
        return;
      }
      if (event.code === "KeyR") {
        event.preventDefault();
        if (event.repeat || rewindHeld || event.ctrlKey) return;
        rewindHeld = true;
        if (event.shiftKey) {
          openTimeline();
        } else {
          void player.rewindOnce().catch(player.error);
        }
        return;
      }
      if (event.ctrlKey || event.shiftKey) return;
      if (
        event.code === "Space" &&
        !(event.target as HTMLElement)?.closest("button,a,summary")
      ) {
        event.preventDefault();
        void player.play().catch(player.error);
      }
    }
    window.addEventListener("keydown", key);
    window.addEventListener("keyup", key);
    window.addEventListener("blur", releaseRewind);
    return () => {
      window.removeEventListener("keydown", key);
      window.removeEventListener("keyup", key);
      window.removeEventListener("blur", releaseRewind);
      player.releaseKeys();
    };
  }, [openTimeline]);
  return (
    <section className="console-player" aria-label="Game player">
      <div className="game-viewport">
        <div className="game-display" id="game-display" ref={displayRef}>
          <canvas
            key={canvasKey}
            width="240"
            height="160"
            ref={screenRef}
            tabIndex={0}
            aria-label="Game screen. Arrows move, X and Z are A and B. Space pauses."
          />
          {!state.game && (
            <EmptyScreen
              ready={state.ready}
              onOpen={(file) => void player.open(file).catch(player.error)}
            />
          )}
          {state.game &&
            !state.playing &&
            !state.changing &&
            !state.rewinding &&
            !state.rewindTimeline &&
            !state.resumeFailed &&
            !timeline && (
              <div className="pause-overlay">
                <div className="pause-card">
                  <span className="pause-eyebrow">GAME ON HOLD</span>
                  <strong>PAUSED</strong>
                  <button
                    onClick={() => void player.play().catch(player.error)}
                  >
                    <span aria-hidden="true">▶</span> Resume game
                  </button>
                  <span className="pause-hint">
                    or press <kbd>Space</kbd> to continue
                  </span>
                </div>
              </div>
            )}
          {timeline && (
            <RewindTimeline
              timeline={timeline}
              selected={selected}
              onSelect={setSelected}
              container={displayRef}
              close={closeTimeline}
            />
          )}
          <button
            className="player-help"
            aria-label="Player help"
            aria-haspopup="dialog"
            disabled={state.changing}
            onClick={onHelp}
          >
            ?
          </button>
        </div>
      </div>
      <div className="quick-bar">
        <button
          disabled={!state.playthrough || state.changing}
          onClick={() => void quickState(1, false).catch(player.error)}
          title="Save slot 1. Shift + 1–9 or 0 selects slots 1–10."
        >
          <span>Quick save</span>
          <kbd>Shift + 1</kbd>
        </button>
        <button
          disabled={!state.playthrough || state.changing}
          onClick={() => void quickState(1, true).catch(player.error)}
          title="Load slot 1. Ctrl + 1–9 or 0 selects slots 1–10."
        >
          <span>Quick load</span>
          <kbd>Ctrl + 1</kbd>
        </button>
        <button
          disabled={!state.game || state.changing}
          onClick={openTimeline}
          title="Choose a rewind checkpoint. R steps back; Shift + R opens history."
        >
          <span>Rewind</span>
          <kbd>Shift + R</kbd>
        </button>
        <button
          disabled={!state.game || state.changing || state.resumeFailed}
          onClick={() => void player.reset().catch(player.error)}
          title="Restart this playthrough using its current in-game save. Progress is kept."
        >
          <span>Reset</span>
          <kbd>Restart</kbd>
        </button>
        <span className="save-indicator" title={state.saveMessage}>
          {state.game
            ? /failed|unavailable|Could not/i.test(state.saveMessage)
              ? "Save unavailable · export"
              : state.saveMessage === "Save stored locally."
                ? "Saved locally"
                : player.releaseLock
                  ? "Autosave active"
                  : "Read-only save"
            : "Local player"}
        </span>
      </div>
    </section>
  );
}

export function PlayerHelp({
  controls,
  toggleControls,
}: {
  controls: boolean;
  toggleControls: () => void;
}) {
  return (
    <div className="controls-guide">
      <p className="muted">
        Keyboard controls · your next adventure starts here.
      </p>
      <div className="controls-primary">
        <section aria-label="Movement controls">
          <h3>Move</h3>
          <div className="controls-arrows" aria-label="Arrow keys">
            <kbd>↑</kbd>
            <kbd>←</kbd>
            <kbd>↓</kbd>
            <kbd>→</kbd>
          </div>
          <p>Arrow keys</p>
        </section>
        <section aria-label="Action buttons">
          <h3>Action buttons</h3>
          <div className="controls-action">
            <kbd>X</kbd>
            <span>→</span>
            <strong>A</strong>
          </div>
          <div className="controls-action">
            <kbd>Z</kbd>
            <span>→</span>
            <strong>B</strong>
          </div>
        </section>
      </div>
      <dl className="controls-grid">
        <div>
          <dt>
            <kbd>Enter</kbd>
          </dt>
          <dd>Start</dd>
        </div>
        <div>
          <dt>
            <kbd>Backspace</kbd>
          </dt>
          <dd>Select</dd>
        </div>
        <div>
          <dt>
            <kbd>A</kbd>
          </dt>
          <dd>L shoulder</dd>
        </div>
        <div>
          <dt>
            <kbd>S</kbd>
          </dt>
          <dd>R shoulder</dd>
        </div>
      </dl>
      <section className="controls-saves">
        <h3>Quick save / load</h3>
        <dl className="controls-grid">
          <div>
            <dt>
              <kbd>Shift</kbd> + <kbd>1–9, 0</kbd>
            </dt>
            <dd>Quick save</dd>
          </div>
          <div>
            <dt>
              <kbd>Ctrl</kbd> + <kbd>1–9, 0</kbd>
            </dt>
            <dd>Quick load</dd>
          </div>
        </dl>
        <p className="muted">
          Ten slots per playthrough; 0 is slot 10. Saving is instant and works
          offline. With cloud backups on, slots also sync to your other devices.
        </p>
      </section>
      <dl className="controls-playback">
        <div>
          <dt>
            <kbd>Tab</kbd>
          </dt>
          <dd>Hold for 16× speed</dd>
        </div>
        <div>
          <dt>
            <kbd>R</kbd>
          </dt>
          <dd>Rewind at least one second</dd>
        </div>
        <div>
          <dt>
            <kbd>Shift</kbd> + <kbd>R</kbd>
          </dt>
          <dd>Open rewind timeline</dd>
        </div>
        <div>
          <dt>
            <kbd>Space</kbd>
          </dt>
          <dd>Play / pause</dd>
        </div>
      </dl>
      <p className="controls-note">
        Game keys work outside text fields and dialogs. Shift + Tab navigates
        away. Rewind uses available checkpoints, not arbitrary frames. Escape
        cancels the timeline without changing the game; rewind isn’t a permanent
        save.
      </p>
      <button
        className="controls-touch"
        aria-pressed={controls}
        onClick={toggleControls}
      >
        {controls ? "Hide" : "Show"} touch controls
      </button>
    </div>
  );
}
