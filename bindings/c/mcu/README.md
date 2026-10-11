# MCU reference firmware (Phase 32 bring-up)

Freestanding C11 subset for the pinned reference target (work-plan M15W1-E3).
Source of truth: [mcu-bring-up.md](../../../docs/spec/mcu-bring-up.md),
[serial-mcu.md](../../../docs/algorithms/router/serial-mcu.md).

- `hello/main.c` — hello-world firmware proving the cross-compile job.
- `Makefile` — `arm-none-eabi-gcc 13.3.rel1`, `-std=c11 -ffreestanding -Os
  -Wall -Wextra -Wpedantic -Wconversion -Wshadow -Werror`.
- CI job: `.github/workflows/mcu-cross.yml` (builds both locked targets, then
  the zero-forbidden-heap symbol audit).

Subset rules (codebase.md §3.3, I-SR5): no heap, no recursion, statically
bounded loops, `stdint.h` fixed-width types only, no `stdio` on the data path.
