# Phase 26 Gate — Static Verification Engine

Chaired at M12W2 (E2; evidence by E2/E4; presented by E4). Evidence is executable
and stays green under the permanent gates. Verdict: **PASS** (the eleven work-plan
#26 rejection classes; `I-V1`..`I-V6` supporting evidence).

Implementation: `crates/drm-workspace/src/verify/engine.rs` (the deterministic
fact catalogue `verify`) and `crates/drm-workspace/src/verify/mod.rs` (`verify_lite`
V-SW slice). Algorithm: [verify-engine.md](../algorithms/toolchain/verify-engine.md).
Owner: E2. Gate chair: E2 (E4 presents evidence).

## Criteria ⇄ evidence

Every negative is rejected with its stable check ID + a REQ citation (`I-V3`); the
valid equivalent passes. Tests live in `verify_engine.rs`, `verify_negatives_v2.rs`,
and `verify_lite.rs`.

| Criterion (work-plan #26) | Check | REQ | Test | Result |
|---|---|---|---|---|
| No publisher | `V-SW` / `V-SW-1` | `REQ-TOOL-064` | `verify_lite::no_publisher_flagged`, `verify_engine::verify_catalogue` | PASS |
| Multiple publishers | `V-SW` / `V-SW-2` | `REQ-TOOL-064` | `verify_lite::multiple_publishers_flagged`, `verify_engine::neg_single_writer` | PASS |
| Invalid schemas | `V-SCH` | `REQ-TOOL-063` | `verify_engine::neg_schema` via `verify_catalogue`/`verify_msg_citations` | PASS |
| Conflicting IDs | `V-ID` | `REQ-TOOL-060` | `verify_engine::verify_handle_conflict`, `verify_engine::neg_identity` | PASS |
| Invalid placement | `V-PLC` | `REQ-TOOL-060` | `verify_engine::neg_placement` | PASS |
| Impossible route | `V-RT` | `REQ-TOOL-062` | `verify_negatives_v2::neg_rt_no_route`, `neg_rt_ambiguous`, `verify_route_integration_single_source` | PASS |
| Framing overflow | `V-RT` | `REQ-TOOL-062`, `REQ-ROUTE-018` | `verify_negatives_v2::neg_rt_framing`, `verify_negatives_v2_route_fit_actionable` | PASS |
| Reliability mismatch | `V-REL` | `REQ-TOOL-061` | `verify_negatives_v2::verify_negatives_v2_reliability` (UDP + serial) | PASS |
| Incompatible consumer requirements | `V-CMP` | `REQ-TOOL-058` | `verify_engine::neg_compatibility`, `verify_negatives_v2_compatibility` | PASS |
| Memory-budget violations | `V-MEM` | `REQ-TOOL-057` | `verify_negatives_v2::neg_mem_budget`, `verify_negatives_v2_memory` | PASS |
| Precommit-budget violations | `V-MEM` | `REQ-TOOL-057` | `verify_negatives_v2::neg_mem_lockable` (page-rounded locked > lockable) | PASS |
| All valid equivalents pass | all | `REQ-TOOL-059` | `verify_engine::verify_positive_full_workspace`, `verify_negatives_v2_cited_and_valid_pass` | PASS |

## Supporting evidence

| Case | Invariant | Test | Result |
|---|---|---|---|
| Catalogue completeness (≥ 1 positive + ≥ 1 negative per check) | `I-V1` | `verify_engine::verify_catalogue` | PASS |
| Determinism (identical report ×100) | `I-V2` | `verify_engine::verify_determinism` | PASS |
| Every failure cites check ID + ≥ 1 REQ ID | `I-V3` | `verify_engine::verify_msg_citations`, `verify_negatives_v2::*` | PASS |
| Estimates labeled `ESTIMATE`; never a guarantee | `I-V4` | `verify_engine::verify_boundary_labels` | PASS |
| Collect-all, never short-circuit | `I-V5` | `verify_engine::verify_no_shortcircuit`, `verify_negatives_v2_no_shortcircuit`, `verify_route_integration_single_source` | PASS |
| Per-consumer deadlines are independent requirements | `I-V6` | `verify_engine::verify_deadlines_independent` | PASS |
| Spatial contracts (two writers, write+transform, undeclared read, index capacity, history bound) | `V-SPAT` | `verify_engine::{verify_spatial_edges,verify_spatial_history_bound}` | PASS |
| RPC route reliability + single-writer event topic | `V-RPC` | `verify_engine::verify_rpc_reliability` | PASS |
| Recorder backpressure on a `reject` topic | `V-REC` | `verify_engine::verify_recorder_required_consumer` | PASS |

## Windows runtime backend (Phase 31 start)

Identity + locks land this week ([windows-backends.md](../algorithms/runtime/windows-backends.md)
§Test Matrix). The `drm-core` Windows suite is green on the `windows-latest`
runner; the socket adapters are `std::net` (cross-platform) with a
platform-neutral error-mapping parity fixture.

| Case | Invariant | Test | Result |
|---|---|---|---|
| `LockFileEx` range released by the kernel on holder death | `I-W3` | `shm::windows::tests::win_lock_release_on_death` | PASS (Windows) |
| Identity incarnation guard (PID reuse) | `I-W5` | `shm::windows::tests::win_pid_reuse`, `win_process_identity` | PASS (Windows) |
| Shared active hold blocks `drm clean` exclusive probe | `I-W8` | `shm::windows::tests::win_active_hold_lifecycle` | PASS (Windows) |
| Create-conflict rollback keeps the winner intact | `I-W7` | `shm::windows::tests::win_create_failure_matrix` | PASS (Windows) |
| Section dies with its last mapping | `I-W4b` | `shm::windows::tests::win_close_last_handle` | PASS (Windows) |
| Socket error mapping parity (UDP/TCP) | `REQ-ROUTE-032`, `REQ-ROUTE-029` | `transport::udp::tests::udp_error_mapping_parity`, `transport::tcp::tests::tcp_error_mapping_parity` | PASS |

## How to reproduce

```text
cargo test -p drm-workspace --test verify_engine
cargo test -p drm-workspace --test verify_negatives_v2
cargo test -p drm-workspace --test verify_lite
cargo test -p drm-core --lib win_          # Windows runner
cargo test -p drm-router --lib transport   # Windows runner
```

## Notes

- `drm verify` reports facts deterministically; estimates are always printed with
  the explicit `ESTIMATE` label and are never presented as guarantees (`I-V4`).
- Route derivation is a single source: `V-RT` consumes the one
  `topology::derive` output, so ambiguity/no-route errors are collected as facts
  without aborting the remaining checks (`I-V5`).
- The full engine is exercised directly by the `drm-workspace` integration
  suites; the CLI `drm verify` path wires the `V-SW` slice
  ([verify/mod.rs](../../crates/drm-workspace/src/verify/mod.rs)) and the engine
  API is the stable contract for the remaining tooling phases.
