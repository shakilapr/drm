/*
 * MCU spatial subset — rigid SE(3) ops and the single straight-line dynamic
 * multiply (docs/algorithms/spatial/spatial-core.md §Functions
 * `emit_mcu_constants`, §Invariants I-S13; docs/algorithms/router/serial-mcu.md
 * I-SR5; work-plan M21W3-E1/E2/E3).
 *
 * Freestanding C11 (codebase.md 3.3): fixed-width types only, no heap, no
 * recursion, statically bounded loops. An MCU needing camera coordinates
 * receives the collapsed `base_to_frame` constant at build time
 * (`emit_mcu_constants`); when it receives the dynamic `world_to_base` edge over
 * the serial/network link it executes one straight-line multiply with zero heap
 * allocation and zero tree traversal (spatial_architecture.md §Shared Memory and
 * Microcontrollers).
 *
 * REQ: REQ-SPATIAL-017 (MCU receives pre-calculated static constants at build;
 * straight-line multiply at runtime), REQ-SPATIAL-049 (contiguous static chains
 * collapse at build), REQ-CORE-087 (MCU data path: no heap, no recursion,
 * bounded loops), REQ-OPS-004 (host interop).
 */
#ifndef DRM_MCU_DRM_SPATIAL_H
#define DRM_MCU_DRM_SPATIAL_H

#include <stdint.h>

#ifdef __cplusplus
extern "C" {
#endif

/* Fixed-size rigid transform: translation [X, Y, Z] plus a unit quaternion
 * [X, Y, Z, W]. No heap, no variable-length layout (I-SR5). */
typedef struct drm_spatial_transform {
    double t[3];
    double q[4];
} drm_spatial_transform;

/* out = a ∘ b (apply b, then a). Renormalizes the composed quaternion. */
void drm_spatial_compose(drm_spatial_transform *out, const drm_spatial_transform *a,
                         const drm_spatial_transform *b);

/* Apply t to a point: rotation then translation. */
void drm_spatial_apply_point(const drm_spatial_transform *t, const double p[3], double out[3]);

/* Apply t to a direction/vector: rotation only. */
void drm_spatial_apply_direction(const drm_spatial_transform *t, const double v[3], double out[3]);

/* I-S13: one straight-line multiply of a collapsed `base_to_frame` constant
 * against a dynamic `world_to_base` edge received over the link, then apply to a
 * point. Zero heap, zero strings, zero tree traversal. */
void drm_spatial_apply_dynamic(const drm_spatial_transform *base_to_frame,
                               const drm_spatial_transform *world_to_base,
                               const double point_in_frame[3], double out_world[3]);

#ifdef __cplusplus
}
#endif

#endif /* DRM_MCU_DRM_SPATIAL_H */
