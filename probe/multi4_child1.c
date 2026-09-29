/* Probe 27b: 4-player multiplayer exchange, first child.
 *
 * Sends 0x2222 without starting; after the parent-started transfer the
 * child reads the second child's word from SIOMULTI2 (expected 0x3333) and
 * reports it. See multi4_parent.c and test/probe_cases.zig.
 */

#include "gba.h"

#define REG_SIOCNT 0x128
#define REG_SIOMLT_SEND 0x12A
#define REG_SIOMULTI2 0x124
#define REG_RCNT 0x134

#define SIO_START 0x0080u
#define SIO_MULTIPLAYER 0x2000u
#define SIO_BAUD3 0x0003u

void report_and_idle(uint32_t value);

int main(void) {
    MEM_IO[REG_RCNT / 2] = 0u;
    MEM_IO[REG_SIOMLT_SEND / 2] = 0x2222u;
    MEM_IO[REG_SIOCNT / 2] = (uint16_t)(SIO_MULTIPLAYER | SIO_BAUD3);

    while (MEM_IO[REG_SIOCNT / 2] & SIO_START) {
    }

    report_and_idle(MEM_IO[REG_SIOMULTI2 / 2]);
    return 0;
}
