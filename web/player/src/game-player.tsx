import { useCallback, useState, useSyncExternalStore } from "react";
import { player } from "./player";
import { draw } from "./media";

const act = (fn: () => Promise<unknown>) => () => void fn().catch(player.error);

/**
 * The complete interactive emulator surface. Game layouts compose this as a
 * child; they never own the canvas, playback state, input, audio or fullscreen.
 */
export function GamePlayer({ onCapture }: { onCapture: () => void }) {
  const s = useSyncExternalStore(player.subscribe, player.snapshot);
  const [muted, setMuted] = useState(() => player.audio.muted);
  const screenRef = useCallback((node: HTMLCanvasElement | null) => {
    player.canvas = node ?? undefined;
    if (node && player.state.game)
      void player.emulator
        .frame()
        .then((pixels) => draw(node, pixels))
        .catch(player.error);
  }, []);

  return (
    <section className="console" aria-label="Game player">
      <div className="bezel" id="screen">
        <div
          className="performance"
          aria-label="Player performance"
          title={`Frame work includes emulation, recording, frame preparation, drawing and audio submission; excludes pacing, message transit and asynchronous compositor/audio work. P95 ${s.performance.frameP95Ms.toFixed(2)} ms · P99 ${s.performance.frameP99Ms.toFixed(2)} ms · Max ${s.performance.frameMaxMs.toFixed(2)} ms. WASM ${(s.performance.memoryBytes / 1048576).toFixed(1)} MiB allocated. Audio underruns ${s.performance.audioUnderruns ?? "unavailable"}, overruns ${s.performance.audioOverruns ?? "unavailable"}. Snapshot ${(s.recording?.snapshotMs ?? 0).toFixed(2)} ms, max ${(s.recording?.snapshotMaxMs ?? 0).toFixed(2)} ms.`}
        >
          <span>
            <b data-testid="fps">{s.performance.fps.toFixed(1)}</b> FPS
          </span>
          <span>
            <b data-testid="frame-ms">{s.performance.frameMs.toFixed(2)}</b> ms
            / frame
          </span>
        </div>
        <div className="screen-wrap">
          <canvas
            width="240"
            height="160"
            tabIndex={0}
            aria-label="Game screen. Arrow keys move, X and Z are A and B."
            ref={screenRef}
          />
          {!s.game && (
            <div className="empty">
              <div className="cartridge">▤</div>
              <h2>A little time to play.</h2>
              <p>Open or drop a game to get started.</p>
              <span>.gba / .zip</span>
            </div>
          )}
        </div>
        <details className="screen-help">
          <summary aria-label="Player help">?</summary>
          <div>
            <strong>Player controls</strong>
            <p>Arrows move · X/Z are A/B · A/S are L/R</p>
            <p>Enter starts · Backspace selects · hold Tab to boost</p>
          </div>
        </details>
      </div>
      <div className="transport">
        <button
          className="primary"
          disabled={!s.game || s.changing}
          onClick={act(() => player.play())}
        >
          {s.playing ? "Pause" : "Play"}
        </button>
        <button
          disabled={!s.game || s.playing || s.changing}
          onClick={act(() => player.frame())}
        >
          Frame +1
        </button>
        <button
          aria-pressed={muted}
          onClick={act(async () => {
            await player.audio.mute();
            setMuted(player.audio.muted);
          })}
        >
          {muted ? "Sound off" : "Sound on"}
        </button>
        <button
          onClick={act(async () => {
            await document.getElementById("screen")?.requestFullscreen();
          })}
        >
          Fullscreen
        </button>
        <button
          disabled={!s.game}
          onClick={act(async () => {
            await player.pause();
            onCapture();
          })}
        >
          Capture bug
        </button>
      </div>
      <div className={`speed-control ${s.boosting ? "boosting" : ""}`}>
        <label htmlFor="playback-speed">Speed</label>
        <span aria-hidden="true" title="Half speed">
          🐢
        </span>
        <input
          id="playback-speed"
          type="range"
          min="0.5"
          max="16"
          step="0.5"
          value={s.speed}
          disabled={!s.ready || s.changing}
          aria-valuetext={`${s.speed} times speed`}
          onChange={(event) => player.setSpeed(Number(event.target.value))}
        />
        <span aria-hidden="true" title="Sixteen times speed">
          🐇
        </span>
        <output aria-label="Effective speed">
          {s.boosting ? "16× BOOST" : `${s.speed}×`}
        </output>
      </div>
      <div className="touch-controls">
        {[
          ["↑", 64],
          ["←", 32],
          ["↓", 128],
          ["→", 16],
          ["B", 2],
          ["A", 1],
          ["L", 512],
          ["R", 256],
          ["Select", 4],
          ["Start", 8],
        ].map(([label, key]) => (
          <button
            key={label}
            onPointerDown={(event) => {
              event.preventDefault();
              event.currentTarget.setPointerCapture(event.pointerId);
              player.touch |= Number(key);
              player.keys();
            }}
            onPointerUp={() => {
              player.touch &= ~Number(key);
              player.keys();
            }}
            onPointerCancel={() => {
              player.touch &= ~Number(key);
              player.keys();
            }}
            onLostPointerCapture={() => {
              player.touch &= ~Number(key);
              player.keys();
            }}
          >
            {label}
          </button>
        ))}
      </div>
      <div className="key-guide" aria-label="Keyboard controls">
        <div className="key-guide-label">CONTROL DECK</div>
        <div className="key-bindings">
          <span>
            <kbd className="dpad-key">↑ ← ↓ →</kbd>
            <span>Move</span>
          </span>
          <span>
            <kbd>X</kbd>
            <span className="gba-button">A</span>
            <kbd>Z</kbd>
            <span className="gba-button">B</span>
          </span>
          <span>
            <kbd>A</kbd>
            <span className="shoulder-key">L</span>
            <kbd>S</kbd>
            <span className="shoulder-key">R</span>
          </span>
          <span>
            <kbd>Enter</kbd>
            <span>Start</span>
          </span>
          <span>
            <kbd>Backspace</kbd>
            <span>Select</span>
          </span>
          <span>
            <kbd>Tab</kbd>
            <span>Hold for 16×</span>
          </span>
        </div>
        <small>
          Tab boosts while the game screen is focused. Shift + Tab moves focus
          out.
        </small>
      </div>
      <p className="save-status">{s.saveMessage}</p>
    </section>
  );
}
