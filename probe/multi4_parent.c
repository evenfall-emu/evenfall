/* Probe 27a: 4-player multiplayer exchange, parent side.
 *
 * Sends 0x1111 and starts a baud-3 multiplayer transfer with three children
 * attached. After completion the parent reads the third child's word from
 * SIOMULTI3 (expected 0x4444) and reports it. Runs with multi4_child1.c,
 * multi4_child2.c, and multi4_child3.c via validator/probe_cases.rs.
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
    MEM_IO[REG_SIOMLT_SEND / 2] = 0x1111u;
    MEM_IO[REG_SIOCNT / 2] = (uint16_t)(SIO_MULTIPLAYER | SIO_BAUD3 | SIO_START);

    while (MEM_IO[REG_SIOCNT / 2] & SIO_START) {
    }

    report_and_idle(MEM_IO[REG_SIOMULTI3 / 2]);
    return 0;
}
