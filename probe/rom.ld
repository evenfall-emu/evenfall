OUTPUT_FORMAT("elf32-littlearm")
ENTRY(_start)

SECTIONS {
    . = 0x08000000;
    .header : { KEEP(*(.header)) }
    /* report_and_idle sits at the fixed address validator/probe_cases.rs polls
     * for. Code follows it rather than preceding it, so a probe of any size
     * links with any compiler version. */
    .idle 0x08000300 : { KEEP(*(.idle)) }
    .text : { *(.text*) *(.rodata*) }
    /* Uninitialized globals live in EWRAM (fresh-zeroed at boot); without
     * this they fall through into ROM, where writes silently vanish. No
     * .data copy: probe ROMs must not rely on initialized globals. */
    . = 0x02000000;
    .bss : { *(.bss*) *(COMMON) }
    /DISCARD/ : { *(.eh_frame*) *(.comment*) *(.ARM.attributes*) }
}
