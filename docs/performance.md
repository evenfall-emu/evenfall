# Performance

Evenfall's GBA core is cycle-accurate. Every fetch is timed, devices see
events in the order the hardware produces them, every frame is fully drawn
and audio is emulated sample by sample. On our benchmark ROMs it still runs
in about half of mGBA's time.

This page gives the current numbers, explains where the speed comes from and
how the fast paths stay exact, and shows how to measure. The
[research notes](research/README.md) go deeper into individual mechanisms.

## Results

### Native

Evenfall's loop time as a fraction of mGBA 0.10.5's on x86-64 (lower is
faster). Both emulators draw every frame and emulate audio, run one frame at
a time with no input and no save media, and reach the same final picture on
every workload. mGBA is its headless release build with idle-loop detection
at its default.

| Workload                                              | Frames |  HLE boot | Skip-BIOS boot |
| ----------------------------------------------------- | -----: | --------: | -------------: |
| Idle: HALT and an empty mode-0 frame                  |  3,600 |     0.36x |          0.41x |
| ARM mixer: ARM code from IWRAM                        |    600 |     0.41x |          0.41x |
| Thumb logic: Thumb code from Game Pak ROM             |    600 |     0.56x |          0.53x |
| Tiles: text backgrounds, sprites, effects and windows |    600 |     0.36x |          0.34x |
| Affine: affine backgrounds, sprites and HBlank DMA    |    600 |     0.78x |          0.86x |
| Bitmap DMA: page flips and small transfers            |    600 |     0.65x |          0.64x |
| Audio: Direct Sound FIFO DMA and PSG                  |    600 |     0.41x |          0.44x |
| **Geometric mean**                                    |        | **0.48x** |      **0.50x** |

HLE boot handles interrupts without any BIOS code. Skip-BIOS boot maps the
open Cult-of-GBA replacement BIOS and runs its interrupt dispatcher, which
the execution engine runs like any other ARM code.

Each ROM exercises one part of the machine and all of them weigh the same in
the mean, so the table doesn't predict how fast a particular game runs.

### WebAssembly

