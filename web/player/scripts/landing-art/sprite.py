#!/usr/bin/env python3
"""Cut one item out of a generated sheet on a flat magenta background and
snap it to a coarse pixel grid with transparency.

Like pixelate.py, each art pixel takes the most common palette color among
the source pixels it covers, but magenta counts as transparent: an art pixel
is transparent when most of its source pixels are background. The result is
a paletted PNG with binary alpha.

Usage: sprite.py SHEET X Y W H WIDTH HEIGHT COLORS OUT
       (X Y W H is the item's rectangle in the sheet, in sheet pixels)
"""
import subprocess
import sys
from collections import Counter

sheet, x, y, w, h, width, height, colors, out = sys.argv[1:]
x, y, w, h, width, height = map(int, (x, y, w, h, width, height))

MAGENTA = (255, 0, 255)


def is_background(r, g, b):
    return r > 190 and g < 90 and b > 190


# Palette-reduce the item with background painted exact magenta, so it maps to
# one palette entry and never tints the item's own colors.
crop = subprocess.run(
    ["magick", sheet, "-crop", f"{w}x{h}+{x}+{y}", "+repage", "-depth", "8", "rgb:-"],
    capture_output=True,
    check=True,
).stdout
cleaned = bytearray(crop)
for i in range(0, len(cleaned), 3):
    if is_background(*cleaned[i : i + 3]):
        cleaned[i : i + 3] = bytes(MAGENTA)
data = subprocess.run(
    ["magick", "-size", f"{w}x{h}", "-depth", "8", "rgb:-", "+dither", "-colors", colors, "-depth", "8", "rgb:-"],
    input=bytes(cleaned),
    capture_output=True,
    check=True,
).stdout

pixels = bytearray()
for gy in range(height):
    y0, y1 = gy * h // height, (gy + 1) * h // height
    for gx in range(width):
        x0, x1 = gx * w // width, (gx + 1) * w // width
        counts = Counter()
        background = 0
        for sy in range(y0, y1):
            for sx in range(x0, x1):
                i = (sy * w + sx) * 3
                rgb = data[i : i + 3]
                if is_background(*rgb):
                    background += 1
                else:
                    counts[rgb] += 1
        total = (y1 - y0) * (x1 - x0)
        if background * 2 >= total or not counts:
            pixels += b"\0\0\0\0"
        else:
            pixels += counts.most_common(1)[0][0] + b"\xff"

subprocess.run(
    ["magick", "-size", f"{width}x{height}", "-depth", "8", "rgba:-", "-strip", out],
    input=bytes(pixels),
    check=True,
)
