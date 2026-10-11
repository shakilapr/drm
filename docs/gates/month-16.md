# Month-16 Review — Phase 32 Sub-Gates (Record/Replay, Serial Interop), M17 Go/No-Go

Chaired by E3 (review chair) at M16W4, Friday. Verdict: **ON_TRACK** — go for
M17 (Phase 32 close-out: introspection/trace/RPC gates, then Studio track
start). Evidence is executable and stays green under the permanent gates.

Normative sources: [work-plan.md](../../work-plan.md) W6 Phase 32;
[record-replay.md](../algorithms/ops/record-replay.md) (I-RC1..I-RC6),
[serial-mcu.md](../algorithms/router/serial-mcu.md) (I-SR1..I-SR7),
[observability-plane.md](../algorithms/ops/observability-plane.md) (I-OB1..I-OB7),
[rpc-actions.md](../algorithms/ops/rpc-actions.md) (I-RPC1..I-RPC10),
[handle-system.md](../algorithms/toolchain/handle-system.md) (I-H3/I-H4/I-H7).
REQ: REQ-QAL-006, REQ-QAL-008, REQ-CORE-086, REQ-CORE-087, REQ-CORE-088,
REQ-CORE-089, REQ-CORE-093, REQ-CORE-094, REQ-OPS-001, REQ-OPS-002,
REQ-OPS-003, REQ-OPS-004, REQ-OPS-005, REQ-OPS-006, REQ-OPS-007,
REQ-OPS-008, REQ-OPS-009, REQ-OPS-010, REQ-OPS-011, REQ-OPS-012,
REQ-OPS-013, REQ-OPS-014, REQ-OPS-015, REQ-OPS-016, REQ-OPS-017,
REQ-OPS-018, REQ-OPS-019, REQ-OPS-020, REQ-OPS-021, REQ-OPS-022,
REQ-OPS-023, REQ-OPS-024, REQ-OPS-025, REQ-ROUTE-024, REQ-ROUTE-025,
REQ-ROUTE-026, REQ-ROUTE-040, REQ-TOOL-036, REQ-TOOL-037, REQ-TOOL-038,
REQ-TOOL-080.

## 1. Variables actuals vs targets (M16)

| Week | Variable | Target | Measured |
|---|---|---|---|
| M16W1 | `RPC_CALL_GREEN` | call/response round-trip green | 1 — `call_round_trip`, `call_remote_failure` (I-RPC1/I-RPC4) |
| M16W1 | `RPC_CID_GREEN` | concurrent dispatch green | 1 — `dispatch_concurrent_no_cross_correlation` (C ∈ 1..=16), `dispatch_admission_busy`, `dispatch_timeout_expires_and_late_terminal_stale` (I-RPC1/3/7/9/10) |
| M16W1 | `REPLAY_E2E` | replay round-trip fixture green | 1 — `replay_roundtrip` (I-RC4) |
| M16W2 | `TRACE_GATES` | I-OB1/I-OB6 suites green | 1 — `trace_never_blocks`, `trace_drop_counter`, `trace_lane_exhaustion` |
| M16W2 | `RPC_DISPATCH_FULL` | I-RPC1 C ∈ 1..=16 green | 1 — `dispatch_concurrent_no_cross_correlation` |
| M16W2 | `RPC_CANCEL_GREEN` | I-RPC2 deterministic cancel green | 1 — `cancel_at_each_phase_boundary`, `action_runs_all_phases` |
| M16W2 | `DIAG_POLISH` | report complete; conditions detected (I-OB4) | 1 — `diag_deadline_matrix`, `diag_condition_matrix`, `diag_aggregate_report` |
| M16W3 | `RPC_CANCEL_GREEN` | deterministic cancel per phase (I-RPC2) | 1 — per-kind counters in `RpcClient`; `cancel_at_each_phase_boundary`, `action_runs_all_phases` |
| M16W3 | `IDS_COMPACT_GREEN` | compaction + tombstones per policy (I-H3/I-H4) | 1 — `handle_compact` 4/4 |
| M16W3 | `SERIAL_MATRIX` | host↔MCU byte-for-byte cells complete | 1 — `serial_cobs_golden`, `mcu_host_interop_matrix`; `make host-test` |
| M16W4 | `RECORD_GATE` | borrow-scope, bounded, policies, MCAP interop, replay faithful | PASS — [m16w4.md](m16w4.md) |
| M16W4 | `SERIAL_GATE` | host↔MCU byte-for-byte + error paths | PASS — [m16w4.md](m16w4.md) |
| M16W4 | `MONTH16_REVIEW` | ON_TRACK | ON_TRACK (this document) |
| M16W4 | `EFFORT_DAYS_ACTUAL` | 5 per engineer | on plan |

## 2. Phase 32 sub-gates ledger

