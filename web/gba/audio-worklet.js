// A bounded stereo ring. The audio callback never drives emulated time.
class EvenfallAudio extends AudioWorkletProcessor {
  constructor(options) {
    super();
    // Keep 100 ms of source audio at the maximum playback rate, including
    // the 50 ms priming threshold at the actual output sample rate.
    this.capacity = Math.max(16384, Math.ceil(sampleRate * 0.1 * 8));
    this.samples = new Int16Array(this.capacity * 2);
    this.read = 0;
    this.write = 0;
    this.queued = 0;
    this.speed = 1;
    this.phase = 0;
    this.started = false;
    this.underruns = 0;
    this.overruns = 0;
    this.blocks = 0;
    this.port.onmessage = ({ data }) => {
      if (Number.isFinite(data.speed) && data.speed >= 0.5 && data.speed <= 8)
        this.speed = data.speed;
      if (data.clear) {
        this.phase = 0;
        this.read = this.write = this.queued = 0;
        this.started = false;
        return;
      }
      this.enqueue(data.samples);
      if (data.recycle && data.samples instanceof Int16Array) {
        const buffer = data.samples.buffer;
        this.port.postMessage({ recycled: buffer }, [buffer]);
      }
    };
    // Optional priming is useful for offline rendering and prebuffered hosts.
    this.enqueue(options.processorOptions?.samples);
  }
  enqueue(input) {
    if (!(input instanceof Int16Array)) return;
    // Rebase after excessive latency instead of playing stale audio for seconds.
    if (this.queued + input.length / 2 > this.capacity) {
      this.read = this.write = this.queued = this.phase = 0;
      this.overruns++;
    }
    for (
      let i = 0;
      i + 1 < input.length && this.queued < this.capacity;
      i += 2
    ) {
      this.samples[this.write * 2] = input[i];
      this.samples[this.write * 2 + 1] = input[i + 1];
      this.write = (this.write + 1) % this.capacity;
      this.queued++;
    }
  }
  process(_inputs, outputs) {
    const [left, right] = outputs[0];
    if (!left || !right) return true;
    if (!this.started && this.queued >= sampleRate * 0.05 * this.speed)
      this.started = true;
    if (this.started) {
      let starved = false;
      for (let i = 0; i < left.length; i++) {
        const consume = Math.floor(this.phase + this.speed);
        if (this.queued < Math.max(1, consume, this.phase > 0 ? 2 : 1)) {
          starved = true;
          break;
        }
        const next = (this.read + 1) % this.capacity;
        const l = this.samples[this.read * 2],
          r = this.samples[this.read * 2 + 1];
        left[i] = Math.max(
          -1,
          Math.min(
            1,
            (this.phase === 0
              ? l
              : l + (this.samples[next * 2] - l) * this.phase) / 512,
          ),
        );
        right[i] = Math.max(
          -1,
          Math.min(
            1,
            (this.phase === 0
              ? r
              : r + (this.samples[next * 2 + 1] - r) * this.phase) / 512,
          ),
        );
        this.phase = this.phase + this.speed - consume;
        this.read = (this.read + consume) % this.capacity;
        this.queued -= consume;
      }
      if (starved) {
        this.underruns++;
        this.started = false;
      }
    }
    if (++this.blocks % 375 === 0)
      this.port.postMessage({
        queued: this.queued,
        underruns: this.underruns,
        overruns: this.overruns,
      });
    return true;
  }
}
registerProcessor("evenfall-audio", EvenfallAudio);
