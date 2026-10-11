/*
 * ODR check — translation unit B (with main). Includes the same header and
 * links against TU A: duplicate non-inline symbols would fail the link.
 * REQ: REQ-BIND-013 (single-header ODR discipline).
 */
#include <drm/drm.hpp>

#include <cstdint>
#include <cstdio>

std::uint64_t drm_odr_probe_a() noexcept;

int main() {
    drm::Borrow<std::uint8_t> guard;
    drm::Context ctx;
    std::uint64_t b = guard.token();
    b += (ctx.get() != nullptr) ? 1u : 0u;
    b += drm::ok(DRM_OK) ? 1u : 0u;

    const std::uint64_t a = drm_odr_probe_a();
    if (a != b) {
        std::fputs("odr: probe mismatch between TUs\n", stderr);
        return 1;
    }
    std::puts("odr: two-TU link ok");
    return 0;
}
