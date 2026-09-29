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
| Idle / HALT and empty mode-0 frame                     |  3,600 |     1.12× |         0.48× |           1.05× |               0.49× |
| ARM mixer / IWRAM code                                 |    600 |     1.00× |         0.42× |           0.99× |               0.42× |
| Thumb logic / Game Pak ROM                             |    600 |     0.89× |         0.57× |           0.89× |               0.58× |
| Tiles / text backgrounds, sprites, effects and windows |    600 |     0.85× |         0.33× |           0.86× |               0.34× |
| Affine / backgrounds, sprites and HBlank DMA           |    600 |     0.78× |         0.74× |           0.66× |               0.73× |
| Bitmap DMA / page flips and small transfers            |    600 |     0.54× |         0.57× |           0.55× |               0.57× |
| Audio / Direct Sound FIFO DMA and PSG                  |    600 |     0.86× |         0.43× |           0.84× |               0.44× |
| **Geometric mean of workload ratios**                  |      — | **0.85×** |     **0.49×** |       **0.82×** |           **0.50×** |

Skip-BIOS boot maps the open Cult-of-GBA replacement BIOS and runs its
interrupt dispatcher; HLE boot services interrupts without BIOS code.
Relative to mGBA directly (same method, matched rounds), Evenfall takes 0.58×
its loop time under HLE boot and 0.61× under skip-BIOS boot. It is faster on
every workload except bitmap DMA (1.04× and 1.05×) and affine under skip-BIOS
boot (1.10×; 0.95× under HLE). Evenfall's affine time rises under skip-BIOS
boot because the replacement BIOS's interrupt dispatcher runs on the
interpreter, outside the execution engine; mGBA's does not change.

In WebAssembly, measured with [`wasm.mjs`](../tools/bench/wasm.mjs) in
Node 24 on the same CPU (five rounds, one warmup, HLE boot), loop time
relative to ZiGBA's shipped module (`ReleaseSafe`) is:

| Workload    | ZiGBA `ReleaseFast` | Evenfall scalar | Evenfall SIMD (shipped) |
| ----------- | ------------------: | --------------: | ----------------------: |
| idle        |               0.98× |           0.51× |                   0.44× |
| arm-mixer   |               0.87× |           0.27× |                   0.28× |
| thumb-logic |               0.87× |           0.35× |                   0.34× |
| tiles       |               0.94× |           0.43× |                   0.24× |
| affine      |               0.81× |           0.52× |                   0.47× |
| bitmap-DMA  |               0.87× |           0.50× |                   0.45× |
| audio       |               0.88× |           0.28× |                   0.28× |
| geomean     |               0.89× |           0.40× |                   0.35× |

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
| ARM ALU operations from IWRAM |     192 |     19 |    47 |
| ARM IWRAM loads and stores    |     235 |     38 |   103 |
| Thumb ALU operations from ROM |     479 |     37 |   184 |

The engine's monomorphized run loops cost build time and code size: a dev
one-line rebuild takes 3.6 s against 2.3 s, a release one 2.5 s against
2.0 s and a wasm-release one 7.7 s against 4.8 s, and the WebAssembly module
grows from 0.87 MB to 1.26 MB. The two ARM loops (IWRAM and EWRAM) are
about 170 KB of that module. One loop with the region as values, and both
op arrays of EWRAM's length, would remove about 85 KB, but in a same-run
comparison on CPU 1 it took 2% longer on arm-mixer (3% more instructions in
the loop) and 1% longer on affine, so the design keeps both loops and pays
their size and the slower wasm rebuild.

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
figures include linking the runtime, unlike a core-only `.rlib` build.

## Where the speed comes from

ZiGBA's optimizations were ported with their ordinary path intact. A fast
path is allowed only when the remaining instruction, transfer or renderer
work cannot reach a guest-visible event; otherwise it falls back. The links
name the Rust owner, the reference path and tests that compare observable
effects rather than merely counting invocations.

