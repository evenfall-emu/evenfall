# Cartridge

A GBA cartridge is more than a ROM image. It can carry a save medium on the
8-bit save bus, a four-bit GPIO port inside the ROM chip with a real-time
clock or sensors behind it, and, for two titles, a tilt sensor in the save
address range. ZiGBA models each of these as a device with its own protocol
state. The core owns every bit-level protocol and its timing; the host owns
files, wall-clock time and the physical value a sensor reports.

| Concern                                           | Code                                                              |
| ------------------------------------------------- | ----------------------------------------------------------------- |
| Address decoding, wait states, EEPROM DMA routing | [`bus/mod.rs`](../../../systems/gba/src/bus/mod.rs)               |
| Save media state machines, SDK-marker scan        | [`cartridge/save.rs`](../../../systems/gba/src/cartridge/save.rs) |
| Title save database, save-type choice, ZIP import | [`rom_image.rs`](../../../systems/gba/src/rom_image.rs)           |
| Save-file persistence for the native frontends    | [`host/host_save.rs`](../../../systems/gba/src/host/host_save.rs) |
| Peripheral detection from the header              | [`cartridge/mod.rs`](../../../systems/gba/src/cartridge/mod.rs)   |
| GPIO port, RTC, solar sensor, gyro, rumble        | [`cartridge/gpio.rs`](../../../systems/gba/src/cartridge/gpio.rs) |
| Tilt sensor                                       | [`cartridge/tilt.rs`](../../../systems/gba/src/cartridge/tilt.rs) |

