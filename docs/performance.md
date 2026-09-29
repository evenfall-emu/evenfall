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

Native x86-64, HLE boot, pinned CPU 3, five measured rounds and one warmup;
ZiGBA `ReleaseFast` versus Evenfall Cargo `release` (opt-level 3, incremental,
**no LTO**). Both render fully and emulate exact audio. Each row's final image
agrees and is deterministic across observations. The source revisions and
frame counts are part of this measured configuration:

| Workload — Evenfall `0b9ede6e776e1a580f395a292acc18fbc7659f43`, ZiGBA `add59e5957b75f29d084813336ee20e300fc0b6e` | Frames | Evenfall/ZiGBA loop time |
| ---------------------------------------------------------------------------------------------------------------- | -----: | -----------------------: |
| Idle / HALT and empty mode-0 frame                                                                               |  3,600 |                    1.24× |
| ARM mixer / IWRAM code                                                                                           |    600 |                    3.08× |
| Thumb logic / Game Pak ROM                                                                                       |    600 |                    1.87× |
| Tiles / text backgrounds, sprites, effects and windows                                                           |    600 |                    1.66× |
| Affine / backgrounds, sprites and HBlank DMA                                                                     |    600 |                    1.42× |
| Bitmap DMA / page flips and small transfers                                                                      |    600 |                    1.42× |
| Audio / Direct Sound FIFO DMA and PSG                                                                            |    600 |                    1.53× |
| **Geometric mean of workload ratios**                                                                            |      — |                **1.67×** |

Ratios above one mean Evenfall takes longer; all seven paths remain slower in
this comparison. ARM mixer points to the IWRAM kernel and timed RAM/bus
accesses; Thumb logic to ROM dispatch and prefetch; tiles and affine to the
compositor, row renderer and raster work; bitmap DMA to transfer scheduling;
audio to FIFO, PSG and DMA clocks; idle to the event horizon, HALT and the
empty frame. These are workload _stresses_, not isolated causal attributions:
profiling or an exact paired experiment is needed to assign a gap to one
mechanism. The equal images do not prove complete state or sample parity.
Small ROMs weight paths differently from games; neither the overall ratio nor
these native measurements predict another host or the browser.

### Build-time budget

On a loaded host, direct rustup Cargo with no output cache, a cold target and
then a one-line change to inlined `Bus::now`'s **body** gives the following
wall times. Native builds target the core and WebAssembly builds link the
shipped web consumer; these are different outputs. The reproducible
[`build-times.sh`](../tools/bench/build-times.sh) creates an isolated snapshot
of committed source, so it never edits a caller's worktree.

| Build                                  |      Cold | One-line rebuild |
| -------------------------------------- | --------: | ---------------: |
| Evenfall dev (core and dependencies)   | 7.4–7.9 s |        2.2–2.5 s |
| Evenfall release (no LTO, incremental) | 5.1–6.8 s |      about 2.0 s |
| Evenfall wasm-release (fat LTO)        |     6.4 s |            4.3 s |
| Pinned ZiGBA native ReleaseFast        |    19.3 s |           15.6 s |
| Pinned ZiGBA WASM ReleaseSafe          |    33.8 s |           30.7 s |

These are build-budget observations, not like-for-like safety or optimization
profiles; host load and incremental cache state affect the times. The WASM
figures include linking the runtime, unlike a core-only `.rlib` build.

## Where the speed comes from

ZiGBA's optimizations were ported with their ordinary path intact. A fast
path is allowed only when the remaining instruction, transfer or renderer
work cannot reach a guest-visible event; otherwise it falls back. The links
name the Rust owner, the reference path and tests that compare observable
effects rather than merely counting invocations.

