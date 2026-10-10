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
