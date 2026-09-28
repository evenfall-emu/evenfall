import { describe, expect, it } from "vite-plus/test";
import {
  drawSea,
  waveOffset,
  waveSource,
  type Sea,
} from "../src/pages/landing/waves";
import { ambient, burst, BURST_SECONDS } from "../src/pages/landing/bubbles";

const sea: Sea = { top: 100, bottom: 160, swell: 110 };

describe("landing waves", () => {
  it("moves water rows by whole art pixels and never the sky", () => {
    for (let t = 0; t < 20; t += 0.37) {
      expect(waveOffset(sea, 99, t)).toBe(0);
      expect(waveOffset(sea, 161, t)).toBe(0);
      for (let row = sea.top; row <= sea.bottom; row++)
        expect(Number.isInteger(waveOffset(sea, row, t))).toBe(true);
    }
  });

  it("sways rows near the viewer further than rows near the horizon", () => {
    const reach = (row: number) => {
      let most = 0;
      for (let t = 0; t < 30; t += 0.05)
        most = Math.max(most, Math.abs(waveOffset(sea, row, t)));
      return most;
    };
    expect(reach(102)).toBeLessThanOrEqual(1);
    expect(reach(158)).toBeGreaterThanOrEqual(2);
    expect(reach(158)).toBeLessThanOrEqual(3);
  });

  it("swells rows toward nearby sea rows, never outside the sea", () => {
    for (let t = 0; t < 20; t += 0.23)
      for (let row = sea.top; row <= sea.bottom; row++) {
        const source = waveSource(sea, row, t);
        expect(Number.isInteger(source)).toBe(true);
        expect(Math.abs(source - row)).toBeLessThanOrEqual(2);
        // Rows above the swell line are never resampled.
        if (row < sea.swell) expect(source).toBe(row);
        else {
          expect(source).toBeGreaterThanOrEqual(sea.swell);
          expect(source).toBeLessThanOrEqual(sea.bottom);
        }
      }
  });

  it("copies every water pixel exactly once per row, from its swell row, wrapped", () => {
    // A recording context: each drawImage copies source columns [sx, sx+sw)
    // of source row `sy` to destination columns at device row `dy`.
    const calls: {
      sx: number;
      sy: number;
      sw: number;
      dx: number;
      dy: number;
      dw: number;
    }[] = [];
    const context = {
      drawImage: (
        _: unknown,
        sx: number,
        sy: number,
        sw: number,
        _sh: number,
        dx: number,
        dy: number,
        dw: number,
      ) => calls.push({ sx, sy, sw, dx, dy, dw }),
    } as unknown as CanvasRenderingContext2D;
    const camera = { scale: 3, x: -30, y: 6 };
    const shore = (row: number) => (row < 110 ? 40 : 0);
    const seconds = 4.2;
    drawSea(
      context,
      {} as CanvasImageSource,
      { width: 200 },
      sea,
      camera,
      seconds,
      shore,
    );
    const byRow = new Map<number, typeof calls>();
    for (const call of calls) {
      const row = (call.dy - camera.y) / 3;
      byRow.set(row, [...(byRow.get(row) ?? []), call]);
    }
    // Every row that moves is drawn; rows that do not move are skipped.
    for (let row = sea.top; row <= sea.bottom; row++) {
      const moves =
        waveOffset(sea, row, seconds) !== 0 ||
        waveSource(sea, row, seconds) !== row;
      expect(byRow.has(row), `row ${row}`).toBe(moves);
    }
    for (const [row, parts] of byRow) {
      const start = shore(row);
      const shift = waveOffset(sea, row, seconds);
      const width = 200 - start;
      // Destination columns, back in art pixels, tile [start, 200) exactly.
      const covered = new Map<number, number>();
      for (const { sx, sy, sw, dx, dw } of parts) {
        expect(sy).toBe(waveSource(sea, row, seconds));
        expect(dw).toBe(sw * 3);
        for (let i = 0; i < sw; i++)
          covered.set((dx - camera.x) / 3 + i, sx + i);
      }
      expect(covered.size).toBe(width);
      for (const [column, source] of covered) {
        expect(column).toBeGreaterThanOrEqual(start);
        // The pixel at `column` comes from `column - shift`, wrapped.
        expect(source).toBe(
          start + ((((column - start - shift) % width) + width) % width),
        );
      }
    }
  });
});

describe("landing bubbles", () => {
  it("keep ambient streams within their bounds and whole pixels", () => {
    const stream = {
      x: 240,
      spread: 60,
      bottom: 90,
      top: 10,
      count: 8,
      rise: 6,
    };
    for (let t = 0; t < 15; t += 0.29)
      for (const bubble of ambient(stream, t, 3)) {
        expect(Number.isInteger(bubble.x)).toBe(true);
        expect(Number.isInteger(bubble.y)).toBe(true);
        expect(bubble.y).toBeGreaterThanOrEqual(10);
        expect(bubble.y).toBeLessThanOrEqual(90);
        // Spread ±30 plus a wobble of at most 1.
        expect(Math.abs(bubble.x - 240)).toBeLessThanOrEqual(31);
      }
  });

  it("spreads the burst across the middle and ends it on time", () => {
    const xs = new Set<number>();
    for (let t = 0; t < BURST_SECONDS; t += 0.1)
      for (const bubble of burst({ x: 240, y: 40 }, t)) xs.add(bubble.x);
    expect(Math.min(...xs)).toBeLessThan(225);
    expect(Math.max(...xs)).toBeGreaterThan(255);
    expect(burst({ x: 240, y: 40 }, BURST_SECONDS)).toEqual([]);
    expect(burst({ x: 240, y: 40 }, -0.1)).toEqual([]);
  });
});
