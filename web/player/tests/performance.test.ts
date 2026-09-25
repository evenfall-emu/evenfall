import { describe, expect, it } from "vitest";
import { FrameMetrics } from "../src/performance";

describe("frame performance sampling", () => {
  it("counts omitted frames and amortizes one presentation across the batch", () => {
    let now = 0;
    const metrics = new FrameMetrics(() => now);
    metrics.frame(2, 16);
    metrics.presentation(3.2);
    now = 1000;
    expect(metrics.sample()).toMatchObject({ fps: 16, frameMs: 2.2, frameMaxMs: 2.2 });
  });
  it("includes deferred drawing work without counting another emulated frame", () => {
    let now = 0;
    const metrics = new FrameMetrics(() => now);
    metrics.frame(2);
    metrics.frame(2);
    metrics.presentation(1);
    now = 1000;
    expect(metrics.sample()).toMatchObject({
      fps: 2,
      frameMs: 2.5,
      frameMaxMs: 3,
    });
    metrics.presentation(1);
    expect(metrics.sample()).toMatchObject({ fps: 0, frameMs: 0 });
  });
  it("uses actual elapsed time and mean work, excluding pacing delays", () => {
    let now = 0;
    const metrics = new FrameMetrics(() => now);
    metrics.frame(4);
    metrics.frame(6);
    now = 2000;
    expect(metrics.sample()).toEqual({
      fps: 1,
      frameMs: 5,
      frameP95Ms: 6,
      frameP99Ms: 6,
      frameMaxMs: 6,
    });
    now = 3000;
    expect(metrics.sample()).toEqual({
      fps: 0,
      frameMs: 0,
      frameP95Ms: 0,
      frameP99Ms: 0,
      frameMaxMs: 0,
    });
  });
  it("discards frames from the previous play session on reset", () => {
    let now = 0;
    const metrics = new FrameMetrics(() => now);
    metrics.frame(10);
    now = 500;
    metrics.reset();
    now = 1000;
    metrics.frame(2);
    expect(metrics.sample()).toEqual({
      fps: 2,
      frameMs: 2,
      frameP95Ms: 2,
      frameP99Ms: 2,
      frameMaxMs: 2,
    });
  });
});

it("reports tail work and bounds percentile storage across a delayed sample", () => {
  const metrics = new FrameMetrics(() => 1000);
  for (let i = 1; i <= 100; i++) metrics.frame(i);
  expect(metrics.sample()).toMatchObject({
    frameP95Ms: 95,
    frameP99Ms: 99,
    frameMaxMs: 100,
  });
  metrics.frame(5000);
  for (let i = 0; i < 2000; i++) metrics.frame(2);
  expect(metrics.sample()).toMatchObject({
    frameP95Ms: 2,
    frameP99Ms: 2,
    frameMaxMs: 5000,
  });
});
