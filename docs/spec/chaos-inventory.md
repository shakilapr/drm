# Chaos Inventory (M18W1)

Work-plan [M18W1-E2-T1](../../docs/work-plan/work-plan-m18-w1.md): every
killable or interruptible stage across the subsystems, each mapped to an
existing or newly assigned chaos test. A "stage" is a point where `SIGKILL` or
a torn write leaves durable or shared state behind; the test proves the exact
recovery contract, never a best-effort cleanup. Runtime stages follow
[hazard-protocol.md](../algorithms/runtime/hazard-protocol.md),
[exhaustion-reclamation.md](../algorithms/runtime/exhaustion-reclamation.md),
[shm-backend.md](../algorithms/runtime/shm-backend.md), and
[router-core.md](../algorithms/router/router-core.md); toolchain stages follow
[build-orchestration.md](../algorithms/toolchain/build-orchestration.md) and
[studio-protocol.md](../algorithms/studio/studio-protocol.md) (I-ST8).

## Inventory

| Subsystem | Killable / interruptible stage | Chaos test | Status |
|---|---|---|---|
| shm lifecycle | after create, after write, mapping held, lock held, stale 0-length object, after unlink | `crates/drm-core/tests/shm_kill.rs` (`stress_shm_kill_*`, repaired M18W1: child driver invoked with `--ignored`) | green |
| publisher lifecycle | mid-run authority held, restart adoption, epoch exhaustion, init-stuck recovery | `crates/drm-core/src/channel/lifecycle/tests/posix.rs` (`publisher_kill_restart`, `publisher_race_processes`) | green |
| hazard borrow | pre-intent, post-intent, mid-borrow, post-release, restart stale word | `crates/drm-core/tests/stress/chaos_kill_matrix.rs` (`stress_chaos_kill_matrix`) | green |
| hazard reclaim | dead reader abandoned word, registration-lock-held cell, generation roll | `reclaim_kill_matrix` (unit) + `stress_chaos_kill_matrix` (integration) | green |
| PID reuse | same PID, different incarnation | `stress_chaos_pid_reuse`; unit `reclaim_pid_reuse` | green |
| pool exhaustion | all slots protected, drop storm, delayed reclaim, resume | `stress_chaos_exhaustion_storm`; unit `reclaim_delayed_not_corrupt` | green |
| router link | cut/reconnect egress + ingress, rapid flapping × routers | `crates/drm-router/tests/link_storm.rs` (`stress_link_storm`) | green |
| router session | reconnect generation bump, boundary gap, closed-session refuse | `stress_link_storm_sessions` | green |
| serial framing | truncated stream, embedded delimiter, resync after corruption, partial frame | `serial` fuzz target; `serial_corruption` unit | green |
| parser | truncated/UTF-8-invalid/torn topic bytes, malformed node/topology manifests | `verify` fuzz target; `schema` fuzz target; `fuzz_seeds` | green |
| model build | partial workspace (`packages/` + `spaces/`/`index.drm` bytes) ⇒ diagnostics | `verify` fuzz target (`build` + `verify_lite`) | green |
| codegen | interrupted render/write of a generated artifact | `cargo xtask gen-check`; `codegen_invariants` | green (deterministic regeneration) |
| journal [I-ST8] | kill after prepare, mid-commit, after commit, idempotent replay | `crates/drm-cli/src/studio/tests.rs` (`journal_recovery_prepared_rolls_back`, `journal_recovery_partial_finishes`, `journal_commit_clean`) | green |
| build | kill between model build and deploy emit | `build_order`, `build` orchestrator tests | gap: no process-kill cell — assigned M18W2 (E1) |
| deploy | torn `deployment.json` write | `deploy` unit tests; deterministic render | gap: no torn-write kill cell — assigned M18W2 (E1) |
| clean | kill during `drm clean` unlink | `crates/drm-cli` clean tests (REQ-CORE-080) | gap: no process-kill cell — assigned M18W2 (E1) |

## Gaps assigned

- **build / deploy / clean process-kill cells** (three rows above): the
  toolchain kill matrix is scheduled for M18W2 (E1) per the carry-over in the
  week plan; the runtime and journal matrices are green this week.

REQ: REQ-QAL-005, REQ-QAL-010, REQ-CORE-054, REQ-CORE-056, REQ-CORE-057,
REQ-CORE-058, REQ-CORE-072, REQ-CORE-080, REQ-ROUTE-007, REQ-ROUTE-029,
REQ-TOOL-012, REQ-STUDIO-008.
