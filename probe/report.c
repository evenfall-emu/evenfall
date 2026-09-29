/* Shared probe reporting: result in r12 at the fixed idle loop observed by
 * the validators (0x08000300 entry, 0x08000304 self-loop; see rom.ld). */

#include <stdint.h>

__attribute__((naked, section(".idle"))) void report_and_idle(uint32_t value) {
    (void)value;
    __asm__ volatile (
        "mov r12, r0\n"
        "idle_loop:\n"
        "b idle_loop\n"
    );
}
