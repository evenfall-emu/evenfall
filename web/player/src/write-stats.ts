import type { ResumeWrite } from "./library";

/**
 * What one autosave wrote, in logical bytes: the values handed to IndexedDB,
 * before the browser's own encoding and storage overhead.
 */
export type WriteSample = ResumeWrite & {
  /** `Date.now()` when the autosave finished. */
  at: number;
  /** Whether the game was playing when the autosave started. */
  playing: boolean;
  /** The autosave's duration, compression and transaction included. */
  ms: number;
  /** The live state before compression. */
  stateRaw: number;
  /** The battery save, when it changed. */
  battery: number;
  /** The largest new rewind payload: an anchor when one was opened. */
  largestPayload: number;
  /** Rewind checkpoints and anchors the index names. */
  historyItems: number;
  /** Button-state changes since the previous autosave. */
  inputs: number;
};

/** An input log would store a cycle delta and a button mask per change. */
const INPUT_LOG_BYTES = 10;
const MAX_SAMPLES = 4096;

const sum = (samples: WriteSample[], pick: (s: WriteSample) => number) =>
  samples.reduce((total, sample) => total + pick(sample), 0);

/**
 * In-memory write statistics for deciding how the player should persist
 * progress. Read them in the console with `await evenfallWrites.report()`.
 */
export class WriteStats {
  samples: WriteSample[] = [];
  /** Button-state changes not yet attributed to an autosave. */
  inputs = 0;
  /** Autosaves that wrote nothing, by reason. */
  skipped: Record<string, number> = {};
  failures = 0;
  lastError?: string;
  private since = Date.now();

  record(sample: Omit<WriteSample, "at" | "inputs">) {
    this.samples.push({ ...sample, at: Date.now(), inputs: this.inputs });
    this.inputs = 0;
    if (this.samples.length > MAX_SAMPLES) this.samples.shift();
  }
  /**
   * "unchanged": paused, nothing ran. "no game", "no save lock" (another tab
   * owns the playthrough), "resume blocked" (a failed resume awaits a
   * choice), "busy" (a load, reset or rewind is running) and "game changed".
   */
  skip(reason: string) {
    this.skipped[reason] = (this.skipped[reason] ?? 0) + 1;
  }
  fail(error: unknown) {
    this.failures++;
    this.lastError = String(error);
  }
  reset() {
    this.samples = [];
    this.inputs = 0;
    this.skipped = {};
    this.failures = 0;
    this.lastError = undefined;
    this.since = Date.now();
  }
  /**
   * Totals and rates over the recorded autosaves, against what the earlier
   * layout (two raw states per save) and an input log would write.
   */
  async report() {
    const samples = this.samples;
    const seconds = Math.max(
      1,
      ((samples.at(-1)?.at ?? Date.now()) - (samples[0]?.at ?? this.since)) /
        1000,
    );
    const parts: Record<string, (s: WriteSample) => number> = {
      state: (s) => s.state,
      previous: (s) => s.previous,
      legacy: (s) => s.legacy,
      battery: (s) => s.battery,
      "rewind index": (s) => s.index,
      "rewind payloads": (s) => s.payloads,
    };
    const total = (s: WriteSample) =>
      Object.values(parts).reduce((bytes, pick) => bytes + pick(s), 0);
    // Before compression and the separate previous record: the latest and
    // previous raw states every save, and the rest. A legacy state was also
    // rewritten every save, which this leaves out.
    const earlier = (s: WriteSample) =>
      2 * s.stateRaw + s.battery + s.index + s.payloads;
    const row = (pick: (s: WriteSample) => number) => {
      const bytes = sum(samples, pick);
      return {
        "total KiB": +(bytes / 1024).toFixed(1),
        "KiB/save": +(bytes / 1024 / Math.max(1, samples.length)).toFixed(1),
        "KiB/s": +(bytes / 1024 / seconds).toFixed(2),
        "MiB/hour": +(((bytes / seconds) * 3600) / 1048576).toFixed(1),
      };
    };
    const table: Record<string, ReturnType<typeof row>> = {
      ...Object.fromEntries(
        Object.entries(parts).map(([name, pick]) => [name, row(pick)]),
      ),
      total: row(total),
      "earlier layout": row(earlier),
      "input log (estimate)": row((s) => s.inputs * INPUT_LOG_BYTES),
    };
    const states = samples.map((s) => s.stateRaw).filter(Boolean);
    const summary = {
      saves: samples.length,
      ...Object.fromEntries(
        Object.entries(this.skipped).map(([reason, n]) => [
          `skipped: ${reason}`,
          n,
        ]),
      ),
      failures: this.failures,
      ...(this.lastError ? { "last error": this.lastError } : {}),
      "seconds covered": Math.round(seconds),
      "saves while playing": samples.filter((s) => s.playing).length,
      "raw state KiB": +((states.at(-1) ?? 0) / 1024).toFixed(1),
      "compression ratio": +(
        sum(samples, (s) => s.state) /
        Math.max(
          1,
          sum(samples, (s) => s.stateRaw),
        )
      ).toFixed(3),
      "new payloads": sum(samples, (s) => s.payloadCount),
      "deleted payloads": sum(samples, (s) => s.deleted),
      "largest payload KiB": +(
        Math.max(0, ...samples.map((s) => s.largestPayload)) / 1024
      ).toFixed(1),
      "history items": samples.at(-1)?.historyItems ?? 0,
      "inputs/s": +(sum(samples, (s) => s.inputs) / seconds).toFixed(2),
      "mean save ms": +(
        sum(samples, (s) => s.ms) / Math.max(1, samples.length)
      ).toFixed(1),
      "slowest save ms": Math.round(Math.max(0, ...samples.map((s) => s.ms))),
      "origin storage MiB": +(
        ((await navigator.storage?.estimate?.())?.usage ?? 0) / 1048576
      ).toFixed(1),
    };
    console.table(summary);
    console.table(table);
    return { summary, table };
  }
}
