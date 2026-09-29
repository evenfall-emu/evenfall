// Only explicitly released presentation buffers enter this bounded pool.
// Ordinary SDK observations remain caller-owned for as long as needed.
export class PresentationPool {
  constructor() {
    this.pixels = [];
    this.audio = [];
    this.allocations = 0;
  }
  takePixels() {
    return new Uint16Array(this.take(this.pixels, 240 * 160 * 2));
  }
  takeAudio() {
    return new Int16Array(this.take(this.audio, 4 * 16384 * 2));
  }
  take(pool, size) {
    if (pool.length) return pool.pop();
    this.allocations++;
    return new ArrayBuffer(size);
  }
  release({ pixels, audio } = {}) {
    if (
      pixels instanceof ArrayBuffer &&
      pixels.byteLength === 240 * 160 * 2 &&
      this.pixels.length < 4 &&
      !this.pixels.includes(pixels)
    )
      this.pixels.push(pixels);
    if (
      audio instanceof ArrayBuffer &&
      audio.byteLength === 4 * 16384 * 2 &&
      this.audio.length < 4 &&
      !this.audio.includes(audio)
    )
      this.audio.push(audio);
  }
}
