/* Probe 56: OBJ window masking (Mode 0).
 *
 * Shows a full-screen red BG0 with one 8x8 sprite at (80,60) in object
 * window mode (attr0 bits 10-11 = 2, DISPCNT WINOBJ bit). WINOUT masks
 * nothing outside while the OBJ-window field enables BG0 only, so the
 * 8x8 sprite rectangle renders red and everything outside shows the black
 * backdrop; the sprite itself is invisible. The window sprite needs an
 * opaque tile: like mGBA (SPRITE_DRAW_PIXEL_16_OBJWIN sets FLAG_OBJWIN
 * only for nonzero pixels) the window covers opaque pixels, so tile 1 is
 * filled solid. After two frames the validator checks every pixel analytically. Exercises the OBJ-window path end to
 * end; WIN0/WIN1 masking stays with the window_rect probe. HLE boot only.
 */

#include "gba.h"

#define REG_DISPCNT 0x00
#define REG_BG0CNT 0x08
#define REG_WINOUT 0x4A
#define REG_VCOUNT 0x06
#define BG_TILES ((volatile uint16_t *)0x06000000)
#define BG_MAP ((volatile uint16_t *)0x0600F800)
#define PALETTE ((volatile uint16_t *)0x05000000)
#define OBJ_TILES ((volatile uint16_t *)0x06010000)
#define OBJ_ATTRS ((volatile uint16_t *)0x07000000)

void report_and_idle(uint32_t value);

int main(void) {
    MEM_IO[REG_DISPCNT / 2] = 0x9100u;
    MEM_IO[REG_BG0CNT / 2] = 0x1F00u;
    MEM_IO[REG_WINOUT / 2] = 0x0100u;

    PALETTE[1] = 0x001Fu;

    for (uint32_t i = 0; i < 16u; i++) {
        BG_TILES[i] = 0x1111u;
    }
    for (uint32_t i = 0; i < 1024u; i++) {
        BG_MAP[i] = 0x0000u;
    }

    OBJ_ATTRS[0] = 0x083Cu;
    OBJ_ATTRS[1] = 0x0050u;
    OBJ_ATTRS[2] = 0x0001u;

    /* The OBJ window covers only opaque pixels: give the sprite a tile. */
    for (uint32_t i = 0; i < 16u; i++) {
        OBJ_TILES[16 + i] = 0x1111u;
    }

    /* Zeroed OAM entries are live (0,0) sprites: park the rest offscreen. */
    for (uint32_t i = 1u; i < 128u; i++) {
        OBJ_ATTRS[i * 4] = 0x0200u;
    }

    while (MEM_IO[REG_VCOUNT / 2] != 0) {
    }
    for (uint32_t f = 0; f < 2u; f++) {
        while (MEM_IO[REG_VCOUNT / 2] == 0) {
        }
        while (MEM_IO[REG_VCOUNT / 2] != 0) {
        }
    }
    report_and_idle(0);
    return 0;
}
