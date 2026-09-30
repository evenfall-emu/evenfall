# Evenfall documentation

Start with [`status.md`](status.md) for what works and what doesn't.
Then pick what you came for.

## Using Evenfall

- [`cli.md`](cli.md): the headless GBA runner and its GDB remote stub.
- [`sdk.md`](sdk.md): the `@evenfall/gba` JavaScript and TypeScript SDK.
- [`player.md`](player.md): the web player, its library and its saves.
- [`server.md`](server.md): the Cloudflare Worker behind the hosted player, and its deployment.
- [`devtools.md`](devtools.md): deterministic scenarios, checkpoint assertions and failure replay.
- [`game-bridges.md`](game-bridges.md): typed, read-only views of game memory.
- [`compatibility.md`](compatibility.md): what is tested, and how to reproduce it.

## How it is built

- [`architecture.md`](architecture.md): the shared crates, the `System` interface, and the GBA core's parts, time model and source layout.
- [`gba/hardware/`](gba/hardware/README.md): how the GBA behaves and how the core models it, part by part.
- [`design/`](design/save-states.md): [save states](design/save-states.md), [rewind](design/rewind.md), the [web runtime](design/web-runtime.md) and [cheats](design/cheats.md).
- [`performance.md`](performance.md): how fast the core is, where the speed comes from, and how the fast paths stay exact.
- [`research/`](research/README.md): deeper notes on individual mechanisms.

## Working on it

- [`testing.md`](testing.md): the test layers and ROM validators.
- [`../probe/README.md`](../probe/README.md): the bare-metal probe ROMs.
- [`../refs/README.md`](../refs/README.md): pinned external references.
- [`../tools/bench/README.md`](../tools/bench/README.md): the benchmark runner, microbenchmarks and profiling.
