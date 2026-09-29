# Web runtime

The browser runs the same deterministic GBA core as native Rust, compiled to
WebAssembly and driven from a module Worker. The private
[`@evenfall/gba`](../sdk.md) package supplies the Worker protocol and a
synchronous `WasmCore` adapter for Node and developer tools. Rendering to a
canvas, playing sound and persisting files remain the host application's jobs.

```text
application                                      one Worker per Emulator
┌─────────────────────────────────┐             ┌──────────────────────────────────┐
│ Emulator (zigba.js)             │  commands   │ worker.js: pacing, recording    │
│ canvas, AudioWorklet, storage   ├────────────▶│ WasmCore (core.js)              │
│                                 │◀────────────┤ evenfall-gba.wasm                │
│ owned pixels, audio, snapshots  │  replies,   │ zb_* → Session → Machine        │
│                                 │  events     │                                  │
└─────────────────────────────────┘             └──────────────────────────────────┘
```

## Rust WebAssembly module

[`web/gba/src/lib.rs`](../../web/gba/src/lib.rs) is the `evenfall-gba-web`
`cdylib`. From the repository root,
`npm run build:wasm -w @evenfall/gba` invokes the
[`build script`](../../web/gba/scripts/build-wasm.sh), which compiles for
`wasm32-unknown-unknown` with the `wasm-release` profile and copies the
gitignored artifact to `web/gba/evenfall-gba.wasm`. The release profile uses
LTO, strips symbols and aborts on panic. Builds use pinned **stable Rust**;
the browser module is **single-threaded**, with no WASM imports, host
allocator, startup function, `SharedArrayBuffer` or cross-origin-isolation
requirement. JavaScript owns clocks, I/O, scheduling and audio devices.

| Linear memory | Linker setting                                       |
| ------------- | ---------------------------------------------------- |
| Initial       | 16 MiB (`--initial-memory=16777216`)                 |
| Maximum       | 512 MiB (`--max-memory=536870912`)                   |
| Stack         | 8 MiB (`-zstack-size=8388608`), inside linear memory |
| Export        | `--export-memory`; no imported memory                |

The maximum is growth headroom, not a preallocated footprint. WASM memory
cannot shrink, so an instance retains its high-water mark until disposal.

### Private ABI

The module exports 44 `zb_*` functions and one memory, with ABI version 1.
`zb_version()` returns 1; [`WasmCore`](../../web/gba/core.js) rejects other
versions. Applications use the JavaScript API rather than the private ABI:

- Negative results leave an error name at `zb_error_ptr/zb_error_len`, which
  `WasmCore` turns into an exception. A bounded run returns 0 at its deadline,
  1 at the first instruction boundary past it, 2 on STOP and 3 on a guest
  fault.
- `zb_prepare(size)` reserves the one upload buffer. JavaScript writes to
  `zb_input_ptr()` before invoking the matching command. Timed button inputs
  are 16-byte records: `u64` cycle, `u32` key mask and zero `u32` reserved.
- `zb_output_ptr/zb_output_len` expose a transient result view, valid only
  until the next command. `WasmCore` immediately copies it into owned host
  storage; exports of resident ROM or save bytes borrow rather than copy
  inside the module.

Each WASM instance contains at most one [`Session`](../../systems/gba/src/session.rs).
Creating another `WasmCore` or Worker creates independent memory and machine
state. Portable [`ZIGBAST1` states](save-states.md) remain byte-compatible with
ZiGBA and do not serialize host pacing or presentation policy.

### Allocation and ownership

Rust's default wasm32 allocator (dlmalloc) serves both small and large
allocations. An isolated large-buffer probe and the shipped-module
[`allocator tests`](../../web/gba/allocator.test.mjs) show near-request-size
growth for a request just over 32 MiB, repeated reuse of its freed address
without memory growth, and live bytes surviving memory growth. A released
bundle export can also supply the next bundle upload. ZiGBA's custom 64 KiB
page-run allocator solves power-of-two waste in its Zig allocator; its bitmap
and in-place-resize rules are not Rust ABI requirements, so Evenfall does not
port it.

A raw ROM upload becomes the session's `Arc<Vec<u8>>` storage without moving
its bytes. The Bus and scratch/reset machines clone that immutable handle,
with a shared `Arc<[u8]>` BIOS and cached media digests, instead of copying a
ROM for validation. A ZIP upload is released after extraction. A successful
rewind-bundle import adopts its upload as stored-history backing and reuses
freed space as entries are evicted ([`rewind.md`](rewind.md)); a failed import
leaves the running session untouched. Old output is released before the next
upload allocation.

Caller-sized uploads, BIOS/save copies, ZIP ROM extraction, memory reads,
portable-state writes and bundle exports reserve fallibly. On allocation
failure they return `OutOfMemory` rather than trapping and leave the session
usable. The [WASM tests](../../web/gba/allocator.test.mjs) exercise a valid
request after growing linear memory to its limit, output failures and reuse
after failure. Fixed-size internal allocations may still abort if the host
exhausts the instance; the ABI is not a general-purpose memory sandbox.

| Input or operation             | Limit                                                              |
| ------------------------------ | ------------------------------------------------------------------ |
| SDK load upload                | 41 MiB combined image, BIOS and save; image at most 40 MiB         |
| ROM / BIOS                     | Raw or extracted ROM at most 32 MiB; BIOS absent or exactly 16 KiB |
| ABI upload / rewind bundle     | 68 MiB; rewind history budget at most 64 MiB                       |
| Portable state / rewind state  | Payload at most 1 MiB; decoded rewind state at most 2 MiB          |
| One ABI run / memory read      | One emulated frame and 1,024 timed inputs / 1 MiB                  |
| One audio drain                | 8,192 stereo samples                                               |
| One Worker request queue       | 128 outstanding requests                                           |
| One manual `Emulator.runUntil` | 600 frames, internally sliced into bounded ABI runs                |

