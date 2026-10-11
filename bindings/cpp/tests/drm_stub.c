/*
 * TEST-ONLY C stub of the drm C ABI (bindings/c/include/drm/drm.h).
 *
 * NOT part of the product and NOT linked by any shipping target. It exists
 * solely so the C++ RAII driver (bindings/cpp/tests/borrow_raii.cpp) can be
 * exercised end-to-end without linking the real drm-ffi cdylib. It mirrors the
 * documented ABI semantics the driver relies on:
 *   - at most one outstanding borrow per consumer (second borrow => Busy);
 *   - explicit release; a double release / token mismatch => InvalidArg;
 *   - empty channel borrow => NoSample;
 *   - explicit close; consumer close with an outstanding borrow => Busy.
 * It also exposes a test-only release counter so the driver can assert a
 * borrow was released exactly once.
 *
 * REQ: REQ-BIND-006 (single borrow), REQ-BIND-001 (explicit release),
 *      REQ-BIND-021 (context lifecycle), REQ-BIND-002 (explicit close).
 */
#include <drm/drm.h>

#include <stdint.h>
#include <stdlib.h>
#include <string.h>

#define STUB_SAMPLE_MAX 64u
#define STUB_TOPIC_ID 0u

struct drm_context {
    uint8_t sample[STUB_SAMPLE_MAX];
    size_t len;
    int has_sample;
};

struct drm_publisher {
    struct drm_context *ctx;
};

struct drm_consumer {
    struct drm_context *ctx;
    uint32_t outstanding; /* 0 = none */
    uint32_t next_token;
};

static uint32_t g_release_count = 0;

/* Test-only hooks (forward-declared so strict C builds stay warning-free). */
uint32_t drm_test_release_count(void);
void drm_test_release_reset(void);

int32_t drm_abi_version(void) { return DRM_ABI_VERSION; }

drm_status drm_context_open(const char *path, drm_context **out) {
    (void)path; /* ignored by the ABI in this phase */
    if (out == NULL) {
        return DRM_ERR_INVALID_ARG;
    }
    *out = NULL;
    struct drm_context *ctx = (struct drm_context *)calloc(1, sizeof *ctx);
    if (ctx == NULL) {
        return DRM_ERR_POOL_EXHAUSTED;
    }
    *out = ctx;
    return DRM_OK;
}

drm_status drm_context_close(drm_context *ctx) {
    if (ctx == NULL) {
        return DRM_ERR_INVALID_ARG;
    }
    free(ctx);
    return DRM_OK;
}

drm_status drm_publisher_open(drm_context *ctx, uint32_t topic_id, drm_publisher **out) {
    if (out == NULL) {
        return DRM_ERR_INVALID_ARG;
    }
    *out = NULL;
    if (ctx == NULL) {
        return DRM_ERR_INVALID_ARG;
    }
    if (topic_id != STUB_TOPIC_ID) {
        return DRM_ERR_PERMISSION; /* undeclared topic */
    }
    struct drm_publisher *p = (struct drm_publisher *)calloc(1, sizeof *p);
    if (p == NULL) {
        return DRM_ERR_POOL_EXHAUSTED;
    }
    p->ctx = ctx;
    *out = p;
    return DRM_OK;
}

drm_status drm_publisher_close(drm_publisher *publisher) {
    if (publisher == NULL) {
        return DRM_ERR_INVALID_ARG;
    }
    free(publisher);
    return DRM_OK;
}

drm_status drm_publish_copy(drm_publisher *publisher, const uint8_t *data, size_t len) {
    if (publisher == NULL || (data == NULL && len != 0)) {
        return DRM_ERR_INVALID_ARG;
    }
    if (len > STUB_SAMPLE_MAX) {
        return DRM_ERR_INVALID_ARG;
    }
    struct drm_context *ctx = publisher->ctx;
    if (ctx == NULL) {
        return DRM_ERR_INVALID_ARG;
    }
    memset(ctx->sample, 0, sizeof ctx->sample);
    if (len != 0) {
        memcpy(ctx->sample, data, len);
    }
    ctx->len = len;
    ctx->has_sample = 1;
    return DRM_OK;
}

