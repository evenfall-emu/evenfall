# Cartridge

A GBA cartridge is more than a ROM with the game's code.
Most also carry a chip that keeps save data, and a few carry extra hardware: a real-time clock in the Pokémon games, a light sensor in Boktai, a gyroscope and rumble motor in WarioWare: Twisted!, a tilt sensor in Yoshi Topsy-Turvy.

The hard part for an emulator is that nothing in the cartridge says what it contains.
There are five kinds of save chip, each with its own protocol, and the header has no field for them.
Evenfall combines several clues to work out which one a game expects ([Save-type detection](#save-type-detection)), and never shrinks an existing save on a guess.

Evenfall models each part of the cartridge as a device with its own protocol state.
The core owns every protocol and its timing.
The host owns files, wall-clock time, and the physical value a sensor reports.

| Concern                                            | Code                                                              |
| -------------------------------------------------- | ----------------------------------------------------------------- |
| Address decoding, wait states, EEPROM DMA routing  | [`bus/mod.rs`](../../../systems/gba/src/bus/mod.rs)               |
| Save chips, and the scan for SDK markers           | [`cartridge/save.rs`](../../../systems/gba/src/cartridge/save.rs) |
| Title database, choosing the save type, ZIP import | [`rom_image.rs`](../../../systems/gba/src/rom_image.rs)           |
| Save files for the native frontends                | [`host/host_save.rs`](../../../systems/gba/src/host/host_save.rs) |
| Detecting extra hardware from the header           | [`cartridge/mod.rs`](../../../systems/gba/src/cartridge/mod.rs)   |
| GPIO port, clock, light sensor, gyro, rumble       | [`cartridge/gpio.rs`](../../../systems/gba/src/cartridge/gpio.rs) |
| Tilt sensor                                        | [`cartridge/tilt.rs`](../../../systems/gba/src/cartridge/tilt.rs) |

No public Nintendo specification of the save chips or extra hardware is known.
This page follows GBATEK's [cartridge chapter](https://rust-console.github.io/gbatek-gbaonly/#gbacartridges), cross-checked against mGBA 0.10.5 and NanoBoyAdvance ([`refs/README.md`](../../../refs/README.md) records the versions).
Where they disagree, the page says which one Evenfall follows.

## A few terms

- **SRAM, Flash, EEPROM.** The three technologies GBA save chips use.
  SRAM is plain battery-backed memory, Flash is written with commands and erased in blocks, and EEPROM is a small chip read and written one bit at a time.
- **GPIO port.** Four general-purpose pins built into some ROM chips, which the clock and sensors are wired to.
- **Game code.** Four letters in the header that identify a game and its region, for example `AXVE` for Pokémon Ruby (US).
- **SDK marker.** A text string such as `FLASH1M_V103` that Nintendo's save library leaves in the ROM.

## Header and ROM bus

The first 192 bytes of the ROM are the header ([GBATEK](https://rust-console.github.io/gbatek-gbaonly/#gbacartridgeheader)):

| Offset        | Size | Field                                            |
| ------------- | ---- | ------------------------------------------------ |
| `000h`        | 4    | ARM branch to the entry point                    |
| `004h`        | 156  | Nintendo logo, compared by the BIOS              |
| `0A0h`        | 12   | title                                            |
| `0ACh`        | 4    | game code                                        |
| `0B0h`        | 2    | maker code                                       |
| `0B2h`        | 1    | fixed value `96h`                                |
| `0B3h`–`0BCh` | 10   | unit code, device type, reserved, version        |
| `0BDh`        | 1    | checksum: `-(sum of 0A0h..0BCh) - 19h`, low byte |
| `0BEh`        | 2    | reserved                                         |

Evenfall doesn't check the header.
HLE boot starts the game at `08000000h` without the logo and checksum checks the real BIOS makes.
With a real BIOS image, those checks run as ordinary BIOS code.
The core reads only the game code, to find extra hardware and to look up the save-type database.

The ROM sits on a 16-bit bus that carries addresses and data on the same wires.
It appears in three 32 MiB windows at `08000000h`, `0A000000h` and `0C000000h`, each with its own wait states.
The frontends and the web session accept ROMs up to 32 MiB.
Writes to ROM cost wait states and change nothing, and reads past the end of the ROM return a pattern made from the address ([`memory.md`](memory.md)).
Two devices sit inside the ROM windows: the GPIO registers at offset `C4h`–`C9h`, and for EEPROM cartridges, part of the `0Dxxxxxxh` range.

The save window covers `0E000000h`–`0FFFFFFFh`.
It is only 8 bits wide.
The chip sees a 16-bit offset, so the save repeats every 64 KiB, and SRAM also repeats every 32 KiB.
A 16- or 32-bit read returns the byte copied into every lane, and a 16- or 32-bit write stores the one byte the low address bits select.
Each access costs the SRAM wait states set in `WAITCNT` bits 1–0.

## Save chips

`cartridge::save::Type`, exposed as `SaveType` by `Machine`, selects the save chip:

| `SaveType`      | CLI name        | Size             | Chip                                     |
| --------------- | --------------- | ---------------- | ---------------------------------------- |
| `Auto`          | `auto`          | 0 until detected | decided while the game runs, below       |
| `None`          | `none`          | 0                | no chip: reads `FFh`, writes are ignored |
| `Sram`          | `sram`          | 32 KiB           | SRAM or FRAM                             |
| `Flash64k`      | `flash64`       | 64 KiB           | Flash, ID `1CC2h`                        |
| `Flash64kAtmel` | `flash64-atmel` | 64 KiB           | Atmel Flash, ID `3D1Fh`                  |
| `Flash128k`     | `flash128`      | 128 KiB          | Flash, ID `09C2h`                        |
| `Eeprom512`     | `eeprom512`     | 512 B            | 4 Kbit EEPROM                            |
| `Eeprom8k`      | `eeprom8k`      | 8 KiB            | 64 Kbit EEPROM                           |

`Machine::save_type()` and `Machine::save_data()` expose the detected type and the bytes.
The chip's bytes and its command state are part of the save state ([`design/save-states.md`](../../design/save-states.md)), so a state saved in the middle of a command resumes at the same point.
An HLE `HardReset` leaves the save chip and all extra hardware alone.

### SRAM and FRAM

SRAM and FRAM look the same to software: 32 KiB at `0E000000h`–`0E007FFFh`, read and written one byte at a time, with no commands and no busy time ([GBATEK](https://rust-console.github.io/gbatek-gbaonly/#gbacartbackupsramfram)).
FRAM only differs in how it keeps its data, so Evenfall treats them as one chip.
A new SRAM save is filled with `FFh`.

### Flash 64 KiB and 128 KiB

Flash occupies `0E000000h`–`0E00FFFFh`.
Programs drive it with byte commands.
Each command starts with an unlock sequence, so a stray write can't change the save.
With `S = 0E000000h`, the unlock sequence is `AAh` to `S+5555h`, then `55h` to `S+2AAAh` ([GBATEK](https://rust-console.github.io/gbatek-gbaonly/#gbacartbackupflashrom)):

| Operation     | Sequence after the unlock                        | What Evenfall does                                                            |
| ------------- | ------------------------------------------------ | ----------------------------------------------------------------------------- |
| Enter ID mode | `90h` to `S+5555h`                               | offsets 0 and 1 read manufacturer and device, other offsets read `FFh`        |
| Exit ID mode  | `F0h` to `S+5555h`                               | normal reads resume                                                           |
| Write byte    | `A0h` to `S+5555h`, then the byte to its address | the byte is ANDed into the cell, so writing can only clear bits               |
| Erase chip    | `80h` to `S+5555h`, unlock, `10h` to `S+5555h`   | every byte (both banks on 128 KiB) becomes `FFh`, readable at once            |
| Erase sector  | `80h` to `S+5555h`, unlock, `30h` to `S+n000h`   | the 4 KiB sector becomes `FFh`, and reads `5Fh` for 30,000 cycles first       |
| Select bank   | `B0h` to `S+5555h`, then the bank to `S+0`       | 128 KiB only: bit 0 picks the 64 KiB bank for reads, writes and sector erases |

A write that doesn't continue the current sequence returns to idle.
A fixed 64 KiB chip ignores the bank command.

GBATEK lists these chip IDs (device in the high byte, manufacturer in the low byte): `D4BFh` SST, `1CC2h` Macronix, `1B32h` Panasonic and `3D1Fh` Atmel for 64 KiB, and `1362h` Sanyo and `09C2h` Macronix for 128 KiB.
Evenfall reports the Macronix IDs.

Nobody has measured how long each vendor's chip takes to write or erase.
GBATEK only lists timeouts, and mGBA's [timing notes](https://github.com/mgba-emu/mgba/blob/0.10.5/src/gba/savedata.c#L21-L31) record erase times from about 2,000 to 30,000 cycles depending on the chip, calling its constants estimates.
Evenfall uses mGBA's 30,000-cycle estimate, and like mGBA only makes sector erase take time: the erased sector reads `5Fh` until then, while other sectors read normally.
Byte writes and chip erase are readable at once, which matches the [jsmolka save tests](https://github.com/jsmolka/gba-tests/tree/master/save).
The `5Fh` value and the delay are Evenfall's choices, not a claim that every chip behaves this way.

Evenfall doesn't model vendor-specific status polling, Macronix's `F0h` command after a timeout, or GBATEK's note that Flash can only be read by code running outside the ROM.

### Atmel 64 KiB

The Atmel chip (`3D1Fh`) writes whole 128-byte pages instead of single bytes.
Evenfall only uses it when asked (`flash64-atmel`), because no ROM marker identifies the vendor and the title database has no Atmel entry.
It differs from other Flash in these ways:

- ID mode returns manufacturer `1Fh`, device `3Dh`.
- `A0h` starts a page write.
  The first byte must be at a 128-byte boundary, otherwise the command is dropped.
  That byte erases the page to `FFh`, then each of the 128 bytes in sequence replaces its cell instead of being ANDed into it.
  A byte out of sequence aborts the page.
- Sector erase does nothing.
  Chip erase works.
- A half-written page survives a save state, so the write can resume.

No busy time is modeled.

### EEPROM 512 B and 8 KiB

An EEPROM is a small serial chip, read and written one bit at a time on bit 0 of the ROM data bus ([GBATEK](https://rust-console.github.io/gbatek-gbaonly/#gbacartbackupeeprom)).
It answers anywhere in `0D000000h`–`0DFFFFFFh` when the ROM is 16 MiB or smaller, and only at `0DFFFF00h`–`0DFFFFFFh` for a larger ROM.
Halfword accesses there reach the EEPROM, and everything else in the range is ordinary ROM.

Data moves in 8-byte blocks.
Games send each bit as bit 0 of one 16-bit DMA3 transfer, most significant bit first:

| Transfer     | 512 B   | 8 KiB   | Stream                                 |
| ------------ | ------- | ------- | -------------------------------------- |
| Read request | 9 bits  | 17 bits | `11`, 6- or 14-bit block address, `0`  |
| Read data    | 68 bits | 68 bits | four dummy `0` bits, then 64 data bits |
| Write        | 73 bits | 81 bits | `10`, block address, 64 data bits, `0` |

The 8 KiB chip uses the low 10 address bits (1,024 blocks).
GBATEK says the upper four should be zero, and Evenfall drops a command in which they aren't.
A command whose last bit is 1 is dropped too.
A write stores all eight bytes at its last bit and keeps the chip busy for 108,368 cycles, GBATEK's figure of about 6.5 ms.

GBATEK reports that sending the bits with the CPU's `LDRH` and `STRH` doesn't work on hardware, so Evenfall only takes input from DMA3 halfword writes.
A CPU store to the window goes to the ROM and is ignored.
Reads are different, because games check for completion with ordinary `LDRH`.
Any halfword read of the window returns the next bit of a pending read, or with no read in progress, the status bit (0 while busy, 1 when ready).
The upper 15 bits read zero.
EEPROM accesses cost the third ROM window's wait states.

When the save chip is an EEPROM, accesses to the 8-bit save window read `FFh`, ignore writes, and don't disturb a transfer in progress.
That is Evenfall's choice for invalid accesses, not a claim about what they do on hardware.

## Save-type detection

Nothing in the header names the save chip.
Nintendo's save library leaves word-aligned strings in the ROM (`EEPROM_Vnnn`, `SRAM_Vnnn`, `FLASH_Vnnn`, `FLASH512_Vnnn`, `FLASH1M_Vnnn`), but homebrew and ROM hacks may lack them, carry stale ones, or carry several.
`EEPROM_Vnnn` doesn't say the capacity either ([GBATEK, backup IDs](https://rust-console.github.io/gbatek-gbaonly/#gbacartbackupids)).

Evenfall therefore goes through several sources of evidence, most specific first:

1. **An explicit type.** A configured type other than `auto` is used as given.
   An existing save file must be at least that type's size.
   `none` ignores any file.
2. **The title database.** `SAVE_OVERRIDES` in [`rom_image.rs`](../../../systems/gba/src/rom_image.rs) lists titles on which mGBA 0.10.5 and NanoBoyAdvance agree:
   - three full-ROM checksums of Pokémon Ruby (`AXVE`) revisions, which select `flash128`,
   - the game codes `A2YE` (Top Gun: Combat Zones, no save), `AGFE` (Golden Sun: The Lost Age, `flash64`) and `RZWE` (WarioWare: Twisted!, `sram`).

   A checksum entry must match exactly, and a game-code entry is the fallback.
   The database comes before SDK markers, so a listed title keeps its type even if it carries a misleading marker.
   NanoBoyAdvance's EEPROM capacities are marked uncertain in its own source, so they aren't imported.

3. **SDK markers.** `detect_type` in [`cartridge/save.rs`](../../../systems/gba/src/cartridge/save.rs) accepts a marker only at a four-byte-aligned offset followed by three digits.
   `SRAM_V`, `FLASH_V`, `FLASH512_V` and `FLASH1M_V` choose their type, while `EEPROM_V` leaves the capacity open.
   Markers of different types are an error that needs an explicit type.
4. **An existing save file.** A file of exactly 512 B, 8 KiB, 32 KiB, 64 KiB or 128 KiB must agree with the database or marker if either applies, and otherwise its size picks the type.
   64 KiB means common Flash, never Atmel.
   With an `EEPROM_V` marker, the exact file size picks the capacity.
   A known type also accepts a longer file and ignores the extra bytes.
   Without a known type, a longer file is ambiguous and needs an explicit type.
   A disagreement fails before emulation starts.
5. **Watching the game.** Anything still undecided starts as `auto`, and the game's first unambiguous access to the save decides it:
   - Reads return `FFh` and decide nothing.
   - An ordinary byte write selects 32 KiB SRAM.
   - The Flash unlock sequence (`AAh` to `5555h`, then `55h` to `2AAAh`) is held back.
     A third write of `80h`, `90h`, `A0h` or `F0h` to `5555h` selects 64 KiB Flash, and `B0h` selects 128 KiB.
     The held-back writes are then replayed into the chip.
     Any other write, or a read, selects SRAM and replays them in order.
   - A later bank-select command upgrades detected Flash to 128 KiB.
     An explicitly configured `flash64` never changes.
   - When DMA3 starts a halfword transfer to the EEPROM window, its length decides the capacity before the first bit arrives: 9 or 73 bits mean 512 B, and 17 or 81 mean 8 KiB.
   - Until then the chip has no bytes, so a game that never touches its save writes no save file.

mGBA also starts undecided and watches the game, but tells SRAM from Flash by the first write's address.
Evenfall waits for the full unlock sequence, so an ordinary SRAM write to `5555h` isn't mistaken for Flash.
open_agb_firm [describes](https://github.com/profi200/open_agb_firm#save-type-detection) a similar database-plus-fallback design, and the same weakness of markers in homebrew and hacks.

Detection can still guess wrong, and the explicit type is always the final override.
The database is small and has no Atmel entry.
GBATEK notes that games hard-code their EEPROM size, so reading it from the DMA length is an emulator convenience.
Even mGBA [notes](https://github.com/mgba-emu/mgba/blob/0.10.5/README.md#L244) that its Flash size detection sometimes fails.

## Save files

The native frontends keep the save next to the ROM (`game.gba` saves to `game.sav`) unless `--save-file` names another path.
An existing file is loaded and checked as above, up to 2 MiB.
On a clean exit, the chip's bytes are written to a temporary file, synced to disk, and renamed over the save, so a crash never leaves half a save.
Nothing is written while the type is still undecided or `none`.
The file holds only the chip's bytes, with no clock or device data.
The web session applies the same rules to save bytes its host supplies.
Flags and examples are in [`cli.md`](../../cli.md#saves-and-states), and the code is in [`host/host_save.rs`](../../../systems/gba/src/host/host_save.rs).

## GPIO port

Some cartridges' ROM chips contain four general-purpose pins, mapped into the ROM window ([GBATEK](https://rust-console.github.io/gbatek-gbaonly/#gbacartioportgpio)):

| Address     | Register  | Bits                        |
| ----------- | --------- | --------------------------- |
| `080000C4h` | DATA      | 3–0 pin levels              |
| `080000C6h` | DIRECTION | 3–0: 0 input, 1 output      |
| `080000C8h` | CONTROL   | 0: 0 write-only, 1 readable |

Evenfall creates the port only when a device is attached to it.
Otherwise these addresses are plain ROM.

- CONTROL starts at write-only.
  While it is 0, reads return the ROM bytes underneath, as GBATEK describes.
  While it is 1, DATA, DIRECTION and CONTROL read back, with their high bytes at 0.
- A DATA write only changes pins set as outputs.
  A DATA read returns `(output latch & direction) | (device outputs & ~direction)`.
- The registers answer at offset `C4h`–`C9h` of all three ROM windows, as in NanoBoyAdvance.
  GBATEK only names `080000C4h`, and mGBA only catches writes in the first window.
- Halfword stores are the reliable case.
  GBATEK says byte stores are ignored and wonders whether that is only true on the DS.
  Evenfall ignores them, and splits a word store into two halfword writes, low half first.
- Writes cost the cartridge wait states.
  An HLE `HardReset` doesn't reset the port.

Several devices can share the port.
Boktai uses the clock and the light sensor together.

### Detecting extra hardware

Writing to the GPIO port can't tell whether devices are there, because a cartridge without them ignores the writes.
`detect_features` in [`cartridge/mod.rs`](../../../systems/gba/src/cartridge/mod.rs) attaches them from the game code, following mGBA's list:

| Game codes                            | Title                                              | Devices             |
| ------------------------------------- | -------------------------------------------------- | ------------------- |
| `AXV`, `AXP`, `BPE` + `J E P I S D F` | Pokémon Ruby, Sapphire, Emerald                    | clock               |
| `U3IJ U3IE U3IP U32J U32E U32P U33J`  | Boktai 1–3                                         | clock, light sensor |
| `BLJJ BLJK BLVJ`, `BR4J`, `BKAJ`      | Legendz, RockMan EXE 4.5, Sennen Kazoku            | clock               |
| `RZW` + `J E P`                       | WarioWare: Twisted!                                | gyro, rumble        |
| `V49` + `J E P`                       | Drill Dozer                                        | rumble              |
| `KYG` + `J E P`, `KHPJ`               | Yoshi Topsy-Turvy, Koro Koro Puzzle Happy Panechu! | tilt                |

A ROM with no listed code but an `RTC_V` string anywhere gets a clock, because ROM hacks and translations often change the game code but keep the clock library.
FireRed, LeafGreen and Pokémon Mystery Dungeon have no clock.
NanoBoyAdvance also gives the Golden Sun games a clock, mGBA doesn't, and Evenfall doesn't guess.

Detection only proposes a configuration.
The host passes the final one in `machine::Config` (`rtc_epoch_seconds`, `solar_level`, `gyro_sample`, `rumble`, `tilt_sample`), and the CLI can force each device on or off ([`host/host_save.rs`](../../../systems/gba/src/host/host_save.rs)).

## Real-time clock

The clock chip is an S3511-compatible serial chip on three GPIO pins ([GBATEK](https://rust-console.github.io/gbatek-gbaonly/#gbacartrealtimeclockrtc)).
Pokémon uses it for berries and time-based events.

| GPIO bit | Signal | Driven by                                            |
| -------- | ------ | ---------------------------------------------------- |
| 0        | SCK    | the GBA                                              |
| 1        | SIO    | the GBA for commands and writes, the clock for reads |
| 2        | CS     | the GBA                                              |
| 3        | unused |                                                      |

No datasheet or hardware clock test ROM was available, so the serial model follows what mGBA and NanoBoyAdvance agree on, and GBATEK's register description.

### Serial protocol

A pin only counts while it is set as an output.
A transaction starts with SCK high and CS low, then CS raised while SCK stays high.
While CS stays high, each low phase of SCK samples SIO into the current command or data bit, and each rising edge of SCK moves to the next bit.
During a read, the rising edge puts the next data bit on SIO, where DATA shows it until SCK falls, as long as pin 1 is an input.
Dropping CS ends the transaction at any point.

Bits travel least significant first.
The command byte has `6h` in bits 3–0, the register in bits 6–4, and bit 7 set for a read.
Like NanoBoyAdvance, Evenfall also accepts a command sent in reverse bit order.
A byte without the `6h` signature ends the transaction.

| Register        | Write | Read  | Payload                       |
| --------------- | ----- | ----- | ----------------------------- |
| 0 force reset   | `06h` | `86h` | none, either direction resets |
| 1 alarm 1       | `16h` | `96h` | none                          |
| 2 date and time | `26h` | `A6h` | 7 bytes                       |
| 3 force IRQ     | `36h` | `B6h` | none, either direction fires  |
| 4 control       | `46h` | `C6h` | 1 byte                        |
| 5 alarm 2       | `56h` | `D6h` | none                          |
| 6 time          | `66h` | `E6h` | 3 bytes                       |
| 7 free          | `76h` | `F6h` | none                          |

GBATEK says the reserved registers read `FFh`, and notes that Boktai writes one byte to alarm 1.
Evenfall follows both emulators in giving them no payload.

### Calendar and control

The date and time are seven BCD bytes: year, month, day, weekday, hour, minute, second.
The time register is the last three.
The whole answer is captured when the command is accepted, so a read that crosses a second boundary stays consistent.
Years `00`–`99` mean 2000–2099, and weekday 0 is Sunday.

The control register:

| Bit | Meaning                            | In Evenfall                           |
| --- | ---------------------------------- | ------------------------------------- |
| 1   | interrupt duty or hold, uncertain  | stored                                |
| 3   | per-minute interrupt enable        | stored, but no interrupt is generated |
| 5   | unknown                            | stored                                |
| 6   | 0 = 12-hour, 1 = 24-hour           | honored                               |
| 7   | power-off flag, cleared by reading | read-only, never set                  |

Control starts at `40h`, 24-hour mode.
That is mGBA's and NanoBoyAdvance's default, not GBATEK's `82h` value after a dead battery, since Evenfall has no dead-battery setting.
In 12-hour mode the hour reads `00`–`11` in BCD with the PM flag in bit 7, where GBATEK puts it on the GBA (the DS uses bit 6).
mGBA leaves out the PM flag and NanoBoyAdvance puts it in bit 6.
Writes also accept `12` for `00`.

Writing the date and time, or just the time, is all or nothing.
Every digit and range is checked first, including the number of days in the month, and an invalid write changes nothing.
A valid write sets the clock to the written value at the cycle of its last bit.
The weekday can be written independently and is kept as an offset from the real weekday, and a time-only write keeps the date and weekday.
Neither mGBA nor NanoBoyAdvance implements clock writes, so this is Evenfall's own design.

### Reset and interrupt

Any access to register 0 resets the chip.
Control becomes `00h` and the calendar becomes 2000-01-01, weekday 0, `00:00:00`.
GBATEK says reset zeroes the fields except month and day, which become `01h`.

Any access to register 3 raises the Game Pak interrupt (`IF` bit 13) at once, when the command byte completes.
GBATEK describes a short pulse, NanoBoyAdvance raises it only for a write, and mGBA doesn't implement it.
Treating the pulse as one event is provisional.

### Where the time comes from

The clock never reads the host's time.
`rtc_epoch_seconds` in `machine::Config` gives a Unix time, read as UTC, and the calendar advances from it with the 16,777,216 Hz master clock.
The same inputs therefore always give the same dates.

The native frontends read the host clock once at startup for a detected clock chip.
`--rtc-epoch` fixes the value and `--no-rtc` removes the chip.
The web session receives the time from its host.
There is no time-zone handling, and a clock the game sets isn't kept between sessions: it is in save states, but not in the `.sav` file, which only holds the save chip.

## Sensors and rumble

The sensors follow one rule: the core owns the protocol and timing, the host supplies the physical value, and a fixed input sequence always gives the same result.
Hosts set values with `Machine::set_solar_level`, `Machine::set_gyro_sample` and `Machine::set_tilt`, or schedule them at exact cycles as `TimedInput` changes.
Sensor state is part of the save state.

### Light sensor (Boktai)

Boktai's cartridge has a light sensor, and the game's vampire-hunting hero charges his weapon with sunlight.
The sensor works like this: software clocks an 8-bit counter that drives a voltage up, until a comparator says the voltage passed the sensor's ([GBATEK](https://rust-console.github.io/gbatek-gbaonly/#gbacartsolarsensor)):

| GPIO bit | Signal                     | Direction |
| -------- | -------------------------- | --------- |
| 0        | counter clock              | output    |
| 1        | counter reset, active high | output    |
| 2        | chip select, active low    | output    |
| 3        | comparator flag            | input     |

While chip select is low, a high reset clears the counter, and each falling edge of the clock adds one.
The flag reads 1 once the counter exceeds `255 - level`.
Level 255 is the brightest (the flag rises after one pulse), and level 0 the darkest (it never rises and the game times out).
Higher counts therefore mean less light: GBATEK reports about `E8h` in the dark and `5xh` at the top of Boktai's gauge.
A level change takes effect at once, even in the middle of a measurement.

NanoBoyAdvance counts on the falling clock edge and mGBA on the rising one, and both count one per pulse.
Evenfall uses the falling edge.
The native frontends start a detected sensor at level 156 and the web session at 128.

### Gyro sensor (WarioWare: Twisted!)

WarioWare: Twisted! is played by turning the console.
It reads a one-axis gyroscope through the GPIO port, and drives its rumble motor from the same port ([GBATEK](https://rust-console.github.io/gbatek-gbaonly/#gbacartgyrosensor)):

| GPIO bit | Signal           | Direction |
| -------- | ---------------- | --------- |
| 0        | start conversion | output    |
| 1        | serial clock     | output    |
| 2        | serial data      | input     |
| 3        | rumble motor     | output    |

Setting start high loads 16 bits (four zero bits then the 12-bit sample) and shows the first.
Each falling clock edge shows the next bit.
The host supplies the raw 12-bit value.
GBATEK's examples run from about `354h` (turning fast counter-clockwise) through `6C0h` at rest to `9E3h` (fast clockwise), and `000h` or `FFFh` usually mean no sensor.
Detected cartridges start at `6C0h`.

GBATEK warns that the hardware needs at least three ARM instructions between clock changes.
Evenfall charges the cartridge wait states but accepts faster toggling, because enforcing an analog limit would need measurements and could break software other emulators run.

### Rumble

WarioWare: Twisted! drives its motor straight from pin 3.
The motor runs while the pin is an output and high, and stops when it is low or an input ([GBATEK](https://rust-console.github.io/gbatek-gbaonly/#gbacartrumble)).
`Machine::rumble_active()` tells the host.
GBATEK isn't sure Drill Dozer uses the same pin, but mGBA sets it up that way and Evenfall follows.

### Tilt sensor (Yoshi, Koro Koro Puzzle)

The tilt sensor isn't on the GPIO port.
Its byte registers sit in the upper half of the save window ([GBATEK](https://rust-console.github.io/gbatek-gbaonly/#gbacarttiltsensor)):

| Address     | Access      | Meaning                    |
| ----------- | ----------- | -------------------------- |
| `0E008000h` | write `55h` | arm a measurement          |
| `0E008100h` | write `AAh` | capture both axes if armed |
| `0E008200h` | read        | X bits 7–0                 |
| `0E008300h` | read        | X bits 11–8, bit 7 ready   |
| `0E008400h` | read        | Y bits 7–0                 |
| `0E008500h` | read        | Y bits 11–8                |

Both axes start at `FFFh`.
The `55h`, `AAh` sequence captures the host's current sample at once, as in mGBA, and the ready bit always reads 1.
Evenfall doesn't invent a measurement delay without a measurement.
A change from the host shows only after the next complete sequence.

The registers decode on the low 24 address bits, so unlike the save chip they don't repeat every 64 KiB, as in mGBA.
Every other save-window address goes to the save chip, which is an EEPROM for these games.
Accesses cost the SRAM wait states, and wider reads repeat the byte.

GBATEK's example cartridge rests near X = `392h`, Y = `3A0h`, with ranges of about `2AFh`–`477h` and `2C3h`–`480h`.
Detected cartridges start at `3A0h` on both axes.
The core reports raw values with no left or right convention, because the direction depends on how the cartridge sits in the console.

## How it is checked

- Unit tests in [`cartridge/save.rs`](../../../systems/gba/src/cartridge/save.rs), [`host/host_save.rs`](../../../systems/gba/src/host/host_save.rs), [`cartridge/mod.rs`](../../../systems/gba/src/cartridge/mod.rs) and [`cartridge/gpio.rs`](../../../systems/gba/src/cartridge/gpio.rs) cover markers, the database, file sizes, EEPROM traffic in the byte window, Atmel pages (starting from zeroed cells, to tell erase-and-replace apart from ANDing), hardware detection and clock state checks.
- [`tests/bus.rs`](../../../systems/gba/tests/bus.rs) covers detection while the game runs, Flash commands and sector erase, EEPROM polling and sizes, the GPIO port, clock commands, writes and interrupts, and the light, gyro, rumble and tilt protocols.
- The `jsmolka-save-none`, `-sram`, `-flash64` and `-flash128` validators run the jsmolka save tests, and `jsmolka-unsafe` covers SRAM mirroring.
- The probes `probe-eeprom`, `probe-rtc`, `probe-rtc-write`, `probe-gyro`, `probe-solar`, `probe-tilt` and `probe-rumble` drive each device from bare-metal code ([`probe/README.md`](../../../probe/README.md)).
  `probe-eeprom` also checks the saved bytes and the detected size, and `probe-solar` that level 252 raises the flag on the fourth pulse.

## Open questions

- How each Flash vendor's chip reports busy and status, including the `5Fh` value and the 30,000-cycle estimate.
- Atmel page-write timing, and what the chip does with misaligned or interrupted pages.
- What EEPROM does with CPU-driven traffic, malformed transfers and writes while busy.
- What 16- and 32-bit accesses do on the 8-bit save bus.
- Whether GPIO writes decode in every ROM window, and what byte and word stores do.
- Whether the clock chip accepts reverse-order commands, and what the reserved registers carry.
- How 12-hour mode encodes midnight and noon, what control bits 1 and 5 do, and which control bits can be written.
- The length and polarity of the forced interrupt, and the timing of the per-minute interrupt.
- Which clock state survives a console reset, removing the cartridge, and a dead battery.
  Evenfall also lacks a persistent clock offset and local time.
- How the light sensor is calibrated, and whether a light change should wait for a counter reset.
- Limits on the gyro's clock rate, how long a tilt measurement takes, and how Drill Dozer's rumble is wired on a real cartridge.
