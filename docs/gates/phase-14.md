# Phase 14 Gate (DRAFT) — Zero-Copy Borrowing & Hazard Protocol

> **DRAFT** (work-plan M04W4-E2-T2). The completion gate is exercised in M05W2
> once the storm completes a 1 h local run; this draft maps criteria ⇄ evidence
> as they stand.

Evidence mapping, [work-plan.md](../../work-plan.md) #14 criteria ⇄ tests.
Implementation: `drm-core::hazard`
([hazard-protocol.md](../algorithms/runtime/hazard-protocol.md)) + pool drop-free
proof ([payload-pool.md](../algorithms/runtime/payload-pool.md)).
Verdict: **DRAFT — pending 1 h storm**.

## Criteria ⇄ evidence

| Criterion | Evidence | Status |
|---|---|---|
| Every race point in the acquisition algorithm is forced | `hazard::race` R1–R6 forcing tests (roll-before-intent, roll-after-intent, generation-wrap, concurrent-acquire, release-during-scan, reuse-during-reread) | DRAFT |
| An actively borrowed slot is never overwritten | `channel::tests::borrowed_slot_never_overwritten`; `hazard_storm` shadow validator (I-HZ1, 0 violations) | DRAFT |
| ABA tests across repeated slot reuse pass | `hazard::aba` (≥ 2¹⁶ reuses, generation wraps; stale pair never validates — I-HZ2), `channel::tests::generation_aba` | DRAFT |
| Borrow release makes slots reusable | `hazard_slot_set_clear_compare` (CAS install/clear), `hazard::race` R5 | DRAFT |
| Multiple readers of the same latest sample remain safe | `channel::tests::hazard_multi_reader` (two consumers share one latest sample; I-HZ4) | DRAFT |

## Invariant coverage (hazard-protocol.md I-HZ1–I-HZ5)

| Invariant | Test |
|---|---|
| I-HZ1 NeverOverwriteBorrowed | `borrowed_slot_never_overwritten`, `hazard_storm` |
| I-HZ2 AbaGuard | `hazard::aba`, `generation_aba` |
| I-HZ4 MultiReaderShared | `hazard_multi_reader` |
| I-HZ7 SingleOutstandingPerConsumer | `borrow_single_outstanding`, `hazard_acquire_lookup_and_busy` |
| I-HZ8 ClosedRecordRefusesAcquire | `close_invalidates`, `hazard_close_vs_acquire` |
| I-P5 DropFree | `hazard::select` selection-vs-borrows property (`pool_size = R+H+1`) |

## Variables (M04W4, draft)

| Variable | Target | Measured |
|---|---|---|
| `RACE_MATRIX_GREEN` | R1–R6 forcing tests green | pending |
| `ABA_GREEN` | generation wrap ≥ 2¹⁶ reuses validated (I-HZ2) | pending |
| `STORM_LOCAL` | 1 h storm, shadow validator 0 violations (I-HZ1) | pending (scaffold runs green at reduced cycles) |
| `SELECTION_PROPERTY` | selection-vs-borrows property green | pending |
| `LANES_E2E` | two-lane loopback with priority wiring green | pending |

## Requirements covered

REQ-CORE-004 (actively borrowed slot never overwritten), REQ-CORE-030
(acquisition protocol), REQ-CORE-031 (generation ABA guard), REQ-CORE-032
(multiple readers share latest), REQ-CORE-033/034/035 (hazard slot semantics,
retry, single outstanding), REQ-CORE-055 (slot selection excludes
referenced/protected), REQ-CORE-003 (borrow vs copy per call).

## Notes

- Completion gate (M05W2) requires the 1 h local storm at 0 shadow-validator
  violations; reduced-cycle runs (`DRM_STRESS_CYCLES=…`) are used for CI.
