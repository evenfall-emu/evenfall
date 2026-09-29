/* Probe 5: Direct Sound FIFO streaming through timer DMA.
 *
 * Main fills 1 KiB of EWRAM with alternating 16-byte blocks of +64 and -64,
 * routes FIFO A stereo at 100% volume on timer0, programs DMA1 for repeated
 * special-timing word transfers into FIFO A, and runs timer0 at one overflow
 * per 512 cycles so each pattern byte surfaces as exactly one DAC sample.
 * Then it spins and idles. The validator drains the stream and checks the
 * analytic result (see test/probe_cases.zig): 32768 Hz stereo
 * samples of exactly +256/-256 in runs of 16, i.e. FIFO bytes x 4 through
 * the unbiased part of the mixer. Pattern exhaustion would read zeroed
 * EWRAM, but the spin window ends first; any startup underrun zeros form a
 * skippable head run.
 */

#include "gba.h"

#define REG_SOUNDCNT_L 0x80
#define REG_SOUNDCNT_H 0x82
#define REG_SOUNDCNT_X 0x84
#define REG_TM0CNT_L 0x100
#define REG_TM0CNT_H 0x102
#define REG_DMA1SAD_L 0xBC
#define REG_DMA1SAD_H 0xBE
#define REG_DMA1DAD_L 0xC0
#define REG_DMA1DAD_H 0xC2
#define REG_DMA1CNT_L 0xC4
#define REG_DMA1CNT_H 0xC6

#define FIFO_A 0x040000A0u
#define PATTERN ((volatile uint8_t *)0x02000000)
#define PATTERN_BYTES 1024u
#define BLOCK_BYTES 16u

void report_and_idle(uint32_t value);

int main(void) {
    for (uint32_t i = 0; i < PATTERN_BYTES; i++) {
        PATTERN[i] = ((i / BLOCK_BYTES) & 1) ? 0xC0u : 0x40u;
    }

    MEM_IO[REG_SOUNDCNT_H / 2] = 0x0304u;
    MEM_IO[REG_SOUNDCNT_X / 2] = 0x0080u;

    MEM_IO[REG_DMA1SAD_L / 2] = (uint16_t)0x0000u;
    MEM_IO[REG_DMA1SAD_H / 2] = (uint16_t)0x0200u;
    MEM_IO[REG_DMA1DAD_L / 2] = (uint16_t)0x00A0u;
    MEM_IO[REG_DMA1DAD_H / 2] = (uint16_t)0x0400u;
    MEM_IO[REG_DMA1CNT_L / 2] = 0x0000u;
    MEM_IO[REG_DMA1CNT_H / 2] = 0xB640u;

    MEM_IO[REG_TM0CNT_L / 2] = 0xFE00u;
    MEM_IO[REG_TM0CNT_H / 2] = 0x0080u;

    for (volatile uint32_t i = 0; i < 8000u; i++) {
    }
    report_and_idle(0);
    return 0;
}
