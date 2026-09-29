/* Probe 18: noise channel LFSR in 7-bit and 15-bit modes.
 *
 * Programs the noise channel for constant volume 15, no length check, and
 * divisor/shift r=1/s=4 (1024-cycle LFSR steps, i.e. two DAC samples per
 * step), first with the 7-bit polynomial then retriggered with the 15-bit
 * one, spinning between the two phases. Routing, master volume, mix ratio,
 * and master enable match the square probe, so HIGH steps surface as +240
 * and LOW steps as 0. The validator checks the stream against the GBATEK
 * polynomial-counter recurrence independently reimplemented from its
 * documented taps (see test/probe_cases.zig); sliding-window
 * matching absorbs the trigger-phase difference between GBATEK's documented
 * reset states (0x40/0x4000) and any other reset on the same cycle, so the
 * taps are proven while the reset value stays a documented open question.
 */

#include "gba.h"

#define REG_NR41_NR42 0x78
#define REG_NR43_NR44 0x7C
#define REG_SOUNDCNT_L 0x80
#define REG_SOUNDCNT_H 0x82
#define REG_SOUNDCNT_X 0x84

void report_and_idle(uint32_t value);

int main(void) {
    /* Master enable first: PSG writes while NR52 bit 7 is clear are lost. */
    MEM_IO[REG_SOUNDCNT_X / 2] = 0x0080u;
    MEM_IO[REG_SOUNDCNT_L / 2] = 0xFF77u;
    MEM_IO[REG_SOUNDCNT_H / 2] = 0x0002u;
    MEM_IO[REG_NR41_NR42 / 2] = 0xF000u;
    MEM_IO[REG_NR43_NR44 / 2] = 0x8049u;

    for (volatile uint32_t i = 0; i < 4000u; i++) {
    }

    MEM_IO[REG_NR43_NR44 / 2] = 0x8041u;

    for (volatile uint32_t i = 0; i < 3000u; i++) {
    }
    report_and_idle(0);
    return 0;
}
