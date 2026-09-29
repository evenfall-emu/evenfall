/* Workload: thumb-logic. Game-logic-shaped Thumb code running from ROM with
 * game wait states (WAITCNT 0x4317): an actor array in EWRAM stepped through
 * a switch-dispatched state machine and per-type behaviour calls, tile-map
 * collision lookups, libgcc division, then an insertion sort by depth into a
 * shadow OAM in IWRAM, the way games prepare sprites for VBlank. Each frame
 * runs a fixed number of passes, sized to leave the CPU busy for most of the
 * frame, then halts until VBlank. */

#include "bench.h"

#define ACTORS 112
#define PASSES 1
#define MAP_SIZE 64

enum state { IDLE, WALK, JUMP, FALL, HURT, STATES };

struct actor {
    int32_t x, y;   /* 24.8 fixed point */
    int16_t vx, vy;
    uint8_t state, type, timer, hp;
    uint32_t seed;
};

static struct actor actors[ACTORS];
static uint8_t map[MAP_SIZE * MAP_SIZE];
IWRAM_BSS static uint8_t order[ACTORS];
IWRAM_BSS static uint16_t shadow_oam[ACTORS * 4];

static uint32_t next_random(struct actor *a) {
    a->seed = a->seed * 1664525u + 1013904223u;
    return a->seed >> 16;
}

static int solid(int32_t x, int32_t y) {
    uint32_t tx = ((uint32_t)x >> 11) & (MAP_SIZE - 1);
    uint32_t ty = ((uint32_t)y >> 11) & (MAP_SIZE - 1);
    return map[ty * MAP_SIZE + tx] >= 200;
}

typedef void (*behaviour)(struct actor *);

static void __attribute__((noinline)) walker(struct actor *a) {
    if (solid(a->x + a->vx * 8, a->y)) a->vx = (int16_t)-a->vx;
}

static void __attribute__((noinline)) hopper(struct actor *a) {
    if (a->state == WALK && (next_random(a) & 63) == 0) {
        a->state = JUMP;
        a->vy = -600;
    }
}

static void __attribute__((noinline)) chaser(struct actor *a) {
    const struct actor *target = &actors[a->timer % ACTORS];
    int32_t dx = (target->x - a->x) / 4096;
    a->vx = (int16_t)(dx > 96 ? 96 : dx < -96 ? -96 : dx);
}

static const behaviour behaviours[] = { walker, hopper, chaser, walker };

static void step(struct actor *a) {
    switch (a->state) {
    case IDLE:
        if (--a->timer == 0) a->state = WALK;
        break;
    case WALK:
        a->x += a->vx;
        if (!solid(a->x, a->y + 2048)) a->state = FALL;
        break;
    case JUMP:
        a->x += a->vx;
        a->y += a->vy;
        a->vy += 24;
        if (a->vy >= 0) a->state = FALL;
        break;
    case FALL:
        a->x += a->vx;
        a->y += a->vy;
        a->vy = (int16_t)(a->vy + 24 > 1024 ? 1024 : a->vy + 24);
        if (solid(a->x, a->y + 2048)) {
            a->y &= ~2047;
            a->vy = 0;
            a->state = WALK;
        }
        break;
    case HURT:
        a->hp = (uint8_t)(a->hp - 1);
        a->state = a->hp ? IDLE : WALK;
        a->timer = (uint8_t)(1 + next_random(a) % 30);
        if (!a->hp) a->hp = (uint8_t)(3 + next_random(a) % 5);
        break;
    }
    behaviours[a->type](a);
    if ((next_random(a) & 255) == 0) a->state = HURT;
}

static uint32_t sort_and_build(void) {
    for (uint32_t i = 0; i < ACTORS; i++) order[i] = (uint8_t)i;
    for (uint32_t i = 1; i < ACTORS; i++) {
        uint8_t key = order[i];
        int32_t depth = actors[key].y;
        uint32_t j = i;
        while (j > 0 && actors[order[j - 1]].y > depth) {
            order[j] = order[j - 1];
            j--;
        }
        order[j] = key;
    }
    uint32_t checksum = 0;
    for (uint32_t i = 0; i < ACTORS; i++) {
        const struct actor *a = &actors[order[i]];
        uint16_t *entry = &shadow_oam[i * 4];
        entry[0] = (uint16_t)(((uint32_t)a->y >> 8) & 255);
        entry[1] = (uint16_t)(((uint32_t)a->x >> 8) & 511);
        entry[2] = (uint16_t)(a->type * 4 + a->state);
        checksum = (checksum ^ entry[0] ^ (uint32_t)entry[1] << 9 ^ (uint32_t)entry[2] << 20) * 16777619u;
    }
    return checksum;
}

static void setup(void) {
    uint32_t seed = 99;
    for (uint32_t i = 0; i < MAP_SIZE * MAP_SIZE; i++) {
        seed = seed * 1103515245u + 12345u;
        map[i] = (uint8_t)(seed >> 24);
    }
    for (uint32_t i = 0; i < ACTORS; i++) {
        struct actor *a = &actors[i];
        a->seed = i * 7919u + 1;
        a->x = (int32_t)(next_random(a) << 8);
        a->y = (int32_t)(next_random(a) << 8);
        a->vx = (int16_t)((int32_t)(next_random(a) & 255) - 128);
        a->state = (uint8_t)(next_random(a) % STATES);
        a->type = (uint8_t)(i & 3);
        a->timer = (uint8_t)(1 + i % 40);
        a->hp = 3;
    }
}

int main(void) {
    runtime_init(0);
    setup();
    REG_DISPCNT = 3 | 0x0400; /* mode 3, BG2 */
    uint32_t checksum = 0;
    start_frames();
    for (;;) {
        show_status(checksum);
        work_begin();
        for (uint32_t pass = 0; pass < PASSES; pass++) {
            for (uint32_t i = 0; i < ACTORS; i++) step(&actors[i]);
            checksum ^= sort_and_build();
        }
        work_end();
        vblank_wait();
    }
}
