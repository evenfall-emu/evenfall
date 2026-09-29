/* Probe 41: Mode 3 bitmap (240x160 direct color, no pages).
 *
 * Fills the single Mode 3 framebuffer with vertical red/blue half stripes
 * (x < 120 red, else blue), selects Mode 3 with BG2 enabled, and idles
 * after two frames. The validator recomputes every pixel analytically.
 * Completes bitmap-mode ROM coverage alongside the Mode 4 and Mode 5
 * probes. HLE boot only.
 */

#include "gba.h"

#define REG_DISPCNT 0x00
#define REG_VCOUNT 0x06
#define MODE3_FB ((volatile uint16_t *)0x06000000)

void report_and_idle(uint32_t value);

int main(void) {
    for (uint32_t y = 0; y < 160u; y++) {
        for (uint32_t x = 0; x < 240u; x++) {
            MODE3_FB[y * 240u + x] = (uint16_t)(x < 120u ? 0x001Fu : 0x7C00u);
        }
    }
    MEM_IO[REG_DISPCNT / 2] = 0x0403u;

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
