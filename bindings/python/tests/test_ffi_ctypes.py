"""drm-ffi ctypes conformance harness (test-only).

Loads the `drm-ffi` cdylib (`libdrm_ffi.so`) via `ctypes` and drives the C ABI
directly — the third, non-Rust consumer of the same shared object — to confirm
the exported `drm.h` surface, the status taxonomy, and the in-process endpoint
behaviour (publish/read/borrow + negatives) match the Rust-side suite.

The cdylib is produced by `cargo build -p drm-ffi` (`crate-type = ["rlib",
"cdylib"]`). When no shared object is present (e.g. a source-only checkout with
no Rust build) every test is skipped with an explicit reason rather than failing
or faking a pass; set `DRM_FFI_LIB` to point at a specific build.

REQ: REQ-BIND-004, REQ-BIND-005, REQ-BIND-006, REQ-BIND-007, REQ-BIND-017,
REQ-BIND-021.
"""

from __future__ import annotations

import ctypes
import os
import platform
from pathlib import Path

import pytest

REPO = Path(__file__).resolve().parents[3]

# Mirrors bindings/c/include/drm/drm.h (single source of truth).
DRM_OK = 0
DRM_ERR_INVALID_ARG = -1
DRM_ERR_BUSY = -11
DRM_ERR_NO_SAMPLE = -12
DRM_ABI_VERSION = 0
ENDPOINT_SAMPLE_MAX = 64

_SO_NAMES = {
    "Linux": "libdrm_ffi.so",
    "Darwin": "libdrm_ffi.dylib",
    "Windows": "drm_ffi.dll",
}


class BorrowView(ctypes.Structure):
    """`drm_borrow_view` — {const void *, size_t, uint64_t, uint32_t}."""

    _fields_ = [
        ("data", ctypes.c_void_p),
        ("len", ctypes.c_size_t),
        ("sequence", ctypes.c_uint64),
        ("token", ctypes.c_uint32),
    ]


def _candidate_paths() -> list[Path]:
    env = os.environ.get("DRM_FFI_LIB")
    name = _SO_NAMES.get(platform.system(), "libdrm_ffi.so")
    paths: list[Path] = []
    if env:
        paths.append(Path(env))
    for profile in ("debug", "release"):
        paths.append(REPO / "target" / profile / name)
    return paths


def _find_library() -> Path | None:
    env = os.environ.get("DRM_FFI_LIB")
    if env:
        # An explicit override is authoritative: a stale path must skip, not
        # silently fall back to a different build.
        path = Path(env)
        return path if path.is_file() else None
    for path in _candidate_paths():
        if path.is_file():
            return path
    return None


def _bind(path: Path) -> ctypes.CDLL:
    lib = ctypes.CDLL(str(path))
    pp_ctx = ctypes.POINTER(ctypes.c_void_p)
    pp_pub = ctypes.POINTER(ctypes.c_void_p)
    pp_con = ctypes.POINTER(ctypes.c_void_p)
    u8p = ctypes.POINTER(ctypes.c_uint8)
    usize_p = ctypes.POINTER(ctypes.c_size_t)
    u64_p = ctypes.POINTER(ctypes.c_uint64)

    lib.drm_abi_version.restype = ctypes.c_int32
    lib.drm_abi_version.argtypes = []

    lib.drm_context_open.restype = ctypes.c_int32
    lib.drm_context_open.argtypes = [ctypes.c_char_p, pp_ctx]
    lib.drm_context_close.restype = ctypes.c_int32
    lib.drm_context_close.argtypes = [ctypes.c_void_p]

    lib.drm_publisher_open.restype = ctypes.c_int32
    lib.drm_publisher_open.argtypes = [ctypes.c_void_p, ctypes.c_uint32, pp_pub]
    lib.drm_publisher_close.restype = ctypes.c_int32
    lib.drm_publisher_close.argtypes = [ctypes.c_void_p]

    lib.drm_consumer_open.restype = ctypes.c_int32
    lib.drm_consumer_open.argtypes = [ctypes.c_void_p, ctypes.c_uint32, pp_con]
    lib.drm_consumer_close.restype = ctypes.c_int32
    lib.drm_consumer_close.argtypes = [ctypes.c_void_p]

    lib.drm_publish_copy.restype = ctypes.c_int32
    lib.drm_publish_copy.argtypes = [ctypes.c_void_p, u8p, ctypes.c_size_t]

    lib.drm_read_latest.restype = ctypes.c_int32
    lib.drm_read_latest.argtypes = [ctypes.c_void_p, u8p, ctypes.c_size_t, usize_p]
    lib.drm_read_next.restype = ctypes.c_int32
    lib.drm_read_next.argtypes = [ctypes.c_void_p, u8p, ctypes.c_size_t, usize_p, u64_p]

    lib.drm_borrow_latest.restype = ctypes.c_int32
    lib.drm_borrow_latest.argtypes = [ctypes.c_void_p, ctypes.POINTER(BorrowView)]
    lib.drm_borrow_release.restype = ctypes.c_int32
    lib.drm_borrow_release.argtypes = [ctypes.c_void_p, ctypes.c_uint32]
    return lib


