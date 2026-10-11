# Phase 32 Gate — Operational Tooling (in progress)

Normative sources: [work-plan.md](../../work-plan.md) W6 Phase 32;
[observability-plane.md](../algorithms/ops/observability-plane.md),
[record-replay.md](../algorithms/ops/record-replay.md),
[rpc-actions.md](../algorithms/ops/rpc-actions.md),
[serial-mcu.md](../algorithms/router/serial-mcu.md). Owner: E1 (trace/RPC),
E2 (spatial parser groundwork), E3 (serial/MCU), E4 (introspection/diag).

Status: **IN PROGRESS** (opens M15, closes M19). Sub-designs locked at M14W4
([tooling-design-lock.md](../spec/tooling-design-lock.md)).

## Wave A (M15) — introspection + trace + diag + MCU start

| Criterion | Evidence | Status |
|---|---|---|
| Trace lanes never block producers; best-effort drop counted | [m15w1.md](m15w1.md); `drm-core` `trace_never_blocks`/`trace_lane_exhaustion`/`trace_drop_counter` (I-OB1/I-OB6) | PASS |
| Compiled numeric IDs + build-time dictionary | `TraceDict`/`DictVersion`; `trace_dict_roundtrip` (I-OB2, REQ-OPS-014) | PASS |
| Diagnostics out-of-band; observed rate vs declared; deadline state | `drm-diag` `diag_no_overhead`/`diag_observed_rate`/`diag_deadline_matrix`/`diag_epoch_sandwich` (I-OB2/3/4/7); aggregation + condition matrix [m15w2.md](m15w2.md) | PASS |
| Introspection read-only; cannot pin bulk samples | `introspect_no_pin`; `drm echo`/`hz`/`log` (I-OB5, REQ-OPS-003) | PASS |
| Spatial parser groundwork (non-breaking) | `parser/spatial.rs` feature `spatial-grammar`; corpus unaffected (M15W1-E2-T1) | PASS |
| MCU toolchain + hello-world bring-up | `bindings/c/mcu/`; `mcu-cross.yml` (M15W1-E3-T1/T2) | PASS |
| MCU zero-heap data path + serial framing parity | `drm_node.{h,c}` static SPSC ring (I-SR5, REQ-CORE-087); `drm_serial.{h,c}` COBS/CRC32C (I-SR2..I-SR4); `make host-test`/`host-audit`; [m15w2.md](m15w2.md) | PASS (host + CI) |
| Serial framing interop byte-for-byte | `drm-router::transport::serial` COBS/CRC32C + cross-OS host-loopback; MCU host parity (REQ-OPS-004) | PASS (host) |
| Record-core ingress (borrow-copy-release, bounded slabs, policies) | `crates/drm-record`; `record_borrow_scope`/`record_bounded`/`record_policies`/`record_gap_ledger` (I-RC1/2/3/5); [m15w2.md](m15w2.md) | PASS |
| Record disk stage + MCAP framing (DRM metadata, gaps) | `crates/drm-record/src/{mcap,disk}.rs`; `record_disk_mcap`/`record_mcap_drm_meta`/`record_mcap_zstd` (I-RC6, REQ-OPS-006); [m15w4.md](m15w4.md) | PASS |
| Replay pipeline (faithful read/publish, scheduler, isolated namespace) | `crates/drm-record/src/replay.rs`; `replay_roundtrip`/`replay_publisher_faithful`/`replay_reader_epoch` (I-RC4, REQ-OPS-009); [m15w4.md](m15w4.md) | PASS |
| RPC/actions, full diag daemon | scheduled M16–M18 | pending |
| RPC request/response + cid dispatch + deadline (unary) | `crates/drm-rpc`; `call_round_trip`/`call_remote_failure`/`dispatch_concurrent_no_cross_correlation`/`dispatch_admission_busy`/`dispatch_timeout_expires_and_late_terminal_stale`/`server_session_stale_dropped`/`event_order_invalid_terminal_first` (I-RPC1/3/4/6/7/9/10, REQ-OPS-011/012/013/020/021/022/023/025); [m16w1.md](m16w1.md) | PASS (M16W1) |
| RPC action phase machine + phase-boundary cancel; dispatch ×16 | `crates/drm-rpc/src/action.rs`; `action_runs_all_phases`/`cancel_at_each_phase_boundary`/`dispatch_concurrent_no_cross_correlation` (C ∈ 1..=16) (I-RPC1/2/8, REQ-OPS-010/011/013); [m16w2.md](m16w2.md) | PASS (M16W2) |
| RPC per-phase state-transition observation + cancel ack | `crates/drm-rpc/src/client.rs` per-kind counters; `action_runs_all_phases`/`cancel_at_each_phase_boundary` (I-RPC2/I-RPC10, REQ-OPS-010/011/025); [m16w3.md](m16w3.md) | PASS (M16W3) |
| Handle compaction gate + coordinated-rebuild guidance | `HandleTable::compact`/`rebuild_guidance`; `drm ids compact --yes`; `handle_compact_gate`/`handle_compact_scope`/`handle_compact_guidance` (I-H3/I-H4/I-H7, REQ-TOOL-038); [m16w3.md](m16w3.md) | PASS (M16W3) |
| Serial host↔MCU interop matrix byte-exact | `serial_cobs_golden`/`mcu_host_interop_matrix`; `bindings/c/mcu/selftest.c` `test_interop_matrix` (I-SR3/I-SR4/I-SR6, REQ-OPS-004/REQ-ROUTE-025/026); [m16w3.md](m16w3.md) | PASS (M16W3) |

