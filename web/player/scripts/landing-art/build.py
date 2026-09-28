#!/usr/bin/env python3
"""Builds the landing fresco plates from two snapped 480-pixel-wide plates.

Usage: build.py SURFACE UNDERWATER

SURFACE is a 480x202 sunset plate and UNDERWATER a 480x200 underwater plate,
both made with pixelate.py. The plates are written to
frontend/src/assets/landing:

surface.png  the snapped sunset plate below 64 rows of starry flat sky, cut
             where its painted waves end (SEA_CUT) and finished with a
             WATERLINE: the underside of the surface seen from just below.
descent.png  underwater rows 14-111: the first 6 dither in from the
             waterline's last color, the last 20 dither out into DEEP.
seabed.png   underwater rows 112-199 whose first 20 rows dither out of DEEP,
             then SAND_ROWS of sand: the plate's last sand rows repeated
             with drifting horizontal offsets, each color stepping darker
             along rippled depth lines until only FLOOR, the footer color,
             remains.

Fades use an 8x8 ordered (Bayer) dither, so every pixel stays a palette color
on the grid.
"""
import math
import subprocess
import sys
from pathlib import Path

OUT = Path(__file__).resolve().parents[2] / "frontend/src/assets/landing"
W = 480
DEEP = bytes.fromhex("012d53")
SEA_BOTTOM = bytes.fromhex("012447")
SKY = bytes.fromhex("072957")
STAR = bytes.fromhex("bcd8af")
SKY_ROWS = 64
STARS = [(37, 11), (96, 40), (151, 23), (212, 52), (268, 8), (333, 31), (389, 57), (441, 18), (19, 49), (304, 44)]
# The sunset plate's last rows are nearly flat dark water; the plate is cut
# after its lowest wave crests and the waterline painted in their place.
SEA_CUT = 178
WATERLINE = 10
# The waterline's colors, from the surface's darkest water down.
LINE = [bytes.fromhex(c) for c in ("012447", "03325b", "034579", "169d9b", "55bca4", "0a7086", "01446b")]
# Each seabed color's next darker step, ending in FLOOR, the footer color.
DARKER = {
    bytes.fromhex(a): bytes.fromhex(b)
    for a, b in (
        ("d1d092", "c5b26e"), ("c5b26e", "549f88"), ("60c5a1", "549f88"),
        ("56977c", "328c86"), ("549f88", "328c86"), ("169894", "328c86"),
        ("328c86", "146884"), ("487484", "146884"), ("0a7086", "0d697b"),
        ("236a73", "0d697b"), ("146884", "0d697b"), ("0d697b", "035777"),
        ("035777", "0a546d"), ("0a546d", "01446b"), ("014974", "01446b"),
        ("01446b", "013b64"), ("013b64", "01345a"), ("043b71", "01345a"),
        ("01345a", "09344d"),
    )
}
FLOOR = bytes.fromhex("09344d")
SAND_ROWS = 40
BAYER = [
    [0, 32, 8, 40, 2, 34, 10, 42],
    [48, 16, 56, 24, 50, 18, 58, 26],
    [12, 44, 4, 36, 14, 46, 6, 38],
    [60, 28, 52, 20, 62, 30, 54, 22],
    [3, 35, 11, 43, 1, 33, 9, 41],
    [51, 19, 59, 27, 49, 17, 57, 25],
    [15, 47, 7, 39, 13, 45, 5, 37],
    [63, 31, 55, 23, 61, 29, 53, 21],
]


def rows_of(path):
    data = subprocess.run(["magick", str(path), "-depth", "8", "rgb:-"], capture_output=True, check=True).stdout
    height = len(data) // (W * 3)
    return [[data[(y * W + x) * 3 : (y * W + x) * 3 + 3] for x in range(W)] for y in range(height)]


def fade(row, y, progress, toward):
    """Replaces the pixels whose threshold is below progress with `toward`."""
    return [toward if BAYER[y % 8][x % 8] / 64 < progress else pixel for x, pixel in enumerate(row)]


def write(rows, name):
    pixels = b"".join(b"".join(row) for row in rows)
    subprocess.run(
        ["magick", "-size", f"{W}x{len(rows)}", "-depth", "8", "rgb:-", "-define", "png:color-type=3", "-strip", str(OUT / name)],
        input=pixels,
        check=True,
    )


