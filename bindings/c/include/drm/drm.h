/*
 * DRM C ABI — handwritten, single source of truth (ADR 0002).
 * REQ: REQ-BIND-007 (one status taxonomy mirrored 1:1 to C codes),
 * REQ-BIND-017 (fixed int32_t, explicit values, unknown codes safe),
 * REQ-BIND-004 (opaque handles; no raw cross-process pointers),
 * REQ-BIND-021 (opaque drm_context with explicit open/close lifecycle),
 * REQ-BIND-008 (access validation once at endpoint open).
 */
#ifndef DRM_DRM_H
#define DRM_DRM_H

#include <stddef.h>
#include <stdint.h>

#ifdef __cplusplus
extern "C" {
#endif

/* ABI version: bump on layout change only; additions do not bump.
 * The status vocabulary below is complete for month 1 (work-plan M01W3-E4-T5);
 * new codes are appended with explicit values only (docs/spec/abi-policy.md). */
#define DRM_ABI_VERSION 0

typedef struct drm_context drm_context;     /* opaque deployment context */
typedef struct drm_publisher drm_publisher; /* opaque publisher endpoint */
typedef struct drm_consumer drm_consumer;   /* opaque consumer endpoint */
typedef struct drm_attached drm_attached;   /* opaque attached (SHM) endpoint */

/* Handle validity (REQ-BIND-004, ABI policy):
 * - Handles are opaque; a client never dereferences, copies semantics of, or
 *   synthesizes a handle value. Invalid (freed/foreign/NULL) handle input is a
 *   caller contract violation; only NULL is checked (non-null dangling stays
 *   caller UB and is never claimed detectable).
 * - A handle is valid from a successful open until its matching close call.
 *   Close is never concurrent with other calls on the same handle. A second
 *   close of an already-closed handle (double close) is caller UB — only NULL
 *   is checked; there is no handle-generation table and it is never claimed
 *   detectable.
 * - drm_context must be closed only after all endpoints derived from it.
 * - Views (drm_borrow_view / drm_write_view) are caller-owned structs whose
 *   data pointer validity is bounded by the matching release/commit/abort.
 */

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
    DRM_ERR_RING_STAMP_EXHAUSTED = -21,
    DRM_ERR_NOT_IMPLEMENTED = -22
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

/* ABI probe; the canonical export is drm-ffi drm_abi_version. */
int32_t drm_abi_version(void);

/* Endpoint lifecycle (opaque handles only — REQ-BIND-004/021). Signatures are
 * fixed here; the drm-ffi bodies for open/close/publish/read/borrow are backed
 * in-process by a drm-core channel (cross-process SHM endpoints are a later
 * phase; see c-abi-surface.md §Functions). An export with no live hook yet
 * validates arguments and returns DRM_ERR_NOT_IMPLEMENTED, never a fabricated
 * handle. */
drm_status drm_context_open(const char *path, drm_context **out);
drm_status drm_context_close(drm_context *ctx);
drm_status drm_publisher_open(drm_context *ctx, uint32_t topic_id, drm_publisher **out);
drm_status drm_publisher_close(drm_publisher *publisher);
drm_status drm_consumer_open(drm_context *ctx, uint32_t topic_id, drm_consumer **out);
drm_status drm_consumer_close(drm_consumer *consumer);

/* Copy publish (REQ-BIND-006): the caller owns `data` for `len` bytes; `len`
 * must not exceed the topic sample size. */
drm_status drm_publish_copy(drm_publisher *publisher, const uint8_t *data, size_t len);

/* Copy reads (REQ-BIND-009): deliver into the caller-owned `buf`;
 * `*out_len` receives the written length (or the required size on a too-small
 * buffer). `drm_read_next` additionally reports a Gap count via `*out_missed`
 * (REQ-BIND-019). */
drm_status drm_read_latest(drm_consumer *consumer, uint8_t *buf, size_t cap, size_t *out_len);
drm_status drm_read_next(drm_consumer *consumer, uint8_t *buf, size_t cap, size_t *out_len,
                         uint64_t *out_missed);

/* Zero-copy borrow (REQ-BIND-003): at most one outstanding borrow per consumer;
 * release is explicit (no RAII in C) — double-release returns
 * DRM_ERR_INVALID_ARG. */
drm_status drm_borrow_latest(drm_consumer *consumer, drm_borrow_view *out);
drm_status drm_borrow_release(drm_consumer *consumer, uint32_t token);

/* --- Attached (cross-process shared-memory) data path --------------------- *
 * Append-only addition (REQ-CORE-107, ADR 0007, attached-data-path.md). Unlike
 * the endpoint group above these DO create/attach a real POSIX shared-memory
 * segment, so a second process can exchange topic samples with the first.
 *
 * Both sides must agree on segment_name, topic_name, lock_dir, the two hashes,
 * topic_handle, sample_max, depth, pool, and overflow_policy; the slot
 * alignment is fixed at 8 for these exports. `mode` is DRM_ATTACHED_CONSUMER
 * (read-only, never creates; timeout_ms bounds the wait) or
 * DRM_ATTACHED_PUBLISHER (acquires the single-writer authority; a second live
 * publisher returns DRM_ERR_PERMISSION, I-AT2). */
#define DRM_ATTACHED_CONSUMER ((uint8_t)0)
#define DRM_ATTACHED_PUBLISHER ((uint8_t)1)

drm_status drm_attached_open(const char *segment_name, const char *topic_name, const char *lock_dir,
                             const uint8_t *schema_hash, const uint8_t *layout_hash,
                             uint32_t topic_handle, uint64_t sample_max, uint64_t depth,
                             uint64_t pool, uint8_t overflow_policy, uint8_t mode,
                             uint64_t timeout_ms, drm_attached **out);

/* Same as drm_attached_open, but the payload-slot alignment is the deployment's
 * `slot_align` instead of the fixed 8. The stride is part of the segment layout,
 * so a publisher that assumes 8 creates a segment whose size a deployment-
 * configured consumer must refuse. `slot_align` 0 means "use the fixed 8".
 * Append-only addition (REQ-CORE-107); drm_attached_open keeps its shape. */
drm_status drm_attached_open_aligned(const char *segment_name, const char *topic_name,
                                     const char *lock_dir, const uint8_t *schema_hash,
                                     const uint8_t *layout_hash, uint32_t topic_handle,
                                     uint64_t sample_max, uint64_t depth, uint64_t pool,
                                     uint8_t overflow_policy, uint8_t mode, uint64_t timeout_ms,
                                     uint64_t slot_align, drm_attached **out);
drm_status drm_attached_close(drm_attached *handle);

/* Copy publish; `out_seq` (optional) receives the sequence the sample took. */
drm_status drm_attached_publish(drm_attached *handle, const uint8_t *data, size_t len,
                                uint64_t *out_seq);

/* Copy reads over the attachment; `*out_len` receives the payload length. Both
 * return DRM_ERR_NO_SAMPLE when nothing is committed yet and DRM_ERR_RETRY when
 * the bounded attempt budget is exhausted; `drm_attached_read_next` additionally
 * returns DRM_ERR_GAP with the missed count in `*out_missed` when the reader
 * fell behind the retained window. */
drm_status drm_attached_read_latest(drm_attached *handle, uint8_t *buf, size_t cap,
                                    size_t *out_len);
drm_status drm_attached_read_next(drm_attached *handle, uint8_t *buf, size_t cap, size_t *out_len,
                                  uint64_t *out_missed);

#ifdef __cplusplus
}
#endif

#endif /* DRM_DRM_H */
