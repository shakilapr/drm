# Phase 0 integration fixture

The baseline every later phase is measured against: a workspace that is small
enough to reason about and complete enough to catch a lying gate.

- `packages/counter/topics/value.drm` — one primitive topic (`value: u64`).
- `packages/counter/nodes/producer/drm.toml` — declares it publishes `counter/value`.
- `packages/counter/nodes/consumer/drm.toml` — declares it consumes `counter/value`.
- `apps/producer`, `apps/consumer` — **two separately built applications**
  (independent cargo projects, separate compilation and link steps) that meet on
  the topic's shared-memory segment.

`cargo xtask integration --check` drives the whole thing: it generates the
workspace's artifacts, runs the strict `gen-check` over this fixture, builds both
apps, runs them against one segment and asserts the exchange.

Deliberate properties, each one a gate:

| Property | What would otherwise slip through |
|---|---|
| At least one topic **and** one node | a fixture that declares nothing passes every check |
| Generated artifacts must exist | a gate that verifies absence reports success |
| Two independently built binaries | a single-binary "integration test" that cannot catch a build-order or identity mismatch |
| Primitive payload only (no ROS) | the exchange is DRM's own; ROS is a later adapter |
