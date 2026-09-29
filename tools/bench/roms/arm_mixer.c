/* Workload: arm-mixer. The ARM-in-IWRAM hot loop of a game sound mixer, the
 * largest CPU cost in commercial games using the common sound driver: eight
 * channels resample 8-bit waves from EWRAM with 12.20 fixed-point steps,
 * scale by per-side volume into 32-bit IWRAM accumulators, then clip to
 * signed 8-bit output in EWRAM. No sound hardware is enabled; this is CPU
 * work only. Each frame mixes a fixed sample count, sized to leave the CPU
 * busy for most of the frame, then halts until VBlank. */

#include "bench.h"

#define CHANNELS 8
#define SAMPLES 736
#define WAVES 4
#define WAVE_LENGTH 4096

struct channel {
    const int8_t *wave;
    uint32_t position;
    uint32_t step;
    int32_t left;
    int32_t right;
};

static int8_t waves[WAVES][WAVE_LENGTH];
static int8_t output[2][SAMPLES];
IWRAM_BSS static struct channel channels[CHANNELS];
IWRAM_BSS static int32_t mix_left[SAMPLES];
IWRAM_BSS static int32_t mix_right[SAMPLES];

static void make_waves(void) {
    uint32_t seed = 0x1234567;
    for (uint32_t i = 0; i < WAVE_LENGTH; i++) {
        int32_t phase = (int32_t)(i & 255) - 128;
        seed = seed * 1103515245u + 12345u;
        waves[0][i] = (int8_t)phase;                                   /* saw */
        waves[1][i] = (int8_t)(phase * (128 - (phase < 0 ? -phase : phase)) / 64); /* sine-ish */
        waves[2][i] = (int8_t)(i & 64 ? 96 : -96);                     /* square */
        waves[3][i] = (int8_t)(seed >> 24);                            /* noise */
    }
}

static void update_channels(uint32_t frame) {
    for (uint32_t c = 0; c < CHANNELS; c++) {
        struct channel *ch = &channels[c];
        if (!ch->wave || (frame + c * 5) % 24 == 0) {
            ch->wave = waves[(frame / 24 + c) % WAVES];
            ch->position = 0;
            ch->step = (1u << 19) + ((frame * 37 + c * 911) & 0x3FFFF);
        }
        uint32_t fade = 63 - ((frame + c * 3) & 31);
        ch->left = (int32_t)((c + 1) * fade / 4);
        ch->right = (int32_t)((CHANNELS - c) * fade / 4);
    }
}

IWRAM_ARM static uint32_t mix(void) {
    for (uint32_t i = 0; i < SAMPLES; i++) {
        mix_left[i] = 0;
        mix_right[i] = 0;
    }
    for (uint32_t c = 0; c < CHANNELS; c++) {
        struct channel *ch = &channels[c];
        const int8_t *wave = ch->wave;
        uint32_t position = ch->position, step = ch->step;
        int32_t left = ch->left, right = ch->right;
        for (uint32_t i = 0; i < SAMPLES; i++) {
            int32_t sample = wave[(position >> 20) & (WAVE_LENGTH - 1)];
            mix_left[i] += sample * left;
            mix_right[i] += sample * right;
            position += step;
        }
        ch->position = position;
    }
    uint32_t checksum = 0;
    for (uint32_t i = 0; i < SAMPLES; i++) {
        int32_t l = mix_left[i] >> 8, r = mix_right[i] >> 8;
        l = l > 127 ? 127 : l < -128 ? -128 : l;
        r = r > 127 ? 127 : r < -128 ? -128 : r;
        output[0][i] = (int8_t)l;
        output[1][i] = (int8_t)r;
        checksum = (checksum ^ (uint32_t)(l << 8 | (r & 255))) * 16777619u;
    }
    return checksum;
}

int main(void) {
    runtime_init(0);
    make_waves();
    REG_DISPCNT = 3 | 0x0400; /* mode 3, BG2 */
    uint32_t checksum = 0;
    start_frames();
    for (;;) {
        show_status(checksum);
        work_begin();
        update_channels(frame_count());
        checksum ^= mix();
        work_end();
        vblank_wait();
    }
}
