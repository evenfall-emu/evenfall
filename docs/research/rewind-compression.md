# Rewind compression and long-history retention

[`crates/state/src/rewind.rs`](../../crates/state/src/rewind.rs) implements the
storage policy: anchored word-run deltas, raw DEFLATE and budgeted, age-biased
retention.
[`rewind_inflate.rs`](../../crates/state/src/rewind_inflate.rs) bounds decoding
of untrusted bundles. [`design/rewind.md`](../design/rewind.md) describes the
Rust session contracts, implemented in
[`systems/gba/src/session.rs`](../../systems/gba/src/session.rs). The portable
state is encoded by
[`#[derive(Codec)]`](../../crates/state-derive/src/lib.rs), which preserves
ZiGBA's declaration-order schema rather than relying on Rust memory layout.

## Question and evidence

Can history retain hours of play in a 15 MiB budget without deflating every
full state on capture? ZiGBA compares states from two public homebrew games
and a user-supplied commercial game under synthetic input. Its states and
trajectories are not redistributed; its measurements are not Evenfall
performance results.

On ZiGBA's commercial-game native workload, word-run deltas with level-1
DEFLATE average 15,068 bytes and 0.57 ms to encode, against 59,408 bytes and
2.2 ms for independently deflated full states. The delta avoids scanning
unchanged state bytes; the compression advantage motivates retaining this
format. Equal-offset XOR still compresses the entire image, while block
bitmaps and content-addressed pages offer less favorable tradeoffs in the
original comparisons. Motion compensation and shift-tolerant matching remain
unimplemented alternatives, not claims about Rust's throughput.

## Storage design

- Each entry is a copy/literal difference against a shared anchor (ZiGBA
  stores same-offset word runs), stored with level-3 raw DEFLATE. An anchor is a full
  state deflated at level 6. A new anchor starts when the delta exceeds half
  its anchor's stored size. A lone dependent can absorb its old anchor.
  Entries do not chain: a restore inflates at most one anchor and applies one
  patch. `copy_diff`, `copy_patch`, `encode_copy_delta` and
  `decode_copy_delta` own the codec; ZiGBA's `diff`/`patch` word runs remain
  for imported bundles.
- Eviction minimizes `gap / ((age + 1)^p × bytes freed)` under a 15 MiB
  default byte budget and 4,096-entry cap. `RewindConfig` defaults to a
  60-frame capture interval and exponent `p = 0.5`, trading some recent
  resolution for older history. Requested speed scales the interval.
- Raw, independently deflated and delta encodings are selectable. Rust uses
  `miniz_oxide` to encode raw DEFLATE and its bounded decoder to inflate; it
  does not use ZiGBA's Zig standard-library compressor. The byte format and
  safety checks are ported, but compressed bytes can differ until DEFLATE
  parity is established. This is not a save-state payload/schema change.

## Codec choice

### Method

Two 32 MiB commercial-base ROM hacks, Game A and Game B as in ZiGBA's
[snapshot study](../../refs/zigba/docs/research/snapshot-deltas.md), start
from five local ZiGBA states: an overworld and a battle in Game A, two
overworld scenes in Game B. ROMs and states stay local. The states are ZiGBA
format version 5. A lab outside the repository converts them to version 6 by
inserting `bus.prescaler_origin` after the timers, chosen so every running
timer's phase lies on the shared prescaler grid. A candidate splice is
accepted only when Evenfall's transactional loader accepts it, its field
tracer places `prescaler_origin` at the splice, and the state re-encodes to
the version 5 bytes plus the inserted field. Loading alone is not proof:
other offsets also load. No state holds a MultiBoot continuation, the other
version 6 change. The timer model differs from version 5, so the converted
sessions do not replay ZiGBA's version 5 behavior; they are HLE Evenfall
sessions from those scenes.

Input is ZiGBA's seeded walker (one direction or none, plus A or B,
re-rolled every 12 frames), or no input ("idle"). A native release build
captures a portable state every 60 frames.

### Variable-length fields in play

