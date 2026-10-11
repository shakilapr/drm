/*
 * MCU static-buffer publish/read path implementation. Zero heap, no recursion,
 * statically bounded loops (I-SR5).
 *
 * REQ: REQ-CORE-087, REQ-ROUTE-025.
 */
#include "drm_node.h"

void drm_node_init(drm_node *node) {
    node->write_seq = 0u;
    node->read_seq = 0u;
    node->dropped = 0u;
    for (uint32_t s = 0; s < DRM_NODE_SLOTS; ++s) {
        node->len[s] = 0u;
        node->seq[s] = 0u;
        for (uint32_t b = 0; b < DRM_NODE_SLOT_BYTES; ++b) {
            node->payload[s][b] = 0u;
        }
    }
}

drm_status drm_node_publish(drm_node *node, const uint8_t *data, uint16_t len) {
    if (node == NULL || (data == NULL && len != 0u)) {
        return DRM_ERR_INVALID_ARG;
    }
    if ((size_t)len > (size_t)DRM_NODE_SLOT_BYTES) {
        return DRM_ERR_INVALID_ARG;
    }
    uint32_t slot = node->write_seq % DRM_NODE_SLOTS;
    if (node->write_seq - node->read_seq >= DRM_NODE_SLOTS) {
        node->read_seq += 1u; /* overwrite oldest; best-effort (I-RT10) */
        node->dropped += 1u;
    }
    for (uint16_t i = 0; i < len; ++i) {
        node->payload[slot][i] = data[i];
    }
    node->len[slot] = len;
    node->seq[slot] = node->write_seq;
    node->write_seq += 1u;
    return DRM_OK;
}

drm_status drm_node_read(drm_node *node, uint8_t *out, uint16_t out_cap, uint16_t *out_len,
                         uint32_t *out_seq) {
    if (node == NULL || out == NULL || out_len == NULL || out_seq == NULL) {
        return DRM_ERR_INVALID_ARG;
    }
    if (node->read_seq >= node->write_seq) {
        return DRM_ERR_NO_SAMPLE;
    }
    uint32_t slot = node->read_seq % DRM_NODE_SLOTS;
    uint16_t l = node->len[slot];
    if ((size_t)l > (size_t)out_cap) {
        return DRM_ERR_INVALID_ARG;
    }
    for (uint16_t i = 0; i < l; ++i) {
        out[i] = node->payload[slot][i];
    }
    *out_len = l;
    *out_seq = node->seq[slot];
    node->read_seq += 1u;
    return DRM_OK;
}
