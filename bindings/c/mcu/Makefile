# Reference MCU cross-compile (work-plan M15W1-E3-T1; docs/spec/mcu-bring-up.md).
# Pinned toolchain: GNU Arm Embedded arm-none-eabi-gcc 13.3.rel1.
# REQ: REQ-CORE-087.

CC := arm-none-eabi-gcc
CFLAGS := -std=c11 -ffreestanding -fno-common -fno-builtin -Os -g3 \
	-Wall -Wextra -Wpedantic -Wconversion -Wshadow -Werror

# Locked targets (mcu-bring-up.md §1): reference STM32H743 (Cortex-M7),
# fallback STM32F407 (Cortex-M4F).
TARGETS := h743 f407

.PHONY: all $(TARGETS) clean
all: $(TARGETS)

h743:
	$(CC) $(CFLAGS) -mcpu=cortex-m7 -mthumb -c hello/main.c -o hello/h743.o

f407:
	$(CC) $(CFLAGS) -mcpu=cortex-m4 -mthumb -c hello/main.c -o hello/f407.o

clean:
	rm -f hello/*.o
