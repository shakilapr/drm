# Metrics — Month 22 (M22)

Normative source: [work-plan.md](../../work-plan.md) W7 Phase 33 close and the
W8 qualification drive. Month 22 closes the DRM Spatial phase at the Phase 33
final gate, migrates the spatial suites into permanent CI gates, and starts the
Studio extension platform. One section per weekly tranche.

## M22W2 — Phase 33 Final Gate (DRM Spatial)

| Variable | Target | Measured |
|---|---|---|
| `GATE_STATUS(Phase 33)` | PASS — every spatial requirement evidenced | PASS — [m22w2.md](../gates/m22w2.md); `cargo test -p drm-spatial`; `cargo test -p drm-workspace --test mcu --test spatial_collapse --test bvh --test spatial_memory` |
| `EFFORT_DAYS_ACTUAL` | 5 per engineer | 5 |

Deliverables:

- **Phase 33 final gate doc** ([m22w2.md](../gates/m22w2.md)): the work-plan #33
  criteria ledger plus the three algorithm-doc §Test Matrices
  ([spatial-core.md](../algorithms/spatial/spatial-core.md) `I-S1`..`I-S13`,
  [spatial-queries.md](../algorithms/spatial/spatial-queries.md) `I-Q1`..`I-Q16`,
  [spatial-index.md](../algorithms/spatial/spatial-index.md) `I-X1`..`I-X14`),
  each mapping criterion → invariant → artifact → test → result. Every entry is
  green; `GATE_STATUS(Phase 33) = PASS` signed by E1.
- **Phase 33 gate status flip** ([phase-33.md](../gates/phase-33.md)): status
  moved from `IN PROGRESS` to `PASS`, with the M19–M22 weekly-gate roll-up.
- **Registry resolution** ([registry.csv](../req/registry.csv)): the Phase 33
  spatial rows whose evidence is green are resolved from the algorithm-doc
  test-matrix anchor to the concrete crate/`tests` path and promoted to
  `tested` (index rows `REQ-SPATIAL-001`/`006`/`013`/`014`/`015`/`018`/`019`/
  `020`/`021`/`026`/`042`/`043`/`044`/`045`/`046`/`052` and the float-determinism
  row `REQ-SPATIAL-053`).
- **Falsifier SPIKE** ([m22w2.md](../gates/m22w2.md) §SPIKE): the Phase 33
  falsification review ranks the top likely falsifiers (grid completeness at
  density extremes; BVH live subtree swaps under a calibration storm; cache
  version races; MCU constant drift, plus carry-over rotated-composition,
  generational-handle ABA, and candidate-factor perf) by likelihood × blast
  radius; the ranked list feeds the W8 qualification spike plan.
- **Suite runs**: all spatial runtime suites, the compiler/index/MCU suites, the
  two zero-allocation harnesses, the float-determinism band, and the
  spatial-under-router soak re-ran green together; full workspace tests,
  traceability, and layering green.

### Weekly gate checklist (chaired by E1, Friday)

- [x] **Phase 33 gate PASS** (every spatial requirement evidenced); signed by E1
- [x] Variables measured; PR hygiene (no plan edits)
- **`GATE_STATUS(Phase 33) = PASS`**

### How to reproduce

```text
cargo test -p drm-spatial
cargo test -p drm-spatial --test grid_zero_alloc --test query_zero_alloc
cargo test -p drm-spatial --test float_determinism
cargo test -p drm-spatial --test soak_regression
cargo test -p drm-workspace --test mcu --test spatial_collapse --test bvh --test spatial_memory
cargo xtask fmt --check && cargo xtask lint
cargo clippy --workspace --target x86_64-unknown-linux-gnu --all-targets -- -D warnings
cargo xtask test && cargo xtask check-layering && cargo xtask audit-traceability
```

## Carry-over

- **M22W3/M22W4**: Studio extension host API + E2E, and the migration of the
  spatial suites into required CI gates (`SPATIAL_REGRESSION`), per
  [work-plan-m22-w3.md](../work-plan/work-plan-m22-w3.md) and
  [work-plan-m22-w4.md](../work-plan/work-plan-m22-w4.md).
- **W8 spike plan**: the M22W2 ranked falsifiers seed the Phase 34 qualification
  spike plan; a spatial red line would open the M23W1 fix window (none found).

REQ: REQ-SPATIAL-001, REQ-SPATIAL-002, REQ-SPATIAL-003, REQ-SPATIAL-004,
REQ-SPATIAL-005, REQ-SPATIAL-006, REQ-SPATIAL-007, REQ-SPATIAL-008,
REQ-SPATIAL-009, REQ-SPATIAL-010, REQ-SPATIAL-011, REQ-SPATIAL-012,
REQ-SPATIAL-013, REQ-SPATIAL-014, REQ-SPATIAL-015, REQ-SPATIAL-016,
REQ-SPATIAL-017, REQ-SPATIAL-018, REQ-SPATIAL-019, REQ-SPATIAL-020,
REQ-SPATIAL-021, REQ-SPATIAL-022, REQ-SPATIAL-023, REQ-SPATIAL-024,
REQ-SPATIAL-025, REQ-SPATIAL-026, REQ-SPATIAL-027, REQ-SPATIAL-028,
REQ-SPATIAL-029, REQ-SPATIAL-030, REQ-SPATIAL-031, REQ-SPATIAL-032,
REQ-SPATIAL-033, REQ-SPATIAL-034, REQ-SPATIAL-035, REQ-SPATIAL-036,
REQ-SPATIAL-037, REQ-SPATIAL-038, REQ-SPATIAL-039, REQ-SPATIAL-040,
REQ-SPATIAL-041, REQ-SPATIAL-042, REQ-SPATIAL-043, REQ-SPATIAL-044,
REQ-SPATIAL-045, REQ-SPATIAL-046, REQ-SPATIAL-047, REQ-SPATIAL-048,
REQ-SPATIAL-049, REQ-SPATIAL-050, REQ-SPATIAL-051, REQ-SPATIAL-052,
REQ-SPATIAL-053.
