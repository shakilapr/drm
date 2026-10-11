# Phase 32 Gate — Operational Tooling

Normative sources: [work-plan.md](../../work-plan.md) W6 Phase 32;
[observability-plane.md](../algorithms/ops/observability-plane.md),
[record-replay.md](../algorithms/ops/record-replay.md),
[rpc-actions.md](../algorithms/ops/rpc-actions.md),
[serial-mcu.md](../algorithms/router/serial-mcu.md). Owner: E1 (trace/RPC),
E2 (spatial parser groundwork), E3 (serial/MCU), E4 (introspection/diag).

Status: **PASS** (M17W2) — every Phase 32 submodule conformance suite is green;
sub-designs locked at M14W4 ([tooling-design-lock.md](../spec/tooling-design-lock.md));
gate evidence [m17w2.md](m17w2.md). Studio protocol doc merged protocol-level
([studio-protocol.md](../algorithms/studio/studio-protocol.md)).

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
| RPC/actions, full diag daemon | `drm-rpc` + `drm-diag`; `dispatch_concurrent_no_cross_correlation`/`cancel_at_each_phase_boundary`/`diag_observed_rate`/`diag_deadline_matrix` (I-RPC1..I-RPC3/I-OB3/I-OB4, REQ-OPS-001/011/013); [m16w1.md](m16w1.md)–[m17w1.md](m17w1.md) | PASS (M17W1) |
| RPC request/response + cid dispatch + deadline (unary) | `crates/drm-rpc`; `call_round_trip`/`call_remote_failure`/`dispatch_concurrent_no_cross_correlation`/`dispatch_admission_busy`/`dispatch_timeout_expires_and_late_terminal_stale`/`server_session_stale_dropped`/`event_order_invalid_terminal_first` (I-RPC1/3/4/6/7/9/10, REQ-OPS-011/012/013/020/021/022/023/025); [m16w1.md](m16w1.md) | PASS (M16W1) |
| RPC action phase machine + phase-boundary cancel; dispatch ×16 | `crates/drm-rpc/src/action.rs`; `action_runs_all_phases`/`cancel_at_each_phase_boundary`/`dispatch_concurrent_no_cross_correlation` (C ∈ 1..=16) (I-RPC1/2/8, REQ-OPS-010/011/013); [m16w2.md](m16w2.md) | PASS (M16W2) |
| RPC per-phase state-transition observation + cancel ack | `crates/drm-rpc/src/client.rs` per-kind counters; `action_runs_all_phases`/`cancel_at_each_phase_boundary` (I-RPC2/I-RPC10, REQ-OPS-010/011/025); [m16w3.md](m16w3.md) | PASS (M16W3) |
| Handle compaction gate + coordinated-rebuild guidance | `HandleTable::compact`/`rebuild_guidance`; `drm ids compact --yes`; `handle_compact_gate`/`handle_compact_scope`/`handle_compact_guidance` (I-H3/I-H4/I-H7, REQ-TOOL-038); [m16w3.md](m16w3.md) | PASS (M16W3) |
| Serial host↔MCU interop matrix byte-exact | `serial_cobs_golden`/`mcu_host_interop_matrix`; `bindings/c/mcu/selftest.c` `test_interop_matrix` (I-SR3/I-SR4/I-SR6, REQ-OPS-004/REQ-ROUTE-025/026); [m16w3.md](m16w3.md) | PASS (M16W3) |
| Record/replay sub-gate (borrow scope, bounded, policies, MCAP interop, replay faithful) | `crates/drm-record/src/{lib,mcap,replay}.rs`; `record_borrow_scope`/`record_bounded`/`record_policies`/`replay_roundtrip`/`record_mcap_interop` (I-RC1..I-RC4/I-RC6, REQ-OPS-005..009); [m16w4.md](m16w4.md) | PASS (M16W4) |
| Serial/interop sub-gate (cross-decode, corruption, oversize, zero-heap audit) | `crates/drm-router/src/transport/serial.rs`; `mcu_host_interop_matrix`/`serial_corruption`/`serial_oversize`; `make -C bindings/c/mcu host-audit` (I-SR2/I-SR3/I-SR5/I-SR6, REQ-ROUTE-025/026/REQ-CORE-087); [m16w4.md](m16w4.md) | PASS (M16W4) |
| Introspection gate (read-only, no-pin, rate tolerance) | `crates/drm-diag/src/tests.rs`; `introspect_no_pin`/`diag_observed_rate`/`observer_copies_owned_bytes`/`log_level_filter` (I-OB3/I-OB5, REQ-OPS-001/003); [m17w1.md](m17w1.md) | PASS (M17W1) |
| Trace gate (never-blocks, drop transparency) | `crates/drm-core/tests/trace.rs`; `trace_never_blocks`/`trace_drop_counter`/`trace_lane_exhaustion` (I-OB1/I-OB6, REQ-OPS-015/017); [m17w1.md](m17w1.md) | PASS (M17W1) |
| RPC gate (correlation, cancel, timeout, capacity) | `crates/drm-rpc/src/tests.rs`; `dispatch_concurrent_no_cross_correlation`/`cancel_at_each_phase_boundary`/`dispatch_timeout_expires_and_late_terminal_stale`/`rpc_capacity_busy_gap_forward` (I-RPC1..I-RPC3/I-RPC6/I-RPC7, REQ-OPS-011/013/021/023); [m17w1.md](m17w1.md) | PASS (M17W1) |
| MCU regression gates wired permanently | `.github/workflows/ci.yml` `mcu` job (`host-test`/`host-audit`); `.github/workflows/mcu-cross.yml` `push: [main]` (I-SR5, REQ-CORE-087); [m17w1.md](m17w1.md) | PASS (M17W1) |

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

