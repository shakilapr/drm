/*
 * MCU spatial subset implementation (work-plan M21W3-E1-T1). Freestanding C11:
 * no heap, no recursion, statically bounded loops (codebase.md 3.3, I-SR5). The
 * math mirrors the host SE(3) reference (`drm-spatial::transform`) and the
 * emitted artifact (`drm-workspace::spatial::emit_mcu_constants`).
 *
 * REQ: REQ-SPATIAL-017, REQ-SPATIAL-049, REQ-CORE-087.
 */
#include "drm_spatial.h"

/* Normalizes t->q in place; a degenerate quaternion collapses to identity. */
static void drm_spatial_quat_normalize(drm_spatial_transform *t) {
    double n = 0.0;
    int i;
    for (i = 0; i < 4; ++i) {
        n += t->q[i] * t->q[i];
    }
    n = __builtin_sqrt(n);
    if (n <= 1e-12) {
        t->q[0] = 0.0;
        t->q[1] = 0.0;
        t->q[2] = 0.0;
        t->q[3] = 1.0;
        return;
    }
    for (i = 0; i < 4; ++i) {
        t->q[i] /= n;
    }
}

/* Rotate a displacement by t->q (rotation only). */
static void drm_spatial_rotate(const drm_spatial_transform *t, const double v[3], double out[3]) {
    double tx = 2.0 * (t->q[1] * v[2] - t->q[2] * v[1]);
    double ty = 2.0 * (t->q[2] * v[0] - t->q[0] * v[2]);
    double tz = 2.0 * (t->q[0] * v[1] - t->q[1] * v[0]);
    out[0] = v[0] + t->q[3] * tx + (t->q[1] * tz - t->q[2] * ty);
    out[1] = v[1] + t->q[3] * ty + (t->q[2] * tx - t->q[0] * tz);
    out[2] = v[2] + t->q[3] * tz + (t->q[0] * ty - t->q[1] * tx);
}

void drm_spatial_compose(drm_spatial_transform *out, const drm_spatial_transform *a,
                         const drm_spatial_transform *b) {
    double rx = b->t[0];
    double ry = b->t[1];
    double rz = b->t[2];
    double tx = 2.0 * (a->q[1] * rz - a->q[2] * ry);
    double ty = 2.0 * (a->q[2] * rx - a->q[0] * rz);
    double tz = 2.0 * (a->q[0] * ry - a->q[1] * rx);
    out->t[0] = a->t[0] + rx + a->q[3] * tx + (a->q[1] * tz - a->q[2] * ty);
    out->t[1] = a->t[1] + ry + a->q[3] * ty + (a->q[2] * tx - a->q[0] * tz);
    out->t[2] = a->t[2] + rz + a->q[3] * tz + (a->q[0] * ty - a->q[1] * tx);
    out->q[0] = a->q[3] * b->q[0] + a->q[0] * b->q[3] + a->q[1] * b->q[2] - a->q[2] * b->q[1];
    out->q[1] = a->q[3] * b->q[1] - a->q[0] * b->q[2] + a->q[1] * b->q[3] + a->q[2] * b->q[0];
    out->q[2] = a->q[3] * b->q[2] + a->q[0] * b->q[1] - a->q[1] * b->q[0] + a->q[2] * b->q[3];
    out->q[3] = a->q[3] * b->q[3] - a->q[0] * b->q[0] - a->q[1] * b->q[1] - a->q[2] * b->q[2];
    drm_spatial_quat_normalize(out);
}

void drm_spatial_apply_point(const drm_spatial_transform *t, const double p[3], double out[3]) {
    double r[3];
    drm_spatial_rotate(t, p, r);
    out[0] = r[0] + t->t[0];
    out[1] = r[1] + t->t[1];
    out[2] = r[2] + t->t[2];
}

void drm_spatial_apply_direction(const drm_spatial_transform *t, const double v[3], double out[3]) {
    drm_spatial_rotate(t, v, out);
}

void drm_spatial_apply_dynamic(const drm_spatial_transform *base_to_frame,
                               const drm_spatial_transform *world_to_base,
                               const double point_in_frame[3], double out_world[3]) {
    drm_spatial_transform world_to_frame = {0};
    drm_spatial_compose(&world_to_frame, world_to_base, base_to_frame);
    drm_spatial_apply_point(&world_to_frame, point_in_frame, out_world);
}
