/* Workload: affine. Raster effects over affine layers, the way racing and
 * flight games draw a perspective floor: mode 2 with a 64x64-tile wrapping
 * BG2 whose matrix and reference point are rewritten for every line by
 * HBlank DMA (the "mode 7" floor), a wrapping 32x32-tile BG3 whose origin
 * an HBlank IRQ handler moves every line (a wobble), and 16 rotating
 * double-size affine sprites. The CPU computes the next frame's line tables
 * and matrices during HDraw, including one division per line, then halts;
 * HBlank IRQs wake it on every line. */

#include "bench.h"

#define LINES 160
#define SPRITES 16

struct floor_line {
    int16_t pa, pb, pc, pd;
    int32_t x, y;
};

/* Line tables are double-buffered: HDMA and the IRQ read one while the CPU
 * fills the other. Entry 160 is the one extra transfer after line 159. */
static struct floor_line floor[2][LINES + 1];
IWRAM_BSS static int32_t wobble[2][LINES];
IWRAM_BSS static volatile uint32_t shown;
static uint16_t shadow_oam[128 * 4];

IWRAM_ARM static void wobble_line(void) {
    uint32_t next = REG_VCOUNT + 1;
    if (next == 228) next = 0;
    if (next < LINES) REG_BG3X = (uint32_t)wobble[shown][next];
}

static void make_graphics(void) {
    /* 64 8bpp BG tiles; affine VRAM takes 16-bit writes only. */
    volatile uint16_t *tiles = (volatile uint16_t *)0x06000000;
    for (uint32_t tile = 0; tile < 64; tile++) {
        for (uint32_t i = 0; i < 32; i++) {
            uint32_t x = (i * 2) & 7, y = i >> 2;
            uint32_t a = (tile * 4 + (x ^ y) + ((x + y) & 4 ? 64 : 0)) & 255;
            uint32_t b = (tile * 4 + ((x + 1) ^ y) + ((x + 1 + y) & 4 ? 64 : 0)) & 255;
            tiles[tile * 32 + i] = (uint16_t)(a | b << 8);
        }
    }
    volatile uint16_t *map2 = (volatile uint16_t *)0x06008000;
    for (uint32_t i = 0; i < 64 * 64 / 2; i++) {
        uint32_t x = (i * 2) & 63, y = (i * 2) >> 6;
        uint32_t a = ((x >> 1) ^ (y >> 1)) & 63, b = (((x + 1) >> 1) ^ (y >> 1)) & 63;
        map2[i] = (uint16_t)(a | b << 8);
    }
    volatile uint16_t *map3 = (volatile uint16_t *)0x0600A000;
    for (uint32_t i = 0; i < 32 * 32 / 2; i++) {
        uint32_t x = (i * 2) & 31, y = (i * 2) >> 5;
        uint32_t a = (x + y) & 8 ? 0 : 1 + ((x * 3 + y) & 31);
        uint32_t b = (x + 1 + y) & 8 ? 0 : 1 + (((x + 1) * 3 + y) & 31);
        map3[i] = (uint16_t)(a | b << 8);
    }
    /* One 16x16 4bpp sprite (4 tiles) in OBJ charblock 4: a pointed shape. */
    volatile uint32_t *obj = (volatile uint32_t *)0x06010000;
    for (uint32_t tile = 0; tile < 4; tile++) {
        for (uint32_t y = 0; y < 8; y++) {
            uint32_t row = 0;
            for (uint32_t x = 0; x < 8; x++) {
                uint32_t px = (tile & 1) * 8 + x, py = (tile >> 1) * 8 + y;
                uint32_t pixel = px + py / 2 < 16 && px >= py / 2 ? 1 + ((px + py) & 7) : 0;
                row |= pixel << (x * 4);
            }
            obj[tile * 8 + y] = row;
        }
    }
    /* Even channel values keep any future blend exact across emulators. */
    for (uint32_t i = 0; i < 256; i++) {
        PALETTE_BG[i] = (uint16_t)((i * 0x0C45u + (i >> 5) * 0x2108u) & 0x7BDE);
        PALETTE_OBJ[i] = (uint16_t)((i * 0x1CE7u) & 0x7BDE);
    }
    for (uint32_t i = 0; i < 128; i++) shadow_oam[i * 4] = 0x0200; /* hidden */
}

