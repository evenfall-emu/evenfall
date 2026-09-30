# Rewind

Rewind lets a player step back through recent play.
While the game runs, the session saves a complete [`ZIGBAST1` save state](save-states.md) every second or so into an in-memory history.
Rewinding picks one of those states and loads it with the same loader as any save state, so there is no separate restore path to keep correct.
Rewind never changes how the guest runs, and none of its settings or history end up in a portable save state.

## How it works

```text
 Session                          RewindBuffer
┌──────────────────────┐        ┌──────────────────────────────────┐
│ frame done           │ state  │ encode as a delta from the       │
│ capture due? ────────┼───────▶│ current anchor, or start a new   │
│                      │        │ anchor                           │
│                      │        │                                  │
│                      │        │ over budget? drop the state with │
│                      │        │ the lowest score                 │
│                      │        │                                  │
│ rewind(steps) ───────┼───────▶│ inflate anchor, apply delta      │
│ load_state, reset    │◀───────┼──────────────────── state bytes  │
│ host audio and video │        │ drop newer states                │
└──────────────────────┘        └──────────────────────────────────┘
```

The work is split between two types.
`Session` ([`systems/gba/src/session.rs`](../../systems/gba/src/session.rs)) decides when to capture and, after a restore, puts the host side back in order.
[`RewindBuffer`](../../crates/state/src/rewind.rs) owns the history.
It compresses states, decides which ones to keep, decodes them, and reads and writes rewind bundles.
The buffer is shared code in `crates/state` and doesn't depend on the GBA core.
It sees state bytes with a frame and cycle attached.

## Capturing

After each `advance`, `run_slice_with_input` or `step`, `Session` checks how many frames have completed.
When a capture is due, it writes a state and pushes it with its frame and cycle.
If selective video skipped drawing the current frame, the session rebuilds the exact framebuffer first, because a state can only be written with the real framebuffer in it.

By default a capture happens every 60 frames, which is about once a second at normal speed.
When the player fast-forwards, the interval grows with the speed, so captures stay about one per second of wall time and the history doesn't fill up faster.
The speed is a Q8 factor between 0.5x and 16x, and the interval becomes `ceil(interval * speed)` frames.
Changing speed keeps the history.

A failed capture never stops emulation.
The session treats it as a capture that happened and tries again one interval later, instead of retrying on every frame.

## Choosing what to keep

| Setting               | Value                          |
| --------------------- | ------------------------------ |
| Default budget        | 15 MiB                         |
| Maximum budget        | 64 MiB, and 0 disables capture |
| Entry cap             | 4,096                          |
| Largest decoded state | 2 MiB                          |

The budget counts the stored payloads, the anchors and 64 bytes per entry.
Real memory use is higher.
The decoded current anchor (up to 2 MiB), its 512 KiB index, metadata and allocator overhead all come on top.

When the history goes over its budget or its entry cap, the buffer removes states one at a time until it fits.
It always keeps the newest state and normally the oldest, and among the states in between it removes the one with the lowest score:

```text
score = gap / (bytes freed * age^p)
```

Here `gap` is the distance between the state's two neighbours, `age` is how long ago the state was taken (both counted in captures), and `p` is the age exponent.
In practice the buffer first drops states that are old, expensive to keep, or close to their neighbours.
On a tie the older state goes.
When only two states are left, the older one goes, and a lone newest state stays even if it alone exceeds a very small budget.

The age exponent is the setting that most changes how rewind feels.
At 1, age counts linearly.
The last few minutes stay densely covered and gaps grow fast further back.
Below 0.5, age quickly stops mattering, and at 0 the states end up evenly spaced over the whole session.
The default of 0.5 keeps plenty of recent states for the first minutes, then spreads them out so that long sessions still have reasonable gaps.
On small games, a 24-hour session in the default 15 MiB keeps gaps of about a minute.

## Storing states

Two states captured a second apart are almost identical, so storing each one in full would waste most of the budget.
Rewind instead stores most states as a difference from a recent complete state, called an anchor.

| Mode              | How each entry is stored                         |
| ----------------- | ------------------------------------------------ |
| `delta` (default) | a copy delta from an anchor, deflated at level 3 |
| `fast`            | the complete state, deflated at level 1          |
| `off`             | the raw state                                    |

An anchor is a complete state deflated at level 6 and shared by the deltas that follow it.
As the game moves on, the deltas grow.
Once a new delta would be larger than half the compressed anchor, the capture starts a new anchor instead.

Deltas are always taken against an anchor and never against the previous delta.
That keeps restores cheap: any entry needs at most one anchor inflate and one patch.
When removing entries leaves an old anchor with a single dependent, the buffer recompresses that entry on its own and frees the anchor.

### Copy deltas

A copy delta (entry encoding 3) describes the new state as a sequence of literal bytes and copies of at least 12 bytes from anywhere in the anchor.

Copying from anywhere, and not only from the same offset, matters because a state's layout shifts.
When a field's encoded length changes, for example an `Option` becoming `Some`, everything after it moves.
A same-offset comparison would see every following byte as changed and store it again.
A copy delta pays for one change of displacement and carries on copying.
Data that moved elsewhere in the state is still found.

To find copies, the buffer keeps the current anchor decoded, along with an `AnchorIndex`.
The index is a 128 Ki-slot table (512 KiB) that maps the 8 bytes at every fourth offset of the anchor to their position.
At each position of the new state, the encoder tries a handful of candidates and keeps the longest match.
The candidates are the displacement of the previous copy, the one before it, the displacement of the last copy of 64 bytes or more, the same offset, and one hit from the index.

