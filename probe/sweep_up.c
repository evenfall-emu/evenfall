/* Probe 66: square channel 1 frequency sweep, upward or with SWEEP_DOWN
 * downward (built as sweep_down.c).
 *
 * Channel 1 triggers with sweep period 2 and shift 1 at constant volume 15,
 * 50% duty, no length check. Upward: frequency 0x080 in addition mode
 * (NR10 0x21), each update multiplying the frequency by 1.5 with no
 * overflow within the ~1.2M-cycle spin. Downward: frequency 0x700 in
 * subtraction mode (NR10 0x29), each update roughly halving it (1792, 896,
 * 448, ...), which can never overflow. Updates land every 262144 cycles.
 * The validator checks that complete runs change monotonically in the
 * sweep's direction (a mid-run update only cuts a run short) and by at
 * least 1.25x overall.
 */

#include "gba.h"

#define REG_NR10 0x60
#define REG_NR11_NR12 0x62
#define REG_NR13_NR14 0x64
#define REG_SOUNDCNT_L 0x80
#define REG_SOUNDCNT_H 0x82
#define REG_SOUNDCNT_X 0x84

void report_and_idle(uint32_t value);

int main(void) {
    /* Master enable first: PSG writes while NR52 bit 7 is clear are lost. */
    MEM_IO[REG_SOUNDCNT_X / 2] = 0x0080u;
    MEM_IO[REG_SOUNDCNT_L / 2] = 0xFF77u;
    MEM_IO[REG_SOUNDCNT_H / 2] = 0x0002u;
#ifdef SWEEP_DOWN
    MEM_IO[REG_NR10 / 2] = 0x0029u;
    MEM_IO[REG_NR11_NR12 / 2] = 0xF080u;
    MEM_IO[REG_NR13_NR14 / 2] = 0x8700u;
#else
    MEM_IO[REG_NR10 / 2] = 0x0021u;
    MEM_IO[REG_NR11_NR12 / 2] = 0xF080u;
    MEM_IO[REG_NR13_NR14 / 2] = 0x8080u;
#endif

    for (volatile uint32_t i = 0; i < 28000u; i++) {
    }
    report_and_idle(0);
    return 0;
}
