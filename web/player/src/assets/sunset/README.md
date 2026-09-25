# Sunset scene assets

The background was generated for ZiGBA on 2026-09-19. The traveler plate was
replaced on 2026-09-24 with an original cast: a bear explorer looking
into the sunset from a log raft, a lone golden star, and a cream-and-copper
airship. No ROM data or extracted game sprites are included.

- `coast.png`: 1672 × 941 background with the lighthouse and distant castle.
- `travelers.png`: 192 × 96 transparent atlas at the renderer's native pixel
  scale. Bear, raft and contact foam: `(0, 0, 112, 72)`; star: `(120, 0, 18, 18)`;
  airship: `(120, 24, 52, 38)`. Alpha is binary, with no baked-in scenery,
  particle trail or glow. All three sprites render 1:1 on the 836 × 471 canvas.
- `PixelifySans-Bold.ttf` and `PixelifySans-Regular.ttf`: Pixelify Sans 700 and
  400 from Google Fonts, licensed under the adjacent SIL Open Font License
  (`OFL.txt`). The “ZiGBA” header wordmark and Library title initials use this
  family; ZiGBA is described in text as “ZiGBA — GBA emulator”, never with a
  platform-holder brand lock-up. The hero, supporting text, controls and player
  use VT323.

An original concept was used to generate a transparent source sheet with an
image-generation tool. The airship faces right to match its flight path; the
bear faces into the scene, with a resting paddle. The sheet was cropped,
alpha-thresholded at 50%, trimmed and nearest-neighbor resized to native scale. The raft crop excluded
the adjacent airship basket. Transparent edges were inspected over cream and
navy backgrounds before compositing onto the unchanged coast. The contact
waterline was then retouched directly on the native pixel grid: turquoise foam,
broken mint highlights and a little reflected sunset overlap the lowest log
pixels to join the raft to the sea. This extends the original 112 × 68 raft
crop by four rows without rescaling it. The bear and sky sprites remain unchanged.

`sunset-renderer.ts` owns compositing, the staggered flight schedule, wave
sampling and foam swells, a two-minute upper-cloud drift, and the lone star's
short fading particle trail. Cloud sampling tapers to zero before the cliffs and
lower cloud bank. Particles are reconstructed from the flight clock, limited to
nine emissions over 1.4 seconds, with no growing particle list or extra timers.
The bear's loose scarf is separated at runtime using a stepped outline that
excludes the head and backpack. Its red tail bends in pixel columns with a
three-column anchor at the neck and up to two pixels of sway. Its flutter shares
the raft's 3.8-second bobbing cycle, offset to lift as the raft dips, with a
smaller secondary ripple. Both layers share the same rounded vertical position.
The lantern's painted light flickers without blurring or spilling onto the sea.
The contact foam is baked into the raft sprite, visible in the initial
reduced-motion illustration, and shares the logs' rounded vertical position.
There are no separate raft splash particles or emissions.
All motion and light changes freeze with the scene.
`AboutScene` owns visibility and lifecycle; `AboutPage`
owns the pause control and the reduced-motion preference. Rendering stops when
paused, scrolled offscreen, hidden, or unmounted. All assets are served locally.
