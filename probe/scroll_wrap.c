/* Probe 50: text scroll wrapping at map edges (Mode 0, 32x32 map).
 *
 * Fills screenblock 31 with tile index (r*32+c)%4 from four solid tiles
 * (palette 1-4: red/green/blue/white) and scrolls by HOFS=200, VOFS=100,
 * wrapping mid-map both ways ((239+200) and (159+100) exceed 256). After
 * two frames the validator recomputes every pixel analytically from the
 * wrapped map coordinates. Exercises map mirroring/wrap end to end; the
 * raster probes use small offsets that never wrap. HLE boot only.
 */

#include "gba.h"

#define REG_DISPCNT 0x00
#define REG_BG0CNT 0x08
#define REG_BG0HOFS 0x10
#define REG_BG0VOFS 0x12
#define REG_VCOUNT 0x06
#define BG_TILES ((volatile uint16_t *)0x06000000)
#define BG_MAP ((volatile uint16_t *)0x0600F800)
#define PALETTE ((volatile uint16_t *)0x05000000)

void report_and_idle(uint32_t value);

int main(void) {
    MEM_IO[REG_DISPCNT / 2] = 0x0100u;
    MEM_IO[REG_BG0CNT / 2] = 0x1F00u;
    MEM_IO[REG_BG0HOFS / 2] = 200u;
    MEM_IO[REG_BG0VOFS / 2] = 100u;

    PALETTE[1] = 0x001Fu;
    PALETTE[2] = 0x03E0u;
    PALETTE[3] = 0x7C00u;
    PALETTE[4] = 0x7FFFu;

    for (uint32_t t = 0; t < 4u; t++) {
        for (uint32_t i = 0; i < 16u; i++) {
            BG_TILES[t * 16 + i] = (uint16_t)(0x1111u * (t + 1u));
        }
    }
    for (uint32_t r = 0; r < 32u; r++) {
        for (uint32_t c = 0; c < 32u; c++) {
            BG_MAP[r * 32u + c] = (uint16_t)((r * 32u + c) % 4u);
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
