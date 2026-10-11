/*
 * C++ Borrow<T> RAII driver — plain main() + <cassert> (NO Catch2).
 *
 * Proves I-B1 (REQ-BIND-013): a Borrow guard releases via drm_borrow_release on
 * every scope exit — normal exit, early return, exception unwinding — exactly
 * once, and that a moved-from guard releases nothing. Release is proven two
 * ways: (a) the ABI's single-outstanding-borrow rule (a second borrow while one
 * is held returns Busy; after release a fresh borrow succeeds, and re-releasing
 * the old token returns InvalidArg), and (b) when compiled with
 * -DDRM_CPP_TEST_HOOKS against the test-only stub, an exact release count.
 *
 * Links the test-only stub (drm_stub.c) by default; it also runs unmodified
 * against the real drm-ffi cdylib (without -DDRM_CPP_TEST_HOOKS).
 *
 * REQ: REQ-BIND-013 (I-B1), REQ-BIND-001, REQ-BIND-006, REQ-BIND-021.
 */
#include <drm/drm.hpp>

#include <cassert>
#include <cstdint>
#include <cstdio>
#include <stdexcept>
#include <utility>

#ifdef DRM_CPP_TEST_HOOKS
extern "C" std::uint32_t drm_test_release_count(void);
extern "C" void drm_test_release_reset(void);
#endif

namespace {

constexpr std::uint32_t kTopic = 0;

/* Publish a small fixed sample on the (single) declared topic. */
void publish_sample(drm::Publisher &pub) {
    const std::uint8_t data[4] = {0xde, 0xad, 0xbe, 0xef};
    assert(pub.publish(data, sizeof data) == DRM_OK);
}

/* A fresh borrow must be available (no outstanding borrow remains). It is
 * released here on scope exit. REQ: REQ-BIND-006. */
void assert_borrowable(const drm::Consumer &con) {
    auto probe = con.borrow_latest<std::uint8_t>();
    assert(probe.has_value());
    assert(static_cast<bool>(*probe));
    assert(!probe->bytes().empty());
}

/* Release is proven exactly-once: the token is no longer outstanding, so a
 * re-release is InvalidArg (double release) and a fresh borrow still works. */
void assert_released_once(const drm::Consumer &con, std::uint32_t token) {
    assert(drm_borrow_release(con.get(), token) == DRM_ERR_INVALID_ARG);
    assert_borrowable(con);
}

/* Scenario 1 — normal scope exit. */
void scenario_normal_exit(const drm::Consumer &con) {
#ifdef DRM_CPP_TEST_HOOKS
    const std::uint32_t before = drm_test_release_count();
#endif
    std::uint32_t token = 0;
    {
        auto borrow = con.borrow_latest<std::uint8_t>();
        assert(borrow.has_value());
        token = borrow->token();
        assert(token != 0);
        /* A second borrow while one is outstanding => Busy (single borrow). */
        auto second = con.borrow_latest<std::uint8_t>();
        assert(!second.has_value());
    } /* ~Borrow releases exactly once */
#ifdef DRM_CPP_TEST_HOOKS
    assert(drm_test_release_count() == before + 1u);
#endif
    assert_released_once(con, token);
}

/* Scenario 2 — early return from inside the guard's scope. */
int borrow_then_early_return(const drm::Consumer &con, std::uint32_t *out_token) {
    auto borrow = con.borrow_latest<std::uint8_t>();
    assert(borrow.has_value());
    *out_token = borrow->token();
    return 0; /* early return; borrow destructs here and releases */
}

void scenario_early_return(const drm::Consumer &con) {
#ifdef DRM_CPP_TEST_HOOKS
    const std::uint32_t before = drm_test_release_count();
#endif
    std::uint32_t token = 0;
    assert(borrow_then_early_return(con, &token) == 0);
#ifdef DRM_CPP_TEST_HOOKS
    assert(drm_test_release_count() == before + 1u);
#endif
    assert_released_once(con, token);
}

/* Scenario 3 — exception propagation releases while unwinding. */
struct unwind_probe {};

void borrow_then_throw(const drm::Consumer &con, std::uint32_t *out_token) {
    auto borrow = con.borrow_latest<std::uint8_t>();
    assert(borrow.has_value());
    *out_token = borrow->token();
    throw unwind_probe{}; /* unwinds; borrow destructs => release */
}

void scenario_exception(const drm::Consumer &con) {
#ifdef DRM_CPP_TEST_HOOKS
    const std::uint32_t before = drm_test_release_count();
#endif
    std::uint32_t token = 0;
    bool caught = false;
    try {
        borrow_then_throw(con, &token);
    } catch (const unwind_probe &) {
        caught = true;
    }
    assert(caught);
#ifdef DRM_CPP_TEST_HOOKS
    assert(drm_test_release_count() == before + 1u);
#endif
    assert_released_once(con, token);
}

/* Scenario 4 — move semantics: exactly one release across a move. */
void scenario_move(const drm::Consumer &con) {
#ifdef DRM_CPP_TEST_HOOKS
    const std::uint32_t before = drm_test_release_count();
#endif
    std::uint32_t token = 0;
    {
        auto source = con.borrow_latest<std::uint8_t>();
        assert(source.has_value());
        token = source->token();
        drm::Borrow<std::uint8_t> moved = std::move(*source);
        assert(static_cast<bool>(moved));
        assert(moved.token() == token);
        /* Moved-from guard is invalid and releases nothing (I-B1). */
        assert(!static_cast<bool>(*source));
        assert(source->token() == 0);
        /* The moved-from guard may be destroyed early without releasing. */
    } /* only the moved-to guard releases */
#ifdef DRM_CPP_TEST_HOOKS
    assert(drm_test_release_count() == before + 1u);
#endif
    assert_released_once(con, token);
}

} // namespace

int main() {
#ifdef DRM_CPP_TEST_HOOKS
    drm_test_release_reset();
#endif

    auto ctx = drm::Context::open("drm-cpp-borrow-test");
    assert(ctx.has_value());
    assert(static_cast<bool>(*ctx));

    /* Undeclared topic => open refused (ABI Permission). REQ: REQ-BIND-008. */
    assert(!ctx->open_publisher(kTopic + 1u).has_value());

    auto pub = ctx->open_publisher(kTopic);
    assert(pub.has_value());
    auto con = ctx->open_consumer(kTopic);
    assert(con.has_value());

    publish_sample(*pub);

    scenario_normal_exit(*con);
    scenario_early_return(*con);
    scenario_exception(*con);
    scenario_move(*con);

    std::puts("borrow_raii: all scenarios passed (I-B1)");
    return 0;
}
