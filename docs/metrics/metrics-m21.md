# Metrics — Month 21 (M21)

Normative source: [work-plan.md](../../work-plan.md) W7 DRM Spatial (Phase 33).
Month 21 implements the runtime spatial index (fixed-capacity incremental hash
grid + compiled static BVH + two-source proximity union), the host/MCU
mathematical reference suites, exact spatial memory verification, and the
reference-suite consolidation with the float-determinism evidence rule. One
section per weekly tranche.

## M21W1 — Hash Grid, Compiled BVH

| Variable | Target | Measured |
|---|---|---|
| `GRID_GREEN` | upsert/query + motion slack + generation guards + epoch-guarded cell edits (I-X1..I-X4, I-X11..I-X14) | 1 — `crates/drm-spatial/src/grid.rs`; `crates/drm-spatial/tests/grid.rs` (`grid`, `insert_query_remove`, `capacity_refused`, `spatial_motion_slack`, `spatial_grid_generation`, `spatial_query_concurrent`, `spatial_huge_radius`, `spatial_numeric_grid`) |
| `BVH_COMPILED` | deterministic compile + AABB correctness | 1 — `crates/drm-workspace/src/spatial/bvh.rs`; `crates/drm-workspace/tests/bvh.rs` (`bvh_aabb_reference`, `bvh_deterministic_compile`, `live_calibration_swap`) |
| `EFFORT_DAYS_ACTUAL` | 5 per engineer | 5 |

Deliverables:

- **Fixed-capacity incremental hash grid** (`crates/drm-spatial/src/grid.rs`):
  activates from a declared `GridConfig`, sizes the cell table to `⌈M/α_max⌉`
  (`α_max = 0.70`, power-of-two mask probing), backward-shift deletion with no
  tombstones; `on_sample` is dependency-gated and rebuckets only on envelope
  exit under an odd grid epoch restored even on every path (`I-X1`..`I-X4`,
  `REQ-SPATIAL-013`/`019`/`042`/`043`).
- **Compiled static BVH** (`crates/drm-workspace/src/spatial/bvh.rs`):
  deterministic SAH-split fixed-layout tree with two-bank live calibration
  slots (`REQ-SPATIAL-006`/`001`/`044`).
- **Verify + budget** (`crates/drm-workspace/src/verify/engine.rs`,
  `src/budget.rs`): `V-SPAT` rejects an over-span slack envelope and an
  over-budget `index.drm` (`REQ-SPATIAL-052`/`018`).

### Weekly gate checklist (chaired by E2, Friday)

- [x] `GRID_GREEN = 1`; `BVH_COMPILED = 1`
- [x] Variables measured; PR hygiene
- **`GATE_STATUS(M21W1) = PASS`**

### How to reproduce

```text
cargo test -p drm-spatial --test grid
cargo test -p drm-workspace --test bvh
cargo xtask fmt --check && cargo xtask lint
cargo clippy --workspace --target x86_64-unknown-linux-gnu --all-targets -- -D warnings
cargo xtask test && cargo xtask check-layering && cargo xtask audit-traceability
```

## M21W2 — Proximity Completeness, Spatial Memory Verification

| Variable | Target | Measured |
|---|---|---|
| `PROX_COMPLETE` | 0 misses over 10k random scenes (I-X6) | 1 (0 misses, 10k scenes) — `crates/drm-spatial/src/proximity.rs`; `crates/drm-spatial/tests/proximity.rs` (`spatial_proximity_complete`, `proximity_false_positive_factor`) |
| `SPATIAL_MEM_VERIFIED` | spatial index memory in the budget math; `index.drm` capacities enforced (I-X9/I-MB9) | 1 — `crates/drm-workspace/src/budget.rs` (`spatial_index_cost`); `crates/drm-workspace/tests/spatial_memory.rs` (`spatial_index_memory_verified`, `spatial_memory_budget_row`) |
| `EFFORT_DAYS_ACTUAL` | 5 per engineer | 5 |

