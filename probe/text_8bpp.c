/* Probe 54: 8bpp text background fetch (Mode 0).
 *
 * BG0 uses 8bpp tiles (BG0CNT 0x1F80) with tile 1 filled with pixel value 5
 * (red through palette 5); palette 0 is blue (backdrop). Rows 0-11 of
 * screenblock 31 point at tile 1 for columns 0-23, with palette bank 3 set
 * on columns 12-23. Per GBATEK the entry palette bits are not used in
 * 256-color text mode, so both halves render red; the validator checks the
 * full frame analytically and would catch any banking wrongly applied to
 * 8bpp text. Exercises 8bpp text fetch end to end (affine 8bpp stays with
 * the affine and mosaic probes). HLE boot only.
 */

#include "gba.h"

#define REG_DISPCNT 0x00
#define REG_BG0CNT 0x08
#define REG_VCOUNT 0x06
#define BG_TILES ((volatile uint16_t *)0x06000000)
#define BG_MAP ((volatile uint16_t *)0x0600F800)
#define PALETTE ((volatile uint16_t *)0x05000000)

void report_and_idle(uint32_t value);

int main(void) {
    MEM_IO[REG_DISPCNT / 2] = 0x0100u;
    MEM_IO[REG_BG0CNT / 2] = 0x1F80u;

    PALETTE[0] = 0x7C00u;
    PALETTE[5] = 0x001Fu;

    for (uint32_t i = 0; i < 32u; i++) {
        BG_TILES[32 + i] = 0x0505u;
    }
    for (uint32_t r = 0; r < 32u; r++) {
        for (uint32_t c = 0; c < 32u; c++) {
            uint16_t entry = 0;
            if (r < 12u && c < 12u) {
                entry = 0x0001u;
            } else if (r < 12u && c < 24u) {
                entry = 0x3001u;
            }
            BG_MAP[r * 32u + c] = entry;
        }
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
