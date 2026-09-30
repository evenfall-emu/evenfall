# Roadmap

- Color effects use exact scalar lanes; evaluate stable-Rust SIMD alternatives against the per-dot reference and measure before replacing the hot path.
- Rewind copy deltas are measured on two ROM hacks with synthetic input ([research](research/rewind-compression.md#codec-choice)); human play, menus and other games remain unmeasured. Game B's walker session ends after 82 emulated minutes at a guest fault that the game causes ([compatibility](compatibility.md#guest-faults-in-user-supplied-games)); longer Game B sessions need input that avoids its battle information menu.
- Machine, Session and Bus share read-only `bus::Media` handles across reset, state-load scratch, rewind bundle validation and video recovery; media digests are cached at construction. The private Vec backing preserves an owned ROM upload's allocation, and release codegen hoists the ROM data pointer before scalar byte reads. Pinned-core uncached ROM reads, ROM-to-RAM DMA and full thumb-logic, arm-mixer and bitmap-DMA workloads show no consistent speed advantage from `Arc<[u8]>`; keep the Vec-backed zero-copy upload path unless a repeatable gain justifies a different upload allocator.
- **Decision: BIOS read protection with an external image** (a deliberate guest-visible difference from ZiGBA, which returns the image's bytes to every read). Evidence: GBATEK, Unpredictable Things: Reading from BIOS Memory (reads while the PC is outside the BIOS return the most recently fetched BIOS opcode, with its examples at `[DCh+8]`, `[134h+8]`, `[13Ch+8]`, `[188h+8]`), and the jsmolka BIOS ROM's four expectations, which are those opcodes of Nintendo's BIOS. Model: opcode fetches latch the aligned BIOS word; CPU data reads see the image only while the latest opcode fetch was from the BIOS; DMA never reads the BIOS ([memory](gba/hardware/memory.md#bios-read-protection)). Pinned NanoBoyAdvance (gate on `r15`) and mGBA 0.10.5 (gate on the active fetch region, latch on leaving) agree on the gate and differ on latching BIOS data reads, which stays an open question. Compatibility effect: external and skip-BIOS runs whose code reads the BIOS region see the latch instead of image bytes; no state-layout change (`bios_latch` is already durable; the gate is derived). The parity harness applies [`bios-protection.patch`](../tools/parity/patches/bios-protection.patch) to its ZiGBA copy so both sides agree. Follow-up: check the latch sources against a user-supplied Nintendo BIOS and a hardware probe.
- `Machine::write_state` buffers its bounded payload before writing to the caller's `Write`, unlike ZiGBA's direct streaming writer. The bytes are identical, but this adds a payload allocation and copy; consider a writer-backed codec sink if streaming cost matters.

## DMA that never releases the bus

**Divergence.** ZiGBA ends a DMA burst that holds the bus for
`max_dma_burst_cycles` (2^24 cycles) with a terminal `dma_livelock` fault and
turns DMA off. Evenfall keeps the machine running: the CPU stays stalled
while video, audio, timers and interrupt flags advance, and it resumes if the
guest's DMA ever releases the bus. This changes behavior only where ZiGBA
stops; every run ZiGBA completes is unchanged, and the parity oracle applies
[`dma-stall.patch`](../tools/parity/patches/dma-stall.patch) so ZiGBA behaves
the same ([hardware page](gba/hardware/timers-dma-interrupts.md#bursts-that-never-release-the-bus)).

**Evidence.** GBATEK's DMA section: "The CPU is paused when DMA transfers
are active", channels rank DMA0 to DMA3 and "lower priority are paused until
channels with higher priority have completed", and a Sound DMA request comes
whenever the FIFO holds only 16 bytes. Requests that return faster than
transfers complete therefore keep the CPU paused indefinitely while every
other device runs. For comparison only: pinned mGBA 0.10.5 services DMA as
scheduler events between instructions and, while `cpuBlocked` is set, ticks
events without running the CPU and returns to the frontend at its own
deadline; the blocked flag and each channel's remaining count, source and
destination are in its save states. Pinned NanoBoyAdvance runs pending DMA
at the start of the CPU's next bus access and loops until no channel is
active, so such a burst never returns to its host.

**Design.** ZiGBA and the port service a burst inside the CPU access it
follows, possibly mid-instruction. Suspending that access across host calls
would need a resumable instruction and bus access (a coroutine, which stable
Rust and single-threaded WebAssembly do not offer, or a transactional replay
of the partial instruction through every bus path); moving all DMA to
instruction boundaries, as mGBA does, would change every DMA's timing and
break parity. The core therefore keeps ZiGBA's in-access burst up to the same
limit and makes only the burst resumable:

- At the limit the burst yields at its next transfer start or beat boundary
  (`Bus::run_dma_burst`). It records each transfer it interrupted (source,
  destination, control, count and beat, `DmaProgress`), keeps its pending
  requests and keeps `servicing_dma` set, so no DMA runs while the
  interrupted instruction completes, as it does in ZiGBA before its fault.
- From that instruction boundary `Machine` continues the burst with
  `Bus::continue_dma_stall` up to each host deadline. The burst resumes the
  outermost interrupted transfer, which resumes the ones nested in it, in the
  order the uninterrupted recursion would run them, so results do not depend
  on slicing. When no request remains the bus is released, the CPU takes any
  interrupt it would take at that boundary (`Cpu::take_irq_at_boundary`) and
  runs on.
- `Machine::step` during a stall returns `Waiting(WaitReason::DmaStall)`
  without advancing; `wait_until` accepts the stall and continues it; linked
  runs continue a stalled machine as they idle a halted one. The SDK's
  `step()` reports it as its own reason, `dma_stall`
  ([`sdk.md`](sdk.md)).

**Consequences.**

- The host call in which a burst reaches the limit still returns about 2^24
  cycles (one second of guest time) late, at the first instruction boundary
  after the limit, because the burst runs inside a CPU access until then.
  Later calls return at their deadline or the first DMA beat boundary after
  it. That one call's audio exceeds the 2048-sample host queue, so its
  oldest samples are lost to the host, as in any call that long.
- The instruction the limit interrupted completes before the stall instead
  of after it: at most one instruction's remaining accesses happen early.
  Bursts shorter than the limit, and everything after the stall begins,
  follow the in-access model exactly.
- Save states are refused while a stall is in progress: its interrupted
  transfers' remaining counts have no durable `ZIGBAST1` v6 field, and
  ZiGBA would read requests pending at an instruction boundary as a burst
  that starts after the next CPU access. `write_state` returns `InvalidSaveState`, rewind
  skips its captures (history keeps the states before the stall, so rewinding
  leaves it), and the web runtime's explicit saves fail until the stall ends.
  Making stalls durable needs a new state version with per-channel transfer
  progress, a separate decision.
- The parity scenario
  [`probe-dma_stall`](../tools/parity/scenarios/probe-dma_stall.json) runs
  [`probe/dma_stall.c`](../probe/dma_stall.c), a stall that a repeating VBlank
  DMA0 ends by stopping the timers after 64 frames, with a checkpoint after
  the stall ends. It sets `audioOutput: false` because the call reaching the
  limit overflows the sample queue differently under each slicing; emulated
  sound is unaffected.

**Decisions.**

- The in-access limit stays at 2^24 cycles, with its one-time overrun of
  about a second. A lower limit would shorten it but diverge from ZiGBA on a
  burst that runs between the two limits and then ends; revisit only with
  measurements from real games.
- The SDK reports the stall as `dma_stall`, distinct from `halted`.
- Durable stalls are deferred: saves stay refused during a stall and the
  state version is unchanged. Saving one needs per-channel transfer
  progress in a new state version.

## mGBA suite ROM findings

The mGBA suite ROM's hardware tables contradict several behaviors that
Evenfall inherits from ZiGBA ([compatibility](compatibility.md#deviations-from-the-recorded-hardware)).
Each fix below diverges from ZiGBA, so each needs a recorded decision, a
note in [`parity.md`](parity.md) for the scenarios it changes, and
replacement of the ported ZiGBA tests that pin the current behavior. After a
change, `evenfall-parity mgba-suite report` must show the named tests
passing and no other test changing, and the jsmolka, probe and hardware-table
suites must still pass.

- **Decision: write-only I/O reads as open bus** (a deliberate guest-visible
  difference from ZiGBA). A write-only register reads like an unused one:
  open bus when no half of its word is readable, zero beside a readable
  half. The write-only registers are the BG scroll offsets, affine
  parameters and reference points, window bounds, `MOSAIC`, `BLDY`, the DMA
  addresses and counts and the sound FIFOs; ZiGBA reads the first four
  groups' written values or zero. Reads of `BG0CNT`/`BG1CNT` clear bit 13
  and reads of `DMA0CNT_H`–`DMA2CNT_H` clear bit 11; the registers keep the
  written bits, which have no effect there. Evidence: the mGBA suite's I/O
  read table (`src/io-read.c`, recorded on hardware), which GBATEK's
  "Unpredictable Things" agrees with for `DMA0SAD` and unused words; its
  `MOSAIC` example, which calls the register readable, is contradicted by the
  table ([memory](gba/hardware/memory.md#open-bus-and-unused-io)).
  Compatibility effect: guest reads of those registers see open bus or zero
  instead of stored values; the pre-decoded engine leaves loads of the DMA
  addresses to the interpreter, which holds the open-bus latch. No
  state-layout change. The parity harness applies
  [`io-read.patch`](../tools/parity/patches/io-read.patch) to its ZiGBA copy.
  The ported test "unmapped and unused MMIO reads distinguish open-bus words
  from paired zero halves" takes its paired-zero example at `66h` instead of
  `4Eh`.
- **Decision: Thumb opcode fetches compose the open-bus word per region**
  (a deliberate guest-visible difference from ZiGBA, whose Thumb fetch
  leaves its halfword zero-extended). A Thumb fetch over a 16-bit bus
  (EWRAM, palette RAM, VRAM, Game Pak ROM) leaves its halfword on both
  halves of the open-bus latch; one from the BIOS or OAM leaves the whole
  aligned word; one from IWRAM replaces only its own half, so the other half
  keeps the bus's previous value, a data word when the previous instruction
  loaded one. ARM fetches and data reads are unchanged. Evidence: GBATEK,
  "Unpredictable Things: Reading from Unused Memory", which lists these
  compositions, and for ROM code the mGBA suite's hardware tables: the upper
  halves of unused and write-only I/O words read the prefetched opcode
  (`src/io-read.c`), and "DMA Prefetch" (`src/misc-edge.c`) reads
  `4003h` in both halves. The BIOS, OAM and IWRAM rules follow GBATEK
  alone; IWRAM's is the bus-level form of GBATEK's `OldLO`/`OldHI` note,
  where mGBA 0.10.5 approximates the other half with the previous opcode.
  Compatibility effect: guest reads of unused memory from Thumb code see
  the new word. Game B's runaway copy reads `D1F9D1F9h` instead of
  `0000D1F9h`; neither holds `FFh`, but the copy writes different bytes,
  so where it ends may change; the game is user-supplied and was not re-run
  ([compatibility](compatibility.md#guest-faults-in-user-supplied-games)).
  The pre-decoded engine composes the latch it leaves the same way: in
  IWRAM it tracks where the last load (or the run's start) left the latch
  and whether a refill followed. No state-layout change. The parity harness
  applies [`thumb-open-bus.patch`](../tools/parity/patches/thumb-open-bus.patch)
  to its ZiGBA copy. The ported test "active IWRAM fetches preserve mirrors
  wrapping widths and complete bus effects" expects the composed halfword
  latch. "DMA Prefetch" now reads the DMA's last value; its break address
  still differs (see the HBlank item below).
- **Decision: the HBlank flag rises at cycle 1006** (a deliberate
  guest-visible difference from ZiGBA, which raises it with the HBlank IRQ
  at 960). The IRQ and HBlank DMA request stay at 960. Evidence: GBATEK, "LCD
  I/O Interrupts and Status" (the flag "is "0" for a total of 1006
  cycles"), and the mGBA suite's "H-blank bit start" flips 4 to 6, which
  match for a rise at 1006 or later and for no earlier cycle
  ([video](gba/hardware/video.md#lcd-timing)). The test still fails: its
  first flips and its wake interval run through the `Halt` wake path, whose
  BIOS cost HLE does not charge (flip 1 reads 89 cycles short under HLE, 49
  under the replacement BIOS), and the wake interval differs from the
  recorded 1,232 by three Thumb instructions that this build of the suite
  schedules after the second `Halt` and not after the first, so no emulator
  can reproduce it with this ROM build
  ([compatibility](compatibility.md#hblank-flag)). No state-layout change:
  the flag is derived from the line position. The parity harness applies
  [`hblank-flag.patch`](../tools/parity/patches/hblank-flag.patch). The
  ported test "LCD timing follows the GBATEK 960/1232-cycle line and
  228-line frame" reads the flag at 1006.
- **DMA count latch.** Latch `DMAxCNT_L` when the enable bit rises and reload
  it only on repeat. Evidence: "DMA count latching". Every DMA suite test
  must keep passing.
- **Decision: flag-setting multiplies set C from the multiplier's carry
  vector** (a deliberate guest-visible difference from ZiGBA, which leaves
  C unchanged). Every failing check of the mGBA suite's multiply-long table
  (`src/multiply-long.c`, recorded on hardware) differed only in C after
  `SMULLS` or `UMULLS`. The model is zayd's reconstruction of the ARM7TDMI
  multiplier from its patents and fuzzing on hardware
  (<https://bmchtech.github.io/post/multiply/>): Booth recoding, four
  carry-save stages per cycle, early termination, and C as the barrel
  shifter's carry-out when the ALU realigns the carry vector
  ([cpu](gba/hardware/cpu.md#the-carry-flag-after-a-multiply)). Evenfall
  implements it in `evenfall_cpu_arm::multiply_carry` for every
  flag-setting multiply, `MULS` and `MLAS` included, since they share the
  datapath; it reproduces the whole table (N, Z, C and both result words)
  and agrees with the model's zlib-licensed reference implementation on two
  million random operands of all six forms, run as a scratch differential.
  The suite does not measure `MULS`, `MLAS`, `UMLALS` or `SMLALS`; for those
  the model's hardware fuzzing is the evidence. Compatibility effect: C
  after a flag-setting multiply; non-flag-setting multiplies are unchanged.
  No state-layout change. The parity harness applies
  [`multiply-carry.patch`](../tools/parity/patches/multiply-carry.patch).
  The ported test "execution region multiply keeps original alias timing
  and conditional effects" expects the model's C instead of the preserved
  flag.
- **Decision: block transfers give the SRAM bus their exact addresses** (a
  deliberate guest-visible difference from ZiGBA, which aligns every
  block-transfer beat). `LDM`, `STM`, Thumb `LDMIA`, `STMIA`, `PUSH` and
  `POP` beats on the 8-bit save bus keep the base's low bits and step by 4,
  as single transfers already do; other regions still see aligned beats.
  HLE word `CpuSet` and `CpuFastSet` copy the same way. Evidence: the mGBA
  suite's memory table (`src/memory.c`, recorded on hardware) loads the
  SRAM byte at an unaligned `CpuSet`/`CpuFastSet` source in all four lanes
  and stores at an unaligned destination without touching the aligned
  byte, in every `swi B 32` and `swi C 32` row; GBATEK's "BIOS Memory Copy"
  confirms that the retail word loops are `LDMIA`/`STMIA` (`r3`, and
  `r2`–`r9` for `CpuFastSet`), so the CPU's block transfers, not only the
  HLE services, present those addresses
  ([memory](gba/hardware/memory.md#region-behavior)). Compatibility effect:
  block transfers on SRAM at an unaligned address, and word copies there
  under HLE and any BIOS. No state-layout change; the HLE work state keeps
  the exact addresses in its existing fields. The parity harness applies
  [`sram-block.patch`](../tools/parity/patches/sram-block.patch).
- **Decision: HLE halfword `CpuSet` loads and stores like `LDRH` and
  `STRH`** (a deliberate guest-visible difference from ZiGBA, whose HLE
  aligns both addresses). Each halfword unit loads the aligned halfword
  rotated right by 8 at an odd address, so a copy from an odd source stores
  the high bytes, and on the 8-bit save bus both the load and the store use
  the exact address. Evidence: GBATEK's "BIOS Memory Copy" names the retail
  loop's `LDRH`/`STRH r3,[r0,r5]`, and the mGBA suite's memory table
  (`src/memory.c`, recorded on hardware) reads `00DE00BEh` from `DEADBEEFh`
  and `00650061h` from SRAM in every `swi B 16 (unaligned)` row; the
  replacement BIOS, which uses `LDRH`, already passes them
  ([bios](gba/hardware/bios.md#copies)). A halfword fill loads its value the
  same way. The word forms are unchanged here. Compatibility effect:
  halfword `CpuSet` calls with an odd source or an SRAM address. The HLE
  work state keeps bit 0 of both addresses in its existing fields; no
  state-layout change. The parity harness applies
  [`cpu-set-halfword.patch`](../tools/parity/patches/cpu-set-halfword.patch).
- **Decision: an HLE `IntrWait` that an IRQ return ends leaves the SWI
  latch** (a deliberate guest-visible difference from ZiGBA). The HLE
  IRQ return sets the protected-BIOS latch to `E55EC002h`, the opcode after
  the retail trampoline's return; when that return ends a pending
  `IntrWait`, the retail BIOS goes on to return from the SWI, so the latch
  becomes `E3A02004h`. A return that leaves the wait pending keeps
  `E55EC002h`. Evidence: GBATEK, "Reading from BIOS Memory" (the opcode at
  `[0188h+8]` after SWI execution), and the mGBA suite's "BIOS load" row,
  which reads `E3A02004h` after `VBlankIntrWait` on hardware
  ([memory](gba/hardware/memory.md#bios-read-protection)). Compatibility
  effect: BIOS-region reads after an interrupt wait under HLE. No
  state-layout change. The parity harness applies
  [`intr-wait-latch.patch`](../tools/parity/patches/intr-wait-latch.patch).
- **Timer enable and IRQ latency.** The timer IRQ suite and the Calibration
  row show a timer counting one cycle early after its enabling write and an
  overflow IRQ taken several cycles early. Pinned NanoBoyAdvance applies a
  timer control write one cycle late; a one-cycle enable delay alone fits the
  prefetch-on ROM, EWRAM and IWRAM Calibration columns but leaves the
  prefetch-off ROM columns one to three cycles low, so the model is still
  incomplete. Isolate the enable delay and the IF-to-IRQ-entry delay with
  probes before changing either; the timer count-up and SIO timing suites
  then show how much of the rest is BIOS cost.
- **HLE BIOS costs.** The timing suite records retail cycle counts for
  `Div`, `Sqrt`, `ArcTan` and `CpuSet`, and the IRQ-timed suites read the
  retail trampoline's cost. Charging them would make HLE timing-faithful for
  those services; it is a change to the HLE's stated goal
  ([`bios.md`](gba/hardware/bios.md#approximations)), not a bug fix.

## Player and Cloudflare server migration

ZiGBA's player and server live in the pinned `refs/zigba` checkout, not in the
Evenfall runtime. A move into this repository keeps the existing GBA library,
cloud saves, shared links and deployment policy usable. The source locations
below identify the current assumptions; **per-system** means the shared
interface or identity needs a system dimension, **GBA-specific** means the
value or integration stays with the GBA implementation, and **naming** means
the old name needs a compatibility-aware migration rather than a new hardware
rule.

### Player and local data

| Assumption                                                                                                                                             | ZiGBA source and symbol                                                                                                                                                                                                                                                      | Disposition                                                                                                                                                        |
| ------------------------------------------------------------------------------------------------------------------------------------------------------ | ---------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------- | ------------------------------------------------------------------------------------------------------------------------------------------------------------------ |
| The screen and rewind preview are 240×160; fullscreen and handheld layouts are 3:2.                                                                    | `frontend/src/console-player.tsx` `ConsolePlayer`; `frontend/src/rewind-timeline.tsx` `RewindTimeline`; `frontend/src/console.css` `.game-display`, `.game-display:fullscreen canvas`                                                                                        | **Per-system:** select geometry with the loaded system; 240×160 is the GBA value.                                                                                  |
| Frames are 38,400 packed 15-bit pixels, expanded through a 32,768-entry RGB LUT.                                                                       | `frontend/src/media.ts` `draw`, `VideoOutput`                                                                                                                                                                                                                                | **Per-system:** describe frame format and dimensions at the video boundary; this LUT is **GBA-specific**.                                                          |
| The player accepts `.gba` and single-game ZIPs; a chosen file is limited to 40 MiB, while patched raw output is limited to 32 MiB.                     | `frontend/src/console-player.tsx` `ConsolePlayer`; `frontend/src/main.tsx` file inputs; `frontend/src/player.ts` `Player.inspectRom`; `frontend/src/patch.ts` `MAX_ROM_SIZE`                                                                                                 | **Per-system:** dispatch imports and validate raw/output limits by system. GBA retains these file types and bounds.                                                |
| The SDK, bridge and optional game package are GBA packages; the bridge's exact-build recognition selects Pokémon dashboards.                           | `frontend/src/services.ts` `makeEmulator`, `Session`; `frontend/src/player.ts` `Player`; `frontend/src/main.tsx` `BridgeProvider`; `frontend/src/games.ts` `GamePlugin`, `createGameRegistry`, `pokemonPlugin`; `frontend/package.json` dependencies                         | **Per-system:** select the runtime and bridge before opening a game. The Pokémon plugin stays **GBA-specific**; keep the plugin registration seam.                 |
| ROM SHA-1 alone identifies games, saves, quick slots, playthroughs, builds and single-writer locks.                                                    | `frontend/src/library.ts` `Game`, `PlaythroughRef`, `runKey`, `quickStateKey`, `playthroughLockKey`, `Library.builds`; `frontend/src/storage.js` `acquireSaveLock`; `frontend/src/games.ts` `createGameRegistry`                                                             | **Per-system:** include system identity in keys, locks and recognition; preserve existing GBA SHA-1 keys while migrating data.                                     |
| IndexedDB `zigba-local` v8 holds games, ROMs, saves, resumes, assets, cloud jobs, playthroughs and builds; legacy `SaveStore` opens the same DB at v2. | `frontend/src/library.ts` `openLibrary`; `frontend/src/storage.js` `SaveStore.open`                                                                                                                                                                                          | **Naming:** migrate/alias the DB without dropping old GBA data or breaking older tabs; add a system dimension to its records/keys (**per-system**).                |
| Offline caches and preferences retain ZiGBA names.                                                                                                     | `frontend/offline.ts` `offlinePlayer` (`zigba-player-*`); `frontend/src/media.ts` `AudioOutput` (`zigba-volume`, `zigba-audio` worklet); `frontend/src/app-update.ts` `AppUpdate` (`zigba-update-reloaded-at`); `frontend/src/storage.js` `acquireSaveLock` (`zigba-save-*`) | **Naming:** coordinate cache eviction, worklet registration, preferences and old-tab locks; do not discard active versions or locks by renaming them in isolation. |
| The catalog's offline cache is shared, but dump names are a GBA-only No-Intro table.                                                                   | `frontend/src/pages/library/use-catalog.ts` `useCatalog` (`evenfall-catalog`), `useDumpNames` (`no-intro-gba.json`)                                                                                                                                                          | **Per-system:** distinguish catalog entries and cached selections; keep the GBA dump-name table **GBA-specific**.                                                  |

### Cloud protocol and catalog

| Assumption                                                                                                                                                                                          | ZiGBA source and symbol                                                                                                                                                                               | Disposition                                                                                                                                                                                                                                                             |
| --------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------- | ----------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------- | ----------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------- |
| State downloads and uploads check the `ZIGBAST1` prefix and use a 2 MiB decoded-state bound; shares identify a ROM by 40-character SHA-1.                                                           | `frontend/src/cloud.ts` `STATE_MAGIC`, `MAX_STATE`, `fetchSharedState`, `CloudSync`; `server/src/cloud-saves.ts` `STATE_MAGIC`, `MAX_RESUME`, `HASH`, `putSave`, `registerCloudSaveRoutes`            | **Per-system:** dispatch format and size checks by system, retaining the `ZIGBAST1` v6 contract for GBA. The server's magic check is only a prefix screen, not a full decode.                                                                                           |
| Battery uploads must be one of 512 B, 8, 32, 64 or 128 KiB and stay under 128 KiB.                                                                                                                  | `server/src/cloud-saves.ts` `BATTERY_SIZES`, `MAX_BATTERY`, `putSave`                                                                                                                                 | **Per-system:** validate against each system's save-media formats; these sizes remain **GBA-specific**.                                                                                                                                                                 |
| Shared ROM downloads expect at most 32 MiB raw, at most 33 MiB gzip, and name the result `shared.gba`; the server bounds the uploaded compressed body at 33 MiB.                                    | `frontend/src/cloud.ts` `MAX_ROM`, `MAX_ROM_COMPRESSED`, `fetchSharedRom`; `server/src/cloud-saves.ts` `MAX_ROM_COMPRESSED`, `registerCloudSaveRoutes`                                                | **Per-system:** advertise the media type and independent raw/compressed limits. Preserve GBA's bounds; the player's 40 MiB import limit is a separate policy.                                                                                                           |
| Requests and replies use `X-Zigba-Account`, `-Rom`, `-Revision`, `-Captured-At`, `-Digest`, `-Title`, `-Playthrough-Name`, `-Uploaded-At` and `-Rom-Included`.                                      | `frontend/src/cloud.ts` `CloudSync.share`, `CloudSync.flush`, `fetchSharedState`, `fetchSharedRom`; `server/src/cloud-saves.ts` `saveAccount`, `putSave`, `saveHeaders`, `registerCloudSaveRoutes`    | **Naming:** version client and server together, accepting old headers for GBA during transition. Auth, conditional ETags and compressed-body SHA-256 integrity remain shared protocol behavior; add a system discriminator to the save/share contract (**per-system**). |
| Save URLs and R2 keys identify an account, ROM hash, playthrough and kind; old `default` keys and public `share/state` and `share/rom` keys lack system identity.                                   | `frontend/src/cloud.ts` `CloudSync`; `server/src/cloud-saves.ts` `registerCloudSaveRoutes`, `shareKey`, `shareRomKey`; `server/src/schema.ts` `playthrough`                                           | **Per-system:** namespace routes, D1 playthrough identity and R2 keys before mixed-system cloud writes; keep legacy GBA links, listings, deletion and account-erasure sweeps readable.                                                                                  |
| Catalog releases and bases use global SHA-1 uniqueness, No-Intro SHA-1/CRC32 dumps, BPS/UPS/IPS patch metadata and save compatibility between versions.                                             | `server/src/schema.ts` `games`, `bases`, `versions`, `versionSaveCompatibility`; `server/src/types.ts` `baseInput`, `versionInput`; `server/src/catalog.ts` catalog queries                           | **Per-system:** give games/bases/versions a system identity and define hash/patch validation where relevant. The current GBA catalog's No-Intro bases and patch profiles remain **GBA-specific**.                                                                       |
| The shipped seed and dump-name data cover GBA releases and Pokémon hacks.                                                                                                                           | `server/src/seed.ts` `seedCatalog`; `server/src/romhacks.json`; `frontend/src/pages/library/use-catalog.ts` `useDumpNames`                                                                            | **GBA-specific:** retain the GBA catalog and plugin data without treating them as defaults for another system.                                                                                                                                                          |
| The frontend and Worker share `/api/games`, `/api/versions`, `/api/bases`, `/api/admin/*`, `/api/auth/*`, `/api/saves`, `/api/shares` and SPA paths `/library`, `/player`, `/share/state/:shareId`. | `frontend/src/pages/library/use-catalog.ts` `useCatalog`; `frontend/src/cloud.ts` `CloudSync`; `server/src/index.ts` `app`, SPA path allowlist; `server/src/cloud-saves.ts` `registerCloudSaveRoutes` | **Per-system:** return/disambiguate systems in catalog, saves and shares; keep auth and ordinary navigation shared. Preserve existing GBA URLs during transition.                                                                                                       |

### Build and deployment boundary

| Assumption                                                                                                                                                                             | ZiGBA source and symbol                                                                                                                                                                                        | Disposition                                                                                                                                                                                                                                                                                            |
| -------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------- | -------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------- | ------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------ |
| The player workspace is `zigba-player`, resolves sources with `zigba-source`, writes `ZIGBA_PLAYER_DIR`, and emits SDK worker, WASM and audio worklet as hashed assets.                | `frontend/package.json` `name`; `frontend/vite.config.ts` `sourceConditions`, `build`; `frontend/offline.ts` `offlinePlayer`                                                                                   | **Naming:** replace ZiGBA names with compatibility for existing cached assets. **Per-system:** bundle/select each system's SDK without mixing incompatible worker/WASM versions.                                                                                                                       |
| Deployment builds a Zig web SDK and checks for `assets/zigba-*.wasm`; it publishes `licenses/ZiGBA.txt` and rejects `.gba`, `.sav`, `.state`, BIOS/archive files.                      | `tools/build-cloudflare.sh` `zig_bin`, `zig build web`, `compgen`, `private_files`, license copy                                                                                                               | **Per-system:** build and check Rust SDK/WASM outputs; retain the user-media rejection rule and the GBA license notice. Asset/license names are **naming** work, not a reason to loosen the publication guard.                                                                                         |
| The hosted Worker/domain, local dev Worker, D1 `zigba-catalog` and R2 `zigba-saves` are instance-specific; a separate patch Worker serves the patch bucket.                            | `wrangler.jsonc` `name`, `routes`, `assets`, `d1_databases`, `r2_buckets`; `server/wrangler.jsonc` local bindings; `server/patches.wrangler.jsonc` `routes`; `server/src/patches.ts` `fetch`                   | **Naming:** plan deployed resource/domain changes separately from source relocation; retain bindings and old data until migrated. The patch CDN remains separate; its BPS/UPS/IPS key filter and allowed origins need review for each hosted patch system (**per-system**).                            |
| Worker auth and mutating saves require the configured origin; the patch CDN allows only listed origins. Static headers specify `nosniff`, referrer policy and immutable hashed assets. | `server/src/auth.ts` `createAuth`; `server/src/index.ts` auth/origin middleware; `server/src/cloud-saves.ts` `uploadRejected`; `server/src/patches.ts` `origins`; `tools/cloudflare-headers` `/*`, `/assets/*` | **Naming:** update trusted origins only as deployment domains change, while keeping same-origin checks and private-save policy. Static caching remains shared; these headers do not currently define CSP or cross-origin isolation, so plan either policy separately rather than assuming it migrates. |

Migration order:

1. Specify the existing GBA state, save, hash, cloud-header, R2-key and browser-storage compatibility contract; retain legacy read paths and export/rollback paths before changing identities.
2. Define a system identifier and per-system media/state/video/import metadata at the SDK–player and client–server boundaries. Keep the GBA values as the first implementation, not shared constants.
3. Move the player and SDK build, wire the GBA plugin and catalogs, and migrate IndexedDB, locks, offline caches and preferences without losing current GBA sessions. Test old data and old tabs as well as fresh installs.
4. Add system-aware catalog validation, D1 keys, cloud routes/metadata and R2 namespaces; dual-read legacy GBA saves and shares, and test sync, conflict resolution, listing, deletion and account erasure across both key forms.
5. Adapt the Cloudflare build, hashed-asset checks, Worker routes, headers and patch CDN origin policy. Exercise local/dev builds and the old GBA link and asset paths before any separately authorized infrastructure or deployment change.