Deliverables:

- **Two-source proximity union** (`crates/drm-spatial/src/proximity.rs`):
  `ProximityIndex` unions grid superset cells and static AABB overlap in the
  anchor frame, deduplicated; non-anchor typed frames are rejected through the
  standard path; the grid epoch is re-read after both scans (`I-X5`/`I-X6`/
  `I-X11`, `REQ-SPATIAL-020`/`045`/`046`).
- **Completeness property** (`crates/drm-spatial/tests/proximity.rs`): a
  deterministic scene generator with a brute-force oracle proves zero misses
  over 10k random scenes (`REQ-SPATIAL-020`).
- **Exact memory verification** (`crates/drm-workspace/src/budget.rs`):
  `spatial_index_cost` folds cell-hash slots, membership pool, object records,
  BVH nodes, and query scratch under `I-MB9` (`REQ-SPATIAL-018`/`052`).

### Weekly gate checklist (chaired by E1, Friday)

- [x] `PROX_COMPLETE = 1`; `SPATIAL_MEM_VERIFIED = 1`
- [x] Variables measured; PR hygiene
- **`GATE_STATUS(M21W2) = PASS`**

### How to reproduce

```text
cargo test -p drm-spatial --test proximity
cargo test -p drm-workspace --test spatial_memory
cargo xtask fmt --check && cargo xtask lint
cargo clippy --workspace --target x86_64-unknown-linux-gnu --all-targets -- -D warnings
cargo xtask test && cargo xtask check-layering && cargo xtask audit-traceability
```

## M21W3 — MCU/Host Paths, Reference Suite

| Variable | Target | Measured |
|---|---|---|
| `SPATIAL_HOST_SUITE` | full reference suite green on host | 1 (types, math, geodetic, clocks, plans, index) — `crates/drm-spatial/tests/host_suite.rs` (`spatial_host_reference_suite`) |
| `SPATIAL_MCU_SUITE` | MCU subset (rigid ops, pre-calculated constants, grid-free queries) green vs references | 1 (host parity + Rust oracle) — `bindings/c/mcu/drm_spatial.{h,c}`; `crates/drm-workspace/tests/mcu.rs` (`spatial_mcu_constant`) |
| `EFFORT_DAYS_ACTUAL` | 5 per engineer | 5 |

Deliverables:

- **Freestanding MCU spatial subset** (`bindings/c/mcu/drm_spatial.{h,c}`):
  fixed-size `drm_spatial_transform`, bounded `drm_spatial_compose`,
  rotation-only `drm_spatial_apply_direction`/`_vector`,
  `drm_spatial_apply_point`, and `drm_spatial_apply_dynamic` (one straight-line
  multiply of a collapsed constant against a dynamic edge, `I-S13`); no heap, no
  recursion, statically bounded loops (codebase §3.3, I-SR5,
  `REQ-SPATIAL-017`/`049`).
- **MCU artifact emission** (`crates/drm-workspace/src/spatial/mod.rs`,
  `emit_mcu_constants`): collapsed constants plus the C subset; cross-compiled
  for the pinned targets, host-compiled for parity (`REQ-CORE-087`).
- **Host reference suite consolidation**
  (`crates/drm-spatial/tests/host_suite.rs`) and the multi-round occupancy
  campaign (`spatial_hash_occupancy`) + rejected-structures audit
  (`spatial_rejected_structures`, I-X10).
- **ADR** (`docs/adr/0006-linear-probe-deletion-knuth-r.md`).

### Weekly gate checklist (chaired by E2, Friday)

- [x] `SPATIAL_HOST_SUITE = 1`; `SPATIAL_MCU_SUITE = 1`
- [x] Variables measured; PR hygiene
- **`GATE_STATUS(M21W3) = PASS`**

### How to reproduce

