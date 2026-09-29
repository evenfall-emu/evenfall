/* Probe 64: green swap exchanges green within pixel pairs (Mode 0).
 *
 * BG0 shows alternating red (even x) and green (odd x) columns with
 * GREENSWAP set. The swap applies to the final picture, exchanging green
 * between each even/odd pair: even pixels render yellow (red plus the
 * green pixel's green) and odd pixels render black. After two frames the
 * validator checks every pixel analytically. Covers the green-swap path
 * end to end. HLE boot only.
 */

#include "gba.h"

#define REG_DISPCNT 0x00
#define REG_GREENSWAP 0x02
#define REG_BG0CNT 0x08
#define REG_VCOUNT 0x06
#define BG_TILES ((volatile uint16_t *)0x06000000)
#define BG_MAP ((volatile uint16_t *)0x0600F800)
#define PALETTE ((volatile uint16_t *)0x05000000)

void report_and_idle(uint32_t value);

int main(void) {
    MEM_IO[REG_DISPCNT / 2] = 0x0100u;
    MEM_IO[REG_BG0CNT / 2] = 0x1F00u;
    MEM_IO[REG_GREENSWAP / 2] = 0x0001u;

    PALETTE[1] = 0x001Fu;
    PALETTE[2] = 0x03E0u;

    for (uint32_t i = 0; i < 16u; i++) {
        BG_TILES[i] = 0x2121u;
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
