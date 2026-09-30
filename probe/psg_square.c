/* Probe 4: PSG square-wave golden waveform through CPU-driven MMIO.
 *
 * Programs square channel 2 for a 512 Hz tone (frequency 0x700), 50% duty,
 * constant volume 15, no length check; routes it left and right at full
 * master volume with the 100% PSG mix ratio; enables master sound. Then
 * spins ~100k loop iterations and idles. The validator drains the
 * deterministic DAC-rate stream and checks it against the analytic waveform
 * (see validator/probe_cases.rs): samples quantize to exactly {0, 240}
 * with 8/32/24-sample high/low/high runs repeating every 64 samples.
 */

#include "gba.h"

#define REG_NR21_NR22 0x68
/* NR23/NR24 live at 0x6C; 0x6A is an unused hole (GBATEK). */
#define REG_NR23_NR24 0x6C
#define REG_SOUNDCNT_L 0x80
#define REG_SOUNDCNT_H 0x82
#define REG_SOUNDCNT_X 0x84

void report_and_idle(uint32_t value);

int main(void) {
    /* Master enable first: PSG writes while NR52 bit 7 is clear are lost. */
    MEM_IO[REG_SOUNDCNT_X / 2] = 0x0080u;
    MEM_IO[REG_SOUNDCNT_L / 2] = 0xFF77u;
    MEM_IO[REG_SOUNDCNT_H / 2] = 0x0002u;
    /* NR21/NR22 and NR23/NR24 share halfwords: duty 50% + constant vol 15,
     * then frequency 0x700 with trigger. */
    MEM_IO[REG_NR21_NR22 / 2] = 0xF080u;
    MEM_IO[REG_NR23_NR24 / 2] = 0x8700u;

    for (volatile uint32_t i = 0; i < 18000u; i++) {
    }
    report_and_idle(0);
    return 0;
}
