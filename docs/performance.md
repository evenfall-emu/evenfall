# Performance

Evenfall keeps ZiGBA's cycle-accurate GBA execution model: timed fetches,
event ordering, full rendering and exact emulated audio. Its hot paths remove
host work without relaxing that contract. The native comparison below uses
small, purpose-built ROMs, not a representative game or a claim about WASM.
ZiGBA remains the port's byte-exact behavioral oracle; final-image agreement
in a speed run is only a useful screen for divergent execution. See the
[`benchmark runner`](../tools/bench/README.md),
[`methodology`](methodology.md) and [research notes](research/README.md).

## How it is measured

[`tools/bench/run.mjs`](../tools/bench/run.mjs) builds the ROMs and the two
harnesses _before_ timing, chooses one allowed CPU before any sample, holds a
host-wide benchmark lane and uses fresh, pinned processes. On this hybrid
host, CPUs 0–3 are performance cores, 4–11 efficiency cores and 12–15
low-power cores. `--cpu auto` samples idle time and prefers a fast core;
explicit `--cpu 3` holds this comparison on one performance core. Results
from different CPUs are not pooled.

Each round runs both adapters on each workload, rotating their order with the
round and workload. Warmups are recorded but excluded. For each workload the
runner takes the geometric mean of Evenfall/ZiGBA **loop-wall-time ratios in
matched rounds**, then the geometric mean across workloads. Median frames per
second is descriptive, not the ratio's denominator. Setup, ROM loading,
hashing and reporting are untimed; each emulator runs a frame at a time for
280,896 guest cycles per frame, renders every line, emulates audio without
draining it, accepts no input and uses no save media. The runner requires the
boot, frame-count, video and audio policies to agree, bounds each endpoint
cycle to the requested frame (allowing an instruction-end overshoot of less
than a scanline), and compares the final 240×160 RGB555 framebuffer's FNV-1a
hash. Equal hashes do not establish PCM, full-state or hardware parity; use
[`tools/parity`](../tools/parity/README.md) and the ordinary tests for that
evidence.

```sh
node tools/bench/run.mjs --emulators zigba,evenfall --boot hle --cpu 3 --rounds 5 --warmups 1
cargo bench -p evenfall-gba --bench hot_paths
cargo build --profile profiling -p evenfall-bench --bin evenfall-harness
taskset -c 3 perf record --call-graph dwarf -- \
  target/profiling/evenfall-harness path/to/workload.gba 600
perf report
```

The [divan benches](../systems/gba/benches/hot_paths.rs) isolate audible and
constant APU runs, APU and SRAM state encode/decode, and a mode-3 frame driven
through the bus. They locate costs; they are not substitutes for the timed ROM
comparison. The `profiling` profile inherits release optimization with full
debug information. `perf` attributes sampled host cycles, not a verdict about
guest correctness or which individual optimization to retain.

## Results

Native x86-64, pinned CPU 3 (a performance core), five measured rounds and
one warmup, full frame counts. Each emulator renders fully and emulates exact
audio; every final image agrees across emulators and observations, and the
parity harness finds every Evenfall state byte identical to ZiGBA's on its
scenarios. The comparison runs ZiGBA's own cross-emulator runner
(`tools/emu-bench` in the pinned checkout) with Evenfall's `release-lto`
harness added through its `--harness` option: ZiGBA builds at `ReleaseFast`,
and mGBA 0.10.5 is its headless CMake `Release` library with idle-loop
detection at its default. Ratios are loop time relative to ZiGBA, the
geometric mean of matched-round ratios; below one is faster. Each figure
belongs to the binaries it was measured on, and the runner's `result.json`
records their source revisions and hashes.

