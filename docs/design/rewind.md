# Rewind

Rewind lets a player step back through recent play. While the game runs, the
session keeps complete [`ZIGBAST1` save states](save-states.md) in an
in-memory history, and rewinding loads one of them with the ordinary state
loader. Rewind never changes how the guest runs, and none of its settings or
history end up in a portable save state.

Two pieces share the work:

- `Session`, in [`systems/gba/src/session.rs`](../../systems/gba/src/session.rs),
  decides when to capture a state, and after a restore puts the host side
  (audio, video recovery) back in order.
- [`RewindBuffer`](../../crates/state/src/rewind.rs) stores the states: it
  compresses them, decides which to keep, decodes them, and reads and writes
  rewind bundles.

## When states are captured

After each `advance`, `run_slice_with_input` or `step`, `Session` checks how
many frames have completed. When a capture is due, it writes a state
(including the exact framebuffer if selective video skipped drawing it) and
pushes it with its frame and cycle.

By default a capture happens every 60 frames, about once a second at normal
speed. Fast-forwarding stretches the interval by the same factor, so captures
stay about one per second of wall time: the speed is a Q8 factor from 0.5x to
16x, and the interval becomes `ceil(interval * speed)` frames. Changing speed
keeps the history.

A failed capture never stops emulation. The session counts it as a capture
and tries again one interval later, not on every following frame.

## What is kept

| Setting               | Value                      |
| --------------------- | -------------------------- |
| Default budget        | 15 MiB                     |
| Maximum budget        | 64 MiB; 0 disables capture |
| Entry cap             | 4,096                      |
| Largest decoded state | 2 MiB                      |

The budget counts stored payloads, anchors and 64 bytes per entry. The
decoded current anchor (up to 2 MiB), its 512 KiB index, metadata and
allocator overhead come on top of it.

When the history goes over its budget or entry cap, the buffer removes
states one at a time until it fits. It always keeps the newest state and
normally the oldest, and removes the state in between with the lowest score:

```text
score = gap / (bytes freed * age^p)
```

`gap` is the distance between the state's two neighbours and `age` how long
ago it was taken, both counted in captures; `p` is the age exponent. The
buffer therefore drops first the states that are old, expensive to keep or
close to their neighbours. On a tie the older state goes. With two states
left, the older one goes; a lone newest state stays even if it alone exceeds
a very small budget.

The age exponent is the setting that most changes how rewind feels. At 1,
age counts linearly: the last minutes stay densely covered, and gaps grow
fast further back. Going below 0.5, age quickly stops mattering, and at 0
states end up evenly spaced over the whole session. The default, 0.5, keeps
plenty of recent states for the first minutes, then spreads out so that long
sessions still have reasonable gaps: on small games, a 24-hour session in the
default 15 MiB keeps gaps of about a minute.

## How states are stored

Consecutive states of a game are mostly identical, so rewind stores most of
them as a difference from a recent complete state, called an anchor.

| Mode              | How each entry is stored                       |
| ----------------- | ---------------------------------------------- |
| `delta` (default) | a copy delta from an anchor, deflated, level 3 |
| `fast`            | the complete state, deflated, level 1          |
| `off`             | the raw state                                  |

An anchor is a complete state deflated at level 6 and shared by the deltas
that follow it. When a new delta would be larger than half the compressed
anchor, the capture starts a new anchor instead. Deltas never chain, so
restoring any entry takes at most one anchor inflate and one patch. When
removing entries leaves an old anchor with a single dependent, that entry is
recompressed on its own and the anchor is freed.

### Copy deltas

A copy delta (entry encoding 3) describes the new state as literal runs and
copies of at least 12 bytes taken from anywhere in the anchor. Copying from
anywhere matters because a state's layout shifts: when a field's encoded
length changes (an `Option` becoming `Some`), everything after it moves. A
same-offset comparison would store all of it again; a copy delta pays for one
change of displacement. Data that moved within the state is still copied.

To find copies, the buffer keeps the current anchor decoded, with an
`AnchorIndex`: a 128 Ki-slot table (512 KiB) of anchor positions keyed by the
8 bytes at every fourth offset. At each position the encoder tries the
previous copy's displacement, the one before it, that of the last copy of 64
bytes or more, the same offset, and one index candidate, and keeps the
longest match.

