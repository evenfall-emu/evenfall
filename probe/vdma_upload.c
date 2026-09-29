/* Probe 11: one-shot VBlank DMA upload into VRAM, verified by rendering.
 *
 * Fills an EWRAM staging buffer with 16 halfwords of 0x3333 (one solid
 * 4bpp tile of palette index 3), shows tile 5 across screenblock 31 in
 * Mode 0, then programs a one-shot VBlank-timing DMA3 transfer of the
 * buffer into charblock tile 5. After two frames the tile must have arrived:
 * every pixel renders palette index 3. The validator asserts the full
 * framebuffer analytically (see test/probe_cases.zig). A missed
 * VBlank trigger would leave zeroes (transparent) and show the backdrop.
 */

#include "gba.h"

#define REG_DISPCNT 0x00
#define REG_BG0CNT 0x08
#define REG_VCOUNT 0x06
#define REG_DMA3SAD_L 0xD4
#define REG_DMA3SAD_H 0xD6
#define REG_DMA3DAD_L 0xD8
#define REG_DMA3DAD_H 0xDA
#define REG_DMA3CNT_L 0xDC
#define REG_DMA3CNT_H 0xDE

#define SCREENBLOCK31 ((volatile uint16_t *)0x0600F800)
#define PALETTE ((volatile uint16_t *)0x05000000)
#define STAGE ((volatile uint16_t *)0x02000000)
#define TILE5 ((volatile uint16_t *)0x060000A0)

void report_and_idle(uint32_t value);

int main(void) {
    MEM_IO[REG_DISPCNT / 2] = 0x0100u;
    MEM_IO[REG_BG0CNT / 2] = 0x1F00u;

    PALETTE[3] = 0x7C00u;

    for (uint32_t i = 0; i < 1024u; i++) {
        SCREENBLOCK31[i] = 0x0005u;
    }
    for (uint32_t i = 0; i < 16u; i++) {
        STAGE[i] = 0x3333u;
    }

    MEM_IO[REG_DMA3SAD_L / 2] = 0x0000u;
    MEM_IO[REG_DMA3SAD_H / 2] = 0x0200u;
    MEM_IO[REG_DMA3DAD_L / 2] = 0x00A0u;
    MEM_IO[REG_DMA3DAD_H / 2] = 0x0600u;
    MEM_IO[REG_DMA3CNT_L / 2] = 0x0010u;
    MEM_IO[REG_DMA3CNT_H / 2] = 0x9000u;

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