drm_status drm_consumer_open(drm_context *ctx, uint32_t topic_id, drm_consumer **out) {
    if (out == NULL) {
        return DRM_ERR_INVALID_ARG;
    }
    *out = NULL;
    if (ctx == NULL) {
        return DRM_ERR_INVALID_ARG;
    }
    if (topic_id != STUB_TOPIC_ID) {
        return DRM_ERR_PERMISSION;
    }
    struct drm_consumer *c = (struct drm_consumer *)calloc(1, sizeof *c);
    if (c == NULL) {
        return DRM_ERR_POOL_EXHAUSTED;
    }
    c->ctx = ctx;
    c->outstanding = 0;
    c->next_token = 1;
    *out = c;
    return DRM_OK;
}

drm_status drm_consumer_close(drm_consumer *consumer) {
    if (consumer == NULL) {
        return DRM_ERR_INVALID_ARG;
    }
    if (consumer->outstanding != 0) {
        return DRM_ERR_BUSY; /* outstanding borrow: refuse (I-CR4) */
    }
    free(consumer);
    return DRM_OK;
}

drm_status drm_borrow_latest(drm_consumer *consumer, drm_borrow_view *out) {
    if (out == NULL) {
        return DRM_ERR_INVALID_ARG;
    }
    memset(out, 0, sizeof *out);
    if (consumer == NULL) {
        return DRM_ERR_INVALID_ARG;
    }
    if (consumer->outstanding != 0) {
        return DRM_ERR_BUSY; /* one outstanding borrow per consumer */
    }
    struct drm_context *ctx = consumer->ctx;
    if (ctx == NULL || !ctx->has_sample) {
        return DRM_ERR_NO_SAMPLE;
    }
    out->data = ctx->sample;
    out->len = ctx->len;
    out->sequence = 1;
    out->token = consumer->next_token++;
    consumer->outstanding = out->token;
    return DRM_OK;
}

drm_status drm_borrow_release(drm_consumer *consumer, uint32_t token) {
    if (consumer == NULL || token == 0) {
        return DRM_ERR_INVALID_ARG;
    }
    if (consumer->outstanding != token) {
        return DRM_ERR_INVALID_ARG; /* no borrow / double release / mismatch */
    }
    consumer->outstanding = 0;
    g_release_count++;
    return DRM_OK;
}

drm_status drm_read_latest(drm_consumer *consumer, uint8_t *buf, size_t cap, size_t *out_len) {
    if (out_len == NULL) {
        return DRM_ERR_INVALID_ARG;
    }
    *out_len = 0;
    if (consumer == NULL || (buf == NULL && cap != 0)) {
        return DRM_ERR_INVALID_ARG;
    }
    if (cap < STUB_SAMPLE_MAX) {
        *out_len = STUB_SAMPLE_MAX;
        return DRM_ERR_INVALID_ARG;
    }
    struct drm_context *ctx = consumer->ctx;
    if (ctx == NULL || !ctx->has_sample) {
        return DRM_ERR_NO_SAMPLE;
    }
    memcpy(buf, ctx->sample, ctx->len);
    *out_len = ctx->len;
    return DRM_OK;
}

drm_status drm_read_next(drm_consumer *consumer, uint8_t *buf, size_t cap, size_t *out_len,
                         uint64_t *out_missed) {
    if (out_missed == NULL) {
        return DRM_ERR_INVALID_ARG;
    }
    *out_missed = 0;
    return drm_read_latest(consumer, buf, cap, out_len);
}

uint32_t drm_test_release_count(void) { return g_release_count; }
void drm_test_release_reset(void) { g_release_count = 0; }
