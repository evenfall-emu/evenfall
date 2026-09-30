/* Probe 15: alpha blending of two full-screen backgrounds (Mode 0).
 *
 * BG0 is solid palette 1 (red) at priority 0 and BG1 is solid palette 2
 * (green) at priority 1; BLDCNT selects alpha blending with BG0 as first
 * target and BG1 as second, EVA=EVB=8. After two frames the validator
 * asserts every pixel analytically: (31*8+0*8)/16 = 15 per red/green
 * channel, i.e. 0x01EF (see validator/probe_cases.rs). Exercises
 * priority resolution, first/second target selection, and the blend
 * equation end to end.
 */

#include "gba.h"

#define REG_DISPCNT 0x00
#define REG_BG0CNT 0x08
#define REG_BG1CNT 0x0A
#define REG_BLDCNT 0x50
#define REG_BLDALPHA 0x52
#define REG_VCOUNT 0x06

#define BG0_MAP ((volatile uint16_t *)0x0600F800)
#define BG1_MAP ((volatile uint16_t *)0x0600F000)
#define BG_TILES ((volatile uint16_t *)0x06000000)
#define PALETTE ((volatile uint16_t *)0x05000000)

void report_and_idle(uint32_t value);

int main(void) {
    MEM_IO[REG_DISPCNT / 2] = 0x0300u;
    MEM_IO[REG_BG0CNT / 2] = 0x1F00u;
    MEM_IO[REG_BG1CNT / 2] = 0x1E01u;
    MEM_IO[REG_BLDCNT / 2] = 0x0241u;
    MEM_IO[REG_BLDALPHA / 2] = 0x0808u;

    PALETTE[1] = 0x001Fu;
    PALETTE[2] = 0x03E0u;

    for (uint32_t i = 0; i < 16u; i++) {
        BG_TILES[i] = 0x1111u;
        BG_TILES[16 + i] = 0x2222u;
    }
    for (uint32_t i = 0; i < 1024u; i++) {
        BG0_MAP[i] = 0x0000u;
        BG1_MAP[i] = 0x0001u;
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