| Workload                                               | Frames | mGBA, HLE | Evenfall, HLE | mGBA, skip-BIOS | Evenfall, skip-BIOS |
| ------------------------------------------------------ | -----: | --------: | ------------: | --------------: | ------------------: |
| Idle / HALT and empty mode-0 frame                     |  3,600 |     1.10× |         0.40× |           1.05× |               0.43× |
| ARM mixer / IWRAM code                                 |    600 |     1.00× |         0.41× |           0.99× |               0.41× |
| Thumb logic / Game Pak ROM                             |    600 |     0.86× |         0.48× |           0.89× |               0.47× |
| Tiles / text backgrounds, sprites, effects and windows |    600 |     0.86× |         0.31× |           0.87× |               0.30× |
| Affine / backgrounds, sprites and HBlank DMA           |    600 |     0.77× |         0.60× |           0.64× |               0.55× |
| Bitmap DMA / page flips and small transfers            |    600 |     0.55× |         0.36× |           0.56× |               0.36× |
| Audio / Direct Sound FIFO DMA and PSG                  |    600 |     0.88× |         0.36× |           0.85× |               0.37× |
| **Geometric mean of workload ratios**                  |      — | **0.84×** |     **0.41×** |       **0.82×** |           **0.41×** |

Skip-BIOS boot maps the open Cult-of-GBA replacement BIOS and runs its
interrupt dispatcher; HLE boot services interrupts without BIOS code.
Relative to mGBA directly (same method, matched rounds), Evenfall takes 0.48×
its loop time under HLE boot and 0.50× under skip-BIOS boot, and is faster on
every workload. The closest are affine (0.78× under HLE, 0.86× under
skip-BIOS) and bitmap DMA (0.65× and 0.64×). The replacement BIOS's ARM code,
its interrupt dispatcher included, runs in the execution engine.

In WebAssembly, measured with [`wasm.mjs`](../tools/bench/wasm.mjs) in
Node 24 on the same CPU (five rounds, one warmup, HLE boot), loop time
relative to ZiGBA's shipped module (`ReleaseSafe`) is:

| Workload    | ZiGBA `ReleaseFast` | Evenfall scalar | Evenfall SIMD (shipped) |
| ----------- | ------------------: | --------------: | ----------------------: |
| idle        |               0.94× |           0.45× |                   0.44× |
| arm-mixer   |               0.86× |           0.27× |                   0.27× |
| thumb-logic |               0.86× |           0.30× |                   0.30× |
| tiles       |               0.93× |           0.41× |                   0.23× |
| affine      |               0.81× |           0.46× |                   0.41× |
| bitmap-DMA  |               0.86× |           0.35× |                   0.30× |
| audio       |               0.87× |           0.25× |                   0.26× |
| geomean     |               0.88× |           0.35× |                   0.31× |

Every image agrees across all modules and with the native harnesses. The
benchmark ROMs weight each path equally, so neither table predicts a game's
ratio.

### Execution engine

The [execution engine](research/execution-engine.md) replaces the ARM IWRAM
and Thumb ROM kernels. On pinned CPU 1 with release-LTO, HLE boot, five
matched rounds and one warmup in one run, Evenfall/ZiGBA loop time for the
kernels' last design, the layer-major renderer on top of them, and the
engine with that renderer:

| Workload    | Kernels | Kernels and renderer | Engine and renderer |
| ----------- | ------: | -------------------: | ------------------: |
| idle        |   1.04× |                0.56× |               0.60× |
| arm-mixer   |   1.96× |                1.69× |               0.43× |
| thumb-logic |   1.52× |                1.22× |               0.57× |
| tiles       |   1.56× |                0.45× |               0.34× |
| affine      |   1.21× |                0.95× |               0.81× |
| bitmap-DMA  |   1.15× |                0.69× |               0.69× |
| audio       |   1.29× |                0.91× |               0.43× |
| geomean     |   1.36× |                0.84× |               0.53× |

Every final image agrees. Against the kernels with the same renderer the
engine takes 0.63× the loop time (geometric mean), 0.25× on arm-mixer and
0.47× on thumb-logic; idle differs by run-to-run noise (equal wall time and
instruction counts in a direct comparison). Every workload shows its status
rows in VBlank, which the kernels left to the interpreter one VRAM store at a
time; the engine runs them.

Host instructions per guest instruction on the dispatch diagnostics
([`tools/bench/diag`](../tools/bench/diag/README.md)), the difference between
330 and 30 frames on the same CPU:

