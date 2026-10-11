/*
 * ODR check — translation unit A. Includes the header-only binding and
 * instantiates its templates so any non-inline definition would collide at
 * link time with TU B. REQ: REQ-BIND-013 (single-header ODR discipline).
 */
#include <drm/drm.hpp>

#include <cstdint>

std::uint64_t drm_odr_probe_a() noexcept {
    drm::Borrow<std::uint8_t> guard; /* default (invalid) guard */
    drm::Context ctx;                /* default (empty) context */
    drm::Publisher pub;
    drm::Consumer con;
    std::uint64_t acc = guard.token();
    acc += (ctx.get() != nullptr) ? 1u : 0u;
    acc += (pub.get() != nullptr) ? 1u : 0u;
    acc += (con.get() != nullptr) ? 1u : 0u;
    acc += drm::ok(DRM_OK) ? 1u : 0u;
    return acc;
}
