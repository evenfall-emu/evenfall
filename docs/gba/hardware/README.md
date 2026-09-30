# Hardware reference

These pages explain how the Game Boy Advance works and how Evenfall models it.
They assume you can read code, not that you know the GBA.
Each page starts with what a part of the console does and why games care, then goes into the details an emulator has to get right.

## The machine in one picture

```text
             ┌──────────────┐
             │ ARM7TDMI CPU │
             └──────┬───────┘
                    │ every fetch, load and store
┌───────┐    ┌──────┴───────────────────────────────────────────┐
│  DMA  │────│                      Bus                         │
└───────┘    └─┬───────┬────────┬────────┬────────┬─────────┬───┘
               │       │        │        │        │         │
            BIOS    RAM      Video    Sound   Timers,    Cartridge:
            ROM   (IWRAM,   (LCD,     (PSG,   serial,    ROM, save chip,
                   EWRAM)   VRAM,    Direct   keypad,    clock, sensors
                            OAM,     Sound)   interrupts
                           palette)
```

The CPU runs the game.
Everything it touches goes through one bus, which decides which chip answers and how many cycles the access takes.
The video chip draws the picture line by line from video memory, the sound chip mixes synthesized tones with sample playback, and timers, DMA and interrupts tie them together.
A cartridge brings the game's code and, often, a save chip or a clock.

All of it runs from one 16.78 MHz master clock.
Evenfall counts time in those cycles, and every device sees events at the exact cycle the hardware would produce them.
That is what makes the emulator deterministic: the same inputs always give the same pixels, samples and saves.

## Pages

| Page                                                   | Covers                                                                                       |
| ------------------------------------------------------ | -------------------------------------------------------------------------------------------- |
| [`cpu.md`](cpu.md)                                     | the ARM7TDMI: its two instruction sets, modes, pipeline, timing, exceptions and invalid code |
| [`memory.md`](memory.md)                               | the memory map, wait states, the cartridge prefetch buffer, open bus and BIOS protection     |
| [`video.md`](video.md)                                 | display timing, backgrounds, sprites, windows, blending, mosaic and mid-line effects         |
| [`timers-dma-interrupts.md`](timers-dma-interrupts.md) | timers, DMA, interrupts, and the HALT and STOP power states                                  |
| [`audio.md`](audio.md)                                 | the Game Boy sound channels, sample playback, mixing and the output stream                   |
| [`serial.md`](serial.md)                               | the link port: link cables, multiplayer, UART and JoyBus                                     |
| [`cartridge.md`](cartridge.md)                         | the cartridge header, save chips and how to detect them, the clock and sensors               |
| [`bios.md`](bios.md)                                   | the real BIOS and Evenfall's replacement, BIOS services, MultiBoot and the sound driver      |

## Sources

Nintendo never published a programming manual for the GBA.
Each claim on these pages names its source, and when sources disagree, the higher one wins:

1. **Arm's manuals** decide CPU behavior: the [ARM7TDMI Technical Reference Manual (DDI 0029G)](https://documentation-service.arm.com/static/5e8e1323fd977155116a3129), its later revision [DDI 0210C](https://documentation-service.arm.com/static/5f4786a179ff4c392c0ff819), and the [ARM Architecture Reference Manual (DDI 0100I)](https://documentation-service.arm.com/static/5f8dacc8f86e16515cdb865a).
2. **Hardware measurements** and public Nintendo patents.
3. **[GBATEK](https://problemkaputt.de/gbatek.htm)**, Martin Korth's reverse-engineered reference, read through the [GBA-only edition](https://rust-console.github.io/gbatek-gbaonly/).
   It is unofficial and marks its own uncertainties, and these pages keep them.
4. **Recorded hardware results** from test suites, notably the [mGBA test suite](https://github.com/mgba-emu/suite) and [NanoBoyAdvance's hardware tests](https://github.com/nba-emu/hw-test).
5. **Other emulators** at pinned revisions, [mGBA 0.10.5](https://github.com/mgba-emu/mgba/tree/0.10.5) and [NanoBoyAdvance](https://github.com/nba-emu/NanoBoyAdvance), as comparisons.
   Where they disagree with each other or with GBATEK, the page says which behavior Evenfall follows and why.
   Evenfall copies no code from them.

[`refs/README.md`](../../../refs/README.md) lists the exact revision of every external reference.

## Conventions

- Addresses and register values are hexadecimal, written `04000200h`.
- Times are in master-clock cycles (16,777,216 per second) unless a page says otherwise.
- The Rust modules linked from a page own the behavior it describes.
- Each page ends with **open questions**: behavior no available source settles, and the choice Evenfall makes until a hardware test does.
  They are the best place to start for anyone with a GBA and a way to measure it.
