/*
 * ABI spike probe (work-plan M01W4-E4-T2; extended M06W1-E4-T2): exhaustively
 * exercises the status enumerator set, opaque-handle patterns, the ABI-version
 * guard, and the endpoint-lifecycle stub signatures.
 * Syntax-only: compiled by CI under gcc+clang x c99+c11 with -Werror.
 * REQ: REQ-BIND-007 (status taxonomy), REQ-BIND-017 (unknown codes safe),
 * REQ-BIND-004 (opaque handles), REQ-BIND-021 (context lifecycle).
 */
#include <drm/drm.h>

#include <stddef.h>

/* All enumerators switch: every status code is named in one place; unknown
 * future codes pass through safely (REQ-BIND-017 — never a C enum trap). */
static int32_t spike_status_class(int32_t code) {
    switch (code) {
    case DRM_OK:
        return 0;
    case DRM_ERR_INVALID_ARG:
        return 1;
    case DRM_ERR_NOT_FOUND:
        return 2;
    case DRM_ERR_ALREADY_EXISTS:
        return 3;
    case DRM_ERR_PERMISSION:
        return 4;
    case DRM_ERR_QUEUE_FULL:
        return 5;
    case DRM_ERR_GAP:
        return 6;
    case DRM_ERR_STALE:
        return 7;
    case DRM_ERR_WRONG_ABI:
        return 8;
    case DRM_ERR_POOL_EXHAUSTED:
        return 9;
    case DRM_ERR_RETRY:
        return 10;
    case DRM_ERR_BUSY:
        return 11;
    case DRM_ERR_NO_SAMPLE:
        return 12;
    case DRM_ERR_INVALID_BORROW:
        return 13;
    case DRM_ERR_GENERATION_EXHAUSTED:
        return 14;
    case DRM_ERR_INVALID_DATA:
        return 15;
    case DRM_ERR_INVALID_CONFIG:
        return 16;
    case DRM_ERR_NOT_READY:
        return 17;
    case DRM_ERR_LOCK_FAILED:
        return 18;
    case DRM_ERR_SEQUENCE_EXHAUSTED:
        return 19;
    case DRM_ERR_EPOCH_EXHAUSTED:
        return 20;
    case DRM_ERR_RING_STAMP_EXHAUSTED:
        return 21;
    case DRM_ERR_NOT_IMPLEMENTED:
        return 22;
    default:
        return -1000;
    }
}

#if DRM_ABI_VERSION >= 0
static int32_t spike_abi_guard(void) { return DRM_ABI_VERSION; }
#else
#error "DRM_ABI_VERSION must never be negative"
#endif

/* Opaque-handle pattern: NULL-only check; handles are never sized, never
 * dereferenced, never compared to anything but 0 (REQ-BIND-004). */
static int32_t spike_handle_probe(drm_context *ctx, drm_publisher *pub, drm_consumer *con) {
    if (ctx == 0 || pub == 0 || con == 0) {
        return DRM_ERR_INVALID_ARG;
    }
    return DRM_OK;
}

/* Endpoint-lifecycle stub prototypes (work-plan M06W1-E4): each re-declaration
 * must match the drm.h declaration exactly, or the compiler rejects the file
 * with a conflicting-type error. Declarations only — no call, so no drm-ffi
 * symbol is referenced and the spike still links standalone. (Declaration
 * presence itself is asserted by drm-ffi's `drm_h_declares_stub_surface` test.) */
drm_status drm_context_open(const char *path, drm_context **out);
drm_status drm_context_close(drm_context *ctx);
drm_status drm_publisher_open(drm_context *ctx, uint32_t topic_id, drm_publisher **out);
drm_status drm_publisher_close(drm_publisher *publisher);
drm_status drm_consumer_open(drm_context *ctx, uint32_t topic_id, drm_consumer **out);
drm_status drm_consumer_close(drm_consumer *consumer);
int32_t drm_abi_version(void);

int main(void) {
    /* Each probe asserted independently (fail-stop on any mismatch). */
    if (spike_status_class(DRM_OK) != 0)
        return 1;
    if (spike_status_class(DRM_ERR_RING_STAMP_EXHAUSTED) != 21)
        return 2;
    if (spike_status_class(DRM_ERR_NOT_IMPLEMENTED) != 22)
        return 6;
    if (spike_status_class(-1000) != -1000)
        return 3; /* unknown code passthrough */
    if (spike_abi_guard() != 0)
        return 4;
    if (spike_handle_probe(0, (drm_publisher *)0, (drm_consumer *)0) != DRM_ERR_INVALID_ARG)
        return 5;
    return 0;
}
