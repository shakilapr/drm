# Phase 17 Gate — Publisher Lifecycle & Epochs

Evidence mapping, [work-plan.md](../../work-plan.md) #17 criteria ⇄ tests.
Implementation: `drm-core::channel::lifecycle`
([publisher-lifecycle.md](../algorithms/runtime/publisher-lifecycle.md)) and the
cursor epoch gates ([cursor-delivery.md](../algorithms/runtime/cursor-delivery.md)).
Verdict: **PASS**.

## Criteria ⇄ evidence

| Criterion | Evidence | Status |
|---|---|---|
| Racing two publishers: exactly one acquires authority | `publisher_race_second_refused` (second `publisher_open` ⇒ refused), `publisher_race_threads` (in-process race), `publisher_race_processes` (cross-process, `test_guard`-serialized) — I-L1 | PASS |
| Killing the active publisher releases authority (OS lock) | `publisher_kill_restart` (SIGKILL ⇒ successor adopts authority; I-L2) | PASS |
| Restart increments epoch exactly once; consumers discard old cursor assumptions | `publisher_epoch_init_and_increment_exactly_once` (fresh ⇒ 1; reuse ⇒ +1 once), `publisher_restart_loop_100` (epoch == cycle+1 ×100; I-L3), `channel::tests::epoch_restart`, `consumer_observes_epoch_reset` (I-L4) | PASS |
| No pre-restart sample is interpreted as post-restart data | `publisher_restart_resets_run_state` + `consumer_observes_epoch_reset` (floor moves to the preserved next sequence; prior-run sequences are unaddressable; `first_valid_seq` watermark; I-L5/L6) | PASS |
| Sequence continuity preserved across restart | `publisher_restart_loop_100` asserts `first_valid_seq == prior write_seq` (continue, epoch disambiguates runs) | PASS |

## Invariant coverage (publisher-lifecycle.md I-L1–I-L7)

| Invariant | Test |
|---|---|
| I-L1 SingleWriter | `publisher_race_second_refused`, `publisher_race_threads`, `publisher_race_processes` |
| I-L2 AuthorityReleasedOnDeath | `publisher_kill_restart` |
| I-L3 EpochExactlyOnce | `publisher_epoch_init_and_increment_exactly_once`, `publisher_restart_loop_100` |
| I-L4 ConsumerDiscardsOldEpoch | `consumer_observes_epoch_reset`, `channel::tests::epoch_restart` |
| I-L5 NoCrossEpochRead | `consumer_observes_epoch_reset`, `publisher_restart_resets_run_state` |
| I-L6 RunStateReset | `publisher_restart_resets_run_state` |
| I-L7 FreshnessTransition | publisher-side reset covered; full consumer freshness machine is a Phase 17 follow-on (see Notes) |

## Variables (M06W2)

| Variable | Target | Measured |
|---|---|---|
| `GATE_STATUS(Phase 17)` | PASS | PASS |
| `LIFECYCLE_UNITS` | authority race + epoch tests green | `cargo test -p drm-core lifecycle`: 16+ passed |
| `RESTART_100` | kill/restart ×100 epoch exactly-once | `publisher_restart_loop_100` (in-memory lifecycle path) |
| `FFI_ENDPOINTS` | open/close/publish/read from C | `drm-ffi` in-process endpoint backing + Rust/C tests (see M06W2 bindings slice) |
| `EFFORT_DAYS_ACTUAL` | 5/eng | on plan |

## Requirements covered

REQ-CORE-018/019/020 (authority, epoch init/increment), REQ-CORE-026 (restart
protocol / run-state reset), REQ-CORE-038 (consumer epoch observe/reset),
REQ-CORE-010/096 (cursor epoch re-read, persistent epoch).

## Notes

- The `RESTART_100` harness exercises the in-memory lifecycle path
  deterministically; the real-POSIX `publisher_kill_restart` covers the
  kernel-lock-release path at ×1 and is serialized against the flock tests by
  `test_guard`.
- The full consumer `FreshnessTracker`/`StreamState` machine remains Phase 17
  consumer-subsystem scope; the publisher-side reset that drives it is covered
  here and in the M06W2 lifecycle e2e.
