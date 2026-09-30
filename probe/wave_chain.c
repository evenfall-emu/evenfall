/* Probe 52: wave channel two-bank 64-sample chaining.
 *
 * While the master is off the CPU sees bank 1, so the 16 setup halfwords
 * all land in bank 1: positions 0-15 hold an ascending 0..15 nibble ramp
 * and positions 16-31 hold constant nibble 7 (bank 0 stays zero). Selected
 * with two-bank mode, constant volume, no length check, triggered at
 * frequency 0x700. The 64-step chain plays bank 0 (zeros) then bank 1, so
 * each cycle shows a 128-zero run, 16 ascending 4-sample runs, and one
 * 64-sample run of 112; a broken chain looping one bank alone could never
 * produce both the zero runs and the 112 marker. The validator checks that
 * analytic structure (see validator/probe_cases.rs).
 * Single-bank sequencing is covered by the wave_ramp probe. HLE boot only.
 */

#include "gba.h"

#define REG_NR30 0x70
#define REG_NR32 0x72
#define REG_NR33_NR34 0x74
#define REG_SOUNDCNT_L 0x80
#define REG_SOUNDCNT_H 0x82
#define REG_SOUNDCNT_X 0x84
#define WAVE_RAM ((volatile uint16_t *)0x04000090)

void report_and_idle(uint32_t value);

int main(void) {
    for (uint32_t h = 0; h < 4u; h++) {
        uint32_t base = h * 4u;
        uint32_t blo = (((base) % 16u) << 4) | ((base + 1u) % 16u);
        uint32_t bhi = (((base + 2u) % 16u) << 4) | ((base + 3u) % 16u);
        WAVE_RAM[h] = (uint16_t)((bhi << 8) | blo);
    }
    for (uint32_t h = 4u; h < 8u; h++) {
        WAVE_RAM[h] = 0x7777u;
    }

    MEM_IO[REG_SOUNDCNT_X / 2] = 0x0080u;
    /* Routing after enable: SOUNDCNT_L is gated while master is off. */
    MEM_IO[REG_SOUNDCNT_L / 2] = 0x4477u;
    MEM_IO[REG_SOUNDCNT_H / 2] = 0x0002u;
    MEM_IO[REG_NR30 / 2] = 0x00A0u;
    MEM_IO[REG_NR32 / 2] = 0x2000u;
    MEM_IO[REG_NR33_NR34 / 2] = 0x8700u;

    for (volatile uint32_t i = 0; i < 9000u; i++) {
    }
    report_and_idle(0);
    return 0;
}
