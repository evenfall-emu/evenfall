/* Shared runtime for the emu-bench ROMs; see bench.h. */

#include "bench.h"

#define BIOS_IF (*(volatile uint16_t *)0x03007FF8)

/* Live before crt0 clears .bss, so they are initialized IWRAM data. */
IWRAM_DATA static void (*volatile hblank)(void) = 0;
IWRAM_DATA static volatile uint32_t vblanks = 0;

static uint32_t work_frame;
static uint32_t work_line;
static uint32_t overruns;
static uint32_t max_lines;

/* Installed by crt0.s. The BIOS dispatcher enters it with `ldr pc`, so it
 * must be ARM. It acknowledges IF and ORs the BIOS mirror IntrWait checks. */
IWRAM_ARM void irq_handler(void);
IWRAM_ARM void irq_handler(void) {
    uint16_t pending = REG_IF & REG_IE;
    if (pending & IRQ_HBLANK) hblank();
    if (pending & IRQ_VBLANK) vblanks++;
    REG_IF = pending;
    BIOS_IF |= pending;
}

void runtime_init(void (*hblank_hook)(void)) {
    REG_WAITCNT = 0x4317;
    if (hblank_hook) {
        hblank = hblank_hook;
        REG_DISPSTAT |= DISPSTAT_HBLANK_IRQ;
        REG_IE |= IRQ_HBLANK;
    }
}

void vblank_wait(void) {
    __asm__ volatile("swi 0x05" ::: "r0", "r1", "r2", "r3", "memory");
}

void start_frames(void) {
    if (vblanks >= START_FRAME) overruns++;
    while (vblanks < START_FRAME) vblank_wait();
}

uint32_t frame_count(void) {
    return vblanks;
}

void work_begin(void) {
    work_frame = vblanks;
    work_line = REG_VCOUNT;
}

/* Scanlines since the last VBlank started (line 160). */
static uint32_t lines_since_vblank(uint32_t line) {
    return (line + 228 - 160) % 228;
}

void work_end(void) {
    uint32_t line = REG_VCOUNT;
    uint32_t frames = vblanks - work_frame;
    uint32_t lines = frames * 228 + lines_since_vblank(line) - lines_since_vblank(work_line);
    if (frames) overruns++;
    if (lines > max_lines) max_lines = lines;
#ifdef CALIBRATE
    /* Workloads that draw a scene have no status rows: after 300 frames
     * stop the workload and show them in mode 3. */
    if (vblanks >= 300) {
        for (uint32_t channel = 0; channel < 4; channel++) REG_DMACNT(channel) = 0;
        REG_IE = IRQ_VBLANK;
        REG_BG2PA = 256;
        REG_BG2PB = 0;
        REG_BG2PC = 0;
        REG_BG2PD = 256;
        REG_BG2X = 0;
        REG_BG2Y = 0;
        REG_DISPCNT = 3 | 0x0400;
        dma3_fill32(0, VRAM, 240 * 160 / 2);
        for (;;) {
            show_status(0);
            vblank_wait();
        }
    }
#endif
}

void show_status(uint32_t checksum) {
    show_word(0, checksum);
    show_word(1, vblanks);
    show_word(2, overruns);
#ifdef CALIBRATE
    show_word(3, max_lines);
#endif
}

void show_word(uint32_t row, uint32_t value) {
    for (uint32_t bit = 0; bit < 32; bit++) {
        uint16_t color = (value >> (31 - bit)) & 1 ? RGB(31, 31, 31) : RGB(0, 0, 8);
        uint32_t fill = color | (uint32_t)color << 16;
        for (uint32_t y = 0; y < 7; y++) {
            volatile uint32_t *line = (volatile uint32_t *)&VRAM[(row * 8 + y) * 240 + bit * 7 + 8];
            line[0] = fill;
            line[1] = fill;
            line[2] = fill;
        }
    }
}

int32_t sine256(uint32_t angle) {
    int32_t x = (int32_t)(angle & 127);
    int32_t value = x * (128 - x) >> 4;
    return angle & 128 ? -value : value;
}

void dma3_copy32(const volatile void *src, volatile void *dst, uint32_t words) {
    REG_DMASAD(3) = (uint32_t)src;
    REG_DMADAD(3) = (uint32_t)dst;
    REG_DMACNT(3) = words | DMA_32 | DMA_ENABLE;
}

void dma3_fill32(uint32_t value, volatile void *dst, uint32_t words) {
    static volatile uint32_t source;
    source = value;
    REG_DMASAD(3) = (uint32_t)&source;
    REG_DMADAD(3) = (uint32_t)dst;
    REG_DMACNT(3) = words | DMA_SRC_FIXED | DMA_32 | DMA_ENABLE;
}
