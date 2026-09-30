# Architecture

Evenfall is a family of console emulators in one Rust codebase, starting with the Game Boy Advance.
Its central idea is that the emulated console is a pure function of its inputs.
Given the same ROM, BIOS, settings and button presses at the same cycles, it produces the same pixels, audio samples, saves and save states, every time, on every host, however the host chooses to slice the work.

Everything else follows from that.
Save states and rewind are exact because nothing outside the machine affects it.
A bug report can be a file that reproduces the bug ([`devtools.md`](devtools.md)).
The browser build can be checked byte for byte against the native one.
And every fast path can be tested against a simple reference, because both must give identical results.

This page describes how the code is layered, how time flows through it, and where each piece lives.
How each part of the GBA behaves is in [`gba/hardware/`](gba/hardware/README.md).

## Layers

```text
 hosts:  CLI (evenfall-gba) · WebAssembly module + JS SDK · test tools
                               │
                               ▼
 Session       host policy: uploads, one frame per call, audio resampling,
               rewind, rebuilding skipped frames
                               │
                               ▼
 Machine       the console: boot, run, step, faults, save states
 (implements evenfall-host::System)
                 │                          │
                 ▼                          ▼
 Cpu            ARM7TDMI state,     Bus    master clock, memory map,
 (cpu-arm Core, execution engine,          wait states, device scheduling
  HLE BIOS)                                 │
                                            ▼
                               video · sound · timers · DMA · interrupts
                               serial · cartridge and save chip
```

Each layer only talks to the one below it, and only the top one touches the outside world.

- **The core has no I/O.**
  `Machine` never reads a file, a clock or an audio device.
  Hosts hand it ROM bytes and timestamped inputs, and read back pixels, samples, save bytes and states.
  That is what makes it deterministic and lets the same core run natively and in a browser.
- **`Session` holds host policy.**
  It decides how much to run per call, resamples audio to the host's rate, keeps rewind history, and skips drawing frames during fast-forward.
  None of that changes what the console does.
- **Hosts own the rest.**
  The CLI reads and writes files.
  The browser's Worker paces play against the wall clock, and the page draws, plays sound and stores saves ([`design/web-runtime.md`](design/web-runtime.md)).

## Shared crates

The `crates/` directory holds code meant to serve more than one console.
A crate is only extracted once there is real code to share, so there is no placeholder for future systems.

- [`evenfall-host`](../crates/host/src/lib.rs) defines `System`, the contract every emulated console implements: the current cycle, bounded runs with timestamped input, the sticky fault, the latest frame, audio samples, battery save bytes, save states, and reading guest memory without side effects.
  Things only one console has, such as CPU registers, cartridge sensors or link cables, stay out of it and are reached through that console's own API.
- [`evenfall-cpu-arm`](../crates/cpu-arm/src/lib.rs) is the ARM7TDMI interpreter: registers, pipeline, and ARM and Thumb execution against a timed memory trait.
  It knows nothing about the GBA, so the Nintendo DS's ARM7 could reuse it.
  There is deliberately no CPU trait shared across instruction sets.
- [`evenfall-state`](../crates/state/src/lib.rs) and [`evenfall-state-derive`](../crates/state-derive/src/lib.rs) are the save-state codec, its layout description, the file envelope and rewind storage ([`design/save-states.md`](design/save-states.md)).
- [`evenfall-audio`](../crates/audio/src/lib.rs) holds the stereo sample type and the resampler.
- [`evenfall-replay`](../crates/replay/src/lib.rs) records timed scenarios and compares exact observations, independent of any instruction set.

## The GBA core