| Sub-gate | Doc | Verdict |
|---|---|---|
| Introspection (echo/hz/log, no-pin) | [phase-32.md](phase-32.md) | PASS (M15) |
| Trace (never-blocks, drop transparency) | [m16w2.md](m16w2.md) | PASS (M16W2) |
| Diag aggregation + deadline state | [m15w2.md](m15w2.md) | PASS (M15) |
| Record/replay | [m16w4.md](m16w4.md) | PASS (M16W4) |
| Serial/MCU interop | [m16w4.md](m16w4.md) | PASS (host); 2 h target soak deferred |
| RPC unary + dispatch ×16 | [m16w1.md](m16w1.md) | PASS (M16W1) |
| RPC action phase machine + cancel | [m16w3.md](m16w3.md) | PASS (M16W3) |
| Handles compaction | [m16w3.md](m16w3.md) | PASS (M16W3) |
| Introspection + trace + RPC close-out | [work-plan-m17-w1.md](../work-plan/work-plan-m17-w1.md) | pending M17W1 |

## 3. Carry-over + quarantine

| Item | Origin | Disposition |
|---|---|---|
| MCU 2 h target soak | M15/M16 serial gates | deferred to `mcu-cross` hardware lane (handover deferral policy); host parity green |
| `drm record`/`drm diag` CLI wiring | Phase 32 wave A note | tracked to M17W1 close-out |
| Studio protocol-level scope | M14W4 design lock | starts M17 after Phase 32 close-out (E4/E2) |
| m10w2 quarantine | M10 | quarantined; repaired or deleted within one week (regression permanence) |

## 4. Risk deltas

| Risk | Status | Mitigation |
|---|---|---|
| MCU target long-tail (board/toolchain availability) | open | reference STM32H743 + fallback STM32F407 locked; QEMU fallback so a missing board never means "not run" ([mcu-bring-up.md](../spec/mcu-bring-up.md)); host parity + linker audit are the in-session gate |
| Trace dictionary versioning | mitigated | `dict_version` append-only per deployment; compaction renumbers under a new version; readers render unknown IDs as raw hex, never a wrong name ([observability-plane.md](../algorithms/ops/observability-plane.md)) |
| Third-party MCAP reader version drift | mitigated | reader version pinned (`mcap 0.25.0`, `Cargo.lock`); `record_mcap_interop` reads the stream through that pinned reader |
| Serial host↔MCU parity drift | mitigated | shared COBS golden vectors + byte-exact cross-decode matrix pin both sides (`serial_cobs_golden`, `mcu_host_interop_matrix`) |

## 5. Registry health; traceability

- Registry lint: 0 findings; citation coverage: every crate/xtask `.rs` file
  carries a REQ citation.
- New this month: none — M16 confirmed existing rows (`REQ-OPS-001`..`025`,
  `REQ-ROUTE-024`..`026`/`040`, `REQ-CORE-086`..`094`, `REQ-TOOL-036`..`038`/
  `080`).
- No architecture divergence and no ADR: every gate restates pinned invariants.

## 6. M17 commitment

- Phase 32 close-out: introspection gate (`INTROSPECT_GATE`), trace gate
  (`TRACE_GATE`), RPC gate (`RPC_GATE`) per [work-plan-m17-w1.md](../work-plan/work-plan-m17-w1.md).
- Studio track start (E4/E2, protocol-level scope) after Phase 32 closes.
- Fix window M17W1 for any M16W4 red; Phase 32 close at M17W2 unaffected.

## How to reproduce

```text
cargo test -p drm-record
cargo test -p drm-router --lib transport::serial
cargo test -p drm-core trace
cargo test -p drm-rpc
make -C bindings/c/mcu host-test host-audit
cargo xtask audit-traceability
cargo xtask check-layering
cargo xtask fmt --check
cargo xtask lint
cargo clippy --workspace --target x86_64-unknown-linux-gnu --all-targets -- -D warnings
cargo xtask test
```

REQ: REQ-QAL-006, REQ-QAL-008, REQ-CORE-086, REQ-CORE-087, REQ-CORE-088,
REQ-CORE-089, REQ-CORE-093, REQ-CORE-094, REQ-OPS-001, REQ-OPS-002,
REQ-OPS-003, REQ-OPS-004, REQ-OPS-005, REQ-OPS-006, REQ-OPS-007,
REQ-OPS-008, REQ-OPS-009, REQ-OPS-010, REQ-OPS-011, REQ-OPS-012,
REQ-OPS-013, REQ-OPS-014, REQ-OPS-015, REQ-OPS-016, REQ-OPS-017,
REQ-OPS-018, REQ-OPS-019, REQ-OPS-020, REQ-OPS-021, REQ-OPS-022,
REQ-OPS-023, REQ-OPS-024, REQ-OPS-025, REQ-ROUTE-024, REQ-ROUTE-025,
REQ-ROUTE-026, REQ-ROUTE-040, REQ-TOOL-036, REQ-TOOL-037, REQ-TOOL-038,
REQ-TOOL-080.
