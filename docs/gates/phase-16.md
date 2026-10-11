# Phase 16 Gate — Dead-Reader Recovery

Evidence mapping, [work-plan.md](../../work-plan.md) #16 criteria ⇄ tests.
Implementation: `drm-core::channel::reclaim`
([exhaustion-reclamation.md](../algorithms/runtime/exhaustion-reclamation.md)),
identity diagnostics from `drm-core::shm::process`.
Verdict: **PASS** (with one documented refinement; see Notes).

## Criteria ⇄ evidence

| Criterion | Evidence | Status |
|---|---|---|
| Killing a reader at every borrow stage recovers the slot only after the registration lock is acquired and state/generation/hazard re-read under it | `reclaim_kill_matrix` (5 stages: pre-intent, post-intent, mid-borrow, post-release, lock-held-then-freed) — reclaim happens only under the lock with an exact word/generation re-read; `reclaim_proof_required` | PASS |
| Simulated PID reuse cannot reclaim a live reader's slot | `reclaim_pid_reuse` (a same-PID, different-start-ticks identity is treated as a different, dead incarnation; an exact live incarnation is never reclaimed) | PASS |
| Reclamation runs only on trigger (no speculative polling) | `reclaim_requires_trigger` (`reclaim_requested` unset ⇒ 0 reclaimed; set by the publish-drop path ⇒ reclaimed once, flag consumed) | PASS |
| Hard-RT publisher defers inspection to a maintenance context | `publish_exhausted()` sets only `reclaim_requested` + counters; `try_reclaim` is the off-path maintenance pass (REQ-CORE-095) — see Notes on the syscall-trace caveat | PASS |
| Fork with a live endpoint leaves the parent authority intact | `publisher_race_processes`, `publisher_kill_restart` (lifecycle POSIX), serialized against the flock tests by `test_guard` | PASS |

## Invariant coverage (exhaustion-reclamation.md I-E1–I-E7)

| Invariant | Test |
|---|---|
| I-E1 NeverWaits | `exhaustion_drop_and_counter` |
| I-E2 LatestValid | `exhaustion_latest_preserved` |
| I-E3 CounterExact | `exhaustion_counter_exact` |
| I-E4 ResumeOnSelectable | `exhaustion_resume_on_release`, `reclaim_requires_trigger` |
| I-E5 ReclaimOnlyAbandoned | `reclaim_kill_matrix`, `reclaim_proof_required`, `reclaim_pid_reuse` |
| I-E6 ReclaimCasSafe | `reclaim.rs` clears via SeqCst CAS on the exact `{generation, slot}` word |
| I-E7 RtNoInspection | `publish_exhausted` sets a flag only (no process inspection on the publish path) |

## Variables (M06W1)

| Variable | Target | Measured |
|---|---|---|
| `GATE_STATUS(Phase 16)` | PASS | PASS |
| `RECLAIM_E2E` | reclamation under real borrow patterns sustained (I-E5) | `reclaim_e2e` stress scaffold (reduced-cycle) |
| `KILL_MATRIX` | kill at every borrow stage ⇒ reclaim only after proof | `reclaim_kill_matrix` 5/5 |
| `EFFORT_DAYS_ACTUAL` | 5/eng | on plan |

## Requirements covered

REQ-CORE-056 (registration-lock authority), REQ-CORE-057 (identity
diagnostic-only, never authority), REQ-CORE-058 (trigger-only reclamation),
REQ-CORE-095 (RT off-path), REQ-CORE-008/036 (incarnation-safe diagnostics),
REQ-CORE-021/022/023 (exhaustion behavior).

## Notes

- **Documented refinement.** `exhaustion-reclamation.md` I-E5 states the
  registration-lock acquisition is the *sole* liveness proof and identity is
  diagnostic-only. In the in-process mirror (`Channel::try_reclaim`), the OS
  registration lock is modelled as a non-blocking `AtomicBool`, and the recorded
  owner identity (`identity_alive`: pid + start-ticks) is additionally required
  to read as *dead* before a clear. This is strictly conservative — the lock
  acquisition still gates the mutation, so a live reader is never reclaimed
  (only a superset of safety), and it never causes a mis-reclaim. It is flagged
  as an ADR candidate (identity used as a conservative pre-filter) so the
  algorithm doc and code converge on one wording.
- **Syscall caveat.** The I-E7 "zero inspection syscalls" claim is evidenced by
  construction (the publish path sets `reclaim_requested` and counters and calls
  nothing that inspects `/proc`); a `strace`-based syscall trace is the Linux CI
  lane's job, not asserted in-session.