## Phase 32 submodule suite mapping (M16W2–M16W3)

Each work-plan #32 criterion maps to a submodule suite with its artifact, test,
and result. `PASS` = green in-session; `deferred` = hardware/CI-owned lane.

| Submodule | Artifact | Test | Result |
|---|---|---|---|
| Introspection (`echo`/`hz`/`log`) | `crates/drm-cli/src/{echo,hz,log}.rs`; `drm-diag` `Observer` | `introspect_no_pin`; `echo_copies_and_reports_empty`, `hz_tolerance`, `render_and_filter` | PASS |
| Diag aggregation | `crates/drm-diag/src/lib.rs` | `diag_no_overhead`, `diag_observed_rate`, `diag_deadline_matrix`, `diag_epoch_sandwich`, `diag_condition_matrix` | PASS |
| Record / replay | `crates/drm-record/src/{lib,disk,mcap,replay}.rs` | `record_borrow_scope`, `record_bounded`, `record_slab_full_boundary_exact`, `record_policies`, `record_mcap_drm_meta`, `replay_roundtrip` | PASS |
| Trace | `crates/drm-core/src/trace/mod.rs` | `trace_never_blocks`, `trace_drop_counter`, `trace_lane_exhaustion`, `trace_dict_roundtrip` | PASS |
| RPC / actions | `crates/drm-rpc/src/{lib,codec,client,server,action}.rs` | `call_round_trip`, `dispatch_concurrent_no_cross_correlation` (C ∈ 1..=16), `cancel_at_each_phase_boundary`, `action_runs_all_phases` | PASS |
| Handles / toolchain | `crates/drm-workspace/src/model/handles.rs`; `crates/drm-cli/src/ids.rs` | `handle_compact_gate`, `handle_compact_scope`, `handle_compact_guidance`, `ids_report_is_deterministic`, `compact_guidance_checklist` | PASS |
| Serial / MCU | `crates/drm-router/src/transport/serial.rs`; `bindings/c/mcu/` | `serial_cobs_golden`, `mcu_host_interop_matrix`; `make host-test host-audit` | PASS (host); 2 h target soak deferred |

## Notes

- No architecture divergence in wave A: pinned invariants only; no ADR.
- Remaining Phase 32 output (`drm record`, replay, `drm diag`, RPC/actions,
  constrained-device data path) is tracked in the M16–M19 weekly plans.
- M16W2 closed the trace gates (I-OB1/I-OB6 re-run, `m16w2.md`) and the RPC
  phase-boundary cancel + `C ∈ 1..=16` dispatch slice (I-RPC2, `m16w2.md`).
- M16W3 closed the per-phase RPC state-transition observations and cancel ack
  (I-RPC2/I-RPC10, `m16w3.md`), the handle compaction gate + coordinated-rebuild
  guidance (I-H3/I-H4/I-H7, `m16w3.md`), and the byte-exact host↔MCU serial
  interop matrix (I-SR3/I-SR4/I-SR6, `m16w3.md`). Remaining Phase 32 output
  (`drm record`/`drm diag` CLI, constrained-device data path) continues in
  M16W4–M19.

REQ: REQ-OPS-001, REQ-OPS-002, REQ-OPS-003, REQ-OPS-004, REQ-OPS-005,
REQ-OPS-006, REQ-OPS-007, REQ-OPS-008, REQ-OPS-009, REQ-OPS-010,
REQ-OPS-011, REQ-OPS-012, REQ-OPS-013, REQ-OPS-014,
REQ-OPS-015, REQ-OPS-016, REQ-OPS-017, REQ-OPS-018, REQ-OPS-019,
REQ-OPS-020, REQ-OPS-021, REQ-OPS-022, REQ-OPS-023, REQ-OPS-024,
REQ-OPS-025,
REQ-CORE-087, REQ-CORE-088, REQ-CORE-089, REQ-CORE-093, REQ-CORE-094,
REQ-ROUTE-025, REQ-ROUTE-026, REQ-QAL-006.
