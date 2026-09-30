/* How soon after a timer overflow the CPU takes its IRQ.
 *
 * The mGBA suite's "Timer IRQ tests" (src/timer-irq.c) enable timer 0 from
 * reload V with its IRQ on, from ARM code in IWRAM, and read the counter
 * after k NOPs. On hardware the read returns the handler's stopped value
 * once the IRQ preempts the load: for V = FFFFh from k = 4, and one NOP
 * later for each step down to FFFAh. Where that happens depends only on the
 * start delay, the overflow and the IRQ latency, not on the BIOS.
 *
 * This probe writes the same start sequence into IWRAM, followed by
 * one-cycle `add r5, r5, #1` instructions in place of the NOPs:
 *
 *     mov  r5, #0
 *     str  r3, [r1]      @ TM0CNT = reload V | (enable | IRQ) << 16
 *     strh r2, [r1]      @ TM0CNT_L = 0
 *     add  r5, r5, #1    @ 20 times
 *
 * The IRQ handler stores r5, the count of additions the CPU completed
 * before it took the IRQ, and stops the timer. The counts for V from FFFFh
 * to FFFAh go to `irq_counts` (EWRAM). The suite's table fixes them at 4 to
 * 9, one more for each step down of V.
 *
 * Reports the number of entries, 6.
 */

#include "gba.h"

#define ROWS 6
#define ADDS 20

#define ARM_PUSH_R5_LR 0xE92D4020u
#define ARM_MOV_R5_0 0xE3A05000u
#define ARM_STR_R3_R1 0xE5813000u
#define ARM_STRH_R2_R1 0xE1C120B0u
#define ARM_ADD_R5_1 0xE2855001u
#define ARM_POP_R5_LR 0xE8BD4020u
#define ARM_BX_LR 0xE12FFF1Eu

uint32_t irq_counts[ROWS];
volatile uint32_t irq_count;

void timer_isr(void);
void report_and_idle(uint32_t value);

/* The handler reads r5 as the interrupted code left it: the BIOS IRQ
 * trampoline saves and uses only r0-r3, r12 and lr. */
__asm__(
    ".arm\n"
    ".global timer_isr\n"
    "timer_isr:\n"
    "    ldr r0, =irq_count\n"
    "    str r5, [r0]\n"
    "    ldr r0, =0x04000100\n"
    "    mov r1, #0\n"
    "    strh r1, [r0, #2]\n"
    "    ldr r0, =0x04000200\n"
    "    mov r1, #8\n"
    "    strh r1, [r0, #2]\n"
    "    bx lr\n"
    ".ltorg\n");

typedef void (*Sequence)(uint32_t r0, uint32_t r1, uint32_t r2, uint32_t r3);

int main(void) {
    volatile uint32_t *code = (volatile uint32_t *)MEM_IWRAM;
    Sequence run = (Sequence)MEM_IWRAM;
    uint32_t n = 0;
    code[n++] = ARM_PUSH_R5_LR;
    code[n++] = ARM_MOV_R5_0;
    code[n++] = ARM_STR_R3_R1;
    code[n++] = ARM_STRH_R2_R1;
    for (uint32_t i = 0; i < ADDS; i++) code[n++] = ARM_ADD_R5_1;
    code[n++] = ARM_POP_R5_LR;
    code[n++] = ARM_BX_LR;

    *IRQ_HANDLER_ADDR = (uint32_t)timer_isr;
    MEM_IO[REG_IF / 2] = IRQ_TIMER0;
    MEM_IO[REG_IE / 2] = IRQ_TIMER0;
    MEM_IO[REG_IME / 2] = 1;
    for (uint32_t row = 0; row < ROWS; row++) {
        irq_count = 0xFFFFFFFFu;
        run(0, 0x04000100u, 0, (uint32_t)(TM_ENABLE | TM_IRQ) << 16 | (0xFFFFu - row));
        irq_counts[row] = irq_count;
    }
    MEM_IO[REG_IME / 2] = 0;
    report_and_idle(ROWS);
    return 0;
}
