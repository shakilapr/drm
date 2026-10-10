# Phase 11 Gate — Reader Cursors & Delivery Semantics

Evidence mapping, [work-plan.md](../../work-plan.md) #11 criteria ⇄ tests.
Implementation: `drm-core::channel` cursors/delivery
([cursor-delivery.md](../algorithms/runtime/cursor-delivery.md)), overflow
gaps from [overflow-policies.md](../algorithms/runtime/overflow-policies.md).
Verdict: **PASS** (copy-path evidence; the borrow path (I-C5) lands with the
hazard protocol in Phase 14 and is reported there).

## Criteria ⇄ evidence

| Criterion | Evidence | Status |
|---|---|---|
| Readers at different speeds never affect each other's cursor | `channel::tests::cursor_independence` (fast/slow), `cursor_independence_lapped` (a lapped reader's `Gap` leaves a fast peer caught up) — I-C1 | PASS |
| Intentional lapping verifies exact `missed: N` | `gap_exact_laps` (k ∈ 1..=32), `overflow_laps` (laps 1, 2, R, R+1), `next_resume_oldest` (I-C2) | PASS |
| `latest()` always resolves the newest committed sample | `latest_newest` (newest committed regardless of backlog), `publish_read_roundtrip` (I-C3) | PASS |
| `next()` never silently skips retained sequences | `next_no_skip` proptest over an independently derived delivery model (400 cases); `next_resume_oldest` (I-C4) | PASS |
| `Gap` is exact under the overflow policy split | `overflow_matrix` (Overwrite/Reject × empty/not-full/full/required states) | PASS |

## Invariant coverage (cursor-delivery.md I-C1–I-C4)

| Invariant | Test |
|---|---|
| I-C1 Independent | `cursor_independence`, `cursor_independence_lapped` |
| I-C2 GapExact | `gap_exact_laps`, `overflow_laps`, `next_resume_oldest` |
| I-C3 LatestNewest | `latest_newest`, `publish_read_roundtrip` |
| I-C4 NextNoSkip | `next_no_skip` (property), `next_resume_oldest` |
| I-C5 BorrowSemantics | deferred to Phase 14 (hazard protocol) — copy-path only this gate |

## Variables (M04W1)

| Variable | Target | Measured |
|---|---|---|
| `GATE_STATUS(Phase 11)` | PASS | PASS |
| `OVERWRITE_GAPS` | multi-lap fixtures exact (I-O1) | `overflow_laps` (1/2/R/R+1) |
| `CURSOR_INDEPENDENCE` | fast/slow/lapped never interact (I-C1) | `cursor_independence`, `cursor_independence_lapped` |
| `NO_SKIP` | exactly-once unless aged out (I-C4) | `next_no_skip` proptest |
| `EFFORT_DAYS_ACTUAL` | 5 per engineer | on plan |

## Requirements covered

REQ-CORE-010/011/012/013 (cursor latest/next, independence, epoch-verified
read), REQ-CORE-028/029 (exact gap, resume at oldest retained),
REQ-CORE-039/040 (overwrite exact gaps, reject gating), REQ-CORE-096/097
(persistent epoch, bounded core reads).

## Notes

- Live demo (M04W1-E4-T1): three readers at different speeds plus a deliberate
  lap, printing the exact `Gap { missed }` — driven by `gap_exact_laps` /
  `overflow_laps`.
- The borrow half of criterion "borrow/copy is an API choice" (REQ-CORE-003)
  is exercised at the API level here (copy form); the hazard-guarded borrow
  lifecycle is proven in Phase 14.
