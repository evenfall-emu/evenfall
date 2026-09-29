/* GBA cartridge header + entry. Logo bytes are zeros: these probe ROMs are
 * CI fixtures only and make no real-hardware boot claim. Fixed byte 0x96 is
 * present; the complement at 0xBD is patched by the Makefile. */

    .section .header,"ax"
    .arm
    .global _start
_start:
    b entry
    .fill 156,1,0                /* Nintendo logo (zeroed: CI only) */
    .ascii "ZB TIMER1   "        /* game title, 12 bytes */
    .ascii "ZB01"                /* game code */
    .ascii "ZB"                  /* maker code */
    .byte 0x96                   /* fixed */
    .byte 0x00                   /* unit code */
    .byte 0x00                   /* device type */
    .space 7,0                   /* reserved */
    .byte 0x00                   /* game version */
    .byte 0x00                   /* header complement (Makefile patches) */
    .space 2,0                   /* checksum */

    .text
entry:
    ldr sp, =0x03007F00
    bl main
    b .
