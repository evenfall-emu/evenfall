/* Probe 23: Yoshi tilt-sensor two-axis sample through SRAM registers.
 *
 * Issues the 55h/AAh arm-and-latch command pair, then reads X bits 0-7,
 * X bits 8-11 with the ready flag in bit 7, and Y bits 0-7/8-11, packing
 * all four bytes into r12. The validator injects axes x=0x392/y=0x3A0 and
 * expects 0x03A08392 (see docs/hardware/cartridge.md for the register
 * map). Covers the tilt command latch, byte-wide conversion reads, and
 * deterministic peripheral injection end to end. HLE boot only.
 */

#include "gba.h"
#include <stdint.h>

#define TILT_CMD_0 ((volatile uint8_t *)0x0E008000)
#define TILT_CMD_1 ((volatile uint8_t *)0x0E008100)
#define TILT_X_LO ((volatile uint8_t *)0x0E008200)
#define TILT_X_HI ((volatile uint8_t *)0x0E008300)
#define TILT_Y_LO ((volatile uint8_t *)0x0E008400)
#define TILT_Y_HI ((volatile uint8_t *)0x0E008500)

void report_and_idle(uint32_t value);

int main(void) {
    *TILT_CMD_0 = 0x55u;
    *TILT_CMD_1 = 0xAAu;

    uint32_t packed = (uint32_t)*TILT_X_LO |
        ((uint32_t)*TILT_X_HI << 8) |
        ((uint32_t)*TILT_Y_LO << 16) |
        ((uint32_t)*TILT_Y_HI << 24);
    report_and_idle(packed);
    return 0;
}
