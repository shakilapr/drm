/*
 * Reference MCU bring-up firmware (work-plan M15W1-E3-T1).
 *
 * Freestanding C11 subset (codebase.md 3.3): fixed-width types only, no heap,
 * no recursion, statically bounded loops. This hello-world proves the pinned
 * arm-none-eabi toolchain and the CI cross-compile job; the DRM serial subset
 * (drm_serial_encode/drm_serial_decode) lands with the Phase 32 data path.
 *
 * REQ: REQ-CORE-087 (MCU data path: no heap, no recursion, bounded loops),
 * REQ-OPS-004 (serial framing interoperates byte-for-byte with the host).
 */

#include <stdint.h>

/* Linker-placed firmware entry; never returns. */
void drm_mcu_main(void);

volatile uint32_t drm_mcu_heartbeat;

void drm_mcu_main(void) {
    for (uint32_t i = 0; i < 8U; ++i) {
        drm_mcu_heartbeat = i;
    }
    for (;;) {
        drm_mcu_heartbeat = (drm_mcu_heartbeat + 1U) & 0xFFFFU;
    }
}
