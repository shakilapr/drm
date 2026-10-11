# Metrics — Month 09 (M09)

Normative source: [work-plan.md](../../work-plan.md) W3 Bindings, Toolchain &
Verification. One section per weekly tranche.

## M09W1 — Phase 24 Gate (Codegen), Build Orchestration Implementation

| Variable | Target | Measured |
|---|---|---|
| `GATE_STATUS(Phase 24)` | PASS | PASS — [phase-24.md](../gates/phase-24.md) (I-G1..I-G6, I-G8, I-G9, R6; I-G7 POSIX green, Windows backend deferred) |
| `BUILD_ORDER_GREEN` | I-BO1 hook test green | `cargo test -p drm-workspace --test build_order`: 5/5 green — `build_order` (exact step sequence), `verify_failure_aborts_before_adapters`, `adapter_matrix`, `info_projection_is_deterministic`, `build_target_subset` |
| `EXAMPLE_BUILD` | `drm build` green on mixed example (c+rust+py nodes) | `cargo test -p drm-cli --test example_build`: `example_build_green` (3 nodes built, deployment artifact written), `example_info_deterministic` |
| `WATCH_SKELETON` | two-category dispatch + I-BO5..I-BO8 | `cargo test -p drm-workspace --test watch_dispatch`: 6/6 green — `dev_affected_nodes`, `watch_reload_before_dispatch`, `watch_pending_dirty_dispatch`, `watch_overflow_rescan`, `watch_binary_source`, `no_source_tracking_probe` |
| `GEN_CHECK_CI` | `xtask gen-check` blocking (R6) | wired; self-test `xtask::gencheck::detects_hand_edited_generated_file` |
| `ROUTER_BASELINE` | loopback perf within bounds | no runtime data-path code changed this week (toolchain only); reference `cargo run --release -p drm-router --example udp_pair` unchanged at ~4.2 µs/exchange (M08W1 band 41.5–51.9 ms / 10 000) |
| `EFFORT_DAYS_ACTUAL` | 5 per engineer | on plan |

Deliverables:

- Orchestrator `crates/drm-workspace/src/orchestrator.rs` — `BuildRequest`,
  `BuildAdapter{Cmake,Cargo,Command}`, `BuildStep` sequence probe (I-BO1),
  injectable `AdapterRunner`, and adapter environment hooks (`DRM_GEN_INCLUDE`,
  `DRM_DEPLOY_METADATA`). `drm build` aborts on a verify failure before any
  adapter runs.
- Deployment artifacts `crates/drm-workspace/src/deploy.rs` — deterministic
  `.drm/deploy/deployment.json` handle table.
- Watch skeleton `crates/drm-workspace/src/watch.rs` — two-category
  classification, reload-before-dispatch (I-BO5), pending-dirty retention
  (I-BO6), rescan recovery (I-BO7), path-only ownership (I-BO8), affected-node
  scoping (I-BO3).
- CLI `drm build` / `drm info` / `drm dev [--once]` (`crates/drm-cli/src/{build,info,dev}.rs`).
- Node manifests carry `language` and `[build]` adapters
  (`crates/drm-workspace/src/model/nodes.rs`).
- `examples/mixed_robot` — one node per language (planner python, controller c,
  driver rust), CI-executed.
- Tests `crates/drm-workspace/tests/{build_order,watch_dispatch}.rs` and
  `crates/drm-cli/tests/example_build.rs`.
- Phase 24 gate doc ([phase-24.md](../gates/phase-24.md)).

Notes:

- The watch skeleton uses a dependency-free mtime poller in `drm dev`; a native
  event watcher (`inotify`/`ReadDirectoryChangesW`) replaces the poller without
  changing the `watch::apply_batch` dispatch core. The two-category logic,
  pending-dirty retention, and rescan recovery are unit-tested independently of
  the poller.
- Example node adapters use an explicit `[build] command` that asserts the
  environment hooks are present, so the mixed build is hermetic (no cmake/cargo
  toolchain required in the gate environment).
- `drm build` persists the stable handle table to `.drm/ids.json`; normal builds
  preserve assigned handles (REQ-TOOL-036).
