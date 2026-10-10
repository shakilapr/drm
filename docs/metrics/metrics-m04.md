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
