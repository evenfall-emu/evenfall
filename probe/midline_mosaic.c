/* Probe 59: mid-scanline BG mosaic-size splits (Mode 0).
 *
 * BG0 shows a tile with eight distinct columns under BG mosaic, starting at
 * 4x4 (MOSAIC 0x33). On line 40 the ROM polls VCOUNT, spins, and writes
 * MOSAIC 0x00 (1x1) mid-line, so the left half renders 4-wide blocks and
 * the right half renders sharp columns; other lines use the 1x1 end state.
 * The validator checks that run structure (see
 * validator/probe_cases.rs). Covers horizontal mosaic sizing per
 * pixel; vertical sizing stays end-of-line by construction (a mid-line
 * write cannot change which source line a line samples). HLE boot only.
 */

#include "gba.h"

#define REG_DISPCNT 0x00
#define REG_BG0CNT 0x08
#define REG_MOSAIC 0x4C
#define REG_VCOUNT 0x06
#define BG_TILES ((volatile uint16_t *)0x06000000)
#define BG_MAP ((volatile uint16_t *)0x0600F800)
#define PALETTE ((volatile uint16_t *)0x05000000)

void report_and_idle(uint32_t value);

int main(void) {
    MEM_IO[REG_DISPCNT / 2] = 0x0100u;
    MEM_IO[REG_BG0CNT / 2] = 0x1F40u;
    MEM_IO[REG_MOSAIC / 2] = 0x0033u;

    PALETTE[1] = 0x001Fu;
    PALETTE[2] = 0x03E0u;
    PALETTE[3] = 0x7C00u;
    PALETTE[4] = 0x7FFFu;
    PALETTE[5] = 0x03FFu;
    PALETTE[6] = 0x7C1Fu;
    PALETTE[7] = 0x7FE0u;
    PALETTE[8] = 0x4210u;

    for (uint32_t row = 0; row < 8u; row++) {
        BG_TILES[16 + row * 2 + 0] = 0x4321u;
        BG_TILES[16 + row * 2 + 1] = 0x8765u;
    }
    for (uint32_t i = 0; i < 1024u; i++) {
        BG_MAP[i] = 0x0001u;
    }

    while (MEM_IO[REG_VCOUNT / 2] != 0) {
    }
    for (uint32_t f = 0; f < 3u; f++) {
        while (MEM_IO[REG_VCOUNT / 2] != 0) {
        }
        MEM_IO[REG_MOSAIC / 2] = 0x0033u;
        while (MEM_IO[REG_VCOUNT / 2] != 40) {
        }
        /* Calibrated for correct Game Pak waitstates; the write lands at x=132. */
        for (volatile uint32_t i = 0; i < 8u; i++) {
        }
        MEM_IO[REG_MOSAIC / 2] = 0x0000u;
        while (MEM_IO[REG_VCOUNT / 2] == 40) {
        }
    }
    report_and_idle(0);
    return 0;
}
