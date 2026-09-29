/* Diagnostic ROMs: a steady loop copied to IWRAM and run forever, with the
 * display in forced blank, no interrupts, no DMA and no timers, so almost all
 * emulator work is CPU dispatch. KIND selects the loop body (32 ops + branch):
 *   0  ARM ALU:          add/eor/orr/sub with registers and immediates
 *   1  ARM IWRAM memory: ldr/str pairs to IWRAM
 *   2  Thumb ALU in ROM: the Thumb equivalent of 0, executed from Game Pak ROM */

    .section .header,"ax"
    .arm
    .global _start
_start:
    b entry
    .fill 156,1,0
    .ascii "EF DIAG     "
    .ascii "EFDG"
    .ascii "EF"
    .byte 0x96
    .byte 0x00
    .byte 0x00
    .space 7,0
    .byte 0x00
    .byte 0x00
    .space 2,0

    .text
    .arm
entry:
    ldr sp, =0x03007F00
    mov r0, #0x04000000
    mov r1, #0x80              /* DISPCNT: forced blank */
    strh r1, [r0]
    mov r1, #0
    str r1, [r0, #0x208]       /* IME off */
.if KIND == 2
    ldr r0, =thumb_loop + 1
    bx r0
.else
    ldr r0, =loop_start        /* copy the loop to IWRAM and jump there */
    ldr r1, =loop_end
    ldr r2, =0x03000000
1:  ldr r3, [r0], #4
    str r3, [r2], #4
    cmp r0, r1
    blo 1b
    mov r4, #1
    mov r5, #3
    mov r6, #5
    ldr r7, =0x03001000
    ldr r2, =0x03000000
    bx r2
.endif

    .arm
    .align 2
loop_start:
.if KIND == 0
    .rept 8
    add r0, r0, r4
    eor r1, r1, r0, lsl #1
    orr r2, r2, #0x10
    sub r3, r3, r5
    .endr
.endif
.if KIND == 1
    .rept 8
    ldr r0, [r7]
    add r0, r0, r4
    str r0, [r7]
    ldr r1, [r7, #4]
    .endr
.endif
    b loop_start
loop_end:

    .thumb
    .align 2
    .thumb_func
thumb_loop:
    .rept 8
    add r0, r0, r4
    eor r1, r0
    orr r2, r5
    sub r3, #1
    .endr
    b thumb_loop
    .pool
