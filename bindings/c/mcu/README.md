# MCU reference firmware (Phase 32 bring-up)

Freestanding C11 subset for the pinned reference target (work-plan M15W1–M15W2,
Phase 32). Source of truth: [mcu-bring-up.md](../../../docs/spec/mcu-bring-up.md),
[serial-mcu.md](../../../docs/algorithms/router/serial-mcu.md).

- `hello/main.c` — hello-world firmware proving the cross-compile job.
- `drm_serial.{h,c}` — COBS + CRC32C bounded serial framing, byte-for-byte
  parity with `drm-router::transport::serial` (I-SR3/I-SR4/I-SR6,
  `REQ-OPS-004`).
- `drm_node.{h,c}` — static single-producer/single-consumer topic ring: the
  zero-heap MCU publish/read data path (I-SR5, `REQ-CORE-087`).
- `drm_spatial.{h,c}` — rigid `SE(3)` subset (compose/apply) and the single
  straight-line multiply applying a build-time collapsed constant to a dynamic
  edge (I-S13, `REQ-SPATIAL-017`).
- `selftest.c` — host self-test over golden COBS/CRC vectors shared with the
  Rust suite and reference cells for the spatial subset; excluded from the
  firmware build.
- `bench.c` — host timing bound for the O(1) spatial subset; excluded from the
  firmware build.
- `Makefile` — `arm-none-eabi-gcc 13.3.rel1` cross-build (`make all`), host
  parity (`make host-test`), spatial timing bound (`make host-bench`), and the
  zero-forbidden-heap symbol audit (`make host-audit`, `make audit`).
- CI job: `.github/workflows/mcu-cross.yml` (builds both locked targets, runs
  the host parity self-test, then the zero-forbidden-heap symbol audit).

Subset rules (codebase.md §3.3, I-SR5): no heap, no recursion, statically
bounded loops, `stdint.h` fixed-width types only, no `stdio` on the data path.
