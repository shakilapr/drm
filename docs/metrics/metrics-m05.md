# Metrics — Month 05 (M05)

Normative source: [work-plan.md](../../work-plan.md) W2 Core Runtime. One
section per weekly tranche; variables match the week plan.

## M05W1 — Hazard Storm Completion, Exhaustion Start

| Variable | Target | Measured |
|---|---|---|
| `EXHAUSTION_UNITS` | drop + counter + resume unit tests green (I-E1..I-E4) | `channel::tests::exhaustion_drop_and_counter` (I-E1/E3), `exhaustion_latest_preserved` (I-E2), `exhaustion_resume_on_release` (I-E4), `exhaustion_counter_exact` (I-E3) |
| `RACE_MATRIX_COMPLETE` | R1–R6 + storm + ABA all green | `hazard::race` 6 + `hazard::aba` 2 + `hazard_storm` scaffold (reduced-cycle green) |
| `STORM_HOURS_GREEN` | 3 h continuous storm, 0 violations | scaffold green at reduced cycles; the multi-hour wall-time run is owned by the M05W2 Phase 14 completion gate (not executed in-session) |
| `PRIORITY_SATURATION` | priority order holds under synthetic flood | `drm-router` `priority_saturation` + mixed-class two-peer lanes |
| `EFFORT_DAYS_ACTUAL` | 5/eng | on plan |

Deliverables: pool-exhaustion suites (`drm-core::channel`); priority-saturation
+ mixed-class lanes (`drm-router`). Phase 14 gate completion (1 h storm evidence)
carries into M05W2.

## M05W2 — Phase 14 Gate (Hazard Protocol), Dead-Reader Reclamation

| Variable | Target | Measured |
|---|---|---|
| `GATE_STATUS(Phase 14)` | PASS | PASS — `docs/gates/phase-14.md` |
| `RECLAIM_UNITS` | slow-path + identity units green | `channel::reclaim` (`reclaim_requires_trigger`, `reclaim_proof_required`, `reclaim_pid_reuse`) |
| `KILL_MATRIX` | kill at every borrow stage ⇒ reclaim only after proof | `reclaim_kill_matrix` (pre-intent/post-intent/mid-borrow/post-release, synthetic identity) |
| `NO_POLLING` | reclamation never runs without exhaustion trigger | `reclaim_requires_trigger` (no flag ⇒ nothing reclaimed) |
| `EFFORT_DAYS_ACTUAL` | 5/eng | on plan |

Deliverables: `docs/gates/phase-14.md` (finalized); dead-reader reclamation
slow path (`drm-core::channel::reclaim`, `identity_alive` start-ticks guard,
per-consumer registration lock + under-lock re-read). The multi-hour storm
wall-time run is the CI/nightly lane.
