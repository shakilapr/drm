/*
 * DRM C++ binding — minimal, header-only RAII over the canonical C ABI
 * (bindings/c/include/drm/drm.h). C++20 only; single header (ODR discipline).
 *
 * REQ: REQ-BIND-013 (C++17/20 RAII; release on every scope exit incl.
 *      exceptions; safe moved-from), REQ-BIND-001 (caller-owned borrow view +
 *      token; explicit release), REQ-BIND-006 (one outstanding borrow per
 *      consumer endpoint), REQ-BIND-021 (opaque context open/close),
 *      REQ-BIND-002 (explicit close), REQ-BIND-004 (opaque handles),
 *      REQ-BIND-011 (byte-backed views, no struct cast of shared memory).
 *
 * Design rules (bindings-cpp-rust-python.md §C++):
 *  - Borrow<T> is move-only; its destructor releases via drm_borrow_release on
 *    every path (normal exit, early return, exception unwinding) and is
 *    noexcept. A moved-from guard carries token == 0 and releases nothing, so
 *    there is no double release (I-B1).
 *  - Guards never hand out a raw pointer into the shared slot; payload access
 *    is byte-backed (I-B2). Generated per-field accessors arrive with codegen.
 *  - Context/Publisher/Consumer are thin move-only RAII owners of the opaque
 *    handles; each close is exactly once, driven by the destructor.
 *  - No allocation on the borrow path: the guard stores the caller-owned
 *    drm_borrow_view by value (REQ-BIND-003).
 *
 * Include paths: this header includes <drm/drm.h> from the C ABI, so a build
 * needs both -I<repo>/bindings/cpp/include and -I<repo>/bindings/c/include.
 */
#ifndef DRM_DRM_HPP
#define DRM_DRM_HPP

#include <drm/drm.h>

#include <cstddef>
#include <cstdint>
#include <optional>
#include <span>
#include <utility>

namespace drm {

/* True when a status is the ABI success code. REQ: REQ-BIND-007. */
[[nodiscard]] constexpr bool ok(drm_status status) noexcept { return status == DRM_OK; }

template <class T> class Borrow;
class Publisher;
class Consumer;

/*
 * RAII owner of a deployment context (opaque drm_context). Move-only.
 * REQ: REQ-BIND-021, REQ-BIND-004, REQ-BIND-002.
 */
class Context {
  public:
    Context() noexcept = default;
    Context(Context &&other) noexcept : ctx_(std::exchange(other.ctx_, nullptr)) {}
    Context &operator=(Context &&other) noexcept {
        if (this != &other) {
            reset();
            ctx_ = std::exchange(other.ctx_, nullptr);
        }
        return *this;
    }
    Context(const Context &) = delete;
    Context &operator=(const Context &) = delete;
    ~Context() noexcept { reset(); }

    /* Open a deployment context; std::nullopt on failure. `path` is a
     * NUL-terminated C string (or nullptr); the current ABI ignores it. */
    [[nodiscard]] static std::optional<Context> open(const char *path = nullptr) noexcept {
        drm_context *handle = nullptr;
        if (drm_context_open(path, &handle) != DRM_OK) {
            return std::nullopt;
        }
        return Context{handle};
    }

    [[nodiscard]] drm_context *get() const noexcept { return ctx_; }
    [[nodiscard]] explicit operator bool() const noexcept { return ctx_ != nullptr; }

    /* Endpoint opens validate the topic once here (REQ-BIND-008); an
     * undeclared topic yields std::nullopt (ABI Permission). */
    [[nodiscard]] std::optional<Publisher> open_publisher(std::uint32_t topic_id) noexcept;
    [[nodiscard]] std::optional<Consumer> open_consumer(std::uint32_t topic_id) noexcept;

  private:
    explicit Context(drm_context *handle) noexcept : ctx_(handle) {}
    void reset() noexcept {
        if (ctx_ != nullptr) {
            (void)drm_context_close(ctx_);
            ctx_ = nullptr;
        }
    }
    drm_context *ctx_ = nullptr;
};

/*
 * RAII owner of a publisher endpoint. Move-only. REQ: REQ-BIND-002.
 */
class Publisher {
  public:
    Publisher() noexcept = default;
    Publisher(Publisher &&other) noexcept : pub_(std::exchange(other.pub_, nullptr)) {}
    Publisher &operator=(Publisher &&other) noexcept {
        if (this != &other) {
            reset();
            pub_ = std::exchange(other.pub_, nullptr);
        }
        return *this;
    }
    Publisher(const Publisher &) = delete;
    Publisher &operator=(const Publisher &) = delete;
    ~Publisher() noexcept { reset(); }

    [[nodiscard]] drm_publisher *get() const noexcept { return pub_; }
    [[nodiscard]] explicit operator bool() const noexcept { return pub_ != nullptr; }

    /* Copy-publish helper (REQ-BIND-006): the caller owns `data` for `len`
     * bytes; `len` must not exceed the topic sample size. */
    [[nodiscard]] drm_status publish(const std::uint8_t *data, std::size_t len) const noexcept {
        return drm_publish_copy(pub_, data, len);
    }
    [[nodiscard]] drm_status publish(std::span<const std::uint8_t> bytes) const noexcept {
        return drm_publish_copy(pub_, bytes.data(), bytes.size());
    }

