# Metrics — Month 07 (M07)

Normative source: [work-plan.md](../../work-plan.md) W3 Bindings, Toolchain &
Verification. One section per weekly tranche.

## M07W1 — Phase 19 Gate (Memory & Precommit), C ABI Completion

| Variable | Target | Measured |
|---|---|---|
| `GATE_STATUS(Phase 19)` | PASS | PASS — `docs/gates/phase-19.md` |
| `FFI_COMPLETE` | full op set real-impl green (open/publish/read/borrow/copy/close) | `drm-ffi` 30 tests; undeclared topic ⇒ `Permission`; empty-channel borrow ⇒ `NoSample`; close symmetry |
| `NO_UNWIND` | fail-stop policy (I-A6: no `catch_unwind`) | `no_unwind` 2 tests: release profile `panic = "abort"`; every export returns a `Status` on bad-but-non-null args |
| `FFI_NEGATIVES` | full edge-case matrix green | `ffi_negative` 12 tests |
| `COUNTERS` | CCB counter surface complete | `drm-core` `counters` (write_seq-derived last_sequence, drop/overrun/pool_exhausted/epoch/reclaim flag) |
| `PRECOMMIT_E2E` | activation success + refusal re-checked after identity change | `drm-core` `precommit_e2e` |
| `EFFORT_DAYS_ACTUAL` | 5/eng | on plan |

Deliverables: `drm-ffi` close symmetry + negative matrix + fail-stop policy
test; `drm-core` `ChannelCounters` accessor + `counters`/`precommit_e2e` suites.
**Fixed:** `drm_consumer_close` now returns `Busy` on an outstanding borrow
(consumer-registration I-CR4) instead of releasing it — removes a doc/impl
divergence. The week plan's "catch_unwind shield" wording is superseded by the
normative I-A6 fail-stop policy.
