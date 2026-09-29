/* Register map and runtime for the emu-bench ROMs. Freestanding: no libgba.
 * The runtime (runtime.c) installs a VBlank IRQ handler, sets game-typical
 * wait states and provides the frame loop helpers used by every workload. */

#ifndef EMU_BENCH_H
#define EMU_BENCH_H

#include <stdint.h>

#define REG16(offset) (*(volatile uint16_t *)(0x04000000 + (offset)))
#define REG32(offset) (*(volatile uint32_t *)(0x04000000 + (offset)))

#define REG_DISPCNT   REG16(0x000)
#define REG_DISPSTAT  REG16(0x004)
#define REG_VCOUNT    REG16(0x006)
#define REG_BGCNT(n)  REG16(0x008 + 2 * (n))
#define REG_BGHOFS(n) REG16(0x010 + 4 * (n))
#define REG_BGVOFS(n) REG16(0x012 + 4 * (n))
#define REG_BG2PA     REG16(0x020)
#define REG_BG2PB     REG16(0x022)
#define REG_BG2PC     REG16(0x024)
#define REG_BG2PD     REG16(0x026)
#define REG_BG2X      REG32(0x028)
#define REG_BG2Y      REG32(0x02C)
#define REG_BG3PA     REG16(0x030)
#define REG_BG3PB     REG16(0x032)
#define REG_BG3PC     REG16(0x034)
#define REG_BG3PD     REG16(0x036)
#define REG_BG3X      REG32(0x038)
#define REG_BG3Y      REG32(0x03C)
#define REG_WIN0H     REG16(0x040)
#define REG_WIN1H     REG16(0x042)
#define REG_WIN0V     REG16(0x044)
#define REG_WIN1V     REG16(0x046)
#define REG_WININ     REG16(0x048)
#define REG_WINOUT    REG16(0x04A)
#define REG_MOSAIC    REG16(0x04C)
#define REG_BLDCNT    REG16(0x050)
#define REG_BLDALPHA  REG16(0x052)
#define REG_BLDY      REG16(0x054)

#define REG_SOUND1CNT_L REG16(0x060)
#define REG_SOUND1CNT_H REG16(0x062)
#define REG_SOUND1CNT_X REG16(0x064)
#define REG_SOUND2CNT_L REG16(0x068)
#define REG_SOUND2CNT_H REG16(0x06C)
#define REG_SOUND3CNT_L REG16(0x070)
#define REG_SOUND3CNT_H REG16(0x072)
#define REG_SOUND3CNT_X REG16(0x074)
#define REG_SOUND4CNT_L REG16(0x078)
#define REG_SOUND4CNT_H REG16(0x07C)
#define REG_SOUNDCNT_L  REG16(0x080)
#define REG_SOUNDCNT_H  REG16(0x082)
#define REG_SOUNDCNT_X  REG16(0x084)
#define REG_SOUNDBIAS   REG16(0x088)
#define WAVE_RAM        ((volatile uint32_t *)0x04000090)
#define REG_FIFO_A      0x040000A0u
#define REG_FIFO_B      0x040000A4u

#define REG_DMASAD(n)   REG32(0x0B0 + 12 * (n))
#define REG_DMADAD(n)   REG32(0x0B4 + 12 * (n))
#define REG_DMACNT(n)   REG32(0x0B8 + 12 * (n))
#define DMA_DST_INC     (0u << 21)
#define DMA_DST_FIXED   (2u << 21)
#define DMA_DST_RELOAD  (3u << 21)
#define DMA_SRC_INC     (0u << 23)
#define DMA_SRC_FIXED   (2u << 23)
#define DMA_REPEAT      (1u << 25)
#define DMA_32          (1u << 26)
#define DMA_AT_VBLANK   (1u << 28)
#define DMA_AT_HBLANK   (2u << 28)
#define DMA_AT_SPECIAL  (3u << 28)
#define DMA_IRQ         (1u << 30)
#define DMA_ENABLE      (1u << 31)

