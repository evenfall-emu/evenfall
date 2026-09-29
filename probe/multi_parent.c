/* Probe 10a: multiplayer link exchange, parent side.
 *
 * Writes 0x1234 to SIOMLT_SEND, selects multiplayer mode at baud 3, and sets
 * the start bit. The child (stop_sio_child.c) only readies its send word;
 * the cable synchronizer assigns IDs and completes the shared transfer.
 * After the start bit clears, the parent reads the child's word from
 * SIOMULTI1 (expected 0xABCD) and reports it. Four-player slot distribution
 * stays with the multi4 probes.
 */

#include "gba.h"

#define REG_SIOCNT 0x128
#define REG_SIOMLT_SEND 0x12A
#define REG_SIOMULTI1 0x122
#define REG_RCNT 0x134

#define SIO_START 0x0080u
#define SIO_MULTIPLAYER 0x2000u
#define SIO_BAUD3 0x0003u

void report_and_idle(uint32_t value);

int main(void) {
    MEM_IO[REG_RCNT / 2] = 0u;
    MEM_IO[REG_SIOMLT_SEND / 2] = 0x1234u;
    MEM_IO[REG_SIOCNT / 2] = (uint16_t)(SIO_MULTIPLAYER | SIO_BAUD3 | SIO_START);

    while (MEM_IO[REG_SIOCNT / 2] & SIO_START) {
    }

    report_and_idle(MEM_IO[REG_SIOMULTI1 / 2]);
    return 0;
}
