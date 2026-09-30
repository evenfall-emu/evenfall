# Agent entry point

Evenfall is a family of deterministic, cycle-aware emulators of classic
consoles in one Rust codebase. Its first system is the Game Boy Advance, a
port of [ZiGBA](https://github.com/evenfall-emu/zigba), which stays the
oracle the Rust core must match exactly. Durable project knowledge lives in
`docs/`.

## Read before acting

| Task                                 | Read                                                                                            |
| ------------------------------------ | ----------------------------------------------------------------------------------------------- |
| Anything                             | [`docs/status.md`](docs/status.md); the index is [`docs/README.md`](docs/README.md)             |
| How work is done and checked         | [`docs/methodology.md`](docs/methodology.md)                                                    |
| GBA behavior                         | [`docs/architecture.md`](docs/architecture.md), then [`docs/gba/hardware/`](docs/gba/hardware/) |
| Porting or parity with ZiGBA         | [`docs/parity.md`](docs/parity.md) and the porting rules below                                  |
| Performance research or benchmarking | [`docs/performance.md`](docs/performance.md)                                                    |
| Save states and the web runtime      | [`docs/design/`](docs/design/)                                                                  |

Until a page exists here, its ZiGBA counterpart in `refs/zigba/docs/` is the
reference (`tools/setup-refs.sh zigba` fetches the pinned checkout).

## Layout

- `crates/`: code shared across systems, split by concern: `host` (the
  `System` trait every system implements), `state` and `state-derive` (the
  canonical save-state codec and envelope), `audio` (sample types and the
  resampler), `cpu-arm` (the ARM7TDMI interpreter), `replay` (timestamped
  input logs and scenarios).
- `systems/gba/`: the Game Boy Advance core, `evenfall-gba`.
- `web/gba/`: the GBA WebAssembly ABI crate and the `@evenfall/gba` JS SDK.
- `tools/`: the ZiGBA parity harness, benchmarks and profiling helpers.
- `probe/`: bare-metal probe ROM sources; `docs/`: documentation.
- `refs/`: pinned external checkouts (gitignored); `artifacts/`: gitignored
  measurements.

A crate exists only when it holds real code. Shared crates are extracted from
concrete uses; there is no placeholder for a future system, JIT or GPU, and
no cross-ISA CPU trait.

## Standing constraints

- Implement original code. ZiGBA is this project's own predecessor and is
  ported directly; pinned third-party emulators (mGBA 0.10.5,
  NanoBoyAdvance, VBA-M) are comparisons, never sources to copy.
- Keep ROMs, BIOS images, keys and saves user-supplied and gitignored. Never
  commit or download them.
- Preserve exact guest behavior: timed fetches, event ordering, audio and
  full rendering. Results never depend on how a host slices execution.
- Every optimized path keeps a reference path in the tree and a test that
  compares them.
- Stable Rust only, pinned in `rust-toolchain.toml`: no `#![feature]`, no
  `-Z` flags, no `portable_simd`. The browser build is single-threaded.
- License: AGPL-3.0-or-later.

## Porting from ZiGBA

The pinned ZiGBA checkout (`refs/zigba`, commit
`add59e5957b75f29d084813336ee20e300fc0b6e`) is read-only. Never change it;
a hook the parity harness needs is a patch the harness applies.

- **Translate faithfully.** One Zig file becomes one Rust module (large
  files may split into submodules by area). Keep names (`camelCase`
  functions become `snake_case`, types keep their names), algorithms, and the
  order of every observable effect. The port is not the place to fix or
  speed up behavior: parity with ZiGBA gates the work.
- **Diverge deliberately.** A ZiGBA bug or a large performance opportunity
  goes into [`docs/roadmap.md`](docs/roadmap.md) with its evidence. Change
  behavior only with a recorded decision, and never silently.
- **Port tests with their oracles.** A ZiGBA test becomes a Rust test with
  the same meaning, a `snake_case` name derived from the Zig name, and the
  original name in its doc comment. Expected values are copied from the Zig
  test or re-derived from the same outside source; never regenerate them from
  the Rust code's output.
- **Integers.** A Zig `uN` or `iN` of a non-byte width is stored in the
  smallest Rust integer that holds it, and masked where Zig would truncate or
  wrap. `+%`, `-%` and `*%` become `wrapping_*`; plain Zig arithmetic stays
  plain, so debug builds trap the same overflows Zig's safe modes do.
- **Control flow.** Error unions become `Result` with module error enums,
  tagged unions become enums, `comptime` specialization becomes const
  generics or macros, `anytype` becomes a generic with a trait bound,
  `std.debug.assert` becomes `debug_assert!` and `unreachable` becomes
  `unreachable!()`.

## Save-state codec

The GBA core reads and writes ZiGBA's `ZIGBAST1` save states byte for byte.
`crates/state` reproduces ZiGBA's reflection-driven codec with
`#[derive(Codec)]`:

- A durable struct lists its fields in ZiGBA's declaration order, with
  ZiGBA's field names, because both the payload and the schema digest follow
  that order. Rust's memory layout is unaffected.
- Non-byte integer widths carry `#[codec(bits = N)]`; enums carry their Zig
  tag width; variant names convert to Zig's `snake_case` names.
- Transient fields (caches, host bookkeeping) carry `#[codec(skip)]` exactly
  where ZiGBA's `isTransientStateField` lists them.
- The schema digest computed from the Rust types must equal ZiGBA's version 6
  digest; a mismatch means the durable layout differs.

## Rust conventions

- Edition 2024; `cargo fmt` with the repository's `rustfmt.toml`;
  `cargo clippy --all-targets -- -D warnings` clean.
- Hot paths pin inlining that LLVM would decide near its threshold. After
  changing the CPU engine, bus device step, DMA, APU mixer or compositor, run
  `node tools/bench/inline-stability.mjs`; it must exit 0.
- Hot paths are monomorphized: no `dyn`, no allocation, no `Rc`/`RefCell`.
  A `Machine` allocates only at construction; arrays of 4 KiB or more live in
  boxes allocated without stack temporaries.
- `unsafe` needs a measured reason and a `// SAFETY:` comment. The workspace
  denies `unsafe_code`; allow it only on the module that has the reason.
  Prefer masks and fixed-size arrays that let the compiler drop bounds
  checks.
- Profiles: `dev` (opt-level 1, dependencies at 3) runs games at playable
  speed; `release` is the optimized day-to-day build without LTO; `profiling`
  adds full debug info for `perf`; `wasm-release` is the only LTO profile.

## Checking work

```sh
cargo fmt --all --check
cargo clippy --workspace --all-targets -- -D warnings
cargo test --workspace
cargo test --workspace --release        # some bugs only show when optimized
```

The parity harness, ROM validators and WebAssembly build are described in
[`docs/testing.md`](docs/testing.md).

## Writing documentation

Documentation describes the current system in the present tense: behavior,
the evidence for it, and open questions. Do not add dates, commit or binary
hashes, test counts, recorded run numbers or narratives of how something was
found or fixed; that history belongs in commit messages. Performance numbers
belong only in `docs/performance.md` and `docs/research/`.
