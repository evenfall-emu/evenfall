/* Probe 32: S3511 date/time write round-trip through GPIO.
 *
 * Sends the date/time write command (0x26) with seven BCD bytes for
 * 2002-12-31 23:59:58 Tuesday, then reads the latch back (0xA6) and compares
 * byte by byte, reporting 0 on a full match (1 = control-command failure
 * preamble, 2+i = byte i mismatch). The write anchors the supplied second
 * at the last payload bit and the read follows thousands of cycles later,
 * far inside the same second, so the round-trip is exact by construction.
 * Covers the validated-write plus mutable-anchor path end to end; plain
 * reads are covered by the rtc_datetime probe. HLE boot only, validator
 * epoch irrelevant (the write replaces the calendar).
 */

#include "gba.h"

#define REG_GPIO_DATA 0xC4
#define REG_GPIO_DIRECTION 0xC6
#define REG_GPIO_CONTROL 0xC8

#define GPIO_BASE 0x08000000u

static volatile uint16_t *const gpio_data = (uint16_t *)(GPIO_BASE + REG_GPIO_DATA);
static volatile uint16_t *const gpio_direction = (uint16_t *)(GPIO_BASE + REG_GPIO_DIRECTION);
static volatile uint16_t *const gpio_control = (uint16_t *)(GPIO_BASE + REG_GPIO_CONTROL);

void report_and_idle(uint32_t value);

static void rtc_begin(void) {
    *gpio_direction = 0x7u;
    *gpio_data = 0x1u;
    *gpio_data = 0x5u;
}

static void rtc_end(void) {
    *gpio_data = 0x1u;
}

static void rtc_send_byte(uint32_t value) {
    for (uint32_t i = 0; i < 8u; i++) {
        uint32_t bit = (value >> i) & 1u;
        *gpio_data = (uint16_t)(0x4u | (bit << 1));
        *gpio_data = (uint16_t)(0x5u | (bit << 1));
    }
}

static uint32_t rtc_read_bit(void) {
    *gpio_data = 0x4u;
    *gpio_data = 0x5u;
    return (*gpio_data >> 1) & 1u;
}

static void rtc_write(uint32_t command, const uint8_t *data, uint32_t count) {
    rtc_begin();
    rtc_send_byte(command);
    for (uint32_t i = 0; i < count; i++) {
        rtc_send_byte(data[i]);
    }
    rtc_end();
}

static void rtc_read(uint32_t command, uint8_t *out, uint32_t count) {
    rtc_begin();
    rtc_send_byte(command);
    *gpio_direction = 0x5u;
    for (uint32_t i = 0; i < count; i++) {
        uint32_t byte = 0;
        for (uint32_t b = 0; b < 8u; b++) {
            byte |= rtc_read_bit() << b;
        }
        out[i] = (uint8_t)byte;
    }
    rtc_end();
}

int main(void) {
    *gpio_control = 0x1u;

    static const uint8_t updated[7] = { 0x02, 0x12, 0x31, 0x02, 0x23, 0x59, 0x58 };
    rtc_write(0x26u, updated, 7u);

    uint8_t back[7];
    rtc_read(0xA6u, back, 7u);
    for (uint32_t i = 0; i < 7u; i++) {
        if (back[i] != updated[i]) {
            report_and_idle(2 + i);
            return 0;
        }
    }
    report_and_idle(0);
    return 0;
}