@pytest.fixture(scope="module")
def lib() -> ctypes.CDLL:
    path = _find_library()
    if path is None:
        pytest.skip(
            "drm-ffi cdylib not built (crate-type includes cdylib). "
            "Run `cargo build -p drm-ffi`, or set DRM_FFI_LIB. Looked in: "
            + ", ".join(str(p) for p in _candidate_paths())
        )
    return _bind(path)


def _open_endpoints(lib: ctypes.CDLL) -> tuple[ctypes.c_void_p, ctypes.c_void_p, ctypes.c_void_p]:
    ctx = ctypes.c_void_p()
    assert lib.drm_context_open(None, ctypes.byref(ctx)) == DRM_OK
    pub = ctypes.c_void_p()
    assert lib.drm_publisher_open(ctx, 0, ctypes.byref(pub)) == DRM_OK
    con = ctypes.c_void_p()
    assert lib.drm_consumer_open(ctx, 0, ctypes.byref(con)) == DRM_OK
    return ctx, pub, con


def _close_endpoints(
    lib: ctypes.CDLL, ctx: ctypes.c_void_p, pub: ctypes.c_void_p, con: ctypes.c_void_p
) -> None:
    assert lib.drm_publisher_close(pub) == DRM_OK
    assert lib.drm_consumer_close(con) == DRM_OK
    assert lib.drm_context_close(ctx) == DRM_OK


def test_abi_version(lib: ctypes.CDLL) -> None:
    """REQ: REQ-BIND-007 — the probe mirrors DRM_ABI_VERSION in drm.h."""
    assert lib.drm_abi_version() == DRM_ABI_VERSION


def test_publish_read_borrow_roundtrip(lib: ctypes.CDLL) -> None:
    """REQ: REQ-BIND-005, REQ-BIND-006 — C-ABI round trip through ctypes."""
    ctx, pub, con = _open_endpoints(lib)

    pattern = bytes([0xA5]) * ENDPOINT_SAMPLE_MAX
    src = (ctypes.c_uint8 * ENDPOINT_SAMPLE_MAX).from_buffer_copy(pattern)
    assert lib.drm_publish_copy(pub, src, ENDPOINT_SAMPLE_MAX) == DRM_OK

    dst = (ctypes.c_uint8 * ENDPOINT_SAMPLE_MAX)()
    out_len = ctypes.c_size_t(0)
    assert lib.drm_read_latest(con, dst, ENDPOINT_SAMPLE_MAX, ctypes.byref(out_len)) == DRM_OK
    assert out_len.value == ENDPOINT_SAMPLE_MAX
    assert bytes(dst) == pattern

    view = BorrowView()
    assert lib.drm_borrow_latest(con, ctypes.byref(view)) == DRM_OK
    assert view.len == ENDPOINT_SAMPLE_MAX
    assert view.token != 0
    assert ctypes.string_at(view.data, view.len) == pattern
    assert lib.drm_borrow_release(con, view.token) == DRM_OK
    # Double release ⇒ InvalidArg.
    assert lib.drm_borrow_release(con, view.token) == DRM_ERR_INVALID_ARG

    _close_endpoints(lib, ctx, pub, con)


