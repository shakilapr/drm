/*
 * MCU serial subset implementation (docs/algorithms/router/serial-mcu.md).
 * Zero heap, no recursion, statically bounded loops (I-SR5).
 *
 * REQ: REQ-CORE-087, REQ-OPS-004, REQ-ROUTE-025, REQ-ROUTE-026.
 */
#include "drm_serial.h"

static void put_le32(uint8_t *out, uint32_t v) {
    out[0] = (uint8_t)(v & 0xFFu);
    out[1] = (uint8_t)((v >> 8) & 0xFFu);
    out[2] = (uint8_t)((v >> 16) & 0xFFu);
    out[3] = (uint8_t)((v >> 24) & 0xFFu);
}

static void put_le64(uint8_t *out, uint64_t v) {
    for (uint32_t i = 0; i < 8u; ++i) {
        out[i] = (uint8_t)((v >> (8u * i)) & 0xFFu);
    }
}

static uint32_t get_le32(const uint8_t *in) {
    return (uint32_t)in[0] | ((uint32_t)in[1] << 8) | ((uint32_t)in[2] << 16) |
           ((uint32_t)in[3] << 24);
}

static uint64_t get_le64(const uint8_t *in) {
    uint64_t v = 0;
    for (uint32_t i = 0; i < 8u; ++i) {
        v |= (uint64_t)in[i] << (8u * i);
    }
    return v;
}

/* CRC32C (Castagnoli, reflected poly 0x82F63B78, init/final 0xFFFFFFFF),
 * bitwise — no tables, no allocation (I-SR3). */
uint32_t drm_crc32c(const uint8_t *data, size_t len) {
    uint32_t crc = 0xFFFFFFFFu;
    if (data == NULL && len != 0u) {
        return 0u;
    }
    for (size_t i = 0; i < len; ++i) {
        crc ^= (uint32_t)data[i];
        for (uint32_t bit = 0; bit < 8u; ++bit) {
            uint32_t mask = (uint32_t)(-(int32_t)(crc & 1u));
            crc = (crc >> 1) ^ (0x82F63B78u & mask);
        }
    }
    return crc ^ 0xFFFFFFFFu;
}

void drm_wire_header_encode(const drm_wire_header *header, uint8_t out[DRM_WIRE_HEADER_LEN]) {
    out[0] = DRM_WIRE_VERSION;
    out[1] = header->flags;
    out[2] = 0u;
    out[3] = 0u;
    put_le32(&out[4], header->wire_channel_id);
    put_le64(&out[8], header->stream_epoch);
    put_le64(&out[16], header->source_seq);
    put_le32(&out[24], header->payload_len);
}

drm_status drm_wire_header_decode(const uint8_t *buf, size_t len, drm_wire_header *out) {
    if (buf == NULL || out == NULL) {
        return DRM_ERR_INVALID_ARG;
    }
    if (len < (size_t)DRM_WIRE_HEADER_LEN) {
        return DRM_ERR_INVALID_ARG;
    }
    if (buf[0] != DRM_WIRE_VERSION) {
        return DRM_ERR_INVALID_DATA;
    }
    if ((buf[1] & (uint8_t)~DRM_WIRE_FLAG_E2E_CRC) != 0u) {
        return DRM_ERR_INVALID_DATA;
    }
    if (buf[2] != 0u || buf[3] != 0u) {
        return DRM_ERR_INVALID_DATA;
    }
    out->flags = buf[1];
    out->wire_channel_id = get_le32(&buf[4]);
    out->stream_epoch = get_le64(&buf[8]);
    out->source_seq = get_le64(&buf[16]);
    out->payload_len = get_le32(&buf[24]);
    return DRM_OK;
}

size_t drm_cobs_max_encoded_len(size_t input_len) {
    return input_len + (input_len + 253u) / 254u + 1u;
}

drm_status drm_cobs_encode(const uint8_t *input, size_t in_len, uint8_t *out, size_t out_cap,
                           size_t *out_len) {
    if (out == NULL || out_len == NULL || (input == NULL && in_len != 0u)) {
        return DRM_ERR_INVALID_ARG;
    }
    if (out_cap < drm_cobs_max_encoded_len(in_len)) {
        return DRM_ERR_INVALID_ARG;
    }
    size_t wr = 0;
    size_t code_idx = 0;
    uint8_t code = 1u;
    out[wr] = 1u; /* placeholder block code */
    wr += 1;
    for (size_t i = 0; i < in_len; ++i) {
        uint8_t b = input[i];
        if (b == 0u) {
            out[code_idx] = code;
            code = 1u;
            code_idx = wr;
            out[wr] = 1u;
            wr += 1;
        } else {
            out[wr] = b;
            wr += 1;
            code = (uint8_t)(code + 1u);
            if (code == 0xFFu) {
                out[code_idx] = code;
                code = 1u;
                code_idx = wr;
                out[wr] = 1u;
                wr += 1;
            }
        }
    }
    out[code_idx] = code;
    *out_len = wr;
    return DRM_OK;
}

