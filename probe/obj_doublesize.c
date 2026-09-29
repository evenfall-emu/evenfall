/* Probe 60: double-size affine OBJ with identity matrix (Mode 0).
 *
 * Shows one 8x8 affine sprite at (20,20) with the double-size bit and an
 * identity matrix, sharing the quadrant tile (red/green/blue/white) and
 * palette of the obj_flip probe. The display area doubles to 16x16 while
 * the image stays centered, so texels resolve at (24..31, 24..31) and the
 * surrounding display area stays transparent over the black backdrop. The
 * validator checks every pixel analytically. This pins the centering
 * convention shared with mGBA (totalWidth doubled, image centered); a
 * corner-pinned variant would shift the image by half the object size.
 * HLE boot only.
 */

#include "gba.h"

#define REG_DISPCNT 0x00
#define REG_VCOUNT 0x06
#define OBJ_TILES ((volatile uint16_t *)0x06010000)
#define OBJ_ATTRS ((volatile uint16_t *)0x07000000)
#define OBJ_PALETTE ((volatile uint16_t *)0x05000200)
#define OBJ_MATRIX ((volatile uint16_t *)0x07000006)

void report_and_idle(uint32_t value);

int main(void) {
    MEM_IO[REG_DISPCNT / 2] = 0x1000u;

    OBJ_PALETTE[1] = 0x001Fu;
    OBJ_PALETTE[2] = 0x03E0u;
    OBJ_PALETTE[3] = 0x7C00u;
    OBJ_PALETTE[4] = 0x7FFFu;

    for (uint32_t row = 0; row < 8u; row++) {
        OBJ_TILES[row * 2 + 0] = (row < 4u) ? 0x1111u : 0x3333u;
        OBJ_TILES[row * 2 + 1] = (row < 4u) ? 0x2222u : 0x4444u;
    }

    OBJ_ATTRS[0] = 0x0314u;
    OBJ_ATTRS[1] = 0x0014u;
    OBJ_ATTRS[2] = 0x0000u;

    /* Zeroed OAM entries are live (0,0) sprites: park the rest offscreen. */
    for (uint32_t i = 1u; i < 128u; i++) {
        OBJ_ATTRS[i * 4] = 0x0200u;
    }

    OBJ_MATRIX[0] = 0x0100u;
    OBJ_MATRIX[4] = 0x0000u;
    OBJ_MATRIX[8] = 0x0000u;
    OBJ_MATRIX[12] = 0x0100u;

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