| Loop                          | Kernels | Engine | ZiGBA |
| ----------------------------- | ------: | -----: | ----: |
| ARM ALU operations from IWRAM |     192 |     20 |    47 |
| ARM IWRAM loads and stores    |     235 |     38 |   103 |
| Thumb ALU operations from ROM |     479 |     35 |   184 |

The engine's monomorphized run loops cost build time and code size: a dev
one-line rebuild takes 3.6 s against 2.3 s, a release one 2.5 s against
2.0 s and a wasm-release one 7.7 s against 4.8 s, and the WebAssembly module
grows from 0.87 MB to 1.26 MB. The two ARM loops (IWRAM and EWRAM) are
about 170 KB of that module. One loop with the region as values, and both
op arrays of EWRAM's length, would remove about 85 KB, but in a same-run
comparison on CPU 1 it took 2% longer on arm-mixer (3% more instructions in
the loop) and 1% longer on affine, so the design keeps both loops and pays
their size and the slower wasm rebuild.

MMIO accesses stay in the engine where the device batch allows. IE, IF,
IME, POSTFLG, HALTCNT and the DMA source, destination and count latches are
read and written by the run itself; it stops only when a store halts the
CPU or raises an IRQ it takes. The run also reads DISPSTAT and VCOUNT
itself, from the PPU position it started at advanced to the access (no PPU
event falls inside a run), and stores the affine parameters and reference
points itself where no dot is visible, as an HBlank handler setting the
next line's transform does. Other registers are accessed through the bus
at the instruction's cycle between passes of the same run. A store to a
register that schedules no device event (background, window and blend
registers, the interrupt registers) keeps the device horizon instead of
finding it again. The Thumb loop refuses every MMIO transfer like any other
op it cannot run, and its driver completes it, so MMIO costs the loop's
other ops nothing. `thumb::run` and `Cpu::run_normal_until` are out of line
so that code changes elsewhere do not move the run loops; `Cpu::step_impl`
stays inlined, since calling it cost affine 3% more cycles.

The hot paths avoid values written in pieces and then read whole, whose
loads wait for the stores to retire (a failed store-to-load forward; `perf`
counts them as `ld_blocks.store_forward`). The Thumb run's state has no
address: the one out-of-line call in its loop takes and returns it by value.
When that call took it by reference, every op's budget, index and prefetch
update went through the stack, and on CPU 1 thumb-logic took 15% more
cycles, audio 14%, affine and tiles 10% and bitmap-DMA 8%. For the same
reason `PpuTiming::elapse` fills the caller's `Events`, the scanline memo
receives the compositor's pages as a value, and `Bus::engine_memory` is
inlined; the first two took idle's cycles down 9% and affine's 4%.
`Bus`'s `WindowMasks` starts on a cache-line boundary: every line refills its
240 bytes with fifteen 16-byte stores, and otherwise where it falls depends
on `Bus`'s field layout. At 8 bytes past a boundary, a quarter of those
stores split across two lines (`mem_inst_retired.split_stores`), and idle
took 17% more cycles on CPU 3.

The hot paths pin their inlining. Fat LTO decides most inlining by comparing a
call's cost with LLVM's threshold, and a call within a few dozen units of it
flips when an unrelated edit changes a caller's size. Without pins, moving
the threshold by 50 either way changes 14 to 29 of those functions and moves
workloads by up to 6% in cycles (audio 2.6% slower at 300, idle 6% faster at
300 and 3.5% slower at 200).
[`inline-stability.mjs`](../tools/bench/inline-stability.mjs) finds such
calls: it builds the native harness and the `simd128` module with the
threshold at 200, 250 (the default) and 300 and reports every CPU-engine,
device-step, DMA, APU-mixer and compositor function whose size changes.
Every such call carries `#[inline(always)]` or `#[inline(never)]`, so none
changes. The direction matters: inlining `thumb::Ctx::half` into the Thumb
loops makes thumb-logic 17% and audio 19% slower in cycles, and calling
`Bus::advance_step` out of line from an inlined `advance_eager` loop, rather
than the reverse, takes affine 3.1% and idle 2.3% fewer cycles. On CPU 3, in
cycles (minimum of five runs of a loaded host), the pinned tree with the
`wide` lanes against the tree before either: audio 0.998, thumb-logic 1.003,
arm-mixer 1.004, tiles 0.961, affine 0.979, bitmap-DMA 0.990, idle 0.980,
with 0.2–1.1% fewer instructions everywhere; the `simd128` module is within
run-to-run noise (±2%) of the one before.

