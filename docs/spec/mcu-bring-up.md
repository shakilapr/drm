# MCU Target & Bring-Up Plan (M14W3)

Non-normative derived spec. Source of truth: [serial-mcu.md](../algorithms/router/serial-mcu.md)
(§Invariants I-SR1..I-SR7, §Protocols MCU steady-state discipline),
[codebase.md](../../codebase.md) §3.3 (MCU subset), [architecture.md](../architecture.md)
§External Device Interoperability. On any conflict the architecture docs win; on any
pseudocode conflict the algorithm docs win (codebase.md §2.4). Owner: E3. Phase 32 prep.
Written M14W3.

Locks the reference constrained-device target, pins the toolchain, and specifies the
cross-compile CI job, the host↔MCU serial bring-up sequence, and the zero-forbidden-heap
audit mechanics required before Phase 32 implementation (M15W1).

## REQ Bindings

| Provisional slug | Architecture anchor | Numeric ID |
|---|---|---|
| `REQ-ROUTE-serial-bounded-frames` | one DRM sample = one bounded serial frame | `REQ-ROUTE-025` |
| `REQ-ROUTE-serial-best-effort` | serial is best-effort; `reliable` rejected at verify | `REQ-ROUTE-024` |
| `REQ-ROUTE-serial-crc` | CRC when the link lacks hardware checksums | `REQ-ROUTE-026` |
| `REQ-ROUTE-serial-blocking` | non-preemptible TX; verify reports worst-case blocking | `REQ-ROUTE-040` |
| `REQ-CORE-mcu-zero-heap` | MCU data path: no heap, no recursion, bounded loops | `REQ-CORE-087` |
| `REQ-CORE-mcu-opaque-payload` | third-party device packets via `bytes[<=N]` opaque mapping | `REQ-CORE-086` |
| `REQ-OPS-mcu-interop` | serial framing interoperates byte-for-byte with host | `REQ-OPS-004` |
| `REQ-SPATIAL-mcu-constants` | MCU receives pre-calculated static constants at build | `REQ-SPATIAL-017` |

## 1. Reference target and fallback (locked)

| Role | Target | RTOS | Rationale |
|---|---|---|---|
| Reference | STM32H743 (ARM Cortex-M7, 480 MHz, 1 MiB SRAM, D-cache) | FreeRTOS kernel 11.1.0 (static allocation only) | Representative high-end MCU: caches exercise the bounded-loop/`volatile` discipline; dual-bank flash supports A/B firmware; widely available on dev boards. |
| Fallback | STM32F407 (ARM Cortex-M4F, 168 MHz, 192 KiB SRAM, no cache) | FreeRTOS kernel 11.1.0 (static allocation only) | Lower-cost, no-cache part; proves the subset is not cache-dependent. Locked as the second CI target so a reference-board shortage cannot block bring-up. |
| Bare-metal check | Cortex-M4F without RTOS | none | Confirms the subset links with no RTOS dependency (`drm_serial_*` only). |

Both targets are fixed for the whole wave; adding a third MCU family requires an ADR
(serial-mcu.md §Escape/stuffing options precedent — decisions re-open only against
pre-stated criteria).

## 2. Toolchain (pinned)

| Component | Pin | Notes |
|---|---|---|
| Compiler | GNU Arm Embedded Toolchain `arm-none-eabi-gcc` 13.3.rel1 | C11 freestanding subset |
| C standard | `-std=c11 -ffreestanding -fno-common` | no hosted libc on the data path |
| Warnings | `-Wall -Wextra -Wpedantic -Wconversion -Wshadow -Werror` | codebase.md §3.3 |
| Optimization | `-Os -g3` | reproducible; `-fno-builtin` |
| Linker | `arm-none-eabi-ld` with a generated memory map | per-target section layout (§5) |
| Emulation | QEMU `qemu-system-arm` (STM32 machine) | CI smoke run when no board is attached |
| RTOS | FreeRTOS 11.1.0, `configSUPPORT_DYNAMIC_ALLOCATION=0`, `configSUPPORT_STATIC_ALLOCATION=1` | dynamic allocation disabled (I-SR5) |

The host-side codec is the existing `drm-router::transport::serial` COBS/CRC32C
implementation; the MCU side is the freestanding `drm_serial_encode`/`drm_serial_decode`
subset (`bindings/c`). Both share the canonical covered-byte sequence (I-SR3) so the
interop gate is byte-for-byte (I-SR6, `REQ-OPS-004`).

## 3. MCU subset rules (from codebase.md §3.3, serial-mcu.md I-SR5)

