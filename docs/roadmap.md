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
  runs continue a stalled machine as they idle a halted one. The web ABI
  reports the stall as the SDK's `halted` step reason.

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

**Open questions.** Whether a shorter in-access limit (a smaller one-time
overrun, at the cost of diverging from ZiGBA on the rare burst between the
new and old limit that then ends) is worth it; whether the SDK should report
a distinct `dma_stall` step reason; and whether stalls should become durable
in a future state version.

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
