# Save states

A save state is a snapshot of the whole emulated console at one instant: CPU registers, memory, video and sound state, timers, pending DMA, the cartridge's save chip, everything.
Loading it puts the console back exactly where it was, and from there it runs exactly as it did the first time.
Players use states for quick saves, rewind is built on them, and a state attached to a bug report reproduces the bug.

A state is different from the game's own save, the battery save, which is just the contents of the cartridge's save chip.
A battery save works in any emulator and survives Evenfall updates.
A save state is tied to the exact ROM and BIOS it was made with, and to this version of the state format.
It is for picking up where you left off, not for long-term storage.

The format is `ZIGBAST1` version 6.
Its name comes from Evenfall's predecessor, and states saved there load here too.

## How it works

```text
 write                                   load
┌──────────────────────────┐            ┌──────────────────────────────────┐
│ check the machine can be │            │ check magic, version, schema,    │
│ saved (no fault, no      │            │ media, length, digest            │
│ half-built frame)        │            │                                  │
│                          │   bytes    │ decode into a scratch machine    │
│ encode cycle, CPU, bus   │──────────▶│                                  │
│ in declaration order     │            │ check hardware invariants        │
│                          │            │                                  │
│ header + payload + digest│            │ swap the scratch machine in      │
└──────────────────────────┘            └──────────────────────────────────┘
```

[`Machine`](../../systems/gba/src/machine.rs) writes and loads states, describes their layout, and checks that a loaded state makes sense for the hardware.
The encoding itself lives in [`crates/state`](../../crates/state/src/lib.rs), a `no_std` crate that knows nothing about the GBA.
It provides the `Codec` trait, a `Sink` that counts, hashes and optionally records what it writes, a bounded `Source`, and the [file envelope](../../crates/state/src/envelope.rs).
[`crates/state-derive`](../../crates/state-derive/src/lib.rs) generates the encoders for the CPU, bus, sound chip and cartridge devices from their Rust types.
[`Session`](../../systems/gba/src/session.rs) adds the host's side: it rebuilds skipped video before saving, and resets audio and rewind after loading ([`rewind.md`](rewind.md)).

Every state has these properties:

- **The same everywhere.** The encoding has no pointers, padding or unused union storage, and integer widths and byte order don't depend on the host.
  A state written by the native build loads in the browser and the other way round.
- **Exact.** Everything the CPU and devices were in the middle of, including pending work, survives a round trip.
- **All or nothing.** A load checks the file and the hardware state in a scratch machine before replacing the running one.
  A bad or incompatible file leaves the running game untouched.
- **Bound to its media.** The boot mode and the exact ROM and BIOS contents are part of the state.
- **One version.** The format accepts only its current version and layout, and doesn't migrate older states.

## File layout

All integers are little-endian.

|  Offset | Size | Field                                      |
| ------: | ---: | ------------------------------------------ |
|       0 |    8 | magic `ZIGBAST1`                           |
|       8 |    4 | version `u32` (6)                          |
|      12 |   32 | BLAKE3 digest of the layout description    |
|      44 |    8 | payload length `u64`, at most 1 MiB        |
|      52 |    1 | boot mode: 0 HLE, 1 skip-BIOS, 2 real BIOS |
|      53 |    8 | ROM length `u64`                           |
|      61 |   32 | ROM BLAKE3 digest                          |
|      93 |    8 | BIOS length `u64` (0 under HLE)            |
|     101 |   32 | BIOS BLAKE3 digest                         |
|     133 |    n | payload                                    |
| 133 + n |   32 | payload BLAKE3 digest                      |

Nothing may follow the final digest.
The payload is the master cycle (`u64`), then the CPU, then the bus, each field in the format's declaration order.
The bus includes memory, I/O registers, video histories, timers, DMA, the serial port, the cartridge and the sound chip.
The ROM and BIOS themselves aren't stored, only their digests.

## How the payload is encoded

| Type             | Encoding                                                                    |
| ---------------- | --------------------------------------------------------------------------- |
| integer          | little-endian, `ceil(bit width / 8)` bytes, narrow widths checked on decode |
| `usize`, `isize` | widened to 64 bits, checked on decode                                       |
| `bool`           | one byte, 0 or 1                                                            |
| float            | raw IEEE bit pattern                                                        |
| enum             | integer tag, unknown tags rejected                                          |
| optional         | presence `bool`, then the value if present                                  |
| array            | elements in order, byte arrays verbatim                                     |
| struct           | fields in declaration order, no padding                                     |
| tagged union     | tag, then only the active variant                                           |

Nobody writes an encoder by hand.
`#[derive(Codec)]` generates both the encoding and a description of the layout from each Rust type, with a few attributes:

- `#[codec(bits = N)]` stores a field that is narrower than its Rust type, such as a 4-bit volume in a `u8`.
- `#[codec(skip)]` leaves out a field that isn't hardware state, such as a cache.
  Decoding writes into existing storage, so skipped fields keep their values.
- `#[codec(flatten)]` lays a nested struct's fields out as if they belonged to the parent, which keeps the CPU's layout flat.

Field renames and custom encoders keep the format's field names stable and give unused array slots a fixed value.

### The layout digest

