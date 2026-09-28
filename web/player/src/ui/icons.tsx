import type { ReactNode, SVGProps } from "react";

// 24-unit stroke icons with square caps, drawn to sit on the pixel type.
const glyphs = {
  alert: (
    <>
      <circle cx="12" cy="12" r="9" />
      <path d="M12 7v6m0 3v1" />
    </>
  ),
  alertFilled: (
    <>
      <circle cx="12" cy="12" r="10" fill="currentColor" stroke="none" />
      <path className="stroke-on-accent" d="M12 6.5v7m0 3v1" />
    </>
  ),
  check: <path d="m4 12.5 5 5L20 6.5" />,
  chevronDown: <path d="m6 9 6 6 6-6" />,
  cloud: (
    <path d="M7 19a5 5 0 0 1-.6-9.96A6.5 6.5 0 0 1 18.8 10.1 4.5 4.5 0 0 1 17.5 19Z" />
  ),
  copy: (
    <>
      <path d="M8 8h11v13H8Z" />
      <path d="M16 8V3H5v13h3" />
    </>
  ),
  dotsVertical: (
    <g fill="currentColor" stroke="none">
      <rect x="10.5" y="3.5" width="3" height="3" />
      <rect x="10.5" y="10.5" width="3" height="3" />
      <rect x="10.5" y="17.5" width="3" height="3" />
    </g>
  ),
  download: <path d="M12 3v12m-5-5 5 5 5-5M4 16v4h16v-4" />,
  externalLink: (
    <>
      <path d="M14 4h6v6m0-6-9 9" />
      <path d="M18 14v6H4V6h6" />
    </>
  ),
  fileExport: (
    <>
      <path d="M12 21H5V3h9l5 5v4" />
      <path d="M14 3v5h5m-2 14v-7m-3 3 3-3 3 3" />
    </>
  ),
  filePlus: (
    <>
      <path d="M12 21H5V3h9l5 5v5" />
      <path d="M14 3v5h5m-1 7v6m-3-3h6" />
    </>
  ),
  history: (
    <>
      <path d="M3.5 13A8.5 8.5 0 1 0 6 6L3 9" />
      <path d="M3 4v5h5m4-2v5l3 3" />
    </>
  ),
  info: (
    <>
      <circle cx="12" cy="12" r="9" />
      <path d="M12 11v6m0-10v1" />
    </>
  ),
  monitor: (
    <>
      <path d="M3 4h18v12H3Z" />
      <path d="M12 16v4m-4 0h8" />
    </>
  ),
  plus: <path d="M12 4v16M4 12h16" />,
  releaseNotes: (
    <>
      <path d="M11 21H5V3h9l5 5v3" />
      <path d="M14 3v5h5M8 12h5m-5 4h2" />
      <circle cx="17.5" cy="17.5" r="4" />
      <path d="M15.5 17.5h4" />
    </>
  ),
  search: (
    <>
      <circle cx="10.5" cy="10.5" r="6.5" />
      <path d="m15.5 15.5 5 5" />
    </>
  ),
  x: <path d="m6 6 12 12M18 6 6 18" />,
} satisfies Record<string, ReactNode>;

export type IconName = keyof typeof glyphs;

/** Decorative by default; give the surrounding control its accessible name. */
export function Icon({
  name,
  ...props
}: { name: IconName } & SVGProps<SVGSVGElement>) {
  return (
    <svg
      viewBox="0 0 24 24"
      fill="none"
      stroke="currentColor"
      strokeWidth={2}
      strokeLinecap="square"
      aria-hidden="true"
      focusable="false"
      {...props}
    >
      {glyphs[name]}
    </svg>
  );
}

// Pixel-art glyphs, one character per pixel ("." is empty). Other characters
// can take their own color through `inks`.
const pixelGlyphs = {
  avatar: [
    ".......######.......",
    ".....##......##.....",
    "...##..........##...",
    "..#..............#..",
    "..#.....oooo.....#..",
    ".#.....oooooo.....#.",
    ".#.....oooooo.....#.",
    "#......oooooo......#",
    "#......oooooo......#",
    "#.......oooo.......#",
    "#..................#",
    "#.....oooooooo.....#",
    "#....oooooooooo....#",
    ".#..oooooooooooo..#.",
    ".#..oooooooooooo..#.",
    "..#oooooooooooooo#..",
    "..#oooooooooooooo#..",
    "...##oooooooooo##...",
    ".....##oooooo##.....",
    ".......######.......",
  ],
  back: ["...##", "..##.", ".##..", "##...", ".##..", "..##.", "...##"],
  book: [
    ".###......###.",
    ".#####..#####.",
    ".#####..#####.",
    ".#####..#####.",
    ".#####..#####.",
    ".#####..#####.",
    ".#####..#####.",
    ".#####..#####.",
    "..####..####..",
    "....##..##....",
  ],
  gear: [
    ".....##.....",
    ".##.####.##.",
    ".##########.",
    "..###..###..",
    ".###....###.",
    "###......###",
    "###......###",
    ".###....###.",
    "..###..###..",
    ".##########.",
    ".##.####.##.",
    ".....##.....",
  ],
  play: [
    "#.....",
    "##....",
    "###...",
    "####..",
    "#####.",
    "######",
    "#####.",
    "####..",
    "###...",
    "##....",
    "#.....",
  ],
  playSmall: ["#...", "##..", "###.", "####", "###.", "##..", "#..."],
} satisfies Record<string, readonly string[]>;

export type PixelIconName = keyof typeof pixelGlyphs;

/** One path per ink, merging horizontal runs of pixels into rectangles. */
const pixelPaths = Object.fromEntries(
  Object.entries(pixelGlyphs).map(([name, rows]) => {
    const paths: Record<string, string> = {};
    rows.forEach((row, y) => {
      for (let x = 0; x < row.length;) {
        const ink = row[x];
        let end = x + 1;
        while (row[end] === ink) end++;
        if (ink !== ".")
          paths[ink] = `${paths[ink] ?? ""}M${x} ${y}h${end - x}v1h${x - end}z`;
        x = end;
      }
    });
    return [name, paths];
  }),
) as Record<PixelIconName, Record<string, string>>;

/**
 * A pixel-art glyph drawn at a whole-number `scale`, so each pixel covers the
 * same number of screen pixels. Every ink uses `currentColor` unless `inks`
 * gives it a class, such as `{ "#": "text-mint" }`.
 */
export function PixelIcon({
  name,
  scale = 2,
  inks = {},
  ...props
}: {
  name: PixelIconName;
  scale?: number;
  inks?: Record<string, string>;
} & SVGProps<SVGSVGElement>) {
  const rows = pixelGlyphs[name];
  const width = rows[0].length;
  return (
    <svg
      viewBox={`0 0 ${width} ${rows.length}`}
      width={width * scale}
      height={rows.length * scale}
      shapeRendering="crispEdges"
      aria-hidden="true"
      focusable="false"
      {...props}
    >
      {Object.entries(pixelPaths[name]).map(([ink, d]) => (
        <path key={ink} d={d} fill="currentColor" className={inks[ink]} />
      ))}
    </svg>
  );
}
