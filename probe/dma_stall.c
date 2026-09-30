/* Probe: a DMA burst that holds the bus for over a second until guest DMA
 * ends it.
 *
 * DMA1 and DMA2 refill FIFO A and B from a fixed EWRAM word while timers 0
 * and 1 overflow every three cycles: each four-word refill takes longer
 * than the FIFO it fills lasts, so a request is always pending and the
 * arbiter, which ranks every DMA channel above the CPU, never returns the
 * bus to the CPU. A repeating VBlank DMA0 rewrites TM0CNT and TM1CNT from a
 * table each frame; the table's 64th entry stops both timers, the requests
 * stop and the CPU gets the bus back.
 *
 * Main starts timer 1 last, then counts polls of TM0CNT_H until the timers
 * stop. With the CPU stalled from the first refill, the count stays tiny;
 * a CPU that ran during the burst would count for 64 frames. The report is
 * 1 when the count is below 1000 and the VBlank and DMA0 interrupt flags,
 * raised only while the CPU was stalled, are both set; otherwise it is
 * 0x80000000 plus the count, or 2 when a flag is missing.
 */

#include "gba.h"

#define REG_SOUNDCNT_H 0x82
#define REG_SOUNDCNT_X 0x84
#define REG_DMA0SAD 0xB0
#define REG_DMA0DAD 0xB4
#define REG_DMA0CNT_L 0xB8
#define REG_DMA0CNT_H 0xBA
#define REG_DMA1SAD 0xBC
#define REG_DMA1DAD 0xC0
#define REG_DMA1CNT_H 0xC6
#define REG_DMA2SAD 0xC8
#define REG_DMA2DAD 0xCC
#define REG_DMA2CNT_H 0xD2
#define REG_TM1CNT_L 0x104

#define MEM_IO32 ((volatile uint32_t *)0x04000000)
#define TABLE ((volatile uint32_t *)0x02004000)
#define ENTRIES 64u
#define RUNNING 0x0080FFFDu
#define STOPPED 0x0000FFFDu

void report_and_idle(uint32_t value);

int main(void) {
    for (uint32_t i = 0; i < ENTRIES; i++) {
        uint32_t timer = i + 1 < ENTRIES ? RUNNING : STOPPED;
        TABLE[2 * i] = timer;
        TABLE[2 * i + 1] = timer;
    }
    MEM_IO[REG_DISPSTAT / 2] = 1u << 3;

    /* DMA0: VBlank, IRQ, 32-bit, repeat, source increment, destination
     * increment/reload over TM0CNT and TM1CNT. */
    MEM_IO32[REG_DMA0SAD / 4] = (uint32_t)TABLE;
    MEM_IO32[REG_DMA0DAD / 4] = 0x04000100u;
    MEM_IO[REG_DMA0CNT_L / 2] = 2;
    MEM_IO[REG_DMA0CNT_H / 2] = 0xD660u;

    /* FIFO A on timer 0, FIFO B on timer 1, both reset; DMA1/DMA2: FIFO
     * timing, 32-bit, repeat, fixed source. */
    MEM_IO[REG_SOUNDCNT_X / 2] = 0x0080u;
    MEM_IO[REG_SOUNDCNT_H / 2] = 0xFB00u;
    MEM_IO32[REG_DMA1SAD / 4] = 0x02002000u;
    MEM_IO32[REG_DMA1DAD / 4] = 0x040000A0u;
    MEM_IO[REG_DMA1CNT_H / 2] = 0xB740u;
    MEM_IO32[REG_DMA2SAD / 4] = 0x02002000u;
    MEM_IO32[REG_DMA2DAD / 4] = 0x040000A4u;
    MEM_IO[REG_DMA2CNT_H / 2] = 0xB740u;

    MEM_IO32[REG_TM0CNT_L / 4] = RUNNING;
    MEM_IO32[REG_TM1CNT_L / 4] = RUNNING;

    uint32_t polls = 0;
    while (MEM_IO[REG_TM0CNT_H / 2] & TM_ENABLE) {
        polls++;
    }
    if (polls >= 1000u) {
        report_and_idle(0x80000000u + polls);
    }
    if ((MEM_IO[REG_IF / 2] & 0x0101u) != 0x0101u) {
        report_and_idle(2);
    }
    report_and_idle(1);
    return 0;
}
