# Metrics — Month 20 (M20)

Normative source: [work-plan.md](../../work-plan.md) W7 DRM Spatial (Phase 33).
Month 20 implements the query API over the compiled spatial plan: the four
operations `latest`/`at`/`predict`/`delayed_view`, sample-time-ordered history
with the interpolation and discontinuity guard, and live calibration. One
section per weekly tranche.

## M20W1 — Query API Implementation (latest/at/predict/delayed_view)

| Variable | Target | Measured |
|---|---|---|
| `QUERY_API_GREEN` | latest/at/predict/delayed_view vs oracle (I-Q1..I-Q6) | 1 — `crates/drm-spatial/src/query.rs` (`QueryEngine::latest`/`at`/`predict`/`delayed_view`), `crates/drm-spatial/src/history.rs` (`TimeHistory::interpolate`), `crates/drm-spatial/src/transform.rs` (`exp_twist`); `cargo test -p drm-spatial queries` compares interpolation and SE(3) exp-map against `tests/oracle/query_oracle.py` (80 interp + 16 predict cases) and exercises coherent `latest`, bracket `at`, bounded `predict`, and `delayed_view` |
| `PLAN_EXECUTION` | runtime = compiled plan, zero traversal (I-Q1) | 1 — `crates/drm-spatial/src/plan.rs` (`compile_plan`, `CompiledPlan`, `execute_steps`); `queries_plan_execution` asserts `compile_count == 1` across repeated `latest`/`at` queries (graph traversal at compile time only) |
| `EFFORT_DAYS_ACTUAL` | 5 per engineer | on plan |

Deliverables:

- **Runtime plan compiler** (`crates/drm-spatial/src/plan.rs`): `compile_plan`
  folds a `SpaceForest` path into constant / dynamic-edge steps plus
  `Multiply`; `CompiledPlan` owns the steps; `execute_steps` is the single
  fixed-stack executor shared with the generated `TransformPlan` (`I-Q1`,
  `REQ-SPATIAL-034`). `PlanStep::CalibrationSlot` carries the nominal transform
  so `Effective = Nominal × Correction` composes at runtime (`REQ-SPATIAL-003`).
- **History rings** (`crates/drm-spatial/src/history.rs`): `TimeHistory` is
  ordered by sample time, enforces `N >= floor(H/P) + 2` (rejecting
  unsatisfiable declarations), counts `DensityViolation` and `LateSampleDropped`,
  and `interpolate` blends translation linearly and rotation via SLERP, never
  extrapolating (`REQ-SPATIAL-033`/`037`/`022`).
- **Query engine** (`crates/drm-spatial/src/query.rs`): `QueryEngine` holds the
  forest, versioned edge caches, per-edge histories, a live calibration table,
  and a per-process compiled-plan cache. `latest` evaluates every input at one
  common coherent `t* = min_i(t_newest,i)` (`I-Q3`); `at` interpolates and
  guards discontinuities (`I-Q4`/`I-Q7`); `predict` integrates the declared
  body-frame twist on `SE(3)` within `max_horizon` (`I-Q5`); `delayed_view`
  resolves `at(now − offset)` (`I-Q6`); cached geometry is reused only under an
  exact input-version match (`I-Q2`).
- **SE(3) exponential map** (`crates/drm-spatial/src/transform.rs`):
  `exp_twist(ξ, dt) = Exp(ξ·dt)` with the left Jacobian `V(ω·dt)`
  (`REQ-SPATIAL-025`).
- **E4 query oracle** (`tests/oracle/query_oracle.py`,
  `tests/fixtures/spatial/query_vectors.json`): independent NumPy interpolation,
  bracket classification, and SE(3) exp-map.
- **Docs / evidence**: [m20w1.md](../gates/m20w1.md),
  [phase-33.md](../gates/phase-33.md); registry rows promoted to `tested`.

### Weekly gate checklist (chaired by E4, Friday)

- [x] `QUERY_API_GREEN = 1`; `PLAN_EXECUTION = 1`
- [x] Variables measured; PR hygiene (no plan edits)
- **`GATE_STATUS(M20W1) = PASS`**

### How to reproduce

```text
python3 tests/oracle/query_oracle.py
cargo test -p drm-spatial queries
cargo test -p drm-spatial
cargo xtask fmt --check
cargo xtask lint
cargo clippy --workspace --target x86_64-unknown-linux-gnu --all-targets -- -D warnings
cargo xtask test
cargo xtask check-layering
cargo xtask audit-traceability
```

## Carry-over

- Estimated-twist derivation and adjoint-based twist-frame conversion for
  `predict` ⇒ M20W2 (E1/E2).
- Full transactional reparenting (Preserve World/Local/Snap, atomic commit) and
  the relocalize/reparent/calibration-jump discontinuity fixture matrix ⇒ M20W2
  (E2), building on the `I-Q7` interpolation guard.

REQ: REQ-SPATIAL-002, REQ-SPATIAL-003, REQ-SPATIAL-016, REQ-SPATIAL-022,
REQ-SPATIAL-023, REQ-SPATIAL-024, REQ-SPATIAL-025, REQ-SPATIAL-033,
REQ-SPATIAL-034, REQ-SPATIAL-037.
