/* Probe 24: brightness increase, or decrease with DARKEN (Mode 0).
 *
 * BG0 is solid palette 1 (red); BLDCNT selects brightness increase (or
 * decrease, built as brighten_dark.c) with BG0 as first target and BLDY
 * factor 8. After two frames the validator asserts every pixel by GBATEK's
 * equations: increase gives 31 + (31-31)*8/16 = 31 red and
 * 0 + (31-0)*8/16 = 15 green and blue, i.e. 0x3DFF; decrease gives
 * 31 - 31*8/16 = 16 red with green and blue at 0, i.e. 0x0010.
 */

#include "gba.h"

#define REG_DISPCNT 0x00
#define REG_BG0CNT 0x08
#define REG_BLDCNT 0x50
#define REG_BLDY 0x54
#define REG_VCOUNT 0x06

#define BG_TILES ((volatile uint16_t *)0x06000000)
#define BG_MAP ((volatile uint16_t *)0x0600F800)
#define PALETTE ((volatile uint16_t *)0x05000000)

void report_and_idle(uint32_t value);

int main(void) {
    MEM_IO[REG_DISPCNT / 2] = 0x0100u;
    MEM_IO[REG_BG0CNT / 2] = 0x1F00u;
#ifdef DARKEN
    MEM_IO[REG_BLDCNT / 2] = 0x00C1u;
#else
    MEM_IO[REG_BLDCNT / 2] = 0x0081u;
#endif
    MEM_IO[REG_BLDY / 2] = 0x0008u;

    PALETTE[1] = 0x001Fu;

    for (uint32_t i = 0; i < 16u; i++) {
        BG_TILES[i] = 0x1111u;
    }
    for (uint32_t i = 0; i < 1024u; i++) {
        BG_MAP[i] = 0x0000u;
    }

    while (MEM_IO[REG_VCOUNT / 2] != 0) {
    }
    for (uint32_t f = 0; f < 2u; f++) {
        while (MEM_IO[REG_VCOUNT / 2] == 0) {
        }
        while (MEM_IO[REG_VCOUNT / 2] != 0) {
        }
    }
    report_and_idle(0);
    return 0;
}
