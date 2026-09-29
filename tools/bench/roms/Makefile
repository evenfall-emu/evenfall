# Builds the emu-bench workload ROMs with any arm-none-eabi-gcc (ARMv4T, no
# libgba): every *.c except runtime.c is one ROM. Game-like code generation:
# Thumb in ROM at -O2, with IWRAM_ARM functions compiled as ARM.
# usage: make -C tools/emu-bench/roms [BUILD=DIR]

CC = arm-none-eabi-gcc
OBJCOPY = arm-none-eabi-objcopy
CFLAGS = -mcpu=arm7tdmi -mthumb -mthumb-interwork -O2 -ffreestanding -nostdlib -Wall -Wextra
LDFLAGS = -T rom.ld -nostdlib -Wl,--no-warn-rwx-segments -lgcc

BUILD = build
# CALIBRATE=1 also shows each ROM's worst-case work scanlines (row 3).
ifdef CALIBRATE
CFLAGS += -DCALIBRATE
endif
ROMS = $(filter-out runtime,$(basename $(wildcard *.c)))

all: $(ROMS:%=$(BUILD)/%.gba)

$(BUILD)/%.elf: %.c runtime.c crt0.s bench.h rom.ld | $(BUILD)
	$(CC) $(CFLAGS) crt0.s $< runtime.c $(LDFLAGS) -o $@

# Patches the header complement over 0xA0..0xBC.
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