| Fast path and owner                                                                                                                                                                                                                                                                                                                                                                                                                                                    | Retained reference and evidence                                                                                                                                                                                                                                                                                                                                                                                                                                                 |
| ---------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------- | ------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------- |
| **Event horizon and deferred devices:** [`Bus`](../systems/gba/src/bus/mod.rs) advances in an open batch with a strict next-event deadline, materializing APU and timers before MMIO, overflow effects or host reads. FIFO-only timer overflows delegate to [`Apu::elapse_clocked`](../systems/gba/src/apu.rs), stopping at the first refill request.                                                                                                                  | `Bus::advance_eager`/`advance_step` remain the reference; `fifo_only_timer_overflows_match_eager_refill_and_irq_schedule` in [`bus/mod.rs`](../systems/gba/src/bus/mod.rs) and `synchronized_reads_and_fifo_writes_keep_the_device_batch_armed` in [`bus_b.rs`](../systems/gba/tests/bus_b.rs) compare eager and batched endpoints, including DMA.                                                                                                                              |
| **Direct DMA beats:** [`Bus::read_dma_source` and `write_dma_destination`](../systems/gba/src/bus/mod.rs) dispatch RAM, ROM and FIFO beats directly while preserving charging and beat-boundary preemption.                                                                                                                                                                                                                                                            | Generic `Bus::read`/`write` handles other regions; `dma_at_lcd_boundary_preserves_deferred_batch` and the FIFO/batched differential test above cover timing and state. The direct-versus-generic dispatch itself has no standalone switchable differential test.                                                                                                                                                                                                                |
| **ARM IWRAM and Thumb ROM kernels:** [`Cpu`](../systems/gba/src/cpu/mod.rs) preflights each instruction within a device horizon and borrows [`Bus` execution regions](../systems/gba/src/bus/kernel.rs). Rust uses bounded loop/match dispatch and the shared ISA executor, **not** ZiGBA's compile-time-labelled switch-handler graph. ARM reuses its condition check across preflight and execution; the passed-condition and data-processing executors are inlined. | Ordinary `Cpu::step_impl`/`Core::execute_arm` and `execute_thumb` execute refused work. `arm_kernel_matches_ordinary_execution_for_random_encodings_and_operands` and `thumb_rom_kernel_matches_ordinary_steps_for_random_code_queues_and_horizons` in [`cpu/mod.rs`](../systems/gba/src/cpu/mod.rs) compare clocks, registers, buses and refusal boundaries.                                                                                                                   |
| **Closed-form prefetch and streamed fetches:** [`ThumbRomRun`](../systems/gba/src/bus/kernel.rs) keeps the ROM queue's head-ready time and duty locally; [`Bus`](../systems/gba/src/bus/mod.rs) streams eligible sequential Thumb fetches and fuses the common branch refill.                                                                                                                                                                                          | The scalar `GamePakPrefetch`/ordinary fetch path stays in `Bus`; `thumb_rom_run_matches_scalar_prefetch` in [`bus/kernel.rs`](../systems/gba/src/bus/kernel.rs), `thumb_refills_match_two_ordinary_fetches_in_every_queue_phase_and_at_deadlines` in [`bus/fetch.rs`](../systems/gba/tests/bus/fetch.rs), and `bounded_thumb_rom_streams_match_eager_fetches_across_duty_boundaries_and_deadlines` in [`machine_a.rs`](../systems/gba/tests/machine_a.rs) cover the boundaries. |
| **Lazy PSG:** [`Apu`](../systems/gba/src/apu.rs) accumulates silent square/noise oscillator time and pushes constant sample runs in bulk.                                                                                                                                                                                                                                                                                                                              | The eager oscillator scheduler is retained as a test oracle in [`apu/tests.rs`](../systems/gba/src/apu/tests.rs): `deferred_square_and_noise_oscillators_match_eager_endpoint_ordering` and `constant_sample_runs_match_the_eager_scheduler_across_silent_and_audible_psg`.                                                                                                                                                                                                     |
| **Tile rows and cache:** [`video/backgrounds.rs`](../systems/gba/src/video/backgrounds.rs) decodes eligible text rows once, skipping hidden lanes; the [content-keyed decoded-row cache](../systems/gba/src/video/tile_row_cache.rs) validates bytes and palette.                                                                                                                                                                                                      | The per-dot `render_text_background` remains for raster/mosaic cases. `text_tile_rows_match_the_per_dot_text_renderer` in [`video/tests.rs`](../systems/gba/src/video/tests.rs) and cache validation in [`tile_row_cache.rs`](../systems/gba/src/video/tile_row_cache.rs) check these paths.                                                                                                                                                                                    |
| **Front-to-back composition:** [`video/render.rs`](../systems/gba/src/video/render.rs) stops BG passes after top/second surfaces close, and [`video/objects.rs`](../systems/gba/src/video/objects.rs) composes regular OBJ rows in lanes.                                                                                                                                                                                                                              | Per-pixel objects and `apply_color_effect_at` remain for special cases; `regular_object_rows_match_the_per_pixel_object_renderer`, `vector_color_effects_match_the_per_dot_effects_unit` and `front_to_back_passes_preserve_priority_and_the_next_visible_bg` in [`video/tests.rs`](../systems/gba/src/video/tests.rs) cover them.                                                                                                                                              |
| **Scanline memo:** [`render_scanline`](../systems/gba/src/video/render.rs) retains a line only while registers and video-memory reads remain valid; marked changes invalidate it.                                                                                                                                                                                                                                                                                      | Full `compose_scanline` and `check_kept_scanline` re-render kept lines. `memoized_scanlines_match_fresh_renders_after_every_kind_of_input_change` in [`video/tests.rs`](../systems/gba/src/video/tests.rs) compares fresh and memoized frames; [research](research/scanline-memo.md).                                                                                                                                                                                           |
| **Selective video:** [`Session`](../systems/gba/src/session.rs) skips only pixel production on unpresented frames; [`VideoRecovery`](../systems/gba/src/video_recovery.rs) replays a bounded journal for exact on-demand pixels.                                                                                                                                                                                                                                       | Full-video execution and scratch replay are references. `selective_session_restores_exact_arbitrary_cycle_states_and_inputs_across_internal_frame_splits` in [`session.rs`](../systems/gba/src/session.rs) compares reconstructed states and frames; [research](research/selective-video.md).                                                                                                                                                                                   |

These tests complement the ZiGBA/Rust [parity traces](parity.md): optimized
paths must preserve timed access, IRQ/DMA ordering, full-frame pixels, DAC
samples and portable state bytes under different host slice sizes. Save-state
and browser behavior are described in [`design/`](design/save-states.md) and
[`sdk.md`](sdk.md).

## Differences, experiments and open questions

Stable Rust uses fixed-size lane loops for text rows, objects and color
effects where ZiGBA uses `@Vector` operations; the loops do not reliably
auto-vectorize. The per-dot paths remain references. The portable-state
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
