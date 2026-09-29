/* Probe 7: keypad IRQ edges counted over validator-driven presses.
 *
 * Main selects the A button in KEYCNT with IRQ enabled (OR mode), enables
 * the keypad IRQ line and master IRQs, then spins until START (not selected
 * in KEYCNT, so silent) reads pressed and reports the ISR's count. The
 * validator presses and releases A three times, each held for 2000
 * instructions, before pressing START. Each press turns the keypad condition
 * true once and must raise exactly one IRQ, which the ISR counts and
 * acknowledges: the expected report is 3. A condition that refires while A
 * stays held, or fires on release, reports more.
 */

#include "gba.h"

#define REG_KEYINPUT 0x130
#define REG_KEYCNT 0x132
#define REG_IE 0x200
#define REG_IF 0x202
#define REG_IME 0x208
#define IRQ_KEYPAD 0x1000u
#define KEY_A 0x0001u
#define KEY_START 0x0008u
#define KEYCNT_ENABLE 0x4000u
#define COUNTER_ADDR ((volatile uint32_t *)0x02000000)

__attribute__((naked)) void keypad_isr(void) {
    __asm__ volatile (
        "ldr r0, =0x02000000\n"
        "ldr r1, [r0]\n"
        "add r1, r1, #1\n"
        "str r1, [r0]\n"
        "ldr r0, =0x04000202\n"
        "ldr r1, =0x1000\n"
        "strh r1, [r0]\n"
        "mov pc, lr\n"
        ".ltorg\n"
    );
}

void report_and_idle(uint32_t value);

int main(void) {
    *COUNTER_ADDR = 0;
    *IRQ_HANDLER_ADDR = (uint32_t)keypad_isr;

    MEM_IO[REG_IF / 2] = IRQ_KEYPAD;
    MEM_IO[REG_KEYCNT / 2] = (uint16_t)(KEYCNT_ENABLE | KEY_A);
    MEM_IO[REG_IE / 2] = IRQ_KEYPAD;
    MEM_IO[REG_IME / 2] = 1u;

    /* KEYINPUT bits read 0 while pressed. */
    while (MEM_IO[REG_KEYINPUT / 2] & KEY_START) {
    }

    MEM_IO[REG_KEYCNT / 2] = 0u;
    MEM_IO[REG_IME / 2] = 0u;
    report_and_idle(*COUNTER_ADDR);
    return 0;
}