Because the encoding follows the types, a change to any hardware struct changes the format.
The layout description catches that.
It lists the codec and payload identities, then every stored field of the CPU and bus with its name, type, width, array length and enum variants, and its BLAKE3 digest goes in the header.

The description Evenfall produces must equal the version 6 [fixture](../../tools/parity/fixtures/schema-v6.bin) byte for byte, not just hash to a plausible value, so an accidental layout change fails a test.
A state whose digest doesn't match is rejected with `SaveStateSchemaMismatch`.

A matching layout doesn't prove the values are right.
A test also pins the exact bytes of a whole state produced by a fixed sequence, against a stored fixture.

## Writing a state

A few machines can't be saved: one stopped on a fault, one whose current picture was skipped and not yet rebuilt, and one with device state the encoder rejects as invalid.
`Session::write_state` rebuilds skipped video first, so a session only refuses to save a faulted or invalid machine.

- `Machine::write_state` encodes the payload twice, once to count its length for the header and once to write it, so the header can go out before the payload.
  It currently buffers the payload before handing it to the caller's `Write`.
- `Machine::write_state_alloc` encodes once and fills in the header afterwards.

Both produce the same bytes.
The envelope also offers `write_streaming` and `write_buffered` for code that uses the `Sink` API directly.

## Loading a state

The loader checks the header in order: magic, version, layout digest, payload length limit, boot mode, ROM, then BIOS.
It then checks the payload length, the payload digest, and that nothing follows it.
Each failure names what didn't match, so a host can tell the player "this state is from another ROM" rather than "invalid file".
The digest detects damage, not tampering, which is why the hardware checks below still run on a file whose digest matches.

`Machine::load_state` then decodes the cycle, CPU and bus into a scratch machine on the heap, sharing the loaded ROM and BIOS, and checks that the result is a state real hardware could be in:

- a valid CPU mode, and a valid state for any BIOS routine still running,
- a possible display position, and timers on the shared prescaler grid,
- DMA and serial deadlines in the future,
- raster histories and the prefetch buffer within their bounds,
- valid cartridge and sound chip state.

Two checks protect the host from crafted files.
A sound oscillator's phase must be less than its channel's longest period, and the real-time clock can't be set past 9999-12-31T23:59:59Z, the same limit sessions and the CLI accept.
A running machine never reaches either value, and either would make the next oscillator step or date read take years.

Only when every check passes does the scratch machine replace the running one.
A failed load leaves the running game exactly as it was.
After a successful load, `Session` also rejects oversized input, reapplies the host's audio and video settings, resets the audio resampler, and clears rewind history.

The ROM and BIOS are shared, immutable buffers, hashed once when loaded.
Nothing can change them behind the state's back.

## What is saved and what isn't

A state holds everything that affects what the console does next: CPU registers and instructions in flight, the master cycle, all memory, I/O registers and device deadlines, the save chip and any command it is in the middle of, and sound phases and queues.

It leaves out what the host owns or can rebuild: audio samples already produced, rendering and timing caches, input cursors, the ROM and BIOS bytes, the resampler, the skipped-video cache and rewind history.
These are reset, recomputed, or kept as host settings.

## Battery saves, states and rewind

|                          | Battery save                  | Save state                            | Rewind bundle                        |
| ------------------------ | ----------------------------- | ------------------------------------- | ------------------------------------ |
| Contains                 | the save chip only            | the whole machine, save chip included | the live state, history and settings |
| Bound to                 | the save type and size        | ROM, BIOS, boot mode and format v6    | the same media, through its states   |
| Survives a format change | yes                           | no                                    | no                                   |
| Real-time clock          | not included                  | included                              | included                             |
| Owned by                 | `Machine::save_data`, session | `Machine`, `Session`                  | `Session`, `RewindBuffer`            |

Loading a state also restores the save chip to that moment.
Resetting with a battery save instead boots fresh with those bytes.
Rewind checkpoints are ordinary save states, restored with the same loader.

## Finding which field differs

When two states that should match don't, tracing says which field the difference is in.
With tracing on, `Sink` records the path and byte range of every field it writes, and `Machine::state_field_ranges` returns them for the whole payload.
Ranges start at the first payload byte, so add the 133-byte header to map a difference in the whole file.
A byte falls in both a struct's range and its field's range, and the narrowest range is the useful one.
Tracing is only a diagnostic and doesn't change the format.

## How it is checked

- [`tests/machine_b.rs`](../../systems/gba/tests/machine_b.rs) pins the exact bytes of a version 6 state, checks that a round trip resumes bit-identical execution, and covers the envelope's digest and framing, media binding, rejected states with the running machine untouched, timer and display invariants, and host audio not being saved.
- `schema_matches_zigba_version_six` compares `Machine::schema_description` with the layout fixture byte for byte.
- [`crates/state/tests/codec.rs`](../../crates/state/tests/codec.rs) locks the encoding of every type, the derive attributes, the layout description, the envelope, the order of checks and tracing.
- `buffered_snapshots_match_streamed_state_bytes_for_owned_media` in [`session.rs`](../../systems/gba/src/session.rs) checks that the two writers produce the same bytes.

## Known limits

- States from older format versions can't be converted.
- Battery saves don't carry the real-time clock or a clock the game has set.
- The digests detect damage, but don't prove who made a file.
