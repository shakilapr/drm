# Phase 19 Gate — Memory Architecture & Precommit

Evidence mapping, [work-plan.md](../../work-plan.md) #19 criteria ⇄ tests.
Implementation: `drm-workspace::budget` + `drm-core::channel` pool/precommit
([memory-budget.md](../algorithms/memory-budget.md),
[payload-pool.md](../algorithms/runtime/payload-pool.md)).
Verdict: **PASS**.

## Criteria ⇄ evidence

| Criterion | Evidence | Status |
|---|---|---|
| Exact worst-case capacity matches independently calculated reference values | `drm-workspace` `budget_reference` — 5 byte-exact fixtures (nav 10752, sensor 100864, edge 10512, arm64 320512, exact 5000) plus formula agreement (I-MB1) | PASS |
| Invalid `pool_size < R + 1` fails | `budget_negative` pool-invariant cases + `channel::tests::pool_invariant_min` (I-MB4) | PASS |
| One topic can exhaust itself without affecting another | `channel::pool::tests::pool_isolation`, `channel::tests::pool_offsets_static` (per-topic partitions) + `exhaustion_*` (topic-scoped drop) | PASS |
| Precommit pre-faults/locks the complete declared set or refuses activation | `drm-core` `precommit` suite (real `mlock` success; injected quota refusal ⇒ no active channel; `precommit_e2e`) — I-MB5, REQ-CORE-090 | PASS |

## Invariant coverage (memory-budget.md I-MB1–I-MB5)

| Invariant | Test |
|---|---|
| I-MB1 ReferenceExact | `budget_reference` |
| I-MB2 BudgetRules | `budget_negative` (LockableExceeded, BudgetExceeded) |
| I-MB3 RamWarning | `budget_negative` (RAM warning band) |
| I-MB4 PoolInvariant | `budget_negative`, `pool_invariant_min` |
| I-MB5 RefuseActivation | `precommit_refusal_*`, `precommit_no_pageable_continuation` |

## Variables (M07W1)

| Variable | Target | Measured |
|---|---|---|
| `GATE_STATUS(Phase 19)` | PASS | PASS |
| `BUDGET_REFERENCE` | worst-case == independent reference (I-MB1) | `budget_reference` 8 passed (5 byte-exact) |
| `PRECOMMIT_UNITS` | pre-fault + lock + refusal green (I-MB5) | `precommit` 8 + `precommit_e2e` |
| `FFI_COMPLETE` | full op set real-impl green | `drm-ffi` open/publish/read/borrow/copy/close |
| `EFFORT_DAYS_ACTUAL` | 5/eng | on plan |

## Requirements covered

REQ-TOOL-008 (budget rule), REQ-CORE-090 (precommit refuse-activation),
REQ-CORE-021/022/023 (exhaustion), REQ-CORE-045/046 (pool floor, isolation),
REQ-CORE-095 (RT off-path reclamation).

## Notes

- The week plan's "no-unwind shield / catch_unwind" wording for the C ABI
  conflicts with the normative [c-abi-surface.md](../algorithms/bindings/c-abi-surface.md)
  I-A6 ("no `catch_unwind`; release `panic = abort`; no `Internal` status").
  The doc governs: exports are structurally panic-free (every fallible path
  returns a `Status`) and the release profile aborts on an internal invariant
  violation — evidenced by the fail-stop policy test, not a shield.
