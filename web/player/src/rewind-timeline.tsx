import { useEffect, useRef, useState, type RefObject } from "react";
import { Dialog } from "@base-ui/react/dialog";
import type {
  RewindCheckpoint,
  RewindTimeline as Timeline,
} from "@evenfall/gba";
import { draw } from "./media";
import { player } from "./player";
import { formatRewindOffset, RewindFrameCache } from "./rewind-model";
import "./rewind-timeline.css";

function checkpointWindow(length: number, selected: number) {
  const count = Math.min(3, length);
  const start = Math.max(0, Math.min(selected - 1, length - count));
  return Array.from({ length: count }, (_, index) => start + index);
}

function Thumbnail({
  timeline,
  checkpoint,
  selected,
  cache,
  choose,
}: {
  timeline: Timeline;
  checkpoint: RewindCheckpoint;
  selected: boolean;
  cache: RewindFrameCache;
  choose: () => void;
}) {
  const canvas = useRef<HTMLCanvasElement>(null);
  const [loading, setLoading] = useState(true);
  useEffect(() => {
    let current = true;
    const key = `${timeline.timeline}:${checkpoint.index}`;
    const cached = cache.get(key);
    const request = cached
      ? Promise.resolve(cached)
      : player.rewindPreview(timeline.timeline, checkpoint.index);
    void request
      .then((pixels) => {
        if (!current || !pixels) return;
        cache.set(key, pixels);
        if (canvas.current) draw(canvas.current, pixels);
        setLoading(false);
      })
      .catch(player.error);
    return () => {
      current = false;
    };
  }, [cache, checkpoint.index, timeline.timeline]);
  return (
    <button
      className={`rewind-thumbnail${selected ? " selected" : ""}`}
      aria-label={`Checkpoint ${formatRewindOffset(timeline.liveCycle, checkpoint.cycle)}`}
      aria-pressed={selected}
      onClick={choose}
    >
      <canvas ref={canvas} width="240" height="160" />
      {loading && <span>Loading…</span>}
    </button>
  );
}

export function RewindTimeline({
  timeline,
  container,
  close,
}: {
  timeline: Timeline;
  container: RefObject<HTMLDivElement | null>;
  close: (checkpoint?: RewindCheckpoint) => void;
}) {
  const [selected, setSelected] = useState(timeline.checkpoints.length - 1);
  const cache = useRef(new RewindFrameCache()).current;
  const range = useRef<HTMLInputElement>(null);
  const checkpoint = timeline.checkpoints[selected];
  const visible = checkpointWindow(timeline.checkpoints.length, selected);
  useEffect(() => {
    return () => cache.clear();
  }, [cache]);
  return (
    <Dialog.Root
      open
      modal="trap-focus"
      disablePointerDismissal
      onOpenChange={(open) => {
        if (!open) close();
      }}
    >
      {/* Stay inside the game display for its positioning and fullscreen root. */}
      <Dialog.Portal container={container}>
        <Dialog.Popup
          render={<section />}
          className="rewind-timeline"
          aria-label="Rewind timeline"
          initialFocus={range}
          finalFocus={() => player.canvas ?? false}
        >
          <header>
            <div>
              <strong>Rewind</strong>
              <span>Choose an available checkpoint</span>
            </div>
            <output aria-live="polite">
              {formatRewindOffset(timeline.liveCycle, checkpoint.cycle)}
            </output>
          </header>
          <div className="rewind-thumbnails" aria-label="Nearby checkpoints">
            {visible.map((index) => (
              <Thumbnail
                key={timeline.checkpoints[index].index}
                timeline={timeline}
                checkpoint={timeline.checkpoints[index]}
                selected={index === selected}
                cache={cache}
                choose={() => setSelected(index)}
              />
            ))}
          </div>
          <input
            ref={range}
            aria-label="Rewind checkpoint"
            type="range"
            min="0"
            max={timeline.checkpoints.length - 1}
            step="1"
            value={selected}
            onChange={(event) => setSelected(Number(event.target.value))}
            onKeyDown={(event) => {
              if (event.key === "Enter") {
                event.preventDefault();
                close(checkpoint);
              }
            }}
          />
          <footer>
            <span>
              {timeline.checkpoints.length} checkpoint
              {timeline.checkpoints.length === 1 ? "" : "s"} available
            </span>
            <div>
              <Dialog.Close>
                Cancel <kbd>Esc</kbd>
              </Dialog.Close>
              <button
                className="rewind-commit"
                onClick={() => close(checkpoint)}
              >
                Rewind here
              </button>
            </div>
          </footer>
        </Dialog.Popup>
      </Dialog.Portal>
    </Dialog.Root>
  );
}
