/* Probe 36: Mode 4 bitmap with page flip (256 colors).
 *
 * Fills page 0 with red/green row stripes and page 1 with blue/white row
 * stripes (whole rows uniform, so halfword pairing cannot matter), selects
 * Mode 4 showing page 1 (DISPCNT bit 4), and idles after two frames. The
 * validator recomputes every pixel analytically. Showing page 1 instead of
 * page 0 proves page selection: a stuck page bit would render the red/green
 * page. Exercises the paletted bitmap path and paging end to end. HLE boot
 * only.
 */

#include "gba.h"

#define REG_DISPCNT 0x00
#define REG_VCOUNT 0x06
#define MODE4_PAGE0 ((volatile uint16_t *)0x06000000)
#define MODE4_PAGE1 ((volatile uint16_t *)0x0600A000)
#define PALETTE ((volatile uint16_t *)0x05000000)

void report_and_idle(uint32_t value);

int main(void) {
    for (uint32_t i = 0; i < 19200u; i++) {
        uint32_t row = i / 120u;
        MODE4_PAGE0[i] = (uint16_t)((row & 1u) ? 0x0202u : 0x0101u);
        MODE4_PAGE1[i] = (uint16_t)((row & 1u) ? 0x0404u : 0x0303u);
    }
    PALETTE[1] = 0x001Fu;
    PALETTE[2] = 0x03E0u;
    PALETTE[3] = 0x7C00u;
    PALETTE[4] = 0x7FFFu;

    MEM_IO[REG_DISPCNT / 2] = 0x0414u;

    while (MEM_IO[REG_VCOUNT / 2] != 0) {
    }
    for (uint32_t f = 0; f < 2u; f++) {
        while (MEM_IO[REG_VCOUNT / 2] == 0) {
        }
        while (MEM_IO[REG_VCOUNT / 2] != 0) {
        }
    }
    report_and_idle(0);
    return 0;
}
