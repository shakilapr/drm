# Phase 33 Gate — DRM Spatial

Normative sources: [work-plan.md](../../work-plan.md) W7 Phase 33;
[spatial_architecture.md](../../docs/spatial_architecture.md);
[spatial-core.md](../algorithms/spatial/spatial-core.md),
[spatial-queries.md](../algorithms/spatial/spatial-queries.md),
[spatial-index.md](../algorithms/spatial/spatial-index.md). Owner: E2
(compiler/graph), E1 (runtime caches), E3 (router/REQ reviews), E4 (tests).

Status: **PASS** (M22W2) — every spatial requirement is evidenced; the final
gate ledger is [m22w2.md](m22w2.md), chaired by E1 and presented by E2. M19W1
landed the spatial type system, math library,
grammar extensions, static-collapse v0, and runtime edge-cache groundwork
([m19w1.md](m19w1.md)); M19W2 added the four-domain clock mapping with
drift/uncertainty, the versioned dynamic-edge cache, compiled-plan execution,
and the per-process plan cache ([m19w2.md](m19w2.md)); M19W3 added forest
disconnected-tree queries with explicit bridges, canonical boundary conventions,
geodetic local-precision bounds, and historical (sample-timestamp) evaluation
([m19w3.md](m19w3.md)); M20W1 added the compiled-plan query surface
`latest`/`at`/`predict`/`delayed_view`, sample-time-ordered history with the
interpolation + discontinuity guard, and live calibration
([m20w1.md](m20w1.md)); M20W2 added per-edge discontinuity markers with the
interpolation guard, the time-scoped plan registry, and transactional
reparenting (Preserve World/Local/Snap) with atomic tree commit
([m20w2.md](m20w2.md)); M20W3 added live calibration slots with a jump-threshold
discontinuity marker, the generational overlay-handle registry, and OS-backed
single-writer edge authority ([m20w3.md](m20w3.md)); M20W4 closed **Gate B**
(`I-Q1`..`I-Q16`) with provable history sizing/order, lockless slot coherence
(with an exhaustive interleaving model), a zero-allocation query executor, and
bounded generation-cached overlay plans ([m20w4.md](m20w4.md),
[month-20.md](month-20.md)). M21 added the runtime spatial index — the dynamic
hash grid, the compiled static BVH, and the two-source proximity union with
exact memory verification ([metrics-m21.md](../metrics/metrics-m21.md)) — and
the freestanding MCU/host reference suites; M21W4 completed the spatial
reference suite with the float-determinism banding evidence rule and drafted the
final gate ([m21w4.md](m21w4.md)). The Phase 33 final gate landed at M22W2
([m22w2.md](m22w2.md)): all three §Test Matrices re-ran green together against
mathematical references, the spatial registry rows were resolved to `tested`,
and `GATE_STATUS(Phase 33) = PASS`.

## Wave A (M19) — types, spaces, collapse

