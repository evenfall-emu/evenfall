# Third-party notices

Evenfall's own code is licensed under the
[GNU Affero General Public License, version 3 or later](LICENSE). This file lists
material from other projects that ships in this repository, and the references
it relies on without redistributing them.

## Distributed with Evenfall

| Material                  | Location                                               | Source                                                                                                                                                                            | License                                                                |
| ------------------------- | ------------------------------------------------------ | --------------------------------------------------------------------------------------------------------------------------------------------------------------------------------- | ---------------------------------------------------------------------- |
| VT323 font                | `web/player/src/assets/VT323-Regular.ttf`              | The VT323 Project Authors                                                                                                                                                         | SIL OFL 1.1 ([text](web/player/src/assets/VT323-LICENSE.txt))          |
| Press Start 2P font       | `web/player/src/assets/fonts/PressStart2P-Regular.ttf` | The Press Start 2P Project Authors                                                                                                                                                | SIL OFL 1.1 ([text](web/player/src/assets/fonts/PressStart2P-OFL.txt)) |
| GBA dump names (No-Intro) | `web/player/src/pages/library/no-intro-gba.json`       | The No-Intro Game Boy Advance DAT, via [libretro-database](https://github.com/libretro/libretro-database), reduced to SHA-1 and name by `web/server/scripts/catalog/no-intro.mjs` | CC BY-SA 4.0 ([text](https://creativecommons.org/licenses/by-sa/4.0/)) |
| Result-screen glyphs      | `systems/gba/tests/validator/jsmolka_cases.rs`         | [jsmolka/gba-tests](https://github.com/jsmolka/gba-tests) `lib/glyphs.asm`, Copyright (c) 2019 Julian Smolka                                                                      | MIT                                                                    |

The JavaScript packages, the player and the server install their dependencies
from npm. Their licenses are recorded in the root `package-lock.json`. The Rust
crates' dependencies and their licenses are recorded in `Cargo.lock`.

## Facts derived from other emulators

These are short factual tables:

- `systems/gba/src/rom_image.rs` lists a few save-type overrides on which
  mGBA 0.10.5 and NanoBoyAdvance's game database agree.

## Referenced, not distributed

`tools/setup-refs.sh` fetches pinned third-party projects into the gitignored
`refs/` directory for comparison and testing. [`refs/README.md`](refs/README.md)
lists each project, commit and license. They include GPL-3.0 and MPL-2.0 code,
which Evenfall studies but does not copy.

## Trademarks

Evenfall is not affiliated with or endorsed by Nintendo. Game Boy Advance is a
trademark of Nintendo. Pokémon is a trademark of Nintendo, Creatures Inc. and
GAME FREAK inc. Other names are trademarks of their respective owners and are
used only to identify compatible software.
