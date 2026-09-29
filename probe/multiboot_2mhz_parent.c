/* Probe 63: multiboot transfer in Normal 2 MHz mode, parent side.
 *
 * Same 0x100-byte payload and SWI 25h parameter struct as
 * multiboot_2slave_parent.c but with r1 = 2 (Normal 2 MHz): one 32-bit
 * transfer per word instead of split halves. The HLE continuation drives
 * the real SIO registers over a 2-player cable; the child
 * (multiboot_normal_child.c) mimics slave responses with the agreed
 * parameters. Reports the SWI result (0 = success). Covers the HLE Normal
 * transfer path at its fastest clock, where slave timing is tightest; the
 * 256 KHz branch differs only in the SIOCNT speed bit. HLE boot only.
 */

#include "gba.h"

#define REG_RCNT 0x134

#define PARAMS ((volatile uint8_t *)0x02000000)
#define PAYLOAD ((volatile uint32_t *)0x02000100)

void report_and_idle(uint32_t value);

static void put_byte(uint32_t offset, uint8_t value) {
    PARAMS[offset] = value;
}

static void put_word(uint32_t offset, uint32_t value) {
    PARAMS[offset + 0] = (uint8_t)(value & 0xFFu);
    PARAMS[offset + 1] = (uint8_t)((value >> 8) & 0xFFu);
    PARAMS[offset + 2] = (uint8_t)((value >> 16) & 0xFFu);
    PARAMS[offset + 3] = (uint8_t)((value >> 24) & 0xFFu);
}

int main(void) {
    MEM_IO[REG_RCNT / 2] = 0u;

    for (uint32_t i = 0; i < 64u; i++) {
        PAYLOAD[i] = (i + 1u) * 0x11111111u;
    }

    put_byte(0x14, 0x11u);
    put_byte(0x19, 0x22u);
    put_byte(0x1A, 0x33u);
    put_byte(0x1B, 0x44u);
    put_byte(0x1C, 0x55u);
    put_byte(0x1E, 0x02u);
    put_word(0x20, 0x02000100u);
    put_word(0x24, 0x02000200u);

    register uint32_t r0 __asm__("r0") = 0x02000000u;
    register uint32_t r1 __asm__("r1") = 2u;
    __asm__ volatile (
        "swi #0x250000\n"
        : "+r" (r0)
        : "r" (r1)
        : "memory", "r2", "r3", "r12", "lr"
    );

    report_and_idle(r0);
    return 0;
}
