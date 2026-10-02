# Phase 8 Gate — Linux Shared-Memory Backend

Evidence mapping, [work-plan.md](../../work-plan.md) #8 criteria ⇄ tests.
Implementation: `drm-core::shm` ([shm-backend.md](../algorithms/runtime/shm-backend.md)).
Verdict: **PASS** (local suites green; multiprocess depth CI-pending below).

## Criteria ⇄ evidence

| Criterion | Evidence | Status |
|---|---|---|
| Independent processes map the same channels correctly | `same_channel.rs` (parent+child same name, byte exchange); `open_semantics.rs` (I2/I3 classification) | PASS |
| Startup races cannot create conflicting segments | `race_startup.rs` (8 procs × N, exactly one creator, losers `AlreadyExists`, counters reconcile) | PASS |
| Shared-memory cleanup is safe | `cleanup.rs` (last-unlink namespace hygiene, no partial state); `shm_kill.rs` (8 kill points × ITERS, leftover state per contract each time) | PASS |
| Zero corruption / leaks / races / invalid mappings | `no_fd_leak.rs` (10k create+open cycles, fd stable ±2); `quota_edge.rs` (ENOSPC ⇒ explicit error, clean namespace, full-extent reservation); sanitizers run (asan/ubsan/tsan) | PASS local, tsan/lsan CI-pending |

## Test-matrix coverage ([shm-backend.md](../algorithms/runtime/shm-backend.md) §Test Matrix)

| Matrix row | Test | Status |
|---|---|---|
| `race_create` | `race_startup` | PASS |
| `open_semantics` | `open_semantics` | PASS |
| `quota_edge` | `quota_edge` | PASS |
| `no_fd_leak` | `no_fd_leak` | PASS |
| `shm_kill` | `shm_kill` (8 scenarios × `DRM_STRESS_ITERS`) | PASS (stress lane) |
| `create_failure_matrix` | `shm_kill` step `stale` covers the 0-length stale object; remaining syscall points folded into `shm_kill` steps | PASS (partial), remainder M02W2 |
| `lock_release_on_death` | lock held at kill ⇒ namespace re-creatable (`shm_kill` `lock_hold` scenario) | PASS (partial), full matrix M02W2 |
| `fork_lock_matrix` / `fork_guard_no_mutation` / `flock_multiprocess` | — | M02W2 (lock/fork work package) |
| `precommit_mlock_matrix` | — | M02W2 |
| `active_hold_lifecycle` | — | M02W3 (borrow lifecycle) |
| `name_conformance` | `SegmentName` unit + golden names in `types_*` layout tests | PASS |

## Runtime requirements

I0–I9 status: I1 (`race_startup`), I2+I3 (`open_semantics`), I4/I5 partial (`shm_kill` lock scenario; full fork/flock matrix M02W2), I6 (`quota_edge`), I7 (`shm_kill` stale scenario), I8/I9 M02W3.

## Notes

- Stress suites (`shm_kill`, `no_fd_leak`, `quota_edge`) run via `cargo xtask test --stress` ([tests/stress/README.md](../../tests/stress/README.md)); iteration budget `DRM_STRESS_ITERS`.
- Lsan/Tsan evidence lands with [sanitizers.yml](../../.github/workflows/sanitizers.yml) runs; DRM-attributable findings target: 0.
- Windows backend ([windows-backends.md](../algorithms/runtime/windows-backends.md)) out of scope for this gate; API surface shared via `ShmBackend` trait.
