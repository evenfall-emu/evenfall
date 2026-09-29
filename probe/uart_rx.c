/* Probe 27b: UART byte stream, receiver side.
 *
 * Readies UART reception with SIOCNT 0x3883, then collects three bytes,
 * polling the RX-empty status bit (SIOCNT bit 5) clear before each SIODATA8
 * read. Packs them little-endian into r12 (expected 0x3C5AA5 for the
 * transmitter's 0xA5, 0x5A, 0x3C). See uart_tx.c and
 * test/probe_cases.zig.
 */

#include "gba.h"

#define REG_SIOCNT 0x128
#define REG_SIODATA8 0x12A
#define REG_RCNT 0x134

#define SIO_RX_EMPTY 0x0020u

void report_and_idle(uint32_t value);

int main(void) {
    MEM_IO[REG_RCNT / 2] = 0u;
    MEM_IO[REG_SIOCNT / 2] = 0x3883u;

    uint32_t packed = 0;
    for (uint32_t i = 0; i < 3u; i++) {
        while (MEM_IO[REG_SIOCNT / 2] & SIO_RX_EMPTY) {
        }
        packed |= (uint32_t)(MEM_IO[REG_SIODATA8 / 2] & 0xFFu) << (i * 8u);
    }
    report_and_idle(packed);
    return 0;
}
