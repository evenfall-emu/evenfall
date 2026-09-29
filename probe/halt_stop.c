/* Probe 33: HALT timer wake followed by STOP keypad wake.
 *
 * Phase one programs timer0 for a fast overflow with IRQ enabled and
 * executes HALT (SWI 2); the overflow wakes the CPU and enters the HLE
 * trampoline ISR. Phase two selects button A in KEYCNT and executes STOP
 * (SWI 3); the validator presses A, waking the CPU into the same ISR. The
 * ISR counts entries and acknowledges both timer0 and keypad flags,
 * returning with `mov pc, lr`. Main reports the count (expected: 2, one per
 * phase). Covers HALT/STOP entry, distinct wake rules, and IRQ entry on a
 * STOP wake end to end. HLE boot only.
 */

#include "gba.h"

#define REG_IE 0x200
#define REG_IF 0x202
#define REG_IME 0x208
#define REG_TM0CNT_L 0x100
#define REG_TM0CNT_H 0x102
#define REG_KEYCNT 0x132
#define IRQ_KEYPAD 0x1000u
#define COUNTER_ADDR ((volatile uint32_t *)0x02000000)

__attribute__((naked)) void power_isr(void) {
    __asm__ volatile (
        "ldr r0, =0x02000000\n"
        "ldr r1, [r0]\n"
        "add r1, r1, #1\n"
        "str r1, [r0]\n"
        "ldr r0, =0x04000202\n"
        "ldr r1, =0x1008\n"
        "strh r1, [r0]\n"
        "mov pc, lr\n"
        ".ltorg\n"
    );
}

void report_and_idle(uint32_t value);

int main(void) {
    *COUNTER_ADDR = 0;
    *IRQ_HANDLER_ADDR = (uint32_t)power_isr;

    MEM_IO[REG_IF / 2] = (uint16_t)(IRQ_TIMER0 | IRQ_KEYPAD);
    MEM_IO[REG_IE / 2] = (uint16_t)(IRQ_TIMER0 | IRQ_KEYPAD);
    MEM_IO[REG_IME / 2] = 1u;

    MEM_IO[REG_TM0CNT_L / 2] = 0xFF00u;
    MEM_IO[REG_TM0CNT_H / 2] = 0x00C0u;
    /* GBA SWI numbers live in instruction bits 23-16 (as jsmolka, cult,
     * and mGBA agree); plain `swi 2` would dispatch service 0. */
    __asm__ volatile ("swi 0x020000");

    MEM_IO[REG_TM0CNT_H / 2] = 0u;
    MEM_IO[REG_KEYCNT / 2] = 0x4001u;
    __asm__ volatile ("swi 0x030000");

    MEM_IO[REG_KEYCNT / 2] = 0u;
    MEM_IO[REG_IME / 2] = 0u;
    report_and_idle(*COUNTER_ADDR);
    return 0;
}
