"""Owned-default reads: no Python object references DRM shared memory (I-B5).

REQ: REQ-BIND-015.
"""

from __future__ import annotations

import pytest

drm = pytest.importorskip("drm", reason="run `maturin develop` to build drm._native")


def test_numpy_is_owned_not_a_shared_view() -> None:
    """REQ: REQ-BIND-015 — the NumPy array's buffer is not the shared payload."""
    numpy = pytest.importorskip("numpy")
    with drm.Context() as context:
        with context.publisher("control/target") as publisher:
            publisher.publish(bytes([0xAB]) * 8)
        with context.topic("control/target") as topic:
            shared = topic._native._shared_payload_addr()
            copy = topic.latest()
            assert copy is not None
            array = copy.numpy()
            assert array.dtype == numpy.uint8
            start = int(array.ctypes.data)
            assert start != shared
            assert shared not in range(start, start + int(array.nbytes))


def test_bytes_read_is_owned() -> None:
    """REQ: REQ-BIND-015 — ``bytes`` copies into application memory."""
    with drm.Context() as context:
        with context.publisher("control/target") as publisher:
            publisher.publish(b"owned")
        with context.topic("control/target") as topic:
            copy = topic.latest()
            assert copy is not None
            raw = copy.bytes()
            assert raw[:5] == b"owned"
            assert raw == bytes(copy)


def test_retained_copy_is_independent_of_later_publishes() -> None:
    """REQ: REQ-BIND-015 — a retained Copy is unaffected by later publishes."""
    with drm.Context() as context:
        with context.publisher("control/target") as publisher:
            publisher.publish(b"first")
        with context.topic("control/target") as topic:
            first = topic.latest()
            assert first is not None
            with context.publisher("control/target") as publisher:
                publisher.publish(b"second")
            assert first.bytes()[:5] == b"first"
            second = topic.latest()
            assert second is not None
            assert second.bytes()[:6] == b"second"
