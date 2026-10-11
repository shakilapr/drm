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
| Diagnostics out-of-band; observed rate vs declared; deadline state | `drm-diag` `diag_no_overhead`/`diag_observed_rate`/`diag_deadline_matrix`/`diag_epoch_sandwich` (I-OB2/3/4/7) | PASS |
| Introspection read-only; cannot pin bulk samples | `introspect_no_pin`; `drm echo`/`hz`/`log` (I-OB5, REQ-OPS-003) | PASS |
| Spatial parser groundwork (non-breaking) | `parser/spatial.rs` feature `spatial-grammar`; corpus unaffected (M15W1-E2-T1) | PASS |
| MCU toolchain + hello-world bring-up | `bindings/c/mcu/`; `mcu-cross.yml` (M15W1-E3-T1/T2) | skeleton |
| Serial framing interop byte-for-byte | existing `drm-router::transport::serial` COBS/CRC32C + cross-OS host-loopback (REQ-OPS-004) | PASS (host) |
| Record/replay, RPC/actions, full diag daemon | scheduled M16–M18 | pending |

## Notes

- No architecture divergence in wave A: pinned invariants only; no ADR.
- Remaining Phase 32 output (`drm record`, replay, `drm diag`, RPC/actions,
  constrained-device data path) is tracked in the M16–M19 weekly plans.

REQ: REQ-OPS-001, REQ-OPS-002, REQ-OPS-003, REQ-OPS-004, REQ-OPS-014,
REQ-OPS-015, REQ-OPS-016, REQ-OPS-017, REQ-CORE-087, REQ-CORE-093,
REQ-CORE-094, REQ-QAL-006.
