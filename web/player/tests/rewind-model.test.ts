import { describe, expect, it } from "vitest";
import {
  formatRewindOffset,
  previousRewindCheckpoint,
  RewindFrameCache,
} from "../src/rewind-model";

describe("rewind timeline model", () => {
  it("selects the nearest checkpoint at least one emulated second earlier", () => {
    const second = 16_777_216n;
    const timeline = {
      timeline: "test",
      liveCycle: 10n * second,
      liveFrame: 600n,
      checkpoints: [
        { index: 0, steps: 3, cycle: 8n * second, frame: 477n, bytes: 1 },
        { index: 1, steps: 2, cycle: 9n * second, frame: 537n, bytes: 1 },
        {
          index: 2,
          steps: 1,
          cycle: 9n * second + second / 2n,
          frame: 567n,
          bytes: 1,
        },
      ],
    };
    expect(previousRewindCheckpoint(timeline)).toMatchObject({
      index: 1,
      steps: 2,
    });
    timeline.liveCycle--;
    expect(previousRewindCheckpoint(timeline)).toMatchObject({
      index: 0,
      steps: 3,
    });
    timeline.checkpoints.shift();
    expect(previousRewindCheckpoint(timeline)).toBeUndefined();
    expect(
      previousRewindCheckpoint({ ...timeline, checkpoints: [] }),
    ).toBeUndefined();
  });

  it("formats emulated-cycle offsets independently of playback speed", () => {
    const second = 16_777_216n;
    expect(formatRewindOffset(100n * second, 70n * second)).toBe("-0:30");
    expect(formatRewindOffset(100n * second, 39n * second)).toBe("-1:01");
    expect(formatRewindOffset(100n * second, 100n * second)).toBe("-0:00");
  });

  it("bounds frames with least-recently-used eviction and timeline keys", () => {
    const cache = new RewindFrameCache(2);
    const a = new Uint16Array([1]);
    const b = new Uint16Array([2]);
    const c = new Uint16Array([3]);
    cache.set("timeline-a:0", a);
    cache.set("timeline-a:1", b);
    expect(cache.get("timeline-a:0")).toBe(a);
    cache.set("timeline-a:2", c);
    expect(cache.get("timeline-a:1")).toBeUndefined();
    expect(cache.get("timeline-a:0")).toBe(a);
    expect(cache.get("timeline-b:0")).toBeUndefined();
    expect(cache.size).toBe(2);
    cache.clear();
    expect(cache.size).toBe(0);
  });
});
