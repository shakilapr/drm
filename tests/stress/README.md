# Stress Suites

Linux-only, destructive, or slow tests. Excluded from the default gate
(`cargo xtask test`); run with `cargo xtask test --stress`, which executes
`cargo test --workspace -- --ignored stress --nocapture`.

- Naming: tests are `#[ignore = "stress suite: cargo xtask test --stress"]`
  and prefixed `stress_`.
- Iteration counts scale via `DRM_STRESS_ITERS` (default 100) so CI and
  nightlies can trade depth for wall time.
- Linux-only suites are `#![cfg(unix)]`; on non-unix targets the file
  compiles to nothing and runs zero tests.

| Suite | Crate | Scope |
| --- | --- | --- |
| `shm_kill` | drm-core | SIGKILL at 8 child lifecycle points; leftover state must match the create/unlink contract exactly |
| `no_fd_leak` | drm-core | 10k create/open+unmap cycles; `/proc/self/fd` count stable within ±2 |
| `quota_edge` | drm-core | `/dev/shm` exhaustion ⇒ explicit `QuotaExceeded`/ENOSPC, namespace stays clean; full-extent reservation (no partial truncate) |
| fuzz campaign | drm-workspace | 1h type-codec campaign, evidence in `docs/metrics/fuzz-campaign.log` (M01) |
