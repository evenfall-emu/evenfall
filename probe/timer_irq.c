/* Probe 1: timer overflow IRQs and cascade counting.
 *
 * Timer0 runs at the /64 prescaler from reload 0xFFF0 (a 1024-cycle period)
 * with its IRQ enabled. Timer1 counts timer0 overflows in cascade mode from
 * reload 0xFFFB, so it overflows on every fifth one, also with its IRQ
 * enabled. Timer2 cascades from timer1 without an IRQ. One ISR, called
 * through the HLE trampoline at 0x03007FFC, counts IRQs per source and
 * acknowledges exactly the IF bits it saw.
 *
 * Main waits for the second timer1 IRQ, stops the timers and reports
 *     timer0 IRQs << 16 | timer1 IRQs << 8 | TM2CNT_L.
 * GBATEK's cascade rule fixes every field: two timer1 overflows take exactly
 * ten timer0 overflows, and timer2 counted both, so the report is 0x000A0202.
 * A missed or doubled IRQ, an IF bit that refires after acknowledgement, or
 * a cascade timer counting cycles instead of overflows each change it. The
 * next timer0 overflow is 1024 cycles away when main stops the timers, far
 * beyond its reaction time.
 */

#include "gba.h"

#define REG_TM1CNT_L 0x104
#define REG_TM1CNT_H 0x106
#define REG_TM2CNT_L 0x108
#define REG_TM2CNT_H 0x10A
#define IRQ_TIMER1 0x0010u
#define TM_CASCADE 0x04u
#define TM_PRESCALE_64 0x01u

static volatile uint32_t timer0_irqs;
static volatile uint32_t timer1_irqs;

static void timer_isr(void) {
    uint16_t pending = MEM_IO[REG_IF / 2] & MEM_IO[REG_IE / 2];
    if (pending & IRQ_TIMER0) timer0_irqs++;
    if (pending & IRQ_TIMER1) timer1_irqs++;
    MEM_IO[REG_IF / 2] = pending;
}

void report_and_idle(uint32_t value);

int main(void) {
    timer0_irqs = 0;
    timer1_irqs = 0;
    *IRQ_HANDLER_ADDR = (uint32_t)timer_isr;

    MEM_IO[REG_TM0CNT_L / 2] = 0xFFF0u;
    MEM_IO[REG_TM1CNT_L / 2] = 0xFFFBu;
    MEM_IO[REG_TM2CNT_L / 2] = 0u;
    MEM_IO[REG_IF / 2] = IRQ_TIMER0 | IRQ_TIMER1;
    MEM_IO[REG_IE / 2] = IRQ_TIMER0 | IRQ_TIMER1;
    MEM_IO[REG_IME / 2] = 1u;
    MEM_IO[REG_TM2CNT_H / 2] = (uint16_t)(TM_ENABLE | TM_CASCADE);
    MEM_IO[REG_TM1CNT_H / 2] = (uint16_t)(TM_ENABLE | TM_IRQ | TM_CASCADE);
    MEM_IO[REG_TM0CNT_H / 2] = (uint16_t)(TM_ENABLE | TM_IRQ | TM_PRESCALE_64);

    while (timer1_irqs < 2u) {
    }

    MEM_IO[REG_TM0CNT_H / 2] = 0u;
    MEM_IO[REG_IME / 2] = 0u;
    report_and_idle(timer0_irqs << 16 | timer1_irqs << 8 | MEM_IO[REG_TM2CNT_L / 2]);
    return 0;
}
