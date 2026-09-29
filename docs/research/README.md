# Research notes

These notes cover mechanisms retained from ZiGBA in the Rust port: what each
does, why it is exact or safe, which alternatives were measured on the Zig
implementation, and what remains open. ZiGBA's
[`performance.md`](../../refs/zigba/docs/performance.md) supplies the original
performance context; Evenfall's [`design/rewind.md`](../design/rewind.md) and
[`design/save-states.md`](../design/save-states.md) describe the Rust contracts.

Measurements here are ZiGBA measurements, not Evenfall benchmarks. They
explain design decisions, not performance guarantees for other games, hosts
or browsers.

| Note                                             | Subject                                                                                                            |
| ------------------------------------------------ | ------------------------------------------------------------------------------------------------------------------ |
| [`scanline-memo.md`](scanline-memo.md)           | Keeping scanlines whose registers and video memory reads are unchanged, with change marks and a self-checking mode |
| [`tile-row-cache.md`](tile-row-cache.md)         | A content-keyed cache of decoded 4bpp background tile rows and their palette colors                                |
| [`selective-video.md`](selective-video.md)       | Omitting pixel production above 2× playback, with exact on-demand reconstruction of the framebuffer                |
| [`rewind-compression.md`](rewind-compression.md) | Compressing rewind history so it can cover hours of play in a small memory budget                                  |
