/* Timer start delay and reload latency, read by the CPU.
 *
 * The mGBA suite's "Timer IRQ tests" (src/timer-irq.c) run this sequence of
 * ARM code from IWRAM and record on hardware what the final load reads:
 *
 *     str  r3, [r1]      @ TM0CNT = reload V | enable (and IRQ) << 16
 *     strh r2, [r1]      @ TM0CNT_L = 0, the reload for every later overflow
 *     nop                @ k times, 0 to 9
 *     ldrh r0, [r1]      @ read TM0CNT_L
 *
 * This probe writes the same instructions into IWRAM for every V from FFFFh
 * down to FFF7h and every k from 0 to 9, with the IRQ off, and stores what
 * each load reads in `timer_reads` (EWRAM), row by row. Timer 0 counts once
 * per cycle (prescaler 1) and every instruction here is one IWRAM access
 * long, so each NOP adds exactly one to the read. The validator compares
 * the table with the suite's recorded rows, extended past the point where
 * the suite's IRQ stops the timer.
 *
 * Reports the number of entries, 90.
 */

#include "gba.h"

#define ROWS 9
#define NOPS 10

#define ARM_STR_R3_R1 0xE5813000u
#define ARM_STRH_R2_R1 0xE1C120B0u
#define ARM_NOP 0xE1A00000u
#define ARM_LDRH_R0_R1 0xE1D100B0u
#define ARM_BX_LR 0xE12FFF1Eu

uint16_t timer_reads[ROWS * NOPS];

void report_and_idle(uint32_t value);

typedef uint32_t (*Sequence)(uint32_t r0, uint32_t r1, uint32_t r2, uint32_t r3);

int main(void) {
    volatile uint32_t *code = (volatile uint32_t *)MEM_IWRAM;
    Sequence run = (Sequence)MEM_IWRAM;
    for (uint32_t row = 0; row < ROWS; row++) {
        uint32_t reload = 0xFFFFu - row;
        for (uint32_t nops = 0; nops < NOPS; nops++) {
            uint32_t n = 0;
            code[n++] = ARM_STR_R3_R1;
            code[n++] = ARM_STRH_R2_R1;
            for (uint32_t i = 0; i < nops; i++) code[n++] = ARM_NOP;
            code[n++] = ARM_LDRH_R0_R1;
            code[n++] = ARM_BX_LR;
            uint32_t read = run(0, 0x04000100u, 0, (uint32_t)TM_ENABLE << 16 | reload);
            MEM_IO[REG_TM0CNT_H / 2] = 0;
            timer_reads[row * NOPS + nops] = (uint16_t)read;
        }
    }
    report_and_idle(ROWS * NOPS);
    return 0;
}
