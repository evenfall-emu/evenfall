# Methodology

Evenfall builds deterministic, cycle-aware emulators one observable behavior
at a time. Each behavior needs evidence from outside its implementation before
it is trusted. This is the contribution method for the GBA port and the method
for building future systems in the family from scratch.

## Goals

The GBA core models CPU execution and timing, memory, interrupts, DMA, timers,
video, audio, BIOS services, save media, cartridge peripherals and serial
links. The same ROM, BIOS, configuration and timestamped inputs must produce
identical pixels, samples, battery saves and state bytes regardless of how the
host divides execution into calls. That determinism makes failures
reproducible and optimizations checkable. Terminal failures are typed faults
with cycle, instruction address and reason, not silent changes to execution;
see [`machine::Fault`](../systems/gba/src/machine.rs) and the CPU's
[`FaultReason`](../crates/cpu-arm/src/lib.rs).

Compilation, unit tests or agreement with one emulator do not alone establish
broad compatibility. Claims need representative, legally supplied software
and a reproducible configuration across boot paths, subsystems and devices,
with limitations and ambiguous hardware behavior stated.

## Evidence and the GBA oracle

Hardware claims rank their sources:

1. **Arm's manuals** for ARM7TDMI and ARMv4T instructions, modes, exceptions
   and bus cycles.
2. **Measured hardware behavior** and public Nintendo material.
3. **GBATEK** and maintained hardware-test reports, including their stated
   uncertainties.
4. **Mature emulators at pinned revisions** (such as mGBA and NanoBoyAdvance)
   for comparison, never code to copy.
5. **Project tests**, authoritative only through a traceable oracle from the
   ranks above.

When sources disagree, higher-ranked evidence governs; record an unresolved
choice as an assumption rather than treating emulator agreement as proof.
ROMs, BIOS images, saves and keys remain user-supplied and uncommitted.

For the **GBA port**, the pinned
[`ZiGBA`](../refs/zigba/src/root.zig) implementation is additionally the
executable oracle: the Rust core must match its guest behavior and its
frames, DAC audio, battery saves and complete `ZIGBAST1` state bytes. This
port-parity contract is distinct from ZiGBA's authority on actual hardware;
if measured hardware contradicts it, investigate the conflict rather than
silently changing the port. The
[`tools/parity`](../tools/parity/README.md) harness records ZiGBA checkpoints
for multiple host slice schedules; its trace comparator checks byte-exact
frame, audio, save and state blobs against the Rust core
([`parity.md`](parity.md)).
The Rust schema description must match the
ZiGBA v6 [`fixture`](../tools/parity/fixtures/schema-v6.bin) byte for byte;
its BLAKE3 digest in the state header proves the durable layout agrees. A
schema match does not prove the state **values** agree, so locked-byte tests
and full-state parity traces remain necessary
([`save-states.md`](design/save-states.md)).

## Changing behavior

1. **State the effect and evidence.** Name the observable behavior and its
   owner. First check the corresponding ZiGBA path and, for a hardware claim,
   the ranked sources above.
2. **Write the oracle first.** Choose inputs where a plausible wrong model
   gives a different result; derive expected values from manuals, measured
   hardware, analytic calculations, independent reference code or ZiGBA's
   original test oracle. Never regenerate an expectation from the Rust code
   under test. Run the focused test against the unchanged code first.
3. **Implement the whole behavior.** Account for register effects, cycle
   costs, event ordering, open bus, audio/video output and save-state
   persistence. Keep ZiGBA's ordering during the port.
4. **Iterate locally, then check broadly.** Run the owning tests in debug and
   release, affected probes and validators, and parity traces at different
   host slice sizes. A visual claim needs frame evidence; an audio claim needs
   sample evidence. Record media identity, boot mode, save type, inputs,
   endpoint and oracle for any compatibility result.

Porting does not provide a license to fix a suspected ZiGBA bug or replace
its behavior for speed. Record a deliberate divergence with evidence and a
follow-up in [`docs/roadmap.md`](roadmap.md), decide its compatibility effect,
then change behavior only with that recorded decision and discriminating
tests. For example, rewind's miniz_oxide compressor produces different
compressed bytes but preserves mutually decodable bundles and byte-identical
decoded states; this does not change guest behavior
([`rewind.md`](design/rewind.md)). A durable-layout change requires an explicit
state-version and compatibility decision, not just a passing round trip.

## Building up from nothing

For a future system, begin with determinism and timing; those are hardest to
retrofit. The GBA is a direct port, not an invitation to add placeholder
systems or a cross-ISA CPU abstraction.

| Stage                    | Build                                                                               | Evidence before moving on                                |
| ------------------------ | ----------------------------------------------------------------------------------- | -------------------------------------------------------- |
| 1. Deterministic shell   | headless core, master clock, trace, media loader and test harness                   | identical traces and states for the same input           |
| 2. Bus                   | region decode, widths, mirroring and timed access                                   | region, width, mirror and alignment tests                |
| 3. CPU                   | ISA, modes, exceptions, pipeline-visible PC and boot entry                          | self-checking instruction ROMs                           |
| 4. Interrupts and timers | interrupt entry/return, timers, low-power states                                    | timer and exception timing tests                         |
| 5. Timed bus and DMA     | wait states, prefetch and DMA arbitration                                           | recorded access and transfer timing                      |
| 6. Video                 | display timing, bitmaps, tile modes, objects, windows and effects                   | timing tests and exact frames                            |
| 7. BIOS                  | user-supplied BIOS execution, then original HLE services                            | arithmetic, copy and decompression tests                 |
| 8. Save media            | device protocols, detection, overrides and persistence                              | protocol tests and saves across restart                  |
| 9. Audio                 | synthesis, timer/DMA FIFO coupling and deterministic resampling                     | sample-exact waveforms                                   |
| 10. Breadth              | raster effects, open-bus edges, peripherals, serial links and save-state versioning | test-ROM and legally supplied software regression matrix |

