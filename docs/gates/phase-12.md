# Phase 12 Gate — History Overflow Policies

Evidence mapping, [work-plan.md](../../work-plan.md) #12 criteria ⇄ tests.
Implementation: `drm-core::channel` overflow path
([overflow-policies.md](../algorithms/runtime/overflow-policies.md)),
payload pool ([payload-pool.md](../algorithms/runtime/payload-pool.md)).
Verdict: **PASS**.

## Criteria ⇄ evidence

| Criterion | Evidence | Status |
|---|---|---|
| Overwrite produces exact gaps | `gap_exact_laps` (k ∈ 1..=32), `overflow_laps` (laps 1/2/R/R+1 ⇒ `Gap { missed: lap }`), `overflow_matrix` cell 3 | PASS |
| Reject returns `QUEUE_FULL` without secretly blocking | `reject_never_blocks` (wall-clock `< 1 ms`, no sleeps), `reject_gating` (`QueueFull` when a required consumer lags) — I-OV2 | PASS |
| Required sequential readers correctly gate rejection | `reject_gating` (advance-past-retirement ⇒ publish proceeds), `overflow_matrix` cells 5/6 (I-OV3) | PASS |
| Publisher behavior matches specification under full history | `reject_never_blocks` + `reject_zero_required` (`Reject` with zero required ⇒ `InvalidConfig` on every publish, I-OV5) | PASS |
| Pool slot offsets static / isolation / reject-before-write | `pool_offsets` (I-P1), `pool_isolation` (I-P2), `pool_size_reject` + `pool_size_reject_sentinel` (I-P3) | PASS |

## Invariant coverage (overflow-policies.md I-O1–I-O5, payload-pool.md I-P1–I-P3)

| Invariant | Test |
|---|---|
| I-O1 OverwriteExactGap | `gap_exact_laps`, `overflow_laps` |
| I-O2 RejectNoBlock | `reject_never_blocks` |
| I-O3 GatingExact | `reject_gating`, `overflow_matrix` |
| I-O5 RejectNeedsRequired | `reject_zero_required`, `overflow_matrix` cell 4 |
| I-P1 OffsetsStatic | `pool_offsets`, `pool_offsets_static` |
| I-P2 PoolIsolation | `pool_isolation` |
| I-P3 RejectBeforeWrite | `pool_size_reject`, `pool_size_reject_sentinel` |

## Variables (M04W2)

| Variable | Target | Measured |
|---|---|---|
| `GATE_STATUS(Phase 12)` | PASS | PASS |
| `POOL_OFFSETS_STATIC` | I-P1: offsets identical across remap/restart | `pool_offsets`, `pool_offsets_static` |
| `POOL_ISOLATION` | I-P2 negative proof | `pool_isolation` |
| `SIZE_REJECTION` | I-P3 max/max+1 rejected pre-write | `pool_size_reject`, `pool_size_reject_sentinel` |
| `EFFORT_DAYS_ACTUAL` | 5 per engineer | on plan |

## Requirements covered

REQ-CORE-039 (overwrite exact gaps), REQ-CORE-040 (reject gating, never
blocks), REQ-CORE-043/044 (dynamic pool count, static offsets), REQ-CORE-046
(per-topic pool isolation), REQ-CORE-041 (latest default pool), REQ-CORE-048
(reject before write), REQ-CORE-059/060 (required registry, ack watermark).

## Notes

- Variable-slot length-header placement is pinned by
  [docs/spec/metadata-layout.md](../spec/metadata-layout.md) (R1: spec + impl
  in the same change).
- The drop-free pool-size proof (`pool_size ≥ R + H + 1`) completes with the
  hazard protocol in Phase 14; this gate proves the static-offset and
  reject-before-write half.