| Criterion | Evidence | Status |
|---|---|---|
| Distinct typed quantities `Point3/Direction3/Vector3/Pose3/Transform3/Twist3` | `crates/drm-spatial/src/types.rs`; `type_semantics` (I-S1, REQ-SPATIAL-036) | PASS (M19W1) |
| Rigid `SE(3)` only (translation + unit quaternion) | `crates/drm-spatial/src/transform.rs`; oracle compose/rotate (REQ-SPATIAL-038) | PASS (M19W1) |
| Compile-time `space:` binding; raw-numerics payload | `parser/spatial.rs`; `payload_layout` (I-S2, REQ-SPATIAL-005) | PASS (M19W1) |
| Dynamic `SpatialId` + `Stamped<T>` composition | `types.rs`; grammar dynamic form (REQ-SPATIAL-009/035) | PASS (M19W1) |
| WGS84→ECEF→ENU geodetic chain (small-offset local frame) | `crates/drm-spatial/src/geodetic.rs`; oracle geodetic cases + `spatial_geodetic_precision` (I-S5, REQ-SPATIAL-012) | PASS (M19W1–M19W3) |
| Canonical conventions; explicit NED/OpenCV/Unity/Unreal boundaries | `crates/drm-spatial/src/conventions.rs`; `conventions` suite + convention oracle (I-S4, REQ-SPATIAL-007/040) | PASS (M19W3) |
| Forests: disconnected trees ⇒ `PathNotFound`; explicit bridges; common ancestor | `crates/drm-spatial/src/forest.rs`; `forest` suite (I-S3, REQ-SPATIAL-010/039) | PASS (M19W3) |
| Historical evaluation at the query sample timestamp | `crates/drm-spatial/src/historical.rs`; `historical` suite (I-S9, REQ-SPATIAL-029) | PASS (M19W3) |
| Four clock domains; affine mapping; identity epoch; unresolved rejected; drift/uncertainty propagation | `crates/drm-spatial/src/clock.rs`; `clocks` suite + `clock` oracle (REQ-SPATIAL-004/041/051) | PASS (M19W1–M19W2) |
| `spaces/` path identity + static/dynamic inference | `crates/drm-workspace/src/spatial/`; `spatial_collapse` (I-D6, REQ-SPATIAL-031/047) | PASS (M19W1) |
| Static chains collapse to one build-time constant; MCU emission | `crates/drm-workspace/src/spatial/`; `spatial_collapse` (REQ-SPATIAL-049/017) | PASS (M19W1) |
| Versioned dynamic-edge cache; discontinuous flag; single-writer edges | `crates/drm-spatial/src/cache.rs`; `cache` suite (REQ-SPATIAL-050/035/016/030) | PASS (M19W1–M19W2) |
| Compiled plan execution + per-process `PlanCache` geometry shortcircuit | `crates/drm-spatial/src/plan.rs`; `spatial_cache_shortcircuit`/`spatial_cache_aging` (I-Q1/I-Q2, REQ-SPATIAL-034/002) | PASS (M19W2) |
| Coherent `latest`, `at`, `predict` (exp-map), `delayed_view` | `crates/drm-spatial/src/query.rs` + `src/history.rs` + `src/transform.rs` (`exp_twist`); `queries` suite + `query_oracle.py` (I-Q1..I-Q6, REQ-SPATIAL-022/023/024/025) | PASS (M20W1) |
| Independent mathematical reference implementation | `tests/oracle/spatial_oracle.py`, `tests/oracle/clock_oracle.py`, `tests/oracle/conventions_oracle.py`, `tests/oracle/query_oracle.py` (NumPy); `math`/`clocks`/`conventions`/`queries` suites | PASS (M19W1–M20W1) |
| Incremental dynamic hash grid; compiled BVH; proximity completeness | `crates/drm-spatial/src/grid.rs`, `src/proximity.rs`; `crates/drm-workspace/src/spatial/bvh.rs`; `grid`/`proximity` suites (I-X1..I-X14, REQ-SPATIAL-001/006/013/014/019/020/042/043/044/045/046) | PASS (M21W1–M21W2) |
| Historical reparenting (Preserve World/Local/Snap, atomic commit); live calibration; generational overlay | `crates/drm-spatial/src/query.rs` (`ReparentMode`, `reparent`, time-scoped `PlanVersion`, `CalibrationSlot`/`update_calibration_at`) + `src/forest.rs` (`TreeTxn`, `add_calibration`) + `src/handle.rs` (`HandleRegistry`) + `src/writer.rs` (`EdgeAuthority`); `reparent`/`calibration` suites + `query_oracle.py` (I-Q7..I-Q12, I-S11/I-S12, REQ-SPATIAL-003/011/016/027/028/029/030) | PASS (M20W2–M20W3) |
| Gate B — transform status object; provable history; slot coherence; zero-alloc query; bounded overlay plans | `crates/drm-spatial/src/plan.rs` (`TransformResult`, `QualityFlags`, `OverlayPlanCache`, `compile_overlay_plan`) + `src/forest.rs` (`add_overlay`/`overlay_depth`/`overlay_attachment`) + `src/history.rs` (`TimeHistory` sizing/order) + `src/cache.rs` (`EdgeSlot` seqlock); `history`/`slot`/`overlay`/`query_zero_alloc` suites (I-Q13..I-Q16, REQ-SPATIAL-008/032/033/034/037/050) | PASS (M20W4) |
| Exact spatial memory verification | `crates/drm-workspace/src/budget.rs` (`spatial_index_cost`); `crates/drm-workspace/tests/spatial_memory.rs` (I-X9/I-MB9, REQ-SPATIAL-018/052) | PASS (M21W2) |
| MCU/host reference suite; float-determinism banding | `bindings/c/mcu/drm_spatial.{h,c}`; `crates/drm-workspace/tests/mcu.rs`; `crates/drm-spatial/src/tolerance.rs`; `tests/float_determinism.rs` (I-S13, REQ-SPATIAL-017/049/053) | PASS (M21W3–M21W4) |

## Notes

- No architecture divergence in wave A: pinned invariants only; no ADR.
- `drm-spatial` is layer `runtime` and depends only on `drm-types`
  (codebase.md 2.1); `drm-workspace` stays toolchain-only. The runtime
  `SpaceForest`/`HistoricalForest` are the flattened runtime counterpart of the
  build-time `drm-workspace` spatial model.
- Regression permanence (AGENTS #14): the M19W1–M20W4 spatial suites
  (`math`/`clocks`/`conventions`/`forest`/`cache`/`queries`/`reparent`/
  `calibration`/`history`/`slot`/`overlay`/`query_zero_alloc`) are blocking
  gates.

REQ: REQ-SPATIAL-002, REQ-SPATIAL-003, REQ-SPATIAL-004, REQ-SPATIAL-005,
REQ-SPATIAL-007, REQ-SPATIAL-008, REQ-SPATIAL-009, REQ-SPATIAL-010,
REQ-SPATIAL-012, REQ-SPATIAL-016, REQ-SPATIAL-017, REQ-SPATIAL-022,
REQ-SPATIAL-023, REQ-SPATIAL-024, REQ-SPATIAL-025, REQ-SPATIAL-027,
REQ-SPATIAL-028, REQ-SPATIAL-029, REQ-SPATIAL-030, REQ-SPATIAL-031,
REQ-SPATIAL-032, REQ-SPATIAL-033, REQ-SPATIAL-034, REQ-SPATIAL-035,
REQ-SPATIAL-036, REQ-SPATIAL-037, REQ-SPATIAL-038, REQ-SPATIAL-039,
REQ-SPATIAL-040, REQ-SPATIAL-041, REQ-SPATIAL-047, REQ-SPATIAL-048,
REQ-SPATIAL-049, REQ-SPATIAL-050, REQ-SPATIAL-051.