## Worker commands and turns

`Emulator.create()` starts one [`worker.js`](../../web/gba/worker.js) and one
WASM instance. Calls are messages `{id, method, args}` with promise replies;
the Worker also sends `frame`, `clear`, `stopped`, `error`, `capture`,
`progress` and `artifact` events. Ordinary commands complete in one event-loop
turn. While a long manual run, scenario or replay is yielding, other commands
receive `SessionBusy`, except key changes during recorded play: a focus-loss
key release must not be lost. Manual `runUntil` uses quarter-frame slices and
yields every four slices; realtime play advances one frame per task so
commands can interleave. Timeline-changing manual execution, state load,
rewind and bundle import require paused play. `captureMemory()` copies EWRAM
and IWRAM in one turn with a cycle, timeline identity and sequence number;
failed timeline replacements preserve the old identity.

[`Session::run_slice_with_input`](../../systems/gba/src/session.rs) caps each
call at one emulated frame. Its separate yield point and input deadline keep
inputs that an atomic instruction crosses eligible across internal slices;
inputs beyond the caller's deadline remain unconsumed. A call may stop at the
first instruction boundary after its target, independent of host slicing.

## Pacing and presentation

The Worker schedules realtime frames against `performance.now()`, using
280,896 cycles per frame at 16,777,216 Hz divided by the requested speed
(0.5×–16×). A timer posts to a `MessageChannel` so frame work runs in a
message task, avoiding nested-timer throttling. It never bursts to catch up:
if more than 200 ms behind, it rebases the wall-clock target. At most two
`frame` events may await acknowledgement; a stalled host cannot accumulate
unbounded presentation messages. Pause and speed changes invalidate queued
wake-ups through a scheduling epoch. Speed also scales rewind capture cadence
and recorder snapshot cadence. These host policies never change emulated
cycles or durable machine state.

At 2× or below the Worker transfers a BGR555 240×160 image and its audio
each emulated frame. The [`presentation pool`](../../web/gba/presentation.js)
reuses only buffers explicitly returned via `recyclePresentation`, keeping at
most four pixel and four audio buffers; ordinary observations remain
caller-owned. A `clear` event with a new generation and timeline invalidates
stale queued frames and audio after pause, speed changes and timeline moves.
Drawing at display refresh cadence, converting BGR555 to canvas pixels and
discarding superseded images are host responsibilities, not SDK storage or
emulation policy.

## Audio and selective video

The GBA APU advances at guest-cycle timing; `Session` resamples its samples
to the chosen host rate (48 kHz by default). At 2× or below the Worker runs
four quarter-frame slices, draining audio after each. Hosts may load the
bundled [`zigba-audio` AudioWorklet](../../web/gba/audio-worklet.js), whose
bounded stereo ring primes before playback, interpolates at the selected
speed and reports underruns and overruns. The audio clock never drives guest
time. Above 2× the Worker suppresses **host** sample production and drains,
not APU, timer or FIFO/DMA activity; portable state is unaffected. Switching
output policy or restoring a state resets the resampler. The Worker sends a
`clear` event so the host can discard queued sound from the old presentation
generation; host PCM is not a sample-exact continuation across restoration.

Above 2× the Worker requests one published image every `ceil(speed)` frames.
Only pixel production on unselected frames is skipped: LCD events, raster
histories, DMA, interrupts and sound remain cycle-exact. The last complete
selected image has a frame ID and cycle. Explicit frame reads and state or
rewind captures still reconstruct exact current pixels via
[`VideoRecovery`](../../systems/gba/src/video_recovery.rs) and its scratch
machine, without advancing the live machine. The recovery anchor and journal
are bounded and transient, not part of a portable state; details and evidence
are in [selective video](../research/selective-video.md).

## Recording and persistence

Gameplay recording is on by default. The Worker wraps a session in a
[`GameplayRecording`](../../web/gba/devtools/journal.js) input journal and
[`FailureRecorder`](../../web/gba/devtools/capture.js). The journal retains a
bounded prefix of timed inputs and exports a deterministic scenario. The
recorder keeps up to three snapshots within 16 MiB, 4,096 operations and an
estimated 4 MiB log, normally snapshotting every 60 frames (scaled with
playback speed). A fault or exception emits a failure bundle with build and
media identity for replay. A gap from disabled recording or an unrecordable
operation invalidates earlier replay anchors rather than claiming an
incomplete replay is reproducible. Scenario runs and failure replays yield to
the event loop. See
[`devtools.md`](../devtools.md) for formats and replay commands.

Neither the SDK nor Worker persists ROMs, saves, states or rewind history.
Hosts choose their own durable store, autosave transaction, single-writer
policy and recovery path; they must pair a resume state with its exact ROM,
BIOS, boot and RTC options. The default RTC epoch is zero for reproducibility;
a host that wants a wall-clock epoch supplies and saves it explicitly.

## Native and WASM parity

[`core.test.mjs`](../../web/gba/core.test.mjs) drives the Rust WASM build and
native [`session_probe`](../../systems/gba/src/bin/session_probe.rs) with the
same ROM and quarter-frame schedule. It compares cycle, frame count, PC,
CPSR, and SHA-256 hashes of complete portable state, EWRAM, framebuffer,
audio stream and battery save. Locally supplied ROMs can extend the schedule
through `ZIGBA_TEST_ROMS`; games and BIOS files are never committed. The two
sides are the same Rust implementation on different targets: this proves
portability of the runtime, not hardware accuracy. ZiGBA parity and ROM
validators are separate checks ([`testing.md`](../testing.md)).