### Build-time budget

On a loaded host, direct rustup Cargo with no output cache, a cold target and
then a one-line change to inlined `Bus::now`'s **body** gives the following
wall times. Native builds target the core and WebAssembly builds link the
shipped web consumer; these are different outputs. The reproducible
[`build-times.sh`](../tools/bench/build-times.sh) creates an isolated snapshot
of committed source, so it never edits a caller's worktree.

| Build                                  |   Cold | One-line rebuild |
| -------------------------------------- | -----: | ---------------: |
| Evenfall dev (core and dependencies)   | 10.0 s |            3.6 s |
| Evenfall release (no LTO, incremental) |  6.5 s |            2.5 s |
| Evenfall wasm-release (fat LTO)        |  9.5 s |            7.7 s |
| Pinned ZiGBA native ReleaseFast        | 19.3 s |           15.6 s |
| Pinned ZiGBA WASM ReleaseSafe          | 33.8 s |           30.7 s |

These are build-budget observations, not like-for-like safety or optimization
profiles; host load and incremental cache state affect the times. The WASM
figures include linking the runtime, unlike a core-only `.rlib` build. The
`wide` crate and its dependencies (`bytemuck`, and `safe_arch` on x86) add
about 4 s to a cold release build and 5 s to a cold wasm-release build,
measured on another loaded run; one-line rebuilds do not change.

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
core, fast paths included, must match ZiGBA byte for byte however the host
slices execution. Most fast paths were ported from ZiGBA along with their
reference.

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

## Differences, experiments and open questions

Lines without relevant raster history use the
[layer-major compositor](research/layer-major-compositor.md): every
background renders a color row, objects render front and minimum-priority
rows, and one kernel composes eight pixels at a time with explicit vector
lanes (SSE2, NEON or WebAssembly `simd128`; scalar elsewhere). LCD events carry the
completed visible lines as a bitset, so HBlank delivery visits only those
lines instead of scanning 160 flags on every event; this also shortens the
CPU-bound workloads. On pinned CPU 3 with release-LTO, HLE boot and five
matched rounds in one run, against the front-to-back design it replaces:

| Workload    | Before | After | Instructions before → after (1,200 frames) |
| ----------- | -----: | ----: | ------------------------------------------ |
| idle        |  1.05× | 0.59× | 0.38 G → 0.26 G (ZiGBA 0.19 G)             |
| arm-mixer   |  1.99× | 1.69× | 29.31 G → 25.25 G (ZiGBA 14.08 G)          |
| thumb-logic |  1.55× | 1.29× | 18.97 G → 15.55 G (ZiGBA 11.05 G)          |
| tiles       |  1.52× | 0.43× | 19.98 G → 8.17 G (ZiGBA 11.91 G)           |
| affine      |  1.21× | 0.93× | 18.59 G → 14.16 G (ZiGBA 13.04 G)          |
| bitmap-DMA  |  1.12× | 0.70× | 27.20 G → 18.15 G (ZiGBA 21.64 G)          |
| audio       |  1.31× | 0.92× | 12.05 G → 8.53 G (ZiGBA 8.24 G)            |
| geomean     |  1.36× | 0.85× |                                            |

Ratios are Evenfall/ZiGBA loop time; every final image agrees. In tiles the
renderer now retires fewer instructions than the Thumb kernel, and host
cycles fall from 4.65 G to 1.30 G per 1,200 frames against ZiGBA's 3.00 G.
Thumb-logic's ratio varies between runs (1.23× in another) with unchanged
instruction counts.
Bitmap-DMA gains most from its VRAM history: writes to the hidden page during
HDraw give 45% of its lines history, and the compositor keeps them because
none of the written pages is read. The kernel and row variants cost about
0.6 s on a release-LTO one-line rebuild of the harness (6.5 s → 7.1 s) and
0.1 s on a `release` one (1.9 s → 2.0 s).

