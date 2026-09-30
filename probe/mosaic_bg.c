/* Probe 17: BG mosaic stretch (Mode 0, 4x4).
 *
 * Shows one 8bpp tile (pixel (x,y) holds index y*8+x) repeated over
 * screenblock 31, with BG mosaic size 4x4 selecting the
 * upper-left source pixel of each group. Palette entries are all distinct.
 * After two frames the validator recomputes every pixel analytically (see
 * validator/probe_cases.rs): mosaic quantization of the output
 * coordinates before scrolling (HOFS/VOFS are zero), per the GBATEK mosaic
 * description. Index 0 stays transparent and shows the backdrop.
 */

#include "gba.h"

#define REG_DISPCNT 0x00
#define REG_BG0CNT 0x08
#define REG_MOSAIC 0x4C
#define REG_VCOUNT 0x06

#define BG_TILES ((volatile uint16_t *)0x06000000)
#define BG_MAP ((volatile uint16_t *)0x0600F800)
#define PALETTE ((volatile uint16_t *)0x05000000)

void report_and_idle(uint32_t value);

int main(void) {
    MEM_IO[REG_DISPCNT / 2] = 0x0100u;
    MEM_IO[REG_BG0CNT / 2] = (uint16_t)(0x1F00u | 0x0080u | 0x0040u);
    MEM_IO[REG_MOSAIC / 2] = 0x0033u;

    for (uint32_t i = 0; i < 64u; i++) {
        uint32_t v = (i * 501u) & 0x7FFFu;
        PALETTE[i] = (uint16_t)v;
    }
    for (uint32_t i = 0; i < 32u; i++) {
        uint32_t lo = i * 2u;
        uint32_t hi = i * 2u + 1u;
        BG_TILES[i] = (uint16_t)(lo | (hi << 8));
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