def test_negative_null_and_oversize(lib: ctypes.CDLL) -> None:
    """REQ: REQ-BIND-005, REQ-BIND-017 — explicit status, never a crash."""
    # Null handles / out-params.
    assert lib.drm_read_latest(None, None, 0, None) == DRM_ERR_INVALID_ARG
    assert lib.drm_read_next(None, None, 0, None, None) == DRM_ERR_INVALID_ARG
    assert lib.drm_borrow_latest(None, None) == DRM_ERR_INVALID_ARG
    assert lib.drm_borrow_release(None, 1) == DRM_ERR_INVALID_ARG
    assert lib.drm_context_open(None, None) == DRM_ERR_INVALID_ARG
    assert lib.drm_context_close(None) == DRM_ERR_INVALID_ARG
    assert lib.drm_publisher_close(None) == DRM_ERR_INVALID_ARG
    assert lib.drm_consumer_close(None) == DRM_ERR_INVALID_ARG

    ctx, pub, con = _open_endpoints(lib)
    src = (ctypes.c_uint8 * ENDPOINT_SAMPLE_MAX).from_buffer_copy(b"\x00" * ENDPOINT_SAMPLE_MAX)

    # Oversize publish length (past the fixed structural maximum).
    assert lib.drm_publish_copy(pub, src, ENDPOINT_SAMPLE_MAX + 1) == DRM_ERR_INVALID_ARG
    assert lib.drm_publish_copy(pub, src, 2**64 - 1) == DRM_ERR_INVALID_ARG
    # Null data with a non-zero length.
    assert lib.drm_publish_copy(pub, None, ENDPOINT_SAMPLE_MAX) == DRM_ERR_INVALID_ARG

    # Empty channel read ⇒ NoSample (a defined status, not a crash).
    empty = (ctypes.c_uint8 * ENDPOINT_SAMPLE_MAX)()
    out_len = ctypes.c_size_t(0)
    assert lib.drm_read_latest(con, empty, ENDPOINT_SAMPLE_MAX, ctypes.byref(out_len)) == DRM_ERR_NO_SAMPLE

    _close_endpoints(lib, ctx, pub, con)


def test_negative_too_small_buffer(lib: ctypes.CDLL) -> None:
    """REQ: REQ-BIND-005 — too-small read buffer: required size, no partial copy."""
    ctx, pub, con = _open_endpoints(lib)
    pattern = bytes([0x3C]) * ENDPOINT_SAMPLE_MAX
    src = (ctypes.c_uint8 * ENDPOINT_SAMPLE_MAX).from_buffer_copy(pattern)
    assert lib.drm_publish_copy(pub, src, ENDPOINT_SAMPLE_MAX) == DRM_OK

    small = (ctypes.c_uint8 * 8)(*([0xEE] * 8))
    out_len = ctypes.c_size_t(999)
    assert lib.drm_read_latest(con, small, 8, ctypes.byref(out_len)) == DRM_ERR_INVALID_ARG
    assert out_len.value == ENDPOINT_SAMPLE_MAX
    assert bytes(small) == bytes([0xEE] * 8), "no partial copy"

    # Zero-cap size query.
    required = ctypes.c_size_t(0)
    assert lib.drm_read_latest(con, None, 0, ctypes.byref(required)) == DRM_ERR_INVALID_ARG
    assert required.value == ENDPOINT_SAMPLE_MAX

    _close_endpoints(lib, ctx, pub, con)


def test_consumer_lease_exhausted(lib: ctypes.CDLL) -> None:
    """REQ: REQ-BIND-010 — the ninth consumer open on one context ⇒ Busy."""
    ctx = ctypes.c_void_p()
    assert lib.drm_context_open(None, ctypes.byref(ctx)) == DRM_OK
    consumers = []
    for _ in range(8):
        con = ctypes.c_void_p()
        assert lib.drm_consumer_open(ctx, 0, ctypes.byref(con)) == DRM_OK
        consumers.append(con)
    overflow = ctypes.c_void_p(0x1234)
    assert lib.drm_consumer_open(ctx, 0, ctypes.byref(overflow)) == DRM_ERR_BUSY
    assert overflow.value in (None, 0), "failed open writes no handle"
    for con in consumers:
        assert lib.drm_consumer_close(con) == DRM_OK
    assert lib.drm_context_close(ctx) == DRM_OK
