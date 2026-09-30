# Evenfall documentation

New here? Read [`status.md`](status.md) for what works and what does not,
then pick the section below that matches what you want to do.

## Using Evenfall

- [`cli.md`](cli.md): the headless GBA runner and its GDB remote stub.
- [`sdk.md`](sdk.md): the `@evenfall/gba` JavaScript and TypeScript SDK.
- [`devtools.md`](devtools.md): deterministic scenarios, checkpoint assertions
  and failure replay.
- [`game-bridges.md`](game-bridges.md): typed, read-only views of game memory.
- [`compatibility.md`](compatibility.md): what is validated, and how to
  reproduce it.
- [`roadmap.md`](roadmap.md): what remains, deliberate differences from
  ZiGBA, and what moving the player and server here needs.

## Understanding and contributing

- [`methodology.md`](methodology.md): the evidence rules and working method
  behind every change.
- [`architecture.md`](architecture.md): the shared crates, the `System`
  interface, and the GBA core's boundaries, time model and source layout.
- [`parity.md`](parity.md): how the GBA core is kept byte-identical to ZiGBA,
  its executable oracle.
- [`gba/hardware/`](gba/hardware/README.md): how the GBA behaves and how the
  core models it, subsystem by subsystem.
- [`design/`](design/save-states.md): formats and runtime:
  [save states](design/save-states.md), [rewind](design/rewind.md) and the
  [web runtime](design/web-runtime.md).
- [`testing.md`](testing.md): the test layers, ROM validators and parity runs.
- [`../probe/README.md`](../probe/README.md): the bare-metal probe ROMs.
- [`../refs/README.md`](../refs/README.md): pinned external references.

## Performance

- [`performance.md`](performance.md): how the core is fast, how that stays
  exact, how it is measured against ZiGBA, and the build times.
- [`research/`](research/README.md): in-depth notes on specific mechanisms.
- [`../tools/bench/README.md`](../tools/bench/README.md): the benchmark
  runner, microbenchmarks and profiling recipe.
