# Web runtime

In the browser, Evenfall runs the same Rust core as the native build, compiled to WebAssembly.
The core runs inside a Web Worker, so emulation never blocks the page, and the page talks to it with messages.
The [`@evenfall/gba`](../sdk.md) package wraps all of this in a JavaScript `Emulator` class, and also offers a synchronous `WasmCore` for Node and developer tools.

The split of work follows one rule: the core only emulates.
It has no clock, no files, no screen and no speakers.
The Worker decides when to run the core and how fast.
The page draws the frames, plays the sound and stores the files.

```text
page                                             one Worker per Emulator
┌─────────────────────────────────┐             ┌──────────────────────────────────┐
│ Emulator (zigba.js)             │  commands   │ worker.js: pacing, recording     │
│ canvas, AudioWorklet, storage   ├────────────▶│ WasmCore (core.js)               │
│                                 │◀────────────┤ evenfall-gba.wasm                │
│ owned pixels, audio, snapshots  │  replies,   │ zb_* → Session → Machine         │
│                                 │  events     │                                  │
└─────────────────────────────────┘             └──────────────────────────────────┘
```

## The WebAssembly module

[`web/gba/src/lib.rs`](../../web/gba/src/lib.rs) is the `evenfall-gba-web` crate.
`npm run build:wasm -w @evenfall/gba` runs the [build script](../../web/gba/scripts/build-wasm.sh), which compiles it for `wasm32-unknown-unknown` with the `wasm-release` profile, twice from the same source:

- `web/gba/evenfall-gba.wasm` uses WebAssembly SIMD (`+simd128`), so the compositor draws eight pixels per instruction.
- `web/gba/evenfall-gba-scalar.wasm` doesn't, for browsers without SIMD (Chrome before 91, Firefox before 89, Safari before 16.4).

Both give identical results.
The SDK loads the SIMD build when `WebAssembly.validate` accepts a tiny SIMD module, and the scalar one otherwise.
Both files are build outputs and stay out of Git.
The profile uses LTO, strips symbols and aborts on panic.

The module is deliberately plain.
It builds with stable Rust, runs on one thread, imports nothing, and needs no host allocator, startup function, `SharedArrayBuffer` or cross-origin isolation, so it works on any web server.
JavaScript owns clocks, I/O, scheduling and audio devices.

| Linear memory | Linker setting                                       |
| ------------- | ---------------------------------------------------- |
| Initial       | 16 MiB (`--initial-memory=16777216`)                 |
| Maximum       | 512 MiB (`--max-memory=536870912`)                   |
| Stack         | 8 MiB (`-zstack-size=8388608`), inside linear memory |
| Export        | `--export-memory`, no imported memory                |

The maximum is room to grow, not memory used up front.
WebAssembly memory can't shrink, so an instance keeps its high-water mark until it is dropped.

## How JavaScript talks to the module

The module exports 44 `zb_*` functions and its memory, at ABI version 1.
This interface is private: applications use the JavaScript API, and [`WasmCore`](../../web/gba/core.js) refuses a module whose `zb_version()` isn't 1.

- **Errors.** A negative result leaves an error name at `zb_error_ptr`, `zb_error_len`, which `WasmCore` turns into an exception.
- **Running.** A bounded run returns 0 at its deadline, 1 at the first instruction boundary past it, 2 on STOP and 3 on a guest fault.
- **Input.** `zb_prepare(size)` reserves the one upload buffer, JavaScript writes into `zb_input_ptr()`, then calls the command that uses it.
  Timed button presses are 16-byte records: a `u64` cycle, a `u32` key mask and a zero `u32`.
- **Output.** `zb_output_ptr`, `zb_output_len` show a result that is only valid until the next command.
  `WasmCore` copies it into JavaScript memory at once.
  Exports of the ROM or save bytes point at the stored bytes instead of copying them inside the module.

Each WebAssembly instance holds at most one [`Session`](../../systems/gba/src/session.rs).
Another `WasmCore` or Worker gets its own memory and machine.
[Save states](save-states.md) don't include how the host paces or presents the game.

## Memory

