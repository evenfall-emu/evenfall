# Architecture

Evenfall's core is a deterministic machine without host I/O. Hosts supply
media and timestamped input and observe pixels, PCM, saves, inspection and
portable state. This page describes the shared Rust crates and the GBA core.
How each GBA device behaves is described in [`gba/hardware/`](gba/hardware/README.md).

```text
CLI · native parity · WebAssembly adapter
                       │
                GBA Session (host policy)
                       │
              GBA Machine ── evenfall-host::System
                 │      │
        GBA Cpu ─┘      └─ GBA Bus (memory and time)
            │                  │
      cpu-arm Core       video · APU · timers · DMA · IRQ
                              serial · cartridge and save
```

## Shared crates and boundaries

[`evenfall-host`](../crates/host/src/lib.rs) defines `System`: a small
system-neutral contract for master cycles, bounded runs, timestamped input,
faults, completed BGR555 video, drained audio, battery save bytes, zero-cycle
inspection and transactional portable state. A system's native `Input` type
already carries its timestamp. CPU registers, cartridge sensors and link
cables are deliberately outside this trait; callers that need them use the
concrete GBA API. No cross-ISA CPU trait is required.

[`evenfall-cpu-arm`](../crates/cpu-arm/src/lib.rs) implements the ARM7TDMI
registers, pipeline and ARM/Thumb execution against a timed memory-access
trait. [`evenfall-audio`](../crates/audio/src/lib.rs) owns stereo sample types
and host-rate resampling. [`evenfall-state`](../crates/state/src/lib.rs) and
[`evenfall-state-derive`](../crates/state-derive/src/lib.rs) provide the
canonical codec, schema, envelope and rewind storage. The independent
[`evenfall-replay`](../crates/replay/src/lib.rs) records timed scenarios and
compares byte-exact observations without depending on an ISA.

[`Machine`](../systems/gba/src/machine.rs) is the GBA hardware seam and
implements `System`. It owns the CPU and Bus, chooses HLE, skip-BIOS or
external-BIOS boot, runs one step or a bounded slice, latches typed faults,
offers inspection and exposes state save/load. Construction allocates the
large machine directly on the heap; Session and Bus share immutable ROM and
BIOS handles, including during transactional scratch loads and resets.
`run_linked_until` schedules connected machines together. No filesystem,
audio device or wall clock belongs in `Machine`.

[`Bus`](../systems/gba/src/bus/mod.rs) owns the master clock, memory map,
wait states, prefetch, open bus, MMIO side effects and device scheduling. CPU
and DMA accesses carry their width, sequential classification and source;
the Bus brings devices to the access cycle and delivers events in order.
The GBA [`Cpu`](../systems/gba/src/cpu/mod.rs) owns architectural and HLE
continuation state, adapts `cpu-arm::Core` to Bus accesses and chooses between
the ordinary runner and one-step execution. Within the ordinary runner, the
[execution engine](../systems/gba/src/cpu/engine/mod.rs) runs code from RAM
and Game Pak ROM out of pre-decoded op caches until the next device event,
and the interpreter takes every instruction it refuses
([research](research/execution-engine.md)). The
[`HLE`](../systems/gba/src/hle/mod.rs) module supplies BIOS services and their
timed continuations without making them part of the generic ARM core.

[`Session`](../systems/gba/src/session.rs) is host policy around Machine. It
owns uploaded ROM/BIOS buffers, validates configuration, limits a call to at
most one frame, resamples PCM, manages rewind and reconstructs omitted video
on demand. [`VideoRecovery`](../systems/gba/src/video_recovery.rs) journals
bounded calls and replays them with full video on a raw copy of the machine
taken at the last presented frame, publishing only exact pixels; the live
CPU, audio and cycle do not change. Host mutations must pass through Session
while selective video is active so they enter its journal.

## Time and faults

`Cycle` is a GBA master-clock count: 16,777,216 Hz, 280,896 cycles per
frame. The Bus is its authority. CPU fetch, data and internal work consume
timed bus cycles; device deadlines, DMA requests and input are delivered at
their emulated cycles. Inputs due now apply before guest work; equal-cycle
inputs retain their caller order. A run may overshoot `yield_at` to complete
an atomic instruction and reports that boundary instead of hiding it.
`input_deadline` keeps future inputs visible across a host's internal slices;
`Slice::stop_at` preserves a caller's STOP wake horizon when Session splits at
a rendered frame boundary. HALT advances eligible devices; STOP freezes
CPU-clocked devices while external inputs and independent serial events may
wake it. Slicing must not change guest results.

