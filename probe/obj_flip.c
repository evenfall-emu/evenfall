/* Probe 55: OBJ horizontal/vertical flips (Mode 0, no BGs).
 *
 * One 8x8 4bpp tile holds asymmetric quadrants (top-left red, top-right
 * green, bottom-left blue, bottom-right white). Four sprites share the tile
 * at y=20, x=20/60/100/140: plain, H-flipped, V-flipped, and both. After
 * two frames the validator recomputes every pixel analytically with the
 * flips applied. Exercises the attr1 flip path end to end; rotation and
 * mosaic variants stay with the obj_rot and obj_mosaic probes. HLE boot
 * only.
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

    OBJ_PALETTE[0] = 0x7C00u;
    OBJ_PALETTE[1] = 0x001Fu;
    OBJ_PALETTE[2] = 0x03E0u;
    OBJ_PALETTE[3] = 0x7C00u;
    OBJ_PALETTE[4] = 0x7FFFu;

    for (uint32_t row = 0; row < 8u; row++) {
        OBJ_TILES[row * 2 + 0] = (row < 4u) ? 0x1111u : 0x3333u;
        OBJ_TILES[row * 2 + 1] = (row < 4u) ? 0x2222u : 0x4444u;
    }

    OBJ_ATTRS[0] = 0x0014u;
    OBJ_ATTRS[1] = 0x0014u;
    OBJ_ATTRS[2] = 0x0000u;
    OBJ_ATTRS[4] = 0x0014u;
    OBJ_ATTRS[5] = 0x103Cu;
    OBJ_ATTRS[6] = 0x0000u;
    OBJ_ATTRS[8] = 0x0014u;
    OBJ_ATTRS[9] = 0x2064u;
    OBJ_ATTRS[10] = 0x0000u;
    OBJ_ATTRS[12] = 0x0014u;
    OBJ_ATTRS[13] = 0x308Cu;
    OBJ_ATTRS[14] = 0x0000u;

    /* Zeroed OAM entries are live (0,0) sprites: park the rest offscreen. */
    for (uint32_t i = 4u; i < 128u; i++) {
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
