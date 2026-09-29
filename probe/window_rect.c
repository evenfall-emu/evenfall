/* Probe 16: WIN0 rectangular window masking (Mode 0).
 *
 * Shows a full-screen red BG0 with WIN0 covering x in [80,160) and y in
 * [40,120). The WIN0 field of WININ enables BG0 only and WINOUT enables
 * nothing, so the window interior renders red and everything outside shows
 * the black backdrop. After two frames the validator checks every pixel
 * analytically (see test/probe_cases.zig), pinning the
 * inclusive/exclusive WIN0H/WIN0V edge semantics end to end.
 */

#include "gba.h"

#define REG_DISPCNT 0x00
#define REG_BG0CNT 0x08
#define REG_WIN0H 0x40
#define REG_WIN0V 0x44
#define REG_WININ 0x48
#define REG_WINOUT 0x4A
#define REG_VCOUNT 0x06

#define BG_TILES ((volatile uint16_t *)0x06000000)
#define BG_MAP ((volatile uint16_t *)0x0600F800)
#define PALETTE ((volatile uint16_t *)0x05000000)

void report_and_idle(uint32_t value);

int main(void) {
    MEM_IO[REG_DISPCNT / 2] = 0x2100u;
    MEM_IO[REG_BG0CNT / 2] = 0x1F00u;
    MEM_IO[REG_WIN0H / 2] = 0x50A0u;
    MEM_IO[REG_WIN0V / 2] = 0x2878u;
    MEM_IO[REG_WININ / 2] = 0x0001u;
    MEM_IO[REG_WINOUT / 2] = 0x0000u;

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