  private:
    friend class Context;
    explicit Publisher(drm_publisher *handle) noexcept : pub_(handle) {}
    void reset() noexcept {
        if (pub_ != nullptr) {
            (void)drm_publisher_close(pub_);
            pub_ = nullptr;
        }
    }
    drm_publisher *pub_ = nullptr;
};

/*
 * RAII owner of a consumer endpoint. Move-only. REQ: REQ-BIND-002, REQ-BIND-006.
 */
class Consumer {
  public:
    Consumer() noexcept = default;
    Consumer(Consumer &&other) noexcept : con_(std::exchange(other.con_, nullptr)) {}
    Consumer &operator=(Consumer &&other) noexcept {
        if (this != &other) {
            reset();
            con_ = std::exchange(other.con_, nullptr);
        }
        return *this;
    }
    Consumer(const Consumer &) = delete;
    Consumer &operator=(const Consumer &) = delete;
    ~Consumer() noexcept { reset(); }

    [[nodiscard]] drm_consumer *get() const noexcept { return con_; }
    [[nodiscard]] explicit operator bool() const noexcept { return con_ != nullptr; }

    /* Zero-copy borrow of the newest sample; std::nullopt on empty channel,
     * Busy (a borrow is already outstanding), or any ABI error.
     * REQ: REQ-BIND-001, REQ-BIND-006. */
    template <class T> [[nodiscard]] std::optional<Borrow<T>> borrow_latest() const noexcept {
        if (con_ == nullptr) {
            return std::nullopt;
        }
        drm_borrow_view view{};
        if (drm_borrow_latest(con_, &view) != DRM_OK) {
            return std::nullopt;
        }
        return std::optional<Borrow<T>>{std::in_place, con_, view};
    }

    /* Copy read into a caller-owned buffer; returns bytes written.
     * REQ: REQ-BIND-002. */
    [[nodiscard]] std::optional<std::size_t>
    read_latest(std::span<std::uint8_t> buf) const noexcept {
        if (con_ == nullptr) {
            return std::nullopt;
        }
        std::size_t out_len = 0;
        if (drm_read_latest(con_, buf.data(), buf.size(), &out_len) != DRM_OK) {
            return std::nullopt;
        }
        return out_len;
    }

  private:
    friend class Context;
    explicit Consumer(drm_consumer *handle) noexcept : con_(handle) {}
    void reset() noexcept {
        if (con_ != nullptr) {
            (void)drm_consumer_close(con_);
            con_ = nullptr;
        }
    }
    drm_consumer *con_ = nullptr;
};

/*
 * Move-only RAII guard over a caller-owned drm_borrow_view (REQ-BIND-001).
 * The destructor releases on every scope exit — normal, early return, or
 * exception unwinding — and is noexcept (I-B1). A moved-from guard has
 * token == 0 and releases nothing, so no double release is possible.
 *
 * T is the generated byte-backed topic type (Phase 24 codegen); the guard
 * itself never yields a pointer into the shared slot (I-B2). The transient
 * byte view is exposed only for this minimal binding and must not outlive the
 * guard.
 */
template <class T> class [[nodiscard]] Borrow {
  public:
    Borrow() noexcept = default;
    Borrow(Borrow &&other) noexcept
        : view_(other.view_), chan_(std::exchange(other.chan_, nullptr)) {
        other.view_.token = 0; /* moved-from invalid: releases nothing (I-B1) */
    }
    Borrow &operator=(Borrow &&) = delete;
    Borrow(const Borrow &) = delete;
    Borrow &operator=(const Borrow &) = delete;
    ~Borrow() noexcept {
        /* I-B1: release on every path; the token guard makes it idempotent. */
        if (view_.token != 0 && chan_ != nullptr) {
            (void)drm_borrow_release(chan_, view_.token);
        }
    }

    [[nodiscard]] std::span<const std::uint8_t> bytes() const noexcept {
        return {static_cast<const std::uint8_t *>(view_.data), view_.len};
    }
    [[nodiscard]] std::size_t size() const noexcept { return view_.len; }
    [[nodiscard]] std::uint64_t sequence() const noexcept { return view_.sequence; }
    [[nodiscard]] std::uint32_t token() const noexcept { return view_.token; }
    [[nodiscard]] explicit operator bool() const noexcept { return view_.token != 0; }

    /* Adopt an ABI borrow view taken by Consumer::borrow_latest. Public so
     * std::optional's in-place constructor (which is not a friend) can see it;
     * a non-zero token is required for the destructor to release anything. */
    Borrow(drm_consumer *chan, const drm_borrow_view &view) noexcept : view_(view), chan_(chan) {}

  private:
    drm_borrow_view view_{};
    drm_consumer *chan_ = nullptr;
};

inline std::optional<Publisher> Context::open_publisher(std::uint32_t topic_id) noexcept {
    if (ctx_ == nullptr) {
        return std::nullopt;
    }
    drm_publisher *handle = nullptr;
    if (drm_publisher_open(ctx_, topic_id, &handle) != DRM_OK) {
        return std::nullopt;
    }
    return Publisher{handle};
}

inline std::optional<Consumer> Context::open_consumer(std::uint32_t topic_id) noexcept {
    if (ctx_ == nullptr) {
        return std::nullopt;
    }
    drm_consumer *handle = nullptr;
    if (drm_consumer_open(ctx_, topic_id, &handle) != DRM_OK) {
        return std::nullopt;
    }
    return Consumer{handle};
}

} // namespace drm

#endif /* DRM_DRM_HPP */
