// Rasterizes evenfall-icon.svg into the bitmaps the site needs beside it: a
// 32x32 tab icon for browsers that do not render SVG favicons, and the 180x180
// iOS home-screen icon. Run `npm run icons -w frontend` after editing the SVG.
import { existsSync } from "node:fs";
import { readFile, writeFile } from "node:fs/promises";
import { resolve } from "node:path";
import { chromium } from "@playwright/test";

const brand = resolve(import.meta.dirname, "../src/assets/brand");
const svg = await readFile(resolve(brand, "evenfall-icon.svg"), "utf8");
const icon = `data:image/svg+xml;base64,${Buffer.from(svg).toString("base64")}`;

// The icon is pixel art on a 32-unit grid: it is drawn at 1:1 and then scaled
// by whole pixels, so no edge falls between pixels.
const targets = [
  // Transparent, so it sits on light and dark tab bars alike.
  { file: "evenfall-icon-32.png", size: 32, scale: 1, background: null },
  // iOS composites transparency onto black and rounds the corners of the
  // square itself; the theme color keeps the icon in the centre 80%.
  { file: "evenfall-icon-180.png", size: 180, scale: 5, background: "#073659" },
];

const executablePath =
  process.env.PLAYWRIGHT_CHROMIUM_EXECUTABLE ??
  (existsSync("/usr/bin/chromium") ? "/usr/bin/chromium" : undefined);
const browser = await chromium.launch({
  executablePath,
  args: ["--no-sandbox"],
});
try {
  for (const target of targets) {
    const page = await browser.newPage({
      viewport: { width: target.size, height: target.size },
    });
    const png = await page.evaluate(
      async ({ icon, ...target }) => {
        const source = new Image();
        source.src = icon;
        await source.decode();
        const grid = document.createElement("canvas");
        grid.width = 32;
        grid.height = 32;
        // Drawn at the SVG's own 32-unit viewBox size, so every edge lands on a
        // pixel; an antialiased result would mean the art no longer is pixel art.
        const gridContext = grid.getContext("2d");
        gridContext.drawImage(source, 0, 0, 32, 32);
        const colors = new Set();
        const { data } = gridContext.getImageData(0, 0, 32, 32);
        for (let i = 0; i < data.length; i += 4)
          colors.add(data.slice(i, i + 4).join(","));
        if (colors.size !== 3)
          throw new Error(
            `evenfall-icon.svg is not pixel art: ${colors.size} colors`,
          );
        const canvas = document.createElement("canvas");
        canvas.width = target.size;
        canvas.height = target.size;
        const context = canvas.getContext("2d");
        if (target.background) {
          context.fillStyle = target.background;
          context.fillRect(0, 0, target.size, target.size);
        }
        const drawn = 32 * target.scale;
        const margin = (target.size - drawn) / 2;
        context.imageSmoothingEnabled = false;
        context.drawImage(grid, margin, margin, drawn, drawn);
        return canvas.toDataURL("image/png");
      },
      { ...target, icon },
    );
    await writeFile(
      resolve(brand, target.file),
      Buffer.from(png.slice("data:image/png;base64,".length), "base64"),
    );
    console.log(`${target.file}: ${target.size}x${target.size}`);
  }
} finally {
  await browser.close();
}
