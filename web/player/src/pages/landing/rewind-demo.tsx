import { useEffect, useState } from "react";
import forest from "../../assets/landing/scenes/forest.png";
import ruin from "../../assets/landing/scenes/ruin.png";
import village from "../../assets/landing/scenes/village.png";
import { cx } from "../../ui/styles";

// An illustration of the player's rewind timeline, not a live one: three
// checkpoints of an original game world, a slider sweeping back through
// history, and the offset it lands on. The scenes are original art.

/** Stops the demo visits, newest first: its thumbnail and offset label. */
const STOPS = [
  { scene: ruin, offset: "−0:42", position: 0.94 },
  { scene: forest, offset: "−3:18:05", position: 0.62 },
  { scene: village, offset: "−2 days", position: 0.18 },
] as const;
const SCENES = [village, forest, ruin];
/** Seconds each stop holds before the slider moves on. */
const HOLD = 2.6;

export function RewindDemo({ still = false }: { still?: boolean }) {
  const [stop, setStop] = useState(1);
  useEffect(() => {
    if (still) return;
    const timer = setInterval(
      () => setStop((current) => (current + 1) % STOPS.length),
      HOLD * 1000,
    );
    return () => clearInterval(timer);
  }, [still]);
  const current = STOPS[stop];

  return (
    <div
      className="w-fit border-2 border-[#2b6a86] bg-[#0b1a26]/90 p-3 shadow-[0_0_0_2px_#0b1a26] sm:p-4"
      aria-hidden="true"
    >
      <div className="flex items-center justify-between gap-4">
        <div className="text-2x tracking-[0.15em] text-[#6cc6f0]">REWIND</div>
        <div className="border border-[#2b6a86] px-3 py-1 text-2x text-heading tabular-nums">
          {current.offset}
        </div>
      </div>
      {/* Thumbnails of the 96 × 114 art, 1x on phones and 2x from the sm
          breakpoint. The height fixes the scale; where three full frames do
          not fit, each narrows and object-cover crops its sides, keeping the
          art at the whole multiple set by the height. */}
      <div className="mt-4 flex gap-2">
        {SCENES.map((scene) => (
          <img
            key={scene}
            src={scene}
            alt=""
            width={96}
            height={114}
            className={cx(
              "block h-[114px] w-[96px] min-w-0 border-2 box-content object-cover [image-rendering:pixelated] sm:h-[228px] sm:w-[192px]",
              scene === current.scene
                ? "border-[#6cc6f0]"
                : "border-[#1d3a4a] opacity-60",
            )}
          />
        ))}
      </div>
      <div className="relative mt-4 h-3 border border-[#2b6a86] bg-[#10202c]">
        <div
          className="absolute inset-y-0 left-0 bg-[#6cc6f0] motion-safe:transition-[width] motion-safe:duration-700 motion-safe:ease-in-out"
          style={{ width: `${current.position * 100}%` }}
        />
        <div
          className="absolute top-1/2 size-4 -translate-x-1/2 -translate-y-1/2 bg-[#6cc6f0] motion-safe:transition-[left] motion-safe:duration-700 motion-safe:ease-in-out"
          style={{ left: `${current.position * 100}%` }}
        />
      </div>
      <div className="mt-3 flex justify-between text-1x5 text-muted">
        <span>2 days ago</span>
        <span>now</span>
      </div>
    </div>
  );
}
