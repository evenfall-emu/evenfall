# Status

Evenfall has one system so far, the Game Boy Advance.
The core models every major part of the machine and is deterministic: the same ROM, BIOS and timestamped inputs give the same frames, audio samples, battery saves and save-state bytes every time, however the host slices execution.
It passes the public and in-house test ROMs, but evidence across real games is still thin, and nothing has been checked against captures from physical hardware.

## What works

- **GBA core** (`systems/gba`): timed ARM and Thumb execution through a pre-decoded execution engine, with the interpreter as its reference; bus timing with wait states, open bus and Game Pak prefetch; video modes 0 to 5 with raster effects; PSG and Direct Sound audio; timers, DMA, interrupts, HALT and STOP; every save chip family, with autodetection; RTC, solar, gyro, tilt and rumble cartridges; serial link modes including MultiBoot; HLE, skip-BIOS and external BIOS boot; save states and rewind.
- **Shared crates**: the `System` interface every system implements (`crates/host`), the state codec and envelope (`crates/state`), the ARM7TDMI interpreter (`crates/cpu-arm`), the resampler (`crates/audio`), and scenarios and traces (`crates/replay`).
- **Hosts**: the headless runner, which also serves GDB clients ([`cli.md`](cli.md)), and a WebAssembly module with the `@evenfall/gba` JavaScript and TypeScript SDK ([`sdk.md`](sdk.md)).
- **Web player**: a game library and player in the browser, with playthroughs, autosave, quick slots, rewind, offline use and optional cloud backups ([`player.md`](player.md)), and the Cloudflare Worker behind the hosted instance ([`server.md`](server.md)).

## How it is checked

- **Test ROMs.** Every pinned [jsmolka/gba-tests](https://github.com/jsmolka/gba-tests) ROM and every project probe ROM passes under HLE boot.
  Under the open Cult-of-GBA BIOS, the jsmolka BIOS read-protection case fails and the others pass ([`compatibility.md`](compatibility.md)).
- **Hardware tables.** The recorded results of the [mGBA test suite](https://github.com/mgba-emu/suite) run as ordinary tests, and every value matches.
  The suite's own ROM also runs, under HLE boot and the replacement BIOS.
  Under HLE, every test passes except three BIOS routine timings and two edge cases ([`compatibility.md`](compatibility.md#mgba-suite-rom)).
- **Fast paths against references.** Every optimized path keeps a simple reference path, and a test compares the two ([`performance.md`](performance.md#where-the-speed-comes-from)).
- **Native against WebAssembly.** Both builds produce identical state, pixels, audio, memory and saves ([`testing.md`](testing.md)).

## Performance

On our benchmark ROMs, the core takes about half of mGBA 0.10.5's time natively and is faster on every workload, while drawing every frame and emulating audio exactly.
It takes about a third of the time of the VBA-M core that the Delta iOS emulator ships.
A pre-decoded execution engine runs ARM and Thumb code, and the compositor draws lines with SIMD lanes (SSE2 on x86-64, NEON on AArch64, `simd128` in WebAssembly).
Results and method are in [`performance.md`](performance.md).

## Known issues

- **A DMA stall's first second runs inside one host call.** A DMA burst that never releases the bus runs inside the CPU access it follows until it has run 2^24 cycles.
  The host call it starts in returns about one second late, and the instruction it interrupted completes before the stall instead of after it.
  From then on the CPU stalls while video and audio keep running, as on hardware ([`gba/hardware/timers-dma-interrupts.md`](gba/hardware/timers-dma-interrupts.md#bursts-that-never-release-the-bus)).
- **The MP2K sound-driver HLE runs inside one step.** A cyclic channel chain, or a track that jumps without waiting, would hold the host call.
  This comes from reading the code and has not been reproduced ([`gba/hardware/bios.md`](gba/hardware/bios.md)).
- **HLE BIOS routine timing.** `Sqrt` and `CpuFastSet` take fewer cycles under HLE than the suite records for Nintendo's BIOS ([`gba/hardware/bios.md`](gba/hardware/bios.md#approximations)).

## Unverified choices

Where no source settles a behavior, the core makes a documented choice.
The most consequential are timer prescalers as taps of one shared free-running counter ([`gba/hardware/timers-dma-interrupts.md`](gba/hardware/timers-dma-interrupts.md)) and background fetches past the BG side of VRAM reading as zero ([`gba/hardware/video.md`](gba/hardware/video.md)).
Each [hardware page](gba/hardware/README.md) ends with its open questions.

## What's next

Broader compatibility evidence (representative games, hardware captures), fixes for the issues above, and then the next systems.
Graphical frontends will also share an optional presentation layer with shader presets, outside the deterministic core ([#1](https://github.com/evenfall-emu/evenfall/issues/1)).