A flat memory array or frame-at-a-time renderer can help bring-up, but loses
the width, wait-state and mid-scanline timing later stages need. Recompilers,
GPU rendering and game-specific shortcuts wait until the interpreter, bus and
event model are trustworthy.

## Probe ROMs and reference suites

[`probe/`](../probe/README.md) holds bare-metal ARMv4T ROMs that exercise
paths public suites cannot isolate. They report a value through `r12` at a
fixed idle self-loop; the Makefile verifies the address, and the validator
requires the stable endpoint before inspecting the value, frame, audio or
save. Run `make -C probe` to build them. The pinned ZiGBA checkout provides
the [`validate` target](../refs/zigba/build.zig) and oracle expectations;
the Rust validator target is a separate test layer as it lands. The parity
harness already runs single-machine probe scenarios through ZiGBA; linked
cases need the multi-machine validator.

A probe must be able to fail. One that waits until its own counter reaches N
and reports N proves nothing about missed or duplicate events. Bracket counts
with an independent event, derive expected values outside the emulator, and
inject plausible faults into the model to check that the probe reports a
different result. A surprising result first warrants checking the probe's
reading of the hardware evidence.

External suites live at pinned, gitignored revisions under `refs/`, never as
committed ROMs. The self-checking jsmolka GBA suite supplies boot-path and
hardware validators; recorded hardware tables from suites that cannot be
built can be used as source-derived host fixtures, with the origin of each
expected value explicit. Agreement with one comparison emulator is not a
hardware oracle.

## Test layers and commands

A test earns its place when a plausible error fails it without duplicating
another layer. Use the smallest seam that still observes the real behavior:

| Layer                   | Rust location or owner                                                                                                             | Catches                                                      |
| ----------------------- | ---------------------------------------------------------------------------------------------------------------------------------- | ------------------------------------------------------------ |
| Unit tests              | beside Rust sources, e.g. [`video/tests.rs`](../systems/gba/src/video/tests.rs), [`apu/tests.rs`](../systems/gba/src/apu/tests.rs) | device state machines, timing, codecs and host policy        |
| Crate integration tests | [`crates/state/tests/codec.rs`](../crates/state/tests/codec.rs), [`crates/replay/tests/trace.rs`](../crates/replay/tests/trace.rs) | wire formats, envelope errors and slice-invariant replay     |
| GBA integration tests   | `systems/gba/tests/` as test ports land                                                                                            | machine and bus ordering that a single ROM cannot isolate    |
| ROM validators          | [`probe/`](../probe/README.md), pinned suites and validator target                                                                 | full guest behavior on the ordinary execution path           |
| Differential tests      | beside optimized paths, e.g. [`cpu-arm`](../crates/cpu-arm/src/lib.rs) and [`video`](../systems/gba/src/video/tests.rs)            | optimized versus retained reference behavior                 |
| Parity traces           | [`tools/parity`](../tools/parity/README.md) and [`crates/replay`](../crates/replay/src/lib.rs)                                     | Zig/Rust byte differences and dependence on host slice sizes |
| Benchmarks              | Divan benchmark targets as they land, with [`tools/bench/roms`](../tools/bench/roms/Makefile) workloads                            | measured speed without relaxing exactness                    |

The existing workspace checks are:

```sh
cargo fmt --all --check
cargo clippy --workspace --all-targets -- -D warnings
cargo test --workspace
cargo test --workspace --release
make -C probe
cargo run -p evenfall-parity -- oracle build
cargo run -p evenfall-parity -- run tools/parity/scenarios/probe-timer_irq.json
```

The final two commands build the **ZiGBA** oracle and compare it against the
Rust GBA core. Inspect saved traces with `cargo run -p evenfall-parity --
compare ORACLE.trace RUST.trace`. The original ZiGBA validator target runs
from a scratch checkout with locally supplied media; no ROM or BIOS is
committed to the repository.
Run benchmark targets on declared workloads with the same guest inputs,
full software rendering and exact audio before accepting a speed claim.

## Keeping optimized code exact

- Keep the ordinary reference path in the tree and compare it against every
  optimized path. The per-beat Thumb RAM transfer, single-cycle prefetch and
  scalar rendering paths remain test oracles for their faster counterparts.
- Compare asymmetric boundary cases and randomized valid inputs where they
  expose differences; assert cycles, state, memory, pixels and audio, not only
  that both versions return successfully.
- Check caches against fresh computation in debug and tests. A cache's own
  metadata must not be the oracle for its correctness.
- Run scripted traces under different slice schedules. Timed inputs belong to
  absolute guest cycles, not host call boundaries; drain audio consistently.
  A pure optimization ends with identical CPU state, cycle, frame, memory,
  audio, battery save and portable state bytes.
- Measure only after parity holds. Keep optimizations that improve a declared
  workload without weakening exactness; keep the evidence with the relevant
  benchmark or performance research.

Existing examples include `thumb_ram_blocks_match_per_beat_reference` in
[`cpu-arm`](../crates/cpu-arm/src/lib.rs),
`ordinary_and_effects_aware_text_spans_agree_across_map_wraps_flips_and_transparency`
in [`video/tests.rs`](../systems/gba/src/video/tests.rs),
`noise_jump_table_matches_per_step_lfsr_advance` in
[`apu/tests.rs`](../systems/gba/src/apu/tests.rs), and
`input_deadline_audio_and_trace_survive_different_slices` in
[`replay/tests/trace.rs`](../crates/replay/tests/trace.rs). The schema fixture
test in [`machine.rs`](../systems/gba/src/machine.rs) checks structural
parity independently of a state round trip.
