"""DRM Python binding facade: owned reads only over the native `drm._native` core.

The native core ([`drm._native`]) wraps `drm-ffi`; this facade adds
manifest-declared topic resolution and raises :class:`ContractError` for an
undeclared topic at first use (bindings-cpp-rust-python.md I-B7). Every read
returns application-owned memory (:class:`Copy`); no Python-visible object ever
references DRM shared memory and the buffer protocol is not implemented
(I-B5/I-B6).

REQ: REQ-BIND-014, REQ-BIND-015, REQ-BIND-021, REQ-BIND-022.
"""

from __future__ import annotations

from . import _native
from ._native import ContractError, Copy, DrmError, GapError

__all__ = [
    "Context",
    "Topic",
    "Publisher",
    "Copy",
    "ContractError",
    "DrmError",
    "GapError",
    "abi_version",
]

# Declared topics for this phase: the in-process backing declares one topic id
# (`control/target`). The generated per-manifest table lands with codegen in
# Phase 24. REQ: REQ-BIND-014.
_DECLARED: dict[str, int] = {"control/target": 0}


def abi_version() -> int:
    """The DRM C ABI version implemented by the native core.

    REQ: REQ-BIND-018.
    """
    return _native.abi_version()


def _resolve(name: str | int) -> int | None:
    """Resolve a topic name or id to a declared id, or ``None`` if undeclared.

    REQ: REQ-BIND-014.
    """
    if isinstance(name, int):
        return name if name in _native.Context.declared_topics() else None
    return _DECLARED.get(name)


class Context:
    """A deployment context and its endpoints; a context manager.

    Endpoints opened through :meth:`topic` / :meth:`publisher` are closed when
    the context closes, before the underlying handle (I-A7).

    REQ: REQ-BIND-021, REQ-BIND-002.
    """

    def __init__(self, path: str | None = None) -> None:
        self._native = _native.Context.open(path)
        self._endpoints: list[object] = []
        self._closed = False

    def _track(self, endpoint: object) -> object:
        self._endpoints.append(endpoint)
        return endpoint

    def _open_consumer(self, name: str | int) -> _native.Consumer:
        topic_id = _resolve(name)
        if topic_id is None:
            raise ContractError(f"topic {name!r} is not declared in the manifest")
        endpoint = self._native.open_consumer(topic_id)
        self._track(endpoint)
        return endpoint

    def _open_publisher(self, name: str | int) -> _native.Publisher:
        topic_id = _resolve(name)
        if topic_id is None:
            raise ContractError(f"topic {name!r} is not declared in the manifest")
        endpoint = self._native.open_publisher(topic_id)
        self._track(endpoint)
        return endpoint

    def topic(self, name: str | int) -> Topic:
        """A consumer endpoint for a declared topic (opened at first use).

        REQ: REQ-BIND-014, REQ-BIND-015.
        """
        return Topic(self, name)

    def publisher(self, name: str | int) -> Publisher:
        """A publisher endpoint for a declared topic (opened at first use).

        REQ: REQ-BIND-014.
        """
        return Publisher(self, name)

    def close(self) -> None:
        """Close every endpoint, then the context. REQ: REQ-BIND-002."""
        if self._closed:
            return
        for endpoint in self._endpoints:
            endpoint.close()  # type: ignore[attr-defined]
        self._endpoints.clear()
        self._native.close()
        self._closed = True

    def __enter__(self) -> Context:
        return self

    def __exit__(self, *exc: object) -> bool:
        self.close()
        return False


class Topic:
    """Owned-read consumer endpoint; a context manager.

    Entering opens the endpoint; exiting closes it, releasing its hazard slot.
    ``latest()`` / ``next()`` copy into :class:`Copy` before returning (I-B5).

    REQ: REQ-BIND-002, REQ-BIND-015, REQ-BIND-022.
    """

    def __init__(self, context: Context, name: str | int) -> None:
        self._context = context
        self._name = name
        self._native: _native.Consumer | None = None
        self._closed = False

    def _ensure(self) -> _native.Consumer:
        if self._closed:
            raise DrmError("topic is closed")
        if self._native is None:
            self._native = self._context._open_consumer(self._name)
        return self._native

    def latest(self) -> Copy | None:
        """Owned read of the newest sample; ``None`` on an empty channel.

        REQ: REQ-BIND-015.
        """
        return self._ensure().latest()

    def next(self) -> Copy | None:
        """Owned sequential read; ``None`` caught up, :class:`GapError` lapped.

        REQ: REQ-BIND-015.
        """
        return self._ensure().next()

    def close(self) -> None:
        """Close the endpoint, releasing its hazard slot. REQ: REQ-BIND-002."""
        if self._native is not None:
            self._native.close()
            if self._native in self._context._endpoints:
                self._context._endpoints.remove(self._native)
            self._native = None
        self._closed = True

    def __enter__(self) -> Topic:
        self._ensure()
        return self

    def __exit__(self, *exc: object) -> bool:
        self.close()
        return False


class Publisher:
    """Publisher endpoint; a context manager. REQ: REQ-BIND-002."""

    def __init__(self, context: Context, name: str | int) -> None:
        self._context = context
        self._name = name
        self._native: _native.Publisher | None = None
        self._closed = False

    def _ensure(self) -> _native.Publisher:
        if self._closed:
            raise DrmError("publisher is closed")
        if self._native is None:
            self._native = self._context._open_publisher(self._name)
        return self._native

    def publish(self, data: bytes) -> None:
        """Copy-publish ``data`` on the endpoint's topic. REQ: REQ-BIND-002."""
        self._ensure().publish(bytes(data))

    def close(self) -> None:
        """Close the endpoint. REQ: REQ-BIND-002."""
        if self._native is not None:
            self._native.close()
            if self._native in self._context._endpoints:
                self._context._endpoints.remove(self._native)
            self._native = None
        self._closed = True

    def __enter__(self) -> Publisher:
        self._ensure()
        return self

    def __exit__(self, *exc: object) -> bool:
        self.close()
        return False
