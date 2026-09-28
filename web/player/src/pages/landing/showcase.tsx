import { useEffect, useState, type ReactNode } from "react";
import cover1 from "../../assets/landing/covers/cover1.png";
import cover2 from "../../assets/landing/covers/cover2.png";
import cover3 from "../../assets/landing/covers/cover3.png";
import cover4 from "../../assets/landing/covers/cover4.png";
import cover5 from "../../assets/landing/covers/cover5.png";
import cover6 from "../../assets/landing/covers/cover6.png";
import bat from "../../assets/landing/creatures/bat.png";
import fox from "../../assets/landing/creatures/fox.png";
import golem from "../../assets/landing/creatures/golem.png";
import owl from "../../assets/landing/creatures/owl.png";
import seahorse from "../../assets/landing/creatures/seahorse.png";
import turtle from "../../assets/landing/creatures/turtle.png";
import cloud from "../../assets/landing/icons/cloud.png";
import laptop from "../../assets/landing/icons/laptop.png";
import link from "../../assets/landing/icons/link.png";
import phone from "../../assets/landing/icons/phone.png";
import { cx } from "../../ui/styles";

// Illustrations for the landing's product story. Every image is original art
// on the landing's pixel grid (see assets/landing/README.md); no game's
// screenshots, characters or names appear. They show what the player does,
// not live data.

/** A pixel-art image drawn at a whole-number multiple of its art size. */
function Sprite({
  src,
  width,
  height,
  scale,
  className,
}: {
  src: string;
  width: number;
  height: number;
  scale: number;
  className?: string;
}) {
  return (
    <img
      src={src}
      alt=""
      width={width * scale}
      height={height * scale}
      className={cx("block [image-rendering:pixelated]", className)}
    />
  );
}

/** One original "base game" cartridge, drawn in the brand's pixel style. */
const CARTRIDGE = [
  "..cccccccccccccc..",
  ".cllllllllllllllc.",
  ".cl............lc.",
  ".cl.mmmmmmmmmm.lc.",
  ".cl.mppppppppm.lc.",
  ".cl.mpyyyyyypm.lc.",
  ".cl.mpyyyyyypm.lc.",
  ".cl.mtttttttm..lc.",
  ".cl.mmmmmmmmmm.lc.",
  ".cl............lc.",
  ".cllllllllllllllc.",
  ".cllllllllllllllc.",
  ".cl.l.l.l.l.l.llc.",
  ".cccccccccccccccc.",
];

function PixelRows({
  rows,
  inks,
  scale,
}: {
  rows: readonly string[];
  inks: Record<string, string>;
  scale: number;
}) {
  const rects: ReactNode[] = [];
  rows.forEach((row, y) => {
    for (let x = 0; x < row.length; ) {
      const ink = row[x];
      let end = x + 1;
      while (row[end] === ink) end++;
      if (ink !== ".")
        rects.push(
          <rect
            key={`${x},${y}`}
            x={x}
            y={y}
            width={end - x}
            height={1}
            fill={inks[ink]}
          />,
        );
      x = end;
    }
  });
  return (
    <svg
      viewBox={`0 0 ${rows[0].length} ${rows.length}`}
      width={rows[0].length * scale}
      height={rows.length * scale}
      shapeRendering="crispEdges"
      aria-hidden="true"
    >
      {rects}
    </svg>
  );
}

const COVERS = [cover1, cover2, cover3, cover4, cover5, cover6];

/**
 * One base game on the left; a fanned stack of community adventures on the
 * right, cycling slowly so a different one comes forward. The covers step
 * by whole pixels and never rotate, so their art pixels stay square.
 */
