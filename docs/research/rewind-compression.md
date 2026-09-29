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

- Each entry is a word-run difference against a shared anchor. Headers precede
  changed words; level-1 raw DEFLATE stores the delta. An anchor is a full
  state deflated at level 6. A new anchor starts when the delta exceeds half
  its anchor's stored size. A lone dependent can absorb its old anchor.
  Entries do not chain: a restore inflates at most one anchor and applies one
  patch. `diff`, `patch`, `encode_delta` and `decode_delta` own the codec.
- Eviction minimizes `gap / ((age + 1)^p × bytes freed)` under a 15 MiB
  default byte budget and 4,096-entry cap. `RewindConfig` defaults to a
  60-frame capture interval and exponent `p = 0.5`, trading some recent
  resolution for older history. Requested speed scales the interval.
- Raw, independently deflated and delta encodings are selectable. Rust uses
  `miniz_oxide` to encode raw DEFLATE and its bounded decoder to inflate; it
  does not use ZiGBA's Zig standard-library compressor. The byte format and
  safety checks are ported, but compressed bytes can differ until DEFLATE
  parity is established. This is not a save-state payload/schema change.

## Boundaries and open questions

- The original workload covers three games and synthetic inputs, not menus,
  battles or broad representative play. Browser memory peaks, capture cost in
  the player and pause latency with a full history are not measured for Rust.
- ZiGBA's retention runs use a linear age bias; the default square-root bias
  has not been measured in long play. Scaling captures to requested speed
  changes their emulated spacing; sizes at high speed remain unmeasured.
- Folding a lone anchor during eviction is the slowest capture path and may
  consume a frame. Moving the fold off the capture path is untested.
- The Rust storage module does not itself decide when a session may capture
  or restore. A host must preserve canonical state and input chronology.
