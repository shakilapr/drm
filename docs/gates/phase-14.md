# Phase 14 Gate — Zero-Copy Borrowing & Hazard Protocol

Evidence mapping, [work-plan.md](../../work-plan.md) #14 criteria ⇄ tests.
Implementation: `drm-core::hazard`
([hazard-protocol.md](../algorithms/runtime/hazard-protocol.md)) + pool drop-free
proof ([payload-pool.md](../algorithms/runtime/payload-pool.md)).
Verdict: **PASS** (all race points forced; storm scaffold green at reduced
cycles — the multi-hour wall-time storm is a CI/nightly lane, not re-run
in-session).

## Criteria ⇄ evidence

| Criterion | Evidence | Status |
|---|---|---|
| Every race point in the acquisition algorithm is forced | `hazard::race` R1–R6 (roll-before-intent, roll-after-intent, generation-wrap, concurrent-acquire over a `Barrier`, release-during-scan, reuse-during-reread) | PASS |
| An actively borrowed slot is never overwritten | `channel::tests::borrowed_slot_never_overwritten`; `hazard_storm` shadow validator (I-HZ1) | PASS |
| ABA tests across repeated slot reuse pass | `hazard::aba` (`hazard_aba_reuse_wrap`: each of 3 slots reused ≥ 2¹⁶×, stale pair never validates; I-HZ2), `channel::tests::generation_aba` | PASS |
| Borrow release makes slots reusable | `hazard_slot_set_clear_compare` (CAS install/clear), `hazard::race` R5; reclaim slow path (Phase 16) | PASS |
| Multiple readers of the same latest sample remain safe | `channel::tests::hazard_multi_reader` (two consumers share one latest sample; I-HZ4) | PASS |

## Invariant coverage (hazard-protocol.md I-HZ1–I-HZ5)

| Invariant | Test |
|---|---|
| I-HZ1 NeverOverwriteBorrowed | `borrowed_slot_never_overwritten`, `hazard_storm` |
| I-HZ2 AbaGuard | `hazard::aba`, `generation_aba` |
| I-HZ4 MultiReaderShared | `hazard_multi_reader` |
| I-HZ7 SingleOutstandingPerConsumer | `borrow_single_outstanding`, `hazard_acquire_lookup_and_busy` |
| I-HZ8 ClosedRecordRefusesAcquire | `close_invalidates`, `hazard_close_vs_acquire` |
| I-P5 DropFree | `hazard::select` selection-vs-borrows property (`pool_size = R+H+1`) |

## Variables (M04W4 → M05W2)

| Variable | Target | Measured |
|---|---|---|
| `RACE_MATRIX_GREEN` | R1–R6 forcing tests green | `hazard::race` 6 tests green |
| `ABA_GREEN` | generation wrap ≥ 2¹⁶ reuses validated (I-HZ2) | `hazard::aba::hazard_aba_reuse_wrap` green |
| `STORM_LOCAL` | storm, shadow validator 0 violations (I-HZ1) | scaffold green at reduced cycles; multi-hour wall-time run is the CI/nightly lane |
| `SELECTION_PROPERTY` | selection-vs-borrows property green | `hazard::select` 2 tests green |
| `LANES_E2E` | two-lane loopback with priority wiring green | `drm-router` `lane_egress` + `priority_saturation` + `lane_mixed_class` green |

## Requirements covered

REQ-CORE-004 (actively borrowed slot never overwritten), REQ-CORE-030
(acquisition protocol), REQ-CORE-031 (generation ABA guard), REQ-CORE-032
(multiple readers share latest), REQ-CORE-033/034/035 (hazard slot semantics,
retry, single outstanding), REQ-CORE-055 (slot selection excludes
referenced/protected), REQ-CORE-003 (borrow vs copy per call).

## Notes

- The multi-hour wall-time storm (1–3 h) is a CI/nightly lane
  ([.github/workflows/fuzz-nightly.yml](../../.github/workflows/fuzz-nightly.yml));
  in-session the `hazard_storm` harness is exercised at reduced cycles
  (`DRM_STRESS_CYCLES`) with 0 shadow-validator violations.
- Dead-reader reclamation (Phase 16) builds on this gate's protection model;
  see [exhaustion-reclamation.md](../algorithms/runtime/exhaustion-reclamation.md).
