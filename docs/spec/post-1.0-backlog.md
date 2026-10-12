# Post-1.0 Backlog

Derived component spec (codebase.md §2.4; non-normative — the algorithm doc
[qualification.md](../algorithms/program/qualification.md) wins conflicts).
`REQ-QAL-031`; the groomed post-1.0 backlog handed to product planning at
program close-out (M24W4). Machine form:
[xtask/src/closeout.rs](../../xtask/src/closeout.rs).

## Studio deferred

Items consciously deferred from the v1.0 Studio surface (codebase §2.3 R4;
[studio-protocol.md](../algorithms/studio/studio-protocol.md)). Each is
additive and lands behind the frozen extension surface
([api-freeze.md](api-freeze.md), I-A8).

- Live 3D viewport renderer (WebGL/WebGPU) — the Graph activity ships a
  read-only route/link projection at v1.0.
- Multi-window docking and layout persistence.
- Extension marketplace discovery and signing (the extension host is the frozen
  v1.0 surface).
- Collaborative multi-user editing sessions.

## Extension roadmap

Planned evolution of the extension API after v1.0. The v1.0 surface is frozen;
these additions are append-only and versioned with a new `DRM_ABI_VERSION`.

- Extension fuel-budget introspection and per-call cost reporting.
- Streaming host-call results for long-running queries.
- Capability-scoped filesystem/network host functions.
- Deterministic replay hooks for extension sessions.

## Verification

```text
cargo xtask closeout --check
```

`POST_10_BACKLOG = 1` iff both sections carry at least one entry; the backlog is
part of the program close-out gate (`REQ-QAL-033`).

REQ: REQ-QAL-031.