drm_status drm_cobs_decode(const uint8_t *input, size_t in_len, uint8_t *out, size_t out_cap,
                           size_t *out_len) {
    if (input == NULL || out == NULL || out_len == NULL) {
        return DRM_ERR_INVALID_ARG;
    }
    size_t rd = 0;
    size_t wr = 0;
    while (rd < in_len) {
        uint8_t code = input[rd];
        rd += 1;
        if (code == 0u) {
            return DRM_ERR_INVALID_DATA;
        }
        size_t data = (size_t)code - 1u;
        if (rd + data > in_len) {
            return DRM_ERR_INVALID_DATA;
        }
        if (wr + data > out_cap) {
            return DRM_ERR_INVALID_ARG;
        }
        for (size_t k = 0; k < data; ++k) {
            out[wr] = input[rd];
            wr += 1;
            rd += 1;
        }
        if (code != 0xFFu && rd < in_len) {
            if (wr >= out_cap) {
                return DRM_ERR_INVALID_ARG;
            }
            out[wr] = 0u;
            wr += 1;
        }
    }
    *out_len = wr;
    return DRM_OK;
}

drm_status drm_serial_encode(const drm_serial_config *cfg, const drm_wire_header *header,
                             const uint8_t *payload, size_t payload_len, const uint32_t *e2e_crc,
                             uint8_t *out, size_t out_cap, size_t *out_len) {
    if (cfg == NULL || header == NULL || out == NULL || out_len == NULL) {
        return DRM_ERR_INVALID_ARG;
    }
    if (payload == NULL && payload_len != 0u) {
        return DRM_ERR_INVALID_ARG;
    }
    if ((header->flags & (uint8_t)~DRM_WIRE_FLAG_E2E_CRC) != 0u) {
        return DRM_ERR_INVALID_ARG;
    }
    int want_e2e = (header->flags & DRM_WIRE_FLAG_E2E_CRC) != 0u;
    if (want_e2e != (e2e_crc != NULL)) {
        return DRM_ERR_INVALID_ARG;
    }
    if ((size_t)header->payload_len != payload_len) {
        return DRM_ERR_INVALID_ARG;
    }
    size_t decoded =
        (size_t)DRM_WIRE_HEADER_LEN + payload_len + (want_e2e ? (size_t)DRM_SERIAL_E2E_LEN : 0u);
    if (decoded > (size_t)cfg->max_frame_bytes) {
        return DRM_ERR_INVALID_ARG; /* I-SR2 */
    }
    size_t coded = decoded + (size_t)DRM_SERIAL_CRC_LEN;
    if (coded > (size_t)DRM_SERIAL_MAX_DECODED) {
        return DRM_ERR_INVALID_ARG;
    }
    size_t need = drm_cobs_max_encoded_len(coded) + 1u;
    if (out_cap < need) {
        return DRM_ERR_INVALID_ARG; /* before any write */
    }

    uint8_t covered[DRM_SERIAL_MAX_DECODED];
    size_t clen = 0;
    uint8_t hdr[DRM_WIRE_HEADER_LEN];
    drm_wire_header_encode(header, hdr);
    for (size_t i = 0; i < (size_t)DRM_WIRE_HEADER_LEN; ++i) {
        covered[clen] = hdr[i];
        clen += 1;
    }
    for (size_t i = 0; i < payload_len; ++i) {
        covered[clen] = payload[i];
        clen += 1;
    }
    if (want_e2e) {
        uint8_t e2e[DRM_SERIAL_E2E_LEN];
        put_le32(e2e, *e2e_crc);
        for (size_t i = 0; i < (size_t)DRM_SERIAL_E2E_LEN; ++i) {
            covered[clen] = e2e[i];
            clen += 1;
        }
    }
    uint32_t crc = drm_crc32c(covered, clen);
    uint8_t crc_bytes[DRM_SERIAL_CRC_LEN];
    put_le32(crc_bytes, crc);
    for (size_t i = 0; i < (size_t)DRM_SERIAL_CRC_LEN; ++i) {
        covered[clen] = crc_bytes[i];
        clen += 1;
    }

    size_t body = 0;
    drm_status st = drm_cobs_encode(covered, clen, out, out_cap, &body);
    if (st != DRM_OK) {
        return st;
    }
    out[body] = DRM_SERIAL_DELIMITER;
    *out_len = body + 1u;
    return DRM_OK;
}

