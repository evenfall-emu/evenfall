# Save states

A portable save state captures durable GBA hardware state at an instruction
boundary. The format is `ZIGBAST1` version 6. Restoring a valid state with
the same media resumes exactly the same execution.
States are for compatible builds, not long-term interchange; battery saves
carry game progress across state-format changes.

[`Machine`](../../systems/gba/src/machine.rs) owns the GBA writer, loader,
schema description and semantic validation. The independent, `no_std` codec in
[`crates/state`](../../crates/state/src/lib.rs) provides `Codec`, counting and
hashing `Sink`, bounded `Source` and the
[`ZIGBAST1` envelope](../../crates/state/src/envelope.rs).
[`crates/state-derive`](../../crates/state-derive/src/lib.rs) generates the
encoders and schema writers used by the CPU, bus, APU and cartridge devices.
[`Session`](../../systems/gba/src/session.rs) applies host policy around state
loads and rewind ([`rewind.md`](rewind.md)).

## Properties

- Encoding has no pointers, padding or inactive union storage; integer widths
  and byte order are independent of host architecture.
- Timed CPU and device state, including pending work, survives a round trip.
- A load validates its envelope and hardware invariants into a scratch machine
  before replacing the running one. Invalid or incompatible bytes leave it
  untouched.
- Boot mode and exact ROM and BIOS contents bind the state to its media.
- The format accepts only its current version and layout, with no implicit
  migration for older states.

## File layout

All integers are little-endian.

|  Offset | Size | Field                                          |
| ------: | ---: | ---------------------------------------------- |
|       0 |    8 | magic `ZIGBAST1`                               |
|       8 |    4 | version `u32` (6)                              |
|      12 |   32 | BLAKE3 schema digest                           |
|      44 |    8 | payload length `u64`, at most 1 MiB            |
|      52 |    1 | boot mode: 0 HLE, 1 skip-BIOS, 2 external BIOS |
|      53 |    8 | ROM length `u64`                               |
|      61 |   32 | ROM BLAKE3 digest                              |
|      93 |    8 | BIOS length `u64` (0 under HLE)                |
|     101 |   32 | BIOS BLAKE3 digest                             |
|     133 |    n | payload                                        |
| 133 + n |   32 | payload BLAKE3 digest                          |

No bytes may follow the digest. The payload is the master cycle (`u64`), CPU,
then bus, with each durable field in the format's declaration order. The bus includes
memories, I/O, video histories, timers, DMA, serial, cartridge and audio state;
the save medium and APU encode their own fields in place. ROM and BIOS bytes
themselves are not stored.

## Canonical codec and layout parity

| Type             | Encoding                                                                |
| ---------------- | ----------------------------------------------------------------------- |
| integer          | little-endian, `ceil(bit width / 8)` bytes; narrow widths are validated |
| `usize`, `isize` | widened to 64 bits, checked on decode                                   |
| `bool`           | one byte, 0 or 1                                                        |
| float            | raw IEEE bit pattern                                                    |
| enum             | integer tag; unknown tags rejected                                      |
| optional         | presence `bool`, then value when present                                |
| array            | elements in order; byte arrays verbatim                                 |
| struct           | durable fields in declaration order without padding                     |
| tagged union     | tag followed by the active payload only                                 |

`#[derive(Codec)]` emits the same value encoding and static schema for the
GBA's Rust fields. `#[codec(bits = N)]` captures non-byte integer widths;
`#[codec(skip)]` excludes transient fields from both value and schema;
`#[codec(flatten)]` splices the split Rust CPU into the format's flat CPU layout.
Module schema prefixes, field renames and custom field encoders preserve the format's
field names and canonicalize unused array slots. Decode writes into existing
storage so skipped fields are not overwritten. `Box<[T; N]>` is wire-transparent.

The value-independent schema description includes codec and payload identities,
then the CPU and bus schemas, with every durable field name, type, integer
width, array extent and enum or union alternative. Its BLAKE3 digest is stored
in the header. Evenfall's complete Rust description must equal the version 6
[`schema-v6.bin`](../../tools/parity/fixtures/schema-v6.bin) fixture and its
[`digest`](../../tools/parity/fixtures/schema-v6.blake3), not merely hash to a
plausible value. `Machine::schema_description` and the
`schema_matches_zigba_version_six` test compare the bytes. A layout change
that does not bump the version is rejected by `SaveStateSchemaMismatch`.

Schema equality cannot detect changed _values_ under unchanged types. The
locked-byte `save_state_v6_byte_schema_is_locked_by_a_deterministic_fixture`
test pins a deterministic whole state against an independent fixture;
[`tools/parity`](../../tools/parity/README.md) compares complete save-state
blobs at checkpoints across host slicing schedules. Both are needed for byte
parity, in addition to schema equality.

