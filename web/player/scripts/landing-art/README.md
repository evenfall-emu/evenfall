# Landing art pipeline

These scripts turn generated paintings into the pixel-art plates of the
Home page, described in
[`web/player/src/assets/landing`](../../src/assets/landing/README.md).
They need Python 3 and ImageMagick 7 (`magick`).

1. **Snap each painting to the grid.** `pixelate.py` reduces the painting to
   a fixed palette without dithering, gives each art pixel the most common
   color among the source pixels it covers, and replaces isolated pixels with
   their neighbours' color. `--keep-above ROW` keeps isolated pixels above
   that row, such as stars.

   ```sh
   python3 web/player/scripts/landing-art/pixelate.py sunset.png 480 202 32 surface-480.png --keep-above 60
   python3 web/player/scripts/landing-art/pixelate.py underwater.png 480 200 28 underwater-480.png
   ```

2. **Build the plates.** `build.py` adds starry sky above the sunset, ends
   its sea where the painted waves end with a waterline (the surface seen
   from just below), cuts the underwater plate into the descent and the
   seabed, joins them with ordered-dither fades, and extends the seabed with
   sand that settles into the footer's flat floor, writing
   `web/player/src/assets/landing/*.png`.

   ```sh
   python3 web/player/scripts/landing-art/build.py surface-480.png underwater-480.png
   ```

3. **Cut the sprites.** `sprite.py` cuts one item out of a generated sheet
   painted on flat magenta and snaps it to the grid like `pixelate.py`, but
   treats magenta as background: an art pixel is transparent when most of
   the source pixels it covers are background. The arguments are the item's
   rectangle in sheet pixels, the sprite's size in art pixels, the palette
   size and the output file.

   ```sh
   python3 web/player/scripts/landing-art/sprite.py SHEET X Y W H WIDTH HEIGHT COLORS OUT
   ```

The source paintings and sheets are not kept in the repository. A
replacement plate must keep the sizes, flat edge rows and sun position
recorded in the assets' README, or `fresco.ts` must change with it.
