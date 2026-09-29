/* Probe 2: DMA word copy from Game Pak ROM to EWRAM through CPU-driven MMIO.
 *
 * Uses DMA3: DMA0's 27-bit source mask normalizes any cartridge address to
 * the shared-latch path (pinned by the mGBA-suite fixtures), so a ROM source
 * must go through DMA1-3; DMA3 is the conventional choice. Main points DMA3
 * at a 64-word ROM pattern, enables an immediate 32-bit incrementing
 * transfer, polls the enable bit for completion, then checksums the 64
 * destination words and reports the sum in r12 (expected 0xBF8007E0, the
 * exact 32-bit sum of 0xCAFE0000..0xCAFE003F, computed independently of both
 * the ROM and the emulator). Any dropped, duplicated, or reordered beat
 * changes the reported sum.
 */

#include "gba.h"

#define DMA3SAD (*(volatile uint32_t *)0x040000D4)
#define DMA3DAD (*(volatile uint32_t *)0x040000D8)
#define DMA3CNT_L (*(volatile uint16_t *)0x040000DC)
#define DMA3CNT_H (*(volatile uint16_t *)0x040000DE)

#define DMA_ENABLE 0x8000u
#define DMA_WORD 0x0400u

#define WORDS 64u
#define DEST ((volatile uint32_t *)0x02000000)

static const uint32_t pattern[WORDS] = {
    0xCAFE0000, 0xCAFE0001, 0xCAFE0002, 0xCAFE0003,
    0xCAFE0004, 0xCAFE0005, 0xCAFE0006, 0xCAFE0007,
    0xCAFE0008, 0xCAFE0009, 0xCAFE000A, 0xCAFE000B,
    0xCAFE000C, 0xCAFE000D, 0xCAFE000E, 0xCAFE000F,
    0xCAFE0010, 0xCAFE0011, 0xCAFE0012, 0xCAFE0013,
    0xCAFE0014, 0xCAFE0015, 0xCAFE0016, 0xCAFE0017,
    0xCAFE0018, 0xCAFE0019, 0xCAFE001A, 0xCAFE001B,
    0xCAFE001C, 0xCAFE001D, 0xCAFE001E, 0xCAFE001F,
    0xCAFE0020, 0xCAFE0021, 0xCAFE0022, 0xCAFE0023,
    0xCAFE0024, 0xCAFE0025, 0xCAFE0026, 0xCAFE0027,
    0xCAFE0028, 0xCAFE0029, 0xCAFE002A, 0xCAFE002B,
    0xCAFE002C, 0xCAFE002D, 0xCAFE002E, 0xCAFE002F,
    0xCAFE0030, 0xCAFE0031, 0xCAFE0032, 0xCAFE0033,
    0xCAFE0034, 0xCAFE0035, 0xCAFE0036, 0xCAFE0037,
    0xCAFE0038, 0xCAFE0039, 0xCAFE003A, 0xCAFE003B,
    0xCAFE003C, 0xCAFE003D, 0xCAFE003E, 0xCAFE003F,
};

void report_and_idle(uint32_t value);

int main(void) {
    DMA3SAD = (uint32_t)pattern;
    DMA3DAD = (uint32_t)DEST;
    DMA3CNT_L = WORDS;
    DMA3CNT_H = (uint16_t)(DMA_ENABLE | DMA_WORD);

    while (DMA3CNT_H & DMA_ENABLE) {
    }

    uint32_t sum = 0;
    for (uint32_t i = 0; i < WORDS; i++) {
        sum += DEST[i];
    }
    report_and_idle(sum);
    return 0;
}
