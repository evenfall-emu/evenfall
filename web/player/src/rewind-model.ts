import type { RewindTimeline } from "@evenfall/gba";

const MASTER_CLOCK = 16_777_216n;

export function previousRewindCheckpoint(timeline: RewindTimeline) {
  for (let index = timeline.checkpoints.length - 1; index >= 0; index--) {
    const checkpoint = timeline.checkpoints[index];
    if (timeline.liveCycle - checkpoint.cycle >= MASTER_CLOCK)
      return checkpoint;
  }
}

export function formatRewindOffset(live: bigint, checkpoint: bigint) {
  const delta = live > checkpoint ? live - checkpoint : 0n;
  const seconds = Number((delta + MASTER_CLOCK / 2n) / MASTER_CLOCK);
  const minutes = Math.floor(seconds / 60);
  return `-${minutes}:${String(seconds % 60).padStart(2, "0")}`;
}

export class RewindFrameCache {
  private frames = new Map<string, Uint16Array>();
  constructor(readonly capacity = 5) {}
  get(key: string) {
    const frame = this.frames.get(key);
    if (!frame) return;
    this.frames.delete(key);
    this.frames.set(key, frame);
    return frame;
  }
  set(key: string, frame: Uint16Array) {
    this.frames.delete(key);
    this.frames.set(key, frame);
    while (this.frames.size > this.capacity)
      this.frames.delete(this.frames.keys().next().value!);
  }
  clear() {
    this.frames.clear();
  }
  get size() {
    return this.frames.size;
  }
}
