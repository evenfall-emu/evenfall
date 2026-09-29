/* Probe 68: affine OBJ with a 90-degree rotation (Mode 0).
 *
 * Shows one 8x8 affine object at (8,8) with matrix 0 set to PA=0, PB=+1,
 * PC=-1, PD=0, tile and palette identical to the regular sprite in
 * obj_sprite.c (red/green halves). The rotation center is the integer
 * middle (4,4) per GBATEK, so screen pixel (8+i,8+j) samples texture
 * (j,8-i): row i=0 maps outside and is transparent, and texture row 0 is
 * never sampled. The validator checks that hand-computed oracle (green/red
 * columns swapped by the rotation with the top row transparent, black
 * elsewhere). This exercises nonzero OBJ rotation terms and the integer
 * rotation-center convention end to end; an identity matrix is covered by
 * the obj_doublesize probe. HLE boot only.
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

    for (uint32_t row = 0; row < 8u; row++) {
        OBJ_TILES[row * 2 + 0] = 0x1111u;
        OBJ_TILES[row * 2 + 1] = 0x2222u;
    }

    OBJ_ATTRS[0] = 0x0108u;
    OBJ_ATTRS[1] = 0x0008u;
    OBJ_ATTRS[2] = 0x0000u;
    for (uint32_t i = 1u; i < 128u; i++) {
        OBJ_ATTRS[i * 4] = 0x0200u;
    }

    OBJ_MATRIX[0] = 0x0000u;
    OBJ_MATRIX[4] = 0x0100u;
    OBJ_MATRIX[8] = 0xFF00u;
    OBJ_MATRIX[12] = 0x0000u;

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
