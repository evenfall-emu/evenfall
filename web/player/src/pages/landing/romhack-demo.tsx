import { useEffect, useRef, useState } from "react";
import cover1 from "../../assets/landing/covers/cover1.png";
import cover2 from "../../assets/landing/covers/cover2.png";
import cover3 from "../../assets/landing/covers/cover3.png";
import cover4 from "../../assets/landing/covers/cover4.png";
import cover5 from "../../assets/landing/covers/cover5.png";
import cover6 from "../../assets/landing/covers/cover6.png";
import { shown } from "./dither";
import { FRAME_SECONDS, watchSceneMotion } from "./motion";

// An illustration of the Library's romhacks, not a live one: one cartridge,
// the ROM the visitor brings, whose plain label Evenfall rewrites into a
// community adventure, checks, and plays, then lets go back to the original
// before the next one. Everything is drawn on one art grid and moves by
// whole art pixels; the covers are original art and the adventures invented.

/** The invented adventures, one per cover. Names fit a 16-column caption. */
const ADVENTURES = [
  { cover: cover1, name: "Banners of Vale" },
  { cover: cover2, name: "Crystal Hollow" },
  { cover: cover3, name: "Ember Lake" },
  { cover: cover4, name: "The Dune Gate" },
  { cover: cover5, name: "Aurora Keep" },
  { cover: cover6, name: "Stormlight Bay" },
];

/** Art pixels per CSS pixel. */
const SCALE = 3;
/** The cartridge, its label window inside it, and the canvas around both. */
const SHELL = { width: 66, height: 102 };
const LABEL = { x: 6, y: 10, width: 54, height: 80 };
const MARGIN = { x: 8, y: 7 };
const WIDTH = SHELL.width + MARGIN.x * 2;
const HEIGHT = SHELL.height + MARGIN.y * 2;

const INK = {
  outline: "#0b1a26",
  body: "#5d7488",
  light: "#8ea3b5",
  dark: "#3a4e60",
  recess: "#223747",
  halo: "#2b6a86",
  mint: "#8cdbbe",
  gold: "#fbde8f",
};

/** Seconds of each step of one adventure, in order. */
const STEPS = [
  { phase: "rom", seconds: 1.4 },
  { phase: "patch", seconds: 1.8 },
  { phase: "check", seconds: 0.7 },
  { phase: "play", seconds: 2.9 },
  { phase: "return", seconds: 0.8 },
] as const;
type Phase = (typeof STEPS)[number]["phase"];
const CYCLE = STEPS.reduce((sum, step) => sum + step.seconds, 0);

/** Who does each step: the visitor brings the ROM, Evenfall does the rest. */
function caption(phase: Phase, adventure: number) {
  switch (phase) {
    case "rom":
    case "return":
      return { step: "YOU BRING", detail: "Your own ROM", accent: false };
    case "patch":
      return {
        step: "EVENFALL PATCHES",
        detail: "On your device",
        accent: false,
      };
    case "check":
      return {
        step: "EVENFALL CHECKS",
        detail: "Exact release",
        accent: false,
      };
    case "play":
      return {
        step: "YOU PLAY",
        detail: ADVENTURES[adventure].name,
        accent: true,
      };
  }
}

/** Where `time` falls: the adventure, its step and the step's progress. */
function at(time: number) {
  const adventure = Math.floor(time / CYCLE) % ADVENTURES.length;
  let local = time % CYCLE;
  for (const step of STEPS) {
    if (local < step.seconds)
      return { adventure, phase: step.phase, progress: local / step.seconds };
    local -= step.seconds;
  }
  return { adventure, phase: "rom" as Phase, progress: 0 };
}

/** A stable pseudo-random number in [0, 1) for integer inputs. */
function noise(a: number, b: number, c: number) {
  let h =
    Math.imul(a, 374761393) ^
    Math.imul(b, 668265263) ^
    Math.imul(c, 2246822519);
  h = Math.imul(h ^ (h >>> 13), 1274126177);
  return ((h ^ (h >>> 16)) >>> 0) / 4294967296;
}

