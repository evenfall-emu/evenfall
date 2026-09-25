# Sunset scene assets

The About page's animated sunset is built from two images.
The images are original artwork made for ZiGBA with an image-generation tool
and finished by hand on the pixel grid; they contain no ROM data or game
sprites.

| File | Contents |
|---|---|
| `coast.png` | 1672 × 941 background: sea, cliffs, a lighthouse and a distant castle |
| `travelers.png` | 192 × 96 transparent atlas at the renderer's native pixel scale |

The atlas holds three sprites, each drawn 1:1 on the 836 × 471 canvas, with
binary alpha and no baked-in scenery or glow:

| Sprite | Rectangle (x, y, width, height) |
|---|---|
| bear explorer on a log raft, with contact foam | `(0, 0, 112, 72)` |
| golden star | `(120, 0, 18, 18)` |
| airship, facing right along its flight path | `(120, 24, 52, 38)` |

The hero text, supporting text, controls and player use VT323. The header's
banner is a crop of `coast.png`. In text ZiGBA is described as
"ZiGBA — GBA emulator", never with a platform holder's brand.

## Rendering

`sunset-renderer.ts` composites the scene: the staggered flight schedule, wave
sampling and foam swells, a two-minute drift of the upper clouds, and the
star's short fading particle trail (at most nine emissions over 1.4 seconds,
reconstructed from the flight clock rather than stored). Cloud sampling tapers
to zero before the cliffs and the lower cloud bank.

The bear's scarf is separated at runtime along a stepped outline that excludes
the head and backpack; its tail bends in pixel columns with a three-column
anchor at the neck and up to two pixels of sway, sharing the raft's
3.8-second bobbing cycle. The lantern flickers without spilling light onto the
sea. With reduced motion the scene is a still illustration.

`AboutScene` owns visibility and lifecycle, and `AboutPage` owns the pause
control and the reduced-motion preference. Rendering stops when the scene is
paused, scrolled offscreen, hidden or unmounted. All assets are served
locally.
