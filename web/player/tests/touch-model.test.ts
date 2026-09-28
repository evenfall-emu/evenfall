import { describe, expect, it } from "vite-plus/test";
import {
  defaultControlLayouts,
  dpadKeys,
  isGameButton,
  KEY,
} from "../src/touch-model";

// A touch at `degrees` clockwise from the right-hand axis (screen y points
// down), `distance` from the centre of a pad of radius 100.
const at = (degrees: number, distance = 80) => {
  const radians = (degrees * Math.PI) / 180;
  return dpadKeys(
    Math.cos(radians) * distance,
    Math.sin(radians) * distance,
    100,
  );
};

describe("D-pad directions", () => {
  it("maps screen directions to GBA keys, with y pointing down", () => {
    expect(at(0)).toBe(KEY.right);
    expect(at(90)).toBe(KEY.down);
    expect(at(180)).toBe(KEY.left);
    expect(at(270)).toBe(KEY.up);
  });

  it("presses nothing within a fifth of the radius of the centre", () => {
    expect(at(0, 19)).toBe(0);
    expect(at(0, 21)).toBe(KEY.right);
    expect(at(225, 19)).toBe(0);
  });

  it("gives each direction 60° and each diagonal 30°", () => {
    expect(at(29)).toBe(KEY.right);
    expect(at(31)).toBe(KEY.right | KEY.down);
    expect(at(59)).toBe(KEY.right | KEY.down);
    expect(at(61)).toBe(KEY.down);
    expect(at(-31)).toBe(KEY.right | KEY.up);
    expect(at(-29)).toBe(KEY.right);
    expect(at(151)).toBe(KEY.left);
    expect(at(149)).toBe(KEY.left | KEY.down);
    expect(at(225)).toBe(KEY.left | KEY.up);
  });

  it("keeps following a finger that drifts off the pad", () => {
    expect(at(0, 250)).toBe(KEY.right);
    expect(at(-45, 250)).toBe(KEY.right | KEY.up);
  });
});

describe("control layouts", () => {
  it("places every control inside its surface", () => {
    for (const layout of Object.values(defaultControlLayouts))
      for (const { x, y } of Object.values(layout)) {
        expect(x).toBeGreaterThan(0);
        expect(x).toBeLessThan(100);
        expect(y).toBeGreaterThan(0);
        expect(y).toBeLessThan(100);
      }
  });

  it("slides only between buttons that hold a key", () => {
    expect(isGameButton("a")).toBe(true);
    expect(isGameButton("start")).toBe(true);
    expect(isGameButton("dpad")).toBe(false);
    expect(isGameButton("fast")).toBe(false);
    expect(isGameButton(undefined)).toBe(false);
  });
});
