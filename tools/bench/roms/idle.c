/* Workload: idle. The floor every emulator pays for guest time with nothing
 * to do: one background-colour write per frame, then HALT in VBlankIntrWait.
 * Video is on (mode 0, no layers), so the renderer still composes backdrop
 * lines; no DMA, timers or sound. The final image is a backdrop colour that
 * encodes the frame count. */

#include "bench.h"

int main(void) {
    runtime_init(0);
    REG_DISPCNT = 0;
    for (;;) {
        uint32_t frame = frame_count();
        PALETTE_BG[0] = (uint16_t)(frame * 0x0421u) & 0x7FFF;
        vblank_wait();
    }
}
