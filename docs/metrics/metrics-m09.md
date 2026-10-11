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

## M09W2 — Verify Engine v1, Watch Completion

| Variable | Target | Measured |
|---|---|---|
| `DEV_STALE_PROOF` | I-BO4 rapid-edits test green | `cargo test -p drm-workspace --test watch_dispatch`: 9/9 green; `cargo test -p drm-workspace dev` selects the 4 I-BO2..I-BO4 cases — `dev_affected_nodes`, `dev_no_source_tracking`, `dev_rapid_edits_final_state` (50 edits ⇒ final state), `dev_rapid_edits_coalesced` (50 edits ⇒ 1 reload); plus `watch_reload_before_dispatch`, `watch_pending_dirty_dispatch`, `watch_overflow_rescan`, `watch_binary_source`, `no_source_tracking_probe` |
| `VERIFY_NEGATIVES_V1` | single-writer + schema + placement negatives green | `cargo test -p drm-workspace --test verify_engine`: 8/8 green — `verify_catalogue` (one negative per `V-SCH`/`V-SW`/`V-PLC`/`V-RT`/`V-REL`/`V-MEM`/`V-CMP`/`V-ID`), `verify_positive_full_workspace`, `verify_msg_citations`, `verify_determinism` (×100), `verify_boundary_labels`, `verify_no_shortcircuit`, `verify_deadlines_independent`, `verify_handle_conflict` |
| `LANE_FREEZE` | lanes-shaping.md design confirmed | frozen in [lanes-shaping.md](../algorithms/router/lanes-shaping.md) — `§Data Structures`/`§Functions` confirmed final against the M03–M05 `drm-router::priority` drafts; no divergence |
| `EFFORT_DAYS_ACTUAL` | 5 per engineer | on plan |

Deliverables:

- Verify engine `crates/drm-workspace/src/verify/engine.rs`
  ([verify-engine.md](../algorithms/toolchain/verify-engine.md)): `CheckId`,
  `VerifyReport` (facts/warnings/estimates), `DeploymentConfig`, and the
  deterministic checks `V-SCH`, `V-SW`, `V-PLC`, `V-RT` (route-derivation +
  framing fit), `V-REL` (transport capability), `V-MEM` (memory-budget
  `verify_deployment`), `V-CMP` (reject-policy + independent deadlines), `V-ID`
  (canonical paths, name collisions, handle conflicts). No short-circuit
  (I-V5); estimates labeled `ESTIMATE` (I-V4).
- Model hook `channel_expectations` (`crates/drm-workspace/src/model/runtime.rs`):
  per-topic pool/bounds exposed for the `V-MEM`/`V-RT` checks (interfaces only).
- Watch completion: `drm dev` rapid-edits behaviour proven (I-BO4) on the
  existing `watch::apply_batch` dispatch core.
- CI job `toolchain` (`.github/workflows/ci.yml`) runs both suites blocking
  (E4-T1).
- Tests `crates/drm-workspace/tests/verify_engine.rs` and the two new
  `watch_dispatch.rs` rapid-edit cases.

Notes:

- `V-SCH`/`V-PLC`/`V-ID` re-validate structural invariants over the built model
  (belt-and-braces): negative fixtures build a model and then mutate a `pub`
  field, proving the verifier rejects a hand-built model the builder would
  normally refuse.
- `V-RT` consumes `topology::derive` and appends its `{plans, errors}` without
  aborting the remaining checks; route ambiguity/override conflicts surface as
  `V-RT` facts.
- Full Phase 26 gate (all `V-*` families incl. `V-SPAT`/`V-RPC`/`V-REC`) lands at
  M09W3.
