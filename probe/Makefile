# Builds the probe ROMs with any arm-none-eabi-gcc (ARMv4T, no libgba):
# every *.c except report.c is one ROM. Output ROMs are gitignored build
# artifacts (build/*.gba).

CC = arm-none-eabi-gcc
OBJCOPY = arm-none-eabi-objcopy
NM = arm-none-eabi-nm
CFLAGS = -mcpu=arm7tdmi -marm -O1 -ffreestanding -nostdlib -Wall -Wextra
LDFLAGS = -T rom.ld -nostdlib -Wl,--no-warn-rwx-segments

BUILD = build
PROBES = $(filter-out report,$(basename $(wildcard *.c)))

all: $(PROBES:%=$(BUILD)/%.gba)

# The validators poll report_and_idle at a fixed address (see rom.ld).
$(BUILD)/%.elf: %.c report.c header.s gba.h rom.ld | $(BUILD)
	$(CC) $(CFLAGS) $(LDFLAGS) header.s $< report.c -o $@
	@$(NM) $@ | grep -q "08000304 . idle_loop" || \
		(echo "$@: idle_loop is not at 0x08000304" && exit 1)

# A variant defines one switch and includes the probe it varies.
$(BUILD)/affine_wrap.elf: affine_rot.c
$(BUILD)/brighten_dark.elf: brighten.c
$(BUILD)/sweep_down.elf: sweep_up.c

# Patches the header complement over 0xA0..0xBC.
$(BUILD)/%.gba: $(BUILD)/%.elf
	$(OBJCOPY) -O binary $< $@
	python3 -c "import sys; d = bytearray(open('$@','rb').read()); \
		d[0xBD] = (~sum(d[0xA0:0xBD]) & 0xFF); \
		open('$@','wb').write(d)"

$(BUILD):
	mkdir -p $(BUILD)

clean:
	rm -rf $(BUILD)

.PHONY: all clean
