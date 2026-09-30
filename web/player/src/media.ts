import { VOLUME_KEY } from "./deployed-names";
// Vite emits the SDK worklet as a separate static file and returns its URL.
import audioWorkletUrl from "@evenfall/gba/audio-worklet?url&no-inline";
// Build through Uint8ClampedArray to preserve the old conversion's exact
// rounding, then use a native-endian packed view for copying RGBA bytes.
const rgba = new Uint8ClampedArray(32768 * 4);
for (let p = 0; p < 32768; p++) {
  rgba[p * 4] = ((p & 31) * 255) / 31;
  rgba[p * 4 + 1] = (((p >> 5) & 31) * 255) / 31;
  rgba[p * 4 + 2] = (((p >> 10) & 31) * 255) / 31;
  rgba[p * 4 + 3] = 255;
}
const colors = new Uint32Array(rgba.buffer);
const surfaces = new WeakMap<
  HTMLCanvasElement,
  { ctx: CanvasRenderingContext2D; image: ImageData; words: Uint32Array }
>();
export function draw(canvas: HTMLCanvasElement, pixels: ArrayLike<number>) {
  let surface = surfaces.get(canvas);
  if (!surface) {
    const ctx = canvas.getContext("2d", { alpha: false })!;
    const image = ctx.createImageData(240, 160);
    surface = {
      ctx,
      image,
      words: new Uint32Array(image.data.buffer, image.data.byteOffset, 38400),
    };
    surfaces.set(canvas, surface);
  }
  for (let i = 0; i < 38400; i++) {
    surface.words[i] = colors[pixels[i] & 0x7fff];
  }
  surface.ctx.putImageData(surface.image, 0, 0);
}

/** Keep only the newest frame until the display can present it. */
export class VideoOutput {
  private canvas?: HTMLCanvasElement;
  private pixels?: Uint16Array;
  private animation?: number;
  constructor(private readonly recycle: (pixels: Uint16Array) => void) {}
  send(canvas: HTMLCanvasElement, pixels: Uint16Array) {
    if (this.pixels) this.recycle(this.pixels);
    this.canvas = canvas;
    this.pixels = pixels;
    this.animation ??= requestAnimationFrame(this.present);
  }
  private present = () => {
    this.animation = undefined;
    const pixels = this.pixels!,
      canvas = this.canvas!;
    this.pixels = this.canvas = undefined;
    try {
      draw(canvas, pixels);
    } finally {
      this.recycle(pixels);
    }
  };
  flush() {
    if (!this.pixels) return;
    if (this.animation !== undefined) cancelAnimationFrame(this.animation);
    this.present();
  }
  clear() {
    if (this.animation !== undefined) cancelAnimationFrame(this.animation);
    this.animation = undefined;
    if (this.pixels) this.recycle(this.pixels);
    this.pixels = this.canvas = undefined;
  }
}
export function download(data: BlobPart | Uint8Array, name: string) {
  const url = URL.createObjectURL(
    new Blob([data instanceof Uint8Array ? data.slice().buffer : data]),
  );
  const a = document.createElement("a");
  a.href = url;
  a.download = name;
  a.click();
  setTimeout(() => URL.revokeObjectURL(url), 1000);
}

/** Owns presentation only. Emulated audio production and drains stay in the Worker. */
export class AudioOutput {
  context?: AudioContext;
  node?: AudioWorkletNode;
  muted = false;
  volume = 1;
  onRecycle?: (samples: Int16Array) => void;
  private gain?: GainNode;
  private speed = 1;
  private sources = new Set<AudioBufferSourceNode>();
  private nextTime = 0;
  private workletStarted = false;
  constructor() {
    try {
      const stored = localStorage.getItem(VOLUME_KEY);
      if (stored !== null) {
        const volume = Number(stored);
        if (Number.isFinite(volume) && volume >= 0 && volume <= 1)
          this.volume = volume;
      }
    } catch {
      // Audio still works when browser storage is unavailable.
    }
  }
  async ready() {
    this.context ??= new AudioContext({ latencyHint: "interactive" });
    if (!this.gain) {
      this.gain = this.context.createGain();
      this.gain.gain.value = this.volume;
      this.gain.connect(this.context.destination);
    }
    return this.context.sampleRate;
  }
  setVolume(volume: number) {
    this.volume = volume;
    if (this.gain) this.gain.gain.value = volume;
    try {
      localStorage.setItem(VOLUME_KEY, String(volume));
    } catch {
      // A denied preference write must not interrupt playback.
    }
  }
  async play() {
    await this.ready();
    if (!this.muted) await this.context!.resume();
    // Buffer sources keep sound available if worklet initialization is delayed or
    // unsupported. Both paths have a bounded presentation queue.
    if (!this.workletStarted && this.context!.audioWorklet) {
      this.workletStarted = true;
      void this.context!.audioWorklet.addModule(audioWorkletUrl)
        .then(() => {
          if (!this.context || this.context.state === "closed") return;
          this.clear();
          this.node = new AudioWorkletNode(this.context, "evenfall-audio", {
            numberOfInputs: 0,
            numberOfOutputs: 1,
            outputChannelCount: [2],
          });
          this.node.port.onmessage = ({ data }) => {
            if (data.recycled) this.onRecycle?.(new Int16Array(data.recycled));
          };
          this.node.port.postMessage({ speed: Math.min(2, this.speed) });
          this.node.connect(this.gain!);
        })
        .catch(() => {
          this.workletStarted = false;
        });
    }
  }
  setSpeed(speed: number) {
    if (speed === this.speed) return;
    this.speed = speed;
    this.clear();
    this.node?.port.postMessage({ speed: Math.min(2, speed) });
  }
  clear() {
    this.node?.port.postMessage({ clear: true });
    for (const source of this.sources) {
      source.onended = null;
      try {
        source.stop();
      } catch {
        /* Already ended. */
      }
      source.disconnect();
    }
    this.sources.clear();
    this.nextTime = 0;
  }
  send(samples: Int16Array) {
    const context = this.context;
    if (
      this.muted ||
      this.speed > 2 ||
      context?.state !== "running" ||
      !samples.length
    ) {
      this.onRecycle?.(samples);
      return;
    }
    if (this.node) {
      this.node.port.postMessage({ samples, recycle: !!this.onRecycle }, [
        samples.buffer,
      ]);
      return;
    }
    if (this.nextTime - context.currentTime > 0.12 || this.sources.size >= 16)
      this.clear();
    const buffer = context.createBuffer(
      2,
      samples.length / 2,
      context.sampleRate,
    );
    const left = buffer.getChannelData(0),
      right = buffer.getChannelData(1);
    for (let i = 0; i < left.length; i++) {
      left[i] = Math.max(-1, Math.min(1, samples[i * 2] / 512));
      right[i] = Math.max(-1, Math.min(1, samples[i * 2 + 1] / 512));
    }
    this.onRecycle?.(samples);
    const source = context.createBufferSource();
    source.buffer = buffer;
    source.playbackRate.value = this.speed;
    source.connect(this.gain!);
    source.onended = () => {
      this.sources.delete(source);
      source.disconnect();
    };
    this.sources.add(source);
    const start = Math.max(context.currentTime + 0.015, this.nextTime);
    source.start(start);
    this.nextTime = start + buffer.duration / this.speed;
  }
  async mute() {
    this.muted = !this.muted;
    this.clear();
    if (this.muted) await this.context?.suspend();
    else await this.play();
  }
  close() {
    this.clear();
    this.node?.disconnect();
    this.gain?.disconnect();
    void this.context?.close();
  }
}
