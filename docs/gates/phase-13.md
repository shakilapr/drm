# Phase 13 Gate — Bulk Payload Pool

Evidence mapping, [work-plan.md](../../work-plan.md) #13 criteria ⇄ tests.
Implementation: `drm-core::channel::pool`
([payload-pool.md](../algorithms/runtime/payload-pool.md)), consumed by the
channel publish path ([channel-publish.md](../algorithms/runtime/channel-publish.md)).
Verdict: **PASS**.

## Criteria ⇄ evidence

| Criterion | Evidence | Status |
|---|---|---|
| Maximum, empty and all intermediate sizes operate correctly | `pool_size_reject` (max accepted, `max+1` rejected, empty accepted), `pool_size_reject_sentinel` (channel-level: oversize leaves the newest sample intact) | PASS |
| Slot offsets never relocate | `pool_offsets` (`base + i*stride`, stable across re-derivation — I-P1), `pool_offsets_static` (per-topic instances independent across remap) | PASS |
| Topic pools cannot borrow from one another | `pool_isolation` (a write into one pool never mutates another's bytes — I-P2) | PASS |
| Attempts beyond the structural maximum rejected before memory overwrite | `pool_size_reject` (sentinel bytes unchanged after a rejected `max+1` write — I-P3), `pool_size_reject_sentinel` | PASS |

## Invariant coverage (payload-pool.md I-P1–I-P5)

| Invariant | Test |
|---|---|
| I-P1 OffsetsStatic | `pool_offsets`, `pool_offsets_static` |
| I-P2 PoolIsolation | `pool_isolation` |
| I-P3 RejectBeforeWrite | `pool_size_reject`, `pool_size_reject_sentinel` |
| I-P4 PoolFloor | `pool_invariant_min` (`pool_size < R + 1` refused at activation) |
| I-P5 DropFree | beyond the M04W3/P14 hazard scope; proven with the acquisition storm (`hazard_storm`) |

## Variables (M04W3)

| Variable | Target | Measured |
|---|---|---|
| `GATE_STATUS(Phase 13)` | PASS | PASS |
| `HAZARD_ACQUIRE_GREEN` | 6-step acquisition single-threaded semantics green | `drm-core::hazard` unit tests (`cargo test -p drm-core hazard`) |
| `SELECTION_PROPERTY` | `select_slot` never returns referenced/protected slot | hazard selection property test |
| `SIZE_REJECTION` | I-P3 max/max+1 rejected pre-write | `pool_size_reject`, `pool_size_reject_sentinel` |
| `LANE_SOCKETS` | per-peer lane connect/disconnect green (Phase 30 groundwork) | `drm-router` `lane` connect/disconnect tests |
| `EFFORT_DAYS_ACTUAL` | 5 per engineer | on plan |

## Requirements covered

REQ-CORE-043/044 (dynamic pool count, static offsets), REQ-CORE-046 (per-topic
pool isolation), REQ-CORE-041 (latest default pool size), REQ-CORE-048 (reject
before write), REQ-CORE-004/031 (hazard slot never overwritten; ABA generation
guard).

## Notes

- The drop-free proof (`pool_size ≥ R + H + 1` ⇒ a free slot always exists)
  completes with the hazard protocol; the storm harness
  (`tests/stress/hazard_storm.rs`) is the continuous-check scaffold.
- Hazard acquisition steps 1–6 mirror architecture §Lockless Consumer
  Acquisition Protocol verbatim (spec review gate, M04W3-E2-T2).
