/* Probe: LCD interrupts counted over exactly two frames.
 *
 * Enables the VBlank, HBlank and VCount-match (line 100) IRQs; one ISR,
 * called through the HLE trampoline, counts each source and acknowledges
 * exactly the IF bits it saw. Main brackets two whole frames between entries
 * into line 80, where no LCD interrupt fires near the start of the line, and
 * reports the counts inside the bracket packed as
 *     hblank << 16 | vcount << 8 | vblank.
 * GBATEK's DISPSTAT description flags HBlank on every one of the 228 lines,
 * VBlank lines included, and VCount and VBlank once per frame, so the report
 * is 456 << 16 | 2 << 8 | 2 = 0x01C80202. HBlank on visible lines only would
 * report 320; a match or VBlank IRQ that refires within its line would
 * inflate its count.
 */

#include "gba.h"

#define REG_VCOUNT 0x06
#define IRQ_HBLANK 0x0002u
#define IRQ_VCOUNT 0x0004u
#define DISPSTAT_IRQS 0x0038u
#define MATCH_LINE 100u
#define BRACKET_LINE 80u

static volatile uint32_t hblanks;
static volatile uint32_t vcounts;
static volatile uint32_t vblanks;

static void lcd_isr(void) {
    uint16_t pending = MEM_IO[REG_IF / 2] & MEM_IO[REG_IE / 2];
    if (pending & IRQ_HBLANK) hblanks++;
    if (pending & IRQ_VCOUNT) vcounts++;
    if (pending & IRQ_VBLANK) vblanks++;
    MEM_IO[REG_IF / 2] = pending;
}

static void enter_bracket_line(void) {
    while (MEM_IO[REG_VCOUNT / 2] == BRACKET_LINE) {
    }
    while (MEM_IO[REG_VCOUNT / 2] != BRACKET_LINE) {
    }
}

void report_and_idle(uint32_t value);

int main(void) {
    hblanks = 0;
    vcounts = 0;
    vblanks = 0;
    *IRQ_HANDLER_ADDR = (uint32_t)lcd_isr;

    MEM_IO[REG_DISPSTAT / 2] = (uint16_t)(MATCH_LINE << 8 | DISPSTAT_IRQS);
    MEM_IO[REG_IF / 2] = IRQ_VBLANK | IRQ_HBLANK | IRQ_VCOUNT;
    MEM_IO[REG_IE / 2] = IRQ_VBLANK | IRQ_HBLANK | IRQ_VCOUNT;
    MEM_IO[REG_IME / 2] = 1u;

    enter_bracket_line();
    uint32_t h = hblanks, v = vcounts, b = vblanks;
    enter_bracket_line();
    enter_bracket_line();
    h = hblanks - h;
    v = vcounts - v;
    b = vblanks - b;

    MEM_IO[REG_IME / 2] = 0u;
    report_and_idle(h << 16 | v << 8 | b);
    return 0;
}
