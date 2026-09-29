/* Probe: STOP woken by SIO completion (multiplayer slave).
 *
 * Readies 0xABCD in SIOMLT_SEND, selects multiplayer mode at baud 3 with
 * the SIO completion IRQ enabled (SIOCNT 0x6003), enables IRQ bit 7 with
 * IME left off, then executes STOP (SWI 3). The parent
 * (multi_parent.c, reused — it sends 0x1234 and starts the transfer)
 * completes the exchange; the SIO completion flag wakes the CPU, which
 * resumes past the SWI with no ISR entered and reports SIOMULTI0
 * (expected 0x1234). Wakeup is level-triggered (IE and IF), so parent and
 * child setup order does not matter. Covers STOP entry plus the SIO wake
 * source end to end — the combination real multiboot slaves use; HALT and
 * STOP+keypad stay with the halt_stop probe. HLE boot only.
 */

#include "gba.h"

#define REG_SIOCNT 0x128
#define REG_SIOMLT_SEND 0x12A
#define REG_SIOMULTI0 0x120
#define REG_RCNT 0x134
#define REG_IE 0x200
#define REG_IF 0x202
#define REG_IME 0x208

#define SIO_MULTIPLAYER 0x2000u
#define SIO_BAUD3 0x0003u
#define SIO_IRQ 0x4000u
#define IRQ_SIO 0x0080u

void report_and_idle(uint32_t value);

int main(void) {
    MEM_IO[REG_RCNT / 2] = 0u;
    MEM_IO[REG_SIOMLT_SEND / 2] = 0xABCDu;
    MEM_IO[REG_SIOCNT / 2] = (uint16_t)(SIO_MULTIPLAYER | SIO_BAUD3 | SIO_IRQ);
    MEM_IO[REG_IF / 2] = IRQ_SIO;
    MEM_IO[REG_IE / 2] = IRQ_SIO;
    MEM_IO[REG_IME / 2] = 0u;

    __asm__ volatile ("swi 0x030000");

    report_and_idle(MEM_IO[REG_SIOMULTI0 / 2]);
    return 0;
}
