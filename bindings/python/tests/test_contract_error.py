"""ContractError for undeclared topics; context managers release endpoints.

REQ: REQ-BIND-014, REQ-BIND-007, REQ-BIND-002.
"""

from __future__ import annotations

import pytest

drm = pytest.importorskip("drm", reason="run `maturin develop` to build drm._native")


def test_undeclared_name_raises_contract_error_at_first_use() -> None:
    """REQ: REQ-BIND-014 — constructing is fine; first use raises."""
    with drm.Context() as context:
        topic = context.topic("does/not/exist")
        with pytest.raises(drm.ContractError):
            topic.latest()


def test_undeclared_id_raises_contract_error() -> None:
    """REQ: REQ-BIND-014 — an undeclared numeric topic id is ContractError."""
    with drm.Context() as context:
        topic = context.topic(99)
        with pytest.raises(drm.ContractError):
            topic.latest()


def test_undeclared_publish_raises_contract_error() -> None:
    """REQ: REQ-BIND-014 — an undeclared publisher raises at first use."""
    with drm.Context() as context:
        publisher = context.publisher("does/not/exist")
        with pytest.raises(drm.ContractError):
            publisher.publish(b"x")


def test_contract_error_is_a_drm_error() -> None:
    """REQ: REQ-BIND-014, REQ-BIND-007 — ContractError derives from DrmError."""
    assert issubclass(drm.ContractError, drm.DrmError)


def test_context_manager_releases_endpoint_on_scope_exit() -> None:
    """REQ: REQ-BIND-002 — exiting the scope closes the endpoint.

    The close succeeds only because no hazard is pinned (a leaked borrow would
    be refused with ``Busy``); a closed topic then refuses further use.
    """
    context = drm.Context()
    with context.topic("control/target") as topic:
        assert topic.latest() is None
    with pytest.raises(drm.DrmError):
        topic.latest()
    context.close()
