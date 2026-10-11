# Metrics — Month 04 (M04)

Normative source: [work-plan.md](../../work-plan.md) W2 Core Runtime. One
section per weekly tranche; variables match the week plan.

## M04W1 — Phase 11 Gate (Cursors), Overflow Start, TCP Adapter

| Variable | Target | Measured |
|---|---|---|
| `GATE_STATUS(Phase 11)` | PASS | PASS — `docs/gates/phase-11.md` |
| `OVERWRITE_GAPS` | multi-lap fixtures exact (I-O1) | `overflow_laps` (laps 1/2/R/R+1, `missed == lap`); `gap_exact_laps` k ∈ 1..=32 |
| `GATING_EXACT` | publish succeeds iff required consumers advanced (I-O3) | `reject_gating`; `overflow_matrix` cell 5/6 (behind ⇒ `QueueFull`; advanced ⇒ proceeds) |
| `REJECT_NOBLOCK` | `QueueFull` < 1 ms under full history (I-O2) | `reject_never_blocks` (wall-clock assertion, no sleeps) |
| `OVERFLOW_MATRIX` | all cells present | `overflow_matrix` (6 cells, pinned count) |
| `TCP_ADAPTER_BASIC` | record send/recv round-trip green | `drm-router` `transport::tcp` (partial-write matrix + loopback round-trip) |
| `EFFORT_DAYS_ACTUAL` | 5/eng | on plan |

Deliverables: `docs/gates/phase-11.md`; multi-lap overwrite fixtures +
overflow combination matrix (`drm-core::channel`); TCP stream adapter
(`drm-router::transport::tcp`).

## M04W2 — Phase 12 Gate (Overflow), Payload Pool, TCP Reconnect

| Variable | Target | Measured |
|---|---|---|
| `GATE_STATUS(Phase 12)` | PASS | PASS — `docs/gates/phase-12.md` |
| `POOL_OFFSETS_STATIC` | I-P1: offsets identical across remap/restart | `pool_offsets`, `pool_offsets_static` |
| `POOL_ISOLATION` | I-P2 negative proof | `pool_isolation` (per-topic partitions independent) |
| `SIZE_REJECTION` | I-P3 max/max+1 rejected pre-write | `pool_size_reject`, `pool_size_reject_sentinel` (sentinel bytes untouched) |
| `REJECT_NOBLOCK` | `QueueFull` < 1 ms, no block | `reject_never_blocks` |
| `GATING_EXACT` | I-O3 | `reject_gating`, `overflow_matrix` |
| `TCP_RECONNECT` | session-scoped reliable asserted | `drm-router` `tcp_reconnect` |
| `EFFORT_DAYS_ACTUAL` | 5/eng | on plan |

Deliverables: `docs/gates/phase-12.md`; named pool suites (I-P1..I-P3); TCP
reconnect / session-scoped reliability (`drm-router::transport::tcp`).

## M04W3 — Phase 13 Gate (Payload Pool), Hazard Protocol Start

| Variable | Target | Measured |
|---|---|---|
| `GATE_STATUS(Phase 13)` | PASS | PASS — `docs/gates/phase-13.md` |
| `HAZARD_ACQUIRE_GREEN` | 6-step acquisition single-threaded semantics green | `drm-core::hazard` tests (10: NONE sentinel, set/clear/CAS, acquire newest, lookup-and-busy, close-vs-acquire, intent-store Retry) |
| `SELECTION_PROPERTY` | `select_slot` never returns referenced/protected slot | `hazard_selection_property` (H=3 distinct consumers, 20 000 iterations, no sleeps) |
| `RACE_MATRIX_DESIGNED` | R1–R6 enumerated with forcing strategies | [hazard-protocol.md](../algorithms/runtime/hazard-protocol.md) race matrix; Retry test mirrors R2 (publisher roll between the two `lookup_sc` snapshots) |
| `LANE_SOCKETS` | per-peer lane connect/disconnect green | `drm-router` `lane` 9 tests (4 new: transition, best-effort/reliable bulk disposition, caps+FIFO while connected) |
| `EFFORT_DAYS_ACTUAL` | 5/eng | on plan |

Deliverables: `docs/gates/phase-13.md`; hazard slot + acquisition + selection
suites and the `hazard_storm` scaffold (`drm-core`); per-peer lane socket
lifecycle (`drm-router::lane`).

## M04W4 — Hazard Race Matrix Runtime (Phase 14 implementation)

| Variable | Target | Measured |
|---|---|---|
| `RACE_MATRIX_GREEN` | R1–R6 forcing tests green | `hazard::race` (roll-before-intent, roll-after-intent, generation-wrap, concurrent-acquire with a `Barrier`, release-during-scan, reuse-during-reread) |
| `ABA_GREEN` | generation wrap ≥ 2¹⁶ reuses validated (I-HZ2) | `hazard::aba` (`hazard_aba_reuse_wrap`: 3 slots each reused ≥ 2¹⁶×, stale `{slot,generation}` never validates; `hazard_aba_generation_no_wrap`) |
| `SELECTION_PROPERTY` | selection-vs-borrows property green | `hazard::select` (`hazard_select_returns_only_free` 5000-iter property; `hazard_select_free_exists_at_r_h_1` drop-free step) |
| `STORM_LOCAL` | 1 h storm, 0 shadow-validator violations (I-HZ1) | scaffold green at reduced cycles; 1 h run owned by the M05W2 Phase 14 completion gate |
| `LANES_E2E` | two-lane loopback with priority wiring green | `drm-router` `lane_egress` (4 tests: classification, strict class service order, two-lane exchange, disconnected send refused) |
| `EFFORT_DAYS_ACTUAL` | 5/eng | on plan |

Deliverables: `docs/gates/phase-14.md` (draft); `drm-core::hazard`
race/aba/select suites; `drm-router::egress` per-peer classification + priority
wiring. Known: `platform::cleanup` flakes rarely under load (queued for
quarantine/repair per AGENTS.md 14).