Rust's default WebAssembly allocator (dlmalloc) handles both small and large allocations well enough that the module needs nothing custom.
The [allocator tests](../../web/gba/allocator.test.mjs) show that a request just over 32 MiB grows memory by about its own size, that its freed space is reused by the next such request without growing again, and that live data survives growth.

Large buffers are shared or reused instead of copied:

- An uploaded ROM becomes the session's `Arc<Vec<u8>>` without moving its bytes.
  The bus, and the scratch machines used to check a state or reset, share that buffer and the BIOS, along with their digests, instead of copying them.
- A ZIP upload is freed once the ROM is extracted.
- A successful rewind bundle import keeps the uploaded bytes as the history's storage, and reuses space as entries are dropped ([`rewind.md`](rewind.md)).
  A released bundle export can also hold the next import.
- Old output is freed before the next upload is allocated.

Every allocation whose size the caller controls can fail cleanly: uploads, BIOS and save copies, ZIP extraction, memory reads, state writes and bundle exports.
When memory runs out they return `OutOfMemory` instead of crashing, and the session stays usable.
The tests grow memory to its limit, then check that a valid request still works, that output failures are reported, and that memory is reused afterwards.
Small fixed-size allocations inside the core can still abort if the instance is exhausted, so the module isn't a general-purpose memory sandbox.

| Input or operation             | Limit                                                                |
| ------------------------------ | -------------------------------------------------------------------- |
| SDK load                       | 41 MiB for image, BIOS and save together, image at most 40 MiB       |
| ROM, BIOS                      | ROM (raw or extracted) at most 32 MiB, BIOS absent or exactly 16 KiB |
| One upload, rewind bundle      | 68 MiB, rewind history budget at most 64 MiB                         |
| Save state, rewind state       | payload at most 1 MiB, decoded rewind state at most 2 MiB            |
| One run, one memory read       | one emulated frame and 1,024 timed inputs, 1 MiB                     |
| One audio drain                | 8,192 stereo samples                                                 |
| Worker request queue           | 128 requests waiting                                                 |
| One manual `Emulator.runUntil` | 600 frames, split into bounded runs internally                       |

## The Worker

`Emulator.create()` starts one [`worker.js`](../../web/gba/worker.js) and one WebAssembly instance.
Each call is a message `{id, method, args}`, answered through a promise.
The Worker also sends `frame`, `clear`, `stopped`, `error`, `capture`, `progress` and `artifact` events.

Most commands finish within one turn of the Worker's event loop.
Long ones (a manual run, a scenario, a replay) give the event loop back regularly.
While one of them is in progress, other commands get `SessionBusy`, except key changes during play, because losing a key release when the page loses focus would leave a button stuck.
A manual `runUntil` runs in quarter-frame slices and yields every four slices.
During play, the Worker advances one frame per task, so commands can slip in between frames.
Anything that changes the timeline (a manual run, loading a state, rewinding, importing a bundle) requires play to be paused.

`captureMemory()` copies EWRAM and IWRAM in one turn, labelled with a cycle, a timeline identity and a sequence number, so a reader such as a [game bridge](../game-bridges.md) always sees one consistent moment.
A failed attempt to replace the timeline keeps the old identity.

[`Session::run_slice_with_input`](../../systems/gba/src/session.rs) runs at most one emulated frame per call.
It keeps its stopping point separate from its input deadline, so an input that an instruction steps over isn't lost between slices, and inputs past the caller's deadline stay queued.
A call may end at the first instruction boundary after its target, however the host slices time.

## Pacing and presentation

The Worker times frames against `performance.now()`.
A frame lasts 280,896 cycles at 16,777,216 Hz, divided by the requested speed (0.5x to 16x).
The Worker wakes itself through a `MessageChannel` instead of nested timers, which browsers throttle.

It never races to catch up.
If it falls more than 200 ms behind, it simply resets its target time, so a stall costs a moment of slowdown, not a burst of fast-forward.
At most two `frame` events may wait for the page to acknowledge them, so a busy page can't pile up frames.
Pausing and changing speed cancel pending wake-ups.
Speed also scales how often rewind and the recorder take snapshots.
None of this changes emulated cycles or saved state.

