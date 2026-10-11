# Month-20 Review — Phase 33 Gate B, Spatial Query Surface

Chaired by E3 (review chair) at M20W4, Friday. Verdict: **ON_TRACK** — month 20
closed Phase 33 Gate B (the compiled-plan query surface: `latest`/`at`/`predict`/
`delayed_view`, the transform result object with hard errors + combinable quality
flags, the interpolation discontinuity guard, transactional temporal reparenting,
live calibration slots, generational overlay handles, single-writer edges, and
bounded generation-cached overlay plans). Every `I-Q1`..`I-Q16` invariant is
evidenced; the wave is go for M21 (dynamic hash grid + compiled BVH +
proximity) (`MONTH20_REVIEW = ON_TRACK`).

Normative sources: [work-plan.md](../../work-plan.md) W7 Phase 33;
[spatial_architecture.md](../../docs/spatial_architecture.md);
[spatial-queries.md](../algorithms/spatial/spatial-queries.md),
[spatial-core.md](../algorithms/spatial/spatial-core.md),
[spatial-index.md](../algorithms/spatial/spatial-index.md) (M21 target).
REQ: REQ-SPATIAL-002, REQ-SPATIAL-003, REQ-SPATIAL-008, REQ-SPATIAL-011,
REQ-SPATIAL-016, REQ-SPATIAL-022, REQ-SPATIAL-023, REQ-SPATIAL-024,
REQ-SPATIAL-025, REQ-SPATIAL-027, REQ-SPATIAL-028, REQ-SPATIAL-029,
REQ-SPATIAL-030, REQ-SPATIAL-032, REQ-SPATIAL-033, REQ-SPATIAL-034,
REQ-SPATIAL-037, REQ-SPATIAL-050.

## 1. Week variables vs targets (M20)

| Week | Variable | Target | Measured |
|---|---|---|---|
| M20W1 | `QUERY_API_GREEN` | `latest`/`at`/`predict`/`delayed_view` vs oracle (`I-Q1`..`I-Q6`) | 1 — [m20w1.md](m20w1.md) |
| M20W1 | `PLAN_EXECUTION` | runtime = compiled plan, zero traversal (`I-Q1`) | 1 — [m20w1.md](m20w1.md) |
| M20W2 | `INTERP_GUARD_GREEN` | no interpolation across a discontinuity (`I-Q7`) | 1 — [m20w2.md](m20w2.md) |
| M20W2 | `REPARENT_TEMPORAL`/`REPARENT_MODES`/`REPARENT_ATOMIC` | `I-Q8`/`I-Q9`/`I-Q10` | 1 — [m20w2.md](m20w2.md) |
| M20W3 | `CALIBRATION_GREEN` | live slot update ⇒ cache invalidation (`I-Q11`) | 1 — [m20w3.md](m20w3.md) |
| M20W3 | `GENERATIONAL_GREEN` | overlay handle invalidation immediate (`I-Q12`) | 1 — [m20w3.md](m20w3.md) |
| M20W3 | `SINGLE_WRITER_GREEN` | second writer refused (`I-S11`) | 1 — [m20w3.md](m20w3.md) |
| M20W4 | `GATE_B_STATUS` | PASS — `I-Q1`..`I-Q16` evidenced | PASS — [m20w4.md](m20w4.md) |
| M20W4 | `MONTH20_REVIEW` | ON_TRACK | ON_TRACK (this document) |
| M20W4 | `EFFORT_DAYS_ACTUAL` | 5 per engineer | on plan |

## 2. Gate B ledger

| Gate | Source | Verdict |
|---|---|---|
| Compiled-plan execution (`I-Q1`), geometry-only cache (`I-Q2`) | [m20w1.md](m20w1.md) | PASS |
| Coherent `latest`/`at`/`predict`/`delayed_view` (`I-Q3`..`I-Q6`) | [m20w1.md](m20w1.md) | PASS |
| Interpolation guard + temporal reparenting (`I-Q7`..`I-Q10`) | [m20w2.md](m20w2.md) | PASS |
| Live calibration + generational handles + single-writer (`I-Q11`, `I-Q12`, `I-S11`) | [m20w3.md](m20w3.md) | PASS |
| Provable history (`I-Q13`), slot coherence + model (`I-Q14`), zero-alloc query (`I-Q15`), bounded overlay plans (`I-Q16`) | [m20w4.md](m20w4.md) | PASS |

## 3. M21 commitment

- **M21W1 (E2)**: write [spatial-index.md](../algorithms/spatial/spatial-index.md)
  before implementation (already the normative doc; the M21W1 task finalizes and
  locks it against the architecture).
- **M21 (E1/E2)**: implement the incremental dynamic hash grid (dependency-vector
  invalidation, grid epoch), the compiled static BVH with dual-bank live
  calibration swaps, and proximity candidate completeness (explicit
  `OutputTooSmall`, never silent truncation); exact spatial memory verification.
- **M21 spike plan (from M20W4 SPIKE)**: falsifier #1 (multi-edge rotated
  composition order) first — rotated 2-edge oracle fixtures + a convention
  decision, with an ADR if a change is required; then the async-rate coherence,
  reparent-race, cache-aging, micro-discontinuity, handle-ABA, and deep-overlay
  probes.

## 4. Risks

| Risk | Mitigation | Status |
|---|---|---|
| Multi-edge rotation composition order unverified | recorded as falsifier #1; M21 oracle fixtures + possible ADR | contained |
| Spatial index scope (grid + BVH + memory verification) is the largest remaining subsystem | M21 starts immediately; algorithm doc locked M21W1 before impl | contained |
| `predict` estimated-twist derivation open | `predict` requires an explicit declared twist today; estimated derivation folded into the M21 spike plan | contained |

## 5. Carry-over

| Item | Origin | Disposition |
|---|---|---|
| Multi-edge compiled-plan composition order (rotation-sensitive) | [m20w2.md](m20w2.md) | reviewed at M20W4 (no divergence from the `compile_plan` pseudocode ⇒ no ADR); falsifier #1 for M21 |
| `predict` adjoint-based twist-frame conversion + estimated-twist derivation | [m20w1.md](m20w1.md) | M21 spike plan |
| Dynamic hash grid, compiled BVH, exact spatial memory verification | [phase-33.md](phase-33.md) | M21–M22 |

REQ: REQ-SPATIAL-002, REQ-SPATIAL-003, REQ-SPATIAL-008, REQ-SPATIAL-011,
REQ-SPATIAL-016, REQ-SPATIAL-022, REQ-SPATIAL-023, REQ-SPATIAL-024,
REQ-SPATIAL-025, REQ-SPATIAL-027, REQ-SPATIAL-028, REQ-SPATIAL-029,
REQ-SPATIAL-030, REQ-SPATIAL-032, REQ-SPATIAL-033, REQ-SPATIAL-034,
REQ-SPATIAL-037, REQ-SPATIAL-050.
