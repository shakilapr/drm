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

## M09W3 — Phase 25 Gate, Verify Negatives v2, Route Doc Absorption

| Variable | Target | Measured |
|---|---|---|
| `GATE_STATUS(Phase 25)` | PASS | PASS — [phase-25.md](../gates/phase-25.md) (I-BO1..I-BO4 + I-BO5..I-BO8 supporting; adapter matrix; dev smoke) |
| `VERIFY_NEGATIVES_V2` | framing/reliability/budget/compatibility negatives green | `cargo test -p drm-workspace --test verify_negatives_v2`: 7/7 green — `V-RT` (framing overflow, ambiguous route, no route), `V-REL` (reliable over UDP + serial), `V-MEM` (budget/lockable/pool invariant), `V-CMP` (reject without a required sequential consumer), collect-all, route integration |
| `ROUTE_DOC` | route-derivation.md merged; old spec absorbed | [route-derivation.md](../algorithms/toolchain/route-derivation.md) normative; [routing-derivation.md](../spec/routing-derivation.md) is now a pointer (M09W3-E3-T0) |
| `EFFORT_DAYS_ACTUAL` | 5 per engineer | on plan |

Deliverables:

- Verify negatives v2 `crates/drm-workspace/tests/verify_negatives_v2.rs`:
  multiple negative fixtures per `V-RT`/`V-REL`/`V-MEM`/`V-CMP` check, each
  rejected with its check ID and a REQ-cited message; valid equivalents pass;
  `I-V5` collect-all across the four families; route integration proves `V-RT`
  consumes the single `topology::derive` output (an ambiguous diamond surfaces
  as a `V-RT` fact naming the topic).
- Dev-mode smoke `crates/drm-cli/tests/dev_smoke.rs`: `drm dev --once` green
  and successive dev cycles after scripted declaration + application-source
  edits on a scratch `examples/mixed_robot` (never the committed tree).
- Phase 25 gate evidence ([phase-25.md](../gates/phase-25.md)).
- Route-derivation absorption: [routing-derivation.md](../spec/routing-derivation.md)
  replaced by a pointer to the normative algorithm doc.
- CI job `toolchain` extended with `verify_negatives_v2` and `dev_smoke` steps.

Notes:

- `V-RT` rejects at two levels, both from the single derivation source: route
  derivation errors (`Ambiguous`, `NoRoute`, `ConflictingRouteOverride`) and the
  encoded-frame fit against `max_datagram_payload`.
- The dev smoke is deterministic by design (build cycles, not a live watcher):
  the poller and debounce core is covered independently by `watch_dispatch.rs`.

## M09W4 — Verify Catalogue Completion, Router Core Data Path

| Variable | Target | Measured |
|---|---|---|
| `VERIFY_FULL` | complete check catalogue green | `cargo test -p drm-workspace verify`: `verify_engine` 12/12 — `verify_catalogue` (I-V1) now spans 11 checks (`V-SCH`/`V-SW`/`V-PLC`/`V-RT`/`V-REL`/`V-MEM`/`V-CMP`/`V-ID`/`V-SPAT`/`V-RPC`/`V-REC`), each with ≥ 1 positive + ≥ 1 negative fixture; `verify_negatives_v2` 7/7; `verify_lite` 1/1 |
| `ROUTER_UDP_GREEN` | UDP egress/ingress over loopback pair | `cargo test -p drm-router --test router_core`: 4/4 green — `egress_udp` (route filter + framed loopback send), `import_native_equivalence`, `ingress_ordering`, `ingress_rejects_before_state`; `cargo run -p drm-router --example udp_pair` loopback green |
| `IMPORT_PROBE` | imported data indistinguishable from local (I-RT4) | `import_native_equivalence` — the imported payload deposits in the same native bytes as the local sample (only the wire header is stripped), so a consumer cannot distinguish them |
| `EFFORT_DAYS_ACTUAL` | 5 per engineer | on plan |

Deliverables:

- Verify catalogue completion `crates/drm-workspace/src/verify/engine.rs`
  ([verify-engine.md](../algorithms/toolchain/verify-engine.md) §Check catalogue):
  `V-SPAT` (dynamic-edge single writer, write+transform conflict, declared
  `[spatial]` reads, `index.drm` capacity within budget (I-MB9), satisfiable
  history bound, no naked-coordinate fields), `V-RPC` (request/cancel/event
  route reliability, single-writer server-owned event topic, concurrency within
  the capacity model), and `V-REC` (`on_full = backpressure` on a `reject` topic
  requires a declared required sequential consumer). `ALL_CHECKS` now enumerates
  11 ids in report order (I-V1).
- Negative fixtures for the three new checks (`neg_spatial`, `neg_rpc`,
  `neg_recorder`) plus positive cases (`verify_spatial_edges`,
  `verify_spatial_history_bound`, `verify_rpc_reliability`,
  `verify_recorder_required_consumer`) in
  `crates/drm-workspace/tests/verify_engine.rs`.
- Router core data path `crates/drm-router/src/data_path.rs` (router-core.md
  §Functions `egress`/`ingress`): `EgressRoute` (link/wire-channel binding,
  I-RT1 remote-consumer filter, I-RT3 `max_datagram_payload`, I-RT18
  `max_record_bytes`), `Router::egress`/`ingress_frame`, the `ImportSink`
  synthetic-publisher deposit with per-`(link, wire_channel_id)` epoch/sequence
  ordering (I-RT9), and `IngressOutcome` (`Imported`/`Duplicate`/`StaleEpoch`/
  `UnknownChannel`).
- Router core suite `crates/drm-router/tests/router_core.rs`: UDP egress
  (oversize rejected before any send; loopback send green), header-stripped
  ingress ordering, reject-before-state (unknown channel, length mismatch,
  receive-bound), and imported-vs-local native equivalence (I-RT4).

Notes:

- The verify catalogue now matches verify-engine.md §Check catalogue exactly;
  the Phase 26 gate doc itself lands at M12W2, so this week completes the
  implementation + negative matrix, not the gate.
- `Router::ingress_frame` strips the wire header and deposits through the
  synthetic-publisher `ImportSink` in native format; ordering state is per
  `(link, wire_channel_id)` and a protocol error never mutates it (I-RT14).