[`Machine`](../systems/gba/src/machine.rs) is the console.
It owns the CPU and the bus, boots in HLE, skip-BIOS or real-BIOS mode, runs one instruction or a bounded slice, keeps a sticky fault, and saves and loads states.
The machine is large, so construction allocates it directly on the heap.
ROM and BIOS bytes are shared, immutable buffers, so the scratch machines used to check a state or reset don't copy them.
`run_linked_until` runs several machines together for link-cable play ([`serial.md`](gba/hardware/serial.md#linking-consoles)).

[`Bus`](../systems/gba/src/bus/mod.rs) owns time.
It holds the master clock, the memory map, wait states, the cartridge prefetch buffer and open bus, and it schedules every device.
Every CPU and DMA access carries its width, whether it is sequential, and who made it.
The bus brings devices up to the cycle of the access and delivers their events in order ([`memory.md`](gba/hardware/memory.md)).

[`Cpu`](../systems/gba/src/cpu/mod.rs) connects the generic `cpu-arm` core to the bus and holds the state of BIOS routines in progress.
Most code runs through the [execution engine](../systems/gba/src/cpu/engine/mod.rs), which runs pre-decoded instructions from RAM and cartridge ROM until the next device event.
The interpreter runs everything the engine refuses, and is the reference it is tested against ([`performance.md`](performance.md#execution-engine)).
The [HLE BIOS](../systems/gba/src/hle/mod.rs) implements the BIOS routines in Rust, outside the generic ARM core ([`bios.md`](gba/hardware/bios.md)).

[`Session`](../systems/gba/src/session.rs) wraps `Machine` with host policy.
It owns uploaded ROM and BIOS buffers, checks settings, runs at most one frame per call, resamples audio, keeps rewind history and rebuilds skipped frames on demand.
[`VideoRecovery`](../systems/gba/src/video_recovery.rs) does the rebuilding: it replays the calls since the last drawn frame, with drawing on, in a copy of the machine taken at that frame, and returns only exact pixels.
The live machine's CPU, audio and cycle don't change.
While frames are being skipped, every change a host makes must go through `Session`, so it is part of what gets replayed.

## How time flows

`Cycle` counts ticks of the GBA's master clock: 16,777,216 per second, 280,896 per frame.
The bus is the only authority on it.
Every instruction fetch, data access and internal cycle the CPU performs costs bus cycles, and device events, DMA requests and inputs all happen at exact cycles.

Here is what happens when a host asks for one frame:

1. The host calls `Session::advance` (or `run_slice_with_input` with inputs), and the session calls `Machine::run_slice` for at most one frame.
2. Inputs due now are applied before any guest code runs, and inputs at the same cycle keep the host's order.
3. The bus opens a batch.
   Devices don't advance on every access, only when something could observe them: an I/O access, a device event, or the end of the batch ([`performance.md`](performance.md#deferred-devices)).
4. The CPU runs instructions, through the execution engine where it can.
   Each run stops just before the next device event, so timers, DMA and the display see every access at its exact cycle.
5. At cycle 960 of each visible line, the display draws the whole line, replaying any register changes made during it ([`video.md`](gba/hardware/video.md#raster-effects)).
   Timer overflows feed sound samples, and DMA copies run inside the access they interrupt.
6. If the CPU halts, the bus skips straight to the next event that could wake it.
   STOP freezes everything clocked by the console, while inputs and link-cable transfers can still wake it.
7. The run ends at the requested cycle, or at the first instruction boundary after it, since an instruction can't be split.
   The result says exactly where it stopped, instead of hiding the overshoot.
8. The host takes the frame and drains the audio samples.

Two details keep slicing from changing results.
A run has a separate yield point and input deadline, so a host can split one request into several calls without changing which inputs apply where.
And when `Session` splits a run at a frame boundary, `Slice::stop_at` keeps the caller's STOP wake-up horizon.

## Faults

When a game does something the hardware doesn't define, such as an UNPREDICTABLE instruction or a DMA that never lets go of the bus, Evenfall doesn't guess.
The machine stops with a typed [`Fault`](../systems/gba/src/machine.rs), stamped with the cycle and instruction.

The fault is sticky.
Later runs report it without advancing time, and a faulted machine can't write a save state.
Loading a valid state or creating a new machine clears it.
The host is never harmed: a fault is data, not a crash ([`cpu.md`](gba/hardware/cpu.md#undefined-versus-unpredictable)).

## Saves and states

`Machine::write_state` writes a `ZIGBAST1` version 6 state, bound to the exact ROM, BIOS, boot mode and state layout.
`load_state` checks the file and the hardware state in a scratch machine, and only then swaps it in, so a failed load leaves the running game untouched.
A state holds everything that affects what the console does next: the CPU, the master cycle, memory, devices and pending work.
Input cursors, audio already produced, caches, the resampler and rewind history are left out.
[`design/save-states.md`](design/save-states.md) and [`design/rewind.md`](design/rewind.md) describe the formats.

The game's own battery save is separate from save states.
Choosing the save file and writing it safely belongs to the native host, in [`host_save`](../systems/gba/src/host/host_save.rs).

## Hosts

- [`evenfall-gba`](../systems/gba/src/bin/evenfall-gba.rs) is the command-line runner ([`cli.md`](cli.md)).
- [`web/gba`](../web/gba/README.md) is the WebAssembly module and the `@evenfall/gba` SDK ([`sdk.md`](sdk.md)).
  They call `Session` and implement no hardware themselves.
- [`session_probe`](../systems/gba/src/bin/session_probe.rs) is the native side of the test that compares the native and WebAssembly builds.

## Where the code lives

Shared crates:

- [`crates/host`](../crates/host/src/lib.rs): the `System` contract.
- [`crates/cpu-arm`](../crates/cpu-arm/src/lib.rs): the ARM7TDMI interpreter.
- [`crates/state`](../crates/state/src/lib.rs), [`crates/state-derive`](../crates/state-derive/src/lib.rs): save-state codec, envelope, layout and rewind storage.
- [`crates/audio`](../crates/audio/src/lib.rs): samples and resampling.
- [`crates/replay`](../crates/replay/src/lib.rs): scenarios and observations.

The GBA core, in `systems/gba/src/`:

- [`machine.rs`](../systems/gba/src/machine.rs): the console, boot, runs, faults and states.
- [`bus/`](../systems/gba/src/bus/mod.rs): the clock, memory map and device scheduling.
- [`cpu/`](../systems/gba/src/cpu/mod.rs): the GBA side of the CPU and the execution engine.
- [`hle/`](../systems/gba/src/hle/mod.rs): the replacement BIOS.
- [`video/`](../systems/gba/src/video/mod.rs): display timing, raster histories and drawing.
- [`apu.rs`](../systems/gba/src/apu.rs): the sound chip.
- [`timer.rs`](../systems/gba/src/timer.rs), [`dma.rs`](../systems/gba/src/dma.rs): timers and DMA channels.
- [`cartridge/`](../systems/gba/src/cartridge/mod.rs), [`multiboot.rs`](../systems/gba/src/multiboot.rs): save chips, the GPIO port, clock and sensors, and MultiBoot.
- [`session.rs`](../systems/gba/src/session.rs), [`video_recovery.rs`](../systems/gba/src/video_recovery.rs): host policy, rewind and rebuilding skipped frames.
- [`rom_image.rs`](../systems/gba/src/rom_image.rs), [`host/host_save.rs`](../systems/gba/src/host/host_save.rs): ROM import, the title database and native save files.
- [`bin/`](../systems/gba/src/bin/evenfall-gba.rs): the native programs.

The browser side is in [`web/gba`](../web/gba/README.md).
Unit tests sit next to their modules, and integration tests in `systems/gba/tests/` drive the machine and the bus from outside.
[`testing.md`](testing.md) describes how everything is checked.