#define REG_TMCNT_L(n)  REG16(0x100 + 4 * (n))
#define REG_TMCNT_H(n)  REG16(0x102 + 4 * (n))
#define TM_CASCADE 0x04
#define TM_IRQ     0x40
#define TM_ENABLE  0x80

#define REG_IE      REG16(0x200)
#define REG_IF      REG16(0x202)
#define REG_WAITCNT REG16(0x204)
#define REG_IME     REG16(0x208)

#define IRQ_VBLANK 0x0001u
#define IRQ_HBLANK 0x0002u
#define IRQ_VCOUNT 0x0004u
#define IRQ_TIMER0 0x0008u
#define IRQ_TIMER1 0x0010u

#define DISPSTAT_VBLANK_IRQ 0x0008u
#define DISPSTAT_HBLANK_IRQ 0x0010u

#define PALETTE_BG  ((volatile uint16_t *)0x05000000)
#define PALETTE_OBJ ((volatile uint16_t *)0x05000200)
#define VRAM        ((volatile uint16_t *)0x06000000)
#define OAM         ((volatile uint16_t *)0x07000000)

#define RGB(r, g, b) ((uint16_t)((r) | (g) << 5 | (b) << 10))

/* ARM code copied to IWRAM by crt0.s, callable from Thumb ROM code. */
#define IWRAM_ARM __attribute__((section(".iwram"), target("arm"), long_call, noinline))
/* Initialized data in IWRAM, copied by crt0.s before .bss is cleared. */
#define IWRAM_DATA __attribute__((section(".iwram.data")))
/* Zero-initialized data in IWRAM (fast 32-bit bus) instead of EWRAM. */
#define IWRAM_BSS __attribute__((section(".iwram_bss")))

/* crt0.s enables the VBlank IRQ at the entry point, so frame_count() counts
 * VBlanks since the cartridge started. runtime_init sets game-typical wait
 * states (WAITCNT 0x4317: WS0 3/1, prefetch on); an optional hook runs in
 * IRQ mode on each HBlank IRQ (pass 0 for none). */
void runtime_init(void (*hblank_hook)(void));
/* Halts through BIOS VBlankIntrWait (SWI 5) until the next VBlank. */
void vblank_wait(void);
/* Waits for VBlank START_FRAME since the entry point. Aligning the frame
 * loop's start with it keeps the final image independent of how long each
 * emulator took to run the setup, which must end before it on all of them
 * (the first VBlank comes up to 160 lines early, depending on where the
 * skipped boot leaves the PPU). A late start counts as an overrun. */
#define START_FRAME 16
void start_frames(void);
/* VBlank IRQs since the cartridge entry point. */
uint32_t frame_count(void);
/* Frame-loop bookkeeping around the work each frame does after its VBlank
 * uploads. An overrun is a frame whose work reached the next VBlank, which
 * drops a frame and changes the final image. CALIBRATE=1 builds also track
 * the most scanlines one frame's work took (it depends on each emulator's
 * timing) and, after 300 frames, stop and show the status rows. */
void work_begin(void);
void work_end(void);
/* Mode 3 status rows drawn in VBlank: checksum, frame count, overruns (and
 * the most work scanlines in CALIBRATE=1 builds). */
void show_status(uint32_t checksum);
/* Mode 3 only: draws `value` as 32 7x7 cells in 8-pixel row `row`, bit 31
 * leftmost. */
void show_word(uint32_t row, uint32_t value);
/* Parabolic sine approximation: one period over 256 steps, range -256..256. */
int32_t sine256(uint32_t angle);
/* DMA3 immediate copy of `words` 32-bit words. */
void dma3_copy32(const volatile void *src, volatile void *dst, uint32_t words);
/* DMA3 immediate 32-bit fill of `words` words with `value`. */
void dma3_fill32(uint32_t value, volatile void *dst, uint32_t words);

#endif
