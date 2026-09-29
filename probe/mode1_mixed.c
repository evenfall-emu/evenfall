/* Probe 42: Mode 1 mixed text plus affine background.
 *
 * BG0 (text, priority 0) shows a solid red tile over map columns 0-14 and
 * transparent tile 0 elsewhere; BG2 (affine identity, priority 1) shows one
 * solid green 8bpp tile repeated over its 128x128 map with overflow
 * transparency past it. After two frames the validator recomputes every
 * pixel analytically: red for x < 120, green for 120 <= x < 128 while
 * y < 128, black backdrop elsewhere. Exercises text/affine priority,
 * transparency fallthrough, and affine overflow in one frame. HLE boot
 * only.
 */

#include "gba.h"

#define REG_DISPCNT 0x00
#define REG_BG0CNT 0x08
#define REG_BG2CNT 0x0C
#define REG_BG2PA 0x20
#define REG_BG2PB 0x22
#define REG_BG2PC 0x24
#define REG_BG2PD 0x26
#define REG_BG2X_L 0x28
#define REG_BG2X_H 0x2A
#define REG_BG2Y_L 0x2C
#define REG_BG2Y_H 0x2E
#define REG_VCOUNT 0x06

#define TEXT_TILES ((volatile uint16_t *)0x06000000)
#define TEXT_MAP ((volatile uint16_t *)0x0600F800)
#define AFFINE_TILES ((volatile uint16_t *)0x06004000)
#define AFFINE_MAP ((volatile uint16_t *)0x06004800)
#define PALETTE ((volatile uint16_t *)0x05000000)

void report_and_idle(uint32_t value);

int main(void) {
    MEM_IO[REG_DISPCNT / 2] = 0x0501u;
    MEM_IO[REG_BG0CNT / 2] = 0x1F00u;
    MEM_IO[REG_BG2CNT / 2] = 0x0905u;

    PALETTE[1] = 0x001Fu;
    PALETTE[2] = 0x03E0u;

    for (uint32_t i = 0; i < 16u; i++) {
        TEXT_TILES[16 + i] = 0x1111u;
    }
    for (uint32_t r = 0; r < 32u; r++) {
        for (uint32_t c = 0; c < 32u; c++) {
            TEXT_MAP[r * 32u + c] = (c < 15u) ? 0x0001u : 0x0000u;
        }
    }
    for (uint32_t i = 0; i < 32u; i++) {
        AFFINE_TILES[i] = 0x0202u;
    }

    MEM_IO[REG_BG2PA / 2] = 0x0100u;
    MEM_IO[REG_BG2PB / 2] = 0x0000u;
    MEM_IO[REG_BG2PC / 2] = 0x0000u;
    MEM_IO[REG_BG2PD / 2] = 0x0100u;
    MEM_IO[REG_BG2X_L / 2] = 0x0000u;
    MEM_IO[REG_BG2X_H / 2] = 0x0000u;
    MEM_IO[REG_BG2Y_L / 2] = 0x0000u;
    MEM_IO[REG_BG2Y_H / 2] = 0x0000u;

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
