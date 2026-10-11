# Reference MCU cross-compile + host parity (work-plan M15W1-E3-T1,
# M15W2-E3-T1/T2; docs/spec/mcu-bring-up.md, serial-mcu.md I-SR5/I-SR6).
# Pinned toolchain: GNU Arm Embedded arm-none-eabi-gcc 13.3.rel1.
# REQ: REQ-CORE-087, REQ-OPS-004.

CC := arm-none-eabi-gcc
HOST_CC := cc
INCLUDES := -I../include
CFLAGS := -std=c11 -ffreestanding -fno-common -fno-builtin -Os -g3 \
	-Wall -Wextra -Wpedantic -Wconversion -Wshadow -Werror $(INCLUDES)
HOST_CFLAGS := -std=c11 -Wall -Wextra -Wpedantic -Wconversion -Wshadow -Werror $(INCLUDES)

# Locked targets (mcu-bring-up.md §1): reference STM32H743 (Cortex-M7),
# fallback STM32F407 (Cortex-M4F).
TARGETS := h743 f407

.PHONY: all $(TARGETS) host-test host-audit host-bench audit clean

all: $(TARGETS)

h743:
	$(CC) $(CFLAGS) -mcpu=cortex-m7 -mthumb -c drm_serial.c -o drm_serial_h743.o
	$(CC) $(CFLAGS) -mcpu=cortex-m7 -mthumb -c drm_node.c -o drm_node_h743.o
	$(CC) $(CFLAGS) -mcpu=cortex-m7 -mthumb -c drm_spatial.c -o drm_spatial_h743.o
	$(CC) $(CFLAGS) -mcpu=cortex-m7 -mthumb -c hello/main.c -o hello_h743.o

f407:
	$(CC) $(CFLAGS) -mcpu=cortex-m4 -mthumb -c drm_serial.c -o drm_serial_f407.o
	$(CC) $(CFLAGS) -mcpu=cortex-m4 -mthumb -c drm_node.c -o drm_node_f407.o
	$(CC) $(CFLAGS) -mcpu=cortex-m4 -mthumb -c drm_spatial.c -o drm_spatial_f407.o
	$(CC) $(CFLAGS) -mcpu=cortex-m4 -mthumb -c hello/main.c -o hello_f407.o

# Host parity: compile the freestanding subset with the host compiler and run
# the golden-vector self-test (byte-for-byte interop with the Rust codec) and
# the spatial subset reference checks.
host-test:
	$(HOST_CC) $(HOST_CFLAGS) drm_serial.c drm_node.c drm_spatial.c selftest.c -o drm_selftest_host -lm
	./drm_selftest_host

# Zero-forbidden-heap audit: no malloc/calloc/realloc/free reachable from the
# data-path objects (I-SR5; the linker/trap barriers are the primary gate, this
# symbol audit is secondary).
host-audit:
	$(HOST_CC) $(HOST_CFLAGS) -ffreestanding -fno-builtin -fno-tree-loop-distribute-patterns -Os -c drm_serial.c -o drm_serial_audit.o
	$(HOST_CC) $(HOST_CFLAGS) -ffreestanding -fno-builtin -fno-tree-loop-distribute-patterns -Os -c drm_node.c -o drm_node_audit.o
	$(HOST_CC) $(HOST_CFLAGS) -ffreestanding -fno-builtin -fno-tree-loop-distribute-patterns -Os -c drm_spatial.c -o drm_spatial_audit.o
	@for obj in drm_serial_audit.o drm_node_audit.o drm_spatial_audit.o; do \
		if nm "$$obj" | grep -E ' (malloc|calloc|realloc|free)$$'; then \
			echo "forbidden allocator symbol reachable from $$obj"; exit 1; \
		fi; \
	done
	@echo "mcu audit: zero forbidden allocator symbols"

# Host timing bound for the O(1) spatial subset (work-plan M21W3-E3-T1): the
# single-multiply dynamic apply and compose must stay under the declared
# constant-time ceiling; the same source is cross-compiled for the targets.
host-bench:
	$(HOST_CC) $(HOST_CFLAGS) drm_spatial.c bench.c -o drm_bench_host -lm
	./drm_bench_host

audit:
	@found=0; for obj in *.o; do \
		[ -e "$$obj" ] || continue; found=1; \
		if arm-none-eabi-nm "$$obj" | grep -E ' (malloc|calloc|realloc|free)$$'; then \
			echo "forbidden allocator symbol reachable from $$obj"; exit 1; \
		fi; \
	done; \
	if [ "$$found" = 0 ]; then echo "mcu audit: no cross objects (run \`make all\`)"; exit 1; fi; \
	echo "mcu audit: zero forbidden allocator symbols"

clean:
	rm -f *.o drm_selftest_host drm_bench_host
