# Phase 30 Gate — Router Scheduling, Reliability & Shaping

Chaired at M12W1 (E4, evidence by E2/E4; E3 presents). Evidence is executable and
stays green under the permanent gates. Verdict: **PASS** (the four work-plan #30
acceptance criteria; `I-LS1`..`I-LS5`/`I-LS7`/`I-LS9` supporting evidence).

Implementation: `crates/drm-router/src/lane.rs` (per-peer realtime + bulk ordered
lanes, static caps, reliability-aware bulk), `crates/drm-router/src/shaping.rs`
(`TokenBucket`, bit·ns fixed point), `crates/drm-router/src/priority.rs`
(pre-stream class queues), `crates/drm-router/src/egress.rs` (`PeerEgress`).
Algorithm: [lanes-shaping.md](../algorithms/router/lanes-shaping.md). Owner: E3.
Gate chair: E4.

## Criteria ⇄ evidence

| Criterion (work-plan #30) | Invariant | Artifact | Test | Result |
|---|---|---|---|---|
| Bulk saturation never delays realtime reliable traffic on the same ordered lane | `I-LS1`, `I-LS6` | `PeerLanes` realtime/bulk split; `PeerEgress` strict class service | `lane::lane_no_cross_class_queueing`, `priority_saturation::priority_saturation` | PASS |
| Configured rate limits hold within defined tolerance | `I-LS3`, `I-LS4` | `shaping::TokenBucket` (10 000 deterministic traces, byte accounting) | `shaping::shaping_property` | PASS |
| Reliable-over-unsupported transport is rejected | `V-REL` | verify engine capability table ([verify-engine.md](../algorithms/toolchain/verify-engine.md)) | `verify_negatives_v2::verify_negatives_v2_reliability` | PASS |
| Priority ordering follows declared policy under artificial saturation | `I-LS2`, `I-LS6` | pre-stream class queues + per-peer lanes | `priority_saturation::priority_saturation`, `lane::lane_priority_order`, `lane::lane_fifo_within_class` | PASS |

## Supporting evidence

| Case | Invariant | Test | Result |
|---|---|---|---|
| FIFO within a class (property) | `I-LS2` | `lane::lane_fifo_property` | PASS |
| Static caps; best-effort drop-oldest vs reliable backpressure | `I-LS5` | `lane::lane_caps_enforced`, `lane::lane_socket_disconnect_disposition_best_effort`, `lane::lane_socket_disconnect_disposition_reliable_bulk` | PASS |
| No permanent fractional loss at low rate | `I-LS7` | `shaping::token_bucket_fractional` | PASS |
| Burst ≥ charge; empty-bucket wait matches the model | `I-LS9` | `shaping::burst_then_refill`, `shaping::shaping_wait_model` | PASS |
| Two-lane exchange / mixed-class two-peer determinism | `I-LS10` | `lane_egress::lane_egress_two_lane_exchange`, `lane_mixed_class::lane_mixed_class_two_peer` | PASS |
| Datagram egress passes the same class scheduling | `I-LS10` | `lane_egress::lane_egress_service_strict_class_order`, `lane_egress::lane_egress_classification_into_lane` | PASS |
| Connect/disconnect lifecycle (session-scoped reliability) | `I-LS5` | `lane::lane_socket_connect_disconnect_transition`, `lane::lane_socket_caps_and_fifo_while_connected` | PASS |

## Windows backend smoke (Phase 31 start)

The Windows SHM + `LockFileEx` backend lands this week
([windows-backends.md](../algorithms/runtime/windows-backends.md)); the
`drm-core` Windows smoke suite is green on the `windows-latest` runner.

| Case | Invariant | Test | Result |
|---|---|---|---|
| Section create/map round-trip; second exclusive create loses | `I-W1` | `shm::windows::tests::win_section_roundtrip` | PASS |
| Open never creates | `I-W2` | `shm::windows::tests::win_open_semantics` | PASS |
| `VirtualLock` locks or refuses | `I-W6` | `shm::windows::tests::win_precommit_roundtrip` | PASS |
| `LockFileEx` acquire/release (exclusive vs shared) | `I-W3`, `I-W8` | `shm::windows::tests::win_lock_roundtrip` | PASS |
| Diagnostic identity + incarnation guard | `I-W5` | `shm::windows::tests::win_process_identity` | PASS |

## How to reproduce

```text
cargo test -p drm-router --test lane
cargo test -p drm-router --test lane_egress
cargo test -p drm-router --test lane_mixed_class
cargo test -p drm-router --test priority_saturation
cargo test -p drm-router shaping
cargo test -p drm-workspace --test verify_negatives_v2
cargo test -p drm-core --lib win_          # Windows runner
```

## Notes

- The two-host physical matrix (bulk flood vs realtime critical on real links) is
  recorded in the M11 runbook; downstream shared resources (NIC/qdisc/switch) can
  still add latency, so the blocking gate is the synthetic `I-LS1` isolation
  suite and the physical matrix is reported as measured, not guaranteed
  ([lanes-shaping.md](../algorithms/router/lanes-shaping.md) `I-LS1`).
- `I-LS4` scope: shaping applies to DRM-generated frames only; foreign traffic is
  outside DRM's control (documented, not a defect).
- The dedicated safety-budget isolation and deficit-round-robin increments
  (`I-LS6` full, `I-LS8`, `I-LS11`, `I-LS12`) extend `LinkScheduler` with the W5
  scheduler hardening; the #30 acceptance criteria above do not depend on them.
