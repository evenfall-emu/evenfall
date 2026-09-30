/* Cartridge header and entry for the emu-bench ROMs. Logo bytes are zeros:
 * the ROMs boot with the BIOS animation skipped, never on real hardware.
 * The header complement at 0xBD is patched by the Makefile. */

    .section .header,"ax"
    .arm
    .global _start
_start:
    b entry
    .fill 156,1,0                /* Nintendo logo (zeroed) */
    .ascii "EMU BENCH   "        /* game title, 12 bytes */
    .ascii "ZEB0"                /* game code: no retail feature string */
    .ascii "ZB"                  /* maker code */
    .byte 0x96                   /* fixed */
    .byte 0x00                   /* unit code */
    .byte 0x00                   /* device type */
    .space 7,0                   /* reserved */
    .byte 0x00                   /* game version */
    .byte 0x00                   /* header complement (Makefile patches) */
    .space 2,0                   /* checksum */

    .text
    .arm
entry:
    /* Establish the state the BIOS leaves at 0x08000000 (IRQ and SVC stacks,
     * system mode with IRQs enabled). A skipped boot does not always do this:
     * pinned NanoBoyAdvance's skip_bios keeps the reset CPSR's IRQ mask. */
    msr cpsr_c, #0xD2
    ldr sp, =0x03007FA0
    msr cpsr_c, #0xD3
    ldr sp, =0x03007FE0
    msr cpsr_c, #0x1F
    ldr sp, =0x03007F00
    /* Copy IWRAM code/data from ROM, then zero both .bss sections. */
    ldr r0, =__iwram_load
    ldr r1, =__iwram_start
    ldr r2, =__iwram_end
1:  cmp r1, r2
    ldrlo r3, [r0], #4
    strlo r3, [r1], #4
    blo 1b
    /* Count VBlanks from the entry point, before any setup: skipped boots
     * leave the PPU at different lines (mGBA 126, NanoBoyAdvance 225), so
     * a VBlank can fall inside a long .bss clear. */
    ldr r0, =0x03007FFC
    ldr r1, =irq_handler
    str r1, [r0]
    mov r0, #0x04000000
    mov r1, #0x0008              /* DISPSTAT: VBlank IRQ */
    strh r1, [r0, #0x04]
    add r0, r0, #0x200
    ldr r1, =0xFFFF
    strh r1, [r0, #0x02]         /* IF: clear stale requests */
    mov r1, #1
    strh r1, [r0]                /* IE: VBlank */
    strh r1, [r0, #0x08]         /* IME */
    mov r3, #0
    ldr r1, =__iwram_bss_start
    ldr r2, =__iwram_bss_end
2:  cmp r1, r2
    strlo r3, [r1], #4
    blo 2b
    ldr r1, =__bss_start
    ldr r2, =__bss_end
3:  cmp r1, r2
    strlo r3, [r1], #4
    blo 3b
    ldr r0, =main
    mov lr, pc
    bx r0
    b .
    .pool