In Node 24, the shipped WebAssembly module takes 0.31x the loop time of the
module of our previous implementation,
[ZiGBA](https://github.com/evenfall-emu/zigba), on the same ROMs with HLE
boot. Built without SIMD lanes it takes 0.35x. The lanes matter most for
drawing tiles, where they bring the ratio from 0.41x down to 0.23x.

### Skipping frames

When the host fast-forwards, it can ask the core to draw only one frame in N
and skip drawing the others
([selective video](research/selective-video.md)). Microseconds per emulated
frame through `Session::advance`, native release build, host audio off:

| Workload    | Every frame | 1 in 4 | 1 in 16 |
| ----------- | ----------: | -----: | ------: |
| idle        |         5.6 |    8.0 |     4.7 |
| arm-mixer   |         132 |    133 |     131 |
| thumb-logic |         150 |    151 |     148 |
| tiles       |         159 |     77 |      52 |
| affine      |         274 |    205 |     184 |
| bitmap-DMA  |         260 |    231 |     221 |
| audio       |          75 |     77 |      74 |

The workloads that spend their time drawing (tiles, affine and bitmap DMA)
gain the most. The others draw very little and stay within a few percent.
Drawing one frame in four, a session uses 3.1 to 3.7 MiB of heap.

### Build times

Wall times for a cold build, and for a rebuild after a one-line change to
the body of the inlined `Bus::now`. Native builds compile the core, and
WebAssembly builds link the shipped web module.
[`build-times.sh`](../tools/bench/build-times.sh) reproduces them from a
snapshot of committed source, so it never touches your worktree.

These are captured from my Ultra X7 Panther Lake CPU, on a loaded host.

| Build                         |   Cold | One-line rebuild |
| ----------------------------- | -----: | ---------------: |
| dev                           | 10.0 s |            3.6 s |
| release (no LTO, incremental) |  6.5 s |            2.5 s |
| wasm-release (fat LTO)        |  9.5 s |            7.7 s |

The `wide` crate and its dependencies add about 4 s to a cold
release build and 5 s to a cold wasm-release build, and nothing to rebuilds.

Most of the rebuild time goes into the execution engine's run loops, which
are generated separately for each memory region. The two ARM loops, one for
IWRAM and one for EWRAM, take about 170 KB of the WebAssembly module. A
single loop taking the region as a value would save about 85 KB but runs 2%
slower on arm-mixer, so the core keeps both.

## Where the speed comes from

A cycle-accurate core spends most of its time on bookkeeping. The simple
way to run it is to decode every instruction, advance every device after
every bus access and draw every pixel one dot at a time. Most of that work
can't change anything the guest sees. Between two device events, most of
what the CPU does is invisible to the timers, the DMA and the display, and a
line that uses no raster effect looks the same whether it is drawn dot by dot
or in one pass.

Each fast path below exploits that. It batches work up to the next point
where the guest could notice a difference, such as a device event, a visible
pixel or an interrupt. When it can't prove that nothing is observable, it
hands over to the ordinary path, which stays in the tree as the reference.

Every fast path has a test that runs it and its reference side by side and
compares what the guest would see (state bytes, pixels, samples and timing).
The [parity traces](parity.md) check the same thing end to end. The whole
core, fast paths included, must produce the same bytes as our previous
implementation however the host slices execution.

### Deferred devices

The [`Bus`](../systems/gba/src/bus/mod.rs) doesn't step the APU and timers on
every access. It lets them run behind in a batch up to the next device event,
and brings them up to date only when something could observe them: an MMIO
access, an overflow with side effects, or a host read. A timer overflow that
only feeds the sound FIFO is left to
[`Apu::elapse_clocked`](../systems/gba/src/apu.rs), which stops at the first
refill request.

- Reference: `Bus::advance_eager` and `advance_step`.
- Checked by: `fifo_only_timer_overflows_match_eager_refill_and_irq_schedule`
  in [`bus/mod.rs`](../systems/gba/src/bus/mod.rs) and
  `synchronized_reads_and_fifo_writes_keep_the_device_batch_armed` in
  [`bus_b.rs`](../systems/gba/tests/bus_b.rs).

### DMA

A DMA beat to or from RAM, ROM or the sound FIFO goes straight to that
memory
([`Bus::read_dma_source` and `write_dma_destination`](../systems/gba/src/bus/mod.rs))
instead of through the generic `Bus::read` and `write`, with the same cost
and the same chance for a higher-priority DMA to step in between beats.

[`Bus::dma_bulk`](../systems/gba/src/bus/mod.rs) goes further: while no other
DMA is pending, it runs every beat that ends before the next device event in
one loop, from RAM or the latch to RAM, VRAM or OAM, as long as each address
stays in its region. Devices catch up at the end. A VRAM write during a
visible pixel goes through the per-beat path at its own cycle while the line's
raster history has room. Once the history is full, writes skip their history
snapshot.
VRAM beats that no visible pixel can see are compared and copied one
scanline-memo page at a time.

- Reference: the per-beat loop in `Bus::run_dma`, which also runs every DMA
  the bulk loop doesn't take.
- Checked by: `bulk_dma_beats_match_the_per_beat_loop`,
  `vram_dma_runs_match_the_per_beat_loop` and
  `vram_writes_skip_snapshots_only_when_history_is_full` in
  [`bus/mod.rs`](../systems/gba/src/bus/mod.rs), which compare state bytes,
  raster history and pixels, and `dma_at_lcd_boundary_preserves_deferred_batch`
  for timing.
- Gap: nothing switches the direct beat dispatch off to compare it with the
  generic one on its own.

### Execution engine

The [execution engine](../systems/gba/src/cpu/engine/mod.rs) runs ARM and
Thumb code from IWRAM and EWRAM, and Thumb code from Game Pak ROM, out of
caches of pre-decoded instructions. It charges each instruction exactly what
the interpreter would, and stops before the next device event. The program
counter, pipeline, CPSR, open bus and prefetch state are only written back
when a run stops.

Inside a run, straight ARM ALU code executes as blocks charged once, Thumb
ROM code follows the prefetch buffer in closed form, and RAM and video memory
accesses run inline. Some MMIO registers are served by the run itself
([`RunRegisters`](../systems/gba/src/bus/kernel.rs)): the interrupt
registers, DMA latches, LCD status, and the affine registers while no pixel
is being drawn. BIOS ARM code runs with the BIOS's fetch timing and read
protection.

Anything else goes to the interpreter: SWI, PSR and mode changes, stores into
code the engine has decoded, and MMIO the device batch can't take. The
[research note](research/execution-engine.md) explains the design.

On the dispatch diagnostics in [`tools/bench/diag`](../tools/bench/diag/README.md),
the engine retires about 20 host instructions per guest ARM ALU instruction
from IWRAM, 38 per ARM load or store, and 35 per Thumb ALU instruction from
ROM.

- Reference: `Cpu::step`, `Core::execute_arm` and `execute_thumb`.
- Checked by: randomized comparisons in
  [`engine/tests.rs`](../systems/gba/src/cpu/engine/tests.rs) and
  [`cpu/mod.rs`](../systems/gba/src/cpu/mod.rs), which compare registers,
  flags, clocks, memory, caches, raster history, prefetch state and where
  runs stop. Tests in `cpu/mod.rs` cover MMIO, DMA setup, IRQ entry and BIOS
  code, and `run_registers_match_the_bus` in
  [`bus/kernel.rs`](../systems/gba/src/bus/kernel.rs) covers the registers a
  run serves itself.

### Streamed Thumb fetches

When the engine refuses Thumb ROM code, the [`Bus`](../systems/gba/src/bus/mod.rs)
still streams sequential fetches for the interpreter and handles the common
refill after a branch in one step.

- Reference: the ordinary `GamePakPrefetch` fetch path.
- Checked by:
  `thumb_refills_match_two_ordinary_fetches_in_every_queue_phase_and_at_deadlines`
  in [`bus/fetch.rs`](../systems/gba/tests/bus/fetch.rs) and
  `bounded_thumb_rom_streams_match_eager_fetches_across_duty_boundaries_and_deadlines`
  in [`machine_a.rs`](../systems/gba/tests/machine_a.rs).

### Lazy PSG

While a square or noise channel is silent, the [`Apu`](../systems/gba/src/apu.rs)
only counts elapsed time, and it pushes runs of constant samples in bulk.

- Reference: the eager oscillator scheduler, kept as a test oracle in
  [`apu/tests.rs`](../systems/gba/src/apu/tests.rs).
- Checked by: `deferred_square_and_noise_oscillators_match_eager_endpoint_ordering`
  and `constant_sample_runs_match_the_eager_scheduler_across_silent_and_audible_psg`.

### Layer-major compositor

Most lines are drawn one layer at a time
([`video/compose.rs`](../systems/gba/src/video/compose.rs)): each background
into a row of colors, the objects into two rows, then one kernel selects,
merges and blends eight pixels at a time. The kernel is written against
[`Lanes`](../systems/gba/src/video/lanes.rs), the `wide` crate's `i16x8`:
SSE2 on x86-64, NEON on AArch64, `simd128` in the WebAssembly build that
enables it, scalar elsewhere.

Lines whose registers or video memory change while they are drawn (raster
history) use the front-to-back reference instead, except when the only
change is a VRAM write to a page the line doesn't read. The
[research note](research/layer-major-compositor.md) explains the design.

- Reference: `compose_scanline_reference`, with the per-dot and per-pixel
  renderers.
- Checked by: `layer_major_lines_match_the_front_to_back_reference` and
  `vram_history_selects_the_reference_only_for_pages_the_line_reads` in
  [`video/tests.rs`](../systems/gba/src/video/tests.rs), and
  `vector_lanes_match_the_scalar_kernel` in `video/compose.rs`.

### Text tiles

[`video/backgrounds.rs`](../systems/gba/src/video/backgrounds.rs) writes a
whole text tile at once, and a
[decoded palette](../systems/gba/src/video/tile_row_cache.rs) turns each 4bpp
byte into two colors per palette bank. The decoded palette is checked against
the palette's bytes before use
([research note](research/tile-row-cache.md)).

- Reference: the per-dot `render_text_background`.
- Checked by: `text_tile_rows_match_the_per_dot_text_renderer` in
  [`video/tests.rs`](../systems/gba/src/video/tests.rs), and the table
  checks in `tile_row_cache.rs`.

### Front-to-back lines

On lines with raster history,
[`video/render.rs`](../systems/gba/src/video/render.rs) stops drawing
backgrounds once the top two surfaces of every pixel are known, and
[`video/objects.rs`](../systems/gba/src/video/objects.rs) draws ordinary
object rows eight pixels at a time.

- Reference: per-pixel objects and `apply_color_effect_at`, which still
  handle the special cases.
- Checked by: `regular_object_rows_match_the_per_pixel_object_renderer`,
  `vector_color_effects_match_the_per_dot_effects_unit` and
  `front_to_back_passes_preserve_priority_and_the_next_visible_bg` in
  [`video/tests.rs`](../systems/gba/src/video/tests.rs).

### Scanline memo

[`render_scanline`](../systems/gba/src/video/render.rs) leaves a line as it
is when none of the registers or video memory it read has changed since it
was last drawn.
Writes mark what they change, and a marked change drops the saved line
([research note](research/scanline-memo.md)).

- Reference: `check_kept_scanline`, which redraws kept lines and compares
  their colors.
- Checked by: `memoized_scanlines_match_fresh_renders_after_every_kind_of_input_change`
  in [`video/tests.rs`](../systems/gba/src/video/tests.rs).

### Selective video

When the host won't show a frame, [`Session`](../systems/gba/src/session.rs)
skips drawing it, and everything else runs as usual. If the host asks for those
pixels later, [`VideoRecovery`](../systems/gba/src/video_recovery.rs) replays
the calls since the last drawn frame with video on and returns the exact
picture ([research note](research/selective-video.md)).

- Reference: running with full video, and loading a portable state.
- Checked by:
  `selective_session_restores_exact_arbitrary_cycle_states_and_inputs_across_internal_frame_splits`
  in [`session.rs`](../systems/gba/src/session.rs) and
  `copies_continue_exactly_like_their_source` in
  [`machine.rs`](../systems/gba/src/machine.rs).

## Writing hot-path code

How LLVM compiles the fast paths moves them by several percent, so hot-path
code follows three rules.

**Pin inlining.** Fat LTO inlines a call when its cost is below LLVM's
threshold. A call within a few dozen units of the threshold flips when an
unrelated edit changes its caller's size, which moves workloads by up to 6%.
Every such call in the CPU engine, device step, DMA, APU mixer and
compositor carries `#[inline(always)]` or `#[inline(never)]`, and
`thumb::run` and `Cpu::run_normal_until` stay out of line so that edits
elsewhere don't move the run loops.
[`inline-stability.mjs`](../tools/bench/inline-stability.mjs) builds with the
threshold at 200, 250 (the default) and 300 and fails if any of those
functions changes size. Run it after touching them. Measure which way to pin
a call: inlining `thumb::Ctx::half` into the Thumb loops, for example, makes
thumb-logic 17% slower.

**Don't write a value in pieces and then read it whole.** The load has to
wait for the stores to retire, because the CPU can't forward them to it
(`perf` counts these as `ld_blocks.store_forward`). This is why the Thumb run
passes its state by value to the one out-of-line call in its loop. Passed by
reference, every op's budget, index and prefetch update went through the
stack, and workloads took 8 to 15% more cycles. For the same reason,
`PpuTiming::elapse` fills the caller's `Events`, the scanline memo receives
the compositor's pages by value, and `Bus::engine_memory` is inlined.

**Mind alignment.** `Bus`'s `WindowMasks` starts on a cache-line boundary.
Every line refills its 240 bytes with fifteen 16-byte stores. With the array
8 bytes past a boundary, a quarter of those stores split across two cache
lines, and idle took 17% more cycles.

## Measuring

```sh
node tools/bench/run.mjs --emulators zigba,evenfall --boot hle
cargo bench -p evenfall-gba --bench hot_paths
cargo build --profile profiling -p evenfall-bench --bin evenfall-harness
perf record --call-graph dwarf -- \
  target/profiling/evenfall-harness path/to/workload.gba 600
perf report
```

[`tools/bench/run.mjs`](../tools/bench/run.mjs) builds the ROMs and harnesses
before timing anything, then runs each emulator in a fresh process pinned to
one CPU core. `--cpu auto` picks an idle fast core. On a CPU with performance
and efficiency cores, pin to a performance core, and never compare runs made
on different cores.

Each round runs every emulator on every workload, in an order that rotates
between rounds. Warmup rounds are discarded. A workload's ratio is the
geometric mean of its per-round loop-time ratios, and the summary is the
geometric mean across workloads. Only the frame loop is timed. Loading,
setup, hashing and reporting are not.

The runner checks that all emulators used the same boot mode, frame count,
video and audio settings, and stopped within a scanline of the requested
frame. It then compares a hash of each emulator's final frame. The hash
catches an emulator that went off the rails, but it doesn't check
correctness. That is the job of the [parity harness](../tools/parity/README.md)
and the tests.

The runner has adapters for Evenfall and for ZiGBA. The mGBA figures above
come from ZiGBA's own cross-emulator runner (`tools/emu-bench` in
`refs/zigba`), with Evenfall's harness plugged in through its `--harness`
option. The [benchmark README](../tools/bench/README.md) documents the
runner, the harness protocol and
[`wasm.mjs`](../tools/bench/README.md#webassembly), which measures
WebAssembly modules the same way in Node.

The [divan benches](../systems/gba/benches/hot_paths.rs) time APU runs, state
encoding and a mode-3 frame through the bus in isolation. They help locate a
cost, but only the ROM comparison says whether a change made the emulator
faster. The `profiling` profile is `release` with full debug info, for
`perf`.

## Open questions

- How do these ratios hold on real games, and in the browser? The benchmark
  ROMs are synthetic and weigh every path the same.