export function RomhackFan({ still = false }: { still?: boolean }) {
  const [front, setFront] = useState(0);
  useEffect(() => {
    if (still) return;
    const timer = setInterval(
      () => setFront((current) => (current + 1) % COVERS.length),
      2200,
    );
    return () => clearInterval(timer);
  }, [still]);
  // The stack spans the front cover plus `step` pixels per cover behind it.
  // The ROM sits beside the stack from 480px, where the row (394 px plus
  // its gaps) fits every column: 432 px at 480, 456 px per lg half.
  const behind = 4;
  const step = 28;
  return (
    <div
      className="flex flex-col items-center gap-4 min-[480px]:flex-row sm:gap-6"
      aria-hidden="true"
    >
      <div className="flex flex-col items-center gap-3">
        <PixelRows
          rows={CARTRIDGE}
          inks={{
            c: "#70879a",
            l: "#223747",
            m: "#3a4e60",
            p: "#f5aa82",
            y: "#fbde8f",
            t: "#034579",
          }}
          scale={5}
        />
        <span className="text-1x5 whitespace-nowrap text-muted">Your ROM</span>
      </div>
      <PixelRows
        rows={["#....", "##...", "#####", "##...", "#...."]}
        inks={{ "#": "#8cdbbe" }}
        scale={4}
      />
      <div
        className="relative"
        style={{ width: 166 + behind * step, height: 250 + behind * 8 }}
      >
        {COVERS.map((cover, i) => {
          // Position in the stack, counted from the front cover.
          const slot = (i - front + COVERS.length) % COVERS.length;
          const shown = slot <= behind;
          return (
            <div
              key={cover}
              className="absolute bottom-0 left-0 border-2 border-[#101c2a] shadow-[4px_4px_0_#0b1a26] motion-safe:transition-all motion-safe:duration-700"
              style={{
                zIndex: COVERS.length - slot,
                opacity: shown ? 1 - slot * 0.14 : 0,
                transform: `translate(${Math.min(slot, behind) * step}px, ${-Math.min(slot, behind) * 8}px)`,
              }}
            >
              <Sprite src={cover} width={54} height={80} scale={3} />
            </div>
          );
        })}
      </div>
    </div>
  );
}

/** A laptop and a phone sharing one run, joined by a cloud and a link. */
export function AccountDevices() {
  return (
    <div className="flex flex-col items-center gap-6" aria-hidden="true">
      {/* Sprites keep whole-multiple sizes (never shrink); on phones they
          drop to 1x, where a 2x row would not fit. */}
      <div className="flex items-end gap-3 sm:gap-8 [&_img]:shrink-0">
        <Sprite
          src={laptop}
          width={96}
          height={80}
          scale={2}
          className="max-sm:h-[80px] max-sm:w-[96px]"
        />
        <div className="flex flex-col items-center gap-2 pb-3 sm:pb-6">
          <Sprite
            src={cloud}
            width={40}
            height={44}
            scale={2}
            className="max-sm:h-[44px] max-sm:w-[40px]"
          />
          <PixelRows rows={["#.#.#.#.#"]} inks={{ "#": "#8cdbbe" }} scale={4} />
        </div>
        <Sprite
          src={phone}
          width={46}
          height={74}
          scale={2}
          className="max-sm:h-[74px] max-sm:w-[46px]"
        />
      </div>
      <div className="flex items-center gap-4 border border-line bg-surface/90 px-4 py-3">
        <Sprite src={link} width={60} height={50} scale={1} />
        <div className="text-left">
          <div className="text-2x text-heading">Share state</div>
          <div className="mt-1 text-1x5 text-muted">
            A link to your exact moment
          </div>
        </div>
      </div>
    </div>
  );
}

// Invented creatures and stats for the game-panel illustration.
const TEAM = [
  { name: "Pyrox", sprite: fox, width: 33, hp: 1, type: "#f5aa82" },
  { name: "Mosshell", sprite: turtle, width: 34, hp: 0.85, type: "#8cdbbe" },
  { name: "Glimbat", sprite: bat, width: 41, hp: 0.6, type: "#a9c7f0" },
  { name: "Voltowl", sprite: owl, width: 28, hp: 1, type: "#ebc477" },
  { name: "Coralyn", sprite: seahorse, width: 27, hp: 0.4, type: "#ff9696" },
  { name: "Cairnox", sprite: golem, width: 35, hp: 0.9, type: "#a9b8c7" },
];

function HpBar({ value }: { value: number }) {
  return (
    <div className="h-2 w-full bg-[#10202c]">
      <div
        className={cx(
          "h-full",
          value > 0.5 ? "bg-mint" : value > 0.25 ? "bg-warning" : "bg-danger",
        )}
        style={{ width: `${value * 100}%` }}
      />
    </div>
  );
}

/**
 * A miniature of the player's game panels: a team read live from memory
 * beside the game screen, the selected member's details below.
 */
