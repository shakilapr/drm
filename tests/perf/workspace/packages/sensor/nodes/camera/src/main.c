/* Perf fixture node: camera (publisher).
 *
 * Smoke source at the same fidelity as `examples/mixed_robot`: the node
 * manifest's build command asserts the generated include tree and deployment
 * metadata exist; this file documents the intended wiring. The perf runner
 * (`cargo xtask perf-matrix`) drives the declared topic layout directly.
 *
 * REQ: REQ-BIND-002 (publisher endpoint), REQ-QAL-011 (latency baseline).
 */
#include <stdint.h>

extern int drm_context_open(const char *path, void **out);
extern int drm_publisher_open(void *ctx, uint32_t topic_id, void **out);
extern int drm_publish_copy(void *publisher, const void *data, uint64_t len);

int main(void) {
    /* sensor/imu and sensor/image publish loops are driven by the perf runner. */
    return 0;
}
