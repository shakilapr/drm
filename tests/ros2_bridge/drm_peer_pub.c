/* DRM peer for the ROS 2 bridge test: publishes into a DRM topic.
 *
 * A DRM application in the ordinary sense — it knows nothing about ROS. It reads
 * its connection parameters from the generated deployment config (passed in by
 * the test harness, which extracts them with jq) so nothing is hand-identified.
 *
 * usage: drm_peer pub <segment> <topic> <lock-dir> <schema-hex> <layout-hex>
 *                     <slot> <capacity> <depth> <pool> <count>
 */
#include <drm/drm.h>

#include <stdio.h>
#include <stdlib.h>
#include <string.h>
#include <time.h>

static int unhex(const char *hex, unsigned char out[16]) {
  if (strlen(hex) != 32) return 0;
  for (int i = 0; i < 16; ++i) {
    unsigned int byte = 0;
    if (sscanf(hex + i * 2, "%2x", &byte) != 1) return 0;
    out[i] = (unsigned char)byte;
  }
  return 1;
}

int main(int argc, char **argv) {
  if (argc != 12) {
    fprintf(stderr, "usage: %s pub <segment> <topic> <lock-dir> <schema> <layout> <slot> "
                    "<capacity> <depth> <pool> <count>\n", argv[0]);
    return 2;
  }
  unsigned char schema[16], layout[16];
  if (!unhex(argv[5], schema) || !unhex(argv[6], layout)) {
    fprintf(stderr, "peer: malformed digest\n");
    return 2;
  }
  drm_attached *handle = NULL;
  const drm_status opened = drm_attached_open(
      argv[2], argv[3], argv[4], schema, layout, (uint32_t)strtoul(argv[7], NULL, 10),
      strtoull(argv[8], NULL, 10), strtoull(argv[9], NULL, 10), strtoull(argv[10], NULL, 10),
      /*overflow=*/0, DRM_ATTACHED_PUBLISHER, /*timeout_ms=*/5000, &handle);
  if (opened != DRM_OK) {
    fprintf(stderr, "peer: open -> %d\n", opened);
    return 1;
  }
  const unsigned long count = strtoul(argv[11], NULL, 10);
  /* Payload: a valid CDR-serialized std_msgs/String, which is what a typed
   * bridge carries. Layout: 4-byte encapsulation header (CDR little-endian),
   * then a uint32 length including the NUL, then the bytes and the NUL. */
  unsigned char payload[256];
  for (unsigned long i = 0; i < count; ++i) {
    char text[64];
    const int written = snprintf(text, sizeof(text), "drm-%lu", i);
    const size_t text_len = (size_t)written + 1; /* include NUL */
    memset(payload, 0, sizeof(payload));
    payload[0] = 0x00;
    payload[1] = 0x01; /* CDR little-endian */
    payload[2] = 0x00;
    payload[3] = 0x00;
    const uint32_t length = (uint32_t)text_len;
    memcpy(payload + 4, &length, sizeof(length));
    memcpy(payload + 8, text, text_len);
    const size_t payload_len = 8 + text_len;
    uint64_t seq = 0;
    const drm_status status = drm_attached_publish(handle, payload, payload_len, &seq);
    if (status != DRM_OK) {
      fprintf(stderr, "peer: publish %lu -> %d\n", i, status);
      drm_attached_close(handle);
      return 1;
    }
    struct timespec pause = {0, 50 * 1000 * 1000};
    nanosleep(&pause, NULL);
  }
  printf("PEER_OK published=%lu\n", count);
  fflush(stdout);
  /* Stay attached so the segment outlives the last publish. */
  struct timespec linger = {2, 0};
  nanosleep(&linger, NULL);
  drm_attached_close(handle);
  return 0;
}
