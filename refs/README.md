# Pinned reference checkouts

This directory holds the outside projects Evenfall tests with, compares against, or reads as documentation.
[`tools/setup-refs.sh`](../tools/setup-refs.sh) clones each one at an exact commit, so every test and benchmark runs against the same code.
The checkouts are kept out of Git, and this file records where they come from.
ROMs, BIOS dumps, saves and other user content are never part of setup.

```sh
tools/setup-refs.sh                   # every reference
tools/setup-refs.sh gba-tests suite   # chosen references, by directory name
```

The script fetches only the pinned commit and checks it out detached.
It skips a checkout already at its pin, and refuses to replace one with local changes or a directory that isn't a Git checkout.

| Directory         | Source                                                     | Pinned commit                                       | License     | Used for                                                               |
| ----------------- | ---------------------------------------------------------- | --------------------------------------------------- | ----------- | ---------------------------------------------------------------------- |
| `gba-tests/`      | <https://github.com/jsmolka/gba-tests>                     | `a7113b67e63f83a9b321696ddd7042ccfad6c881`          | MIT         | the `jsmolka-*` test ROM cases                                         |
| `cult-gba-bios/`  | <https://github.com/Cult-of-GBA/BIOS>                      | `a30e9a96df083628b650724b7d4d7112b4070b98`          | MIT         | an open replacement BIOS for real-BIOS runs and benchmarks             |
| `suite/`          | <https://github.com/mgba-emu/suite>                        | `e6942030d25ffe3ba76c72b73a86da073ec857cc`          | MIT         | the recorded hardware results behind the suite-table tests             |
| `mgba/`           | <https://github.com/mgba-emu/mgba>                         | `26b7884bc25a5933960f3cdcd98bac1ae14d42e2` (0.10.5) | MPL-2.0     | behavior and performance comparison                                    |
| `nanoboyadvance/` | <https://github.com/nba-emu/NanoBoyAdvance>                | `55b5cf0ae3d929582ac5bfd486558173502b8354`          | GPL-3.0     | behavior comparison                                                    |
| `delta/`          | <https://github.com/rileytestut/Delta>                     | `c1d3d068e019e6493eed45654569db3cc5beb86a`          | AGPL-3.0    | where the VBA-M benchmark comes from: Delta pins `gbadeltacore/` here  |
| `gbadeltacore/`   | <https://github.com/rileytestut/GBADeltaCore>              | `869c34aeca9dd2b3fbd329092bb2743b0ae80c98`          | none stated | Delta's VBA-M build (sources, flags, setup) and its bundled SFML (zlib) |
| `vbam/`           | <https://github.com/visualboyadvance-m/visualboyadvance-m> | `453fa0decf179360926fb417725a794aa496d6e3`          | GPL-2.0     | performance comparison: the VBA-M core `gbadeltacore/` pins            |
| `zigba/`          | <https://github.com/evenfall-emu/zigba>                    | `add59e5957b75f29d084813336ee20e300fc0b6e`          | AGPL-3.0    | Evenfall's predecessor, for the internal parity and benchmark tools    |

Other emulators are studied and compared against, never copied.
An emulator's behavior is a comparison point, not a hardware specification.
