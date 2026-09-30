# Selective video

At 8x speed, a screen shows at most one emulated frame in eight, yet drawing is one of the most expensive things the emulator does.
Selective video skips drawing the frames nobody will see.

Skipping frames usually makes emulators inexact: a skipped frame's picture is simply wrong, and a save state taken then captures that wrong picture.
Evenfall keeps it exact.
It skips only the drawing itself.
Display events, affine steps, raster histories, mosaic, DMA, interrupts and sound all keep running on their exact cycles.
And whenever someone needs a skipped frame's real picture, for a save state or a screenshot, it rebuilds it exactly.

The pieces:

- [`Bus`](../../systems/gba/src/bus/mod.rs) skips drawing on frames that aren't selected.
- [`Session`](../../systems/gba/src/session.rs) holds the requested interval.
- [`VideoRecovery`](../../systems/gba/src/video_recovery.rs) rebuilds exact pixels when they are needed between shown frames.
- [`web/gba/worker.js`](../../web/gba/worker.js) chooses the interval for the browser.

The scanline memo still receives change marks during skipped frames ([`scanline-memo.md`](scanline-memo.md)).
[`performance.md`](../performance.md#skipping-frames) shows the speedup.

## Choosing and showing frames

`Session::set_video_interval` accepts 1 to 16.
Above 2x speed the browser asks for `ceil(speed)`, and 1 otherwise.
`Bus::set_video_interval` records the request, and the next frame wrap applies it (`video_render_current`, `video_skip_remaining`).
Asking again for the same interval doesn't restart the countdown.

On a skipped frame, only `render_scanline` is left out, and everything else continues on its usual cycles.
Above 2x the Worker also turns off host audio output, while the sound chip keeps running.

`video_published_pixels` holds the last selected frame, copied after visible line 159, and `video_frame_id` and `video_frame_cycle` identify it.
`published_framebuffer` returns it and never triggers a rebuild.
The ordinary framebuffer can be half drawn, or marked as not canonical, without tearing the published frame.
The browser only sends a frame when a new one is published, and when to display it is the page's decision.

## Rebuilding exact pixels

While the interval is above one, `Session` keeps a journal of every call that advances the machine, every key change and every step, with their ending cycles and timed inputs.
The journal starts from an anchor: a raw copy of the machine at the last drawn frame.

- `rebase` copies the live machine into `VideoRecovery`'s one preallocated replay machine with `Machine::copy_from`, reusing its memory, forgetting decoded code and clearing the scanline memo.
- `materialize` turns full drawing on in the replay machine, with host audio off, and replays the journal on it.
  The replay machine then equals the live one, and becomes the new anchor with an empty journal, so rebuilding never needs a second copy.

The journal holds at most 4,096 operations and 4,096 inputs.
Calls that end at a newly selected frame rebase.
`Session::run_slice_with_input` splits runs at `Bus::next_rendered_frame_cycle`, and before the journal fills or the anchor is 32 frames old, recovery rebuilds and rebases instead of dropping history.
If STOP freezes the CPU, the `Slice` carries the caller's `stop_at` separately from the internal `yield_at`, so a serial transfer can still wake it.

A rebuild checks its own work.
`materialize` checks each replayed operation's end cycle, and compares the replayed CPU, cycle and fault with the live machine.
Only then does it copy back the pixels, and only the pixels, through `Bus::accept_canonical_framebuffer` (which clears the scanline memo).
The live machine's RAM, timers, audio queue and cycle are untouched.
A mismatch returns `VideoRecoveryDiverged`.
A rebuild that fails part-way leaves no anchor, and later rebuilds also return `VideoRecoveryDiverged` until the next rebase.

## Rules for hosts

- **States need real pixels.** `Machine::write_state` refuses a frame that isn't canonical, while `Session::write_state` and `Session::canonical_framebuffer` rebuild first.
- **Nothing here is saved.** Selection, publication and recovery are left out of save states.
  Loading a state or resetting replaces the anchor, and returning to interval 1 rebuilds the current pixels before dropping recovery.
- **Faults stay faults.** A faulted machine can't become a save state through a rebuild.
- **Changes go through `Session`.** Only calls made through the session are journaled.
  A debugger writing directly to the machine, or changes to linked machines, aren't replayed.
- **Host audio isn't rebuilt.** The journal doesn't replay host audio, and isn't a save format.

## How it is checked

[`session.rs`](../../systems/gba/src/session.rs) checks exact state and input restoration at arbitrary cycles with runs split inside frames, bounded history, faults, STOP horizons, and loads and resets that fail without side effects.
`copies_continue_exactly_like_their_source` in [`machine.rs`](../../systems/gba/src/machine.rs) checks that a raw copy continues exactly like its source.
There is no test yet that compares audio, DMA histories or browser publication with full drawing.

## Open questions

- Whether selective video alone reaches 16x depends on how much CPU work a game does, and that limit hasn't been measured.
- A second drawing thread isn't part of this design.
  A separate display worker would need immutable line state and its own raster histories, and replaying the CPU on another core would duplicate work.
- Changes to linked machines bypass recovery.
- An explicit save or frame read during fast-forward pays for replaying everything since the anchor.