No public Nintendo specification of the backup chips or cartridge
peripherals was located. The hardware reference is GBATEK's
[cartridge chapter](https://rust-console.github.io/gbatek-gbaonly/#gbacartridges),
cross-checked against pinned mGBA 0.10.5 and NanoBoyAdvance
([ZiGBA's `refs/README.md`](../../../refs/zigba/refs/README.md) records the revisions). Where they
disagree, this page says which one ZiGBA follows.

## Header and ROM bus

The first 192 bytes of the ROM are the header
([GBATEK](https://rust-console.github.io/gbatek-gbaonly/#gbacartridgeheader)):

| Offset        | Size | Field                                              |
| ------------- | ---- | -------------------------------------------------- |
| `000h`        | 4    | ARM branch to the entry point                      |
| `004h`        | 156  | Nintendo logo, compared by the BIOS                |
| `0A0h`        | 12   | Title                                              |
| `0ACh`        | 4    | Game code                                          |
| `0B0h`        | 2    | Maker code                                         |
| `0B2h`        | 1    | Fixed value `96h`                                  |
| `0B3h`–`0BCh` | 10   | Unit code, device type, reserved, version          |
| `0BDh`        | 1    | Complement: `-(sum of 0A0h..0BCh) - 19h`, low byte |
| `0BEh`        | 2    | Reserved                                           |

ZiGBA does not validate the header. HLE boot enters `08000000h` without the
logo and complement checks the retail BIOS performs; with an external BIOS
image those checks run as ordinary BIOS code. The core reads only the game
code, to select cartridge peripherals and to consult the save-type database.
There is no backup-type field.

The ROM sits on a 16-bit multiplexed bus and appears in three wait-state
windows at `08000000h`, `0A000000h` and `0C000000h`, each 32 MiB. The
frontends and the web session accept images up to 32 MiB. ROM writes cost
Game Pak wait states and change nothing, and reads beyond the image return
the multiplexed address pattern. Wait states, prefetch and those unused reads
are described in [`memory.md`](memory.md). Two devices overlay the ROM
windows: the GPIO registers at offset `C4h`–`C9h` and, for EEPROM
cartridges, part of the `0Dxxxxxxh` range.

The save window covers `0E000000h`–`0FFFFFFFh`. It is 8 bits wide: the
device sees a 16-bit offset, so the medium repeats every 64 KiB, and SRAM
also repeats every 32 KiB. A halfword or word read returns the byte
replicated into every lane; a halfword or word write stores the one byte
selected by the low address bits. Each access costs the SRAM wait state
configured in `WAITCNT` bits 1–0.

## Save media

`cartridge::save::Type` (exposed as `SaveType` by `Machine`) selects the medium:

| `SaveType`         | CLI name        | Size             | Medium                                 |
| ------------------ | --------------- | ---------------- | -------------------------------------- |
| `.auto`            | `auto`          | 0 until resolved | Runtime classification, below          |
| `.none`            | `none`          | 0                | No medium: reads `FFh`, writes ignored |
| `.sram`            | `sram`          | 32 KiB           | SRAM or FRAM                           |
| `.flash_64k`       | `flash64`       | 64 KiB           | Flash, ID `1CC2h`                      |
| `.flash_64k_atmel` | `flash64-atmel` | 64 KiB           | Atmel Flash, ID `3D1Fh`                |
| `.flash_128k`      | `flash128`      | 128 KiB          | Flash, ID `09C2h`                      |
| `.eeprom_512`      | `eeprom512`     | 512 B            | 4 Kbit EEPROM                          |
| `.eeprom_8k`       | `eeprom8k`      | 8 KiB            | 64 Kbit EEPROM                         |

`Machine::save_type()` and `Machine::save_data()` expose the resolved type and
bytes. The medium's bytes and its command and transfer state are part of the
portable save state ([`design/save-states.md`](../../design/save-states.md)),
so a state saved mid-command resumes at the same step. An HLE `HardReset`
leaves the medium and every attached peripheral untouched.

### SRAM and FRAM

SRAM and FRAM share one software interface: 32 KiB at
`0E000000h`–`0E007FFFh`, direct byte loads and stores, no commands and no
busy time
([GBATEK](https://rust-console.github.io/gbatek-gbaonly/#gbacartbackupsramfram)).
FRAM differs only in how it retains data, so ZiGBA treats them as one device.
A new SRAM image is filled with `FFh`. The `jsmolka-unsafe` validator
exercises the 32 KiB mirroring.

### Flash 64 KiB and 128 KiB

Flash occupies `0E000000h`–`0E00FFFFh` and is driven by byte commands. With
`S = 0E000000h`, every command starts with the unlock prefix `AAh` to
`S+5555h`, `55h` to `S+2AAAh`
([GBATEK](https://rust-console.github.io/gbatek-gbaonly/#gbacartbackupflashrom)):

| Operation     | Sequence after the prefix                      | ZiGBA behavior                                                                                 |
| ------------- | ---------------------------------------------- | ---------------------------------------------------------------------------------------------- |
| Enter ID mode | `90h` to `S+5555h`                             | Reads at offset 0 and 1 return manufacturer and device; other offsets read `FFh`               |
| Exit ID mode  | `F0h` to `S+5555h`                             | Normal reads resume                                                                            |
| Program byte  | `A0h` to `S+5555h`, then data to the target    | The byte is ANDed into the cell, so programming can only clear bits                            |
| Erase chip    | `80h` to `S+5555h`, prefix, `10h` to `S+5555h` | Every byte, both banks on 128 KiB, becomes `FFh`, readable at once                             |
| Erase sector  | `80h` to `S+5555h`, prefix, `30h` to `S+n000h` | The 4 KiB sector becomes `FFh`; it reads `5Fh` for 30,000 cycles                               |
| Select bank   | `B0h` to `S+5555h`, then bank to `S+0`         | 128 KiB only; bit 0 of the value selects the 64 KiB bank for reads, programs and sector erases |

A write that does not continue the current sequence returns the state
machine to idle. On a fixed 64 KiB device the bank command is ignored.

GBATEK documents these retail IDs (device in the high byte, manufacturer in
the low byte): `D4BFh` SST, `1CC2h` Macronix, `1B32h` Panasonic and `3D1Fh`
Atmel for 64 KiB; `1362h` Sanyo and `09C2h` Macronix for 128 KiB. ZiGBA
reports the Macronix IDs for common Flash.

Program and erase completion runs on the emulated clock, never on the host
file. Vendor timings are not established: GBATEK lists only timeouts, and
pinned mGBA's
[timing notes](https://github.com/mgba-emu/mgba/blob/0.10.5/src/gba/savedata.c#L21-L31)
record erase times from about 2,000 to 30,000 cycles depending on the chip
and call its constants estimates. ZiGBA adopts mGBA's 30,000-cycle
sector-erase estimate and, like mGBA, ties a settling sector only to sector
erase: reads of that sector return `5Fh` until the deadline while other
sectors read normally. Byte
programs and chip erase are readable immediately, which matches the pinned
[jsmolka save tests](https://github.com/jsmolka/gba-tests/tree/master/save).
The `5Fh` value and deadline are compatibility policy, not a claim that every
chip answers the same way.

ZiGBA does not model vendor status polling, the Macronix-only `F0h`
terminate-after-timeout command, or GBATEK's note that Flash reads work only
from code executing outside the ROM.

### Atmel 64 KiB

The Atmel `3D1Fh` part has 128-byte pages instead of 4 KiB sectors. It is
available only through `flash64-atmel`, because no ROM marker identifies the
vendor and the title database has no Atmel entry. Its differences:

- ID mode returns manufacturer `1Fh`, device `3Dh`.
- `A0h` starts a page write. The first byte must be at an `80h`-aligned
  address, otherwise the command is dropped. That byte erases the page to
  `FFh`, then each of the 128 sequential bytes replaces its cell rather than
  ANDing into it. A byte out of sequence aborts the page.
- The 4 KiB sector erase is inert; chip erase works.
- A partial page's progress survives a save state, so a transaction can
  resume mid-page.

No busy time is modeled. The test "Atmel Flash identifies itself and
erase-writes one aligned 128-byte page" in
[`cartridge/save.rs`](../../../systems/gba/src/cartridge/save.rs)
starts from zeroed cells to tell erase-and-replace apart from a bitwise
program.

### EEPROM 512 B and 8 KiB

EEPROM is a serial chip on bit 0 of the WS2 ROM data bus
([GBATEK](https://rust-console.github.io/gbatek-gbaonly/#gbacartbackupeeprom)).
It answers anywhere in `0D000000h`–`0DFFFFFFh` when the ROM is 16 MiB or
smaller, and only at `0DFFFF00h`–`0DFFFFFFh` for a larger ROM. Halfword
accesses there reach the EEPROM; everything else in the range is ordinary
ROM.

Data moves in 8-byte blocks. Each serial bit is bit 0 of one 16-bit DMA3
unit, and addresses and data are sent MSB first:

| Transfer     | 512 B   | 8 KiB   | Stream                                 |
| ------------ | ------- | ------- | -------------------------------------- |
| Read request | 9 bits  | 17 bits | `11`, 6- or 14-bit block address, `0`  |
| Read data    | 68 bits | 68 bits | four dummy `0` bits, then 64 data bits |
| Write        | 73 bits | 81 bits | `10`, block address, 64 data bits, `0` |

The 8 KiB part uses the low 10 address bits (1,024 blocks); GBATEK says the
upper four should be zero, and ZiGBA discards a command in which they are
not. A command whose final bit is 1 is discarded. A write commits all eight
bytes at its final bit and makes the chip busy for 108,368 cycles, GBATEK's
figure of about 6.5 ms.

GBATEK reports that serial streams sent with `LDRH`/`STRH` do not work on
hardware. ZiGBA accepts serial input only from DMA3 halfword writes; a CPU
store to the window goes down the ROM path and is ignored. Reads are
different: software polls for completion with ordinary `LDRH`, so any
halfword read of the window returns the next pending data bit, or, with no
read in progress, the status bit (0 while busy, 1 when ready). The upper 15
bits read zero. EEPROM accesses cost WS2 wait states.

When EEPROM is the medium, traffic to the 8-bit save window reads `FFh`,
ignores writes and leaves any serial transaction untouched. This is
an explicit policy for invalid traffic, not a claim that such accesses do
anything on hardware. The test "EEPROM byte-window traffic is open and
leaves the serial protocol unchanged" in
[`cartridge/save.rs`](../../../systems/gba/src/cartridge/save.rs)
interleaves them into a read for both capacities.

## Save-type detection

Nothing in the header names the save medium. Nintendo's SDK links
word-aligned ID strings (`EEPROM_Vnnn`, `SRAM_Vnnn`, `FLASH_Vnnn`,
`FLASH512_Vnnn`, `FLASH1M_Vnnn`) into the ROM, but they can be missing from
homebrew and hacks, stale, or duplicated, and `EEPROM_Vnnn` does not give
the capacity
([GBATEK, backup IDs](https://rust-console.github.io/gbatek-gbaonly/#gbacartbackupids)).
ZiGBA therefore layers several sources of evidence, most specific first, and
never truncates an existing save on a guess.

1. **Explicit type.** A configured type other than `auto` is used as given.
   An existing save file must have at least that type's size. `none` ignores
   any file.
2. **Title database.** `SAVE_OVERRIDES` in
   [`rom_image.rs`](../../../systems/gba/src/rom_image.rs) lists entries on which
   pinned mGBA 0.10.5 and NanoBoyAdvance agree: three full-ROM CRC32 values
   for Pokémon Ruby (`AXVE`) revisions select `flash128`, and the game codes
   `A2YE` (Top Gun: Combat Zones, no save), `AGFE` (Golden Sun: The Lost
   Age, `flash64`) and `RZWE` (WarioWare: Twisted!, `sram`). A CRC entry must
   match exactly; a code-only entry is the fallback. The database outranks
   ROM markers, so a listed title keeps its type even if it carries a
   misleading marker. NanoBoyAdvance's EEPROM capacities are marked uncertain
   in its own source and are not imported.
3. **SDK markers.** `detect_type` in
   [`cartridge/save.rs`](../../../systems/gba/src/cartridge/save.rs)
   accepts a marker only at a four-byte-aligned offset followed by three
   decimal digits. `SRAM_V`, `FLASH_V`, `FLASH512_V` and `FLASH1M_V` choose
   their type; `EEPROM_V` leaves the capacity open. Markers of different
   types are an error that requires an explicit type.
4. **Existing save file.** An exact 512 B, 8 KiB, 32 KiB, 64 KiB or 128 KiB
   file must agree with the database or marker, if either applies, and
   otherwise its size chooses the type (64 KiB means common Flash, never
   Atmel). A known type also accepts a nonstandard longer file, ignoring
   everything after the cartridge memory. Without a known type, a longer
   file cannot unambiguously identify the medium. With an `EEPROM_V` marker,
   an exact file size chooses the capacity; a padded file needs an explicit
   type. A disagreement fails before emulation starts.
5. **Runtime classification.** Anything still undecided starts as `auto`,
   and the cartridge's first unambiguous traffic resolves it inside the core:
   - Reads return `FFh` and resolve nothing.
   - An ordinary byte write selects 32 KiB SRAM.
   - `AAh` to `5555h` followed by `55h` to `2AAAh` is buffered. A third write
     of `80h`, `90h`, `A0h` or `F0h` to `5555h` selects 64 KiB Flash, and
     `B0h` selects 128 KiB; the buffered writes are then replayed into the
     device. Any other write, or a read, selects SRAM and replays the
     buffered writes in order. Chip-erase `10h` is not a classifying command
     because it is valid only after erase setup; accepting it would
     misclassify SRAM traffic.
   - A subsequent bank-select command promotes auto-detected Flash to
     128 KiB.
     Explicitly configured `flash64` never promotes.
   - When DMA3 starts a halfword transfer whose destination is the EEPROM
     window, its complete transfer count decides the capacity before the
     first bit arrives: 9 or 73 selects 512 B, 17 or 81 selects 8 KiB. Other
     counts leave the medium unresolved.
   - While unresolved the medium has no bytes, so an unused unknown
     cartridge writes no save file. A program that sends only a partial
     unlock prefix and exits stays unresolved.

mGBA also starts undecided and classifies from traffic, but tells SRAM from
Flash by the first write's address; ZiGBA requires the full unlock sequence
so an ordinary SRAM write to `5555h` is not mistaken for Flash. open_agb_firm
[describes](https://github.com/profi200/open_agb_firm#save-type-detection) a
similar database-plus-fallback design and the same weakness of signatures in
homebrew and hacks.

The limits are deliberate. The database is small and has no Atmel mapping.
GBATEK notes that EEPROM capacity is hardcoded in software, so inferring it
from DMA length is an emulator convenience, not a hardware query. Even mGBA
[notes](https://github.com/mgba-emu/mgba/blob/0.10.5/README.md#L244) that
its Flash-size detection fails in some cases, so the explicit type remains
the final override. Detection is pinned by ZiGBA's runtime-detection tests in
[`bus_test.zig`](../../../refs/zigba/test/bus_test.zig) and the marker, database
and file-size tests in [`cartridge/save.rs`](../../../systems/gba/src/cartridge/save.rs)
and [`host/host_save.rs`](../../../systems/gba/src/host/host_save.rs).

## Save persistence

The native frontends keep the save next to the ROM (`game.gba` saves to
`game.sav`) unless `--save-file` names another path. An existing file is
loaded and validated as above (native imports are bounded at 2 MiB). On clean
exit the medium's raw bytes are written to a temporary file, synced, and
atomically renamed over the destination; nothing is written while the medium
is unresolved or `none`. Exports hold only the medium's bytes, with no RTC or
device trailer. The web session applies the same selection rules to save
bytes its host supplies. Flags and examples are in ZiGBA's
[`cli.md`](../../../refs/zigba/docs/cli.md); the Rust CLI lives in
[`host/host_save.rs`](../../../systems/gba/src/host/host_save.rs).

## GPIO port

The ROM chip in some cartridges contains a four-bit general-purpose port
mapped into the ROM window
([GBATEK](https://rust-console.github.io/gbatek-gbaonly/#gbacartioportgpio)):

| Address     | Register  | Bits                        |
| ----------- | --------- | --------------------------- |
| `080000C4h` | DATA      | 3–0 pin levels              |
| `080000C6h` | DIRECTION | 3–0: 0 input, 1 output      |
| `080000C8h` | CONTROL   | 0: 0 write-only, 1 readable |

ZiGBA creates the port only when a GPIO device is attached; otherwise these
addresses are plain ROM. Its behavior:

- CONTROL starts at write-only. While it is 0, reads return the ROM bytes
  underneath, as GBATEK describes. While it is 1, DATA, DIRECTION and CONTROL
  read back and their high bytes read 0.
- A DATA write changes only pins configured as outputs. A DATA read returns
  `(output latch & direction) | (device outputs & ~direction)`.
- The registers decode at offset `C4h`–`C9h` of each wait-state window
  (`080000C4h`, `0A0000C4h`, `0C0000C4h`), as NanoBoyAdvance does. GBATEK
  names only `080000C4h`, and mGBA intercepts writes only in the first
  window.
- Halfword stores are the reliable case. GBATEK says byte stores are ignored
  and wonders whether that holds only on the DS; ZiGBA ignores them and
  splits a word store into two halfword writes, low half first.
- Writes cost Game Pak wait states. An HLE `HardReset` does not reset the
  port.

Several devices can share the port; Boktai uses the RTC and the solar sensor
together.

### Detection

GPIO devices cannot be discovered by probing, because a cartridge without
them ignores the writes. `detect_features` in
[`cartridge/mod.rs`](../../../systems/gba/src/cartridge/mod.rs) attaches them from the header
game code, following pinned mGBA's override table:

| Game codes                            | Title                                              | Devices           |
| ------------------------------------- | -------------------------------------------------- | ----------------- |
| `AXV`, `AXP`, `BPE` + `J E P I S D F` | Pokémon Ruby, Sapphire, Emerald                    | RTC               |
| `U3IJ U3IE U3IP U32J U32E U32P U33J`  | Boktai 1–3                                         | RTC, solar sensor |
| `BLJJ BLJK BLVJ`, `BR4J`, `BKAJ`      | Legendz, RockMan EXE 4.5, Sennen Kazoku            | RTC               |
| `RZW` + `J E P`                       | WarioWare: Twisted!                                | gyro, rumble      |
| `V49` + `J E P`                       | Drill Dozer                                        | rumble            |
| `KYG` + `J E P`, `KHPJ`               | Yoshi Topsy-Turvy, Koro Koro Puzzle Happy Panechu! | tilt              |

A ROM with no listed code but an `RTC_V` string anywhere in its image gets an
RTC, because ROM hacks and translations often change the game code but keep
the RTC library. FireRed, LeafGreen and Pokémon Mystery Dungeon stay
RTC-free. NanoBoyAdvance also marks the Golden Sun games (`AGSE`, `AGFE`) as
RTC cartridges while mGBA does not; ZiGBA does not guess. Detection only
proposes a configuration: the host passes the final one to `Machine.Config`
(`rtc_epoch_seconds`, `solar_level`, `gyro_sample`, `rumble`,
`tilt_sample`), and the CLI can force each device on or off
(see [`host/host_save.rs`](../../../systems/gba/src/host/host_save.rs)).

## Real-time clock

The RTC is an S3511-compatible serial chip on three GPIO pins
([GBATEK](https://rust-console.github.io/gbatek-gbaonly/#gbacartrealtimeclockrtc)):

| GPIO bit | Signal | Driven by                                      |
| -------- | ------ | ---------------------------------------------- |
| 0        | SCK    | GBA                                            |
| 1        | SIO    | GBA for commands and writes; RTC for read data |
| 2        | CS     | GBA                                            |
| 3        | unused |                                                |

No manufacturer datasheet or hardware RTC test ROM was available, so the
serial model follows the compatible behavior of mGBA and NanoBoyAdvance and
GBATEK's register description.

### Serial protocol

A pin counts only while it is configured as an output. A transaction begins
with the documented sequence: SCK high with CS low, then CS raised while SCK
stays high. While CS stays high, each SCK low phase samples SIO into the
current command or write bit, and each SCK rising edge advances one bit.
During a read, the rising edge puts the next data bit on SIO, where DATA
shows it until SCK falls, provided GPIO1 is an input. DATA writes without a
rising edge consume nothing. Dropping CS ends the transaction at any point,
and the next command needs a fresh start.

Bits travel least significant first. The command byte has signature `6h` in
bits 3–0, the register in bits 6–4, and bit 7 set for a read. Like
NanoBoyAdvance, ZiGBA also accepts a command sent in reverse bit order; a
byte without the signature ends the transaction.

| Register        | Write | Read  | Payload                        |
| --------------- | ----- | ----- | ------------------------------ |
| 0 force reset   | `06h` | `86h` | none; either direction strobes |
| 1 alarm 1       | `16h` | `96h` | none                           |
| 2 date and time | `26h` | `A6h` | 7 bytes                        |
| 3 force IRQ     | `36h` | `B6h` | none; either direction strobes |
| 4 control       | `46h` | `C6h` | 1 byte                         |
| 5 alarm 2       | `56h` | `D6h` | none                           |
| 6 time          | `66h` | `E6h` | 3 bytes                        |
| 7 free          | `76h` | `F6h` | none                           |

GBATEK says the reserved registers read `FFh` and notes that Boktai writes
one byte to alarm 1; ZiGBA follows both emulators in giving them no payload.

### Calendar and control

Date and time are seven BCD bytes: year, month, day, weekday, hour, minute,
second. The time register is the last three. The whole response is latched
when the command is accepted, so a read that crosses a second boundary stays
consistent. Years `00`–`99` map to 2000–2099, and weekday 0 is Sunday.

The control register:

| Bit | Meaning                             | ZiGBA                                |
| --- | ----------------------------------- | ------------------------------------ |
| 1   | IRQ duty or hold, otherwise unknown | stored                               |
| 3   | per-minute IRQ enable               | stored; no periodic IRQ is generated |
| 5   | unknown                             | stored                               |
| 6   | 0 = 12-hour, 1 = 24-hour            | honored                              |
| 7   | power-off flag, cleared by reading  | read-only; never set                 |

Control starts at `40h` (24-hour mode). That is the mGBA and NanoBoyAdvance
compatibility default, not GBATEK's `82h` battery-failure value; ZiGBA has
no cold-battery configuration. In 12-hour mode the hour reads `00`–`11` in
BCD with the PM flag in bit 7, GBATEK's GBA-specific placement (the DS uses
bit 6). mGBA omits the PM flag and NanoBoyAdvance puts it in bit 6. Writes
also accept `12` as an alias for `00`.

A date-and-time or time write is atomic. All BCD digits and ranges,
including the month's day count, are validated first, and an invalid payload
changes nothing. A valid write rebases the clock to the written value at the
cycle of the payload's last bit. The weekday is independently writable and
kept as an offset from the calendar weekday; a time-only write keeps the
date and weekday. Neither reference emulator implements these writes, so
this is ZiGBA's policy rather than a checked behavior.

### Reset and interrupt strobes

Any access to register 0 resets the chip: control becomes `00h`, and the
calendar becomes year `00`, month `01`, day `01`, weekday 0, `00:00:00`.
GBATEK says reset zeroes the used fields except month and day, which become
`01h`.

Any access to register 3 requests the Game Pak interrupt (`IF` bit 13)
immediately, at the GPIO write that completes the command byte. GBATEK
describes a brief active-low pulse through an inverter; NanoBoyAdvance raises
the interrupt only for a write, and mGBA does not implement it. Collapsing
the pulse into one event is provisional.

### Time source

The clock never reads the host. `rtc_epoch_seconds` in `machine::Config`
gives a Unix time, decoded as UTC, and the calendar advances from it at the
16,777,216 Hz master clock, so identical inputs give identical dates. The
native frontends sample the host clock once at startup for a detected RTC
(`--rtc-epoch` fixes the value, `--no-rtc` removes the RTC), and the web
session receives the epoch from its host. There is no local-time or
time-zone policy, and a written clock is not kept between sessions: it lives
in save states but not in the `.sav` file, which holds only the save medium.

## Sensors and rumble

The sensor devices follow one rule: the core owns the protocol and timing,
the host supplies the physical value, and a fixed input sequence is
deterministic. Hosts set values with `Machine::set_solar_level`,
`Machine::set_gyro_sample` and `Machine::set_tilt`, or schedule them at exact
cycles as `TimedInput` changes. Device state is part of the save state.

### Solar sensor (Boktai)

A photodiode feeds a comparator, and software clocks an 8-bit counter until
a counter-driven DAC crosses the sensor voltage
([GBATEK](https://rust-console.github.io/gbatek-gbaonly/#gbacartsolarsensor)):

| GPIO bit | Signal                     | Direction |
| -------- | -------------------------- | --------- |
| 0        | counter clock              | output    |
| 1        | counter reset, active high | output    |
| 2        | chip select, active low    | output    |
| 3        | comparator flag            | input     |

While chip select is low, a high reset clears the counter and each falling
clock edge increments it. The flag reads 1 once the counter exceeds
`255 - level`. Level 255 is the brightest (the flag rises after one pulse)
and level 0 the darkest (the flag never rises and software times out); with
level 252 it rises on the fourth pulse, as `probe-solar` checks. Higher
counts mean less light: GBATEK reports about `E8h` in darkness and `5xh` at
the top of Boktai's gauge. A level change takes effect immediately, even
mid-conversion.

NanoBoyAdvance increments on the falling clock edge and mGBA on the rising
one; both give one count per complete pulse. ZiGBA takes the falling edge.
The native frontends start a detected sensor at level 156 and the web session
at 128.

### Gyro sensor (WarioWare: Twisted!)

WarioWare: Twisted! reads a Z-axis gyro through the GPIO port and drives its
rumble motor from the same port
([GBATEK](https://rust-console.github.io/gbatek-gbaonly/#gbacartgyrosensor)):

| GPIO bit | Signal           | Direction |
| -------- | ---------------- | --------- |
| 0        | start conversion | output    |
| 1        | serial clock     | output    |
| 2        | serial data      | input     |
| 3        | rumble motor     | output    |

Driving start high loads a 16-bit shift value, four zero bits followed by the
12-bit sample, and presents its MSB; each falling clock edge presents the
next bit. The host supplies the raw 12-bit ADC value. GBATEK's examples run
from about `354h` (fast counter-clockwise) through `6C0h` at rest to `9E3h`
(fast clockwise), and `000h` or `FFFh` usually mean no sensor. Detected
cartridges start at `6C0h`.

GBATEK warns that hardware needs at least three sequential WS0 ARM
instructions between clock changes. ZiGBA charges the Game Pak wait states
but accepts faster toggling: enforcing an analog minimum would need
measurements and could break software that other emulators run.

### Rumble

WarioWare: Twisted! drives its motor directly from GPIO3: high with the pin
as an output is on; low, or the pin as an input, is off because of an
external pulldown
([GBATEK](https://rust-console.github.io/gbatek-gbaonly/#gbacartrumble)). The
line may stay asserted. `Machine::rumble_active()` exposes the result.
GBATEK is unsure whether Drill Dozer uses the same line; mGBA configures it
that way and ZiGBA follows.

### Tilt sensor (Yoshi, Koro Koro Puzzle)

The tilt sensor is not a GPIO device. Its byte registers sit in the upper
half of the save window
([GBATEK](https://rust-console.github.io/gbatek-gbaonly/#gbacarttiltsensor)):

| Address     | Access      | Meaning                    |
| ----------- | ----------- | -------------------------- |
| `0E008000h` | write `55h` | arm a conversion           |
| `0E008100h` | write `AAh` | latch both axes when armed |
| `0E008200h` | read        | X bits 7–0                 |
| `0E008300h` | read        | X bits 11–8; bit 7 ready   |
| `0E008400h` | read        | Y bits 7–0                 |
| `0E008500h` | read        | Y bits 11–8                |

Both axes power on as `FFFh`. The `55h`, `AAh` sequence latches the host's
current sample at once, as in mGBA, and the ready bit always reads 1; no
conversion delay is invented without a measurement. A host change stays
invisible until the next complete command. The registers decode on the low
24 address bits, so unlike the save medium they do not repeat every 64 KiB;
this matches mGBA. All other save-window addresses go to the save medium,
which for these titles is EEPROM. Accesses cost SRAM wait states, and wider
reads replicate the byte.

GBATEK's example cartridge centers near X = `392h`, Y = `3A0h`, with ranges
of about `2AFh`–`477h` and `2C3h`–`480h`. Detected cartridges start at
`3A0h` on both axes. The core reports raw values without a left/right
convention because axis direction depends on how the cartridge sits in the
console.

## Tests and probes

- [`cartridge/save.rs`](../../../systems/gba/src/cartridge/save.rs),
  [`host/host_save.rs`](../../../systems/gba/src/host/host_save.rs),
  [`cartridge/mod.rs`](../../../systems/gba/src/cartridge/mod.rs) and
  [`cartridge/gpio.rs`](../../../systems/gba/src/cartridge/gpio.rs) carry unit tests
  for markers, the database, file sizes, EEPROM byte traffic, Atmel pages,
  feature detection and RTC state validation.
- ZiGBA's [`bus_test.zig`](../../../refs/zigba/test/bus_test.zig) covers runtime detection,
  Flash commands and sector erase, EEPROM polling and capacities, GPIO
  masking and fallback, RTC commands, writes and strobes, and the solar,
  gyro, rumble and tilt protocols.
- The `jsmolka-save-none`, `-sram`, `-flash64` and `-flash128` validators
  run the pinned jsmolka save suite, and `jsmolka-unsafe` covers SRAM
  mirroring.
- The probes under "Cartridge devices" in
  [`probe/README.md`](../../../probe/README.md) (`probe-eeprom`, `probe-rtc`,
  `probe-rtc-write`, `probe-gyro`, `probe-solar`, `probe-tilt`,
  `probe-rumble`) drive each device from bare-metal code; `probe-eeprom`
  also checks the persisted bytes and the auto-detected capacity.

## Open questions

- Flash busy and status behavior per vendor, including the `5Fh` sector-erase
  value and the 30,000-cycle estimate.
- Atmel page-write timing, and the chip's response to misaligned or
  interrupted page sequences.
- EEPROM behavior for CPU-driven traffic, malformed transfers and writes
  issued while busy.
- What halfword and word accesses do on the 8-bit save bus.
- Whether GPIO writes decode in every wait-state window, and what byte and
  word stores do.
- Whether the S3511 accepts reverse-order command bytes, and the payload
  framing of the reserved registers.
- 12-hour encoding of midnight and noon, the function of control bits 1 and
  5, and which control bits are writable.
- Force-IRQ pulse width and polarity, and the per-minute interrupt's phase
  and duty cycle.
- Which RTC state survives console reset, cartridge removal and battery
  failure; ZiGBA also lacks a persistent clock offset and local time.
- Solar threshold calibration, and whether a light change should be latched
  at counter reset.
- Gyro clock-rate limits, tilt conversion latency, and Drill Dozer's rumble
  wiring on a real cartridge.
