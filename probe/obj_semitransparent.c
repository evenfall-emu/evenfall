/* Probe 37: semi-transparent OBJ forced alpha blending (Mode 0).
 *
 * Shows a full-screen green BG1 with an 8x8 solid red semi-transparent
 * OBJ0 at (8,8). BLDCNT selects alpha blending with NO first targets set
 * and BG1 as second target: semi-transparent objects force themselves to
 * be the alpha first target regardless, so every sprite pixel must still
 * blend to (31*8+0*8)/16 = 15 per red/green channel, i.e. 0x01EF. A model
 * that honored only the BLDCNT first-target bits would leave the sprite
 * unblended red and fail. After two frames the validator checks the sprite
 * rectangle analytically against green elsewhere. HLE boot only.
 */

#include "gba.h"

#define REG_DISPCNT 0x00
#define REG_BG1CNT 0x0A
#define REG_BLDCNT 0x50
#define REG_BLDALPHA 0x52
#define REG_VCOUNT 0x06
#define BG_TILES ((volatile uint16_t *)0x06000000)
#define BG_MAP ((volatile uint16_t *)0x0600F000)
#define OBJ_TILES ((volatile uint16_t *)0x06010000)
#define OBJ_ATTRS ((volatile uint16_t *)0x07000000)
#define PALETTE ((volatile uint16_t *)0x05000000)
#define OBJ_PALETTE ((volatile uint16_t *)0x05000200)

void report_and_idle(uint32_t value);

int main(void) {
    MEM_IO[REG_DISPCNT / 2] = 0x1200u;
    MEM_IO[REG_BG1CNT / 2] = 0x1E01u;
    MEM_IO[REG_BLDCNT / 2] = 0x0240u;
    MEM_IO[REG_BLDALPHA / 2] = 0x0808u;

    PALETTE[2] = 0x03E0u;
    OBJ_PALETTE[1] = 0x001Fu;

    for (uint32_t i = 0; i < 16u; i++) {
        BG_TILES[i] = 0x2222u;
    }
    for (uint32_t i = 0; i < 1024u; i++) {
        BG_MAP[i] = 0x0000u;
    }
    for (uint32_t i = 0; i < 16u; i++) {
        OBJ_TILES[i] = 0x1111u;
    }

    OBJ_ATTRS[0] = 0x0408u;
    OBJ_ATTRS[1] = 0x0008u;
    OBJ_ATTRS[2] = 0x0000u;
    for (uint32_t i = 1u; i < 128u; i++) {
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
