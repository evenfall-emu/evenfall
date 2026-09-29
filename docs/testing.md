# Testing

Evenfall uses focused unit and integration tests, independent ROM oracles,
and byte-exact parity against ZiGBA. [`methodology.md`](methodology.md) explains
what each layer establishes; [`compatibility.md`](compatibility.md) records the
current GBA validator coverage and limitations.

## Rust core

Run the workspace checks from the repository root:

```sh
cargo fmt --all --check
cargo clippy --workspace --all-targets -- -D warnings
cargo test --workspace
cargo test --workspace --release
```

The GBA library's unit tests cover the CPU, bus, devices, HLE services,
renderer, host policy, state codec, rewind and session. Integration targets
under [`systems/gba/tests/`](../systems/gba/tests/) exercise ROMs and hardware
tables. For a focused check, run `cargo test -p evenfall-gba --lib`, or use a
test target such as `cargo test -p evenfall-gba --test mgba_suite_timing`.
Run affected tests in both debug and release: optimized execution can expose
different errors. The workspace test command also runs the ROM validators;
install their inputs first, or select non-ROM targets while working locally.

## ROM validators

The [`validator`](../systems/gba/tests/validator/main.rs) runs the pinned
jsmolka/gba-tests ROMs and the project's bare-metal probes against the
oracles in [`jsmolka_cases.rs`](../systems/gba/tests/validator/jsmolka_cases.rs) and
[`probe_cases.rs`](../systems/gba/tests/validator/probe_cases.rs). The jsmolka checkout
contains prebuilt ROMs; probes require an `arm-none-eabi-gcc` toolchain with
`objcopy` and `nm`, Make and Python 3. Inputs and build products are ignored,
not checked into Git.

```sh
tools/setup-refs.sh gba-tests
make -C probe
cargo test -p evenfall-gba --test validator -- --require-roms
cargo test -p evenfall-gba --test validator --release -- --require-roms
```

`--require-roms` is the CI gate: every selected ROM must exist, and a guest
fault, ROM hash mismatch, failed register/save/device/audio/frame oracle or
missing ROM fails the target. Without it, absent ROMs are skipped with their
paths and setup reasons; a run with no available cases still fails. The
validator prints a line for each case. Arguments select exact names or
hyphen-boundary prefixes, and `--list` prints paths without loading ROMs:

```sh
cargo test -p evenfall-gba --test validator -- probe-timer jsmolka-ppu
cargo test -p evenfall-gba --test validator -- --list
cargo test -p evenfall-gba --test validator -- --gba-tests refs/gba-tests --probes probe/build probe
```

Paths supplied to these flags are relative to the repository root unless
absolute. `--bios FILE` boots only jsmolka cases through a 16 KiB external
BIOS; probes always use HLE. With no BIOS flag, all cases use HLE. The
`EVENFALL_VALIDATE_BIOS` environment variable is an alternative to `--bios`.
For the open Cult-of-GBA replacement BIOS:

```sh
tools/setup-refs.sh cult-gba-bios
cargo test -p evenfall-gba --test validator -- --bios refs/cult-gba-bios/bios.bin jsmolka
```

The full replacement-BIOS selection exits nonzero because `jsmolka-bios`
expects Nintendo boot's protected-read latch and fails at test 1. Every other
jsmolka case passes with that BIOS; see
[`compatibility.md`](compatibility.md#known-divergences). Do not use
`--require-roms` as a way to ignore this expected failure: it controls missing
media, not oracles. No Nintendo BIOS is supplied or fetched.

## Hardware table fixtures

The pinned mGBA suite's recorded hardware values run as ordinary integration
tests; they do not require devkitARM or a suite ROM build:

```sh
cargo test -p evenfall-gba --test mgba_suite_timing
cargo test -p evenfall-gba --test mgba_suite_source
```

[`mgba_suite_timing.rs`](../systems/gba/tests/mgba_suite_timing.rs) measures
instruction and DMA timing through `Machine` with suite-style calibration;
[`mgba_suite_source.rs`](../systems/gba/tests/mgba_suite_source.rs) checks DMA
source and latch behavior through `Bus`. The recorded numbers come from the
suite, not Rust output. Known residuals and limits are in
[`compatibility.md`](compatibility.md#mgba-suite-hardware-tables).

## Parity traces

The ZiGBA/Rust trace workflow, required reference setup and comparison
commands live in [`tools/parity/README.md`](../tools/parity/README.md).
Its scenarios compare frames, audio, battery saves and complete state bytes
across host slice schedules; linked probes use the ROM validator instead.

## JavaScript SDK and documentation

Node.js 24.11+ is required for the npm workspace. The root scripts check
formatting, lint, TypeScript and the SDK's Node tests:

```sh
npm ci
npm run fmt:check
npm run check
npm run test
```

`npm run fmt` applies the repository formatter, including Markdown. The
[`web/gba/scripts/`](../web/gba/scripts/) directory contains the package and
WASM test helpers; the root npm test command does not run a native/WASM
comparison. Documentation-only edits need formatting, link and whitespace
checks, not an emulator rebuild:

```sh
npm run fmt:check
git diff --check
```
