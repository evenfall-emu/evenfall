/* Probe 35: OBJ mosaic stretch with edge clamping (Mode 0).
 *
 * Shows two 8x8 4bpp objects with 2x2 mosaic enabled: OBJ0 at (8,8) with a
 * 4x4 red/green checker tile at priority 0, and OBJ1 with the same tile at
 * (13,8) priority 1. Per docs/hardware/video.md, mosaic groups align
 * to screen coordinates, so the odd-placed OBJ1 samples group bases
 * outside itself clamped to its first source edge. After two frames the
 * validator recomputes every pixel analytically (see
 * validator/probe_cases.rs), with OBJ0 winning the overlap on
 * priority. Exercises OBJ mosaic grouping, clamping, and compositing end
 * to end. HLE boot only.
 */

#include "gba.h"

#define REG_DISPCNT 0x00
#define REG_MOSAIC 0x4C
#define REG_VCOUNT 0x06
#define OBJ_TILES ((volatile uint16_t *)0x06010000)
#define OBJ_ATTRS ((volatile uint16_t *)0x07000000)
#define OBJ_PALETTE ((volatile uint16_t *)0x05000200)

void report_and_idle(uint32_t value);

int main(void) {
    MEM_IO[REG_DISPCNT / 2] = 0x1000u;
    MEM_IO[REG_MOSAIC / 2] = 0x1100u;

    OBJ_PALETTE[1] = 0x001Fu;
    OBJ_PALETTE[2] = 0x03E0u;

    for (uint32_t row = 0; row < 8u; row++) {
        uint32_t top = (row < 4u) ? 0x1111u : 0x2222u;
        uint32_t bottom = (row < 4u) ? 0x2222u : 0x1111u;
        OBJ_TILES[row * 2 + 0] = (uint16_t)(top & 0xFFFFu);
        OBJ_TILES[row * 2 + 1] = (uint16_t)(bottom & 0xFFFFu);
    }

    OBJ_ATTRS[0] = 0x1008u;
    OBJ_ATTRS[1] = 0x0008u;
    OBJ_ATTRS[2] = 0x0000u;
    OBJ_ATTRS[4] = 0x1008u;
    OBJ_ATTRS[5] = 0x000Du;
    OBJ_ATTRS[6] = 0x0400u;
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
