/*
 * MCU static-buffer publish/read path (work-plan M15W2-E3-T1). A minimal
 * single-producer/single-consumer topic ring with all storage statically
 * placed: no heap, no recursion, statically bounded loops (I-SR5).
 *
 * REQ: REQ-CORE-087 (MCU data path: no heap), REQ-ROUTE-025 (one sample = one
 * frame), REQ-OPS-004 (host interop).
 */
#ifndef DRM_MCU_DRM_NODE_H
#define DRM_MCU_DRM_NODE_H

#include <stddef.h>
#include <stdint.h>

#include "drm/drm.h"

#ifdef __cplusplus
extern "C" {
#endif

/* Fixed ring geometry; storage lives inside `drm_node` (no allocation). */
#define DRM_NODE_SLOTS 4u
#define DRM_NODE_SLOT_BYTES 32u

typedef struct drm_node {
    uint8_t payload[DRM_NODE_SLOTS][DRM_NODE_SLOT_BYTES];
    uint16_t len[DRM_NODE_SLOTS];
    uint32_t seq[DRM_NODE_SLOTS];
    uint32_t write_seq;
    uint32_t read_seq;
    uint32_t dropped;
} drm_node;

void drm_node_init(drm_node *node);
drm_status drm_node_publish(drm_node *node, const uint8_t *data, uint16_t len);
drm_status drm_node_read(drm_node *node, uint8_t *out, uint16_t out_cap, uint16_t *out_len,
                         uint32_t *out_seq);

#ifdef __cplusplus
}
#endif

#endif /* DRM_MCU_DRM_NODE_H */