Invalid guest behavior is a typed, cycle- and instruction-stamped
[`Fault`](../systems/gba/src/machine.rs). The fault is sticky: subsequent runs
report it without advancing time, and a faulted machine cannot write a save
state. A successful state load or fresh construction replaces the faulted
execution. Faults diagnose an unsupported or undefined operation rather than
guessing guest behavior.

## State and host interfaces

`Machine::write_state` and `write_state_alloc` produce the `ZIGBAST1`
version 6 envelope, bound to the exact ROM, BIOS, boot mode and durable
schema. `load_state` checks the envelope and hardware invariants in a
heap-allocated scratch machine before swapping it into service; failed loads
leave the old machine untouched. Hardware state includes the CPU, master
cycle, memory, devices and pending work. Host input cursors, output audio,
caches, resampler and rewind history are transient. See
[`design/save-states.md`](design/save-states.md) and
[`design/rewind.md`](design/rewind.md) for the formats. Cartridge battery
saves are distinct from portable states; choosing and atomically persisting a
save file belongs to [`host_save`](../systems/gba/src/host/host_save.rs).

[`evenfall-gba`](../systems/gba/src/bin/evenfall-gba.rs) is the headless native CLI
([usage](cli.md)); [`session_probe`](../systems/gba/src/bin/session_probe.rs)
is the native side of the native-against-WebAssembly comparison. The
[`web/gba`](../web/gba/README.md) WebAssembly ABI and JS host adapter call
Session rather than implementing hardware. Broader game compatibility remains
open; the source map describes the code present now.

## Rust source map

| Area                                                                                                                                                                                         | Responsibility                                             |
| -------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------- | ---------------------------------------------------------- |
| [`crates/host`](../crates/host/src/lib.rs), [`crates/replay`](../crates/replay/src/lib.rs)                                                                                                   | Generic system contract and scenario observation           |
| [`crates/cpu-arm`](../crates/cpu-arm/src/lib.rs), [`systems/gba/src/cpu`](../systems/gba/src/cpu/mod.rs)                                                                                     | ARM/Thumb ISA and GBA execution/HLE continuation seam      |
| [`systems/gba/src/hle`](../systems/gba/src/hle/mod.rs)                                                                                                                                       | BIOS emulation and timed services                          |
| [`systems/gba/src/machine.rs`](../systems/gba/src/machine.rs), [`systems/gba/src/bus`](../systems/gba/src/bus/mod.rs)                                                                        | Host-facing hardware control; timed memory and devices     |
| [`systems/gba/src/video`](../systems/gba/src/video/mod.rs), [`systems/gba/src/apu.rs`](../systems/gba/src/apu.rs)                                                                            | LCD timing, raster histories, rendering and sound hardware |
| [`systems/gba/src/timer.rs`](../systems/gba/src/timer.rs), [`systems/gba/src/dma.rs`](../systems/gba/src/dma.rs)                                                                             | Timers and DMA channel state                               |
| [`systems/gba/src/cartridge`](../systems/gba/src/cartridge/mod.rs), [`systems/gba/src/multiboot.rs`](../systems/gba/src/multiboot.rs)                                                        | Save protocols, GPIO, RTC, sensors and MultiBoot           |
| [`systems/gba/src/session.rs`](../systems/gba/src/session.rs), [`systems/gba/src/video_recovery.rs`](../systems/gba/src/video_recovery.rs)                                                   | Bounded host session, rewind and skipped-frame recovery    |
| [`crates/state`](../crates/state/src/lib.rs), [`crates/state-derive`](../crates/state-derive/src/lib.rs)                                                                                     | Portable codec, envelope, schema and rewind                |
| [`crates/audio`](../crates/audio/src/lib.rs), [`systems/gba/src/rom_image.rs`](../systems/gba/src/rom_image.rs), [`systems/gba/src/host/host_save.rs`](../systems/gba/src/host/host_save.rs) | Resampling, ROM import and native save-file policy         |
| [`systems/gba/src/bin`](../systems/gba/src/bin/evenfall-gba.rs)                                                                                                                              | Native hosts                                               |
| [`web/gba`](../web/gba/README.md)                                                                                                                                                            | GBA WebAssembly ABI, JS SDK and Worker host                |

Unit tests live next to their modules. Ported integration suites exercise the
Machine and Bus independently.
The checking layers and commands are in [`testing.md`](testing.md).
