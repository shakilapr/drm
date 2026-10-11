# Phase 15 Gate — Pool Exhaustion Behavior

Evidence is executable and remains green under the permanent workspace gates.
Verdict: **PASS**.

Implementation: `drm-core::channel` exhaustion path
([exhaustion-reclamation.md](../algorithms/runtime/exhaustion-reclamation.md)).

## Criteria ⇄ evidence

| Criterion | Evidence | Status |
|---|---|---|
| Holding every usable slot drops without blocking | `exhaustion_drop_and_counter`: all slots held, `PoolExhausted`, `< 1 ms`, no sleeps | PASS |
| Current latest remains valid after exhaustion | `exhaustion_latest_preserved`: rejected publish leaves latest payload unchanged | PASS |
| Exhaustion accounting is exact | `exhaustion_counter_exact`: repeated drops match `pool_exhausted` and `drop_count` | PASS |
| Publishing resumes immediately when capacity returns | `exhaustion_resume_on_release`: next publish succeeds after one hazard release | PASS |
| Real borrow-pattern exhaustion remains safe | hazard-protected channel tests + `hazard_storm` scaffold | PASS (reduced-cycle evidence; long storm is CI/nightly) |

## Invariant coverage (exhaustion-reclamation.md I-E1–I-E4)

| Invariant | Test |
|---|---|
| I-E1 DropNeverBlock | `exhaustion_drop_and_counter` |
| I-E2 LatestPreserved | `exhaustion_latest_preserved` |
| I-E3 CounterExact | `exhaustion_drop_and_counter`, `exhaustion_counter_exact` |
| I-E4 ResumeOnRelease | `exhaustion_resume_on_release` |
| I-E5 IdentitySafeReclaim | Phase 16 `reclaim_*` suites |

## Variables (M05W3 → M05W4)

| Variable | Target | Measured |
|---|---|---|
| `EXHAUSTION_E2E` | real borrow patterns sustained | hazard-protected channel tests; storm scaffold |
| `LIFECYCLE_UNITS` | authority race + epoch tests green | lifecycle suite (M05W3) |
| `RESTART_100` | kill/restart ×100 epoch exact | lifecycle restart harness (M05W3) |
| `SERIAL_LOOPBACK` | bounded frame round-trip | `drm-router::transport::serial` (M05W3) |

## Requirements covered

REQ-CORE-021/022/023 (latest preservation, drop/counter, resume),
REQ-CORE-039/040 (overflow behavior), REQ-CORE-030/031 (hazard acquisition and
ABA safety), REQ-ROUTE-024/025/026 (serial best-effort, bounded frame, CRC).