With tracing enabled, `Sink::field` records nested field paths and half-open
byte ranges in `Sink.trace`; `Machine::state_field_ranges` covers cycle, CPU
and bus using one payload sink. Ranges start at the first payload byte; add the
133-byte header offset when mapping a whole-file difference. A differing byte
may have both a parent and child span; choose the narrowest span. This is
optional diagnostics, not part of the serialized format.

## Media, integrity and loading

The header stores boot mode and the lengths and BLAKE3 digests of the ROM and
BIOS. Evenfall copies borrowed input or adopts an owned ROM into immutable shared
media and computes its digests once. The media handles expose no mutable bytes; replacing
an entire public `Machine.bus` or `Session.machine` is outside the supported
media-binding lifecycle. The reader checks magic, version, schema, payload bound,
boot mode, ROM and BIOS in that order, then validates the payload length,
checksum and absence of trailing data. It reports the specific version,
schema or media mismatch. The checksum detects corruption, not forgery:
hardware values and invariants must still be checked even when it matches.

`Machine::write_state` counts and then writes the canonical payload with its
header and digest; the writer-facing Rust implementation currently buffers
the payload before sending it to the caller's `Write`. `write_state_alloc`
encodes once and fills the header afterward. Both produce the same bytes.
The codec envelope also exposes `write_streaming` and `write_buffered` for
callers using its `Sink` API. A terminal fault, a noncanonical framebuffer,
a busy state or invalid subsystem state cannot be saved. `Session::write_state`
first reconstructs selectively skipped video.

`Machine::load_state` checks the envelope, decodes the cycle, CPU and bus into
a heap-allocated scratch machine with the resident media, validates their
invariants and publishes it only after all checks pass. Semantic checks cover
CPU mode and continuations, PPU position, timer prescaler alignment, future
DMA and serial deadlines, bounded raster histories, prefetch, cartridge and
APU state. Two checks keep a crafted state from stalling the host. An APU
oscillator phase must
stay below its channel's longest period (a frequency change keeps a phase
from a longer period, so the current period is not the bound), and an RTC
epoch must not pass 9999-12-31T23:59:59Z, the limit a session or the CLI
accepts. Neither is reachable by a running machine, and either would make
the next oscillator advance or date read run for years of host time. A
failed load leaves the running machine unchanged. `Session`
rejects oversized input, reapplies host audio/video policy, resets the audio
resampler and clears rewind history after a successful explicit load.

Durable state includes CPU registers and in-flight work, the master cycle,
memories, timed I/O and device deadlines, cartridge save command state, and
APU phases and FIFOs. Skipped/transient state includes host audio already
produced, rendering and timing caches, host input cursors, the ROM and BIOS
bytes, resampler, video recovery cache and rewind history. The latter are
recomputed, reset or kept as host policy, not serialized as hardware.

## Battery saves, states and rewind

|                          | Battery save                   | Portable state                     | Rewind bundle                           |
| ------------------------ | ------------------------------ | ---------------------------------- | --------------------------------------- |
| Contents                 | cartridge save memory only     | entire machine, including save     | live state, retained history and policy |
| Bound to                 | compatible save size and type  | ROM, BIOS, boot mode and v6 schema | the same media through its states       |
| Survives a format change | yes                            | no                                 | no                                      |
| RTC state                | not included                   | included                           | included                                |
| Rust owner               | `Machine::save_data` / session | `Machine` / `Session`              | `Session` and `RewindBuffer`            |

Loading a state restores its save medium to that moment; resetting with a
battery save instead boots fresh with those bytes. Rewind checkpoints are
complete portable states; restoring one uses the same machine loader.

## Verification

The machine's save-state tests live in
[`systems/gba/tests/machine_b.rs`](../../systems/gba/tests/machine_b.rs). They
pin the exact version 6 bytes, check that a round trip resumes bit-identical
execution, and cover the envelope's checksum and framing, media binding,
semantic validation with transactional failure, timer and PPU invariants,
and host audio, which is not durable.
[`crates/state/tests/codec.rs`](../../crates/state/tests/codec.rs) locks the
primitive, derive, schema, envelope, validation-order and tracer rules, and
`buffered_snapshots_match_streamed_state_bytes_for_owned_media` in
[`Session`](../../systems/gba/src/session.rs) compares the two machine
writers.

## Open questions

- No converter exists for older format versions.
- Battery saves do not carry RTC registers or game-written clock offsets.
- The payload digest provides integrity checking, not authentication.
