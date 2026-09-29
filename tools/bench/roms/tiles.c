/* Workload: tiles. The tile compositor under a busy side-scrolling scene:
 * mode 0 with four 4bpp backgrounds scrolling at different speeds (one of
 * them 64x64 tiles, one mosaic), 128 sprites of mixed sizes and flips, some
 * semi-transparent, 50% alpha blending of the top layer over everything below,
 * and a moving window that hides it outside. Each VBlank uploads scroll and
 * window registers and a shadow OAM by DMA; the CPU computes the next
 * frame's sprite positions during HDraw and otherwise halts. */

#include "bench.h"

#define SPRITES 128

static uint16_t shadow_oam[SPRITES * 4];
static uint16_t scroll[8];
static uint16_t window[2];

static void make_tiles(void) {
    /* 64 BG tiles in charblock 0: stripes, checkers, rings and noise. */
    volatile uint32_t *bg = (volatile uint32_t *)0x06000000;
    uint32_t seed = 7;
    for (uint32_t tile = 0; tile < 64; tile++) {
        for (uint32_t y = 0; y < 8; y++) {
            uint32_t row = 0;
            for (uint32_t x = 0; x < 8; x++) {
                uint32_t pixel;
                seed = seed * 1103515245u + 12345u;
                switch (tile & 3) {
                case 0: pixel = (x + y + tile) & 15; break;
                case 1: pixel = ((x ^ y) & 4) ? tile & 15 : 0; break;
                case 2: pixel = ((x - 4) * (x - 4) + (y - 4) * (y - 4)) & 15; break;
                default: pixel = (seed >> 28) & 15; break;
                }
                row |= pixel << (x * 4);
            }
            bg[tile * 8 + y] = row;
        }
    }
    /* 8 sprite images of 32x32 4bpp (16 tiles each) in OBJ charblock 4. */
    volatile uint32_t *obj = (volatile uint32_t *)0x06010000;
    for (uint32_t image = 0; image < 8; image++) {
        for (uint32_t tile = 0; tile < 16; tile++) {
            for (uint32_t y = 0; y < 8; y++) {
                uint32_t row = 0;
                for (uint32_t x = 0; x < 8; x++) {
                    int32_t px = (int32_t)((tile & 3) * 8 + x) - 16;
                    int32_t py = (int32_t)((tile >> 2) * 8 + y) - 16;
                    uint32_t r2 = (uint32_t)(px * px + py * py);
                    uint32_t pixel = r2 < 200 + image * 20 ? 1 + ((r2 >> 4) + image) % 15 : 0;
                    row |= pixel << (x * 4);
                }
                obj[(image * 16 + tile) * 8 + y] = row;
            }
        }
    }
    /* Even channel values: with EVA = EVB = 8 every blend is exact, so
     * emulators that round blends differently still produce equal images. */
    for (uint32_t i = 0; i < 256; i++) {
        PALETTE_BG[i] = (uint16_t)((i * 0x0843u + (i >> 4) * 0x1111u) & 0x7BDE);
        PALETTE_OBJ[i] = (uint16_t)((i * 0x1405u + (i >> 4) * 0x0421u) & 0x7BDE);
    }
}

static void make_maps(void) {
    /* BG0-BG2: 32x32 maps in screenblocks 24-26; BG3: 64x64 in 28-31. */
    volatile uint16_t *map = (volatile uint16_t *)0x0600C000;
    for (uint32_t i = 0; i < 32 * 32 * 3; i++) {
        uint32_t layer = i >> 10, x = i & 31, y = (i >> 5) & 31;
        uint32_t tile = (x * 3 + y * 5 + layer * 17) & 63;
        uint32_t flips = ((x ^ y) & 3) << 10;
        map[i] = (uint16_t)(tile | flips | ((x + y + layer) & 15) << 12);
    }
    map = (volatile uint16_t *)0x0600E000;
    for (uint32_t i = 0; i < 64 * 64; i++) {
        uint32_t x = i & 63, y = i >> 6;
        map[i] = (uint16_t)(((x * 7 + y * 3) & 63) | ((x >> 2) & 15) << 12);
    }
}