```text
cargo test -p drm-spatial
cargo test -p drm-workspace --test mcu
make -C bindings/c/mcu host-test
make -C bindings/c/mcu host-audit
cargo xtask fmt --check && cargo xtask lint
cargo clippy --workspace --target x86_64-unknown-linux-gnu --all-targets -- -D warnings
cargo xtask test && cargo xtask check-layering && cargo xtask audit-traceability
```

## M21W4 — Spatial Reference Suite Complete, Final Gate Prep

| Variable | Target | Measured |
|---|---|---|
| `REF_SUITE_COMPLETE` | all spatial suites vs mathematical references green together | 1 — `cargo test -p drm-spatial`; `cargo test -p drm-workspace --test mcu --test spatial_collapse --test bvh --test spatial_memory`; [m21w4.md](../gates/m21w4.md) §Suite runs |
| `GATE_FINAL_DOC` | Phase 33 final gate doc drafted | drafted — [m21w4.md](../gates/m21w4.md) (criteria → artifact → test → result for all three spatial §Test Matrices) |
| `EFFORT_DAYS_ACTUAL` | 5 per engineer | 5 |

Deliverables:

- **Float-determinism banding** (`crates/drm-spatial/src/tolerance.rs`,
  `ToleranceBand::SPATIAL_REFERENCE`): one combined absolute + relative band
  (`abs = 1e-12`, `rel = 1e-9`) is the single comparison rule for every spatial
  reference suite, absorbing fused-multiply-add vs separate `mul`+`add`
  differences across host toolchains and MCU targets; never bit equality
  (`REQ-SPATIAL-053`).
- **Float-determinism reference test**
  (`crates/drm-spatial/tests/float_determinism.rs`, `spatial_float_determinism`):
  FMA vs separate point rotation and quaternion composition across magnitudes;
  the measured spread stays well inside the band while a perturbed result is
  rejected.
- **Spatial-under-router soak regression**
  (`crates/drm-spatial/tests/soak_regression.rs`, `spatial_under_router_soak`):
  sustained router egress across three topology-shaped links interleaved with
  index rebuckets and proximity queries; 0 misses versus brute force and exact
  router emission accounting (`I-X6`/`I-X11`, `REQ-SPATIAL-020`/`042`/`043`).
- **Phase 33 final gate draft** ([m21w4.md](../gates/m21w4.md)) and the
  algorithm-doc section
  ([spatial-core.md](../algorithms/spatial/spatial-core.md)
  §Reference-Suite Float Determinism, §Implementation Status M21W4);
  `REQ-SPATIAL-053` added to the registry.

### Weekly gate checklist (chaired by E3, Friday)

- [x] `REF_SUITE_COMPLETE = 1`; `GATE_FINAL_DOC = drafted`
- [x] Variables measured; PR hygiene
- **`GATE_STATUS(M21W4) = PASS`**

### How to reproduce

```text
cargo test -p drm-spatial
cargo test -p drm-spatial --test float_determinism
cargo test -p drm-spatial --test soak_regression
cargo test -p drm-workspace --test mcu --test spatial_collapse --test bvh --test spatial_memory
cargo xtask fmt --check && cargo xtask lint
cargo clippy --workspace --target x86_64-unknown-linux-gnu --all-targets -- -D warnings
cargo xtask test && cargo xtask check-layering && cargo xtask audit-traceability
```

## Carry-over

- Final Phase 33 gate at M22W2 (chair E1, presenter E2).
- Hash-grid occupancy campaign and I-X10 audit closed (M21W3); no open
  spatial-index carry-over.

REQ: REQ-SPATIAL-001, REQ-SPATIAL-006, REQ-SPATIAL-013, REQ-SPATIAL-017,
REQ-SPATIAL-018, REQ-SPATIAL-019, REQ-SPATIAL-020, REQ-SPATIAL-042,
REQ-SPATIAL-043, REQ-SPATIAL-044, REQ-SPATIAL-045, REQ-SPATIAL-046,
REQ-SPATIAL-049, REQ-SPATIAL-052, REQ-SPATIAL-053.
