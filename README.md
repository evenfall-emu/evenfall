# Evenfall

[![License: AGPL-3.0-or-later](https://img.shields.io/badge/license-AGPL--3.0--or--later-blue)](LICENSE)

A family of deterministic, cycle-aware emulators of classic consoles in one Rust codebase, native and in the browser.

The first system is the Game Boy Advance.
It is deterministic: the same ROM and timestamped inputs always give the same pixels, audio samples, battery saves and save states, however the host slices execution.
It draws every frame and emulates audio exactly, and still runs in about half of mGBA's time.
It replaces ZiGBA, our earlier emulator written in Zig, whose save states still load.
Later systems (Nintendo DS, N64, GameCube, Wii) will share its crates as they are extracted from concrete uses.

> Evenfall does not include BIOS images or games.
> Bring your own legally
> obtained files.
> Evenfall is not affiliated with or endorsed by Nintendo.

## What is here

| Path          | Contents                                                                                |
| ------------- | --------------------------------------------------------------------------------------- |
| `crates/`     | shared crates: the `System` interface, state codec, ARM7TDMI interpreter, audio, replay |
| `systems/gba` | the Game Boy Advance core and its headless runner                                       |
| `web/gba`     | the WebAssembly module and the `@evenfall/gba` JavaScript SDK                           |
| `tools/`      | test and benchmark tools                                                                |
| `probe/`      | bare-metal probe ROM sources                                                            |
| `docs/`       | [documentation](docs/README.md), starting with [status](docs/status.md)                 |

## Build and test

The toolchain is pinned in `rust-toolchain.toml` (stable Rust; `rustup` installs it on first use).

```sh
cargo test --workspace                    # every Rust test
cargo run --release -p evenfall-gba -- run --steps 5000000 --frame-hash rom.gba
npm ci && npm run build:wasm -w @evenfall/gba && npm test
```

The ROM validators (which need `arm-none-eabi-gcc` for the probe ROMs) are described in [`docs/testing.md`](docs/testing.md), and the benchmarks in [`docs/performance.md`](docs/performance.md).

## License

AGPL-3.0-or-later.
See [`LICENSE`](LICENSE).