/* Sprites drift as a wobbling 16x8 grid. Clustered sprites would exceed the
 * per-line OBJ budget, where emulators disagree; this layout stays within it. */
static void compute_frame(uint32_t frame) {
    for (uint32_t i = 0; i < SPRITES; i++) {
        uint32_t size = i & 3; /* 0: 8x8, 1: 16x16, 2: 32x32, 3: 16x16 */
        int32_t x = (int32_t)((i & 15) * 32 + frame) + (sine256(frame * 2 + i * 13) * 12 >> 8);
        int32_t y = (int32_t)((i >> 4) * 32 + frame / 2) + (sine256(frame * 3 + i * 29) * 8 >> 8);
        uint16_t *entry = &shadow_oam[i * 4];
        uint32_t translucent = (i % 5 == 0) ? 1u << 10 : 0;
        entry[0] = (uint16_t)(((uint32_t)y & 255) | translucent);
        entry[1] = (uint16_t)(((uint32_t)x & 511) | (size == 3 ? 1 : size) << 14 | ((i >> 2) & 3) << 12);
        /* One BG priority (under BG0, over BG1-3) for all sprites: overlapping
         * sprites with different priorities hit an OAM-order quirk that
         * emulators resolve differently. */
        entry[2] = (uint16_t)(((i & 7) * 16) | 1 << 10 | (i & 15) << 12);
    }
    for (uint32_t layer = 0; layer < 4; layer++) {
        scroll[layer * 2] = (uint16_t)(frame * (layer + 1));
        scroll[layer * 2 + 1] = (uint16_t)(sine256(frame + layer * 32) >> 3);
    }
    uint32_t wx = 40 + (uint32_t)((sine256(frame) * 40 >> 8) + 40);
    uint32_t wy = 30 + (uint32_t)((sine256(frame * 2 + 64) * 20 >> 8) + 20);
    /* The right edge stays clear of the screen's: near x=235, pinned mGBA
     * places it differently from ZiGBA and NanoBoyAdvance. */
    window[0] = (uint16_t)(wx << 8 | (wx + 96));
    window[1] = (uint16_t)(wy << 8 | (wy + 80));
}

static void upload(void) {
    for (uint32_t layer = 0; layer < 4; layer++) {
        REG_BGHOFS(layer) = scroll[layer * 2];
        REG_BGVOFS(layer) = scroll[layer * 2 + 1];
    }
    REG_WIN0H = window[0];
    REG_WIN0V = window[1];
    dma3_copy32(shadow_oam, OAM, SPRITES * 2);
}

int main(void) {
    runtime_init(0);
    REG_DISPCNT = 0x0080; /* forced blank during setup */
    make_tiles();
    make_maps();
    REG_BGCNT(0) = 0 << 2 | 24 << 8 | 0;         /* top layer, blended */
    REG_BGCNT(1) = 0 << 2 | 25 << 8 | 1;
    REG_BGCNT(2) = 0 << 2 | 26 << 8 | 2 | 0x40;  /* mosaic */
    REG_BGCNT(3) = 0 << 2 | 28 << 8 | 3 | 3 << 14; /* 64x64 tiles */
    REG_MOSAIC = 0x0011;
    REG_BLDCNT = 0x0001 | 1 << 6 | 0x3E << 8;    /* BG0 over BG1-3, OBJ, backdrop */
    REG_BLDALPHA = 8 | 8 << 8;
    REG_WININ = 0x3F;                            /* inside: all layers, effects */
    REG_WINOUT = 0x3E;                           /* outside: no BG0, effects */
    compute_frame(START_FRAME);
    start_frames();
    for (;;) {
        upload();
        REG_DISPCNT = 0 | 0x0F00 | 0x1000 | 0x0040 | 0x2000; /* BG0-3, OBJ 1D, WIN0 */
        work_begin();
        compute_frame(frame_count() + 1);
        work_end();
        vblank_wait();
    }
}
