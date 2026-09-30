# Status

Evenfall has one system so far: a Game Boy Advance core ported from ZiGBA.
The port is exact. On every workload the parity harness runs, it produces
the same frames, audio samples, battery-save bytes and save-state bytes as
ZiGBA at the pinned commit, however execution is sliced, apart from
deliberate guest-visible differences that the harness applies to ZiGBA too
(listed [below](#deliberate-differences-from-zigba)). It otherwise inherits
ZiGBA's compatibility, and its limits: the core implements every
major GBA subsystem, but evidence across real games is thin and nothing has
been checked against captures from physical hardware. What complete would
mean is defined in [`methodology.md`](methodology.md#goals).

## What works

- **GBA core** (`systems/gba`): timed ARM and Thumb execution through a
  pre-decoded execution engine with the interpreter as its reference, bus
  timing with wait states, open bus and Game Pak prefetch; video
  modes 0–5 with raster effects; PSG and Direct Sound audio; timers, DMA,
  interrupts, HALT and STOP; every save chip family with autodetection; RTC,
  solar, gyro, tilt and rumble cartridges; serial link modes including
  MultiBoot; HLE, skip-BIOS and external BIOS boot; `ZIGBAST1` save states
  and rewind.
- **Shared crates**: the `System` interface every system implements
  (`crates/host`), the canonical state codec and envelope (`crates/state`),
  the ARM7TDMI interpreter (`crates/cpu-arm`), the resampler
  (`crates/audio`), and scenarios and traces (`crates/replay`).
- **Hosts**: the headless runner ([`cli.md`](cli.md)); a WebAssembly module
  with the `@evenfall/gba` JavaScript and TypeScript SDK ([`sdk.md`](sdk.md)),
  whose public API is ZiGBA's; the parity harness ([`parity.md`](parity.md));
  and the benchmark runner ([`performance.md`](performance.md)).

## Validation

- **Parity with ZiGBA.** Every scenario of the harness matches ZiGBA, with
  the deliberate differences below patched into its copy, byte for byte at
  every checkpoint: the jsmolka gba-tests, every probe ROM, the
  benchmark ROMs, the HLE, skip-BIOS and external-BIOS boot paths, and
  linked two- to four-machine cable runs, each under several host slicing
  schedules. For the one behavioral difference below, the oracle is ZiGBA
  with that difference applied as a patch ([`parity.md`](parity.md)).
- **Save-state layout.** The schema description the Rust types produce is
  byte-identical to ZiGBA's version 6, and ZiGBA's locked state fixture
  reproduces exactly, so existing `ZIGBAST1` states load
  ([`design/save-states.md`](design/save-states.md)).
- **ROM validators.** All 71 cases pass under HLE boot: 13 pinned
  `jsmolka/gba-tests` ROMs and 58 project probe ROMs. Under the open
  Cult-of-GBA BIOS, 12 of the 13 jsmolka ROMs pass; the BIOS
  read-protection case fails at its first check because that BIOS leaves
  boot through other opcodes than Nintendo's
  ([`compatibility.md`](compatibility.md#known-divergences)).
- **Ported tests.** ZiGBA's unit and integration tests are ported with their
  oracles, and the SDK's JavaScript tests run against the Rust WebAssembly
  build. Every optimized path is checked against its reference path, and
  native and WebAssembly builds produce identical state, pixels, audio,
  memory and saves ([`testing.md`](testing.md)).

## Performance

On the benchmark workloads the core takes about 0.4× ZiGBA's loop time
natively (half of mGBA 0.10.5's, and faster than mGBA on every workload) and
under a third of it in WebAssembly, with every frame and state byte still
identical to ZiGBA's. A pre-decoded execution engine runs ARM and Thumb code,
and a layer-major compositor renders lines with SIMD lanes (the `wide`
crate: SSE2 on x86-64, NEON on AArch64, `simd128` in the default WebAssembly
build). Results, method and the
remaining gaps are in [`performance.md`](performance.md).

## Deliberate differences from ZiGBA

Each is recorded with its evidence in [`roadmap.md`](roadmap.md). These
change guest behavior, and the parity harness patches its ZiGBA copy to
match:

- With a BIOS image, reads of the BIOS from outside it return the last
  opcode the BIOS fetched, as on hardware; ZiGBA returns the image's bytes
  ([`gba/hardware/memory.md`](gba/hardware/memory.md#bios-read-protection)).
- **DMA that never releases the bus** stalls the CPU instead of ending in
  ZiGBA's terminal `dma_livelock` fault after about one second of guest
  time. From then on video, audio, timers and interrupt flags keep running,
  host calls return at their deadlines, and the CPU resumes if the guest's
  DMA ever releases the bus
  ([`gba/hardware/timers-dma-interrupts.md`](gba/hardware/timers-dma-interrupts.md#bursts-that-never-release-the-bus)).
- **Write-only I/O registers** read as open bus, or as zero beside a
  readable half, as the mGBA suite's hardware table records; ZiGBA reads
  some back and others as zero
  ([`gba/hardware/memory.md`](gba/hardware/memory.md#open-bus-and-unused-io)).
- **Thumb open bus** carries the prefetched halfword in both halves from
  ROM and the other 16-bit buses, and follows GBATEK's rules for BIOS, OAM
  and IWRAM code; ZiGBA zero-extends it
  ([`gba/hardware/memory.md`](gba/hardware/memory.md#open-bus-and-unused-io)).
- **Flag-setting multiplies** set C as the ARM7TDMI's multiplier does,
  where ZiGBA leaves it unchanged
  ([`gba/hardware/cpu.md`](gba/hardware/cpu.md#the-carry-flag-after-a-multiply)).
- **HLE halfword `CpuSet`** copies like the retail loop's `LDRH` and
  `STRH`, rotating a halfword read from an odd source; ZiGBA aligns it
  ([`gba/hardware/bios.md`](gba/hardware/bios.md#copies)).
- **Block transfers on SRAM** present each beat's exact address to the
  8-bit save bus, as single transfers do; ZiGBA aligns them
  ([`gba/hardware/memory.md`](gba/hardware/memory.md#region-behavior)).

The others change no emulated result:

- Compressed rewind entries use a different DEFLATE encoder, so their bytes
  differ while ZiGBA and Evenfall read each other's bundles and decode
  identical states.
- ROM and BIOS media are shared and immutable; ZiGBA's borrowed, mutable
  media mode does not exist.
- The WebAssembly build keeps Rust's default allocator, which reuses freed
  large blocks, and fails caller-sized allocations with `OutOfMemory`.

## Known issues

Because the port is exact, ZiGBA's known issues are also Evenfall's, except
where a deliberate difference above replaces one:

- **A DMA stall's first second runs inside one host call.** A burst that
  never releases the bus is serviced inside the CPU access it follows until
  it has run 2^24 cycles, so the host call it starts in returns about one
  second late and the host loses that call's oldest audio; the instruction it
  interrupted then completes before the stall instead of after it. Save
  states are refused while the stall lasts
  ([`roadmap.md`](roadmap.md#dma-that-never-releases-the-bus)).
- **The MP2K sound-driver HLE runs inside one step.** A cyclic channel chain,
  or a track that jumps without waiting, would hold the host call. This is
  inferred from the code and has not been reproduced
  ([`gba/hardware/bios.md`](gba/hardware/bios.md)).
- **Timing residuals** remain against the mGBA suite's recorded results for
  DMA with prefetch enabled, mixed wait states and DMA from EWRAM code
  ([`compatibility.md`](compatibility.md)).

The player and the account server still live in ZiGBA's repository; what
moving them here needs is listed in [`roadmap.md`](roadmap.md).

## Unverified choices

Where no source settles a behavior, the core makes ZiGBA's documented choice.
The most consequential are timer prescalers as taps of one shared
free-running counter
([`gba/hardware/timers-dma-interrupts.md`](gba/hardware/timers-dma-interrupts.md))
and background fetches past the BG side of VRAM reading as zero
([`gba/hardware/video.md`](gba/hardware/video.md)). Each
[hardware page](gba/hardware/README.md) ends with its open questions.

## What's next

Broader compatibility evidence (the
mGBA suite ROMs, representative games, hardware captures), fixes for the
issues above, moving the player and server into this repository, and then
the next systems. See [`roadmap.md`](roadmap.md).
