/* Probe: square channel 2 envelope decay (constant frequency).
 *
 * Channel 2 triggers at frequency 0x700 with initial volume 15, envelope
 * decrease, period 1 (NR22 0xF1), 50% duty, no length check. Envelope
 * steps land every 262144 cycles, dropping the volume 15, 14, 13, ...;
 * the ~1.2M-cycle spin covers about four steps. The validator drains the
 * stream and checks run amplitudes never rise, start at 240, span at
 * least three distinct levels, and reach 208 or below. Covers envelope
 * volume progression end to end; envelope disable/zero stays a host unit
 * test. HLE boot only.
 */

#include "gba.h"

#define REG_NR21_NR22 0x68
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
    MEM_IO[REG_NR21_NR22 / 2] = 0xF180u;
    MEM_IO[REG_NR23_NR24 / 2] = 0x8700u;

    for (volatile uint32_t i = 0; i < 28000u; i++) {
    }
    report_and_idle(0);
    return 0;
}
