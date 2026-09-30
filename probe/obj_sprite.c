/* Probe 13: OBJ sprites with priority overlap (Mode 0, no BGs).
 *
 * Uploads two 8x8 4bpp 1-tile sprites through CPU halfword stores: OBJ0 at
 * (8,8) with left half palette 1 (red) and right half palette 2 (green) at
 * priority 0, and OBJ1 at (12,8) solid palette 3 (blue) at priority 1.
 * After two frames the validator checks the analytic framebuffer (see
 * validator/probe_cases.rs): backdrop black elsewhere, red/green for
 * x=8..15 with OBJ0 winning the x=12..15 overlap on priority, blue for
 * x=16..19, rows 8..15. Exercises OAM upload, tile fetch, per-pixel
 * priority, and palette-0 transparency end to end.
 */

#include "gba.h"

#define REG_DISPCNT 0x00
#define REG_VCOUNT 0x06
#define OBJ_TILES ((volatile uint16_t *)0x06010000)
#define OBJ_ATTRS ((volatile uint16_t *)0x07000000)
#define OBJ_PALETTE ((volatile uint16_t *)0x05000200)

void report_and_idle(uint32_t value);

int main(void) {
    MEM_IO[REG_DISPCNT / 2] = 0x1000u;

    OBJ_PALETTE[1] = 0x001Fu;
    OBJ_PALETTE[2] = 0x03E0u;
    OBJ_PALETTE[3] = 0x7C00u;

    for (uint32_t row = 0; row < 8u; row++) {
        OBJ_TILES[row * 2 + 0] = 0x1111u;
        OBJ_TILES[row * 2 + 1] = 0x2222u;
        OBJ_TILES[16 + row * 2 + 0] = 0x3333u;
        OBJ_TILES[16 + row * 2 + 1] = 0x3333u;
    }

    OBJ_ATTRS[0] = 0x0008u;
    OBJ_ATTRS[1] = 0x0008u;
    OBJ_ATTRS[2] = 0x0000u;
    OBJ_ATTRS[4] = 0x0008u;
    OBJ_ATTRS[5] = 0x000Cu;
    OBJ_ATTRS[6] = 0x0401u;

    /* Zeroed OAM entries are live (0,0) sprites: park the rest offscreen. */
    for (uint32_t i = 2u; i < 128u; i++) {
        OBJ_ATTRS[i * 4] = 0x0200u;
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