surface = rows_of(sys.argv[1])
underwater = rows_of(sys.argv[2])
assert len(surface) == 202 and len(underwater) == 200
assert all(pixel == SEA_BOTTOM for pixel in surface[-1]), "surface must end in flat water"
assert all(pixel == SKY for pixel in surface[0]), "surface must start in flat sky"

# Extra flat night sky with a few stars, so tall views crop sky rather than
# forcing a larger scale. The stars reuse the plate's star color.
sky = [[SKY] * W for _ in range(SKY_ROWS)]
for x, y in STARS:
    sky[y][x] = STAR


def waterline():
    """The surface seen from just below. The sea's own last rows continue a
    little, then a broken line of light marks where the waves meet the air:
    short streaks of teal and pale mint, glinting gold under the sun, with
    gaps, then the underwater teal. Each pixel's color comes from how far it
    sits below an undulating edge, and the streaks are cut into dashes."""
    sun = 321
    GOLD = bytes.fromhex("fbde8f")
    rows = []
    for y in range(WATERLINE):
        row = []
        for x in range(W):
            edge = 3 + 1.6 * math.sin(x / 13 + 0.5) + 0.9 * math.sin(x / 5.3 + y * 0.2)
            depth = y - edge
            # Dashes: the light line breaks every so often, varying in length.
            dash = (x // 3 + int(2 * math.sin(x / 17))) % 7
            if depth < -1:
                # Above the line, the sea's own texture continues.
                color = surface[SEA_CUT - WATERLINE + y][x]
            elif depth < 0:
                color = LINE[3] if dash < 5 else LINE[2]
            elif depth < 1:
                near = abs(x - sun) < 30 + 6 * math.sin(x / 4)
                if dash >= 5:
                    color = LINE[3]
                elif near:
                    color = GOLD if dash % 2 == 0 else LINE[4]
                else:
                    color = LINE[4] if dash < 3 else LINE[3]
            elif depth < 2:
                color = LINE[3] if dash < 4 else LINE[5]
            elif depth < 4:
                color = LINE[5]
            else:
                color = LINE[6]
            row.append(color)
        rows.append(row)
    return rows


OUT.mkdir(parents=True, exist_ok=True)
write(sky + surface[:SEA_CUT] + waterline(), "surface.png")

# The underwater plate's first rows show the surface from below, which would
# read as a second sea under the first. The descent starts beneath them and
# dithers in, over a few rows, from the waterline's last color.
TOP_CROP = 14
top = [row[:] for row in underwater[TOP_CROP:112]]
for i in range(6):
    top[i] = fade(top[i], i, 1 - (i + 0.5) / 6, LINE[6])
for i in range(20):
    y = len(top) - 20 + i
    top[y] = fade(top[y], y, (i + 0.5) / 20, DEEP)
write(top, "descent.png")

bottom = [row[:] for row in underwater[112:]]
for i in range(20):
    bottom[i] = fade(bottom[i], i, 1 - (i + 0.5) / 20, DEEP)


def darken(color, steps):
    """`color` stepped `steps` times toward FLOOR; unknown dark colors and
    anything past the chain become FLOOR."""
    for _ in range(steps):
        if color == FLOOR:
            break
        color = DARKER.get(color, FLOOR)
    return color


# The sand continues the plate's own texture: its last few rows, which are
# open sand, repeat below it with drifting horizontal offsets, so there is no
# seam and no reflected rock. Each pixel darkens by a number of steps that
# grows with depth, offset by slow ripples so the steps undulate instead of
# forming straight bands or a dither pattern.
SOURCE_ROWS = 5
sand = []
for i in range(SAND_ROWS):
    source = bottom[len(bottom) - SOURCE_ROWS + (i % SOURCE_ROWS)]
    shift = (i * 37) % W
    row = []
    for x in range(W):
        ripple = 2.5 * math.sin(x / 21 + 0.8) + 1.5 * math.sin(x / 8.5 + i / 5)
        steps = max(0, math.floor((i + ripple) / 4))
        row.append(darken(source[(x + shift) % W], steps))
    sand.append(row)
for i in range(SAND_ROWS - 4, SAND_ROWS):
    sand[i] = [FLOOR] * W
write(bottom + sand, "seabed.png")
