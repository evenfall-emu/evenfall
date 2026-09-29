/* Workload: audio. The sound hardware a game keeps busy, with little CPU:
 * Direct Sound A and B fed by timer 0 at 31536 Hz (528 samples a frame, the
 * sound driver's highest standard rate) through special-timing DMA 1 and 2
 * from double-buffered EWRAM, plus all four PSG channels: square 1 with
 * sweep, square 2, the wave channel alternating its two banks, and noise,
 * retriggered with changing envelopes, lengths and frequencies. The CPU
 * synthesises the next frame's 2 x 528 samples in HDraw, like a light
 * software mixer, then halts. */

#include "bench.h"

#define SAMPLES 528
#define TIMER_RELOAD (0x10000 - 532) /* 16777216 / 532 = 31536 Hz */

static int8_t buffers[2][2][SAMPLES] __attribute__((aligned(4)));

static uint32_t synthesize(int8_t *left, int8_t *right, uint32_t frame) {
    uint32_t phase_a = frame * SAMPLES * 7, phase_b = frame * SAMPLES * 11;
    uint32_t checksum = frame;
    for (uint32_t i = 0; i < SAMPLES; i++) {
        phase_a += 7 + (frame & 7);
        phase_b += 11;
        int32_t a = (int32_t)(phase_a & 255) - 128;
        int32_t b = sine256(phase_b) >> 2;
        left[i] = (int8_t)((a + b) >> 1);
        right[i] = (int8_t)((a - b) >> 1);
        checksum = (checksum ^ (uint8_t)left[i] ^ (uint32_t)(uint8_t)right[i] << 8) * 16777619u;
    }
    return checksum;
}

static void retrigger(uint32_t frame) {
    if (frame % 16 == 0) {
        REG_SOUND1CNT_L = (uint16_t)(2 << 4 | (frame & 16 ? 0x08 : 0) | 3); /* sweep */
        REG_SOUND1CNT_H = (uint16_t)(0xF000 | 3 << 8 | 2 << 6 | 20);       /* envelope down */
        REG_SOUND1CNT_X = (uint16_t)(0x8000 | 0x4000 | (1400 + (frame & 255)));
    }
    if (frame % 4 == 0) {
        REG_SOUND2CNT_L = (uint16_t)(0xA000 | 1 << 8 | (frame & 3) << 6 | 40);
        REG_SOUND2CNT_H = (uint16_t)(0x8000 | (1600 + (frame * 13 & 255)));
    }
    if (frame % 8 == 0) {
        uint32_t bank = (frame / 8) & 1;
        REG_SOUND3CNT_L = (uint16_t)(0x80 | bank << 6);                    /* play one bank */
        REG_SOUND3CNT_H = (uint16_t)(1 << 13 | 128);
        REG_SOUND3CNT_X = (uint16_t)(0x8000 | 0x4000 | (1800 + (frame & 127)));
    }
    if (frame % 6 == 0) {
        REG_SOUND4CNT_L = (uint16_t)(0xC000 | 2 << 8 | 30);
        REG_SOUND4CNT_H = (uint16_t)(0x8000 | 0x4000 | ((frame / 6) & 1) << 3 | 2 << 4 | 3);
    }
}

static void start_sound(void) {
    REG_SOUNDCNT_X = 0x0080;              /* master enable */
    REG_SOUNDCNT_L = 0xFF77;              /* PSG 1-4 left and right, full volume */
    REG_SOUNDCNT_H = 0x0002 | 0x0004 | 0x0008 | 0x0300 | 0x0800 | 0x3000 | 0x8000;
    /* Wave RAM: fill bank 1 (the one not playing) with a ramp, then bank 0. */
    for (uint32_t bank = 0; bank < 2; bank++) {
        REG_SOUND3CNT_L = (uint16_t)((1 - bank) << 6);
        for (uint32_t i = 0; i < 4; i++) {
            WAVE_RAM[i] = bank ? 0x01234567u * (i + 1) : 0xFEDCBA98u - i * 0x11111111u;
        }
    }
    REG_TMCNT_L(0) = TIMER_RELOAD;
    REG_TMCNT_H(0) = TM_ENABLE;
}

/* Restarts both FIFO DMAs at the start of the buffer just synthesised. */
static void queue(uint32_t buffer) {
    REG_DMACNT(1) = 0;
    REG_DMACNT(2) = 0;
    REG_DMASAD(1) = (uint32_t)buffers[buffer][0];
    REG_DMADAD(1) = REG_FIFO_A;
    REG_DMACNT(1) = DMA_DST_FIXED | DMA_REPEAT | DMA_32 | DMA_AT_SPECIAL | DMA_ENABLE;
    REG_DMASAD(2) = (uint32_t)buffers[buffer][1];
    REG_DMADAD(2) = REG_FIFO_B;
    REG_DMACNT(2) = DMA_DST_FIXED | DMA_REPEAT | DMA_32 | DMA_AT_SPECIAL | DMA_ENABLE;
}

int main(void) {
    runtime_init(0);
    REG_DISPCNT = 3 | 0x0400; /* mode 3, BG2: status only */
    uint32_t buffer = 0;
    uint32_t checksum = synthesize(buffers[0][0], buffers[0][1], START_FRAME);
    start_sound();
    start_frames();
    for (;;) {
        queue(buffer);
        retrigger(frame_count());
        show_status(checksum);
        buffer ^= 1;
        work_begin();
        checksum ^= synthesize(buffers[buffer][0], buffers[buffer][1], frame_count() + 1);
        work_end();
        vblank_wait();
    }
}