The encoded delta starts with a list of varint ops (literal length, copy length, and the zigzag-encoded change of displacement since the previous copy).
The literal bytes follow the ops.
Bytes after the last copy are stored as trailing literals.

### Word-run deltas

Older bundles use a simpler delta (entry encoding 2).
It compares the new state with the anchor 32-bit word by 32-bit word at the same offsets, after zero-extending or truncating the anchor to the new state's size.
It stores varint skip and copy run headers, then the changed words, then any trailing partial word.
Evenfall still reads this format but no longer writes it.

### Framing and decoding

Both kinds of delta start with the target length and the raw delta length as two `u32`, followed by raw DEFLATE.
Complete compressed states start with the raw length as a `u64`, followed by raw DEFLATE.

Stored bytes can come from an imported file, so decoding treats them as untrusted.
It limits a state to 2 MiB and a raw delta to 4 MiB, and checks every run, copy source and literal range.
[`rewind_inflate`](../../crates/state/src/rewind_inflate.rs) decodes stored, fixed and dynamic raw-DEFLATE blocks and checks that input and output are consumed exactly.

### Compressor

Evenfall writes raw DEFLATE with **miniz_oxide**.
Imported entries are never recompressed, so an imported bundle exports with its entries unchanged.

zstd with the anchor as a dictionary stores about 10% less during play.
Its C library would add about 520 KB to the WebAssembly module (120 KB gzipped), against 14 KB for the copy delta, and would need a C toolchain in the build, which is why rewind doesn't use it.

## Restoring

`Session::rewind(steps)` goes back `steps` checkpoints.
It only counts checkpoints strictly older than the current frame, so a rewind right after a capture still moves back.
It then:

1. decodes the checkpoint and loads it with `Machine::load_state`,
2. restores the host's audio and video settings,
3. rebases selective-video recovery and resets the resampler,
4. drops every checkpoint newer than the one it loaded.

Because it goes through the ordinary state loader, rewinding also works on a machine that stopped on a terminal guest fault.
The host should drop any audio it queued from the abandoned timeline.

To look at a checkpoint without going back, `rewind_info` returns its frame and cycle, and `rewind_framebuffer` renders its picture in a scratch machine.
A host can use these to draw a timeline of previews.

History is cleared when a new timeline starts, which happens on an explicit `load_state`, a reset with battery save, or a new rewind configuration.
A configuration that fails validation leaves the history alone, and so does a speed change.

## Rewind bundles

A bundle saves a whole rewind session to one file.
It holds the exact live state, the retention settings and every retained checkpoint, so loading it brings back the history along with the game.
Loading a plain save state clears the history instead.

`Session::write_rewind_bundle` doesn't advance time or take a checkpoint.
`Session::load_rewind_bundle` validates the whole upload before replacing anything, and a failed load keeps the previous live state and history.
The owned-buffer variant keeps the uploaded bytes and uses them as storage.
Stored payloads stay views into the upload, and later captures reuse the space freed in it.

Bundles don't contain the ROM, the BIOS or host preferences.
The states inside are bound to the ROM, BIOS and boot mode they were made with, and loading checks every one of them against the loaded media.
Compressed bytes are copied verbatim, and the decoded anchors and reference counts are rebuilt on load.

Loading also rejects a bundle over 68 MiB, a bad magic, version or digest, malformed settings or counts, duplicate or unreferenced anchors, out-of-order entries, bad lengths, corrupt compressed data and trailing bytes.
The digest only detects damage.
It says nothing about who made the bundle.

### Format

Bundles are little-endian and versioned separately from save states.
Version 2 adds copy deltas to the original version 1.
The writer emits version 1 whenever no entry is a copy delta, so older readers can still load it.
A version 1 bundle that contains a copy delta is rejected.

| Section               | Contents                                                      |
| --------------------- | ------------------------------------------------------------- |
| Header                | magic `ZGBRWD\0\0` (8 bytes), version `u32` = 1 or 2          |
| Live state            | `u32` length, then a complete portable state of at most 2 MiB |
| Policy                | the retention settings and buffer counters, listed below      |
| Anchors               | per anchor, its id `u32`, stored length `u32` and bytes       |
| Entries, oldest first | per entry, the fields listed below                            |
| Digest                | BLAKE3 of all preceding bytes (32 bytes)                      |

The policy section holds, in order, the budget `u32`, interval `u16`, compression `u8` (0 off, 1 fast, 2 delta), age exponent as `f64` bits, Q8 speed `u16`, next sequence `u64`, last-capture flag `u8` and frame `u64`, current-anchor flag `u8` and id `u32`, next anchor id `u32`, anchor count `u32` and entry count `u32`.

Each entry holds its frame, cycle and sequence (`u64` each), its encoding `u8` (0 raw, 1 deflate, 2 word-run delta, 3 copy delta), its anchor id `u32`, its stored length `u32` and the stored bytes.

## Tests

The buffer's tests live in [`crates/state/tests/rewind.rs`](../../crates/state/tests/rewind.rs), with checks of private storage in [`crates/state/src/rewind.rs`](../../crates/state/src/rewind.rs).
The session's tests live in [`systems/gba/src/session.rs`](../../systems/gba/src/session.rs), and the browser tests in `web/gba` exercise the SDK.
No test yet covers rewind under allocation failure, or a full rewind session in a browser.

A [bundle written by Evenfall's predecessor](../../crates/state/tests/fixtures/zigba-rewind-bundle.bin) checks that Evenfall imports it, decodes every checkpoint to the expected state, and exports the imported entries unchanged.

## Known limits

- If the host can't keep up with the requested speed, captures drift further apart in wall time.
  Capture and anchor folding run on the emulation thread.
- Audio the host has already queued doesn't carry across a restore.
