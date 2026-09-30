# Research notes

Deeper notes on the mechanisms that make the GBA core fast: what each one
does, why it stays exact, and what remains open.
[`performance.md`](../performance.md) gives the overview and the current
numbers.

| Note                                                     | Subject                                                                                                  |
| -------------------------------------------------------- | -------------------------------------------------------------------------------------------------------- |
| [`execution-engine.md`](execution-engine.md)             | Running ARM and Thumb code from pre-decoded op caches at the interpreter's exact cost                    |
| [`layer-major-compositor.md`](layer-major-compositor.md) | Drawing lines from whole background and object rows with one vector kernel                               |
| [`scanline-memo.md`](scanline-memo.md)                   | Keeping lines whose registers and video memory are unchanged, with change marks and a self-checking mode |
| [`tile-row-cache.md`](tile-row-cache.md)                 | Decoding 4bpp tile rows two pixels per lookup through per-bank tables checked against the palette        |
| [`selective-video.md`](selective-video.md)               | Skipping pixel production during fast-forward, with exact on-demand reconstruction of the framebuffer    |