The delta starts with varint ops (literal length, copy length, and the
zigzag change of the copy's displacement from the previous copy), followed
by the literals. Bytes after the last copy are trailing literals.

### ZiGBA's word-run delta

ZiGBA writes a simpler delta (entry encoding 2). It compares 32-bit words at
the same offset against the anchor, zero-extended or truncated to the new
state's size, and stores varint skip/copy run headers, then the changed
words, then any trailing partial word. Evenfall reads it from imported
bundles but no longer writes it.

### Framing and decoding

Both deltas start with the target length and the raw delta length (`u32`
each), followed by raw DEFLATE. Complete compressed states start with the raw
length as a `u64`, followed by raw DEFLATE. Decoding limits a state to 2 MiB
and a raw delta to 4 MiB, and checks every run, copy source and literal
range. [`rewind_inflate`](../../crates/state/src/rewind_inflate.rs) decodes
stored, fixed and dynamic raw-DEFLATE blocks and checks that input and output
are consumed exactly.

### Compressor

Evenfall writes raw DEFLATE with **miniz_oxide**, so its compressed bytes
differ from ZiGBA's. The two still read each other's data: Evenfall reads
ZiGBA's bundles and re-exports imported entries without recompressing them,
and ZiGBA reads Evenfall's bundles as long as they contain no copy delta.

zstd with the anchor as a dictionary stores a little less during play, but its C
library would grow the WebAssembly module and need a C toolchain in the build
([measurements](../research/rewind-compression.md#codec-choice)).

## Restoring

`Session::rewind(steps)` goes back `steps` checkpoints, counting only those
strictly older than the current frame, so it always moves back, even right
after a capture. It decodes the checkpoint, loads it with
`Machine::load_state`, restores the host's audio and video settings, rebases
selective-video recovery, resets the resampler and drops every newer
checkpoint. Rewinding also gets a machine out of a terminal guest fault. The
host should drop any audio it has queued from the abandoned timeline.

To look at a checkpoint without going back, `rewind_info` reads its frame and
cycle, and `rewind_framebuffer` renders its picture in a scratch machine.

History is cleared when a new timeline starts: an explicit `load_state`, a
reset with battery save, or a new rewind configuration. A configuration that
fails validation leaves history alone, and so does a speed change.

## Rewind bundles

A bundle is a whole rewind session in one file: the exact live state, the
retention settings and every retained checkpoint. Loading it restores both
the live state and its history, which is what sets it apart from loading a
save state.

`Session::write_rewind_bundle` neither advances time nor takes a checkpoint.
`Session::load_rewind_bundle` validates the whole upload before replacing
anything. Its owned-buffer variant keeps the upload and uses it as storage:
stored payloads stay views into it, and later captures reuse the space freed
there.

Bundles are little-endian and versioned separately from save states. Version
1 is ZiGBA's; version 2 adds copy deltas. The writer emits version 1 when no
entry is a copy delta, so ZiGBA can read it. A version 1 bundle that
contains a copy delta is rejected.

| Section               | Contents                                                      |
| --------------------- | ------------------------------------------------------------- |
| Header                | magic `ZGBRWD\0\0` (8 bytes), version `u32` = 1 or 2          |
| Live state            | `u32` length, then a complete portable state of at most 2 MiB |
| Policy                | the retention settings and buffer counters, listed below      |
| Anchors               | each anchor: id `u32`, stored length `u32`, stored bytes      |
| Entries, oldest first | each entry: the fields listed below                           |
| Digest                | BLAKE3 of all preceding bytes (32 bytes)                      |

The policy section holds, in order: budget `u32`, interval `u16`,
compression `u8` (0 off, 1 fast, 2 delta), age exponent as `f64` bits, Q8
speed `u16`, next sequence `u64`, last-capture flag `u8` and frame `u64`,
current-anchor flag `u8` and id `u32`, next anchor id `u32`, anchor count
`u32` and entry count `u32`.

Each entry holds its frame, cycle and sequence (`u64` each), its encoding
`u8` (0 raw, 1 deflate, 2 word-run delta, 3 copy delta), its anchor id `u32`,
its stored length `u32` and the stored bytes.

Compressed bytes are copied verbatim; decoded anchors and reference counts
are rebuilt on load. The bundle holds neither the ROM and BIOS nor host
preferences: the states inside it are bound to the loaded ROM, BIOS and boot
mode.

Loading rejects a bundle that is over 68 MiB, has the wrong magic, version or
digest, malformed policy or counts, duplicate or unreferenced anchors,
out-of-order entries, bad lengths, corrupt compressed data or trailing bytes,
or any checkpoint whose state or frame and cycle don't validate against the
loaded media. A failed load keeps the previous live state and history. The
digest only detects damage: it says nothing about who made the bundle.

## Tests

The buffer's tests are in
[`crates/state/tests/rewind.rs`](../../crates/state/tests/rewind.rs), with
checks of private storage in
[`crates/state/src/rewind.rs`](../../crates/state/src/rewind.rs). The
session's tests are in
[`systems/gba/src/session.rs`](../../systems/gba/src/session.rs), and the
browser tests in `web/gba` exercise the SDK. A ported test names its ZiGBA
original in its doc comment. ZiGBA's injected allocation failures and its
browser end-to-end checks are not ported yet.

A [bundle written by ZiGBA](../../crates/state/tests/fixtures/zigba-rewind-bundle.bin)
checks that Evenfall imports it, decodes every checkpoint to the expected
state, and exports the imported entries unchanged. In the other direction,
ZiGBA's DEFLATE decoder accepts miniz_oxide's output.

## Known limits

- If the host can't keep up with the requested speed, captures drift further
  apart in wall time. Capture and anchor folding run on the emulation thread.
- Audio the host has already queued doesn't carry across a restore.