static void compute_frame(uint32_t frame, uint32_t buffer) {
    uint32_t heading = frame / 2;
    int32_t c = sine256(heading + 64), s = sine256(heading);
    int32_t cam_x = (int32_t)(frame * 96) << 8, cam_y = (int32_t)(frame * 64) << 8;
    for (uint32_t line = 0; line <= LINES; line++) {
        int32_t scale = (1 << 14) / (int32_t)(line + 24);  /* 8.8 texels per pixel */
        struct floor_line *out = &floor[buffer][line];
        out->pa = (int16_t)(scale * c >> 8);
        out->pb = 0;
        out->pc = (int16_t)(scale * s >> 8);
        out->pd = 0;
        int32_t depth = scale * 64;
        out->x = cam_x + (depth * s >> 8) - 120 * out->pa;
        out->y = cam_y - (depth * c >> 8) - 120 * out->pc;
    }
    for (uint32_t line = 0; line < LINES; line++) {
        wobble[buffer][line] = (int32_t)(frame << 8) + sine256(frame * 4 + line * 3) * 16;
    }
    for (uint32_t i = 0; i < SPRITES; i++) {
        uint32_t angle = frame * (i + 1) + i * 16;
        int32_t zoom = 256 + (sine256(frame * 2 + i * 32) >> 2);
        int32_t pa = sine256(angle + 64) * 256 / zoom, pb = sine256(angle) * 256 / zoom;
        uint16_t *entry = &shadow_oam[i * 4];
        uint32_t x = (i & 3) * 60 + 8 + (frame & 31), y = (i >> 2) * 40 + 4;
        entry[0] = (uint16_t)(y | 3 << 8);                   /* affine, double size */
        entry[1] = (uint16_t)(x | i << 9 | 1 << 14);          /* matrix i, 16x16 */
        entry[2] = (uint16_t)(0 | (i & 7) << 12);
        shadow_oam[i * 16 + 3] = (uint16_t)pa;
        shadow_oam[i * 16 + 7] = (uint16_t)pb;
        shadow_oam[i * 16 + 11] = (uint16_t)-pb;
        shadow_oam[i * 16 + 15] = (uint16_t)pa;
    }
}

static void upload(uint32_t buffer) {
    const struct floor_line *first = &floor[buffer][0];
    REG_DMACNT(0) = 0;
    REG_BG2PA = (uint16_t)first->pa;
    REG_BG2PB = (uint16_t)first->pb;
    REG_BG2PC = (uint16_t)first->pc;
    REG_BG2PD = (uint16_t)first->pd;
    REG_BG2X = (uint32_t)first->x;
    REG_BG2Y = (uint32_t)first->y;
    REG_DMASAD(0) = (uint32_t)&floor[buffer][1];
    REG_DMADAD(0) = 0x04000020;
    REG_DMACNT(0) = 4 | DMA_DST_RELOAD | DMA_REPEAT | DMA_32 | DMA_AT_HBLANK | DMA_ENABLE;
    shown = buffer;
    REG_BG3X = (uint32_t)wobble[buffer][0];
    dma3_copy32(shadow_oam, OAM, 128 * 2);
}

int main(void) {
    runtime_init(wobble_line);
    REG_DISPCNT = 0x0080; /* forced blank during setup */
    make_graphics();
    REG_BGCNT(2) = 1 | 16 << 8 | 1 << 13 | 2 << 14;  /* 64x64 tiles, wrap */
    REG_BGCNT(3) = 0 | 20 << 8 | 1 << 13 | 1 << 14;  /* 32x32 tiles, wrap */
    REG_BG3PA = 256;
    REG_BG3PD = 256;
    REG_BG3Y = 0;
    uint32_t buffer = 0;
    compute_frame(START_FRAME, buffer);
    start_frames();
    for (;;) {
        upload(buffer);
        REG_DISPCNT = 2 | 0x0C00 | 0x1000 | 0x0040;   /* mode 2, BG2-3, OBJ 1D */
        buffer ^= 1;
        work_begin();
        compute_frame(frame_count() + 1, buffer);
        work_end();
        vblank_wait();
    }
}
