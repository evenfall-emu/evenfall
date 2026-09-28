# Landing fresco plates

Home (`/`) is one vertical pixel-art fresco: three painted plates and a
procedural abyss, all 480 art pixels wide on one shared grid. The images are
original artwork made for ZiGBA with an image-generation tool and snapped to
the pixel grid by
[`tools/landing-art`](../../../../tools/landing-art/README.md); they contain
no ROM data, game sprites or game locations. They are placeholders for
hand-finished art on the same grid.

| File          | Size      | Contents                                                                                                                                                                 |
| ------------- | --------- | ------------------------------------------------------------------------------------------------------------------------------------------------------------------------ |
| `surface.png` | 480 × 252 | 64 rows of starry sky above the coastal sunset: cliffs and lighthouse, a distant castle, the sun over the sea, ending in the waterline: the surface seen from just below |
| `descent.png` | 480 × 98  | Below the waterline: light shafts under the sun, rock walls at both edges, fading into deep water                                                                        |
| `seabed.png`  | 480 × 128 | Deep water fading into a seabed with an original sunken ruin at each side, then sand settling into a flat floor under the footer                                         |

The sun covers art pixels 297–346 across and 159–197 down in
`surface.png`; its column is the same in every plate. The surface's top row
is flat sky `#072957`. Its sea runs from row 198 to 241 and its last ten
rows are the painted waterline, which the descent continues; the descent's
bottom rows and the seabed's top rows are flat `#012d53`, the deep water
behind the product sections. The seabed's ruins end by row 88; its last
rows are the flat floor `#09344d`, under the footer at the page's end.

`scenes/` holds three 96 × 114 thumbnails of an original top-down game world
(a village, a forest bridge and a sunken ruin, with the same small traveller)
for the rewind illustration. They were generated and snapped to the grid
with `pixelate.py` at 32 colors; they depict no existing game.

The product sections' other illustrations are sprites with binary
transparency, cut from generated sheets by `sprite.py`. They depict no
existing game, and none carries text or a logo:

| Folder       | Size         | Contents                                                                                                                                               |
| ------------ | ------------ | ------------------------------------------------------------------------------------------------------------------------------------------------------ |
| `covers/`    | 54 × 80 each | Six box covers for the romhack stack: a mountain valley, a crystal cave, a sunset lake, desert ruins, an aurora over snowy forest, snowy coastal ruins |
| `creatures/` | 27–41 × 40   | The invented team of the game-panel illustration: a fire fox, a moss turtle, a bat, an owl, a seahorse, a stone golem                                  |
| `icons/`     | 40–96 wide   | The account illustration: a laptop (96 × 80), a phone (46 × 74), a cloud (40 × 44) and a link (60 × 50)                                                |

The page draws each at a whole multiple of its size. The rewind thumbnails
keep their scale from their height and, where three full frames do not fit,
are cropped at the sides rather than shrunk.

## Rendering

[`fresco.ts`](../../pages/landing/fresco.ts) draws four layers, each in its
own canvas sized to the canvas's device-pixel box, at a whole-number scale
with smoothing off, so every art pixel covers the same number of device
pixels: the surface, the descent, the abyss behind the product sections and
the seabed under the last section and the footer.
[`camera.ts`](../../pages/landing/camera.ts) chooses the scale nearest four
CSS pixels per art pixel and the offset that puts the sun 72% across on wide
views or near the middle on phones, keeping it whole and clear of the hero
text. The lower layers reuse the surface's scale and horizontal offset, so
their columns line up down the page.

Only the layers on screen animate, at fifteen frames a second, and everything
moves by whole art pixels:

- **Waves.** [`waves.ts`](../../pages/landing/waves.ts) shifts each row of
  the painted sea sideways along two slow swells, and draws it from a row a
  pixel or two above or below along a swell travelling toward the viewer, so
  crests, foam and the sun's reflection roll and heave without new pixels.
  Both motions grow from the horizon toward the viewer; the painted cliff
  foot and the waterline stay still.
- **Sky.** [`sky.ts`](../../pages/landing/sky.ts) drifts the painted cloud
  band sideways over two minutes, still at its edges and right of the
  headland, and pulses the lighthouse lamp by stepping its colors. The stars
  stay still and lit.
- **Bubbles.** [`bubbles.ts`](../../pages/landing/bubbles.ts) draws 1–3 pixel
  sprites: a stream rising across the middle of the descent, a burst fanning
  out from the middle when the waterline first reaches the upper part of the
  view, and two streams from the ruins.
- **The abyss.** [`abyss.ts`](../../pages/landing/abyss.ts) is procedural:
  stepped rock walls in the margins beside the text and faint light shafts,
  both dithered in and out at its ends, with kelp swaying on the walls'
  ledges, sinking marine snow, bioluminescent specks and bubbles in the
  margins.

The scene runs without a pause control. It stops while the page is hidden or
the window is unfocused, and a reduced-motion preference shows it still.
