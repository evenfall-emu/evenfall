/* Minimal GBA register map for zigba probe ROMs. Freestanding: no libgba. */

#ifndef PROBE_GBA_H
#define PROBE_GBA_H

#include <stdint.h>

#define MEM_IO      ((volatile uint16_t *)0x04000000)
#define MEM_IWRAM   ((uint32_t *)0x03000000)

#define REG_DISPSTAT 0x04
#define REG_IE       0x200
#define REG_IF       0x202
#define REG_IME      0x208

#define REG_TM0CNT_L 0x100
#define REG_TM0CNT_H 0x102

#define IRQ_VBLANK 0x0001
#define IRQ_TIMER0 0x0008

#define TM_ENABLE 0x80
#define TM_IRQ    0x40

/* HLE IRQ trampoline: user handler address observed by the BIOS mirror. */
#define IRQ_HANDLER_ADDR ((volatile uint32_t *)0x03007FFC)

#endif
