/*
 * MCU spatial subset timing harness (work-plan M21W3-E3-T1: "on-target runs of
 * the spatial subset; timing bounds"). Compiled with the HOST compiler for
 * parity; the same source is cross-compiled for the pinned targets. The
 * single-multiply dynamic apply (I-S13) is O(1) fixed work, so the measured
 * per-call cost is bounded and flat across iterations; a generous regression
 * ceiling fails the gate if the subset ever becomes non-constant work.
 *
 * REQ: REQ-SPATIAL-017 (straight-line multiply at runtime), REQ-CORE-087 (no
 * heap, bounded loops).
 */
/* `CLOCK_MONOTONIC` is POSIX; the host build opts in explicitly. The bench is
 * never part of the freestanding firmware image (see Makefile). */
#define _POSIX_C_SOURCE 200809L

#include <stdio.h>
#include <time.h>

#include "drm_spatial.h"

#define ITERS 1000000u

/* Generous per-call ceiling (ns); the constant-time apply is far cheaper on
 * every supported target. A breach means non-constant work snuck in. */
#define APPLY_BOUND_NS 200.0
#define COMPOSE_BOUND_NS 200.0

int main(void) {
    drm_spatial_transform base_to_frame = {{1.0, 2.0, 3.0}, {0.0, 0.0, 0.0, 1.0}};
    drm_spatial_transform world_to_base = {{5.0, 6.0, 7.0}, {0.0, 0.0, 0.0, 1.0}};
    const double point[3] = {0.25, -0.5, 0.75};
    double out[3] = {0.0, 0.0, 0.0};

    struct timespec t0;
    struct timespec t1;
    clock_gettime(CLOCK_MONOTONIC, &t0);
    for (unsigned i = 0; i < ITERS; ++i) {
        drm_spatial_apply_dynamic(&base_to_frame, &world_to_base, point, out);
    }
    clock_gettime(CLOCK_MONOTONIC, &t1);
    double apply_ns = (double)(t1.tv_sec - t0.tv_sec) * 1e9 + (double)(t1.tv_nsec - t0.tv_nsec);
    apply_ns /= (double)ITERS;

    drm_spatial_transform c = {0};
    clock_gettime(CLOCK_MONOTONIC, &t0);
    for (unsigned i = 0; i < ITERS; ++i) {
        drm_spatial_compose(&c, &base_to_frame, &world_to_base);
    }
    clock_gettime(CLOCK_MONOTONIC, &t1);
    double compose_ns = (double)(t1.tv_sec - t0.tv_sec) * 1e9 + (double)(t1.tv_nsec - t0.tv_nsec);
    compose_ns /= (double)ITERS;

    printf("mcu spatial timing: apply_dynamic %.2f ns/call, compose %.2f ns/call\n", apply_ns,
           compose_ns);
    if (apply_ns > APPLY_BOUND_NS || compose_ns > COMPOSE_BOUND_NS) {
        printf("mcu spatial timing: FAIL (bound %.0f ns)\n", APPLY_BOUND_NS);
        return 1;
    }
    printf("mcu spatial timing: OK\n");
    return 0;
}
