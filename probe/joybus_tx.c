/* Probe 40: JoyBus slave transmit with the validator as external master.
 *
 * Selects JoyBus mode, publishes 0x44332211 in JOY_TRANS, and polls the
 * JOYSTAT transmit flag (bit 3) until the master's READ-from-GBA command
 * carries it away. The validator schedules that command after setup, reads
 * the five-byte master response itself (0x11, 0x22, 0x33, 0x44, 0x08), and
 * requires the ROM to have observed flag clearance and idled with r12 0.
 * Completes both JoyBus transfer directions at ROM level alongside the
 * slave-receive probe. HLE boot only.
 */

#include "gba.h"

#define REG_RCNT 0x134
#define REG_JOYCNT 0x140
#define REG_JOYSTAT 0x158
#define REG_JOY_TRANS 0x154

#define JOYSTAT_TRANS 0x0008u

void report_and_idle(uint32_t value);

int main(void) {
    MEM_IO[REG_RCNT / 2] = 0xC000u;
    MEM_IO[REG_JOYCNT / 2] = 0x0040u;
    *(volatile uint32_t *)0x04000154u = 0x44332211u;

    while ((MEM_IO[REG_JOYSTAT / 2] & JOYSTAT_TRANS) != 0) {
    }
    report_and_idle(0);
    return 0;
}
