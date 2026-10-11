/*
 * MCU serial subset — bounded COBS/CRC32C framing parity with the host
 * (docs/algorithms/router/serial-mcu.md; work-plan M15W2-E3-T1/T2).
 *
 * Freestanding C11 (codebase.md 3.3): fixed-width types only, no heap, no
 * recursion, statically bounded loops. Every buffer is caller-owned or a fixed
 * `static`-sized struct field, so the publish/read path allocates nothing
 * (I-SR5). The covered-byte sequence and COBS block format match
 * `drm-router::transport::serial` byte-for-byte (I-SR3, I-SR6).
 *
 * REQ: REQ-CORE-087 (MCU data path: no heap, no recursion, bounded loops),
 * REQ-OPS-004 (serial framing interoperates byte-for-byte with the host),
 * REQ-ROUTE-025 (one sample = one bounded frame), REQ-ROUTE-026 (CRC).
 */
#ifndef DRM_MCU_DRM_SERIAL_H
#define DRM_MCU_DRM_SERIAL_H

#include <stddef.h>
#include <stdint.h>

#include "drm/drm.h" /* drm_status, DRM_ERR_* */

#ifdef __cplusplus
extern "C" {
#endif

/* Fixed wire header size and version (router-core I-RT7). */
#define DRM_WIRE_HEADER_LEN 28u
#define DRM_WIRE_VERSION 1u
/* Flag bit 0: end-to-end CRC32C trailer present. */
#define DRM_WIRE_FLAG_E2E_CRC 0x01u
/* Mandatory serial CRC32C trailer and optional end-to-end trailer. */
#define DRM_SERIAL_CRC_LEN 4u
#define DRM_SERIAL_E2E_LEN 4u
/* Single resync byte terminating every COBS frame (I-SR4). */
#define DRM_SERIAL_DELIMITER 0x00u

/* Static decoder bounds (bytes): decoded frame (header + payload + e2e + CRC),
 * its COBS encoding, and the encoded body plus delimiter. */
#define DRM_SERIAL_MAX_DECODED 260u
#define DRM_SERIAL_MAX_CODED (DRM_SERIAL_MAX_DECODED + (DRM_SERIAL_MAX_DECODED / 254u) + 2u)
#define DRM_SERIAL_MAX_WIRE (DRM_SERIAL_MAX_CODED + 1u)

/* Fixed little-endian wire header (router-core I-RT7). */
typedef struct drm_wire_header {
    uint8_t flags;
    uint32_t wire_channel_id;
    uint64_t stream_epoch;
    uint64_t source_seq;
    uint32_t payload_len;
} drm_wire_header;

/* Per-link framing configuration; `max_frame_bytes` is the decoded bound
 * (header + payload + optional e2e), excluding the serial CRC and COBS. */
typedef struct drm_serial_config {
    uint32_t baud;
    uint16_t max_frame_bytes;
} drm_serial_config;

/* One decoded frame produced by the streaming decoder. */
typedef struct drm_serial_frame {
    drm_wire_header header;
    size_t payload_len;
    uint32_t e2e_crc;
    int has_e2e;
    int valid; /* 1 when this call produced a frame */
} drm_serial_frame;

/* Streaming decoder state: fixed storage, O(1) per byte, no allocation. */
typedef struct drm_serial_decoder {
    uint8_t body[DRM_SERIAL_MAX_WIRE];
    size_t body_len;
    uint8_t decoded[DRM_SERIAL_MAX_CODED];
    uint32_t dropped;
} drm_serial_decoder;

uint32_t drm_crc32c(const uint8_t *data, size_t len);
void drm_wire_header_encode(const drm_wire_header *header, uint8_t out[DRM_WIRE_HEADER_LEN]);
drm_status drm_wire_header_decode(const uint8_t *buf, size_t len, drm_wire_header *out);
size_t drm_cobs_max_encoded_len(size_t input_len);
drm_status drm_cobs_encode(const uint8_t *input, size_t in_len, uint8_t *out, size_t out_cap,
                           size_t *out_len);
drm_status drm_cobs_decode(const uint8_t *input, size_t in_len, uint8_t *out, size_t out_cap,
                           size_t *out_len);
drm_status drm_serial_encode(const drm_serial_config *cfg, const drm_wire_header *header,
                             const uint8_t *payload, size_t payload_len, const uint32_t *e2e_crc,
                             uint8_t *out, size_t out_cap, size_t *out_len);
void drm_serial_decoder_init(drm_serial_decoder *dec);
drm_status drm_serial_decoder_feed(drm_serial_decoder *dec, const drm_serial_config *cfg,
                                   uint8_t byte, uint8_t *payload_out, size_t payload_cap,
                                   drm_serial_frame *out);

#ifdef __cplusplus
}
#endif

#endif /* DRM_MCU_DRM_SERIAL_H */
