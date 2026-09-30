# Contributing to Evenfall

Thanks for your interest.
Evenfall is a deterministic, cycle-accurate emulator, and it values evidence over agreement.
A change that makes a game work is welcome when we can explain, from hardware documentation or a test ROM, why the new behavior is right.

Read [`docs/architecture.md`](docs/architecture.md) for how the code is layered, and the [hardware reference](docs/gba/hardware/README.md) for how each part of the GBA is modeled and why.

## Ground rules

- **Write original code.** The emulators pinned under `refs/` are for comparison.
  Study them, but don't copy or translate their source.
- **Never commit ROMs, BIOS images, saves or states**, including in tests, issues or pull requests.
  Identify software by title and SHA-1.
  Test ROMs are built from the C sources in [`probe/`](probe/README.md).
- **Cite the evidence.** Hardware claims rank from Arm's manuals, through measured hardware and GBATEK, down to other emulators, whose agreement alone isn't proof ([hardware sources](docs/gba/hardware/README.md#sources)).
- **Keep guest behavior exact.** Optimizations must keep every timed fetch, event order, audio sample and pixel identical, and every fast path keeps a reference path and a test that compares them.
  Read [`docs/performance.md`](docs/performance.md) before starting performance work.

## Setting up

| Tool                                                     | Needed for                                   |
| -------------------------------------------------------- | -------------------------------------------- |
| Rust, through `rustup` (pinned in `rust-toolchain.toml`) | everything                                   |
| Node.js 24.11 or later, and npm                          | the WebAssembly build, the SDK and its tests |
| `arm-none-eabi-gcc`, Make and Python 3                   | building the probe ROMs                      |

```sh
tools/setup-refs.sh gba-tests cult-gba-bios   # public test ROMs and an open BIOS
make -C probe                                 # probe ROMs
npm ci
cargo test --workspace
```

`tools/setup-refs.sh` fetches the pinned references listed in [`refs/README.md`](refs/README.md).
Plain `cargo test -p evenfall-gba --lib` works on a fresh clone with Rust alone.

## Testing your change

Run the narrowest tests while you iterate, then the full checks before opening a pull request.
[`docs/testing.md`](docs/testing.md) describes every layer.

```sh
cargo fmt --all --check
cargo clippy --workspace --all-targets -- -D warnings
cargo test --workspace
cargo test --workspace --release
npm run fmt:check
npm run check
npm run build:wasm -w @evenfall/gba && npm test
```

| You changed                                        | Run                                                                        |
| -------------------------------------------------- | -------------------------------------------------------------------------- |
| the core (`systems/gba`, `crates/`)                | `cargo test` in debug and release, and the validator with `--require-roms` |
| a hot path (CPU engine, bus, DMA, APU, compositor) | the above, and `node tools/bench/inline-stability.mjs`, which must exit 0  |
| the WebAssembly module or SDK (`web/gba`)          | `npm run build:wasm -w @evenfall/gba`, `npm run check`, `npm test`         |
| documentation only                                 | `npm run fmt:check` and `git diff --check`                                 |

For emulation changes, add a focused test at the narrowest stable place (normally `Machine`, `Bus` or the device that owns the behavior), with expected values from an independent source rather than from the code under test.
When a behavior can only be seen end to end, a small probe ROM in `probe/` with its expected result in the validator is the preferred evidence.

## Pull requests

- Keep each pull request to one change, and explain the visible behavior and its evidence in the description.
- Format Rust with `cargo fmt` and everything else with `npm run fmt`.
- Write commit subjects as short imperative sentences, such as "Keep a running timer's prescaler phase below its new divisor".
- Update the documentation for any behavior, API or save-state format you change: hardware behavior in [`docs/gba/hardware/`](docs/gba/hardware/README.md), formats and runtime design in [`docs/design/`](docs/design/save-states.md).
  Documentation describes the current system, and history belongs in commit messages.
  The writing rules are in [`AGENTS.md`](AGENTS.md#writing-documentation).
- Continuous integration must pass.
  Maintainers may ask for more test ROMs or performance measurements for changes to timing-sensitive code.
- Pull requests are squash-merged using their title, so make the title a good commit subject.

## Reporting bugs

Use the issue forms.
For emulation bugs, include the Evenfall revision, the boot mode, the software's title and SHA-1, and a scenario or failure bundle if you can ([`docs/devtools.md`](docs/devtools.md)).
Report security problems privately, as described in [`SECURITY.md`](SECURITY.md).

## License

By contributing, you agree that your contributions are licensed under the [GNU Affero General Public License, version 3 or later](LICENSE).
Before we merge pull requests from outside contributors, we will also ask you to sign a contributor license agreement, which lets the Evenfall project offer Evenfall under additional terms, such as in a paid app.
It will be linked here once it is in place.
Everyone taking part must follow the [Code of Conduct](CODE_OF_CONDUCT.md).
