# Rewind

Rewind retains complete [`ZIGBAST1` save states](save-states.md) so restoring a
checkpoint uses the ordinary transactional machine loader. `Session` owns
capture timing, framebuffer reconstruction and post-restore host policy in
[`systems/gba/src/session.rs`](../../systems/gba/src/session.rs);
[`RewindBuffer`](../../crates/state/src/rewind.rs) owns storage, compression,
retention, decoding and the import/export bundle. Neither rewind policy nor
history changes guest execution or enters a portable save state.

## Capture and retention

After a bounded `advance`, `run_slice_with_input` or `step`, `Session` checks
the completed frame count. A due capture obtains a canonical state (including
the exact framebuffer when selective video has skipped work) and pushes it
with its frame and cycle. Failures do not fail emulation: recording a skipped
attempt prevents repeated attempts at each subsequent frame. The default
interval is 60 frames. Requested speed is a Q8 scale from 0.5x to 16x, so the
capture interval is `ceil(interval * speed)`; changing speed does not change
the guest or discard history.

| Setting               | Value                      |
| --------------------- | -------------------------- |
| Default budget        | 15 MiB                     |
| Maximum budget        | 64 MiB; 0 disables capture |
| Entry cap             | 4,096                      |
| Largest decoded state | 2 MiB                      |

The budget charges stored payloads, anchors and 64 bytes per entry, not the
current decoded anchor, metadata or allocator overhead. When over budget or
entry cap, the buffer keeps the newest and normally the oldest and removes
the interior entry with the smallest age- and byte-weighted gap score.
Two entries retain the newest: a sole newest entry remains even if it exceeds a
very small budget.

Ties prefer removing the older candidate.

The most impactful behavior knob you can tune is the age exponent: moving it
close to 1 biases the engine to keep recent snapshots, and make larger gaps in
longer sessions. Moving it below 0.5 rapidly move to an uniform distribution where
age doesn't matter. i.e. 1 is linear, 0 is unbiased.
The default age exponent is 0.5, this keeps a decent amount of recent states for
the first minutes, but rapidely makes room for longer session to have more reasonable
gaps. (e.g. 1min gap at 24h session for 15MB budget is achievable easily on small
games.)

## Anchored copy-delta storage

| Mode              | Entry encoding                                          |
| ----------------- | ------------------------------------------------------- |
| `delta` (default) | copy/literal difference from an anchor, deflate level 3 |
| `fast`            | independently deflated state, level 1                   |
| `off`             | raw portable state                                      |

An anchor is a complete state compressed at level 6 and shared by its delta
entries. The current anchor is also kept decoded, with an `AnchorIndex`: a
128 Ki-slot table (512 KiB) of anchor positions keyed by the 8 bytes at every
fourth offset. A copy delta (entry encoding 3) describes the target as
literal runs and copies of at least 12 bytes from anywhere in the anchor. Its
varint ops (literal length, copy length, zigzag change of the copy's
displacement from the previous copy) precede the literals, and bytes after
the last copy are trailing literals. At each position the encoder tries the
previous displacement, the one before it, that of the last copy of 64 bytes
or more, the same offset and one index candidate, keeping the longest. A
field whose encoded length changes (an `Option` becoming `Some`) therefore
costs one displacement change instead of rewriting everything after it, and
data that moved within the state is still copied.

ZiGBA's word-run delta (entry encoding 2) compares same-offset 32-bit words
against an anchor zero-extended or truncated to the target size. It stores
varint skip/copy run headers together, then changed words, then trailing
partial-word bytes. Evenfall decodes it from imported bundles but no longer
writes it. Both deltas follow `u32` target length and `u32` raw delta length
with raw DEFLATE; complete compressed states use `u64` raw length followed by
raw DEFLATE.

