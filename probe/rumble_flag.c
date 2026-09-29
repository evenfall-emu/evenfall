/* Probe 25: cartridge rumble motor state through GPIO.
 *
 * Configures GPIO3 as an output with readback enabled and drives it high.
 * The validator enables the rumble device explicitly and requires
 * Machine.rumbleActive() once the ROM idles. The core never touches host
 * haptics; the observable contract ends at the active flag, which this
 * probe pins end to end. HLE boot only.
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
    *gpio_direction = 0x8u;
    *gpio_data = 0x8u;
    report_and_idle(0);
    return 0;
}
