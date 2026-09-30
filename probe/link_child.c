/* Probe 8b: Normal-8 full-duplex link exchange, child side.
 *
 * Sends 0x5A with the external shift clock and polls the start bit for
 * completion, then reports the received byte (expected 0xA5 from
 * link_parent.c). See link_parent.c and validator/probe_cases.rs.
 */

#include "gba.h"

#define REG_SIOCNT 0x128
#define REG_SIODATA8 0x12A
#define REG_RCNT 0x134

#define SIO_START 0x0080u

void report_and_idle(uint32_t value);

int main(void) {
    /* RCNT powers up at 0x8000; Normal mode needs bit 15 clear. */
    MEM_IO[REG_RCNT / 2] = 0u;
    MEM_IO[REG_SIODATA8 / 2] = 0x005Au;
    MEM_IO[REG_SIOCNT / 2] = SIO_START;

    while (MEM_IO[REG_SIOCNT / 2] & SIO_START) {
    }

    report_and_idle(MEM_IO[REG_SIODATA8 / 2] & 0xFFu);
    return 0;
}
