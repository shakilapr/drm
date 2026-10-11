"""PyO3 native skeleton smoke: import, ABI probe, open/publish/owned-read.

REQ: REQ-BIND-015, REQ-BIND-021, REQ-BIND-002.
"""

from __future__ import annotations

import pytest

drm = pytest.importorskip("drm", reason="run `maturin develop` to build drm._native")


def test_abi_version_probe() -> None:
    """REQ: REQ-BIND-018 — the binding mirrors the C ABI version."""
    assert drm.abi_version() == 0


def test_open_publish_latest_roundtrip() -> None:
    """REQ: REQ-BIND-015, REQ-BIND-021 — open, publish, then an owned read."""
    with drm.Context() as context:
        with context.publisher("control/target") as publisher:
            publisher.publish(b"hello")
        with context.topic("control/target") as topic:
            copy = topic.latest()
            assert copy is not None
            assert copy.bytes()[:5] == b"hello"
            assert len(copy) == 64  # fixed-size topic, zero-padded


def test_latest_empty_channel_is_none() -> None:
    """REQ: REQ-BIND-015 — an empty channel yields None, never an exception."""
    with drm.Context() as context:
        with context.topic("control/target") as topic:
            assert topic.latest() is None
