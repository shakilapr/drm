/*
 * MCU serial subset host self-test (work-plan M15W2-E3-T2; serial-mcu.md
 * §Test Matrix `serial_cobs_boundary`, `serial_corruption`, `mcu_host_interop`).
 *
 * Compiled with the HOST C compiler (not the Arm toolchain) so the freestanding
 * subset's COBS/CRC32C/framing byte-for-byte parity with
 * `drm-router::transport::serial` is verified against the same golden vectors
 * the Rust suite pins. Excluded from the Arm firmware build.
 *
 * REQ: REQ-OPS-004 (host interop byte-for-byte), REQ-ROUTE-025 (bounded frame),
 * REQ-ROUTE-026 (CRC), REQ-CORE-087 (no heap on the data path).
 */
#include <stdio.h>

#include "drm_node.h"
#include "drm_serial.h"

static int failures = 0;

#define CHECK(cond)                                                                                \
    do {                                                                                           \
        if (!(cond)) {                                                                             \
            printf("FAIL %s:%d: %s\n", __FILE__, __LINE__, #cond);                                 \
            failures += 1;                                                                         \
        }                                                                                          \
    } while (0)

static void test_crc32c(void) {
    const uint8_t digits[] = "123456789";
    CHECK(drm_crc32c(digits, 9u) == 0xE3069283u);
    CHECK(drm_crc32c(digits, 0u) == 0x00000000u);
    const uint8_t a[] = "a";
    CHECK(drm_crc32c(a, 1u) == 0xC1D04330u);
}

static void test_cobs_golden(void) {
    uint8_t out[8];
    size_t n = 0;

    CHECK(drm_cobs_encode(NULL, 0u, out, sizeof out, &n) == DRM_OK);
    CHECK(n == 1u && out[0] == 0x01u);

    const uint8_t zero[] = {0x00};
    CHECK(drm_cobs_encode(zero, 1u, out, sizeof out, &n) == DRM_OK);
    CHECK(n == 2u && out[0] == 0x01u && out[1] == 0x01u);

    const uint8_t seq[] = {1u, 2u, 3u};
    CHECK(drm_cobs_encode(seq, 3u, out, sizeof out, &n) == DRM_OK);
    CHECK(n == 4u && out[0] == 0x04u && out[1] == 1u && out[2] == 2u && out[3] == 3u);

    const uint8_t mixed[] = {0x11u, 0x00u, 0x22u};
    CHECK(drm_cobs_encode(mixed, 3u, out, sizeof out, &n) == DRM_OK);
    CHECK(n == 4u && out[0] == 0x02u && out[1] == 0x11u && out[2] == 0x02u && out[3] == 0x22u);

    /* Encode -> decode byte-exact, including embedded zeros. */
    uint8_t decoded[8];
    size_t dn = 0;
    CHECK(drm_cobs_decode(out, n, decoded, sizeof decoded, &dn) == DRM_OK);
    CHECK(dn == 3u && decoded[0] == 0x11u && decoded[1] == 0x00u && decoded[2] == 0x22u);
}

static void test_frame_roundtrip(void) {
    drm_serial_config cfg = {115200u, 64u};
    drm_wire_header header = {0u, 0xABCD1234u, 5u, 6u, 3u};
    const uint8_t payload[] = {0xDEu, 0x00u, 0xADu};
    uint8_t wire[DRM_SERIAL_MAX_WIRE];
    size_t wire_len = 0;
    CHECK(drm_serial_encode(&cfg, &header, payload, 3u, NULL, wire, sizeof wire, &wire_len) ==
          DRM_OK);
    CHECK(wire_len >= 2u && wire[wire_len - 1u] == DRM_SERIAL_DELIMITER);

    drm_serial_decoder dec;
    drm_serial_decoder_init(&dec);
    uint8_t out_payload[DRM_NODE_SLOT_BYTES];
    drm_serial_frame frame;
    int produced = 0;
    for (size_t i = 0; i < wire_len; ++i) {
        CHECK(drm_serial_decoder_feed(&dec, &cfg, wire[i], out_payload, sizeof out_payload,
                                      &frame) == DRM_OK);
        if (frame.valid) {
            produced = 1;
            CHECK(frame.header.wire_channel_id == header.wire_channel_id);
            CHECK(frame.header.source_seq == header.source_seq);
            CHECK(frame.payload_len == 3u);
            CHECK(out_payload[0] == 0xDEu && out_payload[1] == 0x00u && out_payload[2] == 0xADu);
            CHECK(frame.has_e2e == 0);
        }
    }
    CHECK(produced == 1);
    CHECK(dec.dropped == 0u);

    /* e2e variant: trailer covered by the serial CRC. */
    drm_wire_header e2e_header = {DRM_WIRE_FLAG_E2E_CRC, 7u, 1u, 2u, 3u};
    uint32_t e2e = 0xDEADBEEFu;
    CHECK(drm_serial_encode(&cfg, &e2e_header, payload, 3u, &e2e, wire, sizeof wire, &wire_len) ==
          DRM_OK);
    drm_serial_decoder_init(&dec);
    produced = 0;
    for (size_t i = 0; i < wire_len; ++i) {
        CHECK(drm_serial_decoder_feed(&dec, &cfg, wire[i], out_payload, sizeof out_payload,
                                      &frame) == DRM_OK);
        if (frame.valid) {
            produced = 1;
            CHECK(frame.has_e2e == 1);
            CHECK(frame.e2e_crc == e2e);
        }
    }
    CHECK(produced == 1);
}

static void test_corruption_resync(void) {
    drm_serial_config cfg = {115200u, 64u};
    drm_wire_header header = {0u, 1u, 1u, 1u, 2u};
    const uint8_t payload[] = {0x11u, 0x22u};
    uint8_t wire[DRM_SERIAL_MAX_WIRE];
    size_t wire_len = 0;
    CHECK(drm_serial_encode(&cfg, &header, payload, 2u, NULL, wire, sizeof wire, &wire_len) ==
          DRM_OK);

    /* Rebuild a clean body, flip a covered byte, keep the stale serial CRC:
     * the decoder must drop it and resync, producing no frame (I-SR3). */
    uint8_t covered[DRM_SERIAL_MAX_DECODED];
    size_t covered_len = 0;
    CHECK(drm_cobs_decode(wire, wire_len - 1u, covered, sizeof covered, &covered_len) == DRM_OK);
    covered[0] ^= 0x01u; /* header version byte: decode + CRC both reject */
    uint8_t corrupt[DRM_SERIAL_MAX_WIRE];
    size_t corrupt_len = 0;
    CHECK(drm_cobs_encode(covered, covered_len, corrupt, sizeof corrupt, &corrupt_len) == DRM_OK);
    corrupt[corrupt_len] = DRM_SERIAL_DELIMITER;
    corrupt_len += 1u;

    drm_serial_decoder dec;
    drm_serial_decoder_init(&dec);
    uint8_t out_payload[DRM_NODE_SLOT_BYTES];
    drm_serial_frame frame;
    int produced = 0;
    for (size_t i = 0; i < corrupt_len; ++i) {
        CHECK(drm_serial_decoder_feed(&dec, &cfg, corrupt[i], out_payload, sizeof out_payload,
                                      &frame) == DRM_OK);
        if (frame.valid) {
            produced = 1;
        }
    }
    CHECK(produced == 0);
    CHECK(dec.dropped == 1u);
}

/*
 * Host<->MCU serial interop matrix (serial-mcu.md §Test Matrix
 * `mcu_host_interop`; work-plan M16W3-E3-T1): every cell's MCU-encoded wire
 * decodes back byte-for-byte for both e2e variants, and a stale-CRC covered
 * byte is dropped + resynced (I-SR1/I-SR3/I-SR4/I-SR6).
 */
static void test_interop_matrix(void) {
    drm_serial_config cfg = {115200u, 260u};
    const size_t lens[] = {0u, 1u, 3u, 127u, 200u, 224u};
    for (size_t li = 0; li < sizeof lens / sizeof lens[0]; ++li) {
        size_t plen = lens[li];
        for (int e2e_on = 0; e2e_on <= 1; ++e2e_on) {
            uint8_t payload[224];
            for (size_t i = 0; i < plen; ++i) {
                payload[i] = (i % 3u == 0u) ? 0x00u : (uint8_t)((i * 7u + 1u) & 0xFFu);
            }
            drm_wire_header header = {e2e_on ? DRM_WIRE_FLAG_E2E_CRC : 0u, 0x0BADF00Du,
                                      0x1122334455667788ull, (uint64_t)plen, (uint32_t)plen};
            uint32_t e2e = 0x0A5A5A5Au;
            uint8_t wire[DRM_SERIAL_MAX_WIRE];
            size_t wire_len = 0;
            CHECK(drm_serial_encode(&cfg, &header, payload, plen, e2e_on ? &e2e : NULL, wire,
                                    sizeof wire, &wire_len) == DRM_OK);

            drm_serial_decoder dec;
            drm_serial_decoder_init(&dec);
            uint8_t out_payload[DRM_SERIAL_MAX_DECODED];
            drm_serial_frame frame;
            int produced = 0;
            for (size_t i = 0; i < wire_len; ++i) {
                CHECK(drm_serial_decoder_feed(&dec, &cfg, wire[i], out_payload, sizeof out_payload,
                                              &frame) == DRM_OK);
                if (frame.valid) {
                    produced = 1;
                    CHECK(frame.header.payload_len == (uint32_t)plen);
                    CHECK(frame.payload_len == plen);
                    CHECK(frame.has_e2e == e2e_on);
                    if (e2e_on) {
                        CHECK(frame.e2e_crc == e2e);
                    }
                    for (size_t k = 0; k < plen; ++k) {
                        CHECK(out_payload[k] == payload[k]);
                    }
                }
            }
            CHECK(produced == 1);
            CHECK(dec.dropped == 0u);

            /* Error path: flip a covered byte, keep the stale serial CRC. */
            uint8_t covered[DRM_SERIAL_MAX_DECODED];
            size_t covered_len = 0;
            CHECK(drm_cobs_decode(wire, wire_len - 1u, covered, sizeof covered, &covered_len) ==
                  DRM_OK);
            covered[0] ^= 0x01u;
            uint8_t corrupt[DRM_SERIAL_MAX_WIRE];
            size_t corrupt_len = 0;
            CHECK(drm_cobs_encode(covered, covered_len, corrupt, sizeof corrupt, &corrupt_len) ==
                  DRM_OK);
            corrupt[corrupt_len] = DRM_SERIAL_DELIMITER;
            corrupt_len += 1u;
            drm_serial_decoder_init(&dec);
            produced = 0;
            for (size_t i = 0; i < corrupt_len; ++i) {
                CHECK(drm_serial_decoder_feed(&dec, &cfg, corrupt[i], out_payload,
                                              sizeof out_payload, &frame) == DRM_OK);
                if (frame.valid) {
                    produced = 1;
                }
            }
            CHECK(produced == 0);
            CHECK(dec.dropped == 1u);
        }
    }
}

static void test_static_node(void) {
    drm_node node;
    drm_node_init(&node);
    const uint8_t a[] = {1u, 2u, 3u};
    CHECK(drm_node_publish(&node, a, 3u) == DRM_OK);
    CHECK(drm_node_publish(&node, a, 3u) == DRM_OK);
    uint8_t out[DRM_NODE_SLOT_BYTES];
    uint16_t out_len = 0;
    uint32_t out_seq = 0;
    CHECK(drm_node_read(&node, out, sizeof out, &out_len, &out_seq) == DRM_OK);
    CHECK(out_len == 3u && out_seq == 0u && out[0] == 1u);
    CHECK(drm_node_read(&node, out, sizeof out, &out_len, &out_seq) == DRM_OK);
    CHECK(out_seq == 1u);
    CHECK(drm_node_read(&node, out, sizeof out, &out_len, &out_seq) == DRM_ERR_NO_SAMPLE);
    /* Oversize sample is refused, never allocated around. */
    uint8_t big[DRM_NODE_SLOT_BYTES + 1u];
    for (size_t i = 0; i < sizeof big; ++i) {
        big[i] = 0u;
    }
    CHECK(drm_node_publish(&node, big, (uint16_t)sizeof big) == DRM_ERR_INVALID_ARG);
}

int main(void) {
    test_crc32c();
    test_cobs_golden();
    test_frame_roundtrip();
    test_corruption_resync();
    test_interop_matrix();
    test_static_node();
    if (failures == 0) {
        printf("mcu selftest: OK (host parity)\n");
        return 0;
    }
    printf("mcu selftest: %d failure(s)\n", failures);
    return 1;
}
