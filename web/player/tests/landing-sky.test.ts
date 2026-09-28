import { describe, expect, it } from "vitest";
import {
  cloudShift,
  glowStep,
  type Clouds,
  type Glow,
} from "../src/pages/landing/sky";

const clouds: Clouds = {
  top: 100,
  bottom: 160,
  left: () => 0,
  reach: 6,
  cycle: 120,
};

describe("landing sky", () => {
  it("drifts cloud rows by whole pixels, never outside the band or past reach", () => {
    for (let t = 0; t < 240; t += 1.7) {
      expect(cloudShift(clouds, 99, t)).toBe(0);
      expect(cloudShift(clouds, 160, t)).toBe(0);
      for (let row = clouds.top; row < clouds.bottom; row++) {
        const shift = cloudShift(clouds, row, t);
        expect(Number.isInteger(shift)).toBe(true);
        expect(Math.abs(shift)).toBeLessThanOrEqual(6);
      }
    }
  });

  it("drifts most in the middle of the band and barely at its edges", () => {
    const most = (row: number) => {
      let reach = 0;
      for (let t = 0; t < 120; t += 0.5)
        reach = Math.max(reach, Math.abs(cloudShift(clouds, row, t)));
      return reach;
    };
    expect(most(130)).toBe(6);
    expect(most(101)).toBeLessThanOrEqual(1);
    expect(most(159)).toBeLessThanOrEqual(1);
  });

  it("pulses a light through its ramp and back to its painted color", () => {
    const beacon: Glow = {
      pixels: [[0, 0]],
      ramp: ["a", "b", "c", "d"],
      period: 4,
    };
    const steps = new Set<number>();
    for (let t = 0; t < 4; t += 0.05) steps.add(glowStep(beacon, t));
    expect([...steps].sort()).toEqual([0, 1, 2, 3]);
    // At rest at the start of each pulse, brightest halfway through.
    expect(glowStep(beacon, 0)).toBe(0);
    expect(glowStep(beacon, 2)).toBe(3);
    expect(glowStep(beacon, 4)).toBe(0);
  });
});