export function GamePanels({ still = false }: { still?: boolean }) {
  const [selected, setSelected] = useState(0);
  useEffect(() => {
    if (still) return;
    const timer = setInterval(
      () => setSelected((current) => (current + 1) % TEAM.length),
      1800,
    );
    return () => clearInterval(timer);
  }, [still]);
  const member = TEAM[selected];
  // The team sits beside the selected card from sm and above it on phones.
  // The card puts its 3x sprite beside the name where the column is wide,
  // and above it in the 456 px lg half, where the two would not fit.
  return (
    <div
      className="grid w-full max-w-xl grid-cols-1 gap-3 border-2 border-[#2b6a86] bg-[#0b1a26]/90 p-3 text-1x5 sm:grid-cols-[auto_minmax(0,1fr)]"
      aria-hidden="true"
    >
      <div className="flex flex-col gap-2">
        <div className="tracking-[0.12em] text-[#6cc6f0]">YOUR TEAM</div>
        <div className="grid grid-cols-2 gap-1.5">
          {TEAM.map((creature, i) => (
            <div
              key={creature.name}
              className={cx(
                "flex w-28 flex-col items-center justify-end gap-1.5 border px-1.5 pt-1.5 pb-1.5 max-sm:w-auto",
                i === selected
                  ? "border-[#6cc6f0] bg-[#16303f]"
                  : "border-[#1d3a4a] bg-[#10202c]",
              )}
            >
              {/* Whole multiples of the art size keep the pixels square:
                  team sprites at 2x, the selected one at 3x. */}
              <Sprite
                src={creature.sprite}
                width={creature.width}
                height={40}
                scale={2}
              />
              <HpBar value={creature.hp} />
            </div>
          ))}
        </div>
      </div>
      <div className="flex flex-col gap-2">
        <div className="tracking-[0.12em] text-[#6cc6f0]">SELECTED</div>
        <div className="flex flex-1 flex-col gap-2 border border-[#1d3a4a] bg-[#10202c] p-2">
          <div className="flex items-center gap-3 lg:flex-col lg:items-start">
            <Sprite
              src={member.sprite}
              width={member.width}
              height={40}
              scale={3}
              className="shrink-0"
            />
            <div>
              <div className="text-2x text-heading">{member.name}</div>
              <div className="mt-1 text-muted">Lv 20</div>
              <div
                className="mt-1 inline-block h-2 w-10"
                style={{ background: member.type }}
              />
            </div>
          </div>
          <HpBar value={member.hp} />
          <div className="grid grid-cols-2 gap-x-4 gap-y-1 text-content">
            {["HP", "Atk", "Def", "SpA", "SpD", "Spe"].map((stat, i) => (
              <div key={stat} className="flex justify-between gap-2">
                <span className="text-muted">{stat}</span>
                <span className="tabular-nums">
                  {40 + ((selected * 7 + i * 13) % 35)}
                </span>
              </div>
            ))}
          </div>
        </div>
      </div>
    </div>
  );
}

/** Speed as a row of pixel bars and the multiplier they reach. */
export function SpeedMeter({ still = false }: { still?: boolean }) {
  const [level, setLevel] = useState(still ? 5 : 0);
  useEffect(() => {
    if (still) return;
    const timer = setInterval(
      () => setLevel((current) => (current + 1) % 7),
      700,
    );
    return () => clearInterval(timer);
  }, [still]);
  const speeds = ["1×", "2×", "4×", "8×", "16×"];
  const lit = Math.min(level, speeds.length);
  return (
    <div
      className="w-full max-w-sm border-2 border-[#2b6a86] bg-[#0b1a26]/90 p-4"
      aria-hidden="true"
    >
      <div className="flex items-end justify-between gap-2">
        {speeds.map((speed, i) => (
          <div key={speed} className="flex flex-1 flex-col items-center gap-2">
            <div
              className={cx("w-full", i < lit ? "bg-peach" : "bg-[#1d3a4a]")}
              style={{ height: `${16 + i * 16}px` }}
            />
            <span
              className={cx(
                "text-1x5",
                i < lit ? "text-heading" : "text-muted",
              )}
            >
              {speed}
            </span>
          </div>
        ))}
      </div>
    </div>
  );
}
