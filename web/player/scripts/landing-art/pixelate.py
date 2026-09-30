#!/usr/bin/env python3
"""Snap a generated painting onto a coarse logical pixel grid, cleanly.

1. Reduce the full-resolution source to a fixed palette without dithering,
   so every source pixel is already one of the final colors.
2. Give each logical pixel the most common palette color among the source
   pixels it covers (a mode, not an average: no new blended colors).
3. Despeckle: a logical pixel that matches none of its eight neighbours
   takes their most common color. Isolated single pixels in the sky above
   `--keep-above` survive, so stars stay.

Usage: pixelate.py SOURCE WIDTH HEIGHT COLORS OUT [--keep-above ROW]
"""
import argparse
import subprocess
from collections import Counter

parser = argparse.ArgumentParser()
parser.add_argument("source")
parser.add_argument("width", type=int)
parser.add_argument("height", type=int)
parser.add_argument("colors", type=int)
parser.add_argument("out")
parser.add_argument("--keep-above", type=int, default=0)
args = parser.parse_args()


def raw(*command):
    return subprocess.run(["magick", *command, "-depth", "8", "rgb:-"], capture_output=True, check=True).stdout


size = subprocess.run(
    ["magick", "identify", "-format", "%w %h", args.source], capture_output=True, check=True, text=True
).stdout.split()
sw, sh = int(size[0]), int(size[1])
data = raw(args.source, "-colorspace", "sRGB", "+dither", "-colors", str(args.colors))

W, H = args.width, args.height
grid = [[None] * W for _ in range(H)]
for gy in range(H):
    y0, y1 = gy * sh // H, (gy + 1) * sh // H
    for gx in range(W):
        x0, x1 = gx * sw // W, (gx + 1) * sw // W
        counts = Counter()
        for y in range(y0, y1):
            row = y * sw * 3
            for x in range(x0, x1):
                i = row + x * 3
                counts[data[i : i + 3]] += 1
        grid[gy][gx] = counts.most_common(1)[0][0]

cleaned = [row[:] for row in grid]
for y in range(1, H - 1):
    if y < args.keep_above:
        continue
    for x in range(1, W - 1):
        here = grid[y][x]
        around = [grid[y + dy][x + dx] for dy in (-1, 0, 1) for dx in (-1, 0, 1) if dy or dx]
        if here not in around:
            cleaned[y][x] = Counter(around).most_common(1)[0][0]

pixels = b"".join(b"".join(row) for row in cleaned)
subprocess.run(
    ["magick", "-size", f"{W}x{H}", "-depth", "8", "rgb:-", "-define", "png:color-type=3", args.out],
    input=pixels,
    check=True,
)
