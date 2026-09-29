/* Probe 28: JoyBus slave receive with the validator as external master.
 *
 * Selects JoyBus mode (RCNT 0xC000) with JOYCNT 0x0040, polls the JOYSTAT
 * receive flag, reads the 32-bit JOY_RECV payload little-endian, and reports
 * it (expected 0x44332211 from the validator-scheduled WRITE-to-GBA
 * command). Runs under test/probe_cases.zig, which schedules the
 * typed master command at a caller-supplied completion cycle after setup.
 * HLE boot only.
 */

#include "gba.h"

#define REG_RCNT 0x134
#define REG_JOYCNT 0x140
#define REG_JOYSTAT 0x158
#define REG_JOY_RECV_LO 0x150
#define REG_JOY_RECV_HI 0x152

#define JOYSTAT_RECV 0x0002u

void report_and_idle(uint32_t value);

int main(void) {
    MEM_IO[REG_RCNT / 2] = 0xC000u;
    MEM_IO[REG_JOYCNT / 2] = 0x0040u;

    while ((MEM_IO[REG_JOYSTAT / 2] & JOYSTAT_RECV) == 0) {
    }
    uint32_t payload = (uint32_t)MEM_IO[REG_JOY_RECV_LO / 2] |
        ((uint32_t)MEM_IO[REG_JOY_RECV_HI / 2] << 16);
    report_and_idle(payload);
    return 0;
}