const rgb = (hex: string) =>
  [1, 3, 5].map((i) => parseInt(hex.slice(i, i + 2), 16));

/** Whether (x, y) is inside the cartridge: a top-right corner cut, softened. */
function inShell(x: number, y: number) {
  const { width, height } = SHELL;
  if (x < 0 || y < 0 || x >= width || y >= height) return false;
  if (x > width - 1 - Math.max(0, 6 - y)) return false;
  const corner = (x === 0 || x === width - 1) && (y === 0 || y === height - 1);
  return !corner;
}

/** The cartridge shell, lit from the top left, and its hard drop shadow. */
function paintShell() {
  const shell = document.createElement("canvas");
  shell.width = SHELL.width;
  shell.height = SHELL.height;
  const paint = shell.getContext("2d")!;
  const dot = (color: string, x: number, y: number, w = 1, h = 1) => {
    paint.fillStyle = color;
    paint.fillRect(x, y, w, h);
  };
  for (let y = 0; y < SHELL.height; y++)
    for (let x = 0; x < SHELL.width; x++) {
      if (!inShell(x, y)) continue;
      const edge =
        !inShell(x - 1, y) ||
        !inShell(x + 1, y) ||
        !inShell(x, y - 1) ||
        !inShell(x, y + 1);
      const lit = !inShell(x - 2, y) || !inShell(x, y - 2);
      const shade =
        !inShell(x + 2, y) || !inShell(x, y + 2) || !inShell(x + 2, y - 2);
      dot(
        edge ? INK.outline : lit ? INK.light : shade ? INK.dark : INK.body,
        x,
        y,
      );
    }
  // Grip ridges along the top, each a groove over a lit lip.
  for (const y of [3, 5]) {
    dot(INK.dark, 12, y, 38, 1);
    dot(INK.light, 12, y + 1, 38, 1);
  }
  // The label's recess: shaded at the top and left, lit at the bottom and
  // right, as a window pressed into the plastic.
  const r = {
    x: LABEL.x - 2,
    y: LABEL.y - 2,
    w: LABEL.width + 4,
    h: LABEL.height + 4,
  };
  dot(INK.recess, r.x, r.y, r.w, r.h);
  dot(INK.outline, r.x, r.y, r.w, 1);
  dot(INK.outline, r.x, r.y, 1, r.h);
  dot(INK.light, r.x, r.y + r.h, r.w + 1, 1);
  dot(INK.light, r.x + r.w, r.y, 1, r.h);
  // An embossed arrow pointing the way the cartridge goes in.
  [7, 5, 3, 1].forEach((w, i) => {
    dot(INK.dark, 33 - (w >> 1), 94 + i, w, 1);
    dot(INK.light, 33 + (w >> 1) + 1, 94 + i, 1, 1);
  });

  const shadow = document.createElement("canvas");
  shadow.width = SHELL.width;
  shadow.height = SHELL.height;
  const cast = shadow.getContext("2d")!;
  cast.fillStyle = INK.outline;
  for (let y = 0; y < SHELL.height; y++)
    for (let x = 0; x < SHELL.width; x++)
      if (inShell(x, y)) cast.fillRect(x, y, 1, 1);
  return { shell, shadow };
}

/**
 * The original ROM's label, in the cartridge's own slate: a quiet sun over
 * hills and a blank title band. Deliberately plain, so every adventure
 * written over it arrives in color.
 */
function paintBaseLabel() {
  const { width, height } = LABEL;
  const canvas = document.createElement("canvas");
  canvas.width = width;
  canvas.height = height;
  const paint = canvas.getContext("2d")!;
  const dot = (color: string, x: number, y: number, w = 1, h = 1) => {
    paint.fillStyle = color;
    paint.fillRect(x, y, w, h);
  };
  for (let y = 0; y < 62; y++)
    for (let x = 0; x < width; x++) {
      const sun = Math.hypot(x - 27, y - 30) < 8;
      const far = y > 42 + Math.round(4 * Math.sin(x / 6));
      const near = y > 50 + Math.round(3 * Math.sin(x / 4 + 2));
      const sky = shown(x, y, y / 62) ? "#4f6679" : "#3a4e60";
      dot(near ? "#1a2b38" : far ? "#2b3d4d" : sun ? "#a9b8c7" : sky, x, y);
    }
  dot("#1a2b38", 0, 62, width, height - 62);
  dot("#70879a", 12, 68, 30, 2);
  dot("#4f6679", 17, 73, 20, 1);
  dot("#4f6679", 21, 76, 12, 1);
  return paint.getImageData(0, 0, width, height).data;
}

