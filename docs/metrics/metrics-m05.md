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

## M05W3 — Phase 15 Gate Prep, Publisher Lifecycle, Serial Adapter

| Variable | Target | Measured |
|---|---|---|
| `EXHAUSTION_E2E` | exhaustion under real borrow patterns sustained | exhaustion suites + hazard-protected channel tests green |
| `LIFECYCLE_UNITS` | authority race + epoch tests green (I-L1..L5) | `cargo test -p drm-core lifecycle`: 16 passed |
| `RESTART_100` | restart ×100 epoch exact | `publisher_restart_loop_100` (in-memory lifecycle path; epoch +1 exactly, floor preserved) |
| `FRESHNESS_FIXTURES` | state transitions per timing contracts | `consumer_observes_epoch_reset`, publisher reset tests; full consumer freshness subsystem remains Phase 17 scope |
| `SERIAL_LOOPBACK` | bounded frame round-trip ×10k | `drm-router::transport::serial` (7 tests, deterministic 10k escape/loopback coverage) |
| `EFFORT_DAYS_ACTUAL` | 5/eng | on plan |

Deliverables: lifecycle coverage (`drm-core::channel::lifecycle`); bounded
COBS+CRC32C best-effort serial codec (`drm-router::transport::serial`);
`docs/gates/phase-15.md` draft.

## M05W4 — Phase 15 Gate (Exhaustion), Deployment Identity Start

| Variable | Target | Measured |
|---|---|---|
| `GATE_STATUS(Phase 15)` | PASS | PASS — `docs/gates/phase-15.md` |
| `IDENTITY_UNITS` | guard + namespace + cleanup green (I-DP1/I-DP2/I-DP5) | `drm-core` `identity` (guard refuses before SHM touch; namespaces cannot cross-open), `clean` (active hold ⇒ `ActiveUntouched`; inactive ⇒ `Cleaned`, recreatable) |
| `CROSS_DEPLOY` | two deployments concurrently, zero cross-attachment | `drm-router` `cross_deploy_same_topic_disjoint_keys` (two ids, same slot ⇒ disjoint `/drm.<hex-id>.<slot>` keys) |
| `EFFORT_DAYS_ACTUAL` | 5/eng | on plan |

Deliverables: `drm-core::deploy` (fingerprint guard, deployment-namespaced
segment names, `clean` decision tree) + `drm-router::deployment`
(`DeploymentNamespace` reusing `drm_core::deploy::DeploymentId`).