In every session `cpu.hle_interrupt_wait_mask` changed between `None` and
`Some` in 33 to 47% of consecutive captures (195 to 281 of 599 in ten
minutes), shifting everything after the CPU by two bytes. The source
states of both games already differ in length by those two bytes. The framebuffer was
67 to 87% of the bytes changed between captures with the walker and 9 to 41%
when idle.

### Storage per minute of unevicted history

Ten minutes per session under the buffer's anchor rule (a new level-6
anchor when an entry would exceed half the anchor). "Padded" encodes
`hle_interrupt_wait_mask` at its `Some` size; it is simulated, not a format.
Kilobytes stored per emulated minute, anchors included:

| Session           | Word runs, deflate 1 | Copy delta, deflate 3 | Word runs, padded | Copy delta, padded | zstd 3, anchor dictionary |
| ----------------- | -------------------: | --------------------: | ----------------: | -----------------: | ------------------------: |
| A overworld       |                1,656 |                   938 |             1,169 |                926 |                       832 |
| A overworld, idle |                1,304 |                   299 |               317 |                298 |                       314 |
| A battle          |                1,792 |                 1,221 |             1,094 |              1,214 |                     1,114 |
| A battle, idle    |                1,452 |                   299 |               336 |                297 |                       318 |
| B overworld 1     |                1,515 |                 1,044 |               943 |              1,032 |                       949 |
| B overworld 2     |                1,548 |                 1,045 |               962 |              1,033 |                       950 |

Word runs opened 201 to 323 anchors in ten minutes, nearly one per length
change; the copy delta opened one or two.

### Retention in the default 15 MiB buffer

The real `RewindBuffer` at its defaults ran 120 minutes of Game A and 80 of
Game B (Game B's walker reaches a guest fault at 82 minutes under either
codec, a memory corruption caused by the game; see
[compatibility](../compatibility.md#guest-faults-in-user-supplied-games)). A retained
entry was decoded and compared by hash every 120 captures; every sampled
restore matched. Three sessions ran concurrently, so times are indicative:

| Session               | Word runs: entries, gap 1 h back, mean capture | Copy delta: entries, gap 1 h back, mean capture |
| --------------------- | ---------------------------------------------: | ----------------------------------------------: |
| A overworld, 120 min  |                              581, 39 s, 4.6 ms |                               998, 17 s, 2.6 ms |
| A battle, 120 min     |                              566, 69 s, 5.0 ms |                               932, 12 s, 3.0 ms |
| B overworld 1, 80 min |                              729, 42 s, 3.2 ms |                             1,002, 13 s, 2.2 ms |

With padded states the word-run buffer kept 1,097, 682 and 1,023 entries
and the copy-delta buffer 1,018, 939 and 1,016: padding adds up to 2% to
the copy delta, and padded word runs still lose the battle session by 27%.
The copy delta obtains the benefit of a fixed layout without changing the
portable format. Opening an anchor at a third of the anchor's size instead
of half kept 1,535, 652 and 943 entries and raised the slowest capture to
49 ms, so the half rule stays.

### Compressor

zstd 3 with the anchor as a prepared dictionary stores 9 to 11% less than
the copy delta with walker input (5 to 6% more when idle) and encodes
faster. Its C library builds
for `wasm32-unknown-unknown` with clang and imports nothing, but adds about
520 KB (about 120 KB gzipped) to the module, against about 14 KB for the
copy delta, and needs a C toolchain in the build.

## Boundaries and open questions

- The workloads cover two ROM hacks, ZiGBA's three games and synthetic
  inputs, not broad representative play or menus. Browser memory peaks, capture cost in
  the player and pause latency with a full history are not measured for Rust.
- ZiGBA's retention runs use a linear age bias; the default square-root bias
  has not been measured in long play. Scaling captures to requested speed
  changes their emulated spacing; sizes at high speed remain unmeasured.
- Folding a lone anchor during eviction is the slowest capture path and may
  consume a frame. Moving the fold off the capture path is untested.
- The Rust storage module does not itself decide when a session may capture
  or restore. A host must preserve canonical state and input chronology.
