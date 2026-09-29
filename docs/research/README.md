# Research notes

These notes cover mechanisms of the Rust port, most of them retained from
ZiGBA: what each does, why it is exact or safe, which alternatives were
measured, and what remains open. ZiGBA's
[`performance.md`](../../refs/zigba/docs/performance.md) supplies the original
performance context; Evenfall's [`design/rewind.md`](../design/rewind.md) and
[`design/save-states.md`](../design/save-states.md) describe the Rust contracts.

Measurements here are ZiGBA measurements unless a note says it measured
Evenfall. They explain design decisions, not performance guarantees for other
games, hosts or browsers.

| Note                                                     | Subject                                                                                                                                |
| -------------------------------------------------------- | -------------------------------------------------------------------------------------------------------------------------------------- |
| [`scanline-memo.md`](scanline-memo.md)                   | Keeping scanlines whose registers and video memory reads are unchanged, with change marks and a self-checking mode                     |
| [`layer-major-compositor.md`](layer-major-compositor.md) | Composing lines from whole background and object rows with one vector kernel, and when raster history still needs the reference passes |
| [`tile-row-cache.md`](tile-row-cache.md)                 | Decoding 4bpp tile rows two pixels per lookup through per-bank tables validated against the palette's content                          |
| [`selective-video.md`](selective-video.md)               | Omitting pixel production above 2× playback, with exact on-demand reconstruction of the framebuffer                                    |
| [`rewind-compression.md`](rewind-compression.md)         | Compressing rewind history so it can cover hours of play in a small memory budget                                                      |
| [`execution-engine.md`](execution-engine.md)             | Running ARM and Thumb code from pre-decoded op caches at the interpreter's exact cost, with the interpreter as reference               |
