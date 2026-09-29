/* Probe 43: mid-scanline affine matrix splits (Mode 2).
 *
 * Shows an identity-mapped affine layer (red/green/blue tiles, overflow
 * transparency; the map of affine_rot.c unrotated), then on line 40 writes BG2PA=0x200 (2x
 * horizontal zoom) and later restores 0x100, timed by short spin loops
 * after a VCOUNT poll. The validator checks analytic structure: other rows
 * match the identity map exactly, while row 40 must differ from it over a
 * sane mid-line span (zoomed stripes). Exact split pixels drift with poll
 * phase, so structure rather than pixels is asserted. This exercises the
 * per-scanline affine segment log; without it the whole line would show
 * the last matrix written. Per-line reference advances, bitmap modes, and
 * reference-point writes... reference writes are logged too; bitmap modes
 * stay end-of-line.
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

#define TARGET_LINE 40u
/* SPIN_ONE is calibrated for correct Game Pak waitstates (N+1/S+1): the
 * write lands mid-line at 8 and at the line edge by 16. */
#define SPIN_ONE 8u
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
    MEM_IO[REG_DISPCNT / 2] = 0x0402u;
    MEM_IO[REG_BG2CNT / 2] = 0x0800u;

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
    for (uint32_t f = 0; f < 3u; f++) {
        while (MEM_IO[REG_VCOUNT / 2] != TARGET_LINE) {
        }
        spin(SPIN_ONE);
        MEM_IO[REG_BG2PA / 2] = 0x0200u;
        spin(SPIN_TWO);
        MEM_IO[REG_BG2PA / 2] = 0x0100u;
        while (MEM_IO[REG_VCOUNT / 2] == TARGET_LINE) {
        }
    }
    report_and_idle(0);
    return 0;
}
