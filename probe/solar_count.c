/* Probe 22: Boktai solar-sensor threshold count through GPIO.
 *
 * Follows the GBATEK transaction the host tests use: CONTROL read/write,
 * DIRECTION 7 (clock/reset/nCS out, flag in), reset pulse, then clock
 * pulses, reading the GPIO3 comparator flag after each complete pulse and
 * stopping when it goes high (bounded at 16). The validator injects solar
 * level 252, whose adopted threshold (255 - level, shared with
 * NanoBoyAdvance; see docs/hardware/cartridge.md) is 3 with a
 * strict-greater flag, so the flag trips on the fourth complete pulse.
 * Reports the pulse count.
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

int main(void) {
    *gpio_control = 0x1u;
    *gpio_direction = 0x7u;

    *gpio_data = 0x2u;
    *gpio_data = 0x0u;

    uint32_t count = 0;
    for (uint32_t i = 0; i < 16u; i++) {
        *gpio_data = 0x1u;
        *gpio_data = 0x0u;
        count++;
        if (*gpio_data & 0x8u) {
            break;
        }
    }
    report_and_idle(count);
    return 0;
}