| Fast path and owner                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                       | Retained reference and evidence                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                  |
| --------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------- | ------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------ |
| **Event horizon and deferred devices:** [`Bus`](../systems/gba/src/bus/mod.rs) advances in an open batch with a strict next-event deadline, materializing APU and timers before MMIO, overflow effects or host reads. FIFO-only timer overflows delegate to [`Apu::elapse_clocked`](../systems/gba/src/apu.rs), stopping at the first refill request.                                                                                                                                                                                                                                                                                                                                                                                                                                                     | `Bus::advance_eager`/`advance_step` remain the reference; `fifo_only_timer_overflows_match_eager_refill_and_irq_schedule` in [`bus/mod.rs`](../systems/gba/src/bus/mod.rs) and `synchronized_reads_and_fifo_writes_keep_the_device_batch_armed` in [`bus_b.rs`](../systems/gba/tests/bus_b.rs) compare eager and batched endpoints, including DMA.                                                                                                                                                                                               |
| **Direct DMA beats:** [`Bus::read_dma_source` and `write_dma_destination`](../systems/gba/src/bus/mod.rs) dispatch RAM, ROM and FIFO beats directly while preserving charging and beat-boundary preemption.                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                               | Generic `Bus::read`/`write` handles other regions; `dma_at_lcd_boundary_preserves_deferred_batch` and the FIFO/batched differential test above cover timing and state. The direct-versus-generic dispatch itself has no standalone switchable differential test.                                                                                                                                                                                                                                                                                 |
| **Bulk DMA beats:** [`Bus::dma_bulk`](../systems/gba/src/bus/mod.rs) runs the beats of a DMA from RAM or the latch to RAM, VRAM or OAM that end before the device deadline in one loop, with its own clock and PPU position, while no DMA is pending and each address stays in its region. Devices catch up at the batch's end; a VRAM write on a visible dot whose line's history has room goes through the per-beat path at its cycle. VRAM writes skip their history snapshot once the line's history is full.                                                                                                                                                                                                                                                                                         | The per-beat loop in `Bus::run_dma` is the reference and runs everything else. `bulk_dma_beats_match_the_per_beat_loop` and `vram_writes_skip_snapshots_only_when_history_is_full` in [`bus/mod.rs`](../systems/gba/src/bus/mod.rs) compare durable state bytes, memo marks, raster history and pixels; `halfword_vram_writes_match_byte_writes` in [`bus/kernel.rs`](../systems/gba/src/bus/kernel.rs) checks the one-step halfword write.                                                                                                      |
| **Execution engine:** [`cpu/engine`](../systems/gba/src/cpu/engine/mod.rs) runs ARM code from IWRAM and EWRAM and Thumb code from IWRAM, EWRAM and Game Pak ROM out of caches of pre-decoded ops, charging each instruction the interpreter's exact cost against a budget that ends before the next device event, and materializing r15, the pipeline, CPSR, open bus and prefetch state only when a run stops. Straight runs of ARM ALU ops execute as charged-once blocks chained through branches; Thumb ROM runs follow the prefetch buffer in closed form; RAM and video memory accesses run inline, VRAM and palette stores only before the next visible dot. Refusals (MMIO, SWI, PSR and mode changes, stores into decoded code) go to the interpreter; [research](research/execution-engine.md). | Ordinary `Cpu::step`/`Core::execute_arm` and `execute_thumb` execute refused work and are the reference. The randomized differentials in [`engine/tests.rs`](../systems/gba/src/cpu/engine/tests.rs), `arm_kernel_matches_ordinary_execution_for_random_encodings_and_operands` and `thumb_rom_engine_matches_ordinary_steps_for_random_code_queues_and_horizons` in [`cpu/mod.rs`](../systems/gba/src/cpu/mod.rs) compare registers, flags, clocks, memory, video memory and its caches, raster history, prefetch state and refusal boundaries. |
| **Streamed fetches:** where the engine refuses, [`Bus`](../systems/gba/src/bus/mod.rs) streams eligible sequential Thumb fetches for the ordinary runner and fuses the common branch refill.                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                              | The scalar `GamePakPrefetch`/ordinary fetch path stays in `Bus`; `thumb_refills_match_two_ordinary_fetches_in_every_queue_phase_and_at_deadlines` in [`bus/fetch.rs`](../systems/gba/tests/bus/fetch.rs) and `bounded_thumb_rom_streams_match_eager_fetches_across_duty_boundaries_and_deadlines` in [`machine_a.rs`](../systems/gba/tests/machine_a.rs) cover the boundaries.                                                                                                                                                                   |
| **Lazy PSG:** [`Apu`](../systems/gba/src/apu.rs) accumulates silent square/noise oscillator time and pushes constant sample runs in bulk.                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                 | The eager oscillator scheduler is retained as a test oracle in [`apu/tests.rs`](../systems/gba/src/apu/tests.rs): `deferred_square_and_noise_oscillators_match_eager_endpoint_ordering` and `constant_sample_runs_match_the_eager_scheduler_across_silent_and_audible_psg`.                                                                                                                                                                                                                                                                      |
| **Layer-major compositor:** [`video/compose.rs`](../systems/gba/src/video/compose.rs) renders each background into a color row and the objects into front and minimum-priority rows, then one kernel written against [`Lanes`](../systems/gba/src/video/lanes.rs) (SSE2 on x86-64, `simd128` on WebAssembly built with it, scalar elsewhere) selects, merges and blends eight pixels at a time. Lines with raster history use the reference, unless their only history is VRAM writes to pages the line does not read; [research](research/layer-major-compositor.md).                                                                                                                                                                                                                                    | The front-to-back `compose_scanline_reference` with the per-dot and per-pixel renderers. `layer_major_lines_match_the_front_to_back_reference` and `vram_history_selects_the_reference_only_for_pages_the_line_reads` in [`video/tests.rs`](../systems/gba/src/video/tests.rs) and `vector_lanes_match_the_scalar_kernel` in [`video/compose.rs`](../systems/gba/src/video/compose.rs) compare them.                                                                                                                                             |
| **Tile-row decoding:** [`video/backgrounds.rs`](../systems/gba/src/video/backgrounds.rs) writes whole text tiles; the [decoded palette](../systems/gba/src/video/tile_row_cache.rs) maps each 4bpp byte to two colors per bank and is validated against the palette's bytes; [research](research/tile-row-cache.md).                                                                                                                                                                                                                                                                                                                                                                                                                                                                                      | The per-dot `render_text_background`. `text_tile_rows_match_the_per_dot_text_renderer` in [`video/tests.rs`](../systems/gba/src/video/tests.rs) and the table checks in [`tile_row_cache.rs`](../systems/gba/src/video/tile_row_cache.rs).                                                                                                                                                                                                                                                                                                       |
| **Front-to-back composition (history lines):** [`video/render.rs`](../systems/gba/src/video/render.rs) stops BG passes after top/second surfaces close, and [`video/objects.rs`](../systems/gba/src/video/objects.rs) composes regular OBJ rows in lanes.                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                 | Per-pixel objects and `apply_color_effect_at` remain for special cases; `regular_object_rows_match_the_per_pixel_object_renderer`, `vector_color_effects_match_the_per_dot_effects_unit` and `front_to_back_passes_preserve_priority_and_the_next_visible_bg` in [`video/tests.rs`](../systems/gba/src/video/tests.rs) cover them.                                                                                                                                                                                                               |
| **Scanline memo:** [`render_scanline`](../systems/gba/src/video/render.rs) retains a line only while registers and video-memory reads remain valid; marked changes invalidate it.                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                         | `check_kept_scanline` recomposes kept lines and compares their colors. `memoized_scanlines_match_fresh_renders_after_every_kind_of_input_change` in [`video/tests.rs`](../systems/gba/src/video/tests.rs) compares fresh and memoized frames; [research](research/scanline-memo.md).                                                                                                                                                                                                                                                             |
| **Selective video:** [`Session`](../systems/gba/src/session.rs) skips only pixel production on unpresented frames; [`VideoRecovery`](../systems/gba/src/video_recovery.rs) replays a bounded journal for exact on-demand pixels.                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                          | Full-video execution and scratch replay are references. `selective_session_restores_exact_arbitrary_cycle_states_and_inputs_across_internal_frame_splits` in [`session.rs`](../systems/gba/src/session.rs) compares reconstructed states and frames; [research](research/selective-video.md).                                                                                                                                                                                                                                                    |

