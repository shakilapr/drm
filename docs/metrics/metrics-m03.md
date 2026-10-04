# Metrics — Month 3 (M03W1–W4)

Gate packet mixed into W1/W3/W4: variables actual-vs-target across the week
plans (work-plan M03W1–W4). Values marked `CI` are verified on runners only
(local Windows cannot execute them). REQ: REQ-QAL-007, REQ-TOOL-001.

## M03W1 — Phase 5 Gate (Metadata Layering)

| Variable | Target | Measured |
|---|---|---|
| `GATE_STATUS(Phase 5)` | PASS | PASS — `docs/gates/phase-05.md`: work-plan #5 criteria ⇄ evidence mapped; fixed = payload-only, variable = LE `u32` length, attach-once validation, boundary-only transport framing, byte-inspection |
| `DESCRIPTOR_BYTE_EXACT` | I-M1 golden match | `descriptor_golden` 2/2 — 152 B field table + zero-padding sweep + write/read symmetry; offset asserts (I-M5) |
| `ATTACH_ONCE` | I-M2: 1e6 reads ⇒ 0 re-checks | `attach_once` 1e6 steady-state view reads, 0 re-validations |
| `PUBLISH_READ_BASIC` | fixed-size round-trip green | `drm-core` channel lib 23 tests incl. round-trip/lap/ABA/busy; `ordering_smoke` release/acquire visibility + epoch pairing |
| `UDP_OVERSIZE_REJECT` | rejected pre-send | `transport::udp::udp_oversize_reject`; `mtu::udp_cap_boundary`; 10k-clean `examples/udp_pair.rs` |
| `VERIFY_LITE_NEGATIVES` | multi-publisher + unknown schema rejected | `verify_lite` 4/4 (V-SW-1 no-publisher, V-SW-2 multi-publisher, clean, determinism) |
| `EFFORT_DAYS_ACTUAL` | 5/eng | on plan |

Deliverables: `docs/spec/metadata-layout.md` (derived spec, Phase 5),
`docs/gates/phase-05.md`, transport/language conformance cell scaffold
(`tests/conformance/README.md`).

## M03W2 — Structured Channels: Billions-Cycle Stress, Rings

| Variable | Target | Measured |
|---|---|---|
| `BILLIONS_CYCLES_LOCAL` | ≥ 1e9 publish/read; 0 torn / 0 mixed-gen / 0 invalid | 3e9 cycles (1e9 × depths 1/3/16), release build, local Windows: `published=3000000000 dropped=0 torn=0 mixed=0 invalid=0`; `stress_structured_billions.rs` |
| `MONOTONIC_SEQUENCE` | strictly monotonic per channel | 0 violations across 3e9 (pattern sequence checked every read) |
| `RING_DEPTH_TESTS` | unit tests depth 1..=64 green | `ring::tests` 10/10 incl. `retention_depths_1_to_64`, `retention_exact`, `wraparound`, `ring_entry_coherence`, `ring_release_order`, `ring_layout_golden` |
| `UDP_1H_STABILITY` | 1 h continuous loopback, 0 malformed | deferred to CI (long run); adapter round-trip green (`udp_adapter_roundtrip`) |
| `FAST_PATH_AUDIT` | 0 alloc / 0 syscall on steady loop | alloc 0/0 (`alloc_counter`, `--features test-count-allocs`); syscall `strace` audit Linux-only → deferred |
| `EFFORT_DAYS_ACTUAL` | 5/eng | on plan |

Deliverables: `TransportAdapter` + `Reliability` v1 + UDP impl
(`drm-router::transport`, `adapter_capability`, `udp_adapter_roundtrip`);
patterned-payload integrity scheme (`drm-core::channel::pattern`) +
permanent fault-injection (`pattern_corruption`); structured stress harness +
`tests/stress/README.md` runbook. Verify hook R≥1 covered by schema parser
(`Depth(0)` rejected at parse).

## M03W3 — Phase 9 Gate, Rings Complete, Priority Skeleton

| Variable | Target | Measured |
|---|---|---|
| `GATE_STATUS(Phase 9)` | PASS | PASS — `docs/gates/phase-09.md`: 3 × 10⁹ publish/read cycles, 0 torn / 0 mixed-gen / 0 invalid / 0 monotonic violations, fault injection exact-class |
| `RING_DEPTHS_GREEN` | R ∈ 1..=64 exhaustive; sampled 65..4096 | `retention_depths_1_to_64` + `ring_depths_property` 65/100/333/1000/4096; nightly `stress_ring_depth_sweep` pseudo-random 65..8192 |
| `RETENTION_EXACT` | exactly R addressable (I1) | `retention_exact`, `retention_depths_1_to_64`, `ring_depths_property`, `wraparound` |
| `PRIORITY_ORDER` | class order exact in queue tests | `drm-router::priority`: `priority_order` (safety→control→status→diagnostics, FIFO within class), `priority_fifo_and_empty` incl. static-cap `QueueFull` |
| `CURSOR_STRUCT` | `ReaderState`/`ReadOutcome` + empty-channel tests | `drm-core::channel` cursor state/outcome (empty path in round-trip/retention suites); full `next`/`latest` bodies track cursor-delivery |
| `EFFORT_DAYS_ACTUAL` | 5/eng | on plan |

Deliverables: `docs/gates/phase-09.md`; `drm-router::priority` pre-stream
class queues (seed, DRR Phase 30); bounded drop-newest UDP receive path
(`RecvQueue`, `udp_backpressure`); ring nightly property sweep.
