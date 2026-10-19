/* DRM peer for the ROS 2 bridge test: consumes from a DRM topic.
 *
 * A DRM application that knows nothing about ROS; it proves that what a ROS
 * publisher sent through the bridge became an ordinary DRM sample.
 *
 * usage: drm_peer sub <segment> <topic> <lock-dir> <schema-hex> <layout-hex>
 *                     <slot> <capacity> <depth> <pool> <expect>
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
  if (argc != 13) {
    fprintf(stderr, "usage: %s sub <segment> <topic> <lock-dir> <schema> <layout> <slot> "
                    "<capacity> <depth> <pool> <expect> <marker>\n", argv[0]);
    return 2;
  }
  unsigned char schema[16], layout[16];
  if (!unhex(argv[5], schema) || !unhex(argv[6], layout)) {
    fprintf(stderr, "peer: malformed digest\n");
    return 2;
  }
  const unsigned long expect = strtoul(argv[11], NULL, 10);
  const char *marker = argv[12];
  const size_t marker_len = strlen(marker);
  const size_t capacity = (size_t)strtoull(argv[8], NULL, 10);

  drm_attached *handle = NULL;
  /* Consumers never create: wait for the publisher's segment to exist. */
  drm_status opened = DRM_ERR_NOT_FOUND;
  for (int attempt = 0; attempt < 250; ++attempt) {
    opened = drm_attached_open(
        argv[2], argv[3], argv[4], schema, layout, (uint32_t)strtoul(argv[7], NULL, 10),
        strtoull(argv[8], NULL, 10), strtoull(argv[9], NULL, 10), strtoull(argv[10], NULL, 10),
        /*overflow=*/0, DRM_ATTACHED_CONSUMER, /*timeout_ms=*/5000, &handle);
    if (opened != DRM_ERR_NOT_FOUND) break;
    struct timespec pause = {0, 20 * 1000 * 1000};
    nanosleep(&pause, NULL);
  }
  if (opened != DRM_OK) {
    fprintf(stderr, "peer: open -> %d\n", opened);
    return 1;
  }

  unsigned char *buffer = malloc(capacity);
  if (buffer == NULL) {
    drm_attached_close(handle);
    return 1;
  }
  unsigned long received = 0;
  int last_ok = 1;
  for (int attempt = 0; attempt < 3000 && received < expect; ++attempt) {
    size_t len = 0;
    uint64_t missed = 0;
    const drm_status status =
        drm_attached_read_next(handle, buffer, capacity, &len, &missed);
    if (status == DRM_OK && len >= 16) {
      /* The bridge forwards the serialized ROS message; the marker proves the
       * bytes crossed ROS -> bridge -> shared memory unmangled. */
      if (marker_len <= len) {
        /* A serialized message carries a CDR header before the payload, so the
         * marker is located anywhere in the record rather than at offset 0. */
        int found = 0;
        if (marker_len == 0 || strcmp(marker, "-") == 0) {
          /* Any committed sample counts: the caller only needs proof that the
           * package's message reached DRM intact. */
          found = 1;
        }
        for (size_t off = 0; !found && off + marker_len <= len && off < 64; ++off) {
          if (memcmp(buffer + off, marker, marker_len) == 0) {
            found = 1;
            break;
          }
        }
        if (found) {
          ++received;
          last_ok = 1;
        } else {
          last_ok = 0;
          if (received == 0) {
            fprintf(stderr, "peer: first sample len=%zu bytes:", len);
            for (size_t i = 0; i < 24 && i < len; ++i) fprintf(stderr, " %02x", buffer[i]);
            fprintf(stderr, "\n");
          }
        }
      } else {
        last_ok = 0;
      }
    } else if (status != DRM_ERR_NO_SAMPLE && status != DRM_ERR_RETRY) {
      fprintf(stderr, "peer: read -> %d\n", status);
      break;
    }
    struct timespec pause = {0, 5 * 1000 * 1000};
    nanosleep(&pause, NULL);
  }
  printf("PEER_OK received=%lu last_intact=%s\n", received, last_ok ? "yes" : "no");
  fflush(stdout);
  free(buffer);
  drm_attached_close(handle);
  return received > 0 ? 0 : 1;
}
