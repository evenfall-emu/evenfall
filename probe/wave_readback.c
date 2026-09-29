/* Probe 66: wave-RAM shift-register read-back.
 *
 * Wave RAM is a hardware shift register: while channel 3 plays, the active
 * span rotates one nibble per sample step, so CPU read-back sees rotated
 * data (GBATEK wave playback). Phase 1 writes a known pattern to bank 1,
 * plays it at frequency 0 (16384 cycles per step) for exactly 3 steps plus
 * a half-step margin, stops, and checksums the 16 read-back bytes. Phase 2
 * rewrites both banks with distinct patterns, plays the 64-nibble two-bank
 * span for exactly 67 steps plus margin (effective rotation 3), stops, and
 * extends the checksum over both banks. The validator recomputes the FNV-1a
 * over the GBATEK-model rotated patterns (see
 * test/probe_cases.zig). Output samples are covered by the
 * wave_ramp/wave_chain probes; this probe pins storage rotation only.
 */

#include "gba.h"

#define REG_NR30 0x70
#define REG_NR32 0x72
#define REG_NR33_NR34 0x74
#define REG_SOUNDCNT_L 0x80
#define REG_SOUNDCNT_H 0x82
#define REG_SOUNDCNT_X 0x84
#define REG_TM1CNT_L 0x104
#define REG_TM1CNT_H 0x106
#define WAVE_RAM ((volatile uint16_t *)0x04000090)
#define WAVE_BYTES ((volatile uint8_t *)0x04000090)

void report_and_idle(uint32_t value);

static uint32_t fnv1a(const volatile uint8_t *p, uint32_t n, uint32_t h) {
    for (uint32_t i = 0; i < n; i++) {
        h ^= p[i];
        h *= 16777619u;
    }
    return h;
}

static uint32_t elapsed32(void) {
    uint32_t lo = MEM_IO[REG_TM0CNT_L / 2];
    uint32_t hi = MEM_IO[REG_TM1CNT_L / 2];
    return (hi << 16) | lo;
}

static void wait_cycles(uint32_t cycles) {
    uint32_t start = elapsed32();
    while (elapsed32() - start < cycles) {
    }
}

static void write_pattern(uint32_t add) {
    /* Eight halfwords; byte i holds nibbles (4i+add, 4i+add+1) mod 16. */
    for (uint32_t h = 0; h < 8u; h++) {
        uint32_t blo = (((4u * h + add) % 16u) << 4) | ((4u * h + add + 1u) % 16u);
        uint32_t bhi = (((4u * h + add + 2u) % 16u) << 4) | ((4u * h + add + 3u) % 16u);
        WAVE_RAM[h] = (uint16_t)((bhi << 8) | blo);
    }
}

int main(void) {
    /* TM0 free-running at 1 count/cycle, TM1 counts TM0 overflows. */
    MEM_IO[REG_TM0CNT_L / 2] = 0u;
    MEM_IO[REG_TM1CNT_L / 2] = 0u;
    MEM_IO[REG_TM0CNT_H / 2] = (uint16_t)TM_ENABLE;
    MEM_IO[REG_TM1CNT_H / 2] = (uint16_t)(TM_ENABLE | 0x04u);

    /* Phase 1: nibble ramp into bank 1 while master sound is off (the CPU
     * then sees bank 1). */
    write_pattern(0u);

    MEM_IO[REG_SOUNDCNT_X / 2] = 0x0080u;
    /* Routing after enable: SOUNDCNT_L is gated while master is off. */
    MEM_IO[REG_SOUNDCNT_L / 2] = 0x4477u;
    MEM_IO[REG_SOUNDCNT_H / 2] = 0x0002u;
    MEM_IO[REG_NR30 / 2] = 0x00C0u;
    MEM_IO[REG_NR32 / 2] = 0x2000u;
    MEM_IO[REG_NR33_NR34 / 2] = 0x8000u;
    /* 3 steps at 16384 cycles/step plus a half-step margin: the poll exit
     * and stop write land long before step 4 either way. */
    wait_cycles(57344u);
    MEM_IO[REG_NR30 / 2] = 0x0000u;
    /* Select bank 0 so the CPU reads back the played bank 1. */
    uint32_t hash = fnv1a(WAVE_BYTES, 16u, 2166136261u);

    /* Phase 2: rewrite both banks while stopped (select the opposite bank
     * to address each one), then play the two-bank span. */
    MEM_IO[REG_NR30 / 2] = 0x0040u;
    write_pattern(5u);
    MEM_IO[REG_NR30 / 2] = 0x0000u;
    write_pattern(11u);
    MEM_IO[REG_NR30 / 2] = 0x00A0u;
    MEM_IO[REG_NR33_NR34 / 2] = 0x8000u;
    /* 67 steps over the 64-nibble span (effective rotation 3) plus margin. */
    wait_cycles(1106176u);
    MEM_IO[REG_NR30 / 2] = 0x0000u;
    /* Select bank 0: CPU reads bank 1; then select bank 1 for bank 0. */
    hash = fnv1a(WAVE_BYTES, 16u, hash);
    MEM_IO[REG_NR30 / 2] = 0x0040u;
    hash = fnv1a(WAVE_BYTES, 16u, hash);

    report_and_idle(hash);
    return 0;
}
