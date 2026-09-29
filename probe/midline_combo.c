/* Probe 49: combined mid-scanline splits (scroll + backdrop + window).
 *
 * Mode 0 striped BG0 visible only through a static WIN0 x=[0,120) rectangle
 * (WINOUT empty); the right half shows the bare backdrop. On line 40, timed
 * by short spin loops after a VCOUNT poll: BG0HOFS 0 -> 4 -> 0 (scroll
 * split, visible only left of x=120) and palette 0 blue -> red -> blue
 * (backdrop split, visible only from x=120). The validator checks analytic
 * structure per region: the left part carries a genuine stripe-phase split,
 * the right part a genuine color split, each over a sane mid-line span,
 * with everything else unshifted. This exercises scroll, backdrop, and
 * window segment machinery coexisting on one scanline. HLE boot only.
 */

#include "gba.h"

#define REG_DISPCNT 0x00
#define REG_BG0CNT 0x08
#define REG_BG0HOFS 0x10
#define REG_BG0VOFS 0x12
#define REG_WIN0H 0x40
#define REG_WIN0V 0x44
#define REG_WININ 0x48
#define REG_WINOUT 0x4A
#define REG_VCOUNT 0x06
#define BG_TILES ((volatile uint16_t *)0x06000000)
#define BG_MAP ((volatile uint16_t *)0x0600F800)
#define PALETTE ((volatile uint16_t *)0x05000000)

#define TARGET_LINE 40u
/* SPIN_ONE is calibrated for correct Game Pak waitstates (N+1/S+1). */
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
    MEM_IO[REG_DISPCNT / 2] = 0x2100u;
    MEM_IO[REG_BG0CNT / 2] = 0x1F00u;
    MEM_IO[REG_BG0HOFS / 2] = 0u;
    MEM_IO[REG_BG0VOFS / 2] = 0u;
    MEM_IO[REG_WIN0H / 2] = 0x0078u;
    MEM_IO[REG_WIN0V / 2] = 0x00A0u;
    MEM_IO[REG_WININ / 2] = 0x0001u;
    MEM_IO[REG_WINOUT / 2] = 0x0000u;

    PALETTE[0] = 0x7C00u;
    PALETTE[1] = 0x001Fu;
    PALETTE[2] = 0x03E0u;

    for (uint32_t row = 0; row < 8u; row++) {
        BG_TILES[16 + row * 2 + 0] = 0x1111u;
        BG_TILES[16 + row * 2 + 1] = 0x2222u;
    }
    for (uint32_t i = 0; i < 1024u; i++) {
        BG_MAP[i] = 0x0001u;
    }

    while (MEM_IO[REG_VCOUNT / 2] != 0) {
    }
    for (uint32_t f = 0; f < 3u; f++) {
        while (MEM_IO[REG_VCOUNT / 2] != TARGET_LINE) {
        }
        spin(SPIN_ONE);
        MEM_IO[REG_BG0HOFS / 2] = 4u;
        {
            volatile uint16_t *backdrop = (uint16_t *)0x05000000u;
            *backdrop = 0x001Fu;
        }
        spin(SPIN_TWO);
        MEM_IO[REG_BG0HOFS / 2] = 0u;
        {
            volatile uint16_t *backdrop = (uint16_t *)0x05000000u;
            *backdrop = 0x7C00u;
        }
        while (MEM_IO[REG_VCOUNT / 2] == TARGET_LINE) {
        }
    }
    report_and_idle(0);
    return 0;
}