At 2x or below, the Worker sends the 240x160 frame (15-bit BGR color) and its audio every emulated frame.
The [presentation pool](../../web/gba/presentation.js) reuses buffers the page gives back with `recyclePresentation`, keeping at most four pixel buffers and four audio buffers.
Other outputs belong to the caller.
A `clear` event with a new generation number tells the page to drop frames and audio still queued from before a pause, a speed change or a jump in the timeline.

Drawing at the screen's refresh rate, converting colors for the canvas and dropping frames that arrive too late are the page's job.
A shared presentation layer with scaling and shader presets for Evenfall's frontends is planned ([#1](https://github.com/evenfall-emu/evenfall/issues/1)).

## Audio

The sound chip runs on the emulated clock, and `Session` resamples its output to the host's rate, 48 kHz by default.
At 2x or below, the Worker runs four quarter-frame slices per frame and drains audio after each.

Pages can use the bundled [`zigba-audio` AudioWorklet](../../web/gba/audio-worklet.js).
It keeps a bounded stereo buffer, fills it before starting, adjusts for the playback speed, and reports underruns and overruns.
The audio clock never drives emulation.

Above 2x, the Worker stops producing host audio.
Only the output is dropped: the sound chip, timers and sound DMA keep running exactly, and save states are unaffected.
Changing this setting or loading a state resets the resampler, and a `clear` event lets the page drop sound queued from before.
Host audio isn't continuous across a restore.

## Fast-forward

Above 2x, the Worker asks for one picture every `ceil(speed)` frames.
The core skips only drawing the other frames.
Display events, raster effects, DMA, interrupts and sound all stay cycle-exact.

The last drawn picture has a frame number and cycle.
When the page asks for the current frame, or a state or rewind checkpoint needs it, [`VideoRecovery`](../../systems/gba/src/video_recovery.rs) rebuilds the exact picture in a scratch machine without touching the live one.
Its data is bounded, temporary and never part of a save state ([selective video](../research/selective-video.md)).

## Recording

Recording is on by default, so a crash or bug report can be replayed.
The Worker wraps the session in a [`GameplayRecording`](../../web/gba/devtools/journal.js) and a [`FailureRecorder`](../../web/gba/devtools/capture.js):

- The recording keeps the first stretch of timed inputs, up to a bound, and can export them as a deterministic scenario.
- The failure recorder keeps up to three snapshots within 16 MiB, plus up to 4,096 operations and an estimated 4 MiB log.
  It snapshots every 60 frames, scaled with playback speed.
  A fault or exception produces a failure bundle with the build and media identity, ready for replay.

When recording is turned off or an operation can't be recorded, earlier replay starting points are marked unusable, instead of claiming an incomplete replay reproduces the run.
Scenario runs and failure replays give the event loop back as they go.
[`devtools.md`](../devtools.md) describes the formats and replay commands.

## Persistence

Neither the SDK nor the Worker stores anything.
ROMs, saves, states and rewind history are the page's to keep, in whatever storage it chooses, with its own autosave, single-writer and recovery rules.
A resume state must be paired with its exact ROM, BIOS, boot mode and clock settings.
The real-time clock starts at time zero by default so runs are reproducible.
A page that wants the real date passes it in and saves it itself.

## How it is checked

- [`core.test.mjs`](../../web/gba/core.test.mjs) runs the WebAssembly build and the native [`session_probe`](../../systems/gba/src/bin/session_probe.rs) on the same ROM with the same quarter-frame schedule.
  It compares cycle, frame count, program counter, CPSR, and SHA-256 hashes of the whole save state, EWRAM, the frame, the audio stream and the battery save.
  Locally supplied ROMs can extend the schedule through `ZIGBA_TEST_ROMS`, and games and BIOS files are never committed.
  Both sides are the same Rust code on different targets, so this shows the runtime is portable, not that it is accurate.
  Accuracy is what the ROM validators check ([`testing.md`](../testing.md)).
- [`allocator.test.mjs`](../../web/gba/allocator.test.mjs) checks memory growth, reuse and clean failure at the limit.
- [`worker.test.mjs`](../../web/gba/worker.test.mjs) and [`presentation.test.mjs`](../../web/gba/presentation.test.mjs) check the Worker's commands and the presentation pool.

## Known limits

- Small fixed-size allocations inside the core can still abort an instance that has run out of memory.
- Host audio already queued isn't continuous across a restore.
- The module runs on one thread.
