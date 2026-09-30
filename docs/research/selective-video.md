# Selective video

At high playback speeds the host does not present every emulated frame.
[`Bus`](../../systems/gba/src/bus/mod.rs) can skip pixel production on
unselected frames without skipping LCD events, affine references, raster
histories, mosaic, DMA, interrupts or audio. [`Session`](../../systems/gba/src/session.rs)
owns the requested interval and [`VideoRecovery`](../../systems/gba/src/video_recovery.rs)
reconstructs exact pixels for saves and reads between published frames.
[`web/gba/worker.js`](../../web/gba/worker.js) chooses the interval for the
browser. The scanline memo receives change marks across skipped frames
([`scanline-memo.md`](scanline-memo.md)).

## Selection and publication

`Session::set_video_interval` accepts 1–16. Above 2× playback the browser
requests the ceiling of speed, and 1 otherwise. `Bus::set_video_interval`
records the request; frame wrap latches it into `video_render_current` and
`video_skip_remaining`. Repeating a request does not restart the countdown.
On a skipped frame only `render_scanline` is omitted; emulated state continues
on its original cycle timeline. Above 2× the Worker also disables **host**
audio output while leaving emulated sound state active.

`video_published_pixels` stores the last completed selected image, copied
after visible line 159. `video_frame_id` and `video_frame_cycle` identify it;
`published_framebuffer` never forces reconstruction. The ordinary framebuffer
can be partially rendered or marked non-canonical without tearing the
published image. The browser posts only a newly published frame; display
scheduling belongs to the frontend.

## Exact reconstruction

While the interval exceeds one, `Session` journals advancing calls, key
changes and steps, with their ending cycles and timed inputs. As in Evenfall,
the anchor is a raw copy of the live machine: `rebase` copies it into
`VideoRecovery`'s one preallocated replay `Machine` with `Machine::copy_from`,
which reuses the replay machine's boxes, forgets decoded code and invalidates
the scanline memo. `materialize` switches the replay machine to full video
with host audio disabled and replays the journal on it; it then equals the
live machine and anchors an empty journal, so reconstruction needs no second
copy. A reconstruction that fails part-way leaves no anchor, and later ones
return `VideoRecoveryDiverged` until the next rebase. Both operations and
inputs are bounded to 4,096 entries.

Calls ending at a newly completed selected image rebase. `Session::run_slice_with_input`
splits runs at `Bus::next_rendered_frame_cycle`; before the journal fills or
the anchor exceeds 32 frames of emulated time, recovery materializes and
rebases rather than discarding history. Even if STOP freezes the CPU, the
`Slice` carries the caller's `stop_at` separately from the internal
`yield_at`, so a serial event can still wake it.

`materialize` checks each replayed operation's end cycle and compares the
replayed CPU, cycle and fault with the live machine before copying **only**
pixels back through `Bus::accept_canonical_framebuffer` (which invalidates
the scanline memo). Live RAM, timers, audio queues and cycle remain intact.
Mismatch returns `VideoRecoveryDiverged`.

Selection, publication and recovery fields are transient, excluded from the
portable state. `Machine::write_state` rejects a non-canonical framebuffer;
`Session::write_state` and `Session::canonical_framebuffer` reconstruct first.
Loading a state or resetting replaces the anchor; returning to interval 1
materializes current pixels before dropping recovery. A fault remains
terminal and cannot become a resumable save. Hosts must route every
mutation through `Session`: debugger writes directly to its machine and
linked-machine host mutations are not journaled. This cache does not replay
host PCM and is not a portable save format.

## Verification and open questions

[`session.rs`](../../systems/gba/src/session.rs) tests arbitrary-cycle exact
state/input restoration, bounded history, faults, STOP horizons, and
transactional load and reset. There is no test yet that compares audio,
DMA histories or browser publication with full video.

- Whether selective video alone reaches 16x playback depends on how much CPU
  work the game does. That limit is unmeasured.
- A second rendering thread is not part of this design. A pure PPU worker
  would require immutable scanline state and owned raster histories; replaying
  the CPU on another core would duplicate work.
- Linked-machine host mutations bypass recovery. An explicit save or frame
  read during selective playback pays for replay since the anchor.
