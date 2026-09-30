/* Probe 27c: 4-player multiplayer exchange, second child.
 *
 * Sends 0x3333 without starting; after the parent-started transfer the
 * child reads the third child's word from SIOMULTI3 (expected 0x4444) and
 * reports it. See multi4_parent.c and validator/probe_cases.rs.
 */

#include "gba.h"

#define REG_SIOCNT 0x128
#define REG_SIOMLT_SEND 0x12A
#define REG_SIOMULTI3 0x126
#define REG_RCNT 0x134

#define SIO_START 0x0080u
#define SIO_MULTIPLAYER 0x2000u
#define SIO_BAUD3 0x0003u

void report_and_idle(uint32_t value);

int main(void) {
    MEM_IO[REG_RCNT / 2] = 0u;
    MEM_IO[REG_SIOMLT_SEND / 2] = 0x3333u;
    MEM_IO[REG_SIOCNT / 2] = (uint16_t)(SIO_MULTIPLAYER | SIO_BAUD3);

    while (MEM_IO[REG_SIOCNT / 2] & SIO_START) {
    }

    report_and_idle(MEM_IO[REG_SIOMULTI3 / 2]);
    return 0;
}
