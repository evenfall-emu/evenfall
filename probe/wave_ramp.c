/* Probe 19: wave channel ramp through banked wave RAM.
 *
 * Writes a 0..15 nibble ramp (twice) into wave bank 1 while master sound is
 * off, enables master sound, selects DAC on plus play bank 1, sets 100%
 * volume with no length check, and triggers at frequency 0x700 (2048-cycle
 * sample steps, i.e. four DAC samples per nibble). Then spins and idles.
 * The validator drains the stream and checks the analytic result (see
 * test/probe_cases.zig): nibble v surfaces as v*8 through the
 * unbiased mixer in exact 4-sample runs following the ramp order, after a
 * skippable trigger-alignment head run.
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
    /* Bank 1 (bytes 16-31) via halfwords 0-7 while master sound is off;
     * byte k holds nibbles (2k, 2k+1) high-first, nibble[p] = p % 16. */
    for (uint32_t h = 0; h < 8u; h++) {
        uint32_t byte_lo = (((4u * h) % 16u) << 4) | ((4u * h + 1u) % 16u);
        uint32_t byte_hi = (((4u * h + 2u) % 16u) << 4) | ((4u * h + 3u) % 16u);
        WAVE_RAM[h] = (uint16_t)((byte_hi << 8) | byte_lo);
    }

    MEM_IO[REG_SOUNDCNT_X / 2] = 0x0080u;
    /* Routing after enable: SOUNDCNT_L is gated while master is off. */
    MEM_IO[REG_SOUNDCNT_L / 2] = 0x4477u;
    MEM_IO[REG_SOUNDCNT_H / 2] = 0x0002u;
    MEM_IO[REG_NR30 / 2] = 0x00C0u;
    MEM_IO[REG_NR32 / 2] = 0x2000u;
    MEM_IO[REG_NR33_NR34 / 2] = 0x8700u;

    for (volatile uint32_t i = 0; i < 7000u; i++) {
    }
    report_and_idle(0);
    return 0;
}
