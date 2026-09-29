/* Probe 9: EEPROM serial block write/read through DMA3 with runtime save
 * autodetection.
 *
 * Writes 8 bytes to 512 B EEPROM block 5 (10 + 6-bit address + 64 data bits
 * + stop = 73 DMA3 units), polls the busy status bit to ready, reads the
 * block back (11 + address + stop = 9 units out, 68 units in), and reports 0
 * if the round-tripped bytes match (1 = busy timeout, 2 = data mismatch).
 * No save type is configured anywhere: the machine must resolve eeprom_512
 * from the DMA3 command lengths, which the validator asserts through
 * Machine.saveType().
 */

#include "gba.h"

#define REG_DMA3SAD_L 0xD4
#define REG_DMA3SAD_H 0xD6
#define REG_DMA3DAD_L 0xD8
#define REG_DMA3DAD_H 0xDA
#define REG_DMA3CNT_L 0xDC
#define REG_DMA3CNT_H 0xDE

#define DMA_ENABLE 0x8000u
#define DMA_DST_FIXED 0x0040u
#define DMA_SRC_FIXED 0x0100u

#define EEPROM_PORT 0x0D000000u
#define WRITE_BASE ((volatile uint16_t *)0x02001000)
#define READ_BASE ((volatile uint16_t *)0x02001000)
#define FETCH_BASE ((volatile uint16_t *)0x02001100)

#define BLOCK 5u
#define POLL_BOUND 200000u

static const uint8_t payload[8] = { 0x01, 0x23, 0x45, 0x67, 0x89, 0xAB, 0xCD, 0xEF };

void report_and_idle(uint32_t value);

static void put_bit(volatile uint16_t *base, uint32_t *index, uint32_t bit) {
    base[*index] = (uint16_t)bit;
    *index += 1;
}

static void run_dma3(uint32_t src, uint32_t dst, uint16_t count, uint16_t addr_control) {
    MEM_IO[REG_DMA3SAD_L / 2] = (uint16_t)src;
    MEM_IO[REG_DMA3SAD_H / 2] = (uint16_t)(src >> 16);
    MEM_IO[REG_DMA3DAD_L / 2] = (uint16_t)dst;
    MEM_IO[REG_DMA3DAD_H / 2] = (uint16_t)(dst >> 16);
    MEM_IO[REG_DMA3CNT_L / 2] = count;
    MEM_IO[REG_DMA3CNT_H / 2] = (uint16_t)(DMA_ENABLE | addr_control);
}

int main(void) {
    uint32_t n = 0;
    put_bit(WRITE_BASE, &n, 1);
    put_bit(WRITE_BASE, &n, 0);
    for (uint32_t i = 0; i < 6u; i++) {
        put_bit(WRITE_BASE, &n, (BLOCK >> (5u - i)) & 1u);
    }
    for (uint32_t i = 0; i < 8u; i++) {
        for (uint32_t b = 0; b < 8u; b++) {
            put_bit(WRITE_BASE, &n, (payload[i] >> (7u - b)) & 1u);
        }
    }
    put_bit(WRITE_BASE, &n, 0);
    run_dma3(0x02001000u, EEPROM_PORT, (uint16_t)n, DMA_DST_FIXED);

    uint32_t polls = 0;
    while (((*(volatile uint16_t *)0x0D000000u) & 1u) == 0) {
        polls++;
        if (polls > POLL_BOUND) {
            report_and_idle(1);
            return 0;
        }
    }

    n = 0;
    put_bit(READ_BASE, &n, 1);
    put_bit(READ_BASE, &n, 1);
    for (uint32_t i = 0; i < 6u; i++) {
        put_bit(READ_BASE, &n, (BLOCK >> (5u - i)) & 1u);
    }
    put_bit(READ_BASE, &n, 0);
    run_dma3(0x02001000u, EEPROM_PORT, (uint16_t)n, DMA_DST_FIXED);

    run_dma3(EEPROM_PORT, 0x02001100u, 68u, DMA_SRC_FIXED);

    for (uint32_t i = 0; i < 4u; i++) {
        if ((FETCH_BASE[i] & 1u) != 0) {
            report_and_idle(2);
            return 0;
        }
    }
    for (uint32_t i = 0; i < 8u; i++) {
        uint32_t byte = 0;
        for (uint32_t b = 0; b < 8u; b++) {
            byte = (byte << 1) | (FETCH_BASE[4 + i * 8 + b] & 1u);
        }
        if (byte != payload[i]) {
            report_and_idle(2);
            return 0;
        }
    }
    report_and_idle(0);
    return 0;
}
