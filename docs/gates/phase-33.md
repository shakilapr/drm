# Phase 33 Gate — DRM Spatial

Normative sources: [work-plan.md](../../work-plan.md) W7 Phase 33;
[spatial_architecture.md](../../docs/spatial_architecture.md);
[spatial-core.md](../algorithms/spatial/spatial-core.md),
[spatial-queries.md](../algorithms/spatial/spatial-queries.md),
[spatial-index.md](../algorithms/spatial/spatial-index.md). Owner: E2
(compiler/graph), E1 (runtime caches), E3 (router/REQ reviews), E4 (tests).

Status: **IN PROGRESS** — M19W1 landed the spatial type system, math library,
grammar extensions, static-collapse v0, and runtime edge-cache groundwork
([m19w1.md](m19w1.md)); M19W2 added the four-domain clock mapping with
drift/uncertainty, the versioned dynamic-edge cache, compiled-plan execution,
and the per-process plan cache ([m19w2.md](m19w2.md)). Coherent
`latest`/`at`/`predict`/`delayed_view` semantics, the dynamic hash grid, compiled
BVH, and live calibration remain M20–M22.

## Wave A (M19) — types, spaces, collapse

| Criterion | Evidence | Status |
|---|---|---|
| Distinct typed quantities `Point3/Direction3/Vector3/Pose3/Transform3/Twist3` | `crates/drm-spatial/src/types.rs`; `type_semantics` (I-S1, REQ-SPATIAL-036) | PASS (M19W1) |
| Rigid `SE(3)` only (translation + unit quaternion) | `crates/drm-spatial/src/transform.rs`; oracle compose/rotate (REQ-SPATIAL-038) | PASS (M19W1) |
| Compile-time `space:` binding; raw-numerics payload | `parser/spatial.rs`; `payload_layout` (I-S2, REQ-SPATIAL-005) | PASS (M19W1) |
| Dynamic `SpatialId` + `Stamped<T>` composition | `types.rs`; grammar dynamic form (REQ-SPATIAL-009/035) | PASS (M19W1) |
| WGS84→ECEF→ENU geodetic chain (small-offset local frame) | `crates/drm-spatial/src/geodetic.rs`; oracle geodetic cases (I-S5, REQ-SPATIAL-012) | PASS (M19W1) |
| Four clock domains; affine mapping; identity epoch; unresolved rejected; drift/uncertainty propagation | `crates/drm-spatial/src/clock.rs`; `clocks` suite + `clock` oracle (REQ-SPATIAL-004/041/051) | PASS (M19W1–M19W2) |
| `spaces/` path identity + static/dynamic inference | `crates/drm-workspace/src/spatial/`; `spatial_collapse` (I-D6, REQ-SPATIAL-031/047) | PASS (M19W1) |
| Static chains collapse to one build-time constant; MCU emission | `crates/drm-workspace/src/spatial/`; `spatial_collapse` (REQ-SPATIAL-049/017) | PASS (M19W1) |
| Versioned dynamic-edge cache; discontinuous flag; single-writer edges | `crates/drm-spatial/src/cache.rs`; `cache` suite (REQ-SPATIAL-050/035/016/030) | PASS (M19W1–M19W2) |
| Compiled plan execution + per-process `PlanCache` geometry shortcircuit | `crates/drm-spatial/src/plan.rs`; `spatial_cache_shortcircuit`/`spatial_cache_aging` (I-Q1/I-Q2, REQ-SPATIAL-034/002) | PASS (M19W2) |
| Independent mathematical reference implementation | `tests/oracle/spatial_oracle.py`, `tests/oracle/clock_oracle.py` (NumPy); `math`/`clocks` suites | PASS (M19W1–M19W2) |
| Coherent `latest`, `at`, `predict` (exp-map), `delayed_view` | — | pending (M20) |
| Incremental dynamic hash grid; compiled BVH; proximity completeness | — | pending (M20+) |
| Historical reparenting; live calibration; generational overlay | — | pending (M21+) |
| Exact spatial memory verification | — | pending (M21+) |

## Notes

- No architecture divergence in wave A: pinned invariants only; no ADR.
- `drm-spatial` is layer `runtime` and depends only on `drm-types`
  (codebase.md 2.1); `drm-workspace` stays toolchain-only.
- Regression permanence (AGENTS #14): M19W1 and M19W2 suites are blocking gates.

REQ: REQ-SPATIAL-002, REQ-SPATIAL-004, REQ-SPATIAL-005, REQ-SPATIAL-009,
REQ-SPATIAL-012, REQ-SPATIAL-017, REQ-SPATIAL-030, REQ-SPATIAL-031,
REQ-SPATIAL-034, REQ-SPATIAL-035, REQ-SPATIAL-036, REQ-SPATIAL-038,
REQ-SPATIAL-041, REQ-SPATIAL-047, REQ-SPATIAL-048, REQ-SPATIAL-049,
REQ-SPATIAL-050, REQ-SPATIAL-051.