When a delta would exceed half the compressed anchor's size, the capture
starts a new anchor. Eviction or branching can fold a lone dependent of an
old anchor into a self-contained state. Deltas never chain through entries;
restoration requires at most one anchor inflate and one patch. Decoding bounds
the state at 2 MiB and raw delta at 4 MiB and checks every run, copy source
and literal range.
[`rewind_inflate`](../../crates/state/src/rewind_inflate.rs) decodes stored,
fixed and dynamic raw-DEFLATE blocks with exact input/output consumption.

Evenfall uses **miniz_oxide** to write raw DEFLATE. Evenfall reads ZiGBA's
bundles and re-exports imported entries without recompression; ZiGBA reads
Evenfall's bundles only when they hold no copy delta. A general-purpose
compressor with an anchor dictionary (zstd) is not used: its C library would
enlarge the WebAssembly module and require a C toolchain in its build
([measurements](../research/rewind-compression.md#codec-choice)).

## Restoring

`Session::rewind(steps)` chooses a retained checkpoint strictly older than
the current live frame, so it always moves back even immediately after a
capture. It decodes the checkpoint, loads it with `Machine::load_state`,
restores host audio/video preferences, rebases selective-video recovery,
resets the resampler and discards newer checkpoints. A valid checkpoint can
also recover from a terminal guest fault. Audio queued by the host from the
discarded timeline must be dropped.

`rewind_info` reads metadata and `rewind_framebuffer` previews a checkpoint in
a scratch machine without consuming history. An explicit `load_state`, reset
with battery save or successful rewind-configuration replacement starts a new
timeline and clears history. An invalid configuration leaves history intact;
a speed change does not clear it.

## Rewind bundles

A bundle carries the exact live state, retention policy and retained history.
`Session::write_rewind_bundle` does not advance time or take a checkpoint;
`Session::load_rewind_bundle` validates the upload before replacing both live
state and history. The owned-buffer variant adopts a successful upload and
keeps stored payloads as views into it; later captures recycle freed bundle
space. Unlike explicit state load, bundle load keeps its history.

Bundles are little-endian and versioned independently of the save state.
Version 2 adds entry encoding 3 (copy delta) to ZiGBA's version 1; the writer
emits version 1 when no entry is a copy delta, and version 1 input with a copy
delta is rejected.

| Section               | Contents                                                                                                                                                                                                                                                                          |
| --------------------- | --------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------- |
| Header                | magic `ZGBRWD\0\0` (8 bytes), version `u32` = 1 or 2                                                                                                                                                                                                                              |
| Live state            | `u32` length, then a complete portable state of at most 2 MiB                                                                                                                                                                                                                     |
| Policy                | budget `u32`, interval `u16`, compression `u8` (0 off, 1 fast, 2 delta), age exponent `f64` bits, Q8 speed `u16`, next sequence `u64`, last-capture flag `u8` and frame `u64`, current-anchor flag `u8` and id `u32`, next anchor id `u32`, anchor count `u32`, entry count `u32` |
| Anchors               | each anchor: id `u32`, stored length `u32`, stored bytes                                                                                                                                                                                                                          |
| Entries, oldest first | each entry: frame, cycle and sequence (`u64` each), encoding `u8` (0 raw, 1 deflate, 2 word-run delta, 3 copy delta), anchor id `u32`, stored length `u32`, stored bytes                                                                                                          |
| Digest                | BLAKE3 of all preceding bytes (32 bytes)                                                                                                                                                                                                                                          |

Stored compressed bytes are copied verbatim; decoded anchor caches and
reference counts are rebuilt. Neither media bytes nor host preferences are in
the bundle: the states inside bind the loaded ROM, BIOS and boot mode.

Import rejects bundles over 68 MiB, wrong magic or version, bad digest,
malformed policy or counts, duplicate or unreferenced anchors, out-of-order
entries, invalid lengths, corrupt compressed data, trailing bytes and any
checkpoint whose decoded state or frame/cycle labels do not validate. The
loader validates all states with the resident media before publishing the
live machine and history together. A matching outer digest is not proof of
authenticity or hardware validity; a failed import preserves the old live
state and history.

## Verification

ZiGBA's rewind verification areas map to the following Rust tests. Buffer
checks are in [`crates/state/tests/rewind.rs`](../../crates/state/tests/rewind.rs)
and private storage checks in [`crates/state/src/rewind.rs`](../../crates/state/src/rewind.rs).
Session checks live in [`systems/gba/src/session.rs`](../../systems/gba/src/session.rs).
The browser tests in `web/gba` exercise the SDK.

| ZiGBA verification area                        | Rust coverage                                                                                                                                                                                                                                                                                                                                                |
| ---------------------------------------------- | ------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------ |
| Word runs, malformed deltas, length changes    | `word_run_diff_round_trips_length_changes_and_rejects_malformed_deltas`                                                                                                                                                                                                                                                                                      |
| Copy deltas (Evenfall only)                    | `copy_diff_round_trips_shifted_moved_and_resized_targets_and_rejects_malformed_deltas`; bundle versions in `bundle_round_trip_and_corrupt_history_rejection`                                                                                                                                                                                                 |
| Age selection, ties and byte-weighted eviction | `age_exponent_selects_exact_historical_states_with_oldest_first_ties`, `square_root_age_weighting_prefers_freeing_expensive_entries`, `budget_eviction_keeps_newest_and_oldest_and_coarsens_with_age`                                                                                                                                                        |
| Entry cap, anchor release and adopted storage  | `entry_cap_bounds_history_that_never_reaches_the_byte_budget`, `truncate_newer_keeps_the_target_as_newest_and_releases_anchors`, `adopted_histories_survive_compaction_eviction_anchor_folding_and_branching`, `invalid_owned_bundle_returns_the_original_upload`, `delta_entries_are_small_share_one_anchor_and_decode_exactly` (buffer)                    |
| Capture cadence and reconfiguration            | `capture_cadence_scales_with_speed_and_resets`; `invalid_rewind_reconfiguration_preserves_history` (session)                                                                                                                                                                                                                                                 |
| Framing and bounded DEFLATE inflation          | `persisted_inflater_accepts_independent_stored_fixed_and_dynamic_vectors`, `deflate_framing_rejects_overlong_and_truncated_payloads`, `zig_frame_inflates_and_rust_frames_preserve_length_and_content`                                                                                                                                                       |
| Bundle round trip and corrupt input            | `bundle_round_trip_and_corrupt_history_rejection`, `zigba_bundle_imports_and_reexports_original_compressed_entries`; `rewind_bundles_reject_corrupt_envelopes_metadata_codecs_and_foreign_media_atomically` (session)                                                                                                                                        |
| Checkpoint restore, branching and previews     | `rewind_captures_per_frame_history_and_restores_earlier_frames`, `rewind_escapes_a_faulted_machine_with_the_last_good_state`, `explicit_loads_and_resets_start_a_new_rewind_timeline`, `rewind_framebuffer_observation_is_exact_and_does_not_consume_history` (session); `bundle_restores_empty_tiny_budget_and_evicted_histories_after_branching` (session) |
| Allocation failures and WebAssembly bundles    | ZiGBA's injected allocation-failure and browser end-to-end checks are not yet ported to the Rust session and WebAssembly core.                                                                                                                                                                                                                               |

The committed [ZiGBA-produced bundle fixture](../../crates/state/tests/fixtures/zigba-rewind-bundle.bin)
validates media-bound state envelopes and decoded checkpoint hashes before
checking that Rust re-exports the imported entries unchanged. ZiGBA's raw
DEFLATE decoder also accepts Rust's miniz_oxide frames. Byte-identical
compressed output is deliberately not a compatibility requirement.

## Known limits

- If the host cannot attain the requested playback speed, wall-clock capture
  spacing increases. Capture and anchor folding run in the emulation thread.
- Already-queued host audio is not continuous across a restore.
