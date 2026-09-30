# Agent entry point

Evenfall is a family of deterministic, cycle-aware emulators of classic consoles in one Rust codebase.
Its first system is the Game Boy Advance.
Emulated behavior follows hardware evidence: Arm's manuals, hardware measurements, GBATEK and test suites' recorded hardware results.
Other emulators are comparisons, never authorities.
The GBA core began as a port of ZiGBA, the project's earlier emulator in Zig, which no longer gates any change.
Public documentation lives in `docs/`, internal notes in `internal-docs/`.

## Read before acting

| Task                             | Read                                                                                                    |
| -------------------------------- | ------------------------------------------------------------------------------------------------------- |
| Anything                         | [`docs/status.md`](docs/status.md), and the index [`docs/README.md`](docs/README.md)                    |
| How work is checked              | [`docs/testing.md`](docs/testing.md) and [`internal-docs/methodology.md`](internal-docs/methodology.md) |
| GBA behavior                     | [`docs/architecture.md`](docs/architecture.md), then [`docs/gba/hardware/`](docs/gba/hardware/)         |
| Changing emulated behavior       | [Changing emulated behavior](#changing-emulated-behavior), then the hardware page that owns it          |
| Performance work or benchmarking | [`docs/performance.md`](docs/performance.md) and [`tools/bench/README.md`](tools/bench/README.md)       |
| Save states, rewind, cheats, web | [`docs/design/`](docs/design/)                                                                          |
| Web player or its server         | [`docs/player.md`](docs/player.md) and [`docs/server.md`](docs/server.md)                               |
| Plans and known follow-ups       | [`internal-docs/roadmap.md`](internal-docs/roadmap.md)                                                  |

`internal-docs/` is a private Git submodule holding the methodology, the roadmap, measurement logs and investigations.
Fetch it with `git submodule update --init internal-docs`.
A change there is a commit inside the submodule, then a commit in Evenfall that bumps the submodule pointer.

## Layout

- `crates/`: code shared across systems, split by concern.
  `host` defines the `System` trait every system implements, `state` and `state-derive` the save-state codec and envelope, `audio` the sample types and resampler, `cpu-arm` the ARM7TDMI interpreter, and `replay` timestamped input logs and scenarios.
- `systems/gba/`: the Game Boy Advance core, `evenfall-gba`, with its command-line runner and GDB stub (`evenfall-gba gdb`).
- `web/gba/`: the GBA WebAssembly crate and the `@evenfall/gba` JavaScript SDK.
- `web/player/`: the web player, `@evenfall/player` (React and Vite on the SDK).
- `web/gba-game-plugins/`: `@evenfall/gba-game-plugins`, the player's game plugins (Pokémon panels so far).
- `web/server/`: `@evenfall/server`, the Cloudflare Worker behind the hosted player (catalog, accounts, cloud saves, shares, patch uploads), with its D1 migrations.
- `web/cloudflare/`: the Worker's deployment configuration, the static headers, the privacy notice and `build.sh`, which builds the deployed site.
- `tools/`: the scenario runner (slice independence, the mGBA suite ROM), the mGBA suite ROM build, benchmarks and profiling helpers, and `setup-refs.sh`.
- `probe/`: bare-metal probe ROM sources.
- `docs/`: public documentation.
- `refs/`: pinned external checkouts, gitignored and listed in [`refs/README.md`](refs/README.md).
  `tools/setup-refs.sh` fetches them, and fetches `refs/zigba` only when named, for the benchmark's opt-in `zigba` adapter.

A crate exists only when it holds real code.
Shared crates are extracted from concrete uses, with no placeholder for a future system, JIT or GPU, and no cross-ISA CPU trait.

## Standing constraints

- Write original code.
  The emulators pinned in `refs/` (mGBA 0.10.5, NanoBoyAdvance, Delta's VBA-M core) are comparisons, never sources to copy or translate.
- Keep ROMs, BIOS images, keys and saves user-supplied and gitignored.
  Never commit or download them.
- Keep emulation deterministic.
  The same ROM, BIOS, configuration and timestamped inputs give the same pixels, samples, battery saves and save states, however a host slices execution.
- Keep guest behavior exact: every fetch timed, events in hardware order, full rendering and sample-accurate audio.
  Behavior changes only on evidence ([below](#changing-emulated-behavior)), never for speed.
- Every optimized path keeps a reference path in the tree and a test that compares them.
- Stable Rust only, pinned in `rust-toolchain.toml`: no `#![feature]`, no `-Z` flags, no `portable_simd`.
  The browser build is single-threaded.
- Keep deployed identifiers: the Worker name `zigba`, D1 `zigba-catalog`, R2 `zigba-saves` and its key layout.
  The legacy `X-Zigba-*` headers stay accepted and sent by the Worker, and the `LEGACY` browser names in `web/player/src/storage-names.ts` stay as migration sources, until the roadmap retires them.
  Renaming one strands deployed data, so it needs a migration, not an edit.
- Never run Wrangler against remote resources (`deploy`, `--remote`) while working; local `wrangler dev` and Miniflare are fine.
- License: AGPL-3.0-or-later.

## Changing emulated behavior

- **Find the evidence first.** Sources rank, highest first: Arm's manuals, hardware measurements, GBATEK, test suites' recorded hardware results (the mGBA suite, jsmolka's ROMs), then other emulators.
  Agreement between emulators alone is not proof.
  The ranking is in [`docs/gba/hardware/README.md`](docs/gba/hardware/README.md#sources).
- **Write the test before the change.** Its expected values come from that evidence, from arithmetic or from a probe ROM's design, never from Evenfall's own output.
  Choose inputs where a plausible wrong model gives a different result, and check that the test fails on the unchanged code.
  Test at the narrowest place that still observes the behavior (normally `Machine`, `Bus` or the owning device), and add a probe ROM in `probe/` with its expected result in the validator when only an end-to-end run can show it.
- **Record the decision.** Update the hardware page that owns the behavior: what it does, its source, and an open question where no source settles it.
  Name the evidence in the commit message.
  A known gap or a follow-up goes in [`internal-docs/roadmap.md`](internal-docs/roadmap.md).
- **Keep the whole machine consistent.** Account for register effects, cycle costs, event order, open bus, video and audio output, and save-state persistence.
  A change to a stored field is a new save-state format ([below](#save-state-codec)).
- **Integers.** A register narrower than a byte lives in the smallest integer that holds it, masked where the hardware truncates.
  Hardware wraparound uses `wrapping_*`, and other arithmetic stays plain, so debug builds trap unexpected overflow.
- **Doc comments on tests** say in plain words what the test checks beyond its name.

## Save-state codec

The GBA core writes Evenfall's own save states: magic `EVENFALL`, version 7.
`crates/state` derives the codec with `#[derive(Codec)]`, and the payload has a fixed layout, so a state's length depends only on the save type:

- Durable fields encode in declaration order, little-endian, each integer in the fewest whole bytes its width needs.
  The schema digest covers every field's name and type, so adding, removing, reordering, renaming or retyping a durable field changes it.
  Rust's memory layout is unaffected.
- Every `Option` and union takes a fixed slot: a presence or tag byte, then the value, zero-filled to the largest payload.
  Payload types need a fixed length (`Codec::ENCODED_LEN`), and the build fails otherwise.
- The one field whose length depends on its value, the save medium, carries `#[codec(last)]` so that nothing after it moves.
- Non-byte integer widths carry `#[codec(bits = N)]` and fieldless enums `#[codec(tag_bits = N)]`.
  Variant names appear in the schema in `snake_case`.
- Transient fields (caches, host bookkeeping, anything rebuilt on load) carry `#[codec(skip)]`.
- `systems/gba/tests/fixtures/state-layout.txt` records the version, the schema digest and each field's offset and length.
  A layout change is a new format: bump the state version (`VERSION` in `crates/state/src/envelope.rs`, which `Machine::STATE_VERSION` reads) and the rewind bundle version in `crates/state/src/rewind.rs` (bundles embed states), then regenerate the fixture as its test describes.
  States of other versions stop loading.

## Rust conventions

- Edition 2024, `cargo fmt` with the repository's `rustfmt.toml`, and `cargo clippy --all-targets -- -D warnings` clean.
- Hot paths pin inlining that LLVM would decide near its threshold.
  After changing the CPU engine, bus device step, DMA, APU mixer or compositor, run `node tools/bench/inline-stability.mjs`, which must exit 0.
- Hot paths are monomorphized: no `dyn`, no allocation, no `Rc` or `RefCell`.
  A `Machine` allocates only at construction, and arrays of 4 KiB or more live in boxes allocated without stack temporaries.
- `unsafe` needs a measured reason and a `// SAFETY:` comment.
  The workspace denies `unsafe_code`, so allow it only on the module that has the reason.
  Prefer masks and fixed-size arrays that let the compiler drop bounds checks.
- Profiles:
  - `dev` (opt-level 1, dependencies at 3) runs games at playable speed.
  - `release` is the optimized day-to-day build: no LTO, incremental.
  - `release-lto` (fat LTO, one codegen unit) is what the benchmarks measure, the native twin of the shipped WebAssembly.
  - `profiling` is `release-lto` with full debug info, for `perf`.
  - `wasm-release` (fat LTO, stripped) builds the shipped WebAssembly.
- Benchmark and timing builds use rustup's cargo, `~/.cargo/bin/cargo`, with no `RUSTC_WRAPPER`.
  A cargo wrapper earlier on `PATH` can compile LTO profiles incrementally, which makes the code 1-3% slower.
  The tools in `tools/bench/` already do this.
- Benchmarks take a host-wide lock so two never overlap.
  `EVENFALL_LANE_LOCK` names the lock file, and `EVENFALL_LANE_HELD` tells a nested run that its parent already holds it.

## Checking work

Rust, from the repository root:

```sh
cargo fmt --all --check
cargo clippy --workspace --all-targets -- -D warnings
cargo test --workspace
cargo test --workspace --release        # some bugs only show when optimized
```

Test ROMs and slice independence, which need `arm-none-eabi-gcc`:

```sh
tools/setup-refs.sh gba-tests cult-gba-bios suite
make -C probe
make -C tools/bench/roms
cargo test -p evenfall-gba --test validator -- --require-roms
tools/mgba-suite/build.sh               # needs podman or Docker
cargo run --release -p evenfall-scenarios -- run --all
```

Build the probes before the validator, or new probes are skipped.
The scenario runner runs every manifest in `tools/scenarios/manifests/` under several slice sizes and requires identical results.

JavaScript and the WebAssembly build, with Node.js 24.11 or later:

```sh
npm ci
vp fmt --check                          # vp fmt fixes formatting
vp check                                # format, Oxlint and TypeScript
npm run build:wasm -w @evenfall/gba
vp test                                 # Vitest: web/gba, the player, plugin and server, tools/bench
```

The player and its plugin also run in Chromium with Playwright, and the Cloudflare build repeats the site's checks.
Run these after changing `web/`, including `web/gba`:

```sh
npm run build -w @evenfall/player
npm run test:browser -w @evenfall/player
npm run test:browser -w @evenfall/gba-game-plugins
bash web/cloudflare/build.sh            # then dry-run the Worker:
npx wrangler deploy --dry-run --config web/cloudflare/wrangler.jsonc
```

Use `npx vp` when Vite+ isn't installed globally.
Never run Prettier, `tsc` or `node --test` directly, since the npm scripts are aliases of the `vp` commands.

After changing a hot path, also run `node tools/bench/inline-stability.mjs`.
A documentation-only change needs `vp fmt --check` and `git diff --check`.
[`docs/testing.md`](docs/testing.md) describes every layer.

## Writing documentation

`docs/` is public.
Write it for an engineer who wants to understand how Evenfall works inside and why it is built that way.

- Open each page with what the thing does and an overview of how it works.
  Then explain each part and why it is built that way.
  Formats and edge cases come last.
- Explain a setting by its effect, and say when you would change it.
- Edit, don't append.
  When you add something, rewrite the section so it reads as one piece and delete what it replaces.
  Say each fact once, on the page that owns it.
- One claim per sentence, and no semicolons.
  Keep paragraphs short.
  Tables are for short, parallel facts.
  When a cell needs a sentence, use a list or subsections.
- State what is true instead of hedging.
  Put real limits in a "Known limits" section.
  Keep an open question only if someone could act on it.
- Write one sentence per line in the Markdown source.
  Rendered pages are unchanged, and a diff shows exactly which sentence changed.
- Write `2x` and `a * b`, not `×`.
- Describe the current system in the present tense.
  No dates, commit hashes, test counts or stories of how something was found.
  That history belongs in commit messages.
- Public docs don't mention ZiGBA.
  Compare with open references (mGBA, NanoBoyAdvance, GBATEK) instead.
- Performance numbers belong in `docs/performance.md` and `docs/research/`, and only current results.
  A number that helps someone choose a setting may stay next to the setting.
- Experiment logs (pinned-CPU runs, before/after tables, rejected trials), plans and investigations go into `internal-docs/`, which mirrors `docs/`'s depth so links into the source tree still resolve.
