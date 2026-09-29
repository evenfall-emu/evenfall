/* Probe 20: S3511 RTC date/time read through GPIO bit-banging.
 *
 * Follows the exact GPIO sequence the host tests use (see also
 * docs/hardware/cartridge.md): DIRECTION all-outputs, DATA start
 * edge, command byte LSB-first with SCK low/high pairs, DIRECTION to SIO
 * input for the payload, CS drop to terminate. Reads the control register
 * (expected 0x40, 24-hour mode) and the 7-byte date/time latch, comparing
 * against the fixed epoch the validator injects (2001-02-03 13:04:05 UTC,
 * Saturday: year 01, month 02, day 03, weekday 06, hour 13, min 04, sec 05).
 * The whole transfer takes thousands of emulated cycles against a whole
 * second, so no calendar rollover can intervene. Reports 0 on a full match
 * (1 = control mismatch, 2+i = date/time byte i mismatch).
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

    uint8_t control;
    rtc_read(0xC6u, &control, 1u);
    if (control != 0x40u) {
        report_and_idle(1);
        return 0;
    }

    static const uint8_t expected[7] = { 0x01, 0x02, 0x03, 0x06, 0x13, 0x04, 0x05 };
    uint8_t date_time[7];
    rtc_read(0xA6u, date_time, 7u);
    for (uint32_t i = 0; i < 7u; i++) {
        if (date_time[i] != expected[i]) {
            report_and_idle(2 + i);
            return 0;
        }
    }
    report_and_idle(0);
    return 0;
}