- No heap: no allocator linked; `malloc`/`calloc`/`realloc`/`free` are link errors or traps.
- No recursion; all data-path loops statically bounded (`for i in 0..MAX_FRAMES_PER_TICK`).
- `stdint.h` fixed-width types only; no `stdio` on the data path.
- All buffers static storage or linker-placed pools; no stack-overflow-prone patterns.
- Opaque third-party payloads map to `bytes[<=N]` (`REQ-CORE-086`); never a heap copy.

## 4. CI cross-compile job plan

| Job | Trigger | Steps |
|---|---|---|
| `mcu-cross` | every PR touching `bindings/c` or `crates/drm-router/src/transport/serial.rs` | compile the MCU subset for both targets with the pinned toolchain; assert `-Werror` clean; run the host↔MCU codec vectors under QEMU |
| `mcu-zero-heap` | same + nightly | link the firmware; run the linker-section and symbol audits (§5); fail on any reachable allocator symbol |
| `mcu-stack` | nightly (board or QEMU) | run the steady-state suite; assert measured min-free stack ≥ the declared per-profile margin |

The job reuses `xtask test --cross-os --transport serial` for the host-loopback codec and
adds the MCU object files as an additional decoder in the cross-decode matrix (I-SR6). No
silent skips: a missing board falls back to QEMU, never to "not run".

## 5. Zero-forbidden-heap audit mechanics (linker sections)

Enforcement is **preventive first**, audit secondary (I-SR5, codebase.md §3.2):

1. **No heap linked**: the linker script places no `_heap`/`_sbrk` region; `_sbrk` is
   undefined so any allocator call is a link error.
2. **Symbol traps**: `malloc`, `free`, `calloc`, `realloc`, and C++ `operator new`/`delete`
   are defined as `__attribute__((noreturn))` traps; the symbol audit fails if they appear
   reachable from the publish/read call graph.
3. **RTOS allocation off**: `configSUPPORT_DYNAMIC_ALLOCATION=0`; FreeRTOS task/queue/mutex
   objects are created from static pools only.
4. **Linker-section audit**: firmware is linked into named sections
   (`.text.drm_datapath`, `.bss.drm_pools`, `.stack.drm_tasks`); a post-link script asserts
   the allocator symbol set is empty within `.text.drm_datapath` and that pool sizes equal
   the compiled-in declarations.
5. **Allocator hooks (target test)**: a test build interposes `_sbrk` with a counting hook;
   the steady-state suite asserts zero calls across a full publish/read cycle.
6. **Stack high-water**: `configCHECK_FOR_STACK_OVERFLOW` plus a paint-and-measure high-water
   pass; for every task, `measured_min_free_stack >= configured_stack_margin`, where the
   margin is declared per platform/build profile — never a universal percentage (I-SR5).

## 6. Host↔MCU serial bring-up steps

1. Flash the MCU with a firmware exposing the opaque `bytes[<=N]` echo topic
   (`REQ-CORE-086`); confirm the A/B bank select and clock bring-up.
2. Host config declares `[route."motor/command".serial] channel = <u32 wire id>`; the link
   is best-effort (`REQ-ROUTE-024`), bounded (`REQ-ROUTE-025`), CRC-protected
   (`REQ-ROUTE-026`).
3. Host publishes a small structured topic; the MCU decodes streamingly with O(1) state and
   publishes an acknowledgement over its own opaque topic.
4. Host decodes the MCU frames through the standard router ingress path
   (router-core.md §Functions `ingress`).
5. Assert byte-for-byte cross-decode in both directions (I-SR6, `REQ-OPS-004`), the
   COBS boundary vectors (decoded 253/254/255/507/508/509), and the corruption campaign
   (all 1-bit flips, all bursts ≤ 32 bits).
6. Record the blocking estimate `blocking_max = wire_time(max lower-priority frame)` and
   verify it against the declared deadline (`REQ-ROUTE-040`).
7. Confirm the MCU receives the collapsed `base_to_camera` constant at build and performs a
   single straight-line multiply at runtime (`REQ-SPATIAL-017`).

## 7. Acceptance (M14W3)

- [x] Reference MCU/RTOS locked; fallback target noted (this document).
- [x] Toolchain pinned; C subset rules restated from codebase.md §3.3.
- [x] CI cross-compile + zero-heap + stack jobs planned.
- [x] Host↔MCU serial bring-up steps specified.
- [x] Zero-forbidden-heap audit mechanics specified (linker sections, preventive-first).

Deltas: none to serial-mcu.md; this plan fixes the concrete target/toolchain that the
algorithm doc left to the wave. Implementation lands in Phase 32 (M15W1).

REQ: REQ-ROUTE-024, REQ-ROUTE-025, REQ-ROUTE-026, REQ-ROUTE-040, REQ-CORE-086,
REQ-CORE-087, REQ-OPS-004, REQ-SPATIAL-017.
