import {
  useCallback,
  useEffect,
  useRef,
  useState,
  useSyncExternalStore,
} from "react";
import type { RewindTimeline as Timeline } from "@evenfall/gba";
import { gameInputBlocked, player } from "./player";
import { QUICK_SLOTS, quickSlotForKey } from "./library";
import { draw } from "./media";
import { RewindTimeline } from "./rewind-timeline";

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
      player.latest = await player.emulator.loadState(quick.data);
      player.update({ cpu: player.latest, message: `Loaded slot ${slot}` });
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

/** Self-contained display and play controls; a game layout only places this child. */
export function ConsolePlayer({ onHelp }: { onHelp: () => void }) {
  const state = useSyncExternalStore(player.subscribe, player.snapshot);
  const [timeline, setTimeline] = useState<Timeline>();
  const displayRef = useRef<HTMLDivElement>(null);
  const screenRef = useCallback((canvas: HTMLCanvasElement | null) => {
    player.canvas = canvas ?? undefined;
    if (canvas && player.state.game)
      void player.emulator
        .frame()
        .then((pixels) => draw(canvas, pixels))
        .catch(player.error);
  }, []);
  useEffect(() => {
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
          void player
            .openRewindTimeline()
            .then(setTimeline)
            .catch(player.error);
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
  }, []);
  useEffect(() => {
    if (!state.rewindTimeline) setTimeline(undefined);
  }, [state.rewindTimeline]);
  const closeTimeline = useCallback(
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
  return (
    <section className="console-player" aria-label="Game player">
      <div className="game-viewport">
        <div className="game-display" id="game-display" ref={displayRef}>
          <canvas
            width="240"
            height="160"
            ref={screenRef}
            tabIndex={0}
            aria-label="Game screen. Arrows move, X and Z are A and B. Space pauses."
          />
          {!state.game && (
            <div className="screen-empty">
              <span className="pixel-mark">Z</span>
              <h1>Your next adventure.</h1>
              <p>Open a .gba or .zip to begin</p>
              <label className="screen-open">
                Open game
                <input
                  type="file"
                  aria-label="Open game"
                  accept=".gba,.zip"
                  disabled={!state.ready}
                  onChange={(event) => {
                    const file = event.target.files?.[0];
                    event.target.value = "";
                    if (file) void player.open(file).catch(player.error);
                  }}
                />
              </label>
              <small>Games and saves stay on this device.</small>
            </div>
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
          onClick={() =>
            void player
              .openRewindTimeline()
              .then(setTimeline)
              .catch(player.error)
          }
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
