/* Probe 6: HBlank DMA raster splits of BG0HOFS (Mode 0, no IRQs).
 *
 * Sets up a 4bpp striped tile across screenblock 31, then programs DMA2 for
 * repeated HBlank-timing halfword transfers from an EWRAM table of 160
 * horizontal offsets (one per scanline) into BG0HOFS with a fixed
 * destination. After two full frames it idles. Each table entry holds
 * ((i + 1) / 16) * 8, so every 16-line band shifts the stripe phase by two
 * pixels. The validator recomputes every pixel analytically (see
 * test/probe_cases.zig); the exact table-to-row mapping is
 * established in probe/README.md.
 */

#include "gba.h"

#define REG_DISPCNT 0x00
#define REG_BG0CNT 0x08
#define REG_BG0HOFS 0x10
#define REG_BG0VOFS 0x12
#define REG_VCOUNT 0x06
#define REG_DMA2SAD_L 0xC8
#define REG_DMA2SAD_H 0xCA
#define REG_DMA2DAD_L 0xCC
#define REG_DMA2DAD_H 0xCE
#define REG_DMA2CNT_L 0xD0
#define REG_DMA2CNT_H 0xD2

#define CHARBLOCK0 ((volatile uint16_t *)0x06000000)
#define SCREENBLOCK31 ((volatile uint16_t *)0x0600F800)
#define PALETTE ((volatile uint16_t *)0x05000000)
#define HOFS_TABLE ((volatile uint16_t *)0x02000000)
#define TABLE_LINES 160u

void report_and_idle(uint32_t value);

int main(void) {
    MEM_IO[REG_DISPCNT / 2] = 0x0100u;
    MEM_IO[REG_BG0CNT / 2] = 0x1F00u;
    MEM_IO[REG_BG0HOFS / 2] = 0u;
    MEM_IO[REG_BG0VOFS / 2] = 0u;

    PALETTE[1] = 0x001Fu;
    PALETTE[2] = 0x03E0u;

    for (uint32_t row = 0; row < 8u; row++) {
        CHARBLOCK0[16 + row * 2 + 0] = 0x1111u;
        CHARBLOCK0[16 + row * 2 + 1] = 0x2222u;
    }
    for (uint32_t i = 0; i < 1024u; i++) {
        SCREENBLOCK31[i] = 0x0001u;
    }
    for (uint32_t i = 0; i < TABLE_LINES; i++) {
        HOFS_TABLE[i] = (uint16_t)(((i + 1u) / 16u) * 2u);
    }

    MEM_IO[REG_DMA2SAD_L / 2] = 0x0000u;
    MEM_IO[REG_DMA2SAD_H / 2] = 0x0200u;
    MEM_IO[REG_DMA2DAD_L / 2] = 0x0010u;
    MEM_IO[REG_DMA2DAD_H / 2] = 0x0400u;
    MEM_IO[REG_DMA2CNT_L / 2] = 0x0001u;
    MEM_IO[REG_DMA2CNT_H / 2] = 0xA240u;

    for (uint32_t f = 0; f < 3u; f++) {
        /* Rewind the source every frame: with increment + repeat the
         * latched pointer would otherwise run past the 160-entry table
         * into zeroed EWRAM. Rewinding inside VBlank keeps the rewind
         * writes out of every drawn scanline (a line-0 rewind would
         * correctly split row 0 under immediate-application semantics).
         * Re-enabling re-latches from the registers, and the CPU HOFS
         * write covers row 0, which renders before the frame's first
         * HBlank transfer lands. */
        while (MEM_IO[REG_VCOUNT / 2] != 160) {
        }
        MEM_IO[REG_DMA2CNT_H / 2] = 0u;
        MEM_IO[REG_DMA2SAD_L / 2] = 0x0000u;
        MEM_IO[REG_DMA2SAD_H / 2] = 0x0200u;
        MEM_IO[REG_DMA2CNT_H / 2] = 0xA240u;
        MEM_IO[REG_BG0HOFS / 2] = HOFS_TABLE[0];
        while (MEM_IO[REG_VCOUNT / 2] == 160) {
        }
    }
    report_and_idle(0);
    return 0;
}