## Deferred Scope (post-1.0, per M14W4 lock)

Appended at M17W1 ([tooling-design-lock.md](../spec/tooling-design-lock.md);
[studio-prep-survey.md](../spec/studio-prep-survey.md) §5). Phase 32 is
protocol-level; the following Studio UI items stay out of 1.0 scope.

| Item | Disposition |
|---|---|
| Creation UI (wizards/forms) | deferred post-1.0 |
| Editors (text/graph) | deferred post-1.0 |
| Inspector panels | deferred post-1.0 |
| Command palette / keyboard map | deferred post-1.0 |
| DataStore UI | deferred post-1.0 |
| Remote-web serving | deferred post-1.0; direct listening disabled until TLS, origin validation, authentication |
| Extension platform | designed second phase (wasmtime fuel + memory caps) |
| Native adapters | deferred post-1.0 |
| `.drmx` packaging | deferred post-1.0 |
| `drm-visual-schemas` | deferred post-1.0 |

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
- M16W4 closed the record/replay sub-gate (I-RC1..I-RC4 plus third-party MCAP
  interop, I-RC6) and the serial/interop sub-gate (I-SR2/I-SR3/I-SR5/I-SR6);
  month-16 review ON_TRACK ([month-16.md](month-16.md)). Phase 32 close-out
  (introspection/trace/RPC gates) and Studio start carry to M17.
- M17W1 closed the introspection gate (I-OB3/I-OB5, `introspect_no_pin`/
  `diag_observed_rate`), the trace gate (I-OB1/I-OB6), and the RPC gate
  (I-RPC1..I-RPC3 plus the I-RPC6/I-RPC7 capacity + gap-forwarding cell
  `rpc_capacity_busy_gap_forward`); the MCU host parity + zero-forbidden-heap
  suites are wired as permanent blocking CI gates. Deferred post-1.0 Studio UI
  scope recorded above. The Phase 32 gate (all submodules) closes at M17W2
  ([m17w1.md](m17w1.md)).
- M17W2 closed the Phase 32 gate: all submodule conformance suites re-run green
  (introspection/diag 10/10, trace 4/4, RPC 12/12, record/replay 18/18, serial
  15/15, handles/verify 10/10 + 12/12, MCU host parity + zero-heap audit) and
  the protocol-level Studio doc merged
  ([studio-protocol.md](../algorithms/studio/studio-protocol.md), I-ST1..I-ST13;
  [m17w2.md](m17w2.md)). Studio implementation starts M17W3.

REQ: REQ-STUDIO-001, REQ-STUDIO-002, REQ-STUDIO-003, REQ-STUDIO-004,
REQ-STUDIO-005, REQ-STUDIO-006, REQ-STUDIO-007, REQ-STUDIO-008,
REQ-STUDIO-009, REQ-STUDIO-010, REQ-STUDIO-011, REQ-STUDIO-012,
REQ-STUDIO-013, REQ-STUDIO-014,
REQ-OPS-001, REQ-OPS-002, REQ-OPS-003, REQ-OPS-004, REQ-OPS-005,
REQ-OPS-006, REQ-OPS-007, REQ-OPS-008, REQ-OPS-009, REQ-OPS-010,
REQ-OPS-011, REQ-OPS-012, REQ-OPS-013, REQ-OPS-014,
REQ-OPS-015, REQ-OPS-016, REQ-OPS-017, REQ-OPS-018, REQ-OPS-019,
REQ-OPS-020, REQ-OPS-021, REQ-OPS-022, REQ-OPS-023, REQ-OPS-024,
REQ-OPS-025,
REQ-CORE-087, REQ-CORE-088, REQ-CORE-089, REQ-CORE-093, REQ-CORE-094,
REQ-ROUTE-025, REQ-ROUTE-026, REQ-QAL-006.
