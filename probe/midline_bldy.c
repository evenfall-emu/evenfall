/* Probe 41: mid-scanline BLDY brightness wave (Mode 0).
 *
 * Shows a full-screen red BG0 under brightness increase with BG0 as first
 * target and factor 0, then on line 40 writes BLDY=16 (full white) and later
 * restores 0, timed by short spin loops after a VCOUNT poll. The validator
 * checks analytic structure: other rows are all red, while row 40 must
 * contain a genuine white span over a sane mid-line range. Exact split
 * pixels drift with poll phase, so structure rather than pixels is
 * asserted. This exercises the per-scanline blend segment log; without it
 * the whole line would show the last factor written.
 */

#include "gba.h"

#define REG_DISPCNT 0x00
#define REG_BG0CNT 0x08
#define REG_BLDCNT 0x50
#define REG_BLDY 0x54
#define REG_VCOUNT 0x06
#define BG_TILES ((volatile uint16_t *)0x06000000)
#define BG_MAP ((volatile uint16_t *)0x0600F800)
#define PALETTE ((volatile uint16_t *)0x05000000)

#define TARGET_LINE 40u
#define SPIN_ONE 16u
#define SPIN_TWO 16u

void report_and_idle(uint32_t value);

static void spin(uint32_t n) {
    __asm__ volatile (
        "1:\n"
        "subs %0, %0, #1\n"
        "bne 1b\n"
        : "+r" (n)
    );
}

int main(void) {
    MEM_IO[REG_DISPCNT / 2] = 0x0100u;
    MEM_IO[REG_BG0CNT / 2] = 0x1F00u;
    MEM_IO[REG_BLDCNT / 2] = 0x0081u;
    MEM_IO[REG_BLDY / 2] = 0x0000u;

    PALETTE[1] = 0x001Fu;

    for (uint32_t i = 0; i < 16u; i++) {
        BG_TILES[i] = 0x1111u;
    }
    for (uint32_t i = 0; i < 1024u; i++) {
        BG_MAP[i] = 0x0000u;
    }

    while (MEM_IO[REG_VCOUNT / 2] != 0) {
    }
    for (uint32_t f = 0; f < 3u; f++) {
        while (MEM_IO[REG_VCOUNT / 2] != TARGET_LINE) {
        }
        spin(SPIN_ONE);
        MEM_IO[REG_BLDY / 2] = 0x0010u;
        spin(SPIN_TWO);
        MEM_IO[REG_BLDY / 2] = 0x0000u;
        while (MEM_IO[REG_VCOUNT / 2] == TARGET_LINE) {
        }
    }
    report_and_idle(0);
    return 0;
}
