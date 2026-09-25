export interface PerformanceStats {
  fps: number;
  frameMs: number;
  frameP95Ms: number;
  frameP99Ms: number;
  frameMaxMs: number;
}

/** Count emulated work even when one presentation represents several frames. */
export class FrameMetrics {
  private frames = 0;
  private sampleCount = 0;
  private lastFrameCount = 1;
  private workMs = 0;
  private samples = new Float64Array(1024);
  private maximum = 0;
  private since: number;
  constructor(private readonly now = () => performance.now()) {
    this.since = now();
  }
  frame(workMs: number, emulatedFrames = 1) {
    this.samples[this.sampleCount++ % this.samples.length] = workMs;
    this.maximum = Math.max(this.maximum, workMs);
    this.frames += emulatedFrames;
    this.lastFrameCount = emulatedFrames;
    this.workMs += workMs * emulatedFrames;
  }
  presentation(workMs: number) {
    if (!this.frames) return;
    const index = (this.sampleCount - 1) % this.samples.length;
    this.samples[index] += workMs / this.lastFrameCount;
    this.workMs += workMs;
    this.maximum = Math.max(this.maximum, this.samples[index]);
  }
  reset() {
    this.frames = this.workMs = 0;
    this.sampleCount = 0;
    this.lastFrameCount = 1;
    this.maximum = 0;
    this.since = this.now();
  }
  sample(): PerformanceStats {
    const elapsed = this.now() - this.since;
    const sorted = this.samples
      .slice(0, Math.min(this.sampleCount, this.samples.length))
      .sort();
    const percentile = (p: number) =>
      sorted[Math.ceil(sorted.length * p) - 1] ?? 0;
    const result = {
      fps: elapsed > 0 ? (this.frames * 1000) / elapsed : 0,
      frameMs: this.frames ? this.workMs / this.frames : 0,
      frameP95Ms: percentile(0.95),
      frameP99Ms: percentile(0.99),
      frameMaxMs: this.maximum,
    };
    this.reset();
    return result;
  }
}
