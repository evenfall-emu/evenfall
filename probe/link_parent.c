/* Probe 8a: Normal-8 full-duplex link exchange, parent side.
 *
 * Sends 0xA5 with the internal shift clock at 256 Kbit/s and polls the
 * start bit for completion, then reports the received byte (expected 0x5A
 * from link_child.c). Runs against link_child.c on a standard 2-player
 * cable via test/probe_cases.zig, which advances both machines on
 * one deterministic timeline with zigba.runLinkedUntil.
 */

#include "gba.h"

#define REG_SIOCNT 0x128
#define REG_SIODATA8 0x12A
#define REG_RCNT 0x134

#define SIO_START 0x0080u
#define SIO_INTERNAL 0x0001u

void report_and_idle(uint32_t value);

int main(void) {
    /* RCNT powers up at 0x8000; Normal mode needs bit 15 clear. */
    MEM_IO[REG_RCNT / 2] = 0u;
    MEM_IO[REG_SIODATA8 / 2] = 0x00A5u;
    MEM_IO[REG_SIOCNT / 2] = (uint16_t)(SIO_INTERNAL | SIO_START);

    while (MEM_IO[REG_SIOCNT / 2] & SIO_START) {
    }

    report_and_idle(MEM_IO[REG_SIODATA8 / 2] & 0xFFu);
    return 0;
}