The WebAssembly module, measured in Node with
[`tools/bench/wasm.mjs`](../tools/bench/README.md#webassembly) on the same
CPU and ROMs, takes 0.58× the loop time of ZiGBA's shipped ReleaseSafe
module with scalar lanes and 0.48× with `simd128` (geometric means; tiles
0.46× and 0.25×, affine 0.64× and 0.58×), against 0.82× before this design
and 0.88× for ZiGBA built ReleaseFast. The `simd128` module is 0.7% larger
(875,620 against 869,231 bytes) and slower on no workload.

The front-to-back reference keeps fixed-size lane loops for text rows,
objects and color effects where ZiGBA uses `@Vector` operations; the loops do
not reliably auto-vectorize, which is why the layer-major kernel uses explicit
lanes. The per-dot paths remain references. Shared `Arc` ROM and BIOS media
avoid copying immutable uploads among machine, session and scratch loads; the
ROM fetch view keeps a direct byte pointer, and Vec-backed versus
slice-backed reads show no stable difference on measured ROM-bound kernels.
The WASM runtime uses Rust's default `dlmalloc`, not ZiGBA's custom page-run
allocator. These are implementation/performance differences, not permissions
to change guest behavior; see the [roadmap](roadmap.md) and
[`web/gba/README.md`](../web/gba/README.md).

[Selective video](research/selective-video.md) anchors on a raw copy of the
machine, as ZiGBA does. An encoded anchor costs a state encode per presented
frame (about 400 µs natively, more than a whole frame of most workloads),
which makes skipping frames slower than drawing them. Microseconds per frame
through `Session::advance` over 960 frames, native release build pinned to
CPU 3, host audio off, encoded anchor → raw copy:

| Workload    | Every frame drawn | Interval 4 | Interval 16 |
| ----------- | ----------------: | ---------: | ----------: |
| idle        |         5.5 → 5.6 |  131 → 8.0 |    35 → 4.7 |
| arm-mixer   |         132 → 132 |  262 → 133 |   162 → 131 |
| thumb-logic |         151 → 150 |  278 → 151 |   180 → 148 |
| tiles       |         160 → 159 |   204 → 77 |     86 → 52 |
| affine      |         290 → 274 |  336 → 205 |   218 → 184 |
| bitmap-DMA  |         259 → 260 |  358 → 231 |   253 → 221 |
| audio       |           74 → 75 |   201 → 77 |    104 → 74 |

The render-heavy workloads (tiles, affine, bitmap DMA) now gain from
skipping; the others stay within a few percent of full video. The one replay
machine replaces the encoded anchor and the machine each reconstruction
built, so a session at interval 4 holds about 0.5 MiB less heap
(3.1–3.7 MiB).

**What did not pay:** On pinned CPU 2, 240-frame tiles/affine skip-BIOS ROMs
over five matched rounds, the baseline ratios were 1.75×/1.43×. Packed u8
text-admission masks gave 1.68×/1.59×, hoisted affine line-state lookups
1.75×/1.48×, and merged duplicate tile-row cache lookups 1.76×/1.45×.
There was no stable win; all three trials were reverted, not retained as
optimizations.

On CPU 3, skipping offscreen VRAM snapshots cut bitmap-DMA to about 1.30×
but slowed tiles to 1.64× (or 1.62× when restricted to bitmap modes).
Inlining snapshots or separating the Mode 4 row renderer did not remove the
tiled slowdown or improve the retained Mode 4 path; those trials were reverted.
Force-inlining the cache-row lookup raised tile instructions from 21.43 G to
21.85 G per 1,200 frames. Hoisting per-lane frame bounds checks with slices
lowered instructions to 21.29 G but did not lower cycles (about 4.80 G for
both); neither trial is retained.

An eight-lane branch-free mask/select and full-surface copy raised tiles from
20.88 G to 23.68 G instructions per 1,200 frames and from 4.71 G to 4.94 G
cycles; it is not retained. Decoding palette words through `as_chunks` instead
of indexed reads raised bitmap-DMA from 29.08 G to 29.54 G instructions and
from 5.04 G to 5.71 G cycles per 1,200 frames; the indexed loop is retained.

In the layer-major design, per-background passes over surface arrays that
relied on auto-vectorization retired 13.56 G tiles instructions per 1,200
frames against 10.76 G for the explicit lane kernel at that point; LLVM
turned their selects into per-lane conditional stores or loads from selected
addresses, and double-buffered 16-bit planes made it worse (15.53 G). Inside
the new text rows, the content-keyed 4bpp row cache missed constantly on the
tiles ROM, whose map uses all 16 palette banks with flips; per-bank tables
that decode a byte into two colors took tiles from 10.76 G to 9.28 G.

In a separate pinned-CPU-1 HLE comparison (matched rounds with equal final
images), **retained** CPU work reduced the ARM-mixer ratio from 3.21× to
2.77× when the condition result was reused, with Thumb logic unchanged at
1.96× (five rounds); isolated clean release build time moved from 4.86 to
5.04 s. Force-inlining the passed-condition executor changed ARM from
2.75× to 2.68× and Thumb from 1.85× to 1.85× (ten rounds; its 3.43 s
rebuild was not a paired comparison). Inlining the ARM data-processing
executor changed ARM from 2.64× to 2.57× and Thumb from 1.88× to 1.84×
(ten rounds); paired nonincremental CPU+GBA rebuilds were 2.73–2.79 s
without versus 2.77–2.78 s with it. These are _separate paired experiments_,
not additive savings or a refresh of the seven-workload table. The initial
unmatched ARM/Thumb baseline is not used to infer an improvement. The
ordinary interpreter stayed as the reference, and full randomized kernel
comparisons, validators and ZiGBA parity passed for the retained work;
Evenfall is still slower here.

A register file kept in the engine's cache between runs, instead of a
1 KiB array cleared on the stack of every run, cost more than the clear:
addressing registers through a pointer rather than the stack pointer took
the ARM memory diagnostic from 37.8 to 38.7 host instructions per guest
instruction and thumb-logic 2% more cycles, for 1% on affine. It is not
retained. The ARM run loop, entered again after every MMIO access, keeps
r0..r15 alone on its stack instead: an op's register numbers are a
16-valued type that needs no bounds check, and the numbers packed in its
operand word are masked. That removes the clear (a libc `memset` on every
entry) and takes affine 1.3% and skip-BIOS affine 1.9% fewer cycles; the ARM
ALU diagnostic retires 19.8 instead of 19.1 host instructions per guest
instruction in equal cycles. The Thumb loop keeps the cleared 256-entry file
indexed by byte: with 16 entries, masking its third operands cost
thumb-logic and audio 1.3% more instructions than the clear saved.

**What did not pay in the engine:** keeping the BIOS image in the run's
shared memory view, reading BIOS ARM code through the inlined `Mem::load`,
and reusing a run's video floor in later passes (0.3% fewer instructions,
more cycles on affine) were all slower. Aligning every loop to 64 bytes
(`-align-loops=64`) moved the geometric mean 1.6% faster but bitmap-DMA 5%,
arm-mixer 1.6% and thumb-logic 1% slower; a global code-generation flag is
not retained for a mixed result. Building `Events` in locals removed its
store-forwarding stall but added 9% instructions to idle, because the line
set's insert became branch-free over its three words.

**Rejected CPU trials:** replacing the condition match with a bitmask moved
ARM 2.74×→2.77× and Thumb 1.94×→1.96× (ten matched rounds); inlining the
single-transfer executor moved ARM 2.65×→2.65× and Thumb 1.88×→1.85×
(five rounds, no ARM payoff). Neither belongs to the retained design.

Open questions: which costs dominate on representative legally supplied
games and on WASM rather than these synthetic ROMs? Can stable-Rust lane
patterns improve vectorization without losing the scalar oracle? Does the
shared ROM header matter enough to warrant an audited alternate path? Can direct DMA's specialized dispatch be isolated
against the generic path without introducing a production test switch? Each
requires a workload, a reference comparison and an exactness check before
claiming a speed improvement.
