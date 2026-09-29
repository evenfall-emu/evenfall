/* Probe 27a: UART byte stream, transmitter side.
 *
 * Sends 0xA5, 0x5A, 0x3C with no FIFO at baud 3, polling the TX-busy status
 * bit (SIOCNT bit 4) clear before each SIODATA8 write and setting SIOCNT to
 * 0x3483 per byte. The receiver (uart_rx.c) enforces pacing through CTS.
 * Runs against uart_rx.c on a 2-player cable via
 * test/probe_cases.zig. Reports 0.
 */

#include "gba.h"

#define REG_SIOCNT 0x128
#define REG_SIODATA8 0x12A
#define REG_RCNT 0x134

#define SIO_TX_BUSY 0x0010u

void report_and_idle(uint32_t value);

int main(void) {
    MEM_IO[REG_RCNT / 2] = 0u;
    /* Mode first: entering UART clears FIFO state, so the data byte must
     * follow each SIOCNT setup, not precede it. */
    MEM_IO[REG_SIOCNT / 2] = 0x3483u;

    static const uint8_t bytes[3] = { 0xA5u, 0x5Au, 0x3Cu };
    for (uint32_t i = 0; i < 3u; i++) {
        while (MEM_IO[REG_SIOCNT / 2] & SIO_TX_BUSY) {
        }
        MEM_IO[REG_SIODATA8 / 2] = bytes[i];
    }
    while (MEM_IO[REG_SIOCNT / 2] & SIO_TX_BUSY) {
    }
    report_and_idle(0);
    return 0;
}
