# Status

Evenfall has one system so far, the Game Boy Advance.
The core models every major part of the machine and is deterministic: the same ROM, BIOS and timestamped inputs give the same frames, audio samples, battery saves and save-state bytes every time, however the host slices execution.
It passes the public and in-house test ROMs, but evidence across real games is still thin, and nothing has been checked against captures from physical hardware.

## What works

- **GBA core** (`systems/gba`): timed ARM and Thumb execution through a pre-decoded execution engine, with the interpreter as its reference; bus timing with wait states, open bus and Game Pak prefetch; video modes 0 to 5 with raster effects; PSG and Direct Sound audio; timers, DMA, interrupts, HALT and STOP; every save chip family, with autodetection; RTC, solar, gyro, tilt and rumble cartridges; serial link modes including MultiBoot; HLE, skip-BIOS and external BIOS boot; save states and rewind.
- **Shared crates**: the `System` interface every system implements (`crates/host`), the state codec and envelope (`crates/state`), the ARM7TDMI interpreter (`crates/cpu-arm`), the resampler (`crates/audio`), and scenarios and traces (`crates/replay`).
- **Hosts**: the headless runner ([`cli.md`](cli.md)), and a WebAssembly module with the `@evenfall/gba` JavaScript and TypeScript SDK ([`sdk.md`](sdk.md)).

## How it is checked

- **Test ROMs.** Every pinned [jsmolka/gba-tests](https://github.com/jsmolka/gba-tests) ROM and every project probe ROM passes under HLE boot.
  Under the open Cult-of-GBA BIOS, the jsmolka BIOS read-protection case fails and the others pass ([`compatibility.md`](compatibility.md)).
- **Hardware tables.** The recorded results of the [mGBA test suite](https://github.com/mgba-emu/suite) run as ordinary tests.
  A few timing cases still differ ([`compatibility.md`](compatibility.md)).
- **Fast paths against references.** Every optimized path keeps a simple reference path, and a test compares the two ([`performance.md`](performance.md#where-the-speed-comes-from)).
- **Native against WebAssembly.** Both builds produce identical state, pixels, audio, memory and saves ([`testing.md`](testing.md)).

## Performance

On our benchmark ROMs, the core takes about half of mGBA 0.10.5's time natively and is faster on every workload, while drawing every frame and emulating audio exactly.
A pre-decoded execution engine runs ARM and Thumb code, and the compositor draws lines with SIMD lanes (SSE2 on x86-64, NEON on AArch64, `simd128` in WebAssembly).
Results and method are in [`performance.md`](performance.md).

## Known issues

- **DMA that never releases the bus** ends in a terminal `dma_livelock` fault after about one second of guest time.
  On hardware the CPU stalls while video and audio keep running.
- **The MP2K sound-driver HLE runs inside one step.** A cyclic channel chain, or a track that jumps without waiting, would hold the host call.
  This comes from reading the code and has not been reproduced ([`gba/hardware/bios.md`](gba/hardware/bios.md)).
- **BIOS read protection is modeled only in HLE mode.** With a BIOS image, reads from outside the BIOS return the image's bytes instead of the last opcode the BIOS fetched ([`gba/hardware/memory.md`](gba/hardware/memory.md)).
- **Timing differences** remain against the mGBA suite's recorded results for DMA with prefetch enabled, mixed wait states and DMA from EWRAM code ([`compatibility.md`](compatibility.md)).

## Unverified choices

Where no source settles a behavior, the core makes a documented choice.
The most consequential are timer prescalers as taps of one shared free-running counter ([`gba/hardware/timers-dma-interrupts.md`](gba/hardware/timers-dma-interrupts.md)) and background fetches past the BG side of VRAM reading as zero ([`gba/hardware/video.md`](gba/hardware/video.md)).
Each [hardware page](gba/hardware/README.md) ends with its open questions.

## What's next

Broader compatibility evidence (the mGBA suite ROMs, representative games, hardware captures), fixes for the issues above, and then the next systems.
Graphical frontends will also share an optional presentation layer with shader presets, outside the deterministic core ([#1](https://github.com/evenfall-emu/evenfall/issues/1)).
