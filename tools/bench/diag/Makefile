# Dispatch diagnostics: bare loops that isolate the CPU interpreter's cost per
# guest instruction. Display blanked, no interrupts, DMA or timers.
# usage: make -C tools/bench/diag [BUILD=DIR]
#   arm-alu.gba     ARM ALU instructions running from IWRAM
#   arm-mem.gba     ARM IWRAM loads and stores running from IWRAM
#   thumb-rom.gba   Thumb ALU instructions running from Game Pak ROM
# Guest instructions per frame are fixed by construction; see README.md.

CC = arm-none-eabi-gcc
OBJCOPY = arm-none-eabi-objcopy
FLAGS = -mcpu=arm7tdmi -nostdlib -T ../../../probe/rom.ld -Wl,--no-warn-rwx-segments
BUILD = build

all: $(BUILD)/arm-alu.gba $(BUILD)/arm-mem.gba $(BUILD)/thumb-rom.gba

$(BUILD)/arm-alu.elf: KIND = 0
$(BUILD)/arm-mem.elf: KIND = 1
$(BUILD)/thumb-rom.elf: KIND = 2

$(BUILD)/%.elf: dispatch.s | $(BUILD)
	$(CC) $(FLAGS) -Wa,--defsym,KIND=$(KIND) $< -o $@

$(BUILD)/%.gba: $(BUILD)/%.elf
	$(OBJCOPY) -O binary $< $@
	python3 -c "import sys; d = bytearray(open('$@','rb').read()); \
		d[0xBD] = -(sum(d[0xA0:0xBD]) + 0x19) & 0xFF; \
		open('$@','wb').write(d)"

$(BUILD):
	mkdir -p $(BUILD)

clean:
	rm -rf $(BUILD)

.PHONY: all clean
