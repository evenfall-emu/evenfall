/* Probe 30: Mode 5 bitmap page 0 (160x128 direct color).
 *
 * Fills the 160x128 page-0 framebuffer at 0x06000000 with 8-pixel red/blue
 * checker squares, selects Mode 5 page 0, and idles after two frames. The
 * validator recomputes every pixel analytically (direct BGR555, 160-pixel
 * stride, no palette). Exercises the bitmap VRAM path and page selection
 * end to end. HLE boot only.
 */

#include "gba.h"

#define REG_DISPCNT 0x00
#define REG_VCOUNT 0x06
#define MODE5_PAGE0 ((volatile uint16_t *)0x06000000)

void report_and_idle(uint32_t value);

int main(void) {
    for (uint32_t y = 0; y < 128u; y++) {
        for (uint32_t x = 0; x < 160u; x++) {
            uint32_t checker = ((x / 8u) + (y / 8u)) & 1u;
            MODE5_PAGE0[y * 160u + x] = (uint16_t)(checker ? 0x7C00u : 0x001Fu);
        }
    }
    MEM_IO[REG_DISPCNT / 2] = 0x0405u;

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
