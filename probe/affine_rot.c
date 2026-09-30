/* Probe 67: Mode 2 affine background with a 90-degree rotation, clipped
 * or with WRAP wrapping (built as affine_wrap.c).
 *
 * Shows BG2 as a 128x128-pixel 8bpp map (16x16 tiles) rotated 90 degrees:
 * PA=0, PB=-1, PC=+1, PD=0 with reference (127, 0), so screen (x,y) samples
 * map (127-y, x). Tiles 0-2 are solid palette 1-3 (red/green/blue), tile 3
 * is all zero; map entry (r,c) holds (r*16+c)%4. With the overflow bit
 * clear, everything past the rotated 128x128 square is transparent
 * backdrop; with WRAP it is set, out-of-range coordinates (negative ones
 * included) wrap modulo 128, and the whole screen shows the rotated map
 * tiled. After two frames the validator recomputes every pixel
 * analytically (see validator/probe_cases.rs). Exercises nonzero rotation
 * terms, reference-origin handling, per-line reference advance, and both
 * overflow modes end to end; the identity matrix is the mode1_mixed layer.
 */

#include "gba.h"

#define REG_DISPCNT 0x00
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

#define AFFINE_TILES ((volatile uint16_t *)0x06000000)
#define AFFINE_MAP ((volatile uint16_t *)0x06004000)
#define PALETTE ((volatile uint16_t *)0x05000000)

void report_and_idle(uint32_t value);

int main(void) {
    MEM_IO[REG_DISPCNT / 2] = 0x0402u;
#ifdef WRAP
    MEM_IO[REG_BG2CNT / 2] = 0x2800u;
#else
    MEM_IO[REG_BG2CNT / 2] = 0x0800u;
#endif

    PALETTE[1] = 0x001Fu;
    PALETTE[2] = 0x03E0u;
    PALETTE[3] = 0x7C00u;

    for (uint32_t t = 0; t < 3u; t++) {
        for (uint32_t i = 0; i < 32u; i++) {
            AFFINE_TILES[t * 32 + i] = (uint16_t)(0x0101u * (t + 1u));
        }
    }
    for (uint32_t i = 0; i < 32u; i++) {
        AFFINE_TILES[3 * 32 + i] = 0x0000u;
    }
    for (uint32_t r = 0; r < 16u; r++) {
        for (uint32_t c = 0; c < 16u; c += 2u) {
            uint32_t lo = (r * 16u + c) % 4u;
            uint32_t hi = (r * 16u + c + 1u) % 4u;
            AFFINE_MAP[(r * 16u + c) / 2u] = (uint16_t)(lo | (hi << 8));
        }
    }

    MEM_IO[REG_BG2PA / 2] = 0x0000u;
    MEM_IO[REG_BG2PB / 2] = 0xFF00u;
    MEM_IO[REG_BG2PC / 2] = 0x0100u;
    MEM_IO[REG_BG2PD / 2] = 0x0000u;
    MEM_IO[REG_BG2X_L / 2] = 0x7F00u;
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
