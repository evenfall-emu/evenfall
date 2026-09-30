# Cheats

The GBA core runs GameShark v1/v2, Action Replay v3, CodeBreaker and raw address/value codes, as a cheat device plugged into the machine would.
Each code is decoded once into a short list of operations: writes, fills, slides, pointer writes, updates and conditionals.
The machine runs every active cheat at a fixed point of emulated time, just after each VBlank start.
A run with cheats is therefore as repeatable as one without, however the host slices execution.
The active cheats are part of the machine state, so save states, rewind and recordings reproduce them.

[`cheats`](../../systems/gba/src/cheats/mod.rs) owns parsing, decryption and the interpreter.
[`Machine`](../../systems/gba/src/machine.rs) decides when cheats run and how states carry them.
Hosts reach them through the [CLI](../cli.md#cheats) and the [SDK](../sdk.md#cheats).

## Formats

| Name                      | Device                                            | Line                |
| ------------------------- | ------------------------------------------------- | ------------------- |
| `raw`                     | none, one write per line                          | `AAAAAAAA:VV`       |
| `gameshark`               | GameShark Advance, Action Replay v1/v2, encrypted | `AAAAAAAA VVVVVVVV` |
| `gameshark-decrypted`     | the same codes in raw form                        | `AAAAAAAA VVVVVVVV` |
| `action-replay`           | Action Replay v3, also sold as GameShark v3       | `AAAAAAAA VVVVVVVV` |
| `action-replay-decrypted` | the same codes in raw form                        | `AAAAAAAA VVVVVVVV` |
| `codebreaker`             | CodeBreaker, GameShark SP, Xploder                | `AAAAAAAA VVVV`     |

A raw value of 2, 4 or 8 digits writes a byte, halfword or word.
Lines may be separated by newlines, `+`, `,` or `;`, and a line's two halves by a space or nothing (`9266FA6C97BD`).
`#` starts a comment.
Canonical text is uppercase, one line per code line.

Codes sold as "GameShark v3" for Pokémon games are Action Replay v3 codes.

### Detection

`auto` reads lines with a colon as raw and 8+4-digit lines as CodeBreaker.
For 8+8-digit lines it decodes the code four ways: encrypted and raw Action Replay v3, encrypted and raw GameShark v1/v2.
It keeps the readings whose addresses all fall in EWRAM, IWRAM, I/O or video memory, whose hooks point into the Game Pak and whose game IDs are printable.
Device-button and slowdown codes count as implausible, because other formats' readings often decode to them.
Exactly one surviving reading is used.
None or several is an error that names the candidates, and the host must name the format.

## Code types

Code types and fields follow GBATEK's "GBA Cheat Codes" pages.
The Hacking Text's Action Replay v3 table and EnHacklopedia fill in details GBATEK leaves open.
Every format decodes to the same operations.
Accesses of a width are aligned to it, as the devices' ARM stores are.
A conditional skips the next codes, not lines, so a multi-line code counts as one code.

### GameShark v1/v2

- `0`, `1` and `2` write a byte, halfword or word.
- `3000cccc` writes one word to a list of addresses on the following lines. It also writes the value to the address equal to itself, a device bug both EnHacklopedia and the Hacking Text describe.
- `D` tests a halfword for equality and skips the next code when it differs. `E0zz` does the same for the next `zz` codes.
- `F` hooks and `001DC0DE` game IDs are accepted and do nothing.
- GS-button writes and slowdown never fire, because the device button is never pressed.

### Action Replay v3

The address word is `ttaaaaaa`, with a sub-type, a type, a size and an extra bit.
The `a0aaaaa` address takes bits 20-21 as the memory region.

- Type zero writes: byte and halfword fills, word writes, pointer writes, adds and I/O writes (`C6` and `C7`).
- Other types are conditions: equal, not equal, signed and unsigned less and greater, and AND. Size 3 is "always false".
- A condition's sub-type chooses what a false test skips: the next code, the next two, the block up to `ELSE` or `ENDIF`, or every remaining code.
- Special codes: end of list, `ELSE`, `ENDIF`, slowdown, and the byte, halfword and word slides. A slide's `ssccssss` holds the value step, count and address step.
- `DEADFACE` changes the decryption seeds.

### CodeBreaker

- `0` game ID and `1` hook do nothing.
- `2` ORs, `6` ANDs and `E` adds a halfword.
- `3` writes a byte and `8` a halfword.
- `4` is a halfword slide with `iiiicccc ssss` parameters: value step, count, address step.
- `5` writes a byte string that follows, six bytes per line.
- `7` and `A` test a halfword for equality or inequality, `B` and `C` compare it signed, and `F` tests it with AND.
- `D0000020` runs the next code when every listed key is held.
- `9` turns on encryption for every later line.

## Encryption

GameShark and Action Replay encrypt each line with 32 rounds of TEA, keyed by the device's four seeds.
GBATEK gives both seed sets.
Action Replay's `DEADFACE` derives new seeds from two 256-byte tables in the device firmware.
The tables come from the MIT-licensed [kirschju/gameshark-gba-tooling](https://github.com/kirschju/gameshark-gba-tooling) `arcrypt.py`, and key zero reproduces GBATEK's startup seeds.

CodeBreaker's `9xyyxxxx xxxx` code builds a bit permutation and a seed list with GBATEK's random generator, and every later line is decrypted with them.
GBATEK's decryption omits one step.
After the address word is XORed with `word[seedlist+8]`, the value halfword is XORed with `halfword[seedlist+12]`.
Without that step, published encrypted master codes do not decrypt to their published raw forms.

### Evidence

[`cheats/tests.rs`](../../systems/gba/src/cheats/tests.rs) checks decryption only against values from outside the code:

- the published TEA test vector: key 0 and plaintext 0 encrypt to `41EA3A0A 94BAA940`
- GBATEK's Action Replay startup seeds, derived from `DEADFACE 0000`
- Pokémon Emerald's Action Replay master and Infinite Money codes, published both encrypted and decrypted (`C40005EC 00008401`, `04225E90 000F423F`)
- Action Replay and GameShark master codes for Ruby (USA) and Rubin (German), whose `xxxxxxxx 001DC0DE` lines must decrypt to the cartridge game codes `AXVE` and `AXVD`
- encrypted CodeBreaker master codes for Ruby (USA) and Emerald, which decrypt to separately published raw masters (`0000B138 000A 10000428 0007`, `00006FA7 000A`)

Code-type decoding is checked against the tables' own examples, such as GBATEK's `5A` condition and the Hacking Text's group write.

## When cheats run

Real devices run their codes when the game executes a hooked instruction, which the master code chooses.
Evenfall does not model hooks.
It runs every active cheat, in list order, at the first instruction boundary at or after each VBlank start (line 160, dot 0).
mGBA does the same for a code without a hook.
The point follows the video position, not the master cycle, so a stopped LCD in STOP delays it.

Execution chunks, HALT idles, waits and linked-machine idles all end at the next application's cycle.
The machine therefore stops there however the host slices time, and the cheats run at that stop.
The tests compare state bytes and frames of runs sliced into one call, thousands of odd-sized calls and single steps.
The cheats in those runs write RAM, palette, VRAM and I/O, and further runs go through HALT.
Removing the chunk bound fails them.

A cheat store has a CPU store's effects without its cycles.
RAM writes invalidate decoded code, video memory writes respect raster history, and I/O writes have their register side effects, including starting DMA.
Writes to the BIOS, Game Pak and save media are ignored.
Conditions read memory with zero-cycle inspection.

With no cheats, the only added work is a check for the absent device per execution chunk, single step and HALT idle.
The execution engine and devices are unchanged.

## States, recordings and scenarios

A machine without cheats writes exactly the `EVENFALL` version 7 state described in [`save-states.md`](save-states.md).
A machine with cheats writes the same state behind a short envelope that carries the cheat set.
The v7 layout and schema digest stay unchanged, so the state's fixed layout holds in both cases.
The envelope is small and exists only while cheats are on, whereas a fixed slot inside the v7 payload would add the largest cheat set's size to every state.

Loading a state restores its cheat set, and a state without the envelope turns cheats off.
Rewind checkpoints, rewind bundles and failure-bundle snapshots are states, so they carry cheats too.
A reset keeps the cheat device, and loading a new game starts without cheats.
The next application point is not stored: at every boundary a host observes, it is the first VBlank start after the current video position.

Hosts change the cheat set with a call at the current cycle, and the change first applies at the next VBlank start.
Recordings capture each change as a timed event:

- Selective video journals it with the host's other calls.
- A failure bundle records a `setCheats` operation and replays it.
- A gameplay recording exports a scenario with an optional `cheats` list of `{ at, cheats: [{ format, code }] }` events, in cycle order and no later than the last checkpoint.

The scenario runner ends execution chunks at each event's cycle and applies the event at the first stop at or after it, before observing a checkpoint at the same cycle.
A recorded event lands on the cycle where it was made, because that cycle was a stop of the recorded run.
A recording that starts from a cold boot after a reset begins with an event restoring the kept cheats.

### Envelope format

All integers are little-endian.

| Offset | Size | Field                                          |
| -----: | ---: | ---------------------------------------------- |
|      0 |    8 | magic `EVGBCHT1`                               |
|      8 |    4 | cheat block length `n` (`u32`, at most 64 KiB) |
|     12 |    n | cheat block                                    |
| 12 + n |   32 | BLAKE3 of bytes 0 to 12 + n                    |
| 44 + n |  ... | `EVENFALL` state                               |

The cheat block is a `u16` cheat count, then for each cheat:

- a format tag: 0 raw, 1 GameShark, 2 GameShark decrypted, 3 Action Replay, 4 Action Replay decrypted, 5 CodeBreaker
- a `u16` line count
- nine bytes per line: the address word, the value word and the value's digit count

Cheats are stored as entered, with their resolved format, and decoded again on load.
A block that does not decode, or a digest that does not match, rejects the whole state.

## Choices where sources disagree

- GBATEK reads Action Replay `00000000 60000000` as `ELSE`, with a question mark. The Hacking Text calls it "execute all remaining codes". Evenfall follows GBATEK and toggles skipping of the current block. Blocks do not nest.
- Slides write `cc` times, as GBATEK and EnHacklopedia state. The Hacking Text describes `cc + 1` writes.
- CodeBreaker `B` and `C` compare signed, as GBATEK states.
- GBATEK's CodeBreaker `FOR i=0 TO byte[code+3]-91h` loop runs no times when that byte is below `91h`.

## Known limits

- One machine holds at most 256 cheats, 1,024 lines per cheat and 4,096 lines in all. One code may write at most 256 KiB per application.
- ROM patch codes (GameShark `6`, Action Replay `18` to `1E`) are rejected. The Game Pak is shared, immutable media, and patching it would also have to reach the pre-decoded execution engine.
- GameShark v1/v2 `DEADFACE` is rejected, because its seed tables are not published outside emulator sources.
- Hooks, the Action Replay's per-hook code count and the device buttons are not modeled. Button codes never fire.
- Each cheat is its own code list. An end code or "turn off all codes" stops only that cheat. A false "turn off all codes" stops the rest of the cheat for this application, where the device would loop until the condition holds.
- Every game gets VBlank timing. A code whose effect depends on its master code's hook address can behave differently than on the device.
- No encrypted code that uses Action Replay `DEADFACE` has been checked against a published decryption. Only the tables' first entries are pinned, by the startup seeds.
