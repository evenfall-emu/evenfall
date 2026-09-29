/* Workload: bitmap-dma. A software renderer that leans on DMA, like games
 * drawing into bitmap modes: mode 4 with page flipping, where each frame the
 * hidden page is cleared by one 32-bit DMA fill, then filled rectangles
 * (one DMA per row), 32x32 image blits from EWRAM (one DMA per row) and CPU
 * pixel plots with 16-bit read-modify-write are drawn into it. The finished
 * page is shown at VBlank. Many small DMAs make their setup cost count. */

#include "bench.h"

#define RECTS 14
#define BLITS 8
#define DOTS 128
#define PAGE_WORDS (240 * 160 / 4)

static uint32_t images[16][32 * 32 / 4];

static volatile uint8_t *page(uint32_t index) {
    return (volatile uint8_t *)(0x06000000 + index * 0xA000);
}

static void make_assets(void) {
    for (uint32_t image = 0; image < 16; image++) {
        uint8_t *pixels = (uint8_t *)images[image];
        for (uint32_t y = 0; y < 32; y++) {
            for (uint32_t x = 0; x < 32; x++) {
                uint32_t ring = ((x - 16) * (x - 16) + (y - 16) * (y - 16)) >> 5;
                pixels[y * 32 + x] = (uint8_t)(image * 16 + ((ring + x / 4) & 15));
            }
        }
    }
    for (uint32_t i = 0; i < 256; i++) {
        PALETTE_BG[i] = (uint16_t)((i * 0x0C63u + (i >> 4) * 0x1085u) & 0x7BDE);
    }
}

static void draw(volatile uint8_t *target, uint32_t frame) {
    dma3_fill32(0x01010101u * (frame & 255), target, PAGE_WORDS);
    for (uint32_t i = 0; i < RECTS; i++) {
        uint32_t w = 16 + (i * 28 & 63) / 4 * 4, h = 8 + (i * 13 & 31);
        uint32_t x = (uint32_t)(112 + (sine256(frame + i * 23) * (112 - w / 2) >> 8) - w / 2) & ~3u;
        uint32_t y = (uint32_t)(80 + (sine256(frame * 2 + i * 41 + 64) * (72 - h / 2) >> 8) - h / 2);
        uint32_t color = 0x01010101u * ((i * 37 + frame) & 255);
        for (uint32_t row = 0; row < h; row++) {
            dma3_fill32(color, target + (y + row) * 240 + x, w / 4);
        }
    }
    for (uint32_t i = 0; i < BLITS; i++) {
        uint32_t x = (uint32_t)(104 + (sine256(frame * 3 + i * 51) * 100 >> 8)) & ~3u;
        uint32_t y = (uint32_t)(64 + (sine256(frame + i * 29 + 64) * 60 >> 8));
        const uint32_t *image = images[(i + frame / 8) & 15];
        for (uint32_t row = 0; row < 32; row++) {
            dma3_copy32(image + row * 8, target + (y + row) * 240 + x, 8);
        }
    }
    /* Bitmap VRAM takes no byte writes: plot through the containing halfword. */
    for (uint32_t i = 0; i < DOTS; i++) {
        uint32_t x = (uint32_t)(120 + (sine256(frame + i * 5) * 118 >> 8));
        uint32_t y = (uint32_t)(80 + (sine256(frame * 2 + i * 3 + 64) * 78 >> 8));
        volatile uint16_t *pair = (volatile uint16_t *)(target + ((y * 240 + x) & ~1u));
        uint16_t value = *pair;
        *pair = x & 1 ? (uint16_t)((value & 0x00FF) | (i & 255) << 8) : (uint16_t)((value & 0xFF00) | (i & 255));
    }
}

int main(void) {
    runtime_init(0);
    REG_DISPCNT = 0x0080; /* forced blank during setup */
    make_assets();
    uint32_t hidden = 1;
    draw(page(hidden), START_FRAME);
    start_frames();
    for (;;) {
        REG_DISPCNT = 4 | 0x0400 | hidden << 4;   /* mode 4, BG2, show the new page */
        hidden ^= 1;
        work_begin();
        draw(page(hidden), frame_count() + 1);
        work_end();
        vblank_wait();
    }
}
