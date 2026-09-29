/*
 * DRM C ABI — handwritten, single source of truth (ADR 0002).
 * REQ: REQ-BIND-007 (one status taxonomy mirrored 1:1 to C codes),
 * REQ-BIND-017 (fixed int32_t, explicit values, unknown codes safe),
 * REQ-BIND-004 (opaque handles; no raw cross-process pointers),
 * REQ-BIND-008 (access validation once at endpoint open).
 */
#ifndef DRM_DRM_H
#define DRM_DRM_H

#include <stddef.h>
#include <stdint.h>

#ifdef __cplusplus
extern "C" {
#endif

/* ABI version: bump on layout change only; additions do not bump. */
#define DRM_ABI_VERSION 0

typedef struct drm_context drm_context;     /* opaque deployment context */
typedef struct drm_publisher drm_publisher; /* opaque publisher endpoint */
typedef struct drm_consumer drm_consumer;   /* opaque consumer endpoint */

/* Status: fixed int32_t — no C enum, so -fshort-enums cannot alter the ABI.
 * Append-only: explicit values forever; never renumbered, never reused.
 * No DRM_ERR_INTERNAL — fail-stop; OS/environment failures map to precise codes. */
enum {
    DRM_OK = 0,
    DRM_ERR_INVALID_ARG = -1,
    DRM_ERR_NOT_FOUND = -2,
    DRM_ERR_ALREADY_EXISTS = -3,
    DRM_ERR_PERMISSION = -4,
    DRM_ERR_QUEUE_FULL = -5,
    DRM_ERR_GAP = -6,
    DRM_ERR_STALE = -7,
    DRM_ERR_WRONG_ABI = -8,
    DRM_ERR_POOL_EXHAUSTED = -9,
    DRM_ERR_RETRY = -10,
    DRM_ERR_BUSY = -11,
    DRM_ERR_NO_SAMPLE = -12,
    DRM_ERR_INVALID_BORROW = -13,
    DRM_ERR_GENERATION_EXHAUSTED = -14,
    DRM_ERR_INVALID_DATA = -15,
    DRM_ERR_INVALID_CONFIG = -16,
    DRM_ERR_NOT_READY = -17,
    DRM_ERR_LOCK_FAILED = -18,
    DRM_ERR_SEQUENCE_EXHAUSTED = -19,
    DRM_ERR_EPOCH_EXHAUSTED = -20,
    DRM_ERR_RING_STAMP_EXHAUSTED = -21
};

typedef int32_t drm_status; /* DRM_OK on success, one DRM_ERR_* otherwise */

/* Caller-owned borrow view; valid until drm_borrow_release(token). */
typedef struct drm_borrow_view {
    const void *data; /* points into the consumer's LOCAL mapping */
    size_t len;       /* committed canonical sample length (canon-codec) */
    uint64_t sequence;
    uint32_t token; /* opaque; 0 = invalid */
} drm_borrow_view;

/* Publisher loan view; valid until drm_write_commit / drm_write_abort. */
typedef struct drm_write_view {
    void *data;      /* slot start in the publisher's LOCAL mapping */
    size_t capacity; /* fixed topic size, else acquired request size */
    uint64_t sequence;
    uint32_t token; /* opaque; 0 = invalid */
} drm_write_view;

/* ABI probe (W1 skeleton export; vocabulary grows with drm-ffi, W2+). */
int32_t drm_abi_version(void);

#ifdef __cplusplus
}
#endif

#endif /* DRM_DRM_H */