These tests complement the ZiGBA/Rust [parity traces](parity.md): optimized
paths must preserve timed access, IRQ/DMA ordering, full-frame pixels, DAC
samples and portable state bytes under different host slice sizes. Save-state
and browser behavior are described in [`design/`](design/save-states.md) and
[`sdk.md`](sdk.md).

## Differences, experiments and open questions

Lines without relevant raster history use the
[layer-major compositor](research/layer-major-compositor.md): every
background renders a color row, objects render front and minimum-priority
rows, and one kernel composes eight pixels at a time with explicit SSE2 or
WebAssembly `simd128` lanes (scalar elsewhere). LCD events carry the
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
lanes. The per-dot paths remain references. The portable-state
anchor for selective video costs an encode and validated scratch decode
instead of ZiGBA's raw machine copy. Shared `Arc` ROM and BIOS media avoid
copying immutable uploads among machine, session and scratch loads; the ROM
fetch view keeps a direct byte pointer, and Vec-backed versus slice-backed
reads show no stable difference on measured ROM-bound kernels. The WASM
runtime uses Rust's default `dlmalloc`, not ZiGBA's custom page-run allocator.
These are implementation/performance differences, not permissions to change
guest behavior; see the [roadmap](roadmap.md) and
[`web/gba/README.md`](../web/gba/README.md).

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
retained.

**Rejected CPU trials:** replacing the condition match with a bitmask moved
ARM 2.74×→2.77× and Thumb 1.94×→1.96× (ten matched rounds); inlining the
single-transfer executor moved ARM 2.65×→2.65× and Thumb 1.88×→1.85×
(five rounds, no ARM payoff). Neither belongs to the retained design.

Open questions: which costs dominate on representative legally supplied
games and on WASM rather than these synthetic ROMs? Can stable-Rust lane
patterns improve vectorization without losing the scalar oracle? Does the
portable selective-video anchor or shared ROM header matter enough to warrant
an audited alternate path? Can direct DMA's specialized dispatch be isolated
against the generic path without introducing a production test switch? Each
requires a workload, a reference comparison and an exactness check before
claiming a speed improvement.
