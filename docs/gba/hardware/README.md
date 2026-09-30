# Hardware reference

These pages describe how the Game Boy Advance behaves, as far as public
evidence establishes it, and how Evenfall models it. They are written for contributors and for anyone
implementing a GBA emulator.

| Page                                                   | Covers                                                                                                            |
| ------------------------------------------------------ | ----------------------------------------------------------------------------------------------------------------- |
| [`cpu.md`](cpu.md)                                     | the ARM7TDMI: instruction sets, modes, pipeline, cycle classes, exceptions, undefined and unpredictable encodings |
| [`memory.md`](memory.md)                               | the memory map, bus widths and mirrors, wait states, Game Pak prefetch, open bus, BIOS protection, I/O registers  |
| [`video.md`](video.md)                                 | LCD timing, background modes, objects, windows, blending, mosaic, mid-scanline effects                            |
| [`timers-dma-interrupts.md`](timers-dma-interrupts.md) | timers and prescalers, DMA, interrupts, HALT and STOP                                                             |
| [`audio.md`](audio.md)                                 | PSG channels, Direct Sound FIFOs, mixing and the output stream                                                    |
| [`serial.md`](serial.md)                               | serial modes, link cables, multiplayer, UART and JoyBus                                                           |
| [`cartridge.md`](cartridge.md)                         | the cartridge header, save media and save-type detection, GPIO, RTC and sensors                                   |
| [`bios.md`](bios.md)                                   | external and HLE BIOS, BIOS services, MultiBoot, the sound driver, the legal boundary                             |

## Sources

Nintendo has not published a programming manual for the GBA. Each claim here
names its source. When sources disagree, the higher one wins:

- **Arm's manuals** decide CPU behavior: the
  [ARM7TDMI Technical Reference Manual (DDI 0029G)](https://documentation-service.arm.com/static/5e8e1323fd977155116a3129),
  its later revision
  [DDI 0210C](https://documentation-service.arm.com/static/5f4786a179ff4c392c0ff819),
  and the
  [ARM Architecture Reference Manual (DDI 0100I)](https://documentation-service.arm.com/static/5f8dacc8f86e16515cdb865a).
- **Hardware measurements** and public Nintendo patents rank next.
- **[GBATEK](https://problemkaputt.de/gbatek.htm)**, Martin Korth's
  reverse-engineered reference, read through the
  [GBA-only edition](https://rust-console.github.io/gbatek-gbaonly/). It is
  unofficial and marks its own uncertainties; they are kept, not smoothed
  over.
- **Recorded hardware results** from test suites, notably the
  [mGBA test suite](https://github.com/mgba-emu/suite) and
  [NanoBoyAdvance's hardware tests](https://github.com/nba-emu/hw-test).
- **Emulators** at pinned revisions, [mGBA 0.10.5](https://github.com/mgba-emu/mgba/tree/0.10.5)
  and [NanoBoyAdvance](https://github.com/nba-emu/NanoBoyAdvance), as
  comparisons. Where they disagree with each other or with GBATEK, the page
  says which behavior Evenfall follows and why.

Exact revisions of every external reference are listed in
[`refs/README.md`](../../../refs/README.md).

## Conventions

- Addresses and register values are hexadecimal, written `04000200h` or
  `0x04000200`.
- Times are in master-clock cycles (16,777,216 Hz) unless stated otherwise.
- The Rust modules linked from these pages own the behavior they describe.
- Each page ends with **open questions**: behavior no available source
  settles, and the assumption Evenfall makes until a hardware test does. They are
  the best starting points for anyone with a GBA and a way to measure it.
