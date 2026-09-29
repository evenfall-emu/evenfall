/* Probe 21: WarioWare gyro ADC sample through GPIO bit-banging.
 *
 * Mirrors the host-tested GPIO sequence exactly: CONTROL read/write mode,
 * DIRECTION with start/clock/rumble as outputs, then sixteen clock edges
 * sampling GPIO2 into a 16-bit value (four dummy bits plus the raw 12-bit
 * ADC sample). The validator injects gyro sample 0x6C0 and expects it back
 * verbatim in r12. Covers GPIO direction/data/control, the gyro serial
 * device, and deterministic peripheral injection end to end. HLE boot only.
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
    *gpio_direction = 0xBu;

    *gpio_data = 0xBu;
    *gpio_data = 0xAu;
    uint32_t value = 0;
    for (uint32_t i = 0; i < 16u; i++) {
        value = (value << 1) | ((*gpio_data >> 2) & 1u);
        *gpio_data = 0x8u;
        *gpio_data = 0xAu;
    }
    report_and_idle(value);
    return 0;
}
