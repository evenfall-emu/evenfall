/**
 * The site icon. `index.html` declares the brand mark for the tab, for browsers
 * that do not render SVG favicons, and for the iOS home screen; each
 * declaration names a file that exists at the size it claims.
 */
import { readFileSync } from "node:fs";
import { resolve } from "node:path";
import { describe, expect, it } from "vite-plus/test";

const frontend = resolve(import.meta.dirname, "..");
const tags = [
  ...readFileSync(resolve(frontend, "index.html"), "utf8").matchAll(
    /<link\b[^>]*>/g,
  ),
].map(([tag]) => tag);

const attribute = (tag: string, name: string) =>
  tag.match(new RegExp(`${name}="([^"]*)"`))?.[1];

const declared = tags
  .filter((tag) =>
    ["icon", "apple-touch-icon"].includes(attribute(tag, "rel") ?? ""),
  )
  .map((tag) => ({
    rel: attribute(tag, "rel")!,
    type: attribute(tag, "type"),
    sizes: attribute(tag, "sizes"),
    path: resolve(frontend, attribute(tag, "href")!.replace(/^\//, "")),
  }));

/** PNG width and height, read from the IHDR chunk after the signature. */
const png = (path: string) => {
  const header = readFileSync(path);
  return { width: header.readUInt32BE(16), height: header.readUInt32BE(20) };
};

describe("site icon", () => {
  it("declares the SVG brand mark for the tab", () => {
    const icon = declared.find(({ type }) => type === "image/svg+xml");
    expect(icon?.rel).toBe("icon");
    expect(icon?.path).toBe(
      resolve(frontend, "src/assets/brand/evenfall-icon.svg"),
    );
    expect(readFileSync(icon!.path, "utf8")).toContain("<svg");
  });

  it("covers browsers without SVG favicons and the iOS home screen", () => {
    const bitmaps = declared.filter(({ sizes }) => sizes !== undefined);
    expect(bitmaps.map(({ rel, sizes }) => [rel, sizes])).toEqual([
      ["icon", "32x32"],
      ["apple-touch-icon", "180x180"],
    ]);
    for (const { sizes, path } of bitmaps) {
      expect(png(path)).toEqual({
        width: Number(sizes!.split("x")[0]),
        height: Number(sizes!.split("x")[1]),
      });
    }
  });
});
