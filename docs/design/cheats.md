# Cheats

The GBA core runs GameShark v1/v2, Action Replay v3, CodeBreaker and raw
address/value codes as a cheat device plugged into the machine. Codes are
decoded once into operations; the machine runs them at a fixed point of
emulated time, so a run with cheats is as deterministic as one without.
[`cheats`](../../systems/gba/src/cheats/mod.rs) owns parsing, decryption and
the interpreter; [`Machine`](../../systems/gba/src/machine.rs) owns when they
run and how states carry them. Hosts reach them through the
[CLI](../cli.md#cheats) and the [SDK](../sdk.md#cheats).

## Formats

| Name                      | Device                                            | Line                |
| ------------------------- | ------------------------------------------------- | ------------------- |
| `raw`                     | none: one write per line                          | `AAAAAAAA:VV`       |
| `gameshark`               | GameShark Advance, Action Replay v1/v2, encrypted | `AAAAAAAA VVVVVVVV` |
| `gameshark-decrypted`     | the same codes in raw form                        | `AAAAAAAA VVVVVVVV` |
| `action-replay`           | Action Replay v3, also sold as GameShark v3       | `AAAAAAAA VVVVVVVV` |
| `action-replay-decrypted` | the same codes in raw form                        | `AAAAAAAA VVVVVVVV` |
| `codebreaker`             | CodeBreaker, GameShark SP, Xploder                | `AAAAAAAA VVVV`     |

A raw value of 2, 4 or 8 digits writes a byte, halfword or word. Lines may be
separated by newlines, `+`, `,` or `;`, and a line's halves by a space or
nothing (`9266FA6C97BD`); `#` starts a comment. Canonical text is uppercase,
one line per code line.

Codes sold as "GameShark v3" for Pokémon games are Action Replay v3 codes.
Detection (`auto`) reads raw lines by their colon and 8+4-digit lines as
CodeBreaker. For 8+8-digit lines it decodes the code as encrypted and raw
Action Replay v3 and GameShark v1/v2 and keeps the readings whose addresses
all fall inside EWRAM, IWRAM, I/O or video memory, whose hooks point into the
Game Pak and whose game IDs are printable; device-button and slowdown codes
count as implausible. Exactly one surviving reading is used; none or several
is an error that names the candidates, and the host must name the format.

## Code types

Code types and fields follow GBATEK's "GBA Cheat Codes" pages, with the
Hacking Text's Action Replay v3 table (Kenobi) and EnHacklopedia for details
GBATEK leaves open. Every format decodes to the same operations: writes and
fills, slides, pointer writes, add/OR/AND updates, conditionals and blocks.
Addresses of a width are aligned to it, as the devices' ARM stores are.

- **GameShark v1/v2**: 8-, 16- and 32-bit writes; the `3000cccc` group
  write, which also writes its value to the address equal to itself (a device
  bug both EnHacklopedia and the Hacking Text describe); `D` and `E0zz`
  16-bit equality tests of one or `zz` codes. The `F` hook and `001DC0DE` ID
  codes are accepted and do nothing; GS-button codes and slowdown never fire.
- **Action Replay v3**: `ttaaaaaa` with sub-type, type, size and an extra
  bit; the `a0aaaaa` address takes bits 20–21 as the region. Fills, pointer
  writes, adds, I/O writes (`C6`/`C7`), the seven conditions with size 3 as
  "always false", and the one, two, block and "turn off all codes" actions.
  Special codes: end of list, `ELSE`, `ENDIF`, slowdown and the byte,
  halfword and word slides (`ssccssss`: value step, count, address step).
  `DEADFACE` reseeds with GBATEK's formula.
- **CodeBreaker**: `0` game ID and `1` hook (no effect), `2` OR, `3` byte
  write, `4` slide (`iiiicccc ssss`), `5` byte string, `6` AND, `7`/`A`
  equality tests, `8` halfword write, `9` encryption, `B`/`C` signed
  comparisons, `D0000020` joypad test (all listed keys held), `E` add and
  `F` AND test.

A conditional skips the next codes, not lines: a multi-line code is one code.

## Encryption

GameShark and Action Replay encrypt each line with 32 rounds of TEA, keyed by
the device's four seeds (GBATEK gives both sets). Action Replay's `DEADFACE`
derives new seeds from two 256-byte tables, which come from the device
firmware as published in the MIT-licensed
[kirschju/gameshark-gba-tooling](https://github.com/kirschju/gameshark-gba-tooling)
`arcrypt.py`; key zero reproduces GBATEK's startup seeds. CodeBreaker's
`9xyyxxxx xxxx` code builds a bit permutation and seed list with GBATEK's
generator, and every later line is decrypted with them.

GBATEK's CodeBreaker decryption omits one step: after XORing the address word
with `word[seedlist+8]`, the value halfword is XORed with
`halfword[seedlist+12]`. Without it, published encrypted master codes do not
decrypt to their published raw forms; with it, they do.

### Evidence

[`cheats/tests.rs`](../../systems/gba/src/cheats/tests.rs) checks decryption
only against values from outside the code:

- the published TEA test vector (key 0, plaintext 0, `41EA3A0A 94BAA940`);
- GBATEK's Action Replay startup seeds, from `DEADFACE 0000`;
- Pokémon Emerald's Action Replay master and Infinite Money codes, published
  encrypted and decrypted (`C40005EC 00008401`, `04225E90 000F423F`);
- Action Replay and GameShark master codes for Ruby (USA) and Rubin (German),
  whose `xxxxxxxx 001DC0DE` lines must decrypt to the cartridge game codes
  `AXVE` and `AXVD`, and whose GameShark hook matches the Action Replay one;
- encrypted CodeBreaker master codes for Ruby (USA) and Emerald that decrypt
  to separately published raw masters (`0000B138 000A 10000428 0007`,
  `00006FA7 000A`).

Code-type decoding is checked against the tables' own examples, for instance
GBATEK's `5A` condition and the Hacking Text's group write.

## When cheats run

Real devices run their codes when the game executes a hooked instruction,
which the master code chooses. The core does not model hooks. It runs every
active cheat, in order, at the first instruction boundary at or after each
VBlank start (line 160, dot 0), as mGBA 0.10.5 does at VBlank when a code has
no hook. The point is defined by the video position, not the master cycle:
the LCD stops in STOP, so an application falls due only when the video
reaches the next VBlank.

Execution chunks, HALT idles, waits and linked-machine idles all end at the
next application's cycle, so the machine stops there however the host slices
time, and cheats run at that stop. A host call that ends between two
applications is result-neutral, as it is without cheats. The tests compare
state bytes and frames of runs sliced into one call, thousands of odd-sized
calls and single steps, with cheats writing RAM, palette, VRAM and I/O, and of
runs and waits through HALT; removing the chunk bound fails them.

Cheat stores have a CPU store's effects without its cycles: RAM writes
invalidate decoded code, video memory writes respect raster history, and I/O
writes have their register side effects, including starting DMA. Writes to
the BIOS, Game Pak and save media are ignored. Reads for conditions use
zero-cycle inspection. With no cheats, the only added work is a check for the
absent device per execution chunk, single step and HALT idle; the execution
engine and devices are unchanged.

## States, recordings and scenarios

The active cheat set is machine state, but not ZiGBA's: the `ZIGBAST1`
version 6 payload and schema digest are unchanged, and a machine without
cheats writes exactly ZiGBA's bytes. A machine with cheats writes the same
`ZIGBAST1` state behind an envelope:

| Offset | Size | Field                                          |
| -----: | ---: | ---------------------------------------------- |
|      0 |    8 | magic `EVGBCHT1`                               |
|      8 |    4 | cheat block length `n` (`u32`, at most 64 KiB) |
|     12 |    n | cheat block                                    |
| 12 + n |   32 | BLAKE3 of bytes 0 to 12 + n                    |
| 44 + n |    … | `ZIGBAST1` state                               |

The block is a `u16` cheat count, then per cheat a format tag (0 raw,
1 GameShark, 2 GameShark decrypted, 3 Action Replay, 4 Action Replay
decrypted, 5 CodeBreaker), a `u16` line count and nine bytes per line: the
address word, the value word and the value's digit count, all little-endian.
Cheats are stored as entered, with their resolved format, and decoded again on
load; a block that does not decode rejects the state. The next application
point is not stored: at every boundary a host observes, it is the first
VBlank start after the current video position.

Loading a state therefore restores its cheat set, and a state without an
envelope clears it. Rewind checkpoints, rewind bundles and failure-bundle
snapshots are states, so they carry cheats too. A reset keeps the cheat
device; loading a new game starts without cheats.

Hosts change the set with a call at the current cycle; it first applies at the
next VBlank start. Recordings capture each change as a timed event:
selective video journals it with the host's other calls, a failure bundle
records a `setCheats` operation, and a gameplay recording exports a scenario
with an optional `cheats` list of `{ at, cheats: [{ format, code }] }` events
in cycle order, no later than the last checkpoint. The scenario runner ends
execution chunks at each event's cycle and applies the event at the first stop
at or after it, before a checkpoint at the same cycle is observed. A recorded
event lands on the cycle where it was made, because that cycle was a stop of
the recorded run. A recording that starts from a cold boot after a reset
begins with an event restoring the kept cheats.

## Limits and unsupported codes

- At most 256 cheats, 1,024 lines per cheat and 4,096 lines in all; one code
  may write at most 256 KiB per application.
- ROM patch codes (GameShark `6`, Action Replay `18`–`1E`) are rejected: the
  Game Pak is shared, immutable media, and patching it would reach the
  pre-decoded execution engine.
- GameShark v1/v2 `DEADFACE` is rejected: its seed tables are not published
  outside emulator sources.
- Hooks, the Action Replay's per-hook code count and the device buttons are
  not modeled; button codes never fire.
- Each cheat is its own code list: an end code or "turn off all codes" stops
  only that cheat. A false "turn off all codes" stops the rest of the cheat
  for this application; the device instead loops until the condition holds.

## Choices where sources disagree

- GBATEK reads Action Replay `00000000 60000000` as `ELSE`, with a question
  mark; the Hacking Text describes it as "execute all remaining codes". The
  core follows GBATEK: it toggles skipping of the current block. Blocks do
  not nest.
- Slides write `cc` times, as GBATEK and EnHacklopedia state; the Hacking Text
  describes `cc + 1` writes.
- CodeBreaker `B` and `C` compare signed, as GBATEK states.
- GBATEK's CodeBreaker `FOR i=0 TO byte[code+3]-91h` loop runs no times when
  that byte is below `91h`; no published code checks this.

## Open questions

- No encrypted code using Action Replay `DEADFACE` has been checked against a
  published decryption; only the tables' first entries are pinned by the
  startup seeds.
- Timing is VBlank-based for every game. A code whose effect depends on its
  master code's hook address can behave differently than on the device.