async function loadCover(src: string) {
  const image = new Image();
  image.src = src;
  await image.decode();
  const canvas = document.createElement("canvas");
  canvas.width = LABEL.width;
  canvas.height = LABEL.height;
  const paint = canvas.getContext("2d")!;
  paint.drawImage(image, 0, 0);
  return paint.getImageData(0, 0, LABEL.width, LABEL.height).data;
}

/**
 * One cartridge, the ROM the visitor brings. Evenfall writes each community
 * adventure over its label row by row, flashes its check, plays it, and
 * dissolves back to the plain original before the next.
 */
export function RomhackDemo({ still = false }: { still?: boolean }) {
  const canvas = useRef<HTMLCanvasElement>(null);
  // A still view shows an adventure already patched and playing.
  const [shownCaption, setShownCaption] = useState(() =>
    caption(still ? "play" : "rom", still ? ADVENTURES.length - 1 : 0),
  );

  useEffect(() => {
    const context = canvas.current!.getContext("2d")!;
    const { shell, shadow } = paintShell();
    const base = paintBaseLabel();
    const label = context.createImageData(LABEL.width, LABEL.height);
    const [mint, gold] = [rgb(INK.mint), rgb("#e6fff5")];
    let covers: Uint8ClampedArray[] | undefined;
    // A still view stops late in the last adventure's play, past the glint.
    let time = still ? (ADVENTURES.length - 1) * CYCLE + 6.5 : 0;
    let cancelled = false;

    const draw = () => {
      const { adventure, phase, progress } = at(time);
      const frame = Math.floor(time / FRAME_SECONDS);
      const cover = covers?.[adventure] ?? base;
      // The cartridge drifts a pixel up and down, as everything here does.
      const bob = still ? 0 : Math.round(Math.sin((time / 3.2) * Math.PI * 2));
      const ox = MARGIN.x;
      const oy = MARGIN.y + bob;
      context.clearRect(0, 0, WIDTH, HEIGHT);

      // A dithered aura gathers while Evenfall works and fades as play starts.
      const glow =
        phase === "patch"
          ? progress
          : phase === "check"
            ? 1
            : phase === "play"
              ? Math.max(0, 1 - progress * 3)
              : 0;
      if (glow > 0) {
        context.fillStyle = INK.halo;
        for (let y = 0; y < HEIGHT; y++)
          for (let x = 0; x < WIDTH; x++) {
            const dx = Math.max(ox - x, x - (ox + SHELL.width - 1), 0);
            const dy = Math.max(oy - y, y - (oy + SHELL.height - 1), 0);
            // Rounded at the corners, thinning outward.
            const d = Math.hypot(dx, dy);
            if (d > 0 && shown(x, y, glow * (1 - d / 6)))
              context.fillRect(x, y, 1, 1);
          }
      }
      context.drawImage(shadow, ox + 2, oy + 2);
      context.drawImage(shell, ox, oy);

      // The label, pixel by pixel.
      const out = label.data;
      // The patch's write head runs down the label, trailing a band where
      // new bytes land among the old.
      const tail = 7;
      const head =
        phase === "patch"
          ? Math.floor(progress * (LABEL.height + tail + 1)) - 1
          : 0;
      const glint = phase === "play" ? progress * 260 - 30 : -99;
      for (let y = 0; y < LABEL.height; y++)
        for (let x = 0; x < LABEL.width; x++) {
          const i = (y * LABEL.width + x) * 4;
          let source = cover;
          let tint: number[] | undefined;
          let mix = 0;
          if (phase === "rom") source = base;
          else if (phase === "return")
            source = shown(x, y, progress) ? base : cover;
          else if (phase === "patch") {
            const d = y - head;
            if (d === 0) [tint, mix] = [gold, 1];
            else if (d === -1) [tint, mix] = [mint, 0.5];
            else if (d > 0) {
              const left = 1 - d / (tail + 1);
              const roll = noise(x, y, frame);
              if (roll < left * 0.3) [tint, mix] = [mint, 1];
              else if (roll >= left * 0.75) source = base;
            }
          } else if (phase === "play") {
            // A double glint crosses the label once as the game starts.
            const s = x + y * 0.6 - glint;
            if (Math.abs(s) < 1.5 || Math.abs(s + 5) < 0.6)
              [tint, mix] = [[255, 255, 255], 0.45];
          }
          for (let c = 0; c < 3; c++)
            out[i + c] = tint
              ? source[i + c] + (tint[c] - source[i + c]) * mix
              : source[i + c];
          out[i + 3] = 255;
        }
      context.putImageData(label, ox + LABEL.x, oy + LABEL.y);

      // The check: the label's frame blinks mint twice.
      if (phase === "check" && Math.floor(progress * 4) % 2 === 0) {
        context.strokeStyle = INK.mint;
        context.lineWidth = 1;
        context.strokeRect(
          ox + LABEL.x - 0.5,
          oy + LABEL.y - 0.5,
          LABEL.width + 1,
          LABEL.height + 1,
        );
      }

      // Sparks pop around the cartridge while it is patched: a dot, a
      // four-point star, a dot.
      if (phase === "patch" || phase === "check") {
        const slot = Math.floor(time / 0.2);
        for (const s of [slot, slot - 1]) {
          const age = time - s * 0.2;
          if (age > 0.4) continue;
          const side = noise(s, 1, 7);
          const along = noise(s, 2, 7);
          const [x, y] =
            side < 0.5
              ? [
                  side < 0.25 ? ox - 4 : ox + SHELL.width + 3,
                  oy + Math.floor(along * SHELL.height),
                ]
              : [
                  ox + Math.floor(along * SHELL.width),
                  side < 0.75 ? oy - 4 : oy + SHELL.height + 4,
                ];
          context.fillStyle = s % 2 ? INK.gold : INK.mint;
          context.fillRect(x, y, 1, 1);
          if (age > 0.13 && age < 0.27) {
            context.fillRect(x - 1, y, 3, 1);
            context.fillRect(x, y - 1, 1, 3);
          }
        }
      }
      setShownCaption((current) => {
        const next = caption(phase, adventure);
        return current.step === next.step && current.detail === next.detail
          ? current
          : next;
      });
    };

    void Promise.all(ADVENTURES.map(({ cover }) => loadCover(cover))).then(
      (loaded) => {
        if (cancelled) return;
        covers = loaded;
        draw();
      },
    );
    draw();
    if (still)
      return () => {
        cancelled = true;
      };

    let timer: ReturnType<typeof setInterval> | undefined;
    const stopWatching = watchSceneMotion((moving) => {
      clearInterval(timer);
      timer = moving
        ? setInterval(() => {
            time += FRAME_SECONDS;
            draw();
          }, FRAME_SECONDS * 1000)
        : undefined;
    });
    return () => {
      cancelled = true;
      clearInterval(timer);
      stopWatching();
    };
  }, [still]);

  return (
    <div className="flex flex-col items-center gap-3" aria-hidden="true">
      <canvas
        ref={canvas}
        width={WIDTH}
        height={HEIGHT}
        className="block [image-rendering:pixelated]"
        style={{ width: WIDTH * SCALE, height: HEIGHT * SCALE }}
      />
      <div className="text-center">
        <div className="text-1x5 tracking-[0.15em] text-[#6cc6f0]">
          {shownCaption.step}
        </div>
        <div
          className={
            shownCaption.accent
              ? "mt-1 text-2x text-peach"
              : "mt-1 text-2x text-heading"
          }
        >
          {shownCaption.detail}
        </div>
      </div>
    </div>
  );
}
