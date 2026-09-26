# Third-party notices

ZiGBA's own code is licensed under the
[GNU Affero General Public License, version 3 or later](LICENSE). This file lists
material from other projects that ships in this repository, and the references
it relies on without redistributing them.

## Distributed with ZiGBA

| Material | Location | Source | License |
|---|---|---|---|
| VT323 font | `frontend/src/assets/VT323-Regular.ttf` | The VT323 Project Authors | SIL OFL 1.1 ([text](frontend/src/assets/VT323-LICENSE.txt)) |
| Press Start 2P font | `frontend/src/assets/fonts/PressStart2P-Regular.ttf` | The Press Start 2P Project Authors | SIL OFL 1.1 ([text](frontend/src/assets/fonts/PressStart2P-OFL.txt)) |
| GBA dump names (No-Intro) | `frontend/src/pages/library/no-intro-gba.json` | The No-Intro Game Boy Advance DAT, via [libretro-database](https://github.com/libretro/libretro-database), reduced to SHA-1 and name by `tools/catalog/no-intro.mjs` | CC BY-SA 4.0 ([text](https://creativecommons.org/licenses/by-sa/4.0/)) |
| Result-screen glyphs | `test/jsmolka_cases.zig` | [jsmolka/gba-tests](https://github.com/jsmolka/gba-tests) `lib/glyphs.asm`, Copyright (c) 2019 Julian Smolka | MIT |

The JavaScript packages, player and server install their dependencies from
npm. Their licenses are recorded in the root `package-lock.json`. The
published `@evenfall/gba` package has no runtime dependencies.

## Facts derived from other emulators

These are short factual tables, attributed where they are used:

- `src/rom_image.zig` lists a few save-type overrides on which mGBA 0.10.5 and
  NanoBoyAdvance's game database agree.

## Referenced, not distributed

`tools/setup-refs.sh` fetches pinned third-party projects into the gitignored
`refs/` directory for comparison and testing. [`refs/README.md`](refs/README.md)
lists each project, commit and license. They include GPL-3.0 and MPL-2.0 code,
which ZiGBA studies but does not copy.

`tools/mgba-bench/` and `tools/mgba-gdb-diff/` contain original harnesses that
link against a locally built libmgba (MPL-2.0). They are optional
measurement tools and are not part of any ZiGBA binary.

## Trademarks

ZiGBA is not affiliated with or endorsed by Nintendo. Game Boy Advance is a
trademark of Nintendo. Pokémon is a trademark of Nintendo, Creatures Inc. and
GAME FREAK inc. Other names are trademarks of their respective owners and are
used only to identify compatible software.