void drm_serial_decoder_init(drm_serial_decoder *dec) {
    dec->body_len = 0u;
    dec->dropped = 0u;
    for (size_t i = 0; i < (size_t)DRM_SERIAL_MAX_WIRE; ++i) {
        dec->body[i] = 0u;
    }
    for (size_t i = 0; i < (size_t)DRM_SERIAL_MAX_CODED; ++i) {
        dec->decoded[i] = 0u;
    }
}

static drm_status decode_frame(const drm_serial_config *cfg, const uint8_t *body, size_t body_len,
                               uint8_t *decoded, size_t decoded_cap, uint8_t *payload_out,
                               size_t payload_cap, drm_serial_frame *out) {
    size_t decoded_len = 0;
    drm_status st = drm_cobs_decode(body, body_len, decoded, decoded_cap, &decoded_len);
    if (st != DRM_OK) {
        return st;
    }
    if (decoded_len < (size_t)DRM_WIRE_HEADER_LEN + (size_t)DRM_SERIAL_CRC_LEN) {
        return DRM_ERR_INVALID_DATA;
    }
    drm_wire_header header;
    st = drm_wire_header_decode(decoded, decoded_len, &header);
    if (st != DRM_OK) {
        return st;
    }
    int want_e2e = (header.flags & DRM_WIRE_FLAG_E2E_CRC) != 0u;
    size_t frame_len = (size_t)DRM_WIRE_HEADER_LEN + (size_t)header.payload_len +
                       (want_e2e ? (size_t)DRM_SERIAL_E2E_LEN : 0u);
    if (frame_len > (size_t)cfg->max_frame_bytes) {
        return DRM_ERR_INVALID_DATA; /* I-SR2: foreign oversize */
    }
    size_t coded = frame_len + (size_t)DRM_SERIAL_CRC_LEN;
    if (decoded_len != coded) {
        return DRM_ERR_INVALID_DATA; /* exact decoded length */
    }
    uint32_t stored = get_le32(&decoded[coded - (size_t)DRM_SERIAL_CRC_LEN]);
    if (drm_crc32c(decoded, coded - (size_t)DRM_SERIAL_CRC_LEN) != stored) {
        return DRM_ERR_INVALID_DATA; /* I-SR3 */
    }
    if (payload_cap < (size_t)header.payload_len) {
        return DRM_ERR_INVALID_ARG;
    }
    for (size_t i = 0; i < (size_t)header.payload_len; ++i) {
        payload_out[i] = decoded[(size_t)DRM_WIRE_HEADER_LEN + i];
    }
    out->header = header;
    out->payload_len = (size_t)header.payload_len;
    out->has_e2e = want_e2e;
    out->e2e_crc = 0u;
    if (want_e2e) {
        out->e2e_crc = get_le32(&decoded[(size_t)DRM_WIRE_HEADER_LEN + (size_t)header.payload_len]);
    }
    out->valid = 1;
    return DRM_OK;
}

drm_status drm_serial_decoder_feed(drm_serial_decoder *dec, const drm_serial_config *cfg,
                                   uint8_t byte, uint8_t *payload_out, size_t payload_cap,
                                   drm_serial_frame *out) {
    if (dec == NULL || cfg == NULL || out == NULL) {
        return DRM_ERR_INVALID_ARG;
    }
    out->valid = 0;
    if (byte != DRM_SERIAL_DELIMITER) {
        if (dec->body_len >= (size_t)DRM_SERIAL_MAX_WIRE) {
            dec->body_len = 0u;
            dec->dropped += 1u;
            return DRM_OK; /* I-SR2: foreign oversize, resync */
        }
        dec->body[dec->body_len] = byte;
        dec->body_len += 1u;
        return DRM_OK;
    }
    size_t body_len = dec->body_len;
    dec->body_len = 0u;
    if (body_len == 0u) {
        return DRM_OK; /* idle delimiter */
    }
    if (payload_out == NULL) {
        return DRM_ERR_INVALID_ARG;
    }
    drm_status st = decode_frame(cfg, dec->body, body_len, dec->decoded,
                                 (size_t)DRM_SERIAL_MAX_CODED, payload_out, payload_cap, out);
    if (st != DRM_OK) {
        out->valid = 0;
        dec->dropped += 1u;
        return DRM_OK; /* drop + resync at the next delimiter (I-SR3) */
    }
    return DRM_OK;
}
