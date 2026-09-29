OUTPUT_FORMAT("elf32-littlearm")
ENTRY(_start)

MEMORY {
    rom   : ORIGIN = 0x08000000, LENGTH = 32M
    ewram : ORIGIN = 0x02000000, LENGTH = 256K
    iwram : ORIGIN = 0x03000000, LENGTH = 0x7E00
}

SECTIONS {
    .header : { KEEP(*(.header)) } > rom
    .text : { *(.text*) *(.rodata*) . = ALIGN(4); } > rom
    .iwram : {
        __iwram_start = .;
        /* Copied before the VBlank IRQ starts (crt0.s): keep it small. */
        *(.iwram) *(.iwram.data) *(.data*)
        . = ALIGN(4);
        __iwram_end = .;
    } > iwram AT > rom
    __iwram_load = LOADADDR(.iwram);
    .iwram_bss (NOLOAD) : {
        __iwram_bss_start = .;
        *(.iwram_bss*)
        . = ALIGN(4);
        __iwram_bss_end = .;
    } > iwram
    .bss (NOLOAD) : {
        __bss_start = .;
        *(.bss*) *(COMMON)
        . = ALIGN(4);
        __bss_end = .;
    } > ewram
    /DISCARD/ : { *(.eh_frame*) *(.comment*) *(.ARM.attributes*) }
}
